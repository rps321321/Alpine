use crate::clock::UtcTimestamp;
use crate::config::{self, ResolvedSession};
use crate::evidence::{EvidenceWriter, NewRun, SampleRecord, TerminalStatus};
use crate::identity::{sha256_bytes, sha256_file, tree_sha256};
use crate::locking::InterprocessLock;
use crate::process::{resolve_executable, run_bounded};
use crate::qualification::EvidencePhase;
use crate::session::{self, ProcessIdentityStrength};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(900);
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct MicrobenchmarkOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub result_root: PathBuf,
    pub profile: String,
    pub runs: u32,
    pub warmups: u32,
    pub workloads: Vec<String>,
    pub notes: Option<String>,
    pub phase: EvidencePhase,
    pub deep_verify_artifacts: bool,
    pub lease_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExperimentReport {
    pub run_id: String,
    pub status: String,
    pub summary: Value,
}

#[derive(Debug, Deserialize)]
struct ArtifactManifest {
    model: ModelArtifact,
    llama_cpp: BackendArtifact,
}

#[derive(Debug, Deserialize)]
struct ModelArtifact {
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Deserialize)]
struct BackendArtifact {
    commit: String,
}

#[derive(Debug, Deserialize)]
struct SessionState {
    schema: u32,
    transaction_id: String,
    phase: String,
    pid: u32,
    profile: String,
    runtime: String,
    server: PathBuf,
    server_sha256: String,
    runtime_build_sha256: Option<String>,
    profile_sha256: String,
    session_config_sha256: String,
    process_start_epoch_secs: u64,
    arguments: Vec<String>,
    #[serde(default)]
    environment: Value,
}

#[derive(Debug, Deserialize)]
struct WorkloadSuite {
    schema: u32,
    workloads: Vec<WorkloadDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
struct WorkloadDefinition {
    id: String,
    prompt_file: PathBuf,
    #[serde(default = "one")]
    repeat: u32,
    n_predict: u32,
    #[serde(default = "yes")]
    ignore_eos: bool,
    quality: QualityCheck,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum QualityCheck {
    Json,
    Nonempty,
}

#[derive(Debug, Clone)]
struct Workload {
    definition: WorkloadDefinition,
    prompt: String,
}

#[derive(Debug, Clone)]
struct CompletionSample {
    content: String,
    output_sha256: String,
    prompt_tokens: Option<i64>,
    generated_tokens: Option<i64>,
    prefill_tps: Option<f64>,
    decode_tps: Option<f64>,
    drafted_tokens: Option<i64>,
    accepted_tokens: Option<i64>,
    ttft_ms: Option<f64>,
    latency_ms: f64,
    truncated: Option<bool>,
    stop_type: Option<String>,
}

struct PreparedExperiment {
    resolved: ResolvedSession,
    run: NewRun,
    run_id: String,
    raw_directory: PathBuf,
    workloads: Vec<Workload>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ArtifactVerification {
    schema: u32,
    path: PathBuf,
    sha256: String,
    bytes: u64,
    modified_unix_nanos: u128,
    manifest_sha256: String,
    verified_at: String,
    method: String,
}

pub fn run_microbenchmark(options: &MicrobenchmarkOptions) -> Result<ExperimentReport, String> {
    let capacity_path = options.install_root.join("logs/inference.lease");
    let _capacity = InterprocessLock::acquire(&capacity_path, options.lease_timeout)
        .map_err(|error| format!("Inference capacity is already in use; {error}"))?;
    std::fs::create_dir_all(&options.result_root).map_err(|error| {
        format!(
            "failed to create result root {}: {error}",
            options.result_root.display()
        )
    })?;
    std::fs::create_dir_all(options.result_root.join("runs")).map_err(|error| {
        format!(
            "failed to create raw evidence root under {}: {error}",
            options.result_root.display()
        )
    })?;
    let prepared = prepare(options)?;
    std::fs::create_dir(&prepared.raw_directory).map_err(|error| {
        format!(
            "failed to create unique raw evidence directory {}: {error}",
            prepared.raw_directory.display()
        )
    })?;
    let database = options.result_root.join("results.sqlite3");
    let mut writer = EvidenceWriter::open(&database)?;
    let identity = writer.begin_run(&prepared.run)?;
    atomic_json(
        &prepared.raw_directory.join("run.json"),
        &json!({
            "id": prepared.run.id,
            "started_at": prepared.run.started_at,
            "status": "running",
            "kind": prepared.run.kind,
            "profile": prepared.run.profile,
            "git_commit": prepared.run.git_commit,
            "hardware_manifest": prepared.run.hardware_manifest,
            "model_sha256": prepared.run.model_sha256,
            "backend_commit": prepared.run.backend_commit,
            "config": prepared.run.config,
            "notes": prepared.run.notes,
            "identity": identity,
        }),
    )?;

    match execute(&prepared, options, &mut writer) {
        Ok(summary) => {
            let quality = summary
                .get("all_quality_pass")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let (status, terminal) = if quality {
                ("passed", TerminalStatus::Passed)
            } else {
                ("failed-quality", TerminalStatus::FailedQuality)
            };
            let finished_at = utc_now()?;
            writer.finish_run(&prepared.run_id, &finished_at, terminal, &summary)?;
            atomic_json(&prepared.raw_directory.join("summary.json"), &summary)?;
            Ok(ExperimentReport {
                run_id: prepared.run_id,
                status: status.to_owned(),
                summary,
            })
        }
        Err(error) => {
            let failure = json!({"error": "ExperimentFailed", "message": error});
            let finish_result = writer.finish_run(
                &prepared.run_id,
                &utc_now()?,
                TerminalStatus::Error,
                &failure,
            );
            let file_result = atomic_json(&prepared.raw_directory.join("failure.json"), &failure);
            match (finish_result, file_result) {
                (Ok(()), Ok(())) => Err(error),
                (database, file) => Err(format!(
                    "{error}; evidence finalization failed: database={database:?}; file={file:?}"
                )),
            }
        }
    }
}

fn prepare(options: &MicrobenchmarkOptions) -> Result<PreparedExperiment, String> {
    if !(1..=100).contains(&options.runs) {
        return Err("measured runs must be between 1 and 100".to_owned());
    }
    if options.warmups > 20 {
        return Err("warmups must be between 0 and 20".to_owned());
    }
    let repository_root = canonical_directory(&options.repository_root, "repository root")?;
    let resolved = config::resolve(&options.install_root, Some(&options.profile), true)?;
    let state: SessionState = read_json(&resolved.state_file, "Inference Session state")?;
    let observed = session::status(&options.install_root, options.lease_timeout)?;
    validate_active_session(&resolved, &state, &observed)?;

    let artifact_path = repository_root.join("config/artifacts.json");
    let artifacts: ArtifactManifest = read_json(&artifact_path, "artifact manifest")?;
    let artifact_manifest_sha256 = sha256_file(&artifact_path)?;
    require_sha256("model manifest", &artifacts.model.sha256)?;
    require_git_commit("backend commit", &artifacts.llama_cpp.commit)?;
    let model_verification = verify_large_artifact(
        &resolved.model,
        artifacts.model.bytes,
        &artifacts.model.sha256,
        &artifact_manifest_sha256,
        &options.result_root.join("artifact-verification.json"),
        options.deep_verify_artifacts,
    )?;
    let actual_server = sha256_file(&resolved.server)?;
    if actual_server != state.server_sha256 {
        return Err(format!(
            "running server identity mismatch: state={}, observed={actual_server}",
            state.server_sha256
        ));
    }

    let profile_path = resolved
        .install_root
        .join("profiles")
        .join(format!("{}.json", resolved.profile_name));
    let profile_sha256 = sha256_file(&profile_path)?;
    if profile_sha256 != state.profile_sha256 {
        return Err(format!(
            "running Profile identity mismatch: state={}, observed={profile_sha256}",
            state.profile_sha256
        ));
    }
    let (hardware_manifest, hardware_sha256) = latest_hardware_identity(&repository_root)?;
    let (workloads, workload_identity, workload_files) =
        load_workloads(&repository_root, &options.workloads)?;
    let policy_path = repository_root.join("config/promotion-policy.json");
    let policy_sha256 = sha256_file(&policy_path)?;
    let git_commit = repository_commit(&repository_root)?;
    let alpine_binary_sha256 = sha256_file(
        &std::env::current_exe()
            .map_err(|error| format!("failed to locate the running Alpine binary: {error}"))?,
    )?;
    let runtime_identity = state
        .runtime_build_sha256
        .clone()
        .unwrap_or_else(|| state.server_sha256.clone());
    require_sha256("runtime", &runtime_identity)?;

    let benchmark_configuration = json!({
        "name": "micro",
        "schema": 2,
        "sha256": workload_identity,
        "files": workload_files,
        "workloads": if options.workloads.is_empty() { json!("all") } else { json!(options.workloads) },
        "warmups": options.warmups,
        "runs": options.runs,
        "sampler": {"temperature": 0.0, "top_k": 1, "seed": 42},
        "default_ignore_eos": true,
        "cache_prompt": false,
    });
    let material_launch_configuration = json!({
        "runtime": state.runtime,
        "server_sha256": state.server_sha256,
        "runtime_build_sha256": runtime_identity,
        "profile_sha256": profile_sha256,
        "session_config_sha256": state.session_config_sha256,
        "arguments": state.arguments,
        "environment": state.environment,
    });
    let material_configuration = json!({
        "model_sha256": artifacts.model.sha256,
        "profile": resolved.profile,
        "benchmark": benchmark_configuration,
        "launch": material_launch_configuration,
        "qualification_policy_sha256": policy_sha256,
    });
    let configuration_sha256 = sha256_bytes(
        &serde_json::to_vec(&material_configuration)
            .map_err(|error| format!("failed to encode material configuration: {error}"))?,
    );

    let now = UtcTimestamp::now()?;
    let run_id = format!(
        "{}-{}",
        now.compact(),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let run = NewRun {
        id: run_id.clone(),
        started_at: now.rfc3339(),
        kind: "micro".to_owned(),
        profile: resolved.profile_name.clone(),
        git_commit: git_commit.clone(),
        hardware_manifest: hardware_manifest.clone(),
        model_sha256: artifacts.model.sha256.clone(),
        backend_commit: artifacts.llama_cpp.commit.clone(),
        config: json!({
            "identity": {"configuration_sha256": configuration_sha256},
            "hardware": {"path": hardware_manifest, "sha256": hardware_sha256},
            "software": {"git_commit": git_commit, "alpine_binary_sha256": alpine_binary_sha256},
            "model_verification": model_verification,
            "evidence_phase": options.phase,
            "profile": resolved.profile,
            "benchmark": benchmark_configuration,
            "launch": {
                "pid": state.pid,
                "runtime": state.runtime,
                "server": state.server,
                "server_sha256": state.server_sha256,
                "runtime_build_sha256": runtime_identity,
                "profile_sha256": profile_sha256,
                "session_config_sha256": state.session_config_sha256,
                "transaction_id": state.transaction_id,
                "process_start_epoch_secs": state.process_start_epoch_secs,
                "arguments": state.arguments,
                "environment": state.environment,
            },
            "qualification_policy": {"path": "config/promotion-policy.json", "sha256": policy_sha256},
        }),
        notes: options.notes.clone(),
    };
    let raw_directory = options.result_root.join("runs").join(&run_id);
    Ok(PreparedExperiment {
        resolved,
        run,
        run_id,
        raw_directory,
        workloads,
    })
}

fn execute(
    prepared: &PreparedExperiment,
    options: &MicrobenchmarkOptions,
    writer: &mut EvidenceWriter,
) -> Result<Value, String> {
    let api_key = std::fs::read_to_string(&prepared.resolved.api_key_file)
        .map_err(|error| format!("failed to read local API key: {error}"))?
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned();
    if api_key.is_empty() {
        return Err("local API key file is empty".to_owned());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into();
    verify_health(&agent, &prepared.resolved.base_url, &api_key)?;

    let sample_path = prepared.raw_directory.join("samples.jsonl");
    let mut raw_samples = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&sample_path)
        .map_err(|error| format!("failed to create {}: {error}", sample_path.display()))?;
    let mut measured = Vec::new();
    for workload in &prepared.workloads {
        let total = options
            .warmups
            .checked_add(options.runs)
            .ok_or_else(|| "sample count overflow".to_owned())?;
        for offset in 0..total {
            let warmup = offset < options.warmups;
            let iteration = if warmup {
                offset + 1
            } else {
                offset - options.warmups + 1
            };
            let completion =
                stream_completion(&agent, &prepared.resolved.base_url, &api_key, workload)?;
            let quality_pass = quality_pass(&completion.content, workload.definition.quality);
            let raw = completion_json(&completion, workload, iteration, warmup, quality_pass);
            writer.record_sample(
                &prepared.run_id,
                &SampleRecord {
                    workload: workload.definition.id.clone(),
                    iteration,
                    warmup,
                    prompt_tokens: completion.prompt_tokens,
                    generated_tokens: completion.generated_tokens,
                    prefill_tps: completion.prefill_tps,
                    decode_tps: completion.decode_tps,
                    ttft_ms: completion.ttft_ms,
                    latency_ms: Some(completion.latency_ms),
                    output_sha256: Some(completion.output_sha256.clone()),
                    quality_pass: Some(quality_pass),
                    vram_peak_mib: None,
                    gpu_util_mean: None,
                    gpu_power_mean_w: None,
                    gpu_temp_max_c: None,
                    process_working_set_mib: None,
                    raw: raw.clone(),
                },
            )?;
            append_jsonl(&mut raw_samples, &raw)?;
            if !warmup {
                measured.push((workload.definition.id.clone(), completion, quality_pass));
            }
        }
    }
    Ok(summarize(&measured))
}

fn stream_completion(
    agent: &ureq::Agent,
    base_url: &str,
    api_key: &str,
    workload: &Workload,
) -> Result<CompletionSample, String> {
    let url = format!("{base_url}/completion");
    let authorization = format!("Bearer {api_key}");
    let payload = json!({
        "prompt": workload.prompt,
        "n_predict": workload.definition.n_predict,
        "temperature": 0.0,
        "top_k": 1,
        "seed": 42,
        "ignore_eos": workload.definition.ignore_eos,
        "cache_prompt": false,
        "stream": true,
    });
    let started = Instant::now();
    let mut response = agent
        .post(&url)
        .header("Authorization", authorization)
        .send_json(&payload)
        .map_err(|error| format!("completion request failed: {error}"))?;
    let mut reader = BufReader::new(response.body_mut().as_reader());
    let mut line = String::new();
    let mut content = String::new();
    let mut first_content = None;
    let mut final_event = None;
    loop {
        line.clear();
        let count = reader
            .read_line(&mut line)
            .map_err(|error| format!("failed to read completion stream: {error}"))?;
        if count == 0 {
            break;
        }
        if line.len() > MAX_SSE_LINE_BYTES {
            return Err("completion stream event exceeded 1 MiB".to_owned());
        }
        let trimmed = line.trim();
        let Some(encoded) = trimmed.strip_prefix("data: ") else {
            continue;
        };
        let event: Value = serde_json::from_str(encoded)
            .map_err(|error| format!("invalid completion stream event: {error}"))?;
        if let Some(chunk) = event.get("content").and_then(Value::as_str) {
            if !chunk.is_empty() {
                first_content.get_or_insert_with(Instant::now);
                content.push_str(chunk);
            }
        }
        if event.get("stop").and_then(Value::as_bool) == Some(true) {
            final_event = Some(event);
            break;
        }
    }
    let finished = Instant::now();
    let final_event = final_event
        .ok_or_else(|| "completion stream ended without a final timing event".to_owned())?;
    let timings = final_event.get("timings").unwrap_or(&Value::Null);
    Ok(CompletionSample {
        output_sha256: sha256_bytes(content.as_bytes()),
        content,
        prompt_tokens: integer_at(&final_event, "tokens_evaluated")
            .or_else(|| integer_at(timings, "prompt_n")),
        generated_tokens: integer_at(&final_event, "tokens_predicted")
            .or_else(|| integer_at(timings, "predicted_n")),
        prefill_tps: number_at(timings, "prompt_per_second"),
        decode_tps: number_at(timings, "predicted_per_second"),
        drafted_tokens: integer_at(timings, "draft_n"),
        accepted_tokens: integer_at(timings, "draft_n_accepted"),
        ttft_ms: first_content.map(|instant| (instant - started).as_secs_f64() * 1000.0),
        latency_ms: (finished - started).as_secs_f64() * 1000.0,
        truncated: final_event.get("truncated").and_then(Value::as_bool),
        stop_type: final_event
            .get("stop_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn completion_json(
    sample: &CompletionSample,
    workload: &Workload,
    iteration: u32,
    warmup: bool,
    quality_pass: bool,
) -> Value {
    json!({
        "workload": workload.definition.id,
        "iteration": iteration,
        "warmup": warmup,
        "content": sample.content,
        "output_sha256": sample.output_sha256,
        "prompt_tokens": sample.prompt_tokens,
        "generated_tokens": sample.generated_tokens,
        "prefill_tps": sample.prefill_tps,
        "decode_tps": sample.decode_tps,
        "drafted_tokens": sample.drafted_tokens,
        "accepted_tokens": sample.accepted_tokens,
        "ttft_ms": sample.ttft_ms,
        "latency_ms": sample.latency_ms,
        "truncated": sample.truncated,
        "stop_type": sample.stop_type,
        "quality_pass": quality_pass,
    })
}

fn summarize(samples: &[(String, CompletionSample, bool)]) -> Value {
    let mut grouped: BTreeMap<&str, Vec<&(String, CompletionSample, bool)>> = BTreeMap::new();
    for sample in samples {
        grouped.entry(&sample.0).or_default().push(sample);
    }
    let mut workloads = serde_json::Map::new();
    for (name, rows) in grouped {
        let hashes = rows
            .iter()
            .map(|row| row.1.output_sha256.clone())
            .collect::<BTreeSet<_>>();
        workloads.insert(
            name.to_owned(),
            json!({
                "decode_tps": describe(rows.iter().filter_map(|row| row.1.decode_tps).collect()),
                "prefill_tps": describe(rows.iter().filter_map(|row| row.1.prefill_tps).collect()),
                "ttft_ms": describe(rows.iter().filter_map(|row| row.1.ttft_ms).collect()),
                "latency_ms": describe(rows.iter().map(|row| row.1.latency_ms).collect()),
                "draft_acceptance_rate": describe(rows.iter().filter_map(|row| {
                    let drafted = row.1.drafted_tokens?;
                    (drafted > 0).then(|| row.1.accepted_tokens.unwrap_or(0) as f64 / drafted as f64)
                }).collect()),
                "quality_pass_rate": rows.iter().filter(|row| row.2).count() as f64 / rows.len() as f64,
                "unique_output_hashes": hashes,
                "deterministic": hashes.len() <= 1,
            }),
        );
    }
    let all_quality_pass = !samples.is_empty() && samples.iter().all(|row| row.2);
    let all_deterministic = workloads
        .values()
        .all(|value| value["deterministic"].as_bool() == Some(true));
    json!({
        "workloads": workloads,
        "all_quality_pass": all_quality_pass,
        "all_deterministic": all_deterministic,
    })
}

fn describe(mut values: Vec<f64>) -> Value {
    values.retain(|value| value.is_finite());
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return json!({
            "n": 0, "min": null, "max": null, "mean": null, "median": null,
            "stdev": null, "p50": null, "p90": null, "p95": null
        });
    }
    let count = values.len();
    let mean = values.iter().sum::<f64>() / count as f64;
    let stdev = if count > 1 {
        let variance = values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (count - 1) as f64;
        variance.sqrt()
    } else {
        0.0
    };
    json!({
        "n": count,
        "min": values[0],
        "max": values[count - 1],
        "mean": mean,
        "median": percentile(&values, 0.5),
        "stdev": stdev,
        "p50": percentile(&values, 0.5),
        "p90": percentile(&values, 0.9),
        "p95": percentile(&values, 0.95),
    })
}

fn percentile(values: &[f64], probability: f64) -> f64 {
    let position = (values.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    values[lower] + (values[upper] - values[lower]) * (position - lower as f64)
}

fn load_workloads(
    repository_root: &Path,
    selected: &[String],
) -> Result<(Vec<Workload>, String, Vec<String>), String> {
    let root = repository_root.join("benchmarks/micro");
    let definition_path = root.join("workloads.json");
    let suite: WorkloadSuite = read_json(&definition_path, "microbenchmark suite")?;
    if suite.schema != 2 {
        return Err(format!(
            "unsupported microbenchmark schema {}; expected 2",
            suite.schema
        ));
    }
    let requested = selected.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if requested.len() != selected.len() {
        return Err("workload selection contains duplicates".to_owned());
    }
    let known = suite
        .workloads
        .iter()
        .map(|workload| workload.id.as_str())
        .collect::<BTreeSet<_>>();
    let unknown = requested.difference(&known).copied().collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(format!("unknown workloads: {}", unknown.join(", ")));
    }
    let mut files = vec![definition_path];
    let mut workloads = Vec::new();
    for definition in suite.workloads {
        validate_workload(&definition)?;
        let prompt_path = root.join(&definition.prompt_file);
        files.push(prompt_path.clone());
        if !requested.is_empty() && !requested.contains(definition.id.as_str()) {
            continue;
        }
        let base_prompt = std::fs::read_to_string(&prompt_path)
            .map_err(|error| format!("failed to read prompt {}: {error}", prompt_path.display()))?;
        let repeat = usize::try_from(definition.repeat)
            .map_err(|_| format!("workload {} repeat is too large", definition.id))?;
        let prompt = base_prompt.repeat(repeat);
        workloads.push(Workload { definition, prompt });
    }
    if workloads.is_empty() {
        return Err("no microbenchmark workloads matched the selection".to_owned());
    }
    let identity = tree_sha256(&root, &files)?;
    let file_names = files
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .expect("suite files are below suite root")
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect();
    Ok((workloads, identity, file_names))
}

pub(crate) fn current_microbenchmark_identity(
    repository_root: &Path,
    selected: &[String],
) -> Result<String, String> {
    let (_, identity, _) = load_workloads(repository_root, selected)?;
    Ok(identity)
}

fn validate_workload(workload: &WorkloadDefinition) -> Result<(), String> {
    if workload.id.is_empty()
        || !workload
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("invalid workload id '{}'", workload.id));
    }
    if workload.repeat == 0 || workload.n_predict == 0 {
        return Err(format!(
            "workload {} repeat and n_predict must be positive",
            workload.id
        ));
    }
    if workload.prompt_file.is_absolute()
        || workload
            .prompt_file
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "workload {} prompt path must stay inside the suite",
            workload.id
        ));
    }
    Ok(())
}

fn validate_active_session(
    resolved: &ResolvedSession,
    state: &SessionState,
    observed: &session::SessionStatus,
) -> Result<(), String> {
    if state.schema != 2 {
        return Err(format!(
            "Inference Session state schema {} is not eligible for evidence; expected Rust schema 2",
            state.schema
        ));
    }
    if state.phase != "healthy" {
        return Err(format!(
            "Inference Session is not healthy; observed phase '{}'",
            state.phase
        ));
    }
    if state.profile != resolved.profile_name || state.runtime != resolved.runtime_name {
        return Err(format!(
            "running session is {}/{}, requested {}/{}",
            state.profile, state.runtime, resolved.profile_name, resolved.runtime_name
        ));
    }
    if state.profile_sha256 != resolved.profile_sha256 {
        return Err("running Profile hash does not match the selected Profile bytes".to_owned());
    }
    if state.session_config_sha256 != resolved.session_config_sha256 {
        return Err(
            "running Session Config hash does not match the selected config bytes".to_owned(),
        );
    }
    if !observed.active
        || !observed.healthy
        || observed.foreign
        || observed.identity_strength != ProcessIdentityStrength::Verified
        || observed.pid != Some(state.pid)
        || observed.transaction_id.as_deref() != Some(state.transaction_id.as_str())
        || observed.process_start_epoch_secs != Some(state.process_start_epoch_secs)
    {
        return Err(format!(
            "Inference Session process identity is not verified: {:?}",
            observed.identity_failures
        ));
    }
    let state_server = std::fs::canonicalize(&state.server).map_err(|error| {
        format!(
            "running server path is unavailable at {}: {error}",
            state.server.display()
        )
    })?;
    let selected_server = std::fs::canonicalize(&resolved.server).map_err(|error| {
        format!(
            "selected server path is unavailable at {}: {error}",
            resolved.server.display()
        )
    })?;
    if state_server != selected_server {
        return Err(format!(
            "running server {} does not match selected runtime {}",
            state.server.display(),
            resolved.server.display()
        ));
    }
    Ok(())
}

fn latest_hardware_identity(repository_root: &Path) -> Result<(String, String), String> {
    let inventory = repository_root.join("inventory");
    let mut candidates = std::fs::read_dir(&inventory)
        .map_err(|error| format!("hardware inventory is unavailable: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                return false;
            };
            path.is_file()
                && name.ends_with(".json")
                && (name.starts_with("hardware-") || name.starts_with("hardware-"))
        })
        .collect::<Vec<_>>();
    candidates.sort();
    let path = candidates
        .pop()
        .ok_or_else(|| "no hardware inventory is available".to_owned())?;
    let relative = path
        .strip_prefix(repository_root)
        .expect("inventory is below repository root")
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    Ok((relative, sha256_file(&path)?))
}

fn verify_large_artifact(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    manifest_sha256: &str,
    cache_path: &Path,
    deep: bool,
) -> Result<ArtifactVerification, String> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        format!(
            "model artifact is unavailable at {}: {error}",
            path.display()
        )
    })?;
    let metadata = std::fs::metadata(&canonical)
        .map_err(|error| format!("failed to inspect model artifact: {error}"))?;
    if !metadata.is_file() || metadata.len() != expected_bytes {
        return Err(format!(
            "installed model size mismatch: expected {expected_bytes}, observed {}",
            metadata.len()
        ));
    }
    let modified_unix_nanos = metadata
        .modified()
        .map_err(|error| format!("model modification time is unavailable: {error}"))?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "model modification time predates the Unix epoch".to_owned())?
        .as_nanos();
    if !deep && cache_path.is_file() {
        let cache: ArtifactVerification = read_json(cache_path, "artifact verification cache")?;
        if cache.schema == 1
            && cache.path == canonical
            && cache.sha256 == expected_sha256
            && cache.bytes == expected_bytes
            && cache.modified_unix_nanos == modified_unix_nanos
            && cache.manifest_sha256 == manifest_sha256
        {
            return Ok(ArtifactVerification {
                method: "cached-metadata".to_owned(),
                ..cache
            });
        }
    }

    let observed = sha256_file(&canonical)?;
    if observed != expected_sha256 {
        return Err(format!(
            "installed model identity mismatch: expected {expected_sha256}, observed {observed}"
        ));
    }
    let verification = ArtifactVerification {
        schema: 1,
        path: canonical,
        sha256: observed,
        bytes: expected_bytes,
        modified_unix_nanos,
        manifest_sha256: manifest_sha256.to_owned(),
        verified_at: utc_now()?,
        method: "full-sha256".to_owned(),
    };
    atomic_json_replace(
        cache_path,
        &serde_json::to_value(&verification)
            .map_err(|error| format!("failed to encode artifact verification: {error}"))?,
    )?;
    Ok(verification)
}

fn repository_commit(repository_root: &Path) -> Result<String, String> {
    let git = resolve_executable("git").ok_or_else(|| "git is unavailable on PATH".to_owned())?;
    let output = run_bounded(
        &git,
        &[
            OsStr::new("-C"),
            repository_root.as_os_str(),
            OsStr::new("rev-parse"),
            OsStr::new("HEAD"),
        ],
        Duration::from_secs(10),
    )
    .map_err(|error| format!("failed to inspect repository identity: {error}"))?;
    if output.timed_out || !output.status.success() {
        return Err(format!(
            "failed to inspect repository identity: {}",
            output.stderr.trim()
        ));
    }
    let commit = output.stdout.trim().to_owned();
    require_git_commit("repository", &commit)?;
    Ok(commit)
}

fn verify_health(agent: &ureq::Agent, base_url: &str, api_key: &str) -> Result<(), String> {
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
    let healthy = health.get("status").and_then(Value::as_str) == Some("ok")
        || health.get("status").and_then(Value::as_str) == Some("ready");
    if healthy {
        Ok(())
    } else {
        Err(format!("Inference Server is not ready: {health}"))
    }
}

fn quality_pass(content: &str, check: QualityCheck) -> bool {
    match check {
        QualityCheck::Nonempty => !content.trim().is_empty(),
        QualityCheck::Json => serde_json::from_str::<Value>(content).is_ok_and(|value| {
            let Some(object) = value.as_object() else {
                return false;
            };
            object.len() == 3
                && object.get("safe").is_some_and(Value::is_boolean)
                && object.get("reason").is_some_and(Value::is_string)
                && object.get("files").is_some_and(|files| {
                    files
                        .as_array()
                        .is_some_and(|items| items.len() == 2 && items.iter().all(Value::is_string))
                })
        }),
    }
}

fn append_jsonl(file: &mut File, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *file, value)
        .map_err(|error| format!("failed to encode raw sample: {error}"))?;
    file.write_all(b"\n")
        .map_err(|error| format!("failed to append raw sample: {error}"))?;
    file.sync_data()
        .map_err(|error| format!("failed to flush raw sample: {error}"))
}

fn atomic_json(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("output path has no parent: {}", path.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("failed to create temporary evidence file: {error}"))?;
    serde_json::to_writer_pretty(&mut temporary, value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("failed to flush {}: {error}", path.display()))?;
    temporary
        .persist_noclobber(path)
        .map_err(|error| format!("failed to publish {}: {}", path.display(), error.error))?;
    Ok(())
}

fn atomic_json_replace(path: &Path, value: &Value) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("output path has no parent: {}", path.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("failed to create temporary evidence file: {error}"))?;
    serde_json::to_writer_pretty(&mut temporary, value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    temporary
        .write_all(b"\n")
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("failed to flush {}: {error}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| format!("failed to publish {}: {}", path.display(), error.error))?;
    Ok(())
}

fn utc_now() -> Result<String, String> {
    Ok(UtcTimestamp::now()?.rfc3339())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, kind: &str) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "{kind} missing or unreadable at {}: {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Malformed {kind} {}: {error}", path.display()))
}

fn canonical_directory(path: &Path, name: &str) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("{name} is unavailable at {}: {error}", path.display()))?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("{name} is not a directory: {}", path.display()))
    }
}

fn number_at(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn integer_at(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn require_sha256(name: &str, value: &str) -> Result<(), String> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("{name} must be a hexadecimal SHA-256"))
    }
}

fn require_git_commit(name: &str, value: &str) -> Result<(), String> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("{name} must be a hexadecimal Git commit"))
    }
}

const fn one() -> u32 {
    1
}

const fn yes() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_matches_sample_statistics_and_determinism() {
        let sample = |hash: &str, speed: f64| {
            (
                "novel".to_owned(),
                CompletionSample {
                    content: "ok".to_owned(),
                    output_sha256: hash.to_owned(),
                    prompt_tokens: Some(1),
                    generated_tokens: Some(1),
                    prefill_tps: Some(100.0),
                    decode_tps: Some(speed),
                    drafted_tokens: Some(2),
                    accepted_tokens: Some(1),
                    ttft_ms: Some(10.0),
                    latency_ms: 20.0,
                    truncated: Some(false),
                    stop_type: Some("limit".to_owned()),
                },
                true,
            )
        };
        let summary = summarize(&[sample("a", 10.0), sample("a", 20.0)]);
        assert_eq!(summary["workloads"]["novel"]["decode_tps"]["mean"], 15.0);
        assert_eq!(summary["workloads"]["novel"]["deterministic"], true);
        assert_eq!(summary["all_quality_pass"], true);
    }

    #[test]
    fn structured_quality_requires_the_exact_contract() {
        assert!(quality_pass(
            r#"{"safe":true,"files":["a","b"],"reason":"ok"}"#,
            QualityCheck::Json
        ));
        assert!(!quality_pass(
            r#"{"safe":true,"files":["a"],"reason":"ok"}"#,
            QualityCheck::Json
        ));
    }

    #[test]
    fn unchanged_large_artifact_reuses_an_explicit_verification_attestation() {
        let directory = tempfile::tempdir().unwrap();
        let artifact = directory.path().join("model.gguf");
        let cache = directory.path().join("artifact-verification.json");
        std::fs::write(&artifact, b"fixture model").unwrap();
        let expected = sha256_file(&artifact).unwrap();
        let first = verify_large_artifact(
            &artifact,
            13,
            &expected,
            &sha256_bytes(b"manifest"),
            &cache,
            false,
        )
        .unwrap();
        assert_eq!(first.method, "full-sha256");
        let second = verify_large_artifact(
            &artifact,
            13,
            &expected,
            &sha256_bytes(b"manifest"),
            &cache,
            false,
        )
        .unwrap();
        assert_eq!(second.method, "cached-metadata");
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(first.verified_at, second.verified_at);
    }
}
