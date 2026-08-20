use alpine_control_plane::Alpine;
use std::process::Command;

#[test]
fn rust_reads_identity_complete_python_evidence() {
    let directory = tempfile::tempdir().expect("temporary evidence directory");
    let database = directory.path().join("results.sqlite3");
    let script = r#"
import sys
from pathlib import Path
from localmodel.store import ResultStore

store = ResultStore(Path(sys.argv[1]))
try:
    store.create_run({
        "id": "python-run",
        "started_at": "2026-08-20T00:00:00Z",
        "status": "running",
        "kind": "micro",
        "profile": "stable-16k",
        "git_commit": "software",
        "hardware_manifest": "inventory/fixture.json",
        "model_sha256": "model",
        "backend_commit": "backend",
        "config": {
            "hardware": {"sha256": "hardware"},
            "launch": {"runtime_build_sha256": "runtime"},
            "benchmark": {"sha256": "workload"},
            "qualification_policy": {"sha256": "policy"},
        },
        "notes": "cross-language fixture",
    })
    store.add_sample("python-run", {
        "workload": "novel-256",
        "iteration": 0,
        "warmup": False,
        "generated_tokens": 1,
        "quality_pass": True,
    })
    store.finish_run(
        "python-run",
        "2026-08-20T00:01:00Z",
        "passed",
        {"all_quality_pass": True},
    )
finally:
    store.close()
"#;
    let output = Command::new("python")
        .args([
            "-c",
            script,
            database.to_str().expect("Unicode database path"),
        ])
        .output()
        .expect("create Python evidence fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let runs = Alpine::list_runs(&database, 10).expect("list Python-created evidence");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, "python-run");
    assert_eq!(runs[0].sample_count, 1);

    let evidence =
        Alpine::run_evidence(&database, "python-run").expect("read Python-created evidence");
    assert!(evidence.identity_complete);
    assert!(evidence.missing_identity_fields.is_empty());
    assert_eq!(evidence.identity.hardware.as_deref(), Some("hardware"));
    assert_eq!(evidence.identity.software.as_deref(), Some("software"));
    assert_eq!(evidence.identity.model.as_deref(), Some("model"));
    assert_eq!(evidence.identity.runtime.as_deref(), Some("runtime"));
    assert_eq!(evidence.identity.workload.as_deref(), Some("workload"));
    assert_eq!(evidence.identity.policy.as_deref(), Some("policy"));
    assert!(evidence.identity.configuration.is_some());
}
