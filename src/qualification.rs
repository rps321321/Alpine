use crate::config;
use crate::decision::Decision;
use crate::evidence::{EvidenceStore, MeasuredSample, RunEvidence, StoredIdentity};
use crate::experiment::current_microbenchmark_identity;
use crate::external::{self, ExternalEvidenceStatus, ExternalEvidenceStatusKind};
use crate::identity::{sha256_bytes, sha256_file};
use crate::support::{self, SupportReport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceIdentity {
    pub hardware: String,
    pub software: String,
    pub model: String,
    pub runtime: String,
    pub workload: String,
    pub configuration: String,
    pub policy: String,
}

impl EvidenceIdentity {
    fn complete(&self) -> bool {
        [
            &self.hardware,
            &self.software,
            &self.model,
            &self.runtime,
            &self.workload,
            &self.configuration,
            &self.policy,
        ]
        .into_iter()
        .all(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidencePhase {
    Tuning,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub phase: EvidencePhase,
    pub identity: EvidenceIdentity,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualificationRequest {
    pub schema: u32,
    pub support: Decision,
    pub claim_identity: EvidenceIdentity,
    #[serde(default)]
    pub tuning_evidence_ids: Vec<String>,
    #[serde(default)]
    pub final_evidence_ids: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRecord>,
    pub regression_detected: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QualificationReport {
    pub schema: u32,
    pub decision: Decision,
    pub reasons: Vec<String>,
    pub accepted_final_evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QualificationTarget {
    Candidate,
    Validated,
    Production,
}

impl QualificationTarget {
    fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Validated => "validated",
            Self::Production => "production",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunQualificationOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub database: PathBuf,
    pub final_run_id: String,
    pub tuning_run_ids: Vec<String>,
    pub target: QualificationTarget,
    pub support_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QualificationCheck {
    pub name: String,
    pub passed: bool,
    pub observed: Value,
    pub required: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunQualificationReport {
    pub schema: u32,
    pub decision: Decision,
    pub target: QualificationTarget,
    pub final_run_id: String,
    pub tuning_run_ids: Vec<String>,
    pub identity: Option<EvidenceIdentity>,
    pub support: SupportReport,
    pub checks: Vec<QualificationCheck>,
    pub external_evidence: Vec<ExternalEvidenceStatus>,
    pub missing_external_evidence: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PromotionGate {
    inherits: Option<String>,
    required_workloads: Option<Vec<String>>,
    minimum_measured_samples_per_workload: Option<u32>,
    require_quality_pass: Option<bool>,
    require_deterministic_outputs: Option<bool>,
    performance_metric_by_workload: Option<BTreeMap<String, PerformanceMetric>>,
    maximum_performance_coefficient_of_variation: Option<f64>,
    maximum_median_performance_regression_fraction: Option<f64>,
    minimum_tuning_selection_improvement_fraction: Option<f64>,
    #[serde(default)]
    requires_external_evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PerformanceMetric {
    PrefillTps,
    DecodeTps,
}

impl PerformanceMetric {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PrefillTps => "prefill_tps",
            Self::DecodeTps => "decode_tps",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PromotionPolicy {
    schema: u32,
    lifecycle: Vec<String>,
    gates: BTreeMap<String, PromotionGate>,
}

struct LoadedRun {
    evidence: RunEvidence,
    identity: Option<EvidenceIdentity>,
    phase: Option<EvidencePhase>,
    samples: Vec<MeasuredSample>,
}

pub fn qualify(request: &QualificationRequest) -> Result<QualificationReport, String> {
    if request.schema != 1 {
        return Err(format!(
            "unsupported Qualification Request schema {}; expected 1",
            request.schema
        ));
    }
    if !request.claim_identity.complete() {
        return Ok(report(
            Decision::NotProven,
            "claim identity is incomplete",
            vec![],
        ));
    }
    if request.support == Decision::Unsupported {
        return Ok(report(
            Decision::Unsupported,
            "Support Envelope rejected this configuration",
            vec![],
        ));
    }
    if request.support == Decision::Inconclusive {
        return Ok(report(
            Decision::Inconclusive,
            "support could not be established",
            vec![],
        ));
    }
    if request.support == Decision::Regressed {
        return Ok(report(
            Decision::Regressed,
            "support evidence already records a regression",
            vec![],
        ));
    }
    if request.regression_detected == Some(true) {
        return Ok(report(
            Decision::Regressed,
            "a policy-defined regression was measured",
            vec![],
        ));
    }

    let tuning: BTreeSet<_> = request.tuning_evidence_ids.iter().collect();
    let final_ids: BTreeSet<_> = request.final_evidence_ids.iter().collect();
    if tuning.len() != request.tuning_evidence_ids.len()
        || final_ids.len() != request.final_evidence_ids.len()
    {
        return Ok(report(
            Decision::NotProven,
            "qualification evidence references contain duplicate ids",
            vec![],
        ));
    }
    if final_ids.is_empty() {
        return Ok(report(
            Decision::NotProven,
            "final qualification evidence is missing",
            vec![],
        ));
    }
    if tuning.iter().any(|id| final_ids.contains(id)) {
        return Ok(report(
            Decision::NotProven,
            "final qualification evidence overlaps tuning or selection evidence",
            vec![],
        ));
    }

    let records: BTreeMap<_, _> = request
        .evidence
        .iter()
        .map(|record| (&record.id, record))
        .collect();
    if records.len() != request.evidence.len() {
        return Ok(report(
            Decision::NotProven,
            "qualification evidence contains duplicate record ids",
            vec![],
        ));
    }
    for id in tuning {
        let Some(record) = records.get(id) else {
            return Ok(report(
                Decision::NotProven,
                "referenced tuning evidence is missing",
                vec![],
            ));
        };
        if record.phase != EvidencePhase::Tuning || record.identity != request.claim_identity {
            return Ok(report(
                Decision::NotProven,
                "tuning evidence phase or identity does not match the qualification claim",
                vec![],
            ));
        }
    }
    let mut accepted = Vec::new();
    for id in final_ids {
        let Some(record) = records.get(id) else {
            return Ok(report(
                Decision::NotProven,
                "referenced final evidence is missing",
                vec![],
            ));
        };
        if record.phase != EvidencePhase::Final {
            return Ok(report(
                Decision::NotProven,
                "final evidence has the wrong phase",
                vec![],
            ));
        }
        if record.identity != request.claim_identity {
            return Ok(report(
                Decision::NotProven,
                "final evidence identity does not match the qualification claim",
                vec![],
            ));
        }
        if !record.passed {
            return Ok(report(
                Decision::Unsupported,
                "final correctness evidence failed",
                vec![],
            ));
        }
        accepted.push(record.id.clone());
    }
    if request.regression_detected.is_none() {
        return Ok(report(
            Decision::Inconclusive,
            "regression comparison was not established",
            accepted,
        ));
    }
    Ok(report(
        Decision::Qualified,
        "all required independent evidence passed",
        accepted,
    ))
}

pub fn qualify_run(options: &RunQualificationOptions) -> Result<RunQualificationReport, String> {
    let repository_root = std::fs::canonicalize(&options.repository_root).map_err(|error| {
        format!(
            "failed to resolve repository root {}: {error}",
            options.repository_root.display()
        )
    })?;
    if !repository_root.is_dir() {
        return Err(format!(
            "repository root is not a directory: {}",
            repository_root.display()
        ));
    }
    let policy_path = repository_root.join("config/promotion-policy.json");
    let policy_bytes = std::fs::read(&policy_path)
        .map_err(|error| format!("failed to read {}: {error}", policy_path.display()))?;
    let policy: PromotionPolicy = serde_json::from_slice(&policy_bytes).map_err(|error| {
        format!(
            "invalid promotion policy {}: {error}",
            policy_path.display()
        )
    })?;
    let gate = inherited_gate(&policy, options.target)?;
    let policy_sha256 = sha256_bytes(&policy_bytes);

    let envelope_path = repository_root.join("config/support-envelope.json");
    let (envelope, envelope_bytes) = support::read_envelope(&envelope_path)?;
    let support = support::inspect(&envelope, &envelope_bytes, options.support_timeout)?;

    let store = EvidenceStore::open_read_only(&options.database)?;
    let final_run = load_run(&store, &options.final_run_id)?;
    let mut tuning_runs = Vec::with_capacity(options.tuning_run_ids.len());
    for id in &options.tuning_run_ids {
        tuning_runs.push(load_run(&store, id)?);
    }

    let mut checks = Vec::new();
    let mut not_proven = false;
    let mut unsupported = false;
    let mut inconclusive = false;
    let mut regressed = false;

    let tuning_ids = options.tuning_run_ids.iter().collect::<BTreeSet<_>>();
    let independent_ids = !options.tuning_run_ids.is_empty()
        && tuning_ids.len() == options.tuning_run_ids.len()
        && !tuning_ids.contains(&options.final_run_id);
    add_check(
        &mut checks,
        "independent-evidence-ids",
        independent_ids,
        json!({"final": options.final_run_id, "tuning": options.tuning_run_ids}),
        json!("one or more unique tuning ids, none equal to the final id"),
    );
    not_proven |= !independent_ids;

    let final_passed = final_run.evidence.summary.status == "passed"
        && final_run.evidence.summary.finished_at.is_some();
    add_check(
        &mut checks,
        "final-run-status",
        final_passed,
        json!({
            "status": final_run.evidence.summary.status,
            "finished_at": final_run.evidence.summary.finished_at,
        }),
        json!("finished passed run"),
    );
    unsupported |= !final_passed && final_run.evidence.summary.status == "failed-quality";
    not_proven |= !final_passed && !unsupported;

    let final_phase = final_run.phase == Some(EvidencePhase::Final);
    add_check(
        &mut checks,
        "final-evidence-phase",
        final_phase,
        json!(final_run.phase),
        json!(EvidencePhase::Final),
    );
    not_proven |= !final_phase;
    for run in &tuning_runs {
        let passed = run.phase == Some(EvidencePhase::Tuning)
            && run.evidence.summary.status == "passed"
            && run.evidence.summary.finished_at.is_some();
        add_check(
            &mut checks,
            &format!("tuning-evidence:{}", run.evidence.summary.id),
            passed,
            json!({"phase": run.phase, "status": run.evidence.summary.status}),
            json!({"phase": EvidencePhase::Tuning, "status": "passed"}),
        );
        not_proven |= !passed;
    }

    let claim_identity = final_run.identity.clone();
    let identities_match = claim_identity.as_ref().is_some_and(|identity| {
        tuning_runs
            .iter()
            .all(|run| run.identity.as_ref() == Some(identity))
    });
    add_check(
        &mut checks,
        "exact-material-identity",
        identities_match,
        json!({
            "final": claim_identity,
            "tuning": tuning_runs.iter().map(|run| &run.identity).collect::<Vec<_>>(),
        }),
        json!("all seven identity dimensions are complete and exactly equal"),
    );
    not_proven |= !identities_match;

    let current_policy = claim_identity
        .as_ref()
        .is_some_and(|identity| identity.policy == policy_sha256);
    add_check(
        &mut checks,
        "current-promotion-policy",
        current_policy,
        json!(claim_identity.as_ref().map(|identity| &identity.policy)),
        json!(policy_sha256),
    );
    not_proven |= !current_policy;

    let selected_workloads = configured_workloads(&final_run.evidence.config)?;
    let current_workload_sha256 =
        current_microbenchmark_identity(&repository_root, &selected_workloads)?;
    let current_workloads = claim_identity
        .as_ref()
        .is_some_and(|identity| identity.workload == current_workload_sha256);
    add_check(
        &mut checks,
        "current-workload-suite",
        current_workloads,
        json!(claim_identity.as_ref().map(|identity| &identity.workload)),
        json!(current_workload_sha256),
    );
    not_proven |= !current_workloads;

    let full_model_hash = final_run
        .evidence
        .config
        .pointer("/model_verification/method")
        .and_then(Value::as_str)
        == Some("full-sha256")
        && final_run
            .evidence
            .config
            .pointer("/model_verification/sha256")
            .and_then(Value::as_str)
            == final_run.evidence.model_sha256.as_deref();
    add_check(
        &mut checks,
        "fresh-final-model-digest",
        full_model_hash,
        final_run
            .evidence
            .config
            .pointer("/model_verification")
            .cloned()
            .unwrap_or(Value::Null),
        json!("full-sha256 matching the run model identity"),
    );
    not_proven |= !full_model_hash;

    let declared_configurations = std::iter::once(&final_run)
        .chain(tuning_runs.iter())
        .map(|run| {
            Ok((
                run.evidence.summary.id.clone(),
                run.identity
                    .as_ref()
                    .map(|identity| identity.configuration.clone()),
                Some(material_configuration_sha256(&run.evidence)?),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let configuration_claims_match = declared_configurations
        .iter()
        .all(|(_, declared, computed)| declared.is_some() && declared == computed);
    add_check(
        &mut checks,
        "recomputed-material-configuration",
        configuration_claims_match,
        json!(declared_configurations),
        json!("declared configuration identity equals recomputed material configuration"),
    );
    not_proven |= !configuration_claims_match;

    let current_binary_sha256 = sha256_file(
        &std::env::current_exe()
            .map_err(|error| format!("failed to locate Alpine executable: {error}"))?,
    )?;
    let current_software = claim_identity
        .as_ref()
        .is_some_and(|identity| identity.software == current_binary_sha256);
    add_check(
        &mut checks,
        "current-alpine-binary",
        current_software,
        json!(claim_identity.as_ref().map(|identity| &identity.software)),
        json!(current_binary_sha256),
    );
    not_proven |= !current_software;

    let hardware_current = current_hardware_identity(&repository_root, &final_run.evidence)?
        == claim_identity
            .as_ref()
            .map(|identity| identity.hardware.clone());
    add_check(
        &mut checks,
        "current-hardware-manifest",
        hardware_current,
        json!(claim_identity.as_ref().map(|identity| &identity.hardware)),
        json!("current bytes at the run's repository-relative hardware manifest"),
    );
    not_proven |= !hardware_current;

    let benchmark_contract = final_run
        .evidence
        .config
        .pointer("/benchmark/name")
        .and_then(Value::as_str)
        == Some("micro")
        && final_run
            .evidence
            .config
            .pointer("/benchmark/schema")
            .and_then(Value::as_u64)
            == Some(2);
    add_check(
        &mut checks,
        "benchmark-contract",
        benchmark_contract,
        json!({
            "name": final_run.evidence.config.pointer("/benchmark/name"),
            "schema": final_run.evidence.config.pointer("/benchmark/schema"),
        }),
        json!({"name": "micro", "schema": 2}),
    );
    not_proven |= !benchmark_contract;

    let resolved = config::resolve(
        &options.install_root,
        Some(&final_run.evidence.summary.profile),
        true,
    )?;
    let launch = final_run
        .evidence
        .config
        .get("launch")
        .ok_or_else(|| "final run launch evidence is missing".to_owned())?;
    let current_server_sha256 = sha256_file(&resolved.server)?;
    let current_install = launch.get("profile_sha256").and_then(Value::as_str)
        == Some(resolved.profile_sha256.as_str())
        && launch.get("session_config_sha256").and_then(Value::as_str)
            == Some(resolved.session_config_sha256.as_str())
        && launch.get("server_sha256").and_then(Value::as_str)
            == Some(current_server_sha256.as_str())
        && canonical_json_path(launch.get("server"))?
            == std::fs::canonicalize(&resolved.server).ok()
        && canonical_json_path(
            final_run
                .evidence
                .config
                .pointer("/model_verification/path"),
        )? == std::fs::canonicalize(&resolved.model).ok()
        && unchanged_verified_model(&final_run.evidence)?;
    add_check(
        &mut checks,
        "current-installation-artifacts",
        current_install,
        json!({
            "profile_sha256": launch.get("profile_sha256"),
            "session_config_sha256": launch.get("session_config_sha256"),
            "server_sha256": launch.get("server_sha256"),
            "model_verification": final_run.evidence.config.get("model_verification"),
        }),
        json!("current Profile, Session Config, runtime and model match final evidence"),
    );
    not_proven |= !current_install;

    let required_workloads = gate.required_workloads.clone().unwrap_or_default();
    let performance_metrics = gate
        .performance_metric_by_workload
        .clone()
        .unwrap_or_default();
    let minimum_samples = gate.minimum_measured_samples_per_workload.unwrap_or(0);
    let maximum_cv = gate.maximum_performance_coefficient_of_variation;
    let maximum_regression = gate.maximum_median_performance_regression_fraction;
    let final_groups = group_samples(&final_run.samples);
    let tuning_samples = tuning_runs
        .iter()
        .flat_map(|run| run.samples.iter().cloned())
        .collect::<Vec<_>>();
    let tuning_groups = group_samples(&tuning_samples);

    if gate.require_quality_pass.unwrap_or(false) {
        let global_quality = !final_run.samples.is_empty()
            && final_run
                .samples
                .iter()
                .all(|sample| sample.quality_pass == Some(true));
        add_check(
            &mut checks,
            "all-measured-rows:quality",
            global_quality,
            json!({
                "measured_rows": final_run.samples.len(),
                "failed_or_missing": final_run.samples.iter()
                    .filter(|sample| sample.quality_pass != Some(true))
                    .count(),
            }),
            json!("every non-warmup SQLite row passes quality"),
        );
        unsupported |= !global_quality && !final_run.samples.is_empty();
        not_proven |= final_run.samples.is_empty();
    }
    if gate.require_deterministic_outputs.unwrap_or(false) {
        let global_determinism = !final_groups.is_empty()
            && final_groups.values().all(|rows| {
                let hashes = rows
                    .iter()
                    .filter_map(|sample| sample.output_sha256.as_deref())
                    .collect::<BTreeSet<_>>();
                hashes.len() == 1
                    && rows
                        .iter()
                        .all(|sample| sample.output_sha256.as_deref().is_some_and(is_sha256))
            });
        add_check(
            &mut checks,
            "all-measured-rows:determinism",
            global_determinism,
            json!({"measured_workloads": final_groups.keys().collect::<Vec<_>>() }),
            json!("one complete valid output digest per measured workload"),
        );
        unsupported |= !global_determinism && !final_run.samples.is_empty();
        not_proven |= final_run.samples.is_empty();
    }

    for workload in &required_workloads {
        let metric = performance_metrics
            .get(workload)
            .copied()
            .ok_or_else(|| format!("performance metric is missing for {workload}"))?;
        let final_rows = final_groups
            .get(workload.as_str())
            .cloned()
            .unwrap_or_default();
        let enough_samples = final_rows.len() >= minimum_samples as usize;
        add_check(
            &mut checks,
            &format!("{workload}:sample-count"),
            enough_samples,
            json!(final_rows.len()),
            json!({"minimum": minimum_samples}),
        );
        not_proven |= !enough_samples;
        let iterations = final_rows
            .iter()
            .map(|sample| sample.iteration)
            .collect::<Vec<_>>();
        let contiguous_iterations = iterations
            .iter()
            .enumerate()
            .all(|(offset, iteration)| *iteration == offset as u32 + 1);
        add_check(
            &mut checks,
            &format!("{workload}:sample-index-integrity"),
            contiguous_iterations,
            json!(iterations),
            json!("contiguous one-based measured iterations"),
        );
        not_proven |= !contiguous_iterations;

        if gate.require_quality_pass.unwrap_or(false) {
            let quality = !final_rows.is_empty()
                && final_rows
                    .iter()
                    .all(|sample| sample.quality_pass == Some(true));
            add_check(
                &mut checks,
                &format!("{workload}:quality"),
                quality,
                json!(
                    final_rows
                        .iter()
                        .map(|sample| sample.quality_pass)
                        .collect::<Vec<_>>()
                ),
                json!("every measured sample passes"),
            );
            unsupported |= !quality && !final_rows.is_empty();
            not_proven |= final_rows.is_empty();
        }

        if gate.require_deterministic_outputs.unwrap_or(false) {
            let hashes = final_rows
                .iter()
                .filter_map(|sample| sample.output_sha256.as_deref())
                .collect::<BTreeSet<_>>();
            let deterministic = !final_rows.is_empty()
                && hashes.len() == 1
                && final_rows
                    .iter()
                    .all(|sample| sample.output_sha256.as_deref().is_some_and(is_sha256));
            add_check(
                &mut checks,
                &format!("{workload}:determinism"),
                deterministic,
                json!(hashes),
                json!("one complete output digest across measured samples"),
            );
            unsupported |= !deterministic && !final_rows.is_empty();
            not_proven |= final_rows.is_empty();
        }

        let final_distribution = performance_distribution(&final_rows, metric);
        if let Some(limit) = maximum_cv {
            let cv = final_distribution
                .as_ref()
                .and_then(|(_, values)| coefficient_of_variation(values));
            let stable = cv.is_some_and(|value| value <= limit);
            add_check(
                &mut checks,
                &format!("{workload}:performance-cv"),
                stable,
                json!(cv),
                json!({"maximum": limit}),
            );
            if cv.is_some() {
                inconclusive |= !stable;
            } else {
                not_proven = true;
            }
        }

        let tuning_rows = tuning_groups
            .get(workload.as_str())
            .cloned()
            .unwrap_or_default();
        let tuning_distribution = performance_distribution(&tuning_rows, metric);
        let comparison = final_distribution
            .as_ref()
            .zip(tuning_distribution.as_ref())
            .and_then(
                |((final_metric, final_values), (tuning_metric, tuning_values))| {
                    (final_metric == tuning_metric).then(|| {
                        let final_median = median(final_values);
                        let tuning_median = median(tuning_values);
                        let regression_fraction = if tuning_median > 0.0 {
                            (tuning_median - final_median) / tuning_median
                        } else {
                            f64::NAN
                        };
                        (
                            *final_metric,
                            final_median,
                            tuning_median,
                            regression_fraction,
                        )
                    })
                },
            );
        let regression_pass = maximum_regression
            .zip(comparison)
            .is_some_and(|(limit, (_, _, _, fraction))| fraction.is_finite() && fraction <= limit);
        add_check(
            &mut checks,
            &format!("{workload}:independent-regression"),
            regression_pass,
            json!(
                comparison.map(|(metric, final_median, tuning_median, fraction)| json!({
                    "metric": metric,
                    "final_median": final_median,
                    "tuning_median": tuning_median,
                    "regression_fraction": fraction,
                }))
            ),
            json!(maximum_regression.map(|limit| json!({"maximum_fraction": limit}))),
        );
        match (maximum_regression, comparison) {
            (Some(limit), Some((_, _, _, fraction))) if fraction.is_finite() => {
                regressed |= fraction > limit;
            }
            _ => not_proven = true,
        }
    }

    let external_evidence = if let Some(identity) = claim_identity.as_ref() {
        let database_path = std::fs::canonicalize(&options.database).map_err(|error| {
            format!(
                "failed to resolve evidence database {}: {error}",
                options.database.display()
            )
        })?;
        let result_root = database_path
            .parent()
            .ok_or_else(|| "evidence database has no result-root parent".to_owned())?;
        external::inspect_required(
            &store,
            &options.final_run_id,
            identity,
            &gate.requires_external_evidence,
            &repository_root,
            result_root,
            &current_binary_sha256,
        )?
    } else {
        Vec::new()
    };
    let missing_external_evidence = if claim_identity.is_some() {
        external_evidence
            .iter()
            .filter(|evidence| evidence.status != ExternalEvidenceStatusKind::Satisfied)
            .map(|evidence| evidence.name.clone())
            .collect::<Vec<_>>()
    } else {
        gate.requires_external_evidence.clone()
    };
    add_check(
        &mut checks,
        "external-evidence",
        missing_external_evidence.is_empty(),
        json!({"missing": missing_external_evidence}),
        json!("all inherited external-evidence requirements satisfied"),
    );
    not_proven |= !missing_external_evidence.is_empty();

    let (decision, reason) = match support.decision {
        Decision::Unsupported => (
            Decision::Unsupported,
            "the current host is outside the Support Envelope",
        ),
        Decision::Inconclusive => (
            Decision::Inconclusive,
            "the current host Support Envelope probes are inconclusive",
        ),
        Decision::Regressed => (
            Decision::Regressed,
            "the current host Support Envelope reports a regression",
        ),
        _ if unsupported => (
            Decision::Unsupported,
            "independent final correctness evidence failed",
        ),
        _ if regressed => (
            Decision::Regressed,
            "independent final performance regressed beyond policy",
        ),
        _ if inconclusive => (
            Decision::Inconclusive,
            "independent final measurements are too unstable for qualification",
        ),
        _ if not_proven => (
            Decision::NotProven,
            "required independent identity-bound evidence is incomplete",
        ),
        _ => (
            Decision::Qualified,
            "all inherited promotion gates passed with independent final evidence",
        ),
    };
    Ok(RunQualificationReport {
        schema: 1,
        decision,
        target: options.target,
        final_run_id: options.final_run_id.clone(),
        tuning_run_ids: options.tuning_run_ids.clone(),
        identity: claim_identity,
        support,
        checks,
        external_evidence,
        missing_external_evidence,
        reasons: vec![reason.to_owned()],
    })
}

fn load_run(store: &EvidenceStore, id: &str) -> Result<LoadedRun, String> {
    let evidence = store
        .run(id)?
        .ok_or_else(|| format!("evidence run not found: {id}"))?;
    if evidence.summary.kind != "micro" {
        return Err(format!(
            "run {id} has kind '{}'; only Rust microbenchmark evidence is qualifiable",
            evidence.summary.kind
        ));
    }
    let identity = evidence_identity(&evidence.identity);
    let phase = evidence
        .config
        .get("evidence_phase")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    let samples = store.measured_samples(id)?;
    Ok(LoadedRun {
        evidence,
        identity,
        phase,
        samples,
    })
}

fn evidence_identity(identity: &StoredIdentity) -> Option<EvidenceIdentity> {
    let identity = EvidenceIdentity {
        hardware: identity.hardware.clone()?,
        software: identity.software.clone()?,
        model: identity.model.clone()?,
        runtime: identity.runtime.clone()?,
        workload: identity.workload.clone()?,
        configuration: identity.configuration.clone()?,
        policy: identity.policy.clone()?,
    };
    let valid = [
        &identity.hardware,
        &identity.software,
        &identity.model,
        &identity.runtime,
        &identity.workload,
        &identity.configuration,
        &identity.policy,
    ]
    .into_iter()
    .all(|value| is_sha256(value));
    valid.then_some(identity)
}

fn configured_workloads(config: &Value) -> Result<Vec<String>, String> {
    let value = config
        .pointer("/benchmark/workloads")
        .ok_or_else(|| "run benchmark workload selection is missing".to_owned())?;
    if value.as_str() == Some("all") {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .ok_or_else(|| "run benchmark workloads must be 'all' or an array".to_owned())?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "run benchmark workload id must be a string".to_owned())
        })
        .collect()
}

pub(crate) fn material_configuration_sha256(evidence: &RunEvidence) -> Result<String, String> {
    let model_sha256 = evidence
        .model_sha256
        .as_deref()
        .ok_or_else(|| format!("run {} model identity is missing", evidence.summary.id))?;
    let profile = evidence
        .config
        .get("profile")
        .ok_or_else(|| format!("run {} Profile evidence is missing", evidence.summary.id))?;
    let benchmark = evidence
        .config
        .get("benchmark")
        .ok_or_else(|| format!("run {} benchmark evidence is missing", evidence.summary.id))?;
    let launch = evidence
        .config
        .get("launch")
        .ok_or_else(|| format!("run {} launch evidence is missing", evidence.summary.id))?;
    let policy_sha256 = evidence
        .config
        .pointer("/qualification_policy/sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("run {} policy identity is missing", evidence.summary.id))?;
    let material_launch = json!({
        "runtime": launch.get("runtime"),
        "server_sha256": launch.get("server_sha256"),
        "runtime_build_sha256": launch.get("runtime_build_sha256"),
        "profile_sha256": launch.get("profile_sha256"),
        "session_config_sha256": launch.get("session_config_sha256"),
        "arguments": launch.get("arguments"),
        "environment": launch.get("environment"),
    });
    let material = json!({
        "model_sha256": model_sha256,
        "profile": profile,
        "benchmark": benchmark,
        "launch": material_launch,
        "qualification_policy_sha256": policy_sha256,
    });
    let bytes = serde_json::to_vec(&material)
        .map_err(|error| format!("failed to encode material configuration: {error}"))?;
    Ok(sha256_bytes(&bytes))
}

pub(crate) fn current_hardware_identity(
    repository_root: &Path,
    evidence: &RunEvidence,
) -> Result<Option<String>, String> {
    let relative = evidence
        .hardware_manifest
        .as_deref()
        .ok_or_else(|| format!("run {} hardware manifest is missing", evidence.summary.id))?;
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "run {} hardware manifest must stay inside the repository",
            evidence.summary.id
        ));
    }
    if evidence
        .config
        .pointer("/hardware/path")
        .and_then(Value::as_str)
        != Some(relative)
    {
        return Ok(None);
    }
    sha256_file(&repository_root.join(path)).map(Some)
}

fn canonical_json_path(value: Option<&Value>) -> Result<Option<PathBuf>, String> {
    let Some(path) = value.and_then(Value::as_str) else {
        return Ok(None);
    };
    std::fs::canonicalize(path)
        .map(Some)
        .map_err(|error| format!("recorded artifact path is unavailable at {path}: {error}"))
}

fn unchanged_verified_model(evidence: &RunEvidence) -> Result<bool, String> {
    let verification = evidence
        .config
        .get("model_verification")
        .ok_or_else(|| "final run model verification is missing".to_owned())?;
    let path = verification
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "final run model verification path is missing".to_owned())?;
    let expected_bytes = verification
        .get("bytes")
        .and_then(Value::as_u64)
        .ok_or_else(|| "final run model byte count is missing".to_owned())?;
    let expected_modified = verification
        .get("modified_unix_nanos")
        .and_then(Value::as_u64)
        .ok_or_else(|| "final run model modification time is missing".to_owned())?;
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect verified model at {path}: {error}"))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("verified model modification time is unavailable: {error}"))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "verified model modification time predates Unix epoch".to_owned())?
        .as_nanos();
    Ok(metadata.is_file()
        && metadata.len() == expected_bytes
        && modified == u128::from(expected_modified))
}

fn inherited_gate(
    policy: &PromotionPolicy,
    target: QualificationTarget,
) -> Result<PromotionGate, String> {
    if policy.schema != 3 {
        return Err(format!(
            "unsupported promotion policy schema {}; expected 3",
            policy.schema
        ));
    }
    if policy.lifecycle != ["experimental", "candidate", "validated", "production"] {
        return Err(
            "promotion policy schema 3 lifecycle must be experimental, candidate, validated, production"
                .to_owned(),
        );
    }
    validate_policy_graph(policy)?;
    let target = target.as_str();
    if !policy.lifecycle.iter().any(|stage| stage == target) {
        return Err(format!(
            "promotion policy lifecycle does not contain target {target}"
        ));
    }
    let mut chain = Vec::new();
    let mut current = Some(target.to_owned());
    let mut seen = BTreeSet::new();
    while let Some(name) = current {
        if !seen.insert(name.clone()) {
            return Err("promotion policy contains an inheritance cycle".to_owned());
        }
        let gate = policy
            .gates
            .get(&name)
            .ok_or_else(|| format!("promotion policy gate is missing: {name}"))?;
        chain.push(gate.clone());
        current = gate.inherits.clone();
    }
    let mut merged = PromotionGate::default();
    for gate in chain.into_iter().rev() {
        if gate.required_workloads.is_some() {
            merged.required_workloads = gate.required_workloads;
        }
        if gate.minimum_measured_samples_per_workload.is_some() {
            merged.minimum_measured_samples_per_workload =
                gate.minimum_measured_samples_per_workload;
        }
        if gate.require_quality_pass.is_some() {
            merged.require_quality_pass = gate.require_quality_pass;
        }
        if gate.require_deterministic_outputs.is_some() {
            merged.require_deterministic_outputs = gate.require_deterministic_outputs;
        }
        if gate.performance_metric_by_workload.is_some() {
            merged.performance_metric_by_workload = gate.performance_metric_by_workload;
        }
        if gate.maximum_performance_coefficient_of_variation.is_some() {
            merged.maximum_performance_coefficient_of_variation =
                gate.maximum_performance_coefficient_of_variation;
        }
        if gate
            .maximum_median_performance_regression_fraction
            .is_some()
        {
            merged.maximum_median_performance_regression_fraction =
                gate.maximum_median_performance_regression_fraction;
        }
        if gate.minimum_tuning_selection_improvement_fraction.is_some() {
            merged.minimum_tuning_selection_improvement_fraction =
                gate.minimum_tuning_selection_improvement_fraction;
        }
        for evidence in gate.requires_external_evidence {
            if !merged.requires_external_evidence.contains(&evidence) {
                merged.requires_external_evidence.push(evidence);
            }
        }
    }
    let workloads = merged
        .required_workloads
        .as_ref()
        .ok_or_else(|| format!("promotion policy target {target} has no required workloads"))?;
    if workloads.is_empty() || workloads.iter().collect::<BTreeSet<_>>().len() != workloads.len() {
        return Err(format!(
            "promotion policy target {target} requires a non-empty unique workload list"
        ));
    }
    if merged
        .minimum_measured_samples_per_workload
        .is_none_or(|value| value == 0)
    {
        return Err("minimum measured samples must be present and positive".to_owned());
    }
    if merged.require_quality_pass.is_none() || merged.require_deterministic_outputs.is_none() {
        return Err("quality and determinism policy requirements must be explicit".to_owned());
    }
    let metrics = merged
        .performance_metric_by_workload
        .as_ref()
        .ok_or_else(|| "performance metrics must be explicit for every workload".to_owned())?;
    if metrics.keys().collect::<BTreeSet<_>>() != workloads.iter().collect::<BTreeSet<_>>() {
        return Err("performance metric keys must exactly match required workloads".to_owned());
    }
    if merged
        .maximum_performance_coefficient_of_variation
        .is_none_or(|value| !value.is_finite() || value < 0.0)
    {
        return Err("maximum coefficient of variation must be finite and non-negative".to_owned());
    }
    if merged
        .maximum_median_performance_regression_fraction
        .is_none_or(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err("maximum performance regression must be between zero and one".to_owned());
    }
    if merged
        .minimum_tuning_selection_improvement_fraction
        .is_none_or(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err("minimum tuning improvement must be between zero and one".to_owned());
    }
    if merged
        .requires_external_evidence
        .iter()
        .any(|name| name.trim().is_empty())
    {
        return Err("external-evidence names must not be empty".to_owned());
    }
    Ok(merged)
}

fn validate_policy_graph(policy: &PromotionPolicy) -> Result<(), String> {
    let expected = ["candidate", "production", "validated"]
        .into_iter()
        .collect::<BTreeSet<_>>();
    let observed = policy
        .gates
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if observed != expected {
        return Err(
            "promotion policy must define exactly candidate, validated, and production gates"
                .to_owned(),
        );
    }
    for (name, gate) in &policy.gates {
        if gate
            .required_workloads
            .as_ref()
            .is_some_and(|values| values.iter().collect::<BTreeSet<_>>().len() != values.len())
        {
            return Err(format!(
                "promotion policy gate {name} contains duplicate workload names"
            ));
        }
        if gate
            .requires_external_evidence
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != gate.requires_external_evidence.len()
        {
            return Err(format!(
                "promotion policy gate {name} contains duplicate external-evidence names"
            ));
        }
        for evidence in &gate.requires_external_evidence {
            external::parse_kind(evidence)?;
        }
        if let Some(parent) = &gate.inherits {
            if !policy.gates.contains_key(parent) {
                return Err(format!(
                    "promotion policy gate {name} inherits missing gate {parent}"
                ));
            }
        }
    }
    for start in policy.gates.keys() {
        let mut current = Some(start.as_str());
        let mut seen = BTreeSet::new();
        while let Some(name) = current {
            if !seen.insert(name) {
                return Err(format!(
                    "promotion policy inheritance cycle includes gate {name}"
                ));
            }
            current = policy
                .gates
                .get(name)
                .and_then(|gate| gate.inherits.as_deref());
        }
    }
    Ok(())
}

fn group_samples(samples: &[MeasuredSample]) -> BTreeMap<&str, Vec<&MeasuredSample>> {
    let mut groups: BTreeMap<&str, Vec<&MeasuredSample>> = BTreeMap::new();
    for sample in samples {
        groups.entry(&sample.workload).or_default().push(sample);
    }
    groups
}

fn performance_distribution(
    samples: &[&MeasuredSample],
    metric: PerformanceMetric,
) -> Option<(&'static str, Vec<f64>)> {
    let values = samples
        .iter()
        .map(|sample| match metric {
            PerformanceMetric::PrefillTps => sample.prefill_tps,
            PerformanceMetric::DecodeTps => sample.decode_tps,
        })
        .collect::<Option<Vec<_>>>()?;
    (!values.is_empty() && values.iter().all(|value| value.is_finite() && *value > 0.0))
        .then_some((metric.label(), values))
}

fn coefficient_of_variation(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if !mean.is_finite() || mean <= 0.0 {
        return None;
    }
    let variance = if values.len() > 1 {
        values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (values.len() - 1) as f64
    } else {
        0.0
    };
    let cv = variance.sqrt() / mean;
    cv.is_finite().then_some(cv)
}

fn median(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let midpoint = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[midpoint - 1] + values[midpoint]) / 2.0
    } else {
        values[midpoint]
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn add_check(
    checks: &mut Vec<QualificationCheck>,
    name: &str,
    passed: bool,
    observed: Value,
    required: Value,
) {
    checks.push(QualificationCheck {
        name: name.to_owned(),
        passed,
        observed,
        required,
    });
}

pub fn read_request(path: &Path) -> Result<QualificationRequest, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "failed to read Qualification Request {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid Qualification Request {}: {error}", path.display()))
}

fn report(
    decision: Decision,
    reason: &str,
    accepted_final_evidence: Vec<String>,
) -> QualificationReport {
    QualificationReport {
        schema: 1,
        decision,
        reasons: vec![reason.to_owned()],
        accepted_final_evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(value: &str) -> EvidenceIdentity {
        EvidenceIdentity {
            hardware: value.to_owned(),
            software: value.to_owned(),
            model: value.to_owned(),
            runtime: value.to_owned(),
            workload: value.to_owned(),
            configuration: value.to_owned(),
            policy: value.to_owned(),
        }
    }

    fn request() -> QualificationRequest {
        QualificationRequest {
            schema: 1,
            support: Decision::NotProven,
            claim_identity: identity("claim"),
            tuning_evidence_ids: vec!["tune".to_owned()],
            final_evidence_ids: vec!["final".to_owned()],
            evidence: vec![
                EvidenceRecord {
                    id: "tune".to_owned(),
                    phase: EvidencePhase::Tuning,
                    identity: identity("claim"),
                    passed: true,
                },
                EvidenceRecord {
                    id: "final".to_owned(),
                    phase: EvidencePhase::Final,
                    identity: identity("claim"),
                    passed: true,
                },
            ],
            regression_detected: Some(false),
        }
    }

    #[test]
    fn independent_identity_bound_final_evidence_can_qualify() {
        assert_eq!(qualify(&request()).unwrap().decision, Decision::Qualified);
    }

    #[test]
    fn missing_or_reused_final_evidence_is_not_proven() {
        let mut missing = request();
        missing.final_evidence_ids.clear();
        assert_eq!(qualify(&missing).unwrap().decision, Decision::NotProven);

        let mut reused = request();
        reused.tuning_evidence_ids = vec!["final".to_owned()];
        assert_eq!(qualify(&reused).unwrap().decision, Decision::NotProven);
    }

    #[test]
    fn material_identity_change_stales_final_evidence() {
        let mut changed = request();
        changed.claim_identity.runtime = "changed-runtime".to_owned();
        assert_eq!(qualify(&changed).unwrap().decision, Decision::NotProven);
    }

    #[test]
    fn regression_and_uncertainty_are_not_reported_as_qualified() {
        let mut regressed = request();
        regressed.regression_detected = Some(true);
        assert_eq!(qualify(&regressed).unwrap().decision, Decision::Regressed);

        let mut inconclusive = request();
        inconclusive.regression_detected = None;
        assert_eq!(
            qualify(&inconclusive).unwrap().decision,
            Decision::Inconclusive
        );
    }

    #[test]
    fn rejected_support_or_failed_final_evidence_is_unsupported() {
        let mut support = request();
        support.support = Decision::Unsupported;
        assert_eq!(qualify(&support).unwrap().decision, Decision::Unsupported);

        let mut correctness = request();
        correctness.evidence[1].passed = false;
        assert_eq!(
            qualify(&correctness).unwrap().decision,
            Decision::Unsupported
        );
    }

    #[test]
    fn missing_or_stale_tuning_evidence_cannot_support_a_claim() {
        let mut missing = request();
        missing.evidence.remove(0);
        assert_eq!(qualify(&missing).unwrap().decision, Decision::NotProven);

        let mut stale = request();
        stale.evidence[0].identity.policy = "old-policy".to_owned();
        assert_eq!(qualify(&stale).unwrap().decision, Decision::NotProven);
    }

    fn promotion_policy() -> PromotionPolicy {
        serde_json::from_value(json!({
            "schema": 3,
            "lifecycle": ["experimental", "candidate", "validated", "production"],
            "gates": {
                "candidate": {
                    "required_workloads": ["fixture"],
                    "minimum_measured_samples_per_workload": 2,
                    "require_quality_pass": true,
                    "require_deterministic_outputs": true,
                    "performance_metric_by_workload": {"fixture": "decode-tps"},
                    "maximum_performance_coefficient_of_variation": 0.1,
                    "maximum_median_performance_regression_fraction": 0.1,
                    "minimum_tuning_selection_improvement_fraction": 0.03
                },
                "validated": {"inherits": "candidate"},
                "production": {"inherits": "validated"}
            }
        }))
        .unwrap()
    }

    #[test]
    fn policy_requires_versioned_regression_and_rejects_unknown_fields() {
        assert!(inherited_gate(&promotion_policy(), QualificationTarget::Candidate).is_ok());

        let mut missing = promotion_policy();
        missing
            .gates
            .get_mut("candidate")
            .unwrap()
            .maximum_median_performance_regression_fraction = None;
        assert!(inherited_gate(&missing, QualificationTarget::Candidate).is_err());

        let unknown = serde_json::from_value::<PromotionPolicy>(json!({
            "schema": 3,
            "lifecycle": ["experimental", "candidate", "validated", "production"],
            "gates": {
                "candidate": {
                    "required_workloads": ["fixture"],
                    "minimum_measured_samples_per_workload": 2,
                    "require_quality_pass": true,
                    "require_deterministic_outputs": true,
                    "performance_metric_by_workload": {"fixture": "decode-tps"},
                    "maximum_performance_coefficient_of_variation": 0.1,
                    "maximum_median_performance_regression_fraction": 0.1,
                    "minimum_tuning_selection_improvement_fraction": 0.03,
                    "maximum_typo": 0.1
                }
            }
        }));
        assert!(unknown.is_err());
    }

    #[test]
    fn policy_rejects_ambiguous_external_evidence_and_invalid_graphs() {
        let mut duplicate = promotion_policy();
        duplicate
            .gates
            .get_mut("validated")
            .unwrap()
            .requires_external_evidence = vec![
            "near-limit-context-stress".to_owned(),
            "near-limit-context-stress".to_owned(),
        ];
        assert!(inherited_gate(&duplicate, QualificationTarget::Validated).is_err());

        let mut unknown = promotion_policy();
        unknown
            .gates
            .get_mut("validated")
            .unwrap()
            .requires_external_evidence = vec!["unversioned-manual-claim".to_owned()];
        assert!(inherited_gate(&unknown, QualificationTarget::Validated).is_err());

        let mut missing_parent = promotion_policy();
        missing_parent.gates.get_mut("production").unwrap().inherits = Some("missing".to_owned());
        assert!(inherited_gate(&missing_parent, QualificationTarget::Candidate).is_err());

        let mut cycle = promotion_policy();
        cycle.gates.get_mut("candidate").unwrap().inherits = Some("production".to_owned());
        assert!(inherited_gate(&cycle, QualificationTarget::Candidate).is_err());
    }

    #[test]
    fn decode_policy_never_falls_back_to_prefill() {
        let sample = MeasuredSample {
            workload: "fixture".to_owned(),
            iteration: 1,
            prefill_tps: Some(100.0),
            decode_tps: None,
            output_sha256: Some("a".repeat(64)),
            quality_pass: Some(true),
        };
        assert!(performance_distribution(&[&sample], PerformanceMetric::DecodeTps).is_none());
        assert_eq!(
            performance_distribution(&[&sample], PerformanceMetric::PrefillTps)
                .unwrap()
                .1,
            vec![100.0]
        );
        assert_eq!(coefficient_of_variation(&[10.0, 10.0]), Some(0.0));
    }
}
