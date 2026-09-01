use alpine_desktop::store::{
    ApprovalState, CreateExecution, CreateTask, DesktopStore, ExecutionState, MessageRole,
    ModelSource, NewExecutionSpecification, NewTaskEvent, NewTaskMessage, NewToolApproval,
    RegisterModelArtifact, TaskSummary,
};
use rusqlite::Connection;
use std::path::Path;

fn specification(seed: char) -> NewExecutionSpecification {
    NewExecutionSpecification {
        model_registry_id: format!("model-{seed}"),
        model_repo_id: "Qwen/Qwen-GGUF".to_owned(),
        model_revision: Some(format!("{seed}").repeat(40)),
        model_filename: "Qwen-Q4_K_M.gguf".to_owned(),
        model_sha256: format!("{seed}").repeat(64),
        session_config_sha256: "b".repeat(64),
        profile_name: "stable-16k".to_owned(),
        profile_sha256: "c".repeat(64),
        runtime_name: "official".to_owned(),
        runtime_identity: "d".repeat(64),
        adapter_identity: "pi-agent-core@0.84.2".to_owned(),
        policy_identity: "alpine-desktop-project-tools-v1".to_owned(),
        context_window: 16_384,
        max_tokens: 2_048,
        temperature_millis: 200,
    }
}

fn setup() -> (tempfile::TempDir, DesktopStore, String) {
    let temporary = tempfile::tempdir().unwrap();
    let project_root = temporary.path().join("selected-project");
    std::fs::create_dir(&project_root).unwrap();
    let store = DesktopStore::open(temporary.path().join("desktop.sqlite3")).unwrap();
    let project = store.create_project("Alpine", &project_root).unwrap();
    let task = store
        .create_task(CreateTask {
            project_id: project.id,
            title: "Inspect the repository".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .unwrap();
    (temporary, store, task.id)
}

#[test]
fn multiple_executions_preserve_failure_retry_identity_and_ordered_history() {
    let (temporary, store, task_id) = setup();
    let first = store
        .create_execution(CreateExecution {
            task_id: task_id.clone(),
            specification: specification('a'),
        })
        .unwrap();
    store
        .transition_execution(first.id.as_str(), ExecutionState::Preparing, None)
        .unwrap();
    store
        .transition_execution(first.id.as_str(), ExecutionState::Running, None)
        .unwrap();
    store
        .append_message(NewTaskMessage {
            task_id: task_id.clone(),
            execution_id: first.id.clone(),
            role: MessageRole::Assistant,
            content: "The first attempt failed.".to_owned(),
        })
        .unwrap();
    store
        .append_event(NewTaskEvent {
            task_id: task_id.clone(),
            execution_id: first.id.clone(),
            kind: "execution.metrics".to_owned(),
            payload: serde_json::json!({"durationMs": 12}),
        })
        .unwrap();
    store
        .transition_execution(
            first.id.as_str(),
            ExecutionState::Failed,
            Some("provider stopped"),
        )
        .unwrap();

    let second = store
        .create_execution(CreateExecution {
            task_id: task_id.clone(),
            specification: specification('e'),
        })
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_ne!(first.execution_spec_id, second.execution_spec_id);
    store
        .transition_execution(second.id.as_str(), ExecutionState::Preparing, None)
        .unwrap();
    store
        .transition_execution(second.id.as_str(), ExecutionState::Running, None)
        .unwrap();
    store
        .transition_execution(second.id.as_str(), ExecutionState::Completed, None)
        .unwrap();
    assert!(
        store
            .transition_execution(second.id.as_str(), ExecutionState::Running, None)
            .is_err()
    );
    drop(store);

    let reopened = DesktopStore::open(temporary.path().join("desktop.sqlite3")).unwrap();
    let detail = reopened.load_task(&task_id).unwrap().unwrap();
    assert_eq!(detail.task.summary, TaskSummary::Done);
    assert_eq!(detail.task.active_execution_id, None);
    assert_eq!(detail.task.latest_execution_id.as_ref(), Some(&second.id));
    assert_eq!(detail.executions.len(), 2);
    assert_eq!(detail.executions[0].id, first.id);
    assert_eq!(detail.executions[0].state, ExecutionState::Failed);
    assert_eq!(
        detail.executions[0].failure.as_deref(),
        Some("provider stopped")
    );
    assert_eq!(detail.executions[1].id, second.id);
    assert_eq!(detail.executions[1].state, ExecutionState::Completed);
    assert_eq!(
        detail.executions[0].specification.model_sha256.as_deref(),
        Some(&"a".repeat(64))
    );
    assert_eq!(
        detail.executions[1].specification.model_sha256.as_deref(),
        Some(&"e".repeat(64))
    );
    assert_eq!(detail.messages[0].execution_id, first.id);
    assert_eq!(detail.events[0].execution_id, first.id);
}

#[test]
fn cancellation_is_a_validated_execution_lifecycle() {
    let (_temporary, store, task_id) = setup();
    let execution = store
        .create_execution(CreateExecution {
            task_id: task_id.clone(),
            specification: specification('a'),
        })
        .unwrap();
    store
        .transition_execution(execution.id.as_str(), ExecutionState::Preparing, None)
        .unwrap();
    store
        .transition_execution(execution.id.as_str(), ExecutionState::Cancelling, None)
        .unwrap();
    let cancelled = store
        .transition_execution(execution.id.as_str(), ExecutionState::Cancelled, None)
        .unwrap();
    assert!(cancelled.finished_at_ms.is_some());
    assert_eq!(
        store.load_task(&task_id).unwrap().unwrap().task.summary,
        TaskSummary::Ready
    );
    assert!(
        store
            .transition_execution(execution.id.as_str(), ExecutionState::Completed, None)
            .is_err()
    );
}

#[test]
fn restart_interrupts_the_exact_execution_and_its_pending_approval() {
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
    let execution = store
        .create_execution(CreateExecution {
            task_id: task.id.clone(),
            specification: specification('a'),
        })
        .unwrap();
    store
        .transition_execution(execution.id.as_str(), ExecutionState::Preparing, None)
        .unwrap();
    store
        .transition_execution(execution.id.as_str(), ExecutionState::Running, None)
        .unwrap();
    let approval = store
        .request_tool_approval(NewToolApproval {
            task_id: task.id.clone(),
            execution_id: execution.id.clone(),
            tool_call_id: "call-1".to_owned(),
            operation: "shell".to_owned(),
            proposal: serde_json::json!({"command": "cargo test", "timeoutSeconds": 30}),
        })
        .unwrap();
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    let restored = reopened
        .get_execution(execution.id.as_str())
        .unwrap()
        .unwrap();
    assert_eq!(restored.state, ExecutionState::Interrupted);
    assert!(
        restored
            .failure
            .as_deref()
            .is_some_and(|value| value.contains("restarted"))
    );
    assert_eq!(
        reopened
            .get_tool_approval(&approval.id)
            .unwrap()
            .unwrap()
            .state,
        ApprovalState::Interrupted
    );
    assert_eq!(
        reopened.load_task(&task.id).unwrap().unwrap().task.summary,
        TaskSummary::NeedsAttention
    );
}

#[test]
fn approvals_messages_and_events_are_bound_to_one_execution() {
    let (_temporary, store, task_id) = setup();
    let execution = store
        .create_execution(CreateExecution {
            task_id: task_id.clone(),
            specification: specification('a'),
        })
        .unwrap();
    let proposal = serde_json::json!({"path": "README.md", "oldText": "old", "newText": "new"});
    let pending = store
        .request_tool_approval(NewToolApproval {
            task_id: task_id.clone(),
            execution_id: execution.id.clone(),
            tool_call_id: "call-1".to_owned(),
            operation: "edit".to_owned(),
            proposal: proposal.clone(),
        })
        .unwrap();
    let decision = store
        .decide_tool_approval_with_event(&pending.id, true)
        .unwrap();
    assert_eq!(decision.approval.execution_id, execution.id);
    assert_eq!(decision.event.execution_id, execution.id);
    store
        .claim_tool_approval(&pending.id, "edit", &proposal)
        .unwrap();
    assert!(
        store
            .claim_tool_approval(&pending.id, "edit", &proposal)
            .is_err()
    );
}

#[test]
fn deleting_a_task_cascades_all_execution_owned_records() {
    let (_temporary, store, task_id) = setup();
    let execution = store
        .create_execution(CreateExecution {
            task_id: task_id.clone(),
            specification: specification('a'),
        })
        .unwrap();
    let approval = store
        .request_tool_approval(NewToolApproval {
            task_id: task_id.clone(),
            execution_id: execution.id.clone(),
            tool_call_id: "call-1".to_owned(),
            operation: "edit".to_owned(),
            proposal: serde_json::json!({"path": "README.md", "oldText": "a", "newText": "b"}),
        })
        .unwrap();
    store
        .append_message(NewTaskMessage {
            task_id: task_id.clone(),
            execution_id: execution.id.clone(),
            role: MessageRole::User,
            content: "Delete me".to_owned(),
        })
        .unwrap();
    store
        .append_event(NewTaskEvent {
            task_id: task_id.clone(),
            execution_id: execution.id.clone(),
            kind: "agent.started".to_owned(),
            payload: serde_json::json!({}),
        })
        .unwrap();

    store.delete_task(&task_id).unwrap();
    assert!(store.load_task(&task_id).unwrap().is_none());
    assert!(
        store
            .get_execution(execution.id.as_str())
            .unwrap()
            .is_none()
    );
    assert!(store.get_tool_approval(&approval.id).unwrap().is_none());
}

#[test]
fn schema_three_history_migrates_to_explicitly_unverified_synthetic_execution() {
    let temporary = tempfile::tempdir().unwrap();
    let database = temporary.path().join("desktop.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE desktop_schema (singleton INTEGER PRIMARY KEY, version INTEGER NOT NULL);
             INSERT INTO desktop_schema VALUES (1, 3);
             CREATE TABLE projects (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, root TEXT NOT NULL UNIQUE,
                created_at_ms INTEGER NOT NULL, last_opened_at_ms INTEGER NOT NULL
             );
             CREATE TABLE tasks (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                title TEXT NOT NULL, status TEXT NOT NULL,
                model_repo_id TEXT NOT NULL, model_filename TEXT NOT NULL, profile TEXT NOT NULL,
                error TEXT, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE task_messages (
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL, UNIQUE(task_id, sequence)
             );
             CREATE TABLE task_events (
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL, kind TEXT NOT NULL, payload_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL, UNIQUE(task_id, sequence)
             );
             CREATE TABLE tool_approvals (
                id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                tool_call_id TEXT NOT NULL, operation TEXT NOT NULL, proposal_json TEXT NOT NULL,
                state TEXT NOT NULL, detail TEXT, created_at_ms INTEGER NOT NULL,
                decided_at_ms INTEGER, settled_at_ms INTEGER, UNIQUE(task_id, tool_call_id)
             );
             CREATE INDEX tool_approvals_task_state ON tool_approvals(task_id, state, created_at_ms);
             CREATE TABLE model_artifacts (
                id TEXT PRIMARY KEY, source TEXT NOT NULL, repo_id TEXT, revision TEXT,
                filename TEXT NOT NULL, local_path TEXT NOT NULL UNIQUE,
                observed_bytes INTEGER NOT NULL, sha256 TEXT NOT NULL, origin_url TEXT,
                created_at_ms INTEGER NOT NULL, verified_at_ms INTEGER NOT NULL
             );
             INSERT INTO projects VALUES ('project-legacy','Legacy','C:/legacy',1,1);
             INSERT INTO tasks VALUES (
                'task-legacy','project-legacy','Legacy run','completed','legacy/repo',
                'legacy.gguf','stable-16k',NULL,2,3
             );
             INSERT INTO task_messages VALUES (
                'message-legacy','task-legacy',1,'assistant','legacy output',3
             );
             INSERT INTO task_events VALUES (
                'event-legacy','task-legacy',1,'agent.finished','{}',3
             );
             INSERT INTO tool_approvals VALUES (
                'approval-legacy','task-legacy','call-1','edit','{}','completed',NULL,3,3,3
             );",
        )
        .unwrap();
    drop(connection);

    let store = DesktopStore::open(&database).unwrap();
    let detail = store.load_task("task-legacy").unwrap().unwrap();
    assert_eq!(detail.executions.len(), 1);
    let execution = &detail.executions[0];
    assert_eq!(execution.id.as_str(), "legacy-execution-task-legacy");
    assert!(execution.specification.legacy_unverified);
    assert_eq!(
        execution.specification.runtime_identity,
        "legacy-unverified"
    );
    assert_eq!(detail.messages[0].execution_id, execution.id);
    assert_eq!(detail.events[0].execution_id, execution.id);
    assert_eq!(
        store
            .get_tool_approval("approval-legacy")
            .unwrap()
            .unwrap()
            .execution_id,
        execution.id
    );
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
            sha256: "a".repeat(64),
            origin_url: Some("https://huggingface.co/Qwen/Qwen-GGUF".to_owned()),
        })
        .unwrap();
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    assert_eq!(reopened.list_model_artifacts().unwrap(), vec![registered]);
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
    assert!(
        store
            .create_project("Duplicate", project_root.join("."))
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
}
