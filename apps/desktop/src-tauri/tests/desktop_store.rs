use alpine_desktop::store::{
    ApprovalState, CreateTask, DesktopStore, MessageRole, ModelSource, NewTaskEvent,
    NewTaskMessage, NewToolApproval, RegisterModelArtifact, TaskStatus,
};
use std::path::Path;

#[test]
fn project_task_history_survives_reopen_with_ordered_events() {
    let temporary = tempfile::tempdir().unwrap();
    let project_root = temporary.path().join("selected-project");
    std::fs::create_dir(&project_root).unwrap();
    let database = temporary.path().join("desktop.sqlite3");

    let store = DesktopStore::open(&database).unwrap();
    let project = store
        .create_project("Alpine", &project_root)
        .expect("project should be created");
    let task = store
        .create_task(CreateTask {
            project_id: project.id.clone(),
            title: "Inspect the repository".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .expect("task should be created");

    store
        .append_message(NewTaskMessage {
            task_id: task.id.clone(),
            role: MessageRole::User,
            content: "Find the verifier.".to_owned(),
        })
        .unwrap();
    store
        .append_event(NewTaskEvent {
            task_id: task.id.clone(),
            kind: "tool.requested".to_owned(),
            payload: serde_json::json!({"tool": "search", "query": "alpine-verify"}),
        })
        .unwrap();
    store
        .append_message(NewTaskMessage {
            task_id: task.id.clone(),
            role: MessageRole::Assistant,
            content: "The verifier is a Rust binary.".to_owned(),
        })
        .unwrap();
    store
        .set_task_status(&task.id, TaskStatus::Completed, None)
        .unwrap();
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    let restored = reopened.load_task(&task.id).unwrap().unwrap();

    assert_eq!(restored.task.status, TaskStatus::Completed);
    assert_eq!(restored.messages.len(), 2);
    assert_eq!(restored.messages[0].sequence, 1);
    assert_eq!(restored.messages[1].sequence, 2);
    assert_eq!(restored.events.len(), 1);
    assert_eq!(restored.events[0].sequence, 1);
    assert_eq!(restored.events[0].payload["tool"], "search");
}

#[test]
fn model_registry_retains_exact_local_artifact_provenance() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("desktop.sqlite3");
    let artifact = temporary.path().join("Qwen-Q4_K_M.gguf");
    std::fs::write(&artifact, b"gguf fixture").unwrap();
    let store = DesktopStore::open(&database).unwrap();
    let registered = store
        .register_model_artifact(RegisterModelArtifact {
            source: ModelSource::HuggingFace,
            repo_id: Some("Qwen/Qwen-GGUF".to_owned()),
            revision: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            filename: "Qwen-Q4_K_M.gguf".to_owned(),
            local_path: artifact.to_string_lossy().into_owned(),
            observed_bytes: 12,
            sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            origin_url: Some("https://huggingface.co/Qwen/Qwen-GGUF".to_owned()),
        })
        .unwrap();
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    let models = reopened.list_model_artifacts().unwrap();
    assert_eq!(models, vec![registered]);
    assert_eq!(models[0].source, ModelSource::HuggingFace);
    assert_eq!(
        models[0].revision.as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
}

#[test]
fn tool_approval_is_exact_and_can_only_be_claimed_once() {
    let temporary = tempfile::tempdir().unwrap();
    let project_root = temporary.path().join("selected-project");
    std::fs::create_dir(&project_root).unwrap();
    let store = DesktopStore::open(temporary.path().join("desktop.sqlite3")).unwrap();
    let project = store.create_project("Alpine", &project_root).unwrap();
    let task = store
        .create_task(CreateTask {
            project_id: project.id,
            title: "Edit the readme".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .unwrap();
    let proposal = serde_json::json!({"path": "README.md", "oldText": "old", "newText": "new"});
    let pending = store
        .request_tool_approval(NewToolApproval {
            task_id: task.id.clone(),
            tool_call_id: "call-1".to_owned(),
            operation: "edit".to_owned(),
            proposal: proposal.clone(),
        })
        .unwrap();
    assert_eq!(pending.state, ApprovalState::Pending);

    let decision = store
        .decide_tool_approval_with_event(&pending.id, true)
        .unwrap();
    assert_eq!(decision.approval.state, ApprovalState::Approved);
    assert_eq!(decision.event.kind, "approval.decided");
    let persisted_events = store.load_task(&task.id).unwrap().unwrap().events;
    assert_eq!(persisted_events.len(), 1);
    assert_eq!(persisted_events[0].id, decision.event.id);
    store
        .claim_tool_approval(&pending.id, "edit", &proposal)
        .unwrap();
    assert!(
        store
            .claim_tool_approval(&pending.id, "edit", &proposal)
            .is_err()
    );
    assert!(
        store
            .claim_tool_approval(
                &pending.id,
                "edit",
                &serde_json::json!({"path": "README.md", "oldText": "different", "newText": "new"}),
            )
            .is_err()
    );
}

#[test]
fn project_roots_are_canonical_and_unique() {
    let temporary = tempfile::tempdir().unwrap();
    let project_root = temporary.path().join("selected-project");
    std::fs::create_dir(&project_root).unwrap();
    let store = DesktopStore::open(temporary.path().join("desktop.sqlite3")).unwrap();

    let created = store.create_project("Alpine", &project_root).unwrap();
    assert_eq!(
        Path::new(&created.root),
        project_root.canonicalize().unwrap()
    );

    let error = store
        .create_project("Duplicate", project_root.join("."))
        .unwrap_err();
    assert!(error.to_string().contains("already exists"));
}

#[test]
fn reopening_interrupts_unsettled_tasks_without_replaying_effects() {
    let temporary = tempfile::tempdir().unwrap();
    let project_root = temporary.path().join("selected-project");
    std::fs::create_dir(&project_root).unwrap();
    let database = temporary.path().join("desktop.sqlite3");
    let store = DesktopStore::open(&database).unwrap();
    let project = store.create_project("Alpine", &project_root).unwrap();
    let task = store
        .create_task(CreateTask {
            project_id: project.id,
            title: "Run tests".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .unwrap();
    store
        .set_task_status(&task.id, TaskStatus::Running, None)
        .unwrap();
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    let restored = reopened.load_task(&task.id).unwrap().unwrap();
    assert_eq!(restored.task.status, TaskStatus::Interrupted);
    assert_eq!(
        restored.task.error.as_deref(),
        Some("Alpine Desktop restarted while the task was active")
    );
}
