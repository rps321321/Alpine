//! Project-scoped file and shell capabilities used by the Pi Agent Runtime Adapter.

use crate::store::{DesktopStore, StoreError, ToolProposal, ToolResult};
use alpine_control_plane::sanitized_process_environment;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_SEARCH_FILE_BYTES: u64 = 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_LINES: usize = 2_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub path: String,
    pub kind: &'static str,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRead {
    pub path: String,
    pub content: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSearchMatch {
    pub path: String,
    pub line: usize,
    pub preview: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceEdit {
    pub path: String,
    pub old_text: String,
    pub new_text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEditResult {
    pub path: String,
    pub replacements: usize,
    pub diff: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceShell {
    pub command: String,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceShellResult {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub truncated: bool,
}

pub fn list_project_files(
    store: &DesktopStore,
    task_id: &str,
    limit: usize,
) -> Result<Vec<WorkspaceEntry>, StoreError> {
    if !(1..=10_000).contains(&limit) {
        return Err(error("file listing limit must be between 1 and 10000"));
    }
    let root = project_root(store, task_id)?;
    let mut entries = Vec::new();
    visit_directory(&root, &root, limit, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries.truncate(limit);
    Ok(entries)
}

pub fn read_project_file(
    store: &DesktopStore,
    task_id: &str,
    relative_path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<WorkspaceRead, StoreError> {
    let root = project_root(store, task_id)?;
    let path = existing_scoped_path(&root, relative_path)?;
    let metadata = std::fs::metadata(&path).map_err(|cause| io_error("inspect", &path, cause))?;
    if !metadata.is_file() {
        return Err(error("the requested project path is not a file"));
    }
    if metadata.len() > MAX_READ_BYTES {
        return Err(error(format!(
            "the requested file exceeds the {} KiB read limit",
            MAX_READ_BYTES / 1024
        )));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|cause| io_error("read UTF-8 file", &path, cause))?;
    let lines = content.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let start_line = offset.unwrap_or(1);
    if start_line == 0 || (total_lines > 0 && start_line > total_lines) {
        return Err(error("the requested line offset is outside the file"));
    }
    let requested = limit.unwrap_or(400);
    if requested == 0 || requested > 2_000 {
        return Err(error("the requested line limit must be between 1 and 2000"));
    }
    let start_index = start_line.saturating_sub(1);
    let end_index = (start_index + requested).min(total_lines);
    Ok(WorkspaceRead {
        path: display_relative(&root, &path)?,
        content: lines[start_index..end_index].join("\n"),
        start_line,
        end_line: end_index,
        total_lines,
        truncated: end_index < total_lines,
    })
}

pub fn search_project_files(
    store: &DesktopStore,
    task_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<WorkspaceSearchMatch>, StoreError> {
    let query = query.trim();
    if query.is_empty() || query.len() > 512 {
        return Err(error("search query must contain between 1 and 512 bytes"));
    }
    if !(1..=500).contains(&limit) {
        return Err(error("search result limit must be between 1 and 500"));
    }
    let root = project_root(store, task_id)?;
    let mut files = Vec::new();
    collect_files(&root, &root, 20_000, &mut files)?;
    files.sort();
    let mut matches = Vec::new();
    for path in files {
        if matches.len() >= limit {
            break;
        }
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (index, line) in content.lines().enumerate() {
            if line.contains(query) {
                matches.push(WorkspaceSearchMatch {
                    path: display_relative(&root, &path)?,
                    line: index + 1,
                    preview: truncate_text(line.trim(), 320),
                });
                if matches.len() >= limit {
                    break;
                }
            }
        }
    }
    Ok(matches)
}

pub fn edit_project_file(
    store: &DesktopStore,
    task_id: &str,
    approval_id: &str,
    edit: WorkspaceEdit,
) -> Result<WorkspaceEditResult, StoreError> {
    let proposal = ToolProposal::from(&edit);
    store.claim_tool_effect(approval_id, &proposal)?;
    let result = execute_edit(store, task_id, edit);
    match &result {
        Ok(value) => {
            let detail = format!("edited {}", value.path);
            store.settle_tool_effect(
                approval_id,
                true,
                ToolResult::from(value.clone()),
                Some(&detail),
            )?;
        }
        Err(cause) => {
            let message = cause.to_string();
            store.settle_tool_effect(
                approval_id,
                false,
                ToolResult::Failure {
                    message: message.clone(),
                },
                Some(&message),
            )?;
        }
    }
    result
}

pub fn run_project_shell(
    store: &DesktopStore,
    task_id: &str,
    approval_id: &str,
    shell: WorkspaceShell,
) -> Result<WorkspaceShellResult, StoreError> {
    let proposal = ToolProposal::from(&shell);
    store.claim_tool_effect(approval_id, &proposal)?;
    let result = execute_shell(store, task_id, shell);
    match &result {
        Ok(value) => {
            let succeeded = value.exit_code == 0;
            let detail = format!("exit {} in {} ms", value.exit_code, value.duration_ms);
            store.settle_tool_effect(
                approval_id,
                succeeded,
                ToolResult::from(value.clone()),
                Some(&detail),
            )?;
        }
        Err(cause) => {
            let message = cause.to_string();
            store.settle_tool_effect(
                approval_id,
                false,
                ToolResult::Failure {
                    message: message.clone(),
                },
                Some(&message),
            )?;
        }
    }
    result
}

pub(crate) fn execute_edit(
    store: &DesktopStore,
    task_id: &str,
    edit: WorkspaceEdit,
) -> Result<WorkspaceEditResult, StoreError> {
    if edit.old_text.is_empty() {
        return Err(error("an exact edit requires non-empty oldText"));
    }
    if edit.old_text.len() > 1024 * 1024 || edit.new_text.len() > 1024 * 1024 {
        return Err(error("an exact edit cannot exceed 1 MiB per text block"));
    }
    let root = project_root(store, task_id)?;
    let path = existing_scoped_path(&root, &edit.path)?;
    let original = std::fs::read_to_string(&path)
        .map_err(|cause| io_error("read UTF-8 file", &path, cause))?;
    let replacements = original.match_indices(&edit.old_text).count();
    if replacements != 1 {
        return Err(error(format!(
            "exact edit expected one oldText match but found {replacements}"
        )));
    }
    let updated = original.replacen(&edit.old_text, &edit.new_text, 1);
    std::fs::write(&path, updated).map_err(|cause| io_error("write", &path, cause))?;
    Ok(WorkspaceEditResult {
        path: display_relative(&root, &path)?,
        replacements,
        diff: format_diff(&edit.path, &edit.old_text, &edit.new_text),
    })
}

pub(crate) fn execute_shell(
    store: &DesktopStore,
    task_id: &str,
    shell: WorkspaceShell,
) -> Result<WorkspaceShellResult, StoreError> {
    let command_text = shell.command.trim();
    if command_text.is_empty() || command_text.len() > 64 * 1024 {
        return Err(error(
            "shell command must contain between 1 and 65536 bytes",
        ));
    }
    if !(1..=3_600).contains(&shell.timeout_seconds) {
        return Err(error("shell timeout must be between 1 and 3600 seconds"));
    }
    let root = project_root(store, task_id)?;
    let mut command = if cfg!(windows) {
        let mut command = Command::new("powershell.exe");
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command_text,
        ]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-lc", command_text]);
        command
    };
    command
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(sanitized_process_environment());
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|cause| error(format!("failed to start approved shell command: {cause}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| error("shell stdout was unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| error("shell stderr was unavailable"))?;
    let stdout_reader = std::thread::spawn(move || read_output(stdout));
    let stderr_reader = std::thread::spawn(move || read_output(stderr));
    let timeout = Duration::from_secs(shell.timeout_seconds);
    let status = loop {
        match child.try_wait().map_err(|cause| {
            error(format!(
                "failed to wait for approved shell command: {cause}"
            ))
        })? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(error(format!(
                    "approved shell command timed out after {} seconds",
                    shell.timeout_seconds
                )));
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| error("shell stdout reader failed"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| error("shell stderr reader failed"))??;
    let (stdout, stdout_truncated) = truncate_output(&stdout);
    let (stderr, stderr_truncated) = truncate_output(&stderr);
    Ok(WorkspaceShellResult {
        command: command_text.to_owned(),
        exit_code: status.code().unwrap_or(-1),
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        truncated: stdout_truncated || stderr_truncated,
    })
}

fn project_root(store: &DesktopStore, task_id: &str) -> Result<PathBuf, StoreError> {
    let root = PathBuf::from(store.project_for_task(task_id)?.root);
    root.canonicalize()
        .map_err(|cause| io_error("resolve Selected Project", &root, cause))
}

fn existing_scoped_path(root: &Path, relative: &str) -> Result<PathBuf, StoreError> {
    if relative.trim().is_empty() {
        return Err(error("a project-relative path is required"));
    }
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(error("the path must remain inside the Selected Project"));
    }
    let addressed = root.join(relative_path);
    let canonical = addressed
        .canonicalize()
        .map_err(|cause| io_error("resolve project path", &addressed, cause))?;
    if !canonical.starts_with(root) {
        return Err(error("the resolved path escapes the Selected Project"));
    }
    Ok(canonical)
}

fn visit_directory(
    root: &Path,
    directory: &Path,
    limit: usize,
    entries: &mut Vec<WorkspaceEntry>,
) -> Result<(), StoreError> {
    if entries.len() >= limit {
        return Ok(());
    }
    let children =
        std::fs::read_dir(directory).map_err(|cause| io_error("list", directory, cause))?;
    for child in children {
        let child = child.map_err(|cause| io_error("read directory entry", directory, cause))?;
        if ignored_name(&child.file_name()) {
            continue;
        }
        let path = child.path();
        let metadata = child
            .metadata()
            .map_err(|cause| io_error("inspect", &path, cause))?;
        let (kind, size_bytes) = if metadata.is_dir() {
            ("directory", 0)
        } else if metadata.is_file() {
            ("file", metadata.len())
        } else {
            continue;
        };
        entries.push(WorkspaceEntry {
            path: display_relative(root, &path)?,
            kind,
            size_bytes,
        });
        if entries.len() >= limit {
            return Ok(());
        }
        if metadata.is_dir() {
            visit_directory(root, &path, limit, entries)?;
        }
    }
    Ok(())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    limit: usize,
    files: &mut Vec<PathBuf>,
) -> Result<(), StoreError> {
    if files.len() >= limit {
        return Ok(());
    }
    let children =
        std::fs::read_dir(directory).map_err(|cause| io_error("list", directory, cause))?;
    for child in children {
        let child = child.map_err(|cause| io_error("read directory entry", directory, cause))?;
        if ignored_name(&child.file_name()) {
            continue;
        }
        let path = child.path();
        let metadata = child
            .metadata()
            .map_err(|cause| io_error("inspect", &path, cause))?;
        if metadata.is_dir() {
            collect_files(root, &path, limit, files)?;
        } else if metadata.is_file() {
            let canonical = path
                .canonicalize()
                .map_err(|cause| io_error("resolve project file", &path, cause))?;
            if !canonical.starts_with(root) {
                continue;
            }
            files.push(canonical);
        }
        if files.len() >= limit {
            break;
        }
    }
    Ok(())
}

fn ignored_name(name: &OsStr) -> bool {
    matches!(
        name.to_string_lossy().to_ascii_lowercase().as_str(),
        ".git" | "node_modules" | "target" | "dist" | ".next" | ".venv" | "__pycache__"
    )
}

fn display_relative(root: &Path, path: &Path) -> Result<String, StoreError> {
    path.strip_prefix(root)
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .map_err(|_| error("the path is outside the Selected Project"))
}

fn read_output(mut stream: impl Read) -> Result<Vec<u8>, StoreError> {
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|cause| error(format!("failed to capture shell output: {cause}")))?;
    Ok(bytes)
}

fn truncate_output(bytes: &[u8]) -> (String, bool) {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    let mut truncated = false;
    if text.len() > MAX_OUTPUT_BYTES {
        let start = text.len() - MAX_OUTPUT_BYTES;
        text = String::from_utf8_lossy(&text.as_bytes()[start..]).into_owned();
        truncated = true;
    }
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() > MAX_OUTPUT_LINES {
        text = lines[lines.len() - MAX_OUTPUT_LINES..].join("\n");
        truncated = true;
    }
    (text.trim_end().to_owned(), truncated)
}

fn truncate_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        value.to_owned()
    } else {
        format!("{}…", String::from_utf8_lossy(&value.as_bytes()[..limit]))
    }
}

fn format_diff(path: &str, old_text: &str, new_text: &str) -> String {
    let removed = old_text
        .lines()
        .map(|line| format!("-{line}"))
        .collect::<Vec<_>>();
    let added = new_text
        .lines()
        .map(|line| format!("+{line}"))
        .collect::<Vec<_>>();
    let mut lines = vec![format!("--- a/{path}"), format!("+++ b/{path}")];
    lines.extend(removed);
    lines.extend(added);
    truncate_text(&lines.join("\n"), 64 * 1024)
}

fn io_error(action: &str, path: &Path, cause: std::io::Error) -> StoreError {
    error(format!("failed to {action} {}: {cause}", path.display()))
}

fn error(message: impl Into<String>) -> StoreError {
    StoreError::message(message)
}
