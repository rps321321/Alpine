use serde_json::Value;
use std::path::Path;
use std::process::Command;

fn alpine(root: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_alpine"))
        .args(arguments)
        .arg("--install-root")
        .arg(root)
        .arg("--compact")
        .output()
        .expect("run alpine")
}

#[test]
fn deployment_cli_initializes_and_records_an_incident_without_changing_roles() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::create_dir_all(root.join("profiles")).unwrap();
    std::fs::write(root.join("profiles/stable-16k.json"), b"{}\n").unwrap();

    let initial = alpine(root, &["deployment-status"]);
    assert!(initial.status.success());
    let initial: Value = serde_json::from_slice(&initial.stdout).unwrap();
    assert_eq!(initial["initialized"], false);

    let initialized = alpine(
        root,
        &[
            "deployment-init",
            "--daily-default",
            "stable-16k",
            "--rollback-profile",
            "stable-16k",
            "--operator",
            "test-operator",
            "--reason",
            "initialize conservative roles",
        ],
    );
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );

    let incident = alpine(
        root,
        &[
            "incident",
            "--profile",
            "stable-16k",
            "--operator",
            "test-operator",
            "--reason",
            "contradictory operational evidence",
        ],
    );
    assert!(
        incident.status.success(),
        "{}",
        String::from_utf8_lossy(&incident.stderr)
    );
    let incident: Value = serde_json::from_slice(&incident.stdout).unwrap();
    assert_eq!(incident["status"]["roles"]["daily_default"], "stable-16k");
    assert_eq!(
        incident["status"]["open_suspensions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
