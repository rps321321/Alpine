use crate::clock::UtcTimestamp;
use crate::evidence::{EvidenceStore, EvidenceWriter, RunEvidence};
use crate::identity::{sha256_bytes, sha256_file, tree_sha256};
use crate::qualification::EvidenceIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
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
    let payload = ExternalEvidence {
        schema: 1,
        kind: options.kind,
        decision: EvidenceDecision::Pass,
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

pub(crate) fn inspect_required(
    store: &EvidenceStore,
    anchor_run_id: &str,
    identity: &EvidenceIdentity,
    required: &[String],
    repository_root: &Path,
    result_root: &Path,
    producer_sha256: &str,
) -> Result<Vec<ExternalEvidenceStatus>, String> {
    let artifacts = store.artifacts(anchor_run_id)?;
    let mut by_kind: BTreeMap<&str, Vec<_>> = BTreeMap::new();
    for artifact in &artifacts {
        by_kind.entry(&artifact.kind).or_default().push(artifact);
    }
    let mut statuses = Vec::with_capacity(required.len());
    for name in required {
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
            repository_root,
            producer_sha256,
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
    repository_root: &Path,
    producer_sha256: &str,
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
    validate_current(&payload, kind, repository_root, producer_sha256)
        .map_err(|reason| (ExternalEvidenceStatusKind::Stale, reason))?;
    validate_semantics(&payload, kind)
        .map_err(|reason| (ExternalEvidenceStatusKind::Invalid, reason))?;
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
        || payload.decision != EvidenceDecision::Pass
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
) -> Result<(), String> {
    if payload.producer_sha256 != producer_sha256 {
        return Err("artifact was produced by a stale Alpine binary".to_owned());
    }
    if kind == ExternalEvidenceKind::GoldenAgentTaskPass {
        let task_id = string(&payload.evidence, "/task_id")?;
        let suite = string(&payload.evidence, "/suite_sha256")?;
        let task_root = repository_root.join("benchmarks/golden").join(task_id);
        let files = std::fs::read_dir(&task_root)
            .map_err(|error| format!("golden task is unavailable: {error}"))?
            .filter_map(Result::ok)
            .flat_map(|entry| walk_files(entry.path()))
            .filter(|path| {
                !path
                    .components()
                    .any(|part| part.as_os_str() == "__pycache__")
            })
            .collect::<Vec<_>>();
        if tree_sha256(&task_root, &files)? != suite {
            return Err("golden task suite identity is stale".to_owned());
        }
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
            require_u64(&payload.evidence, "/clean_restarts", 10)?;
            require_u64(&payload.evidence, "/unique_output_hashes", 1)?;
            require_u64(&payload.evidence, "/unique_process_identities", 10)?;
            require_true(&payload.evidence, "/restored_prior_session")?;
        }
        ExternalEvidenceKind::NearLimitContextStress => {
            let ratio = number(&payload.evidence, "/ratio")?;
            if !(0.85..=0.95).contains(&ratio) {
                return Err("context evidence ratio must be between 0.85 and 0.95".to_owned());
            }
            require_u64(&payload.evidence, "/runs", 2)?;
            require_true(&payload.evidence, "/all_quality_pass")?;
            let context = integer(&payload.evidence, "/context_tokens")?;
            let prompt = integer(&payload.evidence, "/minimum_prompt_tokens")?;
            if prompt < (context as f64 * ratio * 0.98) as u64 {
                return Err(
                    "context evidence did not reach its claimed near-limit target".to_owned(),
                );
            }
        }
        ExternalEvidenceKind::GoldenAgentTaskPass => {
            require_true(&payload.evidence, "/success")?;
            require_true(&payload.evidence, "/protected_paths_unchanged")?;
            require_u64_exact(&payload.evidence, "/agent_exit_code", 0)?;
            require_u64_exact(&payload.evidence, "/tests_exit_code", 0)?;
            string(&payload.evidence, "/task_id")?;
            string(&payload.evidence, "/suite_sha256")?;
        }
        ExternalEvidenceKind::OperatorReviewedCapabilityReport => {
            if payload
                .reviewed_by
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err("operator capability evidence has no explicit reviewer".to_owned());
            }
            require_true(&payload.evidence, "/capability_review_passed")?;
        }
        ExternalEvidenceKind::RollbackProfileAvailable => {
            if string(&payload.evidence, "/profile")? != "stable-16k" {
                return Err("rollback evidence must prove stable-16k".to_owned());
            }
            require_true(&payload.evidence, "/transaction_verified")?;
            require_true(&payload.evidence, "/restored_prior_session")?;
        }
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

fn require_true(value: &Value, pointer: &str) -> Result<(), String> {
    if value.pointer(pointer).and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(format!("evidence field {pointer} must be true"))
    }
}

fn require_u64(value: &Value, pointer: &str, minimum: u64) -> Result<(), String> {
    let observed = integer(value, pointer)?;
    if observed >= minimum {
        Ok(())
    } else {
        Err(format!(
            "evidence field {pointer} is {observed}; minimum is {minimum}"
        ))
    }
}

fn require_u64_exact(value: &Value, pointer: &str, expected: u64) -> Result<(), String> {
    let observed = integer(value, pointer)?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "evidence field {pointer} is {observed}; expected {expected}"
        ))
    }
}

fn integer(value: &Value, pointer: &str) -> Result<u64, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("evidence field {pointer} must be a non-negative integer"))
}

fn number(value: &Value, pointer: &str) -> Result<f64, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| format!("evidence field {pointer} must be finite"))
}

fn string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("evidence field {pointer} must be a non-empty string"))
}

fn walk_files(path: PathBuf) -> Vec<PathBuf> {
    if path.is_file() {
        return vec![path];
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .flat_map(|entry| walk_files(entry.path()))
        .collect()
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
}
