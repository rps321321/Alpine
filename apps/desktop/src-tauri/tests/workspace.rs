use alpine_control_plane::sanitized_process_environment;
use alpine_desktop::store::{
    CreateExecution, CreateTask, DesktopStore, ExecutionId, NewExecutionSpecification,
    NewToolApproval,
};
use alpine_desktop::workspace::{
    WorkspaceEdit, WorkspaceShell, edit_project_file, list_project_files, read_project_file,
    run_project_shell, search_project_files,
};

fn execution_specification() -> NewExecutionSpecification {
    NewExecutionSpecification {
        model_registry_id: "model-workspace".to_owned(),
        model_repo_id: "Qwen/Qwen-GGUF".to_owned(),
        model_revision: Some("a".repeat(40)),
        model_filename: "model.gguf".to_owned(),
        model_sha256: "b".repeat(64),
        session_config_sha256: "c".repeat(64),
        profile_name: "stable-16k".to_owned(),
        profile_sha256: "d".repeat(64),
        runtime_name: "official".to_owned(),
        runtime_identity: "e".repeat(64),
        adapter_identity: "pi-agent-core@0.84.2".to_owned(),
        policy_identity: "alpine-desktop-project-tools-v1".to_owned(),
        context_window: 16_384,
        max_tokens: 2_048,
        temperature_millis: 200,
    }
}

fn setup() -> (tempfile::TempDir, DesktopStore, String, ExecutionId) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("selected-project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("README.md"), "alpha\nrun alpine-verify\nomega\n").unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    std::fs::create_dir(root.join("node_modules")).unwrap();
    std::fs::write(
        root.join("node_modules").join("ignored.js"),
        "alpine-verify\n",
    )
    .unwrap();
    let store = DesktopStore::open(temporary.path().join("desktop.sqlite3")).unwrap();
    let project = store.create_project("Selected", &root).unwrap();
    let task = store
        .create_task(CreateTask {
            project_id: project.id,
            title: "Inspect project".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .unwrap();
    let execution = store
        .create_execution(CreateExecution {
            task_id: task.id.clone(),
            specification: execution_specification(),
        })
        .unwrap();
    (temporary, store, task.id, execution.id)
}

#[test]
fn read_list_and_search_stay_inside_the_selected_project() {
    let (_temporary, store, task_id, _execution_id) = setup();

    let files = list_project_files(&store, &task_id, 100).unwrap();
    assert_eq!(
        files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec!["README.md", "src", "src/main.rs"]
    );
    let read = read_project_file(&store, &task_id, "README.md", Some(2), Some(1)).unwrap();
    assert_eq!(read.content, "run alpine-verify");
    assert_eq!(read.start_line, 2);
    let matches = search_project_files(&store, &task_id, "alpine-verify", 20).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].path, "README.md");
    assert_eq!(matches[0].line, 2);

    let traversal = read_project_file(&store, &task_id, "../outside.txt", None, None).unwrap_err();
    assert!(traversal.to_string().contains("Selected Project"));
}

#[test]
fn an_exact_approved_edit_returns_a_reviewable_diff() {
    let (temporary, store, task_id, execution_id) = setup();
    let edit = WorkspaceEdit {
        path: "README.md".to_owned(),
        old_text: "run alpine-verify".to_owned(),
        new_text: "run cargo test".to_owned(),
    };
    let proposal = serde_json::to_value(&edit).unwrap();
    let approval = store
        .request_tool_approval(NewToolApproval {
            task_id: task_id.clone(),
            execution_id: execution_id.clone(),
            tool_call_id: "edit-1".to_owned(),
            operation: "edit".to_owned(),
            proposal,
        })
        .unwrap();
    store.decide_tool_approval(&approval.id, true).unwrap();

    let result = edit_project_file(&store, &task_id, &approval.id, edit).unwrap();
    assert!(result.diff.contains("-run alpine-verify"));
    assert!(result.diff.contains("+run cargo test"));
    assert_eq!(
        std::fs::read_to_string(temporary.path().join("selected-project").join("README.md"))
            .unwrap(),
        "alpha\nrun cargo test\nomega\n"
    );
}

#[test]
fn an_exact_approved_shell_command_captures_output() {
    let (_temporary, store, task_id, execution_id) = setup();
    let shell = WorkspaceShell {
        command: if cfg!(windows) {
            "Write-Output ALPINE_SHELL_OK".to_owned()
        } else {
            "printf ALPINE_SHELL_OK".to_owned()
        },
        timeout_seconds: 10,
    };
    let approval = store
        .request_tool_approval(NewToolApproval {
            task_id: task_id.clone(),
            execution_id: execution_id.clone(),
            tool_call_id: "shell-1".to_owned(),
            operation: "shell".to_owned(),
            proposal: serde_json::to_value(&shell).unwrap(),
        })
        .unwrap();
    store.decide_tool_approval(&approval.id, true).unwrap();

    let result = run_project_shell(&store, &task_id, &approval.id, shell).unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("ALPINE_SHELL_OK"));
    assert!(result.stderr.is_empty());
}

#[test]
fn shared_child_environment_preserves_system_context_without_secret_carriers() {
    let environment = sanitized_process_environment();
    assert!(environment.iter().any(|(name, _)| {
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case("PATH")
    }));
    #[cfg(windows)]
    assert!(environment.iter().any(|(name, _)| {
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case("SystemRoot")
    }));
    for (name, _) in environment {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        assert!(
            [
                "TOKEN",
                "SECRET",
                "PASSWORD",
                "PASSWD",
                "API_KEY",
                "APIKEY",
                "ACCESS_KEY",
                "CREDENTIAL",
                "DATABASE_URL",
                "REDIS_URL",
                "MONGO_URI",
            ]
            .iter()
            .all(|marker| !upper.contains(marker)),
            "secret-shaped environment variable survived: {upper}"
        );
    }
}
