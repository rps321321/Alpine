use crate::config;
use crate::hardware;
use crate::identity::{runtime_bundle_sha256, sha256_bytes, sha256_file};
use crate::opencode::{command_compatible_path, sanitized_environment};
use crate::process::{resolve_executable, run_command_bounded};
use crate::qualification::EvidenceIdentity;
use crate::session::{
    self, AcquireSessionOptions, ReleaseSessionOptions, SessionAcquisition, SessionSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const REQUIRED_CANDIDATES: [&str; 4] = [
    "opencode-process",
    "pi-sdk-core",
    "pi-process-rpc",
    "cline-agents",
];

struct ReviewedSourcePin {
    repository: &'static str,
    version: &'static str,
    commit: &'static str,
    package: &'static str,
    package_integrity: &'static str,
    license: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentEngineBakeoffPlanSummary {
    pub schema: u32,
    pub plan_id: String,
    pub candidate_ids: Vec<String>,
    pub required_scenarios: Vec<String>,
    pub request_budget: u32,
    pub max_event_queue: u32,
    pub recommendation: String,
}

#[derive(Debug, Clone)]
pub struct AgentEngineBakeoffOptions {
    pub plan: PathBuf,
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub candidate_root: PathBuf,
    pub profile: Option<String>,
    pub lock_timeout: Duration,
    pub startup_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentEngineBakeoffReport {
    pub schema: u32,
    pub plan_id: String,
    pub plan_sha256: String,
    pub identity: EvidenceIdentity,
    pub evidence_complete: bool,
    pub all_scenarios_demonstrated: bool,
    pub all_prior_sessions_restored: bool,
    pub recommendation: String,
    pub candidates: Vec<AgentEngineCandidateReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentEngineCandidateReport {
    pub id: String,
    pub source_version: String,
    pub source_commit: String,
    pub adapter_executable_sha256: String,
    pub budget: AgentEngineBudgetObservation,
    pub restored_prior_session: bool,
    pub restorations_verified: u32,
    pub candidate_state_disposed: bool,
    pub scenarios: Vec<AgentEngineScenarioReport>,
    pub events: Vec<AgentEngineEvent>,
    pub errors: Vec<AgentEngineError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentEngineBudgetObservation {
    pub requests_used: u32,
    pub max_wall_ms_observed: u64,
    pub max_input_tokens_observed: u32,
    pub max_output_tokens_observed: u32,
    pub retries_used: u32,
    pub worker_restarts_used: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentEngineScenarioReport {
    pub scenario: AgentEngineScenario,
    pub outcome: AgentEngineScenarioOutcome,
    pub requests_used: u32,
    pub wall_ms: u64,
    pub session_restored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentEngineScenario {
    Streaming,
    Tools,
    Steering,
    FollowUp,
    Cancellation,
    Retry,
    Compaction,
    Backpressure,
    WorkerRestart,
    ContinuationRecovery,
    NormalizedErrors,
}

impl AgentEngineScenario {
    const ALL: [Self; 11] = [
        Self::Streaming,
        Self::Tools,
        Self::Steering,
        Self::FollowUp,
        Self::Cancellation,
        Self::Retry,
        Self::Compaction,
        Self::Backpressure,
        Self::WorkerRestart,
        Self::ContinuationRecovery,
        Self::NormalizedErrors,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Streaming => "streaming",
            Self::Tools => "tools",
            Self::Steering => "steering",
            Self::FollowUp => "follow-up",
            Self::Cancellation => "cancellation",
            Self::Retry => "retry",
            Self::Compaction => "compaction",
            Self::Backpressure => "backpressure",
            Self::WorkerRestart => "worker-restart",
            Self::ContinuationRecovery => "continuation-recovery",
            Self::NormalizedErrors => "normalized-errors",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentEngineScenarioOutcome {
    Demonstrated,
    ExplicitFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEngineEvent {
    pub sequence: u64,
    pub scenario: AgentEngineScenario,
    #[serde(rename = "event")]
    pub kind: AgentEngineEventKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AgentEngineEventKind {
    StreamStarted,
    StreamDelta {
        bytes: u32,
    },
    StreamFinished,
    ToolStarted {
        call_id: String,
        tool: String,
    },
    ToolFinished {
        call_id: String,
        tool: String,
        succeeded: bool,
    },
    SteeringAccepted,
    FollowUpAccepted,
    CancellationRequested,
    CancellationConfirmed,
    RetryScheduled {
        attempt: u32,
    },
    RetryCompleted {
        attempt: u32,
    },
    CompactionStarted {
        tokens_before: u32,
    },
    CompactionFinished {
        tokens_after: u32,
    },
    BackpressureApplied {
        queue_capacity: u32,
    },
    WorkerExited {
        exit_code: i32,
    },
    WorkerRestarted {
        restart: u32,
    },
    ContinuationRestored {
        cursor_sha256: String,
    },
    ErrorNormalized {
        kind: AgentEngineErrorKind,
        code: String,
    },
}

impl AgentEngineEventKind {
    fn scenario(&self) -> AgentEngineScenario {
        match self {
            Self::StreamStarted | Self::StreamDelta { .. } | Self::StreamFinished => {
                AgentEngineScenario::Streaming
            }
            Self::ToolStarted { .. } | Self::ToolFinished { .. } => AgentEngineScenario::Tools,
            Self::SteeringAccepted => AgentEngineScenario::Steering,
            Self::FollowUpAccepted => AgentEngineScenario::FollowUp,
            Self::CancellationRequested | Self::CancellationConfirmed => {
                AgentEngineScenario::Cancellation
            }
            Self::RetryScheduled { .. } | Self::RetryCompleted { .. } => AgentEngineScenario::Retry,
            Self::CompactionStarted { .. } | Self::CompactionFinished { .. } => {
                AgentEngineScenario::Compaction
            }
            Self::BackpressureApplied { .. } => AgentEngineScenario::Backpressure,
            Self::WorkerExited { .. } | Self::WorkerRestarted { .. } => {
                AgentEngineScenario::WorkerRestart
            }
            Self::ContinuationRestored { .. } => AgentEngineScenario::ContinuationRecovery,
            Self::ErrorNormalized { .. } => AgentEngineScenario::NormalizedErrors,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentEngineErrorKind {
    AdapterUnavailable,
    UnsupportedCapability,
    ProtocolViolation,
    BudgetExceeded,
    Cancelled,
    WorkerExit,
    RetryExhausted,
    CompactionFailed,
    ContinuationFailed,
    SessionRestoreFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEngineError {
    pub scenario: AgentEngineScenario,
    pub kind: AgentEngineErrorKind,
    pub code: String,
    pub retryable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentEngineBakeoffPlan {
    schema: u32,
    id: String,
    evidence_identity: EvidenceIdentityContract,
    inputs: BakeoffInputs,
    budget: BakeoffBudget,
    required_scenarios: Vec<AgentEngineScenario>,
    supporting_packages: Vec<CandidateSource>,
    candidates: Vec<CandidateReview>,
    recommendation: Recommendation,
    privacy: PrivacyContract,
}

#[derive(Debug)]
struct AgentEngineEvidence {
    schema: u32,
    plan_id: String,
    plan_sha256: String,
    identity: StrictEvidenceIdentity,
    candidates: Vec<CandidateEvidence>,
}

#[derive(Debug)]
struct CandidateEvidence {
    id: String,
    source_version: String,
    source_commit: String,
    adapter_executable_sha256: String,
    identity: StrictEvidenceIdentity,
    requests_used: u32,
    max_wall_ms_observed: u64,
    max_input_tokens_observed: u32,
    max_output_tokens_observed: u32,
    retries_used: u32,
    worker_restarts_used: u32,
    candidate_state_disposed: bool,
    restorations_verified: u32,
    prior_session: StrictSessionSnapshot,
    restored_session: StrictSessionSnapshot,
    scenarios: Vec<ScenarioEvidence>,
    events: Vec<AgentEngineEvent>,
    errors: Vec<AgentEngineError>,
}

#[derive(Debug, Clone)]
struct ScenarioEvidence {
    scenario: AgentEngineScenario,
    requests_used: u32,
    wall_ms: u64,
    session_restored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StrictEvidenceIdentity {
    hardware: String,
    software: String,
    model: String,
    runtime: String,
    workload: String,
    configuration: String,
    policy: String,
}

impl From<StrictEvidenceIdentity> for EvidenceIdentity {
    fn from(identity: StrictEvidenceIdentity) -> Self {
        Self {
            hardware: identity.hardware,
            software: identity.software,
            model: identity.model,
            runtime: identity.runtime,
            workload: identity.workload,
            configuration: identity.configuration,
            policy: identity.policy,
        }
    }
}

impl From<EvidenceIdentity> for StrictEvidenceIdentity {
    fn from(identity: EvidenceIdentity) -> Self {
        Self {
            hardware: identity.hardware,
            software: identity.software,
            model: identity.model,
            runtime: identity.runtime,
            workload: identity.workload,
            configuration: identity.configuration,
            policy: identity.policy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StrictSessionSnapshot {
    active: bool,
    healthy: bool,
    profile: String,
    vision: bool,
    runtime: String,
    fallback: Option<String>,
    arguments: Vec<String>,
    environment: BTreeMap<String, Option<String>>,
    session_identity: Option<String>,
}

impl From<StrictSessionSnapshot> for SessionSnapshot {
    fn from(snapshot: StrictSessionSnapshot) -> Self {
        Self {
            active: snapshot.active,
            healthy: snapshot.healthy,
            profile: snapshot.profile,
            vision: snapshot.vision,
            runtime: snapshot.runtime,
            fallback: snapshot.fallback,
            arguments: snapshot.arguments,
            environment: snapshot.environment,
            session_identity: snapshot.session_identity,
        }
    }
}

impl From<SessionSnapshot> for StrictSessionSnapshot {
    fn from(snapshot: SessionSnapshot) -> Self {
        Self {
            active: snapshot.active,
            healthy: snapshot.healthy,
            profile: snapshot.profile,
            vision: snapshot.vision,
            runtime: snapshot.runtime,
            fallback: snapshot.fallback,
            arguments: snapshot.arguments,
            environment: snapshot.environment,
            session_identity: snapshot.session_identity,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceIdentityContract {
    required_fields: Vec<String>,
    identical_material_inputs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BakeoffInputs {
    profile: String,
    model_id: String,
    temperature: u32,
    fixture: String,
    prompt_tool_policy: String,
    template_source: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicFixture {
    schema: u32,
    id: String,
    system: String,
    target_file: String,
    target_text: String,
    prompts: BTreeMap<String, String>,
    tool_policy: FixtureToolPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureToolPolicy {
    allowed: Vec<String>,
    denied: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterReceipt {
    schema: u32,
    candidate: String,
    scenario: AgentEngineScenario,
    requests_used: u32,
    max_input_tokens_observed: u32,
    max_output_tokens_observed: u32,
    retries_used: u32,
    worker_restarts_used: u32,
    events: Vec<AgentEngineEventKind>,
    errors: Vec<AdapterError>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterError {
    kind: AgentEngineErrorKind,
    code: String,
    retryable: bool,
}

struct CandidateRun {
    adapter_executable_sha256: String,
    requests_used: u32,
    max_wall_ms_observed: u64,
    max_input_tokens_observed: u32,
    max_output_tokens_observed: u32,
    retries_used: u32,
    worker_restarts_used: u32,
    candidate_state_disposed: bool,
    restorations_verified: u32,
    scenarios: Vec<ScenarioEvidence>,
    events: Vec<AgentEngineEvent>,
    errors: Vec<AgentEngineError>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BakeoffBudget {
    requests: u32,
    request_timeout_ms: u64,
    max_input_tokens: u32,
    max_output_tokens: u32,
    max_retries: u32,
    max_worker_restarts: u32,
    max_event_queue: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateReview {
    id: String,
    surface: String,
    source: CandidateSource,
    dependencies: DependencyReview,
    maintenance: MaintenanceReview,
    security: SecurityReview,
    adapter_boundary: String,
    smallest_missing_upstream_hook: String,
    decision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateSource {
    repository: String,
    version: String,
    commit: String,
    package: String,
    package_integrity: String,
    license: String,
    reviewed_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyReview {
    direct_runtime_dependencies: u32,
    locked_package_count: Option<u32>,
    unpacked_size_bytes: Option<u64>,
    runtime_requirement: String,
    packaging_cost: String,
    update_cost: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaintenanceReview {
    repository_archived: bool,
    source_commit_at: String,
    signals: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecurityReview {
    built_in_sandbox: bool,
    authority: String,
    credential_boundary: String,
    known_risks: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recommendation {
    decision: String,
    rationale_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivacyContract {
    publish_raw_prompts: bool,
    publish_credentials: bool,
    publish_machine_paths: bool,
    publish_raw_evidence: bool,
    publish_model_artifacts: bool,
}

pub(crate) fn inspect_plan(path: &Path) -> Result<AgentEngineBakeoffPlanSummary, String> {
    let (_, plan) = read_plan(path)?;

    Ok(AgentEngineBakeoffPlanSummary {
        schema: plan.schema,
        plan_id: plan.id,
        candidate_ids: plan
            .candidates
            .into_iter()
            .map(|candidate| candidate.id)
            .collect(),
        required_scenarios: plan
            .required_scenarios
            .into_iter()
            .map(|scenario| scenario.as_str().to_owned())
            .collect(),
        request_budget: plan.budget.requests,
        max_event_queue: plan.budget.max_event_queue,
        recommendation: "no-go".to_owned(),
    })
}

pub(crate) fn run(options: &AgentEngineBakeoffOptions) -> Result<AgentEngineBakeoffReport, String> {
    let (plan_sha256, plan) = read_plan(&options.plan)?;
    let repository_root = canonical_directory(&options.repository_root, "repository root")?;
    let candidate_root = canonical_directory(&options.candidate_root, "candidate root")?;
    if options
        .profile
        .as_deref()
        .is_some_and(|profile| profile != plan.inputs.profile)
    {
        return Err("agent-engine bake-off Profile must match the pinned plan input".to_owned());
    }
    let resolved = config::resolve(&options.install_root, Some(&plan.inputs.profile), true)?;
    let fixture = canonical_repository_file(
        &repository_root,
        Path::new(&plan.inputs.fixture),
        "agent-engine fixture",
    )?;
    let fixture_contract = read_public_fixture(&fixture)?;
    let worker = canonical_repository_file(
        &repository_root,
        Path::new("scripts/agent-engine-bakeoff-worker.mjs"),
        "agent-engine worker",
    )?;
    let node = resolve_executable("node")
        .ok_or_else(|| "agent-engine bake-off requires Node.js on PATH".to_owned())?;
    validate_candidate_packages(&candidate_root, &plan.candidates, &plan.supporting_packages)?;
    let package_closure_sha256 = package_closure_sha256(&candidate_root)?;
    let alpine_executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|error| format!("failed to resolve current Alpine executable: {error}"))?;
    let identity = build_identity(
        &plan_sha256,
        &plan,
        &resolved,
        &fixture,
        &fixture_contract,
        &worker,
        &node,
        &alpine_executable,
        &package_closure_sha256,
    )?;

    let mut evidence = AgentEngineEvidence {
        schema: 1,
        plan_id: plan.id.clone(),
        plan_sha256: plan_sha256.clone(),
        identity: identity.into(),
        candidates: Vec::with_capacity(plan.candidates.len()),
    };
    for reviewed in &plan.candidates {
        evidence.candidates.push(run_candidate(
            options,
            &plan,
            reviewed,
            &resolved,
            &fixture,
            &fixture_contract,
            &worker,
            &node,
            &candidate_root,
            &evidence.identity,
        )?);
    }
    assess_evidence(plan_sha256, plan, evidence)
}

fn assess_evidence(
    plan_sha256: String,
    plan: AgentEngineBakeoffPlan,
    evidence: AgentEngineEvidence,
) -> Result<AgentEngineBakeoffReport, String> {
    if evidence.schema != 1 || evidence.plan_id != plan.id || evidence.plan_sha256 != plan_sha256 {
        return Err("agent-engine evidence does not match the exact bake-off plan".to_owned());
    }
    validate_identity(&evidence.identity)?;
    if evidence.candidates.len() != plan.candidates.len() {
        return Err("agent-engine evidence does not contain every planned candidate".to_owned());
    }

    let mut candidates = Vec::with_capacity(evidence.candidates.len());
    for (candidate, reviewed) in evidence.candidates.into_iter().zip(&plan.candidates) {
        candidates.push(assess_candidate(
            candidate,
            reviewed,
            &evidence.identity,
            &plan.budget,
        )?);
    }
    let all_prior_sessions_restored = candidates
        .iter()
        .all(|candidate| candidate.restored_prior_session);
    let all_scenarios_demonstrated = candidates.iter().all(|candidate| {
        candidate
            .scenarios
            .iter()
            .all(|scenario| scenario.outcome == AgentEngineScenarioOutcome::Demonstrated)
    });

    Ok(AgentEngineBakeoffReport {
        schema: 1,
        plan_id: plan.id,
        plan_sha256,
        identity: evidence.identity.into(),
        evidence_complete: true,
        all_scenarios_demonstrated,
        all_prior_sessions_restored,
        recommendation: plan.recommendation.decision,
        candidates,
    })
}

fn canonical_directory(path: &Path, name: &str) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {name} {}: {error}", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{name} is not a directory: {}", path.display()));
    }
    Ok(path)
}

fn canonical_repository_file(root: &Path, relative: &Path, name: &str) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(root.join(relative)).map_err(|error| {
        format!(
            "failed to resolve {name} {}: {error}",
            root.join(relative).display()
        )
    })?;
    if !path.starts_with(root) || !path.is_file() {
        return Err(format!("{name} is outside the repository or not a file"));
    }
    Ok(path)
}

fn read_public_fixture(path: &Path) -> Result<PublicFixture, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read agent-engine fixture: {error}"))?;
    let fixture: PublicFixture = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to decode agent-engine fixture: {error}"))?;
    let expected_prompts = AgentEngineScenario::ALL
        .iter()
        .map(|scenario| scenario.as_str())
        .collect::<BTreeSet<_>>();
    let actual_prompts = fixture
        .prompts
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if fixture.schema != 1
        || fixture.id != "agent-engine-public-v1"
        || fixture.system.trim().is_empty()
        || fixture.target_file != "target.txt"
        || fixture.target_text.trim().is_empty()
        || actual_prompts != expected_prompts
        || fixture.tool_policy.allowed != ["read"]
        || fixture.tool_policy.denied != ["write", "edit", "shell", "network"]
    {
        return Err("agent-engine fixture or exact read-only policy changed".to_owned());
    }
    Ok(fixture)
}

fn effective_policy(plan: &AgentEngineBakeoffPlan, fixture: &PublicFixture) -> serde_json::Value {
    json!({
        "schema": 1,
        "id": plan.inputs.prompt_tool_policy,
        "system": fixture.system,
        "target_file": fixture.target_file,
        "target_sha256": sha256_bytes(format!("{}\n", fixture.target_text).as_bytes()),
        "allowed_tools": fixture.tool_policy.allowed,
        "denied_tools": fixture.tool_policy.denied,
        "temperature": plan.inputs.temperature,
        "max_input_tokens": plan.budget.max_input_tokens,
        "max_output_tokens": plan.budget.max_output_tokens,
        "request_timeout_ms": plan.budget.request_timeout_ms,
        "max_event_queue": plan.budget.max_event_queue
    })
}

fn validate_candidate_packages(
    candidate_root: &Path,
    candidates: &[CandidateReview],
    supporting_packages: &[CandidateSource],
) -> Result<(), String> {
    let lockfile = candidate_root.join("package-lock.json");
    let lock_bytes = std::fs::read(&lockfile)
        .map_err(|error| format!("failed to read isolated candidate package lock: {error}"))?;
    let lock: serde_json::Value = serde_json::from_slice(&lock_bytes)
        .map_err(|error| format!("failed to decode isolated candidate package lock: {error}"))?;
    for source in candidates
        .iter()
        .map(|candidate| &candidate.source)
        .chain(supporting_packages)
    {
        let relative = Path::new("node_modules").join(&source.package);
        let package_json = candidate_root.join(relative).join("package.json");
        let package_bytes = std::fs::read(&package_json).map_err(|error| {
            format!(
                "reviewed package '{}' is unavailable in the isolated root: {error}",
                source.package
            )
        })?;
        let package: serde_json::Value =
            serde_json::from_slice(&package_bytes).map_err(|error| {
                format!(
                    "reviewed package '{}' manifest is invalid: {error}",
                    source.package
                )
            })?;
        if package.get("version").and_then(serde_json::Value::as_str)
            != Some(source.version.as_str())
        {
            return Err(format!(
                "reviewed package '{}' version does not match the reviewed pin",
                source.package
            ));
        }
        let lock_key = format!("node_modules/{}", source.package);
        let locked = lock
            .get("packages")
            .and_then(serde_json::Value::as_object)
            .and_then(|packages| packages.get(&lock_key))
            .ok_or_else(|| {
                format!(
                    "reviewed package '{}' is absent from the isolated package lock",
                    source.package
                )
            })?;
        if locked.get("version").and_then(serde_json::Value::as_str)
            != Some(source.version.as_str())
            || locked.get("integrity").and_then(serde_json::Value::as_str)
                != Some(source.package_integrity.as_str())
        {
            return Err(format!(
                "reviewed package '{}' lock does not match the reviewed version and integrity",
                source.package
            ));
        }
    }
    for candidate in candidates {
        for entry in candidate_entry_files(candidate_root, candidate) {
            let entry = std::fs::canonicalize(&entry).map_err(|error| {
                format!(
                    "candidate '{}' executable entry is unavailable: {error}",
                    candidate.id
                )
            })?;
            if !entry.starts_with(candidate_root) || !entry.is_file() {
                return Err(format!(
                    "candidate '{}' executable entry escaped the isolated root",
                    candidate.id
                ));
            }
        }
    }
    Ok(())
}

fn package_closure_sha256(candidate_root: &Path) -> Result<String, String> {
    let modules = candidate_root.join("node_modules");
    if !modules.is_dir() {
        return Err("isolated candidate root has no node_modules closure".to_owned());
    }
    let mut material = vec![format!(
        "package-lock.json:{}",
        sha256_file(&candidate_root.join("package-lock.json"))?
    )];
    collect_tree_identity(&modules, &modules, &mut material)?;
    material.sort();
    Ok(sha256_bytes(material.join("\n").as_bytes()))
}

fn collect_tree_identity(
    root: &Path,
    directory: &Path,
    material: &mut Vec<String>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("failed to enumerate candidate package closure: {error}"))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| format!("failed to enumerate candidate package closure: {error}"))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect candidate package closure: {error}"))?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| "candidate package path escaped its closure".to_owned())?
            .to_string_lossy()
            .replace('\\', "/");
        if metadata.is_dir() {
            collect_tree_identity(root, &path, material)?;
        } else if metadata.is_file() {
            material.push(format!("file:{relative}:{}", sha256_file(&path)?));
        } else if metadata.file_type().is_symlink() {
            let target = std::fs::read_link(&path)
                .map_err(|error| format!("failed to read candidate package symlink: {error}"))?;
            let resolved = std::fs::canonicalize(&path)
                .map_err(|error| format!("failed to resolve candidate package symlink: {error}"))?;
            if !resolved.starts_with(root) {
                return Err("candidate package symlink escaped the isolated closure".to_owned());
            }
            material.push(format!(
                "link:{relative}:{}",
                target.to_string_lossy().replace('\\', "/")
            ));
        } else {
            return Err("candidate package closure contains an unsupported file type".to_owned());
        }
    }
    Ok(())
}

fn candidate_entry_files(candidate_root: &Path, candidate: &CandidateReview) -> Vec<PathBuf> {
    let modules = candidate_root.join("node_modules");
    match candidate.id.as_str() {
        "opencode-process" => vec![modules.join("opencode-ai/bin/opencode.exe")],
        "pi-sdk-core" => vec![
            modules.join("@earendil-works/pi-agent-core/dist/index.js"),
            modules.join("@earendil-works/pi-ai/dist/api/openai-completions.js"),
        ],
        "pi-process-rpc" => {
            vec![modules.join("@earendil-works/pi-coding-agent/dist/cli.js")]
        }
        "cline-agents" => vec![modules.join("@cline/agents/dist/index.js")],
        _ => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_identity(
    plan_sha256: &str,
    plan: &AgentEngineBakeoffPlan,
    resolved: &config::ResolvedSession,
    fixture: &Path,
    fixture_contract: &PublicFixture,
    worker: &Path,
    node: &Path,
    alpine_executable: &Path,
    package_closure_sha256: &str,
) -> Result<EvidenceIdentity, String> {
    let hardware = hardware::report(Duration::from_secs(10))?.sha256;
    let model = sha256_file(&resolved.model)?;
    let runtime = runtime_bundle_sha256(&resolved.server)?;
    let software_material = [
        sha256_file(alpine_executable)?,
        sha256_file(node)?,
        sha256_file(worker)?,
        package_closure_sha256.to_owned(),
    ];
    let software = sha256_bytes(software_material.join(":").as_bytes());
    let workload = sha256_bytes(
        format!(
            "{}:{}:{}:{}",
            sha256_file(fixture)?,
            plan.required_scenarios
                .iter()
                .map(|scenario| scenario.as_str())
                .collect::<Vec<_>>()
                .join(","),
            plan.budget.requests,
            plan.inputs.prompt_tool_policy
        )
        .as_bytes(),
    );
    let configuration = sha256_bytes(
        format!(
            "{}:{}:{}",
            resolved.session_config_sha256,
            resolved.profile_sha256,
            sha256_file(&resolved.chat_template)?
        )
        .as_bytes(),
    );
    let policy = sha256_bytes(
        format!(
            "{}:{}",
            plan_sha256,
            sha256_bytes(
                &serde_json::to_vec(&effective_policy(plan, fixture_contract)).map_err(
                    |error| format!("failed to encode effective bake-off policy: {error}")
                )?
            )
        )
        .as_bytes(),
    );
    Ok(EvidenceIdentity {
        hardware,
        software,
        model,
        runtime,
        workload,
        configuration,
        policy,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_candidate(
    options: &AgentEngineBakeoffOptions,
    plan: &AgentEngineBakeoffPlan,
    reviewed: &CandidateReview,
    resolved: &config::ResolvedSession,
    fixture: &Path,
    fixture_contract: &PublicFixture,
    worker: &Path,
    node: &Path,
    candidate_root: &Path,
    identity: &StrictEvidenceIdentity,
) -> Result<CandidateEvidence, String> {
    let acquisition = session::acquire(&AcquireSessionOptions {
        install_root: options.install_root.clone(),
        profile: Some(plan.inputs.profile.clone()),
        vision: false,
        force_fallback: false,
        allow_legacy_identity: false,
        lock_timeout: options.lock_timeout,
        startup_timeout: options.startup_timeout,
    })?;
    let prior_session = acquisition.prior.clone();
    let acquired_session = match session::snapshot_under_capacity(
        &options.install_root,
        options.lock_timeout,
    ) {
        Ok(snapshot) => snapshot,
        Err(snapshot_error) => {
            return match release_candidate_session(options, acquisition) {
                Ok(()) => Err(snapshot_error),
                Err(release_error) => Err(format!(
                    "failed to capture acquired Session: {snapshot_error}; Session release also failed: {release_error}"
                )),
            };
        }
    };
    let run_result = run_candidate_scenarios(
        options,
        plan,
        reviewed,
        resolved,
        fixture,
        fixture_contract,
        worker,
        node,
        candidate_root,
        &acquired_session,
        &identity.software,
    );
    let release_result = release_candidate_session(options, acquisition);
    let restored_session = release_result.and_then(|_| {
        session::snapshot_under_capacity(&options.install_root, options.lock_timeout)
    });
    let (run, restored_session) = match (run_result, restored_session) {
        (Ok(run), Ok(restored)) => (run, restored),
        (Err(run_error), Ok(_)) => return Err(run_error),
        (Ok(_), Err(release_error)) => return Err(release_error),
        (Err(run_error), Err(release_error)) => {
            return Err(format!(
                "agent-engine candidate run failed: {run_error}; Session release also failed: {release_error}"
            ));
        }
    };
    Ok(CandidateEvidence {
        id: reviewed.id.clone(),
        source_version: reviewed.source.version.clone(),
        source_commit: reviewed.source.commit.clone(),
        adapter_executable_sha256: run.adapter_executable_sha256,
        identity: identity.clone(),
        requests_used: run.requests_used,
        max_wall_ms_observed: run.max_wall_ms_observed,
        max_input_tokens_observed: run.max_input_tokens_observed,
        max_output_tokens_observed: run.max_output_tokens_observed,
        retries_used: run.retries_used,
        worker_restarts_used: run.worker_restarts_used,
        candidate_state_disposed: run.candidate_state_disposed,
        restorations_verified: run.restorations_verified,
        prior_session: prior_session.into(),
        restored_session: restored_session.into(),
        scenarios: run.scenarios,
        events: run.events,
        errors: run.errors,
    })
}

fn release_candidate_session(
    options: &AgentEngineBakeoffOptions,
    acquisition: SessionAcquisition,
) -> Result<(), String> {
    session::release(&ReleaseSessionOptions {
        install_root: options.install_root.clone(),
        acquisition,
        keep_server: false,
        lock_timeout: options.lock_timeout,
        startup_timeout: options.startup_timeout,
    })
    .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
fn run_candidate_scenarios(
    options: &AgentEngineBakeoffOptions,
    plan: &AgentEngineBakeoffPlan,
    reviewed: &CandidateReview,
    resolved: &config::ResolvedSession,
    fixture: &Path,
    fixture_contract: &PublicFixture,
    worker: &Path,
    node: &Path,
    candidate_root: &Path,
    acquired_session: &SessionSnapshot,
    software_identity: &str,
) -> Result<CandidateRun, String> {
    let candidate_state = tempfile::Builder::new()
        .prefix("alpine-agent-engine-")
        .tempdir()
        .map_err(|error| format!("failed to create disposable candidate state: {error}"))?;
    let state_path = candidate_state.path().to_path_buf();
    let run_result = (|| -> Result<CandidateRun, String> {
        let api_key = std::fs::read_to_string(&resolved.api_key_file)
            .map_err(|error| format!("failed to read local API key: {error}"))?;
        if api_key.trim().is_empty() {
            return Err("local API key is empty".to_owned());
        }
        let adapter_executable_sha256 = sha256_bytes(
            format!(
                "{}:{}:{}:{}",
                software_identity, reviewed.id, reviewed.source.version, reviewed.source.commit
            )
            .as_bytes(),
        );
        let mut requests_used = 0_u32;
        let mut max_wall_ms_observed = 0_u64;
        let mut max_input_tokens_observed = 0_u32;
        let mut max_output_tokens_observed = 0_u32;
        let mut retries_used = 0_u32;
        let mut worker_restarts_used = 0_u32;
        let mut restorations_verified = 0_u32;
        let mut scenarios = Vec::with_capacity(AgentEngineScenario::ALL.len());
        let mut events = Vec::new();
        let mut errors = Vec::new();
        let mut sequence = 0_u64;

        for scenario in AgentEngineScenario::ALL {
            let remaining_budget = remaining_budget(
                &plan.budget,
                requests_used,
                retries_used,
                worker_restarts_used,
                events.len(),
            )?;
            let (mut receipt, wall_ms) = invoke_adapter(
                plan,
                &remaining_budget,
                reviewed,
                resolved,
                fixture,
                fixture_contract,
                worker,
                node,
                candidate_root,
                &state_path,
                scenario,
                api_key.trim(),
            )?;
            validate_adapter_receipt(&receipt, reviewed, scenario, &remaining_budget)?;
            let observed =
                session::snapshot_under_capacity(&options.install_root, options.lock_timeout)?;
            let session_restored =
                if session::snapshots_materially_equal(&observed, acquired_session) {
                    true
                } else {
                    session::restore_snapshot_under_capacity(
                        &options.install_root,
                        acquired_session,
                        options.lock_timeout,
                        options.startup_timeout,
                    )?;
                    receipt.events.clear();
                    receipt.errors = vec![AdapterError {
                        kind: AgentEngineErrorKind::SessionRestoreFailed,
                        code: "session-drift-restored".to_owned(),
                        retryable: false,
                    }];
                    true
                };
            restorations_verified += u32::from(session_restored);
            requests_used = requests_used
                .checked_add(receipt.requests_used)
                .ok_or_else(|| "agent-engine request accounting overflowed".to_owned())?;
            max_wall_ms_observed = max_wall_ms_observed.max(wall_ms);
            max_input_tokens_observed =
                max_input_tokens_observed.max(receipt.max_input_tokens_observed);
            max_output_tokens_observed =
                max_output_tokens_observed.max(receipt.max_output_tokens_observed);
            retries_used = retries_used
                .checked_add(receipt.retries_used)
                .ok_or_else(|| "agent-engine retry accounting overflowed".to_owned())?;
            worker_restarts_used = worker_restarts_used
                .checked_add(receipt.worker_restarts_used)
                .ok_or_else(|| "agent-engine restart accounting overflowed".to_owned())?;
            scenarios.push(ScenarioEvidence {
                scenario,
                requests_used: receipt.requests_used,
                wall_ms,
                session_restored,
            });
            for kind in receipt.events {
                sequence += 1;
                events.push(AgentEngineEvent {
                    sequence,
                    scenario,
                    kind,
                });
            }
            errors.extend(receipt.errors.into_iter().map(|error| AgentEngineError {
                scenario,
                kind: error.kind,
                code: error.code,
                retryable: error.retryable,
            }));
        }
        Ok(CandidateRun {
            adapter_executable_sha256,
            requests_used,
            max_wall_ms_observed,
            max_input_tokens_observed,
            max_output_tokens_observed,
            retries_used,
            worker_restarts_used,
            candidate_state_disposed: false,
            restorations_verified,
            scenarios,
            events,
            errors,
        })
    })();
    let (mut run, candidate_state_disposed) =
        close_candidate_state(candidate_state, &state_path, run_result)?;
    run.candidate_state_disposed = candidate_state_disposed;
    Ok(run)
}

fn close_candidate_state<T>(
    candidate_state: tempfile::TempDir,
    state_path: &Path,
    run_result: Result<T, String>,
) -> Result<(T, bool), String> {
    let cleanup_result = candidate_state
        .close()
        .map_err(|error| format!("failed to dispose candidate state: {error}"));
    match (run_result, cleanup_result) {
        (Ok(run), Ok(())) => Ok((run, !state_path.exists())),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(primary), Err(cleanup)) => Err(format!("{primary}; additionally {cleanup}")),
    }
}

fn remaining_budget(
    budget: &BakeoffBudget,
    requests_used: u32,
    retries_used: u32,
    worker_restarts_used: u32,
    events_used: usize,
) -> Result<BakeoffBudget, String> {
    let events_used = u32::try_from(events_used)
        .map_err(|_| "agent-engine event accounting overflowed".to_owned())?;
    Ok(BakeoffBudget {
        requests: budget
            .requests
            .checked_sub(requests_used)
            .ok_or_else(|| "agent-engine request budget was already exceeded".to_owned())?,
        request_timeout_ms: budget.request_timeout_ms,
        max_input_tokens: budget.max_input_tokens,
        max_output_tokens: budget.max_output_tokens,
        max_retries: budget
            .max_retries
            .checked_sub(retries_used)
            .ok_or_else(|| "agent-engine retry budget was already exceeded".to_owned())?,
        max_worker_restarts: budget
            .max_worker_restarts
            .checked_sub(worker_restarts_used)
            .ok_or_else(|| "agent-engine restart budget was already exceeded".to_owned())?,
        max_event_queue: budget
            .max_event_queue
            .checked_sub(events_used)
            .ok_or_else(|| "agent-engine event budget was already exceeded".to_owned())?,
    })
}

#[allow(clippy::too_many_arguments)]
fn invoke_adapter(
    plan: &AgentEngineBakeoffPlan,
    adapter_budget: &BakeoffBudget,
    reviewed: &CandidateReview,
    resolved: &config::ResolvedSession,
    fixture: &Path,
    fixture_contract: &PublicFixture,
    worker: &Path,
    node: &Path,
    candidate_root: &Path,
    state_root: &Path,
    scenario: AgentEngineScenario,
    api_key: &str,
) -> Result<(AdapterReceipt, u64), String> {
    let worker_argument = command_compatible_path(worker);
    let candidate_root_argument = command_compatible_path(candidate_root);
    let fixture_argument = command_compatible_path(fixture);
    let request_path = state_root.join(format!("request-{}.json", scenario.as_str()));
    let request = json!({
        "schema": 1,
        "candidate": reviewed.id,
        "scenario": scenario,
        "source_version": reviewed.source.version,
        "base_url": resolved.base_url,
        "model_id": plan.inputs.model_id,
        "fixture": fixture_argument,
        "effective_policy": effective_policy(plan, fixture_contract),
        "state_root": state_root,
        "budget": {
            "requests": adapter_budget.requests,
            "request_timeout_ms": adapter_budget.request_timeout_ms,
            "max_input_tokens": adapter_budget.max_input_tokens,
            "max_output_tokens": adapter_budget.max_output_tokens,
            "max_retries": adapter_budget.max_retries,
            "max_worker_restarts": adapter_budget.max_worker_restarts,
            "max_event_queue": adapter_budget.max_event_queue
        },
        "prompt_tool_policy": plan.inputs.prompt_tool_policy
    });
    std::fs::write(
        &request_path,
        serde_json::to_vec(&request)
            .map_err(|error| format!("failed to encode adapter request: {error}"))?,
    )
    .map_err(|error| format!("failed to write adapter request: {error}"))?;
    let mut command = Command::new(node);
    command
        .args([
            worker_argument.as_os_str(),
            OsStr::new("--candidate-root"),
            candidate_root_argument.as_os_str(),
            OsStr::new("--request"),
            request_path.as_os_str(),
        ])
        .current_dir(state_root)
        .env_clear()
        .envs(sanitized_environment())
        .env("ALPINE_BAKEOFF_API_KEY", api_key);
    let started = Instant::now();
    let output = run_command_bounded(
        &mut command,
        Duration::from_millis(plan.budget.request_timeout_ms),
    )
    .map_err(|error| format!("failed to run candidate adapter: {error}"))?;
    let wall_ms = u64::try_from(started.elapsed().as_millis())
        .unwrap_or(u64::MAX)
        .min(plan.budget.request_timeout_ms);
    validate_adapter_completion(output.timed_out, output.status.success())?;
    let receipt = serde_json::from_str::<AdapterReceipt>(output.stdout.trim()).map_err(|_| {
        "candidate adapter emitted malformed or non-exclusive JSON evidence".to_owned()
    })?;
    Ok((receipt, wall_ms))
}

fn validate_adapter_completion(timed_out: bool, succeeded: bool) -> Result<(), String> {
    if timed_out {
        Err("candidate adapter timed out without a complete accounting receipt".to_owned())
    } else if !succeeded {
        Err("candidate adapter exited without a complete accounting receipt".to_owned())
    } else {
        Ok(())
    }
}

fn validate_adapter_receipt(
    receipt: &AdapterReceipt,
    reviewed: &CandidateReview,
    scenario: AgentEngineScenario,
    budget: &BakeoffBudget,
) -> Result<(), String> {
    if receipt.schema != 1 || receipt.candidate != reviewed.id || receipt.scenario != scenario {
        return Err("candidate adapter receipt crossed its assigned boundary".to_owned());
    }
    if receipt.requests_used > budget.requests
        || receipt.max_input_tokens_observed > budget.max_input_tokens
        || receipt.max_output_tokens_observed > budget.max_output_tokens
        || receipt.retries_used > budget.max_retries
        || receipt.worker_restarts_used > budget.max_worker_restarts
        || receipt.events.len() > budget.max_event_queue as usize
    {
        return Err("candidate adapter receipt exceeded the shared budget".to_owned());
    }
    let normalized_error_pair = receipt.scenario == AgentEngineScenario::NormalizedErrors
        && receipt.errors.len() == 1
        && receipt.events.len() == 1
        && matches!(
            (&receipt.events[0], &receipt.errors[0]),
            (
                AgentEngineEventKind::ErrorNormalized { kind, code },
                AdapterError {
                    kind: error_kind,
                    code: error_code,
                    ..
                }
            ) if kind == error_kind && code == error_code
        );
    if receipt.errors.len() > 1
        || (!receipt.errors.is_empty() && !receipt.events.is_empty() && !normalized_error_pair)
        || (receipt.errors.is_empty() && receipt.events.is_empty())
    {
        return Err("candidate adapter receipt has ambiguous scenario evidence".to_owned());
    }
    if receipt.errors.iter().any(|error| !safe_code(&error.code)) {
        return Err("candidate adapter emitted an unsafe error code".to_owned());
    }
    Ok(())
}

fn read_plan(path: &Path) -> Result<(String, AgentEngineBakeoffPlan), String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read agent-engine bake-off plan: {error}"))?;
    let plan: AgentEngineBakeoffPlan = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to decode agent-engine bake-off plan: {error}"))?;
    validate_plan(&plan)?;
    Ok((sha256_bytes(&bytes), plan))
}

fn assess_candidate(
    candidate: CandidateEvidence,
    reviewed: &CandidateReview,
    identity: &StrictEvidenceIdentity,
    budget: &BakeoffBudget,
) -> Result<AgentEngineCandidateReport, String> {
    if candidate.id != reviewed.id
        || candidate.source_version != reviewed.source.version
        || candidate.source_commit != reviewed.source.commit
    {
        return Err(format!(
            "candidate '{}' evidence does not match its reviewed source pin",
            candidate.id
        ));
    }
    if &candidate.identity != identity {
        return Err(format!(
            "candidate '{}' did not use the shared Evidence Identity",
            candidate.id
        ));
    }
    if candidate.requests_used > budget.requests
        || candidate.max_wall_ms_observed > budget.request_timeout_ms
        || candidate.max_input_tokens_observed > budget.max_input_tokens
        || candidate.max_output_tokens_observed > budget.max_output_tokens
        || candidate.retries_used > budget.max_retries
        || candidate.worker_restarts_used > budget.max_worker_restarts
        || candidate.events.len() > budget.max_event_queue as usize
    {
        return Err(format!(
            "candidate '{}' exceeded or changed the shared budget",
            candidate.id
        ));
    }
    validate_sha256(
        &candidate.adapter_executable_sha256,
        "adapter executable identity",
    )?;
    if !candidate.candidate_state_disposed {
        return Err(format!(
            "candidate '{}' retained worker-owned state",
            candidate.id
        ));
    }
    if candidate.restorations_verified != AgentEngineScenario::ALL.len() as u32
        || candidate.scenarios.len() != AgentEngineScenario::ALL.len()
        || candidate
            .scenarios
            .iter()
            .map(|scenario| scenario.scenario)
            .ne(AgentEngineScenario::ALL)
        || candidate
            .scenarios
            .iter()
            .any(|scenario| !scenario.session_restored)
    {
        return Err(format!(
            "candidate '{}' did not verify Session restoration after every scenario",
            candidate.id
        ));
    }
    let prior_session: SessionSnapshot = candidate.prior_session.clone().into();
    let restored_session: SessionSnapshot = candidate.restored_session.clone().into();
    let restored_prior_session =
        session::snapshots_materially_equal(&prior_session, &restored_session);
    if !restored_prior_session {
        return Err(format!(
            "candidate '{}' did not restore the exact prior material Session",
            candidate.id
        ));
    }
    validate_events(
        &candidate.events,
        budget,
        candidate.retries_used,
        candidate.worker_restarts_used,
    )?;
    validate_errors(&candidate.errors)?;

    let scenarios = candidate
        .scenarios
        .iter()
        .map(|scenario| assess_scenario(scenario, &candidate.events, &candidate.errors))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(AgentEngineCandidateReport {
        id: candidate.id,
        source_version: candidate.source_version,
        source_commit: candidate.source_commit,
        adapter_executable_sha256: candidate.adapter_executable_sha256,
        budget: AgentEngineBudgetObservation {
            requests_used: candidate.requests_used,
            max_wall_ms_observed: candidate.max_wall_ms_observed,
            max_input_tokens_observed: candidate.max_input_tokens_observed,
            max_output_tokens_observed: candidate.max_output_tokens_observed,
            retries_used: candidate.retries_used,
            worker_restarts_used: candidate.worker_restarts_used,
        },
        restored_prior_session,
        restorations_verified: candidate.restorations_verified,
        candidate_state_disposed: candidate.candidate_state_disposed,
        scenarios,
        events: candidate.events,
        errors: candidate.errors,
    })
}

fn validate_identity(identity: &StrictEvidenceIdentity) -> Result<(), String> {
    for (name, value) in [
        ("hardware", &identity.hardware),
        ("software", &identity.software),
        ("model", &identity.model),
        ("runtime", &identity.runtime),
        ("workload", &identity.workload),
        ("configuration", &identity.configuration),
        ("policy", &identity.policy),
    ] {
        validate_sha256(value, &format!("Evidence Identity {name}"))?;
    }
    Ok(())
}

fn validate_sha256(value: &str, name: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{name} is not a complete SHA-256 identity"));
    }
    Ok(())
}

fn validate_events(
    events: &[AgentEngineEvent],
    budget: &BakeoffBudget,
    retries_used: u32,
    worker_restarts_used: u32,
) -> Result<(), String> {
    let mut prior = 0;
    let mut tools = BTreeMap::<&str, &str>::new();
    for event in events {
        if event.sequence == 0 || event.sequence <= prior || event.scenario != event.kind.scenario()
        {
            return Err(
                "agent-engine events are out of order or cross scenario boundaries".to_owned(),
            );
        }
        prior = event.sequence;
        match &event.kind {
            AgentEngineEventKind::StreamDelta { bytes } if *bytes == 0 => {
                return Err("agent-engine stream delta is empty".to_owned());
            }
            AgentEngineEventKind::ToolStarted { call_id, tool }
            | AgentEngineEventKind::ToolFinished { call_id, tool, .. }
                if !safe_code(call_id) || !safe_code(tool) =>
            {
                return Err("agent-engine tool identity is unsafe or empty".to_owned());
            }
            AgentEngineEventKind::ToolStarted { call_id, tool } => {
                if tools.insert(call_id, tool).is_some() {
                    return Err(
                        "agent-engine tool lifecycle reused a live call identity".to_owned()
                    );
                }
            }
            AgentEngineEventKind::ToolFinished {
                call_id,
                tool,
                succeeded,
            } => {
                if !*succeeded {
                    return Err(
                        "agent-engine tool lifecycle reported an unsuccessful execution".to_owned(),
                    );
                }
                if tools.remove(call_id.as_str()) != Some(tool.as_str()) {
                    return Err(
                        "agent-engine tool lifecycle did not finish the typed call that started"
                            .to_owned(),
                    );
                }
            }
            AgentEngineEventKind::RetryScheduled { attempt }
            | AgentEngineEventKind::RetryCompleted { attempt }
                if *attempt == 0 || *attempt > budget.max_retries =>
            {
                return Err("agent-engine retry event exceeds the shared budget".to_owned());
            }
            AgentEngineEventKind::CompactionStarted { tokens_before }
                if *tokens_before == 0 || *tokens_before > budget.max_input_tokens =>
            {
                return Err("agent-engine compaction start is outside the token budget".to_owned());
            }
            AgentEngineEventKind::CompactionFinished { tokens_after }
                if *tokens_after > budget.max_input_tokens =>
            {
                return Err("agent-engine compaction result is outside the token budget".to_owned());
            }
            AgentEngineEventKind::BackpressureApplied { queue_capacity }
                if *queue_capacity == 0 || *queue_capacity > budget.max_event_queue =>
            {
                return Err("agent-engine backpressure event exceeds the bounded queue".to_owned());
            }
            AgentEngineEventKind::WorkerRestarted { restart }
                if *restart == 0 || *restart > budget.max_worker_restarts =>
            {
                return Err("agent-engine restart event exceeds the shared budget".to_owned());
            }
            AgentEngineEventKind::ContinuationRestored { cursor_sha256 } => {
                validate_sha256(cursor_sha256, "continuation cursor")?;
            }
            AgentEngineEventKind::ErrorNormalized { code, .. } if !safe_code(code) => {
                return Err("agent-engine normalized error code is unsafe or empty".to_owned());
            }
            _ => {}
        }
    }
    if !tools.is_empty() {
        return Err("agent-engine tool lifecycle left an unfinished call".to_owned());
    }
    validate_stream_lifecycle(events)?;
    validate_lifecycle(
        events,
        |kind| matches!(kind, AgentEngineEventKind::CancellationRequested),
        |kind| matches!(kind, AgentEngineEventKind::CancellationConfirmed),
        "cancellation lifecycle",
    )?;
    validate_lifecycle(
        events,
        |kind| matches!(kind, AgentEngineEventKind::CompactionStarted { .. }),
        |kind| matches!(kind, AgentEngineEventKind::CompactionFinished { .. }),
        "compaction lifecycle",
    )?;
    validate_lifecycle(
        events,
        |kind| matches!(kind, AgentEngineEventKind::WorkerExited { .. }),
        |kind| matches!(kind, AgentEngineEventKind::WorkerRestarted { .. }),
        "worker restart lifecycle",
    )?;
    let retry_scheduled = events
        .iter()
        .filter_map(|event| match event.kind {
            AgentEngineEventKind::RetryScheduled { attempt } => Some(attempt),
            _ => None,
        })
        .collect::<Vec<_>>();
    let retry_completed = events
        .iter()
        .filter_map(|event| match event.kind {
            AgentEngineEventKind::RetryCompleted { attempt } => Some(attempt),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected_retries = (1..=retries_used).collect::<Vec<_>>();
    if retry_scheduled != expected_retries || retry_completed != expected_retries {
        return Err(
            "agent-engine retry lifecycle does not match the observed retry count".to_owned(),
        );
    }
    let worker_exits = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEngineEventKind::WorkerExited { .. }))
        .count();
    let worker_restarts = events
        .iter()
        .filter_map(|event| match event.kind {
            AgentEngineEventKind::WorkerRestarted { restart } => Some(restart),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected_restarts = (1..=worker_restarts_used).collect::<Vec<_>>();
    if worker_exits != worker_restarts_used as usize || worker_restarts != expected_restarts {
        return Err(
            "agent-engine restart lifecycle does not match the observed restart count".to_owned(),
        );
    }
    for event in events {
        if let AgentEngineEventKind::RetryCompleted { attempt } = &event.kind {
            let scheduled = events.iter().any(|prior| {
                prior.sequence < event.sequence
                    && matches!(
                        prior.kind,
                        AgentEngineEventKind::RetryScheduled { attempt: scheduled }
                            if scheduled == *attempt
                    )
            });
            if !scheduled {
                return Err(
                    "agent-engine retry lifecycle completed an unscheduled attempt".to_owned(),
                );
            }
        }
        if let AgentEngineEventKind::CompactionFinished { tokens_after } = &event.kind {
            let tokens_before = events.iter().rev().find_map(|prior| {
                (prior.sequence < event.sequence)
                    .then_some(&prior.kind)
                    .and_then(|kind| match kind {
                        AgentEngineEventKind::CompactionStarted { tokens_before } => {
                            Some(*tokens_before)
                        }
                        _ => None,
                    })
            });
            if tokens_before.is_some_and(|before| *tokens_after >= before) {
                return Err(
                    "agent-engine compaction did not reduce the observed context".to_owned(),
                );
            }
        }
        if matches!(
            event.kind,
            AgentEngineEventKind::ContinuationRestored { .. }
        ) && !events.iter().any(|prior| {
            prior.sequence < event.sequence
                && matches!(prior.kind, AgentEngineEventKind::WorkerRestarted { .. })
        }) {
            return Err(
                "agent-engine continuation recovery was not observed after worker restart"
                    .to_owned(),
            );
        }
    }
    Ok(())
}

fn validate_stream_lifecycle(events: &[AgentEngineEvent]) -> Result<(), String> {
    let mut state = 0_u8;
    for event in events {
        match event.kind {
            AgentEngineEventKind::StreamStarted => {
                if state != 0 {
                    return Err(
                        "agent-engine stream lifecycle is duplicated or reopened".to_owned()
                    );
                }
                state = 1;
            }
            AgentEngineEventKind::StreamDelta { .. } if state != 1 => {
                return Err("agent-engine stream delta is outside its lifecycle".to_owned());
            }
            AgentEngineEventKind::StreamFinished => {
                if state != 1 {
                    return Err("agent-engine stream lifecycle is out of order".to_owned());
                }
                state = 2;
            }
            _ => {}
        }
    }
    if state == 1 {
        Err("agent-engine stream lifecycle is incomplete".to_owned())
    } else {
        Ok(())
    }
}

fn validate_lifecycle(
    events: &[AgentEngineEvent],
    starts: impl Fn(&AgentEngineEventKind) -> bool,
    finishes: impl Fn(&AgentEngineEventKind) -> bool,
    lifecycle: &str,
) -> Result<(), String> {
    let mut state = 0_u8;
    for event in events {
        if starts(&event.kind) {
            if state != 0 {
                return Err(format!(
                    "agent-engine {lifecycle} is duplicated or reopened"
                ));
            }
            state = 1;
        } else if finishes(&event.kind) {
            if state != 1 {
                return Err(format!("agent-engine {lifecycle} is out of order"));
            }
            state = 2;
        }
    }
    if state == 1 {
        Err(format!("agent-engine {lifecycle} is incomplete"))
    } else {
        Ok(())
    }
}

fn validate_errors(errors: &[AgentEngineError]) -> Result<(), String> {
    if errors.iter().any(|error| !safe_code(&error.code)) {
        return Err("agent-engine error code is unsafe or empty".to_owned());
    }
    Ok(())
}

fn safe_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn assess_scenario(
    observation: &ScenarioEvidence,
    events: &[AgentEngineEvent],
    errors: &[AgentEngineError],
) -> Result<AgentEngineScenarioReport, String> {
    let scenario = observation.scenario;
    let scenario_events = events
        .iter()
        .filter(|event| event.scenario == scenario)
        .collect::<Vec<_>>();
    let scenario_errors = errors
        .iter()
        .filter(|error| error.scenario == scenario)
        .collect::<Vec<_>>();
    let demonstrated = match scenario {
        AgentEngineScenario::Streaming => {
            has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::StreamStarted)
            }) && has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::StreamDelta { .. })
            }) && has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::StreamFinished)
            })
        }
        AgentEngineScenario::Tools => {
            has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::ToolStarted { .. })
            }) && has_event(&scenario_events, |kind| {
                matches!(
                    kind,
                    AgentEngineEventKind::ToolFinished {
                        succeeded: true,
                        ..
                    }
                )
            })
        }
        AgentEngineScenario::Steering => has_event(&scenario_events, |kind| {
            matches!(kind, AgentEngineEventKind::SteeringAccepted)
        }),
        AgentEngineScenario::FollowUp => has_event(&scenario_events, |kind| {
            matches!(kind, AgentEngineEventKind::FollowUpAccepted)
        }),
        AgentEngineScenario::Cancellation => {
            has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::CancellationRequested)
            }) && has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::CancellationConfirmed)
            })
        }
        AgentEngineScenario::Retry => {
            has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::RetryScheduled { .. })
            }) && has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::RetryCompleted { .. })
            })
        }
        AgentEngineScenario::Compaction => {
            has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::CompactionStarted { .. })
            }) && has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::CompactionFinished { .. })
            })
        }
        AgentEngineScenario::Backpressure => has_event(&scenario_events, |kind| {
            matches!(kind, AgentEngineEventKind::BackpressureApplied { .. })
        }),
        AgentEngineScenario::WorkerRestart => {
            has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::WorkerExited { .. })
            }) && has_event(&scenario_events, |kind| {
                matches!(kind, AgentEngineEventKind::WorkerRestarted { .. })
            })
        }
        AgentEngineScenario::ContinuationRecovery => has_event(&scenario_events, |kind| {
            matches!(kind, AgentEngineEventKind::ContinuationRestored { .. })
        }),
        AgentEngineScenario::NormalizedErrors => scenario_errors.iter().any(|error| {
            scenario_events.iter().any(|event| {
                matches!(
                    &event.kind,
                    AgentEngineEventKind::ErrorNormalized { kind, code }
                        if *kind == error.kind && code == &error.code
                )
            })
        }),
    };
    if demonstrated {
        return Ok(AgentEngineScenarioReport {
            scenario,
            outcome: AgentEngineScenarioOutcome::Demonstrated,
            requests_used: observation.requests_used,
            wall_ms: observation.wall_ms,
            session_restored: observation.session_restored,
            error_code: None,
        });
    }
    if !scenario_events.is_empty() {
        return Err(format!(
            "agent-engine scenario {scenario:?} contains partial lifecycle evidence"
        ));
    }
    let error = scenario_errors.first().ok_or_else(|| {
        format!(
            "agent-engine scenario {scenario:?} is neither demonstrated nor an explicit failure"
        )
    })?;
    Ok(AgentEngineScenarioReport {
        scenario,
        outcome: AgentEngineScenarioOutcome::ExplicitFailure,
        requests_used: observation.requests_used,
        wall_ms: observation.wall_ms,
        session_restored: observation.session_restored,
        error_code: Some(error.code.clone()),
    })
}

fn has_event(
    events: &[&AgentEngineEvent],
    predicate: impl Fn(&AgentEngineEventKind) -> bool,
) -> bool {
    events.iter().any(|event| predicate(&event.kind))
}

fn validate_plan(plan: &AgentEngineBakeoffPlan) -> Result<(), String> {
    if plan.schema != 1 || plan.id != "agent-engine-bakeoff-v1" {
        return Err("agent-engine bake-off plan must use schema 1 and the v1 identity".to_owned());
    }
    let candidate_ids = plan
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<Vec<_>>();
    if candidate_ids != REQUIRED_CANDIDATES {
        return Err("agent-engine bake-off candidates or ordering changed".to_owned());
    }
    if plan.required_scenarios != AgentEngineScenario::ALL {
        return Err("agent-engine bake-off recovery scenarios or ordering changed".to_owned());
    }
    if plan.inputs.profile != "stable-16k"
        || plan.inputs.model_id != "local-qwen"
        || plan.inputs.temperature != 0
        || plan.inputs.prompt_tool_policy != "read-only-exact-fixture-v1"
        || plan.inputs.template_source != "session-config"
        || Path::new(&plan.inputs.fixture).is_absolute()
        || Path::new(&plan.inputs.fixture)
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("agent-engine bake-off material inputs are incomplete or unsafe".to_owned());
    }
    if plan.budget.requests == 0
        || plan.budget.requests > 64
        || plan.budget.request_timeout_ms == 0
        || plan.budget.max_input_tokens == 0
        || plan.budget.max_output_tokens == 0
        || plan.budget.max_output_tokens > plan.budget.max_input_tokens
        || plan.budget.max_retries > 8
        || plan.budget.max_worker_restarts > 4
        || plan.budget.max_event_queue == 0
        || plan.budget.max_event_queue > 4096
    {
        return Err("agent-engine bake-off budget is empty, inconsistent, or unbounded".to_owned());
    }
    let identity_fields = plan
        .evidence_identity
        .required_fields
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if identity_fields
        != [
            "configuration",
            "hardware",
            "model",
            "policy",
            "runtime",
            "software",
            "workload",
        ]
        .into_iter()
        .collect()
    {
        return Err("agent-engine bake-off requires complete Evidence Identity".to_owned());
    }
    let material_inputs = plan
        .evidence_identity
        .identical_material_inputs
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for required in [
        "model-artifact",
        "profile",
        "runtime",
        "template",
        "prompt-tool-policy",
        "task-fixtures",
        "request-budget",
        "recovery-scenarios",
    ] {
        if !material_inputs.contains(required) {
            return Err(format!(
                "agent-engine bake-off omits identical material input '{required}'"
            ));
        }
    }
    for candidate in &plan.candidates {
        validate_candidate(candidate)?;
    }
    if plan.supporting_packages.len() != 1 {
        return Err("agent-engine supporting package pins changed".to_owned());
    }
    for source in &plan.supporting_packages {
        validate_source(source, "supporting package")?;
    }
    if !source_matches_reviewed_pin(
        &plan.supporting_packages[0],
        &reviewed_supporting_source_pin(),
    ) {
        return Err("supporting package source pin changed".to_owned());
    }
    if plan.recommendation.decision != "no-go" {
        return Err("schema v1 recommendation must remain no-go".to_owned());
    }
    if plan
        .recommendation
        .rationale_codes
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        != [
            "live-comparison-shows-policy-and-capability-gaps",
            "exact-restart-recovery-not-yet-proven",
            "backpressure-contract-incomplete",
            "no-engine-boundary-adoption-before-second-engine-value",
        ]
    {
        return Err("schema v1 recommendation rationale changed".to_owned());
    }
    if plan.privacy.publish_raw_prompts
        || plan.privacy.publish_credentials
        || plan.privacy.publish_machine_paths
        || plan.privacy.publish_raw_evidence
        || plan.privacy.publish_model_artifacts
    {
        return Err("agent-engine bake-off privacy allowlist permits private material".to_owned());
    }
    Ok(())
}

fn validate_candidate(candidate: &CandidateReview) -> Result<(), String> {
    validate_source(&candidate.source, &format!("candidate '{}'", candidate.id))?;
    let expected = reviewed_candidate_source_pin(&candidate.id)
        .ok_or_else(|| format!("candidate '{}' is not reviewed by schema v1", candidate.id))?;
    if !source_matches_reviewed_pin(&candidate.source, &expected) {
        return Err(format!("candidate '{}' source pin changed", candidate.id));
    }
    if candidate.surface.trim().is_empty()
        || candidate.dependencies.direct_runtime_dependencies == 0
        || candidate.dependencies.runtime_requirement.trim().is_empty()
        || candidate.dependencies.packaging_cost.trim().is_empty()
        || candidate.dependencies.update_cost.trim().is_empty()
        || candidate.maintenance.repository_archived
        || candidate.maintenance.source_commit_at.trim().is_empty()
        || candidate.maintenance.signals.is_empty()
        || candidate.security.authority.trim().is_empty()
        || candidate.security.credential_boundary.trim().is_empty()
        || candidate.security.known_risks.is_empty()
        || candidate.adapter_boundary.trim().is_empty()
        || candidate.smallest_missing_upstream_hook.trim().is_empty()
        || !matches!(candidate.decision.as_str(), "conditional-go" | "no-go")
    {
        return Err(format!(
            "candidate '{}' has an incomplete source, dependency, maintenance, security, or decision review",
            candidate.id
        ));
    }
    if candidate.security.built_in_sandbox {
        return Err(format!(
            "candidate '{}' incorrectly treats an in-process policy as a built-in sandbox",
            candidate.id
        ));
    }
    if candidate
        .dependencies
        .locked_package_count
        .is_some_and(|count| count < candidate.dependencies.direct_runtime_dependencies)
        || candidate.dependencies.unpacked_size_bytes == Some(0)
    {
        return Err(format!(
            "candidate '{}' has inconsistent dependency facts",
            candidate.id
        ));
    }
    Ok(())
}

fn source_matches_reviewed_pin(source: &CandidateSource, expected: &ReviewedSourcePin) -> bool {
    source.repository == expected.repository
        && source.version == expected.version
        && source.commit == expected.commit
        && source.package == expected.package
        && source.package_integrity == expected.package_integrity
        && source.license == expected.license
}

fn reviewed_supporting_source_pin() -> ReviewedSourcePin {
    ReviewedSourcePin {
        repository: "https://github.com/earendil-works/pi",
        version: "0.84.2",
        commit: "914cf1472e715297caa30db4b9535d534a9eb718",
        package: "@earendil-works/pi-ai",
        package_integrity: "sha512-6MzsrYIYNVlE7SfpbL2yYb67Qo58p/7Q+xWG1RZvoX1P80aRCHSod2/13aFpxkow1lPO2LEh3c495J0Gwmyjig==",
        license: "MIT",
    }
}

fn reviewed_candidate_source_pin(id: &str) -> Option<ReviewedSourcePin> {
    match id {
        "opencode-process" => Some(ReviewedSourcePin {
            repository: "https://github.com/anomalyco/opencode",
            version: "1.18.21",
            commit: "826d9ad46a22bef0294998e08daa3c4904fea28f",
            package: "opencode-ai",
            package_integrity: "sha512-BxQyxpD0y2X0sXJUKLOooXVmi9QIoeKPtdH68r7QRiqXJ/YulK1MQvSe8KyA8183zoPV0G6JAtgz1OqmE3OGUw==",
            license: "MIT",
        }),
        "pi-sdk-core" => Some(ReviewedSourcePin {
            repository: "https://github.com/earendil-works/pi",
            version: "0.84.2",
            commit: "914cf1472e715297caa30db4b9535d534a9eb718",
            package: "@earendil-works/pi-agent-core",
            package_integrity: "sha512-8Pn3wSCxj0cfo5I6jxQYVB/3uuQRmHhAlEclyjqpOuMEdQMIODHizRogv56FLdbU+dTiGnybeHQ2N+sV1/L2YA==",
            license: "MIT",
        }),
        "pi-process-rpc" => Some(ReviewedSourcePin {
            repository: "https://github.com/earendil-works/pi",
            version: "0.84.2",
            commit: "914cf1472e715297caa30db4b9535d534a9eb718",
            package: "@earendil-works/pi-coding-agent",
            package_integrity: "sha512-l4E+B7hgXKWddRo8bC/eSue2aWZjEgJ9xIpf5p0Og+lq8a2TArCwJ0HCoCPCgaBP/tN4zbYH/wOwvx9pJpeLCA==",
            license: "MIT",
        }),
        "cline-agents" => Some(ReviewedSourcePin {
            repository: "https://github.com/cline/cline",
            version: "0.0.78",
            commit: "be8b984d10d1ad0e9a3917e051ac697f592587d2",
            package: "@cline/agents",
            package_integrity: "sha512-wHVVwtkR4uSTQuAayjs68NZSvN4X+W5tquCMN+ONqDjYnCfuG6BaLlbpwil53DmlxumCm+rFuwbnN0RJ24JWxQ==",
            license: "Apache-2.0",
        }),
        _ => None,
    }
}

fn validate_source(source: &CandidateSource, label: &str) -> Result<(), String> {
    if source.repository.trim().is_empty()
        || source.version.trim().is_empty()
        || source.package.trim().is_empty()
        || !source.package_integrity.starts_with("sha512-")
        || source.license.trim().is_empty()
        || source.reviewed_at != "2026-08-23"
        || source.commit.len() != 40
        || !source.commit.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("{label} has an incomplete source review"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        sequence: u64,
        scenario: AgentEngineScenario,
        kind: AgentEngineEventKind,
    ) -> AgentEngineEvent {
        AgentEngineEvent {
            sequence,
            scenario,
            kind,
        }
    }

    fn budget() -> BakeoffBudget {
        BakeoffBudget {
            requests: 24,
            request_timeout_ms: 120_000,
            max_input_tokens: 16_384,
            max_output_tokens: 2_048,
            max_retries: 2,
            max_worker_restarts: 1,
            max_event_queue: 128,
        }
    }

    #[test]
    fn lifecycle_state_machine_rejects_reopened_stream() {
        let events = vec![
            event(
                1,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamStarted,
            ),
            event(
                2,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamFinished,
            ),
            event(
                3,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamStarted,
            ),
            event(
                4,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamDelta { bytes: 1 },
            ),
        ];

        let error = validate_events(&events, &budget(), 0, 0).unwrap_err();

        assert!(error.contains("duplicated or reopened"));
    }

    #[test]
    fn stream_deltas_must_be_inside_the_stream_lifecycle() {
        let before_start = vec![
            event(
                1,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamDelta { bytes: 1 },
            ),
            event(
                2,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamStarted,
            ),
            event(
                3,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamFinished,
            ),
        ];
        let after_finish = vec![
            event(
                1,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamStarted,
            ),
            event(
                2,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamFinished,
            ),
            event(
                3,
                AgentEngineScenario::Streaming,
                AgentEngineEventKind::StreamDelta { bytes: 1 },
            ),
        ];

        assert!(validate_events(&before_start, &budget(), 0, 0).is_err());
        assert!(validate_events(&after_finish, &budget(), 0, 0).is_err());
    }

    #[test]
    fn failed_tool_execution_is_not_demonstrated() {
        let observation = ScenarioEvidence {
            scenario: AgentEngineScenario::Tools,
            requests_used: 1,
            wall_ms: 10,
            session_restored: true,
        };
        let events = vec![
            event(
                1,
                AgentEngineScenario::Tools,
                AgentEngineEventKind::ToolStarted {
                    call_id: "call-1".to_owned(),
                    tool: "read".to_owned(),
                },
            ),
            event(
                2,
                AgentEngineScenario::Tools,
                AgentEngineEventKind::ToolFinished {
                    call_id: "call-1".to_owned(),
                    tool: "read".to_owned(),
                    succeeded: false,
                },
            ),
        ];

        let error = assess_scenario(&observation, &events, &[]).unwrap_err();

        assert!(error.contains("partial lifecycle evidence"));
    }

    #[test]
    fn each_adapter_receives_only_the_remaining_candidate_budget() {
        let remaining = remaining_budget(&budget(), 20, 1, 1, 120).unwrap();

        assert_eq!(remaining.requests, 4);
        assert_eq!(remaining.max_retries, 1);
        assert_eq!(remaining.max_worker_restarts, 0);
        assert_eq!(remaining.max_event_queue, 8);
        assert!(remaining_budget(&budget(), 25, 0, 0, 0).is_err());
    }

    #[test]
    fn repository_worker_accepts_the_sanitized_alpine_request_boundary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (_, plan) = read_plan(&root.join("config/agent-engine-bakeoff.json")).unwrap();
        let fixture_path = root.join("benchmarks/agent-engine-bakeoff/public-v1/task.json");
        let fixture = read_public_fixture(&fixture_path).unwrap();
        let state = tempfile::tempdir().unwrap();
        let request_path = state.path().join("request.json");
        std::fs::write(
            &request_path,
            serde_json::to_vec(&json!({
                "schema": 1,
                "candidate": "opencode-process",
                "scenario": "streaming",
                "source_version": "1.18.21",
                "base_url": "http://127.0.0.1:1",
                "model_id": plan.inputs.model_id,
                "fixture": fixture_path,
                "effective_policy": effective_policy(&plan, &fixture),
                "state_root": state.path(),
                "budget": {
                    "requests": plan.budget.requests,
                    "request_timeout_ms": plan.budget.request_timeout_ms,
                    "max_input_tokens": plan.budget.max_input_tokens,
                    "max_output_tokens": plan.budget.max_output_tokens,
                    "max_retries": plan.budget.max_retries,
                    "max_worker_restarts": plan.budget.max_worker_restarts,
                    "max_event_queue": plan.budget.max_event_queue
                },
                "prompt_tool_policy": plan.inputs.prompt_tool_policy
            }))
            .unwrap(),
        )
        .unwrap();
        let node = resolve_executable("node").unwrap();
        let mut command = Command::new(node);
        command
            .args([
                root.join("scripts/agent-engine-bakeoff-worker.mjs")
                    .as_os_str(),
                OsStr::new("--candidate-root"),
                state.path().as_os_str(),
                OsStr::new("--request"),
                request_path.as_os_str(),
            ])
            .current_dir(state.path())
            .env_clear()
            .envs(sanitized_environment())
            .env("ALPINE_BAKEOFF_API_KEY", "test-only");

        let output = run_command_bounded(&mut command, Duration::from_secs(10)).unwrap();

        assert!(
            output.status.success(),
            "worker exit: {:?}",
            output.status.code()
        );
        let receipt: AdapterReceipt = serde_json::from_str(output.stdout.trim()).unwrap();
        assert_eq!(receipt.requests_used, 0);
        assert_eq!(receipt.errors[0].code, "no-exact-system-prompt-override");
    }

    #[test]
    fn lifecycle_counts_must_match_observed_retry_budget() {
        let events = vec![
            event(
                1,
                AgentEngineScenario::Retry,
                AgentEngineEventKind::RetryScheduled { attempt: 1 },
            ),
            event(
                2,
                AgentEngineScenario::Retry,
                AgentEngineEventKind::RetryCompleted { attempt: 1 },
            ),
        ];

        let error = validate_events(&events, &budget(), 0, 0).unwrap_err();

        assert!(error.contains("observed retry count"));
    }

    #[test]
    fn partial_events_cannot_be_hidden_behind_an_explicit_failure() {
        let observation = ScenarioEvidence {
            scenario: AgentEngineScenario::Cancellation,
            requests_used: 1,
            wall_ms: 10,
            session_restored: true,
        };
        let events = vec![event(
            1,
            AgentEngineScenario::Cancellation,
            AgentEngineEventKind::CancellationRequested,
        )];
        let errors = vec![AgentEngineError {
            scenario: AgentEngineScenario::Cancellation,
            kind: AgentEngineErrorKind::Cancelled,
            code: "cancel-incomplete".to_owned(),
            retryable: false,
        }];

        let error = assess_scenario(&observation, &events, &errors).unwrap_err();

        assert!(error.contains("partial lifecycle evidence"));
    }

    #[test]
    fn adapter_receipt_rejects_unallowlisted_private_fields() {
        let value = serde_json::json!({
            "schema": 1,
            "candidate": "pi-sdk-core",
            "scenario": "streaming",
            "requests_used": 1,
            "max_input_tokens_observed": 1,
            "max_output_tokens_observed": 1,
            "retries_used": 0,
            "worker_restarts_used": 0,
            "events": [{"type": "stream-started"}],
            "errors": [],
            "raw_prompt": "private"
        });

        let error = serde_json::from_value::<AdapterReceipt>(value).unwrap_err();

        assert!(error.to_string().contains("unknown field `raw_prompt`"));
    }

    #[test]
    fn isolated_package_lock_must_match_every_reviewed_integrity_pin() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (_, plan) = read_plan(&root.join("config/agent-engine-bakeoff.json")).unwrap();
        let isolated = tempfile::tempdir().unwrap();
        let mut packages = serde_json::Map::new();
        packages.insert(String::new(), serde_json::json!({"name": "isolated"}));
        for source in plan
            .candidates
            .iter()
            .map(|candidate| &candidate.source)
            .chain(&plan.supporting_packages)
        {
            let package_root = isolated.path().join("node_modules").join(&source.package);
            std::fs::create_dir_all(&package_root).unwrap();
            std::fs::write(
                package_root.join("package.json"),
                serde_json::to_vec(&serde_json::json!({
                    "name": source.package,
                    "version": source.version
                }))
                .unwrap(),
            )
            .unwrap();
            packages.insert(
                format!("node_modules/{}", source.package),
                serde_json::json!({
                    "version": source.version,
                    "integrity": source.package_integrity
                }),
            );
        }
        for candidate in &plan.candidates {
            for entry in candidate_entry_files(isolated.path(), candidate) {
                std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
                std::fs::write(entry, b"reviewed-entry").unwrap();
            }
        }
        let lock_path = isolated.path().join("package-lock.json");
        std::fs::write(
            &lock_path,
            serde_json::to_vec(&serde_json::json!({
                "lockfileVersion": 3,
                "packages": packages
            }))
            .unwrap(),
        )
        .unwrap();

        let isolated_root = canonical_directory(isolated.path(), "isolated root").unwrap();
        assert!(
            validate_candidate_packages(
                &isolated_root,
                &plan.candidates,
                &plan.supporting_packages
            )
            .is_ok()
        );

        let mut lock: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&lock_path).unwrap()).unwrap();
        lock["packages"]["node_modules/opencode-ai"]["integrity"] =
            serde_json::json!("sha512-tampered");
        std::fs::write(&lock_path, serde_json::to_vec(&lock).unwrap()).unwrap();
        let error = validate_candidate_packages(
            &isolated_root,
            &plan.candidates,
            &plan.supporting_packages,
        )
        .unwrap_err();
        assert!(error.contains("reviewed version and integrity"));
    }

    #[test]
    fn schema_v1_rejects_candidate_source_substitution() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (_, mut plan) = read_plan(&root.join("config/agent-engine-bakeoff.json")).unwrap();
        std::mem::swap(
            &mut plan.candidates[0].source,
            &mut plan.supporting_packages[0],
        );

        let error = validate_plan(&plan).unwrap_err();

        assert!(error.contains("candidate 'opencode-process' source pin changed"));
    }

    #[test]
    fn candidate_state_is_closed_when_the_run_fails() {
        let state = tempfile::tempdir().unwrap();
        let path = state.path().to_path_buf();

        let error = close_candidate_state::<()>(state, &path, Err("primary failure".to_owned()))
            .unwrap_err();

        assert!(error.contains("primary failure"));
        assert!(!path.exists());
    }

    #[test]
    fn schema_v1_rejects_supporting_source_integrity_substitution() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (_, mut plan) = read_plan(&root.join("config/agent-engine-bakeoff.json")).unwrap();
        plan.supporting_packages[0].package_integrity = "sha512-substituted".to_owned();

        let error = validate_plan(&plan).unwrap_err();

        assert!(error.contains("supporting package source pin changed"));
    }

    #[test]
    fn schema_v1_cannot_publish_a_go_recommendation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let (_, mut plan) = read_plan(&root.join("config/agent-engine-bakeoff.json")).unwrap();
        plan.recommendation.decision = "go".to_owned();

        let error = validate_plan(&plan).unwrap_err();

        assert!(error.contains("schema v1 recommendation must remain no-go"));
    }

    #[test]
    fn missing_adapter_receipts_are_incomplete_evidence() {
        assert!(validate_adapter_completion(true, false).is_err());
        assert!(validate_adapter_completion(false, false).is_err());
        assert!(validate_adapter_completion(false, true).is_ok());
    }
}
