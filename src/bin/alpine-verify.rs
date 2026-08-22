use regex::Regex;
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("alpine-verify: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: Vec<std::ffi::OsString>) -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match arguments.as_slice() {
        [] => verify_repository(&root),
        [command] if command == "audit" => audit_public_tree(&root),
        [command, range] if command == "dco" => verify_dco(&root, range),
        _ => Err("usage: alpine-verify [audit | dco <base..head>]".to_owned()),
    }
}

fn verify_repository(root: &Path) -> Result<(), String> {
    audit_public_tree(root)?;
    run_stage(
        root,
        cargo(),
        &["fmt", "--all", "--", "--check"],
        "cargo fmt",
    )?;
    run_stage(
        root,
        cargo(),
        &[
            "clippy",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
        "cargo clippy",
    )?;
    run_stage(
        root,
        cargo(),
        &["test", "--all-targets", "--all-features"],
        "cargo test",
    )?;
    run_stage(
        root,
        OsStr::new("python"),
        &["-m", "unittest", "discover", "-s", "tests"],
        "legacy compatibility tests",
    )
}

fn cargo() -> &'static OsStr {
    OsStr::new("cargo")
}

fn run_stage(
    root: &Path,
    executable: &OsStr,
    arguments: &[&str],
    stage: &str,
) -> Result<(), String> {
    eprintln!("==> {stage}");
    let mut command = Command::new(executable);
    command.args(arguments).current_dir(root);
    // On Windows, `cargo run --bin alpine-verify` keeps the verifier executable
    // locked for this process's lifetime. Build verification targets in an
    // isolated directory so `cargo test --all-targets` never tries to relink
    // the executable that is currently running.
    if executable == cargo() {
        command.env("CARGO_TARGET_DIR", root.join("target/verification"));
    }
    let status = command
        .status()
        .map_err(|error| format!("failed to start {stage}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{stage} failed with exit code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

fn audit_public_tree(root: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .args([
            "-C",
            root.to_str()
                .ok_or_else(|| "repository root is not valid Unicode".to_owned())?,
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .map_err(|error| format!("could not enumerate the proposed public tree: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not enumerate the proposed public tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let proposed = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| {
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|error| format!("public-tree path is not UTF-8: {error}"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;

    for required in [
        "LICENSE",
        "NOTICE",
        "DCO",
        "THIRD_PARTY.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        "SUPPORT.md",
        "GOVERNANCE.md",
        "third_party/llama.cpp-LICENSE",
        ".github/workflows/verify.yml",
        ".github/pull_request_template.md",
    ] {
        if !root.join(required).is_file() {
            return Err(format!("public source contract is missing: {required}"));
        }
    }

    let forbidden_prefixes = [
        ".artifacts/",
        ".codex/",
        "build/",
        "dist/",
        "inventory/",
        "logs/",
        "models/",
        "results/",
        "runtime-official/",
        "runtime-custom/",
        "target/",
    ];
    let forbidden_extensions = [
        ".7z",
        ".bin",
        ".db",
        ".dll",
        ".env",
        ".exe",
        ".gguf",
        ".gz",
        ".jsonl",
        ".key",
        ".log",
        ".p12",
        ".pem",
        ".pfx",
        ".safetensors",
        ".sqlite",
        ".sqlite3",
        ".tar",
        ".zip",
    ];
    let home_path = home_path_regex()?;
    let private_key = Regex::new(r"-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----")
        .map_err(|error| format!("invalid private-key audit regex: {error}"))?;
    let aws_key = Regex::new(r"\bAKIA[0-9A-Z]{16}\b")
        .map_err(|error| format!("invalid AWS-key audit regex: {error}"))?;
    let github_token = Regex::new(r"\bgh[pousr]_[A-Za-z0-9_]{20,}\b")
        .map_err(|error| format!("invalid GitHub-token audit regex: {error}"))?;
    let mut checked = 0usize;
    for relative in proposed {
        let path = root.join(&relative);
        if !path.is_file() {
            continue;
        }
        checked += 1;
        let slash = relative.replace('\\', "/");
        let lower = slash.to_ascii_lowercase();
        if forbidden_prefixes
            .iter()
            .any(|prefix| lower.starts_with(prefix))
        {
            return Err(format!(
                "generated/private path is present in the proposed public tree: {relative}"
            ));
        }
        let extension = Path::new(&relative)
            .extension()
            .and_then(OsStr::to_str)
            .map(|value| format!(".{}", value.to_ascii_lowercase()));
        if extension
            .as_deref()
            .is_some_and(|value| forbidden_extensions.contains(&value))
        {
            return Err(format!(
                "binary/private artifact extension is present in the proposed public tree: {relative}"
            ));
        }
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("failed to inspect {relative}: {error}"))?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(format!(
                "unexpected source file larger than 2 MiB requires explicit release review: {relative}"
            ));
        }
        let bytes =
            fs::read(&path).map_err(|error| format!("failed to read {relative}: {error}"))?;
        if bytes.contains(&0) {
            return Err(format!(
                "unexpected binary content is present in the proposed public tree: {relative}"
            ));
        }
        let text = String::from_utf8_lossy(&bytes);
        if contains_disallowed_home_path(&text, &home_path) {
            return Err(format!(
                "personal Windows home path is present in the proposed public tree: {relative}"
            ));
        }
        if private_key.is_match(&text) {
            return Err(format!(
                "private-key material is present in the proposed public tree: {relative}"
            ));
        }
        if aws_key.is_match(&text) || github_token.is_match(&text) {
            return Err(format!(
                "provider credential shape is present in the proposed public tree: {relative}"
            ));
        }
    }
    println!("Public-tree audit passed: {checked} source files checked.");
    Ok(())
}

fn verify_dco(root: &Path, range: &OsStr) -> Result<(), String> {
    let commits = git_output(root, [OsStr::new("rev-list"), range])?;
    let trailer = dco_trailer_regex()?;
    let mut missing = Vec::new();
    for commit in commits.lines().filter(|line| !line.trim().is_empty()) {
        let message = git_output(
            root,
            [
                OsStr::new("show"),
                OsStr::new("-s"),
                OsStr::new("--format=%B"),
                OsStr::new(commit),
            ],
        )?;
        if !trailer.is_match(&message) {
            missing.push(commit.to_owned());
        }
    }
    if missing.is_empty() {
        println!("DCO verification passed.");
        Ok(())
    } else {
        Err(format!(
            "DCO Signed-off-by trailer missing from: {}",
            missing.join(", ")
        ))
    }
}

fn home_path_regex() -> Result<Regex, String> {
    Regex::new(r"(?i)C:\\Users\\([^\\\r\n]+)\\")
        .map_err(|error| format!("invalid home-path audit regex: {error}"))
}

fn contains_disallowed_home_path(text: &str, pattern: &Regex) -> bool {
    const ALLOWED: [&str; 5] = ["<you>", "private-user", "fixture", "Public", "Default"];
    pattern.captures_iter(text).any(|capture| {
        let user = capture
            .get(1)
            .map(|value| value.as_str())
            .unwrap_or_default();
        !ALLOWED
            .iter()
            .any(|allowed| user.eq_ignore_ascii_case(allowed))
    })
}

fn dco_trailer_regex() -> Result<Regex, String> {
    Regex::new(r"(?m)^Signed-off-by: .+ <[^>]+>\r?$")
        .map_err(|error| format!("invalid DCO regex: {error}"))
}

fn git_output<I, S>(root: &Path, arguments: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .map_err(|error| format!("failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    String::from_utf8(output.stdout).map_err(|error| format!("git output was not UTF-8: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dco_check_requires_a_complete_line_trailer() {
        let pattern = dco_trailer_regex().unwrap();
        assert!(pattern.is_match("subject\n\nSigned-off-by: Example <example@example.com>\n"));
        assert!(!pattern.is_match("subject Signed-off-by: Example <example@example.com>"));
        assert!(!pattern.is_match("Signed-off-by: Example"));
    }

    #[test]
    fn public_audit_allows_placeholders_but_rejects_personal_home_paths() {
        let pattern = home_path_regex().unwrap();
        assert!(!contains_disallowed_home_path(
            r"C:\Users\fixture\project",
            &pattern
        ));
        assert!(!contains_disallowed_home_path(
            r"C:\Users\Public\project",
            &pattern
        ));
        let personal = [r"C:\Users", "ARealPerson", "project"].join(r"\");
        assert!(contains_disallowed_home_path(&personal, &pattern));
    }
}
