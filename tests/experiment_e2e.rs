use alpine_control_plane::{Alpine, MicrobenchmarkOptions};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fmt::Write as FmtWrite;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::thread;

#[test]
fn rust_microbenchmark_writes_complete_identity_bound_evidence() {
    let directory = tempfile::tempdir().expect("fixture root");
    let repository = directory.path().join("repository");
    let install = directory.path().join("install");
    let results = directory.path().join("results");
    let (port, server) = mock_server();
    write_repository(&repository);
    write_install(&install, port);

    let report = Alpine::run_microbenchmark(&MicrobenchmarkOptions {
        repository_root: repository,
        install_root: install,
        result_root: results.clone(),
        profile: "fixture".to_owned(),
        runs: 1,
        warmups: 0,
        workloads: vec!["fixture".to_owned()],
        notes: Some("integration fixture".to_owned()),
        deep_verify_artifacts: false,
    })
    .expect("Rust benchmark");
    server.join().expect("mock server");

    assert_eq!(report.status, "passed");
    assert_eq!(report.summary["all_quality_pass"], true);
    let database = results.join("results.sqlite3");
    let evidence = Alpine::run_evidence(&database, &report.run_id).expect("stored evidence");
    assert!(evidence.identity_complete);
    assert_eq!(evidence.summary.sample_count, 1);
    assert_eq!(evidence.summary.status, "passed");
    assert_eq!(
        evidence.config["model_verification"]["method"],
        "full-sha256"
    );
    let raw = results.join("runs").join(&report.run_id);
    assert!(raw.join("run.json").is_file());
    assert!(raw.join("samples.jsonl").is_file());
    assert!(raw.join("summary.json").is_file());
}

fn write_repository(root: &Path) {
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::create_dir_all(root.join("inventory")).unwrap();
    std::fs::create_dir_all(root.join("benchmarks/micro/prompts")).unwrap();
    let model = b"fixture model";
    std::fs::write(
        root.join("config/artifacts.json"),
        serde_json::to_vec(&json!({
            "model": {"sha256": sha256(model), "bytes": model.len()},
            "llama_cpp": {"commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("config/promotion-policy.json"), b"{\"schema\":1}").unwrap();
    std::fs::write(
        root.join("inventory/hardware-fixture.json"),
        b"{\"gpu\":\"fixture\"}",
    )
    .unwrap();
    std::fs::write(
        root.join("benchmarks/micro/workloads.json"),
        serde_json::to_vec(&json!({
            "schema": 2,
            "workloads": [{
                "id": "fixture", "prompt_file": "prompts/fixture.txt",
                "repeat": 1, "n_predict": 1, "quality": "nonempty"
            }]
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(root.join("benchmarks/micro/prompts/fixture.txt"), b"Hello").unwrap();

    let init = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(init.success());
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Alpine Test",
            "-c",
            "user.email=alpine@example.invalid",
            "commit",
            "--allow-empty",
            "--quiet",
            "-m",
            "fixture",
        ])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(commit.success());
}

fn write_install(root: &Path, port: u16) {
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::create_dir_all(root.join("profiles")).unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("runtime")).unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
    let model = root.join("models/model.gguf");
    let runtime = root.join("runtime/llama-server.exe");
    let profile = root.join("profiles/fixture.json");
    std::fs::write(&model, b"fixture model").unwrap();
    std::fs::write(&runtime, b"fixture runtime").unwrap();
    std::fs::write(root.join("config/api-key.txt"), b"fixture-secret").unwrap();
    std::fs::write(
        &profile,
        serde_json::to_vec(&json!({
            "name": "fixture", "status": "experimental", "runtime": "official",
            "context": 1024, "output": 16, "parallel": 1, "threads": 1,
            "batch_size": 32, "ubatch_size": 16, "kv_cache": "f16",
            "tensor_cpu_through_block": 0, "mtp_depth": 1, "ngram_mod": false,
            "ngram_reset_on_begin": false, "external_skills": false,
            "skill_tool": false, "vision_fit": false, "fit_target_mib": 1
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("config/session.json"),
        serde_json::to_vec(&json!({
            "schema": 3, "root": root, "host": "127.0.0.1", "port": port,
            "active_profile": "fixture", "runtimes": {"official": runtime},
            "model": model, "mmproj": root.join("models/mmproj.gguf"),
            "chat_template": root.join("config/chat.jinja"),
            "api_key_file": root.join("config/api-key.txt"),
            "base_url_file": root.join("config/base-url.txt"),
            "state_file": root.join("logs/session-state.json"), "cleanup": {"enabled": false}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("logs/session-state.json"),
        serde_json::to_vec(&json!({
            "phase": "healthy", "pid": 1, "profile": "fixture", "runtime": "official",
            "server": runtime, "server_sha256": sha256(b"fixture runtime"),
            "runtime_build_sha256": sha256(b"fixture build"),
            "profile_sha256": sha256(&std::fs::read(profile).unwrap()),
            "arguments": ["--fixture"], "environment": {}, "session_identity": "fixture-session"
        }))
        .unwrap(),
    )
    .unwrap();
}

fn mock_server() -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            respond(stream);
        }
    });
    (port, handle)
}

fn respond(mut stream: TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap();
        }
    }
    let mut request_body = vec![0; content_length];
    reader.read_exact(&mut request_body).unwrap();
    let body = if request_line.contains(" /health ") {
        "{\"status\":\"ok\"}".to_owned()
    } else {
        concat!(
            "data: {\"content\":\"ok\",\"stop\":false}\n\n",
            "data: {\"content\":\"\",\"stop\":true,\"tokens_evaluated\":1,",
            "\"tokens_predicted\":1,\"truncated\":false,\"stop_type\":\"limit\",",
            "\"timings\":{\"prompt_n\":1,\"predicted_n\":1,\"prompt_per_second\":100.0,",
            "\"predicted_per_second\":10.0,\"draft_n\":2,\"draft_n_accepted\":1}}\n\n"
        )
        .to_owned()
    };
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    stream.flush().unwrap();
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").unwrap();
    }
    encoded
}
