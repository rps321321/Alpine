pub mod assessment;
pub mod browser;
pub mod catalog;
pub mod store;
pub mod supervisor;
pub mod workspace;

use alpine_control_plane::{
    Alpine, EvaluationOptions, EvaluationPlan, QualificationTarget, SessionStatus,
    StartSessionOptions, StopSessionOptions,
};
use assessment::{HardwareCapacity, ModelAssessment, PlacementPlan};
use browser::BrowserRegistry;
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
use store::{
    CreateTask, DesktopProject, DesktopStore, DesktopTask, ModelRegistryEntry, ModelSource,
    NewExecutionSpecification, RegisterModelArtifact, TaskDetail, ToolApproval,
};
use supervisor::TaskSupervisor;
use tauri::{AppHandle, Emitter, Manager, State};
use workspace::{WorkspaceEntry, WorkspaceRead, WorkspaceSearchMatch};

const SETTINGS_SCHEMA: u32 = 4;
const PI_ADAPTER_IDENTITY: &str = "pi-agent-core@0.84.2";
const PI_POLICY_IDENTITY: &str = "alpine-desktop-project-tools-v1";

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
    #[serde(default)]
    registry_id: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
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
    #[serde(default)]
    evaluation_repository_root: String,
    #[serde(default)]
    browser_allowed_hosts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsUpdate {
    install_root: String,
    default_profile: String,
    local_metrics_enabled: bool,
    evaluation_repository_root: String,
    browser_allowed_hosts: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HardwareProfile {
    cpu: String,
    memory_bytes: u64,
    gpu: Option<String>,
    vram_bytes: u64,
    driver: Option<String>,
    platform: String,
    architecture: String,
    os_version: Option<String>,
    physical_cores: Option<usize>,
    logical_processors: usize,
    compute_devices: Vec<ComputeDevice>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComputeDevice {
    name: String,
    memory_bytes: u64,
    driver: String,
    backend: &'static str,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiLaunchConfig {
    pub(crate) model_id: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) context_window: u32,
    pub(crate) max_tokens: u32,
    pub(crate) temperature: f32,
    pub(crate) specification: NewExecutionSpecification,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationProgress {
    state: &'static str,
    scope: String,
    message: String,
}

fn same_inference_session_state(before: &SessionStatus, after: &SessionStatus) -> bool {
    before.active == after.active
        && before.healthy == after.healthy
        && before.foreign == after.foreign
        && before.profile == after.profile
        && before.vision == after.vision
        && before.runtime == after.runtime
        && before.fallback == after.fallback
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FullEvaluationSummary {
    evaluation_id: String,
    scope: String,
    plan_id: String,
    plan_sha256: String,
    decision: String,
    production_decision: Option<String>,
    selected_profile: Option<String>,
    recommendation: String,
    artifact_path: String,
    tuning_measurements: Vec<alpine_control_plane::ProfileMeasurement>,
    tuning: Option<serde_json::Value>,
    final_evidence: Option<serde_json::Value>,
    candidate_qualification: Option<serde_json::Value>,
    validated_qualification: Option<serde_json::Value>,
    production_qualification: Option<serde_json::Value>,
    same_process_requests: Option<u32>,
    clean_restarts: Option<u32>,
    near_limit_context_tokens: Option<u32>,
    golden_tool_calls: Option<u64>,
    golden_tool_failures: Option<u64>,
    rollback_profile: &'static str,
    rollback_proved: bool,
    prior_session_restored: bool,
    deployment_changed: bool,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    repo_id: String,
    filename: String,
    bytes_written: u64,
    total_bytes: u64,
    state: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadedModel {
    registry_id: Option<String>,
    filename: String,
    size_bytes: u64,
    state: &'static str,
    source: Option<String>,
    repo_id: Option<String>,
    revision: Option<String>,
    sha256: Option<String>,
    local_path: String,
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
    let compute_devices = report
        .snapshot
        .nvidia_gpus
        .iter()
        .map(|gpu| ComputeDevice {
            name: gpu.name.clone(),
            memory_bytes: gpu.vram_mib.saturating_mul(1024 * 1024),
            driver: gpu.driver_version.clone(),
            backend: "cuda",
        })
        .collect();
    Ok((
        HardwareProfile {
            cpu: report.snapshot.cpu.brand,
            memory_bytes: report.snapshot.physical_memory_bytes,
            gpu: primary_gpu.map(|gpu| gpu.name.clone()),
            vram_bytes,
            driver: primary_gpu.map(|gpu| gpu.driver_version.clone()),
            platform: report.snapshot.platform.os,
            architecture: report.snapshot.platform.architecture,
            os_version: report.snapshot.platform.os_version,
            physical_cores: report.snapshot.cpu.physical_cores,
            logical_processors: report.snapshot.cpu.logical_processors,
            compute_devices,
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
                registry_id: None,
                revision: None,
                sha256: None,
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
        evaluation_repository_root: discover_evaluation_repository_root()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
        browser_allowed_hosts: Vec::new(),
    })
}

fn discover_evaluation_repository_root() -> Option<PathBuf> {
    let mut starts = Vec::new();
    if let Ok(path) = std::env::current_dir() {
        starts.push(path);
    }
    if let Ok(path) = std::env::current_exe()
        && let Some(parent) = path.parent()
    {
        starts.push(parent.to_path_buf());
    }
    starts.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    for start in starts {
        for candidate in start.ancestors().take(8) {
            if candidate.join("config/evaluation-plan.json").is_file()
                && candidate.join("benchmarks/micro/workloads.json").is_file()
            {
                return candidate.canonicalize().ok();
            }
        }
    }
    None
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
    if settings.schema == 0 || settings.schema > SETTINGS_SCHEMA {
        return Err(format!(
            "unsupported desktop settings schema {}; expected {SETTINGS_SCHEMA}",
            settings.schema
        ));
    }
    settings.schema = SETTINGS_SCHEMA;
    if settings.evaluation_repository_root.is_empty() {
        settings.evaluation_repository_root = discover_evaluation_repository_root()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
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
fn update_settings(
    app: AppHandle,
    browser_registry: State<'_, BrowserRegistry>,
    update: SettingsUpdate,
) -> Result<DesktopSettings, String> {
    let install_root = PathBuf::from(update.install_root.trim());
    if !install_root.is_absolute() {
        return Err("the Alpine installation root must be an absolute path".to_owned());
    }
    let default_profile = validate_profile_name(update.default_profile.trim())?.to_owned();
    let evaluation_repository_root = PathBuf::from(update.evaluation_repository_root.trim());
    if !evaluation_repository_root.is_absolute() {
        return Err("the evaluation repository root must be an absolute path".to_owned());
    }
    if !evaluation_repository_root
        .join("config/evaluation-plan.json")
        .is_file()
        || !evaluation_repository_root
            .join("benchmarks/micro/workloads.json")
            .is_file()
    {
        return Err(
            "the evaluation repository root must contain Alpine config and benchmark resources"
                .to_owned(),
        );
    }
    let mut settings = read_settings(&app)?;
    settings.install_root = install_root.to_string_lossy().into_owned();
    settings.default_profile = default_profile;
    settings.local_metrics_enabled = update.local_metrics_enabled;
    settings.browser_allowed_hosts = validate_browser_hosts(update.browser_allowed_hosts)?;
    settings.evaluation_repository_root = evaluation_repository_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve evaluation repository root: {error}"))?
        .to_string_lossy()
        .into_owned();
    write_settings(&app, &settings)?;
    browser_registry.replace_persistent_hosts(settings.browser_allowed_hosts.clone());
    Ok(settings)
}

fn validate_browser_hosts(values: Vec<String>) -> Result<Vec<String>, String> {
    let mut hosts = values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if hosts.iter().any(|host| {
        host.len() > 253
            || host.contains('/')
            || host.contains(':')
            || host.chars().any(char::is_whitespace)
    }) {
        return Err("a saved browser host is invalid".to_owned());
    }
    hosts.sort();
    hosts.dedup();
    Ok(hosts)
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
fn plan_model_placement(artifact_bytes: u64) -> Result<PlacementPlan, String> {
    if artifact_bytes == 0 {
        return Err("artifact size is required before planning model placement".to_owned());
    }
    let (_, capacity) = capture_capacity()?;
    Ok(assessment::plan_placement(&capacity, artifact_bytes))
}

fn start_runtime_blocking(app: AppHandle) -> Result<RuntimeSnapshot, String> {
    let settings = read_settings(&app)?;
    Alpine::start_session(&StartSessionOptions {
        install_root: PathBuf::from(&settings.install_root),
        profile: Some(settings.default_profile.clone()),
        vision: false,
        force_fallback: false,
        lock_timeout: Duration::from_secs(15),
        startup_timeout: Duration::from_secs(600),
    })
    .map_err(|error| format!("the local llama.cpp session could not start: {error}"))?;
    Ok(runtime_snapshot(&settings))
}

#[tauri::command]
async fn start_runtime(app: AppHandle) -> Result<RuntimeSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || start_runtime_blocking(app))
        .await
        .map_err(|error| format!("runtime start worker failed: {error}"))?
}

fn stop_runtime_blocking(app: AppHandle) -> Result<RuntimeSnapshot, String> {
    let settings = read_settings(&app)?;
    Alpine::stop_session(&StopSessionOptions {
        install_root: PathBuf::from(&settings.install_root),
        lock_timeout: Duration::from_secs(15),
        allow_legacy_identity: false,
    })
    .map_err(|error| format!("the verified local llama.cpp session could not stop: {error}"))?;
    Ok(runtime_snapshot(&settings))
}

#[tauri::command]
async fn stop_runtime(app: AppHandle) -> Result<RuntimeSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || stop_runtime_blocking(app))
        .await
        .map_err(|error| format!("runtime stop worker failed: {error}"))?
}

#[tauri::command]
fn set_default_model(
    app: AppHandle,
    store: State<'_, Arc<DesktopStore>>,
    selection: ModelSelection,
) -> Result<DesktopSettings, String> {
    let registered = store
        .list_model_artifacts()
        .map_err(|error| error.to_string())?;
    let selection = registered_default_selection(selection, &registered)?;
    let mut settings = read_settings(&app)?;
    settings.default_model = Some(selection);
    write_settings(&app, &settings)?;
    Ok(settings)
}

fn registered_default_selection(
    mut selection: ModelSelection,
    registered: &[ModelRegistryEntry],
) -> Result<ModelSelection, String> {
    if selection.repo_id.trim().is_empty() {
        return Err("the default model must identify an exact GGUF artifact".to_owned());
    }
    let artifact_path = catalog::validated_remote_artifact_path(selection.filename.trim())?;
    selection.filename = Path::new(artifact_path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "the default model filename is invalid".to_owned())?
        .to_owned();
    let registry_id = selection.registry_id.as_deref().ok_or_else(|| {
        "the default model must reference a verified Model Registry entry".to_owned()
    })?;
    let artifact = registered
        .iter()
        .find(|artifact| artifact.id == registry_id)
        .ok_or_else(|| "the selected Model Registry entry no longer exists".to_owned())?;
    if artifact.filename != selection.filename {
        return Err("the selected filename does not match its Model Registry entry".to_owned());
    }
    if !selection
        .sha256
        .as_deref()
        .is_some_and(|digest| digest.eq_ignore_ascii_case(&artifact.sha256))
    {
        return Err("the selected digest does not match its Model Registry entry".to_owned());
    }
    match artifact.source {
        ModelSource::HuggingFace => {
            if artifact.repo_id.as_deref() != Some(selection.repo_id.as_str())
                || artifact.revision != selection.revision
            {
                return Err(
                    "the selected Hugging Face repository or revision does not match its Model Registry entry"
                        .to_owned(),
                );
            }
        }
        ModelSource::Import => {
            let expected = format!("local/import/{}", artifact.sha256);
            if selection.repo_id != expected || selection.revision.is_some() {
                return Err(
                    "the selected imported model identity does not match its Model Registry entry"
                        .to_owned(),
                );
            }
        }
    }
    selection.registry_id = Some(artifact.id.clone());
    selection.revision = artifact.revision.clone();
    selection.sha256 = Some(artifact.sha256.clone());
    Ok(selection)
}

pub(crate) fn resolve_pi_launch_blocking(app: AppHandle) -> Result<PiLaunchConfig, String> {
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
    let model_registry_id = selected.registry_id.ok_or_else(|| {
        "the selected default model has no verified Model Registry identity".to_owned()
    })?;
    let model_sha256 = selected
        .sha256
        .ok_or_else(|| "the selected default model has no verified SHA-256 identity".to_owned())?;
    let model_id = selected.filename;
    let runtime_identity = file_sha256(&resolved.server)?;
    let context_window = resolved.profile.context;
    let max_tokens = resolved.profile.output;
    let specification = NewExecutionSpecification {
        model_registry_id,
        model_repo_id: selected.repo_id,
        model_revision: selected.revision,
        model_filename: model_id.clone(),
        model_sha256,
        session_config_sha256: resolved.session_config_sha256,
        profile_name: resolved.profile_name,
        profile_sha256: resolved.profile_sha256,
        runtime_name: resolved.runtime_name,
        runtime_identity,
        adapter_identity: PI_ADAPTER_IDENTITY.to_owned(),
        policy_identity: PI_POLICY_IDENTITY.to_owned(),
        context_window,
        max_tokens,
        temperature_millis: 200,
    };
    Ok(PiLaunchConfig {
        model_id,
        base_url: resolved.base_url,
        api_key,
        context_window,
        max_tokens,
        temperature: 0.2,
        specification,
    })
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

fn serialized_label(value: impl Serialize) -> Result<String, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("failed to encode evaluation decision: {error}"))?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "evaluation decision did not encode as a string".to_owned())
}

fn run_full_evaluation_blocking(
    app: AppHandle,
    scope: String,
) -> Result<FullEvaluationSummary, String> {
    let target = match scope.as_str() {
        "candidate" => QualificationTarget::Candidate,
        "validated" => QualificationTarget::Validated,
        "production" => QualificationTarget::Production,
        _ => return Err("evaluation scope must be candidate, validated, or production".to_owned()),
    };
    let settings = read_settings(&app)?;
    if settings.evaluation_repository_root.is_empty() {
        return Err("configure the Alpine evaluation repository root in Settings".to_owned());
    }
    let repository_root = PathBuf::from(&settings.evaluation_repository_root)
        .canonicalize()
        .map_err(|error| format!("failed to resolve evaluation repository root: {error}"))?;
    let install_root = PathBuf::from(&settings.install_root);
    let prior_session = Alpine::session_status(&install_root, Duration::from_secs(15))
        .map_err(|error| format!("failed to capture the prior Inference Session: {error}"))?;
    let source_plan_path = repository_root.join("config/evaluation-plan.json");
    let source_plan = std::fs::read(&source_plan_path).map_err(|error| {
        format!(
            "failed to read evaluation plan {}: {error}",
            source_plan_path.display()
        )
    })?;
    let mut plan: EvaluationPlan = serde_json::from_slice(&source_plan).map_err(|error| {
        format!(
            "invalid evaluation plan {}: {error}",
            source_plan_path.display()
        )
    })?;
    plan.target = target;
    plan.id = format!("{}-desktop-{scope}", plan.id);
    let result_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve evaluation evidence directory: {error}"))?
        .join("evidence");
    let plan_root = result_root.join("plans");
    std::fs::create_dir_all(&plan_root)
        .map_err(|error| format!("failed to create {}: {error}", plan_root.display()))?;
    let plan_path = plan_root.join(format!("{scope}.json"));
    std::fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&plan)
            .map_err(|error| format!("failed to encode desktop evaluation plan: {error}"))?,
    )
    .map_err(|error| format!("failed to write {}: {error}", plan_path.display()))?;
    let _ = app.emit(
        "evaluation-progress",
        EvaluationProgress {
            state: "running",
            scope: scope.clone(),
            message: format!(
                "Measuring {} against {} with {} measured runs per workload",
                std::iter::once(&plan.baseline_profile)
                    .chain(plan.candidate_profiles.iter())
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" and "),
                plan.workloads.join(", "),
                plan.microbenchmark.runs
            ),
        },
    );
    let report = Alpine::run_evaluation(&EvaluationOptions {
        repository_root: repository_root.clone(),
        install_root: install_root.clone(),
        result_root: result_root.clone(),
        plan: plan_path,
        allow_legacy_identity: false,
    })
    .map_err(|error| error.to_string())?;
    let restored_session = Alpine::session_status(&install_root, Duration::from_secs(15))
        .map_err(|error| format!("evaluation completed but the restored Inference Session could not be verified: {error}"))?;
    let prior_session_restored = same_inference_session_state(&prior_session, &restored_session);
    let database = result_root.join("results.sqlite3");
    let final_evidence = report
        .final_run_id
        .as_deref()
        .map(|id| Alpine::run_evidence(&database, id).map_err(|error| error.to_string()))
        .transpose()?
        .map(|evidence| {
            serde_json::to_value(evidence)
                .map_err(|error| format!("failed to encode final run evidence: {error}"))
        })
        .transpose()?;
    let tuning = report
        .tuning
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("failed to encode tuning report: {error}"))?;
    let recommendation = match (&report.selected_profile, &report.tuning) {
        (Some(profile), Some(tuning)) if tuning.reasons.is_empty() => {
            format!("Use {profile} for this Evidence Identity; no Deployment Role was changed.")
        }
        (Some(profile), Some(tuning)) => format!(
            "Use {profile} for this Evidence Identity. {} No Deployment Role was changed.",
            tuning.reasons.join(" ")
        ),
        (Some(profile), None) => {
            format!("{profile} completed measurement; no Deployment Role was changed.")
        }
        (None, Some(tuning)) if !tuning.reasons.is_empty() => format!(
            "Retain the baseline: {} No Deployment Role was changed.",
            tuning.reasons.join(" ")
        ),
        _ => "No configuration was proven. Retain the current default and inspect the evidence."
            .to_owned(),
    };
    let summary = FullEvaluationSummary {
        evaluation_id: report.evaluation_id.clone(),
        scope: scope.clone(),
        plan_id: report.plan_id.clone(),
        plan_sha256: report.plan_sha256.clone(),
        decision: serialized_label(report.decision)?,
        production_decision: report
            .production_decision
            .map(serialized_label)
            .transpose()?,
        selected_profile: report.selected_profile.clone(),
        recommendation,
        artifact_path: report.artifact_path.to_string_lossy().into_owned(),
        tuning_measurements: report.tuning_measurements,
        tuning,
        final_evidence,
        candidate_qualification: report
            .candidate_qualification
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| format!("failed to encode candidate qualification: {error}"))?,
        validated_qualification: report
            .validated_qualification
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| format!("failed to encode validated qualification: {error}"))?,
        production_qualification: report
            .production_qualification
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| format!("failed to encode production qualification: {error}"))?,
        same_process_requests: report
            .same_process_stability
            .as_ref()
            .map(|value| value.requests),
        clean_restarts: report
            .clean_restart_stability
            .as_ref()
            .map(|value| value.clean_restarts),
        near_limit_context_tokens: report
            .near_limit_context
            .as_ref()
            .map(|value| value.actual_prompt_tokens),
        golden_tool_calls: report.golden_agent.as_ref().map(|value| value.tool_calls),
        golden_tool_failures: report
            .golden_agent
            .as_ref()
            .map(|value| value.tool_failures),
        rollback_profile: "stable-16k",
        rollback_proved: report
            .rollback_proof
            .as_ref()
            .is_some_and(|value| value.restored_prior_session),
        prior_session_restored,
        deployment_changed: false,
    };
    let _ = app.emit(
        "evaluation-progress",
        EvaluationProgress {
            state: "completed",
            scope,
            message: format!(
                "Evaluation {} completed with decision {}",
                summary.evaluation_id, summary.decision
            ),
        },
    );
    Ok(summary)
}

#[tauri::command]
async fn run_full_evaluation(
    app: AppHandle,
    scope: String,
) -> Result<FullEvaluationSummary, String> {
    let worker_app = app.clone();
    let worker_scope = scope.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_full_evaluation_blocking(worker_app, worker_scope)
    })
    .await
    .map_err(|error| format!("evaluation worker failed: {error}"))?;
    if let Err(error) = &result {
        let _ = app.emit(
            "evaluation-progress",
            EvaluationProgress {
                state: "failed",
                scope,
                message: error.clone(),
            },
        );
    }
    result
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
    revision: String,
    expected_bytes: u64,
    expected_sha256: Option<String>,
    cancelled: Arc<AtomicBool>,
) -> Result<DownloadReceipt, String> {
    let repo_id = validated_repo_id(selection.repo_id.trim())?;
    let revision = catalog::validated_revision(&revision)?;
    let remote_path = catalog::validated_remote_artifact_path(selection.filename.trim())?;
    let filename = Path::new(remote_path)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "the remote artifact has no usable filename".to_owned())?;
    let filename = catalog::validated_artifact_filename(filename)?;
    let emit_progress = |bytes_written: u64, state: &'static str| {
        let _ = app.emit(
            "download-progress",
            DownloadProgress {
                repo_id: repo_id.to_owned(),
                filename: filename.to_owned(),
                bytes_written,
                total_bytes: expected_bytes,
                state,
            },
        );
    };
    let expected_sha256 = validated_sha256(expected_sha256)?;
    let settings = read_settings(&app)?;
    let model_root = PathBuf::from(settings.install_root).join("models");
    std::fs::create_dir_all(&model_root)
        .map_err(|error| format!("failed to create {}: {error}", model_root.display()))?;
    let destination = model_root.join(filename);
    if destination.exists() {
        emit_progress(0, "validating");
        let bytes = validate_download(&destination, expected_bytes, expected_sha256.as_deref())
            .map_err(|error| {
                format!("the existing model is not the requested artifact: {error}")
            })?;
        emit_progress(bytes, "completed");
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
        emit_progress(partial_bytes, "validating");
        if let Ok(bytes) = validate_download(&partial, expected_bytes, expected_sha256.as_deref()) {
            std::fs::rename(&partial, &destination).map_err(|error| {
                format!(
                    "failed to finalize {} as {}: {error}",
                    partial.display(),
                    destination.display()
                )
            })?;
            emit_progress(bytes, "completed");
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
    let mut request = agent.get(&catalog::download_url(repo_id, revision, remote_path));
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
    let mut last_progress = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    emit_progress(bytes_written, "downloading");
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        if cancelled.load(Ordering::Relaxed) {
            writer.flush().map_err(|error| {
                format!(
                    "failed to flush cancelled transfer {}: {error}",
                    partial.display()
                )
            })?;
            emit_progress(bytes_written, "cancelled");
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
        if last_progress.elapsed() >= Duration::from_millis(250) {
            emit_progress(bytes_written, "downloading");
            last_progress = Instant::now();
        }
    }
    writer
        .flush()
        .map_err(|error| format!("failed to flush {}: {error}", partial.display()))?;
    drop(writer);
    emit_progress(bytes_written, "validating");
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
    emit_progress(bytes_written, "completed");
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
    store: State<'_, Arc<DesktopStore>>,
    selection: ModelSelection,
    revision: String,
    expected_bytes: u64,
    expected_sha256: Option<String>,
) -> Result<DownloadReceipt, String> {
    let key = format!("{}\n{}", selection.repo_id, selection.filename);
    let provenance = selection.clone();
    let provenance_revision = revision.clone();
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
    let store = Arc::clone(store.inner());
    let result = tauri::async_runtime::spawn_blocking(move || {
        let receipt = download_model_blocking(
            app,
            selection,
            revision,
            expected_bytes,
            expected_sha256,
            cancelled,
        )?;
        let digest = file_sha256(Path::new(&receipt.path))?;
        store
            .register_model_artifact(RegisterModelArtifact {
                source: ModelSource::HuggingFace,
                repo_id: Some(provenance.repo_id.clone()),
                revision: Some(provenance_revision.clone()),
                filename: provenance.filename.clone(),
                local_path: receipt.path.clone(),
                observed_bytes: receipt.bytes_written,
                sha256: digest,
                origin_url: Some(catalog::download_url(
                    &provenance.repo_id,
                    &provenance_revision,
                    &provenance.filename,
                )),
            })
            .map_err(|error| format!("downloaded model could not be registered: {error}"))?;
        Ok(receipt)
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
fn list_downloads(
    app: AppHandle,
    store: State<'_, Arc<DesktopStore>>,
) -> Result<Vec<DownloadedModel>, String> {
    let registered = store
        .list_model_artifacts()
        .map_err(|error| error.to_string())?;
    let mut models = registered
        .iter()
        .filter(|model| Path::new(&model.local_path).is_file())
        .map(|model| DownloadedModel {
            registry_id: Some(model.id.clone()),
            filename: model.filename.clone(),
            size_bytes: model.observed_bytes,
            state: "installed",
            source: Some(match model.source {
                ModelSource::HuggingFace => "hugging-face".to_owned(),
                ModelSource::Import => "import".to_owned(),
            }),
            repo_id: model.repo_id.clone(),
            revision: model.revision.clone(),
            sha256: Some(model.sha256.clone()),
            local_path: model.local_path.clone(),
        })
        .collect::<Vec<_>>();
    let registered_paths = registered
        .iter()
        .map(|entry| PathBuf::from(&entry.local_path))
        .collect::<Vec<_>>();
    let model_root = PathBuf::from(read_settings(&app)?.install_root).join("models");
    let Ok(entries) = std::fs::read_dir(&model_root) else {
        return Ok(models);
    };
    models.extend(entries.filter_map(Result::ok).filter_map(|entry| {
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
        let local_path = entry.path();
        if registered_paths
            .iter()
            .any(|registered| registered == &local_path)
        {
            return None;
        }
        Some(DownloadedModel {
            registry_id: None,
            filename,
            size_bytes: metadata.len(),
            state,
            source: None,
            repo_id: None,
            revision: None,
            sha256: None,
            local_path: local_path.to_string_lossy().into_owned(),
        })
    }));
    models.sort_by(|left, right| left.filename.cmp(&right.filename));
    Ok(models)
}

fn import_model_blocking(
    app: AppHandle,
    store: Arc<DesktopStore>,
    source_path: String,
) -> Result<ModelRegistryEntry, String> {
    let source = PathBuf::from(source_path.trim());
    if !source.is_absolute() {
        return Err("the imported GGUF path must be absolute".to_owned());
    }
    let source = source.canonicalize().map_err(|error| {
        format!(
            "failed to resolve imported GGUF {}: {error}",
            source.display()
        )
    })?;
    if !source.is_file() {
        return Err("the imported GGUF path must identify a file".to_owned());
    }
    let filename = source
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "the imported GGUF filename is invalid".to_owned())?;
    let filename = catalog::validated_artifact_filename(filename)?.to_owned();
    let settings = read_settings(&app)?;
    let model_root = PathBuf::from(settings.install_root).join("models");
    std::fs::create_dir_all(&model_root)
        .map_err(|error| format!("failed to create {}: {error}", model_root.display()))?;
    let destination = model_root.join(&filename);
    let same_file = destination
        .canonicalize()
        .ok()
        .is_some_and(|path| path == source);
    if !same_file {
        if destination.exists() {
            let source_hash = file_sha256(&source)?;
            let destination_hash = file_sha256(&destination)?;
            if source_hash != destination_hash {
                return Err(format!(
                    "{} already exists with different content; Alpine did not overwrite it",
                    destination.display()
                ));
            }
        } else {
            let partial = model_root.join(format!("{filename}.import.part"));
            std::fs::copy(&source, &partial).map_err(|error| {
                format!(
                    "failed to copy imported GGUF from {} to {}: {error}",
                    source.display(),
                    partial.display()
                )
            })?;
            std::fs::rename(&partial, &destination).map_err(|error| {
                format!(
                    "failed to publish imported GGUF {} as {}: {error}",
                    partial.display(),
                    destination.display()
                )
            })?;
        }
    }
    let observed_bytes = destination
        .metadata()
        .map_err(|error| format!("failed to inspect {}: {error}", destination.display()))?
        .len();
    let sha256 = file_sha256(&destination)?;
    store
        .register_model_artifact(RegisterModelArtifact {
            source: ModelSource::Import,
            repo_id: None,
            revision: None,
            filename,
            local_path: destination.to_string_lossy().into_owned(),
            observed_bytes,
            sha256,
            origin_url: None,
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn import_model(
    app: AppHandle,
    store: State<'_, Arc<DesktopStore>>,
    source_path: String,
) -> Result<ModelRegistryEntry, String> {
    let store = Arc::clone(store.inner());
    tauri::async_runtime::spawn_blocking(move || import_model_blocking(app, store, source_path))
        .await
        .map_err(|error| format!("model import worker failed: {error}"))?
}

#[tauri::command]
fn list_model_registry(
    store: State<'_, Arc<DesktopStore>>,
) -> Result<Vec<ModelRegistryEntry>, String> {
    store
        .list_model_artifacts()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_projects(store: State<'_, Arc<DesktopStore>>) -> Result<Vec<DesktopProject>, String> {
    store.list_projects().map_err(|error| error.to_string())
}

#[tauri::command]
fn create_project(
    store: State<'_, Arc<DesktopStore>>,
    name: String,
    root: String,
) -> Result<DesktopProject, String> {
    store
        .create_project(&name, root)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_tasks(
    store: State<'_, Arc<DesktopStore>>,
    project_id: String,
) -> Result<Vec<DesktopTask>, String> {
    store
        .list_tasks(&project_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn create_task(
    store: State<'_, Arc<DesktopStore>>,
    input: CreateTask,
) -> Result<DesktopTask, String> {
    store.create_task(input).map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_task(store: State<'_, Arc<DesktopStore>>, task_id: String) -> Result<(), String> {
    store
        .delete_task(&task_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn load_task(
    store: State<'_, Arc<DesktopStore>>,
    task_id: String,
) -> Result<Option<TaskDetail>, String> {
    store.load_task(&task_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn list_pending_approvals(
    store: State<'_, Arc<DesktopStore>>,
    task_id: String,
) -> Result<Vec<ToolApproval>, String> {
    store
        .list_pending_approvals(&task_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn list_project_files(
    store: State<'_, Arc<DesktopStore>>,
    task_id: String,
    limit: usize,
) -> Result<Vec<WorkspaceEntry>, String> {
    workspace::list_project_files(&store, &task_id, limit).map_err(|error| error.to_string())
}

#[tauri::command]
fn read_project_file(
    store: State<'_, Arc<DesktopStore>>,
    task_id: String,
    path: String,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<WorkspaceRead, String> {
    workspace::read_project_file(&store, &task_id, &path, offset, limit)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn search_project_files(
    store: State<'_, Arc<DesktopStore>>,
    task_id: String,
    query: String,
    limit: usize,
) -> Result<Vec<WorkspaceSearchMatch>, String> {
    workspace::search_project_files(&store, &task_id, &query, limit)
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(DownloadRegistry::default())
        .manage(Arc::new(TaskSupervisor::default()))
        .invoke_handler(tauri::generate_handler![
            bootstrap_snapshot,
            update_settings,
            search_models,
            assess_model,
            plan_model_placement,
            set_default_model,
            start_runtime,
            stop_runtime,
            run_runtime_probe,
            run_full_evaluation,
            download_model,
            cancel_download,
            list_downloads,
            import_model,
            list_model_registry,
            list_projects,
            create_project,
            list_tasks,
            create_task,
            delete_task,
            load_task,
            supervisor::connect_agent_worker,
            supervisor::subscribe_execution_updates,
            supervisor::submit_prompt,
            supervisor::cancel_execution,
            supervisor::steer_execution,
            supervisor::queue_follow_up,
            supervisor::decide_tool_approval,
            supervisor::agent_request_tool_approval,
            supervisor::agent_execute_edit,
            supervisor::agent_run_shell,
            supervisor::agent_worker_event,
            list_pending_approvals,
            list_project_files,
            read_project_file,
            search_project_files,
            browser::browser_navigate,
            browser::browser_sync_surface,
            browser::browser_command,
            browser::browser_clear_data,
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            app.manage(Arc::new(DesktopStore::open(
                data_dir.join("desktop.sqlite3"),
            )?));
            #[cfg(desktop)]
            tauri::WebviewWindowBuilder::new(
                app,
                "agent-worker",
                tauri::WebviewUrl::App("agent.html".into()),
            )
            .title("Alpine Agent Worker")
            .visible(false)
            .skip_taskbar(true)
            .resizable(false)
            .inner_size(1.0, 1.0)
            .build()?;
            let browser_hosts = read_settings(app.handle())
                .map(|settings| settings.browser_allowed_hosts)
                .unwrap_or_default();
            app.manage(BrowserRegistry::new(browser_hosts));
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
    use super::{
        ModelRegistryEntry, ModelSelection, ModelSource, decode_probe_response,
        registered_default_selection,
    };

    #[test]
    fn runtime_probe_requires_exact_visible_output() {
        let passing =
            r#"{"choices":[{"message":{"content":"ALPINE_OK"}}],"usage":{"completion_tokens":4}}"#;
        let reasoning = r#"{"choices":[{"message":{"content":"Reasoning... ALPINE_OK"}}],"usage":{"completion_tokens":9}}"#;

        assert_eq!(decode_probe_response(passing).unwrap(), (true, Some(4)));
        assert_eq!(decode_probe_response(reasoning).unwrap(), (false, Some(9)));
    }

    fn registered_hugging_face_model() -> ModelRegistryEntry {
        ModelRegistryEntry {
            id: "model-1".to_owned(),
            source: ModelSource::HuggingFace,
            repo_id: Some("Qwen/Qwen-GGUF".to_owned()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            filename: "Qwen-Q4_K_M.gguf".to_owned(),
            local_path: "C:\\models\\Qwen-Q4_K_M.gguf".to_owned(),
            observed_bytes: 42,
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            origin_url: None,
            created_at_ms: 1,
            verified_at_ms: 1,
        }
    }

    #[test]
    fn default_model_requires_exact_registered_identity() {
        let artifact = registered_hugging_face_model();
        let exact = ModelSelection {
            repo_id: artifact.repo_id.clone().unwrap(),
            filename: artifact.filename.clone(),
            registry_id: Some(artifact.id.clone()),
            revision: artifact.revision.clone(),
            sha256: Some(artifact.sha256.clone()),
        };

        assert_eq!(
            registered_default_selection(exact.clone(), std::slice::from_ref(&artifact))
                .unwrap()
                .registry_id
                .as_deref(),
            Some("model-1")
        );

        let missing_registry = ModelSelection {
            registry_id: None,
            ..exact.clone()
        };
        assert!(
            registered_default_selection(missing_registry, std::slice::from_ref(&artifact))
                .unwrap_err()
                .contains("verified Model Registry entry")
        );

        let mutable_revision = ModelSelection {
            revision: Some("main".to_owned()),
            ..exact
        };
        assert!(
            registered_default_selection(mutable_revision, &[artifact])
                .unwrap_err()
                .contains("repository or revision")
        );
    }

    #[test]
    fn imported_default_uses_its_full_digest_identity() {
        let mut artifact = registered_hugging_face_model();
        artifact.id = "import-1".to_owned();
        artifact.source = ModelSource::Import;
        artifact.repo_id = None;
        artifact.revision = None;
        let selection = ModelSelection {
            repo_id: format!("local/import/{}", artifact.sha256),
            filename: artifact.filename.clone(),
            registry_id: Some(artifact.id.clone()),
            revision: None,
            sha256: Some(artifact.sha256.clone()),
        };

        assert!(registered_default_selection(selection, &[artifact]).is_ok());
    }
}
