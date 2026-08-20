use alpine_control_plane::Alpine;
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

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

fn write_fixture(root: &Path) {
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::create_dir_all(root.join("profiles")).unwrap();
    std::fs::create_dir_all(root.join("runtime")).unwrap();
    let server = root.join("runtime/llama-server.exe");
    std::fs::write(&server, b"fixture").unwrap();
    std::fs::write(
        root.join("profiles/stable-16k.json"),
        serde_json::to_vec(&json!({
            "name": "stable-16k", "status": "production", "runtime": "official",
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
