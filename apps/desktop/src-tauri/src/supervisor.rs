//! Host-owned task execution authority and the isolated Pi worker protocol.

use crate::{
    PiLaunchConfig, resolve_pi_launch_blocking,
    store::{
        CreateExecution, DesktopStore, Execution, ExecutionId, ExecutionState, MessageRole,
        NewTaskEvent, NewTaskMessage, NewToolApproval, TaskEvent, TaskMessage, ToolApproval,
        ToolApprovalDecision,
    },
    workspace::{self, WorkspaceEdit, WorkspaceEditResult, WorkspaceShell, WorkspaceShellResult},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
        config: PiLaunchConfig,
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
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentWorkerEvent {
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
    Event {
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

impl AgentWorkerEvent {
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
            | Self::Event {
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

fn fail_execution(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    execution_id: &str,
    message: impl Into<String>,
) -> Result<Execution, String> {
    let message = message.into();
    let current = store
        .get_execution(execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution no longer exists".to_owned())?;
    let failed = if current.state.is_terminal() {
        current
    } else {
        store
            .transition_execution(execution_id, ExecutionState::Failed, Some(&message))
            .map_err(|error| error.to_string())?
    };
    let _ = supervisor.send_worker(AgentWorkerCommand::Cancel {
        execution_id: execution_id.to_owned(),
    });
    supervisor.finish(execution_id);
    supervisor.broadcast(ExecutionUpdate::Error {
        task_id: failed.task_id.clone(),
        execution_id: execution_id.to_owned(),
        scope: "persistence",
        message: message.clone(),
    });
    supervisor.broadcast(terminal_update(failed.clone(), "failed"));
    Ok(failed)
}

fn append_metrics(
    store: &DesktopStore,
    execution: &Execution,
    duration_ms: u64,
    response_characters: u64,
) -> Result<TaskEvent, String> {
    store
        .append_event(NewTaskEvent {
            task_id: execution.task_id.clone(),
            execution_id: execution.id.clone(),
            kind: "execution.metrics".to_owned(),
            payload: json!({
                "durationMs": duration_ms,
                "responseCharacters": response_characters,
                "authority": "host-supervisor",
            }),
        })
        .map_err(|error| error.to_string())
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
    let mut current = store
        .get_execution(execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution no longer exists".to_owned())?;
    if current.state.is_terminal() {
        supervisor.finish(execution_id);
        return Ok(current);
    }
    if current.state == ExecutionState::WaitingForApproval {
        current = store
            .transition_execution(execution_id, ExecutionState::Running, None)
            .map_err(|error| error.to_string())?;
    }
    let event = append_metrics(store, &current, duration_ms, response_characters)?;
    supervisor.broadcast(ExecutionUpdate::Event {
        task_id: current.task_id.clone(),
        execution_id: execution_id.to_owned(),
        event,
    });
    let next = if current.state == ExecutionState::Cancelling {
        ExecutionState::Cancelled
    } else {
        requested
    };
    let execution = store
        .transition_execution(execution_id, next, failure)
        .map_err(|error| error.to_string())?;
    supervisor.finish(execution_id);
    let outcome = match next {
        ExecutionState::Completed => "completed",
        ExecutionState::Cancelled => "cancelled",
        ExecutionState::Failed => "failed",
        _ => return Err("terminal settlement selected a non-terminal state".to_owned()),
    };
    supervisor.broadcast(state_update(execution.clone()));
    supervisor.broadcast(terminal_update(execution.clone(), outcome));
    Ok(execution)
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
        let execution = store
            .create_execution(CreateExecution {
                task_id: task_id.clone(),
                specification: launch.specification.clone(),
            })
            .map_err(|error| error.to_string())?;
        let prompt_message = match store.append_message(NewTaskMessage {
            task_id: task_id.clone(),
            execution_id: execution.id.clone(),
            role: MessageRole::User,
            content: prompt.clone(),
        }) {
            Ok(message) => message,
            Err(error) => {
                let _ = fail_execution(
                    &store,
                    &supervisor,
                    execution.id.as_str(),
                    error.to_string(),
                );
                return Err(error.to_string());
            }
        };
        let execution = match store.transition_execution(
            execution.id.as_str(),
            ExecutionState::Preparing,
            None,
        ) {
            Ok(execution) => execution,
            Err(error) => {
                let _ = fail_execution(
                    &store,
                    &supervisor,
                    execution.id.as_str(),
                    error.to_string(),
                );
                return Err(error.to_string());
            }
        };
        supervisor.activate(&task_id, execution.id.as_str())?;
        supervisor.broadcast(ExecutionUpdate::Message {
            task_id: task_id.clone(),
            execution_id: execution.id.to_string(),
            message: prompt_message.clone(),
        });
        supervisor.broadcast(state_update(execution.clone()));
        if let Err(error) = supervisor.send_worker(AgentWorkerCommand::Start {
            task_id: task_id.clone(),
            execution_id: execution.id.to_string(),
            prompt,
            history,
            config: launch,
        }) {
            let _ = fail_execution(&store, &supervisor, execution.id.as_str(), error.clone());
            return Err(error);
        }
        Ok(SubmitPromptResult {
            execution,
            prompt_message,
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
        if let Ok(decision) = store.decide_tool_approval_with_event(&approval.id, false) {
            supervisor.broadcast(ExecutionUpdate::Event {
                task_id: active.task_id.clone(),
                execution_id: execution_id.clone(),
                event: decision.event,
            });
            let _ = supervisor.send_worker(AgentWorkerCommand::ApprovalDecision {
                approval_id: approval.id,
                approved: false,
            });
        }
    }
    let cancelling = if current.state == ExecutionState::Cancelling {
        current
    } else {
        store
            .transition_execution(&execution_id, ExecutionState::Cancelling, None)
            .map_err(|error| error.to_string())?
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
    let execution = store
        .get_execution(execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if !matches!(
        execution.state,
        ExecutionState::Running | ExecutionState::WaitingForApproval
    ) {
        return Err(format!(
            "Execution {execution_id} cannot accept direction while {}",
            execution.state.as_str()
        ));
    }
    let message = store
        .append_message(NewTaskMessage {
            task_id: active.task_id.clone(),
            execution_id: execution.id.clone(),
            role: MessageRole::User,
            content: text.clone(),
        })
        .map_err(|error| error.to_string())?;
    supervisor.broadcast(ExecutionUpdate::Message {
        task_id: active.task_id,
        execution_id: execution_id.to_owned(),
        message: message.clone(),
    });
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
    Ok(message)
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
    let decision = store
        .decide_tool_approval_with_event(&approval_id, approved)
        .map_err(|error| error.to_string())?;
    let execution_id = decision.approval.execution_id.to_string();
    let task_id = decision.approval.task_id.clone();
    supervisor.verify_active(&task_id, &execution_id)?;
    let current = store
        .get_execution(&execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if current.state == ExecutionState::WaitingForApproval {
        let running = store
            .transition_execution(&execution_id, ExecutionState::Running, None)
            .map_err(|error| error.to_string())?;
        supervisor.broadcast(state_update(running));
    }
    supervisor.broadcast(ExecutionUpdate::Event {
        task_id: task_id.clone(),
        execution_id: execution_id.clone(),
        event: decision.event.clone(),
    });
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
    let current = store
        .get_execution(input.execution_id.as_str())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if current.state != ExecutionState::Running {
        return Err("the Execution is not ready to request a Tool Approval".to_owned());
    }
    let approval = store
        .request_tool_approval(input)
        .map_err(|error| error.to_string())?;
    let waiting = match store.transition_execution(
        approval.execution_id.as_str(),
        ExecutionState::WaitingForApproval,
        None,
    ) {
        Ok(execution) => execution,
        Err(error) => {
            let _ = store.decide_tool_approval_with_event(&approval.id, false);
            return Err(error.to_string());
        }
    };
    supervisor.broadcast(state_update(waiting));
    supervisor.broadcast(ExecutionUpdate::Approval {
        task_id: approval.task_id.clone(),
        execution_id: approval.execution_id.to_string(),
        approval: approval.clone(),
    });
    Ok(approval)
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
    workspace::edit_project_file(&store, &task_id, &approval_id, edit)
        .map_err(|error| error.to_string())
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
    let store = Arc::clone(store.inner());
    tauri::async_runtime::spawn_blocking(move || {
        workspace::run_project_shell(&store, &task_id, &approval_id, shell)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("workspace shell worker failed: {error}"))?
}

fn inspect_tool_event(
    supervisor: &TaskSupervisor,
    task_id: &str,
    execution_id: &str,
    kind: &str,
    payload: &Value,
) {
    if kind != "tool.finished" {
        return;
    }
    let tool_name = payload.get("toolName").and_then(Value::as_str);
    let tab = match tool_name {
        Some("edit_file") => Some("changes"),
        Some("run_command") => Some("terminal"),
        _ => None,
    };
    if let Some(tab) = tab {
        supervisor.broadcast(ExecutionUpdate::Inspector {
            task_id: task_id.to_owned(),
            execution_id: execution_id.to_owned(),
            tab,
        });
    }
}

fn handle_worker_event(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    event: AgentWorkerEvent,
) -> Result<(), String> {
    let (task_id, execution_id) = event.identity();
    supervisor.verify_active(task_id, execution_id)?;
    match event {
        AgentWorkerEvent::Started { execution_id, .. } => {
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
            let running = store
                .transition_execution(&execution_id, ExecutionState::Running, None)
                .map_err(|error| error.to_string())?;
            supervisor.broadcast(state_update(running));
        }
        AgentWorkerEvent::Delta {
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
        AgentWorkerEvent::Message {
            task_id,
            execution_id,
            role,
            content,
        } => {
            if role != MessageRole::Assistant {
                return Err("the Agent Worker may persist only assistant messages".to_owned());
            }
            let message = store
                .append_message(NewTaskMessage {
                    task_id: task_id.clone(),
                    execution_id: ExecutionId(execution_id.clone()),
                    role,
                    content,
                })
                .map_err(|error| error.to_string())?;
            supervisor.broadcast(ExecutionUpdate::Message {
                task_id,
                execution_id,
                message,
            });
        }
        AgentWorkerEvent::Event {
            task_id,
            execution_id,
            kind,
            payload,
        } => {
            let persisted = store
                .append_event(NewTaskEvent {
                    task_id: task_id.clone(),
                    execution_id: ExecutionId(execution_id.clone()),
                    kind: kind.clone(),
                    payload: payload.clone(),
                })
                .map_err(|error| error.to_string())?;
            supervisor.broadcast(ExecutionUpdate::Event {
                task_id: task_id.clone(),
                execution_id: execution_id.clone(),
                event: persisted,
            });
            inspect_tool_event(supervisor, &task_id, &execution_id, &kind, &payload);
        }
        AgentWorkerEvent::Completed {
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
        AgentWorkerEvent::Cancelled {
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
        AgentWorkerEvent::Failed {
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
    let (_, execution_id) = event.identity();
    let execution_id = execution_id.to_owned();
    match handle_worker_event(&store, &supervisor, event) {
        Ok(()) => Ok(()),
        Err(error) => {
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
}
