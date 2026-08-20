use crate::config;
use crate::external::{
    self, CleanRestartStabilityEvidence, ExternalEvidenceKind, ProcessEvidence,
    RecordExternalEvidenceOptions, RecordedExternalEvidence, RestartRequestEvidence,
    SameProcessStabilityEvidence, StabilityRequestContract, StabilityRequestEvidence,
    StabilityRequestRole,
};
use crate::identity::{sha256_bytes, sha256_file};
use crate::locking::InterprocessLock;
use crate::session::{
    self, AcquireSessionOptions, ProcessIdentityStrength, ReleaseSessionOptions,
    SessionAcquisition, SessionStatus, StartSessionOptions, StopSessionOptions,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::Duration;

const TARGET_REQUESTS: u32 = 50;
const CLEAN_RESTARTS: u32 = 10;
const TARGET_N_PREDICT: u32 = 128;
const CONTAMINANT_N_PREDICT: u32 = 16;
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SameProcessStabilityOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub database: PathBuf,
    pub result_root: PathBuf,
    pub anchor_run_id: String,
    pub allow_legacy_identity: bool,
    pub lease_timeout: Duration,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SameProcessStabilityReport {
    pub anchor_run_id: String,
    pub profile: String,
    pub requests: u32,
    pub target_token_sha256: String,
    pub pid: u32,
    pub process_start_epoch_secs: u64,
    pub session_identity: String,
    pub restored_prior_session: bool,
    pub artifact: RecordedExternalEvidence,
}

#[derive(Debug, Clone)]
pub struct CleanRestartStabilityOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub database: PathBuf,
    pub result_root: PathBuf,
    pub anchor_run_id: String,
    pub allow_legacy_identity: bool,
    pub lease_timeout: Duration,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CleanRestartStabilityReport {
    pub anchor_run_id: String,
    pub profile: String,
    pub clean_restarts: u32,
    pub target_token_sha256: String,
    pub restored_prior_session: bool,
    pub artifact: RecordedExternalEvidence,
}

pub fn run_same_process(
    options: &SameProcessStabilityOptions,
) -> Result<SameProcessStabilityReport, String> {
    validate_options(options)?;
    let anchor = external::current_anchor(&options.database, &options.anchor_run_id)?;
    let resolved = config::resolve(&options.install_root, Some(&anchor.summary.profile), true)?;
    let target_prompt_path = options
        .repository_root
        .join("benchmarks/micro/prompts/repeat-code.txt");
    let target_prompt = std::fs::read_to_string(&target_prompt_path).map_err(|error| {
        format!(
            "failed to read stability target prompt {}: {error}",
            target_prompt_path.display()
        )
    })?;
    if target_prompt.trim().is_empty() {
        return Err("stability target prompt is empty".to_owned());
    }
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
    let attempt = execute_same_process(
        &anchor.config,
        &resolved,
        &acquisition,
        &target_prompt,
        options.request_timeout,
        options.lease_timeout,
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
            Ok(true)
        } else {
            Err("stability harness did not restore the exact prior Session snapshot".to_owned())
        }
    });
    let mut evidence = match (attempt, restored) {
        (Ok(evidence), Ok(true)) => evidence,
        (Err(attempt_error), Ok(_)) => return Err(attempt_error),
        (Ok(_), Err(restoration_error)) => return Err(restoration_error),
        (Err(attempt_error), Err(restoration_error)) => {
            return Err(format!(
                "stability harness failed: {attempt_error}; restoration also failed: {restoration_error}"
            ));
        }
        (_, Ok(false)) => unreachable!("restoration returns true or an error"),
    };
    evidence.restored_prior_session = true;
    let target_token_sha256 = evidence
        .requests
        .iter()
        .filter(|request| request.role == StabilityRequestRole::Target)
        .map(|request| request.token_sha256.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .next()
        .ok_or_else(|| "stability harness produced no target token digest".to_owned())?;
    let process = evidence.process_before.clone();
    let artifact = external::record(&RecordExternalEvidenceOptions {
        repository_root: options.repository_root.clone(),
        database: options.database.clone(),
        result_root: options.result_root.clone(),
        anchor_run_id: options.anchor_run_id.clone(),
        kind: ExternalEvidenceKind::SameProcess50RequestGreedyStability,
        evidence: serde_json::to_value(evidence)
            .map_err(|error| format!("failed to encode stability evidence: {error}"))?,
        reviewed_by: None,
    })?;
    Ok(SameProcessStabilityReport {
        anchor_run_id: options.anchor_run_id.clone(),
        profile: anchor.summary.profile,
        requests: TARGET_REQUESTS * 2,
        target_token_sha256,
        pid: process.pid,
        process_start_epoch_secs: process.process_start_epoch_secs,
        session_identity: process.session_identity,
        restored_prior_session: true,
        artifact,
    })
}

pub fn run_clean_restarts(
    options: &CleanRestartStabilityOptions,
) -> Result<CleanRestartStabilityReport, String> {
    validate_restart_options(options)?;
    let anchor = external::current_anchor(&options.database, &options.anchor_run_id)?;
    let resolved = config::resolve(&options.install_root, Some(&anchor.summary.profile), true)?;
    let target_prompt_path = options
        .repository_root
        .join("benchmarks/micro/prompts/repeat-code.txt");
    let target_prompt = std::fs::read_to_string(&target_prompt_path).map_err(|error| {
        format!(
            "failed to read restart target prompt {}: {error}",
            target_prompt_path.display()
        )
    })?;
    if target_prompt.trim().is_empty() {
        return Err("restart target prompt is empty".to_owned());
    }
    let capacity_path = resolved.install_root.join("logs/inference.lease");
    let _capacity = InterprocessLock::acquire(&capacity_path, options.lease_timeout)
        .map_err(|error| format!("inference capacity is unavailable: {error}"))?;
    let prior = session::snapshot_under_capacity(&options.install_root, options.lease_timeout)?;
    let mut mutated = false;
    let attempt = execute_clean_restarts(
        &anchor.config,
        &resolved,
        &target_prompt,
        options,
        &mut mutated,
    );
    let restored = if mutated {
        session::restore_snapshot_under_capacity(
            &options.install_root,
            &prior,
            options.lease_timeout,
            options.startup_timeout,
        )
    } else {
        Ok(())
    };
    let mut evidence = match (attempt, restored) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(attempt_error), Ok(())) => return Err(attempt_error),
        (Ok(_), Err(restoration_error)) => return Err(restoration_error),
        (Err(attempt_error), Err(restoration_error)) => {
            return Err(format!(
                "clean-restart harness failed: {attempt_error}; restoration also failed: {restoration_error}"
            ));
        }
    };
    evidence.restored_prior_session = true;
    let target_token_sha256 = evidence
        .restarts
        .iter()
        .map(|restart| restart.token_sha256.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .next()
        .ok_or_else(|| "clean-restart harness produced no target token digest".to_owned())?;
    let artifact = external::record(&RecordExternalEvidenceOptions {
        repository_root: options.repository_root.clone(),
        database: options.database.clone(),
        result_root: options.result_root.clone(),
        anchor_run_id: options.anchor_run_id.clone(),
        kind: ExternalEvidenceKind::TenCleanRestartGreedyStability,
        evidence: serde_json::to_value(evidence)
            .map_err(|error| format!("failed to encode clean-restart evidence: {error}"))?,
        reviewed_by: None,
    })?;
    Ok(CleanRestartStabilityReport {
        anchor_run_id: options.anchor_run_id.clone(),
        profile: anchor.summary.profile,
        clean_restarts: CLEAN_RESTARTS,
        target_token_sha256,
        restored_prior_session: true,
        artifact,
    })
}

fn execute_clean_restarts(
    anchor_config: &Value,
    resolved: &config::ResolvedSession,
    target_prompt: &str,
    options: &CleanRestartStabilityOptions,
    mutated: &mut bool,
) -> Result<CleanRestartStabilityEvidence, String> {
    let api_key = std::fs::read_to_string(&resolved.api_key_file)
        .map_err(|error| format!("failed to read local API key: {error}"))?
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned();
    if api_key.is_empty() {
        return Err("local API key file is empty".to_owned());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(options.request_timeout))
        .build()
        .into();
    let target_prompt_sha256 = sha256_bytes(target_prompt.as_bytes());
    let contract = StabilityRequestContract {
        target_prompt_sha256: target_prompt_sha256.clone(),
        target_n_predict: TARGET_N_PREDICT,
        contaminant_n_predict: 0,
        temperature: 0.0,
        top_k: 1,
        seed: 42,
        ignore_eos: true,
        cache_prompt: false,
        return_tokens: true,
    };
    let mut restarts = Vec::with_capacity(CLEAN_RESTARTS as usize);
    for sequence in 1..=CLEAN_RESTARTS {
        session::stop_under_capacity(&StopSessionOptions {
            install_root: options.install_root.clone(),
            lock_timeout: options.lease_timeout,
            allow_legacy_identity: options.allow_legacy_identity,
        })?;
        *mutated = true;
        let started = session::start_under_capacity(&StartSessionOptions {
            install_root: options.install_root.clone(),
            profile: Some(resolved.profile_name.clone()),
            vision: false,
            force_fallback: false,
            lock_timeout: options.lease_timeout,
            startup_timeout: options.startup_timeout,
        })?;
        if !started.started {
            return Err(format!(
                "clean restart {sequence} reused a process instead of starting a fresh one"
            ));
        }
        let acquisition = session::acquire_under_capacity(&AcquireSessionOptions {
            install_root: options.install_root.clone(),
            profile: Some(resolved.profile_name.clone()),
            vision: false,
            force_fallback: false,
            allow_legacy_identity: false,
            lock_timeout: options.lease_timeout,
            startup_timeout: options.startup_timeout,
        })?;
        if acquisition.changed {
            return Err(format!(
                "clean restart {sequence} did not preserve the just-started Session"
            ));
        }
        verify_acquisition(anchor_config, &acquisition)?;
        let before = session::status(&options.install_root, options.lease_timeout)?;
        let process = process_evidence(&before, &acquisition)?;
        verify_health(&agent, &resolved.base_url, &api_key)?;
        let tokens = completion_tokens(
            &agent,
            &resolved.base_url,
            &api_key,
            sequence,
            target_prompt,
            TARGET_N_PREDICT,
        )?;
        let after = session::status(&options.install_root, options.lease_timeout)?;
        if process_evidence(&after, &acquisition)? != process {
            return Err(format!(
                "clean restart {sequence} changed process during its request"
            ));
        }
        let token_bytes = serde_json::to_vec(&tokens)
            .map_err(|error| format!("failed to encode restart tokens: {error}"))?;
        restarts.push(RestartRequestEvidence {
            sequence,
            process,
            prompt_sha256: target_prompt_sha256.clone(),
            token_sha256: sha256_bytes(&token_bytes),
            tokens,
        });
    }
    Ok(CleanRestartStabilityEvidence {
        schema: 1,
        profile: resolved.profile_name.clone(),
        request_contract: contract,
        restarts,
        restored_prior_session: false,
    })
}

fn execute_same_process(
    anchor_config: &Value,
    resolved: &config::ResolvedSession,
    acquisition: &SessionAcquisition,
    target_prompt: &str,
    request_timeout: Duration,
    lock_timeout: Duration,
) -> Result<SameProcessStabilityEvidence, String> {
    verify_acquisition(anchor_config, acquisition)?;
    let before = session::status(&resolved.install_root, lock_timeout)?;
    let process_before = process_evidence(&before, acquisition)?;
    let api_key = std::fs::read_to_string(&resolved.api_key_file)
        .map_err(|error| format!("failed to read local API key: {error}"))?
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned();
    if api_key.is_empty() {
        return Err("local API key file is empty".to_owned());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(request_timeout))
        .build()
        .into();
    verify_health(&agent, &resolved.base_url, &api_key)?;
    let target_prompt_sha256 = sha256_bytes(target_prompt.as_bytes());
    let contract = StabilityRequestContract {
        target_prompt_sha256: target_prompt_sha256.clone(),
        target_n_predict: TARGET_N_PREDICT,
        contaminant_n_predict: CONTAMINANT_N_PREDICT,
        temperature: 0.0,
        top_k: 1,
        seed: 42,
        ignore_eos: true,
        cache_prompt: false,
        return_tokens: true,
    };
    let mut requests = Vec::with_capacity((TARGET_REQUESTS * 2) as usize);
    for iteration in 1..=TARGET_REQUESTS {
        let contaminant = contaminant_prompt(iteration);
        requests.push(request(
            &agent,
            &resolved.base_url,
            &api_key,
            requests.len() as u32 + 1,
            StabilityRequestRole::Contaminant,
            &contaminant,
            CONTAMINANT_N_PREDICT,
        )?);
        requests.push(request(
            &agent,
            &resolved.base_url,
            &api_key,
            requests.len() as u32 + 1,
            StabilityRequestRole::Target,
            target_prompt,
            TARGET_N_PREDICT,
        )?);
    }
    let after = session::status(&resolved.install_root, lock_timeout)?;
    let process_after = process_evidence(&after, acquisition)?;
    let evidence = SameProcessStabilityEvidence {
        schema: 1,
        profile: acquisition.profile.clone(),
        process_before,
        process_after,
        request_contract: contract,
        requests,
        restored_prior_session: false,
    };
    Ok(evidence)
}

fn request(
    agent: &ureq::Agent,
    base_url: &str,
    api_key: &str,
    sequence: u32,
    role: StabilityRequestRole,
    prompt: &str,
    n_predict: u32,
) -> Result<StabilityRequestEvidence, String> {
    let tokens = completion_tokens(agent, base_url, api_key, sequence, prompt, n_predict)?;
    let token_bytes = serde_json::to_vec(&tokens)
        .map_err(|error| format!("failed to encode stability tokens: {error}"))?;
    Ok(StabilityRequestEvidence {
        sequence,
        role,
        prompt_sha256: sha256_bytes(prompt.as_bytes()),
        token_sha256: sha256_bytes(&token_bytes),
        tokens,
    })
}

fn completion_tokens(
    agent: &ureq::Agent,
    base_url: &str,
    api_key: &str,
    sequence: u32,
    prompt: &str,
    n_predict: u32,
) -> Result<Vec<u32>, String> {
    let url = format!("{base_url}/completion");
    let authorization = format!("Bearer {api_key}");
    let payload = json!({
        "prompt": prompt,
        "n_predict": n_predict,
        "temperature": 0.0,
        "top_k": 1,
        "seed": 42,
        "ignore_eos": true,
        "cache_prompt": false,
        "return_tokens": true,
        "stream": true,
    });
    let mut response = agent
        .post(&url)
        .header("Authorization", authorization)
        .send_json(&payload)
        .map_err(|error| format!("stability completion request {sequence} failed: {error}"))?;
    let mut reader = BufReader::new(response.body_mut().as_reader());
    let mut line = String::new();
    let mut tokens = Vec::with_capacity(n_predict as usize);
    let mut stopped = false;
    loop {
        line.clear();
        let count = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read stability stream {sequence}: {error}"))?;
        if count == 0 {
            break;
        }
        if line.len() > MAX_SSE_LINE_BYTES {
            return Err(format!("stability stream {sequence} event exceeded 1 MiB"));
        }
        let Some(encoded) = line.trim().strip_prefix("data: ") else {
            continue;
        };
        let event: Value = serde_json::from_str(encoded)
            .map_err(|error| format!("invalid stability stream event {sequence}: {error}"))?;
        if let Some(values) = event.get("tokens").and_then(Value::as_array) {
            for value in values {
                let token = value
                    .as_u64()
                    .and_then(|token| u32::try_from(token).ok())
                    .ok_or_else(|| {
                        format!("stability stream {sequence} returned an invalid token id")
                    })?;
                tokens.push(token);
            }
        }
        if event.get("stop").and_then(Value::as_bool) == Some(true) {
            stopped = true;
            break;
        }
    }
    if !stopped || tokens.len() != n_predict as usize {
        return Err(format!(
            "stability request {sequence} generated {} of {n_predict} required tokens",
            tokens.len()
        ));
    }
    Ok(tokens)
}

pub(crate) fn verify_acquisition(
    config: &Value,
    acquisition: &SessionAcquisition,
) -> Result<(), String> {
    let launch = config
        .get("launch")
        .ok_or_else(|| "anchor run launch evidence is missing".to_owned())?;
    let profile = config
        .pointer("/profile/name")
        .and_then(Value::as_str)
        .ok_or_else(|| "anchor run Profile name is missing".to_owned())?;
    let expected_runtime_build = launch
        .get("runtime_build_sha256")
        .cloned()
        .unwrap_or(Value::Null);
    let matches = acquisition.profile == profile
        && launch.get("runtime").and_then(Value::as_str) == Some(&acquisition.runtime)
        && launch.get("server_sha256").and_then(Value::as_str) == Some(&acquisition.server_sha256)
        && launch.get("profile_sha256").and_then(Value::as_str)
            == Some(&acquisition.profile_sha256)
        && launch.get("session_config_sha256").and_then(Value::as_str)
            == Some(&acquisition.session_config_sha256)
        && expected_runtime_build == json!(acquisition.runtime_build_sha256)
        && launch.get("arguments") == Some(&json!(acquisition.arguments))
        && launch.get("environment") == Some(&json!(acquisition.environment));
    if matches && sha256_file(&acquisition.server)? == acquisition.server_sha256 {
        Ok(())
    } else {
        Err("live Session does not exactly match the final run launch identity".to_owned())
    }
}

pub(crate) fn process_evidence(
    status: &SessionStatus,
    acquisition: &SessionAcquisition,
) -> Result<ProcessEvidence, String> {
    if !status.active
        || !status.healthy
        || status.foreign
        || status.identity_strength != ProcessIdentityStrength::Verified
        || status.profile != acquisition.profile
        || status.transaction_id.as_deref() != Some(&acquisition.session_identity)
    {
        return Err("stability harness lost the verified acquired Session".to_owned());
    }
    Ok(ProcessEvidence {
        pid: status
            .pid
            .ok_or_else(|| "verified Session has no process id".to_owned())?,
        process_start_epoch_secs: status
            .process_start_epoch_secs
            .ok_or_else(|| "verified Session has no process-start identity".to_owned())?,
        session_identity: acquisition.session_identity.clone(),
    })
}

pub(crate) fn verify_health(
    agent: &ureq::Agent,
    base_url: &str,
    api_key: &str,
) -> Result<(), String> {
    let authorization = format!("Bearer {api_key}");
    let mut response = agent
        .get(&format!("{base_url}/health"))
        .header("Authorization", authorization)
        .call()
        .map_err(|error| format!("Inference Server health check failed: {error}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("failed to read health response: {error}"))?;
    let health: Value = serde_json::from_str(&body)
        .map_err(|error| format!("Inference Server health response was not JSON: {error}"))?;
    if matches!(
        health.get("status").and_then(Value::as_str),
        Some("ok" | "ready")
    ) {
        Ok(())
    } else {
        Err(format!("Inference Server is not ready: {health}"))
    }
}

fn contaminant_prompt(iteration: u32) -> String {
    let nonce = sha256_bytes(format!("alpine-stability-contaminant-{iteration}").as_bytes());
    format!(
        "Write exactly one short sentence explaining deterministic system #{iteration}. Include nonce {nonce} verbatim."
    )
}

fn validate_options(options: &SameProcessStabilityOptions) -> Result<(), String> {
    if options.anchor_run_id.trim().is_empty() {
        return Err("anchor run id must not be empty".to_owned());
    }
    if options.lease_timeout.is_zero()
        || options.startup_timeout.is_zero()
        || options.request_timeout.is_zero()
    {
        return Err("stability timeouts must be positive".to_owned());
    }
    Ok(())
}

fn validate_restart_options(options: &CleanRestartStabilityOptions) -> Result<(), String> {
    if options.anchor_run_id.trim().is_empty() {
        return Err("anchor run id must not be empty".to_owned());
    }
    if options.lease_timeout.is_zero()
        || options.startup_timeout.is_zero()
        || options.request_timeout.is_zero()
    {
        return Err("clean-restart timeouts must be positive".to_owned());
    }
    Ok(())
}
