use crate::identity::sha256_bytes;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    pub schema: u32,
    pub root: PathBuf,
    pub host: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
    pub runtimes: BTreeMap<String, Option<PathBuf>>,
    pub model: PathBuf,
    pub mmproj: PathBuf,
    pub chat_template: PathBuf,
    pub api_key_file: PathBuf,
    pub base_url_file: PathBuf,
    pub state_file: PathBuf,
    #[serde(default)]
    pub cleanup: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CleanupConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub executable: Option<PathBuf>,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub stdout: Option<PathBuf>,
    #[serde(default)]
    pub stderr: Option<PathBuf>,
    #[serde(default)]
    pub health: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub runtime: String,
    pub context: u32,
    pub output: u32,
    pub parallel: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub ubatch_size: u32,
    pub kv_cache: String,
    pub tensor_cpu_through_block: u32,
    pub mtp_depth: u32,
    pub ngram_mod: bool,
    pub ngram_reset_on_begin: bool,
    pub external_skills: bool,
    pub skill_tool: bool,
    pub vision_fit: bool,
    pub fit_target_mib: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileCapabilityContract {
    schema: u32,
    maximum_threads: u32,
    runtimes: BTreeMap<String, RuntimeCapabilities>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeCapabilities {
    kv_cache: Vec<String>,
    request_local_ngram: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedSession {
    pub install_root: PathBuf,
    pub session_config_path: PathBuf,
    pub session_config_sha256: String,
    pub session: SessionConfig,
    pub profile_path: PathBuf,
    pub profile_sha256: String,
    pub profile_name: String,
    pub profile: Profile,
    pub runtime_name: String,
    pub server: PathBuf,
    pub model: PathBuf,
    pub mmproj: PathBuf,
    pub chat_template: PathBuf,
    pub api_key_file: PathBuf,
    pub base_url_file: PathBuf,
    pub state_file: PathBuf,
    pub base_url: String,
}

pub fn resolve(
    install_root: &Path,
    selected_profile: Option<&str>,
    require_runtime: bool,
) -> Result<ResolvedSession, String> {
    let install_root = canonical_directory(install_root, "install root")?;
    let publication_marker = install_root.join(".setup-publishing.json");
    if publication_marker.exists() {
        return Err(format!(
            "Setup publication is incomplete: {}. Re-run setup to restore the prior installation before using it.",
            publication_marker.display()
        ));
    }

    let session_path = install_root.join("config/session.json");
    let (session, session_bytes): (SessionConfig, _) =
        read_json_with_bytes(&session_path, "Session Config")?;
    validate_session(&session, &session_path, &install_root)?;

    let deployment_default = if selected_profile.is_none() {
        crate::deployment::daily_default(&install_root)?
    } else {
        None
    };
    let profile_name = selected_profile
        .or(deployment_default.as_deref())
        .or(session.active_profile.as_deref())
        .ok_or_else(|| {
            "no Profile was selected and deployment history has no daily_default".to_owned()
        })?;
    validate_profile_name(profile_name)?;
    let profile_path = install_root
        .join("profiles")
        .join(format!("{profile_name}.json"));
    let (profile, profile_bytes) = read_profile_with_bytes(&profile_path)?;
    validate_profile(&profile, profile_name, &profile_path)?;

    let server = session
        .runtimes
        .get(&profile.runtime)
        .and_then(Clone::clone)
        .ok_or_else(|| {
            format!(
                "Runtime '{}' is unavailable for Profile '{}'.",
                profile.runtime, profile.name
            )
        })?;
    let server = normalized_absolute_path(&server, "runtime path")?;
    if require_runtime && !server.is_file() {
        return Err(format!(
            "Runtime '{}' is unavailable at {}.",
            profile.runtime,
            server.display()
        ));
    }

    let base_url = format!("http://{}:{}", session.host, session.port);
    Ok(ResolvedSession {
        install_root,
        session_config_path: session_path,
        session_config_sha256: sha256_bytes(&session_bytes),
        profile_path,
        profile_sha256: sha256_bytes(&profile_bytes),
        profile_name: profile.name.clone(),
        runtime_name: profile.runtime.clone(),
        server,
        model: normalized_absolute_path(&session.model, "model path")?,
        mmproj: normalized_absolute_path(&session.mmproj, "vision projector path")?,
        chat_template: normalized_absolute_path(&session.chat_template, "chat template path")?,
        api_key_file: normalized_absolute_path(&session.api_key_file, "API key path")?,
        base_url_file: normalized_absolute_path(&session.base_url_file, "base URL path")?,
        state_file: normalized_absolute_path(&session.state_file, "state path")?,
        base_url,
        session,
        profile,
    })
}

fn validate_session(
    session: &SessionConfig,
    path: &Path,
    install_root: &Path,
) -> Result<(), String> {
    if !matches!(session.schema, 3..=5) {
        return Err(format!(
            "Unsupported Session Config schema '{}' in {}; expected 3, 4 or 5.",
            session.schema,
            path.display()
        ));
    }
    let configured_root = canonical_directory(&session.root, "Session Config root")?;
    if configured_root != install_root {
        return Err(format!(
            "Session Config root '{}' does not match install root '{}'.",
            configured_root.display(),
            install_root.display()
        ));
    }
    if session.port == 0 {
        return Err(format!(
            "Session Config port must be between 1 and 65535: {}",
            path.display()
        ));
    }
    if session.host != "localhost"
        && session
            .host
            .parse::<IpAddr>()
            .map_or(true, |address| !address.is_loopback())
    {
        return Err(format!(
            "Session Config host must resolve explicitly to loopback: {}",
            path.display()
        ));
    }
    if session.host.trim().is_empty() {
        return Err(format!(
            "{}: required value 'host' must be a non-empty string",
            path.display()
        ));
    }
    let capabilities = profile_capabilities()?;
    let unknown_runtimes = session
        .runtimes
        .keys()
        .filter(|name| !capabilities.runtimes.contains_key(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown_runtimes.is_empty() {
        return Err(format!(
            "Session Config contains unsupported runtime names {}; expected only {}: {}",
            unknown_runtimes.join(", "),
            capabilities
                .runtimes
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            path.display()
        ));
    }
    match (session.schema, session.active_profile.as_deref()) {
        (3, Some(profile)) if !profile.trim().is_empty() => {}
        (3, _) => {
            return Err(format!(
                "{}: schema 3 requires a non-empty legacy active_profile",
                path.display()
            ));
        }
        (4 | 5, None) => {}
        (4 | 5, Some(_)) => {
            return Err(format!(
                "{}: schema {} stores deployment roles in append-only deployment history, not active_profile",
                path.display(),
                session.schema
            ));
        }
        _ => unreachable!(),
    }
    cleanup_config(session)?;
    Ok(())
}

pub(crate) fn cleanup_config(session: &SessionConfig) -> Result<Option<CleanupConfig>, String> {
    if session.schema < 5 {
        let enabled = match &session.cleanup {
            serde_json::Value::Null => false,
            serde_json::Value::Object(cleanup) => match cleanup.get("enabled") {
                None => false,
                Some(serde_json::Value::Bool(enabled)) => *enabled,
                Some(_) => {
                    return Err(format!(
                        "Session Config schema {} cleanup.enabled must be a Boolean",
                        session.schema
                    ));
                }
            },
            _ => {
                return Err(format!(
                    "Session Config schema {} cleanup must be an object or null",
                    session.schema
                ));
            }
        };
        if enabled {
            return Err(format!(
                "Session Config schema {} uses the retired cleanup start_script contract; migrate it to schema 5 with executable, arguments, stdout and stderr before starting or stopping inference",
                session.schema
            ));
        }
        if session
            .cleanup
            .as_object()
            .is_some_and(|cleanup| cleanup.keys().any(|key| key != "enabled"))
        {
            return Err(format!(
                "Session Config schema {} cleanup contains unknown fields; only enabled is supported",
                session.schema
            ));
        }
        return Ok(None);
    }
    let cleanup: CleanupConfig = serde_json::from_value(session.cleanup.clone())
        .map_err(|error| format!("invalid schema 5 cleanup configuration: {error}"))?;
    Ok(cleanup.enabled.then_some(cleanup))
}

pub(crate) fn validate_profile(
    profile: &Profile,
    selected: &str,
    path: &Path,
) -> Result<(), String> {
    if profile.name != selected {
        return Err(format!(
            "Profile name '{}' does not match selected name '{}'.",
            profile.name, selected
        ));
    }
    let capabilities = profile_capabilities()?;
    let supported_runtimes = capabilities.runtimes.keys().cloned().collect::<Vec<_>>();
    let Some(runtime) = capabilities.runtimes.get(&profile.runtime) else {
        return Err(format!(
            "Profile runtime '{}' is unsupported; expected one of: {}: {}",
            profile.runtime,
            supported_runtimes.join(", "),
            path.display()
        ));
    };
    if !runtime.kv_cache.contains(&profile.kv_cache) {
        return Err(format!(
            "Profile kv_cache '{}' is unsupported by runtime '{}'; expected one of: {}: {}",
            profile.kv_cache,
            profile.runtime,
            runtime.kv_cache.join(", "),
            path.display()
        ));
    }
    for (name, value) in [
        ("context", profile.context),
        ("output", profile.output),
        ("parallel", profile.parallel),
        ("threads", profile.threads),
        ("batch_size", profile.batch_size),
        ("ubatch_size", profile.ubatch_size),
        ("mtp_depth", profile.mtp_depth),
        ("fit_target_mib", profile.fit_target_mib),
    ] {
        if value == 0 {
            return Err(format!(
                "Profile value '{name}' must be a positive integer: {}",
                path.display()
            ));
        }
    }
    if profile.output > profile.context {
        return Err(format!(
            "Profile output ({}) must not exceed context ({}): {}",
            profile.output,
            profile.context,
            path.display()
        ));
    }
    if profile.ubatch_size > profile.batch_size {
        return Err(format!(
            "Profile ubatch_size ({}) must not exceed batch_size ({}): {}",
            profile.ubatch_size,
            profile.batch_size,
            path.display()
        ));
    }
    if profile.threads > capabilities.maximum_threads {
        return Err(format!(
            "Profile threads ({}) exceeds Alpine's supported sanity limit of {}: {}",
            profile.threads,
            capabilities.maximum_threads,
            path.display()
        ));
    }
    if profile.ngram_reset_on_begin && !profile.ngram_mod {
        return Err(format!(
            "Profile ngram_reset_on_begin requires ngram_mod: {}",
            path.display()
        ));
    }
    if profile.ngram_mod && !runtime.request_local_ngram {
        return Err(format!(
            "Profile runtime '{}' does not support request-local ngram_mod; select a runtime whose capability contract enables it: {}",
            profile.runtime,
            path.display()
        ));
    }
    Ok(())
}

fn profile_capabilities() -> Result<ProfileCapabilityContract, String> {
    let contract: ProfileCapabilityContract =
        serde_json::from_str(include_str!("../config/profile-capabilities.json"))
            .map_err(|error| format!("invalid embedded Profile capability contract: {error}"))?;
    if contract.schema != 1
        || contract.maximum_threads == 0
        || contract.runtimes.is_empty()
        || contract.runtimes.iter().any(|(name, runtime)| {
            name.trim().is_empty()
                || runtime.kv_cache.is_empty()
                || runtime.kv_cache.iter().any(|value| value.trim().is_empty())
                || runtime
                    .kv_cache
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != runtime.kv_cache.len()
        })
    {
        return Err("embedded Profile capability contract is incomplete or invalid".to_owned());
    }
    Ok(contract)
}

fn validate_profile_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        });
    if valid {
        Ok(())
    } else {
        Err(format!("invalid Profile name '{name}'"))
    }
}

fn read_json_with_bytes<T: for<'de> Deserialize<'de>>(
    path: &Path,
    kind: &str,
) -> Result<(T, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "{kind} missing or unreadable at {}: {error}",
            path.display()
        )
    })?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Malformed {kind} {}: {error}", path.display()))?;
    Ok((value, bytes))
}

fn read_profile_with_bytes(path: &Path) -> Result<(Profile, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|error| {
        format!(
            "Profile missing or unreadable at {}: {error}",
            path.display()
        )
    })?;
    let raw: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Malformed Profile {}: {error}", path.display()))?;
    if raw.get("status").is_some() {
        return Err(format!(
            "Malformed Profile {}: field 'status' is not a Profile setting; lifecycle and deployment roles belong to append-only deployment history",
            path.display()
        ));
    }
    let profile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Malformed Profile {}: {error}", path.display()))?;
    Ok((profile, bytes))
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("{label} is unavailable at {}: {error}", path.display()))?;
    let canonical = without_windows_verbatim_prefix(canonical);
    if canonical.is_dir() {
        Ok(canonical)
    } else {
        Err(format!(
            "{label} is not a directory: {}",
            canonical.display()
        ))
    }
}

fn without_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let rendered = path.to_string_lossy();
    if let Some(unc) = rendered.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(local) = rendered.strip_prefix(r"\\?\") {
        PathBuf::from(local)
    } else {
        path
    }
}

fn normalized_absolute_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be absolute: {}", path.display()));
    }
    let mut existing = path;
    let mut suffix = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            return Err(format!(
                "{label} has no existing ancestor: {}",
                path.display()
            ));
        };
        suffix.push(name.to_os_string());
        existing = existing
            .parent()
            .ok_or_else(|| format!("{label} has no existing ancestor: {}", path.display()))?;
    }
    let mut normalized =
        without_windows_verbatim_prefix(std::fs::canonicalize(existing).map_err(|error| {
            format!("failed to normalize {label} at {}: {error}", path.display())
        })?);
    for component in suffix.into_iter().rev() {
        normalized.push(component);
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_fixture(root: &Path) {
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::create_dir_all(root.join("profiles")).unwrap();
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        let server = root.join("runtime/llama-server.exe");
        std::fs::write(&server, b"fixture").unwrap();
        std::fs::write(
            root.join("profiles/stable-16k.json"),
            serde_json::to_vec(&json!({
                "name": "stable-16k", "runtime": "official",
                "context": 16384, "output": 4096, "parallel": 1, "threads": 16,
                "batch_size": 2048, "ubatch_size": 768, "kv_cache": "q8_0",
                "tensor_cpu_through_block": 43, "mtp_depth": 3, "ngram_mod": false,
                "ngram_reset_on_begin": false, "external_skills": false, "skill_tool": false,
                "vision_fit": true, "fit_target_mib": 512
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join("config/session.json"),
            serde_json::to_vec(&json!({
                "schema": 3, "root": root, "host": "127.0.0.1", "port": 8123,
                "active_profile": "stable-16k", "runtimes": {"official": server},
                "model": root.join("models/model.gguf"), "mmproj": root.join("models/mmproj.gguf"),
                "chat_template": root.join("config/chat.jinja"),
                "api_key_file": root.join("config/api-key.txt"),
                "base_url_file": root.join("config/base-url.txt"),
                "state_file": root.join("logs/session-state.json"), "cleanup": {"enabled": false}
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn resolves_the_complete_session_contract() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        let resolved = resolve(directory.path(), None, true).unwrap();
        assert_eq!(resolved.profile_name, "stable-16k");
        assert_eq!(resolved.runtime_name, "official");
        assert_eq!(resolved.base_url, "http://127.0.0.1:8123");
        assert!(resolved.server.is_file());
        assert!(!resolved.install_root.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn schema_five_uses_deployment_selection_and_rejects_legacy_active_profile() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        let path = directory.path().join("config/session.json");
        let mut session: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        session["schema"] = json!(5);
        session.as_object_mut().unwrap().remove("active_profile");
        std::fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        let resolved = resolve(directory.path(), Some("stable-16k"), true).unwrap();
        assert_eq!(resolved.session.schema, 5);

        session["active_profile"] = json!("stable-16k");
        std::fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        let error = resolve(directory.path(), Some("stable-16k"), true).unwrap_err();
        assert!(error.contains("schema 5"));
        assert!(error.contains("deployment history"));
    }

    #[test]
    fn enabled_cleanup_requires_explicit_schema_five_launch_contract() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        let path = directory.path().join("config/session.json");
        let mut session: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        session["cleanup"] = json!({
            "enabled": true,
            "port": 8090,
            "exe": directory.path().join("cleanup/cleanup.exe"),
            "start_script": directory.path().join("cleanup/start.ps1"),
            "health": "http://127.0.0.1:8090/health"
        });
        std::fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        let error = resolve(directory.path(), Some("stable-16k"), true).unwrap_err();
        assert!(error.contains("retired cleanup start_script contract"));
        assert!(error.contains("migrate it to schema 5"));

        session["schema"] = json!(5);
        session.as_object_mut().unwrap().remove("active_profile");
        session["cleanup"] = json!({
            "enabled": true,
            "port": 8090,
            "executable": directory.path().join("cleanup/cleanup.exe"),
            "arguments": ["--host", "127.0.0.1", "--port", "8090"],
            "stdout": directory.path().join("cleanup/logs/stdout.log"),
            "stderr": directory.path().join("cleanup/logs/stderr.log"),
            "health": "http://127.0.0.1:8090/health"
        });
        std::fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        assert!(resolve(directory.path(), Some("stable-16k"), true).is_ok());

        session["cleanup"]["start_script"] = json!(directory.path().join("cleanup/start.ps1"));
        std::fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        assert!(
            resolve(directory.path(), Some("stable-16k"), true)
                .unwrap_err()
                .contains("unknown field `start_script`")
        );
    }

    #[test]
    fn incomplete_publication_and_profile_traversal_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        std::fs::write(directory.path().join(".setup-publishing.json"), b"{}").unwrap();
        assert!(
            resolve(directory.path(), None, true)
                .unwrap_err()
                .contains("incomplete")
        );
        std::fs::remove_file(directory.path().join(".setup-publishing.json")).unwrap();
        assert!(
            resolve(directory.path(), Some("../outside"), true)
                .unwrap_err()
                .contains("invalid Profile name")
        );
    }

    #[test]
    fn profile_and_session_unknown_fields_fail_closed_actionably() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        let profile_path = directory.path().join("profiles/stable-16k.json");
        let original_profile = std::fs::read(&profile_path).unwrap();
        let mut profile: serde_json::Value = serde_json::from_slice(&original_profile).unwrap();
        profile["status"] = json!("production");
        std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        let error = resolve(directory.path(), None, true).unwrap_err();
        assert!(error.contains("status"));
        assert!(error.contains("deployment history"));

        std::fs::write(&profile_path, &original_profile).unwrap();
        let session_path = directory.path().join("config/session.json");
        let mut session: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&session_path).unwrap()).unwrap();
        session["llama_server"] = json!("obsolete");
        std::fs::write(&session_path, serde_json::to_vec(&session).unwrap()).unwrap();
        assert!(
            resolve(directory.path(), None, true)
                .unwrap_err()
                .contains("unknown field `llama_server`")
        );
    }

    #[test]
    fn invalid_profile_relationships_and_enumerations_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        let profile_path = directory.path().join("profiles/stable-16k.json");
        let original: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&profile_path).unwrap()).unwrap();
        for (field, value, expected) in [
            ("output", json!(32768), "output"),
            ("ubatch_size", json!(4096), "ubatch_size"),
            ("kv_cache", json!("mystery"), "kv_cache"),
            ("runtime", json!("mystery"), "runtime"),
            ("threads", json!(257), "threads"),
        ] {
            let mut profile = original.clone();
            profile[field] = value;
            std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
            let error = resolve(directory.path(), None, true).unwrap_err();
            assert!(error.contains(expected), "unexpected error: {error}");
        }

        for (ngram_mod, reset, expected) in [
            (true, true, "runtime"),
            (false, true, "ngram_reset_on_begin"),
        ] {
            let mut profile = original.clone();
            profile["ngram_mod"] = json!(ngram_mod);
            profile["ngram_reset_on_begin"] = json!(reset);
            std::fs::write(&profile_path, serde_json::to_vec(&profile).unwrap()).unwrap();
            let error = resolve(directory.path(), None, true).unwrap_err();
            assert!(error.contains(expected), "unexpected error: {error}");
        }
    }

    #[test]
    fn every_checked_in_profile_satisfies_the_closed_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for entry in std::fs::read_dir(root.join("config/profiles")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let name = path.file_stem().unwrap().to_str().unwrap();
            let (profile, _): (Profile, Vec<u8>) = read_json_with_bytes(&path, "Profile").unwrap();
            validate_profile(&profile, name, &path).unwrap();
        }
    }

    #[test]
    fn non_loopback_endpoint_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        let path = directory.path().join("config/session.json");
        let mut session: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        session["host"] = serde_json::Value::String("0.0.0.0".to_owned());
        std::fs::write(&path, serde_json::to_vec(&session).unwrap()).unwrap();
        assert!(
            resolve(directory.path(), None, true)
                .unwrap_err()
                .contains("loopback")
        );
    }

    #[test]
    fn resolution_binds_exact_session_and_profile_bytes() {
        let directory = tempfile::tempdir().unwrap();
        write_fixture(directory.path());
        let first = resolve(directory.path(), None, true).unwrap();
        let profile_path = directory.path().join("profiles/stable-16k.json");
        let mut bytes = std::fs::read(&profile_path).unwrap();
        bytes.push(b'\n');
        std::fs::write(&profile_path, bytes).unwrap();
        let second = resolve(directory.path(), None, true).unwrap();
        assert_eq!(first.session_config_sha256, second.session_config_sha256);
        assert_ne!(first.profile_sha256, second.profile_sha256);
    }

    #[test]
    fn deployment_default_overrides_legacy_selection_without_mutating_material_config() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        write_fixture(root);
        let stable_path = root.join("profiles/stable-16k.json");
        let mut turbo: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&stable_path).unwrap()).unwrap();
        turbo["name"] = json!("turbo-16k");
        std::fs::write(
            root.join("profiles/turbo-16k.json"),
            serde_json::to_vec(&turbo).unwrap(),
        )
        .unwrap();
        let legacy = resolve(root, None, true).unwrap();
        crate::deployment::bootstrap(&crate::deployment::BootstrapDeploymentOptions {
            install_root: root.to_path_buf(),
            daily_default: "turbo-16k".to_owned(),
            rollback_profile: "stable-16k".to_owned(),
            operator: "test-operator".to_owned(),
            reason: "prove role selection is separate from material configuration".to_owned(),
            lock_timeout: std::time::Duration::from_secs(1),
        })
        .unwrap();
        let deployed = resolve(root, None, true).unwrap();
        let override_session = resolve(root, Some("stable-16k"), true).unwrap();
        assert_eq!(legacy.profile_name, "stable-16k");
        assert_eq!(deployed.profile_name, "turbo-16k");
        assert_eq!(override_session.profile_name, "stable-16k");
        assert_eq!(legacy.session_config_sha256, deployed.session_config_sha256);
    }
}
