use alpine_desktop::store::{
    ApprovalState, CreateTask, DesktopStore, ExecutionId, ExecutionState, LegacyCausalOrder,
    LegacySource, ModelSource, NewExecutionSpecification, NewToolApproval, RegisterModelArtifact,
    TaskJournalEvent, TaskSummary, ToolProposal, ToolResult, ToolSettlementState,
};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

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

fn active_execution(store: &DesktopStore, task_id: &str, seed: char) -> ExecutionId {
    let execution = store
        .accept_prompt(task_id, "Run the task", specification(seed))
        .unwrap()
        .execution;
    store
        .record_execution_state(execution.id.as_str(), ExecutionState::Preparing)
        .unwrap();
    store
        .record_execution_state(execution.id.as_str(), ExecutionState::Running)
        .unwrap();
    execution.id
}

fn assert_contiguous_journal(store: &DesktopStore, task_id: &str) {
    let records = store.load_journal(task_id).unwrap();
    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.sequence, i64::try_from(index + 1).unwrap());
        assert_eq!(record.version, 1);
    }
}

#[test]
fn multiple_executions_preserve_failure_retry_identity_and_ordered_history() {
    let (temporary, store, task_id) = setup();
    let first = active_execution(&store, &task_id, 'a');
    store
        .record_assistant_message(first.as_str(), "The first attempt failed.")
        .unwrap();
    store
        .finish_execution(
            first.as_str(),
            ExecutionState::Failed,
            Some("provider stopped"),
            Some(12),
            Some(25),
        )
        .unwrap();

    let second = active_execution(&store, &task_id, 'e');
    assert_ne!(first, second);
    store
        .finish_execution(
            second.as_str(),
            ExecutionState::Completed,
            None,
            Some(20),
            Some(8),
        )
        .unwrap();
    assert!(
        store
            .record_execution_state(second.as_str(), ExecutionState::Running)
            .is_err()
    );
    assert_contiguous_journal(&store, &task_id);
    drop(store);

    let reopened = DesktopStore::open(temporary.path().join("desktop.sqlite3")).unwrap();
    let detail = reopened.load_task(&task_id).unwrap().unwrap();
    assert_eq!(detail.task.summary, TaskSummary::Done);
    assert_eq!(detail.task.active_execution_id, None);
    assert_eq!(detail.task.latest_execution_id.as_ref(), Some(&second));
    assert_eq!(detail.executions.len(), 2);
    assert_eq!(detail.executions[0].id, first);
    assert_eq!(detail.executions[0].state, ExecutionState::Failed);
    assert_eq!(
        detail.executions[0].failure.as_deref(),
        Some("provider stopped")
    );
    assert_eq!(detail.executions[1].id, second);
    assert_eq!(detail.executions[1].state, ExecutionState::Completed);
    assert_eq!(
        detail.executions[0].specification.model_sha256.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(
        detail.executions[1].specification.model_sha256.as_deref(),
        Some("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
    );
    assert!(
        detail
            .messages
            .iter()
            .any(|message| message.execution_id == first)
    );
    assert!(
        detail
            .messages
            .iter()
            .any(|message| message.execution_id == second)
    );
    assert!(
        detail
            .events
            .iter()
            .all(|record| record.execution_id.is_some())
    );
}

#[test]
fn cancellation_is_a_validated_execution_lifecycle() {
    let (_temporary, store, task_id) = setup();
    let execution = store
        .accept_prompt(&task_id, "Cancel this", specification('a'))
        .unwrap()
        .execution;
    store
        .record_execution_state(execution.id.as_str(), ExecutionState::Preparing)
        .unwrap();
    store
        .record_execution_state(execution.id.as_str(), ExecutionState::Cancelling)
        .unwrap();
    let cancelled = store
        .finish_execution(
            execution.id.as_str(),
            ExecutionState::Cancelled,
            None,
            Some(4),
            Some(0),
        )
        .unwrap()
        .execution;
    assert!(cancelled.finished_at_ms.is_some());
    assert_eq!(
        store.load_task(&task_id).unwrap().unwrap().task.summary,
        TaskSummary::Ready
    );
    assert!(
        store
            .finish_execution(
                execution.id.as_str(),
                ExecutionState::Completed,
                None,
                Some(5),
                Some(0),
            )
            .unwrap()
            .records
            .is_empty()
    );
}

#[test]
fn queued_cancellation_does_not_invent_a_start_timestamp() {
    let (_temporary, store, task_id) = setup();
    let execution = store
        .accept_prompt(&task_id, "Cancel before launch", specification('a'))
        .unwrap()
        .execution;

    let cancelled = store
        .finish_execution(
            execution.id.as_str(),
            ExecutionState::Cancelled,
            None,
            Some(0),
            Some(0),
        )
        .unwrap()
        .execution;

    assert_eq!(cancelled.started_at_ms, None);
    assert!(cancelled.finished_at_ms.is_some());
}

#[test]
fn crash_between_prompt_acceptance_and_provider_launch_is_explicitly_interrupted() {
    let temporary = tempfile::tempdir().unwrap();
    let project_root = temporary.path().join("selected-project");
    std::fs::create_dir(&project_root).unwrap();
    let database = temporary.path().join("desktop.sqlite3");
    let store = DesktopStore::open(&database).unwrap();
    let project = store.create_project("Alpine", &project_root).unwrap();
    let task = store
        .create_task(CreateTask {
            project_id: project.id,
            title: "Crash before provider".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .unwrap();
    let execution = store
        .accept_prompt(&task.id, "Persist me before launch", specification('a'))
        .unwrap()
        .execution;
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    let detail = reopened.load_task(&task.id).unwrap().unwrap();
    assert_eq!(detail.messages.len(), 1);
    assert_eq!(detail.messages[0].content, "Persist me before launch");
    assert_eq!(detail.executions[0].id, execution.id);
    assert_eq!(detail.executions[0].state, ExecutionState::Interrupted);
    assert!(matches!(
        detail.events[0].event,
        TaskJournalEvent::UserPromptAccepted { .. }
    ));
    assert!(matches!(
        detail.events[1].event,
        TaskJournalEvent::ExecutionQueued { .. }
    ));
    assert!(matches!(
        detail.events.last().unwrap().event,
        TaskJournalEvent::ExecutionFinished {
            outcome: alpine_desktop::store::ExecutionOutcome::Interrupted,
            ..
        }
    ));
    assert_contiguous_journal(&reopened, &task.id);
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
    let execution_id = active_execution(&store, &task.id, 'a');
    let approval = store
        .propose_tool(NewToolApproval {
            task_id: task.id.clone(),
            execution_id: execution_id.clone(),
            tool_call_id: "call-1".to_owned(),
            proposal: ToolProposal::Shell {
                command: "cargo test".to_owned(),
                timeout_seconds: 30,
            },
        })
        .unwrap()
        .approval;
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    let restored = reopened
        .get_execution(execution_id.as_str())
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
    let journal = reopened.load_journal(&task.id).unwrap();
    assert!(
        journal
            .iter()
            .any(|record| matches!(record.event, TaskJournalEvent::ApprovalInterrupted { .. }))
    );
    assert!(journal.iter().any(|record| matches!(
        record.event,
        TaskJournalEvent::ToolSettled {
            state: ToolSettlementState::Interrupted,
            ..
        }
    )));
}

#[test]
fn crash_during_tool_settlement_does_not_invent_a_result() {
    let temporary = tempfile::tempdir().unwrap();
    let project_root = temporary.path().join("selected-project");
    std::fs::create_dir(&project_root).unwrap();
    let database = temporary.path().join("desktop.sqlite3");
    let store = DesktopStore::open(&database).unwrap();
    let project = store.create_project("Alpine", &project_root).unwrap();
    let task = store
        .create_task(CreateTask {
            project_id: project.id,
            title: "Crash during tool".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .unwrap();
    let execution_id = active_execution(&store, &task.id, 'a');
    let proposal = ToolProposal::Edit {
        path: "README.md".to_owned(),
        old_text: "old".to_owned(),
        new_text: "new".to_owned(),
    };
    let approval = store
        .propose_tool(NewToolApproval {
            task_id: task.id.clone(),
            execution_id: execution_id.clone(),
            tool_call_id: "edit-crash".to_owned(),
            proposal: proposal.clone(),
        })
        .unwrap()
        .approval;
    store
        .decide_tool_approval_recorded(&approval.id, true)
        .unwrap();
    store.claim_tool_effect(&approval.id, &proposal).unwrap();
    drop(store);

    let reopened = DesktopStore::open(&database).unwrap();
    let journal = reopened.load_journal(&task.id).unwrap();
    assert!(
        journal
            .iter()
            .any(|record| matches!(record.event, TaskJournalEvent::ToolStarted { .. }))
    );
    assert!(
        !journal
            .iter()
            .any(|record| matches!(record.event, TaskJournalEvent::ToolResultRecorded { .. }))
    );
    assert!(journal.iter().any(|record| matches!(
        record.event,
        TaskJournalEvent::ToolSettled {
            state: ToolSettlementState::Interrupted,
            ..
        }
    )));
    assert_eq!(
        reopened
            .get_tool_approval(&approval.id)
            .unwrap()
            .unwrap()
            .state,
        ApprovalState::Interrupted
    );
    assert_eq!(
        reopened
            .get_execution(execution_id.as_str())
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Interrupted
    );
}

#[test]
fn approval_decision_and_execution_resume_share_one_journal_transaction() {
    let (_temporary, store, task_id) = setup();
    let execution_id = active_execution(&store, &task_id, 'a');
    let approval = store
        .propose_tool(NewToolApproval {
            task_id,
            execution_id: execution_id.clone(),
            tool_call_id: "call-atomic".to_owned(),
            proposal: ToolProposal::Shell {
                command: "cargo test".to_owned(),
                timeout_seconds: 30,
            },
        })
        .unwrap()
        .approval;

    let decision = store
        .decide_tool_approval_recorded(&approval.id, true)
        .unwrap();

    assert_eq!(decision.approval.state, ApprovalState::Approved);
    assert_eq!(decision.execution.state, ExecutionState::Running);
    assert_eq!(decision.records.len(), 2);
    assert!(matches!(
        decision.records[0].event,
        TaskJournalEvent::ApprovalDecided { approved: true, .. }
    ));
    assert!(matches!(
        decision.records[1].event,
        TaskJournalEvent::ExecutionResumed { .. }
    ));
    assert_eq!(
        decision.records[1].sequence,
        decision.records[0].sequence + 1
    );
}

#[test]
fn exact_tool_claim_is_bound_to_one_execution_and_one_typed_proposal() {
    let (_temporary, store, task_id) = setup();
    let execution_id = active_execution(&store, &task_id, 'a');
    let proposal = ToolProposal::Edit {
        path: "README.md".to_owned(),
        old_text: "old".to_owned(),
        new_text: "new".to_owned(),
    };
    let pending = store
        .propose_tool(NewToolApproval {
            task_id,
            execution_id: execution_id.clone(),
            tool_call_id: "call-1".to_owned(),
            proposal: proposal.clone(),
        })
        .unwrap()
        .approval;
    let decision = store
        .decide_tool_approval_recorded(&pending.id, true)
        .unwrap();
    assert_eq!(decision.approval.execution_id, execution_id);
    assert!(
        decision
            .records
            .iter()
            .all(|record| record.execution_id.as_ref() == Some(&execution_id))
    );
    store.claim_tool_effect(&pending.id, &proposal).unwrap();
    assert!(store.claim_tool_effect(&pending.id, &proposal).is_err());
}

#[test]
fn projection_rebuild_is_deterministic_for_transcript_summary_approvals_and_tools() {
    let (_temporary, store, task_id) = setup();
    let execution_id = active_execution(&store, &task_id, 'a');
    store
        .record_assistant_message(execution_id.as_str(), "I will edit README.")
        .unwrap();
    let proposal = ToolProposal::Edit {
        path: "README.md".to_owned(),
        old_text: "old".to_owned(),
        new_text: "new".to_owned(),
    };
    let approval = store
        .propose_tool(NewToolApproval {
            task_id: task_id.clone(),
            execution_id: execution_id.clone(),
            tool_call_id: "edit-1".to_owned(),
            proposal: proposal.clone(),
        })
        .unwrap()
        .approval;
    store
        .decide_tool_approval_recorded(&approval.id, true)
        .unwrap();
    store.claim_tool_effect(&approval.id, &proposal).unwrap();
    store
        .settle_tool_effect(
            &approval.id,
            true,
            ToolResult::Edit {
                path: "README.md".to_owned(),
                replacements: 1,
                diff: "-old\n+new".to_owned(),
            },
            Some("edited README.md"),
        )
        .unwrap();
    store
        .finish_execution(
            execution_id.as_str(),
            ExecutionState::Completed,
            None,
            Some(42),
            Some(18),
        )
        .unwrap();

    let before = store.load_task(&task_id).unwrap().unwrap();
    let before_events = before
        .events
        .iter()
        .map(|record| (record.sequence, record.version, record.event.clone()))
        .collect::<Vec<_>>();
    let before_messages = before
        .messages
        .iter()
        .map(|message| (message.sequence, message.role, message.content.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        store
            .get_tool_approval(&approval.id)
            .unwrap()
            .unwrap()
            .state,
        ApprovalState::Completed
    );

    store.rebuild_task_projections(&task_id).unwrap();

    let after = store.load_task(&task_id).unwrap().unwrap();
    assert_eq!(after.task.summary, TaskSummary::Done);
    assert_eq!(after.executions.len(), 1);
    assert_eq!(after.executions[0].state, ExecutionState::Completed);
    assert_eq!(
        after
            .messages
            .iter()
            .map(|message| (message.sequence, message.role, message.content.clone()))
            .collect::<Vec<_>>(),
        before_messages
    );
    assert_eq!(
        after
            .events
            .iter()
            .map(|record| (record.sequence, record.version, record.event.clone()))
            .collect::<Vec<_>>(),
        before_events
    );
    assert_eq!(
        store
            .get_tool_approval(&approval.id)
            .unwrap()
            .unwrap()
            .state,
        ApprovalState::Completed
    );
    assert!(store.list_pending_approvals(&task_id).unwrap().is_empty());
    assert!(after.events.iter().any(|record| matches!(
        &record.event,
        TaskJournalEvent::ToolResultRecorded {
            result: ToolResult::Edit { diff, .. },
            ..
        } if diff == "-old\n+new"
    )));
    assert!(after.events.iter().any(|record| matches!(
        record.event,
        TaskJournalEvent::ExecutionFinished {
            outcome: alpine_desktop::store::ExecutionOutcome::Completed,
            ..
        }
    )));
}

#[test]
fn deleting_a_task_cascades_journal_and_all_execution_projections() {
    let (_temporary, store, task_id) = setup();
    let execution_id = active_execution(&store, &task_id, 'a');
    let approval = store
        .propose_tool(NewToolApproval {
            task_id: task_id.clone(),
            execution_id: execution_id.clone(),
            tool_call_id: "call-1".to_owned(),
            proposal: ToolProposal::Edit {
                path: "README.md".to_owned(),
                old_text: "a".to_owned(),
                new_text: "b".to_owned(),
            },
        })
        .unwrap()
        .approval;

    store.delete_task(&task_id).unwrap();
    assert!(store.load_task(&task_id).unwrap().is_none());
    assert!(
        store
            .get_execution(execution_id.as_str())
            .unwrap()
            .is_none()
    );
    assert!(store.get_tool_approval(&approval.id).unwrap().is_none());
    assert!(store.load_journal(&task_id).unwrap().is_empty());
}

#[test]
fn schema_three_history_migrates_with_explicit_unverified_legacy_provenance() {
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
    assert_eq!(execution.finished_at_ms, Some(3));
    assert_eq!(
        execution.specification.runtime_identity,
        "legacy-unverified"
    );
    assert_eq!(detail.messages[0].execution_id, execution.id);
    assert_eq!(detail.events.len(), 4);
    assert!(detail.events.iter().all(|record| matches!(
        record.event,
        TaskJournalEvent::LegacyImported {
            causal_order: LegacyCausalOrder::Unverified,
            ..
        }
    )));
    assert!(detail.events.iter().any(|record| matches!(
        record.event,
        TaskJournalEvent::LegacyImported {
            source: LegacySource::Message,
            source_sequence: Some(1),
            ..
        }
    )));
    assert!(detail.events.iter().any(|record| matches!(
        record.event,
        TaskJournalEvent::LegacyImported {
            source: LegacySource::Event,
            source_sequence: Some(1),
            ..
        }
    )));
    assert_eq!(
        store
            .get_tool_approval("approval-legacy")
            .unwrap()
            .unwrap()
            .execution_id,
        execution.id
    );
    drop(store);

    let connection = Connection::open(&database).unwrap();
    let old_event_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'task_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_event_tables, 0);
}

fn completed_database() -> (tempfile::TempDir, PathBuf, String, ExecutionId) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("selected-project");
    std::fs::create_dir(&root).unwrap();
    let database = temporary.path().join("desktop.sqlite3");
    let store = DesktopStore::open(&database).unwrap();
    let project = store.create_project("Alpine", &root).unwrap();
    let task = store
        .create_task(CreateTask {
            project_id: project.id,
            title: "Contract boundary".to_owned(),
            model_repo_id: "local/alpine-install".to_owned(),
            model_filename: "model.gguf".to_owned(),
            profile: "stable-16k".to_owned(),
        })
        .unwrap();
    let execution_id = active_execution(&store, &task.id, 'a');
    store
        .finish_execution(
            execution_id.as_str(),
            ExecutionState::Completed,
            None,
            Some(1),
            Some(1),
        )
        .unwrap();
    drop(store);
    (temporary, database, task.id, execution_id)
}

fn inject_journal_row(
    database: &Path,
    task_id: &str,
    execution_id: &ExecutionId,
    id: &str,
    version: i64,
    event_json: &str,
) {
    let connection = Connection::open(database).unwrap();
    let sequence: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM task_journal WHERE task_id = ?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_journal
             (id, task_id, execution_id, sequence, version, event_json, occurred_at_ms, source_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 999, NULL)",
            params![
                id,
                task_id,
                execution_id.as_str(),
                sequence,
                version,
                event_json
            ],
        )
        .unwrap();
}

#[test]
fn persistence_boundary_rejects_unsupported_journal_versions() {
    let (_temporary, database, task_id, execution_id) = completed_database();
    inject_journal_row(
        &database,
        &task_id,
        &execution_id,
        "unsupported-version",
        2,
        r#"{"type":"execution-preparing"}"#,
    );
    let store = DesktopStore::open(&database).unwrap();
    assert!(store.load_journal(&task_id).is_err());
}

#[test]
fn persistence_boundary_rejects_unknown_fields_in_typed_events() {
    let (_temporary, database, task_id, execution_id) = completed_database();
    inject_journal_row(
        &database,
        &task_id,
        &execution_id,
        "unknown-field",
        1,
        r#"{"type":"execution-preparing","unexpected":true}"#,
    );
    let store = DesktopStore::open(&database).unwrap();
    assert!(store.load_journal(&task_id).is_err());
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
