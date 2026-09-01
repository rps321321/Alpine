//! Host-owned task execution authority and the isolated Pi worker protocol.

use crate::{
    PiLaunchConfig, resolve_pi_launch_blocking,
    store::{
        ApprovalState, DesktopStore, Execution, ExecutionState, MessageRole, NewToolApproval,
        TaskEvent, TaskMessage, ToolApproval, ToolApprovalDecision, ToolProposal, ToolResult,
        UserDirection,
    },
    workspace::{self, WorkspaceEdit, WorkspaceEditResult, WorkspaceShell, WorkspaceShellResult},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State, Webview, ipc::Channel};

const MAIN_WEBVIEW: &str = "main";
const AGENT_WORKER_WEBVIEW: &str = "agent-worker";
const MAX_PROMPT_BYTES: usize = 4 * 1024 * 1024;
const MAX_DIRECTION_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitPromptResult {
    pub execution: Execution,
    pub prompt_message: TaskMessage,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum AgentWorkerCommand {
    Start {
        task_id: String,
        execution_id: String,
        prompt: String,
        history: Vec<TaskMessage>,
        config: Box<PiLaunchConfig>,
    },
    Cancel {
        execution_id: String,
    },
    Steer {
        execution_id: String,
        text: String,
    },
    FollowUp {
        execution_id: String,
        text: String,
    },
    ApprovalDecision {
        approval_id: String,
        approved: bool,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentWorkerEvent {
    sequence: u64,
    #[serde(flatten)]
    event: AgentWorkerEventKind,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum AgentWorkerEventKind {
    Started {
        task_id: String,
        execution_id: String,
    },
    Delta {
        task_id: String,
        execution_id: String,
        delta: String,
    },
    Message {
        task_id: String,
        execution_id: String,
        role: MessageRole,
        content: String,
    },
    Trace {
        task_id: String,
        execution_id: String,
        kind: String,
        payload: Value,
    },
    Completed {
        task_id: String,
        execution_id: String,
        duration_ms: u64,
        response_characters: u64,
    },
    Cancelled {
        task_id: String,
        execution_id: String,
        duration_ms: u64,
        response_characters: u64,
    },
    Failed {
        task_id: String,
        execution_id: String,
        error: String,
        duration_ms: u64,
        response_characters: u64,
    },
}

impl AgentWorkerEventKind {
    fn identity(&self) -> (&str, &str) {
        match self {
            Self::Started {
                task_id,
                execution_id,
            }
            | Self::Delta {
                task_id,
                execution_id,
                ..
            }
            | Self::Message {
                task_id,
                execution_id,
                ..
            }
            | Self::Trace {
                task_id,
                execution_id,
                ..
            }
            | Self::Completed {
                task_id,
                execution_id,
                ..
            }
            | Self::Cancelled {
                task_id,
                execution_id,
                ..
            }
            | Self::Failed {
                task_id,
                execution_id,
                ..
            } => (task_id, execution_id),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ExecutionUpdate {
    State {
        task_id: String,
        execution_id: String,
        execution: Execution,
    },
    Delta {
        task_id: String,
        execution_id: String,
        delta: String,
    },
    Message {
        task_id: String,
        execution_id: String,
        message: TaskMessage,
    },
    Event {
        task_id: String,
        execution_id: String,
        event: TaskEvent,
    },
    Approval {
        task_id: String,
        execution_id: String,
        approval: ToolApproval,
    },
    Inspector {
        task_id: String,
        execution_id: String,
        tab: &'static str,
    },
    Terminal {
        task_id: String,
        execution_id: String,
        execution: Execution,
        outcome: &'static str,
        error: Option<String>,
    },
    Error {
        task_id: String,
        execution_id: String,
        scope: &'static str,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ActiveExecution {
    task_id: String,
    execution_id: String,
    last_worker_sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerSequenceDisposition {
    Process,
    Duplicate,
}

#[derive(Default)]
struct SupervisorState {
    worker_channel: Option<Channel<AgentWorkerCommand>>,
    renderer_channel: Option<Channel<ExecutionUpdate>>,
    reserved_task_id: Option<String>,
    active: Option<ActiveExecution>,
}

#[derive(Default)]
pub struct TaskSupervisor {
    state: Mutex<SupervisorState>,
}

impl TaskSupervisor {
    fn state(&self) -> Result<std::sync::MutexGuard<'_, SupervisorState>, String> {
        self.state
            .lock()
            .map_err(|_| "the Task Supervisor is unavailable".to_owned())
    }

    fn connect_worker(&self, channel: Channel<AgentWorkerCommand>) -> Result<(), String> {
        self.state()?.worker_channel = Some(channel);
        Ok(())
    }

    fn subscribe_renderer(&self, channel: Channel<ExecutionUpdate>) -> Result<(), String> {
        self.state()?.renderer_channel = Some(channel);
        Ok(())
    }

    fn ensure_worker(&self) -> Result<(), String> {
        if self.state()?.worker_channel.is_some() {
            Ok(())
        } else {
            Err("the isolated Agent Worker is still starting".to_owned())
        }
    }

    fn reserve(&self, task_id: &str) -> Result<(), String> {
        let mut state = self.state()?;
        if let Some(active) = &state.active {
            return Err(format!(
                "Execution {} already owns local inference capacity",
                active.execution_id
            ));
        }
        if state.reserved_task_id.is_some() {
            return Err("another Execution is being prepared".to_owned());
        }
        state.reserved_task_id = Some(task_id.to_owned());
        Ok(())
    }

    fn release_reservation(&self, task_id: &str) {
        if let Ok(mut state) = self.state()
            && state.reserved_task_id.as_deref() == Some(task_id)
        {
            state.reserved_task_id = None;
        }
    }

    fn activate(&self, task_id: &str, execution_id: &str) -> Result<(), String> {
        let mut state = self.state()?;
        if state.reserved_task_id.as_deref() != Some(task_id) || state.active.is_some() {
            return Err("the Execution reservation changed before activation".to_owned());
        }
        state.reserved_task_id = None;
        state.active = Some(ActiveExecution {
            task_id: task_id.to_owned(),
            execution_id: execution_id.to_owned(),
            last_worker_sequence: 0,
        });
        Ok(())
    }

    fn verify_active(&self, task_id: &str, execution_id: &str) -> Result<(), String> {
        let state = self.state()?;
        match &state.active {
            Some(active) if active.task_id == task_id && active.execution_id == execution_id => {
                Ok(())
            }
            Some(active) => Err(format!(
                "worker update for {execution_id} does not match active Execution {}",
                active.execution_id
            )),
            None => Err("there is no active host-owned Execution".to_owned()),
        }
    }

    fn accept_worker_sequence(
        &self,
        task_id: &str,
        execution_id: &str,
        sequence: u64,
    ) -> Result<WorkerSequenceDisposition, String> {
        let mut state = self.state()?;
        let active = state
            .active
            .as_mut()
            .ok_or_else(|| "there is no active host-owned Execution".to_owned())?;
        if active.task_id != task_id || active.execution_id != execution_id {
            return Err(format!(
                "worker update for {execution_id} does not match active Execution {}",
                active.execution_id
            ));
        }
        if sequence <= active.last_worker_sequence {
            return Ok(WorkerSequenceDisposition::Duplicate);
        }
        let expected = active.last_worker_sequence.saturating_add(1);
        if sequence != expected {
            return Err(format!(
                "Agent Worker event arrived out of order: expected {expected}, received {sequence}"
            ));
        }
        active.last_worker_sequence = sequence;
        Ok(WorkerSequenceDisposition::Process)
    }

    fn active_for_execution(&self, execution_id: &str) -> Result<ActiveExecution, String> {
        let state = self.state()?;
        match &state.active {
            Some(active) if active.execution_id == execution_id => Ok(active.clone()),
            Some(active) => Err(format!(
                "Execution {execution_id} does not own local inference capacity; {} does",
                active.execution_id
            )),
            None => Err("there is no active host-owned Execution".to_owned()),
        }
    }

    fn finish(&self, execution_id: &str) {
        if let Ok(mut state) = self.state()
            && state
                .active
                .as_ref()
                .is_some_and(|active| active.execution_id == execution_id)
        {
            state.active = None;
        }
    }

    fn send_worker(&self, command: AgentWorkerCommand) -> Result<(), String> {
        let channel = self
            .state()?
            .worker_channel
            .clone()
            .ok_or_else(|| "the isolated Agent Worker is not connected".to_owned())?;
        channel
            .send(command)
            .map_err(|error| format!("failed to send command to the Agent Worker: {error}"))
    }

    fn broadcast(&self, update: ExecutionUpdate) {
        let channel = self
            .state()
            .ok()
            .and_then(|state| state.renderer_channel.clone());
        if let Some(channel) = channel {
            let _ = channel.send(update);
        }
    }
}

fn require_webview(webview: &Webview, expected: &str) -> Result<(), String> {
    if webview.label() == expected {
        Ok(())
    } else {
        Err(format!(
            "webview '{}' cannot invoke this host authority",
            webview.label()
        ))
    }
}

fn validate_text(label: &str, value: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > max {
        return Err(format!("{label} must contain between 1 and {max} bytes"));
    }
    Ok(value.to_owned())
}

fn state_update(execution: Execution) -> ExecutionUpdate {
    ExecutionUpdate::State {
        task_id: execution.task_id.clone(),
        execution_id: execution.id.to_string(),
        execution,
    }
}

fn terminal_update(execution: Execution, outcome: &'static str) -> ExecutionUpdate {
    ExecutionUpdate::Terminal {
        task_id: execution.task_id.clone(),
        execution_id: execution.id.to_string(),
        error: execution.failure.clone(),
        execution,
        outcome,
    }
}

fn broadcast_records(supervisor: &TaskSupervisor, records: &[TaskEvent]) {
    for record in records {
        if let Some(execution_id) = &record.execution_id {
            supervisor.broadcast(ExecutionUpdate::Event {
                task_id: record.task_id.clone(),
                execution_id: execution_id.to_string(),
                event: record.clone(),
            });
        }
    }
}

fn fail_execution(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    execution_id: &str,
    message: impl Into<String>,
) -> Result<Execution, String> {
    let message = message.into();
    let _ = supervisor.send_worker(AgentWorkerCommand::Cancel {
        execution_id: execution_id.to_owned(),
    });
    supervisor.finish(execution_id);
    let current = store
        .get_execution(execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution no longer exists".to_owned())?;
    let outcome = if current.state.is_terminal() {
        crate::store::ExecutionTransitionOutcome {
            execution: current,
            records: Vec::new(),
        }
    } else {
        store
            .finish_execution(
                execution_id,
                ExecutionState::Failed,
                Some(&message),
                None,
                None,
            )
            .map_err(|error| error.to_string())?
    };
    broadcast_records(supervisor, &outcome.records);
    supervisor.broadcast(ExecutionUpdate::Error {
        task_id: outcome.execution.task_id.clone(),
        execution_id: execution_id.to_owned(),
        scope: "persistence",
        message: message.clone(),
    });
    supervisor.broadcast(state_update(outcome.execution.clone()));
    supervisor.broadcast(terminal_update(outcome.execution.clone(), "failed"));
    Ok(outcome.execution)
}

fn terminalize(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    execution_id: &str,
    requested: ExecutionState,
    failure: Option<&str>,
    duration_ms: u64,
    response_characters: u64,
) -> Result<Execution, String> {
    let current = store
        .get_execution(execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution no longer exists".to_owned())?;
    if current.state.is_terminal() {
        supervisor.finish(execution_id);
        return Ok(current);
    }
    let outcome = store
        .finish_execution(
            execution_id,
            requested,
            failure,
            Some(duration_ms),
            Some(response_characters),
        )
        .map_err(|error| error.to_string())?;
    supervisor.finish(execution_id);
    broadcast_records(supervisor, &outcome.records);
    let result = outcome.execution;
    let label = match result.state {
        ExecutionState::Completed => "completed",
        ExecutionState::Cancelled => "cancelled",
        ExecutionState::Failed => "failed",
        _ => return Err("terminal settlement selected a non-terminal state".to_owned()),
    };
    supervisor.broadcast(state_update(result.clone()));
    supervisor.broadcast(terminal_update(result.clone(), label));
    Ok(result)
}

#[tauri::command]
pub fn connect_agent_worker(
    webview: Webview,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    channel: Channel<AgentWorkerCommand>,
) -> Result<(), String> {
    require_webview(&webview, AGENT_WORKER_WEBVIEW)?;
    supervisor.connect_worker(channel)
}

#[tauri::command]
pub fn subscribe_execution_updates(
    webview: Webview,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    channel: Channel<ExecutionUpdate>,
) -> Result<(), String> {
    require_webview(&webview, MAIN_WEBVIEW)?;
    supervisor.subscribe_renderer(channel)
}

#[tauri::command]
pub async fn submit_prompt(
    app: AppHandle,
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    task_id: String,
    prompt: String,
) -> Result<SubmitPromptResult, String> {
    require_webview(&webview, MAIN_WEBVIEW)?;
    let prompt = validate_text("prompt", &prompt, MAX_PROMPT_BYTES)?;
    let store = Arc::clone(store.inner());
    let supervisor = Arc::clone(supervisor.inner());
    supervisor.ensure_worker()?;
    supervisor.reserve(&task_id)?;

    let result = async {
        let detail = store
            .load_task(&task_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "the Task does not exist".to_owned())?;
        let history = detail.messages;
        let launch_app = app.clone();
        let launch =
            tauri::async_runtime::spawn_blocking(move || resolve_pi_launch_blocking(launch_app))
                .await
                .map_err(|error| format!("Agent launch worker failed: {error}"))??;
        let accepted = store
            .accept_prompt(&task_id, &prompt, launch.specification.clone())
            .map_err(|error| error.to_string())?;
        let execution_id = accepted.execution.id.to_string();
        broadcast_records(&supervisor, &accepted.records);
        supervisor.broadcast(ExecutionUpdate::Message {
            task_id: task_id.clone(),
            execution_id: execution_id.clone(),
            message: accepted.prompt_message.clone(),
        });
        let preparing = match store.record_execution_state(&execution_id, ExecutionState::Preparing)
        {
            Ok(value) => value,
            Err(error) => {
                let _ = fail_execution(&store, &supervisor, &execution_id, error.to_string());
                return Err(error.to_string());
            }
        };
        broadcast_records(&supervisor, &preparing.records);
        supervisor.broadcast(state_update(preparing.execution.clone()));
        if let Err(error) = supervisor.activate(&task_id, &execution_id) {
            let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
            return Err(error);
        }
        if let Err(error) = supervisor.send_worker(AgentWorkerCommand::Start {
            task_id: task_id.clone(),
            execution_id: execution_id.clone(),
            prompt,
            history,
            config: Box::new(launch),
        }) {
            let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
            return Err(error);
        }
        Ok(SubmitPromptResult {
            execution: preparing.execution,
            prompt_message: accepted.prompt_message,
        })
    }
    .await;

    if result.is_err() {
        supervisor.release_reservation(&task_id);
    }
    result
}

#[tauri::command]
pub fn cancel_execution(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    execution_id: String,
) -> Result<Execution, String> {
    require_webview(&webview, MAIN_WEBVIEW)?;
    let active = supervisor.active_for_execution(&execution_id)?;
    let current = store
        .get_execution(&execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if current.state.is_terminal() {
        return Ok(current);
    }
    for approval in store
        .list_pending_approvals(&active.task_id)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|approval| approval.execution_id.as_str() == execution_id)
    {
        if let Ok(decision) = store.decide_tool_approval_recorded(&approval.id, false) {
            broadcast_records(&supervisor, &decision.records);
            supervisor.broadcast(state_update(decision.execution));
            let _ = supervisor.send_worker(AgentWorkerCommand::ApprovalDecision {
                approval_id: approval.id,
                approved: false,
            });
        }
    }
    let cancelling = if current.state == ExecutionState::Cancelling {
        current
    } else {
        let outcome = store
            .record_execution_state(&execution_id, ExecutionState::Cancelling)
            .map_err(|error| error.to_string())?;
        broadcast_records(&supervisor, &outcome.records);
        outcome.execution
    };
    supervisor.broadcast(state_update(cancelling.clone()));
    if let Err(error) = supervisor.send_worker(AgentWorkerCommand::Cancel {
        execution_id: execution_id.clone(),
    }) {
        return fail_execution(&store, &supervisor, &execution_id, error);
    }
    Ok(cancelling)
}

fn send_direction(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    execution_id: &str,
    text: &str,
    follow_up: bool,
) -> Result<TaskMessage, String> {
    let text = validate_text("direction", text, MAX_DIRECTION_BYTES)?;
    let active = supervisor.active_for_execution(execution_id)?;
    let direction = if follow_up {
        UserDirection::FollowUp
    } else {
        UserDirection::Steer
    };
    let recorded = store
        .record_direction(execution_id, direction, &text)
        .map_err(|error| error.to_string())?;
    supervisor.broadcast(ExecutionUpdate::Message {
        task_id: active.task_id,
        execution_id: execution_id.to_owned(),
        message: recorded.message.clone(),
    });
    broadcast_records(supervisor, std::slice::from_ref(&recorded.record));
    let command = if follow_up {
        AgentWorkerCommand::FollowUp {
            execution_id: execution_id.to_owned(),
            text,
        }
    } else {
        AgentWorkerCommand::Steer {
            execution_id: execution_id.to_owned(),
            text,
        }
    };
    if let Err(error) = supervisor.send_worker(command) {
        let _ = fail_execution(store, supervisor, execution_id, error.clone());
        return Err(error);
    }
    Ok(recorded.message)
}

#[tauri::command]
pub fn steer_execution(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    execution_id: String,
    text: String,
) -> Result<TaskMessage, String> {
    require_webview(&webview, MAIN_WEBVIEW)?;
    send_direction(&store, &supervisor, &execution_id, &text, false)
}

#[tauri::command]
pub fn queue_follow_up(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    execution_id: String,
    text: String,
) -> Result<TaskMessage, String> {
    require_webview(&webview, MAIN_WEBVIEW)?;
    send_direction(&store, &supervisor, &execution_id, &text, true)
}

#[tauri::command]
pub fn decide_tool_approval(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    approval_id: String,
    approved: bool,
) -> Result<ToolApprovalDecision, String> {
    require_webview(&webview, MAIN_WEBVIEW)?;
    let approval = store
        .get_tool_approval(&approval_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Tool Approval does not exist".to_owned())?;
    let execution_id = approval.execution_id.to_string();
    supervisor.verify_active(&approval.task_id, &execution_id)?;
    let decision = store
        .decide_tool_approval_recorded(&approval_id, approved)
        .map_err(|error| error.to_string())?;
    broadcast_records(&supervisor, &decision.records);
    supervisor.broadcast(state_update(decision.execution.clone()));
    if let Err(error) = supervisor.send_worker(AgentWorkerCommand::ApprovalDecision {
        approval_id,
        approved,
    }) {
        let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
        return Err(error);
    }
    Ok(decision)
}

#[tauri::command]
pub fn agent_request_tool_approval(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    input: NewToolApproval,
) -> Result<ToolApproval, String> {
    require_webview(&webview, AGENT_WORKER_WEBVIEW)?;
    supervisor.verify_active(&input.task_id, input.execution_id.as_str())?;
    let outcome = store
        .propose_tool(input)
        .map_err(|error| error.to_string())?;
    broadcast_records(&supervisor, &outcome.records);
    supervisor.broadcast(state_update(outcome.execution));
    supervisor.broadcast(ExecutionUpdate::Approval {
        task_id: outcome.approval.task_id.clone(),
        execution_id: outcome.approval.execution_id.to_string(),
        approval: outcome.approval.clone(),
    });
    Ok(outcome.approval)
}

fn verify_worker_effect(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    task_id: &str,
    execution_id: &str,
    approval_id: &str,
) -> Result<(), String> {
    supervisor.verify_active(task_id, execution_id)?;
    let execution = store
        .get_execution(execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if execution.state != ExecutionState::Running {
        return Err("the Execution is not running an approved effect".to_owned());
    }
    let approval = store
        .get_tool_approval(approval_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Tool Approval does not exist".to_owned())?;
    if approval.task_id != task_id || approval.execution_id.as_str() != execution_id {
        return Err("the Tool Approval belongs to a different Execution".to_owned());
    }
    if approval.state != ApprovalState::Approved {
        return Err("the Tool Approval is not approved for execution".to_owned());
    }
    Ok(())
}

#[tauri::command]
pub fn agent_execute_edit(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    task_id: String,
    execution_id: String,
    approval_id: String,
    edit: WorkspaceEdit,
) -> Result<WorkspaceEditResult, String> {
    require_webview(&webview, AGENT_WORKER_WEBVIEW)?;
    verify_worker_effect(&store, &supervisor, &task_id, &execution_id, &approval_id)?;
    let proposal = ToolProposal::from(&edit);
    let claim = store
        .claim_tool_effect(&approval_id, &proposal)
        .map_err(|error| error.to_string())?;
    broadcast_records(&supervisor, std::slice::from_ref(&claim.record));
    match workspace::execute_edit(&store, &task_id, edit) {
        Ok(result) => {
            let detail = format!("edited {}", result.path);
            let settled = store
                .settle_tool_effect(
                    &approval_id,
                    true,
                    ToolResult::from(result.clone()),
                    Some(&detail),
                )
                .map_err(|error| error.to_string())?;
            broadcast_records(&supervisor, &settled.records);
            supervisor.broadcast(ExecutionUpdate::Inspector {
                task_id,
                execution_id,
                tab: "changes",
            });
            Ok(result)
        }
        Err(error) => {
            let message = error.to_string();
            let settled = store
                .settle_tool_effect(
                    &approval_id,
                    false,
                    ToolResult::Failure {
                        message: message.clone(),
                    },
                    Some(&message),
                )
                .map_err(|cause| cause.to_string())?;
            broadcast_records(&supervisor, &settled.records);
            Err(message)
        }
    }
}

#[tauri::command]
pub async fn agent_run_shell(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    task_id: String,
    execution_id: String,
    approval_id: String,
    shell: WorkspaceShell,
) -> Result<WorkspaceShellResult, String> {
    require_webview(&webview, AGENT_WORKER_WEBVIEW)?;
    verify_worker_effect(&store, &supervisor, &task_id, &execution_id, &approval_id)?;
    let proposal = ToolProposal::from(&shell);
    let claim = store
        .claim_tool_effect(&approval_id, &proposal)
        .map_err(|error| error.to_string())?;
    broadcast_records(&supervisor, std::slice::from_ref(&claim.record));
    let store_for_shell = Arc::clone(store.inner());
    let task_for_shell = task_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        workspace::execute_shell(&store_for_shell, &task_for_shell, shell)
    })
    .await
    .map_err(|error| format!("workspace shell worker failed: {error}"))?;
    match result {
        Ok(result) => {
            let succeeded = result.exit_code == 0;
            let detail = format!("exit {} in {} ms", result.exit_code, result.duration_ms);
            let settled = store
                .settle_tool_effect(
                    &approval_id,
                    succeeded,
                    ToolResult::from(result.clone()),
                    Some(&detail),
                )
                .map_err(|error| error.to_string())?;
            broadcast_records(&supervisor, &settled.records);
            supervisor.broadcast(ExecutionUpdate::Inspector {
                task_id,
                execution_id,
                tab: "terminal",
            });
            Ok(result)
        }
        Err(error) => {
            let message = error.to_string();
            let settled = store
                .settle_tool_effect(
                    &approval_id,
                    false,
                    ToolResult::Failure {
                        message: message.clone(),
                    },
                    Some(&message),
                )
                .map_err(|cause| cause.to_string())?;
            broadcast_records(&supervisor, &settled.records);
            Err(message)
        }
    }
}

fn handle_worker_event(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    event: AgentWorkerEventKind,
) -> Result<(), String> {
    let (task_id, execution_id) = event.identity();
    supervisor.verify_active(task_id, execution_id)?;
    match event {
        AgentWorkerEventKind::Started { execution_id, .. } => {
            let current = store
                .get_execution(&execution_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "the Execution does not exist".to_owned())?;
            if current.state == ExecutionState::Cancelling {
                supervisor.send_worker(AgentWorkerCommand::Cancel { execution_id })?;
                return Ok(());
            }
            if current.state != ExecutionState::Preparing {
                return Err(format!(
                    "Agent Worker started while Execution was {}",
                    current.state.as_str()
                ));
            }
            let outcome = store
                .record_execution_state(&execution_id, ExecutionState::Running)
                .map_err(|error| error.to_string())?;
            broadcast_records(supervisor, &outcome.records);
            supervisor.broadcast(state_update(outcome.execution));
        }
        AgentWorkerEventKind::Delta {
            task_id,
            execution_id,
            delta,
        } => {
            if delta.len() > 256 * 1024 {
                return Err("Agent Worker delta exceeds 256 KiB".to_owned());
            }
            supervisor.broadcast(ExecutionUpdate::Delta {
                task_id,
                execution_id,
                delta,
            });
        }
        AgentWorkerEventKind::Message {
            task_id,
            execution_id,
            role,
            content,
        } => {
            if role != MessageRole::Assistant {
                return Err("the Agent Worker may persist only assistant messages".to_owned());
            }
            let recorded = store
                .record_assistant_message(&execution_id, &content)
                .map_err(|error| error.to_string())?;
            supervisor.broadcast(ExecutionUpdate::Message {
                task_id,
                execution_id,
                message: recorded.message,
            });
            broadcast_records(supervisor, std::slice::from_ref(&recorded.record));
        }
        AgentWorkerEventKind::Trace { kind, payload, .. } => {
            let _ = (kind, payload);
        }
        AgentWorkerEventKind::Completed {
            execution_id,
            duration_ms,
            response_characters,
            ..
        } => {
            terminalize(
                store,
                supervisor,
                &execution_id,
                ExecutionState::Completed,
                None,
                duration_ms,
                response_characters,
            )?;
        }
        AgentWorkerEventKind::Cancelled {
            execution_id,
            duration_ms,
            response_characters,
            ..
        } => {
            terminalize(
                store,
                supervisor,
                &execution_id,
                ExecutionState::Cancelled,
                None,
                duration_ms,
                response_characters,
            )?;
        }
        AgentWorkerEventKind::Failed {
            execution_id,
            error,
            duration_ms,
            response_characters,
            ..
        } => {
            terminalize(
                store,
                supervisor,
                &execution_id,
                ExecutionState::Failed,
                Some(&error),
                duration_ms,
                response_characters,
            )?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn agent_worker_event(
    webview: Webview,
    store: State<'_, Arc<DesktopStore>>,
    supervisor: State<'_, Arc<TaskSupervisor>>,
    event: AgentWorkerEvent,
) -> Result<(), String> {
    require_webview(&webview, AGENT_WORKER_WEBVIEW)?;
    let (task_id, execution_id) = event.event.identity();
    let task_id = task_id.to_owned();
    let execution_id = execution_id.to_owned();
    let current = store
        .get_execution(&execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if current.state.is_terminal() {
        return Ok(());
    }
    match supervisor.accept_worker_sequence(&task_id, &execution_id, event.sequence) {
        Ok(WorkerSequenceDisposition::Duplicate) => return Ok(()),
        Ok(WorkerSequenceDisposition::Process) => {}
        Err(error) => {
            let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
            return Err(error);
        }
    }
    match handle_worker_event(&store, &supervisor, event.event) {
        Ok(()) => Ok(()),
        Err(error) => {
            let current = store
                .get_execution(&execution_id)
                .map_err(|cause| cause.to_string())?
                .ok_or_else(|| "the Execution no longer exists".to_owned())?;
            if current.state.is_terminal() {
                return Ok(());
            }
            let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_supervisor_reservation_owns_local_capacity() {
        let supervisor = TaskSupervisor::default();
        supervisor.reserve("task-1").unwrap();
        assert!(supervisor.reserve("task-2").is_err());
        supervisor.activate("task-1", "execution-1").unwrap();
        assert!(supervisor.reserve("task-2").is_err());
        assert!(supervisor.verify_active("task-1", "execution-1").is_ok());
        supervisor.finish("execution-1");
        assert!(supervisor.reserve("task-2").is_ok());
    }

    #[test]
    fn active_identity_rejects_cross_task_worker_updates() {
        let supervisor = TaskSupervisor::default();
        supervisor.reserve("task-1").unwrap();
        supervisor.activate("task-1", "execution-1").unwrap();
        assert!(supervisor.verify_active("task-2", "execution-1").is_err());
        assert!(supervisor.verify_active("task-1", "execution-2").is_err());
    }

    #[test]
    fn duplicate_worker_delivery_is_idempotent_and_gaps_are_rejected() {
        let supervisor = TaskSupervisor::default();
        supervisor.reserve("task-1").unwrap();
        supervisor.activate("task-1", "execution-1").unwrap();
        assert_eq!(
            supervisor
                .accept_worker_sequence("task-1", "execution-1", 1)
                .unwrap(),
            WorkerSequenceDisposition::Process
        );
        assert_eq!(
            supervisor
                .accept_worker_sequence("task-1", "execution-1", 1)
                .unwrap(),
            WorkerSequenceDisposition::Duplicate
        );
        assert!(
            supervisor
                .accept_worker_sequence("task-1", "execution-1", 3)
                .unwrap_err()
                .contains("out of order")
        );
        assert_eq!(
            supervisor
                .accept_worker_sequence("task-1", "execution-1", 2)
                .unwrap(),
            WorkerSequenceDisposition::Process
        );
    }
}
