//! Durable execution identity, specification, lifecycle, and task projections.

use super::{
    DesktopStore, DesktopTask, StoreError, TaskStatus, new_id, now_ms, validate_nonempty,
    validate_optional_identifier,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ExecutionId(pub String);

impl ExecutionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ExecutionId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskSummary {
    #[default]
    Ready,
    Active,
    Done,
    NeedsAttention,
}

impl TaskSummary {
    pub(super) fn from_legacy_status(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Running | TaskStatus::Cancelling => Self::Active,
            TaskStatus::Completed => Self::Done,
            TaskStatus::Failed | TaskStatus::Interrupted => Self::NeedsAttention,
            TaskStatus::Draft | TaskStatus::Cancelled => Self::Ready,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionState {
    Queued,
    Preparing,
    Running,
    WaitingForApproval,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl ExecutionState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::WaitingForApproval => "waiting-for-approval",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "queued" => Ok(Self::Queued),
            "preparing" => Ok(Self::Preparing),
            "running" => Ok(Self::Running),
            "waiting-for-approval" => Ok(Self::WaitingForApproval),
            "cancelling" => Ok(Self::Cancelling),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(StoreError::message(format!(
                "unknown Execution state '{value}'"
            ))),
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Preparing
                | Self::Running
                | Self::WaitingForApproval
                | Self::Cancelling
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::Interrupted
        )
    }

    fn allows(self, next: Self) -> bool {
        match self {
            Self::Queued => matches!(
                next,
                Self::Preparing
                    | Self::Cancelling
                    | Self::Cancelled
                    | Self::Failed
                    | Self::Interrupted
            ),
            Self::Preparing => matches!(
                next,
                Self::Running | Self::Cancelling | Self::Failed | Self::Interrupted
            ),
            Self::Running => matches!(
                next,
                Self::WaitingForApproval
                    | Self::Cancelling
                    | Self::Completed
                    | Self::Failed
                    | Self::Interrupted
            ),
            Self::WaitingForApproval => matches!(
                next,
                Self::Running | Self::Cancelling | Self::Failed | Self::Interrupted
            ),
            Self::Cancelling => {
                matches!(next, Self::Cancelled | Self::Failed | Self::Interrupted)
            }
            Self::Completed | Self::Cancelled | Self::Failed | Self::Interrupted => false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewExecutionSpecification {
    pub model_registry_id: String,
    pub model_repo_id: String,
    pub model_revision: Option<String>,
    pub model_filename: String,
    pub model_sha256: String,
    pub session_config_sha256: String,
    pub profile_name: String,
    pub profile_sha256: String,
    pub runtime_name: String,
    pub runtime_identity: String,
    pub adapter_identity: String,
    pub policy_identity: String,
    pub context_window: u32,
    pub max_tokens: u32,
    pub temperature_millis: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionSpecification {
    pub id: String,
    pub task_id: String,
    pub model_registry_id: Option<String>,
    pub model_repo_id: String,
    pub model_revision: Option<String>,
    pub model_filename: String,
    pub model_sha256: Option<String>,
    pub session_config_sha256: Option<String>,
    pub profile_name: String,
    pub profile_sha256: Option<String>,
    pub runtime_name: String,
    pub runtime_identity: String,
    pub adapter_identity: String,
    pub policy_identity: String,
    pub context_window: u32,
    pub max_tokens: u32,
    pub temperature_millis: i64,
    pub legacy_unverified: bool,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateExecution {
    pub task_id: String,
    pub specification: NewExecutionSpecification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    pub id: ExecutionId,
    pub task_id: String,
    pub execution_spec_id: String,
    pub specification: ExecutionSpecification,
    pub state: ExecutionState,
    pub failure: Option<String>,
    pub queued_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub updated_at_ms: i64,
}

impl DesktopStore {
    pub fn create_execution(&self, input: CreateExecution) -> Result<Execution, StoreError> {
        let task_id = validate_nonempty("Task identifier", &input.task_id, 160)?;
        let specification = validate_new_specification(input.specification)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
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
        let execution_id = ExecutionId(new_id(&transaction));
        let specification_id = new_id(&transaction);
        let created_at_ms = now_ms();
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
                created_at_ms,
            ],
        )?;
        transaction.execute(
            "INSERT INTO executions
             (id, task_id, execution_spec_id, state, failure, queued_at_ms, started_at_ms,
              finished_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 'queued', NULL, ?4, NULL, NULL, ?4)",
            params![
                execution_id.as_str(),
                task_id,
                specification_id,
                created_at_ms
            ],
        )?;
        transaction.execute(
            "UPDATE tasks SET updated_at_ms = ?2 WHERE id = ?1",
            params![task_id, created_at_ms],
        )?;
        let execution = load_execution(&transaction, execution_id.as_str())?
            .ok_or_else(|| StoreError::message("the Execution was not persisted"))?;
        transaction.commit()?;
        Ok(execution)
    }

    pub fn get_execution(&self, execution_id: &str) -> Result<Option<Execution>, StoreError> {
        let connection = self.connection()?;
        load_execution(&connection, execution_id)
    }

    pub fn list_executions(&self, task_id: &str) -> Result<Vec<Execution>, StoreError> {
        let connection = self.connection()?;
        list_task_executions(&connection, task_id)
    }

    pub fn transition_execution(
        &self,
        execution_id: &str,
        next: ExecutionState,
        failure: Option<&str>,
    ) -> Result<Execution, StoreError> {
        let execution_id = validate_nonempty("Execution identifier", execution_id, 160)?;
        let failure = validate_transition_failure(next, failure)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let current = load_execution(&transaction, execution_id)?
            .ok_or_else(|| StoreError::message("the Execution does not exist"))?;
        if !current.state.allows(next) {
            return Err(StoreError::message(format!(
                "invalid Execution transition from '{}' to '{}'",
                current.state.as_str(),
                next.as_str()
            )));
        }
        let timestamp = now_ms();
        let started_at_ms = current.started_at_ms.or_else(|| {
            (!matches!(next, ExecutionState::Queued | ExecutionState::Cancelled))
                .then_some(timestamp)
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
        transaction.execute(
            "UPDATE tasks SET updated_at_ms = ?2 WHERE id = ?1",
            params![current.task_id, timestamp],
        )?;
        let execution = load_execution(&transaction, execution_id)?
            .ok_or_else(|| StoreError::message("the Execution no longer exists"))?;
        transaction.commit()?;
        Ok(execution)
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

fn validate_new_specification(
    mut specification: NewExecutionSpecification,
) -> Result<NewExecutionSpecification, StoreError> {
    specification.model_registry_id = validate_nonempty(
        "Execution model registry identifier",
        &specification.model_registry_id,
        160,
    )?
    .to_owned();
    specification.model_repo_id = validate_nonempty(
        "Execution model repository",
        &specification.model_repo_id,
        240,
    )?
    .to_owned();
    specification.model_revision = validate_optional_identifier(
        "Execution model revision",
        specification.model_revision.as_deref(),
        160,
    )?
    .map(str::to_owned);
    specification.model_filename = validate_nonempty(
        "Execution model filename",
        &specification.model_filename,
        255,
    )?
    .to_owned();
    specification.model_sha256 =
        validate_sha256("Execution model SHA-256", &specification.model_sha256)?.to_owned();
    specification.session_config_sha256 = validate_sha256(
        "Execution Session Config SHA-256",
        &specification.session_config_sha256,
    )?
    .to_owned();
    specification.profile_name =
        validate_nonempty("Execution Profile", &specification.profile_name, 96)?.to_owned();
    specification.profile_sha256 =
        validate_sha256("Execution Profile SHA-256", &specification.profile_sha256)?.to_owned();
    specification.runtime_name =
        validate_nonempty("Execution Runtime", &specification.runtime_name, 96)?.to_owned();
    specification.runtime_identity = validate_sha256(
        "Execution Runtime identity",
        &specification.runtime_identity,
    )?
    .to_owned();
    specification.adapter_identity = validate_nonempty(
        "Execution adapter identity",
        &specification.adapter_identity,
        240,
    )?
    .to_owned();
    specification.policy_identity = validate_nonempty(
        "Execution policy identity",
        &specification.policy_identity,
        240,
    )?
    .to_owned();
    if specification.context_window == 0 || specification.max_tokens == 0 {
        return Err(StoreError::message(
            "Execution context and output limits must be positive",
        ));
    }
    if !(0..=2_000).contains(&specification.temperature_millis) {
        return Err(StoreError::message(
            "Execution temperature must be between 0 and 2000 millis",
        ));
    }
    Ok(specification)
}

fn validate_sha256<'a>(label: &str, value: &'a str) -> Result<&'a str, StoreError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(StoreError::message(format!("{label} is invalid")))
    }
}

fn validate_transition_failure(
    next: ExecutionState,
    failure: Option<&str>,
) -> Result<Option<&str>, StoreError> {
    match next {
        ExecutionState::Failed | ExecutionState::Interrupted => failure
            .map(|value| validate_nonempty("Execution failure", value, 64 * 1024))
            .transpose()?
            .ok_or_else(|| {
                StoreError::message("failed and interrupted Executions require a failure detail")
            })
            .map(Some),
        _ if failure.is_some() => Err(StoreError::message(
            "only failed or interrupted Executions may record a failure detail",
        )),
        _ => Ok(None),
    }
}

pub(super) fn ensure_execution_for_task(
    connection: &Connection,
    task_id: &str,
    execution_id: &str,
) -> Result<(), StoreError> {
    let matches: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM executions WHERE id = ?1 AND task_id = ?2
         )",
        params![execution_id, task_id],
        |row| row.get(0),
    )?;
    if matches {
        Ok(())
    } else {
        Err(StoreError::message(
            "the Execution does not belong to the Task",
        ))
    }
}

pub(super) fn task_has_executions(
    connection: &Connection,
    task_id: &str,
) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM executions WHERE task_id = ?1)",
            [task_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(super) fn project_task(
    connection: &Connection,
    mut task: DesktopTask,
) -> Result<DesktopTask, StoreError> {
    let active = connection
        .query_row(
            "SELECT id, state FROM executions
             WHERE task_id = ?1
               AND state IN ('queued','preparing','running','waiting-for-approval','cancelling')
             ORDER BY updated_at_ms DESC, queued_at_ms DESC, id DESC LIMIT 1",
            [&task.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let latest = connection
        .query_row(
            "SELECT id, state, failure FROM executions
             WHERE task_id = ?1
             ORDER BY queued_at_ms DESC, id DESC LIMIT 1",
            [&task.id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;

    task.active_execution_id = active.as_ref().map(|(id, _)| ExecutionId(id.clone()));
    task.latest_execution_id = latest.as_ref().map(|(id, _, _)| ExecutionId(id.clone()));

    if let Some((_, state)) = active {
        let state = ExecutionState::parse(&state)?;
        task.summary = TaskSummary::Active;
        task.status = if state == ExecutionState::Cancelling {
            TaskStatus::Cancelling
        } else {
            TaskStatus::Running
        };
        task.error = None;
        return Ok(task);
    }

    if let Some((_, state, failure)) = latest {
        let state = ExecutionState::parse(&state)?;
        match state {
            ExecutionState::Completed => {
                task.summary = TaskSummary::Done;
                task.status = TaskStatus::Completed;
                task.error = None;
            }
            ExecutionState::Failed => {
                task.summary = TaskSummary::NeedsAttention;
                task.status = TaskStatus::Failed;
                task.error = failure;
            }
            ExecutionState::Interrupted => {
                task.summary = TaskSummary::NeedsAttention;
                task.status = TaskStatus::Interrupted;
                task.error = failure;
            }
            ExecutionState::Cancelled => {
                task.summary = TaskSummary::Ready;
                task.status = TaskStatus::Cancelled;
                task.error = None;
            }
            state if state.is_active() => {
                return Err(StoreError::message(
                    "an active Execution was omitted from the active projection",
                ));
            }
            _ => {}
        }
    }
    Ok(task)
}

pub(super) fn interrupt_unsettled(
    connection: &Connection,
    failure: &str,
) -> Result<(), StoreError> {
    let timestamp = now_ms();
    connection.execute(
        "UPDATE executions
         SET state = 'interrupted', failure = ?1, finished_at_ms = ?2, updated_at_ms = ?2
         WHERE state IN ('queued','preparing','running','waiting-for-approval','cancelling')",
        params![failure, timestamp],
    )?;
    connection.execute(
        "UPDATE tasks SET updated_at_ms = ?1
         WHERE id IN (
            SELECT task_id FROM executions
            WHERE state = 'interrupted' AND updated_at_ms = ?1
         )",
        [timestamp],
    )?;
    Ok(())
}

pub(super) fn list_task_executions(
    connection: &Connection,
    task_id: &str,
) -> Result<Vec<Execution>, StoreError> {
    let sql = format!("{EXECUTION_SELECT} WHERE e.task_id = ?1 ORDER BY e.queued_at_ms, e.id");
    let mut statement = connection.prepare(&sql)?;
    statement
        .query_map([task_id], execution_from_row)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn load_execution(
    connection: &Connection,
    execution_id: &str,
) -> Result<Option<Execution>, StoreError> {
    let sql = format!("{EXECUTION_SELECT} WHERE e.id = ?1");
    connection
        .query_row(&sql, [execution_id], execution_from_row)
        .optional()
        .map_err(Into::into)
}

const EXECUTION_SELECT: &str = "SELECT e.id, e.task_id, e.execution_spec_id, e.state, e.failure,
            e.queued_at_ms, e.started_at_ms, e.finished_at_ms, e.updated_at_ms,
            s.id, s.task_id, s.model_registry_id, s.model_repo_id, s.model_revision,
            s.model_filename, s.model_sha256, s.session_config_sha256, s.profile_name,
            s.profile_sha256, s.runtime_name, s.runtime_identity, s.adapter_identity,
            s.policy_identity, s.context_window, s.max_tokens, s.temperature_millis,
            s.legacy_unverified, s.created_at_ms
     FROM executions e
     INNER JOIN execution_specs s ON s.id = e.execution_spec_id";

fn execution_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Execution> {
    let state: String = row.get(3)?;
    let state = ExecutionState::parse(&state).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let context_window = checked_u32(row, 23)?;
    let max_tokens = checked_u32(row, 24)?;
    Ok(Execution {
        id: ExecutionId(row.get(0)?),
        task_id: row.get(1)?,
        execution_spec_id: row.get(2)?,
        state,
        failure: row.get(4)?,
        queued_at_ms: row.get(5)?,
        started_at_ms: row.get(6)?,
        finished_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
        specification: ExecutionSpecification {
            id: row.get(9)?,
            task_id: row.get(10)?,
            model_registry_id: row.get(11)?,
            model_repo_id: row.get(12)?,
            model_revision: row.get(13)?,
            model_filename: row.get(14)?,
            model_sha256: row.get(15)?,
            session_config_sha256: row.get(16)?,
            profile_name: row.get(17)?,
            profile_sha256: row.get(18)?,
            runtime_name: row.get(19)?,
            runtime_identity: row.get(20)?,
            adapter_identity: row.get(21)?,
            policy_identity: row.get(22)?,
            context_window,
            max_tokens,
            temperature_millis: row.get(25)?,
            legacy_unverified: row.get::<_, i64>(26)? != 0,
            created_at_ms: row.get(27)?,
        },
    })
}

fn checked_u32(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u32> {
    let value: i64 = row.get(index)?;
    u32::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

pub(super) fn migrate_v4(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        r#"BEGIN IMMEDIATE;
        CREATE TABLE execution_specs (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            model_registry_id TEXT,
            model_repo_id TEXT NOT NULL,
            model_revision TEXT,
            model_filename TEXT NOT NULL,
            model_sha256 TEXT,
            session_config_sha256 TEXT,
            profile_name TEXT NOT NULL,
            profile_sha256 TEXT,
            runtime_name TEXT NOT NULL,
            runtime_identity TEXT NOT NULL,
            adapter_identity TEXT NOT NULL,
            policy_identity TEXT NOT NULL,
            context_window INTEGER NOT NULL CHECK (context_window >= 0),
            max_tokens INTEGER NOT NULL CHECK (max_tokens >= 0),
            temperature_millis INTEGER NOT NULL,
            legacy_unverified INTEGER NOT NULL CHECK (legacy_unverified IN (0,1)),
            created_at_ms INTEGER NOT NULL
        );
        CREATE INDEX execution_specs_task_created
            ON execution_specs(task_id, created_at_ms, id);
        CREATE TABLE executions (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            execution_spec_id TEXT NOT NULL UNIQUE REFERENCES execution_specs(id) ON DELETE CASCADE,
            state TEXT NOT NULL CHECK (state IN (
                'queued','preparing','running','waiting-for-approval','cancelling',
                'completed','cancelled','failed','interrupted'
            )),
            failure TEXT,
            queued_at_ms INTEGER NOT NULL,
            started_at_ms INTEGER,
            finished_at_ms INTEGER,
            updated_at_ms INTEGER NOT NULL
        );
        CREATE INDEX executions_task_history
            ON executions(task_id, queued_at_ms DESC, id DESC);
        CREATE UNIQUE INDEX executions_one_active_per_task
            ON executions(task_id)
            WHERE state IN ('queued','preparing','running','waiting-for-approval','cancelling');

        INSERT INTO execution_specs
        (id, task_id, model_registry_id, model_repo_id, model_revision, model_filename,
         model_sha256, session_config_sha256, profile_name, profile_sha256, runtime_name,
         runtime_identity, adapter_identity, policy_identity, context_window, max_tokens,
         temperature_millis, legacy_unverified, created_at_ms)
        SELECT 'legacy-spec-' || tasks.id, tasks.id, NULL, tasks.model_repo_id, NULL,
               tasks.model_filename, NULL, NULL, tasks.profile, NULL, 'legacy-unverified',
               'legacy-unverified', 'legacy-unverified', 'legacy-unverified', 0, 0, 0, 1,
               tasks.created_at_ms
        FROM tasks
        WHERE tasks.status <> 'draft'
           OR EXISTS (SELECT 1 FROM task_messages WHERE task_messages.task_id = tasks.id)
           OR EXISTS (SELECT 1 FROM task_events WHERE task_events.task_id = tasks.id)
           OR EXISTS (SELECT 1 FROM tool_approvals WHERE tool_approvals.task_id = tasks.id);

        INSERT INTO executions
        (id, task_id, execution_spec_id, state, failure, queued_at_ms, started_at_ms,
         finished_at_ms, updated_at_ms)
        SELECT 'legacy-execution-' || tasks.id, tasks.id, 'legacy-spec-' || tasks.id,
               CASE tasks.status
                   WHEN 'running' THEN 'interrupted'
                   WHEN 'cancelling' THEN 'interrupted'
                   WHEN 'draft' THEN 'interrupted'
                   ELSE tasks.status
               END,
               CASE
                   WHEN tasks.status IN ('running','cancelling')
                       THEN 'Alpine Desktop restarted while the execution was active'
                   WHEN tasks.status = 'draft'
                       THEN 'Legacy task activity had no authoritative Execution lifecycle or exact execution identity'
                   WHEN tasks.status IN ('failed','interrupted')
                       THEN COALESCE(NULLIF(tasks.error, ''),
                           'Legacy task ended without a recorded failure detail')
                   ELSE NULL
               END,
               tasks.created_at_ms, NULL, tasks.updated_at_ms, tasks.updated_at_ms
        FROM tasks
        WHERE EXISTS (
            SELECT 1 FROM execution_specs WHERE execution_specs.task_id = tasks.id
        );

        ALTER TABLE task_messages RENAME TO task_messages_v3;
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
        SELECT id, task_id, 'legacy-execution-' || task_id, sequence, role, content,
               created_at_ms
        FROM task_messages_v3;
        DROP TABLE task_messages_v3;

        ALTER TABLE task_events RENAME TO task_events_v3;
        CREATE TABLE task_events (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            execution_id TEXT NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
            sequence INTEGER NOT NULL,
            kind TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            UNIQUE(task_id, sequence)
        );
        INSERT INTO task_events
            (id, task_id, execution_id, sequence, kind, payload_json, created_at_ms)
        SELECT id, task_id, 'legacy-execution-' || task_id, sequence, kind, payload_json,
               created_at_ms
        FROM task_events_v3;
        DROP TABLE task_events_v3;

        ALTER TABLE tool_approvals RENAME TO tool_approvals_v3;
        CREATE TABLE tool_approvals (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            execution_id TEXT NOT NULL REFERENCES executions(id) ON DELETE CASCADE,
            tool_call_id TEXT NOT NULL,
            operation TEXT NOT NULL CHECK (operation IN ('edit', 'shell')),
            proposal_json TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN (
                'pending','approved','denied','executing','completed','failed','interrupted'
            )),
            detail TEXT,
            created_at_ms INTEGER NOT NULL,
            decided_at_ms INTEGER,
            settled_at_ms INTEGER,
            UNIQUE(execution_id, tool_call_id)
        );
        INSERT INTO tool_approvals
            (id, task_id, execution_id, tool_call_id, operation, proposal_json, state,
             detail, created_at_ms, decided_at_ms, settled_at_ms)
        SELECT id, task_id, 'legacy-execution-' || task_id, tool_call_id, operation,
               proposal_json, state, detail, created_at_ms, decided_at_ms, settled_at_ms
        FROM tool_approvals_v3;
        DROP TABLE tool_approvals_v3;
        CREATE INDEX tool_approvals_task_state
            ON tool_approvals(task_id, state, created_at_ms);
        CREATE INDEX tool_approvals_execution_state
            ON tool_approvals(execution_id, state, created_at_ms);

        UPDATE desktop_schema SET version = 4 WHERE singleton = 1;
        COMMIT;"#,
    )?;
    Ok(())
}
