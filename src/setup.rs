use crate::clock::UtcTimestamp;
use crate::deployment::{self, BootstrapDeploymentOptions};
use crate::identity::{sha256_file, tree_sha256};
use crate::locking::InterprocessLock;
use crate::process::{resolve_executable, run_command_bounded};
use crate::tooling::{self, BuildLauncherOptions, PackageRuntimeOptions};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

const PINNED_OPENCODE: &str = "1.18.18";
const SETUP_SCHEMA: u32 = 1;
const SESSION_SCHEMA: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupRuntime {
    Custom,
    Official,
}

#[derive(Debug, Clone)]
pub struct SetupOptions {
    pub repository_root: PathBuf,
    pub install_root: PathBuf,
    pub profile: String,
    pub runtime: SetupRuntime,
    pub reuse_artifacts_from: Option<PathBuf>,
    pub install_prerequisites: bool,
    pub skip_vision: bool,
    pub verify_only: bool,
    pub no_shortcut: bool,
    pub lock_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupReport {
    pub install_root: PathBuf,
    pub profile: String,
    pub runtime: SetupRuntime,
    pub verified: bool,
    pub verify_only: bool,
    pub recovered_interrupted_publication: bool,
    pub deployment_initialized: bool,
    pub shortcuts_updated: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ArtifactManifest {
    model: Artifact,
    mmproj: Artifact,
    chat_template: Artifact,
    llama_cpp: LlamaCpp,
}

#[derive(Debug, Clone, Deserialize)]
struct Artifact {
    filename: String,
    relative_path: PathBuf,
    url: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct LlamaCpp {
    repo: String,
    commit: String,
    patch: PathBuf,
    official_runtime: Download,
    official_cuda: Download,
    custom_build: CustomBuild,
}

#[derive(Debug, Clone, Deserialize)]
struct Download {
    filename: String,
    url: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct CustomBuild {
    cuda: String,
    cmake: String,
    generator: String,
    architecture: String,
    options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublicationItem {
    stage: PathBuf,
    destination: PathBuf,
    had_prior: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct PublicationMarker {
    schema: u32,
    transaction_id: String,
    started_at: String,
    stage_root: PathBuf,
    backup_root: PathBuf,
    items: Vec<PublicationItem>,
}

#[derive(Debug, Clone)]
struct RequestedPublicationItem {
    stage: PathBuf,
    destination: PathBuf,
}

struct StageGuard {
    install_root: PathBuf,
    stage: PathBuf,
    armed: bool,
}

impl StageGuard {
    fn new(install_root: &Path, stage: &Path) -> Self {
        Self {
            install_root: install_root.to_owned(),
            stage: stage.to_owned(),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_contained(&self.install_root, &self.stage);
        }
    }
}

#[derive(Debug, Serialize)]
struct ControlPlaneIdentity {
    schema: u32,
    source_commit: Option<String>,
    source_dirty: Option<bool>,
    source_tree_sha256: String,
    files: Vec<ControlPlaneFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ControlPlaneFile {
    path: String,
    sha256: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    generated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeFileIdentity {
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OfficialRuntimeIdentity {
    schema: u32,
    llama_cpp_commit: String,
    runtime_archive_sha256: String,
    cuda_archive_sha256: String,
    files: BTreeMap<String, RuntimeFileIdentity>,
}

#[derive(Debug, Deserialize)]
struct CustomRuntimeIdentity {
    schema: u32,
    llama_cpp_commit: String,
    source_patch_sha256: String,
    cuda_toolkit: String,
    cuda_architecture: String,
    cmake_options: Vec<String>,
    files: BTreeMap<String, RuntimeFileIdentity>,
}

pub fn run(options: &SetupOptions) -> Result<SetupReport, String> {
    validate_profile_name(&options.profile)?;
    let repository_root = canonical_directory(&options.repository_root, "repository root")?;
    let reuse_artifacts_from = options
        .reuse_artifacts_from
        .as_deref()
        .map(|path| canonical_directory(path, "artifact reuse root"))
        .transpose()?;
    let profile_path = repository_root
        .join("config/profiles")
        .join(format!("{}.json", options.profile));
    let selected_profile: crate::config::Profile = read_json(&profile_path, "Profile")?;
    crate::config::validate_profile(&selected_profile, &options.profile, &profile_path)?;
    if selected_profile.name != options.profile {
        return Err(format!(
            "Profile name '{}' does not match requested Profile '{}'.",
            selected_profile.name, options.profile
        ));
    }
    if selected_profile.runtime == "custom" && options.runtime == SetupRuntime::Official {
        return Err(format!(
            "{} requires the custom runtime, but setup was asked for Official only.",
            options.profile
        ));
    }
    let manifest_path = repository_root.join("config/artifacts.json");
    let manifest: ArtifactManifest = read_json(&manifest_path, "artifact contract")?;
    validate_manifest(&repository_root, &manifest)?;
    let install_root = absolute_path(&options.install_root)?;
    std::fs::create_dir_all(&install_root).map_err(|error| {
        format!(
            "failed to create installation root {}: {error}",
            install_root.display()
        )
    })?;
    let install_root = std::fs::canonicalize(&install_root)
        .map_err(|error| format!("failed to resolve installation root: {error}"))?;

    let _setup_lock =
        InterprocessLock::acquire(&install_root.join(".setup.lock"), options.lock_timeout)
            .map_err(|error| {
                format!(
                    "Another setup transaction owns {}. {error}",
                    install_root.display()
                )
            })?;
    let recovered = repair_interrupted_publication(&install_root)?;
    if options.verify_only {
        verify_install(
            &repository_root,
            &install_root,
            &options.profile,
            options.skip_vision,
            &manifest,
        )?;
        return Ok(SetupReport {
            install_root,
            profile: options.profile.clone(),
            runtime: options.runtime,
            verified: true,
            verify_only: true,
            recovered_interrupted_publication: recovered,
            deployment_initialized: false,
            shortcuts_updated: false,
        });
    }

    if options.install_prerequisites {
        install_prerequisites(&manifest)?;
    }
    for relative in ["config", "models", "logs", ".artifacts"] {
        std::fs::create_dir_all(install_root.join(relative))
            .map_err(|error| format!("failed to create {relative}: {error}"))?;
    }
    install_artifact(
        &install_root,
        reuse_artifacts_from.as_deref(),
        &manifest.model,
    )?;
    if !options.skip_vision {
        install_artifact(
            &install_root,
            reuse_artifacts_from.as_deref(),
            &manifest.mmproj,
        )?;
    }
    install_artifact(
        &install_root,
        reuse_artifacts_from.as_deref(),
        &manifest.chat_template,
    )?;

    let stage = install_root.join(format!(".control-plane-stage-{}", Uuid::new_v4().simple()));
    std::fs::create_dir(&stage)
        .map_err(|error| format!("failed to create setup stage: {error}"))?;
    let mut stage_guard = StageGuard::new(&install_root, &stage);
    let staged = (|| {
        let official_server = install_official_runtime(
            &repository_root,
            &install_root,
            &stage,
            reuse_artifacts_from.as_deref(),
            &manifest,
        )?;
        let custom_server = if options.runtime == SetupRuntime::Custom {
            Some(install_custom_runtime(
                &repository_root,
                &install_root,
                &stage,
                reuse_artifacts_from.as_deref(),
                &manifest,
            )?)
        } else {
            None
        };
        copy_control_plane(&repository_root, &stage, &manifest_path)?;
        build_alpine_control_plane(&repository_root, &stage)?;
        write_session_config(
            &install_root,
            &stage,
            &manifest,
            &official_server,
            custom_server.as_deref(),
        )?;
        tooling::build_launcher(&BuildLauncherOptions {
            root: stage.clone(),
            output: Some(stage.join("Open Local Qwen.exe")),
            no_shortcut: true,
            shortcut_only: false,
        })?;
        write_control_plane_identity(&repository_root, &stage, &manifest_path)?;
        Ok::<(), String>(())
    })();
    if let Err(error) = staged {
        remove_contained(&install_root, &stage)?;
        return Err(error);
    }

    let _deployment_lock =
        InterprocessLock::acquire(&install_root.join(".deployment.lock"), options.lock_timeout)
            .map_err(|error| format!("could not lock deployment publication: {error}"))?;
    let initialize_deployment = !has_deployment_events(&install_root)?;
    if initialize_deployment {
        deployment::bootstrap(&BootstrapDeploymentOptions {
            install_root: stage.clone(),
            daily_default: "stable-16k".to_owned(),
            rollback_profile: "stable-16k".to_owned(),
            operator: "setup".to_owned(),
            reason: "Initialize conservative deployment roles; qualification is not inherited."
                .to_owned(),
            lock_timeout: options.lock_timeout,
        })?;
    }
    let items = publication_items(&stage, initialize_deployment)?;
    publish_bundle(&install_root, &stage, &items)?;
    stage_guard.disarm();

    let shortcuts_updated = if options.no_shortcut {
        false
    } else {
        tooling::build_launcher(&BuildLauncherOptions {
            root: install_root.clone(),
            output: Some(install_root.join("Open Local Qwen.exe")),
            no_shortcut: false,
            shortcut_only: true,
        })
        .map_err(|error| committed_postcheck_failure("shortcut update", &error))?;
        true
    };
    verify_install(
        &repository_root,
        &install_root,
        &options.profile,
        options.skip_vision,
        &manifest,
    )
    .map_err(|error| committed_postcheck_failure("final verification", &error))?;
    Ok(SetupReport {
        install_root,
        profile: options.profile.clone(),
        runtime: options.runtime,
        verified: true,
        verify_only: false,
        recovered_interrupted_publication: recovered,
        deployment_initialized: initialize_deployment,
        shortcuts_updated,
    })
}

fn committed_postcheck_failure(label: &str, error: &str) -> String {
    format!(
        "Setup publication committed, but {label} failed: {error} The new generation remains installed and was not reported as rolled back."
    )
}

fn publication_items(
    stage: &Path,
    include_deployment: bool,
) -> Result<Vec<RequestedPublicationItem>, String> {
    let mut items = Vec::new();
    for relative in ["runtime-official", "runtime-custom"] {
        if stage.join(relative).exists() {
            items.push(requested(relative, relative)?);
        }
    }
    if include_deployment {
        items.push(requested("deployment", "deployment")?);
    }
    for relative in ["launcher", "profiles"] {
        items.push(requested(relative, relative)?);
    }
    for relative in [
        "config/artifacts.json",
        "config/control-plane.json",
        "config/profile-capabilities.json",
        "config/session.json",
        "alpine.exe",
        "Open Minimal OpenCode.cmd",
        "Open Local Qwen.exe",
    ] {
        items.push(requested(relative, relative)?);
    }
    Ok(items)
}

fn requested(stage: &str, destination: &str) -> Result<RequestedPublicationItem, String> {
    let stage = validate_relative_path(Path::new(stage), "stage item")?;
    let destination = validate_relative_path(Path::new(destination), "publication destination")?;
    Ok(RequestedPublicationItem { stage, destination })
}

fn publish_bundle(
    install_root: &Path,
    stage_root: &Path,
    items: &[RequestedPublicationItem],
) -> Result<(), String> {
    let install_root = canonical_directory(install_root, "installation root")?;
    let stage_root = contained_existing(&install_root, stage_root, "setup stage")?;
    if stage_root == install_root {
        return Err("setup stage cannot be the installation root".to_owned());
    }
    let marker_path = install_root.join(".setup-publishing.json");
    if marker_path.exists() {
        return Err(format!(
            "An incomplete setup publication exists: {}. Recover it before publishing.",
            marker_path.display()
        ));
    }
    let backup_root = install_root.join(format!(".setup-backup-{}", Uuid::new_v4().simple()));
    contained_target(&install_root, &backup_root, "setup backup")?;
    let mut journal = Vec::new();
    for requested in items {
        let stage_relative = validate_relative_path(&requested.stage, "stage item")?;
        let destination_relative =
            validate_relative_path(&requested.destination, "publication destination")?;
        let source = stage_root.join(&stage_relative);
        contained_existing(&stage_root, &source, "staged publication item")?;
        let destination = install_root.join(&destination_relative);
        contained_target(&install_root, &destination, "publication destination")?;
        journal.push(PublicationItem {
            stage: stage_relative,
            destination: destination_relative,
            had_prior: destination.exists(),
        });
    }
    let marker = PublicationMarker {
        schema: SETUP_SCHEMA,
        transaction_id: Uuid::new_v4().simple().to_string(),
        started_at: UtcTimestamp::now()?.rfc3339(),
        stage_root: stage_root.clone(),
        backup_root: backup_root.clone(),
        items: journal,
    };
    validate_publication_journal(&marker.items)?;
    write_json_atomic(&marker_path, &marker)?;

    let publication = (|| {
        for item in &marker.items {
            let source = contained_existing(
                &stage_root,
                &stage_root.join(&item.stage),
                "staged publication item",
            )?;
            let destination = install_root.join(&item.destination);
            contained_target(&install_root, &destination, "publication destination")?;
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    format!("failed to create publication destination: {error}")
                })?;
                contained_existing(&install_root, parent, "publication parent")?;
            }
            if item.had_prior {
                let backup = backup_root.join(&item.destination);
                contained_target(&install_root, &backup, "publication backup")?;
                if let Some(parent) = backup.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| format!("failed to create setup backup: {error}"))?;
                    contained_existing(&install_root, parent, "setup backup parent")?;
                }
                std::fs::rename(&destination, &backup).map_err(|error| {
                    format!(
                        "failed to preserve prior publication {}: {error}",
                        destination.display()
                    )
                })?;
            }
            std::fs::rename(&source, &destination).map_err(|error| {
                format!(
                    "failed to publish {} to {}: {error}",
                    source.display(),
                    destination.display()
                )
            })?;
        }
        Ok::<(), String>(())
    })();
    if let Err(failure) = publication {
        return match repair_interrupted_publication(&install_root) {
            Ok(_) => Err(failure),
            Err(rollback) => Err(format!(
                "Setup publication failed: {failure} Automatic rollback also failed: {rollback}"
            )),
        };
    }

    // Marker removal is the commit point. Cleanup after this point is best-effort
    // and cannot truthfully be reported as a rolled-back publication failure.
    std::fs::remove_file(&marker_path)
        .map_err(|error| format!("failed to commit setup publication: {error}"))?;
    let _ = remove_contained(&install_root, &backup_root);
    let _ = remove_contained(&install_root, &stage_root);
    Ok(())
}

fn repair_interrupted_publication(install_root: &Path) -> Result<bool, String> {
    let install_root = canonical_directory(install_root, "installation root")?;
    let marker_path = install_root.join(".setup-publishing.json");
    if !marker_path.is_file() {
        return Ok(false);
    }
    let marker: PublicationMarker = read_json(&marker_path, "setup publication marker")?;
    if marker.schema != SETUP_SCHEMA {
        return Err(format!(
            "Setup publication marker has unsupported schema {}: {}. Preserve the installation and repair it manually.",
            marker.schema,
            marker_path.display()
        ));
    }
    let stage_root = contained_marker_root(&install_root, &marker.stage_root, "setup stage")?;
    let backup_root = contained_marker_root(&install_root, &marker.backup_root, "setup backup")?;
    validate_publication_journal(&marker.items)?;
    for item in marker.items.iter().rev() {
        let destination_relative =
            validate_relative_path(&item.destination, "publication destination")?;
        validate_relative_path(&item.stage, "stage item")?;
        let destination = install_root.join(&destination_relative);
        contained_target(&install_root, &destination, "publication destination")?;
        let backup = backup_root.join(&destination_relative);
        contained_target(&install_root, &backup, "publication backup")?;
        if backup.exists() {
            contained_existing(&install_root, &backup, "publication backup")?;
            if destination.exists() {
                remove_contained(&install_root, &destination)?;
            }
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to recreate publication parent: {error}"))?;
                contained_existing(&install_root, parent, "publication parent")?;
            }
            std::fs::rename(&backup, &destination)
                .map_err(|error| format!("failed to restore {}: {error}", destination.display()))?;
        } else if !item.had_prior && destination.exists() {
            remove_contained(&install_root, &destination)?;
        }
    }
    if backup_root.exists() {
        remove_contained(&install_root, &backup_root)?;
    }
    if stage_root.exists() {
        remove_contained(&install_root, &stage_root)?;
    }
    std::fs::remove_file(&marker_path)
        .map_err(|error| format!("failed to remove recovered setup marker: {error}"))?;
    Ok(true)
}

fn contained_marker_root(root: &Path, path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(format!("{label} is not a normalized absolute path"));
    }
    let prefix = if label == "setup stage" {
        ".control-plane-stage-"
    } else {
        ".setup-backup-"
    };
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} has no parent"))?;
    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|error| format!("failed to resolve {label} parent: {error}"))?;
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| format!("failed to resolve installation root: {error}"))?;
    if canonical_parent != canonical_root {
        return Err(format!(
            "{label} escapes the installation root: {}",
            path.display()
        ));
    }
    let name_os = path
        .file_name()
        .ok_or_else(|| format!("{label} must be a direct installation child"))?;
    let name = name_os.to_string_lossy();
    let identity = name
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{label} has an invalid transaction name"))?;
    if identity.len() != 32
        || !identity
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("{label} has an invalid transaction identity"));
    }
    let normalized = root.join(name_os);
    contained_target(root, &normalized, label)?;
    Ok(normalized)
}

fn validate_publication_journal(items: &[PublicationItem]) -> Result<(), String> {
    let allowlist: BTreeSet<&str> = [
        "runtime-official",
        "runtime-custom",
        "deployment",
        "scripts",
        "launcher",
        "profiles",
        "config/artifacts.json",
        "config/control-plane.json",
        "config/profile-capabilities.json",
        "config/session.json",
        "alpine.exe",
        "Open Minimal OpenCode.cmd",
        "Open Local Qwen.exe",
    ]
    .into_iter()
    .collect();
    let mut seen = BTreeSet::new();
    for item in items {
        let stage = validate_relative_path(&item.stage, "stage item")?;
        let destination = validate_relative_path(&item.destination, "publication destination")?;
        let stage = stage.to_string_lossy().replace('\\', "/");
        let destination = destination.to_string_lossy().replace('\\', "/");
        if stage != destination || !allowlist.contains(destination.as_str()) {
            return Err(format!(
                "setup publication journal contains a non-allowlisted mapping: {stage} -> {destination}"
            ));
        }
        if !seen.insert(destination) {
            return Err("setup publication journal contains a duplicate destination".to_owned());
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!("{label} must be a non-empty relative path"));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            _ => {
                return Err(format!(
                    "{label} contains a forbidden path component: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(normalized)
}

fn contained_existing(root: &Path, path: &Path, label: &str) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("failed to resolve containment root: {error}"))?;
    let resolved = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {label} {}: {error}", path.display()))?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "{label} resolves outside the installation boundary: {}",
            path.display()
        ));
    }
    Ok(resolved)
}

fn contained_target(root: &Path, path: &Path, label: &str) -> Result<(), String> {
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("failed to resolve containment root: {error}"))?;
    let mut existing = path;
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| format!("{label} has no existing ancestor: {}", path.display()))?;
    }
    let resolved = std::fs::canonicalize(existing).map_err(|error| {
        format!(
            "failed to resolve existing {label} ancestor {}: {error}",
            existing.display()
        )
    })?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "{label} crosses a symlink or reparse boundary outside the installation: {}",
            path.display()
        ));
    }
    Ok(())
}

fn remove_contained(root: &Path, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("failed to resolve removal root: {error}"))?;
    let resolved = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve removal target: {error}"))?;
    if resolved == root || !resolved.starts_with(&root) {
        return Err(format!(
            "refusing to remove path outside the installation boundary: {}",
            path.display()
        ));
    }
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect removal target: {error}"))?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display()))
    } else {
        std::fs::remove_file(path)
            .map_err(|error| format!("failed to remove {}: {error}", path.display()))
    }
}

fn validate_manifest(repository_root: &Path, manifest: &ArtifactManifest) -> Result<(), String> {
    let mut artifact_paths = BTreeSet::new();
    for artifact in [&manifest.model, &manifest.mmproj, &manifest.chat_template] {
        let relative = validate_relative_path(&artifact.relative_path, "artifact relative_path")?;
        validate_filename(&artifact.filename, "artifact filename")?;
        if relative.file_name() != Some(OsStr::new(&artifact.filename)) {
            return Err(
                "artifact filename must match the final relative_path component".to_owned(),
            );
        }
        let normalized = relative.to_string_lossy().replace('\\', "/");
        if !artifact_paths.insert(normalized.clone()) {
            return Err(format!(
                "artifact relative_path is duplicated: {normalized}"
            ));
        }
        if reserved_setup_path(&normalized) {
            return Err(format!(
                "artifact relative_path collides with a reserved setup destination: {normalized}"
            ));
        }
        validate_sha256(&artifact.sha256, "artifact SHA-256")?;
        if artifact.bytes == 0
            || artifact.url.trim().is_empty()
            || artifact.filename.trim().is_empty()
        {
            return Err("artifact contract contains an empty required value".to_owned());
        }
    }
    for download in [
        &manifest.llama_cpp.official_runtime,
        &manifest.llama_cpp.official_cuda,
    ] {
        validate_filename(&download.filename, "runtime filename")?;
        validate_sha256(&download.sha256, "runtime SHA-256")?;
        if download.bytes == 0
            || download.url.trim().is_empty()
            || download.filename.trim().is_empty()
        {
            return Err("runtime artifact contract contains an empty required value".to_owned());
        }
    }
    validate_relative_path(&manifest.llama_cpp.patch, "llama.cpp patch path")?;
    if !repository_root.join(&manifest.llama_cpp.patch).is_file() {
        return Err(format!(
            "pinned llama.cpp patch is missing: {}",
            repository_root.join(&manifest.llama_cpp.patch).display()
        ));
    }
    if manifest.llama_cpp.commit.len() != 40
        || !manifest
            .llama_cpp
            .commit
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("llama.cpp commit must be a full 40-character hexadecimal identity".to_owned());
    }
    Ok(())
}

fn reserved_setup_path(path: &str) -> bool {
    matches!(
        path,
        "config/artifacts.json"
            | "config/control-plane.json"
            | "config/profile-capabilities.json"
            | "config/session.json"
            | "alpine.exe"
            | "Open Minimal OpenCode.cmd"
            | "Open Local Qwen.exe"
    ) || [
        "runtime-official/",
        "runtime-custom/",
        "deployment/",
        "scripts/",
        "launcher/",
        "profiles/",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("{label} must be a 64-character hexadecimal digest"))
    }
}

fn validate_filename(value: &str, label: &str) -> Result<(), String> {
    let path = Path::new(value);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) if name != "." && name != ".." => Ok(()),
        _ => Err(format!("{label} must be exactly one normal path component")),
    }
}

fn validate_profile_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains("..")
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        Err(format!("invalid Profile name: {name}"))
    } else {
        Ok(())
    }
}

fn install_artifact(
    install_root: &Path,
    reuse_root: Option<&Path>,
    artifact: &Artifact,
) -> Result<PathBuf, String> {
    let relative = validate_relative_path(&artifact.relative_path, "artifact relative_path")?;
    let destination = install_root.join(relative);
    contained_target(install_root, &destination, "artifact destination")?;
    if destination.is_file() {
        match assert_artifact(&destination, artifact.bytes, &artifact.sha256) {
            Ok(()) => return Ok(destination),
            Err(_) => {
                quarantine_invalid(&destination)?;
            }
        }
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create artifact directory: {error}"))?;
        contained_existing(install_root, parent, "artifact directory")?;
    }
    if let Some(reuse_root) = reuse_root {
        let source = reuse_root.join(&artifact.relative_path);
        if source.is_file() {
            contained_existing(reuse_root, &source, "reused artifact")?;
            assert_artifact(&source, artifact.bytes, &artifact.sha256)?;
            if std::fs::hard_link(&source, &destination).is_err() {
                copy_atomic(&source, &destination)?;
            }
            if let Err(error) = assert_artifact(&destination, artifact.bytes, &artifact.sha256) {
                quarantine_invalid(&destination)?;
                return Err(format!(
                    "reused artifact failed verification after publication: {error}"
                ));
            }
            return Ok(destination);
        }
    }
    let partial = append_suffix(&destination, ".part");
    if publish_completed_partial(&partial, &destination, artifact.bytes, &artifact.sha256)? {
        return Ok(destination);
    }
    let curl = resolve_executable("curl.exe")
        .or_else(|| resolve_executable("curl"))
        .ok_or_else(|| "curl is required to download pinned artifacts".to_owned())?;
    let mut command = Command::new(curl);
    command.args([
        OsStr::new("--fail"),
        OsStr::new("--location"),
        OsStr::new("--retry"),
        OsStr::new("6"),
        OsStr::new("--retry-all-errors"),
        OsStr::new("--continue-at"),
        OsStr::new("-"),
        OsStr::new("--output"),
        partial.as_os_str(),
        OsStr::new(&artifact.url),
    ]);
    run_inherited(&mut command, "artifact download")?;
    publish_verified_download(&partial, &destination, artifact.bytes, &artifact.sha256)?;
    Ok(destination)
}

fn publish_completed_partial(
    partial: &Path,
    destination: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<bool, String> {
    let Ok(metadata) = partial.metadata() else {
        return Ok(false);
    };
    if !metadata.is_file() || metadata.len() != expected_size {
        return Ok(false);
    }
    match publish_verified_download(partial, destination, expected_size, expected_sha256) {
        Ok(()) => Ok(true),
        Err(error) if !partial.exists() => {
            eprintln!("warning: {error}");
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn publish_verified_download(
    partial: &Path,
    destination: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    let metadata = partial
        .metadata()
        .map_err(|_| format!("Incomplete download is missing: {}", partial.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "Incomplete download is not a file: {}",
            partial.display()
        ));
    }
    if metadata.len() != expected_size {
        return Err(format!(
            "Incomplete download has size {}, expected {}. Keep it for resumable download.",
            metadata.len(),
            expected_size
        ));
    }
    let observed = sha256_file(partial)?;
    if observed != expected_sha256.to_ascii_lowercase() {
        let invalid = quarantine_invalid(partial)?;
        return Err(format!(
            "Downloaded artifact checksum mismatch. Preserved invalid file at {}; retry will start a fresh partial download.",
            invalid.display()
        ));
    }
    if destination.exists() {
        return Err(format!(
            "refusing to replace existing download destination: {}",
            destination.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create download destination: {error}"))?;
    }
    std::fs::rename(partial, destination)
        .map_err(|error| format!("failed to publish verified download: {error}"))
}

fn assert_artifact(path: &Path, expected_size: u64, expected_sha256: &str) -> Result<(), String> {
    let metadata = path
        .metadata()
        .map_err(|_| format!("Artifact missing: {}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("Artifact is not a file: {}", path.display()));
    }
    if metadata.len() != expected_size {
        return Err(format!(
            "Artifact size mismatch: {} (got {}, expected {})",
            path.display(),
            metadata.len(),
            expected_size
        ));
    }
    let digest = sha256_file(path)?;
    if digest != expected_sha256.to_ascii_lowercase() {
        return Err(format!(
            "Artifact SHA-256 mismatch: {} (got {digest})",
            path.display()
        ));
    }
    Ok(())
}

fn quarantine_invalid(path: &Path) -> Result<PathBuf, String> {
    let suffix = format!(
        ".invalid-{}-{}",
        UtcTimestamp::now()?.compact(),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let quarantine = append_suffix(path, &suffix);
    std::fs::rename(path, &quarantine).map_err(|error| {
        format!(
            "failed to preserve invalid artifact {}: {error}",
            path.display()
        )
    })?;
    Ok(quarantine)
}

fn install_official_runtime(
    _repository_root: &Path,
    install_root: &Path,
    stage_root: &Path,
    reuse_root: Option<&Path>,
    manifest: &ArtifactManifest,
) -> Result<PathBuf, String> {
    let cache = install_root.join(".artifacts");
    std::fs::create_dir_all(&cache)
        .map_err(|error| format!("failed to create runtime cache: {error}"))?;
    for download in [
        &manifest.llama_cpp.official_runtime,
        &manifest.llama_cpp.official_cuda,
    ] {
        let artifact = Artifact {
            filename: download.filename.clone(),
            relative_path: PathBuf::from(".artifacts").join(&download.filename),
            url: download.url.clone(),
            sha256: download.sha256.clone(),
            bytes: download.bytes,
        };
        if let Some(reuse_root) = reuse_root {
            let legacy = reuse_root.join("runtime").join(&download.filename);
            let target = cache.join(&download.filename);
            if !target.exists() && legacy.is_file() {
                assert_artifact(&legacy, download.bytes, &download.sha256)?;
                if std::fs::hard_link(&legacy, &target).is_err() {
                    copy_atomic(&legacy, &target)?;
                }
                assert_artifact(&target, download.bytes, &download.sha256)?;
            }
        }
        install_artifact(install_root, reuse_root, &artifact)?;
    }
    let runtime = install_root.join("runtime-official");
    let server = runtime.join("llama-server.exe");
    if runtime.join("official-manifest.json").is_file()
        && assert_official_runtime(&runtime, manifest).is_ok()
    {
        return Ok(server);
    }
    let stage = stage_root.join("runtime-official");
    std::fs::create_dir(&stage)
        .map_err(|error| format!("failed to create official runtime stage: {error}"))?;
    extract_zip_safely(
        &cache.join(&manifest.llama_cpp.official_runtime.filename),
        &stage,
    )?;
    extract_zip_safely(
        &cache.join(&manifest.llama_cpp.official_cuda.filename),
        &stage,
    )?;
    let discovered = find_file(&stage, "llama-server.exe")?
        .ok_or_else(|| "Official runtime archive did not contain llama-server.exe.".to_owned())?;
    let parent = discovered
        .parent()
        .ok_or_else(|| "official runtime server has no parent".to_owned())?;
    if parent != stage {
        for entry in std::fs::read_dir(parent)
            .map_err(|error| format!("failed to enumerate official runtime: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("failed to inspect runtime file: {error}"))?;
            if entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                copy_atomic(&entry.path(), &stage.join(entry.file_name()))?;
            }
        }
    }
    let staged_server = stage.join("llama-server.exe");
    let version = native_version(&staged_server)?;
    if !version.contains(short_commit(&manifest.llama_cpp.commit)) {
        return Err(format!("Official runtime version mismatch:\n{version}"));
    }
    write_official_runtime_identity(&stage, manifest)?;
    assert_official_runtime(&stage, manifest)?;
    Ok(server)
}

fn write_official_runtime_identity(
    runtime: &Path,
    manifest: &ArtifactManifest,
) -> Result<(), String> {
    let identity = OfficialRuntimeIdentity {
        schema: 1,
        llama_cpp_commit: manifest.llama_cpp.commit.clone(),
        runtime_archive_sha256: manifest.llama_cpp.official_runtime.sha256.clone(),
        cuda_archive_sha256: manifest.llama_cpp.official_cuda.sha256.clone(),
        files: runtime_files(runtime, &["official-manifest.json"])?,
    };
    write_json_atomic(&runtime.join("official-manifest.json"), &identity)
}

fn assert_official_runtime(runtime: &Path, manifest: &ArtifactManifest) -> Result<(), String> {
    let identity: OfficialRuntimeIdentity = read_json(
        &runtime.join("official-manifest.json"),
        "official runtime identity",
    )?;
    if identity.schema != 1
        || identity.llama_cpp_commit != manifest.llama_cpp.commit
        || identity.runtime_archive_sha256 != manifest.llama_cpp.official_runtime.sha256
        || identity.cuda_archive_sha256 != manifest.llama_cpp.official_cuda.sha256
    {
        return Err(
            "official runtime identity does not match the pinned artifact contract".to_owned(),
        );
    }
    validate_runtime_files(runtime, &identity.files, &["official-manifest.json"])?;
    let version = native_version(&runtime.join("llama-server.exe"))?;
    if !version.contains(short_commit(&manifest.llama_cpp.commit)) {
        return Err(format!("Official runtime version mismatch:\n{version}"));
    }
    Ok(())
}

fn extract_zip_safely(archive: &Path, destination: &Path) -> Result<(), String> {
    let file = File::open(archive).map_err(|error| {
        format!(
            "failed to open runtime archive {}: {error}",
            archive.display()
        )
    })?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| format!("invalid runtime ZIP {}: {error}", archive.display()))?;
    let destination = canonical_directory(destination, "runtime extraction directory")?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| format!("failed to read ZIP entry {index}: {error}"))?;
        if entry.is_symlink() {
            return Err(format!(
                "runtime ZIP contains a forbidden symlink: {}",
                entry.name()
            ));
        }
        let relative = entry.enclosed_name().ok_or_else(|| {
            format!(
                "runtime ZIP entry escapes extraction root: {}",
                entry.name()
            )
        })?;
        let relative = validate_relative_path(&relative, "runtime ZIP entry")?;
        let output = destination.join(relative);
        contained_target(&destination, &output, "runtime ZIP output")?;
        if entry.is_dir() {
            std::fs::create_dir_all(&output)
                .map_err(|error| format!("failed to create ZIP directory: {error}"))?;
        } else {
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create ZIP parent: {error}"))?;
                contained_existing(&destination, parent, "runtime ZIP parent")?;
            }
            let mut target = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&output)
                .map_err(|error| {
                    format!("failed to create ZIP output {}: {error}", output.display())
                })?;
            std::io::copy(&mut entry, &mut target)
                .map_err(|error| format!("failed to extract {}: {error}", output.display()))?;
            target
                .flush()
                .map_err(|error| format!("failed to flush ZIP output: {error}"))?;
        }
    }
    Ok(())
}

fn install_custom_runtime(
    repository_root: &Path,
    install_root: &Path,
    stage_root: &Path,
    reuse_root: Option<&Path>,
    manifest: &ArtifactManifest,
) -> Result<PathBuf, String> {
    let runtime = install_root.join("runtime-custom");
    if runtime.join("build-manifest.json").is_file() {
        assert_custom_runtime(repository_root, &runtime, manifest)?;
        return Ok(runtime.join("llama-server.exe"));
    }
    if let Some(reuse_root) = reuse_root {
        let reusable = reuse_root.join("runtime-custom");
        if reusable.join("build-manifest.json").is_file() {
            assert_custom_runtime(repository_root, &reusable, manifest)?;
            let stage = stage_root.join("runtime-custom");
            std::fs::create_dir(&stage)
                .map_err(|error| format!("failed to create custom runtime stage: {error}"))?;
            for entry in std::fs::read_dir(&reusable)
                .map_err(|error| format!("failed to enumerate reusable runtime: {error}"))?
            {
                let entry = entry.map_err(|error| format!("failed to inspect runtime: {error}"))?;
                if entry
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_file()
                {
                    let target = stage.join(entry.file_name());
                    if std::fs::hard_link(entry.path(), &target).is_err() {
                        copy_atomic(&entry.path(), &target)?;
                    }
                }
            }
            assert_custom_runtime(repository_root, &stage, manifest)?;
            return Ok(runtime.join("llama-server.exe"));
        }
    }
    assert_custom_build_tools(manifest)?;
    let source = install_root.join("build/llama.cpp-b10453-ngram-reset");
    let build = source.join("build-sm120");
    prepare_custom_source(repository_root, &source, manifest)?;
    let cmake = resolve_executable("cmake")
        .ok_or_else(|| "cmake is required for the custom runtime".to_owned())?;
    let mut configure = Command::new(&cmake);
    configure
        .arg("-S")
        .arg(&source)
        .arg("-B")
        .arg(&build)
        .arg("-G")
        .arg(&manifest.llama_cpp.custom_build.generator)
        .args(["-A", "x64"]);
    for option in &manifest.llama_cpp.custom_build.options {
        configure.arg(format!("-D{option}"));
    }
    configure.arg(format!(
        "-DCMAKE_CUDA_ARCHITECTURES={}",
        manifest.llama_cpp.custom_build.architecture
    ));
    run_inherited(&mut configure, "custom runtime CMake configuration")?;
    let mut build_command = Command::new(cmake);
    build_command.arg("--build").arg(&build).args([
        "--config",
        "Release",
        "--target",
        "llama-server",
        "--parallel",
        "16",
    ]);
    run_inherited(&mut build_command, "custom runtime build")?;
    let built = build.join("bin/Release");
    let stage = stage_root.join("runtime-custom");
    tooling::package_runtime(&PackageRuntimeOptions {
        repository_root: repository_root.to_owned(),
        built_runtime: built,
        output: stage.clone(),
        cuda_bin: cuda_root(&manifest.llama_cpp.custom_build.cuda).join("bin/x64"),
    })?;
    assert_custom_runtime(repository_root, &stage, manifest)?;
    Ok(runtime.join("llama-server.exe"))
}

fn assert_custom_build_tools(manifest: &ArtifactManifest) -> Result<(), String> {
    for command in ["git", "cmake"] {
        if resolve_executable(command).is_none() {
            return Err(format!(
                "{command} is required for the custom runtime. Re-run with --install-prerequisites."
            ));
        }
    }
    let nvcc = cuda_root(&manifest.llama_cpp.custom_build.cuda).join("bin/nvcc.exe");
    if !nvcc.is_file() {
        return Err(format!(
            "CUDA Toolkit {} is required for the pinned custom runtime. Re-run with --install-prerequisites.",
            manifest.llama_cpp.custom_build.cuda
        ));
    }
    let vswhere = program_files_x86().join("Microsoft Visual Studio/Installer/vswhere.exe");
    if !vswhere.is_file() {
        return Err(
            "Visual Studio 2022 Build Tools with C++ are required. Re-run with --install-prerequisites."
                .to_owned(),
        );
    }
    Ok(())
}

fn prepare_custom_source(
    repository_root: &Path,
    source: &Path,
    manifest: &ArtifactManifest,
) -> Result<(), String> {
    let git = resolve_executable("git").ok_or_else(|| "git is required".to_owned())?;
    if !source.join(".git").is_dir() {
        if source.exists() {
            return Err(format!(
                "custom source path exists without a Git identity: {}",
                source.display()
            ));
        }
        if let Some(parent) = source.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create custom source parent: {error}"))?;
        }
        let mut clone = Command::new(&git);
        clone
            .args(["clone", "--no-checkout"])
            .arg(&manifest.llama_cpp.repo)
            .arg(source);
        run_inherited(&mut clone, "llama.cpp clone")?;
        let mut checkout = Command::new(&git);
        checkout
            .arg("-C")
            .arg(source)
            .args(["checkout", "--detach", &manifest.llama_cpp.commit]);
        run_inherited(&mut checkout, "pinned llama.cpp checkout")?;
        let patch = repository_root.join(&manifest.llama_cpp.patch);
        let mut check = Command::new(&git);
        check
            .arg("-C")
            .arg(source)
            .args(["apply", "--check"])
            .arg(&patch);
        run_capture_success(&mut check, "pinned patch applicability")?;
        let mut apply = Command::new(&git);
        apply.arg("-C").arg(source).arg("apply").arg(&patch);
        run_capture_success(&mut apply, "pinned patch application")?;
    }
    let head = git_text(&git, source, &["rev-parse", "HEAD"])?;
    if head.trim() != manifest.llama_cpp.commit {
        return Err(format!("Custom source commit mismatch: {}", head.trim()));
    }
    assert_exact_patch_state(repository_root, source, manifest)
}

fn assert_exact_patch_state(
    repository_root: &Path,
    source: &Path,
    manifest: &ArtifactManifest,
) -> Result<(), String> {
    let git = resolve_executable("git").ok_or_else(|| "git is required".to_owned())?;
    let patch_path = repository_root.join(&manifest.llama_cpp.patch);
    let expected = normalize_newlines(
        &std::fs::read_to_string(&patch_path)
            .map_err(|error| format!("failed to read pinned patch: {error}"))?,
    );
    let names = git_text(&git, source, &["diff", "--name-only", "HEAD", "--"])?;
    let expected_names = patch_paths(&expected)?;
    let observed_names: BTreeSet<String> = names
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.trim().replace('\\', "/"))
        .collect();
    if observed_names != expected_names {
        return Err(format!(
            "custom source has changes outside the exact pinned patch: {:?}",
            observed_names
        ));
    }
    let mut arguments = vec![
        "-c",
        "core.abbrev=7",
        "diff",
        "--no-ext-diff",
        "--binary",
        "HEAD",
        "--",
    ];
    let names: Vec<&str> = expected_names.iter().map(String::as_str).collect();
    arguments.extend(names);
    let observed = normalize_newlines(&git_text(&git, source, &arguments)?);
    if observed.trim_end() != expected.trim_end() {
        return Err("custom source diff does not exactly match the pinned patch".to_owned());
    }
    let status = git_text(
        &git,
        source,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    for line in status.lines().filter(|line| !line.trim().is_empty()) {
        if line == " M common/speculative.cpp"
            || line
                .strip_prefix("?? ")
                .is_some_and(|path| path == "build-sm120" || path.starts_with("build-sm120/"))
        {
            continue;
        }
        return Err(format!(
            "custom source contains an unexpected tracked or untracked path: {line}"
        ));
    }
    let cached = git_status(&git, source, &["diff", "--cached", "--quiet"])?;
    if !cached.success() {
        return Err(
            "custom source contains staged changes outside the pinned worktree patch".to_owned(),
        );
    }
    let mut reverse = Command::new(&git);
    reverse
        .arg("-C")
        .arg(source)
        .args(["apply", "--reverse", "--check"])
        .arg(&patch_path);
    run_capture_success(&mut reverse, "exact pinned patch reverse check")
}

fn patch_paths(patch: &str) -> Result<BTreeSet<String>, String> {
    let paths: BTreeSet<String> = patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .map(str::to_owned)
        .collect();
    if paths.is_empty() {
        Err("pinned patch contains no destination paths".to_owned())
    } else {
        Ok(paths)
    }
}

fn assert_custom_runtime(
    repository_root: &Path,
    runtime: &Path,
    manifest: &ArtifactManifest,
) -> Result<(), String> {
    let build_path = runtime.join("build-manifest.json");
    let build: CustomRuntimeIdentity = read_json(&build_path, "custom runtime manifest")?;
    validate_custom_runtime_contract(&build, manifest)?;
    let expected_patch = sha256_file(&repository_root.join(&manifest.llama_cpp.patch))?;
    if build.source_patch_sha256 != expected_patch {
        return Err("Custom runtime patch hash does not match the source tree.".to_owned());
    }
    validate_runtime_files(runtime, &build.files, &["build-manifest.json"])?;
    let server = runtime.join("llama-server.exe");
    let version = native_version(&server)?;
    if !version.contains(short_commit(&manifest.llama_cpp.commit)) {
        return Err(format!("Custom runtime version mismatch:\n{version}"));
    }
    Ok(())
}

fn validate_custom_runtime_contract(
    build: &CustomRuntimeIdentity,
    manifest: &ArtifactManifest,
) -> Result<(), String> {
    if build.schema != 1
        || build.llama_cpp_commit != manifest.llama_cpp.commit
        || build.cuda_toolkit != manifest.llama_cpp.custom_build.cuda
        || build.cuda_architecture != manifest.llama_cpp.custom_build.architecture
        || build.cmake_options != manifest.llama_cpp.custom_build.options
    {
        Err("Custom runtime build identity does not match the artifact contract.".to_owned())
    } else {
        Ok(())
    }
}

fn copy_control_plane(
    repository_root: &Path,
    stage: &Path,
    manifest_path: &Path,
) -> Result<(), String> {
    for (source, destination) in [
        (
            repository_root.join("runtime/launcher"),
            stage.join("launcher"),
        ),
        (
            repository_root.join("config/profiles"),
            stage.join("profiles"),
        ),
    ] {
        copy_flat_directory(&source, &destination)?;
    }
    std::fs::create_dir_all(stage.join("config"))
        .map_err(|error| format!("failed to create staged config directory: {error}"))?;
    copy_atomic(manifest_path, &stage.join("config/artifacts.json"))?;
    copy_atomic(
        &repository_root.join("config/profile-capabilities.json"),
        &stage.join("config/profile-capabilities.json"),
    )
}

fn build_alpine_control_plane(repository_root: &Path, stage: &Path) -> Result<(), String> {
    let cargo = resolve_cargo().ok_or_else(|| {
        "Rust Cargo is required to build the pinned Alpine control plane from this source checkout."
            .to_owned()
    })?;
    let target = alpine_build_target(stage);
    let mut command = Command::new(cargo);
    command
        .args([
            "build",
            "--locked",
            "--release",
            "--bin",
            "alpine",
            "--manifest-path",
        ])
        .arg(repository_root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target);
    run_inherited(&mut command, "Alpine release build")?;
    let binary = target.join("release/alpine.exe");
    if !binary.is_file() {
        return Err(format!(
            "Alpine release binary is missing: {}",
            binary.display()
        ));
    }
    copy_atomic(&binary, &stage.join("alpine.exe"))
}

fn alpine_build_target(stage: &Path) -> PathBuf {
    stage.join(".alpine-build-target")
}

fn write_session_config(
    install_root: &Path,
    stage: &Path,
    manifest: &ArtifactManifest,
    official_server: &Path,
    custom_server: Option<&Path>,
) -> Result<(), String> {
    let existing = install_root.join("config/session.json");
    let cleanup = if existing.is_file() {
        let value: Value = read_json(&existing, "existing Session Config")?;
        preserved_cleanup(value.get("cleanup"))?
    } else {
        disabled_cleanup()
    };
    let mut runtimes = Map::new();
    runtimes.insert(
        "official".to_owned(),
        Value::String(official_server.to_string_lossy().into_owned()),
    );
    runtimes.insert(
        "custom".to_owned(),
        custom_server
            .map(|path| Value::String(path.to_string_lossy().into_owned()))
            .unwrap_or(Value::Null),
    );
    let config = serde_json::json!({
        "schema": SESSION_SCHEMA,
        "root": install_root,
        "host": "127.0.0.1",
        "port": 8100,
        "runtimes": runtimes,
        "model": install_root.join(&manifest.model.relative_path),
        "mmproj": install_root.join(&manifest.mmproj.relative_path),
        "chat_template": install_root.join(&manifest.chat_template.relative_path),
        "api_key_file": install_root.join("config/api-key.txt"),
        "base_url_file": install_root.join("config/base-url.txt"),
        "state_file": install_root.join("logs/session-state.json"),
        "cleanup": cleanup,
    });
    std::fs::create_dir_all(stage.join("config"))
        .map_err(|error| format!("failed to create staged config directory: {error}"))?;
    write_json_atomic(&stage.join("config/session.json"), &config)
}

fn preserved_cleanup(existing: Option<&Value>) -> Result<Value, String> {
    let Some(existing) = existing else {
        return Ok(disabled_cleanup());
    };
    let object = existing
        .as_object()
        .ok_or_else(|| "cleanup configuration must be an object".to_owned())?;
    let enabled = object
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Ok(disabled_cleanup());
    }
    if object.contains_key("exe") || object.contains_key("start_script") {
        return Err("Enabled cleanup configuration uses the retired exe/start_script contract. Replace it with schema 5 executable, arguments, stdout and stderr values before setup.".to_owned());
    }
    let required = |name: &str| -> Result<String, String> {
        object
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                "Enabled cleanup configuration requires executable, arguments, stdout, stderr and health values."
                    .to_owned()
            })
    };
    let arguments = object
        .get("arguments")
        .and_then(Value::as_array)
        .filter(|arguments| !arguments.is_empty())
        .ok_or_else(|| {
            "Enabled cleanup configuration requires executable, arguments, stdout, stderr and health values."
                .to_owned()
        })?
        .iter()
        .map(|argument| {
            argument.as_str().map(str::to_owned).ok_or_else(|| {
                "Enabled cleanup configuration arguments must all be strings.".to_owned()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let port = object
        .get("port")
        .and_then(Value::as_u64)
        .filter(|port| (1..=65_535).contains(port))
        .ok_or_else(|| {
            "Enabled cleanup configuration requires a port between 1 and 65535.".to_owned()
        })?;
    Ok(serde_json::json!({
        "enabled": true,
        "port": port,
        "executable": required("executable")?,
        "arguments": arguments,
        "stdout": required("stdout")?,
        "stderr": required("stderr")?,
        "health": required("health")?,
    }))
}

fn disabled_cleanup() -> Value {
    serde_json::json!({ "enabled": false })
}

fn write_control_plane_identity(
    repository_root: &Path,
    stage: &Path,
    manifest_path: &Path,
) -> Result<(), String> {
    let mut files = Vec::new();
    for (source, destination) in [
        (
            repository_root.join("runtime/launcher"),
            ("launcher", stage.join("launcher")),
        ),
        (
            repository_root.join("config/profiles"),
            ("profiles", stage.join("profiles")),
        ),
    ] {
        for entry in sorted_files(&source)? {
            let name = entry
                .file_name()
                .ok_or_else(|| "control-plane source has no filename".to_owned())?;
            let installed = destination.1.join(name);
            let source_digest = sha256_file(&entry)?;
            if sha256_file(&installed)? != source_digest {
                return Err(format!(
                    "Copied control-plane file differs: {}/{}",
                    destination.0,
                    name.to_string_lossy()
                ));
            }
            files.push(ControlPlaneFile {
                path: format!("{}/{}", destination.0, name.to_string_lossy()),
                sha256: source_digest,
                generated: false,
            });
        }
    }
    let artifact_digest = sha256_file(manifest_path)?;
    if sha256_file(&stage.join("config/artifacts.json"))? != artifact_digest {
        return Err("Copied artifact manifest differs.".to_owned());
    }
    files.push(ControlPlaneFile {
        path: "config/artifacts.json".to_owned(),
        sha256: artifact_digest,
        generated: false,
    });
    let capability_source = repository_root.join("config/profile-capabilities.json");
    let capability_digest = sha256_file(&capability_source)?;
    if sha256_file(&stage.join("config/profile-capabilities.json"))? != capability_digest {
        return Err("Copied Profile capability contract differs.".to_owned());
    }
    files.push(ControlPlaneFile {
        path: "config/profile-capabilities.json".to_owned(),
        sha256: capability_digest,
        generated: false,
    });
    for relative in [
        "alpine.exe",
        "Open Minimal OpenCode.cmd",
        "Open Local Qwen.exe",
    ] {
        let path = stage.join(relative);
        if !path.is_file() {
            return Err(format!(
                "generated control-plane file is missing: {relative}"
            ));
        }
        files.push(ControlPlaneFile {
            path: relative.to_owned(),
            sha256: sha256_file(&path)?,
            generated: true,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let git = resolve_executable("git");
    let source_commit = git
        .as_ref()
        .and_then(|git| git_text(git, repository_root, &["rev-parse", "HEAD"]).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let source_dirty = git.as_ref().and_then(|git| {
        git_text(git, repository_root, &["status", "--porcelain"])
            .ok()
            .map(|value| !value.trim().is_empty())
    });
    write_json_atomic(
        &stage.join("config/control-plane.json"),
        &ControlPlaneIdentity {
            schema: 1,
            source_commit,
            source_dirty,
            source_tree_sha256: source_tree_identity(repository_root)?,
            files,
        },
    )
}

fn verify_install(
    repository_root: &Path,
    install_root: &Path,
    profile: &str,
    skip_vision: bool,
    manifest: &ArtifactManifest,
) -> Result<(), String> {
    assert_artifact(
        &install_root.join(&manifest.model.relative_path),
        manifest.model.bytes,
        &manifest.model.sha256,
    )?;
    if !skip_vision {
        assert_artifact(
            &install_root.join(&manifest.mmproj.relative_path),
            manifest.mmproj.bytes,
            &manifest.mmproj.sha256,
        )?;
    }
    assert_artifact(
        &install_root.join(&manifest.chat_template.relative_path),
        manifest.chat_template.bytes,
        &manifest.chat_template.sha256,
    )?;
    for relative in ["alpine.exe", "Open Local Qwen.exe"] {
        if !install_root.join(relative).is_file() {
            return Err(format!(
                "Install file missing: {}",
                install_root.join(relative).display()
            ));
        }
    }
    verify_control_plane_identity(repository_root, install_root)?;
    let installed_alpine = install_root.join("alpine.exe");
    installed_alpine_json(
        &installed_alpine,
        &[
            OsString::from("resolve"),
            OsString::from("--install-root"),
            install_root.as_os_str().to_owned(),
            OsString::from("--profile"),
            OsString::from(profile),
            OsString::from("--compact"),
        ],
        "selected Profile resolution",
    )?;
    installed_alpine_json(
        &installed_alpine,
        &[
            OsString::from("deployment-status"),
            OsString::from("--install-root"),
            install_root.as_os_str().to_owned(),
            OsString::from("--compact"),
        ],
        "deployment status",
    )?;
    installed_alpine_json(
        &installed_alpine,
        &[
            OsString::from("resolve"),
            OsString::from("--install-root"),
            install_root.as_os_str().to_owned(),
            OsString::from("--compact"),
        ],
        "daily-default resolution",
    )?;
    let resolved = crate::config::resolve(install_root, Some(profile), true)?;
    let status = deployment::status(install_root)?;
    let roles = status
        .roles
        .ok_or_else(|| "Installed Alpine has no complete deployment roles.".to_owned())?;
    if roles.daily_default.is_empty() || roles.rollback_profile.is_empty() {
        return Err("Installed Alpine has no complete deployment roles.".to_owned());
    }
    crate::config::resolve(install_root, None, true)?;
    let version = native_version(&resolved.server)?;
    if !version.contains(short_commit(&manifest.llama_cpp.commit)) {
        return Err(format!(
            "llama-server is not pinned to commit {}:\n{version}",
            short_commit(&manifest.llama_cpp.commit)
        ));
    }
    if resolved.runtime_name == "custom" {
        assert_custom_runtime(
            repository_root,
            resolved.server.parent().unwrap_or(install_root),
            manifest,
        )?;
    } else if resolved.runtime_name == "official" {
        assert_official_runtime(resolved.server.parent().unwrap_or(install_root), manifest)?;
    } else {
        return Err(format!(
            "unknown installed runtime identity: {}",
            resolved.runtime_name
        ));
    }
    Ok(())
}

fn verify_control_plane_identity(
    repository_root: &Path,
    install_root: &Path,
) -> Result<(), String> {
    #[derive(Deserialize)]
    struct Identity {
        schema: u32,
        source_tree_sha256: String,
        files: Vec<ControlPlaneFile>,
    }
    let identity: Identity = read_json(
        &install_root.join("config/control-plane.json"),
        "control-plane identity",
    )?;
    if identity.schema != 1 {
        return Err("unsupported control-plane identity schema".to_owned());
    }
    validate_sha256(
        &identity.source_tree_sha256,
        "control-plane source-tree SHA-256",
    )?;
    if identity.source_tree_sha256 != source_tree_identity(repository_root)? {
        return Err(
            "control-plane source tree differs from the installed binary identity".to_owned(),
        );
    }
    let expected = expected_control_plane_paths(repository_root)?;
    let generated: BTreeSet<&str> = [
        "alpine.exe",
        "Open Minimal OpenCode.cmd",
        "Open Local Qwen.exe",
    ]
    .into_iter()
    .collect();
    let mut seen = BTreeSet::new();
    for entry in identity.files {
        validate_sha256(&entry.sha256, "control-plane file SHA-256")?;
        let relative =
            validate_relative_path(Path::new(&entry.path), "control-plane identity path")?;
        let normalized = relative.to_string_lossy().replace('\\', "/");
        if !seen.insert(normalized.clone()) {
            return Err("control-plane identity contains duplicate paths".to_owned());
        }
        if entry.generated != generated.contains(normalized.as_str()) {
            return Err(format!(
                "control-plane generated flag is wrong: {normalized}"
            ));
        }
        let installed = install_root.join(&relative);
        contained_existing(install_root, &installed, "control-plane identity file")?;
        if sha256_file(&installed)? != entry.sha256 {
            return Err(format!(
                "installed control-plane file differs: {normalized}"
            ));
        }
        if !entry.generated {
            let source = match normalized.split_once('/') {
                Some(("launcher", tail)) => repository_root.join("runtime/launcher").join(tail),
                Some(("profiles", tail)) => repository_root.join("config/profiles").join(tail),
                _ if normalized == "config/artifacts.json" => {
                    repository_root.join("config/artifacts.json")
                }
                _ if normalized == "config/profile-capabilities.json" => {
                    repository_root.join("config/profile-capabilities.json")
                }
                _ => return Err(format!("unknown control-plane source path: {normalized}")),
            };
            if sha256_file(&source)? != entry.sha256 {
                return Err(format!("control-plane source is stale: {normalized}"));
            }
        }
    }
    if seen != expected {
        return Err(format!(
            "control-plane identity path set is incomplete or unexpected; expected {:?}, observed {:?}",
            expected, seen
        ));
    }
    Ok(())
}

fn expected_control_plane_paths(repository_root: &Path) -> Result<BTreeSet<String>, String> {
    let mut expected: BTreeSet<String> = [
        "config/artifacts.json".to_owned(),
        "config/profile-capabilities.json".to_owned(),
        "alpine.exe".to_owned(),
        "Open Minimal OpenCode.cmd".to_owned(),
        "Open Local Qwen.exe".to_owned(),
    ]
    .into_iter()
    .collect();
    for (root, prefix) in [
        (repository_root.join("runtime/launcher"), "launcher"),
        (repository_root.join("config/profiles"), "profiles"),
    ] {
        for path in sorted_files(&root)? {
            expected.insert(format!(
                "{prefix}/{}",
                path.file_name()
                    .ok_or_else(|| "source identity path has no filename".to_owned())?
                    .to_string_lossy()
            ));
        }
    }
    Ok(expected)
}

fn source_tree_identity(repository_root: &Path) -> Result<String, String> {
    let mut paths = vec![
        repository_root.join("Cargo.toml"),
        repository_root.join("Cargo.lock"),
    ];
    let mut pending = vec![repository_root.join("src")];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| format!("failed to enumerate Alpine source tree: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("failed to inspect Alpine source: {error}"))?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() && entry.path().extension() == Some(OsStr::new("rs")) {
                paths.push(entry.path());
            }
        }
    }
    tree_sha256(repository_root, &paths)
}

fn installed_alpine_json(
    executable: &Path,
    arguments: &[OsString],
    label: &str,
) -> Result<Value, String> {
    let mut command = Command::new(executable);
    command.args(arguments);
    let output = run_command_bounded(&mut command, Duration::from_secs(30))
        .map_err(|error| format!("failed to run installed Alpine {label}: {error}"))?;
    if output.timed_out || !output.status.success() {
        return Err(format!(
            "installed Alpine {label} failed: {}{}",
            output.stdout, output.stderr
        ));
    }
    serde_json::from_str(&output.stdout)
        .map_err(|error| format!("installed Alpine {label} returned invalid JSON: {error}"))
}

fn install_prerequisites(manifest: &ArtifactManifest) -> Result<(), String> {
    let winget = resolve_executable("winget.exe")
        .or_else(|| resolve_executable("winget"))
        .ok_or_else(|| "winget is required to install prerequisites".to_owned())?;
    for (id, version, extra) in prerequisite_plan(manifest) {
        if (id == "OpenJS.NodeJS.LTS" && resolve_executable("npm").is_some())
            || (id == "Rustlang.Rustup" && resolve_cargo().is_some())
        {
            continue;
        }
        let mut command = Command::new(&winget);
        command.args(["install", "--id", id, "--exact"]);
        if let Some(version) = version {
            command.args(["--version", version]);
        }
        command.args([
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]);
        command.args(extra);
        run_inherited(&mut command, &format!("winget prerequisite {id}"))?;
    }
    let npm = resolve_executable("npm")
        .or_else(|| {
            let candidate = program_files().join("nodejs/npm.cmd");
            candidate.is_file().then_some(candidate)
        })
        .ok_or_else(|| "npm remains unavailable after prerequisite installation".to_owned())?;
    let mut command = Command::new(npm);
    command.args([
        "install",
        "--global",
        &format!("opencode-ai@{PINNED_OPENCODE}"),
    ]);
    run_inherited(&mut command, "OpenCode installation")
}

fn prerequisite_plan(manifest: &ArtifactManifest) -> Vec<(&str, Option<&str>, Vec<&str>)> {
    vec![
        ("Git.Git", None, Vec::new()),
        (
            "Kitware.CMake",
            Some(manifest.llama_cpp.custom_build.cmake.as_str()),
            Vec::new(),
        ),
        (
            "Nvidia.CUDA",
            Some(manifest.llama_cpp.custom_build.cuda.as_str()),
            Vec::new(),
        ),
        (
            "Microsoft.VisualStudio.2022.BuildTools",
            None,
            vec![
                "--override",
                "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended",
            ],
        ),
        ("OpenJS.NodeJS.LTS", None, Vec::new()),
        ("Rustlang.Rustup", None, Vec::new()),
    ]
}

fn has_deployment_events(install_root: &Path) -> Result<bool, String> {
    let events = install_root.join("deployment/events");
    if !events.is_dir() {
        return Ok(false);
    }
    for entry in std::fs::read_dir(events)
        .map_err(|error| format!("failed to inspect deployment history: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("failed to inspect deployment event: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
            && entry.path().extension() == Some(OsStr::new("json"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn copy_flat_directory(source: &Path, destination: &Path) -> Result<(), String> {
    canonical_directory(source, "control-plane source directory")?;
    std::fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create control-plane stage: {error}"))?;
    for source in sorted_files(source)? {
        let name = source
            .file_name()
            .ok_or_else(|| "control-plane source has no filename".to_owned())?;
        copy_atomic(&source, &destination.join(name))?;
    }
    Ok(())
}

fn sorted_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)
        .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to inspect source file: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

fn find_file(root: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("failed to inspect runtime: {error}"))?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() && entry.file_name().eq_ignore_ascii_case(name) {
                return Ok(Some(entry.path()));
            }
        }
    }
    Ok(None)
}

fn runtime_files(
    root: &Path,
    excluded: &[&str],
) -> Result<BTreeMap<String, RuntimeFileIdentity>, String> {
    let root = canonical_directory(root, "runtime directory")?;
    let excluded: BTreeSet<&str> = excluded.iter().copied().collect();
    let mut files = BTreeMap::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .map_err(|error| format!("failed to enumerate runtime tree: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("failed to inspect runtime tree: {error}"))?;
            let metadata = std::fs::symlink_metadata(entry.path())
                .map_err(|error| format!("failed to inspect runtime entry: {error}"))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "runtime tree contains a forbidden symlink or reparse link: {}",
                    entry.path().display()
                ));
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(&root)
                    .map_err(|_| "runtime entry escaped its root".to_owned())?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !excluded.contains(relative.as_str()) {
                    files.insert(
                        relative,
                        RuntimeFileIdentity {
                            bytes: metadata.len(),
                            sha256: sha256_file(&entry.path())?,
                        },
                    );
                }
            }
        }
    }
    Ok(files)
}

fn validate_runtime_files(
    root: &Path,
    expected: &BTreeMap<String, RuntimeFileIdentity>,
    excluded: &[&str],
) -> Result<(), String> {
    for (path, identity) in expected {
        validate_relative_path(Path::new(path), "runtime manifest path")?;
        validate_sha256(&identity.sha256, "runtime file SHA-256")?;
    }
    let observed = runtime_files(root, excluded)?;
    if &observed != expected {
        let expected_paths: BTreeSet<_> = expected.keys().collect();
        let observed_paths: BTreeSet<_> = observed.keys().collect();
        return Err(format!(
            "runtime file set or identity differs; expected {:?}, observed {:?}",
            expected_paths, observed_paths
        ));
    }
    Ok(())
}

fn copy_atomic(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!("copy source is missing: {}", source.display()));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create copy destination: {error}"))?;
    }
    let temporary = append_suffix(destination, &format!(".{}.tmp", Uuid::new_v4().simple()));
    std::fs::copy(source, &temporary).map_err(|error| {
        format!(
            "failed to copy {} to temporary destination: {error}",
            source.display()
        )
    })?;
    if destination.exists() {
        let backup = append_suffix(destination, &format!(".{}.bak", Uuid::new_v4().simple()));
        #[cfg(windows)]
        {
            std::fs::rename(destination, &backup)
                .map_err(|error| format!("failed to stage replaced file: {error}"))?;
            let result = std::fs::rename(&temporary, destination);
            if result.is_err() {
                let _ = std::fs::rename(&backup, destination);
            }
            result.map_err(|error| format!("failed to publish copied file: {error}"))?;
            let _ = std::fs::remove_file(backup);
        }
        #[cfg(not(windows))]
        {
            std::fs::rename(&temporary, destination)
                .map_err(|error| format!("failed to publish copied file: {error}"))?;
        }
    } else {
        std::fs::rename(&temporary, destination)
            .map_err(|error| format!("failed to publish copied file: {error}"))?;
    }
    if temporary.exists() {
        let _ = std::fs::remove_file(temporary);
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, label: &str) -> Result<T, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("failed to read {label} {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("malformed {label} {}: {error}", path.display()))
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    bytes.push(b'\n');
    crate::session::atomic_replace(path, &bytes)
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("failed to resolve current directory: {error}"))
    }
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {label} {}: {error}", path.display()))?;
    if path.is_dir() {
        Ok(path)
    } else {
        Err(format!("{label} is not a directory: {}", path.display()))
    }
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn native_version(executable: &Path) -> Result<String, String> {
    let mut command = Command::new(executable);
    command.arg("--version");
    let output = run_command_bounded(&mut command, Duration::from_secs(30))
        .map_err(|error| format!("failed to run native version probe: {error}"))?;
    if output.timed_out || !output.status.success() {
        return Err(format!(
            "Native version probe failed for {}: {}{}",
            executable.display(),
            output.stdout,
            output.stderr
        ));
    }
    let text = format!("{}{}", output.stdout, output.stderr)
        .trim()
        .to_owned();
    if text.is_empty() {
        Err(format!(
            "Native version probe returned no text: {}",
            executable.display()
        ))
    } else {
        Ok(text)
    }
}

fn short_commit(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

fn run_inherited(command: &mut Command, label: &str) -> Result<(), String> {
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = command
        .status()
        .map_err(|error| format!("failed to start {label}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed with {status}"))
    }
}

fn run_capture_success(command: &mut Command, label: &str) -> Result<(), String> {
    let output = run_command_bounded(command, Duration::from_secs(120))
        .map_err(|error| format!("failed to run {label}: {error}"))?;
    if output.timed_out || !output.status.success() {
        Err(format!(
            "{label} failed: {}{}",
            output.stdout.trim(),
            output.stderr.trim()
        ))
    } else {
        Ok(())
    }
}

fn git_text(git: &Path, root: &Path, arguments: &[&str]) -> Result<String, String> {
    let mut command = Command::new(git);
    command.arg("-C").arg(root).args(arguments);
    let output = run_command_bounded(&mut command, Duration::from_secs(30))
        .map_err(|error| format!("failed to run git: {error}"))?;
    if output.timed_out || !output.status.success() {
        Err(format!("git failed: {}", output.stderr.trim()))
    } else {
        Ok(output.stdout)
    }
}

fn git_status(
    git: &Path,
    root: &Path,
    arguments: &[&str],
) -> Result<std::process::ExitStatus, String> {
    let mut command = Command::new(git);
    command.arg("-C").arg(root).args(arguments);
    command
        .status()
        .map_err(|error| format!("failed to run git: {error}"))
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n")
}

fn resolve_cargo() -> Option<PathBuf> {
    resolve_executable("cargo").or_else(|| {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .map(|root| root.join(".cargo/bin/cargo.exe"))
            .filter(|path| path.is_file())
    })
}

fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

fn program_files_x86() -> PathBuf {
    std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"))
}

fn cuda_root(version: &str) -> PathBuf {
    program_files().join(format!("NVIDIA GPU Computing Toolkit/CUDA/v{version}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::sha256_bytes;

    fn transaction_root(root: &Path, prefix: &str) -> PathBuf {
        root.join(format!("{prefix}{}", "a".repeat(32)))
    }

    fn request(path: &str) -> RequestedPublicationItem {
        requested(path, path).unwrap()
    }

    fn manifest_fixture() -> ArtifactManifest {
        let artifact = |filename: &str, relative: &str| Artifact {
            filename: filename.to_owned(),
            relative_path: PathBuf::from(relative),
            url: "https://example.invalid/artifact".to_owned(),
            sha256: "0".repeat(64),
            bytes: 1,
        };
        ArtifactManifest {
            model: artifact("model.gguf", "models/model.gguf"),
            mmproj: artifact("mmproj.gguf", "models/mmproj.gguf"),
            chat_template: artifact("chat.jinja", "config/chat.jinja"),
            llama_cpp: LlamaCpp {
                repo: "https://example.invalid/llama.cpp".to_owned(),
                commit: "1".repeat(40),
                patch: PathBuf::from("patches/pinned.patch"),
                official_runtime: Download {
                    filename: "runtime.zip".to_owned(),
                    url: "https://example.invalid/runtime".to_owned(),
                    sha256: "2".repeat(64),
                    bytes: 2,
                },
                official_cuda: Download {
                    filename: "cuda.zip".to_owned(),
                    url: "https://example.invalid/cuda".to_owned(),
                    sha256: "3".repeat(64),
                    bytes: 3,
                },
                custom_build: CustomBuild {
                    cuda: "13.2".to_owned(),
                    cmake: "4.2.3".to_owned(),
                    generator: "Visual Studio 17 2022".to_owned(),
                    architecture: "120".to_owned(),
                    options: vec!["GGML_CUDA=ON".to_owned(), "LLAMA_CURL=OFF".to_owned()],
                },
            },
        }
    }

    fn write_repository_contract(root: &Path, profile_runtime: &str) {
        std::fs::create_dir_all(root.join("config/profiles")).unwrap();
        std::fs::create_dir_all(root.join("patches")).unwrap();
        std::fs::write(root.join("patches/pinned.patch"), b"patch").unwrap();
        std::fs::write(
            root.join("config/profiles/stable-16k.json"),
            serde_json::to_vec(&serde_json::json!({
                "name": "stable-16k",
                "runtime": profile_runtime,
                "context": 16384,
                "output": 4096,
                "parallel": 1,
                "threads": 16,
                "batch_size": 2048,
                "ubatch_size": 768,
                "kv_cache": "q8_0",
                "tensor_cpu_through_block": 43,
                "mtp_depth": 3,
                "ngram_mod": false,
                "ngram_reset_on_begin": false,
                "external_skills": false,
                "skill_tool": false,
                "vision_fit": true,
                "fit_target_mib": 512
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("config/artifacts.json"),
            serde_json::to_vec(&serde_json::json!({
                "model": {"filename":"model.gguf","relative_path":"models/model.gguf","url":"https://example.invalid/model","sha256":"0".repeat(64),"bytes":1},
                "mmproj": {"filename":"mmproj.gguf","relative_path":"models/mmproj.gguf","url":"https://example.invalid/mmproj","sha256":"0".repeat(64),"bytes":1},
                "chat_template": {"filename":"chat.jinja","relative_path":"config/chat.jinja","url":"https://example.invalid/chat","sha256":"0".repeat(64),"bytes":1},
                "llama_cpp": {
                    "repo":"https://example.invalid/llama.cpp","commit":"1".repeat(40),"patch":"patches/pinned.patch",
                    "official_runtime":{"filename":"runtime.zip","url":"https://example.invalid/runtime","sha256":"2".repeat(64),"bytes":2},
                    "official_cuda":{"filename":"cuda.zip","url":"https://example.invalid/cuda","sha256":"3".repeat(64),"bytes":3},
                    "custom_build":{"cuda":"13.2","cmake":"4.2.3","generator":"Visual Studio 17 2022","architecture":"120","options":["GGML_CUDA=ON"]}
                }
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn setup_options(repository_root: &Path, install_root: &Path) -> SetupOptions {
        SetupOptions {
            repository_root: repository_root.to_owned(),
            install_root: install_root.to_owned(),
            profile: "stable-16k".to_owned(),
            runtime: SetupRuntime::Official,
            reuse_artifacts_from: None,
            install_prerequisites: false,
            skip_vision: false,
            verify_only: false,
            no_shortcut: true,
            lock_timeout: Duration::from_millis(100),
        }
    }

    #[test]
    fn invalid_or_incompatible_profile_has_no_install_side_effect() {
        let directory = tempfile::tempdir().unwrap();
        let repository = directory.path().join("repo");
        std::fs::create_dir(&repository).unwrap();
        let install = directory.path().join("install");
        let mut options = setup_options(&repository, &install);
        options.profile = "missing".to_owned();
        assert!(run(&options).unwrap_err().contains("Profile"));
        assert!(!install.exists());

        write_repository_contract(&repository, "official");
        let profile_path = repository.join("config/profiles/stable-16k.json");
        let mut invalid: Value =
            serde_json::from_slice(&std::fs::read(&profile_path).unwrap()).unwrap();
        invalid["output"] = serde_json::json!(32768);
        std::fs::write(&profile_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        let error = run(&setup_options(&repository, &install)).unwrap_err();
        assert!(error.contains("output"));
        assert!(!install.exists());

        write_repository_contract(&repository, "custom");
        let options = setup_options(&repository, &install);
        assert!(
            run(&options)
                .unwrap_err()
                .contains("requires the custom runtime")
        );
        assert!(!install.exists());
    }

    #[test]
    fn profile_and_artifact_paths_reject_traversal() {
        for profile in ["../stable", "stable/../../escape", ".", "a..b"] {
            assert!(validate_profile_name(profile).is_err());
        }
        for filename in ["../runtime.zip", "nested/runtime.zip", ".", ".."] {
            assert!(validate_filename(filename, "fixture").is_err());
        }
    }

    #[test]
    fn cleanup_migration_is_typed_and_fail_closed() {
        let disabled = serde_json::json!({
            "enabled": false,
            "exe": "old.exe",
            "start_script": "old.ps1"
        });
        assert_eq!(
            preserved_cleanup(Some(&disabled)).unwrap(),
            disabled_cleanup()
        );
        let typed = serde_json::json!({
            "enabled": true,
            "port": 9191,
            "executable": r"C:\fixture\cleanup.exe",
            "arguments": ["--port", "9191"],
            "stdout": r"C:\fixture\out.log",
            "stderr": r"C:\fixture\err.log",
            "health": "http://127.0.0.1:9191/health"
        });
        assert_eq!(preserved_cleanup(Some(&typed)).unwrap(), typed);
        let legacy = serde_json::json!({
            "enabled": true,
            "port": 9191,
            "exe": "old.exe",
            "start_script": "old.ps1"
        });
        assert!(
            preserved_cleanup(Some(&legacy))
                .unwrap_err()
                .contains("retired exe/start_script")
        );
    }

    #[test]
    fn incomplete_partial_is_resumable_and_bad_complete_partial_is_quarantined() {
        let directory = tempfile::tempdir().unwrap();
        let partial = directory.path().join("artifact.part");
        let destination = directory.path().join("artifact");
        let complete = b"complete-artifact";
        std::fs::write(&partial, &complete[..5]).unwrap();
        let digest = sha256_bytes(complete);
        assert!(
            publish_verified_download(&partial, &destination, complete.len() as u64, &digest)
                .unwrap_err()
                .contains("Keep it for resumable download")
        );
        assert!(partial.is_file());
        assert!(!destination.exists());

        std::fs::write(&partial, vec![b'x'; complete.len()]).unwrap();
        assert!(
            !publish_completed_partial(&partial, &destination, complete.len() as u64, &digest)
                .unwrap()
        );
        assert!(!partial.exists());
        assert_eq!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".part.invalid-"))
                .count(),
            1
        );
        assert!(!destination.exists());
    }

    #[test]
    fn reuse_is_verified_before_and_after_publication() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        let reuse = directory.path().join("reuse");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(reuse.join("models")).unwrap();
        let bytes = b"verified-reuse";
        std::fs::write(reuse.join("models/model.gguf"), bytes).unwrap();
        let artifact = Artifact {
            filename: "model.gguf".to_owned(),
            relative_path: PathBuf::from("models/model.gguf"),
            url: "https://example.invalid".to_owned(),
            sha256: sha256_bytes(bytes),
            bytes: bytes.len() as u64,
        };
        let installed = install_artifact(&install, Some(&reuse), &artifact).unwrap();
        assert_eq!(std::fs::read(installed).unwrap(), bytes);

        std::fs::write(reuse.join("models/model.gguf"), b"wrong").unwrap();
        std::fs::remove_file(install.join("models/model.gguf")).unwrap();
        assert!(install_artifact(&install, Some(&reuse), &artifact).is_err());
        assert!(!install.join("models/model.gguf").exists());
    }

    #[test]
    fn publication_preflights_all_sources_before_first_move() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        std::fs::create_dir_all(install.join("launcher")).unwrap();
        std::fs::write(install.join("launcher/version"), b"old").unwrap();
        let stage = transaction_root(&install, ".control-plane-stage-");
        std::fs::create_dir_all(stage.join("launcher")).unwrap();
        std::fs::write(stage.join("launcher/version"), b"new").unwrap();
        let error = publish_bundle(
            &install,
            &stage,
            &[request("launcher"), request("profiles")],
        )
        .unwrap_err();
        assert!(error.contains("staged publication item"));
        assert_eq!(
            std::fs::read(install.join("launcher/version")).unwrap(),
            b"old"
        );
        assert!(!install.join(".setup-publishing.json").exists());
    }

    #[test]
    fn publication_commits_complete_bundle_and_mid_move_failure_rolls_back() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        std::fs::create_dir_all(install.join("launcher")).unwrap();
        std::fs::write(install.join("launcher/version"), b"old").unwrap();
        let stage = transaction_root(&install, ".control-plane-stage-");
        std::fs::create_dir_all(stage.join("launcher")).unwrap();
        std::fs::write(stage.join("launcher/version"), b"new").unwrap();
        publish_bundle(&install, &stage, &[request("launcher")]).unwrap();
        assert_eq!(
            std::fs::read(install.join("launcher/version")).unwrap(),
            b"new"
        );
        assert!(!stage.exists());
        assert!(!install.join(".setup-publishing.json").exists());

        std::fs::write(install.join("config"), b"parent-is-file").unwrap();
        let stage = install.join(format!(".control-plane-stage-{}", "b".repeat(32)));
        std::fs::create_dir_all(stage.join("launcher")).unwrap();
        std::fs::write(stage.join("launcher/version"), b"newer").unwrap();
        std::fs::create_dir_all(stage.join("config")).unwrap();
        std::fs::write(stage.join("config/session.json"), b"new-session").unwrap();
        let error = publish_bundle(
            &install,
            &stage,
            &[request("launcher"), request("config/session.json")],
        )
        .unwrap_err();
        assert!(error.contains("publication"));
        assert_eq!(
            std::fs::read(install.join("launcher/version")).unwrap(),
            b"new"
        );
        assert_eq!(
            std::fs::read(install.join("config")).unwrap(),
            b"parent-is-file"
        );
        assert!(!install.join(".setup-publishing.json").exists());
    }

    #[test]
    fn stale_powershell_style_marker_recovers_but_non_allowlisted_marker_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        std::fs::create_dir_all(install.join("launcher")).unwrap();
        std::fs::write(install.join("launcher/version"), b"partial").unwrap();
        let stage = transaction_root(&install, ".control-plane-stage-");
        let backup = transaction_root(&install, ".setup-backup-");
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::create_dir_all(backup.join("launcher")).unwrap();
        std::fs::write(backup.join("launcher/version"), b"prior").unwrap();
        let ordinary = |path: &Path| {
            PathBuf::from(
                path.to_string_lossy()
                    .strip_prefix(r"\\?\")
                    .unwrap_or(&path.to_string_lossy())
                    .to_owned(),
            )
        };
        let marker = PublicationMarker {
            schema: 1,
            transaction_id: "fixture".to_owned(),
            started_at: "2026-08-22T00:00:00Z".to_owned(),
            stage_root: ordinary(&stage),
            backup_root: ordinary(&backup),
            items: vec![PublicationItem {
                stage: PathBuf::from("launcher"),
                destination: PathBuf::from("launcher"),
                had_prior: true,
            }],
        };
        write_json_atomic(&install.join(".setup-publishing.json"), &marker).unwrap();
        assert!(repair_interrupted_publication(&install).unwrap());
        assert_eq!(
            std::fs::read(install.join("launcher/version")).unwrap(),
            b"prior"
        );

        std::fs::create_dir_all(&stage).unwrap();
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::create_dir_all(install.join("models")).unwrap();
        std::fs::write(install.join("models/model.gguf"), b"keep").unwrap();
        let hostile = PublicationMarker {
            items: vec![PublicationItem {
                stage: PathBuf::from("models/model.gguf"),
                destination: PathBuf::from("models/model.gguf"),
                had_prior: false,
            }],
            ..marker
        };
        write_json_atomic(&install.join(".setup-publishing.json"), &hostile).unwrap();
        assert!(repair_interrupted_publication(&install).is_err());
        assert_eq!(
            std::fs::read(install.join("models/model.gguf")).unwrap(),
            b"keep"
        );
        assert!(install.join(".setup-publishing.json").is_file());
    }

    #[test]
    fn journal_rejects_duplicate_destinations_and_unsafe_roots() {
        let item = PublicationItem {
            stage: PathBuf::from("launcher"),
            destination: PathBuf::from("launcher"),
            had_prior: false,
        };
        assert!(validate_publication_journal(&[item.clone(), item]).is_err());
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        assert!(contained_marker_root(&root, &root.join("models"), "setup stage").is_err());
        assert!(
            contained_marker_root(
                &root,
                &root
                    .join(format!(".control-plane-stage-{}", "a".repeat(32)))
                    .join("nested"),
                "setup stage"
            )
            .is_err()
        );
    }

    #[test]
    fn zip_extraction_rejects_traversal_and_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let traversal = directory.path().join("traversal.zip");
        {
            let file = File::create(&traversal).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .start_file("../escape.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"escape").unwrap();
            writer.finish().unwrap();
        }
        let destination = directory.path().join("extract");
        std::fs::create_dir(&destination).unwrap();
        assert!(extract_zip_safely(&traversal, &destination).is_err());
        assert!(!directory.path().join("escape.txt").exists());

        let symlink = directory.path().join("symlink.zip");
        {
            let file = File::create(&symlink).unwrap();
            let mut writer = zip::ZipWriter::new(file);
            writer
                .add_symlink("link", "outside", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.finish().unwrap();
        }
        assert!(extract_zip_safely(&symlink, &destination).is_err());
    }

    #[test]
    fn runtime_identity_requires_exact_contract_and_file_set() {
        let manifest = manifest_fixture();
        let build = CustomRuntimeIdentity {
            schema: 1,
            llama_cpp_commit: manifest.llama_cpp.commit.clone(),
            source_patch_sha256: "0".repeat(64),
            cuda_toolkit: manifest.llama_cpp.custom_build.cuda.clone(),
            cuda_architecture: manifest.llama_cpp.custom_build.architecture.clone(),
            cmake_options: vec!["WRONG=ON".to_owned()],
            files: BTreeMap::new(),
        };
        assert!(validate_custom_runtime_contract(&build, &manifest).is_err());

        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.dll"), b"a").unwrap();
        let mut expected = BTreeMap::new();
        expected.insert(
            "a.dll".to_owned(),
            RuntimeFileIdentity {
                bytes: 1,
                sha256: sha256_bytes(b"a"),
            },
        );
        validate_runtime_files(directory.path(), &expected, &[]).unwrap();
        std::fs::write(directory.path().join("extra.dll"), b"extra").unwrap();
        assert!(validate_runtime_files(directory.path(), &expected, &[]).is_err());
    }

    #[test]
    fn control_plane_identity_rejects_omissions_and_bad_digests() {
        let directory = tempfile::tempdir().unwrap();
        let repo = directory.path().join("repo");
        let install = directory.path().join("install");
        for path in [
            repo.join("runtime/launcher"),
            repo.join("config/profiles"),
            repo.join("config"),
            repo.join("src"),
            install.join("launcher"),
            install.join("profiles"),
            install.join("config"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        for (relative, bytes) in [
            ("runtime/launcher/OpenLocalQwen.cs", b"source".as_slice()),
            ("config/profiles/stable-16k.json", b"{}".as_slice()),
            ("config/artifacts.json", b"{}".as_slice()),
            (
                "config/profile-capabilities.json",
                b"{\"schema\":1}".as_slice(),
            ),
            ("Cargo.toml", b"[package]\nname='fixture'\n".as_slice()),
            ("Cargo.lock", b"# lock\n".as_slice()),
            ("src/lib.rs", b"pub fn fixture() {}\n".as_slice()),
        ] {
            let path = repo.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, bytes).unwrap();
        }
        copy_atomic(
            &repo.join("runtime/launcher/OpenLocalQwen.cs"),
            &install.join("launcher/OpenLocalQwen.cs"),
        )
        .unwrap();
        copy_atomic(
            &repo.join("config/profiles/stable-16k.json"),
            &install.join("profiles/stable-16k.json"),
        )
        .unwrap();
        copy_atomic(
            &repo.join("config/artifacts.json"),
            &install.join("config/artifacts.json"),
        )
        .unwrap();
        copy_atomic(
            &repo.join("config/profile-capabilities.json"),
            &install.join("config/profile-capabilities.json"),
        )
        .unwrap();
        for relative in [
            "alpine.exe",
            "Open Minimal OpenCode.cmd",
            "Open Local Qwen.exe",
        ] {
            std::fs::write(install.join(relative), relative.as_bytes()).unwrap();
        }
        write_control_plane_identity(&repo, &install, &repo.join("config/artifacts.json")).unwrap();
        verify_control_plane_identity(&repo, &install).unwrap();

        let identity_path = install.join("config/control-plane.json");
        let mut identity: Value = read_json(&identity_path, "identity").unwrap();
        identity["files"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["path"] != "alpine.exe");
        write_json_atomic(&identity_path, &identity).unwrap();
        assert!(
            verify_control_plane_identity(&repo, &install)
                .unwrap_err()
                .contains("path set")
        );

        write_control_plane_identity(&repo, &install, &repo.join("config/artifacts.json")).unwrap();
        let mut identity: Value = read_json(&identity_path, "identity").unwrap();
        identity["files"][0]["sha256"] = Value::String("bad".to_owned());
        write_json_atomic(&identity_path, &identity).unwrap();
        assert!(
            verify_control_plane_identity(&repo, &install)
                .unwrap_err()
                .contains("64-character")
        );
    }

    #[test]
    fn build_target_is_stage_local_and_prerequisites_are_exact() {
        let stage = Path::new(r"C:\fixture\.control-plane-stage-a");
        assert_eq!(
            alpine_build_target(stage),
            stage.join(".alpine-build-target")
        );
        let manifest = manifest_fixture();
        let plan = prerequisite_plan(&manifest);
        assert_eq!(plan[1].1, Some("4.2.3"));
        assert_eq!(plan[2].1, Some("13.2"));
        assert!(plan.iter().any(|(id, _, _)| *id == "Rustlang.Rustup"));
    }

    #[test]
    fn stage_guard_cleans_unpublished_stage() {
        let directory = tempfile::tempdir().unwrap();
        let install = directory.path().join("install");
        std::fs::create_dir(&install).unwrap();
        let stage = transaction_root(&install, ".control-plane-stage-");
        std::fs::create_dir(&stage).unwrap();
        {
            let _guard = StageGuard::new(&install, &stage);
        }
        assert!(!stage.exists());
    }
}
