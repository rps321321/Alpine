pub mod assessment;
pub mod catalog;

use alpine_control_plane::{Alpine, StartSessionOptions};
use assessment::{HardwareCapacity, ModelAssessment};
use catalog::ModelSearchResult;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

const SETTINGS_SCHEMA: u32 = 1;

fn settings_schema() -> u32 {
    SETTINGS_SCHEMA
}

fn default_profile() -> String {
    "stable-16k".to_owned()
}

fn local_metrics_default() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelSelection {
    repo_id: String,
    filename: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopSettings {
    #[serde(default = "settings_schema")]
    schema: u32,
    default_model: Option<ModelSelection>,
    install_root: String,
    #[serde(default = "default_profile")]
    default_profile: String,
    #[serde(default = "local_metrics_default")]
    local_metrics_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsUpdate {
    install_root: String,
    default_profile: String,
    local_metrics_enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HardwareProfile {
    cpu: String,
    memory_bytes: u64,
    gpu: Option<String>,
    vram_bytes: u64,
    driver: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeSnapshot {
    state: String,
    profile: String,
    model: Option<String>,
    detail: String,
    available_profiles: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BootstrapSnapshot {
    hardware: HardwareProfile,
    settings: DesktopSettings,
    runtime: RuntimeSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PiLaunchConfig {
    model_id: String,
    base_url: String,
    api_key: String,
    context_window: u32,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeProbeReport {
    model: String,
    profile: String,
    latency_ms: u64,
    output_tokens: Option<u64>,
    quality_pass: bool,
    evidence_label: &'static str,
}

#[derive(Debug, Deserialize)]
struct ProbeResponse {
    choices: Vec<ProbeChoice>,
    usage: Option<ProbeUsage>,
}

#[derive(Debug, Deserialize)]
struct ProbeChoice {
    message: ProbeMessage,
}

#[derive(Debug, Deserialize)]
struct ProbeMessage {
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeUsage {
    completion_tokens: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadReceipt {
    path: String,
    bytes_written: u64,
    already_present: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadedModel {
    filename: String,
    size_bytes: u64,
    state: &'static str,
}

#[derive(Clone, Default)]
struct DownloadRegistry(Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>);

fn capture_capacity() -> Result<(HardwareProfile, HardwareCapacity), String> {
    let report =
        Alpine::inspect_hardware(Duration::from_secs(4)).map_err(|error| error.to_string())?;
    let primary_gpu = report.snapshot.nvidia_gpus.first();
    let vram_bytes = primary_gpu
        .map(|gpu| gpu.vram_mib.saturating_mul(1024 * 1024))
        .unwrap_or(0);
    Ok((
        HardwareProfile {
            cpu: report.snapshot.cpu.brand,
            memory_bytes: report.snapshot.physical_memory_bytes,
            gpu: primary_gpu.map(|gpu| gpu.name.clone()),
            vram_bytes,
            driver: primary_gpu.map(|gpu| gpu.driver_version.clone()),
        },
        HardwareCapacity {
            total_memory_bytes: report.snapshot.physical_memory_bytes,
            dedicated_vram_bytes: vram_bytes,
        },
    ))
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("settings.json"))
        .map_err(|error| format!("failed to resolve Alpine settings directory: {error}"))
}

fn default_settings(app: &AppHandle) -> Result<DesktopSettings, String> {
    let install_root = app
        .path()
        .home_dir()
        .map_err(|error| format!("failed to resolve the user profile directory: {error}"))?
        .join("local-models");
    let resolved = Alpine::resolve_session(&install_root, None, false).ok();
    let default_model = resolved.as_ref().and_then(|session| {
        session
            .model
            .file_name()
            .and_then(|filename| filename.to_str())
            .map(|filename| ModelSelection {
                repo_id: "local/alpine-install".to_owned(),
                filename: filename.to_owned(),
            })
    });
    Ok(DesktopSettings {
        schema: SETTINGS_SCHEMA,
        default_model,
        install_root: install_root.to_string_lossy().into_owned(),
        default_profile: resolved
            .map(|session| session.profile_name)
            .unwrap_or_else(default_profile),
        local_metrics_enabled: true,
    })
}

fn read_settings(app: &AppHandle) -> Result<DesktopSettings, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return default_settings(app);
    }
    let value = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut settings: DesktopSettings = serde_json::from_str(&value)
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))?;
    if settings.schema != SETTINGS_SCHEMA {
        return Err(format!(
            "unsupported desktop settings schema {}; expected {SETTINGS_SCHEMA}",
            settings.schema
        ));
    }
    let legacy_generated_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve the legacy model directory: {error}"))?
        .join("models");
    if PathBuf::from(&settings.install_root) == legacy_generated_root {
        settings.install_root = default_settings(app)?.install_root;
    }
    Ok(settings)
}

fn validate_profile_name(value: &str) -> Result<&str, String> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    valid
        .then_some(value)
        .ok_or_else(|| "the default profile name is invalid".to_owned())
}

fn available_profiles(install_root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(install_root.join("profiles")) else {
        return Vec::new();
    };
    let mut profiles = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|value| value.to_str()) == Some("json"))
                .then(|| path.file_stem()?.to_str().map(str::to_owned))?
        })
        .collect::<Vec<_>>();
    profiles.sort();
    profiles
}

fn runtime_snapshot(settings: &DesktopSettings) -> RuntimeSnapshot {
    let install_root = Path::new(&settings.install_root);
    let available_profiles = available_profiles(install_root);
    match Alpine::resolve_session(install_root, Some(&settings.default_profile), true) {
        Ok(resolved) => {
            let model = resolved
                .model
                .file_name()
                .and_then(|filename| filename.to_str())
                .map(str::to_owned);
            match Alpine::session_status(install_root, Duration::from_secs(4)) {
                Ok(status) if status.active && status.healthy => RuntimeSnapshot {
                    state: "running".to_owned(),
                    profile: resolved.profile_name,
                    model,
                    detail: "A verified local llama.cpp session is running.".to_owned(),
                    available_profiles,
                },
                Ok(_) => RuntimeSnapshot {
                    state: "configured".to_owned(),
                    profile: resolved.profile_name,
                    model,
                    detail: "The runtime is configured and will start with the next Pi task."
                        .to_owned(),
                    available_profiles,
                },
                Err(error) => RuntimeSnapshot {
                    state: "unavailable".to_owned(),
                    profile: resolved.profile_name,
                    model,
                    detail: format!("Runtime status could not be verified: {error}"),
                    available_profiles,
                },
            }
        }
        Err(error) => RuntimeSnapshot {
            state: "unconfigured".to_owned(),
            profile: settings.default_profile.clone(),
            model: None,
            detail: error.to_string(),
            available_profiles,
        },
    }
}

fn write_settings(app: &AppHandle, settings: &DesktopSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    let value = serde_json::to_vec_pretty(settings)
        .map_err(|error| format!("failed to encode Alpine settings: {error}"))?;
    std::fs::write(&path, value)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

#[tauri::command]
fn bootstrap_snapshot(app: AppHandle) -> Result<BootstrapSnapshot, String> {
    let (hardware, _) = capture_capacity()?;
    let settings = read_settings(&app)?;
    let runtime = runtime_snapshot(&settings);
    Ok(BootstrapSnapshot {
        hardware,
        settings,
        runtime,
    })
}

#[tauri::command]
fn update_settings(app: AppHandle, update: SettingsUpdate) -> Result<DesktopSettings, String> {
    let install_root = PathBuf::from(update.install_root.trim());
    if !install_root.is_absolute() {
        return Err("the Alpine installation root must be an absolute path".to_owned());
    }
    let default_profile = validate_profile_name(update.default_profile.trim())?.to_owned();
    let mut settings = read_settings(&app)?;
    settings.install_root = install_root.to_string_lossy().into_owned();
    settings.default_profile = default_profile;
    settings.local_metrics_enabled = update.local_metrics_enabled;
    write_settings(&app, &settings)?;
    Ok(settings)
}

fn read_response(mut response: ureq::http::Response<ureq::Body>) -> Result<String, String> {
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| format!("failed to read Hugging Face response: {error}"))
}

#[tauri::command]
fn search_models(query: String) -> Result<Vec<ModelSearchResult>, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 120 {
        return Err("model search must contain between 1 and 120 characters".to_owned());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .user_agent("Alpine-Desktop/0.1")
        .build()
        .into();
    let response = agent
        .get("https://huggingface.co/api/models")
        .query("search", query)
        .query("filter", "gguf")
        .query("sort", "downloads")
        .query("direction", "-1")
        .query("limit", "8")
        .query("full", "true")
        .call()
        .map_err(|error| format!("Hugging Face search failed: {error}"))?;
    let mut models = catalog::decode_hugging_face_models(&read_response(response)?)?;
    for model in &mut models {
        let url = format!("https://huggingface.co/api/models/{}/tree/main", model.id);
        if let Ok(response) = agent
            .get(&url)
            .query("recursive", "true")
            .query("expand", "false")
            .call()
        {
            if let Ok(body) = read_response(response) {
                let _ = catalog::hydrate_model_artifacts(model, &body);
            }
        }
    }
    Ok(models)
}

#[tauri::command]
fn assess_model(artifact_bytes: u64) -> Result<ModelAssessment, String> {
    if artifact_bytes == 0 {
        return Err("artifact size is required before estimating model fit".to_owned());
    }
    let (_, capacity) = capture_capacity()?;
    Ok(assessment::assess_model(&capacity, artifact_bytes))
}

#[tauri::command]
fn set_default_model(
    app: AppHandle,
    mut selection: ModelSelection,
) -> Result<DesktopSettings, String> {
    if selection.repo_id.trim().is_empty() {
        return Err("the default model must identify an exact GGUF artifact".to_owned());
    }
    let artifact_path = catalog::validated_remote_artifact_path(selection.filename.trim())?;
    selection.filename = Path::new(artifact_path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "the default model filename is invalid".to_owned())?
        .to_owned();
    let mut settings = read_settings(&app)?;
    settings.default_model = Some(selection);
    write_settings(&app, &settings)?;
    Ok(settings)
}

fn resolve_pi_launch_blocking(app: AppHandle) -> Result<PiLaunchConfig, String> {
    let settings = read_settings(&app)?;
    let selected = settings
        .default_model
        .ok_or_else(|| "choose a default model before starting a Pi task".to_owned())?;
    let install_root = Path::new(&settings.install_root);
    let resolved = Alpine::resolve_session(install_root, Some(&settings.default_profile), true)
        .map_err(|error| format!("the local runtime is not configured: {error}"))?;
    let runtime_model = resolved
        .model
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "the configured runtime model filename is invalid".to_owned())?;
    if !runtime_model.eq_ignore_ascii_case(&selected.filename) {
        return Err(format!(
            "the selected default '{}' is not the model configured for profile '{}'; select the active runtime model or configure this artifact before launching Pi",
            selected.filename, settings.default_profile
        ));
    }
    Alpine::start_session(&StartSessionOptions {
        install_root: install_root.to_path_buf(),
        profile: Some(settings.default_profile),
        vision: false,
        force_fallback: false,
        lock_timeout: Duration::from_secs(15),
        startup_timeout: Duration::from_secs(600),
    })
    .map_err(|error| format!("the local llama.cpp session could not start: {error}"))?;
    let api_key = std::fs::read_to_string(&resolved.api_key_file)
        .map_err(|error| format!("failed to read the local runtime credential: {error}"))?
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned();
    if api_key.is_empty() {
        return Err("the local runtime credential is empty".to_owned());
    }
    Ok(PiLaunchConfig {
        model_id: selected.filename,
        base_url: resolved.base_url,
        api_key,
        context_window: resolved.profile.context,
        max_tokens: resolved.profile.output,
        temperature: 0.2,
    })
}

#[tauri::command]
async fn resolve_pi_launch(app: AppHandle) -> Result<PiLaunchConfig, String> {
    tauri::async_runtime::spawn_blocking(move || resolve_pi_launch_blocking(app))
        .await
        .map_err(|error| format!("Pi launch worker failed: {error}"))?
}

fn decode_probe_response(body: &str) -> Result<(bool, Option<u64>), String> {
    let response: ProbeResponse = serde_json::from_str(body)
        .map_err(|error| format!("the local runtime returned invalid completion JSON: {error}"))?;
    let content = response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_deref())
        .ok_or_else(|| "the local runtime returned no assistant content".to_owned())?;
    Ok((
        content.trim() == "ALPINE_OK",
        response.usage.and_then(|usage| usage.completion_tokens),
    ))
}

fn run_runtime_probe_blocking(app: AppHandle) -> Result<RuntimeProbeReport, String> {
    let settings = read_settings(&app)?;
    let profile = settings.default_profile.clone();
    let launch = resolve_pi_launch_blocking(app)?;
    let url = format!("{}/chat/completions", launch.base_url.trim_end_matches('/'));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent("Alpine-Desktop/0.1")
        .build()
        .into();
    let started = Instant::now();
    let response = agent
        .post(&url)
        .header("Authorization", format!("Bearer {}", launch.api_key))
        .send_json(serde_json::json!({
            "model": launch.model_id,
            "messages": [{"role": "user", "content": "Reply with exactly ALPINE_OK and nothing else."}],
            "temperature": 0,
            "max_tokens": 32,
            "stream": false
        }))
        .map_err(|error| format!("the local runtime probe failed: {error}"))?;
    let latency_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    let body = read_response(response)?;
    let (quality_pass, output_tokens) = decode_probe_response(&body)?;
    Ok(RuntimeProbeReport {
        model: launch.model_id,
        profile,
        latency_ms,
        output_tokens,
        quality_pass,
        evidence_label: "Measured diagnostic — not qualification",
    })
}

#[tauri::command]
async fn run_runtime_probe(app: AppHandle) -> Result<RuntimeProbeReport, String> {
    tauri::async_runtime::spawn_blocking(move || run_runtime_probe_blocking(app))
        .await
        .map_err(|error| format!("runtime probe worker failed: {error}"))?
}

fn validated_repo_id(value: &str) -> Result<&str, String> {
    let segments = value.split('/').collect::<Vec<_>>();
    let valid = segments.len() == 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= 96
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        });
    valid
        .then_some(value)
        .ok_or_else(|| "the Hugging Face repository identifier is invalid".to_owned())
}

fn validated_sha256(value: Option<String>) -> Result<Option<String>, String> {
    value
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                Ok(value)
            } else {
                Err("the expected model SHA-256 digest is invalid".to_owned())
            }
        })
        .transpose()
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("failed to open {} for hashing: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(encoded)
}

fn validate_download(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: Option<&str>,
) -> Result<u64, String> {
    let bytes = path
        .metadata()
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
        .len();
    if bytes == 0 {
        return Err("model artifact is empty".to_owned());
    }
    if expected_bytes > 0 && bytes != expected_bytes {
        return Err(format!(
            "model byte count mismatch: expected {expected_bytes}, received {bytes}"
        ));
    }
    if let Some(expected) = expected_sha256 {
        let observed = file_sha256(path)?;
        if observed != expected {
            return Err(format!(
                "model SHA-256 mismatch: expected {expected}, received {observed}"
            ));
        }
    }
    Ok(bytes)
}

fn download_model_blocking(
    app: AppHandle,
    selection: ModelSelection,
    expected_bytes: u64,
    expected_sha256: Option<String>,
    cancelled: Arc<AtomicBool>,
) -> Result<DownloadReceipt, String> {
    let repo_id = validated_repo_id(selection.repo_id.trim())?;
    let remote_path = catalog::validated_remote_artifact_path(selection.filename.trim())?;
    let filename = Path::new(remote_path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "the remote artifact has no usable filename".to_owned())?;
    let filename = catalog::validated_artifact_filename(filename)?;
    let expected_sha256 = validated_sha256(expected_sha256)?;
    let settings = read_settings(&app)?;
    let model_root = PathBuf::from(settings.install_root).join("models");
    std::fs::create_dir_all(&model_root)
        .map_err(|error| format!("failed to create {}: {error}", model_root.display()))?;
    let destination = model_root.join(filename);
    if destination.exists() {
        let bytes = validate_download(&destination, expected_bytes, expected_sha256.as_deref())
            .map_err(|error| {
                format!("the existing model is not the requested artifact: {error}")
            })?;
        return Ok(DownloadReceipt {
            path: destination.to_string_lossy().into_owned(),
            bytes_written: bytes,
            already_present: true,
        });
    }
    let partial = model_root.join(format!("{filename}.part"));
    let partial_bytes = partial
        .metadata()
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if expected_bytes > 0 && partial_bytes == expected_bytes {
        if let Ok(bytes) = validate_download(&partial, expected_bytes, expected_sha256.as_deref()) {
            std::fs::rename(&partial, &destination).map_err(|error| {
                format!(
                    "failed to finalize {} as {}: {error}",
                    partial.display(),
                    destination.display()
                )
            })?;
            return Ok(DownloadReceipt {
                path: destination.to_string_lossy().into_owned(),
                bytes_written: bytes,
                already_present: false,
            });
        }
    }
    let resume_from =
        if partial_bytes > 0 && (expected_bytes == 0 || partial_bytes < expected_bytes) {
            partial_bytes
        } else {
            0
        };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60 * 60)))
        .user_agent("Alpine-Desktop/0.1")
        .build()
        .into();
    let mut request = agent.get(&catalog::download_url(repo_id, remote_path));
    if resume_from > 0 {
        request = request.header("Range", format!("bytes={resume_from}-"));
    }
    let mut response = request
        .call()
        .map_err(|error| format!("Hugging Face download failed: {error}"))?;
    let append = resume_from > 0 && response.status() == ureq::http::StatusCode::PARTIAL_CONTENT;
    let initial_bytes = if append { resume_from } else { 0 };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(&partial)
        .map_err(|error| format!("failed to create {}: {error}", partial.display()))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut reader = response.body_mut().as_reader();
    let mut bytes_written = initial_bytes;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            writer.flush().map_err(|error| {
                format!(
                    "failed to flush cancelled transfer {}: {error}",
                    partial.display()
                )
            })?;
            return Err("model download cancelled; the partial file was retained".to_owned());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("failed while reading the model response: {error}"))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| format!("failed while writing {}: {error}", partial.display()))?;
        bytes_written = bytes_written.saturating_add(read as u64);
    }
    writer
        .flush()
        .map_err(|error| format!("failed to flush {}: {error}", partial.display()))?;
    drop(writer);
    validate_download(&partial, expected_bytes, expected_sha256.as_deref()).map_err(|error| {
        format!("download validation failed for {filename}: {error}; the partial file was retained")
    })?;
    std::fs::rename(&partial, &destination).map_err(|error| {
        format!(
            "failed to finalize {} as {}: {error}",
            partial.display(),
            destination.display()
        )
    })?;
    Ok(DownloadReceipt {
        path: destination.to_string_lossy().into_owned(),
        bytes_written,
        already_present: false,
    })
}

#[tauri::command]
async fn download_model(
    app: AppHandle,
    registry: State<'_, DownloadRegistry>,
    selection: ModelSelection,
    expected_bytes: u64,
    expected_sha256: Option<String>,
) -> Result<DownloadReceipt, String> {
    let key = format!("{}\n{}", selection.repo_id, selection.filename);
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut active = registry
            .0
            .lock()
            .map_err(|_| "the download registry is unavailable".to_owned())?;
        if active.contains_key(&key) {
            return Err("this model download is already active".to_owned());
        }
        active.insert(key.clone(), Arc::clone(&cancelled));
    }
    let active = Arc::clone(&registry.0);
    let result = tauri::async_runtime::spawn_blocking(move || {
        download_model_blocking(app, selection, expected_bytes, expected_sha256, cancelled)
    })
    .await
    .map_err(|error| format!("model download worker failed: {error}"));
    if let Ok(mut downloads) = active.lock() {
        downloads.remove(&key);
    }
    result?
}

#[tauri::command]
fn cancel_download(
    registry: State<'_, DownloadRegistry>,
    selection: ModelSelection,
) -> Result<bool, String> {
    let key = format!("{}\n{}", selection.repo_id, selection.filename);
    let active = registry
        .0
        .lock()
        .map_err(|_| "the download registry is unavailable".to_owned())?;
    Ok(active.get(&key).is_some_and(|cancelled| {
        cancelled.store(true, Ordering::Relaxed);
        true
    }))
}

#[tauri::command]
fn list_downloads(app: AppHandle) -> Result<Vec<DownloadedModel>, String> {
    let model_root = PathBuf::from(read_settings(&app)?.install_root).join("models");
    let Ok(entries) = std::fs::read_dir(&model_root) else {
        return Ok(Vec::new());
    };
    let mut models = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            let filename = entry.file_name().to_str()?.to_owned();
            let state = if filename.to_ascii_lowercase().ends_with(".gguf") {
                "installed"
            } else if filename.to_ascii_lowercase().ends_with(".gguf.part") {
                "partial"
            } else {
                return None;
            };
            Some(DownloadedModel {
                filename,
                size_bytes: metadata.len(),
                state,
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| left.filename.cmp(&right.filename));
    Ok(models)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(DownloadRegistry::default())
        .invoke_handler(tauri::generate_handler![
            bootstrap_snapshot,
            update_settings,
            search_models,
            assess_model,
            set_default_model,
            resolve_pi_launch,
            run_runtime_probe,
            download_model,
            cancel_download,
            list_downloads,
        ])
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::decode_probe_response;

    #[test]
    fn runtime_probe_requires_exact_visible_output() {
        let passing =
            r#"{"choices":[{"message":{"content":"ALPINE_OK"}}],"usage":{"completion_tokens":4}}"#;
        let reasoning = r#"{"choices":[{"message":{"content":"Reasoning... ALPINE_OK"}}],"usage":{"completion_tokens":9}}"#;

        assert_eq!(decode_probe_response(passing).unwrap(), (true, Some(4)));
        assert_eq!(decode_probe_response(reasoning).unwrap(), (false, Some(9)));
    }
}
