use crate::clock::UtcTimestamp;
use crate::config;
use crate::locking::InterprocessLock;
use crate::process::{resolve_executable, run_command_bounded};
use crate::session::{
    self, AcquireSessionOptions, ReleaseSessionOptions, SessionAcquisition, SessionSnapshot,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) const MODEL_ID: &str = "local-models/Qwen3.8-27B-ABLITERATED";
const INTERRUPT_GRACE: Duration = Duration::from_secs(10);
const JOURNAL_SCHEMA: u32 = 1;
const FAILURE_LOG_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static INTERRUPT_HANDLER: OnceLock<Result<(), String>> = OnceLock::new();

#[derive(Debug, Clone)]
pub struct OpenCodeOptions {
    pub install_root: PathBuf,
    pub project: PathBuf,
    pub profile: Option<String>,
    pub launch_id: String,
    pub vision: bool,
    pub lean: bool,
    pub with_convex: bool,
    pub with_skills: bool,
    pub with_project_config: bool,
    pub with_plugins: bool,
    pub keep_server: bool,
    pub check: bool,
    pub diagnostic_failure: bool,
    pub allow_legacy_identity: bool,
    pub lock_timeout: Duration,
    pub startup_timeout: Duration,
    pub opencode_args: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenCodeReport {
    pub profile: String,
    pub project: PathBuf,
    pub context_tokens: u32,
    pub exit_code: i32,
    pub checked_only: bool,
    pub interrupted: bool,
    pub restored_prior_session: bool,
    pub failure_log: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct HarnessPolicyOptions {
    pub lean: bool,
    pub skills_enabled: bool,
    pub with_convex: bool,
}

const TOOL_OUTPUT_MAX_LINES: u64 = 500;
const TOOL_OUTPUT_MAX_BYTES: u64 = 12 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchJournal {
    schema: u32,
    launch_id: String,
    owner_pid: u32,
    created_at: String,
    phase: JournalPhase,
    prior: SessionSnapshot,
    acquisition: Option<SessionAcquisition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum JournalPhase {
    Preparing,
    Acquired,
}

pub fn run(options: &OpenCodeOptions) -> Result<OpenCodeReport, String> {
    match run_inner(options) {
        Ok(mut report) => {
            if report.exit_code != 0 {
                let reason = if report.interrupted {
                    "OpenCode was interrupted; Alpine restored the prior Inference Session."
                        .to_owned()
                } else {
                    format!(
                        "OpenCode exited with code {}. Its native diagnostic remained in the terminal.",
                        report.exit_code
                    )
                };
                report.failure_log = Some(publish_failure(options, &reason)?);
            }
            Ok(report)
        }
        Err(error) => {
            let redacted = redact_failure(&error);
            match publish_failure(options, &redacted) {
                Ok(path) => Err(format!(
                    "{redacted}\nLauncher failure log: {}",
                    path.display()
                )),
                Err(log_error) => Err(format!(
                    "{redacted}\nThe launcher failure log could not be published: {log_error}"
                )),
            }
        }
    }
}

fn run_inner(options: &OpenCodeOptions) -> Result<OpenCodeReport, String> {
    validate_options(options)?;
    if options.diagnostic_failure {
        return Err("Deterministic installed launcher diagnostic failure.".to_owned());
    }
    let project = std::fs::canonicalize(&options.project).map_err(|error| {
        format!(
            "failed to resolve OpenCode project {}: {error}",
            options.project.display()
        )
    })?;
    let project = command_compatible_path(&project);
    if !project.is_dir() {
        return Err(format!(
            "OpenCode project is not a directory: {}",
            project.display()
        ));
    }
    let resolved = config::resolve(&options.install_root, options.profile.as_deref(), true)?;
    session::ensure_provider_files(&resolved, options.lock_timeout)?;
    let skills_enabled = resolved.profile.external_skills || options.with_skills;
    let policy = harness_policy(
        &resolved,
        &HarnessPolicyOptions {
            lean: options.lean,
            skills_enabled,
            with_convex: options.with_convex,
        },
    );
    let policy_json = serde_json::to_string(&policy)
        .map_err(|error| format!("failed to encode OpenCode policy: {error}"))?;
    let environment =
        harness_environment(&policy_json, skills_enabled, options.with_project_config);
    let opencode = resolve_executable("opencode")
        .ok_or_else(|| "OpenCode executable is unavailable on PATH".to_owned())?;
    let opencode = std::fs::canonicalize(&opencode)
        .map_err(|error| format!("failed to resolve OpenCode executable: {error}"))?;
    verify_effective_policy(
        &opencode,
        &project,
        &environment,
        !options.with_plugins,
        &resolved,
        skills_enabled,
    )?;
    if options.check {
        return Ok(OpenCodeReport {
            profile: resolved.profile_name,
            project,
            context_tokens: resolved.profile.context,
            exit_code: 0,
            checked_only: true,
            interrupted: false,
            restored_prior_session: true,
            failure_log: None,
        });
    }

    ensure_interrupt_handler()?;
    INTERRUPTED.store(false, Ordering::SeqCst);
    let capacity_path = resolved.install_root.join("logs/inference.lease");
    let _capacity = InterprocessLock::acquire(&capacity_path, options.lock_timeout)
        .map_err(|error| format!("inference capacity is unavailable: {error}"))?;
    recover_stale_journals(options)?;
    let prior = session::snapshot_under_capacity(&options.install_root, options.lock_timeout)?;
    let journal_path = write_preparing_journal(options, &prior)?;
    let acquisition = match session::acquire_under_capacity(&AcquireSessionOptions {
        install_root: options.install_root.clone(),
        profile: Some(resolved.profile_name.clone()),
        vision: options.vision,
        force_fallback: false,
        allow_legacy_identity: options.allow_legacy_identity,
        lock_timeout: options.lock_timeout,
        startup_timeout: options.startup_timeout,
    }) {
        Ok(acquisition) => acquisition,
        Err(acquisition_error) => {
            let observed =
                session::snapshot_under_capacity(&options.install_root, options.lock_timeout)?;
            if session::snapshots_materially_equal(&observed, &prior) {
                remove_journal(&journal_path)?;
                return Err(acquisition_error);
            }
            return Err(format!(
                "{acquisition_error}; the interrupted acquisition did not restore its prior Session and remains journaled at {}",
                journal_path.display()
            ));
        }
    };
    if !session::snapshots_materially_equal(&acquisition.prior, &prior) {
        let release = session::release_under_capacity(&ReleaseSessionOptions {
            install_root: options.install_root.clone(),
            acquisition,
            keep_server: false,
            lock_timeout: options.lock_timeout,
            startup_timeout: options.startup_timeout,
        });
        return match release {
            Ok(_) => {
                remove_journal(&journal_path)?;
                Err(
                    "Inference Session changed between recovery journaling and acquisition"
                        .to_owned(),
                )
            }
            Err(release_error) => Err(format!(
                "Inference Session changed between recovery journaling and acquisition; restoration failed: {release_error}; recovery remains journaled at {}",
                journal_path.display()
            )),
        };
    }
    if let Err(journal_error) = publish_acquired_journal(options, &journal_path, &acquisition) {
        let release = session::release_under_capacity(&ReleaseSessionOptions {
            install_root: options.install_root.clone(),
            acquisition,
            keep_server: false,
            lock_timeout: options.lock_timeout,
            startup_timeout: options.startup_timeout,
        });
        return match release {
            Ok(_) => {
                remove_journal(&journal_path)?;
                Err(journal_error)
            }
            Err(release_error) => Err(format!(
                "{journal_error}; prior-session restoration also failed: {release_error}"
            )),
        };
    }
    println!(
        "Opening OpenCode: {} | context={} | project={}",
        resolved.profile_name,
        resolved.profile.context,
        project.display()
    );
    println!(
        "Capabilities: core tools on | skills={} | plugins={} | project config={}",
        skills_enabled, options.with_plugins, options.with_project_config
    );
    println!(
        "Boundary: consent gates for destructive, external, credential, and privilege effects; no topic or technique filter."
    );

    let child_result = run_interactive_child(
        &opencode,
        &project,
        &environment,
        options.with_plugins,
        &options.opencode_args,
    );
    let interrupted = INTERRUPTED.swap(false, Ordering::SeqCst);
    let release = session::release_under_capacity(&ReleaseSessionOptions {
        install_root: options.install_root.clone(),
        acquisition: acquisition.clone(),
        keep_server: options.keep_server,
        lock_timeout: options.lock_timeout,
        startup_timeout: options.startup_timeout,
    });
    let restoration = release.and_then(|_| {
        if options.keep_server {
            return Ok(false);
        }
        let observed =
            session::snapshot_under_capacity(&options.install_root, options.lock_timeout)?;
        if session::snapshots_materially_equal(&observed, &acquisition.prior) {
            Ok(true)
        } else {
            Err("OpenCode launcher did not restore the prior material Session".to_owned())
        }
    });
    if restoration.is_ok() {
        remove_journal(&journal_path)?;
    }
    let restored_prior_session = match restoration {
        Ok(restored) => restored,
        Err(restoration_error) => {
            return match child_result {
                Ok(_) => Err(restoration_error),
                Err(child_error) => Err(format!(
                    "OpenCode launch failed: {child_error}; restoration also failed: {restoration_error}"
                )),
            };
        }
    };
    let status = child_result?;
    let exit_code = if interrupted {
        130
    } else {
        status.code().unwrap_or(1)
    };
    Ok(OpenCodeReport {
        profile: resolved.profile_name,
        project,
        context_tokens: resolved.profile.context,
        exit_code,
        checked_only: false,
        interrupted,
        restored_prior_session,
        failure_log: None,
    })
}

fn run_interactive_child(
    opencode: &Path,
    project: &Path,
    environment: &[(OsString, OsString)],
    with_plugins: bool,
    extra_args: &[OsString],
) -> Result<ExitStatus, String> {
    let mut command = Command::new(opencode);
    if !with_plugins {
        command.arg("--pure");
    }
    command
        .args([OsStr::new("--model"), OsStr::new(MODEL_ID)])
        .arg(project)
        .args(extra_args)
        .current_dir(project)
        .env_clear()
        .envs(environment.iter().cloned())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start OpenCode: {error}"))?;
    let mut interrupt_started = None;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed while waiting for OpenCode: {error}"))?
        {
            return Ok(status);
        }
        if INTERRUPTED.load(Ordering::SeqCst) {
            let started = interrupt_started.get_or_insert_with(Instant::now);
            if started.elapsed() >= INTERRUPT_GRACE {
                child.kill().map_err(|error| {
                    format!("OpenCode ignored Ctrl-C and could not be terminated: {error}")
                })?;
                return child
                    .wait()
                    .map_err(|error| format!("failed to reap interrupted OpenCode: {error}"));
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn ensure_interrupt_handler() -> Result<(), String> {
    INTERRUPT_HANDLER
        .get_or_init(|| {
            ctrlc::set_handler(|| {
                INTERRUPTED.store(true, Ordering::SeqCst);
            })
            .map_err(|error| format!("failed to install Ctrl-C restoration handler: {error}"))
        })
        .clone()
}

fn validate_options(options: &OpenCodeOptions) -> Result<(), String> {
    if options
        .profile
        .as_deref()
        .is_some_and(|profile| profile.trim().is_empty())
        || options.launch_id.len() != 32
        || !options
            .launch_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(
            "OpenCode Profile override must be non-empty and a 32-character launch id is required"
                .to_owned(),
        );
    }
    if options.lock_timeout.is_zero() || options.startup_timeout.is_zero() {
        return Err("OpenCode lock and startup timeouts must be positive".to_owned());
    }
    if options
        .opencode_args
        .iter()
        .any(|argument| argument.to_string_lossy().starts_with("--auto"))
    {
        return Err(
            "--auto disables the operator-consent boundary and is not accepted by this launcher"
                .to_owned(),
        );
    }
    Ok(())
}

pub(crate) fn harness_policy(
    resolved: &config::ResolvedSession,
    options: &HarnessPolicyOptions,
) -> Value {
    let api_path = slash_path(&resolved.api_key_file);
    let base_path = slash_path(&resolved.base_url_file);
    let mut credential_read = Map::new();
    credential_read.insert("*".to_owned(), json!("allow"));
    for &path in credential_paths() {
        credential_read.insert(path.to_owned(), json!("deny"));
    }
    credential_read.insert(api_path.clone(), json!("deny"));

    let mut external = Map::new();
    external.insert("*".to_owned(), json!("ask"));
    for &path in credential_paths() {
        external.insert(path.to_owned(), json!("deny"));
    }
    external.insert(api_path.clone(), json!("deny"));

    let mut bash = Map::new();
    bash.insert("*".to_owned(), json!("allow"));
    for &command in safety_gated_commands() {
        bash.insert(command.to_owned(), json!("ask"));
    }
    for command in ["git remote", "git remote *"] {
        bash.insert(command.to_owned(), json!("allow"));
    }
    let provider_options = json!({
        "baseURL": format!("{{file:{base_path}}}"),
        "apiKey": format!("{{file:{api_path}}}")
    });
    let mut permission = Map::new();
    permission.insert("webfetch".to_owned(), json!("allow"));
    permission.insert("websearch".to_owned(), json!("allow"));
    permission.insert("external_directory".to_owned(), Value::Object(external));
    permission.insert("read".to_owned(), Value::Object(credential_read));
    permission.insert("bash".to_owned(), Value::Object(bash));
    for capability in [
        "edit",
        "write",
        "patch",
        "glob",
        "grep",
        "task",
        "todowrite",
    ] {
        permission.insert(capability.to_owned(), json!("allow"));
    }
    if !options.skills_enabled {
        permission.insert("skill".to_owned(), json!("deny"));
    }
    let mut agent = Map::new();
    agent.insert("title".to_owned(), json!({"disable": true}));
    if options.lean {
        let prompt = "Act as a production coding agent. Follow the user's request, inspect before editing, preserve unrelated work, use available tools when useful, and verify changes proportionately. Ask before destructive, irreversible, external-write, credential, privilege, or privacy-sensitive effects. Never expose secrets. This policy does not restrict topics, reasoning, or technical methods. Report failed or skipped checks.";
        agent.insert("build".to_owned(), json!({"prompt": prompt}));
        agent.insert("plan".to_owned(), json!({"prompt": prompt}));
    }
    json!({
        "provider": {
            "local-models": {
                "npm": "@ai-sdk/openai-compatible",
                "name": "Pinned local Qwen",
                "options": provider_options,
                "models": {
                    "Qwen3.8-27B-ABLITERATED": {
                        "name": format!("Qwen3.8-27B Abliterated ({})", resolved.profile.name),
                        "limit": {
                            "context": resolved.profile.context,
                            "output": resolved.profile.output
                        }
                    }
                }
            }
        },
        "agent": Value::Object(agent),
        "tool_output": {
            "max_lines": TOOL_OUTPUT_MAX_LINES,
            "max_bytes": TOOL_OUTPUT_MAX_BYTES
        },
        "mcp": {"convex": {"enabled": options.with_convex}},
        "permission": Value::Object(permission)
    })
}

pub(crate) fn harness_environment(
    policy: &str,
    skills_enabled: bool,
    with_project_config: bool,
) -> Vec<(OsString, OsString)> {
    let mut environment = sanitized_environment();
    environment.extend([
        (
            OsString::from("OPENCODE_DISABLE_CLAUDE_CODE_PROMPT"),
            OsString::from("true"),
        ),
        (
            OsString::from("OPENCODE_DISABLE_EXTERNAL_SKILLS"),
            OsString::from(if skills_enabled { "false" } else { "true" }),
        ),
        (
            OsString::from("OPENCODE_DISABLE_PROJECT_CONFIG"),
            OsString::from(if with_project_config { "false" } else { "true" }),
        ),
        (
            OsString::from("OPENCODE_ENABLE_EXA"),
            OsString::from("true"),
        ),
        (
            OsString::from("OPENCODE_CONFIG_CONTENT"),
            OsString::from(policy),
        ),
    ]);
    environment
}

pub(crate) fn sanitized_environment() -> Vec<(OsString, OsString)> {
    const EXACT: [&str; 16] = [
        "SSH_AUTH_SOCK",
        "SSH_AGENT_PID",
        "GIT_ASKPASS",
        "SSH_ASKPASS",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "AWS_SHARED_CREDENTIALS_FILE",
        "AZURE_CONFIG_DIR",
        "KUBECONFIG",
        "DOCKER_CONFIG",
        "NPM_CONFIG_USERCONFIG",
        "NETRC",
        "PGPASSWORD",
        "GITHUB_PAT",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
    ];
    std::env::vars_os()
        .filter(|(name, _)| {
            let upper = name.to_string_lossy().to_ascii_uppercase();
            !EXACT.contains(&upper.as_str()) && !secret_name(&upper)
        })
        .collect()
}

pub(crate) fn assert_effective_policy(
    effective: &Value,
    resolved: &config::ResolvedSession,
    skills_enabled: bool,
) -> Result<(), String> {
    let model = effective
        .pointer("/provider/local-models/models/Qwen3.8-27B-ABLITERATED")
        .ok_or_else(|| "OpenCode effective policy lost the local model".to_owned())?;
    if model.pointer("/limit/context").and_then(Value::as_u64)
        != Some(u64::from(resolved.profile.context))
        || model.pointer("/limit/output").and_then(Value::as_u64)
            != Some(u64::from(resolved.profile.output))
    {
        return Err("OpenCode and Inference Session context limits differ".to_owned());
    }
    let permission = effective
        .get("permission")
        .and_then(Value::as_object)
        .ok_or_else(|| "OpenCode effective permissions are absent".to_owned())?;
    for capability in ["webfetch", "websearch"] {
        if permission.get(capability).and_then(Value::as_str) != Some("allow") {
            return Err(format!(
                "read-only web capability is unavailable: {capability}"
            ));
        }
    }
    for capability in [
        "edit",
        "write",
        "patch",
        "glob",
        "grep",
        "task",
        "todowrite",
    ] {
        if permission.get(capability).and_then(Value::as_str) != Some("allow") {
            return Err(format!(
                "core capability is not explicitly available: {capability}"
            ));
        }
    }
    let skill = permission.get("skill").and_then(Value::as_str);
    if (skills_enabled && skill == Some("deny")) || (!skills_enabled && skill != Some("deny")) {
        return Err("OpenCode skill-catalog policy does not match the explicit mode".to_owned());
    }
    let bash = permission
        .get("bash")
        .and_then(Value::as_object)
        .ok_or_else(|| "OpenCode effective shell permissions are absent".to_owned())?;
    if bash.get("*").and_then(Value::as_str) != Some("allow") {
        return Err("routine local shell work is not allowed".to_owned());
    }
    for &command in safety_gated_commands() {
        if bash.get(command).and_then(Value::as_str) != Some("ask") {
            return Err(format!(
                "safety-sensitive effect is not consent-gated: {command}"
            ));
        }
    }
    if bash.get("git remote *").and_then(Value::as_str) != Some("allow") {
        return Err("read-only Git remote inspection is not allowed".to_owned());
    }
    let api_path = slash_path(&resolved.api_key_file);
    let read = permission
        .get("read")
        .and_then(Value::as_object)
        .ok_or_else(|| "OpenCode effective read permissions are absent".to_owned())?;
    if read.get("*").and_then(Value::as_str) != Some("allow")
        || read.get(&api_path).and_then(Value::as_str) != Some("deny")
    {
        return Err("the local provider credential is not denied to agent tools".to_owned());
    }
    let external = permission
        .get("external_directory")
        .and_then(Value::as_object)
        .ok_or_else(|| "OpenCode external-directory permissions are absent".to_owned())?;
    if external.get("*").and_then(Value::as_str) != Some("ask") {
        return Err("outside-project access is not consent-gated".to_owned());
    }
    if effective
        .pointer("/tool_output/max_lines")
        .and_then(Value::as_u64)
        != Some(TOOL_OUTPUT_MAX_LINES)
        || effective
            .pointer("/tool_output/max_bytes")
            .and_then(Value::as_u64)
            != Some(TOOL_OUTPUT_MAX_BYTES)
    {
        return Err("OpenCode tool-output bounds do not match the 16K harness".to_owned());
    }
    for &path in credential_paths() {
        if read.get(path).and_then(Value::as_str) != Some("deny")
            || external.get(path).and_then(Value::as_str) != Some("deny")
        {
            return Err(format!("raw credential path is not denied: {path}"));
        }
    }
    Ok(())
}

fn verify_effective_policy(
    opencode: &Path,
    project: &Path,
    environment: &[(OsString, OsString)],
    pure: bool,
    resolved: &config::ResolvedSession,
    skills_enabled: bool,
) -> Result<(), String> {
    let mut command = Command::new(opencode);
    if pure {
        command.arg("--pure");
    }
    command
        .args(["debug", "config"])
        .current_dir(project)
        .env_clear()
        .envs(environment.iter().cloned());
    let output = run_command_bounded(&mut command, Duration::from_secs(60))
        .map_err(|error| format!("failed to inspect OpenCode policy: {error}"))?;
    if output.timed_out || !output.status.success() {
        return Err(format!(
            "OpenCode effective-config check failed: {}",
            output.stderr.trim()
        ));
    }
    let effective: Value = serde_json::from_str(&output.stdout)
        .map_err(|error| format!("OpenCode effective config was invalid JSON: {error}"))?;
    assert_effective_policy(&effective, resolved, skills_enabled)
}

fn credential_paths() -> &'static [&'static str] {
    &[
        "~/.ssh/id_rsa",
        "~/.ssh/id_dsa",
        "~/.ssh/id_ecdsa",
        "~/.ssh/id_ed25519",
        "~/.aws/credentials",
        "~/.azure/accessTokens.json",
        "~/.azure/msal_token_cache.json",
        "~/.kube/config",
        "~/.config/gcloud/application_default_credentials.json",
        "~/.config/gcloud/credentials.db",
        "~/.docker/config.json",
        "~/.config/gh/hosts.yml",
        "~/AppData/Roaming/GitHub CLI/hosts.yml",
        "~/.git-credentials",
        "~/.npmrc",
        "~/.pypirc",
        "~/.local/share/opencode/auth.json",
    ]
}

fn safety_gated_commands() -> &'static [&'static str] {
    &[
        "git push",
        "git push *",
        "git pull",
        "git pull *",
        "gh api * --method POST *",
        "gh api * --method PUT *",
        "gh api * --method PATCH *",
        "gh api * --method DELETE *",
        "gh api * --method=POST *",
        "gh api * --method=PUT *",
        "gh api * --method=PATCH *",
        "gh api * --method=DELETE *",
        "gh api * -f *",
        "gh api * -F *",
        "gh api * --raw-field *",
        "gh api * --field *",
        "gh api * --input *",
        "gh issue create *",
        "gh issue edit *",
        "gh issue close *",
        "gh issue reopen *",
        "gh issue delete *",
        "gh issue comment *",
        "gh pr create *",
        "gh pr edit *",
        "gh pr close *",
        "gh pr reopen *",
        "gh pr comment *",
        "gh pr review *",
        "gh pr ready *",
        "gh pr merge *",
        "gh repo create *",
        "gh release create *",
        "npm publish",
        "npm publish *",
        "pnpm publish *",
        "yarn npm publish *",
        "vercel deploy *",
        "vercel --prod *",
        "wrangler deploy *",
        "git reset --hard",
        "git reset --hard *",
        "git clean *",
        "git rebase",
        "git rebase *",
        "git filter-branch *",
        "git filter-repo *",
        "git checkout -- *",
        "git checkout -f *",
        "git restore *",
        "git commit --amend",
        "git commit --amend *",
        "git branch -D *",
        "git stash drop *",
        "git stash clear",
        "git worktree remove *",
        "git remote add *",
        "git remote remove *",
        "git remote rename *",
        "git remote set-url *",
        "git config --global *",
        "git config --system *",
        "ssh",
        "ssh *",
        "scp *",
        "sftp *",
        "rsync *",
        "curl * -X POST *",
        "curl * -X PUT *",
        "curl * -X PATCH *",
        "curl * -X DELETE *",
        "curl * --request POST *",
        "curl * --request PUT *",
        "curl * --request PATCH *",
        "curl * --request DELETE *",
        "curl * --data *",
        "curl * --data-raw *",
        "curl * --data-binary *",
        "curl * --json *",
        "curl * --form *",
        "curl * --upload-file *",
        "curl * --user *",
        "curl * --oauth2-bearer *",
        "wget * --method=POST *",
        "wget * --method=PUT *",
        "wget * --method=PATCH *",
        "wget * --method=DELETE *",
        "wget * --post-data *",
        "wget * --post-file *",
        "wget * --body-data *",
        "wget * --body-file *",
        "wget * --user *",
        "wget * --password *",
        "invoke-webrequest * -method post *",
        "invoke-webrequest * -method put *",
        "invoke-webrequest * -method patch *",
        "invoke-webrequest * -method delete *",
        "invoke-webrequest * -body *",
        "invoke-webrequest * -infile *",
        "invoke-webrequest * -credential *",
        "invoke-webrequest * -authentication *",
        "invoke-webrequest * -token *",
        "invoke-restmethod * -method post *",
        "invoke-restmethod * -method put *",
        "invoke-restmethod * -method patch *",
        "invoke-restmethod * -method delete *",
        "invoke-restmethod * -body *",
        "invoke-restmethod * -infile *",
        "invoke-restmethod * -credential *",
        "invoke-restmethod * -authentication *",
        "invoke-restmethod * -token *",
        "rm *",
        "remove-item *",
        "del *",
        "erase *",
        "rmdir *",
        "rd *",
        "stop-process *",
        "taskkill *",
        "stop-service *",
        "restart-service *",
        "restart-computer *",
        "shutdown *",
        "runas *",
        "sudo *",
        "start-process * -verb runas *",
        "set-executionpolicy *",
        "reg add *",
        "reg delete *",
        "format-volume *",
        "clear-disk *",
        "initialize-disk *",
    ]
}

fn write_preparing_journal(
    options: &OpenCodeOptions,
    prior: &SessionSnapshot,
) -> Result<PathBuf, String> {
    let path = journal_directory(&options.install_root)
        .join(format!("{}.json", options.launch_id.to_ascii_lowercase()));
    let journal = LaunchJournal {
        schema: JOURNAL_SCHEMA,
        launch_id: options.launch_id.to_ascii_lowercase(),
        owner_pid: std::process::id(),
        created_at: UtcTimestamp::now()?.rfc3339(),
        phase: JournalPhase::Preparing,
        prior: prior.clone(),
        acquisition: None,
    };
    publish_journal(&path, &journal)?;
    Ok(path)
}

fn publish_acquired_journal(
    options: &OpenCodeOptions,
    path: &Path,
    acquisition: &SessionAcquisition,
) -> Result<(), String> {
    let journal = LaunchJournal {
        schema: JOURNAL_SCHEMA,
        launch_id: options.launch_id.to_ascii_lowercase(),
        owner_pid: std::process::id(),
        created_at: UtcTimestamp::now()?.rfc3339(),
        phase: JournalPhase::Acquired,
        prior: acquisition.prior.clone(),
        acquisition: Some(acquisition.clone()),
    };
    publish_journal(path, &journal)
}

fn publish_journal(path: &Path, journal: &LaunchJournal) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(&journal)
        .map_err(|error| format!("failed to encode OpenCode recovery journal: {error}"))?;
    bytes.push(b'\n');
    session::atomic_replace(path, &bytes)?;
    Ok(())
}

fn recover_stale_journals(options: &OpenCodeOptions) -> Result<(), String> {
    let directory = journal_directory(&options.install_root);
    if !directory.exists() {
        return Ok(());
    }
    let mut paths = std::fs::read_dir(&directory)
        .map_err(|error| format!("failed to inspect OpenCode recovery journals: {error}"))?
        .map(|entry| {
            entry
                .map(|value| value.path())
                .map_err(|error| format!("failed to inspect OpenCode recovery journal: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    for path in paths {
        if path.extension() != Some(OsStr::new("json")) {
            continue;
        }
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.len() > 1024 * 1024 {
            return Err(format!(
                "OpenCode recovery journal exceeds 1 MiB: {}",
                path.display()
            ));
        }
        let journal: LaunchJournal = serde_json::from_slice(
            &std::fs::read(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?,
        )
        .map_err(|error| {
            format!(
                "invalid OpenCode recovery journal {}: {error}",
                path.display()
            )
        })?;
        if journal.schema != JOURNAL_SCHEMA
            || journal.launch_id.len() != 32
            || journal.owner_pid == 0
        {
            return Err(format!(
                "invalid OpenCode recovery journal contract: {}",
                path.display()
            ));
        }
        let observed =
            session::snapshot_under_capacity(&options.install_root, options.lock_timeout)?;
        if session::snapshots_materially_equal(&observed, &journal.prior) {
            remove_journal(&path)?;
            continue;
        }
        match (journal.phase, journal.acquisition.as_ref()) {
            (JournalPhase::Acquired, Some(acquisition))
                if session::snapshots_materially_equal(&acquisition.prior, &journal.prior) =>
            {
                session::release_under_capacity(&ReleaseSessionOptions {
                    install_root: options.install_root.clone(),
                    acquisition: acquisition.clone(),
                    keep_server: false,
                    lock_timeout: options.lock_timeout,
                    startup_timeout: options.startup_timeout,
                })?;
            }
            (JournalPhase::Preparing, None) => {
                session::restore_snapshot_under_capacity(
                    &options.install_root,
                    &journal.prior,
                    options.lock_timeout,
                    options.startup_timeout,
                )?;
            }
            _ => {
                return Err(format!(
                    "invalid OpenCode recovery journal phase: {}",
                    path.display()
                ));
            }
        }
        let restored =
            session::snapshot_under_capacity(&options.install_root, options.lock_timeout)?;
        if !session::snapshots_materially_equal(&restored, &journal.prior) {
            return Err(format!(
                "stale OpenCode launch {} could not restore its prior Session",
                journal.launch_id
            ));
        }
        remove_journal(&path)?;
    }
    Ok(())
}

fn remove_journal(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to retire OpenCode recovery journal {}: {error}",
            path.display()
        )),
    }
}

fn journal_directory(install_root: &Path) -> PathBuf {
    install_root.join("logs/opencode-acquisitions")
}

fn publish_failure(options: &OpenCodeOptions, error: &str) -> Result<PathBuf, String> {
    let logs = options.install_root.join("logs");
    let invocation = logs
        .join("launcher-errors")
        .join(format!("{}.log", options.launch_id.to_ascii_lowercase()));
    let stable = logs.join("launcher-last-error.log");
    let content = format!(
        "timestamp={}\nlaunch_id={}\nprofile={}\nproject={}\nerror:\n{}\n",
        UtcTimestamp::now()?.rfc3339(),
        options.launch_id.to_ascii_lowercase(),
        options
            .profile
            .as_deref()
            .map(redact_failure)
            .unwrap_or_else(|| "<deployment-default>".to_owned()),
        redact_failure(&options.project.display().to_string()),
        redact_failure(error)
    );
    session::atomic_replace(&invocation, content.as_bytes())?;
    let _lock = InterprocessLock::acquire(
        &logs.join("launcher-failure-log.lock"),
        FAILURE_LOG_LOCK_TIMEOUT,
    )?;
    session::atomic_replace(&stable, content.as_bytes())?;
    Ok(stable)
}

fn redact_failure(value: &str) -> String {
    let mut redacted = value.to_owned();
    let mut secrets = std::env::vars()
        .filter(|(name, value)| secret_name(&name.to_ascii_uppercase()) && !value.is_empty())
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for secret in secrets {
        redacted = redacted.replace(&secret, "<REDACTED>");
    }
    for expression in [
        r"(?i)\bsk-[A-Za-z0-9_-]{6,}\b",
        r#"(?i)(authorization\s*:\s*bearer\s+)[^\s\"']+"#,
        r#"(?i)\b(api[_-]?key|access[_-]?key|client[_-]?secret|private[_-]?key|token|password|secret|credential|database[_-]?url|redis[_-]?url|dsn|connection[_-]?string)\b(\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s;]+)"#,
        r#"(?i)\b(user\s*id|uid|password|pwd)\b(\s*=\s*)(?:"[^"]*"|'[^']*'|[^;\s]+)"#,
    ] {
        if let Ok(regex) = Regex::new(expression) {
            redacted = regex.replace_all(&redacted, "$1$2<REDACTED>").into_owned();
        }
    }
    if let Ok(regex) = Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://)([^/\s:@]+):([^@\s/]+)@") {
        redacted = regex
            .replace_all(&redacted, "$1<REDACTED>:<REDACTED>@")
            .into_owned();
    }
    redacted
}

fn secret_name(upper: &str) -> bool {
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "API_KEY",
        "APIKEY",
        "ACCESS_KEY",
        "CREDENTIAL",
        "AUTH",
        "DATABASE_URL",
        "REDIS_URL",
        "MONGO_URI",
        "PGPASSWORD",
        "GITHUB_PAT",
    ]
    .iter()
    .any(|word| upper.contains(word))
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn command_compatible_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn resolved() -> config::ResolvedSession {
        let root = PathBuf::from(r"C:\fixture");
        config::ResolvedSession {
            install_root: root.clone(),
            session_config_path: root.join("config/session.json"),
            session_config_sha256: "a".repeat(64),
            session: config::SessionConfig {
                schema: 3,
                root: root.clone(),
                host: "127.0.0.1".to_owned(),
                port: 8100,
                active_profile: Some("stable-16k".to_owned()),
                runtimes: BTreeMap::new(),
                model: root.join("model.gguf"),
                mmproj: root.join("mmproj.gguf"),
                chat_template: root.join("chat.jinja"),
                api_key_file: root.join("config/api-key.txt"),
                base_url_file: root.join("config/base-url.txt"),
                state_file: root.join("logs/session-state.json"),
                cleanup: Value::Null,
            },
            profile_path: root.join("profiles/stable-16k.json"),
            profile_sha256: "b".repeat(64),
            profile_name: "stable-16k".to_owned(),
            profile: config::Profile {
                name: "stable-16k".to_owned(),
                runtime: "official".to_owned(),
                context: 16_384,
                output: 4096,
                parallel: 1,
                threads: 16,
                batch_size: 2048,
                ubatch_size: 768,
                kv_cache: "q8_0".to_owned(),
                tensor_cpu_through_block: 43,
                mtp_depth: 3,
                ngram_mod: false,
                ngram_reset_on_begin: false,
                external_skills: false,
                skill_tool: false,
                vision_fit: true,
                fit_target_mib: 512,
            },
            runtime_name: "official".to_owned(),
            server: root.join("runtime/llama-server.exe"),
            model: root.join("model.gguf"),
            mmproj: root.join("mmproj.gguf"),
            chat_template: root.join("chat.jinja"),
            api_key_file: root.join("config/api-key.txt"),
            base_url_file: root.join("config/base-url.txt"),
            state_file: root.join("logs/session-state.json"),
            base_url: "http://127.0.0.1:8100".to_owned(),
        }
    }

    fn options(install_root: PathBuf) -> OpenCodeOptions {
        OpenCodeOptions {
            install_root: install_root.clone(),
            project: install_root.join("project"),
            profile: Some("stable-16k".to_owned()),
            launch_id: "a".repeat(32),
            vision: false,
            lean: true,
            with_convex: false,
            with_skills: false,
            with_project_config: false,
            with_plugins: false,
            keep_server: false,
            check: false,
            diagnostic_failure: false,
            allow_legacy_identity: false,
            lock_timeout: Duration::from_secs(1),
            startup_timeout: Duration::from_secs(1),
            opencode_args: Vec::new(),
        }
    }

    #[test]
    fn redaction_covers_inline_credentials_without_hiding_diagnostics() {
        let raw = "startup failed token=inline-secret Authorization: Bearer bearer-secret postgresql://alice:db-password@localhost/app sk-example123";
        let redacted = redact_failure(raw);
        assert!(redacted.contains("startup failed"));
        for secret in [
            "inline-secret",
            "bearer-secret",
            "alice",
            "db-password",
            "sk-example123",
        ] {
            assert!(!redacted.contains(secret));
        }
    }

    #[test]
    fn auto_mode_is_rejected_before_launch() {
        let mut options = options(PathBuf::from(r"C:\fixture"));
        options.opencode_args = vec![OsString::from("--auto")];
        assert!(validate_options(&options).unwrap_err().contains("consent"));
    }

    #[test]
    fn safety_rules_gate_effects_without_disabling_core_tools() {
        let gated = safety_gated_commands();
        assert!(gated.contains(&"git push *"));
        assert!(gated.contains(&"gh issue create *"));
        assert!(gated.contains(&"rm *"));
        assert!(gated.contains(&"curl * --data *"));
        assert!(gated.contains(&"gh api * --method POST *"));
        for read_only_research in [
            "gh api *",
            "curl *",
            "wget *",
            "invoke-webrequest *",
            "invoke-restmethod *",
        ] {
            assert!(!gated.contains(&read_only_research));
        }
        assert!(!gated.iter().any(|command| command.contains("search")));
        assert!(!gated.iter().any(|command| command.contains("read source")));
    }

    #[test]
    fn effective_policy_verifier_rejects_any_post_merge_weakening() {
        let resolved = resolved();
        let policy = harness_policy(
            &resolved,
            &HarnessPolicyOptions {
                lean: true,
                skills_enabled: false,
                with_convex: false,
            },
        );
        assert!(assert_effective_policy(&policy, &resolved, false).is_ok());

        let mut weakened = policy.clone();
        weakened["permission"]["bash"]["gh api * --method POST *"] = json!("allow");
        assert!(assert_effective_policy(&weakened, &resolved, false).is_err());

        let mut capability_removed = policy.clone();
        capability_removed["permission"]["task"] = json!("deny");
        assert!(assert_effective_policy(&capability_removed, &resolved, false).is_err());

        let mut websearch_removed = policy.clone();
        websearch_removed["permission"]["websearch"] = json!("deny");
        assert!(assert_effective_policy(&websearch_removed, &resolved, false).is_err());

        let mut output_unbounded = policy;
        output_unbounded["tool_output"]["max_bytes"] = json!(51_200);
        assert!(assert_effective_policy(&output_unbounded, &resolved, false).is_err());
    }

    #[test]
    fn harness_explicitly_enables_websearch_for_the_local_provider() {
        let environment = harness_environment("{}", false, false);
        assert!(
            environment
                .iter()
                .any(|(name, value)| { name == "OPENCODE_ENABLE_EXA" && value == "true" })
        );
    }

    #[test]
    fn hard_denials_are_exact_secrets_not_entire_capability_directories() {
        let paths = credential_paths();
        assert!(paths.contains(&"~/.aws/credentials"));
        assert!(paths.contains(&"~/.ssh/id_ed25519"));
        assert!(!paths.contains(&"~/.aws/**"));
        assert!(!paths.contains(&"~/.ssh"));
    }

    #[test]
    fn common_credential_carriers_are_scrubbed_by_name() {
        for name in [
            "DATABASE_URL",
            "REDIS_URL",
            "MONGO_URI",
            "PGPASSWORD",
            "GITHUB_PAT",
            "MY_API_KEY",
        ] {
            assert!(secret_name(name), "{name}");
        }
    }

    #[test]
    fn recovery_journal_is_published_before_session_acquisition() {
        let directory = tempfile::tempdir().unwrap();
        let options = options(directory.path().to_path_buf());
        let prior = SessionSnapshot {
            active: false,
            healthy: false,
            profile: String::new(),
            vision: false,
            runtime: String::new(),
            fallback: None,
            arguments: Vec::new(),
            environment: BTreeMap::new(),
            session_identity: None,
        };
        let path = write_preparing_journal(&options, &prior).unwrap();
        let journal: LaunchJournal =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(journal.phase, JournalPhase::Preparing);
        assert_eq!(journal.prior, prior);
        assert!(journal.acquisition.is_none());
    }

    #[cfg(windows)]
    #[test]
    fn command_paths_do_not_expose_verbatim_drive_prefixes_to_cmd() {
        assert_eq!(
            command_compatible_path(Path::new(r"\\?\C:\fixture\project")),
            PathBuf::from(r"C:\fixture\project")
        );
        assert_eq!(
            command_compatible_path(Path::new(r"\\?\UNC\server\share\project")),
            PathBuf::from(r"\\server\share\project")
        );
    }
}
