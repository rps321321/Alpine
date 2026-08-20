use crate::clock::UtcTimestamp;
use crate::context::{self, NearLimitContextOptions, NearLimitContextReport};
use crate::decision::Decision;
use crate::experiment::{self, ExperimentReport, MicrobenchmarkOptions};
use crate::golden::{self, GoldenAgentOptions, GoldenAgentReport};
use crate::identity::sha256_bytes;
use crate::qualification::{
    EvidencePhase, QualificationTarget, RunQualificationOptions, RunQualificationReport,
};
use crate::rollback::{self, RollbackProofOptions, RollbackProofReport};
use crate::session::{self, AcquireSessionOptions, ReleaseSessionOptions};
use crate::stability::{
    self, CleanRestartStabilityOptions, CleanRestartStabilityReport, SameProcessStabilityOptions,
    SameProcessStabilityReport,
};
use crate::support::{self, SupportReport};
use crate::tuning::{self, TuningDisposition, TuningOptions, TuningReport};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

const MAX_PLAN_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct EvaluationOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub result_root: PathBuf,
    pub plan: PathBuf,
    pub allow_legacy_identity: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationPlan {
    pub schema: u32,
    pub id: String,
    pub baseline_profile: String,
    pub candidate_profiles: Vec<String>,
    pub workloads: Vec<String>,
    pub microbenchmark: MicrobenchmarkBudget,
    pub target: QualificationTarget,
    pub qualification: QualificationPlan,
    pub limits: EvaluationLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MicrobenchmarkBudget {
    pub warmups: u32,
    pub runs: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationPlan {
    pub near_limit_context_ratio: f64,
    pub golden_task: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationLimits {
    pub maximum_profiles: u32,
    pub maximum_microbenchmark_requests: u32,
    pub support_timeout_seconds: u64,
    pub lease_timeout_seconds: u64,
    pub startup_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileMeasurement {
    pub profile: String,
    pub run_id: String,
}

#[derive(Debug, Serialize)]
pub struct EvaluationReport {
    pub schema: u32,
    pub evaluation_id: String,
    pub plan_id: String,
    pub plan_sha256: String,
    pub started_at: String,
    pub finished_at: String,
    pub artifact_path: PathBuf,
    pub decision: Decision,
    pub production_decision: Option<Decision>,
    pub support: SupportReport,
    pub tuning_measurements: Vec<ProfileMeasurement>,
    pub tuning: Option<TuningReport>,
    pub selected_profile: Option<String>,
    pub final_run_id: Option<String>,
    pub candidate_qualification: Option<RunQualificationReport>,
    pub same_process_stability: Option<SameProcessStabilityReport>,
    pub clean_restart_stability: Option<CleanRestartStabilityReport>,
    pub near_limit_context: Option<NearLimitContextReport>,
    pub golden_agent: Option<GoldenAgentReport>,
    pub validated_qualification: Option<RunQualificationReport>,
    pub rollback_proof: Option<RollbackProofReport>,
    pub production_qualification: Option<RunQualificationReport>,
}

pub fn run(options: &EvaluationOptions) -> Result<EvaluationReport, String> {
    let repository_root = std::fs::canonicalize(&options.repository_root).map_err(|error| {
        format!(
            "failed to resolve repository root {}: {error}",
            options.repository_root.display()
        )
    })?;
    let result_root = absolute_or_join(&repository_root, &options.result_root);
    std::fs::create_dir_all(&result_root).map_err(|error| {
        format!(
            "failed to create evaluation result root {}: {error}",
            result_root.display()
        )
    })?;
    let database = result_root.join("results.sqlite3");
    let plan_path = absolute_or_join(&repository_root, &options.plan);
    let (plan, plan_sha256) = read_plan(&plan_path)?;
    validate_plan(&plan)?;
    validate_profiles(&options.install_root, &plan)?;

    let now = UtcTimestamp::now()?;
    let evaluation_id = format!(
        "{}-{}",
        now.compact(),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let artifact_path = result_root
        .join("evaluations")
        .join(format!("{evaluation_id}.json"));
    let support_path = repository_root.join("config/support-envelope.json");
    let (envelope, envelope_bytes) = support::read_envelope(&support_path)?;
    let support = support::inspect(
        &envelope,
        &envelope_bytes,
        Duration::from_secs(plan.limits.support_timeout_seconds),
    )?;
    let mut report = EvaluationReport {
        schema: 1,
        evaluation_id,
        plan_id: plan.id.clone(),
        plan_sha256,
        started_at: now.rfc3339(),
        finished_at: String::new(),
        artifact_path,
        decision: support.decision,
        production_decision: None,
        support,
        tuning_measurements: Vec::new(),
        tuning: None,
        selected_profile: None,
        final_run_id: None,
        candidate_qualification: None,
        same_process_stability: None,
        clean_restart_stability: None,
        near_limit_context: None,
        golden_agent: None,
        validated_qualification: None,
        rollback_proof: None,
        production_qualification: None,
    };
    if report.support.decision == Decision::Unsupported {
        return finish(report);
    }

    let profiles = std::iter::once(&plan.baseline_profile)
        .chain(plan.candidate_profiles.iter())
        .cloned()
        .collect::<Vec<_>>();
    for profile in &profiles {
        let measurement = measure_profile(
            options,
            &repository_root,
            &result_root,
            &plan,
            profile,
            EvidencePhase::Tuning,
            false,
        )?;
        report.tuning_measurements.push(ProfileMeasurement {
            profile: profile.clone(),
            run_id: measurement.run_id,
        });
    }
    let baseline_run_id = report.tuning_measurements[0].run_id.clone();
    let candidate_run_ids = report
        .tuning_measurements
        .iter()
        .skip(1)
        .map(|measurement| measurement.run_id.clone())
        .collect::<Vec<_>>();
    let tuning = tuning::tune(&TuningOptions {
        repository_root: repository_root.clone(),
        database: database.clone(),
        baseline_run_id,
        candidate_run_ids,
    })?;
    let selected_run_id = tuning.selected_run_id.clone();
    report.decision = if tuning.disposition == TuningDisposition::NotProven {
        Decision::NotProven
    } else {
        report.decision
    };
    report.tuning = Some(tuning);
    let Some(selected_run_id) = selected_run_id else {
        return finish(report);
    };
    let selected_profile = report
        .tuning_measurements
        .iter()
        .find(|measurement| measurement.run_id == selected_run_id)
        .map(|measurement| measurement.profile.clone())
        .ok_or_else(|| "tuner selected a run outside the declared search space".to_owned())?;
    report.selected_profile = Some(selected_profile.clone());

    let final_measurement = measure_profile(
        options,
        &repository_root,
        &result_root,
        &plan,
        &selected_profile,
        EvidencePhase::Final,
        true,
    )?;
    let final_run_id = final_measurement.run_id;
    report.final_run_id = Some(final_run_id.clone());
    let qualification_options = |target| RunQualificationOptions {
        repository_root: repository_root.clone(),
        install_root: options.install_root.clone(),
        database: database.clone(),
        final_run_id: final_run_id.clone(),
        tuning_run_ids: vec![selected_run_id.clone()],
        target,
        support_timeout: Duration::from_secs(plan.limits.support_timeout_seconds),
    };
    let candidate =
        crate::qualification::qualify_run(&qualification_options(QualificationTarget::Candidate))?;
    report.decision = candidate.decision;
    let candidate_passed = candidate.decision == Decision::Qualified;
    report.candidate_qualification = Some(candidate);
    if !candidate_passed {
        return finish(report);
    }
    if plan.target == QualificationTarget::Candidate {
        return finish(report);
    }

    let lease_timeout = Duration::from_secs(plan.limits.lease_timeout_seconds);
    let startup_timeout = Duration::from_secs(plan.limits.startup_timeout_seconds);
    let request_timeout = Duration::from_secs(plan.limits.request_timeout_seconds);
    report.same_process_stability =
        Some(stability::run_same_process(&SameProcessStabilityOptions {
            repository_root: repository_root.clone(),
            install_root: options.install_root.clone(),
            database: database.clone(),
            result_root: result_root.clone(),
            anchor_run_id: final_run_id.clone(),
            allow_legacy_identity: options.allow_legacy_identity,
            lease_timeout,
            startup_timeout,
            request_timeout,
        })?);
    report.clean_restart_stability = Some(stability::run_clean_restarts(
        &CleanRestartStabilityOptions {
            repository_root: repository_root.clone(),
            install_root: options.install_root.clone(),
            database: database.clone(),
            result_root: result_root.clone(),
            anchor_run_id: final_run_id.clone(),
            allow_legacy_identity: options.allow_legacy_identity,
            lease_timeout,
            startup_timeout,
            request_timeout,
        },
    )?);
    report.near_limit_context = Some(context::run(&NearLimitContextOptions {
        repository_root: repository_root.clone(),
        install_root: options.install_root.clone(),
        database: database.clone(),
        result_root: result_root.clone(),
        anchor_run_id: final_run_id.clone(),
        ratio: plan.qualification.near_limit_context_ratio,
        allow_legacy_identity: options.allow_legacy_identity,
        lease_timeout,
        startup_timeout,
        request_timeout,
    })?);
    report.golden_agent = Some(golden::run(&GoldenAgentOptions {
        repository_root: repository_root.clone(),
        install_root: options.install_root.clone(),
        database: database.clone(),
        result_root: result_root.clone(),
        anchor_run_id: final_run_id.clone(),
        task_id: plan.qualification.golden_task.clone(),
        allow_legacy_identity: options.allow_legacy_identity,
        lease_timeout,
        startup_timeout,
    })?);

    let validated =
        crate::qualification::qualify_run(&qualification_options(QualificationTarget::Validated))?;
    report.decision = validated.decision;
    let validated_passed = validated.decision == Decision::Qualified;
    report.validated_qualification = Some(validated);
    if !validated_passed {
        return finish(report);
    }
    if plan.target == QualificationTarget::Validated {
        return finish(report);
    }

    report.rollback_proof = Some(rollback::run(&RollbackProofOptions {
        repository_root: repository_root.clone(),
        install_root: options.install_root.clone(),
        database: database.clone(),
        result_root,
        anchor_run_id: final_run_id.clone(),
        allow_legacy_identity: options.allow_legacy_identity,
        lease_timeout,
        startup_timeout,
        request_timeout,
    })?);
    let production =
        crate::qualification::qualify_run(&qualification_options(QualificationTarget::Production))?;
    report.decision = production.decision;
    report.production_decision = Some(production.decision);
    report.production_qualification = Some(production);
    finish(report)
}

fn measure_profile(
    options: &EvaluationOptions,
    repository_root: &Path,
    result_root: &Path,
    plan: &EvaluationPlan,
    profile: &str,
    phase: EvidencePhase,
    deep_verify_artifacts: bool,
) -> Result<ExperimentReport, String> {
    let lease_timeout = Duration::from_secs(plan.limits.lease_timeout_seconds);
    let startup_timeout = Duration::from_secs(plan.limits.startup_timeout_seconds);
    let acquisition = session::acquire(&AcquireSessionOptions {
        install_root: options.install_root.clone(),
        profile: Some(profile.to_owned()),
        vision: false,
        force_fallback: false,
        allow_legacy_identity: options.allow_legacy_identity,
        lock_timeout: lease_timeout,
        startup_timeout,
    })?;
    let attempt = experiment::run_microbenchmark(&MicrobenchmarkOptions {
        repository_root: repository_root.to_path_buf(),
        install_root: options.install_root.clone(),
        result_root: result_root.to_path_buf(),
        profile: profile.to_owned(),
        runs: plan.microbenchmark.runs,
        warmups: plan.microbenchmark.warmups,
        workloads: plan.workloads.clone(),
        notes: Some(format!("automated evaluation {}", plan.id)),
        phase,
        deep_verify_artifacts,
        lease_timeout,
    });
    let release = session::release(&ReleaseSessionOptions {
        install_root: options.install_root.clone(),
        acquisition,
        keep_server: false,
        lock_timeout: lease_timeout,
        startup_timeout,
    });
    match (attempt, release) {
        (Ok(report), Ok(_)) => Ok(report),
        (Err(error), Ok(_)) => Err(error),
        (Ok(_), Err(error)) => Err(format!(
            "{profile} measurement passed but prior Session restoration failed: {error}"
        )),
        (Err(attempt_error), Err(release_error)) => Err(format!(
            "{profile} measurement failed: {attempt_error}; prior Session restoration also failed: {release_error}"
        )),
    }
}

fn read_plan(path: &Path) -> Result<(EvaluationPlan, String), String> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        format!(
            "failed to inspect evaluation plan {}: {error}",
            path.display()
        )
    })?;
    if metadata.len() > MAX_PLAN_BYTES {
        return Err(format!(
            "evaluation plan exceeds the 1 MiB input limit: {}",
            path.display()
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read evaluation plan {}: {error}", path.display()))?;
    let plan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid evaluation plan {}: {error}", path.display()))?;
    Ok((plan, sha256_bytes(&bytes)))
}

fn validate_plan(plan: &EvaluationPlan) -> Result<(), String> {
    if plan.schema != 1 {
        return Err(format!(
            "unsupported evaluation plan schema {}; expected 1",
            plan.schema
        ));
    }
    if plan.id.trim().is_empty()
        || plan.baseline_profile.trim().is_empty()
        || plan.qualification.golden_task.trim().is_empty()
    {
        return Err("evaluation plan ids and profile names must not be empty".to_owned());
    }
    if plan.candidate_profiles.is_empty() || plan.workloads.is_empty() {
        return Err(
            "evaluation plan requires at least one candidate and one explicit workload".to_owned(),
        );
    }
    let profiles = std::iter::once(&plan.baseline_profile)
        .chain(plan.candidate_profiles.iter())
        .collect::<BTreeSet<_>>();
    if profiles.len() != plan.candidate_profiles.len() + 1 {
        return Err("evaluation profiles must be unique and baseline-distinct".to_owned());
    }
    if profiles.iter().any(|profile| profile.trim().is_empty()) {
        return Err("evaluation profile names must not be empty".to_owned());
    }
    let workloads = plan.workloads.iter().collect::<BTreeSet<_>>();
    if workloads.len() != plan.workloads.len()
        || workloads.iter().any(|workload| workload.trim().is_empty())
    {
        return Err("evaluation workload ids must be nonempty and unique".to_owned());
    }
    if plan.microbenchmark.runs == 0 {
        return Err("evaluation measured run count must be positive".to_owned());
    }
    if !plan.qualification.near_limit_context_ratio.is_finite()
        || !(0.85..=0.95).contains(&plan.qualification.near_limit_context_ratio)
    {
        return Err("evaluation near-limit ratio must be between 0.85 and 0.95".to_owned());
    }
    let profile_count = u32::try_from(profiles.len())
        .map_err(|_| "evaluation profile count is too large".to_owned())?;
    if plan.limits.maximum_profiles == 0 || profile_count > plan.limits.maximum_profiles {
        return Err(format!(
            "evaluation search has {profile_count} profiles but the budget permits {}",
            plan.limits.maximum_profiles
        ));
    }
    let workload_count = u32::try_from(workloads.len())
        .map_err(|_| "evaluation workload count is too large".to_owned())?;
    let iterations = plan
        .microbenchmark
        .runs
        .checked_add(plan.microbenchmark.warmups)
        .ok_or_else(|| "evaluation iteration count overflows".to_owned())?;
    let tuning_and_final_profiles = profile_count
        .checked_add(1)
        .ok_or_else(|| "evaluation profile count overflows".to_owned())?;
    let requests = tuning_and_final_profiles
        .checked_mul(workload_count)
        .and_then(|value| value.checked_mul(iterations))
        .ok_or_else(|| "evaluation request budget overflows".to_owned())?;
    if plan.limits.maximum_microbenchmark_requests == 0
        || requests > plan.limits.maximum_microbenchmark_requests
    {
        return Err(format!(
            "evaluation requires {requests} microbenchmark requests but the budget permits {}",
            plan.limits.maximum_microbenchmark_requests
        ));
    }
    if [
        plan.limits.support_timeout_seconds,
        plan.limits.lease_timeout_seconds,
        plan.limits.startup_timeout_seconds,
        plan.limits.request_timeout_seconds,
    ]
    .contains(&0)
    {
        return Err("evaluation timeouts must be positive".to_owned());
    }
    Ok(())
}

fn validate_profiles(install_root: &Path, plan: &EvaluationPlan) -> Result<(), String> {
    for profile in std::iter::once(&plan.baseline_profile).chain(plan.candidate_profiles.iter()) {
        crate::config::resolve(install_root, Some(profile), true)?;
    }
    Ok(())
}

fn finish(mut report: EvaluationReport) -> Result<EvaluationReport, String> {
    report.finished_at = UtcTimestamp::now()?.rfc3339();
    let parent = report
        .artifact_path
        .parent()
        .ok_or_else(|| "evaluation artifact has no parent directory".to_owned())?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed to create evaluation artifact directory {}: {error}",
            parent.display()
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("failed to create evaluation artifact: {error}"))?;
    serde_json::to_writer_pretty(&mut temporary, &report)
        .map_err(|error| format!("failed to encode evaluation artifact: {error}"))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("failed to write evaluation artifact: {error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("failed to sync evaluation artifact: {error}"))?;
    temporary.persist(&report.artifact_path).map_err(|error| {
        format!(
            "failed to publish evaluation artifact {}: {error}",
            report.artifact_path.display()
        )
    })?;
    Ok(report)
}

fn absolute_or_join(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> EvaluationPlan {
        EvaluationPlan {
            schema: 1,
            id: "fixture".to_owned(),
            baseline_profile: "stable".to_owned(),
            candidate_profiles: vec!["candidate".to_owned()],
            workloads: vec!["one".to_owned(), "two".to_owned()],
            microbenchmark: MicrobenchmarkBudget {
                warmups: 1,
                runs: 5,
            },
            target: QualificationTarget::Production,
            qualification: QualificationPlan {
                near_limit_context_ratio: 0.85,
                golden_task: "fixture".to_owned(),
            },
            limits: EvaluationLimits {
                maximum_profiles: 2,
                maximum_microbenchmark_requests: 36,
                support_timeout_seconds: 10,
                lease_timeout_seconds: 15,
                startup_timeout_seconds: 600,
                request_timeout_seconds: 600,
            },
        }
    }

    #[test]
    fn plan_enforces_the_declared_search_request_budget() {
        assert!(validate_plan(&plan()).is_ok());
        let mut insufficient = plan();
        insufficient.limits.maximum_microbenchmark_requests = 35;
        assert!(validate_plan(&insufficient).is_err());
    }

    #[test]
    fn plan_rejects_duplicate_profiles_and_unknown_fields() {
        let mut duplicate = plan();
        duplicate.candidate_profiles = vec!["stable".to_owned()];
        assert!(validate_plan(&duplicate).is_err());
        let bytes = serde_json::to_vec(&plan()).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["limits"]["unbounded"] = serde_json::json!(true);
        assert!(serde_json::from_value::<EvaluationPlan>(value).is_err());
    }

    #[test]
    fn checked_in_production_plan_is_strict_and_budgeted() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("config")
            .join("evaluation-plan.json");
        let (plan, digest) = read_plan(&path).unwrap();
        validate_plan(&plan).unwrap();
        assert_eq!(plan.target, QualificationTarget::Production);
        assert_eq!(plan.limits.maximum_microbenchmark_requests, 72);
        assert_eq!(digest.len(), 64);
    }
}
