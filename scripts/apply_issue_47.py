from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def read(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def write(path: str, value: str) -> None:
    (ROOT / path).write_text(value, encoding="utf-8")


def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one literal match, found {count}")
    return value.replace(old, new, 1)


def sub_once(value: str, pattern: str, replacement: str, label: str) -> str:
    updated, count = re.subn(pattern, replacement, value, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{label}: expected one regex match, found {count}")
    return updated


def patch_desktop() -> None:
    path = "apps/desktop/src/desktop.ts"
    value = read(path)
    value = replace_once(
        value,
        'import { invoke } from "@tauri-apps/api/core";',
        'import { Channel, invoke } from "@tauri-apps/api/core";',
        "desktop Channel import",
    )
    value = sub_once(
        value,
        r'\nexport interface PiLaunchConfig \{.*?\n\}\n',
        "\n",
        "remove renderer launch credential contract",
    )
    value = sub_once(
        value,
        r'\nexport interface CreateExecutionInput \{.*?\n\}\n',
        "\n",
        "remove renderer execution creation input",
    )
    execution_update = '''

export interface SubmitPromptResult {
  execution: Execution;
  promptMessage: TaskMessage;
}

export type ExecutionUpdate =
  | {
      type: "state";
      taskId: string;
      executionId: string;
      execution: Execution;
    }
  | { type: "delta"; taskId: string; executionId: string; delta: string }
  | {
      type: "message";
      taskId: string;
      executionId: string;
      message: TaskMessage;
    }
  | {
      type: "event";
      taskId: string;
      executionId: string;
      event: TaskEvent;
    }
  | {
      type: "approval";
      taskId: string;
      executionId: string;
      approval: ToolApproval;
    }
  | {
      type: "inspector";
      taskId: string;
      executionId: string;
      tab: "changes" | "terminal";
    }
  | {
      type: "terminal";
      taskId: string;
      executionId: string;
      execution: Execution;
      outcome: "completed" | "cancelled" | "failed";
      error: string | null;
    }
  | {
      type: "error";
      taskId: string;
      executionId: string;
      scope: "persistence" | "run";
      message: string;
    };
'''
    value = replace_once(
        value,
        "\nexport interface DesktopTask {",
        execution_update + "\nexport interface DesktopTask {",
        "insert execution update contract",
    )

    desktop_interface = '''export interface DesktopClient {
  browser: BrowserAdapter;
  bootstrap(): Promise<BootstrapSnapshot>;
  updateSettings(update: SettingsUpdate): Promise<DesktopSettings>;
  searchModels(query: string): Promise<ModelSearchResult[]>;
  assessModel(artifactBytes: number): Promise<ModelAssessment>;
  planModelPlacement(artifactBytes: number): Promise<PlacementPlan>;
  setDefaultModel(selection: ModelSelection): Promise<DesktopSettings>;
  startRuntime(): Promise<RuntimeSnapshot>;
  stopRuntime(): Promise<RuntimeSnapshot>;
  runRuntimeProbe(): Promise<RuntimeProbeReport>;
  runFullEvaluation(scope: EvaluationScope): Promise<FullEvaluationSummary>;
  subscribeEvaluationProgress(
    listener: (progress: EvaluationProgress) => void,
  ): Promise<() => void>;
  downloadModel(
    selection: ModelSelection,
    revision: string,
    expectedBytes: number,
    expectedSha256: string | null,
  ): Promise<DownloadReceipt>;
  cancelDownload(selection: ModelSelection): Promise<boolean>;
  listDownloads(): Promise<DownloadedModel[]>;
  importModel(sourcePath: string): Promise<ModelRegistryEntry>;
  listModelRegistry(): Promise<ModelRegistryEntry[]>;
  subscribeDownloadProgress(
    listener: (progress: DownloadProgress) => void,
  ): Promise<() => void>;
  listProjects(): Promise<DesktopProject[]>;
  createProject(name: string, root: string): Promise<DesktopProject>;
  listTasks(projectId: string): Promise<DesktopTask[]>;
  createTask(input: CreateTaskInput): Promise<DesktopTask>;
  deleteTask(taskId: string): Promise<void>;
  loadTask(taskId: string): Promise<TaskDetail | null>;
  submitPrompt(taskId: string, prompt: string): Promise<SubmitPromptResult>;
  cancelExecution(executionId: string): Promise<Execution>;
  steerExecution(executionId: string, text: string): Promise<TaskMessage>;
  queueFollowUp(executionId: string, text: string): Promise<TaskMessage>;
  subscribeExecutionUpdates(
    listener: (update: ExecutionUpdate) => void,
  ): Promise<() => void>;
  listPendingApprovals(taskId: string): Promise<ToolApproval[]>;
  decideToolApproval(
    approvalId: string,
    approved: boolean,
  ): Promise<ToolApprovalDecision>;
  listProjectFiles(taskId: string, limit?: number): Promise<WorkspaceEntry[]>;
  readProjectFile(
    taskId: string,
    path: string,
    offset?: number,
    limit?: number,
  ): Promise<WorkspaceRead>;
  searchProjectFiles(
    taskId: string,
    query: string,
    limit?: number,
  ): Promise<WorkspaceSearchMatch[]>;
}
'''
    value = sub_once(
        value,
        r'export interface DesktopClient \{.*?\n\}\n\nconst tauriBrowserAdapter',
        desktop_interface + "\nconst tauriBrowserAdapter",
        "replace DesktopClient authority surface",
    )

    tauri_client = '''export const tauriDesktopClient: DesktopClient = {
  browser: tauriBrowserAdapter,
  bootstrap: () => invoke<BootstrapSnapshot>("bootstrap_snapshot"),
  updateSettings: (update) =>
    invoke<DesktopSettings>("update_settings", { update }),
  searchModels: (query) =>
    invoke<ModelSearchResult[]>("search_models", { query }),
  assessModel: (artifactBytes) =>
    invoke<ModelAssessment>("assess_model", { artifactBytes }),
  planModelPlacement: (artifactBytes) =>
    invoke<PlacementPlan>("plan_model_placement", { artifactBytes }),
  setDefaultModel: (selection) =>
    invoke<DesktopSettings>("set_default_model", { selection }),
  startRuntime: () => invoke<RuntimeSnapshot>("start_runtime"),
  stopRuntime: () => invoke<RuntimeSnapshot>("stop_runtime"),
  runRuntimeProbe: () => invoke<RuntimeProbeReport>("run_runtime_probe"),
  runFullEvaluation: (scope) =>
    invoke<FullEvaluationSummary>("run_full_evaluation", { scope }),
  subscribeEvaluationProgress: async (listener) =>
    listen<EvaluationProgress>("evaluation-progress", (event) =>
      listener(event.payload),
    ),
  downloadModel: (selection, revision, expectedBytes, expectedSha256) =>
    invoke<DownloadReceipt>("download_model", {
      selection,
      revision,
      expectedBytes,
      expectedSha256,
    }),
  cancelDownload: (selection) =>
    invoke<boolean>("cancel_download", { selection }),
  listDownloads: () => invoke<DownloadedModel[]>("list_downloads"),
  importModel: (sourcePath) =>
    invoke<ModelRegistryEntry>("import_model", { sourcePath }),
  listModelRegistry: () => invoke<ModelRegistryEntry[]>("list_model_registry"),
  subscribeDownloadProgress: async (listener) =>
    listen<DownloadProgress>("download-progress", (event) =>
      listener(event.payload),
    ),
  listProjects: () => invoke<DesktopProject[]>("list_projects"),
  createProject: (name, root) =>
    invoke<DesktopProject>("create_project", { name, root }),
  listTasks: (projectId) => invoke<DesktopTask[]>("list_tasks", { projectId }),
  createTask: (input) => invoke<DesktopTask>("create_task", { input }),
  deleteTask: (taskId) => invoke<void>("delete_task", { taskId }),
  loadTask: (taskId) => invoke<TaskDetail | null>("load_task", { taskId }),
  submitPrompt: (taskId, prompt) =>
    invoke<SubmitPromptResult>("submit_prompt", { taskId, prompt }),
  cancelExecution: (executionId) =>
    invoke<Execution>("cancel_execution", { executionId }),
  steerExecution: (executionId, text) =>
    invoke<TaskMessage>("steer_execution", { executionId, text }),
  queueFollowUp: (executionId, text) =>
    invoke<TaskMessage>("queue_follow_up", { executionId, text }),
  subscribeExecutionUpdates: async (listener) => {
    const channel = new Channel<ExecutionUpdate>();
    channel.onmessage = listener;
    await invoke<void>("subscribe_execution_updates", { channel });
    return () => {
      channel.onmessage = () => undefined;
    };
  },
  listPendingApprovals: (taskId) =>
    invoke<ToolApproval[]>("list_pending_approvals", { taskId }),
  decideToolApproval: (approvalId, approved) =>
    invoke<ToolApprovalDecision>("decide_tool_approval", {
      approvalId,
      approved,
    }),
  listProjectFiles: (taskId, limit = 2_000) =>
    invoke<WorkspaceEntry[]>("list_project_files", { taskId, limit }),
  readProjectFile: (taskId, path, offset, limit) =>
    invoke<WorkspaceRead>("read_project_file", { taskId, path, offset, limit }),
  searchProjectFiles: (taskId, query, limit = 200) =>
    invoke<WorkspaceSearchMatch[]>("search_project_files", {
      taskId,
      query,
      limit,
    }),
};
'''
    value = sub_once(
        value,
        r'export const tauriDesktopClient: DesktopClient = \{.*?\n\};\n\nconst gib',
        tauri_client + "\nconst gib",
        "replace Tauri client authority surface",
    )

    value = replace_once(
        value,
        "const previewApprovals = new Map<string, ToolApproval>();\n",
        "const previewApprovals = new Map<string, ToolApproval>();\n"
        "const previewExecutionListeners = new Set<(update: ExecutionUpdate) => void>();\n",
        "preview execution listeners",
    )
    value = replace_once(
        value,
        "function previewBrowserEvent(event: BrowserEvent) {\n  for (const listener of previewBrowserListeners) listener(event);\n}\n",
        "function previewBrowserEvent(event: BrowserEvent) {\n"
        "  for (const listener of previewBrowserListeners) listener(event);\n"
        "}\n\n"
        "function previewExecutionUpdate(update: ExecutionUpdate) {\n"
        "  for (const listener of previewExecutionListeners) listener(update);\n"
        "}\n\n"
        "function previewExecution(executionId: string) {\n"
        "  for (const detail of previewDetails.values()) {\n"
        "    const execution = detail.executions?.find((candidate) => candidate.id === executionId);\n"
        "    if (execution) return { detail, execution };\n"
        "  }\n"
        "  return null;\n"
        "}\n",
        "preview execution helpers",
    )
    value = sub_once(
        value,
        r'\n  async resolvePiLaunch\(\) \{.*?\n  \},\n  async runRuntimeProbe',
        "\n  async runRuntimeProbe",
        "remove preview launch credential",
    )

    preview_tasks = '''  async createTask(input) {
    const now = Date.now();
    const task: DesktopTask = {
      id: previewId("task"),
      ...input,
      status: "draft",
      summary: "ready",
      activeExecutionId: null,
      latestExecutionId: null,
      error: null,
      createdAtMs: now,
      updatedAtMs: now,
    };
    previewTasks.unshift(task);
    previewDetails.set(task.id, { task, executions: [], messages: [], events: [] });
    return task;
  },
  async deleteTask(taskId) {
    const index = previewTasks.findIndex((task) => task.id === taskId);
    if (index >= 0) previewTasks.splice(index, 1);
    previewDetails.delete(taskId);
  },
  async loadTask(taskId) {
    return previewDetails.get(taskId) ?? null;
  },
  async submitPrompt(taskId, prompt) {
    const detail = previewDetails.get(taskId);
    if (!detail) throw new Error("Preview task does not exist");
    if (
      (detail.executions ?? []).some((execution) =>
        ["queued", "preparing", "running", "waiting-for-approval", "cancelling"].includes(
          execution.state,
        ),
      )
    )
      throw new Error("Preview task already has an active Execution");
    const now = Date.now();
    const executionId = previewId("execution");
    const specificationId = previewId("execution-spec");
    const execution: Execution = {
      id: executionId,
      taskId,
      executionSpecId: specificationId,
      specification: {
        id: specificationId,
        taskId,
        modelRegistryId: "preview-model-1",
        modelRepoId: detail.task.modelRepoId,
        modelRevision: "0123456789abcdef0123456789abcdef01234567",
        modelFilename: detail.task.modelFilename,
        modelSha256: "a".repeat(64),
        sessionConfigSha256: "b".repeat(64),
        profileName: detail.task.profile,
        profileSha256: "c".repeat(64),
        runtimeName: "official",
        runtimeIdentity: "d".repeat(64),
        adapterIdentity: "pi-agent-core@0.84.2",
        policyIdentity: "alpine-desktop-project-tools-v1",
        contextWindow: 16_384,
        maxTokens: 2_048,
        temperatureMillis: 200,
        legacyUnverified: false,
        createdAtMs: now,
      },
      state: "preparing",
      failure: null,
      queuedAtMs: now,
      startedAtMs: now,
      finishedAtMs: null,
      updatedAtMs: now,
    };
    detail.executions = [...(detail.executions ?? []), execution];
    detail.task.status = "running";
    detail.task.summary = "active";
    detail.task.activeExecutionId = execution.id;
    detail.task.latestExecutionId = execution.id;
    const promptMessage: TaskMessage = {
      id: previewId("message"),
      taskId,
      executionId,
      sequence: detail.messages.length + 1,
      role: "user",
      content: prompt,
      createdAtMs: now,
    };
    detail.messages.push(promptMessage);
    previewExecutionUpdate({
      type: "message",
      taskId,
      executionId,
      message: promptMessage,
    });
    previewExecutionUpdate({
      type: "state",
      taskId,
      executionId,
      execution,
    });
    window.setTimeout(() => {
      if (execution.state !== "preparing") return;
      execution.state = "running";
      previewExecutionUpdate({ type: "state", taskId, executionId, execution });
      const content = "Preview mode accepted the host-owned Execution.";
      previewExecutionUpdate({ type: "delta", taskId, executionId, delta: content });
      const assistant: TaskMessage = {
        id: previewId("message"),
        taskId,
        executionId,
        sequence: detail.messages.length + 1,
        role: "assistant",
        content,
        createdAtMs: Date.now(),
      };
      detail.messages.push(assistant);
      previewExecutionUpdate({
        type: "message",
        taskId,
        executionId,
        message: assistant,
      });
      execution.state = "completed";
      execution.finishedAtMs = Date.now();
      execution.updatedAtMs = execution.finishedAtMs;
      detail.task.status = "completed";
      detail.task.summary = "done";
      detail.task.activeExecutionId = null;
      previewExecutionUpdate({ type: "state", taskId, executionId, execution });
      previewExecutionUpdate({
        type: "terminal",
        taskId,
        executionId,
        execution,
        outcome: "completed",
        error: null,
      });
    }, 10);
    return { execution, promptMessage };
  },
  async cancelExecution(executionId) {
    const found = previewExecution(executionId);
    if (!found) throw new Error("Preview Execution does not exist");
    const { detail, execution } = found;
    execution.state = "cancelling";
    execution.updatedAtMs = Date.now();
    previewExecutionUpdate({
      type: "state",
      taskId: execution.taskId,
      executionId,
      execution,
    });
    window.setTimeout(() => {
      execution.state = "cancelled";
      execution.finishedAtMs = Date.now();
      execution.updatedAtMs = execution.finishedAtMs;
      detail.task.status = "cancelled";
      detail.task.summary = "ready";
      detail.task.activeExecutionId = null;
      previewExecutionUpdate({
        type: "terminal",
        taskId: execution.taskId,
        executionId,
        execution,
        outcome: "cancelled",
        error: null,
      });
    }, 0);
    return execution;
  },
  async steerExecution(executionId, text) {
    const found = previewExecution(executionId);
    if (!found) throw new Error("Preview Execution does not exist");
    const message: TaskMessage = {
      id: previewId("message"),
      taskId: found.execution.taskId,
      executionId,
      sequence: found.detail.messages.length + 1,
      role: "user",
      content: text,
      createdAtMs: Date.now(),
    };
    found.detail.messages.push(message);
    previewExecutionUpdate({
      type: "message",
      taskId: message.taskId,
      executionId,
      message,
    });
    return message;
  },
  async queueFollowUp(executionId, text) {
    return this.steerExecution(executionId, text);
  },
  async subscribeExecutionUpdates(listener) {
    previewExecutionListeners.add(listener);
    return () => previewExecutionListeners.delete(listener);
  },
  async listPendingApprovals(taskId) {
    return [...previewApprovals.values()].filter(
      (approval) => approval.taskId === taskId && approval.state === "pending",
    );
  },
  async decideToolApproval(approvalId, approved) {
    const approval = previewApprovals.get(approvalId);
    if (!approval || approval.state !== "pending")
      throw new Error("Approval already settled");
    approval.state = approved ? "approved" : "denied";
    approval.decidedAtMs = Date.now();
    const detail = previewDetails.get(approval.taskId);
    if (!detail) throw new Error("Preview task does not exist");
    const event: TaskEvent = {
      id: previewId("event"),
      taskId: approval.taskId,
      executionId: approval.executionId,
      sequence: detail.events.length + 1,
      kind: "approval.decided",
      payload: {
        approvalId: approval.id,
        operation: approval.operation,
        approved,
      },
      createdAtMs: approval.decidedAtMs,
    };
    detail.events.push(event);
    previewExecutionUpdate({
      type: "event",
      taskId: approval.taskId,
      executionId: approval.executionId,
      event,
    });
    return { approval, event };
  },
'''
    value = sub_once(
        value,
        r'  async createTask\(input\) \{.*?(?=  async listProjectFiles\(\) \{)',
        preview_tasks,
        "replace preview task authority",
    )

    forbidden = [
        "resolvePiLaunch",
        "createExecution:",
        "transitionExecution:",
        "appendTaskMessage:",
        "appendTaskEvent:",
        "setTaskStatus:",
        "requestToolApproval:",
        "getToolApproval:",
        "editProjectFile:",
        "runProjectShell:",
        "apiKey: \"preview-local-token\"",
    ]
    for symbol in forbidden:
        if symbol in value:
            raise RuntimeError(f"renderer authority remained in desktop.ts: {symbol}")
    write(path, value)


def patch_lib() -> None:
    path = "apps/desktop/src-tauri/src/lib.rs"
    value = read(path)
    value = replace_once(value, "pub mod store;\n", "pub mod store;\npub mod supervisor;\n", "supervisor module")
    value = sub_once(
        value,
        r'use store::\{.*?\};',
        '''use store::{
    CreateTask, DesktopProject, DesktopStore, DesktopTask, ModelRegistryEntry, ModelSource,
    NewExecutionSpecification, RegisterModelArtifact, TaskDetail, ToolApproval,
};''',
        "trim store imports",
    )
    value = replace_once(
        value,
        "use tauri::{AppHandle, Emitter, Manager, State};",
        "use supervisor::TaskSupervisor;\nuse tauri::{AppHandle, Emitter, Manager, State};",
        "supervisor import",
    )
    value = sub_once(
        value,
        r'#\[derive\(Debug, Serialize\)\]\n#\[serde\(rename_all = "camelCase"\)\]\nstruct PiLaunchConfig \{.*?\n\}',
        '''#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PiLaunchConfig {
    pub(crate) model_id: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) context_window: u32,
    pub(crate) max_tokens: u32,
    pub(crate) temperature: f32,
    pub(crate) specification: NewExecutionSpecification,
}''',
        "private worker launch config",
    )
    value = replace_once(
        value,
        "fn resolve_pi_launch_blocking(app: AppHandle) -> Result<PiLaunchConfig, String> {",
        "pub(crate) fn resolve_pi_launch_blocking(app: AppHandle) -> Result<PiLaunchConfig, String> {",
        "expose launch to supervisor only",
    )
    value = sub_once(
        value,
        r'\n#\[tauri::command\]\nasync fn resolve_pi_launch\(app: AppHandle\).*?\n\}\n',
        "\n",
        "remove renderer launch command",
    )
    for name in [
        "create_execution",
        "transition_execution",
        "append_task_message",
        "append_task_event",
        "set_task_status",
        "request_tool_approval",
        "get_tool_approval",
        "decide_tool_approval",
        "edit_project_file",
        "run_project_shell",
    ]:
        value = sub_once(
            value,
            rf'\n#\[tauri::command\]\n(?:async )?fn {name}\(.*?\n\}}\n',
            "\n",
            f"remove direct command {name}",
        )

    old_handler = '''            resolve_pi_launch,
            run_runtime_probe,'''
    value = replace_once(value, old_handler, "            run_runtime_probe,", "remove launch handler")
    for handler in [
        "            create_execution,\n",
        "            transition_execution,\n",
        "            append_task_message,\n",
        "            append_task_event,\n",
        "            set_task_status,\n",
        "            request_tool_approval,\n",
        "            get_tool_approval,\n",
        "            decide_tool_approval,\n",
        "            edit_project_file,\n",
        "            run_project_shell,\n",
    ]:
        value = replace_once(value, handler, "", f"remove handler {handler.strip()}")
    value = replace_once(
        value,
        "            load_task,\n",
        '''            load_task,
            supervisor::connect_agent_worker,
            supervisor::subscribe_execution_updates,
            supervisor::submit_prompt,
            supervisor::cancel_execution,
            supervisor::steer_execution,
            supervisor::queue_follow_up,
            supervisor::decide_tool_approval,
            supervisor::agent_request_tool_approval,
            supervisor::agent_execute_edit,
            supervisor::agent_run_shell,
            supervisor::agent_worker_event,
''',
        "add supervisor command surface",
    )
    value = replace_once(
        value,
        "        .manage(DownloadRegistry::default())\n",
        "        .manage(DownloadRegistry::default())\n        .manage(Arc::new(TaskSupervisor::default()))\n",
        "manage supervisor",
    )
    value = replace_once(
        value,
        '''            app.manage(Arc::new(DesktopStore::open(
                data_dir.join("desktop.sqlite3"),
            )?));
''',
        '''            app.manage(Arc::new(DesktopStore::open(
                data_dir.join("desktop.sqlite3"),
            )?));
            #[cfg(desktop)]
            tauri::WebviewWindowBuilder::new(
                app,
                "agent-worker",
                tauri::WebviewUrl::App("agent.html".into()),
            )
            .title("Alpine Agent Worker")
            .visible(false)
            .skip_taskbar(true)
            .resizable(false)
            .inner_size(1.0, 1.0)
            .build()?;
''',
        "create hidden worker",
    )
    forbidden = [
        "async fn resolve_pi_launch(",
        "fn create_execution(",
        "fn transition_execution(",
        "fn append_task_message(",
        "fn append_task_event(",
        "fn set_task_status(",
        "fn request_tool_approval(",
        "fn get_tool_approval(",
        "fn edit_project_file(",
        "fn run_project_shell(",
    ]
    for symbol in forbidden:
        if symbol in value:
            raise RuntimeError(f"direct renderer command remained in lib.rs: {symbol}")
    write(path, value)


def patch_app_tests() -> None:
    path = "apps/desktop/src/App.test.tsx"
    value = read(path)
    value = sub_once(
        value,
        r'\nvi\.mock\("\./harness/pi".*?\n\}\)\);\n',
        "\n",
        "remove Pi renderer mock",
    )
    value = replace_once(
        value,
        "function client(): DesktopClient {\n  return {",
        '''function client(): DesktopClient {
  const executionListeners = new Set<
    (update: import("./desktop").ExecutionUpdate) => void
  >();
  let executionSequence = 0;
  const emitExecution = (update: import("./desktop").ExecutionUpdate) => {
    for (const listener of executionListeners) listener(update);
  };
  return {''',
        "test execution listeners",
    )
    value = sub_once(
        value,
        r'    resolvePiLaunch: vi\.fn\(\)\.mockResolvedValue\(\{.*?\n    \}\),\n',
        "",
        "remove test launch credential",
    )
    host_double = '''    submitPrompt: vi.fn().mockImplementation(async (taskId, prompt) => {
      executionSequence += 1;
      const executionId = `execution-${executionSequence}`;
      const now = Date.now();
      const execution: import("./desktop").Execution = {
        id: executionId,
        taskId,
        executionSpecId: `spec-${executionSequence}`,
        specification: {
          id: `spec-${executionSequence}`,
          taskId,
          modelRegistryId: "model-1",
          modelRepoId: "Qwen/Qwen3.5-9B-GGUF",
          modelRevision: "a".repeat(40),
          modelFilename: "Qwen3.5-9B-Q4_K_M.gguf",
          modelSha256: "b".repeat(64),
          sessionConfigSha256: "c".repeat(64),
          profileName: "stable-16k",
          profileSha256: "d".repeat(64),
          runtimeName: "official",
          runtimeIdentity: "e".repeat(64),
          adapterIdentity: "pi-agent-core@0.84.2",
          policyIdentity: "alpine-desktop-project-tools-v1",
          contextWindow: 16_384,
          maxTokens: 2_048,
          temperatureMillis: 200,
          legacyUnverified: false,
          createdAtMs: now,
        },
        state: "preparing",
        failure: null,
        queuedAtMs: now,
        startedAtMs: now,
        finishedAtMs: null,
        updatedAtMs: now,
      };
      const promptMessage: import("./desktop").TaskMessage = {
        id: `prompt-${executionSequence}`,
        taskId,
        executionId,
        sequence: 1,
        role: "user",
        content: prompt,
        createdAtMs: now,
      };
      queueMicrotask(() => {
        const completed = {
          ...execution,
          state: "completed" as const,
          finishedAtMs: Date.now(),
        };
        emitExecution({
          type: "terminal",
          taskId,
          executionId,
          execution: completed,
          outcome: "completed",
          error: null,
        });
      });
      return { execution, promptMessage };
    }),
    cancelExecution: vi.fn(),
    steerExecution: vi.fn(),
    queueFollowUp: vi.fn(),
    subscribeExecutionUpdates: vi.fn().mockImplementation(async (listener) => {
      executionListeners.add(listener);
      return () => executionListeners.delete(listener);
    }),
    deleteTask: vi.fn().mockResolvedValue(undefined),
'''
    value = sub_once(
        value,
        r'    createExecution:.*?    deleteTask: vi\.fn\(\)\.mockResolvedValue\(undefined\),\n    appendTaskMessage: vi\.fn\(\),\n    appendTaskEvent: vi\.fn\(\),\n    setTaskStatus: vi\.fn\(\),\n    requestToolApproval: vi\.fn\(\),\n    getToolApproval: vi\.fn\(\)\.mockResolvedValue\(null\),\n',
        host_double,
        "replace App test host double",
    )
    value = replace_once(value, "    editProjectFile: vi.fn(),\n    runProjectShell: vi.fn(),\n", "", "remove effect methods from App double")
    value = re.sub(
        r'    vi\.mocked\(desktop\.setTaskStatus\)\.mockImplementation\(.*?\n    \);\n',
        "",
        value,
        flags=re.S,
    )
    value = replace_once(
        value,
        '''    vi.mocked(desktop.resolvePiLaunch).mockRejectedValue(
      new Error("Local runtime is unavailable"),
    );
''',
        '''    vi.mocked(desktop.submitPrompt).mockRejectedValue(
      new Error("Local runtime is unavailable"),
    );
''',
        "App failure host command",
    )
    value = replace_once(
        value,
        "await waitFor(() => expect(desktop.resolvePiLaunch).toHaveBeenCalledTimes(2));",
        "await waitFor(() => expect(desktop.submitPrompt).toHaveBeenCalledTimes(2));",
        "App retry host command",
    )
    for symbol in ["resolvePiLaunch", "createExecution", "transitionExecution", "setTaskStatus"]:
        if symbol in value:
            raise RuntimeError(f"legacy test authority remained: {symbol}")
    write(path, value)


def patch_pi_test() -> None:
    path = "apps/desktop/src/harness/pi.test.ts"
    value = read(path)
    value = replace_once(
        value,
        'import { PiHarness, localPiModel } from "./pi";\nimport type { DesktopClient } from "../desktop";',
        'import { PiHarness, localPiModel } from "./pi";\nimport type { PiToolClient } from "./pi";',
        "Pi test tool type",
    )
    value = replace_once(
        value,
        '''      taskId: "task-1",
      desktop: {} as DesktopClient,
''',
        '''      taskId: "task-1",
      executionId: "execution-1",
      tools: {} as PiToolClient,
''',
        "Pi test worker dependencies",
    )
    write(path, value)


def patch_policy() -> None:
    path = "config/architecture-policy.json"
    policy = json.loads(read(path))
    policy["providerImports"]["temporaryExceptions"] = []
    policy["rendererMutationBoundary"]["symbols"] = [
        "resolvePiLaunch",
        "createExecution",
        "transitionExecution",
        "appendTaskMessage",
        "appendTaskEvent",
        "setTaskStatus",
        "requestToolApproval",
        "getToolApproval",
        "editProjectFile",
        "runProjectShell",
    ]
    policy["rendererMutationBoundary"]["temporaryAllowedFiles"] = []
    write(path, json.dumps(policy, indent=2) + "\n")


def patch_docs() -> None:
    path = "docs/adr/0027-tauri-desktop-and-pi-agent-runtime.md"
    value = read(path)
    marker = "## Decision\n\n"
    amendment = '''## Decision

Amended 2026-09-01: a host-owned `TaskSupervisor` is now the sole execution
lifecycle authority. The visible renderer submits prompt, cancellation,
steering, follow-up and approval-decision intents and consumes typed execution
projections over an ordered Tauri channel. It never receives the local runtime
credential, constructs Pi, writes durable task facts or decides terminal state.
Pi runs inside a hidden Alpine-owned worker webview whose command surface is
bound to the `agent-worker` webview identity. The Rust host creates the immutable
Execution, persists prompts and normalized results, governs local inference
capacity, settles cancellation and wakes the exact approval continuation.

'''
    value = replace_once(value, marker, amendment, "ADR 0027 amendment")
    old = '''The renderer consumes an Alpine-owned Task execution interface rather than Pi
events or Pi agent state. That module owns launch readiness, Pi construction,
history restoration, event normalization, ordered persistence, animation-frame
stream coalescing, cancellation settlement, steering, follow-up and local timing
marks. The low-level Pi object is private to the adapter; its public descriptor
reports only the selected model, bound Alpine tools and queue modes. A checked
capability manifest lists supported and unsupported Pi behavior in Settings so
experimental adapter status cannot be mistaken for terminal or AgentHarness
parity.
'''
    new = '''The renderer consumes an Alpine-owned Task execution interface rather than Pi
events or Pi agent state. The host supervisor owns launch readiness, immutable
Execution creation, ordered persistence, cancellation settlement, approval
continuations and terminal outcomes. The isolated worker owns only the in-memory
Pi adapter loop and converts provider events into bounded Alpine worker events;
it cannot write durable task history or transition an Execution directly. The
low-level Pi object is private to the worker adapter, and a checked capability
manifest prevents experimental adapter status from being mistaken for terminal
or AgentHarness parity.
'''
    value = replace_once(value, old, new, "ADR 0027 authority paragraph")
    write(path, value)

    path = "docs/adr/0028-desktop-state-and-workspace-authority.md"
    value = read(path)
    marker = "## Decision\n\n"
    amendment = '''## Decision

Amended 2026-09-01: durable task facts are written only by the Tauri host's
`TaskSupervisor` and workspace services. The visible renderer has no arbitrary
message/event/status append commands. The isolated Agent Worker can propose
exact effects and report bounded adapter events, but host-side webview identity
checks, Execution identity checks and SQLite transitions remain authoritative.
Approval decisions are persisted by the host and delivered directly to the
specific waiting worker continuation; the database is not polled as a message
queue.

'''
    value = replace_once(value, marker, amendment, "ADR 0028 amendment")
    write(path, value)

    path = "docs/executions.md"
    value = read(path)
    value += '''

## Host-owned execution authority

The visible React renderer is a projection client. It can submit prompt,
cancellation, steering, follow-up and approval intents, but it cannot construct
the provider adapter, receive the runtime credential, append durable task facts
or transition Execution state. A Rust `TaskSupervisor` reserves local inference
capacity, creates the immutable Execution and persists every authoritative
outcome. Pi runs in an isolated `agent-worker` webview and reports bounded events
over a Tauri channel; worker-originated calls are checked against both webview
identity and the active Task/Execution pair.
'''
    write(path, value)


def add_authority_test() -> None:
    path = ROOT / "apps/desktop/src/authority-boundary.test.ts"
    path.write_text(
        '''import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(path, "utf8");

describe("desktop execution authority", () => {
  it("keeps credentials and durable mutation primitives out of the visible renderer", () => {
    const desktop = read("src/desktop.ts");
    const taskExecution = read("src/task-execution.ts");
    for (const forbidden of [
      "apiKey",
      "resolvePiLaunch",
      "createExecution",
      "transitionExecution",
      "appendTaskMessage",
      "appendTaskEvent",
      "setTaskStatus",
      "requestToolApproval",
      "getToolApproval",
      "editProjectFile",
      "runProjectShell",
    ]) {
      expect(desktop).not.toContain(forbidden);
    }
    expect(taskExecution).not.toContain("@earendil-works/pi-");
    expect(taskExecution).toContain("submitPrompt");
    expect(taskExecution).toContain("subscribeExecutionUpdates");
  });

  it("binds provider execution to the isolated worker and host supervisor", () => {
    const host = read("src-tauri/src/lib.rs");
    const supervisor = read("src-tauri/src/supervisor.rs");
    const worker = read("src/agent-worker.ts");
    expect(host).toContain('"agent-worker"');
    expect(host).toContain("supervisor::submit_prompt");
    expect(host).not.toContain("resolve_pi_launch,");
    expect(supervisor).toContain("require_webview");
    expect(supervisor).toContain("ExecutionState::Cancelling");
    expect(worker).toContain("connect_agent_worker");
    expect(worker).toContain("PiHarness");
  });
});
''',
        encoding="utf-8",
    )


patch_desktop()
patch_lib()
patch_app_tests()
patch_pi_test()
patch_policy()
patch_docs()
add_authority_test()
print("issue #47 host-supervisor cutover applied")
