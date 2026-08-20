use crate::config;
use crate::external::{
    self, ContextRunEvidence, ExternalEvidenceKind, NearLimitContextEvidence,
    RecordExternalEvidenceOptions, RecordedExternalEvidence,
};
use crate::identity::sha256_bytes;
use crate::locking::InterprocessLock;
use crate::session::{self, AcquireSessionOptions, ReleaseSessionOptions};
use crate::stability::{process_evidence, verify_acquisition, verify_health};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

const NEEDLES: [&str; 3] = ["CEDAR-48291", "ORBIT-73064", "VIOLET-19538"];
const RUNS: u32 = 2;
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct NearLimitContextOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub database: PathBuf,
    pub result_root: PathBuf,
    pub anchor_run_id: String,
    pub ratio: f64,
    pub allow_legacy_identity: bool,
    pub lease_timeout: Duration,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NearLimitContextReport {
    pub anchor_run_id: String,
    pub profile: String,
    pub context_tokens: u32,
    pub target_prompt_tokens: u32,
    pub actual_prompt_tokens: u32,
    pub runs: u32,
    pub restored_prior_session: bool,
    pub artifact: RecordedExternalEvidence,
}

pub fn run(options: &NearLimitContextOptions) -> Result<NearLimitContextReport, String> {
    validate_options(options)?;
    let anchor = external::current_anchor(&options.database, &options.anchor_run_id)?;
    let resolved = config::resolve(&options.install_root, Some(&anchor.summary.profile), true)?;
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
    let attempt = execute(&anchor.config, &resolved, &acquisition, options);
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
            Err("context harness did not restore the prior material Session".to_owned())
        }
    });
    let mut evidence = match (attempt, restored) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(attempt_error), Ok(())) => return Err(attempt_error),
        (Ok(_), Err(restoration_error)) => return Err(restoration_error),
        (Err(attempt_error), Err(restoration_error)) => {
            return Err(format!(
                "context harness failed: {attempt_error}; restoration also failed: {restoration_error}"
            ));
        }
    };
    evidence.restored_prior_session = true;
    let target_prompt_tokens = evidence.target_prompt_tokens;
    let actual_prompt_tokens = evidence.actual_prompt_tokens;
    let context_tokens = evidence.context_tokens;
    let artifact = external::record(&RecordExternalEvidenceOptions {
        repository_root: options.repository_root.clone(),
        database: options.database.clone(),
        result_root: options.result_root.clone(),
        anchor_run_id: options.anchor_run_id.clone(),
        kind: ExternalEvidenceKind::NearLimitContextStress,
        evidence: serde_json::to_value(evidence)
            .map_err(|error| format!("failed to encode context evidence: {error}"))?,
        reviewed_by: None,
    })?;
    Ok(NearLimitContextReport {
        anchor_run_id: options.anchor_run_id.clone(),
        profile: anchor.summary.profile,
        context_tokens,
        target_prompt_tokens,
        actual_prompt_tokens,
        runs: RUNS,
        restored_prior_session: true,
        artifact,
    })
}

fn execute(
    anchor_config: &Value,
    resolved: &config::ResolvedSession,
    acquisition: &session::SessionAcquisition,
    options: &NearLimitContextOptions,
) -> Result<NearLimitContextEvidence, String> {
    verify_acquisition(anchor_config, acquisition)?;
    let before = session::status(&resolved.install_root, options.lease_timeout)?;
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
        .timeout_global(Some(options.request_timeout))
        .build()
        .into();
    verify_health(&agent, &resolved.base_url, &api_key)?;
    let target_prompt_tokens = (f64::from(resolved.profile.context) * options.ratio).floor() as u32;
    let (prompt, actual_prompt_tokens) =
        prompt_near_tokens(&agent, &resolved.base_url, &api_key, target_prompt_tokens)?;
    let expected = NEEDLES.join("|");
    let mut runs = Vec::with_capacity(RUNS as usize);
    for sequence in 1..=RUNS {
        let completion = complete_context(&agent, &resolved.base_url, &api_key, sequence, &prompt)?;
        if completion.content.trim() != expected {
            return Err(format!(
                "context retrieval {sequence} failed: observed {:?}, expected {expected:?}",
                completion.content.trim()
            ));
        }
        runs.push(ContextRunEvidence {
            sequence,
            content_sha256: sha256_bytes(completion.content.as_bytes()),
            token_sha256: sha256_bytes(
                &serde_json::to_vec(&completion.tokens)
                    .map_err(|error| format!("failed to encode context tokens: {error}"))?,
            ),
            content: completion.content,
            tokens: completion.tokens,
        });
    }
    let after = session::status(&resolved.install_root, options.lease_timeout)?;
    let process_after = process_evidence(&after, acquisition)?;
    Ok(NearLimitContextEvidence {
        schema: 1,
        profile: resolved.profile_name.clone(),
        generator: "immutable-ledger-v1".to_owned(),
        context_tokens: resolved.profile.context,
        ratio: options.ratio,
        target_prompt_tokens,
        actual_prompt_tokens,
        prompt_sha256: sha256_bytes(prompt.as_bytes()),
        needles: NEEDLES.iter().map(|needle| (*needle).to_owned()).collect(),
        process_before,
        process_after,
        runs,
        restored_prior_session: false,
    })
}

struct ContextCompletion {
    content: String,
    tokens: Vec<u32>,
}

fn complete_context(
    agent: &ureq::Agent,
    base_url: &str,
    api_key: &str,
    sequence: u32,
    prompt: &str,
) -> Result<ContextCompletion, String> {
    let authorization = format!("Bearer {api_key}");
    let mut response = agent
        .post(&format!("{base_url}/completion"))
        .header("Authorization", authorization)
        .send_json(json!({
            "prompt": prompt,
            "n_predict": 64,
            "temperature": 0.0,
            "top_k": 1,
            "seed": 42,
            "ignore_eos": false,
            "cache_prompt": false,
            "return_tokens": true,
            "stream": false,
        }))
        .map_err(|error| format!("context completion {sequence} failed: {error}"))?;
    let body = read_bounded(response.body_mut().as_reader(), MAX_RESPONSE_BYTES)?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("context completion {sequence} was invalid JSON: {error}"))?;
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("context completion {sequence} has no content"))?
        .to_owned();
    let tokens = parse_tokens(&value, "context completion")?;
    if tokens.is_empty() || tokens.len() > 64 {
        return Err(format!(
            "context completion {sequence} returned an invalid token count"
        ));
    }
    Ok(ContextCompletion { content, tokens })
}

fn token_count(
    agent: &ureq::Agent,
    base_url: &str,
    api_key: &str,
    content: &str,
) -> Result<u32, String> {
    let authorization = format!("Bearer {api_key}");
    let mut response = agent
        .post(&format!("{base_url}/tokenize"))
        .header("Authorization", authorization)
        .send_json(json!({"content": content, "add_special": true}))
        .map_err(|error| format!("context tokenization failed: {error}"))?;
    let body = read_bounded(response.body_mut().as_reader(), MAX_RESPONSE_BYTES)?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("context tokenization returned invalid JSON: {error}"))?;
    let count = value
        .get("tokens")
        .and_then(Value::as_array)
        .ok_or_else(|| "context tokenization response has no token array".to_owned())?
        .len();
    u32::try_from(count).map_err(|_| "context token count exceeds u32".to_owned())
}

fn prompt_near_tokens(
    agent: &ureq::Agent,
    base_url: &str,
    api_key: &str,
    target_tokens: u32,
) -> Result<(String, u32), String> {
    let mut low = 1_u32;
    let mut high = 32_u32.max(target_tokens / 4);
    while token_count(agent, base_url, api_key, &make_prompt(high))? < target_tokens {
        low = high;
        high = high
            .checked_mul(2)
            .filter(|value| *value <= 1_000_000)
            .ok_or_else(|| "context prompt search exceeded its line bound".to_owned())?;
    }
    let mut best_prompt = make_prompt(low);
    let mut best_count = token_count(agent, base_url, api_key, &best_prompt)?;
    while low <= high {
        let middle = low + (high - low) / 2;
        let prompt = make_prompt(middle);
        let count = token_count(agent, base_url, api_key, &prompt)?;
        if count <= target_tokens {
            best_prompt = prompt;
            best_count = count;
            low = middle.saturating_add(1);
        } else {
            if middle == 0 {
                break;
            }
            high = middle - 1;
        }
    }
    Ok((best_prompt, best_count))
}

fn make_prompt(lines: u32) -> String {
    let positions = [
        ((f64::from(lines) * 0.10).floor() as u32, NEEDLES[0]),
        ((f64::from(lines) * 0.50).floor() as u32, NEEDLES[1]),
        ((f64::from(lines) * 0.90).floor() as u32, NEEDLES[2]),
    ];
    let mut prompt = String::from(
        "You are checking a long immutable ledger. Remember every line marked IMPORTANT.\nMost records are filler and must not change the answer.\n",
    );
    let mut checkpoint = 0_u32;
    for index in 0..lines {
        if let Some((_, needle)) = positions.iter().find(|(position, _)| *position == index) {
            checkpoint += 1;
            prompt.push_str(&format!("IMPORTANT checkpoint {checkpoint}: {needle}\n"));
        }
        prompt.push_str(&format!(
            "Record {index:05}: alpha beta gamma delta epsilon zeta eta theta.\n"
        ));
    }
    prompt.push_str(
        "\nReturn exactly the three checkpoint values in order, separated by a single vertical bar. Do not explain. Answer:",
    );
    prompt
}

fn parse_tokens(value: &Value, name: &str) -> Result<Vec<u32>, String> {
    value
        .get("tokens")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{name} has no token array"))?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|token| u32::try_from(token).ok())
                .ok_or_else(|| format!("{name} contains an invalid token id"))
        })
        .collect()
}

fn read_bounded(reader: impl Read, maximum: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read Inference Server response: {error}"))?;
    if bytes.len() as u64 > maximum {
        return Err(format!(
            "Inference Server response exceeded {maximum} bytes"
        ));
    }
    Ok(bytes)
}

fn validate_options(options: &NearLimitContextOptions) -> Result<(), String> {
    if options.anchor_run_id.trim().is_empty() {
        return Err("anchor run id must not be empty".to_owned());
    }
    if !options.ratio.is_finite() || !(0.85..=0.95).contains(&options.ratio) {
        return Err("context ratio must be between 0.85 and 0.95".to_owned());
    }
    if options.lease_timeout.is_zero()
        || options.startup_timeout.is_zero()
        || options.request_timeout.is_zero()
    {
        return Err("context timeouts must be positive".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_generator_places_each_needle_once() {
        let prompt = make_prompt(100);
        for needle in NEEDLES {
            assert_eq!(prompt.matches(needle).count(), 1);
        }
    }
}
