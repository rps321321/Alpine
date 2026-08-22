use crate::config;
use crate::identity::sha256_file;
use crate::opencode::{
    HarnessPolicyOptions, MODEL_ID, harness_environment, harness_policy, sanitized_environment,
};
use crate::process::{resolve_executable, run_command_bounded};
use crate::session::{self, ProcessIdentityStrength};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use uuid::Uuid;

const TARGET_FILE: &str = "benchmark-target.txt";
const SENTINEL_FILE: &str = "benchmark-sentinel.txt";
const SENTINEL: &str = "alpine-harness-read-only\n";
const PI_API_KEY_ENV: &str = "ALPINE_PI_API_KEY";

#[derive(Debug, Clone)]
pub struct HarnessBenchmarkOptions {
    pub install_root: PathBuf,
    pub project: PathBuf,
    pub profile: Option<String>,
    pub runs: u32,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessBenchmarkReport {
    pub schema: u32,
    pub profile: String,
    pub profile_sha256: String,
    pub session_config_sha256: String,
    pub session_identity: String,
    pub context_tokens: u32,
    pub output_tokens: u32,
    pub task: String,
    pub runs: u32,
    pub results: Vec<HarnessSample>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessSample {
    pub harness: String,
    pub run: u32,
    pub position_in_pair: u8,
    pub wall_ms: u128,
    pub exit_code: i32,
    pub timed_out: bool,
    /// Uncached input tokens reported by the harness.
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Normalized input work: uncached input + cache reads + cache writes.
    pub prompt_tokens: Option<u64>,
    pub tool_calls: u64,
    pub tool_failures: u64,
    pub expected_answer: bool,
    pub executable_path: PathBuf,
    pub executable_sha256: String,
}

pub fn run(options: &HarnessBenchmarkOptions) -> Result<HarnessBenchmarkReport, String> {
    if options.runs == 0 || options.runs > 20 {
        return Err("harness benchmark runs must be between 1 and 20".to_owned());
    }
    let project = std::fs::canonicalize(&options.project)
        .map_err(|error| format!("failed to resolve benchmark project: {error}"))?;
    if !project.is_dir() {
        return Err("benchmark project must be a directory".to_owned());
    }

    let resolved = config::resolve(&options.install_root, options.profile.as_deref(), true)?;
    let status = verify_session_identity(&resolved)?;
    let session_identity = status
        .transaction_id
        .ok_or_else(|| "verified Inference Session has no transaction identity".to_owned())?;
    let api_key = std::fs::read_to_string(&resolved.api_key_file)
        .map_err(|error| format!("failed to read local API key: {error}"))?;
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("local API key is empty".to_owned());
    }

    let pi = canonical_executable("pi")?;
    let opencode = canonical_executable("opencode")?;
    let temporary = tempfile::Builder::new()
        .prefix("alpine-harness-")
        .tempdir()
        .map_err(|error| format!("failed to create temporary harness root: {error}"))?;
    let fixture = temporary.path().join("fixture");
    std::fs::create_dir(&fixture)
        .map_err(|error| format!("failed to create temporary benchmark fixture: {error}"))?;
    std::fs::write(fixture.join(SENTINEL_FILE), SENTINEL)
        .map_err(|error| format!("failed to create benchmark sentinel: {error}"))?;
    write_pi_config(temporary.path(), &resolved)?;

    let policy = read_only_opencode_policy(&resolved)?;
    let policy_json = serde_json::to_string(&policy)
        .map_err(|error| format!("failed to encode OpenCode policy: {error}"))?;
    let opencode_environment = harness_environment(&policy_json, false, false);

    let mut results = Vec::new();
    for run in 1..=options.runs {
        let expected = format!("ALPINE-HARNESS-{}", Uuid::new_v4().simple());
        std::fs::write(fixture.join(TARGET_FILE), format!("{expected}\n"))
            .map_err(|error| format!("failed to prepare benchmark target: {error}"))?;
        let prompt = format!(
            "Benchmark pair {run}. Use exactly one read tool call to read {TARGET_FILE}. Read no other path. Then report only the exact text contained in that file."
        );

        let pi_run = || {
            run_pi(
                &pi,
                temporary.path(),
                &fixture,
                &prompt,
                &expected,
                api_key,
                run,
                if run % 2 == 1 { 1 } else { 2 },
                options.request_timeout,
            )
        };
        let opencode_run = || {
            run_opencode(
                &opencode,
                &opencode_environment,
                &fixture,
                &prompt,
                &expected,
                run,
                if run % 2 == 1 { 2 } else { 1 },
                options.request_timeout,
            )
        };
        if run % 2 == 1 {
            results.push(pi_run()?);
            verify_fixture(&fixture, &expected)?;
            results.push(opencode_run()?);
        } else {
            results.push(opencode_run()?);
            verify_fixture(&fixture, &expected)?;
            results.push(pi_run()?);
        }
        verify_fixture(&fixture, &expected)?;
    }

    Ok(HarnessBenchmarkReport {
        schema: 2,
        profile: resolved.profile_name,
        profile_sha256: resolved.profile_sha256,
        session_config_sha256: resolved.session_config_sha256,
        session_identity,
        context_tokens: resolved.profile.context,
        output_tokens: resolved.profile.output,
        task: "isolated-read-and-exact-report".to_owned(),
        runs: options.runs,
        results,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_pi(
    executable: &Path,
    config_dir: &Path,
    project: &Path,
    prompt: &str,
    expected: &str,
    api_key: &str,
    run: u32,
    position_in_pair: u8,
    timeout: Duration,
) -> Result<HarnessSample, String> {
    let mut command = Command::new(executable);
    command
        .args([
            OsStr::new("--provider"),
            OsStr::new("alpine-local"),
            OsStr::new("--model"),
            OsStr::new(MODEL_ID),
            OsStr::new("--no-session"),
            OsStr::new("--no-extensions"),
            OsStr::new("--no-skills"),
            OsStr::new("--no-prompt-templates"),
            OsStr::new("--no-context-files"),
            OsStr::new("--tools"),
            OsStr::new("read"),
            OsStr::new("--mode"),
            OsStr::new("json"),
            OsStr::new("--print"),
            OsStr::new(prompt),
        ])
        .current_dir(project)
        .env_clear()
        .envs(sanitized_environment())
        .env("PI_CODING_AGENT_DIR", config_dir)
        .env("PI_TELEMETRY", "0")
        .env("PI_OFFLINE", "1")
        .env(PI_API_KEY_ENV, api_key);
    measure(
        HarnessKind::Pi,
        executable,
        run,
        position_in_pair,
        expected,
        &mut command,
        timeout,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_opencode(
    executable: &Path,
    environment: &[(std::ffi::OsString, std::ffi::OsString)],
    project: &Path,
    prompt: &str,
    expected: &str,
    run: u32,
    position_in_pair: u8,
    timeout: Duration,
) -> Result<HarnessSample, String> {
    let mut command = Command::new(executable);
    command
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
            project.as_os_str(),
            OsStr::new(prompt),
        ])
        .current_dir(project)
        .env_clear()
        .envs(environment.iter().cloned());
    measure(
        HarnessKind::OpenCode,
        executable,
        run,
        position_in_pair,
        expected,
        &mut command,
        timeout,
    )
}

#[derive(Clone, Copy)]
enum HarnessKind {
    Pi,
    OpenCode,
}

impl HarnessKind {
    fn name(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::OpenCode => "opencode",
        }
    }
}

fn measure(
    harness: HarnessKind,
    executable: &Path,
    run: u32,
    position_in_pair: u8,
    expected: &str,
    command: &mut Command,
    timeout: Duration,
) -> Result<HarnessSample, String> {
    let started = Instant::now();
    let output = run_command_bounded(command, timeout)
        .map_err(|error| format!("failed to run {} benchmark: {error}", harness.name()))?;
    let metrics = parse_metrics(harness, &output.stdout)?;
    let exact_answer = metrics
        .final_text
        .as_deref()
        .is_some_and(|text| text.trim() == expected);
    let tool_protocol_failures = u64::from(metrics.tool_calls != 1);
    Ok(HarnessSample {
        harness: harness.name().to_owned(),
        run,
        position_in_pair,
        wall_ms: started.elapsed().as_millis(),
        exit_code: output.status.code().unwrap_or(-1),
        timed_out: output.timed_out,
        input_tokens: Some(metrics.input),
        output_tokens: Some(metrics.output),
        cache_read_tokens: Some(metrics.cache_read),
        cache_write_tokens: Some(metrics.cache_write),
        prompt_tokens: Some(metrics.prompt_tokens()?),
        tool_calls: metrics.tool_calls,
        tool_failures: metrics.tool_failures + tool_protocol_failures,
        expected_answer: exact_answer,
        executable_path: executable.to_owned(),
        executable_sha256: sha256_file(executable)?,
    })
}

#[derive(Default)]
struct Metrics {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    usage_events: u64,
    tool_calls: u64,
    tool_failures: u64,
    final_text: Option<String>,
}

impl Metrics {
    fn add_usage(&mut self, usage: &Value, cache_path: &str) -> Result<(), String> {
        self.input = checked_add(self.input, required_u64(usage, "/input")?)?;
        self.output = checked_add(self.output, required_u64(usage, "/output")?)?;
        let (cache_read_path, cache_write_path) = if cache_path.is_empty() {
            ("/cacheRead".to_owned(), "/cacheWrite".to_owned())
        } else {
            (format!("{cache_path}/read"), format!("{cache_path}/write"))
        };
        self.cache_read = checked_add(self.cache_read, required_u64(usage, &cache_read_path)?)?;
        self.cache_write = checked_add(self.cache_write, required_u64(usage, &cache_write_path)?)?;
        self.usage_events += 1;
        Ok(())
    }

    fn prompt_tokens(&self) -> Result<u64, String> {
        checked_add(checked_add(self.input, self.cache_read)?, self.cache_write)
    }

    fn validate(self, harness: &str) -> Result<Self, String> {
        if self.usage_events == 0 {
            return Err(format!("{harness} emitted no complete usage events"));
        }
        if self.final_text.is_none() {
            return Err(format!("{harness} emitted no final assistant text"));
        }
        Ok(self)
    }
}

fn parse_metrics(harness: HarnessKind, stdout: &str) -> Result<Metrics, String> {
    let events = parse_ndjson(stdout, harness.name())?;
    match harness {
        HarnessKind::Pi => parse_pi_metrics(&events),
        HarnessKind::OpenCode => parse_opencode_metrics(&events),
    }
}

fn parse_ndjson(stdout: &str, harness: &str) -> Result<Vec<Value>, String> {
    let mut events = Vec::new();
    for (index, line) in stdout.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str::<Value>(line).map_err(|error| {
            format!(
                "{harness} emitted malformed NDJSON on line {}: {error}",
                index + 1
            )
        })?;
        if value.get("type").and_then(Value::as_str).is_none() {
            return Err(format!(
                "{harness} emitted an event without a type on line {}",
                index + 1
            ));
        }
        events.push(value);
    }
    if events.is_empty() {
        return Err(format!("{harness} emitted no JSON events"));
    }
    Ok(events)
}

fn parse_pi_metrics(events: &[Value]) -> Result<Metrics, String> {
    let mut metrics = Metrics::default();
    let mut tools = BTreeMap::<String, bool>::new();
    for event in events {
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "message_end" => {
                let message = event
                    .get("message")
                    .ok_or_else(|| "Pi message_end event is missing message".to_owned())?;
                if message.get("role").and_then(Value::as_str) == Some("assistant") {
                    let usage = message
                        .get("usage")
                        .ok_or_else(|| "Pi assistant message is missing usage".to_owned())?;
                    metrics.add_usage(usage, "")?;
                    metrics.final_text = assistant_content_text(message)?;
                }
            }
            "tool_execution_end" => {
                let id = required_str(event, "/toolCallId")?.to_owned();
                if required_str(event, "/toolName")? != "read" {
                    return Err("Pi used a tool other than read".to_owned());
                }
                let failed = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "Pi tool completion is missing isError".to_owned())?;
                tools
                    .entry(id)
                    .and_modify(|prior| *prior |= failed)
                    .or_insert(failed);
            }
            _ => {}
        }
    }
    metrics.tool_calls = tools.len() as u64;
    metrics.tool_failures = tools.values().filter(|failed| **failed).count() as u64;
    metrics.validate("Pi")
}

fn parse_opencode_metrics(events: &[Value]) -> Result<Metrics, String> {
    let mut metrics = Metrics::default();
    let mut tools = BTreeMap::<String, bool>::new();
    let mut final_message_id: Option<String> = None;
    for event in events {
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "step_finish" => {
                let tokens = event
                    .pointer("/part/tokens")
                    .ok_or_else(|| "OpenCode step_finish event is missing tokens".to_owned())?;
                metrics.add_usage(tokens, "/cache")?;
            }
            "tool_use" => {
                let part = event
                    .get("part")
                    .ok_or_else(|| "OpenCode tool_use event is missing part".to_owned())?;
                if required_str(part, "/type")? != "tool" || required_str(part, "/tool")? != "read"
                {
                    return Err("OpenCode used a tool other than read".to_owned());
                }
                let id = required_str(part, "/callID")?.to_owned();
                let failed = match required_str(part, "/state/status")? {
                    "completed" => false,
                    "error" => true,
                    other => {
                        return Err(format!(
                            "OpenCode tool completion has unsupported status '{other}'"
                        ));
                    }
                };
                tools
                    .entry(id)
                    .and_modify(|prior| *prior |= failed)
                    .or_insert(failed);
            }
            "text" => {
                let message_id = required_str(event, "/part/messageID")?.to_owned();
                let text = required_str(event, "/part/text")?;
                if final_message_id.as_deref() != Some(&message_id) {
                    final_message_id = Some(message_id);
                    metrics.final_text = Some(String::new());
                }
                metrics
                    .final_text
                    .as_mut()
                    .expect("initialized above")
                    .push_str(text);
            }
            _ => {}
        }
    }
    metrics.tool_calls = tools.len() as u64;
    metrics.tool_failures = tools.values().filter(|failed| **failed).count() as u64;
    metrics.validate("OpenCode")
}

fn assistant_content_text(message: &Value) -> Result<Option<String>, String> {
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| "Pi assistant message is missing content".to_owned())?;
    let mut result = String::new();
    let mut found = false;
    for item in content {
        if item.get("type").and_then(Value::as_str) == Some("text") {
            result.push_str(required_str(item, "/text")?);
            found = true;
        }
    }
    Ok(found.then_some(result))
}

fn required_u64(value: &Value, pointer: &str) -> Result<u64, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("usage field '{pointer}' is missing or invalid"))
}

fn required_str<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("event field '{pointer}' is missing or invalid"))
}

fn checked_add(left: u64, right: u64) -> Result<u64, String> {
    left.checked_add(right)
        .ok_or_else(|| "token accounting overflowed".to_owned())
}

fn read_only_opencode_policy(resolved: &config::ResolvedSession) -> Result<Value, String> {
    let mut policy = harness_policy(
        resolved,
        &HarnessPolicyOptions {
            lean: true,
            skills_enabled: false,
            with_convex: false,
        },
    );
    let permission = policy
        .get_mut("permission")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "OpenCode harness policy has no permission object".to_owned())?;
    let read = permission
        .remove("read")
        .ok_or_else(|| "OpenCode harness policy has no read permission".to_owned())?;
    let mut restricted = Map::new();
    restricted.insert("*".to_owned(), json!("deny"));
    restricted.insert("read".to_owned(), read);
    for capability in permission.keys() {
        restricted.insert(capability.clone(), json!("deny"));
    }
    policy["permission"] = Value::Object(restricted);
    Ok(policy)
}

fn write_pi_config(root: &Path, resolved: &config::ResolvedSession) -> Result<(), String> {
    let value = json!({"providers":{"alpine-local":{
        "baseUrl": format!("{}/v1", resolved.base_url), "api":"openai-completions", "apiKey":format!("${PI_API_KEY_ENV}"),
        "compat":{"supportsDeveloperRole":false,"supportsReasoningEffort":false,"maxTokensField":"max_tokens"},
        "models":[{"id":MODEL_ID,"reasoning":false,"contextWindow":resolved.profile.context,"maxTokens":resolved.profile.output,"samplingParams":{"temperature":0}}]
    }}});
    let bytes = serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("failed to encode temporary Pi model config: {error}"))?;
    std::fs::write(root.join("models.json"), bytes)
        .map_err(|error| format!("failed to write temporary Pi model config: {error}"))
}

fn verify_session_identity(
    resolved: &config::ResolvedSession,
) -> Result<session::SessionStatus, String> {
    let status = session::status(&resolved.install_root, Duration::from_secs(10))?;
    if !status.active
        || !status.healthy
        || status.foreign
        || status.identity_strength != ProcessIdentityStrength::Verified
        || status.profile != resolved.profile_name
        || status.runtime != resolved.runtime_name
        || status.fallback.is_some()
    {
        return Err(format!(
            "live Inference Session does not exactly match requested Profile '{}': active={}, healthy={}, foreign={}, actual_profile='{}', actual_runtime='{}', fallback={:?}, identity={:?}",
            resolved.profile_name,
            status.active,
            status.healthy,
            status.foreign,
            status.profile,
            status.runtime,
            status.fallback,
            status.identity_strength
        ));
    }
    let observed = status
        .process_path
        .as_deref()
        .ok_or_else(|| "verified Inference Session has no process path".to_owned())?;
    let observed = std::fs::canonicalize(observed)
        .map_err(|error| format!("failed to resolve live runtime path: {error}"))?;
    let expected = std::fs::canonicalize(&resolved.server)
        .map_err(|error| format!("failed to resolve configured runtime path: {error}"))?;
    if observed != expected {
        return Err("live Inference Session runtime does not match requested Profile".to_owned());
    }
    Ok(status)
}

fn verify_fixture(root: &Path, expected: &str) -> Result<(), String> {
    let target = std::fs::read_to_string(root.join(TARGET_FILE))
        .map_err(|error| format!("benchmark target was removed or unreadable: {error}"))?;
    let sentinel = std::fs::read_to_string(root.join(SENTINEL_FILE))
        .map_err(|error| format!("benchmark sentinel was removed or unreadable: {error}"))?;
    if target != format!("{expected}\n") || sentinel != SENTINEL {
        return Err("a harness modified the read-only benchmark fixture".to_owned());
    }
    let entries = std::fs::read_dir(root)
        .map_err(|error| format!("failed to inspect benchmark fixture: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect benchmark fixture entry: {error}"))?;
    if entries.len() != 2 {
        return Err("a harness created an unexpected file in the benchmark fixture".to_owned());
    }
    Ok(())
}

fn canonical_executable(name: &str) -> Result<PathBuf, String> {
    let path = resolve_executable(name)
        .ok_or_else(|| format!("{name} executable is unavailable on PATH"))?;
    std::fs::canonicalize(&path)
        .map_err(|error| format!("failed to resolve {name} executable: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pi_message(text: Option<&str>, input: u64, output: u64) -> Value {
        let content = text
            .map(|text| vec![json!({"type":"text","text":text})])
            .unwrap_or_else(|| {
                vec![json!({"type":"toolCall","toolCallId":"call-1","toolName":"read","input":{}})]
            });
        json!({"type":"message_end","message":{"role":"assistant","content":content,"usage":{
            "input":input,"output":output,"cacheRead":3,"cacheWrite":4
        }}})
    }

    #[test]
    fn pi_uses_only_final_assistant_text_and_deduplicates_tool_completion() {
        let events = vec![
            pi_message(None, 10, 2),
            json!({"type":"tool_execution_end","toolCallId":"call-1","toolName":"read","isError":false,"result":{"content":[{"type":"text","text":"EXPECTED"}]}}),
            json!({"type":"tool_execution_end","toolCallId":"call-1","toolName":"read","isError":false,"result":{}}),
            pi_message(Some("WRONG"), 5, 1),
        ];
        let metrics = parse_pi_metrics(&events).unwrap();
        assert_eq!(metrics.final_text.as_deref(), Some("WRONG"));
        assert_eq!(metrics.tool_calls, 1);
        assert_eq!(metrics.tool_failures, 0);
        assert_eq!(metrics.input, 15);
        assert_eq!(metrics.prompt_tokens().unwrap(), 29);
    }

    #[test]
    fn pi_reads_top_level_tool_error() {
        let events = vec![
            json!({"type":"tool_execution_end","toolCallId":"call-1","toolName":"read","isError":true,"result":{}}),
            pi_message(Some("answer"), 5, 1),
        ];
        let metrics = parse_pi_metrics(&events).unwrap();
        assert_eq!(metrics.tool_calls, 1);
        assert_eq!(metrics.tool_failures, 1);
    }

    #[test]
    fn opencode_parses_completed_calls_final_text_and_normalized_usage() {
        let events = vec![
            json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"call-1","state":{"status":"completed"}}}),
            json!({"type":"step_finish","part":{"tokens":{"input":11,"output":2,"cache":{"read":3,"write":4}}}}),
            json!({"type":"text","part":{"messageID":"message-2","text":"EXPECTED"}}),
            json!({"type":"step_finish","part":{"tokens":{"input":5,"output":1,"cache":{"read":7,"write":0}}}}),
        ];
        let metrics = parse_opencode_metrics(&events).unwrap();
        assert_eq!(metrics.final_text.as_deref(), Some("EXPECTED"));
        assert_eq!(metrics.tool_calls, 1);
        assert_eq!(metrics.tool_failures, 0);
        assert_eq!(metrics.input, 16);
        assert_eq!(metrics.output, 3);
        assert_eq!(metrics.prompt_tokens().unwrap(), 30);
    }

    #[test]
    fn opencode_deduplicates_calls_and_records_error_status() {
        let events = vec![
            json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"call-1","state":{"status":"completed"}}}),
            json!({"type":"tool_use","part":{"type":"tool","tool":"read","callID":"call-1","state":{"status":"error"}}}),
            json!({"type":"text","part":{"messageID":"message-2","text":"answer"}}),
            json!({"type":"step_finish","part":{"tokens":{"input":1,"output":1,"cache":{"read":0,"write":0}}}}),
        ];
        let metrics = parse_opencode_metrics(&events).unwrap();
        assert_eq!(metrics.tool_calls, 1);
        assert_eq!(metrics.tool_failures, 1);
    }

    #[test]
    fn parser_rejects_malformed_ndjson_and_missing_usage() {
        assert!(parse_ndjson("{not-json}\n", "Pi").is_err());
        let events = parse_ndjson(
            "{\"type\":\"text\",\"part\":{\"messageID\":\"m\",\"text\":\"answer\"}}\n",
            "OpenCode",
        )
        .unwrap();
        assert!(parse_opencode_metrics(&events).is_err());
    }

    #[test]
    fn fixture_verification_rejects_mutation_and_extra_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(TARGET_FILE), "EXPECTED\n").unwrap();
        std::fs::write(root.path().join(SENTINEL_FILE), SENTINEL).unwrap();
        verify_fixture(root.path(), "EXPECTED").unwrap();
        std::fs::write(root.path().join("unexpected.txt"), "bad").unwrap();
        assert!(verify_fixture(root.path(), "EXPECTED").is_err());
    }

    #[test]
    fn pi_config_reference_is_not_a_shell_command() {
        let serialized =
            serde_json::to_string(&json!({"apiKey": format!("${PI_API_KEY_ENV}")})).unwrap();
        assert!(serialized.contains(PI_API_KEY_ENV));
        assert!(!serialized.contains("!node"));
        assert!(!serialized.contains("!cmd"));
    }
}
