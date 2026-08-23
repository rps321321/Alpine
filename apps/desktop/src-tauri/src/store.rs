//! Alpine-owned durable state for Desktop Projects, Tasks, Messages, and Events.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: i64 = 3;
const RESTART_ERROR: &str = "Alpine Desktop restarted while the task was active";

#[derive(Debug)]
pub struct StoreError(String);

impl StoreError {
    pub(crate) fn message(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl Display for StoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self(error.to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopProject {
    pub id: String,
    pub name: String,
    pub root: String,
    pub created_at_ms: i64,
    pub last_opened_at_ms: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Draft,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

impl TaskStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "draft" => Ok(Self::Draft),
            "running" => Ok(Self::Running),
            "cancelling" => Ok(Self::Cancelling),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(StoreError::message(format!(
                "unknown task status '{value}'"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopTask {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub status: TaskStatus,
    pub model_repo_id: String,
    pub model_filename: String,
    pub profile: String,
    pub error: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTask {
    pub project_id: String,
    pub title: String,
    pub model_repo_id: String,
    pub model_filename: String,
    pub profile: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl MessageRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "system" => Ok(Self::System),
            _ => Err(StoreError::message(format!(
                "unknown message role '{value}'"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMessage {
    pub id: String,
    pub task_id: String,
    pub sequence: i64,
    pub role: MessageRole,
    pub content: String,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewTaskMessage {
    pub task_id: String,
    pub role: MessageRole,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub id: String,
    pub task_id: String,
    pub sequence: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewTaskEvent {
    pub task_id: String,
    pub kind: String,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub task: DesktopTask,
    pub messages: Vec<TaskMessage>,
    pub events: Vec<TaskEvent>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalState {
    Pending,
    Approved,
    Denied,
    Executing,
    Completed,
    Failed,
    Interrupted,
}

impl ApprovalState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "denied" => Ok(Self::Denied),
            "executing" => Ok(Self::Executing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(StoreError::message(format!(
                "unknown Tool Approval state '{value}'"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolApproval {
    pub id: String,
    pub task_id: String,
    pub tool_call_id: String,
    pub operation: String,
    pub proposal: Value,
    pub state: ApprovalState,
    pub detail: Option<String>,
    pub created_at_ms: i64,
    pub decided_at_ms: Option<i64>,
    pub settled_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewToolApproval {
    pub task_id: String,
    pub tool_call_id: String,
    pub operation: String,
    pub proposal: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelSource {
    HuggingFace,
    Import,
}

impl ModelSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::HuggingFace => "hugging-face",
            Self::Import => "import",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "hugging-face" => Ok(Self::HuggingFace),
            "import" => Ok(Self::Import),
            _ => Err(StoreError::message(format!(
                "unknown model source '{value}'"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRegistryEntry {
    pub id: String,
    pub source: ModelSource,
    pub repo_id: Option<String>,
    pub revision: Option<String>,
    pub filename: String,
    pub local_path: String,
    pub observed_bytes: u64,
    pub sha256: String,
    pub origin_url: Option<String>,
    pub created_at_ms: i64,
    pub verified_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterModelArtifact {
    pub source: ModelSource,
    pub repo_id: Option<String>,
    pub revision: Option<String>,
    pub filename: String,
    pub local_path: String,
    pub observed_bytes: u64,
    pub sha256: String,
    pub origin_url: Option<String>,
}

pub struct DesktopStore {
    connection: Mutex<Connection>,
}

impl DesktopStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                StoreError::message(format!("failed to create {}: {error}", parent.display()))
            })?;
        }
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&connection)?;
        connection.execute(
            "UPDATE tasks SET status = 'interrupted', error = ?1, updated_at_ms = ?2
             WHERE status IN ('running', 'cancelling')",
            params![RESTART_ERROR, now_ms()],
        )?;
        connection.execute(
            "UPDATE tool_approvals
             SET state = 'interrupted', detail = ?1, settled_at_ms = ?2
             WHERE state IN ('pending', 'approved', 'executing')",
            params![RESTART_ERROR, now_ms()],
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn create_project(
        &self,
        name: &str,
        root: impl AsRef<Path>,
    ) -> Result<DesktopProject, StoreError> {
        let name = validate_nonempty("project name", name, 160)?;
        let canonical = root.as_ref().canonicalize().map_err(|error| {
            StoreError::message(format!(
                "failed to resolve Selected Project {}: {error}",
                root.as_ref().display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(StoreError::message(
                "the Selected Project must be a directory",
            ));
        }
        let root = canonical.to_string_lossy().into_owned();
        let created_at_ms = now_ms();
        let connection = self.connection()?;
        let id = new_id(&connection);
        connection
            .execute(
                "INSERT INTO projects (id, name, root, created_at_ms, last_opened_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, name, root, created_at_ms],
            )
            .map_err(|error| match &error {
                rusqlite::Error::SqliteFailure(value, _)
                    if value.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::message("a Desktop Project Record for this root already exists")
                }
                _ => error.into(),
            })?;
        Ok(DesktopProject {
            id,
            name: name.to_owned(),
            root,
            created_at_ms,
            last_opened_at_ms: created_at_ms,
        })
    }

    pub fn list_projects(&self) -> Result<Vec<DesktopProject>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, name, root, created_at_ms, last_opened_at_ms
             FROM projects ORDER BY last_opened_at_ms DESC, name",
        )?;
        let rows = statement.query_map([], project_from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn create_task(&self, input: CreateTask) -> Result<DesktopTask, StoreError> {
        let title = validate_nonempty("task title", &input.title, 240)?;
        let model_repo_id = validate_nonempty("model repository", &input.model_repo_id, 240)?;
        let model_filename = validate_nonempty("model filename", &input.model_filename, 240)?;
        let profile = validate_nonempty("profile", &input.profile, 96)?;
        let connection = self.connection()?;
        let project_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
            [&input.project_id],
            |row| row.get(0),
        )?;
        if !project_exists {
            return Err(StoreError::message(
                "the Desktop Project Record does not exist",
            ));
        }
        let id = new_id(&connection);
        let created_at_ms = now_ms();
        connection.execute(
            "INSERT INTO tasks
             (id, project_id, title, status, model_repo_id, model_filename, profile, error, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, 'draft', ?4, ?5, ?6, NULL, ?7, ?7)",
            params![
                id,
                input.project_id,
                title,
                model_repo_id,
                model_filename,
                profile,
                created_at_ms
            ],
        )?;
        Ok(DesktopTask {
            id,
            project_id: input.project_id,
            title: title.to_owned(),
            status: TaskStatus::Draft,
            model_repo_id: model_repo_id.to_owned(),
            model_filename: model_filename.to_owned(),
            profile: profile.to_owned(),
            error: None,
            created_at_ms,
            updated_at_ms: created_at_ms,
        })
    }

    pub fn list_tasks(&self, project_id: &str) -> Result<Vec<DesktopTask>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, title, status, model_repo_id, model_filename, profile, error,
                    created_at_ms, updated_at_ms
             FROM tasks WHERE project_id = ?1 ORDER BY updated_at_ms DESC, created_at_ms DESC",
        )?;
        let rows = statement.query_map([project_id], task_from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn project_for_task(&self, task_id: &str) -> Result<DesktopProject, StoreError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT projects.id, projects.name, projects.root, projects.created_at_ms,
                        projects.last_opened_at_ms
                 FROM projects INNER JOIN tasks ON tasks.project_id = projects.id
                 WHERE tasks.id = ?1",
                [task_id],
                project_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                StoreError::message("the Task or its Desktop Project Record does not exist")
            })
    }

    pub fn load_task(&self, task_id: &str) -> Result<Option<TaskDetail>, StoreError> {
        let connection = self.connection()?;
        let task = connection
            .query_row(
                "SELECT id, project_id, title, status, model_repo_id, model_filename, profile, error,
                        created_at_ms, updated_at_ms FROM tasks WHERE id = ?1",
                [task_id],
                task_from_row,
            )
            .optional()?;
        let Some(task) = task else {
            return Ok(None);
        };
        let mut message_statement = connection.prepare(
            "SELECT id, task_id, sequence, role, content, created_at_ms
             FROM task_messages WHERE task_id = ?1 ORDER BY sequence",
        )?;
        let messages = message_statement
            .query_map([task_id], message_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let mut event_statement = connection.prepare(
            "SELECT id, task_id, sequence, kind, payload_json, created_at_ms
             FROM task_events WHERE task_id = ?1 ORDER BY sequence",
        )?;
        let events = event_statement
            .query_map([task_id], event_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(TaskDetail {
            task,
            messages,
            events,
        }))
    }

    pub fn append_message(&self, input: NewTaskMessage) -> Result<TaskMessage, StoreError> {
        let content = validate_nonempty("message content", &input.content, 4 * 1024 * 1024)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let sequence = next_sequence(&transaction, "task_messages", &input.task_id)?;
        let id = new_id(&transaction);
        let created_at_ms = now_ms();
        transaction.execute(
            "INSERT INTO task_messages (id, task_id, sequence, role, content, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                input.task_id,
                sequence,
                input.role.as_str(),
                content,
                created_at_ms
            ],
        )?;
        transaction.execute(
            "UPDATE tasks SET updated_at_ms = ?2 WHERE id = ?1",
            params![input.task_id, created_at_ms],
        )?;
        transaction.commit()?;
        Ok(TaskMessage {
            id,
            task_id: input.task_id,
            sequence,
            role: input.role,
            content: content.to_owned(),
            created_at_ms,
        })
    }

    pub fn append_event(&self, input: NewTaskEvent) -> Result<TaskEvent, StoreError> {
        let kind = validate_event_kind(&input.kind)?;
        let payload_json = serde_json::to_string(&input.payload).map_err(|error| {
            StoreError::message(format!("failed to encode Task Event: {error}"))
        })?;
        if payload_json.len() > 1024 * 1024 {
            return Err(StoreError::message("Task Event payload exceeds 1 MiB"));
        }
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let sequence = next_sequence(&transaction, "task_events", &input.task_id)?;
        let id = new_id(&transaction);
        let created_at_ms = now_ms();
        transaction.execute(
            "INSERT INTO task_events (id, task_id, sequence, kind, payload_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                input.task_id,
                sequence,
                kind,
                payload_json,
                created_at_ms
            ],
        )?;
        transaction.execute(
            "UPDATE tasks SET updated_at_ms = ?2 WHERE id = ?1",
            params![input.task_id, created_at_ms],
        )?;
        transaction.commit()?;
        Ok(TaskEvent {
            id,
            task_id: input.task_id,
            sequence,
            kind: kind.to_owned(),
            payload: input.payload,
            created_at_ms,
        })
    }

    pub fn set_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
        error: Option<&str>,
    ) -> Result<DesktopTask, StoreError> {
        if error.is_some_and(|value| value.len() > 16 * 1024) {
            return Err(StoreError::message("task error exceeds 16 KiB"));
        }
        let connection = self.connection()?;
        let changed = connection.execute(
            "UPDATE tasks SET status = ?2, error = ?3, updated_at_ms = ?4 WHERE id = ?1",
            params![task_id, status.as_str(), error, now_ms()],
        )?;
        if changed == 0 {
            return Err(StoreError::message("the Task does not exist"));
        }
        connection
            .query_row(
                "SELECT id, project_id, title, status, model_repo_id, model_filename, profile, error,
                        created_at_ms, updated_at_ms FROM tasks WHERE id = ?1",
                [task_id],
                task_from_row,
            )
            .map_err(Into::into)
    }

    pub fn request_tool_approval(
        &self,
        input: NewToolApproval,
    ) -> Result<ToolApproval, StoreError> {
        let tool_call_id = validate_nonempty("tool call identifier", &input.tool_call_id, 160)?;
        let operation = validate_operation(&input.operation)?;
        let proposal_json = serde_json::to_string(&input.proposal).map_err(|error| {
            StoreError::message(format!("failed to encode Tool Approval: {error}"))
        })?;
        if proposal_json.len() > 1024 * 1024 {
            return Err(StoreError::message("Tool Approval proposal exceeds 1 MiB"));
        }
        let connection = self.connection()?;
        let task_exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
            [&input.task_id],
            |row| row.get(0),
        )?;
        if !task_exists {
            return Err(StoreError::message("the Task does not exist"));
        }
        let id = new_id(&connection);
        let created_at_ms = now_ms();
        connection.execute(
            "INSERT INTO tool_approvals
             (id, task_id, tool_call_id, operation, proposal_json, state, detail, created_at_ms, decided_at_ms, settled_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, 'pending', NULL, ?6, NULL, NULL)",
            params![id, input.task_id, tool_call_id, operation, proposal_json, created_at_ms],
        )?;
        drop(connection);
        self.get_tool_approval(&id)?
            .ok_or_else(|| StoreError::message("the Tool Approval was not persisted"))
    }

    pub fn get_tool_approval(&self, approval_id: &str) -> Result<Option<ToolApproval>, StoreError> {
        let connection = self.connection()?;
        connection
            .query_row(
                "SELECT id, task_id, tool_call_id, operation, proposal_json, state, detail,
                        created_at_ms, decided_at_ms, settled_at_ms
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
            "SELECT id, task_id, tool_call_id, operation, proposal_json, state, detail,
                    created_at_ms, decided_at_ms, settled_at_ms
             FROM tool_approvals WHERE task_id = ?1 AND state = 'pending'
             ORDER BY created_at_ms, id",
        )?;
        statement
            .query_map([task_id], approval_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn decide_tool_approval(
        &self,
        approval_id: &str,
        approved: bool,
    ) -> Result<ToolApproval, StoreError> {
        let connection = self.connection()?;
        let next = if approved {
            ApprovalState::Approved
        } else {
            ApprovalState::Denied
        };
        let changed = connection.execute(
            "UPDATE tool_approvals SET state = ?2, decided_at_ms = ?3
             WHERE id = ?1 AND state = 'pending'",
            params![approval_id, next.as_str(), now_ms()],
        )?;
        if changed == 0 {
            return Err(StoreError::message(
                "the Tool Approval is missing or has already been decided",
            ));
        }
        drop(connection);
        self.get_tool_approval(approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval no longer exists"))
    }

    pub fn claim_tool_approval(
        &self,
        approval_id: &str,
        operation: &str,
        proposal: &Value,
    ) -> Result<ToolApproval, StoreError> {
        let operation = validate_operation(operation)?;
        let connection = self.connection()?;
        let approval = connection
            .query_row(
                "SELECT id, task_id, tool_call_id, operation, proposal_json, state, detail,
                        created_at_ms, decided_at_ms, settled_at_ms
                 FROM tool_approvals WHERE id = ?1",
                [approval_id],
                approval_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::message("the Tool Approval does not exist"))?;
        if approval.operation != operation || approval.proposal != *proposal {
            return Err(StoreError::message(
                "the Tool Approval does not match the exact proposed operation",
            ));
        }
        let changed = connection.execute(
            "UPDATE tool_approvals SET state = 'executing'
             WHERE id = ?1 AND state = 'approved'",
            [approval_id],
        )?;
        if changed == 0 {
            return Err(StoreError::message(
                "the Tool Approval is not approved or has already been claimed",
            ));
        }
        drop(connection);
        self.get_tool_approval(approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval no longer exists"))
    }

    pub fn settle_tool_approval(
        &self,
        approval_id: &str,
        succeeded: bool,
        detail: Option<&str>,
    ) -> Result<ToolApproval, StoreError> {
        if detail.is_some_and(|value| value.len() > 64 * 1024) {
            return Err(StoreError::message("Tool Approval result exceeds 64 KiB"));
        }
        let connection = self.connection()?;
        let state = if succeeded {
            ApprovalState::Completed
        } else {
            ApprovalState::Failed
        };
        let changed = connection.execute(
            "UPDATE tool_approvals SET state = ?2, detail = ?3, settled_at_ms = ?4
             WHERE id = ?1 AND state = 'executing'",
            params![approval_id, state.as_str(), detail, now_ms()],
        )?;
        if changed == 0 {
            return Err(StoreError::message(
                "the Tool Approval is not executing or has already settled",
            ));
        }
        drop(connection);
        self.get_tool_approval(approval_id)?
            .ok_or_else(|| StoreError::message("the Tool Approval no longer exists"))
    }

    pub fn register_model_artifact(
        &self,
        input: RegisterModelArtifact,
    ) -> Result<ModelRegistryEntry, StoreError> {
        let filename = validate_model_filename(&input.filename)?;
        let path = Path::new(&input.local_path);
        if !path.is_absolute() {
            return Err(StoreError::message(
                "Model Registry local path must be absolute",
            ));
        }
        let canonical = path.canonicalize().map_err(|error| {
            StoreError::message(format!(
                "failed to resolve model artifact {}: {error}",
                path.display()
            ))
        })?;
        let metadata = canonical.metadata().map_err(|error| {
            StoreError::message(format!(
                "failed to inspect model artifact {}: {error}",
                canonical.display()
            ))
        })?;
        if !metadata.is_file()
            || metadata.len() != input.observed_bytes
            || input.observed_bytes == 0
        {
            return Err(StoreError::message(
                "Model Registry byte count must match a non-empty local file",
            ));
        }
        let observed_bytes = i64::try_from(input.observed_bytes)
            .map_err(|_| StoreError::message("model artifact byte count exceeds SQLite range"))?;
        let sha256 = validate_digest(&input.sha256)?;
        let repo_id = validate_optional("model repository", input.repo_id.as_deref(), 240)?;
        let revision =
            validate_optional_identifier("model revision", input.revision.as_deref(), 160)?;
        let origin_url = validate_optional("model origin URL", input.origin_url.as_deref(), 2048)?;
        if input.source == ModelSource::HuggingFace && (repo_id.is_none() || revision.is_none()) {
            return Err(StoreError::message(
                "a Hugging Face Model Registry entry requires repository and revision provenance",
            ));
        }
        let local_path = canonical.to_string_lossy().into_owned();
        let connection = self.connection()?;
        let id = connection
            .query_row(
                "SELECT id FROM model_artifacts WHERE local_path = ?1",
                [&local_path],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| new_id(&connection));
        let now = now_ms();
        connection.execute(
            "INSERT INTO model_artifacts
             (id, source, repo_id, revision, filename, local_path, observed_bytes, sha256, origin_url, created_at_ms, verified_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(local_path) DO UPDATE SET
                source = excluded.source,
                repo_id = excluded.repo_id,
                revision = excluded.revision,
                filename = excluded.filename,
                observed_bytes = excluded.observed_bytes,
                sha256 = excluded.sha256,
                origin_url = excluded.origin_url,
                verified_at_ms = excluded.verified_at_ms",
            params![
                id,
                input.source.as_str(),
                repo_id,
                revision,
                filename,
                local_path,
                observed_bytes,
                sha256,
                origin_url,
                now
            ],
        )?;
        connection
            .query_row(
                "SELECT id, source, repo_id, revision, filename, local_path, observed_bytes,
                        sha256, origin_url, created_at_ms, verified_at_ms
                 FROM model_artifacts WHERE local_path = ?1",
                [&local_path],
                model_artifact_from_row,
            )
            .map_err(Into::into)
    }

    pub fn list_model_artifacts(&self) -> Result<Vec<ModelRegistryEntry>, StoreError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT id, source, repo_id, revision, filename, local_path, observed_bytes,
                    sha256, origin_url, created_at_ms, verified_at_ms
             FROM model_artifacts ORDER BY verified_at_ms DESC, filename",
        )?;
        statement
            .query_map([], model_artifact_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StoreError> {
        self.connection
            .lock()
            .map_err(|_| StoreError::message("the desktop state database is unavailable"))
    }
}

fn migrate(connection: &Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS desktop_schema (
            singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
            version INTEGER NOT NULL
        );
        INSERT OR IGNORE INTO desktop_schema (singleton, version) VALUES (1, 0);",
    )?;
    let version: i64 = connection.query_row(
        "SELECT version FROM desktop_schema WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if version > SCHEMA_VERSION {
        return Err(StoreError::message(format!(
            "desktop state schema {version} is newer than supported schema {SCHEMA_VERSION}"
        )));
    }
    if version == 0 {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
            CREATE TABLE projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                root TEXT NOT NULL UNIQUE,
                created_at_ms INTEGER NOT NULL,
                last_opened_at_ms INTEGER NOT NULL
            );
            CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                title TEXT NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('draft','running','cancelling','completed','cancelled','failed','interrupted')),
                model_repo_id TEXT NOT NULL,
                model_filename TEXT NOT NULL,
                profile TEXT NOT NULL,
                error TEXT,
                created_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE INDEX tasks_project_updated ON tasks(project_id, updated_at_ms DESC);
            CREATE TABLE task_messages (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL,
                role TEXT NOT NULL CHECK (role IN ('user','assistant','system')),
                content TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                UNIQUE(task_id, sequence)
            );
            CREATE TABLE task_events (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                sequence INTEGER NOT NULL,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                UNIQUE(task_id, sequence)
            );
            UPDATE desktop_schema SET version = 1 WHERE singleton = 1;
            COMMIT;",
        )?;
    }
    let version: i64 = connection.query_row(
        "SELECT version FROM desktop_schema WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if version == 1 {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
            CREATE TABLE tool_approvals (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                tool_call_id TEXT NOT NULL,
                operation TEXT NOT NULL CHECK (operation IN ('edit', 'shell')),
                proposal_json TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('pending','approved','denied','executing','completed','failed','interrupted')),
                detail TEXT,
                created_at_ms INTEGER NOT NULL,
                decided_at_ms INTEGER,
                settled_at_ms INTEGER,
                UNIQUE(task_id, tool_call_id)
            );
            CREATE INDEX tool_approvals_task_state ON tool_approvals(task_id, state, created_at_ms);
            UPDATE desktop_schema SET version = 2 WHERE singleton = 1;
            COMMIT;",
        )?;
    }
    let version: i64 = connection.query_row(
        "SELECT version FROM desktop_schema WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    if version == 2 {
        connection.execute_batch(
            "BEGIN IMMEDIATE;
            CREATE TABLE model_artifacts (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL CHECK (source IN ('hugging-face', 'import')),
                repo_id TEXT,
                revision TEXT,
                filename TEXT NOT NULL,
                local_path TEXT NOT NULL UNIQUE,
                observed_bytes INTEGER NOT NULL CHECK (observed_bytes > 0),
                sha256 TEXT NOT NULL,
                origin_url TEXT,
                created_at_ms INTEGER NOT NULL,
                verified_at_ms INTEGER NOT NULL
            );
            CREATE INDEX model_artifacts_filename ON model_artifacts(filename);
            UPDATE desktop_schema SET version = 3 WHERE singleton = 1;
            COMMIT;",
        )?;
    }
    Ok(())
}

fn validate_nonempty<'a>(label: &str, value: &'a str, max: usize) -> Result<&'a str, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max {
        return Err(StoreError::message(format!(
            "{label} must contain between 1 and {max} bytes"
        )));
    }
    Ok(value)
}

fn validate_event_kind(value: &str) -> Result<&str, StoreError> {
    let value = validate_nonempty("Task Event kind", value, 96)?;
    if value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
    }) {
        Ok(value)
    } else {
        Err(StoreError::message("Task Event kind is invalid"))
    }
}

fn validate_operation(value: &str) -> Result<&str, StoreError> {
    match value {
        "edit" | "shell" => Ok(value),
        _ => Err(StoreError::message("Tool Approval operation is invalid")),
    }
}

fn validate_model_filename(value: &str) -> Result<&str, StoreError> {
    let value = validate_nonempty("model filename", value, 255)?;
    let path = Path::new(value);
    if path.file_name().is_some_and(|name| name == value)
        && value.to_ascii_lowercase().ends_with(".gguf")
    {
        Ok(value)
    } else {
        Err(StoreError::message(
            "Model Registry filename must be one safe GGUF filename",
        ))
    }
}

fn validate_digest(value: &str) -> Result<&str, StoreError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(value)
    } else {
        Err(StoreError::message(
            "Model Registry SHA-256 digest is invalid",
        ))
    }
}

fn validate_optional<'a>(
    label: &str,
    value: Option<&'a str>,
    max: usize,
) -> Result<Option<&'a str>, StoreError> {
    value
        .map(|value| validate_nonempty(label, value, max))
        .transpose()
}

fn validate_optional_identifier<'a>(
    label: &str,
    value: Option<&'a str>,
    max: usize,
) -> Result<Option<&'a str>, StoreError> {
    let value = validate_optional(label, value, max)?;
    if value.is_none_or(|value| {
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    }) {
        Ok(value)
    } else {
        Err(StoreError::message(format!("{label} is invalid")))
    }
}

fn new_id(connection: &Connection) -> String {
    connection
        .query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))
        .expect("SQLite random identifier generation should not fail")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn next_sequence(
    transaction: &Transaction<'_>,
    table: &str,
    task_id: &str,
) -> Result<i64, StoreError> {
    debug_assert!(matches!(table, "task_messages" | "task_events"));
    let task_exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
        [task_id],
        |row| row.get(0),
    )?;
    if !task_exists {
        return Err(StoreError::message("the Task does not exist"));
    }
    let statement =
        format!("SELECT COALESCE(MAX(sequence), 0) + 1 FROM {table} WHERE task_id = ?1");
    transaction
        .query_row(&statement, [task_id], |row| row.get(0))
        .map_err(Into::into)
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesktopProject> {
    Ok(DesktopProject {
        id: row.get(0)?,
        name: row.get(1)?,
        root: row.get(2)?,
        created_at_ms: row.get(3)?,
        last_opened_at_ms: row.get(4)?,
    })
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DesktopTask> {
    let status: String = row.get(3)?;
    let status = TaskStatus::parse(&status).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(DesktopTask {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        status,
        model_repo_id: row.get(4)?,
        model_filename: row.get(5)?,
        profile: row.get(6)?,
        error: row.get(7)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
    })
}

fn message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskMessage> {
    let role: String = row.get(3)?;
    let role = MessageRole::parse(&role).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(TaskMessage {
        id: row.get(0)?,
        task_id: row.get(1)?,
        sequence: row.get(2)?,
        role,
        content: row.get(4)?,
        created_at_ms: row.get(5)?,
    })
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEvent> {
    let payload_json: String = row.get(4)?;
    let payload = serde_json::from_str(&payload_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(TaskEvent {
        id: row.get(0)?,
        task_id: row.get(1)?,
        sequence: row.get(2)?,
        kind: row.get(3)?,
        payload,
        created_at_ms: row.get(5)?,
    })
}

fn approval_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolApproval> {
    let proposal_json: String = row.get(4)?;
    let proposal = serde_json::from_str(&proposal_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let state: String = row.get(5)?;
    let state = ApprovalState::parse(&state).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(ToolApproval {
        id: row.get(0)?,
        task_id: row.get(1)?,
        tool_call_id: row.get(2)?,
        operation: row.get(3)?,
        proposal,
        state,
        detail: row.get(6)?,
        created_at_ms: row.get(7)?,
        decided_at_ms: row.get(8)?,
        settled_at_ms: row.get(9)?,
    })
}

fn model_artifact_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRegistryEntry> {
    let source: String = row.get(1)?;
    let source = ModelSource::parse(&source).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let observed: i64 = row.get(6)?;
    let observed_bytes = u64::try_from(observed).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(ModelRegistryEntry {
        id: row.get(0)?,
        source,
        repo_id: row.get(2)?,
        revision: row.get(3)?,
        filename: row.get(4)?,
        local_path: row.get(5)?,
        observed_bytes,
        sha256: row.get(7)?,
        origin_url: row.get(8)?,
        created_at_ms: row.get(9)?,
        verified_at_ms: row.get(10)?,
    })
}
