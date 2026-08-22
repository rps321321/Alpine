use alpine_control_plane::Alpine;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

type JsonMutation = fn(&mut Value);

#[test]
fn rust_and_python_resolve_the_same_legacy_contract() {
    let directory = tempfile::tempdir().expect("temporary install");
    write_fixture(directory.path());
    let rust = Alpine::resolve_session(directory.path(), None, true).expect("Rust resolver");

    let script = r#"
import json
import sys
from pathlib import Path
from localmodel.config import resolve_session
r = resolve_session(Path(sys.argv[1]), require_runtime=True)
print(json.dumps({
    "profile_name": r.profile_name,
    "runtime_name": r.runtime_name,
    "server": str(r.server),
    "model": str(r.model),
    "mmproj": str(r.mmproj),
    "chat_template": str(r.chat_template),
    "api_key_file": str(r.api_key_file),
    "base_url_file": str(r.base_url_file),
    "state_file": str(r.state_file),
    "base_url": r.base_url,
    "profile": r.profile,
}))
"#;
    let output = Command::new("python")
        .args([
            "-c",
            script,
            directory.path().to_str().expect("Unicode path"),
        ])
        .output()
        .expect("run Python resolver");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let python: Value = serde_json::from_slice(&output.stdout).expect("Python JSON");

    assert_eq!(python["profile_name"], rust.profile_name);
    assert_eq!(python["runtime_name"], rust.runtime_name);
    assert_eq!(python["server"], rust.server.to_string_lossy().as_ref());
    assert_eq!(python["model"], rust.model.to_string_lossy().as_ref());
    assert_eq!(python["mmproj"], rust.mmproj.to_string_lossy().as_ref());
    assert_eq!(
        python["chat_template"],
        rust.chat_template.to_string_lossy().as_ref()
    );
    assert_eq!(
        python["api_key_file"],
        rust.api_key_file.to_string_lossy().as_ref()
    );
    assert_eq!(
        python["base_url_file"],
        rust.base_url_file.to_string_lossy().as_ref()
    );
    assert_eq!(
        python["state_file"],
        rust.state_file.to_string_lossy().as_ref()
    );
    assert_eq!(python["base_url"], rust.base_url);
    assert_eq!(
        python["profile"],
        serde_json::to_value(&rust.profile).unwrap()
    );
}

#[test]
fn retained_adapters_accept_supported_schema_four_with_an_explicit_profile() {
    let directory = tempfile::tempdir().expect("temporary install");
    write_fixture(directory.path());
    mutate_json(&directory.path().join("config/session.json"), |session| {
        session["schema"] = json!(4);
        session.as_object_mut().unwrap().remove("active_profile");
    });
    assert_all_resolve(directory.path());
}

#[test]
fn retained_adapters_reject_the_same_session_and_profile_boundaries() {
    let session_cases: Vec<JsonMutation> = vec![
        |session| session["host"] = json!("203.0.113.10"),
        |session| session["host"] = json!("Localhost"),
        |session| {
            let host = session.as_object_mut().unwrap().remove("host").unwrap();
            session["Host"] = host;
        },
        |session| session["schema"] = json!("3"),
        |session| session["model"] = json!(123),
        |session| session["runtimes"]["custom"] = json!(123),
        |session| session["cleanup"] = json!({"enabled": false, "start_script": "obsolete.ps1"}),
        |session| {
            session["schema"] = json!(5);
            session.as_object_mut().unwrap().remove("active_profile");
            session["cleanup"] = json!({"enabled": "false", "arguments": "--wrong-type"});
        },
    ];
    for mutation in session_cases {
        let directory = tempfile::tempdir().expect("temporary install");
        write_fixture(directory.path());
        mutate_json(&directory.path().join("config/session.json"), mutation);
        assert_all_reject(directory.path());
    }

    for (ngram_mod, reset) in [(true, true), (false, true)] {
        let directory = tempfile::tempdir().expect("temporary install");
        write_fixture(directory.path());
        mutate_json(
            &directory.path().join("profiles/stable-16k.json"),
            |profile| {
                profile["ngram_mod"] = json!(ngram_mod);
                profile["ngram_reset_on_begin"] = json!(reset);
            },
        );
        assert_all_reject(directory.path());
    }

    let directory = tempfile::tempdir().expect("temporary install");
    write_fixture(directory.path());
    mutate_json(
        &directory.path().join("profiles/stable-16k.json"),
        |profile| profile["context"] = json!(16.0),
    );
    assert_all_reject(directory.path());

    for (field, value) in [("runtime", "Official"), ("kv_cache", "Q8_0")] {
        let directory = tempfile::tempdir().expect("temporary install");
        write_fixture(directory.path());
        mutate_json(
            &directory.path().join("profiles/stable-16k.json"),
            |profile| profile[field] = json!(value),
        );
        assert_all_reject(directory.path());
    }
}

#[test]
fn published_powershell_adapter_uses_the_identity_bound_capability_contract() {
    let directory = tempfile::tempdir().expect("temporary install");
    let root = directory.path();
    write_fixture(root);
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime/scripts/lib.ps1"),
        root.join("scripts/lib.ps1"),
    )
    .unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("config/profile-capabilities.json"),
        root.join("config/profile-capabilities.json"),
    )
    .unwrap();
    let output = powershell_output(root, &root.join("scripts/lib.ps1"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn powershell_rejects_malformed_capability_records() {
    let cases = [
        (
            json!({
                "schema": 1,
                "maximum_threads": 256,
                "runtimes": {
                    "official": {"kv_cache": ["f16", "q8_0"], "request_local_ngram": "false"}
                }
            }),
            "request_local_ngram",
        ),
        (
            json!({
                "schema": 1,
                "maximum_threads": 256,
                "runtimes": {
                    "   ": {"kv_cache": ["f16"], "request_local_ngram": false}
                }
            }),
            "runtime name",
        ),
        (
            json!({
                "schema": 1,
                "maximum_threads": 256,
                "runtimes": {
                    "official": {"kv_cache": ["   "], "request_local_ngram": false}
                }
            }),
            "kv_cache",
        ),
    ];
    for (contract, message) in cases {
        let directory = tempfile::tempdir().expect("temporary contract");
        let path = directory.path().join("profile-capabilities.json");
        std::fs::write(&path, serde_json::to_vec(&contract).unwrap()).unwrap();
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                ". $env:ALPINE_TEST_MODULE; Read-ProfileCapabilityContract $env:ALPINE_TEST_CONTRACT | Out-Null",
            ])
            .env(
                "ALPINE_TEST_MODULE",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime/scripts/lib.ps1"),
            )
            .env("ALPINE_TEST_CONTRACT", path)
            .output()
            .expect("run PowerShell capability parser");
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(message),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn assert_all_resolve(root: &Path) {
    Alpine::resolve_session(root, Some("stable-16k"), true).expect("Rust resolver");
    assert!(adapter_output("python", root).status.success());
    let powershell = adapter_output("powershell", root);
    assert!(
        powershell.status.success(),
        "{}",
        String::from_utf8_lossy(&powershell.stderr)
    );
}

fn assert_all_reject(root: &Path) {
    assert!(Alpine::resolve_session(root, Some("stable-16k"), true).is_err());
    for adapter in ["python", "powershell"] {
        let output = adapter_output(adapter, root);
        assert!(
            !output.status.success(),
            "{adapter} unexpectedly accepted invalid config: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

fn adapter_output(adapter: &str, root: &Path) -> std::process::Output {
    match adapter {
        "python" => Command::new("python")
            .args([
                "-c",
                "import os; from pathlib import Path; from localmodel.config import resolve_session; resolve_session(Path(os.environ['ALPINE_TEST_ROOT']), 'stable-16k', require_runtime=True)",
            ])
            .env("ALPINE_TEST_ROOT", root)
            .output()
            .expect("run Python resolver"),
        "powershell" => powershell_output(
            root,
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime/scripts/lib.ps1"),
        ),
        _ => unreachable!(),
    }
}

fn powershell_output(root: &Path, module: &Path) -> std::process::Output {
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            ". $env:ALPINE_TEST_MODULE; Get-ResolvedSession -InstallRoot $env:ALPINE_TEST_ROOT -Name stable-16k -RequireRuntime | Out-Null",
        ])
        .env("ALPINE_TEST_ROOT", root)
        .env("ALPINE_TEST_MODULE", module)
        .output()
        .expect("run PowerShell resolver")
}

fn mutate_json(path: &Path, mutation: impl FnOnce(&mut Value)) {
    let mut value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    mutation(&mut value);
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

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
