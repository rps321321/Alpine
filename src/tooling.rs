use crate::identity::sha256_file;
use crate::process::{resolve_executable, run_command_bounded};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct PackageRuntimeOptions {
    pub repository_root: PathBuf,
    pub built_runtime: PathBuf,
    pub output: PathBuf,
    pub cuda_bin: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageRuntimeReport {
    pub output: PathBuf,
    pub manifest: PathBuf,
    pub files: usize,
    pub server_version: String,
}

#[derive(Debug, Clone)]
pub struct BuildLauncherOptions {
    pub root: PathBuf,
    pub output: Option<PathBuf>,
    pub no_shortcut: bool,
    pub shortcut_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildLauncherReport {
    pub output: PathBuf,
    pub built: bool,
    pub shortcuts_updated: bool,
}

#[derive(Debug, Deserialize)]
struct Artifacts {
    llama_cpp: LlamaCpp,
}

#[derive(Debug, Deserialize)]
struct LlamaCpp {
    commit: String,
    patch: PathBuf,
    custom_build: CustomBuild,
}

#[derive(Debug, Deserialize)]
struct CustomBuild {
    cuda: String,
    architecture: String,
    options: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RuntimeManifest {
    schema: u32,
    llama_cpp_commit: String,
    source_patch_sha256: String,
    cuda_toolkit: String,
    cuda_architecture: String,
    cmake_options: Vec<String>,
    server_version: String,
    files: BTreeMap<String, RuntimeFile>,
}

#[derive(Debug, Serialize)]
struct RuntimeFile {
    bytes: u64,
    sha256: String,
}

pub fn package_runtime(options: &PackageRuntimeOptions) -> Result<PackageRuntimeReport, String> {
    let repository_root = directory(&options.repository_root, "repository root")?;
    let built_runtime = directory(&options.built_runtime, "built runtime")?;
    let cuda_bin = directory(&options.cuda_bin, "CUDA binary directory")?;
    std::fs::create_dir_all(&options.output)
        .map_err(|error| format!("failed to create runtime output: {error}"))?;
    let output = directory(&options.output, "runtime output")?;
    if output == built_runtime {
        return Err("built runtime and package output must be different directories".to_owned());
    }

    for entry in std::fs::read_dir(&built_runtime)
        .map_err(|error| format!("failed to enumerate built runtime: {error}"))?
    {
        let entry = entry.map_err(|error| format!("failed to inspect built runtime: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("failed to inspect runtime entry: {error}"))?
            .is_file()
        {
            copy_file(&entry.path(), &output.join(entry.file_name()))?;
        }
    }
    for name in ["cublas64_13.dll", "cublasLt64_13.dll", "cudart64_13.dll"] {
        let source = cuda_bin.join(name);
        if !source.is_file() {
            return Err(format!(
                "CUDA runtime dependency missing: {}",
                source.display()
            ));
        }
        copy_file(&source, &output.join(name))?;
    }

    let artifacts_path = repository_root.join("config/artifacts.json");
    let artifacts: Artifacts = serde_json::from_slice(
        &std::fs::read(&artifacts_path)
            .map_err(|error| format!("failed to read {}: {error}", artifacts_path.display()))?,
    )
    .map_err(|error| format!("invalid artifact contract: {error}"))?;
    let server = output.join("llama-server.exe");
    let version = native_text(&server, &["--version"], Duration::from_secs(30))?;
    let expected_short = artifacts
        .llama_cpp
        .commit
        .get(..7)
        .unwrap_or(&artifacts.llama_cpp.commit);
    if !version.contains(expected_short) {
        return Err(format!("unexpected llama-server build:\n{version}"));
    }

    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(&output)
        .map_err(|error| format!("failed to enumerate packaged runtime: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("failed to inspect packaged runtime: {error}"))?;
        if entry.file_name() == "build-manifest.json" {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("failed to inspect packaged file: {error}"))?;
        if metadata.is_file() {
            files.insert(
                entry.file_name().to_string_lossy().into_owned(),
                RuntimeFile {
                    bytes: metadata.len(),
                    sha256: sha256_file(&entry.path())?,
                },
            );
        }
    }
    let manifest = RuntimeManifest {
        schema: 1,
        llama_cpp_commit: artifacts.llama_cpp.commit,
        source_patch_sha256: sha256_file(&repository_root.join(artifacts.llama_cpp.patch))?,
        cuda_toolkit: artifacts.llama_cpp.custom_build.cuda,
        cuda_architecture: artifacts.llama_cpp.custom_build.architecture,
        cmake_options: artifacts.llama_cpp.custom_build.options,
        server_version: version.clone(),
        files,
    };
    let manifest_path = output.join("build-manifest.json");
    write_json(&manifest_path, &manifest)?;
    Ok(PackageRuntimeReport {
        output,
        manifest: manifest_path,
        files: manifest.files.len(),
        server_version: version,
    })
}

pub fn build_launcher(options: &BuildLauncherOptions) -> Result<BuildLauncherReport, String> {
    let root = directory(&options.root, "launcher root")?;
    let output = absolute_from(
        &root,
        options
            .output
            .as_deref()
            .unwrap_or(Path::new("Open Local Qwen.exe")),
    );
    if options.shortcut_only && options.no_shortcut {
        return Err("--shortcut-only and --no-shortcut cannot be combined".to_owned());
    }
    let mut built = false;
    if !options.shortcut_only {
        let source = root.join("launcher/OpenLocalQwen.cs");
        if !source.is_file() {
            return Err(format!("launcher source is missing: {}", source.display()));
        }
        let csc = resolve_csc()?;
        let temporary = tempfile::tempdir()
            .map_err(|error| format!("failed to create launcher build directory: {error}"))?;
        let staged = temporary.path().join("Open Local Qwen.exe");
        let out_arg = format!("/out:{}", staged.display());
        let compiler_source = native_command_path(&source);
        let source_arg = compiler_source.as_os_str();
        let mut command = Command::new(csc);
        command.args([
            OsStr::new("/nologo"),
            OsStr::new("/target:winexe"),
            OsStr::new("/optimize+"),
            OsStr::new("/reference:System.dll"),
            OsStr::new("/reference:System.Windows.Forms.dll"),
            OsStr::new("/reference:System.Drawing.dll"),
            OsStr::new(&out_arg),
            source_arg,
        ]);
        let result = run_command_bounded(&mut command, Duration::from_secs(120))
            .map_err(|error| format!("failed to run C# launcher compiler: {error}"))?;
        if result.timed_out || !result.status.success() {
            return Err(format!(
                "launcher compilation failed: {}{}",
                result.stdout.trim(),
                result.stderr.trim()
            ));
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create launcher output directory: {e}"))?;
        }
        copy_file(&staged, &output)?;
        copy_file(
            &root.join("launcher/Open Minimal OpenCode.cmd"),
            &output
                .parent()
                .unwrap_or(Path::new("."))
                .join("Open Minimal OpenCode.cmd"),
        )?;
        built = true;
    }
    if !output.is_file() {
        return Err(format!("launcher is missing: {}", output.display()));
    }
    let shortcuts_updated = if options.no_shortcut {
        false
    } else {
        create_shortcuts(&output)?;
        true
    };
    Ok(BuildLauncherReport {
        output,
        built,
        shortcuts_updated,
    })
}

fn create_shortcuts(target: &Path) -> Result<(), String> {
    let cscript =
        resolve_executable("cscript.exe").ok_or_else(|| "cscript.exe is unavailable".to_owned())?;
    let temporary = tempfile::Builder::new()
        .suffix(".js")
        .tempfile()
        .map_err(|error| format!("failed to create shortcut helper: {error}"))?;
    std::fs::write(temporary.path(), SHORTCUT_SCRIPT)
        .map_err(|error| format!("failed to write shortcut helper: {error}"))?;
    let mut command = Command::new(cscript);
    command.args([
        OsStr::new("//nologo"),
        temporary.path().as_os_str(),
        target.as_os_str(),
    ]);
    let result = run_command_bounded(&mut command, Duration::from_secs(30))
        .map_err(|error| format!("failed to create launcher shortcuts: {error}"))?;
    if result.timed_out || !result.status.success() {
        Err(format!(
            "shortcut creation failed: {}",
            result.stderr.trim()
        ))
    } else {
        Ok(())
    }
}

const SHORTCUT_SCRIPT: &str = r#"var s=new ActiveXObject("WScript.Shell");var d=s.SpecialFolders("Desktop");var t=WScript.Arguments(0);var x=[["Open Local Qwen.lnk","","Current deployment daily default"],["Open Local Qwen 32K.lnk","--profile fast-32k","Candidate general agent profile"],["Open Local Qwen 16K Stable.lnk","--profile stable-16k","Known-good rollback profile override"],["Open Local Qwen 16K Turbo.lnk","--profile turbo-16k","Repetitive-code candidate profile"],["Open Local Qwen 64K Long.lnk","--profile long-64k","Experimental long-context profile"],["Open Local Qwen Vision.lnk","--profile fast-32k --vision","Vision profile"]];for(var i=0;i<x.length;i++){var l=s.CreateShortcut(d+"\\"+x[i][0]);l.TargetPath=t;l.Arguments=x[i][1];l.WorkingDirectory=d;l.Description=x[i][2];l.IconLocation=t+",0";l.Save();}"#;

fn resolve_csc() -> Result<PathBuf, String> {
    if let Some(path) = resolve_executable("csc.exe") {
        return Ok(path);
    }
    let windows = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or_else(|| "SystemRoot is unavailable".to_owned())?;
    for relative in [
        "Microsoft.NET/Framework64/v4.0.30319/csc.exe",
        "Microsoft.NET/Framework/v4.0.30319/csc.exe",
    ] {
        let candidate = windows.join(relative);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("the .NET Framework C# compiler is unavailable".to_owned())
}

fn native_text(executable: &Path, arguments: &[&str], timeout: Duration) -> Result<String, String> {
    let mut command = Command::new(executable);
    command.args(arguments);
    let output = run_command_bounded(&mut command, timeout)
        .map_err(|error| format!("failed to run {}: {error}", executable.display()))?;
    if output.timed_out || !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            executable.display(),
            output.stderr.trim()
        ));
    }
    Ok(format!("{}{}", output.stdout, output.stderr)
        .trim()
        .to_owned())
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_file() {
        return Err(format!("required file is missing: {}", source.display()));
    }
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| {
            format!(
                "failed to copy {} to {}: {error}",
                source.display(),
                destination.display()
            )
        })
}

fn directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map_err(|error| format!("failed to resolve {label}: {error}"))
        .and_then(|path| {
            if path.is_dir() {
                Ok(path)
            } else {
                Err(format!("{label} is not a directory"))
            }
        })
}

fn absolute_from(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    }
}

#[cfg(windows)]
fn native_command_path(path: &Path) -> PathBuf {
    let value = path.as_os_str().to_string_lossy();
    value
        .strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_owned())
}

#[cfg(not(windows))]
fn native_command_path(path: &Path) -> PathBuf {
    path.to_owned()
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("failed to encode {}: {error}", path.display()))?;
    let mut bytes = bytes;
    bytes.push(b'\n');
    crate::session::atomic_replace(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_rejects_conflicting_shortcut_modes_before_side_effects() {
        let root = tempfile::tempdir().expect("temporary launcher root");
        let error = build_launcher(&BuildLauncherOptions {
            root: root.path().to_owned(),
            output: None,
            no_shortcut: true,
            shortcut_only: true,
        })
        .expect_err("conflicting modes must fail");
        assert!(error.contains("cannot be combined"));
    }

    #[test]
    fn relative_launcher_output_is_root_relative() {
        let root = Path::new(r"C:\fixture");
        assert_eq!(
            absolute_from(root, Path::new("launcher.exe")),
            root.join("launcher.exe")
        );
    }
}
