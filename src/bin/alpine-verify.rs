use regex::Regex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicBinaryAssetManifest {
    schema: u32,
    assets: Vec<PublicBinaryAsset>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicBinaryAsset {
    path: String,
    sha256: String,
    max_bytes: u64,
    provenance: String,
}

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
        _ => Err("usage: alpine-verify [audit]".to_owned()),
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
    let public_binary_assets = load_public_binary_assets(root)?;
    let mut reviewed_binary_assets = BTreeSet::new();
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
        if let Some(asset) = public_binary_assets.get(&slash) {
            verify_public_binary_asset(asset, &bytes)?;
            reviewed_binary_assets.insert(slash);
            continue;
        }
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
    let missing = public_binary_assets
        .keys()
        .filter(|path| !reviewed_binary_assets.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "reviewed public binary assets are missing from the proposed tree: {}",
            missing.join(", ")
        ));
    }
    println!("Public-tree audit passed: {checked} source files checked.");
    Ok(())
}

fn load_public_binary_assets(root: &Path) -> Result<BTreeMap<String, PublicBinaryAsset>, String> {
    let path = root.join("config/public-binary-assets.json");
    let manifest: PublicBinaryAssetManifest = serde_json::from_slice(
        &fs::read(&path).map_err(|error| format!("could not read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("invalid public binary asset manifest: {error}"))?;
    if manifest.schema != 1 {
        return Err(format!(
            "unsupported public binary asset manifest schema {}; expected 1",
            manifest.schema
        ));
    }

    let mut assets = BTreeMap::new();
    for asset in manifest.assets {
        let normalized = asset.path.replace('\\', "/");
        let extension = Path::new(&normalized)
            .extension()
            .and_then(OsStr::to_str)
            .map(str::to_ascii_lowercase);
        if !normalized.starts_with("apps/desktop/src/assets/")
            || normalized.contains("../")
            || !matches!(extension.as_deref(), Some("png" | "jpg" | "jpeg" | "webp"))
        {
            return Err(format!(
                "public binary asset must be a reviewed desktop raster path: {}",
                asset.path
            ));
        }
        if asset.max_bytes == 0 || asset.max_bytes > MAX_SOURCE_BYTES {
            return Err(format!(
                "public binary asset max_bytes must be between 1 and {MAX_SOURCE_BYTES}: {}",
                asset.path
            ));
        }
        if asset.provenance.trim().is_empty() {
            return Err(format!(
                "public binary asset requires provenance: {}",
                asset.path
            ));
        }
        if asset.sha256.len() != 64
            || !asset
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(format!(
                "public binary asset requires a lowercase SHA-256 digest: {}",
                asset.path
            ));
        }
        if assets.insert(normalized.clone(), asset).is_some() {
            return Err(format!(
                "public binary asset is listed more than once: {normalized}"
            ));
        }
    }
    Ok(assets)
}

fn verify_public_binary_asset(asset: &PublicBinaryAsset, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > asset.max_bytes {
        return Err(format!(
            "reviewed public binary asset exceeds its size bound: {}",
            asset.path
        ));
    }
    let actual = sha256_hex(bytes);
    if actual != asset.sha256 {
        return Err(format!(
            "reviewed public binary asset digest changed: {}",
            asset.path
        ));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut digest, byte| {
            write!(&mut digest, "{byte:02x}").expect("writing to a String cannot fail");
            digest
        })
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn reviewed_binary_asset_is_digest_and_size_bound() {
        let bytes = b"reviewed raster fixture";
        let asset = PublicBinaryAsset {
            path: "apps/desktop/src/assets/fixture.webp".to_owned(),
            sha256: sha256_hex(bytes),
            max_bytes: bytes.len() as u64,
            provenance: "test fixture".to_owned(),
        };
        assert!(verify_public_binary_asset(&asset, bytes).is_ok());

        let changed = b"changed raster fixture";
        assert!(verify_public_binary_asset(&asset, changed).is_err());

        let too_small = PublicBinaryAsset {
            max_bytes: 1,
            ..asset
        };
        assert!(verify_public_binary_asset(&too_small, bytes).is_err());
    }
}
