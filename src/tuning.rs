use crate::evidence::{EvidenceStore, MeasuredSample, StoredIdentity};
use crate::experiment::current_microbenchmark_identity;
use crate::identity::{sha256_bytes, sha256_file};
use crate::qualification::{
    EvidencePhase, PerformanceMetric, current_hardware_identity, material_configuration_sha256,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct TuningOptions {
    pub repository_root: PathBuf,
    pub database: PathBuf,
    pub baseline_run_id: String,
    pub candidate_run_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TuningDisposition {
    SelectedCandidate,
    RetainBaseline,
    NotProven,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TuningCandidate {
    pub run_id: String,
    pub profile: String,
    pub runtime_sha256: Option<String>,
    pub configuration_sha256: Option<String>,
    pub eligible: bool,
    pub general_score: Option<f64>,
    pub repeated_specialization_score: Option<f64>,
    pub general_improvement_fraction: Option<f64>,
    pub repeated_specialization_improvement_fraction: Option<f64>,
    pub workload_medians: BTreeMap<String, f64>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TuningReport {
    pub schema: u32,
    pub disposition: TuningDisposition,
    pub baseline_run_id: String,
    pub selected_run_id: Option<String>,
    pub candidates: Vec<TuningCandidate>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema: u32,
    lifecycle: Vec<String>,
    gates: BTreeMap<String, Gate>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Gate {
    inherits: Option<String>,
    required_workloads: Option<Vec<String>>,
    minimum_measured_samples_per_workload: Option<u32>,
    require_quality_pass: Option<bool>,
    require_deterministic_outputs: Option<bool>,
    performance_metric_by_workload: Option<BTreeMap<String, PerformanceMetric>>,
    general_score_workloads: Option<Vec<String>>,
    repeated_specialization_workloads: Option<Vec<String>>,
    maximum_performance_coefficient_of_variation: Option<f64>,
    maximum_median_performance_regression_fraction: Option<f64>,
    minimum_tuning_selection_improvement_fraction: Option<f64>,
    #[serde(default)]
    requires_external_evidence: Vec<String>,
    golden_evidence: Option<Value>,
}

#[derive(Clone)]
struct SelectionPolicy {
    required_workloads: Vec<String>,
    general_score_workloads: Vec<String>,
    repeated_specialization_workloads: Vec<String>,
    performance_metrics: BTreeMap<String, PerformanceMetric>,
    minimum_samples: usize,
    maximum_cv: f64,
    maximum_regression: f64,
    minimum_improvement: f64,
}

struct EvaluatedRun {
    candidate: TuningCandidate,
    identity: StoredIdentity,
}

#[derive(Debug, Clone, Copy)]
struct SelectionScores {
    general: Option<f64>,
    repeated_specialization: Option<f64>,
}

pub fn tune(options: &TuningOptions) -> Result<TuningReport, String> {
    if options.candidate_run_ids.is_empty() {
        return Err("at least one candidate tuning run is required".to_owned());
    }
    let ids = options.candidate_run_ids.iter().collect::<BTreeSet<_>>();
    if ids.len() != options.candidate_run_ids.len() || ids.contains(&options.baseline_run_id) {
        return Err(
            "candidate tuning ids must be unique and distinct from the baseline id".to_owned(),
        );
    }
    let repository_root = std::fs::canonicalize(&options.repository_root).map_err(|error| {
        format!(
            "failed to resolve repository root {}: {error}",
            options.repository_root.display()
        )
    })?;
    let policy_path = repository_root.join("config/promotion-policy.json");
    let policy_bytes = std::fs::read(&policy_path)
        .map_err(|error| format!("failed to read {}: {error}", policy_path.display()))?;
    let policy: Policy = serde_json::from_slice(&policy_bytes).map_err(|error| {
        format!(
            "invalid promotion policy {}: {error}",
            policy_path.display()
        )
    })?;
    let selection = selection_policy(&policy)?;
    let policy_sha256 = sha256_bytes(&policy_bytes);
    let binary_sha256 = sha256_file(
        &std::env::current_exe()
            .map_err(|error| format!("failed to locate Alpine executable: {error}"))?,
    )?;
    let current_workload_sha256 = current_microbenchmark_identity(&repository_root, &[])?;
    let store = EvidenceStore::open_read_only(&options.database)?;
    let baseline = evaluate_run(
        &store,
        &options.baseline_run_id,
        &selection,
        &policy_sha256,
        &binary_sha256,
        &current_workload_sha256,
        &repository_root,
    )?;
    let mut candidates = Vec::with_capacity(options.candidate_run_ids.len());
    for id in &options.candidate_run_ids {
        candidates.push(evaluate_run(
            &store,
            id,
            &selection,
            &policy_sha256,
            &binary_sha256,
            &current_workload_sha256,
            &repository_root,
        )?);
    }

    let baseline_score = baseline.candidate.general_score;
    let baseline_repeated_score = baseline.candidate.repeated_specialization_score;
    let baseline_medians = baseline.candidate.workload_medians.clone();
    let baseline_comparable = baseline.candidate.eligible && baseline_score.is_some();
    for candidate in &mut candidates {
        if !comparable_identity(&baseline.identity, &candidate.identity) {
            candidate.candidate.eligible = false;
            candidate.candidate.reasons.push(
                "identity differs outside the runtime/configuration search dimensions".to_owned(),
            );
        }
        if let (Some(score), Some(reference)) = (candidate.candidate.general_score, baseline_score)
        {
            candidate.candidate.general_improvement_fraction = Some(score / reference - 1.0);
        }
        if let (Some(score), Some(reference)) = (
            candidate.candidate.repeated_specialization_score,
            baseline_repeated_score,
        ) {
            candidate
                .candidate
                .repeated_specialization_improvement_fraction = Some(score / reference - 1.0);
        }
        for workload in &selection.required_workloads {
            let Some(reference) = baseline_medians.get(workload) else {
                candidate.candidate.eligible = false;
                continue;
            };
            let Some(observed) = candidate.candidate.workload_medians.get(workload) else {
                candidate.candidate.eligible = false;
                continue;
            };
            let floor = reference * (1.0 - selection.maximum_regression);
            if observed < &floor {
                candidate.candidate.eligible = false;
                candidate.candidate.reasons.push(format!(
                    "{workload} median {observed:.4} is below the {floor:.4} regression floor"
                ));
            }
        }
    }

    let mut rendered = std::iter::once(baseline.candidate.clone())
        .chain(candidates.iter().map(|value| value.candidate.clone()))
        .collect::<Vec<_>>();
    rendered.sort_by(|left, right| {
        right
            .general_score
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&left.general_score.unwrap_or(f64::NEG_INFINITY))
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    if !baseline_comparable {
        return Ok(TuningReport {
            schema: 2,
            disposition: TuningDisposition::NotProven,
            baseline_run_id: options.baseline_run_id.clone(),
            selected_run_id: None,
            candidates: rendered,
            reasons: vec![
                "the tuning baseline is not eligible under the current policy".to_owned(),
            ],
        });
    }
    let candidate_rows = candidates
        .iter()
        .map(|candidate| candidate.candidate.clone())
        .collect::<Vec<_>>();
    let (disposition, winner) = tuning_disposition(&candidate_rows, selection.minimum_improvement);
    if disposition == TuningDisposition::SelectedCandidate {
        let winner = winner.expect("selected disposition has a winner");
        Ok(TuningReport {
            schema: 2,
            disposition,
            baseline_run_id: options.baseline_run_id.clone(),
            selected_run_id: Some(winner.run_id.clone()),
            candidates: rendered,
            reasons: vec![format!(
                "selected candidate general score exceeds the baseline by at least {:.2}% without a per-workload regression; repeated specialization is reported separately",
                selection.minimum_improvement * 100.0
            )],
        })
    } else {
        Ok(TuningReport {
            schema: 2,
            disposition,
            baseline_run_id: options.baseline_run_id.clone(),
            selected_run_id: Some(options.baseline_run_id.clone()),
            candidates: rendered,
            reasons: vec![format!(
                "no eligible candidate improved the general score by the required {:.2}%; repeated specialization cannot select the default",
                selection.minimum_improvement * 100.0
            )],
        })
    }
}

fn tuning_disposition(
    candidates: &[TuningCandidate],
    minimum_improvement: f64,
) -> (TuningDisposition, Option<&TuningCandidate>) {
    let winner = candidates
        .iter()
        .filter(|candidate| candidate.eligible)
        .filter(|candidate| {
            candidate
                .general_improvement_fraction
                .is_some_and(|value| value >= minimum_improvement)
        })
        .max_by(|left, right| {
            left.general_score
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&right.general_score.unwrap_or(f64::NEG_INFINITY))
                .then_with(|| right.run_id.cmp(&left.run_id))
        });
    if winner.is_some() {
        (TuningDisposition::SelectedCandidate, winner)
    } else {
        (TuningDisposition::RetainBaseline, None)
    }
}

fn evaluate_run(
    store: &EvidenceStore,
    id: &str,
    policy: &SelectionPolicy,
    policy_sha256: &str,
    binary_sha256: &str,
    workload_sha256: &str,
    repository_root: &std::path::Path,
) -> Result<EvaluatedRun, String> {
    let evidence = store
        .run(id)?
        .ok_or_else(|| format!("evidence run not found: {id}"))?;
    let samples = store.measured_samples(id)?;
    let mut reasons = Vec::new();
    if evidence.summary.kind != "micro"
        || evidence.summary.status != "passed"
        || evidence.summary.finished_at.is_none()
    {
        reasons.push("run is not a finished, passed microbenchmark".to_owned());
    }
    let phase = evidence
        .config
        .get("evidence_phase")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    if phase != Some(EvidencePhase::Tuning) {
        reasons.push("run is not explicitly tuning evidence".to_owned());
    }
    if !valid_identity(&evidence.identity) {
        reasons.push("run identity is incomplete or malformed".to_owned());
    }
    if evidence.identity.policy.as_deref() != Some(policy_sha256) {
        reasons.push("run uses a stale promotion policy".to_owned());
    }
    if evidence.identity.software.as_deref() != Some(binary_sha256) {
        reasons.push("run was produced by a different Alpine binary".to_owned());
    }
    if evidence.identity.workload.as_deref() != Some(workload_sha256)
        || evidence
            .config
            .pointer("/benchmark/name")
            .and_then(Value::as_str)
            != Some("micro")
        || evidence
            .config
            .pointer("/benchmark/schema")
            .and_then(Value::as_u64)
            != Some(3)
    {
        reasons.push("run uses a stale or non-micro workload contract".to_owned());
    }
    if material_configuration_sha256(&evidence).ok().as_deref()
        != evidence.identity.configuration.as_deref()
    {
        reasons.push("run material configuration identity does not recompute".to_owned());
    }
    if current_hardware_identity(repository_root, &evidence)?.as_deref()
        != evidence.identity.hardware.as_deref()
    {
        reasons.push("run hardware manifest is stale or internally inconsistent".to_owned());
    }
    if evidence
        .config
        .pointer("/model_verification/sha256")
        .and_then(Value::as_str)
        != evidence.identity.model.as_deref()
    {
        reasons.push("run model verification does not match model identity".to_owned());
    }
    let runtime_current = evidence
        .config
        .pointer("/launch/server")
        .and_then(Value::as_str)
        .and_then(|path| sha256_file(PathBuf::from(path).as_path()).ok())
        .as_deref()
        == evidence
            .config
            .pointer("/launch/server_sha256")
            .and_then(Value::as_str);
    if !runtime_current {
        reasons.push("run inference runtime artifact is stale".to_owned());
    }

    let groups = group_samples(&samples);
    let mut medians = BTreeMap::new();
    for workload in &policy.required_workloads {
        let metric = policy
            .performance_metrics
            .get(workload)
            .copied()
            .ok_or_else(|| format!("performance metric is missing for {workload}"))?;
        let rows = groups.get(workload.as_str()).cloned().unwrap_or_default();
        if rows.len() < policy.minimum_samples {
            reasons.push(format!(
                "{workload} has {} samples; {} required",
                rows.len(),
                policy.minimum_samples
            ));
            continue;
        }
        if rows.iter().any(|row| row.quality_pass != Some(true)) {
            reasons.push(format!("{workload} has a failed or missing quality result"));
        }
        let hashes = rows
            .iter()
            .filter_map(|row| row.output_sha256.as_deref())
            .collect::<BTreeSet<_>>();
        if hashes.len() != 1
            || rows.iter().any(|row| {
                row.output_sha256
                    .as_deref()
                    .is_none_or(|value| !is_sha256(value))
            })
        {
            reasons.push(format!("{workload} output is not deterministic"));
        }
        let Some(values) = performance_distribution(&rows, metric) else {
            reasons.push(format!(
                "{workload} {} measurements are incomplete or invalid",
                metric.label()
            ));
            continue;
        };
        let cv = coefficient_of_variation(&values);
        if cv.is_none_or(|value| value > policy.maximum_cv) {
            reasons.push(format!(
                "{workload} {} variability exceeds policy",
                metric.label()
            ));
        }
        medians.insert(workload.clone(), median(&values));
    }
    let scores = selection_scores(&medians, policy);
    Ok(EvaluatedRun {
        candidate: TuningCandidate {
            run_id: evidence.summary.id,
            profile: evidence.summary.profile,
            runtime_sha256: evidence.identity.runtime.clone(),
            configuration_sha256: evidence.identity.configuration.clone(),
            eligible: reasons.is_empty() && scores.general.is_some(),
            general_score: scores.general,
            repeated_specialization_score: scores.repeated_specialization,
            general_improvement_fraction: None,
            repeated_specialization_improvement_fraction: None,
            workload_medians: medians,
            reasons,
        },
        identity: evidence.identity,
    })
}

fn selection_policy(policy: &Policy) -> Result<SelectionPolicy, String> {
    if policy.schema != 4
        || policy.lifecycle != ["experimental", "candidate", "validated", "production"]
    {
        return Err("unsupported promotion policy lifecycle or schema".to_owned());
    }
    let gate = policy
        .gates
        .get("candidate")
        .ok_or_else(|| "candidate promotion gate is missing".to_owned())?;
    if gate.inherits.is_some()
        || !gate.requires_external_evidence.is_empty()
        || gate.golden_evidence.is_some()
    {
        return Err("candidate gate must be the root automated gate".to_owned());
    }
    let workloads = gate
        .required_workloads
        .clone()
        .filter(|values| {
            !values.is_empty() && values.iter().collect::<BTreeSet<_>>().len() == values.len()
        })
        .ok_or_else(|| "candidate required workloads are missing or duplicated".to_owned())?;
    let minimum_samples = gate
        .minimum_measured_samples_per_workload
        .filter(|value| *value > 0)
        .ok_or_else(|| "candidate minimum sample count must be positive".to_owned())?
        as usize;
    if gate.require_quality_pass != Some(true) || gate.require_deterministic_outputs != Some(true) {
        return Err("tuning requires explicit quality and determinism gates".to_owned());
    }
    let performance_metrics = gate
        .performance_metric_by_workload
        .clone()
        .ok_or_else(|| "candidate performance metric map is missing".to_owned())?;
    if performance_metrics.keys().collect::<BTreeSet<_>>()
        != workloads.iter().collect::<BTreeSet<_>>()
    {
        return Err("candidate performance metric keys must match required workloads".to_owned());
    }
    let general_score_workloads = unique_workloads(
        gate.general_score_workloads.clone(),
        "candidate general score workloads",
        false,
    )?;
    let repeated_specialization_workloads = unique_workloads(
        gate.repeated_specialization_workloads.clone(),
        "candidate repeated specialization workloads",
        true,
    )?;
    let general = general_score_workloads.iter().collect::<BTreeSet<_>>();
    let repeated = repeated_specialization_workloads
        .iter()
        .collect::<BTreeSet<_>>();
    let required = workloads.iter().collect::<BTreeSet<_>>();
    if !general.is_disjoint(&repeated)
        || general.union(&repeated).copied().collect::<BTreeSet<_>>() != required
    {
        return Err(
            "general and repeated specialization workloads must be disjoint and cover required workloads"
                .to_owned(),
        );
    }
    let maximum_cv = finite_fraction(
        gate.maximum_performance_coefficient_of_variation,
        "maximum performance coefficient of variation",
    )?;
    let maximum_regression = finite_fraction(
        gate.maximum_median_performance_regression_fraction,
        "maximum median regression fraction",
    )?;
    let minimum_improvement = finite_fraction(
        gate.minimum_tuning_selection_improvement_fraction,
        "minimum tuning improvement fraction",
    )?;
    Ok(SelectionPolicy {
        required_workloads: workloads,
        general_score_workloads,
        repeated_specialization_workloads,
        performance_metrics,
        minimum_samples,
        maximum_cv,
        maximum_regression,
        minimum_improvement,
    })
}

fn unique_workloads(
    values: Option<Vec<String>>,
    name: &str,
    allow_empty: bool,
) -> Result<Vec<String>, String> {
    values
        .filter(|values| {
            (allow_empty || !values.is_empty())
                && values.iter().all(|value| !value.trim().is_empty())
                && values.iter().collect::<BTreeSet<_>>().len() == values.len()
        })
        .ok_or_else(|| format!("{name} are missing, invalid, or duplicated"))
}

fn comparable_identity(left: &StoredIdentity, right: &StoredIdentity) -> bool {
    left.hardware == right.hardware
        && left.software == right.software
        && left.model == right.model
        && left.workload == right.workload
        && left.policy == right.policy
}

fn valid_identity(identity: &StoredIdentity) -> bool {
    [
        identity.hardware.as_deref(),
        identity.software.as_deref(),
        identity.model.as_deref(),
        identity.runtime.as_deref(),
        identity.workload.as_deref(),
        identity.configuration.as_deref(),
        identity.policy.as_deref(),
    ]
    .into_iter()
    .all(|value| value.is_some_and(is_sha256))
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
) -> Option<Vec<f64>> {
    let values = samples
        .iter()
        .map(|sample| match metric {
            PerformanceMetric::PrefillTps => sample.prefill_tps,
            PerformanceMetric::DecodeTps => sample.decode_tps,
        })
        .collect::<Option<Vec<_>>>()?;
    (!values.is_empty() && values.iter().all(|value| value.is_finite() && *value > 0.0))
        .then_some(values)
}

fn coefficient_of_variation(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    if mean <= 0.0 || !mean.is_finite() {
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
    let result = variance.sqrt() / mean;
    result.is_finite().then_some(result)
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

fn geometric_mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values = values.collect::<Vec<_>>();
    if values.is_empty()
        || values
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return None;
    }
    Some((values.iter().map(|value| value.ln()).sum::<f64>() / values.len() as f64).exp())
}

fn selection_scores(medians: &BTreeMap<String, f64>, policy: &SelectionPolicy) -> SelectionScores {
    let score = |workloads: &[String]| {
        if workloads.is_empty() {
            return None;
        }
        workloads
            .iter()
            .map(|workload| medians.get(workload).copied())
            .collect::<Option<Vec<_>>>()
            .and_then(|values| geometric_mean(values.into_iter()))
    };
    SelectionScores {
        general: score(&policy.general_score_workloads),
        repeated_specialization: score(&policy.repeated_specialization_workloads),
    }
}

fn finite_fraction(value: Option<f64>, name: &str) -> Result<f64, String> {
    value
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or_else(|| format!("{name} must be an explicit finite value between zero and one"))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_visible_output_is_reported_but_cannot_drive_default_selection() {
        let policy = SelectionPolicy {
            required_workloads: vec!["novel".to_owned(), "repeat".to_owned()],
            general_score_workloads: vec!["novel".to_owned()],
            repeated_specialization_workloads: vec!["repeat".to_owned()],
            performance_metrics: BTreeMap::from([
                ("novel".to_owned(), PerformanceMetric::DecodeTps),
                ("repeat".to_owned(), PerformanceMetric::DecodeTps),
            ]),
            minimum_samples: 1,
            maximum_cv: 0.1,
            maximum_regression: 0.1,
            minimum_improvement: 0.03,
        };
        let baseline = BTreeMap::from([("novel".to_owned(), 10.0), ("repeat".to_owned(), 10.0)]);
        let repeated_only_gain =
            BTreeMap::from([("novel".to_owned(), 10.0), ("repeat".to_owned(), 100.0)]);

        let baseline_scores = selection_scores(&baseline, &policy);
        let candidate_scores = selection_scores(&repeated_only_gain, &policy);
        assert_eq!(candidate_scores.general, baseline_scores.general);
        assert!(
            candidate_scores.repeated_specialization.unwrap()
                > baseline_scores.repeated_specialization.unwrap()
        );
        let candidate = TuningCandidate {
            run_id: "repeat-only".to_owned(),
            profile: "turbo-16k".to_owned(),
            runtime_sha256: Some("a".repeat(64)),
            configuration_sha256: Some("b".repeat(64)),
            eligible: true,
            general_score: candidate_scores.general,
            repeated_specialization_score: candidate_scores.repeated_specialization,
            general_improvement_fraction: Some(0.0),
            repeated_specialization_improvement_fraction: Some(9.0),
            workload_medians: repeated_only_gain,
            reasons: Vec::new(),
        };
        assert_eq!(
            tuning_disposition(std::slice::from_ref(&candidate), policy.minimum_improvement).0,
            TuningDisposition::RetainBaseline
        );
    }

    #[test]
    fn decode_metrics_are_fail_closed() {
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
            performance_distribution(&[&sample], PerformanceMetric::PrefillTps),
            Some(vec![100.0])
        );
    }
}
