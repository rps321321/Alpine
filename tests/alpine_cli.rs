use alpine_control_plane::Decision;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn qualified_fixture_exits_successfully_and_reports_the_decision() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new(env!("CARGO_BIN_EXE_alpine"))
        .args([
            "qualify",
            "--request",
            root.join("tests/fixtures/alpine/qualified.json")
                .to_str()
                .expect("fixture path is Unicode"),
            "--compact",
        ])
        .output()
        .expect("run alpine");

    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).expect("valid report JSON");
    assert_eq!(
        serde_json::from_value::<Decision>(value["decision"].clone()).unwrap(),
        Decision::Qualified
    );
}
