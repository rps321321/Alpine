use crate::config;
use crate::external::{
    self, ExternalEvidenceKind, RecordExternalEvidenceOptions, RecordedExternalEvidence,
    RollbackProfileEvidence,
};
use crate::identity::{runtime_bundle_sha256, sha256_bytes};
use crate::locking::InterprocessLock;
use crate::session::{self, AcquireSessionOptions, ReleaseSessionOptions};
use crate::stability::{completion_tokens, process_evidence, verify_health};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

const ROLLBACK_PROFILE: &str = "stable-16k";
const SMOKE_TOKENS: u32 = 16;

#[derive(Debug, Clone)]
pub struct RollbackProofOptions {
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
pub struct RollbackProofReport {
    pub anchor_run_id: String,
    pub profile: String,
    pub context_tokens: u32,
    pub runtime: String,
    pub smoke_tokens: u32,
    pub restored_prior_session: bool,
    pub artifact: RecordedExternalEvidence,
}

pub fn run(options: &RollbackProofOptions) -> Result<RollbackProofReport, String> {
    validate_options(options)?;
    external::current_anchor(&options.database, &options.anchor_run_id)?;
    let resolved = config::resolve(&options.install_root, Some(ROLLBACK_PROFILE), true)?;
    if resolved.profile.status != config::ProfileStatus::Production
        || resolved.profile.context != 16_384
    {
        return Err("stable-16k is not a 16K production rollback Profile".to_owned());
    }
    let capacity_path = resolved.install_root.join("logs/inference.lease");
    let _capacity = InterprocessLock::acquire(&capacity_path, options.lease_timeout)
        .map_err(|error| format!("inference capacity is unavailable: {error}"))?;
    let acquisition = session::acquire_under_capacity(&AcquireSessionOptions {
        install_root: options.install_root.clone(),
        profile: Some(ROLLBACK_PROFILE.to_owned()),
        vision: false,
        force_fallback: false,
        allow_legacy_identity: options.allow_legacy_identity,
        lock_timeout: options.lease_timeout,
        startup_timeout: options.startup_timeout,
    })?;
    let prior = acquisition.prior.clone();
    let attempt = execute(&resolved, &acquisition, options);
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
            Err("rollback proof did not restore the prior material Session".to_owned())
        }
    });
    let mut evidence = match (attempt, restored) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(attempt_error), Ok(())) => return Err(attempt_error),
        (Ok(_), Err(restoration_error)) => return Err(restoration_error),
        (Err(attempt_error), Err(restoration_error)) => {
            return Err(format!(
                "rollback proof failed: {attempt_error}; restoration also failed: {restoration_error}"
            ));
        }
    };
    evidence.restored_prior_session = true;
    let runtime = evidence.runtime.clone();
    let context_tokens = evidence.context_tokens;
    let artifact = external::record(&RecordExternalEvidenceOptions {
        repository_root: options.repository_root.clone(),
        database: options.database.clone(),
        result_root: options.result_root.clone(),
        anchor_run_id: options.anchor_run_id.clone(),
        kind: ExternalEvidenceKind::RollbackProfileAvailable,
        evidence: serde_json::to_value(evidence)
            .map_err(|error| format!("failed to encode rollback evidence: {error}"))?,
        reviewed_by: None,
    })?;
    Ok(RollbackProofReport {
        anchor_run_id: options.anchor_run_id.clone(),
        profile: ROLLBACK_PROFILE.to_owned(),
        context_tokens,
        runtime,
        smoke_tokens: SMOKE_TOKENS,
        restored_prior_session: true,
        artifact,
    })
}

fn execute(
    resolved: &config::ResolvedSession,
    acquisition: &session::SessionAcquisition,
    options: &RollbackProofOptions,
) -> Result<RollbackProfileEvidence, String> {
    let acquisition_matches = acquisition.profile == resolved.profile_name
        && acquisition.runtime == resolved.runtime_name
        && acquisition.server == resolved.server
        && acquisition.profile_sha256 == resolved.profile_sha256
        && acquisition.session_config_sha256 == resolved.session_config_sha256;
    if !acquisition_matches {
        return Err("acquired rollback Session does not match stable-16k inputs".to_owned());
    }
    let runtime_build_sha256 = runtime_bundle_sha256(&resolved.server)?;
    if acquisition.runtime_build_sha256.as_deref() != Some(&runtime_build_sha256) {
        return Err(
            "acquired rollback Session is not bound to the current complete runtime bundle"
                .to_owned(),
        );
    }
    let status = session::status(&resolved.install_root, options.lease_timeout)?;
    let process = process_evidence(&status, acquisition)?;
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
    verify_health(&agent, &resolved.base_url, &api_key)?;
    let prompt = "State one concise reason transactional rollback matters in production systems.";
    let tokens = completion_tokens(
        &agent,
        &resolved.base_url,
        &api_key,
        1,
        prompt,
        SMOKE_TOKENS,
    )?;
    let after = session::status(&resolved.install_root, options.lease_timeout)?;
    if process_evidence(&after, acquisition)? != process {
        return Err("rollback Session changed process during its smoke request".to_owned());
    }
    let token_sha256 = sha256_bytes(
        &serde_json::to_vec(&tokens)
            .map_err(|error| format!("failed to encode rollback tokens: {error}"))?,
    );
    Ok(RollbackProfileEvidence {
        schema: 1,
        profile: ROLLBACK_PROFILE.to_owned(),
        profile_path: std::fs::canonicalize(&resolved.profile_path)
            .map_err(|error| format!("failed to resolve rollback Profile: {error}"))?,
        profile_sha256: resolved.profile_sha256.clone(),
        session_config_path: std::fs::canonicalize(&resolved.session_config_path)
            .map_err(|error| format!("failed to resolve Session Config: {error}"))?,
        session_config_sha256: resolved.session_config_sha256.clone(),
        runtime: resolved.runtime_name.clone(),
        server_path: std::fs::canonicalize(&resolved.server)
            .map_err(|error| format!("failed to resolve rollback runtime: {error}"))?,
        server_sha256: acquisition.server_sha256.clone(),
        runtime_build_sha256,
        context_tokens: resolved.profile.context,
        process,
        smoke_prompt_sha256: sha256_bytes(prompt.as_bytes()),
        smoke_token_sha256: token_sha256,
        smoke_tokens: tokens,
        restored_prior_session: false,
    })
}

fn validate_options(options: &RollbackProofOptions) -> Result<(), String> {
    if options.anchor_run_id.trim().is_empty() {
        return Err("rollback anchor run id must not be empty".to_owned());
    }
    if options.lease_timeout.is_zero()
        || options.startup_timeout.is_zero()
        || options.request_timeout.is_zero()
    {
        return Err("rollback timeouts must be positive".to_owned());
    }
    Ok(())
}
