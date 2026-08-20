use crate::config::{self, Profile, ResolvedSession, SessionConfig};
use crate::locking::InterprocessLock;
use crate::process::{resolve_executable, run_bounded};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionAction {
    Refuse,
    Start,
    Reuse,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessIdentityStrength {
    Verified,
    LegacyCompatible,
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InferenceArguments {
    pub profile: String,
    pub vision: bool,
    pub fallback: Option<String>,
    pub arguments: Vec<String>,
    pub environment: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionStatus {
    pub active: bool,
    pub foreign: bool,
    pub healthy: bool,
    pub profile: String,
    pub vision: bool,
    pub runtime: String,
    pub pid: Option<u32>,
    pub process_path: Option<PathBuf>,
    pub expected_path: PathBuf,
    pub fallback: Option<String>,
    pub phase: Option<String>,
    pub transaction_id: Option<String>,
    pub process_start_epoch_secs: Option<u64>,
    pub identity_strength: ProcessIdentityStrength,
    pub identity_failures: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SessionState {
    #[allow(dead_code)]
    #[serde(default)]
    schema: Option<u32>,
    #[serde(default)]
    transaction_id: Option<String>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    server: Option<PathBuf>,
    #[serde(default)]
    vision: bool,
    #[serde(default)]
    fallback: Option<String>,
    #[serde(default)]
    process_start_epoch_secs: Option<u64>,
}

#[derive(Debug, Clone)]
struct ObservedProcess {
    pid: u32,
    path: Option<PathBuf>,
    arguments: Vec<String>,
    start_epoch_secs: u64,
}

pub fn resolve_action(
    current: &SessionStatus,
    requested_profile: &str,
    requested_vision: bool,
) -> SessionAction {
    if current.foreign {
        SessionAction::Refuse
    } else if !current.active {
        SessionAction::Start
    } else if current.healthy
        && current.profile == requested_profile
        && current.vision == requested_vision
    {
        SessionAction::Reuse
    } else {
        SessionAction::Replace
    }
}

pub fn plan_arguments(
    install_root: &Path,
    profile: Option<&str>,
    vision: bool,
    force_fallback: bool,
) -> Result<InferenceArguments, String> {
    let resolved = config::resolve(install_root, profile, true)?;
    build_arguments(&resolved.session, &resolved.profile, vision, force_fallback)
}

fn build_arguments(
    session: &SessionConfig,
    profile: &Profile,
    vision: bool,
    force_fallback: bool,
) -> Result<InferenceArguments, String> {
    let path = |value: &Path| value.to_string_lossy().into_owned();
    let mut arguments = vec![
        "-m".to_owned(),
        path(&session.model),
        "--host".to_owned(),
        session.host.clone(),
        "--port".to_owned(),
        session.port.to_string(),
        "-c".to_owned(),
        profile.context.to_string(),
        "-np".to_owned(),
        profile.parallel.to_string(),
        "--threads".to_owned(),
        profile.threads.to_string(),
        "--threads-batch".to_owned(),
        profile.threads.to_string(),
        "-b".to_owned(),
        profile.batch_size.to_string(),
        "-ub".to_owned(),
        profile.ubatch_size.to_string(),
        "--no-webui".to_owned(),
        "--jinja".to_owned(),
        "--chat-template-file".to_owned(),
        path(&session.chat_template),
        "--api-key-file".to_owned(),
        path(&session.api_key_file),
        "-fa".to_owned(),
        "on".to_owned(),
        "-ctk".to_owned(),
        profile.kv_cache.clone(),
        "-ctv".to_owned(),
        profile.kv_cache.clone(),
        "--reasoning".to_owned(),
        "off".to_owned(),
    ];

    if vision {
        arguments.extend([
            "--mmproj".to_owned(),
            path(&session.mmproj),
            "--fit".to_owned(),
            "on".to_owned(),
            "--fit-ctx".to_owned(),
            profile.context.to_string(),
            "--fit-target".to_owned(),
            profile.fit_target_mib.to_string(),
        ]);
    } else {
        arguments.extend([
            "-ngl".to_owned(),
            "all".to_owned(),
            "--fit".to_owned(),
            "off".to_owned(),
            "-ot".to_owned(),
            tensor_override(profile.tensor_cpu_through_block)?,
            "--load-mode".to_owned(),
            "none".to_owned(),
        ]);
    }

    let optimized_ngram = !force_fallback && profile.ngram_mod;
    let spec_type = if optimized_ngram {
        "draft-mtp,ngram-mod"
    } else {
        "draft-mtp"
    };
    arguments.extend([
        "--spec-type".to_owned(),
        spec_type.to_owned(),
        "--spec-draft-n-max".to_owned(),
        profile.mtp_depth.to_string(),
    ]);
    if optimized_ngram {
        arguments.extend([
            "--spec-ngram-mod-n-match".to_owned(),
            "24".to_owned(),
            "--spec-ngram-mod-n-min".to_owned(),
            "16".to_owned(),
            "--spec-ngram-mod-n-max".to_owned(),
            "64".to_owned(),
        ]);
    }

    let mut environment = BTreeMap::new();
    environment.insert(
        "LLAMA_NGRAM_MOD_RESET_ON_BEGIN".to_owned(),
        (!force_fallback && profile.ngram_reset_on_begin).then(|| "1".to_owned()),
    );
    Ok(InferenceArguments {
        profile: profile.name.clone(),
        vision,
        fallback: force_fallback.then(|| "mtp-only".to_owned()),
        arguments,
        environment,
    })
}

fn tensor_override(through_block: u32) -> Result<String, String> {
    const MAX_TENSOR_OVERRIDE_BLOCK: u32 = 4096;
    if through_block > MAX_TENSOR_OVERRIDE_BLOCK {
        return Err(format!(
            "tensor_cpu_through_block {through_block} exceeds the bounded argument-planning limit {MAX_TENSOR_OVERRIDE_BLOCK}"
        ));
    }
    let blocks = (0..=through_block)
        .map(|block| block.to_string())
        .collect::<Vec<_>>()
        .join("|");
    Ok(format!(r"blk\.({blocks})\.ffn_.*=CPU"))
}

pub fn status(install_root: &Path, lock_timeout: Duration) -> Result<SessionStatus, String> {
    let resolved = config::resolve(install_root, None, false)?;
    let session_lock_path = lock_path(&resolved.state_file, ".session.lock");
    let _session_lock = InterprocessLock::acquire(&session_lock_path, lock_timeout)?;
    status_locked(&resolved, lock_timeout)
}

fn status_locked(
    resolved: &ResolvedSession,
    lock_timeout: Duration,
) -> Result<SessionStatus, String> {
    let state = read_state(&resolved.state_file, lock_timeout)?;
    let listener_pid = listener_pid(resolved.session.port, Duration::from_secs(10))?;
    let observed = listener_pid.map(observe_process).transpose()?;
    let (owned, identity_strength, identity_failures) =
        classify_identity(resolved.session.port, state.as_ref(), observed.as_ref());
    let healthy = owned && health_is_ok(&resolved.base_url, Duration::from_secs(3));
    let state_profile = state
        .as_ref()
        .and_then(|value| value.profile.as_deref())
        .filter(|value| !value.is_empty());
    let state_runtime = state
        .as_ref()
        .and_then(|value| value.runtime.as_deref())
        .filter(|value| !value.is_empty());
    let expected_path = state
        .as_ref()
        .and_then(|value| value.server.clone())
        .unwrap_or_else(|| resolved.server.clone());

    Ok(SessionStatus {
        active: owned,
        foreign: observed.is_some() && !owned,
        healthy,
        profile: state_profile.unwrap_or(&resolved.profile_name).to_owned(),
        vision: state.as_ref().is_some_and(|value| value.vision),
        runtime: state_runtime.unwrap_or(&resolved.runtime_name).to_owned(),
        pid: observed.as_ref().map(|value| value.pid),
        process_path: observed.as_ref().and_then(|value| value.path.clone()),
        expected_path,
        fallback: state.as_ref().and_then(|value| value.fallback.clone()),
        phase: state.as_ref().and_then(|value| value.phase.clone()),
        transaction_id: state
            .as_ref()
            .and_then(|value| value.transaction_id.clone()),
        process_start_epoch_secs: observed.as_ref().map(|value| value.start_epoch_secs),
        identity_strength,
        identity_failures,
    })
}

fn lock_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn read_state(path: &Path, lock_timeout: Duration) -> Result<Option<SessionState>, String> {
    let write_lock_path = lock_path(path, ".write.lock");
    let _write_lock = InterprocessLock::acquire(&write_lock_path, lock_timeout)?;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "session state missing or unreadable at {}: {error}",
                path.display()
            ));
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("malformed session state {}: {error}", path.display()))
}

fn listener_pid(port: u16, timeout: Duration) -> Result<Option<u32>, String> {
    let executable = system_netstat()?;
    let output = run_bounded(
        &executable,
        &[
            OsStr::new("-a"),
            OsStr::new("-n"),
            OsStr::new("-o"),
            OsStr::new("-p"),
            OsStr::new("tcp"),
        ],
        timeout,
    )
    .map_err(|error| format!("failed to inspect listeners with netstat: {error}"))?;
    if output.timed_out {
        return Err("netstat listener inspection timed out".to_owned());
    }
    if !output.status.success() {
        return Err(format!(
            "netstat listener inspection failed: {}",
            output.stderr.trim()
        ));
    }
    parse_listener_pids(&output.stdout, port)
}

fn system_netstat() -> Result<PathBuf, String> {
    if cfg!(windows) {
        let system_root = std::env::var_os("SystemRoot").ok_or_else(|| {
            "SystemRoot is unavailable; cannot locate system netstat.exe".to_owned()
        })?;
        let executable = PathBuf::from(system_root).join("System32/netstat.exe");
        if executable.is_file() {
            return Ok(executable);
        }
        return Err(format!(
            "system netstat.exe is unavailable at {}",
            executable.display()
        ));
    }
    resolve_executable("netstat")
        .ok_or_else(|| "netstat is required to inspect the inference listener".to_owned())
}

fn parse_listener_pids(output: &str, port: u16) -> Result<Option<u32>, String> {
    let mut pids = BTreeSet::new();
    for line in output.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5
            || !fields[0].eq_ignore_ascii_case("TCP")
            || !fields[3].eq_ignore_ascii_case("LISTENING")
            || address_port(fields[1]) != Some(port)
        {
            continue;
        }
        let pid = fields[4]
            .parse::<u32>()
            .map_err(|_| format!("netstat returned an invalid listener PID: {}", fields[4]))?;
        pids.insert(pid);
    }
    match pids.len() {
        0 => Ok(None),
        1 => Ok(pids.into_iter().next()),
        _ => Err(format!(
            "multiple processes are listening on inference port {port}: {pids:?}"
        )),
    }
}

fn address_port(address: &str) -> Option<u16> {
    address.rsplit_once(':')?.1.parse().ok()
}

fn observe_process(pid: u32) -> Result<ObservedProcess, String> {
    let sysinfo_pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sysinfo_pid]),
        true,
        ProcessRefreshKind::everything(),
    );
    let process = system
        .process(sysinfo_pid)
        .ok_or_else(|| format!("listener PID {pid} exited before its identity could be read"))?;
    Ok(ObservedProcess {
        pid,
        path: process.exe().map(Path::to_path_buf),
        arguments: process
            .cmd()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
        start_epoch_secs: process.start_time(),
    })
}

fn classify_identity(
    port: u16,
    state: Option<&SessionState>,
    process: Option<&ObservedProcess>,
) -> (bool, ProcessIdentityStrength, Vec<String>) {
    let Some(process) = process else {
        return (false, ProcessIdentityStrength::Unverified, Vec::new());
    };
    let Some(state) = state else {
        return (
            false,
            ProcessIdentityStrength::Unverified,
            vec!["session state is absent".to_owned()],
        );
    };
    let mut failures = Vec::new();
    if state.pid != Some(process.pid) {
        failures.push("listener PID does not match session state".to_owned());
    }
    match (state.server.as_deref(), process.path.as_deref()) {
        (Some(expected), Some(actual)) if paths_equal(expected, actual) => {}
        (Some(_), Some(_)) => {
            failures.push("listener executable does not match session state".to_owned())
        }
        (None, _) => failures.push("session state has no server path".to_owned()),
        (_, None) => failures.push("listener executable path is unavailable".to_owned()),
    }
    if !command_line_has_port(&process.arguments, port) {
        failures.push("listener command line does not contain the configured port".to_owned());
    }
    if let Some(expected_start) = state.process_start_epoch_secs {
        if expected_start != process.start_epoch_secs {
            failures.push("listener start time does not match session state".to_owned());
        }
    }
    if !failures.is_empty() {
        return (false, ProcessIdentityStrength::Unverified, failures);
    }
    let strength = if state.process_start_epoch_secs.is_some() {
        ProcessIdentityStrength::Verified
    } else {
        ProcessIdentityStrength::LegacyCompatible
    };
    (true, strength, failures)
}

fn command_line_has_port(arguments: &[String], port: u16) -> bool {
    let expected = port.to_string();
    arguments.iter().enumerate().any(|(index, argument)| {
        (argument.eq_ignore_ascii_case("--port") && arguments.get(index + 1) == Some(&expected))
            || argument.split_once('=').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("--port") && value == expected
            })
    })
}

fn paths_equal(expected: &Path, observed: &Path) -> bool {
    let normalize = |path: &Path| {
        let rendered = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        if !cfg!(windows) {
            return rendered;
        }
        if let Some(unc) = rendered.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}").replace('/', "\\")
        } else {
            rendered
                .strip_prefix(r"\\?\")
                .unwrap_or(&rendered)
                .replace('/', "\\")
        }
    };
    if cfg!(windows) {
        normalize(expected).eq_ignore_ascii_case(&normalize(observed))
    } else {
        normalize(expected) == normalize(observed)
    }
}

fn health_is_ok(base_url: &str, timeout: Duration) -> bool {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into();
    agent.get(&format!("{base_url}/health")).call().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProfileStatus;

    fn fixture_profile() -> Profile {
        Profile {
            name: "turbo-16k".to_owned(),
            status: ProfileStatus::Candidate,
            runtime: "custom".to_owned(),
            context: 16384,
            output: 4096,
            parallel: 1,
            threads: 16,
            batch_size: 2048,
            ubatch_size: 768,
            kv_cache: "q8_0".to_owned(),
            tensor_cpu_through_block: 2,
            mtp_depth: 3,
            ngram_mod: true,
            ngram_reset_on_begin: true,
            external_skills: false,
            skill_tool: false,
            vision_fit: true,
            fit_target_mib: 512,
        }
    }

    fn fixture_session() -> SessionConfig {
        SessionConfig {
            schema: 3,
            root: PathBuf::from(r"C:\fixture"),
            host: "127.0.0.1".to_owned(),
            port: 8100,
            active_profile: "stable-16k".to_owned(),
            runtimes: BTreeMap::new(),
            model: PathBuf::from(r"C:\fixture\model.gguf"),
            mmproj: PathBuf::from(r"C:\fixture\mmproj.gguf"),
            chat_template: PathBuf::from(r"C:\fixture\chat.jinja"),
            api_key_file: PathBuf::from(r"C:\fixture\key.txt"),
            base_url_file: PathBuf::from(r"C:\fixture\url.txt"),
            state_file: PathBuf::from(r"C:\fixture\state.json"),
            cleanup: serde_json::Value::Null,
        }
    }

    #[test]
    fn argument_plan_matches_the_pinned_optimized_contract() {
        let plan = build_arguments(&fixture_session(), &fixture_profile(), false, false).unwrap();
        assert_eq!(
            plan.arguments,
            vec![
                "-m",
                r"C:\fixture\model.gguf",
                "--host",
                "127.0.0.1",
                "--port",
                "8100",
                "-c",
                "16384",
                "-np",
                "1",
                "--threads",
                "16",
                "--threads-batch",
                "16",
                "-b",
                "2048",
                "-ub",
                "768",
                "--no-webui",
                "--jinja",
                "--chat-template-file",
                r"C:\fixture\chat.jinja",
                "--api-key-file",
                r"C:\fixture\key.txt",
                "-fa",
                "on",
                "-ctk",
                "q8_0",
                "-ctv",
                "q8_0",
                "--reasoning",
                "off",
                "-ngl",
                "all",
                "--fit",
                "off",
                "-ot",
                r"blk\.(0|1|2)\.ffn_.*=CPU",
                "--load-mode",
                "none",
                "--spec-type",
                "draft-mtp,ngram-mod",
                "--spec-draft-n-max",
                "3",
                "--spec-ngram-mod-n-match",
                "24",
                "--spec-ngram-mod-n-min",
                "16",
                "--spec-ngram-mod-n-max",
                "64"
            ]
        );
        assert_eq!(
            plan.environment["LLAMA_NGRAM_MOD_RESET_ON_BEGIN"],
            Some("1".to_owned())
        );
    }

    #[test]
    fn fallback_and_vision_are_explicit_and_do_not_mix_tensor_placement() {
        let plan = build_arguments(&fixture_session(), &fixture_profile(), true, true).unwrap();
        assert_eq!(plan.fallback.as_deref(), Some("mtp-only"));
        assert!(
            plan.arguments
                .windows(2)
                .any(|pair| pair == ["--fit", "on"])
        );
        assert!(plan.arguments.iter().any(|value| value == "--mmproj"));
        assert!(!plan.arguments.iter().any(|value| value == "-ot"));
        assert!(
            plan.arguments
                .windows(2)
                .any(|pair| pair == ["--spec-type", "draft-mtp"])
        );
        assert_eq!(plan.environment["LLAMA_NGRAM_MOD_RESET_ON_BEGIN"], None);
    }

    #[test]
    fn listener_parser_deduplicates_dual_stack_and_refuses_multiple_owners() {
        let dual_stack = "\
  TCP    127.0.0.1:8100       0.0.0.0:0       LISTENING       4900\n\
  TCP    [::1]:8100           [::]:0          LISTENING       4900\n";
        assert_eq!(parse_listener_pids(dual_stack, 8100).unwrap(), Some(4900));
        let conflicting = format!("{dual_stack}  TCP  0.0.0.0:8100  0.0.0.0:0  LISTENING  12\n");
        assert!(
            parse_listener_pids(&conflicting, 8100)
                .unwrap_err()
                .contains("multiple")
        );
    }

    #[test]
    fn process_identity_requires_pid_executable_port_and_new_start_time() {
        let state = SessionState {
            schema: Some(1),
            transaction_id: Some("tx".to_owned()),
            phase: Some("healthy".to_owned()),
            pid: Some(4900),
            profile: Some("fast-32k".to_owned()),
            runtime: Some("custom".to_owned()),
            server: Some(PathBuf::from(r"C:\runtime\server.exe")),
            vision: false,
            fallback: None,
            process_start_epoch_secs: None,
        };
        let process = ObservedProcess {
            pid: 4900,
            path: Some(PathBuf::from(r"C:\runtime\server.exe")),
            arguments: vec!["server.exe".to_owned(), "--port=8100".to_owned()],
            start_epoch_secs: 100,
        };
        let (owned, strength, failures) = classify_identity(8100, Some(&state), Some(&process));
        assert!(owned);
        assert_eq!(strength, ProcessIdentityStrength::LegacyCompatible);
        assert!(failures.is_empty());

        let mut new_state = state.clone();
        new_state.process_start_epoch_secs = Some(99);
        let (owned, strength, failures) = classify_identity(8100, Some(&new_state), Some(&process));
        assert!(!owned);
        assert_eq!(strength, ProcessIdentityStrength::Unverified);
        assert!(failures.iter().any(|value| value.contains("start time")));
    }

    #[test]
    fn action_planner_matches_the_legacy_state_machine() {
        let mut status = SessionStatus {
            active: false,
            foreign: false,
            healthy: false,
            profile: "stable-16k".to_owned(),
            vision: false,
            runtime: "official".to_owned(),
            pid: None,
            process_path: None,
            expected_path: PathBuf::from("server"),
            fallback: None,
            phase: None,
            transaction_id: None,
            process_start_epoch_secs: None,
            identity_strength: ProcessIdentityStrength::Unverified,
            identity_failures: Vec::new(),
        };
        assert_eq!(
            resolve_action(&status, "stable-16k", false),
            SessionAction::Start
        );
        status.foreign = true;
        assert_eq!(
            resolve_action(&status, "stable-16k", false),
            SessionAction::Refuse
        );
        status.foreign = false;
        status.active = true;
        status.healthy = true;
        assert_eq!(
            resolve_action(&status, "stable-16k", false),
            SessionAction::Reuse
        );
        assert_eq!(
            resolve_action(&status, "turbo-16k", false),
            SessionAction::Replace
        );
    }
}
