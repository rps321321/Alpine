use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    let requested = Path::new(name);
    if requested.components().count() > 1 {
        return requested.is_file().then(|| requested.to_path_buf());
    }

    let extensions: Vec<OsString> = if cfg!(windows) && requested.extension().is_none() {
        std::env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|value| !value.is_empty())
                    .map(OsString::from)
                    .collect()
            })
            .unwrap_or_else(|| {
                [".COM", ".EXE", ".BAT", ".CMD"]
                    .map(OsString::from)
                    .to_vec()
            })
    } else {
        vec![OsString::new()]
    };

    std::env::var_os("PATH").and_then(|value| {
        std::env::split_paths(&value).find_map(|directory| {
            extensions.iter().find_map(|extension| {
                let mut candidate_name = OsString::from(name);
                candidate_name.push(extension);
                let candidate = directory.join(candidate_name);
                candidate.is_file().then_some(candidate)
            })
        })
    })
}

pub fn run_bounded(
    executable: &Path,
    arguments: &[&OsStr],
    timeout: Duration,
) -> io::Result<ProcessOutput> {
    let mut command = Command::new(executable);
    command.args(arguments);
    run_command_bounded(&mut command, timeout)
}

pub(crate) fn run_command_bounded(
    command: &mut Command,
    timeout: Duration,
) -> io::Result<ProcessOutput> {
    let stdout = tempfile::tempfile()?;
    let stderr = tempfile::tempfile()?;
    let mut stdout_reader = stdout.try_clone()?;
    let mut stderr_reader = stderr.try_clone()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()?;
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.kill()?;
            break (child.wait()?, true);
        }
        thread::sleep(Duration::from_millis(20));
    };

    Ok(ProcessOutput {
        status,
        stdout: read_text(&mut stdout_reader)?,
        stderr: read_text(&mut stderr_reader)?,
        timed_out,
    })
}

fn read_text(file: &mut File) -> io::Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_current_rust_compiler() {
        assert!(resolve_executable("rustc").is_some());
    }

    #[test]
    fn bounded_runner_captures_without_pipes() {
        let rustc = resolve_executable("rustc").expect("rustc on PATH");
        let result = run_bounded(&rustc, &[OsStr::new("--version")], Duration::from_secs(5))
            .expect("rustc probe succeeds");
        assert!(result.status.success());
        assert!(!result.timed_out);
        assert!(result.stdout.starts_with("rustc "));
    }
}
