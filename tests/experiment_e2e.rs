use alpine_control_plane::{
    Alpine, Decision, EvidencePhase, ExternalEvidenceStatusKind, MicrobenchmarkOptions,
    OperatorReviewOptions, QualificationTarget, RunQualificationOptions, TuningDisposition,
    TuningOptions, runtime_bundle_sha256,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as FmtWrite;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use sysinfo::{Pid, System};

fn capability_scenario(id: &str, category: &str) -> Value {
    json!({
        "id": id,
        "category": category,
        "task": format!("exercise {category}"),
        "expected_capability": "the complete workflow behaves usefully and safely",
        "observed_behavior": "the complete workflow behaved as expected",
        "outcome": "pass",
        "limitations": [],
        "disposition": null,
        "disposition_rationale": null,
        "accepted_risk_ids": [],
        "supporting_artifact_sha256": null
    })
}

#[test]
fn rust_microbenchmark_writes_complete_identity_bound_evidence() {
    let directory = tempfile::tempdir().expect("fixture root");
    let repository = directory.path().join("repository");
    let install = directory.path().join("install");
    let results = directory.path().join("results");
    let (port, mut server, runtime, process_start_epoch_secs) = mock_server(directory.path(), 9);
    write_repository(&repository);
    write_install(
        &install,
        port,
        &runtime,
        server.id(),
        process_start_epoch_secs,
    );

    let mut options = MicrobenchmarkOptions {
        repository_root: repository.clone(),
        install_root: install.clone(),
        result_root: results.clone(),
        profile: "fixture".to_owned(),
        runs: 1,
        warmups: 0,
        workloads: vec!["fixture".to_owned()],
        notes: Some("integration fixture".to_owned()),
        phase: EvidencePhase::Tuning,
        deep_verify_artifacts: false,
        lease_timeout: Duration::from_secs(1),
        request_timeout: Duration::from_secs(5),
    };
    let tuning = Alpine::run_microbenchmark(&options).expect("Rust tuning benchmark");
    let tuning_candidate =
        Alpine::run_microbenchmark(&options).expect("Rust candidate tuning benchmark");
    options.phase = EvidencePhase::Final;
    options.deep_verify_artifacts = true;
    let report = Alpine::run_microbenchmark(&options).expect("Rust final benchmark");
    assert!(server.wait().expect("mock server").success());

    assert_eq!(report.status, "passed");
    assert_eq!(report.summary["all_quality_pass"], true);
    let database = results.join("results.sqlite3");
    let evidence = Alpine::run_evidence(&database, &report.run_id).expect("stored evidence");
    assert!(evidence.identity_complete);
    assert_eq!(evidence.summary.sample_count, 1);
    assert_eq!(evidence.summary.status, "passed");
    assert_eq!(evidence.config["evidence_phase"], "final");
    assert_eq!(
        evidence.hardware_manifest.as_deref(),
        Some("inline:rust-hardware-snapshot-v1")
    );
    assert_eq!(evidence.config["hardware"]["schema"], 2);
    assert_eq!(evidence.config["hardware"]["source"], "live-rust-probes");
    assert_eq!(
        evidence.config["hardware"]["sha256"].as_str().map(str::len),
        Some(64)
    );
    assert!(evidence.config["hardware"]["snapshot"].is_object());
    assert_eq!(
        evidence.config["launch"]["session_config_sha256"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    assert_eq!(
        evidence.config["model_verification"]["method"],
        "full-sha256"
    );
    let raw = results.join("runs").join(&report.run_id);
    assert!(raw.join("run.json").is_file());
    assert!(raw.join("samples.jsonl").is_file());
    assert!(raw.join("summary.json").is_file());

    let tuning_report = Alpine::tune(&TuningOptions {
        repository_root: repository.clone(),
        database: database.clone(),
        baseline_run_id: tuning.run_id.clone(),
        candidate_run_ids: vec![tuning_candidate.run_id],
    })
    .expect("bounded Rust tuning");
    assert_eq!(tuning_report.disposition, TuningDisposition::RetainBaseline);
    assert_eq!(tuning_report.selected_run_id, Some(tuning.run_id.clone()));

    let qualify_options = RunQualificationOptions {
        repository_root: repository,
        install_root: install,
        database: database.clone(),
        final_run_id: report.run_id.clone(),
        tuning_run_ids: vec![tuning.run_id],
        target: QualificationTarget::Candidate,
        support_timeout: Duration::from_secs(1),
    };
    let qualification =
        Alpine::qualify_run(&qualify_options).expect("database-backed qualification");
    assert_eq!(qualification.decision, Decision::Qualified);
    assert!(qualification.checks.iter().all(|check| check.passed));
    assert!(
        qualification
            .checks
            .iter()
            .any(|check| check.name == "current-model-digest" && check.passed)
    );
    assert!(
        qualification
            .checks
            .iter()
            .any(|check| { check.name == "current-live-hardware-identity" && check.passed })
    );

    let mut validated_options = qualify_options.clone();
    validated_options.target = QualificationTarget::Validated;
    let validated = Alpine::qualify_run(&validated_options).expect("validated gate");
    assert_eq!(validated.decision, Decision::NotProven);
    assert_eq!(
        validated.missing_external_evidence,
        vec!["operator-reviewed-capability-report"]
    );

    let evidence_options = OperatorReviewOptions {
        repository_root: qualify_options.repository_root.clone(),
        database: database.clone(),
        result_root: results.clone(),
        anchor_run_id: report.run_id.clone(),
        evidence: json!({
            "schema": 1,
            "reviewer_role": "integration-test-reviewer",
            "scenarios": [
                capability_scenario("trivial-conversation", "trivial-conversation"),
                capability_scenario("repository-orientation", "repository-orientation"),
                capability_scenario("diagnosis", "diagnosis"),
                capability_scenario("scoped-code-change", "scoped-code-change"),
                capability_scenario("longer-horizon-work", "longer-horizon-work"),
                capability_scenario("web-research", "web-research"),
                capability_scenario("permission-boundary", "permission-boundary"),
                capability_scenario("session-lifecycle", "session-lifecycle")
            ],
            "accepted_residual_risks": [],
            "final_decision": "approved",
            "final_rationale": "all required capability categories passed"
        }),
        reviewed_by: "integration-test-reviewer".to_owned(),
    };
    let recorded = Alpine::record_operator_review(&evidence_options)
        .expect("record immutable external evidence");
    let retried = Alpine::record_operator_review(&evidence_options)
        .expect("retry identical external evidence");
    assert_eq!(retried, recorded);
    let mut conflicting = evidence_options.clone();
    conflicting.evidence["final_decision"] = json!("rejected");
    assert!(Alpine::record_operator_review(&conflicting).is_err());

    let validated = Alpine::qualify_run(&validated_options).expect("satisfied validated gate");
    assert_eq!(validated.decision, Decision::Qualified);

    let original_artifact = std::fs::read(&recorded.path).unwrap();
    std::fs::write(&recorded.path, b"tampered").unwrap();
    let stale = Alpine::qualify_run(&validated_options).expect("tampered external evidence report");
    assert_eq!(stale.decision, Decision::NotProven);
    assert_eq!(
        stale.external_evidence[0].status,
        ExternalEvidenceStatusKind::Stale
    );
    std::fs::write(&recorded.path, original_artifact).unwrap();
    assert_eq!(
        Alpine::qualify_run(&validated_options)
            .expect("restored external evidence")
            .decision,
        Decision::Qualified
    );

    let connection = rusqlite::Connection::open(&database).unwrap();
    let original_config: String = connection
        .query_row(
            "SELECT config_json FROM runs WHERE id=?1",
            [&report.run_id],
            |row| row.get(0),
        )
        .unwrap();
    let mut altered_config: serde_json::Value = serde_json::from_str(&original_config).unwrap();
    let memory = altered_config["hardware"]["snapshot"]["physical_memory_bytes"]
        .as_u64()
        .unwrap();
    altered_config["hardware"]["snapshot"]["physical_memory_bytes"] = json!(memory + 1);
    connection
        .execute(
            "UPDATE runs SET config_json=?1 WHERE id=?2",
            rusqlite::params![
                serde_json::to_string(&altered_config).unwrap(),
                report.run_id
            ],
        )
        .unwrap();
    let stale_hardware =
        Alpine::qualify_run(&qualify_options).expect("tampered hardware identity report");
    assert_eq!(stale_hardware.decision, Decision::NotProven);
    assert!(
        stale_hardware
            .checks
            .iter()
            .any(|check| { check.name == "current-live-hardware-identity" && !check.passed })
    );
    connection
        .execute(
            "UPDATE runs SET config_json=?1 WHERE id=?2",
            rusqlite::params![original_config, report.run_id],
        )
        .unwrap();

    connection
        .execute(
            "UPDATE samples SET quality_pass=0 WHERE run_id=?1 AND warmup=0",
            [&report.run_id],
        )
        .unwrap();
    let tampered = Alpine::qualify_run(&qualify_options).expect("row-backed qualification");
    assert_eq!(tampered.decision, Decision::Unsupported);
    assert!(
        tampered
            .checks
            .iter()
            .any(|check| check.name == "fixture:quality" && !check.passed)
    );
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
    std::fs::write(
        root.join("config/promotion-policy.json"),
        serde_json::to_vec(&json!({
            "schema": 3,
            "lifecycle": ["experimental", "candidate", "validated", "production"],
            "gates": {
                "candidate": {
                    "required_workloads": ["fixture"],
                    "minimum_measured_samples_per_workload": 1,
                    "require_quality_pass": true,
                    "require_deterministic_outputs": true,
                    "performance_metric_by_workload": {"fixture": "decode-tps"},
                    "maximum_performance_coefficient_of_variation": 0.10,
                    "maximum_median_performance_regression_fraction": 0.10,
                    "minimum_tuning_selection_improvement_fraction": 0.03
                },
                "validated": {
                    "inherits": "candidate",
                    "requires_external_evidence": [
                        "operator-reviewed-capability-report"
                    ]
                },
                "production": {"inherits": "validated"}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("config/support-envelope.json"),
        serde_json::to_vec(&json!({
            "schema": 1,
            "id": "fixture",
            "platforms": [{
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH
            }],
            "required_probes": [],
            "optional_probes": []
        }))
        .unwrap(),
    )
    .unwrap();
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

fn write_install(root: &Path, port: u16, runtime: &Path, pid: u32, process_start_epoch_secs: u64) {
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::create_dir_all(root.join("profiles")).unwrap();
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("runtime")).unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
    let model = root.join("models/model.gguf");
    let profile = root.join("profiles/fixture.json");
    std::fs::write(&model, b"fixture model").unwrap();
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
            "schema": 2, "transaction_id": "fixture-session", "phase": "healthy",
            "pid": pid, "process_start_epoch_secs": process_start_epoch_secs,
            "profile": "fixture", "runtime": "official",
            "server": runtime, "server_sha256": sha256(&std::fs::read(runtime).unwrap()),
            "runtime_build_sha256": runtime_bundle_sha256(runtime).unwrap(),
            "profile_sha256": sha256(&std::fs::read(profile).unwrap()),
            "session_config_sha256": sha256(&std::fs::read(root.join("config/session.json")).unwrap()),
            "arguments": ["--fixture"], "environment": {}
        }))
        .unwrap(),
    )
    .unwrap();
}

fn mock_server(root: &Path, requests: u32) -> (u16, Child, PathBuf, u64) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let python = Command::new("python")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .expect("locate Python");
    assert!(python.status.success());
    let executable = PathBuf::from(String::from_utf8(python.stdout).unwrap().trim());
    let script = root.join("fixture_server.py");
    let ready = root.join("fixture-server.ready");
    std::fs::write(
        &script,
        r#"import argparse
import json
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('--port', type=int, required=True)
parser.add_argument('--ready', required=True)
parser.add_argument('--requests', type=int, required=True)
args = parser.parse_args()

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass
    def do_GET(self):
        body = b'{\"status\":\"ok\"}'
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_POST(self):
        length = int(self.headers.get('Content-Length', '0'))
        self.rfile.read(length)
        body = (b'data: {\"content\":\"ok\",\"stop\":false}\n\n'
                b'data: {\"content\":\"\",\"stop\":true,\"tokens_evaluated\":1,'
                b'\"tokens_predicted\":1,\"truncated\":false,\"stop_type\":\"limit\",'
                b'\"timings\":{\"prompt_n\":1,\"predicted_n\":1,\"prompt_per_second\":100.0,'
                b'\"predicted_per_second\":10.0,\"draft_n\":2,\"draft_n_accepted\":1}}\n\n')
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

server = HTTPServer(('127.0.0.1', args.port), Handler)
Path(args.ready).write_text('ready', encoding='utf-8')
for _ in range(args.requests):
    server.handle_request()
"#,
    )
    .unwrap();
    let mut child = Command::new(&executable)
        .arg(&script)
        .args(["--port", &port.to_string(), "--ready"])
        .arg(&ready)
        .args(["--requests", &requests.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start mock server");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready.is_file() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("mock server exited before readiness: {status}");
        }
        assert!(Instant::now() < deadline, "mock server readiness timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let process_start_epoch_secs = loop {
        let system = System::new_all();
        if let Some(process) = system.process(Pid::from_u32(child.id())) {
            break process.start_time();
        }
        assert!(Instant::now() < deadline, "mock server identity timed out");
        std::thread::sleep(Duration::from_millis(10));
    };
    (port, child, executable, process_start_epoch_secs)
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").unwrap();
    }
    encoded
}
