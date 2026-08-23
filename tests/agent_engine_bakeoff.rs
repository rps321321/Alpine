use alpine_control_plane::Alpine;
use serde_json::Value;
use std::path::Path;
use std::process::Command;

#[test]
fn bakeoff_plan_pins_the_four_reviewed_candidate_seams_and_material_inputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let plan = root.join("config").join("agent-engine-bakeoff.json");

    let summary = Alpine::inspect_agent_engine_bakeoff_plan(&plan).unwrap();

    assert_eq!(summary.schema, 1);
    assert_eq!(summary.plan_id, "agent-engine-bakeoff-v1");
    assert_eq!(
        summary.candidate_ids,
        [
            "opencode-process",
            "pi-sdk-core",
            "pi-process-rpc",
            "cline-agents",
        ]
    );
    assert_eq!(summary.required_scenarios.len(), 11);
    assert!(summary.request_budget > 0);
    assert!(summary.max_event_queue > 0);
    assert_eq!(summary.recommendation, "no-go");

    let document: Value = serde_json::from_slice(&std::fs::read(&plan).unwrap()).unwrap();
    assert_eq!(document["inputs"]["profile"], "stable-16k");
    assert_eq!(document["inputs"]["model_id"], "local-qwen");
    assert_eq!(
        document["inputs"]["fixture"],
        "benchmarks/agent-engine-bakeoff/public-v1/task.json"
    );
}

#[test]
fn cli_runs_isolated_adapters_instead_of_importing_caller_authored_evidence() {
    let output = Command::new(env!("CARGO_BIN_EXE_alpine"))
        .args(["agent-engine-bakeoff", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--candidate-root"));
    assert!(help.contains("--install-root"));
    assert!(help.contains("--repository-root"));
    assert!(!help.contains("--evidence"));
}

#[test]
fn public_fixture_and_worker_are_repository_owned_and_prompt_safe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixture = root
        .join("benchmarks")
        .join("agent-engine-bakeoff")
        .join("public-v1")
        .join("task.json");
    let worker = root.join("scripts").join("agent-engine-bakeoff-worker.mjs");

    let fixture_document: Value = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
    assert_eq!(fixture_document["schema"], 1);
    assert_eq!(fixture_document["tool_policy"]["allowed"][0], "read");
    let worker_text = std::fs::read_to_string(worker).unwrap();
    assert!(worker_text.contains("runPiSdk"));
    assert!(worker_text.contains("runPiProcess"));
    assert!(worker_text.contains("runOpenCode"));
    assert!(worker_text.contains("runCline"));
    assert!(!worker_text.contains("process.stdout.write(error"));
}
