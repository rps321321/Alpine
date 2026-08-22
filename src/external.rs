use crate::clock::UtcTimestamp;
use crate::evidence::{EvidenceStore, EvidenceWriter, RunEvidence};
use crate::identity::{runtime_bundle_sha256, sha256_bytes, sha256_file, tree_sha256};
use crate::qualification::EvidenceIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalEvidenceKind {
    SameProcess50RequestGreedyStability,
    TenCleanRestartGreedyStability,
    NearLimitContextStress,
    GoldenAgentTaskPass,
    OperatorReviewedCapabilityReport,
    RollbackProfileAvailable,
}

impl ExternalEvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameProcess50RequestGreedyStability => "same-process-50-request-greedy-stability",
            Self::TenCleanRestartGreedyStability => "ten-clean-restart-greedy-stability",
            Self::NearLimitContextStress => "near-limit-context-stress",
            Self::GoldenAgentTaskPass => "golden-agent-task-pass",
            Self::OperatorReviewedCapabilityReport => "operator-reviewed-capability-report",
            Self::RollbackProfileAvailable => "rollback-profile-available",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum EvidenceDecision {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalEvidence {
    schema: u32,
    kind: ExternalEvidenceKind,
    decision: EvidenceDecision,
    anchor_run_id: String,
    identity: EvidenceIdentity,
    producer_sha256: String,
    created_at: String,
    evidence: Value,
    reviewed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProcessEvidence {
    pub pid: u32,
    pub process_start_epoch_secs: u64,
    pub session_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum StabilityRequestRole {
    Contaminant,
    Target,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StabilityRequestContract {
    pub target_prompt_sha256: String,
    pub target_n_predict: u32,
    pub contaminant_n_predict: u32,
    pub temperature: f64,
    pub top_k: u32,
    pub seed: u64,
    pub ignore_eos: bool,
    pub cache_prompt: bool,
    pub return_tokens: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StabilityRequestEvidence {
    pub sequence: u32,
    pub role: StabilityRequestRole,
    pub prompt_sha256: String,
    pub token_sha256: String,
    pub tokens: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SameProcessStabilityEvidence {
    pub schema: u32,
    pub profile: String,
    pub process_before: ProcessEvidence,
    pub process_after: ProcessEvidence,
    pub request_contract: StabilityRequestContract,
    pub requests: Vec<StabilityRequestEvidence>,
    pub restored_prior_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestartRequestEvidence {
    pub sequence: u32,
    pub process: ProcessEvidence,
    pub prompt_sha256: String,
    pub token_sha256: String,
    pub tokens: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CleanRestartStabilityEvidence {
    pub schema: u32,
    pub profile: String,
    pub request_contract: StabilityRequestContract,
    pub restarts: Vec<RestartRequestEvidence>,
    pub restored_prior_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextRunEvidence {
    pub sequence: u32,
    pub content: String,
    pub content_sha256: String,
    pub token_sha256: String,
    pub tokens: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NearLimitContextEvidence {
    pub schema: u32,
    pub profile: String,
    pub generator: String,
    pub context_tokens: u32,
    pub ratio: f64,
    pub target_prompt_tokens: u32,
    pub actual_prompt_tokens: u32,
    pub prompt_sha256: String,
    pub needles: Vec<String>,
    pub process_before: ProcessEvidence,
    pub process_after: ProcessEvidence,
    pub runs: Vec<ContextRunEvidence>,
    pub restored_prior_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GoldenAgentEvidence {
    pub schema: u32,
    pub task_id: String,
    pub capabilities: Vec<crate::golden::GoldenCapability>,
    pub tool_calls: u64,
    pub tool_failures: u64,
    pub required_failure_matches: u64,
    pub suite_sha256: String,
    pub opencode_path: PathBuf,
    pub opencode_sha256: String,
    pub harness_policy_sha256: String,
    pub effective_config_sha256: String,
    pub test_executable_path: PathBuf,
    pub test_executable_sha256: String,
    pub agent_exit_code: i32,
    pub tests_exit_code: i32,
    pub protected_before: BTreeMap<String, String>,
    pub protected_after: BTreeMap<String, String>,
    pub unexpected_files: Vec<String>,
    pub agent_stdout_sha256: String,
    pub agent_stderr_sha256: String,
    pub tests_stdout_sha256: String,
    pub tests_stderr_sha256: String,
    pub restored_prior_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GoldenEvidenceRequirement {
    pub task_id: String,
    pub required_capabilities: Vec<crate::golden::GoldenCapability>,
}

pub(crate) struct ExternalEvidenceRequirements<'a> {
    pub kinds: &'a [String],
    pub golden: Option<&'a GoldenEvidenceRequirement>,
}

struct CurrentEvidenceContext<'a> {
    repository_root: &'a Path,
    producer_sha256: &'a str,
    golden_requirement: Option<&'a GoldenEvidenceRequirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RollbackProfileEvidence {
    pub schema: u32,
    pub profile: String,
    pub profile_path: PathBuf,
    pub profile_sha256: String,
    pub session_config_path: PathBuf,
    pub session_config_sha256: String,
    pub runtime: String,
    pub server_path: PathBuf,
    pub server_sha256: String,
    pub runtime_build_sha256: String,
    pub context_tokens: u32,
    pub process: ProcessEvidence,
    pub smoke_prompt_sha256: String,
    pub smoke_token_sha256: String,
    pub smoke_tokens: Vec<u32>,
    pub restored_prior_session: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityCategory {
    TrivialConversation,
    RepositoryOrientation,
    Diagnosis,
    ScopedCodeChange,
    LongerHorizonWork,
    WebResearch,
    PermissionBoundary,
    SessionLifecycle,
}

impl CapabilityCategory {
    const REQUIRED: [Self; 8] = [
        Self::TrivialConversation,
        Self::RepositoryOrientation,
        Self::Diagnosis,
        Self::ScopedCodeChange,
        Self::LongerHorizonWork,
        Self::WebResearch,
        Self::PermissionBoundary,
        Self::SessionLifecycle,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrivialConversation => "trivial-conversation",
            Self::RepositoryOrientation => "repository-orientation",
            Self::Diagnosis => "diagnosis",
            Self::ScopedCodeChange => "scoped-code-change",
            Self::LongerHorizonWork => "longer-horizon-work",
            Self::WebResearch => "web-research",
            Self::PermissionBoundary => "permission-boundary",
            Self::SessionLifecycle => "session-lifecycle",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityOutcome {
    Pass,
    Fail,
    Inconclusive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityDisposition {
    Blocking,
    AcceptedRisk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityReviewDecision {
    Approved,
    Rejected,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedResidualRisk {
    pub id: String,
    pub description: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityScenario {
    pub id: String,
    pub category: CapabilityCategory,
    pub task: String,
    pub expected_capability: String,
    pub observed_behavior: String,
    pub outcome: CapabilityOutcome,
    #[serde(default)]
    pub limitations: Vec<String>,
    pub disposition: Option<CapabilityDisposition>,
    pub disposition_rationale: Option<String>,
    #[serde(default)]
    pub accepted_risk_ids: Vec<String>,
    pub supporting_artifact_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityReviewEvidence {
    pub schema: u32,
    pub reviewer_role: String,
    pub scenarios: Vec<CapabilityScenario>,
    #[serde(default)]
    pub accepted_residual_risks: Vec<AcceptedResidualRisk>,
    pub final_decision: CapabilityReviewDecision,
    pub final_rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicCapabilityReviewFacts {
    pub schema: u32,
    pub reviewer_role: String,
    pub category_coverage: BTreeMap<String, u32>,
    pub outcome_counts: BTreeMap<String, u32>,
    pub disposition_counts: BTreeMap<String, u32>,
    pub scenario_count: u32,
    pub accepted_residual_risk_count: u32,
    pub final_decision: CapabilityReviewDecision,
    pub supporting_artifact_sha256: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RecordExternalEvidenceOptions {
    pub(crate) repository_root: PathBuf,
    pub(crate) database: PathBuf,
    pub(crate) result_root: PathBuf,
    pub(crate) anchor_run_id: String,
    pub(crate) kind: ExternalEvidenceKind,
    pub(crate) evidence: Value,
    pub(crate) reviewed_by: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OperatorReviewOptions {
    pub repository_root: PathBuf,
    pub database: PathBuf,
    pub result_root: PathBuf,
    pub anchor_run_id: String,
    pub evidence: Value,
    pub reviewed_by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordedExternalEvidence {
    pub kind: ExternalEvidenceKind,
    pub anchor_run_id: String,
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalEvidenceStatusKind {
    Satisfied,
    Missing,
    Rejected,
    Inconclusive,
    Stale,
    IdentityMismatched,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalEvidenceStatus {
    pub name: String,
    pub status: ExternalEvidenceStatusKind,
    pub path: Option<PathBuf>,
    pub sha256: Option<String>,
    pub reason: Option<String>,
}

pub(crate) fn record(
    options: &RecordExternalEvidenceOptions,
) -> Result<RecordedExternalEvidence, String> {
    let evidence_bytes = serde_json::to_vec(&options.evidence)
        .map_err(|error| format!("failed to serialize evidence details: {error}"))?;
    if evidence_bytes.len() > 1024 * 1024 {
        return Err("evidence details must be no larger than 1 MiB".to_owned());
    }
    let database_path = std::fs::canonicalize(&options.database).map_err(|error| {
        format!(
            "failed to resolve evidence database {}: {error}",
            options.database.display()
        )
    })?;
    let database_result_root = database_path
        .parent()
        .ok_or_else(|| "evidence database has no result-root parent".to_owned())?;
    let result_root = std::fs::canonicalize(&options.result_root).map_err(|error| {
        format!(
            "failed to resolve result root {}: {error}",
            options.result_root.display()
        )
    })?;
    if result_root != database_result_root {
        return Err(
            "result root must be the directory containing the evidence database".to_owned(),
        );
    }
    let store = EvidenceStore::open_read_only(&options.database)?;
    let anchor = store
        .run(&options.anchor_run_id)?
        .ok_or_else(|| format!("evidence run not found: {}", options.anchor_run_id))?;
    let identity = complete_identity(&anchor)?;
    if anchor.summary.kind != "micro"
        || anchor.config.get("evidence_phase").and_then(Value::as_str) != Some("final")
        || anchor.summary.status != "passed"
        || anchor.summary.finished_at.is_none()
    {
        return Err("external evidence can only attach to a finished, passed final run".to_owned());
    }
    let producer_sha256 = sha256_file(
        &std::env::current_exe()
            .map_err(|error| format!("failed to locate Alpine executable: {error}"))?,
    )?;
    if producer_sha256 != identity.software {
        return Err("current Alpine binary does not match the anchor software identity".to_owned());
    }
    let reviewed_by = options
        .reviewed_by
        .as_ref()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if options.kind == ExternalEvidenceKind::OperatorReviewedCapabilityReport
        && reviewed_by.is_none()
    {
        return Err("operator capability evidence requires an explicit reviewer".to_owned());
    }
    if reviewed_by
        .as_ref()
        .is_some_and(|value| value.len() > 200 || value.chars().any(char::is_control))
    {
        return Err("evidence reviewer must be at most 200 printable characters".to_owned());
    }
    let decision = evidence_decision(options.kind, &options.evidence)?;
    let payload = ExternalEvidence {
        schema: 1,
        kind: options.kind,
        decision,
        anchor_run_id: options.anchor_run_id.clone(),
        identity: identity.clone(),
        producer_sha256,
        created_at: UtcTimestamp::now()?.rfc3339(),
        evidence: options.evidence.clone(),
        reviewed_by,
    };
    validate_contract(&payload, options.kind, &options.anchor_run_id, &identity)?;
    validate_current(
        &payload,
        options.kind,
        &options.repository_root,
        &identity.software,
        None,
    )?;
    validate_semantics(&payload, options.kind)?;
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("failed to serialize external evidence: {error}"))?;
    let directory = result_root
        .join("runs")
        .join(&options.anchor_run_id)
        .join("external");
    std::fs::create_dir_all(&directory).map_err(|error| {
        format!(
            "failed to create external evidence directory {}: {error}",
            directory.display()
        )
    })?;
    let path = directory.join(format!("{}.json", options.kind.as_str()));
    let digest = if path.exists() {
        let existing_bytes = std::fs::read(&path).map_err(|error| {
            format!(
                "failed to read existing external evidence {}: {error}",
                path.display()
            )
        })?;
        let existing: ExternalEvidence =
            serde_json::from_slice(&existing_bytes).map_err(|error| {
                format!("external evidence is immutable and the existing file is invalid: {error}")
            })?;
        validate_contract(&existing, options.kind, &options.anchor_run_id, &identity)?;
        validate_current(
            &existing,
            options.kind,
            &options.repository_root,
            &identity.software,
            None,
        )?;
        validate_semantics(&existing, options.kind)?;
        if existing.evidence != options.evidence || existing.reviewed_by != payload.reviewed_by {
            return Err(format!(
                "external evidence is immutable and already exists with different content: {}",
                path.display()
            ));
        }
        sha256_bytes(&existing_bytes)
    } else {
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)
            .map_err(|error| format!("failed to stage external evidence: {error}"))?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file_mut().sync_all())
            .map_err(|error| format!("failed to durably stage external evidence: {error}"))?;
        temporary
            .persist(&path)
            .map_err(|error| format!("failed to publish external evidence: {}", error.error))?;
        sha256_bytes(&bytes)
    };
    drop(store);
    let mut writer = EvidenceWriter::open(&options.database)?;
    if let Err(error) = writer.attach_artifact(
        &options.anchor_run_id,
        options.kind.as_str(),
        &path,
        &digest,
    ) {
        return Err(format!(
            "{error}; artifact file was preserved for recovery at {}",
            path.display()
        ));
    }
    Ok(RecordedExternalEvidence {
        kind: options.kind,
        anchor_run_id: options.anchor_run_id.clone(),
        path,
        sha256: digest,
    })
}

pub fn record_operator_review(
    options: &OperatorReviewOptions,
) -> Result<RecordedExternalEvidence, String> {
    record(&RecordExternalEvidenceOptions {
        repository_root: options.repository_root.clone(),
        database: options.database.clone(),
        result_root: options.result_root.clone(),
        anchor_run_id: options.anchor_run_id.clone(),
        kind: ExternalEvidenceKind::OperatorReviewedCapabilityReport,
        evidence: options.evidence.clone(),
        reviewed_by: Some(options.reviewed_by.clone()),
    })
}

pub(crate) fn current_anchor(database: &Path, anchor_run_id: &str) -> Result<RunEvidence, String> {
    let store = EvidenceStore::open_read_only(database)?;
    let anchor = store
        .run(anchor_run_id)?
        .ok_or_else(|| format!("evidence run not found: {anchor_run_id}"))?;
    let identity = complete_identity(&anchor)?;
    if anchor.summary.kind != "micro"
        || anchor.config.get("evidence_phase").and_then(Value::as_str) != Some("final")
        || anchor.summary.status != "passed"
        || anchor.summary.finished_at.is_none()
    {
        return Err("external evidence can only attach to a finished, passed final run".to_owned());
    }
    let current_binary_sha256 = sha256_file(
        &std::env::current_exe()
            .map_err(|error| format!("failed to locate Alpine executable: {error}"))?,
    )?;
    if current_binary_sha256 != identity.software {
        return Err("current Alpine binary does not match the anchor software identity".to_owned());
    }
    Ok(anchor)
}

pub(crate) fn public_capability_review_facts(
    path: &Path,
    declared_sha256: &str,
    anchor_run_id: &str,
    identity: &EvidenceIdentity,
) -> Result<PublicCapabilityReviewFacts, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("capability review artifact is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 2 * 1024 * 1024
    {
        return Err(
            "capability review artifact must be a real file no larger than 2 MiB".to_owned(),
        );
    }
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read capability review artifact: {error}"))?;
    if sha256_bytes(&bytes) != declared_sha256 {
        return Err("capability review artifact digest does not match Qualification".to_owned());
    }
    let payload: ExternalEvidence = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid capability review artifact: {error}"))?;
    validate_contract(
        &payload,
        ExternalEvidenceKind::OperatorReviewedCapabilityReport,
        anchor_run_id,
        identity,
    )?;
    validate_semantics(
        &payload,
        ExternalEvidenceKind::OperatorReviewedCapabilityReport,
    )?;
    let review: CapabilityReviewEvidence = serde_json::from_value(payload.evidence)
        .map_err(|error| format!("invalid structured capability review: {error}"))?;

    let mut category_coverage = BTreeMap::new();
    let mut outcome_counts = BTreeMap::from([
        ("pass".to_owned(), 0),
        ("fail".to_owned(), 0),
        ("inconclusive".to_owned(), 0),
    ]);
    let mut disposition_counts = BTreeMap::from([
        ("blocking".to_owned(), 0),
        ("accepted-risk".to_owned(), 0),
        ("not-applicable".to_owned(), 0),
    ]);
    let mut supporting_artifact_sha256 = BTreeSet::new();
    for scenario in &review.scenarios {
        *category_coverage
            .entry(scenario.category.as_str().to_owned())
            .or_insert(0) += 1;
        let outcome = match scenario.outcome {
            CapabilityOutcome::Pass => "pass",
            CapabilityOutcome::Fail => "fail",
            CapabilityOutcome::Inconclusive => "inconclusive",
        };
        *outcome_counts.entry(outcome.to_owned()).or_insert(0) += 1;
        let disposition = match scenario.disposition {
            Some(CapabilityDisposition::Blocking) => "blocking",
            Some(CapabilityDisposition::AcceptedRisk) => "accepted-risk",
            None => "not-applicable",
        };
        *disposition_counts
            .entry(disposition.to_owned())
            .or_insert(0) += 1;
        if let Some(digest) = &scenario.supporting_artifact_sha256 {
            supporting_artifact_sha256.insert(digest.clone());
        }
    }
    Ok(PublicCapabilityReviewFacts {
        schema: review.schema,
        reviewer_role: review.reviewer_role,
        category_coverage,
        outcome_counts,
        disposition_counts,
        scenario_count: review.scenarios.len() as u32,
        accepted_residual_risk_count: review.accepted_residual_risks.len() as u32,
        final_decision: review.final_decision,
        supporting_artifact_sha256: supporting_artifact_sha256.into_iter().collect(),
    })
}

pub(crate) fn inspect_required(
    store: &EvidenceStore,
    anchor_run_id: &str,
    identity: &EvidenceIdentity,
    requirements: &ExternalEvidenceRequirements<'_>,
    repository_root: &Path,
    result_root: &Path,
    producer_sha256: &str,
) -> Result<Vec<ExternalEvidenceStatus>, String> {
    let artifacts = store.artifacts(anchor_run_id)?;
    let mut by_kind: BTreeMap<&str, Vec<_>> = BTreeMap::new();
    for artifact in &artifacts {
        by_kind.entry(&artifact.kind).or_default().push(artifact);
    }
    let mut statuses = Vec::with_capacity(requirements.kinds.len());
    let current = CurrentEvidenceContext {
        repository_root,
        producer_sha256,
        golden_requirement: requirements.golden,
    };
    for name in requirements.kinds {
        let kind = parse_kind(name)?;
        let Some(matches) = by_kind.get(name.as_str()) else {
            statuses.push(status(
                name,
                ExternalEvidenceStatusKind::Missing,
                None,
                None,
            ));
            continue;
        };
        if matches.len() != 1 {
            statuses.push(status(
                name,
                ExternalEvidenceStatusKind::Invalid,
                None,
                Some("multiple artifacts claim the same evidence kind".to_owned()),
            ));
            continue;
        }
        let artifact = matches[0];
        let expected_path = result_root
            .join("runs")
            .join(anchor_run_id)
            .join("external")
            .join(format!("{}.json", kind.as_str()));
        if artifact.path != expected_path {
            statuses.push(ExternalEvidenceStatus {
                name: name.clone(),
                status: ExternalEvidenceStatusKind::Invalid,
                path: Some(artifact.path.clone()),
                sha256: artifact.sha256.clone(),
                reason: Some("artifact path is outside its canonical run directory".to_owned()),
            });
            continue;
        }
        let result = inspect_artifact(
            artifact.path.as_path(),
            artifact.sha256.as_deref(),
            kind,
            anchor_run_id,
            identity,
            &current,
        );
        statuses.push(match result {
            Ok(digest) => ExternalEvidenceStatus {
                name: name.clone(),
                status: ExternalEvidenceStatusKind::Satisfied,
                path: Some(artifact.path.clone()),
                sha256: Some(digest),
                reason: None,
            },
            Err((kind, reason)) => ExternalEvidenceStatus {
                name: name.clone(),
                status: kind,
                path: Some(artifact.path.clone()),
                sha256: artifact.sha256.clone(),
                reason: Some(reason),
            },
        });
    }
    Ok(statuses)
}

fn inspect_artifact(
    path: &Path,
    declared_sha256: Option<&str>,
    kind: ExternalEvidenceKind,
    anchor_run_id: &str,
    identity: &EvidenceIdentity,
    current: &CurrentEvidenceContext<'_>,
) -> Result<String, (ExternalEvidenceStatusKind, String)> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        (
            ExternalEvidenceStatusKind::Stale,
            format!("artifact is unavailable: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 {
        return Err((
            ExternalEvidenceStatusKind::Invalid,
            "artifact must be a file no larger than 2 MiB".to_owned(),
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        (
            ExternalEvidenceStatusKind::Stale,
            format!("artifact is unavailable: {error}"),
        )
    })?;
    let digest = sha256_bytes(&bytes);
    if declared_sha256 != Some(digest.as_str()) {
        return Err((
            ExternalEvidenceStatusKind::Stale,
            "artifact digest does not match SQLite".to_owned(),
        ));
    }
    let payload: ExternalEvidence = serde_json::from_slice(&bytes).map_err(|error| {
        (
            ExternalEvidenceStatusKind::Invalid,
            format!("invalid external evidence JSON: {error}"),
        )
    })?;
    validate_contract(&payload, kind, anchor_run_id, identity)
        .map_err(|reason| (ExternalEvidenceStatusKind::IdentityMismatched, reason))?;
    validate_current(
        &payload,
        kind,
        current.repository_root,
        current.producer_sha256,
        current.golden_requirement,
    )
    .map_err(|reason| (ExternalEvidenceStatusKind::Stale, reason))?;
    validate_semantics(&payload, kind)
        .map_err(|reason| (ExternalEvidenceStatusKind::Invalid, reason))?;
    if kind == ExternalEvidenceKind::OperatorReviewedCapabilityReport {
        match payload.decision {
            EvidenceDecision::Pass => {}
            EvidenceDecision::Fail => {
                return Err((
                    ExternalEvidenceStatusKind::Rejected,
                    "the human capability review rejected this deployment".to_owned(),
                ));
            }
            EvidenceDecision::Inconclusive => {
                return Err((
                    ExternalEvidenceStatusKind::Inconclusive,
                    "the human capability review was inconclusive".to_owned(),
                ));
            }
        }
    }
    Ok(digest)
}

fn validate_contract(
    payload: &ExternalEvidence,
    kind: ExternalEvidenceKind,
    anchor_run_id: &str,
    identity: &EvidenceIdentity,
) -> Result<(), String> {
    if payload.schema != 1
        || payload.kind != kind
        || payload.anchor_run_id != anchor_run_id
        || &payload.identity != identity
    {
        return Err("artifact contract, anchor, or identity does not match the claim".to_owned());
    }
    if payload.producer_sha256 != identity.software {
        return Err("artifact producer does not match the claim software identity".to_owned());
    }
    Ok(())
}

fn validate_current(
    payload: &ExternalEvidence,
    kind: ExternalEvidenceKind,
    repository_root: &Path,
    producer_sha256: &str,
    golden_requirement: Option<&GoldenEvidenceRequirement>,
) -> Result<(), String> {
    if payload.producer_sha256 != producer_sha256 {
        return Err("artifact was produced by a stale Alpine binary".to_owned());
    }
    if kind == ExternalEvidenceKind::GoldenAgentTaskPass {
        let evidence: GoldenAgentEvidence = serde_json::from_value(payload.evidence.clone())
            .map_err(|error| format!("invalid golden-agent evidence: {error}"))?;
        if !valid_golden_task_id(&evidence.task_id) {
            return Err("golden-agent evidence task id is invalid".to_owned());
        }
        if let Some(requirement) = golden_requirement {
            validate_golden_requirement(&evidence, requirement)?;
        }
        let task_root = repository_root
            .join("benchmarks/golden")
            .join(&evidence.task_id);
        let files = walk_files(&task_root)?
            .into_iter()
            .filter(|path| {
                !path
                    .components()
                    .any(|part| part.as_os_str() == "__pycache__")
            })
            .collect::<Vec<_>>();
        if tree_sha256(&task_root, &files)? != evidence.suite_sha256 {
            return Err("golden task suite identity is stale".to_owned());
        }
        let task_path = task_root.join("task.json");
        let task: Value = serde_json::from_slice(&std::fs::read(&task_path).map_err(|error| {
            format!(
                "golden task contract is unavailable at {}: {error}",
                task_path.display()
            )
        })?)
        .map_err(|error| format!("golden task contract is invalid: {error}"))?;
        let capabilities: Vec<crate::golden::GoldenCapability> = serde_json::from_value(
            task.get("capabilities")
                .cloned()
                .ok_or_else(|| "golden task capabilities are missing".to_owned())?,
        )
        .map_err(|error| format!("golden task capabilities are invalid: {error}"))?;
        if task.get("schema").and_then(Value::as_u64) != Some(2)
            || task.get("visibility").and_then(Value::as_str) != Some("public")
            || capabilities != evidence.capabilities
        {
            return Err(
                "golden-agent evidence capabilities do not match the public task".to_owned(),
            );
        }
        let required_failure_count = task
            .pointer("/trace_gate/expected_failure/exact_count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if evidence.required_failure_matches != required_failure_count {
            return Err(
                "golden-agent evidence does not match the current required failure effect"
                    .to_owned(),
            );
        }
        let current_opencode = crate::process::resolve_executable("opencode")
            .ok_or_else(|| "OpenCode executable is unavailable".to_owned())?;
        validate_executable_identity(
            &current_opencode,
            &evidence.opencode_path,
            &evidence.opencode_sha256,
            "OpenCode",
        )?;
        let test_command = task
            .get("test_command")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_str)
            .ok_or_else(|| "golden task test executable is missing".to_owned())?;
        let current_test = crate::process::resolve_executable(test_command)
            .ok_or_else(|| "golden task test executable is unavailable".to_owned())?;
        validate_executable_identity(
            &current_test,
            &evidence.test_executable_path,
            &evidence.test_executable_sha256,
            "test",
        )?;
    }
    if kind == ExternalEvidenceKind::RollbackProfileAvailable {
        let evidence: RollbackProfileEvidence = serde_json::from_value(payload.evidence.clone())
            .map_err(|error| format!("invalid rollback-profile evidence: {error}"))?;
        let profile_path = std::fs::canonicalize(&evidence.profile_path)
            .map_err(|error| format!("rollback Profile is unavailable: {error}"))?;
        let session_path = std::fs::canonicalize(&evidence.session_config_path)
            .map_err(|error| format!("rollback Session Config is unavailable: {error}"))?;
        let server_path = std::fs::canonicalize(&evidence.server_path)
            .map_err(|error| format!("rollback runtime is unavailable: {error}"))?;
        let install_root = profile_path
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| "rollback Profile has no install root".to_owned())?;
        if profile_path != install_root.join("profiles/stable-16k.json")
            || session_path != install_root.join("config/session.json")
            || sha256_file(&profile_path)? != evidence.profile_sha256
            || sha256_file(&session_path)? != evidence.session_config_sha256
            || sha256_file(&server_path)? != evidence.server_sha256
            || runtime_bundle_sha256(&server_path)? != evidence.runtime_build_sha256
        {
            return Err(
                "rollback Profile, Session Config, or runtime identity is stale".to_owned(),
            );
        }
        let profile: crate::config::Profile = serde_json::from_slice(
            &std::fs::read(&profile_path)
                .map_err(|error| format!("failed to read rollback Profile: {error}"))?,
        )
        .map_err(|error| format!("invalid rollback Profile: {error}"))?;
        let deployment = crate::deployment::status(install_root)?;
        let rollback_role = deployment
            .roles
            .as_ref()
            .map(|roles| roles.rollback_profile.as_str());
        if profile.name != "stable-16k"
            || rollback_role != Some("stable-16k")
            || profile.context != evidence.context_tokens
            || profile.runtime != evidence.runtime
        {
            return Err(
                "rollback Profile no longer satisfies the current rollback-role contract"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

fn validate_golden_requirement(
    evidence: &GoldenAgentEvidence,
    requirement: &GoldenEvidenceRequirement,
) -> Result<(), String> {
    let required = requirement
        .required_capabilities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let observed = evidence
        .capabilities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if evidence.task_id != requirement.task_id || !required.is_subset(&observed) {
        return Err(format!(
            "golden-agent evidence does not satisfy required task '{}' and capabilities",
            requirement.task_id
        ));
    }
    Ok(())
}

fn validate_executable_identity(
    current: &Path,
    recorded: &Path,
    recorded_sha256: &str,
    label: &str,
) -> Result<(), String> {
    let current = std::fs::canonicalize(current)
        .map_err(|error| format!("failed to resolve golden {label} executable: {error}"))?;
    let recorded = std::fs::canonicalize(recorded)
        .map_err(|error| format!("recorded golden {label} executable is unavailable: {error}"))?;
    if current != recorded || sha256_file(&current)? != recorded_sha256 {
        return Err(format!("golden-agent {label} executable identity is stale"));
    }
    Ok(())
}

fn validate_semantics(
    payload: &ExternalEvidence,
    kind: ExternalEvidenceKind,
) -> Result<(), String> {
    match kind {
        ExternalEvidenceKind::SameProcess50RequestGreedyStability => {
            validate_same_process(&payload.evidence)?;
        }
        ExternalEvidenceKind::TenCleanRestartGreedyStability => {
            validate_clean_restarts(&payload.evidence)?;
        }
        ExternalEvidenceKind::NearLimitContextStress => {
            validate_near_limit_context(&payload.evidence)?;
        }
        ExternalEvidenceKind::GoldenAgentTaskPass => {
            validate_golden_agent(&payload.evidence)?;
        }
        ExternalEvidenceKind::OperatorReviewedCapabilityReport => {
            if payload
                .reviewed_by
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err("operator capability evidence has no explicit reviewer".to_owned());
            }
            let review = validate_capability_review(&payload.evidence)?;
            let expected = match review.final_decision {
                CapabilityReviewDecision::Approved => EvidenceDecision::Pass,
                CapabilityReviewDecision::Rejected => EvidenceDecision::Fail,
                CapabilityReviewDecision::Inconclusive => EvidenceDecision::Inconclusive,
            };
            if payload.decision != expected {
                return Err(
                    "operator capability decision does not match the structured review".to_owned(),
                );
            }
        }
        ExternalEvidenceKind::RollbackProfileAvailable => {
            validate_rollback_profile(&payload.evidence)?;
        }
    }
    if kind != ExternalEvidenceKind::OperatorReviewedCapabilityReport
        && payload.decision != EvidenceDecision::Pass
    {
        return Err("automated external evidence must carry a pass decision".to_owned());
    }
    Ok(())
}

fn evidence_decision(
    kind: ExternalEvidenceKind,
    value: &Value,
) -> Result<EvidenceDecision, String> {
    if kind != ExternalEvidenceKind::OperatorReviewedCapabilityReport {
        return Ok(EvidenceDecision::Pass);
    }
    let review = validate_capability_review(value)?;
    Ok(match review.final_decision {
        CapabilityReviewDecision::Approved => EvidenceDecision::Pass,
        CapabilityReviewDecision::Rejected => EvidenceDecision::Fail,
        CapabilityReviewDecision::Inconclusive => EvidenceDecision::Inconclusive,
    })
}

fn validate_capability_review(value: &Value) -> Result<CapabilityReviewEvidence, String> {
    let review: CapabilityReviewEvidence = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid capability-review evidence: {error}"))?;
    if review.schema != 1 {
        return Err("capability-review evidence schema must be 1".to_owned());
    }
    validate_review_text("reviewer_role", &review.reviewer_role, 200)?;
    validate_review_text("final_rationale", &review.final_rationale, 4000)?;
    if review.scenarios.len() < CapabilityCategory::REQUIRED.len() || review.scenarios.len() > 64 {
        return Err("capability review must contain between 8 and 64 scenarios".to_owned());
    }
    if review.accepted_residual_risks.len() > 64 {
        return Err("capability review contains too many accepted residual risks".to_owned());
    }

    let mut scenario_ids = BTreeSet::new();
    let mut categories = BTreeSet::new();
    let mut risk_ids = BTreeSet::new();
    for risk in &review.accepted_residual_risks {
        validate_review_id("accepted residual risk", &risk.id)?;
        validate_review_text(
            "accepted residual risk description",
            &risk.description,
            4000,
        )?;
        validate_review_text("accepted residual risk rationale", &risk.rationale, 4000)?;
        if !risk_ids.insert(risk.id.as_str()) {
            return Err(format!("duplicate accepted residual risk id: {}", risk.id));
        }
    }

    let mut has_blocking = false;
    for scenario in &review.scenarios {
        validate_review_id("scenario", &scenario.id)?;
        if !scenario_ids.insert(scenario.id.as_str()) {
            return Err(format!("duplicate capability scenario id: {}", scenario.id));
        }
        categories.insert(scenario.category);
        validate_review_text("scenario task", &scenario.task, 8000)?;
        validate_review_text(
            "scenario expected_capability",
            &scenario.expected_capability,
            4000,
        )?;
        validate_review_text(
            "scenario observed_behavior",
            &scenario.observed_behavior,
            8000,
        )?;
        if scenario.limitations.len() > 32 {
            return Err(format!(
                "scenario {} contains too many limitations",
                scenario.id
            ));
        }
        for limitation in &scenario.limitations {
            validate_review_text("scenario limitation", limitation, 4000)?;
        }
        if let Some(digest) = scenario.supporting_artifact_sha256.as_deref() {
            if !is_sha256(digest) {
                return Err(format!(
                    "scenario {} supporting artifact digest is not SHA-256",
                    scenario.id
                ));
            }
        }

        match scenario.outcome {
            CapabilityOutcome::Pass => {
                if scenario.disposition.is_some()
                    || scenario.disposition_rationale.is_some()
                    || !scenario.accepted_risk_ids.is_empty()
                {
                    return Err(format!(
                        "passing scenario {} must not carry a failure disposition",
                        scenario.id
                    ));
                }
            }
            CapabilityOutcome::Fail | CapabilityOutcome::Inconclusive => {
                let disposition = scenario.disposition.ok_or_else(|| {
                    format!(
                        "non-passing scenario {} requires a blocking or accepted-risk disposition",
                        scenario.id
                    )
                })?;
                validate_review_text(
                    "scenario disposition rationale",
                    scenario
                        .disposition_rationale
                        .as_deref()
                        .unwrap_or_default(),
                    4000,
                )?;
                match disposition {
                    CapabilityDisposition::Blocking => {
                        has_blocking = true;
                        if !scenario.accepted_risk_ids.is_empty() {
                            return Err(format!(
                                "blocking scenario {} cannot cite accepted residual risks",
                                scenario.id
                            ));
                        }
                    }
                    CapabilityDisposition::AcceptedRisk => {
                        if scenario.accepted_risk_ids.is_empty() {
                            return Err(format!(
                                "accepted-risk scenario {} must cite at least one residual risk",
                                scenario.id
                            ));
                        }
                        for id in &scenario.accepted_risk_ids {
                            if !risk_ids.contains(id.as_str()) {
                                return Err(format!(
                                    "scenario {} cites unknown residual risk {}",
                                    scenario.id, id
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    let missing = CapabilityCategory::REQUIRED
        .into_iter()
        .filter(|category| !categories.contains(category))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "capability review is missing required categories: {:?}",
            missing
        ));
    }
    if review.final_decision == CapabilityReviewDecision::Approved && has_blocking {
        return Err("approved capability review cannot contain a blocking disposition".to_owned());
    }
    Ok(review)
}

fn validate_review_id(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!(
            "{label} id must be 1-100 lowercase ASCII letters, digits, or hyphens"
        ));
    }
    Ok(())
}

fn validate_review_text(label: &str, value: &str, maximum: usize) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > maximum
        || trimmed
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(format!(
            "{label} must be non-empty, at most {maximum} characters, and contain no unsafe controls"
        ));
    }
    Ok(())
}

fn validate_same_process(value: &Value) -> Result<(), String> {
    let evidence: SameProcessStabilityEvidence = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid same-process evidence: {error}"))?;
    if evidence.schema != 1
        || evidence.profile.trim().is_empty()
        || evidence.process_before != evidence.process_after
        || evidence.process_before.pid == 0
        || evidence.process_before.process_start_epoch_secs == 0
        || !is_session_identity(&evidence.process_before.session_identity)
        || !evidence.restored_prior_session
    {
        return Err(
            "same-process evidence lacks a stable verified process or restoration proof".to_owned(),
        );
    }
    let contract = &evidence.request_contract;
    if !is_sha256(&contract.target_prompt_sha256)
        || contract.target_n_predict < 128
        || contract.contaminant_n_predict < 16
        || contract.temperature != 0.0
        || contract.top_k != 1
        || contract.seed != 42
        || !contract.ignore_eos
        || contract.cache_prompt
        || !contract.return_tokens
    {
        return Err("same-process request contract is weaker than the versioned gate".to_owned());
    }
    if evidence.requests.len() != 100 {
        return Err("same-process evidence must contain exactly 100 request records".to_owned());
    }
    let mut target_hashes = std::collections::BTreeSet::new();
    let mut contaminant_prompts = std::collections::BTreeSet::new();
    for (offset, request) in evidence.requests.iter().enumerate() {
        let expected_sequence = u32::try_from(offset + 1)
            .map_err(|_| "same-process request sequence overflow".to_owned())?;
        let expected_role = if offset % 2 == 0 {
            StabilityRequestRole::Contaminant
        } else {
            StabilityRequestRole::Target
        };
        if request.sequence != expected_sequence || request.role != expected_role {
            return Err(
                "same-process requests must alternate contaminant then target in exact sequence"
                    .to_owned(),
            );
        }
        if !is_sha256(&request.prompt_sha256) || !is_sha256(&request.token_sha256) {
            return Err("same-process request contains a malformed SHA-256 identity".to_owned());
        }
        let token_bytes = serde_json::to_vec(&request.tokens)
            .map_err(|error| format!("failed to encode stability tokens: {error}"))?;
        if sha256_bytes(&token_bytes) != request.token_sha256 {
            return Err("same-process request token digest does not match raw tokens".to_owned());
        }
        let required_tokens = match request.role {
            StabilityRequestRole::Contaminant => {
                contaminant_prompts.insert(request.prompt_sha256.clone());
                contract.contaminant_n_predict
            }
            StabilityRequestRole::Target => {
                if request.prompt_sha256 != contract.target_prompt_sha256 {
                    return Err("same-process target prompt identity changed".to_owned());
                }
                target_hashes.insert(request.token_sha256.clone());
                contract.target_n_predict
            }
        };
        if request.tokens.len() != required_tokens as usize {
            return Err("same-process request did not generate its full token target".to_owned());
        }
    }
    if contaminant_prompts.len() != 50 || target_hashes.len() != 1 {
        return Err(
            "same-process evidence requires 50 distinct contaminants and one target token hash"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_clean_restarts(value: &Value) -> Result<(), String> {
    let evidence: CleanRestartStabilityEvidence = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid clean-restart evidence: {error}"))?;
    if evidence.schema != 1
        || evidence.profile.trim().is_empty()
        || !evidence.restored_prior_session
        || evidence.restarts.len() != 10
    {
        return Err(
            "clean-restart evidence requires ten records and exact prior restoration".to_owned(),
        );
    }
    let contract = &evidence.request_contract;
    if !is_sha256(&contract.target_prompt_sha256)
        || contract.target_n_predict < 128
        || contract.contaminant_n_predict != 0
        || contract.temperature != 0.0
        || contract.top_k != 1
        || contract.seed != 42
        || !contract.ignore_eos
        || contract.cache_prompt
        || !contract.return_tokens
    {
        return Err("clean-restart request contract is weaker than the versioned gate".to_owned());
    }
    let mut process_identities = std::collections::BTreeSet::new();
    let mut token_hashes = std::collections::BTreeSet::new();
    for (offset, restart) in evidence.restarts.iter().enumerate() {
        let expected_sequence = u32::try_from(offset + 1)
            .map_err(|_| "clean-restart request sequence overflow".to_owned())?;
        if restart.sequence != expected_sequence
            || restart.prompt_sha256 != contract.target_prompt_sha256
            || !valid_process(&restart.process)
            || !is_sha256(&restart.token_sha256)
            || restart.tokens.len() != contract.target_n_predict as usize
        {
            return Err("clean-restart record has an invalid sequence or identity".to_owned());
        }
        let token_bytes = serde_json::to_vec(&restart.tokens)
            .map_err(|error| format!("failed to encode restart tokens: {error}"))?;
        if sha256_bytes(&token_bytes) != restart.token_sha256 {
            return Err("clean-restart token digest does not match raw tokens".to_owned());
        }
        process_identities.insert((
            restart.process.pid,
            restart.process.process_start_epoch_secs,
            restart.process.session_identity.clone(),
        ));
        token_hashes.insert(restart.token_sha256.clone());
    }
    if process_identities.len() != 10 || token_hashes.len() != 1 {
        return Err(
            "clean-restart evidence requires ten unique processes and one target token hash"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_near_limit_context(value: &Value) -> Result<(), String> {
    const NEEDLES: [&str; 3] = ["CEDAR-48291", "ORBIT-73064", "VIOLET-19538"];
    let evidence: NearLimitContextEvidence = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid near-limit context evidence: {error}"))?;
    if evidence.schema != 1
        || evidence.profile.trim().is_empty()
        || evidence.generator != "immutable-ledger-v1"
        || evidence.context_tokens < 1024
        || !(0.85..=0.95).contains(&evidence.ratio)
        || evidence.process_before != evidence.process_after
        || !valid_process(&evidence.process_before)
        || !evidence.restored_prior_session
        || !is_sha256(&evidence.prompt_sha256)
        || evidence.needles != NEEDLES
        || evidence.runs.len() != 2
    {
        return Err("near-limit context evidence contract is incomplete".to_owned());
    }
    let expected_target = (f64::from(evidence.context_tokens) * evidence.ratio).floor() as u32;
    let minimum_prompt =
        (f64::from(evidence.context_tokens) * evidence.ratio * 0.98).floor() as u32;
    if evidence.target_prompt_tokens != expected_target
        || evidence.actual_prompt_tokens < minimum_prompt
        || evidence.actual_prompt_tokens > expected_target
    {
        return Err("context evidence did not reach its claimed near-limit target".to_owned());
    }
    let expected = NEEDLES.join("|");
    for (offset, run) in evidence.runs.iter().enumerate() {
        let expected_sequence = u32::try_from(offset + 1)
            .map_err(|_| "context request sequence overflow".to_owned())?;
        if run.sequence != expected_sequence
            || run.content.trim() != expected
            || run.tokens.is_empty()
            || !is_sha256(&run.content_sha256)
            || !is_sha256(&run.token_sha256)
            || sha256_bytes(run.content.as_bytes()) != run.content_sha256
            || sha256_bytes(
                &serde_json::to_vec(&run.tokens)
                    .map_err(|error| format!("failed to encode context tokens: {error}"))?,
            ) != run.token_sha256
        {
            return Err("context run failed raw retrieval or digest verification".to_owned());
        }
    }
    Ok(())
}

fn validate_golden_agent(value: &Value) -> Result<(), String> {
    let evidence: GoldenAgentEvidence = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid golden-agent evidence: {error}"))?;
    let hashes = [
        &evidence.suite_sha256,
        &evidence.opencode_sha256,
        &evidence.harness_policy_sha256,
        &evidence.effective_config_sha256,
        &evidence.test_executable_sha256,
        &evidence.agent_stdout_sha256,
        &evidence.agent_stderr_sha256,
        &evidence.tests_stdout_sha256,
        &evidence.tests_stderr_sha256,
    ];
    if evidence.schema != 3
        || !valid_golden_task_id(&evidence.task_id)
        || evidence.capabilities.is_empty()
        || evidence.capabilities.iter().collect::<BTreeSet<_>>().len()
            != evidence.capabilities.len()
        || evidence.tool_calls == 0
        || evidence.tool_failures > evidence.tool_calls
        || evidence.required_failure_matches > evidence.tool_failures
        || !evidence.opencode_path.is_absolute()
        || !evidence.test_executable_path.is_absolute()
        || hashes.into_iter().any(|hash| !is_sha256(hash))
        || evidence.agent_exit_code != 0
        || evidence.tests_exit_code != 0
        || evidence.protected_before.is_empty()
        || evidence.protected_before != evidence.protected_after
        || !evidence.unexpected_files.is_empty()
        || !evidence.restored_prior_session
    {
        return Err("golden-agent evidence did not pass its raw result contract".to_owned());
    }
    if evidence
        .protected_before
        .iter()
        .any(|(path, hash)| path.trim().is_empty() || !is_sha256(hash))
    {
        return Err("golden-agent protected path evidence is malformed".to_owned());
    }
    Ok(())
}

fn valid_golden_task_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
}

fn validate_rollback_profile(value: &Value) -> Result<(), String> {
    let evidence: RollbackProfileEvidence = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid rollback-profile evidence: {error}"))?;
    let hashes = [
        &evidence.profile_sha256,
        &evidence.session_config_sha256,
        &evidence.server_sha256,
        &evidence.smoke_prompt_sha256,
        &evidence.smoke_token_sha256,
    ];
    if evidence.schema != 1
        || evidence.profile != "stable-16k"
        || evidence.runtime.trim().is_empty()
        || !evidence.profile_path.is_absolute()
        || !evidence.session_config_path.is_absolute()
        || !evidence.server_path.is_absolute()
        || evidence.context_tokens != 16_384
        || !valid_process(&evidence.process)
        || hashes.into_iter().any(|hash| !is_sha256(hash))
        || !is_sha256(&evidence.runtime_build_sha256)
        || evidence.smoke_tokens.len() < 16
        || sha256_bytes(
            &serde_json::to_vec(&evidence.smoke_tokens)
                .map_err(|error| format!("failed to encode rollback tokens: {error}"))?,
        ) != evidence.smoke_token_sha256
        || !evidence.restored_prior_session
    {
        return Err("rollback-profile evidence failed its raw availability contract".to_owned());
    }
    Ok(())
}

fn valid_process(process: &ProcessEvidence) -> bool {
    process.pid != 0
        && process.process_start_epoch_secs != 0
        && is_session_identity(&process.session_identity)
}

fn complete_identity(run: &RunEvidence) -> Result<EvidenceIdentity, String> {
    let identity = EvidenceIdentity {
        hardware: run
            .identity
            .hardware
            .clone()
            .ok_or("missing hardware identity")?,
        software: run
            .identity
            .software
            .clone()
            .ok_or("missing software identity")?,
        model: run.identity.model.clone().ok_or("missing model identity")?,
        runtime: run
            .identity
            .runtime
            .clone()
            .ok_or("missing runtime identity")?,
        workload: run
            .identity
            .workload
            .clone()
            .ok_or("missing workload identity")?,
        configuration: run
            .identity
            .configuration
            .clone()
            .ok_or("missing configuration identity")?,
        policy: run
            .identity
            .policy
            .clone()
            .ok_or("missing policy identity")?,
    };
    let complete = [
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
    complete
        .then_some(identity)
        .ok_or_else(|| "anchor identity is malformed".to_owned())
}

pub(crate) fn parse_kind(value: &str) -> Result<ExternalEvidenceKind, String> {
    [
        ExternalEvidenceKind::SameProcess50RequestGreedyStability,
        ExternalEvidenceKind::TenCleanRestartGreedyStability,
        ExternalEvidenceKind::NearLimitContextStress,
        ExternalEvidenceKind::GoldenAgentTaskPass,
        ExternalEvidenceKind::OperatorReviewedCapabilityReport,
        ExternalEvidenceKind::RollbackProfileAvailable,
    ]
    .into_iter()
    .find(|kind| kind.as_str() == value)
    .ok_or_else(|| format!("unsupported external evidence kind: {value}"))
}

fn status(
    name: &str,
    kind: ExternalEvidenceStatusKind,
    path: Option<PathBuf>,
    reason: Option<String>,
) -> ExternalEvidenceStatus {
    ExternalEvidenceStatus {
        name: name.to_owned(),
        status: kind,
        path,
        sha256: None,
        reason,
    }
}

fn walk_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect evidence path {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "evidence path must not be a symbolic link: {}",
            path.display()
        ));
    }
    if metadata.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !metadata.is_dir() {
        return Err(format!(
            "evidence path is neither a file nor a directory: {}",
            path.display()
        ));
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path).map_err(|error| {
        format!(
            "failed to enumerate evidence path {}: {error}",
            path.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate evidence path {}: {error}",
                path.display()
            )
        })?;
        files.extend(walk_files(&entry.path())?);
    }
    Ok(files)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_session_identity(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        sequence: u32,
        role: StabilityRequestRole,
        prompt_sha256: String,
        tokens: Vec<u32>,
    ) -> StabilityRequestEvidence {
        let bytes = serde_json::to_vec(&tokens).unwrap();
        StabilityRequestEvidence {
            sequence,
            role,
            prompt_sha256,
            token_sha256: sha256_bytes(&bytes),
            tokens,
        }
    }

    fn same_process() -> SameProcessStabilityEvidence {
        let target_prompt_sha256 = "b".repeat(64);
        let process = ProcessEvidence {
            pid: 42,
            process_start_epoch_secs: 123,
            session_identity: "a".repeat(32),
        };
        let mut requests = Vec::new();
        for iteration in 1..=50_u32 {
            requests.push(request(
                requests.len() as u32 + 1,
                StabilityRequestRole::Contaminant,
                sha256_bytes(format!("contaminant-{iteration}").as_bytes()),
                vec![iteration; 16],
            ));
            requests.push(request(
                requests.len() as u32 + 1,
                StabilityRequestRole::Target,
                target_prompt_sha256.clone(),
                vec![7; 128],
            ));
        }
        SameProcessStabilityEvidence {
            schema: 1,
            profile: "turbo-16k".to_owned(),
            process_before: process.clone(),
            process_after: process,
            request_contract: StabilityRequestContract {
                target_prompt_sha256,
                target_n_predict: 128,
                contaminant_n_predict: 16,
                temperature: 0.0,
                top_k: 1,
                seed: 42,
                ignore_eos: true,
                cache_prompt: false,
                return_tokens: true,
            },
            requests,
            restored_prior_session: true,
        }
    }

    fn clean_restarts() -> CleanRestartStabilityEvidence {
        let target_prompt_sha256 = "b".repeat(64);
        let tokens = vec![7; 128];
        let token_sha256 = sha256_bytes(&serde_json::to_vec(&tokens).unwrap());
        CleanRestartStabilityEvidence {
            schema: 1,
            profile: "turbo-16k".to_owned(),
            request_contract: StabilityRequestContract {
                target_prompt_sha256: target_prompt_sha256.clone(),
                target_n_predict: 128,
                contaminant_n_predict: 0,
                temperature: 0.0,
                top_k: 1,
                seed: 42,
                ignore_eos: true,
                cache_prompt: false,
                return_tokens: true,
            },
            restarts: (1..=10_u32)
                .map(|sequence| RestartRequestEvidence {
                    sequence,
                    process: ProcessEvidence {
                        pid: 100 + sequence,
                        process_start_epoch_secs: 1000 + u64::from(sequence),
                        session_identity: format!("{sequence:032x}"),
                    },
                    prompt_sha256: target_prompt_sha256.clone(),
                    token_sha256: token_sha256.clone(),
                    tokens: tokens.clone(),
                })
                .collect(),
            restored_prior_session: true,
        }
    }

    fn near_limit_context() -> NearLimitContextEvidence {
        let process = ProcessEvidence {
            pid: 42,
            process_start_epoch_secs: 123,
            session_identity: "a".repeat(32),
        };
        let content = "CEDAR-48291|ORBIT-73064|VIOLET-19538".to_owned();
        let tokens = vec![1, 2, 3, 4];
        let run = |sequence| ContextRunEvidence {
            sequence,
            content_sha256: sha256_bytes(content.as_bytes()),
            token_sha256: sha256_bytes(&serde_json::to_vec(&tokens).unwrap()),
            content: content.clone(),
            tokens: tokens.clone(),
        };
        NearLimitContextEvidence {
            schema: 1,
            profile: "turbo-16k".to_owned(),
            generator: "immutable-ledger-v1".to_owned(),
            context_tokens: 16_384,
            ratio: 0.85,
            target_prompt_tokens: 13_926,
            actual_prompt_tokens: 13_920,
            prompt_sha256: "b".repeat(64),
            needles: vec![
                "CEDAR-48291".to_owned(),
                "ORBIT-73064".to_owned(),
                "VIOLET-19538".to_owned(),
            ],
            process_before: process.clone(),
            process_after: process,
            runs: vec![run(1), run(2)],
            restored_prior_session: true,
        }
    }

    fn golden_agent() -> GoldenAgentEvidence {
        let protected = BTreeMap::from([("tests/test_ranges.py".to_owned(), "a".repeat(64))]);
        GoldenAgentEvidence {
            schema: 3,
            task_id: "python-off-by-one".to_owned(),
            capabilities: vec![crate::golden::GoldenCapability::SingleFileRepair],
            tool_calls: 2,
            tool_failures: 0,
            required_failure_matches: 0,
            suite_sha256: "b".repeat(64),
            opencode_path: PathBuf::from(r"C:\fixture\opencode.cmd"),
            opencode_sha256: "c".repeat(64),
            harness_policy_sha256: "3".repeat(64),
            effective_config_sha256: "d".repeat(64),
            test_executable_path: PathBuf::from(r"C:\fixture\python.exe"),
            test_executable_sha256: "4".repeat(64),
            agent_exit_code: 0,
            tests_exit_code: 0,
            protected_before: protected.clone(),
            protected_after: protected,
            unexpected_files: Vec::new(),
            agent_stdout_sha256: "e".repeat(64),
            agent_stderr_sha256: "f".repeat(64),
            tests_stdout_sha256: "1".repeat(64),
            tests_stderr_sha256: "2".repeat(64),
            restored_prior_session: true,
        }
    }

    fn rollback_profile() -> RollbackProfileEvidence {
        let tokens = vec![1; 16];
        RollbackProfileEvidence {
            schema: 1,
            profile: "stable-16k".to_owned(),
            profile_path: PathBuf::from(r"C:\fixture\profiles\stable-16k.json"),
            profile_sha256: "a".repeat(64),
            session_config_path: PathBuf::from(r"C:\fixture\config\session.json"),
            session_config_sha256: "b".repeat(64),
            runtime: "official".to_owned(),
            server_path: PathBuf::from(r"C:\fixture\runtime\llama-server.exe"),
            server_sha256: "c".repeat(64),
            runtime_build_sha256: "d".repeat(64),
            context_tokens: 16_384,
            process: ProcessEvidence {
                pid: 42,
                process_start_epoch_secs: 123,
                session_identity: "e".repeat(32),
            },
            smoke_prompt_sha256: "f".repeat(64),
            smoke_token_sha256: sha256_bytes(&serde_json::to_vec(&tokens).unwrap()),
            smoke_tokens: tokens,
            restored_prior_session: true,
        }
    }

    fn capability_review() -> CapabilityReviewEvidence {
        CapabilityReviewEvidence {
            schema: 1,
            reviewer_role: "project-owner-and-daily-operator".to_owned(),
            scenarios: CapabilityCategory::REQUIRED
                .into_iter()
                .enumerate()
                .map(|(offset, category)| CapabilityScenario {
                    id: format!("scenario-{}", offset + 1),
                    category,
                    task: format!("Realistic task for category {}", offset + 1),
                    expected_capability: "Complete the task usefully and safely".to_owned(),
                    observed_behavior: "The complete production workflow behaved as expected"
                        .to_owned(),
                    outcome: CapabilityOutcome::Pass,
                    limitations: Vec::new(),
                    disposition: None,
                    disposition_rationale: None,
                    accepted_risk_ids: Vec::new(),
                    supporting_artifact_sha256: None,
                })
                .collect(),
            accepted_residual_risks: Vec::new(),
            final_decision: CapabilityReviewDecision::Approved,
            final_rationale: "All required categories were exercised with no blocking findings"
                .to_owned(),
        }
    }

    #[test]
    fn same_process_gate_recomputes_raw_token_and_sequence_evidence() {
        let valid = serde_json::to_value(same_process()).unwrap();
        assert!(validate_same_process(&valid).is_ok());

        let mut tampered = same_process();
        tampered.requests[1].tokens[0] = 8;
        assert!(validate_same_process(&serde_json::to_value(tampered).unwrap()).is_err());

        let mut reordered = same_process();
        reordered.requests.swap(0, 1);
        assert!(validate_same_process(&serde_json::to_value(reordered).unwrap()).is_err());

        let mut changed_process = same_process();
        changed_process.process_after.pid += 1;
        assert!(validate_same_process(&serde_json::to_value(changed_process).unwrap()).is_err());
    }

    #[test]
    fn restart_gate_recomputes_process_and_token_evidence() {
        let valid = serde_json::to_value(clean_restarts()).unwrap();
        assert!(validate_clean_restarts(&valid).is_ok());

        let mut reused = clean_restarts();
        reused.restarts[1].process = reused.restarts[0].process.clone();
        assert!(validate_clean_restarts(&serde_json::to_value(reused).unwrap()).is_err());

        let mut divergent = clean_restarts();
        divergent.restarts[1].tokens[0] = 8;
        divergent.restarts[1].token_sha256 =
            sha256_bytes(&serde_json::to_vec(&divergent.restarts[1].tokens).unwrap());
        assert!(validate_clean_restarts(&serde_json::to_value(divergent).unwrap()).is_err());
    }

    #[test]
    fn context_gate_recomputes_distance_and_raw_retrieval() {
        let valid = serde_json::to_value(near_limit_context()).unwrap();
        assert!(validate_near_limit_context(&valid).is_ok());

        let mut shallow = near_limit_context();
        shallow.actual_prompt_tokens = 100;
        assert!(validate_near_limit_context(&serde_json::to_value(shallow).unwrap()).is_err());

        let mut wrong = near_limit_context();
        wrong.runs[1].content = "wrong".to_owned();
        wrong.runs[1].content_sha256 = sha256_bytes(b"wrong");
        assert!(validate_near_limit_context(&serde_json::to_value(wrong).unwrap()).is_err());
    }

    #[test]
    fn golden_gate_recomputes_exit_protection_and_workspace_contract() {
        let valid = serde_json::to_value(golden_agent()).unwrap();
        assert!(validate_golden_agent(&valid).is_ok());

        let mut changed_test = golden_agent();
        changed_test
            .protected_after
            .insert("tests/test_ranges.py".to_owned(), "9".repeat(64));
        assert!(validate_golden_agent(&serde_json::to_value(changed_test).unwrap()).is_err());

        let mut unexpected = golden_agent();
        unexpected
            .unexpected_files
            .push("unrequested.txt".to_owned());
        assert!(validate_golden_agent(&serde_json::to_value(unexpected).unwrap()).is_err());

        let mut traversal = golden_agent();
        traversal.task_id = "../outside".to_owned();
        assert!(validate_golden_agent(&serde_json::to_value(traversal).unwrap()).is_err());

        let mut impossible_failure_count = golden_agent();
        impossible_failure_count.required_failure_matches = 1;
        assert!(
            validate_golden_agent(&serde_json::to_value(impossible_failure_count).unwrap())
                .is_err()
        );

        let requirement = GoldenEvidenceRequirement {
            task_id: "public-v1".to_owned(),
            required_capabilities: vec![
                crate::golden::GoldenCapability::MultiFileTddRepair,
                crate::golden::GoldenCapability::ToolErrorRecovery,
                crate::golden::GoldenCapability::ConstraintRetention,
            ],
        };
        assert!(validate_golden_requirement(&golden_agent(), &requirement).is_err());

        let mut stale_runner = golden_agent();
        stale_runner.test_executable_sha256 = "9".repeat(64);
        assert!(validate_golden_agent(&serde_json::to_value(stale_runner).unwrap()).is_ok());
    }

    #[test]
    fn golden_test_executable_identity_detects_tampering() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("python-fixture.exe");
        std::fs::write(&executable, b"first identity").unwrap();
        let digest = sha256_file(&executable).unwrap();
        validate_executable_identity(&executable, &executable, &digest, "test").unwrap();

        std::fs::write(&executable, b"tampered identity").unwrap();
        assert!(validate_executable_identity(&executable, &executable, &digest, "test").is_err());
    }

    #[test]
    fn rollback_gate_recomputes_smoke_and_restoration_contract() {
        let valid = serde_json::to_value(rollback_profile()).unwrap();
        assert!(validate_rollback_profile(&valid).is_ok());

        let mut short = rollback_profile();
        short.smoke_tokens.truncate(1);
        short.smoke_token_sha256 = sha256_bytes(&serde_json::to_vec(&short.smoke_tokens).unwrap());
        assert!(validate_rollback_profile(&serde_json::to_value(short).unwrap()).is_err());

        let mut not_restored = rollback_profile();
        not_restored.restored_prior_session = false;
        assert!(validate_rollback_profile(&serde_json::to_value(not_restored).unwrap()).is_err());
    }

    #[test]
    fn capability_review_requires_complete_human_evidence_without_deciding_risk() {
        let valid = capability_review();
        assert!(validate_capability_review(&serde_json::to_value(&valid).unwrap()).is_ok());
        assert_eq!(
            evidence_decision(
                ExternalEvidenceKind::OperatorReviewedCapabilityReport,
                &serde_json::to_value(&valid).unwrap()
            )
            .unwrap(),
            EvidenceDecision::Pass
        );

        let mut missing = capability_review();
        missing.scenarios.pop();
        assert!(validate_capability_review(&serde_json::to_value(missing).unwrap()).is_err());

        let mut incomplete = capability_review();
        incomplete.scenarios[0].outcome = CapabilityOutcome::Inconclusive;
        assert!(validate_capability_review(&serde_json::to_value(incomplete).unwrap()).is_err());

        let mut accepted = capability_review();
        accepted.accepted_residual_risks = vec![AcceptedResidualRisk {
            id: "slow-first-response".to_owned(),
            description: "The first response after loading can be slow".to_owned(),
            rationale: "The delay is visible, bounded, and acceptable for local operation"
                .to_owned(),
        }];
        accepted.scenarios[0].outcome = CapabilityOutcome::Inconclusive;
        accepted.scenarios[0].disposition = Some(CapabilityDisposition::AcceptedRisk);
        accepted.scenarios[0].disposition_rationale = Some(
            "The workflow remained useful and the bounded delay is explicitly accepted".to_owned(),
        );
        accepted.scenarios[0].accepted_risk_ids = vec!["slow-first-response".to_owned()];
        assert!(validate_capability_review(&serde_json::to_value(accepted).unwrap()).is_ok());

        let mut blocking = capability_review();
        blocking.scenarios[0].outcome = CapabilityOutcome::Fail;
        blocking.scenarios[0].disposition = Some(CapabilityDisposition::Blocking);
        blocking.scenarios[0].disposition_rationale =
            Some("The workflow changed an unrelated file".to_owned());
        assert!(validate_capability_review(&serde_json::to_value(blocking).unwrap()).is_err());
    }

    #[test]
    fn public_capability_projection_excludes_private_review_content_and_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capability.json");
        let mut review = capability_review();
        review.scenarios[0].task = "PRIVATE TASK CONTENT".to_owned();
        review.scenarios[0].observed_behavior = "PRIVATE OBSERVATION CONTENT".to_owned();
        review.scenarios[0].supporting_artifact_sha256 = Some("9".repeat(64));
        let identity = EvidenceIdentity {
            hardware: "a".repeat(64),
            software: "b".repeat(64),
            model: "c".repeat(64),
            runtime: "d".repeat(64),
            workload: "e".repeat(64),
            configuration: "f".repeat(64),
            policy: "1".repeat(64),
        };
        let payload = ExternalEvidence {
            schema: 1,
            kind: ExternalEvidenceKind::OperatorReviewedCapabilityReport,
            decision: EvidenceDecision::Pass,
            anchor_run_id: "final-run".to_owned(),
            identity: identity.clone(),
            producer_sha256: identity.software.clone(),
            created_at: "2026-08-20T00:00:00Z".to_owned(),
            evidence: serde_json::to_value(review).unwrap(),
            reviewed_by: Some("PRIVATE REVIEWER LABEL".to_owned()),
        };
        let mut bytes = serde_json::to_vec_pretty(&payload).unwrap();
        bytes.push(b'\n');
        std::fs::write(&path, &bytes).unwrap();
        let facts =
            public_capability_review_facts(&path, &sha256_bytes(&bytes), "final-run", &identity)
                .unwrap();
        let rendered = serde_json::to_string(&facts).unwrap();
        assert!(!rendered.contains("PRIVATE TASK CONTENT"));
        assert!(!rendered.contains("PRIVATE OBSERVATION CONTENT"));
        assert!(!rendered.contains("PRIVATE REVIEWER LABEL"));
        assert!(rendered.contains("project-owner-and-daily-operator"));
        assert!(rendered.contains(&"9".repeat(64)));
    }
}
