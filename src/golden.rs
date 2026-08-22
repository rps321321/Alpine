use crate::config;
use crate::external::{
    self, ExternalEvidenceKind, GoldenAgentEvidence, RecordExternalEvidenceOptions,
    RecordedExternalEvidence,
};
use crate::identity::{sha256_bytes, sha256_file, tree_sha256};
use crate::locking::InterprocessLock;
use crate::opencode::{
    HarnessPolicyOptions, MODEL_ID, assert_effective_policy, harness_environment, harness_policy,
    sanitized_environment,
};
use crate::process::{resolve_executable, run_command_bounded};
use crate::session::{self, AcquireSessionOptions, ReleaseSessionOptions};
use crate::stability::verify_acquisition;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct GoldenAgentOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub database: PathBuf,
    pub result_root: PathBuf,
    pub anchor_run_id: String,
    pub task_id: String,
    pub allow_legacy_identity: bool,
    pub lease_timeout: Duration,
    pub startup_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GoldenAgentReport {
    pub anchor_run_id: String,
    pub profile: String,
    pub task_id: String,
    pub capabilities: Vec<GoldenCapability>,
    pub tool_calls: u64,
    pub tool_failures: u64,
    pub required_failure_matches: u64,
    pub agent_exit_code: i32,
    pub tests_exit_code: i32,
    pub protected_paths_unchanged: bool,
    pub unexpected_files: Vec<String>,
    pub restored_prior_session: bool,
    pub artifact: RecordedExternalEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenTask {
    schema: u32,
    id: String,
    visibility: TaskVisibility,
    capabilities: Vec<GoldenCapability>,
    language: String,
    prompt: String,
    test_command: Vec<String>,
    protected_paths: Vec<String>,
    allowed_changed_paths: Vec<String>,
    timeout_seconds: u64,
    #[serde(default)]
    trace_gate: Option<TraceGate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TaskVisibility {
    Public,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GoldenCapability {
    SingleFileRepair,
    MultiFileTddRepair,
    ToolErrorRecovery,
    ConstraintRetention,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceGate {
    minimum_tool_failures: u64,
    maximum_tool_failures: u64,
    #[serde(default)]
    expected_failure: Option<ExpectedFailure>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFailure {
    tool: String,
    input_path_field: String,
    path: String,
    exact_count: u64,
    require_subsequent_success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolCallEffect {
    call_id: String,
    tool: String,
    input: Value,
    failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentTraceSummary {
    tool_calls: u64,
    tool_failures: u64,
    effects: Vec<ToolCallEffect>,
}

pub fn run(options: &GoldenAgentOptions) -> Result<GoldenAgentReport, String> {
    validate_options(options)?;
    let anchor = external::current_anchor(&options.database, &options.anchor_run_id)?;
    let resolved = config::resolve(&options.install_root, Some(&anchor.summary.profile), true)?;
    let task_root = options
        .repository_root
        .join("benchmarks/golden")
        .join(&options.task_id);
    let task = read_task(&task_root, &options.task_id)?;
    let fixture = task_root.join("fixture");
    if !fixture.is_dir() {
        return Err(format!("golden fixture is missing: {}", fixture.display()));
    }
    let suite_files = walk_files(&task_root)?
        .into_iter()
        .filter(|path| !is_generated(path))
        .collect::<Vec<_>>();
    let suite_sha256 = tree_sha256(&task_root, &suite_files)?;
    let result_root = std::fs::canonicalize(&options.result_root).map_err(|error| {
        format!(
            "failed to resolve result root {}: {error}",
            options.result_root.display()
        )
    })?;
    let temporary = tempfile::Builder::new()
        .prefix("golden-agent-")
        .tempdir_in(&result_root)
        .map_err(|error| format!("failed to create golden worktree: {error}"))?;
    let worktree = temporary.path().join("worktree");
    copy_tree(&fixture, &worktree)?;
    let protected_before = hash_paths(&worktree, &task.protected_paths)?;

    let capacity_path = resolved.install_root.join("logs/inference.lease");
    let _capacity = InterprocessLock::acquire(&capacity_path, options.lease_timeout)
        .map_err(|error| format!("inference capacity is unavailable: {error}"))?;
    let acquisition = session::acquire_under_capacity(&AcquireSessionOptions {
        install_root: options.install_root.clone(),
        profile: Some(anchor.summary.profile.clone()),
        vision: false,
        force_fallback: false,
        allow_legacy_identity: options.allow_legacy_identity,
        lock_timeout: options.lease_timeout,
        startup_timeout: options.startup_timeout,
    })?;
    let prior = acquisition.prior.clone();
    let attempt = execute(
        &anchor.config,
        &resolved,
        &acquisition,
        &task,
        &worktree,
        suite_sha256,
        protected_before,
    );
    let release = session::release_under_capacity(&ReleaseSessionOptions {
        install_root: options.install_root.clone(),
        acquisition,
        keep_server: false,
        lock_timeout: options.lease_timeout,
        startup_timeout: options.startup_timeout,
    });
    let restored = release.and_then(|_| {
        let observed =
            session::snapshot_under_capacity(&options.install_root, options.lease_timeout)?;
        if session::snapshots_materially_equal(&observed, &prior) {
            Ok(())
        } else {
            Err("golden-agent harness did not restore the prior material Session".to_owned())
        }
    });
    let mut evidence = match (attempt, restored) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(attempt_error), Ok(())) => return Err(attempt_error),
        (Ok(_), Err(restoration_error)) => return Err(restoration_error),
        (Err(attempt_error), Err(restoration_error)) => {
            return Err(format!(
                "golden-agent harness failed: {attempt_error}; restoration also failed: {restoration_error}"
            ));
        }
    };
    evidence.restored_prior_session = true;
    let agent_exit_code = evidence.agent_exit_code;
    let tests_exit_code = evidence.tests_exit_code;
    let capabilities = evidence.capabilities.clone();
    let tool_calls = evidence.tool_calls;
    let tool_failures = evidence.tool_failures;
    let required_failure_matches = evidence.required_failure_matches;
    let protected_paths_unchanged = evidence.protected_before == evidence.protected_after;
    let unexpected_files = evidence.unexpected_files.clone();
    let artifact = external::record(&RecordExternalEvidenceOptions {
        repository_root: options.repository_root.clone(),
        database: options.database.clone(),
        result_root: options.result_root.clone(),
        anchor_run_id: options.anchor_run_id.clone(),
        kind: ExternalEvidenceKind::GoldenAgentTaskPass,
        evidence: serde_json::to_value(evidence)
            .map_err(|error| format!("failed to encode golden-agent evidence: {error}"))?,
        reviewed_by: None,
    })?;
    Ok(GoldenAgentReport {
        anchor_run_id: options.anchor_run_id.clone(),
        profile: anchor.summary.profile,
        task_id: options.task_id.clone(),
        capabilities,
        tool_calls,
        tool_failures,
        required_failure_matches,
        agent_exit_code,
        tests_exit_code,
        protected_paths_unchanged,
        unexpected_files,
        restored_prior_session: true,
        artifact,
    })
}

fn execute(
    anchor_config: &Value,
    resolved: &config::ResolvedSession,
    acquisition: &session::SessionAcquisition,
    task: &GoldenTask,
    worktree: &Path,
    suite_sha256: String,
    protected_before: BTreeMap<String, String>,
) -> Result<GoldenAgentEvidence, String> {
    verify_acquisition(anchor_config, acquisition)?;
    let opencode = resolve_executable("opencode")
        .ok_or_else(|| "OpenCode executable is unavailable on PATH".to_owned())?;
    let opencode = std::fs::canonicalize(&opencode)
        .map_err(|error| format!("failed to resolve OpenCode executable: {error}"))?;
    let opencode_sha256 = sha256_file(&opencode)?;
    let policy = harness_policy(
        resolved,
        &HarnessPolicyOptions {
            lean: true,
            skills_enabled: false,
            with_convex: false,
        },
    );
    let policy_bytes = serde_json::to_vec(&policy)
        .map_err(|error| format!("failed to encode OpenCode policy: {error}"))?;
    let policy_json = String::from_utf8(policy_bytes.clone())
        .map_err(|error| format!("OpenCode policy was not UTF-8: {error}"))?;
    let environment = harness_environment(&policy_json, false, false);

    let mut check = Command::new(&opencode);
    check
        .args(["--pure", "debug", "config"])
        .current_dir(worktree)
        .env_clear()
        .envs(environment.clone());
    let checked = run_command_bounded(&mut check, Duration::from_secs(60))
        .map_err(|error| format!("failed to inspect OpenCode policy: {error}"))?;
    if checked.timed_out || !checked.status.success() {
        return Err(format!(
            "OpenCode effective-config check failed: {}",
            checked.stderr.trim()
        ));
    }
    let effective: Value = serde_json::from_str(&checked.stdout)
        .map_err(|error| format!("OpenCode effective config was invalid JSON: {error}"))?;
    assert_effective_policy(&effective, resolved, false)?;

    let timeout = Duration::from_secs(task.timeout_seconds);
    let mut agent = Command::new(&opencode);
    agent
        .args([
            OsStr::new("run"),
            OsStr::new("--pure"),
            OsStr::new("--model"),
            OsStr::new(MODEL_ID),
            OsStr::new("--agent"),
            OsStr::new("build"),
            OsStr::new("--format"),
            OsStr::new("json"),
            OsStr::new("--dir"),
            worktree.as_os_str(),
            OsStr::new(&task.prompt),
        ])
        .current_dir(worktree)
        .env_clear()
        .envs(environment.clone());
    let agent_output = run_command_bounded(&mut agent, timeout)
        .map_err(|error| format!("failed to run OpenCode golden agent: {error}"))?;
    let trace = analyze_agent_trace(&agent_output.stdout)?;
    if trace.tool_calls == 0 {
        return Err("golden task produced no machine-observable tool calls".to_owned());
    }
    let required_failure_matches = task
        .trace_gate
        .as_ref()
        .map(|gate| validate_trace_gate(&trace, gate, worktree))
        .transpose()?
        .unwrap_or(0);

    let test_executable = resolve_executable(&task.test_command[0]).ok_or_else(|| {
        format!(
            "golden test executable is unavailable: {}",
            task.test_command[0]
        )
    })?;
    let test_executable = std::fs::canonicalize(&test_executable)
        .map_err(|error| format!("failed to resolve golden test executable: {error}"))?;
    let test_executable_sha256 = sha256_file(&test_executable)?;
    let mut tests = Command::new(&test_executable);
    tests
        .args(task.test_command.iter().skip(1))
        .current_dir(worktree)
        .env_clear()
        .envs(sanitized_environment());
    let test_output = run_command_bounded(&mut tests, Duration::from_secs(120))
        .map_err(|error| format!("failed to run golden tests: {error}"))?;

    let protected_after = hash_paths(worktree, &task.protected_paths)?;
    let source_files = walk_files(worktree)?
        .into_iter()
        .filter(|path| !is_generated(path))
        .map(|path| {
            path.strip_prefix(worktree)
                .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                .map_err(|error| format!("failed to relativize golden output: {error}"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    let allowed = task
        .allowed_changed_paths
        .iter()
        .chain(task.protected_paths.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let unexpected_files = source_files
        .difference(&allowed)
        .cloned()
        .collect::<Vec<_>>();
    let agent_exit_code = exit_code(&agent_output);
    let tests_exit_code = exit_code(&test_output);
    if agent_exit_code != 0
        || tests_exit_code != 0
        || protected_before != protected_after
        || !unexpected_files.is_empty()
    {
        return Err(format!(
            "golden task failed: agent={agent_exit_code} tests={tests_exit_code} protected={} unexpected={unexpected_files:?}; agent stderr: {}; test stderr: {}",
            protected_before == protected_after,
            agent_output.stderr.trim(),
            test_output.stderr.trim(),
        ));
    }
    Ok(GoldenAgentEvidence {
        schema: 3,
        task_id: task.id.clone(),
        capabilities: task.capabilities.clone(),
        tool_calls: trace.tool_calls,
        tool_failures: trace.tool_failures,
        required_failure_matches,
        suite_sha256,
        opencode_path: opencode,
        opencode_sha256,
        harness_policy_sha256: sha256_bytes(&policy_bytes),
        effective_config_sha256: sha256_bytes(checked.stdout.as_bytes()),
        test_executable_path: test_executable,
        test_executable_sha256,
        agent_exit_code,
        tests_exit_code,
        protected_before,
        protected_after,
        unexpected_files,
        agent_stdout_sha256: sha256_bytes(agent_output.stdout.as_bytes()),
        agent_stderr_sha256: sha256_bytes(agent_output.stderr.as_bytes()),
        tests_stdout_sha256: sha256_bytes(test_output.stdout.as_bytes()),
        tests_stderr_sha256: sha256_bytes(test_output.stderr.as_bytes()),
        restored_prior_session: false,
    })
}

fn read_task(task_root: &Path, expected_id: &str) -> Result<GoldenTask, String> {
    let path = task_root.join("task.json");
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("failed to read golden task {}: {error}", path.display()))?;
    let task: GoldenTask = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid golden task {}: {error}", path.display()))?;
    if task.schema != 2
        || task.id != expected_id
        || task.visibility != TaskVisibility::Public
        || task.capabilities.is_empty()
        || task.capabilities.iter().collect::<BTreeSet<_>>().len() != task.capabilities.len()
        || task.language.trim().is_empty()
        || task.prompt.trim().is_empty()
        || task.test_command.is_empty()
        || task.protected_paths.is_empty()
        || task.allowed_changed_paths.is_empty()
        || task.timeout_seconds == 0
        || task.timeout_seconds > 3600
        || task.trace_gate.as_ref().is_some_and(|gate| {
            gate.minimum_tool_failures > gate.maximum_tool_failures
                || gate.maximum_tool_failures > 100
        })
    {
        return Err("golden task contract is incomplete or out of bounds".to_owned());
    }
    for path in task
        .protected_paths
        .iter()
        .chain(task.allowed_changed_paths.iter())
    {
        validate_relative_path(path)?;
    }
    let requires_recovery = task
        .capabilities
        .contains(&GoldenCapability::ToolErrorRecovery);
    match (
        task.trace_gate
            .as_ref()
            .and_then(|gate| gate.expected_failure.as_ref()),
        requires_recovery,
    ) {
        (Some(expected), true) => {
            if expected.tool.trim().is_empty()
                || expected.input_path_field.trim().is_empty()
                || expected.exact_count == 0
                || !expected.require_subsequent_success
                || task
                    .trace_gate
                    .as_ref()
                    .is_some_and(|gate| expected.exact_count > gate.maximum_tool_failures)
            {
                return Err(
                    "golden recovery effect contract is incomplete or out of bounds".to_owned(),
                );
            }
            validate_relative_path(&expected.path)?;
        }
        (None, true) => {
            return Err(
                "golden tool-error-recovery capability requires an exact failure effect".to_owned(),
            );
        }
        (Some(_), false) => {
            return Err(
                "golden failure effect requires the tool-error-recovery capability".to_owned(),
            );
        }
        (None, false) => {}
    }
    Ok(task)
}

fn analyze_agent_trace(stdout: &str) -> Result<AgentTraceSummary, String> {
    let mut effects = Vec::<ToolCallEffect>::new();
    let mut call_indexes = BTreeMap::<String, usize>::new();
    for (index, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(line).map_err(|error| {
            format!(
                "OpenCode golden trace line {} is invalid JSON: {error}",
                index + 1
            )
        })?;
        if event.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let part = event
            .get("part")
            .ok_or_else(|| "OpenCode golden tool event is missing part".to_owned())?;
        if part.get("type").and_then(Value::as_str) != Some("tool") {
            return Err("OpenCode golden tool event has an invalid part type".to_owned());
        }
        let call_id = part
            .get("callID")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "OpenCode golden tool event is missing callID".to_owned())?;
        let tool = part
            .get("tool")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "OpenCode golden tool event is missing tool".to_owned())?;
        let input = part
            .pointer("/state/input")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| "OpenCode golden tool event has missing or invalid input".to_owned())?;
        let failed = match part.pointer("/state/status").and_then(Value::as_str) {
            Some("completed") => false,
            Some("error") => true,
            Some(other) => {
                return Err(format!(
                    "OpenCode golden tool completion has unsupported status '{other}'"
                ));
            }
            None => return Err("OpenCode golden tool event is missing status".to_owned()),
        };
        let effect = ToolCallEffect {
            call_id: call_id.to_owned(),
            tool: tool.to_owned(),
            input,
            failed,
        };
        if let Some(index) = call_indexes.get(call_id) {
            if effects[*index] != effect {
                return Err(format!(
                    "OpenCode golden trace reused callID '{call_id}' with conflicting tool effects"
                ));
            }
        } else {
            call_indexes.insert(call_id.to_owned(), effects.len());
            effects.push(effect);
        }
    }
    Ok(AgentTraceSummary {
        tool_calls: effects.len() as u64,
        tool_failures: effects.iter().filter(|effect| effect.failed).count() as u64,
        effects,
    })
}

fn validate_trace_gate(
    summary: &AgentTraceSummary,
    gate: &TraceGate,
    worktree: &Path,
) -> Result<u64, String> {
    if summary.tool_failures < gate.minimum_tool_failures
        || summary.tool_failures > gate.maximum_tool_failures
    {
        return Err(format!(
            "golden trace recorded {} tool failures; expected between {} and {}",
            summary.tool_failures, gate.minimum_tool_failures, gate.maximum_tool_failures
        ));
    }
    let Some(expected) = &gate.expected_failure else {
        return Ok(0);
    };
    let matches = summary
        .effects
        .iter()
        .enumerate()
        .filter(|(_, effect)| {
            effect.failed
                && effect.tool == expected.tool
                && effect
                    .input
                    .get(&expected.input_path_field)
                    .and_then(Value::as_str)
                    .is_some_and(|path| {
                        Path::new(path) == Path::new(&expected.path)
                            || Path::new(path) == worktree.join(&expected.path)
                    })
        })
        .collect::<Vec<_>>();
    if matches.len() as u64 != expected.exact_count {
        return Err(format!(
            "golden trace recorded {} matching required failure effects; expected exactly {}",
            matches.len(),
            expected.exact_count
        ));
    }
    if expected.require_subsequent_success {
        let final_match = matches
            .last()
            .map(|(index, _)| *index)
            .unwrap_or(usize::MAX);
        if !summary
            .effects
            .iter()
            .skip(final_match.saturating_add(1))
            .any(|effect| !effect.failed)
        {
            return Err(
                "golden trace did not record a successful tool effect after the required failure"
                    .to_owned(),
            );
        }
    }
    Ok(matches.len() as u64)
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "golden task path must be a simple relative path: {value}"
        ));
    }
    Ok(())
}

fn hash_paths(root: &Path, paths: &[String]) -> Result<BTreeMap<String, String>, String> {
    paths
        .iter()
        .map(|relative| {
            let path = root.join(relative);
            if !path.is_file() {
                return Err(format!(
                    "golden protected path is missing: {}",
                    path.display()
                ));
            }
            Ok((relative.clone(), sha256_file(&path)?))
        })
        .collect()
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destination).map_err(|error| {
        format!(
            "failed to create golden worktree {}: {error}",
            destination.display()
        )
    })?;
    for entry in std::fs::read_dir(source).map_err(|error| {
        format!(
            "failed to read golden fixture {}: {error}",
            source.display()
        )
    })? {
        let entry =
            entry.map_err(|error| format!("failed to read golden fixture entry: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect golden fixture entry: {error}"))?;
        if file_type.is_symlink() {
            return Err(format!(
                "golden fixture must not contain symbolic links: {}",
                path.display()
            ));
        }
        if is_generated(&path) {
            continue;
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&path, &target)?;
        } else if file_type.is_file() {
            std::fs::copy(&path, &target).map_err(|error| {
                format!(
                    "failed to copy golden fixture {} to {}: {error}",
                    path.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        format!(
            "failed to inspect golden suite path {}: {error}",
            root.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "golden suite must not contain symbolic links: {}",
            root.display()
        ));
    }
    if metadata.is_file() {
        return Ok(vec![root.to_path_buf()]);
    }
    if !metadata.is_dir() {
        return Err(format!(
            "golden suite path is neither a file nor a directory: {}",
            root.display()
        ));
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|error| {
        format!(
            "failed to enumerate golden suite path {}: {error}",
            root.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to enumerate golden suite path {}: {error}",
                root.display()
            )
        })?;
        files.extend(walk_files(&entry.path())?);
    }
    Ok(files)
}

fn is_generated(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "__pycache__")
        || path.extension() == Some(OsStr::new("pyc"))
}

fn exit_code(output: &crate::process::ProcessOutput) -> i32 {
    if output.timed_out {
        -2
    } else {
        output.status.code().unwrap_or(-1)
    }
}

fn validate_options(options: &GoldenAgentOptions) -> Result<(), String> {
    if options.anchor_run_id.trim().is_empty() || options.task_id.trim().is_empty() {
        return Err("golden anchor and task ids must not be empty".to_owned());
    }
    validate_relative_path(&options.task_id)?;
    if options.lease_timeout.is_zero() || options.startup_timeout.is_zero() {
        return Err("golden timeouts must be positive".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agent_trace_binds_recovery_effect_and_rejects_duplicate_effects() {
        let trace = [
            json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"failed-read","state":{"status":"error","input":{"filePath":"missing-recovery-sentinel.txt"}}}}),
            json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"failed-read","state":{"status":"error","input":{"filePath":"missing-recovery-sentinel.txt"}}}}),
            json!({"type":"tool_use","part":{"type":"tool","tool":"edit","callID":"repair","state":{"status":"completed","input":{"filePath":"src/app.py"}}}}),
        ]
        .into_iter()
        .map(|event| serde_json::to_string(&event).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
        let summary = analyze_agent_trace(&trace).unwrap();
        assert_eq!(summary.tool_calls, 2);
        assert_eq!(summary.tool_failures, 1);
        let gate = TraceGate {
            minimum_tool_failures: 1,
            maximum_tool_failures: 2,
            expected_failure: Some(ExpectedFailure {
                tool: "read".to_owned(),
                input_path_field: "filePath".to_owned(),
                path: "missing-recovery-sentinel.txt".to_owned(),
                exact_count: 1,
                require_subsequent_success: true,
            }),
        };
        assert_eq!(
            validate_trace_gate(&summary, &gate, Path::new("."),).unwrap(),
            1
        );

        let unrelated = trace.replace("missing-recovery-sentinel.txt", "other-missing.txt");
        assert!(
            validate_trace_gate(
                &analyze_agent_trace(&unrelated).unwrap(),
                &gate,
                Path::new(".")
            )
            .is_err()
        );

        let duplicate_effect = format!(
            "{trace}\n{}",
            serde_json::to_string(&json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"second-failed-read","state":{"status":"error","input":{"filePath":"missing-recovery-sentinel.txt"}}}})).unwrap()
        );
        assert!(
            validate_trace_gate(
                &analyze_agent_trace(&duplicate_effect).unwrap(),
                &gate,
                Path::new(".")
            )
            .is_err()
        );

        let wrong_order = trace.lines().rev().collect::<Vec<_>>().join("\n");
        assert!(
            validate_trace_gate(
                &analyze_agent_trace(&wrong_order).unwrap(),
                &gate,
                Path::new(".")
            )
            .is_err()
        );
    }

    #[test]
    fn agent_trace_rejects_malformed_and_conflicting_tool_events() {
        let malformed = serde_json::to_string(&json!({
            "type":"tool_use", "part":{"type":"tool", "callID":"missing-tool", "state":{"status":"error", "input":{}}}
        })).unwrap();
        assert!(analyze_agent_trace(&malformed).is_err());

        let conflicting = [
            json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"same","state":{"status":"error","input":{"filePath":"missing.txt"}}}}),
            json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"same","state":{"status":"completed","input":{"filePath":"missing.txt"}}}}),
        ].into_iter().map(|event| serde_json::to_string(&event).unwrap()).collect::<Vec<_>>().join("\n");
        assert!(analyze_agent_trace(&conflicting).is_err());
    }

    #[test]
    fn public_v1_task_declares_three_machine_checked_capabilities() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/golden/public-v1");
        let task = read_task(&root, "public-v1").unwrap();
        assert_eq!(task.visibility, TaskVisibility::Public);
        assert_eq!(
            task.capabilities.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                GoldenCapability::MultiFileTddRepair,
                GoldenCapability::ToolErrorRecovery,
                GoldenCapability::ConstraintRetention,
            ])
        );
        assert!(task.trace_gate.is_some());
        let expected = task.trace_gate.unwrap().expected_failure.unwrap();
        assert_eq!(expected.tool, "read");
        assert_eq!(expected.path, "missing-recovery-sentinel.txt");
        assert_eq!(expected.exact_count, 1);
        assert!(expected.require_subsequent_success);
    }

    #[test]
    fn public_v1_fixture_tests_reject_defects_and_accept_the_exact_repair() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("benchmarks/golden/public-v1");
        let task = read_task(&root, "public-v1").unwrap();
        let temporary = tempfile::tempdir().unwrap();
        copy_tree(&root.join("fixture"), temporary.path()).unwrap();
        let run_tests = || {
            let executable = resolve_executable(&task.test_command[0]).unwrap();
            let mut command = Command::new(executable);
            command
                .args(task.test_command.iter().skip(1))
                .current_dir(temporary.path())
                .env_clear()
                .envs(sanitized_environment());
            run_command_bounded(&mut command, Duration::from_secs(30)).unwrap()
        };
        assert!(!run_tests().status.success());

        std::fs::write(
            temporary.path().join("src/pricing.py"),
            concat!(
                "def subtotal_cents(unit_cents: int, quantity: int) -> int:\n",
                "    if unit_cents < 0:\n        raise ValueError(\"unit_cents must be non-negative\")\n",
                "    if quantity < 0:\n        raise ValueError(\"quantity must be non-negative\")\n",
                "    return unit_cents * quantity\n",
            ),
        )
        .unwrap();
        std::fs::write(
            temporary.path().join("src/rendering.py"),
            concat!(
                "from src.pricing import subtotal_cents\n\n",
                "def render_receipt(unit_cents: int, quantity: int) -> str:\n",
                "    subtotal = subtotal_cents(unit_cents, quantity)\n",
                "    return f\"ALPINE-PUBLIC-V1|items={quantity}|subtotal={subtotal}\"\n",
            ),
        )
        .unwrap();
        assert!(run_tests().status.success());
    }

    #[test]
    fn policy_keeps_core_capabilities_and_gates_only_effects() {
        let root = PathBuf::from(r"C:\fixture");
        let resolved = config::ResolvedSession {
            install_root: root.clone(),
            session_config_path: root.join("session.json"),
            session_config_sha256: "a".repeat(64),
            session: config::SessionConfig {
                schema: 1,
                root: root.clone(),
                host: "127.0.0.1".to_owned(),
                port: 8100,
                active_profile: Some("turbo-16k".to_owned()),
                runtimes: BTreeMap::new(),
                model: root.join("model"),
                mmproj: root.join("mmproj"),
                chat_template: root.join("template"),
                api_key_file: root.join("api-key.txt"),
                base_url_file: root.join("base-url.txt"),
                state_file: root.join("state.json"),
                cleanup: Value::Null,
            },
            profile_path: root.join("profile.json"),
            profile_sha256: "b".repeat(64),
            profile_name: "turbo-16k".to_owned(),
            profile: config::Profile {
                name: "turbo-16k".to_owned(),
                runtime: "custom".to_owned(),
                context: 16_384,
                output: 4096,
                parallel: 1,
                threads: 16,
                batch_size: 2048,
                ubatch_size: 768,
                kv_cache: "q8_0".to_owned(),
                tensor_cpu_through_block: 43,
                mtp_depth: 3,
                ngram_mod: true,
                ngram_reset_on_begin: true,
                external_skills: false,
                skill_tool: false,
                vision_fit: true,
                fit_target_mib: 512,
            },
            runtime_name: "custom".to_owned(),
            server: root.join("server.exe"),
            model: root.join("model"),
            mmproj: root.join("mmproj"),
            chat_template: root.join("template"),
            api_key_file: root.join("api-key.txt"),
            base_url_file: root.join("base-url.txt"),
            state_file: root.join("state.json"),
            base_url: "http://127.0.0.1:8100".to_owned(),
        };
        let policy = harness_policy(
            &resolved,
            &HarnessPolicyOptions {
                lean: true,
                skills_enabled: false,
                with_convex: false,
            },
        );
        assert_eq!(policy.pointer("/permission/skill"), Some(&json!("deny")));
        assert_eq!(policy.pointer("/permission/task"), Some(&json!("allow")));
        assert_eq!(
            policy.pointer("/permission/todowrite"),
            Some(&json!("allow"))
        );
        assert_eq!(
            policy.pointer("/permission/bash/git push *"),
            Some(&json!("ask"))
        );
        assert_eq!(
            policy.pointer("/permission/bash/git remote *"),
            Some(&json!("allow"))
        );
    }
}
