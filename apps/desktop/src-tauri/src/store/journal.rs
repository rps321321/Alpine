//! Versioned append-only Task journal and deterministic projection rebuilding.

use super::{
    ApprovalState, DesktopStore, Execution, ExecutionId, ExecutionState, MessageRole,
    NewExecutionSpecification, NewToolApproval, StoreError, TaskEvent, TaskMessage, ToolApproval,
    ToolApprovalDecision, approval_from_row, execution, new_id, now_ms, validate_nonempty,
};
use crate::workspace::{WorkspaceEdit, WorkspaceEditResult, WorkspaceShell, WorkspaceShellResult};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const TASK_JOURNAL_VERSION: u16 = 1;
const MAX_JOURNAL_EVENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserDirection {
    Steer,
    FollowUp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolOperation {
    Edit,
    Shell,
}

impl ToolOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Shell => "shell",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ToolProposal {
    Edit {
        path: String,
        old_text: String,
        new_text: String,
    },
    Shell {
        command: String,
        timeout_seconds: u64,
    },
}

impl ToolProposal {
    pub fn operation(&self) -> ToolOperation {
        match self {
            Self::Edit { .. } => ToolOperation::Edit,
            Self::Shell { .. } => ToolOperation::Shell,
        }
    }
}

impl From<WorkspaceEdit> for ToolProposal {
    fn from(value: WorkspaceEdit) -> Self {
        Self::Edit {
            path: value.path,
            old_text: value.old_text,
            new_text: value.new_text,
        }
    }
}

impl From<&WorkspaceEdit> for ToolProposal {
    fn from(value: &WorkspaceEdit) -> Self {
        Self::Edit {
            path: value.path.clone(),
            old_text: value.old_text.clone(),
            new_text: value.new_text.clone(),
        }
    }
}

impl From<WorkspaceShell> for ToolProposal {
    fn from(value: WorkspaceShell) -> Self {
        Self::Shell {
            command: value.command,
            timeout_seconds: value.timeout_seconds,
        }
    }
}

impl From<&WorkspaceShell> for ToolProposal {
    fn from(value: &WorkspaceShell) -> Self {
        Self::Shell {
            command: value.command.clone(),
            timeout_seconds: value.timeout_seconds,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ToolResult {
    Edit {
        path: String,
        replacements: usize,
        diff: String,
    },
    Shell {
        command: String,
        exit_code: i32,
        stdout: String,
        stderr: String,
        duration_ms: u64,
        truncated: bool,
    },
    Failure {
        message: String,
    },
}

impl From<WorkspaceEditResult> for ToolResult {
    fn from(value: WorkspaceEditResult) -> Self {
        Self::Edit {
            path: value.path,
            replacements: value.replacements,
            diff: value.diff,
        }
    }
}

impl From<WorkspaceShellResult> for ToolResult {
    fn from(value: WorkspaceShellResult) -> Self {
        Self::Shell {
            command: value.command,
            exit_code: value.exit_code,
            stdout: value.stdout,
            stderr: value.stderr,
            duration_ms: value.duration_ms,
            truncated: value.truncated,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolSettlementState {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionOutcome {
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl ExecutionOutcome {
    fn from_state(state: ExecutionState) -> Result<Self, StoreError> {
        match state {
            ExecutionState::Completed => Ok(Self::Completed),
            ExecutionState::Cancelled => Ok(Self::Cancelled),
            ExecutionState::Failed => Ok(Self::Failed),
            ExecutionState::Interrupted => Ok(Self::Interrupted),
            _ => Err(StoreError::message(
                "Execution outcome requires a terminal Execution state",
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LegacySource {
    Execution,
    Message,
    Event,
    Approval,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LegacyCausalOrder {
    Unverified,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TaskJournalEvent {
    UserPromptAccepted {
        content: String,
    },
    UserDirectionAccepted {
        direction: UserDirection,
        content: String,
    },
    ExecutionQueued {
        execution_spec_id: String,
    },
    ExecutionPreparing,
    ExecutionStarted,
    AssistantMessageCompleted {
        content: String,
    },
    ToolProposed {
        approval_id: String,
        tool_call_id: String,
        proposal: ToolProposal,
    },
    ExecutionWaitingForApproval {
        approval_id: String,
    },
    ApprovalDecided {
        approval_id: String,
        approved: bool,
    },
    ExecutionResumed {
        approval_id: String,
    },
    ToolStarted {
        approval_id: String,
        proposal: ToolProposal,
    },
    ToolResultRecorded {
        approval_id: String,
        succeeded: bool,
        result: ToolResult,
    },
    ToolSettled {
        approval_id: String,
        state: ToolSettlementState,
        detail: Option<String>,
    },
    ApprovalInterrupted {
        approval_id: String,
        detail: String,
    },
    ExecutionCancelling,
    ExecutionFinished {
        outcome: ExecutionOutcome,
        failure: Option<String>,
        duration_ms: Option<u64>,
        response_characters: Option<u64>,
    },
    LegacyImported {
        source: LegacySource,
        source_id: String,
        source_sequence: Option<i64>,
        source_occurred_at_ms: i64,
        causal_order: LegacyCausalOrder,
        data: Value,
    },
}

#[derive(Clone, Debug)]
pub struct AcceptedPrompt {
    pub execution: Execution,
    pub prompt_message: TaskMessage,
    pub records: Vec<TaskEvent>,
}

#[derive(Clone, Debug)]
pub struct RecordedMessage {
    pub message: TaskMessage,
    pub record: TaskEvent,
}

#[derive(Clone, Debug)]
pub struct ToolProposalOutcome {
    pub approval: ToolApproval,
    pub records: Vec<TaskEvent>,
    pub execution: Execution,
}

#[derive(Clone, Debug)]
pub struct ToolClaimOutcome {
    pub approval: ToolApproval,
    pub record: TaskEvent,
}

#[derive(Clone, Debug)]
pub struct ToolSettlementOutcome {
    pub approval: ToolApproval,
    pub records: Vec<TaskEvent>,
}

#[derive(Clone, Debug)]
pub struct ExecutionTransitionOutcome {
    pub execution: Execution,
    pub records: Vec<TaskEvent>,
}

impl DesktopStore {
    pub fn load_journal(&self, task_id: &str) -> Result<Vec<TaskEvent>, StoreError> {
        let connection = self.connection()?;
        list_records(&connection, task_id)
    }

    pub fn accept_prompt(
        &self,
        task_id: &str,
        prompt: &str,
        specification: NewExecutionSpecification,
    ) -> Result<AcceptedPrompt, StoreError> {
        let task_id = validate_nonempty("Task identifier", task_id, 160)?.to_owned();
        let prompt = validate_nonempty("user prompt", prompt, 4 * 1024 * 1024)?.to_owned();
        let specification = execution::validate_new_specification(specification)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let timestamp = now_ms();
        let execution = insert_execution(&transaction, &task_id, specification, timestamp)?;
        let prompt_record = append_record(
            &transaction,
            &task_id,
            Some(execution.id.as_str()),
            TaskJournalEvent::UserPromptAccepted {
                content: prompt.clone(),
            },
            timestamp,
            None,
        )?;
        let prompt_message =
            insert_message_projection(&transaction, &prompt_record, MessageRole::User, &prompt)?;
        let queued_record = append_record(
            &transaction,
            &task_id,
            Some(execution.id.as_str()),
            TaskJournalEvent::ExecutionQueued {
                execution_spec_id: execution.execution_spec_id.clone(),
            },
            timestamp,
            None,
        )?;
        touch_task(&transaction, &task_id, timestamp)?;
        transaction.commit()?;
        Ok(AcceptedPrompt {
            execution,
            prompt_message,
            records: vec![prompt_record, queued_record],
        })
    }

    pub fn record_direction(
        &self,
        execution_id: &str,
        direction: UserDirection,
        text: &str,
    ) -> Result<RecordedMessage, StoreError> {
        let text = validate_nonempty("direction", text, 256 * 1024)?.to_owned();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let execution = execution::load_execution(&transaction, execution_id)?
            .ok_or_else(|| StoreError::message("the Execution does not exist"))?;
        if !matches!(
            execution.state,
            ExecutionState::Running | ExecutionState::WaitingForApproval
        ) {
            return Err(StoreError::message(format!(
                "Execution {execution_id} cannot accept direction while {}",
                execution.state.as_str()
            )));
        }
        let timestamp = now_ms();
        let record = append_record(
            &transaction,
            &execution.task_id,
            Some(execution_id),
            TaskJournalEvent::UserDirectionAccepted {
                direction,
                content: text.clone(),
            },
            timestamp,
            None,
        )?;
        let message = insert_message_projection(&transaction, &record, MessageRole::User, &text)?;
        touch_task(&transaction, &execution.task_id, timestamp)?;
        transaction.commit()?;
        Ok(RecordedMessage { message, record })
    }

    pub fn record_assistant_message(
        &self,
        execution_id: &str,
        content: &str,
    ) -> Result<RecordedMessage, StoreError> {
        let content = validate_nonempty("assistant message", content, 4 * 1024 * 1024)?.to_owned();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let execution = execution::load_execution(&transaction, execution_id)?
            .ok_or_else(|| StoreError::message("the Execution does not exist"))?;
        if execution.state.is_terminal() {
            return Err(StoreError::message(
                "a terminal Execution cannot append an assistant message",
            ));
        }
        let timestamp = now_ms();
        let record = append_record(
            &transaction,
            &execution.task_id,
            Some(execution_id),
            TaskJournalEvent::AssistantMessageCompleted {
                content: content.clone(),
            },
            timestamp,
            None,
        )?;
        let message =
            insert_message_projection(&transaction, &record, MessageRole::Assistant, &content)?;
        touch_task(&transaction, &execution.task_id, timestamp)?;
        transaction.commit()?;
        Ok(RecordedMessage { message, record })
    }

    pub fn record_execution_state(
        &self,
        execution_id: &str,
        next: ExecutionState,
    ) -> Result<ExecutionTransitionOutcome, StoreError> {
        let event = match next {
            ExecutionState::Preparing => TaskJournalEvent::ExecutionPreparing,
            ExecutionState::Running => TaskJournalEvent::ExecutionStarted,
            ExecutionState::Cancelling => TaskJournalEvent::ExecutionCancelling,
            _ => {
                return Err(StoreError::message(
                    "this Execution transition requires a specialized journal operation",
                ));
            }
        };
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let timestamp = now_ms();
        let execution = transition_execution(&transaction, execution_id, next, None, timestamp)?;
        let record = append_record(
            &transaction,
            &execution.task_id,
            Some(execution_id),
            event,
            timestamp,
            None,
        )?;
        touch_task(&transaction, &execution.task_id, timestamp)?;
        transaction.commit()?;
        Ok(ExecutionTransitionOutcome {
            execution,
            records: vec![record],
        })
    }

    pub fn finish_execution(
        &self,
        execution_id: &str,
        requested: ExecutionState,
        failure: Option<&str>,
        duration_ms: Option<u64>,
        response_characters: Option<u64>,
    ) -> Result<ExecutionTransitionOutcome, StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let current = execution::load_execution(&transaction, execution_id)?
            .ok_or_else(|| StoreError::message("the Execution does not exist"))?;
        if current.state.is_terminal() {
            transaction.commit()?;
            return Ok(ExecutionTransitionOutcome {
                execution: current,
                records: Vec::new(),
            });
        }
        let next = if current.state == ExecutionState::Cancelling {
            ExecutionState::Cancelled
        } else {
            requested
        };
        if !next.is_terminal() {
            return Err(StoreError::message(
                "finish_execution requires a terminal state",
            ));
        }
        let failure = validate_terminal_failure(next, failure)?;
        let timestamp = now_ms();
        let execution = transition_execution(
            &transaction,
            execution_id,
            next,
            failure.as_deref(),
            timestamp,
        )?;
        let record = append_record(
            &transaction,
            &execution.task_id,
            Some(execution_id),
            TaskJournalEvent::ExecutionFinished {
                outcome: ExecutionOutcome::from_state(next)?,
                failure,
                duration_ms,
                response_characters,
            },
            timestamp,
            None,
        )?;
        touch_task(&transaction, &execution.task_id, timestamp)?;
        transaction.commit()?;
        Ok(ExecutionTransitionOutcome {
            execution,
            records: vec![record],
        })
    }

    pub fn propose_tool(&self, input: NewToolApproval) -> Result<ToolProposalOutcome, StoreError> {
        let tool_call_id =
            validate_nonempty("tool call identifier", &input.tool_call_id, 160)?.to_owned();
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let execution = execution::load_execution(&transaction, input.execution_id.as_str())?
            .ok_or_else(|| StoreError::message("the Execution does not exist"))?;
        if execution.task_id != input.task_id || execution.state != ExecutionState::Running {
            return Err(StoreError::message(
                "the Execution is not running this Tool proposal",
            ));
        }
        let approval_id = new_id(&transaction);
        let timestamp = now_ms();
        let proposed = append_record(
            &transaction,
            &input.task_id,
            Some(input.execution_id.as_str()),
            TaskJournalEvent::ToolProposed {
                approval_id: approval_id.clone(),
                tool_call_id: tool_call_id.clone(),
                proposal: input.proposal.clone(),
            },
            timestamp,
            None,
        )?;
        insert_approval_projection(
            &transaction,
            &approval_id,
            &input.task_id,
            input.execution_id.as_str(),
            &tool_call_id,
            &input.proposal,
            timestamp,
        )?;
        let execution = transition_execution(
            &transaction,
            input.execution_id.as_str(),
            ExecutionState::WaitingForApproval,
            None,
            timestamp,
        )?;
        let waiting = append_record(
            &transaction,
            &input.task_id,
            Some(input.execution_id.as_str()),
            TaskJournalEvent::ExecutionWaitingForApproval {
                approval_id: approval_id.clone(),
            },
            timestamp,
            None,
        )?;
        let approval = load_approval(&transaction, &approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval was not projected"))?;
        touch_task(&transaction, &input.task_id, timestamp)?;
        transaction.commit()?;
        Ok(ToolProposalOutcome {
            approval,
            records: vec![proposed, waiting],
            execution,
        })
    }

    pub fn decide_tool_approval_recorded(
        &self,
        approval_id: &str,
        approved: bool,
    ) -> Result<ToolApprovalDecision, StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let approval = load_approval(&transaction, approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval does not exist"))?;
        if approval.state != ApprovalState::Pending {
            return Err(StoreError::message(
                "the Tool Approval has already been decided",
            ));
        }
        let execution = execution::load_execution(&transaction, approval.execution_id.as_str())?
            .ok_or_else(|| StoreError::message("the Execution does not exist"))?;
        if execution.state != ExecutionState::WaitingForApproval {
            return Err(StoreError::message(
                "the Execution is not waiting for this Tool Approval",
            ));
        }
        let timestamp = now_ms();
        let next = if approved {
            ApprovalState::Approved
        } else {
            ApprovalState::Denied
        };
        let changed = transaction.execute(
            "UPDATE tool_approvals SET state = ?2, decided_at_ms = ?3
             WHERE id = ?1 AND state = 'pending'",
            params![approval_id, next.as_str(), timestamp],
        )?;
        if changed != 1 {
            return Err(StoreError::message(
                "the Tool Approval changed concurrently",
            ));
        }
        let decision_record = append_record(
            &transaction,
            &approval.task_id,
            Some(approval.execution_id.as_str()),
            TaskJournalEvent::ApprovalDecided {
                approval_id: approval_id.to_owned(),
                approved,
            },
            timestamp,
            None,
        )?;
        let execution = transition_execution(
            &transaction,
            approval.execution_id.as_str(),
            ExecutionState::Running,
            None,
            timestamp,
        )?;
        let resumed_record = append_record(
            &transaction,
            &approval.task_id,
            Some(approval.execution_id.as_str()),
            TaskJournalEvent::ExecutionResumed {
                approval_id: approval_id.to_owned(),
            },
            timestamp,
            None,
        )?;
        let approval = load_approval(&transaction, approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval no longer exists"))?;
        touch_task(&transaction, &approval.task_id, timestamp)?;
        transaction.commit()?;
        Ok(ToolApprovalDecision {
            approval,
            execution,
            records: vec![decision_record, resumed_record],
        })
    }

    pub fn claim_tool_effect(
        &self,
        approval_id: &str,
        proposal: &ToolProposal,
    ) -> Result<ToolClaimOutcome, StoreError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let approval = load_approval(&transaction, approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval does not exist"))?;
        if approval.state != ApprovalState::Approved {
            return Err(StoreError::message(
                "the Tool Approval is not approved or has already been claimed",
            ));
        }
        let stored: ToolProposal =
            serde_json::from_value(approval.proposal.clone()).map_err(|error| {
                StoreError::message(format!("stored Tool Approval proposal is invalid: {error}"))
            })?;
        if &stored != proposal {
            return Err(StoreError::message(
                "the Tool Approval does not match the exact proposed operation",
            ));
        }
        let timestamp = now_ms();
        let changed = transaction.execute(
            "UPDATE tool_approvals SET state = 'executing' WHERE id = ?1 AND state = 'approved'",
            [approval_id],
        )?;
        if changed != 1 {
            return Err(StoreError::message(
                "the Tool Approval changed concurrently",
            ));
        }
        let record = append_record(
            &transaction,
            &approval.task_id,
            Some(approval.execution_id.as_str()),
            TaskJournalEvent::ToolStarted {
                approval_id: approval_id.to_owned(),
                proposal: proposal.clone(),
            },
            timestamp,
            None,
        )?;
        let approval = load_approval(&transaction, approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval no longer exists"))?;
        touch_task(&transaction, &approval.task_id, timestamp)?;
        transaction.commit()?;
        Ok(ToolClaimOutcome { approval, record })
    }

    pub fn settle_tool_effect(
        &self,
        approval_id: &str,
        succeeded: bool,
        result: ToolResult,
        detail: Option<&str>,
    ) -> Result<ToolSettlementOutcome, StoreError> {
        let detail = detail
            .map(|value| validate_nonempty("Tool Approval result", value, 64 * 1024))
            .transpose()?
            .map(str::to_owned);
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let approval = load_approval(&transaction, approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval does not exist"))?;
        if approval.state != ApprovalState::Executing {
            return Err(StoreError::message(
                "the Tool Approval is not executing or has already settled",
            ));
        }
        let state = if succeeded {
            ApprovalState::Completed
        } else {
            ApprovalState::Failed
        };
        let settlement = if succeeded {
            ToolSettlementState::Completed
        } else {
            ToolSettlementState::Failed
        };
        let timestamp = now_ms();
        let result_record = append_record(
            &transaction,
            &approval.task_id,
            Some(approval.execution_id.as_str()),
            TaskJournalEvent::ToolResultRecorded {
                approval_id: approval_id.to_owned(),
                succeeded,
                result,
            },
            timestamp,
            None,
        )?;
        let changed = transaction.execute(
            "UPDATE tool_approvals SET state = ?2, detail = ?3, settled_at_ms = ?4
             WHERE id = ?1 AND state = 'executing'",
            params![approval_id, state.as_str(), detail, timestamp],
        )?;
        if changed != 1 {
            return Err(StoreError::message(
                "the Tool Approval changed concurrently",
            ));
        }
        let settled_record = append_record(
            &transaction,
            &approval.task_id,
            Some(approval.execution_id.as_str()),
            TaskJournalEvent::ToolSettled {
                approval_id: approval_id.to_owned(),
                state: settlement,
                detail: detail.clone(),
            },
            timestamp,
            None,
        )?;
        let approval = load_approval(&transaction, approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval no longer exists"))?;
        touch_task(&transaction, &approval.task_id, timestamp)?;
        transaction.commit()?;
        Ok(ToolSettlementOutcome {
            approval,
            records: vec![result_record, settled_record],
        })
    }

    pub fn rebuild_task_projections(&self, task_id: &str) -> Result<(), StoreError> {
        let mut connection = self.connection()?;
        rebuild_task_projections(&mut connection, task_id)
    }
}

pub(super) fn migrate_v5(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "BEGIN IMMEDIATE;
         CREATE TABLE task_journal (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            execution_id TEXT,
            sequence INTEGER NOT NULL,
            version INTEGER NOT NULL,
            event_json TEXT NOT NULL,
            occurred_at_ms INTEGER NOT NULL,
            source_key TEXT,
            UNIQUE(task_id, sequence),
            UNIQUE(task_id, source_key)
         );
         CREATE INDEX task_journal_execution_sequence
            ON task_journal(execution_id, sequence);
         COMMIT;",
    )?;

    let transaction = connection.unchecked_transaction()?;
    let task_ids = {
        let mut statement =
            transaction.prepare("SELECT id FROM tasks ORDER BY created_at_ms, id")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
    };
    for task_id in task_ids {
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
         DROP TABLE task_messages_v4_projection;",
    )?;
    transaction.execute("DROP TABLE task_events", [])?;
    transaction.execute(
        "UPDATE desktop_schema SET version = 5 WHERE singleton = 1",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn interrupt_after_restart(
    connection: &mut Connection,
    failure: &str,
) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    let active = {
        let mut statement = transaction.prepare(
            "SELECT id, task_id FROM executions
             WHERE state IN ('queued','preparing','running','waiting-for-approval','cancelling')
             ORDER BY queued_at_ms, id",
        )?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    for (execution_id, task_id) in active {
        let approvals = {
            let mut statement = transaction.prepare(
                "SELECT id FROM tool_approvals
                 WHERE execution_id = ?1 AND state IN ('pending','approved','executing')
                 ORDER BY created_at_ms, id",
            )?;
            statement
                .query_map([&execution_id], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        let timestamp = now_ms();
        for approval_id in approvals {
            transaction.execute(
                "UPDATE tool_approvals
                 SET state = 'interrupted', detail = ?2, settled_at_ms = ?3
                 WHERE id = ?1 AND state IN ('pending','approved','executing')",
                params![approval_id, failure, timestamp],
            )?;
            append_record(
                &transaction,
                &task_id,
                Some(&execution_id),
                TaskJournalEvent::ApprovalInterrupted {
                    approval_id: approval_id.clone(),
                    detail: failure.to_owned(),
                },
                timestamp,
                None,
            )?;
            append_record(
                &transaction,
                &task_id,
                Some(&execution_id),
                TaskJournalEvent::ToolSettled {
                    approval_id,
                    state: ToolSettlementState::Interrupted,
                    detail: Some(failure.to_owned()),
                },
                timestamp,
                None,
            )?;
        }
        transition_execution(
            &transaction,
            &execution_id,
            ExecutionState::Interrupted,
            Some(failure),
            timestamp,
        )?;
        append_record(
            &transaction,
            &task_id,
            Some(&execution_id),
            TaskJournalEvent::ExecutionFinished {
                outcome: ExecutionOutcome::Interrupted,
                failure: Some(failure.to_owned()),
                duration_ms: None,
                response_characters: None,
            },
            timestamp,
            None,
        )?;
        touch_task(&transaction, &task_id, timestamp)?;
    }
    transaction.commit()?;
    Ok(())
}

fn insert_execution(
    transaction: &Transaction<'_>,
    task_id: &str,
    specification: NewExecutionSpecification,
    timestamp: i64,
) -> Result<Execution, StoreError> {
    let task_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
        [task_id],
        |row| row.get(0),
    )?;
    if !task_exists {
        return Err(StoreError::message("the Task does not exist"));
    }
    let active_exists: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM executions
            WHERE task_id = ?1
              AND state IN ('queued','preparing','running','waiting-for-approval','cancelling')
         )",
        [task_id],
        |row| row.get(0),
    )?;
    if active_exists {
        return Err(StoreError::message(
            "the Task already has an active Execution",
        ));
    }
    let execution_id = ExecutionId(new_id(transaction));
    let specification_id = new_id(transaction);
    transaction.execute(
        "INSERT INTO execution_specs
         (id, task_id, model_registry_id, model_repo_id, model_revision, model_filename,
          model_sha256, session_config_sha256, profile_name, profile_sha256, runtime_name,
          runtime_identity, adapter_identity, policy_identity, context_window, max_tokens,
          temperature_millis, legacy_unverified, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                 ?15, ?16, ?17, 0, ?18)",
        params![
            specification_id,
            task_id,
            specification.model_registry_id,
            specification.model_repo_id,
            specification.model_revision,
            specification.model_filename,
            specification.model_sha256,
            specification.session_config_sha256,
            specification.profile_name,
            specification.profile_sha256,
            specification.runtime_name,
            specification.runtime_identity,
            specification.adapter_identity,
            specification.policy_identity,
            i64::from(specification.context_window),
            i64::from(specification.max_tokens),
            specification.temperature_millis,
            timestamp,
        ],
    )?;
    transaction.execute(
        "INSERT INTO executions
         (id, task_id, execution_spec_id, state, failure, queued_at_ms, started_at_ms,
          finished_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, 'queued', NULL, ?4, NULL, NULL, ?4)",
        params![execution_id.as_str(), task_id, specification_id, timestamp],
    )?;
    execution::load_execution(transaction, execution_id.as_str())?
        .ok_or_else(|| StoreError::message("the Execution was not projected"))
}

fn transition_execution(
    transaction: &Transaction<'_>,
    execution_id: &str,
    next: ExecutionState,
    failure: Option<&str>,
    timestamp: i64,
) -> Result<Execution, StoreError> {
    let current = execution::load_execution(transaction, execution_id)?
        .ok_or_else(|| StoreError::message("the Execution does not exist"))?;
    if !state_allows(current.state, next) {
        return Err(StoreError::message(format!(
            "invalid Execution transition from '{}' to '{}'",
            current.state.as_str(),
            next.as_str()
        )));
    }
    let started_at_ms = current.started_at_ms.or_else(|| {
        (!matches!(next, ExecutionState::Queued | ExecutionState::Cancelled)).then_some(timestamp)
    });
    let finished_at_ms = next.is_terminal().then_some(timestamp);
    let changed = transaction.execute(
        "UPDATE executions
         SET state = ?2, failure = ?3, started_at_ms = ?4, finished_at_ms = ?5,
             updated_at_ms = ?6
         WHERE id = ?1 AND state = ?7",
        params![
            execution_id,
            next.as_str(),
            failure,
            started_at_ms,
            finished_at_ms,
            timestamp,
            current.state.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::message(
            "the Execution changed concurrently; reload it before retrying",
        ));
    }
    execution::load_execution(transaction, execution_id)?
        .ok_or_else(|| StoreError::message("the Execution no longer exists"))
}

fn state_allows(current: ExecutionState, next: ExecutionState) -> bool {
    match current {
        ExecutionState::Queued => matches!(
            next,
            ExecutionState::Preparing
                | ExecutionState::Cancelling
                | ExecutionState::Cancelled
                | ExecutionState::Failed
                | ExecutionState::Interrupted
        ),
        ExecutionState::Preparing => matches!(
            next,
            ExecutionState::Running
                | ExecutionState::Cancelling
                | ExecutionState::Failed
                | ExecutionState::Interrupted
        ),
        ExecutionState::Running => matches!(
            next,
            ExecutionState::WaitingForApproval
                | ExecutionState::Cancelling
                | ExecutionState::Completed
                | ExecutionState::Failed
                | ExecutionState::Interrupted
        ),
        ExecutionState::WaitingForApproval => matches!(
            next,
            ExecutionState::Running
                | ExecutionState::Cancelling
                | ExecutionState::Failed
                | ExecutionState::Interrupted
        ),
        ExecutionState::Cancelling => matches!(
            next,
            ExecutionState::Cancelled | ExecutionState::Failed | ExecutionState::Interrupted
        ),
        ExecutionState::Completed
        | ExecutionState::Cancelled
        | ExecutionState::Failed
        | ExecutionState::Interrupted => false,
    }
}

fn validate_terminal_failure(
    next: ExecutionState,
    failure: Option<&str>,
) -> Result<Option<String>, StoreError> {
    match next {
        ExecutionState::Failed | ExecutionState::Interrupted => failure
            .map(|value| validate_nonempty("Execution failure", value, 64 * 1024))
            .transpose()?
            .ok_or_else(|| {
                StoreError::message("failed and interrupted Executions require a failure detail")
            })
            .map(|value| Some(value.to_owned())),
        _ if failure.is_some() => Err(StoreError::message(
            "only failed or interrupted Executions may store a failure detail",
        )),
        _ => Ok(None),
    }
}

fn append_record(
    transaction: &Transaction<'_>,
    task_id: &str,
    execution_id: Option<&str>,
    event: TaskJournalEvent,
    occurred_at_ms: i64,
    source_key: Option<&str>,
) -> Result<TaskEvent, StoreError> {
    let event_json = serde_json::to_string(&event).map_err(|error| {
        StoreError::message(format!("failed to encode Task Journal event: {error}"))
    })?;
    if event_json.len() > MAX_JOURNAL_EVENT_BYTES {
        return Err(StoreError::message("Task Journal event exceeds 4 MiB"));
    }
    let task_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
        [task_id],
        |row| row.get(0),
    )?;
    if !task_exists {
        return Err(StoreError::message("the Task does not exist"));
    }
    let sequence: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence), 0) + 1 FROM task_journal WHERE task_id = ?1",
        [task_id],
        |row| row.get(0),
    )?;
    let id = new_id(transaction);
    transaction.execute(
        "INSERT INTO task_journal
         (id, task_id, execution_id, sequence, version, event_json, occurred_at_ms, source_key)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            task_id,
            execution_id,
            sequence,
            i64::from(TASK_JOURNAL_VERSION),
            event_json,
            occurred_at_ms,
            source_key,
        ],
    )?;
    Ok(TaskEvent {
        id,
        task_id: task_id.to_owned(),
        execution_id: execution_id.map(|value| ExecutionId(value.to_owned())),
        sequence,
        version: TASK_JOURNAL_VERSION,
        event,
        created_at_ms: occurred_at_ms,
    })
}

fn insert_message_projection(
    transaction: &Transaction<'_>,
    record: &TaskEvent,
    role: MessageRole,
    content: &str,
) -> Result<TaskMessage, StoreError> {
    let execution_id = record
        .execution_id
        .as_ref()
        .ok_or_else(|| StoreError::message("message Journal record requires an Execution ID"))?;
    transaction.execute(
        "INSERT INTO task_messages
         (id, task_id, execution_id, sequence, role, content, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            record.id,
            record.task_id,
            execution_id.as_str(),
            record.sequence,
            role.as_str(),
            content,
            record.created_at_ms,
        ],
    )?;
    Ok(TaskMessage {
        id: record.id.clone(),
        task_id: record.task_id.clone(),
        execution_id: execution_id.clone(),
        sequence: record.sequence,
        role,
        content: content.to_owned(),
        created_at_ms: record.created_at_ms,
    })
}

fn insert_approval_projection(
    transaction: &Transaction<'_>,
    approval_id: &str,
    task_id: &str,
    execution_id: &str,
    tool_call_id: &str,
    proposal: &ToolProposal,
    timestamp: i64,
) -> Result<(), StoreError> {
    let proposal_json = serde_json::to_string(proposal).map_err(|error| {
        StoreError::message(format!("failed to encode Tool Approval proposal: {error}"))
    })?;
    transaction.execute(
        "INSERT INTO tool_approvals
         (id, task_id, execution_id, tool_call_id, operation, proposal_json, state, detail,
          created_at_ms, decided_at_ms, settled_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', NULL, ?7, NULL, NULL)",
        params![
            approval_id,
            task_id,
            execution_id,
            tool_call_id,
            proposal.operation().as_str(),
            proposal_json,
            timestamp,
        ],
    )?;
    Ok(())
}

fn load_approval(
    connection: &Connection,
    approval_id: &str,
) -> Result<Option<ToolApproval>, StoreError> {
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

fn touch_task(
    transaction: &Transaction<'_>,
    task_id: &str,
    timestamp: i64,
) -> Result<(), StoreError> {
    transaction.execute(
        "UPDATE tasks SET updated_at_ms = MAX(updated_at_ms, ?2) WHERE id = ?1",
        params![task_id, timestamp],
    )?;
    Ok(())
}

pub(super) fn list_records(
    connection: &Connection,
    task_id: &str,
) -> Result<Vec<TaskEvent>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT id, task_id, execution_id, sequence, version, event_json, occurred_at_ms
         FROM task_journal WHERE task_id = ?1 ORDER BY sequence",
    )?;
    statement
        .query_map([task_id], record_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn decode_task_journal_event(event_json: &str) -> Result<TaskJournalEvent, StoreError> {
    let value: Value = serde_json::from_str(event_json).map_err(|error| {
        StoreError::message(format!("invalid Task Journal event JSON: {error}"))
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| StoreError::message("Task Journal event must be a JSON object"))?;
    let event_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::message("Task Journal event requires a string type"))?
        .to_owned();
    let allowed: &[&str] = match event_type.as_str() {
        "user-prompt-accepted" => &["type", "content"],
        "user-direction-accepted" => &["type", "direction", "content"],
        "execution-queued" => &["type", "executionSpecId"],
        "execution-preparing" => &["type"],
        "execution-started" => &["type"],
        "assistant-message-completed" => &["type", "content"],
        "tool-proposed" => &["type", "approvalId", "toolCallId", "proposal"],
        "execution-waiting-for-approval" => &["type", "approvalId"],
        "approval-decided" => &["type", "approvalId", "approved"],
        "execution-resumed" => &["type", "approvalId"],
        "tool-started" => &["type", "approvalId", "proposal"],
        "tool-result-recorded" => &["type", "approvalId", "succeeded", "result"],
        "tool-settled" => &["type", "approvalId", "state", "detail"],
        "approval-interrupted" => &["type", "approvalId", "detail"],
        "execution-cancelling" => &["type"],
        "execution-finished" => &[
            "type",
            "outcome",
            "failure",
            "durationMs",
            "responseCharacters",
        ],
        "legacy-imported" => &[
            "type",
            "source",
            "sourceId",
            "sourceSequence",
            "sourceOccurredAtMs",
            "causalOrder",
            "data",
        ],
        _ => {
            return Err(StoreError::message(format!(
                "unsupported Task Journal event type '{event_type}'"
            )));
        }
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(StoreError::message(format!(
                "Task Journal event '{event_type}' contains unknown field '{key}'"
            )));
        }
    }
    serde_json::from_value(value)
        .map_err(|error| StoreError::message(format!("invalid Task Journal event: {error}")))
}

pub(super) fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEvent> {
    let version_i64: i64 = row.get(4)?;
    let version = u16::try_from(version_i64).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    if version != TASK_JOURNAL_VERSION {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Integer,
            Box::new(StoreError::message(format!(
                "unsupported Task Journal version {version}"
            ))),
        ));
    }
    let event_json: String = row.get(5)?;
    let event = decode_task_journal_event(&event_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(TaskEvent {
        id: row.get(0)?,
        task_id: row.get(1)?,
        execution_id: row.get::<_, Option<String>>(2)?.map(ExecutionId),
        sequence: row.get(3)?,
        version,
        event,
        created_at_ms: row.get(6)?,
    })
}

#[derive(Debug)]
struct LegacyEnvelope {
    occurred_at_ms: i64,
    rank: u8,
    source_sequence: Option<i64>,
    source_id: String,
    execution_id: Option<String>,
    source: LegacySource,
    data: Value,
}

fn migrate_legacy_task(transaction: &Transaction<'_>, task_id: &str) -> Result<(), StoreError> {
    let mut rows = Vec::new();
    {
        let mut statement = transaction.prepare(
            "SELECT id, execution_spec_id, state, failure, queued_at_ms, started_at_ms,
                    finished_at_ms, updated_at_ms
             FROM executions WHERE task_id = ?1 ORDER BY queued_at_ms, id",
        )?;
        let values = statement.query_map([task_id], |row| {
            let id: String = row.get(0)?;
            let queued_at_ms: i64 = row.get(4)?;
            Ok(LegacyEnvelope {
                occurred_at_ms: queued_at_ms,
                rank: 0,
                source_sequence: None,
                source_id: id.clone(),
                execution_id: Some(id),
                source: LegacySource::Execution,
                data: json!({
                    "executionSpecId": row.get::<_, String>(1)?,
                    "state": row.get::<_, String>(2)?,
                    "failure": row.get::<_, Option<String>>(3)?,
                    "queuedAtMs": queued_at_ms,
                    "startedAtMs": row.get::<_, Option<i64>>(5)?,
                    "finishedAtMs": row.get::<_, Option<i64>>(6)?,
                    "updatedAtMs": row.get::<_, i64>(7)?,
                }),
            })
        })?;
        rows.extend(values.collect::<Result<Vec<_>, _>>()?);
    }
    {
        let mut statement = transaction.prepare(
            "SELECT id, execution_id, sequence, role, content, created_at_ms
             FROM task_messages WHERE task_id = ?1 ORDER BY sequence, id",
        )?;
        let values = statement.query_map([task_id], |row| {
            let id: String = row.get(0)?;
            let execution_id: String = row.get(1)?;
            let sequence: i64 = row.get(2)?;
            let occurred_at_ms: i64 = row.get(5)?;
            Ok(LegacyEnvelope {
                occurred_at_ms,
                rank: 1,
                source_sequence: Some(sequence),
                source_id: id,
                execution_id: Some(execution_id),
                source: LegacySource::Message,
                data: json!({
                    "role": row.get::<_, String>(3)?,
                    "content": row.get::<_, String>(4)?,
                }),
            })
        })?;
        rows.extend(values.collect::<Result<Vec<_>, _>>()?);
    }
    {
        let mut statement = transaction.prepare(
            "SELECT id, execution_id, sequence, kind, payload_json, created_at_ms
             FROM task_events WHERE task_id = ?1 ORDER BY sequence, id",
        )?;
        let values = statement.query_map([task_id], |row| {
            let id: String = row.get(0)?;
            let execution_id: String = row.get(1)?;
            let sequence: i64 = row.get(2)?;
            let payload_json: String = row.get(4)?;
            let payload: Value = serde_json::from_str(&payload_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            let occurred_at_ms: i64 = row.get(5)?;
            Ok(LegacyEnvelope {
                occurred_at_ms,
                rank: 2,
                source_sequence: Some(sequence),
                source_id: id,
                execution_id: Some(execution_id),
                source: LegacySource::Event,
                data: json!({
                    "kind": row.get::<_, String>(3)?,
                    "payload": payload,
                }),
            })
        })?;
        rows.extend(values.collect::<Result<Vec<_>, _>>()?);
    }
    {
        let mut statement = transaction.prepare(
            "SELECT id, execution_id, tool_call_id, operation, proposal_json, state, detail,
                    created_at_ms, decided_at_ms, settled_at_ms
             FROM tool_approvals WHERE task_id = ?1 ORDER BY created_at_ms, id",
        )?;
        let values = statement.query_map([task_id], |row| {
            let id: String = row.get(0)?;
            let execution_id: String = row.get(1)?;
            let proposal_json: String = row.get(4)?;
            let proposal: Value = serde_json::from_str(&proposal_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            let occurred_at_ms: i64 = row.get(7)?;
            Ok(LegacyEnvelope {
                occurred_at_ms,
                rank: 3,
                source_sequence: None,
                source_id: id,
                execution_id: Some(execution_id),
                source: LegacySource::Approval,
                data: json!({
                    "toolCallId": row.get::<_, String>(2)?,
                    "operation": row.get::<_, String>(3)?,
                    "proposal": proposal,
                    "state": row.get::<_, String>(5)?,
                    "detail": row.get::<_, Option<String>>(6)?,
                    "decidedAtMs": row.get::<_, Option<i64>>(8)?,
                    "settledAtMs": row.get::<_, Option<i64>>(9)?,
                }),
            })
        })?;
        rows.extend(values.collect::<Result<Vec<_>, _>>()?);
    }
    rows.sort_by(|left, right| {
        (
            left.occurred_at_ms,
            left.rank,
            left.source_sequence.unwrap_or(i64::MAX),
            &left.source_id,
        )
            .cmp(&(
                right.occurred_at_ms,
                right.rank,
                right.source_sequence.unwrap_or(i64::MAX),
                &right.source_id,
            ))
    });
    for row in rows {
        let source_key = format!("legacy:{:?}:{}", row.source, row.source_id).to_lowercase();
        append_record(
            transaction,
            task_id,
            row.execution_id.as_deref(),
            TaskJournalEvent::LegacyImported {
                source: row.source,
                source_id: row.source_id,
                source_sequence: row.source_sequence,
                source_occurred_at_ms: row.occurred_at_ms,
                causal_order: LegacyCausalOrder::Unverified,
                data: row.data,
            },
            row.occurred_at_ms,
            Some(&source_key),
        )?;
    }
    Ok(())
}

fn rebuild_task_projections(connection: &mut Connection, task_id: &str) -> Result<(), StoreError> {
    let records = list_records(connection, task_id)?;
    let transaction = connection.transaction()?;
    transaction.execute("DELETE FROM tool_approvals WHERE task_id = ?1", [task_id])?;
    transaction.execute("DELETE FROM task_messages WHERE task_id = ?1", [task_id])?;
    transaction.execute("DELETE FROM executions WHERE task_id = ?1", [task_id])?;

    // Prompt acceptance is deliberately journaled before ExecutionQueued so a crash can
    // never expose a provider launch without its accepted user intent. Projection replay
    // therefore creates every Execution first, then replays the remaining causal facts.
    for record in &records {
        if is_execution_seed(record) {
            apply_record_projection(&transaction, record)?;
        }
    }
    for record in &records {
        if !is_execution_seed(record) {
            apply_record_projection(&transaction, record)?;
        }
    }
    if let Some(timestamp) = records.iter().map(|record| record.created_at_ms).max() {
        touch_task(&transaction, task_id, timestamp)?;
    }
    transaction.commit()?;
    Ok(())
}

fn is_execution_seed(record: &TaskEvent) -> bool {
    matches!(&record.event, TaskJournalEvent::ExecutionQueued { .. })
        || matches!(
            &record.event,
            TaskJournalEvent::LegacyImported {
                source: LegacySource::Execution,
                ..
            }
        )
}

fn apply_record_projection(
    transaction: &Transaction<'_>,
    record: &TaskEvent,
) -> Result<(), StoreError> {
    let execution_id = record.execution_id.as_ref().map(ExecutionId::as_str);
    match &record.event {
        TaskJournalEvent::UserPromptAccepted { content }
        | TaskJournalEvent::AssistantMessageCompleted { content } => {
            let role = if matches!(&record.event, TaskJournalEvent::UserPromptAccepted { .. }) {
                MessageRole::User
            } else {
                MessageRole::Assistant
            };
            insert_message_projection(transaction, record, role, content)?;
        }
        TaskJournalEvent::UserDirectionAccepted { content, .. } => {
            insert_message_projection(transaction, record, MessageRole::User, content)?;
        }
        TaskJournalEvent::ExecutionQueued { execution_spec_id } => {
            let execution_id = execution_id.ok_or_else(|| {
                StoreError::message("ExecutionQueued Journal record is missing an Execution ID")
            })?;
            transaction.execute(
                "INSERT INTO executions
                 (id, task_id, execution_spec_id, state, failure, queued_at_ms, started_at_ms,
                  finished_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 'queued', NULL, ?4, NULL, NULL, ?4)",
                params![
                    execution_id,
                    record.task_id,
                    execution_spec_id,
                    record.created_at_ms
                ],
            )?;
        }
        TaskJournalEvent::ExecutionPreparing => {
            rebuild_execution_state(transaction, execution_id, "preparing", record, None)?;
        }
        TaskJournalEvent::ExecutionStarted | TaskJournalEvent::ExecutionResumed { .. } => {
            rebuild_execution_state(transaction, execution_id, "running", record, None)?;
        }
        TaskJournalEvent::ExecutionWaitingForApproval { .. } => {
            rebuild_execution_state(
                transaction,
                execution_id,
                "waiting-for-approval",
                record,
                None,
            )?;
        }
        TaskJournalEvent::ExecutionCancelling => {
            rebuild_execution_state(transaction, execution_id, "cancelling", record, None)?;
        }
        TaskJournalEvent::ExecutionFinished {
            outcome, failure, ..
        } => {
            let state = match outcome {
                ExecutionOutcome::Completed => "completed",
                ExecutionOutcome::Cancelled => "cancelled",
                ExecutionOutcome::Failed => "failed",
                ExecutionOutcome::Interrupted => "interrupted",
            };
            rebuild_execution_state(transaction, execution_id, state, record, failure.as_deref())?;
        }
        TaskJournalEvent::ToolProposed {
            approval_id,
            tool_call_id,
            proposal,
        } => {
            let execution_id = execution_id.ok_or_else(|| {
                StoreError::message("ToolProposed Journal record is missing an Execution ID")
            })?;
            insert_approval_projection(
                transaction,
                approval_id,
                &record.task_id,
                execution_id,
                tool_call_id,
                proposal,
                record.created_at_ms,
            )?;
        }
        TaskJournalEvent::ApprovalDecided {
            approval_id,
            approved,
        } => {
            let state = if *approved { "approved" } else { "denied" };
            transaction.execute(
                "UPDATE tool_approvals SET state = ?2, decided_at_ms = ?3 WHERE id = ?1",
                params![approval_id, state, record.created_at_ms],
            )?;
        }
        TaskJournalEvent::ToolStarted { approval_id, .. } => {
            transaction.execute(
                "UPDATE tool_approvals SET state = 'executing' WHERE id = ?1",
                [approval_id],
            )?;
        }
        TaskJournalEvent::ToolSettled {
            approval_id,
            state,
            detail,
        } => {
            let state = match state {
                ToolSettlementState::Completed => "completed",
                ToolSettlementState::Failed => "failed",
                ToolSettlementState::Interrupted => "interrupted",
            };
            transaction.execute(
                "UPDATE tool_approvals SET state = ?2, detail = ?3, settled_at_ms = ?4
                 WHERE id = ?1",
                params![approval_id, state, detail, record.created_at_ms],
            )?;
        }
        TaskJournalEvent::ApprovalInterrupted {
            approval_id,
            detail,
        } => {
            transaction.execute(
                "UPDATE tool_approvals SET state = 'interrupted', detail = ?2, settled_at_ms = ?3
                 WHERE id = ?1",
                params![approval_id, detail, record.created_at_ms],
            )?;
        }
        TaskJournalEvent::ToolResultRecorded { .. } => {}
        TaskJournalEvent::LegacyImported {
            source,
            source_id,
            source_sequence: _,
            source_occurred_at_ms,
            data,
            ..
        } => match source {
            LegacySource::Execution => rebuild_legacy_execution(
                transaction,
                &record.task_id,
                execution_id,
                source_occurred_at_ms,
                data,
            )?,
            LegacySource::Message => rebuild_legacy_message(
                transaction,
                &record.task_id,
                execution_id,
                source_id,
                record.sequence,
                *source_occurred_at_ms,
                data,
            )?,
            LegacySource::Event => {}
            LegacySource::Approval => rebuild_legacy_approval(
                transaction,
                &record.task_id,
                execution_id,
                source_id,
                *source_occurred_at_ms,
                data,
            )?,
        },
    }
    Ok(())
}

fn rebuild_execution_state(
    transaction: &Transaction<'_>,
    execution_id: Option<&str>,
    state: &str,
    record: &TaskEvent,
    failure: Option<&str>,
) -> Result<(), StoreError> {
    let execution_id = execution_id.ok_or_else(|| {
        StoreError::message("Execution Journal record is missing an Execution ID")
    })?;
    let terminal = matches!(state, "completed" | "cancelled" | "failed" | "interrupted");
    transaction.execute(
        "UPDATE executions
         SET state = ?2,
             failure = ?3,
             started_at_ms = CASE
                 WHEN started_at_ms IS NULL AND ?2 NOT IN ('queued','cancelled') THEN ?4
                 ELSE started_at_ms END,
             finished_at_ms = CASE WHEN ?5 THEN ?4 ELSE NULL END,
             updated_at_ms = ?4
         WHERE id = ?1",
        params![execution_id, state, failure, record.created_at_ms, terminal,],
    )?;
    Ok(())
}

fn rebuild_legacy_execution(
    transaction: &Transaction<'_>,
    task_id: &str,
    execution_id: Option<&str>,
    occurred_at_ms: &i64,
    data: &Value,
) -> Result<(), StoreError> {
    let execution_id = execution_id
        .ok_or_else(|| StoreError::message("legacy Execution is missing its Execution ID"))?;
    transaction.execute(
        "INSERT INTO executions
         (id, task_id, execution_spec_id, state, failure, queued_at_ms, started_at_ms,
          finished_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            execution_id,
            task_id,
            json_string(data, "executionSpecId")?,
            json_string(data, "state")?,
            json_optional_string(data, "failure")?,
            json_i64(data, "queuedAtMs")?.unwrap_or(*occurred_at_ms),
            json_i64(data, "startedAtMs")?,
            json_i64(data, "finishedAtMs")?,
            json_i64(data, "updatedAtMs")?.unwrap_or(*occurred_at_ms),
        ],
    )?;
    Ok(())
}

fn rebuild_legacy_message(
    transaction: &Transaction<'_>,
    task_id: &str,
    execution_id: Option<&str>,
    source_id: &str,
    journal_sequence: i64,
    occurred_at_ms: i64,
    data: &Value,
) -> Result<(), StoreError> {
    let execution_id = execution_id
        .ok_or_else(|| StoreError::message("legacy message is missing its Execution ID"))?;
    let role = json_string(data, "role")?;
    MessageRole::parse(&role)?;
    transaction.execute(
        "INSERT INTO task_messages
         (id, task_id, execution_id, sequence, role, content, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            source_id,
            task_id,
            execution_id,
            journal_sequence,
            role,
            json_string(data, "content")?,
            occurred_at_ms,
        ],
    )?;
    Ok(())
}

fn rebuild_legacy_approval(
    transaction: &Transaction<'_>,
    task_id: &str,
    execution_id: Option<&str>,
    source_id: &str,
    occurred_at_ms: i64,
    data: &Value,
) -> Result<(), StoreError> {
    let execution_id = execution_id
        .ok_or_else(|| StoreError::message("legacy approval is missing its Execution ID"))?;
    transaction.execute(
        "INSERT INTO tool_approvals
         (id, task_id, execution_id, tool_call_id, operation, proposal_json, state, detail,
          created_at_ms, decided_at_ms, settled_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            source_id,
            task_id,
            execution_id,
            json_string(data, "toolCallId")?,
            json_string(data, "operation")?,
            serde_json::to_string(
                data.get("proposal")
                    .ok_or_else(|| StoreError::message("legacy approval proposal is missing"))?
            )
            .map_err(|error| StoreError::message(error.to_string()))?,
            json_string(data, "state")?,
            json_optional_string(data, "detail")?,
            occurred_at_ms,
            json_i64(data, "decidedAtMs")?,
            json_i64(data, "settledAtMs")?,
        ],
    )?;
    Ok(())
}

fn json_string(value: &Value, key: &str) -> Result<String, StoreError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StoreError::message(format!("legacy Journal field '{key}' is invalid")))
}

fn json_optional_string(value: &Value, key: &str) -> Result<Option<String>, StoreError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(StoreError::message(format!(
            "legacy Journal field '{key}' is invalid"
        ))),
    }
}

fn json_i64(value: &Value, key: &str) -> Result<Option<i64>, StoreError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_i64()
            .map(Some)
            .ok_or_else(|| StoreError::message(format!("legacy Journal field '{key}' is invalid"))),
        _ => Err(StoreError::message(format!(
            "legacy Journal field '{key}' is invalid"
        ))),
    }
}
