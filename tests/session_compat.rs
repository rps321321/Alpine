#![cfg(windows)]

use alpine_control_plane::Alpine;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

#[test]
fn rust_and_powershell_plan_identical_inference_arguments() {
    let directory = tempfile::tempdir().expect("temporary install");
    write_fixture(directory.path());
    let root = directory.path();
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));

    let script = r#"param([string]$InstallRoot, [string]$RepositoryRoot)
$ErrorActionPreference = 'Stop'
. (Join-Path $RepositoryRoot 'runtime/scripts/lib.ps1')
. (Join-Path $RepositoryRoot 'runtime/scripts/inference-session.ps1')
$resolved = Get-ResolvedSession -InstallRoot $InstallRoot -Name 'turbo-16k' -RequireRuntime
$cases = @(
    [pscustomobject]@{ vision=$false; fallback=$false; arguments=@(New-InferenceArguments $resolved.Session $resolved.Profile ([string]$resolved.ServerPath) $false) },
    [pscustomobject]@{ vision=$false; fallback=$true; arguments=@(New-InferenceArguments $resolved.Session $resolved.Profile ([string]$resolved.ServerPath) $false -Fallback) },
    [pscustomobject]@{ vision=$true; fallback=$false; arguments=@(New-InferenceArguments $resolved.Session $resolved.Profile ([string]$resolved.ServerPath) $true) },
    [pscustomobject]@{ vision=$true; fallback=$true; arguments=@(New-InferenceArguments $resolved.Session $resolved.Profile ([string]$resolved.ServerPath) $true -Fallback) }
)
$cases | ConvertTo-Json -Depth 8 -Compress
"#;
    let script_path = root.join("session-compat.ps1");
    std::fs::write(&script_path, script).unwrap();
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            script_path.to_str().expect("Unicode script path"),
            root.to_str().expect("Unicode fixture path"),
            repository.to_str().expect("Unicode repository path"),
        ])
        .output()
        .expect("run PowerShell planner");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let legacy: Vec<Value> = serde_json::from_slice(&output.stdout).expect("PowerShell JSON");
    assert_eq!(legacy.len(), 4);

    for case in legacy {
        let vision = case["vision"].as_bool().unwrap();
        let fallback = case["fallback"].as_bool().unwrap();
        let rust = Alpine::plan_session_arguments(root, Some("turbo-16k"), vision, fallback)
            .expect("Rust planner");
        let legacy_arguments = case["arguments"]
            .as_array()
            .and_then(|values| (values.len() == 1 && values[0].is_array()).then_some(&values[0]))
            .unwrap_or(&case["arguments"]);
        assert_eq!(*legacy_arguments, json!(rust.arguments));
    }
}

fn write_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::create_dir_all(root.join("profiles")).unwrap();
    std::fs::create_dir_all(root.join("runtime")).unwrap();
    let server = root.join("runtime/llama-server.exe");
    std::fs::write(&server, b"fixture").unwrap();
    std::fs::write(
        root.join("profiles/turbo-16k.json"),
        serde_json::to_vec(&json!({
            "name": "turbo-16k", "status": "candidate", "runtime": "custom",
            "context": 16384, "output": 4096, "parallel": 1, "threads": 16,
            "batch_size": 2048, "ubatch_size": 768, "kv_cache": "q8_0",
            "tensor_cpu_through_block": 43, "mtp_depth": 3, "ngram_mod": true,
            "ngram_reset_on_begin": true, "external_skills": false, "skill_tool": false,
            "vision_fit": true, "fit_target_mib": 512
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("config/session.json"),
        serde_json::to_vec(&json!({
            "schema": 3, "root": root, "host": "127.0.0.1", "port": 8123,
            "active_profile": "turbo-16k", "runtimes": {"custom": server},
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
