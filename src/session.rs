use crate::clock::UtcTimestamp;
use crate::config::{self, Profile, ResolvedSession, SessionConfig};
use crate::identity::sha256_file;
use crate::locking::InterprocessLock;
use crate::process::{resolve_executable, run_bounded};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use uuid::Uuid;

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

#[derive(Debug, Clone)]
pub struct StartSessionOptions {
    pub install_root: PathBuf,
    pub profile: Option<String>,
    pub vision: bool,
    pub force_fallback: bool,
    pub lock_timeout: Duration,
    pub startup_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct StopSessionOptions {
    pub install_root: PathBuf,
    pub lock_timeout: Duration,
    pub allow_legacy_identity: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartSessionReport {
    pub started: bool,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StopSessionReport {
    pub stopped: bool,
    pub stopped_pid: Option<u32>,
    pub status: SessionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionState {
    #[serde(default = "legacy_state_schema")]
    schema: u32,
    #[serde(default)]
    transaction_id: Option<String>,
    #[serde(default)]
    phase: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    stopped_at: Option<String>,
    #[serde(default)]
    pid: Option<u32>,
    #[serde(default)]
    process_started_at: Option<String>,
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
    #[serde(default)]
    server_sha256: Option<String>,
    #[serde(default)]
    runtime_build_sha256: Option<String>,
    #[serde(default)]
    profile_sha256: Option<String>,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    environment: BTreeMap<String, Option<String>>,
    #[serde(default)]
    cleanup_paused: bool,
    #[serde(default)]
    cleanup_pid: Option<u32>,
    #[serde(default)]
    healthy_at: Option<String>,
    #[serde(default)]
    failed: Option<String>,
    #[serde(default)]
    failed_at: Option<String>,
    #[serde(default)]
    cleanup_restore_failed: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

const fn legacy_state_schema() -> u32 {
    1
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            schema: legacy_state_schema(),
            transaction_id: None,
            phase: None,
            started_at: None,
            stopped_at: None,
            pid: None,
            process_started_at: None,
            profile: None,
            runtime: None,
            server: None,
            vision: false,
            fallback: None,
            process_start_epoch_secs: None,
            server_sha256: None,
            runtime_build_sha256: None,
            profile_sha256: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            cleanup_paused: false,
            cleanup_pid: None,
            healthy_at: None,
            failed: None,
            failed_at: None,
            cleanup_restore_failed: None,
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CleanupConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    exe: Option<PathBuf>,
    #[serde(default)]
    start_script: Option<PathBuf>,
    #[serde(default)]
    health: Option<String>,
}

#[derive(Debug, Clone)]
struct ObservedProcess {
    pid: u32,
    path: Option<PathBuf>,
    arguments: Vec<String>,
    start_epoch_secs: u64,
}

trait SessionPlatform {
    fn listener(&self, port: u16, timeout: Duration) -> Result<Option<ObservedProcess>, String>;
    fn observe(&self, pid: u32) -> Result<Option<ObservedProcess>, String>;
    fn health_is_ok(&self, url: &str, timeout: Duration) -> bool;
    fn spawn(
        &self,
        executable: &Path,
        arguments: &[String],
        environment: &BTreeMap<String, Option<String>>,
        stdout: &Path,
        stderr: &Path,
    ) -> Result<ObservedProcess, String>;
    fn terminate(&self, pid: u32, expected_start_epoch_secs: u64) -> Result<(), String>;
    fn run_cleanup_script(&self, script: &Path, timeout: Duration) -> Result<(), String>;
    fn sleep(&self, duration: Duration);
    fn utc_now(&self) -> Result<UtcTimestamp, String>;
}

#[derive(Debug, Clone, Copy)]
struct SystemPlatform;

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

pub fn start(options: &StartSessionOptions) -> Result<StartSessionReport, String> {
    const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
    if options.startup_timeout > MAX_STARTUP_TIMEOUT {
        return Err("startup timeout may not exceed 30 minutes".to_owned());
    }
    let resolved = config::resolve(&options.install_root, options.profile.as_deref(), true)?;
    let capacity_path = resolved.install_root.join("logs/inference.lease");
    let _capacity = InterprocessLock::acquire(&capacity_path, options.lock_timeout)
        .map_err(|error| format!("inference capacity is unavailable: {error}"))?;
    let session_lock_path = lock_path(&resolved.state_file, ".session.lock");
    let _session_lock = InterprocessLock::acquire(&session_lock_path, options.lock_timeout)?;
    start_locked_with(&resolved, options, &SystemPlatform)
}

fn start_locked_with<P: SessionPlatform>(
    resolved: &ResolvedSession,
    options: &StartSessionOptions,
    platform: &P,
) -> Result<StartSessionReport, String> {
    let current = status_locked_with(resolved, options.lock_timeout, platform)?;
    match resolve_action(&current, &resolved.profile_name, options.vision) {
        SessionAction::Reuse => {
            return Ok(StartSessionReport {
                started: false,
                status: current,
            });
        }
        SessionAction::Refuse => {
            return Err(format!(
                "port {} is occupied by an unrecognized listener; refusing to steal it",
                resolved.session.port
            ));
        }
        SessionAction::Replace => {
            return Err(format!(
                "a different or unhealthy owned inference session is active; replace it transactionally before starting {}",
                resolved.profile_name
            ));
        }
        SessionAction::Start => {}
    }

    validate_start_artifacts(resolved, options.vision)?;
    let cleanup = cleanup_config(&resolved.session)?;
    ensure_local_api_key(&resolved.api_key_file, options.lock_timeout)?;
    atomic_replace(
        &resolved.base_url_file,
        format!("{}/v1", resolved.base_url).as_bytes(),
    )?;

    let now = platform.utc_now()?;
    let initial_plan = build_arguments(
        &resolved.session,
        &resolved.profile,
        options.vision,
        options.force_fallback,
    )?;
    let profile_path = resolved
        .install_root
        .join("profiles")
        .join(format!("{}.json", resolved.profile_name));
    let build_manifest = resolved
        .server
        .parent()
        .map(|parent| parent.join("build-manifest.json"));
    let mut state = SessionState {
        schema: 2,
        transaction_id: Some(Uuid::new_v4().simple().to_string()),
        phase: Some("starting".to_owned()),
        started_at: Some(now.rfc3339()),
        stopped_at: None,
        pid: None,
        process_started_at: None,
        profile: Some(resolved.profile_name.clone()),
        runtime: Some(resolved.runtime_name.clone()),
        server: Some(resolved.server.clone()),
        vision: options.vision,
        fallback: initial_plan.fallback.clone(),
        process_start_epoch_secs: None,
        server_sha256: Some(sha256_file(&resolved.server)?),
        runtime_build_sha256: build_manifest
            .filter(|path| path.is_file())
            .map(|path| sha256_file(&path))
            .transpose()?,
        profile_sha256: Some(sha256_file(&profile_path)?),
        arguments: initial_plan.arguments.clone(),
        environment: initial_plan.environment.clone(),
        cleanup_paused: false,
        cleanup_pid: None,
        healthy_at: None,
        failed: None,
        failed_at: None,
        cleanup_restore_failed: None,
        extra: BTreeMap::new(),
    };
    save_state(&resolved.state_file, &state, options.lock_timeout)?;

    if let Err(error) = pause_cleanup(
        cleanup.as_ref(),
        &mut state,
        &resolved.state_file,
        options.lock_timeout,
        platform,
    ) {
        let cleanup_error = restore_cleanup(cleanup.as_ref(), &mut state, platform).err();
        let combined = [Some(error), cleanup_error.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        mark_start_failed(
            &mut state,
            &resolved.state_file,
            options.lock_timeout,
            &combined,
            cleanup_error.as_deref(),
            platform,
        )?;
        return Err(combined);
    }

    let logs = resolved.install_root.join("logs");
    let stdout = logs.join("session-out.log");
    let stderr = logs.join("session-err.log");
    let mut running: Option<ObservedProcess> = None;
    let result = run_start_attempts(
        resolved,
        options,
        &stdout,
        &stderr,
        &mut state,
        &mut running,
        platform,
    )
    .and_then(|()| {
        state.phase = Some("healthy".to_owned());
        state.healthy_at = Some(platform.utc_now()?.rfc3339());
        save_state(&resolved.state_file, &state, options.lock_timeout)?;
        let status = status_locked_with(resolved, options.lock_timeout, platform)?;
        if !status.active
            || !status.healthy
            || status.identity_strength != ProcessIdentityStrength::Verified
        {
            return Err(
                "post-start verification did not prove a healthy, exactly identified session"
                    .to_owned(),
            );
        }
        Ok(status)
    });
    let status = match result {
        Ok(status) => status,
        Err(start_error) => {
            let termination_error = running.as_ref().and_then(|process| {
                platform
                    .terminate(process.pid, process.start_epoch_secs)
                    .err()
            });
            let port_error = if termination_error.is_none() {
                wait_port_free(resolved.session.port, Duration::from_secs(30), platform).err()
            } else {
                None
            };
            let cleanup_error = if termination_error.is_none() && port_error.is_none() {
                restore_cleanup(cleanup.as_ref(), &mut state, platform).err()
            } else {
                Some(
                    "cleanup was not restored because inference termination was not proven"
                        .to_owned(),
                )
            };
            let combined = [
                Some(start_error),
                termination_error,
                port_error,
                cleanup_error.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            mark_start_failed(
                &mut state,
                &resolved.state_file,
                options.lock_timeout,
                &combined,
                cleanup_error.as_deref(),
                platform,
            )?;
            return Err(combined);
        }
    };
    Ok(StartSessionReport {
        started: true,
        status,
    })
}

#[allow(clippy::too_many_arguments)]
fn run_start_attempts<P: SessionPlatform>(
    resolved: &ResolvedSession,
    options: &StartSessionOptions,
    stdout: &Path,
    stderr: &Path,
    state: &mut SessionState,
    running: &mut Option<ObservedProcess>,
    platform: &P,
) -> Result<(), String> {
    let mut fallback = options.force_fallback;
    loop {
        let plan = build_arguments(
            &resolved.session,
            &resolved.profile,
            options.vision,
            fallback,
        )?;
        state.phase = Some("starting".to_owned());
        state.pid = None;
        state.process_started_at = None;
        state.process_start_epoch_secs = None;
        state.fallback = plan.fallback.clone();
        state.arguments = plan.arguments.clone();
        state.environment = plan.environment.clone();
        save_state(&resolved.state_file, state, options.lock_timeout)?;

        let process = platform.spawn(
            &resolved.server,
            &plan.arguments,
            &plan.environment,
            stdout,
            stderr,
        )?;
        let identity_matches = process
            .path
            .as_deref()
            .is_some_and(|path| paths_equal(&resolved.server, path))
            && command_line_has_port(&process.arguments, resolved.session.port);
        if !identity_matches {
            platform.terminate(process.pid, process.start_epoch_secs)?;
            return Err(
                "spawned process identity does not match the planned runtime and port".to_owned(),
            );
        }
        if let Err(error) = verify_spawn_inputs_unchanged(resolved, state) {
            platform.terminate(process.pid, process.start_epoch_secs)?;
            return Err(error);
        }
        *running = Some(process.clone());
        state.pid = Some(process.pid);
        state.process_start_epoch_secs = Some(process.start_epoch_secs);
        state.process_started_at = Some(epoch_seconds_rfc3339(process.start_epoch_secs)?);
        save_state(&resolved.state_file, state, options.lock_timeout)?;

        if wait_started_healthy(resolved, state, options.startup_timeout, platform)? {
            return Ok(());
        }
        let failed_process = running
            .take()
            .ok_or_else(|| "started process identity was lost".to_owned())?;
        platform.terminate(failed_process.pid, failed_process.start_epoch_secs)?;
        wait_port_free(resolved.session.port, Duration::from_secs(30), platform)?;
        if fallback {
            return Err("pinned MTP-only inference start failed health verification".to_owned());
        }
        fallback = true;
    }
}

fn verify_spawn_inputs_unchanged(
    resolved: &ResolvedSession,
    state: &SessionState,
) -> Result<(), String> {
    let runtime_hash = sha256_file(&resolved.server)?;
    if state.server_sha256.as_deref() != Some(&runtime_hash) {
        return Err(
            "runtime executable changed between identity capture and process start".to_owned(),
        );
    }
    let profile_path = resolved
        .install_root
        .join("profiles")
        .join(format!("{}.json", resolved.profile_name));
    let profile_hash = sha256_file(&profile_path)?;
    if state.profile_sha256.as_deref() != Some(&profile_hash) {
        return Err("Profile changed between argument planning and process start".to_owned());
    }
    Ok(())
}

pub fn stop(options: &StopSessionOptions) -> Result<StopSessionReport, String> {
    let resolved = config::resolve(&options.install_root, None, false)?;
    let capacity_path = resolved.install_root.join("logs/inference.lease");
    let _capacity = InterprocessLock::acquire(&capacity_path, options.lock_timeout)
        .map_err(|error| format!("inference capacity is unavailable: {error}"))?;
    let session_lock_path = lock_path(&resolved.state_file, ".session.lock");
    let _session_lock = InterprocessLock::acquire(&session_lock_path, options.lock_timeout)?;
    stop_locked_with(&resolved, options, &SystemPlatform)
}

fn stop_locked_with<P: SessionPlatform>(
    resolved: &ResolvedSession,
    options: &StopSessionOptions,
    platform: &P,
) -> Result<StopSessionReport, String> {
    let current = status_locked_with(resolved, options.lock_timeout, platform)?;
    if current.foreign {
        return Err(format!(
            "port {} belongs to PID {}; refusing to terminate an unowned listener",
            resolved.session.port,
            current
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_owned())
        ));
    }
    if current.active
        && current.identity_strength == ProcessIdentityStrength::LegacyCompatible
        && !options.allow_legacy_identity
    {
        return Err(
            "the active session has legacy process identity; repeat with explicit legacy-identity authorization for the one-time migration"
                .to_owned(),
        );
    }

    let cleanup = cleanup_config(&resolved.session)?;
    let mut state = read_state(&resolved.state_file, options.lock_timeout)?;
    let stopped_pid = if current.active {
        let pid = current
            .pid
            .ok_or_else(|| "owned session has no observed PID".to_owned())?;
        let start = current
            .process_start_epoch_secs
            .ok_or_else(|| "owned session has no observed process start time".to_owned())?;
        platform.terminate(pid, start)?;
        if let Err(error) = wait_port_free(resolved.session.port, Duration::from_secs(30), platform)
        {
            if let Some(state) = state.as_mut() {
                state.phase = Some("failed".to_owned());
                state.failed = Some(error.clone());
                state.failed_at = Some(platform.utc_now()?.rfc3339());
                state.cleanup_restore_failed = Some(
                    "cleanup was not restored because inference termination was not proven"
                        .to_owned(),
                );
                save_state(&resolved.state_file, state, options.lock_timeout)?;
            }
            return Err(error);
        }
        Some(pid)
    } else {
        None
    };

    if let Some(state) = state.as_mut() {
        if let Err(error) = restore_cleanup(cleanup.as_ref(), state, platform) {
            state.phase = Some("failed".to_owned());
            state.cleanup_restore_failed = Some(error.clone());
            state.failed = Some(error.clone());
            state.failed_at = Some(platform.utc_now()?.rfc3339());
            save_state(&resolved.state_file, state, options.lock_timeout)?;
            return Err(error);
        }
        state.phase = Some("stopped".to_owned());
        state.stopped_at = Some(platform.utc_now()?.rfc3339());
        save_state(&resolved.state_file, state, options.lock_timeout)?;
    }
    let status = status_locked_with(resolved, options.lock_timeout, platform)?;
    if status.active || status.foreign {
        return Err("post-stop verification found a listener on the inference port".to_owned());
    }
    Ok(StopSessionReport {
        stopped: stopped_pid.is_some(),
        stopped_pid,
        status,
    })
}

fn validate_start_artifacts(resolved: &ResolvedSession, vision: bool) -> Result<(), String> {
    for path in [&resolved.server, &resolved.model, &resolved.chat_template] {
        if !path.is_file() {
            return Err(format!(
                "required inference artifact is missing: {}",
                path.display()
            ));
        }
    }
    if vision && !resolved.mmproj.is_file() {
        return Err(format!(
            "required vision projector is missing: {}",
            resolved.mmproj.display()
        ));
    }
    Ok(())
}

fn cleanup_config(session: &SessionConfig) -> Result<Option<CleanupConfig>, String> {
    if session.cleanup.is_null() {
        return Ok(None);
    }
    let cleanup: CleanupConfig = serde_json::from_value(session.cleanup.clone())
        .map_err(|error| format!("invalid cleanup configuration: {error}"))?;
    if !cleanup.enabled {
        return Ok(None);
    }
    let port = cleanup
        .port
        .filter(|port| *port > 0)
        .ok_or_else(|| "enabled cleanup configuration requires a valid port".to_owned())?;
    let executable = cleanup
        .exe
        .as_deref()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            "enabled cleanup configuration requires an absolute executable path".to_owned()
        })?;
    let script = cleanup
        .start_script
        .as_deref()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            "enabled cleanup configuration requires an absolute start script".to_owned()
        })?;
    if !executable.is_file() {
        return Err(format!(
            "cleanup executable is unavailable: {}",
            executable.display()
        ));
    }
    if !script.is_file() {
        return Err(format!(
            "cleanup start script is unavailable: {}",
            script.display()
        ));
    }
    let health = cleanup
        .health
        .as_deref()
        .ok_or_else(|| "enabled cleanup configuration requires a health endpoint".to_owned())?;
    let allowed = [
        format!("http://127.0.0.1:{port}/"),
        format!("http://localhost:{port}/"),
        format!("http://[::1]:{port}/"),
    ];
    if !allowed.iter().any(|prefix| health.starts_with(prefix)) {
        return Err(format!(
            "cleanup health endpoint must use configured loopback port {port}"
        ));
    }
    Ok(Some(cleanup))
}

fn pause_cleanup<P: SessionPlatform>(
    cleanup: Option<&CleanupConfig>,
    state: &mut SessionState,
    state_path: &Path,
    lock_timeout: Duration,
    platform: &P,
) -> Result<(), String> {
    let Some(cleanup) = cleanup else {
        return Ok(());
    };
    let port = cleanup.port.expect("validated cleanup port");
    let Some(process) = platform.listener(port, Duration::from_secs(10))? else {
        return Ok(());
    };
    if !cleanup_process_owned(cleanup, &process) {
        return Ok(());
    }
    platform.terminate(process.pid, process.start_epoch_secs)?;
    state.cleanup_paused = true;
    state.cleanup_pid = Some(process.pid);
    save_state(state_path, state, lock_timeout)?;
    wait_port_free(port, Duration::from_secs(30), platform)
}

fn restore_cleanup<P: SessionPlatform>(
    cleanup: Option<&CleanupConfig>,
    state: &mut SessionState,
    platform: &P,
) -> Result<(), String> {
    if !state.cleanup_paused {
        return Ok(());
    }
    let cleanup = cleanup.ok_or_else(|| {
        "session state requires cleanup restoration but cleanup is disabled".to_owned()
    })?;
    let port = cleanup.port.expect("validated cleanup port");
    let mut process = platform.listener(port, Duration::from_secs(10))?;
    if process.is_none() {
        platform.run_cleanup_script(
            cleanup.start_script.as_deref().expect("validated script"),
            Duration::from_secs(120),
        )?;
        if !wait_health(
            cleanup.health.as_deref().expect("validated health"),
            Duration::from_secs(120),
            platform,
        ) {
            return Err("cleanup process did not become healthy after restoration".to_owned());
        }
        process = platform.listener(port, Duration::from_secs(10))?;
    }
    let process = process.ok_or_else(|| "cleanup restoration produced no listener".to_owned())?;
    if !cleanup_process_owned(cleanup, &process) {
        return Err("cleanup restoration produced an unrecognized process".to_owned());
    }
    if !platform.health_is_ok(
        cleanup.health.as_deref().expect("validated health"),
        Duration::from_secs(3),
    ) {
        return Err("restored cleanup process is not healthy".to_owned());
    }
    state.cleanup_paused = false;
    state.cleanup_pid = None;
    Ok(())
}

fn cleanup_process_owned(cleanup: &CleanupConfig, process: &ObservedProcess) -> bool {
    let Some(path) = process.path.as_deref() else {
        return false;
    };
    paths_equal(cleanup.exe.as_deref().expect("validated executable"), path)
        && command_line_has_port(&process.arguments, cleanup.port.expect("validated port"))
}

fn wait_started_healthy<P: SessionPlatform>(
    resolved: &ResolvedSession,
    state: &SessionState,
    timeout: Duration,
    platform: &P,
) -> Result<bool, String> {
    let attempts = poll_attempts(timeout, Duration::from_secs(2));
    for attempt in 0..attempts {
        if platform.health_is_ok(
            &format!("{}/health", resolved.base_url),
            Duration::from_secs(3),
        ) {
            let listener = platform.listener(resolved.session.port, Duration::from_secs(10))?;
            let (owned, strength, _) =
                classify_identity(resolved.session.port, Some(state), listener.as_ref());
            if owned && strength == ProcessIdentityStrength::Verified {
                return Ok(true);
            }
            if listener.is_some() {
                return Ok(false);
            }
        }
        let Some(pid) = state.pid else {
            return Ok(false);
        };
        if platform.observe(pid)?.is_none() {
            return Ok(false);
        }
        if attempt + 1 < attempts {
            platform.sleep(Duration::from_secs(2));
        }
    }
    Ok(false)
}

fn wait_health<P: SessionPlatform>(url: &str, timeout: Duration, platform: &P) -> bool {
    let attempts = poll_attempts(timeout, Duration::from_secs(2));
    for attempt in 0..attempts {
        if platform.health_is_ok(url, Duration::from_secs(3)) {
            return true;
        }
        if attempt + 1 < attempts {
            platform.sleep(Duration::from_secs(2));
        }
    }
    false
}

fn wait_port_free<P: SessionPlatform>(
    port: u16,
    timeout: Duration,
    platform: &P,
) -> Result<(), String> {
    let attempts = poll_attempts(timeout, Duration::from_millis(250));
    for attempt in 0..attempts {
        if platform.listener(port, Duration::from_secs(10))?.is_none() {
            return Ok(());
        }
        if attempt + 1 < attempts {
            platform.sleep(Duration::from_millis(250));
        }
    }
    Err(format!(
        "port {port} did not become free within {} seconds",
        timeout.as_secs()
    ))
}

fn poll_attempts(timeout: Duration, interval: Duration) -> u64 {
    let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
    let interval_ms = u64::try_from(interval.as_millis())
        .unwrap_or(u64::MAX)
        .max(1);
    timeout_ms.saturating_add(interval_ms - 1) / interval_ms + 1
}

fn mark_start_failed<P: SessionPlatform>(
    state: &mut SessionState,
    state_path: &Path,
    lock_timeout: Duration,
    error: &str,
    cleanup_error: Option<&str>,
    platform: &P,
) -> Result<(), String> {
    state.phase = Some("failed".to_owned());
    state.failed = Some(error.to_owned());
    state.failed_at = Some(platform.utc_now()?.rfc3339());
    state.cleanup_restore_failed = cleanup_error.map(str::to_owned);
    save_state(state_path, state, lock_timeout)
}

fn ensure_local_api_key(path: &Path, lock_timeout: Duration) -> Result<(), String> {
    let lock = lock_path(path, ".lock");
    let _key_lock = InterprocessLock::acquire(&lock, lock_timeout)?;
    match std::fs::read_to_string(path) {
        Ok(value) if !value.trim_start_matches('\u{feff}').trim().is_empty() => return Ok(()),
        Ok(_) => return Err(format!("local API key file is empty: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "local API key file is unreadable at {}: {error}",
                path.display()
            ));
        }
    }
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("operating system random source failed: {error}"))?;
    let mut key = String::with_capacity(73);
    key.push_str("sk-local-");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut key, "{byte:02x}").expect("writing to String cannot fail");
    }
    atomic_replace(path, key.as_bytes())
}

fn save_state(path: &Path, state: &SessionState, lock_timeout: Duration) -> Result<(), String> {
    let write_lock_path = lock_path(path, ".write.lock");
    let _write_lock = InterprocessLock::acquire(&write_lock_path, lock_timeout)?;
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("failed to encode session state: {error}"))?;
    bytes.push(b'\n');
    atomic_replace(path, &bytes)
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("output path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed to create output directory {}: {error}",
            parent.display()
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "failed to create temporary file in {}: {error}",
            parent.display()
        )
    })?;
    temporary.write_all(bytes).map_err(|error| {
        format!(
            "failed to write temporary file for {}: {error}",
            path.display()
        )
    })?;
    temporary.as_file().sync_all().map_err(|error| {
        format!(
            "failed to flush temporary file for {}: {error}",
            path.display()
        )
    })?;
    temporary
        .persist(path)
        .map_err(|error| format!("failed to publish {}: {}", path.display(), error.error))?;
    Ok(())
}

fn epoch_seconds_rfc3339(epoch_seconds: u64) -> Result<String, String> {
    Ok(UtcTimestamp {
        epoch_seconds,
        nanoseconds: 0,
    }
    .rfc3339())
}

pub fn status(install_root: &Path, lock_timeout: Duration) -> Result<SessionStatus, String> {
    let resolved = config::resolve(install_root, None, false)?;
    let session_lock_path = lock_path(&resolved.state_file, ".session.lock");
    let _session_lock = InterprocessLock::acquire(&session_lock_path, lock_timeout)?;
    status_locked_with(&resolved, lock_timeout, &SystemPlatform)
}

fn status_locked_with<P: SessionPlatform>(
    resolved: &ResolvedSession,
    lock_timeout: Duration,
    platform: &P,
) -> Result<SessionStatus, String> {
    let state = read_state(&resolved.state_file, lock_timeout)?;
    let observed = platform.listener(resolved.session.port, Duration::from_secs(10))?;
    let (owned, identity_strength, identity_failures) =
        classify_identity(resolved.session.port, state.as_ref(), observed.as_ref());
    let healthy = owned
        && platform.health_is_ok(
            &format!("{}/health", resolved.base_url),
            Duration::from_secs(3),
        );
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
    observe_process_optional(pid)?
        .ok_or_else(|| format!("listener PID {pid} exited before its identity could be read"))
}

fn observe_process_optional(pid: u32) -> Result<Option<ObservedProcess>, String> {
    let sysinfo_pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[sysinfo_pid]),
        true,
        ProcessRefreshKind::everything(),
    );
    let Some(process) = system.process(sysinfo_pid) else {
        return Ok(None);
    };
    Ok(Some(ObservedProcess {
        pid,
        path: process.exe().map(Path::to_path_buf),
        arguments: process
            .cmd()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
        start_epoch_secs: process.start_time(),
    }))
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
    agent.get(base_url).call().is_ok()
}

impl SessionPlatform for SystemPlatform {
    fn listener(&self, port: u16, timeout: Duration) -> Result<Option<ObservedProcess>, String> {
        listener_pid(port, timeout)?
            .map(observe_process)
            .transpose()
    }

    fn observe(&self, pid: u32) -> Result<Option<ObservedProcess>, String> {
        observe_process_optional(pid)
    }

    fn health_is_ok(&self, url: &str, timeout: Duration) -> bool {
        health_is_ok(url, timeout)
    }

    fn spawn(
        &self,
        executable: &Path,
        arguments: &[String],
        environment: &BTreeMap<String, Option<String>>,
        stdout: &Path,
        stderr: &Path,
    ) -> Result<ObservedProcess, String> {
        let stdout_file = create_log(stdout)?;
        let stderr_file = create_log(stderr)?;
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout_file))
            .stderr(Stdio::from(stderr_file));
        for (name, value) in environment {
            command.env_remove(name);
            if let Some(value) = value {
                command.env(name, value);
            }
        }
        hide_process_window(&mut command);
        let mut child = command.spawn().map_err(|error| {
            format!(
                "failed to start inference runtime {}: {error}",
                executable.display()
            )
        })?;
        let pid = child.id();
        for _ in 0..100 {
            match observe_process_optional(pid) {
                Ok(Some(observed)) if observed.path.is_some() && !observed.arguments.is_empty() => {
                    return Ok(observed);
                }
                Ok(_) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            }
            match child.try_wait() {
                Ok(Some(_)) => {
                    return Err(format!(
                        "inference runtime PID {pid} exited before its identity could be recorded"
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("failed to inspect spawned PID {pid}: {error}"));
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(format!(
            "could not establish exact identity for spawned inference PID {pid}"
        ))
    }

    fn terminate(&self, pid: u32, expected_start_epoch_secs: u64) -> Result<(), String> {
        let sysinfo_pid = Pid::from_u32(pid);
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[sysinfo_pid]),
            true,
            ProcessRefreshKind::everything(),
        );
        let Some(process) = system.process(sysinfo_pid) else {
            return Ok(());
        };
        if process.start_time() != expected_start_epoch_secs {
            return Err(format!(
                "PID {pid} was reused; refusing to terminate a process with a different start time"
            ));
        }
        if process.kill() {
            Ok(())
        } else {
            Err(format!("operating system refused to terminate PID {pid}"))
        }
    }

    fn run_cleanup_script(&self, script: &Path, timeout: Duration) -> Result<(), String> {
        let executable = system_powershell()?;
        let output = run_bounded(
            &executable,
            &[
                OsStr::new("-NoProfile"),
                OsStr::new("-ExecutionPolicy"),
                OsStr::new("Bypass"),
                OsStr::new("-File"),
                script.as_os_str(),
            ],
            timeout,
        )
        .map_err(|error| format!("failed to run cleanup start script: {error}"))?;
        if output.timed_out {
            return Err(format!(
                "cleanup start script timed out after {} seconds",
                timeout.as_secs()
            ));
        }
        if !output.status.success() {
            return Err(format!(
                "cleanup start script failed with exit code {}",
                output
                    .status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unavailable".to_owned())
            ));
        }
        Ok(())
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }

    fn utc_now(&self) -> Result<UtcTimestamp, String> {
        UtcTimestamp::now()
    }
}

fn create_log(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create log directory {}: {error}",
                parent.display()
            )
        })?;
    }
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to open process log {}: {error}", path.display()))
}

#[cfg(windows)]
fn hide_process_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_process_window(_command: &mut Command) {}

fn system_powershell() -> Result<PathBuf, String> {
    if cfg!(windows) {
        let system_root = std::env::var_os("SystemRoot").ok_or_else(|| {
            "SystemRoot is unavailable; cannot locate system Windows PowerShell".to_owned()
        })?;
        let executable =
            PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        if executable.is_file() {
            return Ok(executable);
        }
        return Err(format!(
            "system Windows PowerShell is unavailable at {}",
            executable.display()
        ));
    }
    resolve_executable("pwsh")
        .or_else(|| resolve_executable("powershell"))
        .ok_or_else(|| "PowerShell is required by the configured cleanup hook".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProfileStatus;
    use serde_json::json;
    use std::cell::RefCell;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakePlatformState {
        listeners: BTreeMap<u16, ObservedProcess>,
        processes: BTreeMap<u32, ObservedProcess>,
        spawns: Vec<Vec<String>>,
        next_pid: u32,
        cleanup_runs: u32,
    }

    struct FakePlatform {
        state: RefCell<FakePlatformState>,
        main_port: u16,
        cleanup: Option<(u16, PathBuf)>,
        unhealthy_ngram: bool,
        unhealthy_all: bool,
    }

    impl FakePlatform {
        fn new(
            main_port: u16,
            cleanup: Option<(u16, PathBuf)>,
            unhealthy_ngram: bool,
            unhealthy_all: bool,
        ) -> Self {
            Self {
                state: RefCell::new(FakePlatformState {
                    next_pid: 100,
                    ..FakePlatformState::default()
                }),
                main_port,
                cleanup,
                unhealthy_ngram,
                unhealthy_all,
            }
        }

        fn add_cleanup_listener(&self) {
            let (port, executable) = self.cleanup.as_ref().expect("cleanup fixture");
            self.add_process(*port, executable.clone(), 90);
        }

        fn add_process(&self, port: u16, executable: PathBuf, pid: u32) {
            let process = ObservedProcess {
                pid,
                path: Some(executable),
                arguments: vec![
                    "fixture.exe".to_owned(),
                    "--port".to_owned(),
                    port.to_string(),
                ],
                start_epoch_secs: 1_000 + u64::from(pid),
            };
            let mut state = self.state.borrow_mut();
            state.listeners.insert(port, process.clone());
            state.processes.insert(pid, process);
        }

        fn port_from_arguments(arguments: &[String]) -> Option<u16> {
            arguments.iter().enumerate().find_map(|(index, argument)| {
                if argument.eq_ignore_ascii_case("--port") {
                    return arguments.get(index + 1)?.parse().ok();
                }
                let (name, value) = argument.split_once('=')?;
                name.eq_ignore_ascii_case("--port")
                    .then(|| value.parse().ok())
                    .flatten()
            })
        }
    }

    impl SessionPlatform for FakePlatform {
        fn listener(
            &self,
            port: u16,
            _timeout: Duration,
        ) -> Result<Option<ObservedProcess>, String> {
            Ok(self.state.borrow().listeners.get(&port).cloned())
        }

        fn observe(&self, pid: u32) -> Result<Option<ObservedProcess>, String> {
            Ok(self.state.borrow().processes.get(&pid).cloned())
        }

        fn health_is_ok(&self, url: &str, _timeout: Duration) -> bool {
            let state = self.state.borrow();
            if url.contains(&format!(":{}/", self.main_port)) {
                let Some(process) = state.listeners.get(&self.main_port) else {
                    return false;
                };
                let optimized = process
                    .arguments
                    .iter()
                    .any(|argument| argument == "draft-mtp,ngram-mod");
                return !(self.unhealthy_all || self.unhealthy_ngram && optimized);
            }
            self.cleanup.as_ref().is_some_and(|(port, _)| {
                url.contains(&format!(":{port}/")) && state.listeners.contains_key(port)
            })
        }

        fn spawn(
            &self,
            executable: &Path,
            arguments: &[String],
            _environment: &BTreeMap<String, Option<String>>,
            _stdout: &Path,
            _stderr: &Path,
        ) -> Result<ObservedProcess, String> {
            let port = Self::port_from_arguments(arguments)
                .ok_or_else(|| "fixture spawn has no port".to_owned())?;
            let mut state = self.state.borrow_mut();
            state.next_pid += 1;
            let process = ObservedProcess {
                pid: state.next_pid,
                path: Some(executable.to_path_buf()),
                arguments: std::iter::once(executable.to_string_lossy().into_owned())
                    .chain(arguments.iter().cloned())
                    .collect(),
                start_epoch_secs: 1_000 + u64::from(state.next_pid),
            };
            state.spawns.push(arguments.to_vec());
            state.listeners.insert(port, process.clone());
            state.processes.insert(process.pid, process.clone());
            Ok(process)
        }

        fn terminate(&self, pid: u32, expected_start_epoch_secs: u64) -> Result<(), String> {
            let mut state = self.state.borrow_mut();
            let Some(process) = state.processes.get(&pid) else {
                return Ok(());
            };
            if process.start_epoch_secs != expected_start_epoch_secs {
                return Err("fixture PID reuse".to_owned());
            }
            state.processes.remove(&pid);
            state.listeners.retain(|_, process| process.pid != pid);
            Ok(())
        }

        fn run_cleanup_script(&self, _script: &Path, _timeout: Duration) -> Result<(), String> {
            let (port, executable) = self.cleanup.as_ref().expect("cleanup fixture");
            let mut state = self.state.borrow_mut();
            state.cleanup_runs += 1;
            state.next_pid += 1;
            let process = ObservedProcess {
                pid: state.next_pid,
                path: Some(executable.clone()),
                arguments: vec![
                    executable.to_string_lossy().into_owned(),
                    "--port".to_owned(),
                    port.to_string(),
                ],
                start_epoch_secs: 1_000 + u64::from(state.next_pid),
            };
            state.listeners.insert(*port, process.clone());
            state.processes.insert(process.pid, process);
            Ok(())
        }

        fn sleep(&self, _duration: Duration) {}

        fn utc_now(&self) -> Result<UtcTimestamp, String> {
            Ok(UtcTimestamp {
                epoch_seconds: 1_800_000_000,
                nanoseconds: 0,
            })
        }
    }

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
            schema: 1,
            transaction_id: Some("tx".to_owned()),
            phase: Some("healthy".to_owned()),
            pid: Some(4900),
            profile: Some("fast-32k".to_owned()),
            runtime: Some("custom".to_owned()),
            server: Some(PathBuf::from(r"C:\runtime\server.exe")),
            vision: false,
            fallback: None,
            process_start_epoch_secs: None,
            ..SessionState::default()
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

    #[test]
    fn lifecycle_falls_back_restores_cleanup_and_stops_exact_process() {
        let directory = tempfile::tempdir().unwrap();
        write_lifecycle_fixture(directory.path(), true);
        let resolved = config::resolve(directory.path(), Some("turbo-16k"), true).unwrap();
        let cleanup_exe = directory.path().join("cleanup/cleanup.exe");
        let platform = FakePlatform::new(8123, Some((8090, cleanup_exe)), true, false);
        platform.add_cleanup_listener();
        let start_options = StartSessionOptions {
            install_root: directory.path().to_path_buf(),
            profile: Some("turbo-16k".to_owned()),
            vision: false,
            force_fallback: false,
            lock_timeout: Duration::from_secs(1),
            startup_timeout: Duration::from_secs(2),
        };
        let started = start_locked_with(&resolved, &start_options, &platform).unwrap();
        assert!(started.started);
        assert_eq!(
            started.status.identity_strength,
            ProcessIdentityStrength::Verified
        );
        assert_eq!(started.status.fallback.as_deref(), Some("mtp-only"));
        assert_eq!(platform.state.borrow().spawns.len(), 2);
        assert!(!platform.state.borrow().listeners.contains_key(&8090));

        let stop_options = StopSessionOptions {
            install_root: directory.path().to_path_buf(),
            lock_timeout: Duration::from_secs(1),
            allow_legacy_identity: false,
        };
        let stopped = stop_locked_with(&resolved, &stop_options, &platform).unwrap();
        assert!(stopped.stopped);
        assert!(!stopped.status.active);
        assert_eq!(stopped.status.phase.as_deref(), Some("stopped"));
        assert!(platform.state.borrow().listeners.contains_key(&8090));
        assert_eq!(platform.state.borrow().cleanup_runs, 1);
        let state = read_state(&resolved.state_file, Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(state.schema, 2);
        assert!(!state.cleanup_paused);
        assert!(state.cleanup_restore_failed.is_none());
    }

    #[test]
    fn failed_fallback_terminates_inference_restores_cleanup_and_records_failure() {
        let directory = tempfile::tempdir().unwrap();
        write_lifecycle_fixture(directory.path(), true);
        let resolved = config::resolve(directory.path(), Some("turbo-16k"), true).unwrap();
        let cleanup_exe = directory.path().join("cleanup/cleanup.exe");
        let platform = FakePlatform::new(8123, Some((8090, cleanup_exe)), true, true);
        platform.add_cleanup_listener();
        let options = StartSessionOptions {
            install_root: directory.path().to_path_buf(),
            profile: Some("turbo-16k".to_owned()),
            vision: false,
            force_fallback: false,
            lock_timeout: Duration::from_secs(1),
            startup_timeout: Duration::ZERO,
        };
        let error = start_locked_with(&resolved, &options, &platform).unwrap_err();
        assert!(error.contains("MTP-only"));
        assert!(!platform.state.borrow().listeners.contains_key(&8123));
        assert!(platform.state.borrow().listeners.contains_key(&8090));
        let state = read_state(&resolved.state_file, Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(state.phase.as_deref(), Some("failed"));
        assert!(
            state
                .failed
                .as_deref()
                .is_some_and(|value| value.contains("MTP-only"))
        );
        assert!(!state.cleanup_paused);
    }

    #[test]
    fn legacy_process_identity_requires_explicit_one_time_stop_authorization() {
        let directory = tempfile::tempdir().unwrap();
        write_lifecycle_fixture(directory.path(), false);
        let resolved = config::resolve(directory.path(), Some("turbo-16k"), true).unwrap();
        let platform = FakePlatform::new(8123, None, false, false);
        platform.add_process(8123, resolved.server.clone(), 90);
        let legacy = SessionState {
            schema: 1,
            transaction_id: Some("legacy".to_owned()),
            phase: Some("healthy".to_owned()),
            pid: Some(90),
            profile: Some("turbo-16k".to_owned()),
            runtime: Some("custom".to_owned()),
            server: Some(resolved.server.clone()),
            ..SessionState::default()
        };
        save_state(&resolved.state_file, &legacy, Duration::from_secs(1)).unwrap();
        let mut options = StopSessionOptions {
            install_root: directory.path().to_path_buf(),
            lock_timeout: Duration::from_secs(1),
            allow_legacy_identity: false,
        };
        let error = stop_locked_with(&resolved, &options, &platform).unwrap_err();
        assert!(error.contains("legacy process identity"));
        assert!(platform.state.borrow().listeners.contains_key(&8123));

        options.allow_legacy_identity = true;
        let report = stop_locked_with(&resolved, &options, &platform).unwrap();
        assert!(report.stopped);
        assert!(!platform.state.borrow().listeners.contains_key(&8123));
    }

    #[test]
    fn concurrent_api_key_initialization_converges_without_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = Arc::new(directory.path().join("api-key.txt"));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let path = Arc::clone(&path);
            workers.push(std::thread::spawn(move || {
                ensure_local_api_key(&path, Duration::from_secs(5)).unwrap();
                std::fs::read_to_string(&*path).unwrap()
            }));
        }
        let values = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(values.len(), 1);
        let key = values.into_iter().next().unwrap();
        assert!(key.starts_with("sk-local-"));
        assert_eq!(key.len(), 73);
    }

    #[test]
    fn spawned_runtime_and_profile_must_still_match_captured_identity() {
        let directory = tempfile::tempdir().unwrap();
        write_lifecycle_fixture(directory.path(), false);
        let resolved = config::resolve(directory.path(), Some("turbo-16k"), true).unwrap();
        let profile_path = directory.path().join("profiles/turbo-16k.json");
        let state = SessionState {
            server_sha256: Some(sha256_file(&resolved.server).unwrap()),
            profile_sha256: Some(sha256_file(&profile_path).unwrap()),
            ..SessionState::default()
        };
        verify_spawn_inputs_unchanged(&resolved, &state).unwrap();
        std::fs::write(&profile_path, b"{}\n").unwrap();
        assert!(
            verify_spawn_inputs_unchanged(&resolved, &state)
                .unwrap_err()
                .contains("Profile changed")
        );
    }

    #[test]
    fn state_publication_is_atomic_under_concurrent_rust_readers_and_writers() {
        let directory = tempfile::tempdir().unwrap();
        let path = Arc::new(directory.path().join("session-state.json"));
        save_state(&path, &SessionState::default(), Duration::from_secs(1)).unwrap();
        let mut workers = Vec::new();
        for writer in 0..4 {
            let path = Arc::clone(&path);
            workers.push(std::thread::spawn(move || {
                for iteration in 0..25 {
                    let state = SessionState {
                        schema: 2,
                        transaction_id: Some(format!("{writer}-{iteration}")),
                        phase: Some("starting".to_owned()),
                        ..SessionState::default()
                    };
                    save_state(&path, &state, Duration::from_secs(5)).unwrap();
                }
            }));
        }
        for _ in 0..2 {
            let path = Arc::clone(&path);
            workers.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    let state = read_state(&path, Duration::from_secs(5)).unwrap().unwrap();
                    assert!(matches!(state.schema, 1 | 2));
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    }

    fn write_lifecycle_fixture(root: &Path, cleanup_enabled: bool) {
        for directory in ["config", "profiles", "runtime", "models", "cleanup"] {
            std::fs::create_dir_all(root.join(directory)).unwrap();
        }
        let server = root.join("runtime/llama-server.exe");
        let model = root.join("models/model.gguf");
        let mmproj = root.join("models/mmproj.gguf");
        let template = root.join("config/chat.jinja");
        let cleanup_exe = root.join("cleanup/cleanup.exe");
        let cleanup_script = root.join("cleanup/start.ps1");
        for path in [
            &server,
            &model,
            &mmproj,
            &template,
            &cleanup_exe,
            &cleanup_script,
        ] {
            std::fs::write(path, b"fixture").unwrap();
        }
        std::fs::write(
            root.join("profiles/turbo-16k.json"),
            serde_json::to_vec(&json!({
                "name": "turbo-16k", "status": "candidate", "runtime": "custom",
                "context": 16384, "output": 4096, "parallel": 1, "threads": 16,
                "batch_size": 2048, "ubatch_size": 768, "kv_cache": "q8_0",
                "tensor_cpu_through_block": 43, "mtp_depth": 3, "ngram_mod": true,
                "ngram_reset_on_begin": true, "external_skills": false,
                "skill_tool": false, "vision_fit": true, "fit_target_mib": 512
            }))
            .unwrap(),
        )
        .unwrap();
        let cleanup = if cleanup_enabled {
            json!({
                "enabled": true, "port": 8090, "exe": cleanup_exe,
                "start_script": cleanup_script, "health": "http://127.0.0.1:8090/health"
            })
        } else {
            json!({"enabled": false})
        };
        std::fs::write(
            root.join("config/session.json"),
            serde_json::to_vec(&json!({
                "schema": 3, "root": root, "host": "127.0.0.1", "port": 8123,
                "active_profile": "turbo-16k", "runtimes": {"custom": server},
                "model": model, "mmproj": mmproj, "chat_template": template,
                "api_key_file": root.join("config/api-key.txt"),
                "base_url_file": root.join("config/base-url.txt"),
                "state_file": root.join("logs/session-state.json"), "cleanup": cleanup
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
