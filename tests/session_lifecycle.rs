#![cfg(windows)]

use alpine_control_plane::{Alpine, StartSessionOptions};
use serde_json::{Value, json};
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

#[test]
fn real_process_failure_is_bounded_recorded_and_leaves_no_listener() {
    let directory = tempfile::tempdir().expect("temporary install");
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve fixture port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    write_fixture(directory.path(), port);

    let error = Alpine::start_session(&StartSessionOptions {
        install_root: directory.path().to_path_buf(),
        profile: Some("fixture".to_owned()),
        vision: false,
        force_fallback: true,
        lock_timeout: Duration::from_secs(5),
        startup_timeout: Duration::ZERO,
    })
    .expect_err("the Alpine CLI is not a llama.cpp server");
    assert!(
        error.to_string().contains("exited before its identity")
            || error.to_string().contains("health verification")
    );

    let state: Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("logs/session-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(state["schema"], 2);
    assert_eq!(state["phase"], "failed");
    assert!(
        state["failed"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    let status = Alpine::session_status(directory.path(), Duration::from_secs(5)).unwrap();
    assert!(!status.active);
    assert!(!status.foreign);
}

fn write_fixture(root: &Path, port: u16) {
    for directory in ["config", "profiles", "models"] {
        std::fs::create_dir_all(root.join(directory)).unwrap();
    }
    let model = root.join("models/model.gguf");
    let mmproj = root.join("models/mmproj.gguf");
    let template = root.join("config/chat.jinja");
    for path in [&model, &mmproj, &template] {
        std::fs::write(path, b"fixture").unwrap();
    }
    let runtime = Path::new(env!("CARGO_BIN_EXE_alpine"));
    std::fs::write(
        root.join("profiles/fixture.json"),
        serde_json::to_vec(&json!({
            "name": "fixture", "status": "experimental", "runtime": "fixture",
            "context": 128, "output": 16, "parallel": 1, "threads": 1,
            "batch_size": 32, "ubatch_size": 16, "kv_cache": "q8_0",
            "tensor_cpu_through_block": 0, "mtp_depth": 1, "ngram_mod": false,
            "ngram_reset_on_begin": false, "external_skills": false,
            "skill_tool": false, "vision_fit": false, "fit_target_mib": 64
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("config/session.json"),
        serde_json::to_vec(&json!({
            "schema": 3, "root": root, "host": "127.0.0.1", "port": port,
            "active_profile": "fixture", "runtimes": {"fixture": runtime},
            "model": model, "mmproj": mmproj, "chat_template": template,
            "api_key_file": root.join("config/api-key.txt"),
            "base_url_file": root.join("config/base-url.txt"),
            "state_file": root.join("logs/session-state.json"),
            "cleanup": {"enabled": false}
        }))
        .unwrap(),
    )
    .unwrap();
}
