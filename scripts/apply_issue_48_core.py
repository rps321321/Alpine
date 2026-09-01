from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def write(path, value):
    (ROOT / path).write_text(value, encoding="utf-8")


def replace_once(value, old, new, label):
    count = value.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one literal match, found {count}")
    return value.replace(old, new, 1)


def sub_once(value, pattern, replacement, label):
    updated, count = re.subn(pattern, replacement, value, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{label}: expected one regex match, found {count}")
    return updated


def patch_store():
    path = "apps/desktop/src-tauri/src/store.rs"
    value = read(path)
    value = replace_once(
        value,
        "//! Alpine-owned durable state for Projects, Tasks, Executions, Messages, and Events.\n\nmod execution;\npub use execution::{\n    CreateExecution, Execution, ExecutionId, ExecutionSpecification, ExecutionState,\n    NewExecutionSpecification, TaskSummary,\n};\n",
        "//! Alpine-owned durable state with a typed append-only Task journal.\n\nmod execution;\nmod journal;\npub use execution::{\n    Execution, ExecutionId, ExecutionSpecification, ExecutionState, NewExecutionSpecification,\n    TaskSummary,\n};\npub use journal::{\n    ExecutionOutcome, ExecutionTransitionOutcome, LegacyCausalOrder, LegacySource,\n    TASK_JOURNAL_VERSION, TaskJournalEvent, ToolOperation, ToolProposal, ToolResult,\n    ToolSettlementState, UserDirection,\n};\n",
        "store module exports",
    )
    value = value.replace(
        "use rusqlite::{Connection, OptionalExtension, Transaction, params};",
        "use rusqlite::{Connection, OptionalExtension, params};",
    )
    value = value.replace("use serde_json::{Value, json};", "use serde_json::Value;")
    value = value.replace("const SCHEMA_VERSION: i64 = 4;", "const SCHEMA_VERSION: i64 = 5;")

    value = sub_once(
        value,
        r"#\[derive\(Clone, Debug, Deserialize, Serialize\)\]\n#\[serde\(rename_all = \"camelCase\"\)\]\npub struct TaskMessage \{.*?\n\}\n\n#\[derive\(Clone, Debug, Deserialize\)\]\n#\[serde\(rename_all = \"camelCase\", deny_unknown_fields\)\]\npub struct NewTaskMessage \{.*?\n\}\n\n#\[derive\(Clone, Debug, Deserialize, Serialize\)\]\n#\[serde\(rename_all = \"camelCase\"\)\]\npub struct TaskEvent \{.*?\n\}\n\n#\[derive\(Clone, Debug, Deserialize\)\]\n#\[serde\(rename_all = \"camelCase\", deny_unknown_fields\)\]\npub struct NewTaskEvent \{.*?\n\}",
        '''#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMessage {
    pub id: String,
    pub task_id: String,
    pub execution_id: ExecutionId,
    pub sequence: i64,
    pub role: MessageRole,
    pub content: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub id: String,
    pub task_id: String,
    pub execution_id: Option<ExecutionId>,
    pub sequence: i64,
    pub version: u16,
    pub event: TaskJournalEvent,
    pub created_at_ms: i64,
}''',
        "message and journal record types",
    )

    value = sub_once(
        value,
        r"#\[derive\(Clone, Debug, Serialize\)\]\n#\[serde\(rename_all = \"camelCase\"\)\]\npub struct ToolApprovalDecision \{.*?\n\}\n\n#\[derive\(Clone, Debug, Deserialize\)\]\n#\[serde\(rename_all = \"camelCase\", deny_unknown_fields\)\]\npub struct NewToolApproval \{.*?\n\}",
        '''#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolApprovalDecision {
    pub approval: ToolApproval,
    pub execution: Execution,
    pub records: Vec<TaskEvent>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewToolApproval {
    pub task_id: String,
    pub execution_id: ExecutionId,
    pub tool_call_id: String,
    pub proposal: ToolProposal,
}''',
        "typed Tool Approval input",
    )

    value = replace_once(
        value,
        '''        let connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&connection)?;
        execution::interrupt_unsettled(&connection, RESTART_ERROR)?;
        connection.execute(
            "UPDATE tool_approvals
             SET state = 'interrupted', detail = ?1, settled_at_ms = ?2
             WHERE state IN ('pending', 'approved', 'executing')",
            params![RESTART_ERROR, now_ms()],
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })''',
        '''        let mut connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&connection)?;
        journal::interrupt_after_restart(&mut connection, RESTART_ERROR)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })''',
        "journal-owned restart interruption",
    )

    value = sub_once(
        value,
        r"        let mut event_statement = connection.prepare\(\n            \"SELECT id, task_id, execution_id, sequence, kind, payload_json, created_at_ms\n             FROM task_events WHERE task_id = \?1 ORDER BY sequence\",\n        \)\?;\n        let events = event_statement\n            \.query_map\(\[task_id\], event_from_row\)\?\n            \.collect::<Result<Vec<_>, _>>\(\)\?;",
        "        let events = journal::list_records(&connection, task_id)?;",
        "load Task journal",
    )

    value = sub_once(
        value,
        r"\n    pub fn append_message\(&self, input: NewTaskMessage\).*?\n    pub fn set_task_status\(",
        "\n    pub fn set_task_status(",
        "remove arbitrary message/event append APIs",
    )

    value = sub_once(
        value,
        r"\n    pub fn request_tool_approval\(.*?\n    pub fn register_model_artifact\(",
        '''
    pub fn get_tool_approval(&self, approval_id: &str) -> Result<Option<ToolApproval>, StoreError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, task_id, execution_id, tool_call_id, operation, proposal_json, state,
                        detail, created_at_ms, decided_at_ms, settled_at_ms
                 FROM tool_approvals WHERE id = ?1",
                [approval_id],
                approval_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn list_pending_approvals(&self, task_id: &str) -> Result<Vec<ToolApproval>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, task_id, execution_id, tool_call_id, operation, proposal_json, state,
                    detail, created_at_ms, decided_at_ms, settled_at_ms
             FROM tool_approvals WHERE task_id = ?1 AND state = 'pending'
             ORDER BY created_at_ms, id",
        )?;
        statement
            .query_map([task_id], approval_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn register_model_artifact(''',
        "remove direct Tool Approval mutations",
    )

    value = replace_once(
        value,
        '''    if version == 3 {
        execution::migrate_v4(connection)?;
    }
    Ok(())''',
        '''    if version == 3 {
        execution::migrate_v4(connection)?;
    }
    let version: i64 = connection.query_row(
        "SELECT version FROM desktop_schema WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if version == 4 {
        journal::migrate_v5(connection)?;
    }
    Ok(())''',
        "schema v5 migration",
    )

    value = sub_once(
        value,
        r"\nfn validate_event_kind\(.*?\nfn validate_model_filename\(",
        "\nfn validate_model_filename(",
        "remove arbitrary event/operation validators",
    )
    value = sub_once(
        value,
        r"\nfn next_sequence\(.*?\nfn project_from_row\(",
        "\nfn project_from_row(",
        "remove split-table sequence allocator",
    )
    value = sub_once(
        value,
        r"\nfn event_from_row\(.*?\nfn approval_from_row\(",
        "\nfn approval_from_row(",
        "remove legacy task_events row parser",
    )
    write(path, value)


def patch_execution():
    path = "apps/desktop/src-tauri/src/store/execution.rs"
    value = read(path)
    value = value.replace(
        "    DesktopStore, DesktopTask, StoreError, TaskStatus, new_id, now_ms, validate_nonempty,\n",
        "    DesktopStore, DesktopTask, StoreError, TaskStatus, validate_nonempty,\n",
    )
    value = sub_once(
        value,
        r"\n    fn allows\(self, next: Self\) -> bool \{.*?\n    \}\n\}",
        "\n}",
        "remove non-journal transition authority",
    )
    value = sub_once(
        value,
        r"impl DesktopStore \{\n    pub fn create_execution\(.*?\n\}\n\nfn validate_new_specification\(",
        '''impl DesktopStore {
    pub fn get_execution(&self, execution_id: &str) -> Result<Option<Execution>, StoreError> {
        let connection = self.connection()?;
        load_execution(&connection, execution_id)
    }

    pub fn list_executions(&self, task_id: &str) -> Result<Vec<Execution>, StoreError> {
        let connection = self.connection()?;
        list_task_executions(&connection, task_id)
    }

    pub fn delete_task(&self, task_id: &str) -> Result<(), StoreError> {
        let connection = self.connection()?;
        let changed = connection.execute("DELETE FROM tasks WHERE id = ?1", [task_id])?;
        if changed == 0 {
            return Err(StoreError::message("the Task does not exist"));
        }
        Ok(())
    }
}

pub(super) fn validate_new_specification(''',
        "replace direct Execution writers",
    )
    value = sub_once(
        value,
        r"\nfn validate_transition_failure\(.*?\n\}\n\n",
        "\n",
        "remove direct transition failure helper",
    )
    value = sub_once(
        value,
        r"\npub\(super\) fn interrupt_unsettled\(.*?\n\}\n\n",
        "\n",
        "remove non-journal restart mutation",
    )
    value = value.replace(
        "fn load_execution(\n",
        "pub(super) fn load_execution(\n",
        1,
    )
    write(path, value)


def patch_workspace():
    path = "apps/desktop/src-tauri/src/workspace.rs"
    value = read(path)
    value = value.replace(
        "use crate::store::{DesktopStore, StoreError};",
        "use crate::store::{DesktopStore, StoreError, ToolProposal, ToolResult};",
    )
    value = sub_once(
        value,
        r"pub fn edit_project_file\(.*?\n\}\n\npub fn run_project_shell\(.*?\n\}\n\nfn execute_edit\(",
        '''pub fn edit_project_file(
    store: &DesktopStore,
    task_id: &str,
    approval_id: &str,
    edit: WorkspaceEdit,
) -> Result<WorkspaceEditResult, StoreError> {
    let proposal = ToolProposal::from(&edit);
    store.claim_tool_effect(approval_id, &proposal)?;
    let result = execute_edit(store, task_id, edit);
    match &result {
        Ok(value) => {
            let detail = format!("edited {}", value.path);
            store.settle_tool_effect(
                approval_id,
                true,
                ToolResult::from(value.clone()),
                Some(&detail),
            )?;
        }
        Err(cause) => {
            let message = cause.to_string();
            store.settle_tool_effect(
                approval_id,
                false,
                ToolResult::Failure {
                    message: message.clone(),
                },
                Some(&message),
            )?;
        }
    }
    result
}

pub fn run_project_shell(
    store: &DesktopStore,
    task_id: &str,
    approval_id: &str,
    shell: WorkspaceShell,
) -> Result<WorkspaceShellResult, StoreError> {
    let proposal = ToolProposal::from(&shell);
    store.claim_tool_effect(approval_id, &proposal)?;
    let result = execute_shell(store, task_id, shell);
    match &result {
        Ok(value) => {
            let succeeded = value.exit_code == 0;
            let detail = format!("exit {} in {} ms", value.exit_code, value.duration_ms);
            store.settle_tool_effect(
                approval_id,
                succeeded,
                ToolResult::from(value.clone()),
                Some(&detail),
            )?;
        }
        Err(cause) => {
            let message = cause.to_string();
            store.settle_tool_effect(
                approval_id,
                false,
                ToolResult::Failure {
                    message: message.clone(),
                },
                Some(&message),
            )?;
        }
    }
    result
}

pub(crate) fn execute_edit(''',
        "journal-aware workspace effects",
    )
    value = value.replace("fn execute_shell(\n", "pub(crate) fn execute_shell(\n", 1)
    write(path, value)


def patch_journal_migration_projection_sequences():
    path = "apps/desktop/src-tauri/src/store/journal.rs"
    value = read(path)
    value = replace_once(
        value,
        '''    for task_id in task_ids {
        migrate_legacy_task(&transaction, &task_id)?;
    }
    transaction.execute("DROP TABLE task_events", [])?;''',
        '''    for task_id in task_ids {
        migrate_legacy_task(&transaction, &task_id)?;
    }
    transaction.execute_batch(
        "ALTER TABLE task_messages RENAME TO task_messages_v4_projection;
         CREATE TABLE task_messages (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            execution_id TEXT NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
            sequence INTEGER NOT NULL,
            role TEXT NOT NULL CHECK (role IN ('user','assistant','system')),
            content TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            UNIQUE(task_id, sequence)
         );
         INSERT INTO task_messages
            (id, task_id, execution_id, sequence, role, content, created_at_ms)
         SELECT m.id, m.task_id, m.execution_id, j.sequence, m.role, m.content, m.created_at_ms
         FROM task_messages_v4_projection m
         INNER JOIN task_journal j
           ON j.task_id = m.task_id
          AND j.source_key = 'legacy:message:' || m.id;
         DROP TABLE task_messages_v4_projection;"
    )?;
    transaction.execute("DROP TABLE task_events", [])?;''',
        "migrated message sequence projection",
    )
    value = replace_once(
        value,
        '''            LegacySource::Message => rebuild_legacy_message(
                transaction,
                &record.task_id,
                execution_id,
                source_id,
                *source_sequence,
                *source_occurred_at_ms,
                data,
            )?,''',
        '''            LegacySource::Message => rebuild_legacy_message(
                transaction,
                &record.task_id,
                execution_id,
                source_id,
                record.sequence,
                *source_occurred_at_ms,
                data,
            )?,''',
        "legacy message rebuild journal sequence",
    )
    value = value.replace(
        "    source_sequence: Option<i64>,\n    occurred_at_ms: i64,",
        "    journal_sequence: i64,\n    occurred_at_ms: i64,",
        1,
    )
    value = value.replace(
        "            source_sequence.unwrap_or(0),\n            role,",
        "            journal_sequence,\n            role,",
        1,
    )
    write(path, value)


patch_store()
patch_execution()
patch_workspace()
patch_journal_migration_projection_sequences()
print("issue #48 core cutover applied")
