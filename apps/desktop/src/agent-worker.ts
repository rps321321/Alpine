import { Channel, invoke } from "@tauri-apps/api/core";
import { ApprovalContinuations } from "./approval-continuations";
import type {
  TaskMessage,
  ToolApproval,
  WorkspaceEdit,
  WorkspaceEditResult,
  WorkspaceEntry,
  WorkspaceRead,
  WorkspaceSearchMatch,
  WorkspaceShell,
  WorkspaceShellResult,
} from "./desktop";
import {
  PiHarness,
  type PiHarnessEvent,
  type PiLocalModelConfig,
  type PiToolClient,
} from "./harness/pi";

type WorkerLaunchConfig = PiLocalModelConfig & {
  specification: unknown;
};

type AgentWorkerCommand =
  | {
      type: "start";
      taskId: string;
      executionId: string;
      prompt: string;
      history: TaskMessage[];
      config: WorkerLaunchConfig;
    }
  | { type: "cancel"; executionId: string }
  | { type: "steer"; executionId: string; text: string }
  | { type: "follow-up"; executionId: string; text: string }
  | { type: "approval-decision"; approvalId: string; approved: boolean };

type AgentWorkerEventPayload =
  | { type: "started"; taskId: string; executionId: string }
  | { type: "delta"; taskId: string; executionId: string; delta: string }
  | {
      type: "message";
      taskId: string;
      executionId: string;
      role: "assistant";
      content: string;
    }
  | {
      type: "trace";
      taskId: string;
      executionId: string;
      kind: string;
      payload: unknown;
    }
  | {
      type: "completed" | "cancelled";
      taskId: string;
      executionId: string;
      durationMs: number;
      responseCharacters: number;
    }
  | {
      type: "failed";
      taskId: string;
      executionId: string;
      error: string;
      durationMs: number;
      responseCharacters: number;
    };

type AgentWorkerEvent = AgentWorkerEventPayload & { sequence: number };

type ActiveWorkerRun = {
  taskId: string;
  executionId: string;
  harness: PiHarness;
  unsubscribe: () => void;
  cancelled: boolean;
  startedAt: number;
  responseCharacters: number;
  deltaBuffer: string;
  deltaTimer: number | null;
  reportSequence: number;
  reportQueue: Promise<void>;
  reportFailure: Error | null;
};

const approvalContinuations = new ApprovalContinuations();
let active: ActiveWorkerRun | null = null;

function asError(error: unknown) {
  return error instanceof Error ? error : new Error(String(error));
}

function matchesActive(executionId: string) {
  return active?.executionId === executionId ? active : null;
}

function duration(run: ActiveWorkerRun) {
  return Math.max(0, Math.round(performance.now() - run.startedAt));
}

function sequenceReport(run: ActiveWorkerRun, event: AgentWorkerEventPayload) {
  run.reportSequence += 1;
  return { ...event, sequence: run.reportSequence } as AgentWorkerEvent;
}

function queueReport(run: ActiveWorkerRun, event: AgentWorkerEventPayload) {
  const sequenced = sequenceReport(run, event);
  run.reportQueue = run.reportQueue
    .then(async () => {
      if (run.reportFailure) return;
      await invoke<void>("agent_worker_event", { event: sequenced });
    })
    .catch((error: unknown) => {
      run.reportFailure = asError(error);
      run.harness.abort();
    });
  return run.reportQueue;
}

function clearDeltaTimer(run: ActiveWorkerRun) {
  if (run.deltaTimer != null) window.clearTimeout(run.deltaTimer);
  run.deltaTimer = null;
}

function flushDelta(run: ActiveWorkerRun) {
  clearDeltaTimer(run);
  if (!run.deltaBuffer) return;
  const delta = run.deltaBuffer;
  run.deltaBuffer = "";
  queueReport(run, {
    type: "delta",
    taskId: run.taskId,
    executionId: run.executionId,
    delta,
  });
}

function bufferDelta(run: ActiveWorkerRun, delta: string) {
  run.responseCharacters += delta.length;
  run.deltaBuffer += delta;
  if (run.deltaTimer != null) return;
  run.deltaTimer = window.setTimeout(() => flushDelta(run), 16);
}

function handlePiEvent(run: ActiveWorkerRun, event: PiHarnessEvent) {
  if (active !== run || run.reportFailure) return;
  if (event.type === "delta") {
    bufferDelta(run, event.delta);
    return;
  }
  flushDelta(run);
  if (event.type === "message") {
    queueReport(run, {
      type: "message",
      taskId: run.taskId,
      executionId: run.executionId,
      role: event.role,
      content: event.content,
    });
    return;
  }
  queueReport(run, {
    type: "trace",
    taskId: run.taskId,
    executionId: run.executionId,
    kind: event.kind,
    payload: event.payload,
  });
}

function rejectApprovals(error: Error) {
  approvalContinuations.rejectAll(error);
}

function settleApproval(approvalId: string, approved: boolean) {
  approvalContinuations.settle(approvalId, approved);
}

function waitForApproval(approvalId: string, signal?: AbortSignal) {
  return approvalContinuations.wait(approvalId, signal);
}

function toolClient(): PiToolClient {
  return {
    listProjectFiles: (taskId, limit) =>
      invoke<WorkspaceEntry[]>("list_project_files", { taskId, limit }),
    readProjectFile: (taskId, path, offset, limit) =>
      invoke<WorkspaceRead>("read_project_file", {
        taskId,
        path,
        offset,
        limit,
      }),
    searchProjectFiles: (taskId, query, limit) =>
      invoke<WorkspaceSearchMatch[]>("search_project_files", {
        taskId,
        query,
        limit,
      }),
    proposeEffectApproval: (input) =>
      invoke<ToolApproval>("agent_request_tool_approval", { input }),
    waitForApproval,
    executeApprovedEdit: (taskId, executionId, approvalId, edit: WorkspaceEdit) =>
      invoke<WorkspaceEditResult>("agent_execute_edit", {
        taskId,
        executionId,
        approvalId,
        edit,
      }),
    executeApprovedShell: (
      taskId,
      executionId,
      approvalId,
      shell: WorkspaceShell,
    ) =>
      invoke<WorkspaceShellResult>("agent_run_shell", {
        taskId,
        executionId,
        approvalId,
        shell,
      }),
  };
}

async function start(command: Extract<AgentWorkerCommand, { type: "start" }>) {
  if (active) {
    const event: AgentWorkerEvent = {
      type: "failed",
      taskId: command.taskId,
      executionId: command.executionId,
      error: `Agent Worker already owns Execution ${active.executionId}`,
      durationMs: 0,
      responseCharacters: 0,
      sequence: 1,
    };
    await invoke<void>("agent_worker_event", { event });
    return;
  }

  const harness = new PiHarness(command.config, {
    taskId: command.taskId,
    executionId: command.executionId,
    tools: toolClient(),
    history: command.history,
  });
  const run: ActiveWorkerRun = {
    taskId: command.taskId,
    executionId: command.executionId,
    harness,
    unsubscribe: () => undefined,
    cancelled: false,
    startedAt: performance.now(),
    responseCharacters: 0,
    deltaBuffer: "",
    deltaTimer: null,
    reportSequence: 0,
    reportQueue: Promise.resolve(),
    reportFailure: null,
  };
  active = run;
  run.unsubscribe = harness.subscribe((event) => handlePiEvent(run, event));

  try {
    await queueReport(run, {
      type: "started",
      taskId: run.taskId,
      executionId: run.executionId,
    });
    if (run.reportFailure) throw run.reportFailure;
    await harness.prompt(command.prompt);
    flushDelta(run);
    await run.reportQueue;
    if (run.reportFailure) throw run.reportFailure;

    if (run.cancelled) {
      await queueReport(run, {
        type: "cancelled",
        taskId: run.taskId,
        executionId: run.executionId,
        durationMs: duration(run),
        responseCharacters: run.responseCharacters,
      });
    } else if (harness.errorMessage) {
      await queueReport(run, {
        type: "failed",
        taskId: run.taskId,
        executionId: run.executionId,
        error: harness.errorMessage,
        durationMs: duration(run),
        responseCharacters: run.responseCharacters,
      });
    } else {
      await queueReport(run, {
        type: "completed",
        taskId: run.taskId,
        executionId: run.executionId,
        durationMs: duration(run),
        responseCharacters: run.responseCharacters,
      });
    }
    await run.reportQueue;
  } catch (error) {
    const failure = asError(error);
    if (!run.reportFailure) {
      const event = sequenceReport(run, {
        type: run.cancelled ? "cancelled" : "failed",
        taskId: run.taskId,
        executionId: run.executionId,
        ...(run.cancelled ? {} : { error: failure.message }),
        durationMs: duration(run),
        responseCharacters: run.responseCharacters,
      } as AgentWorkerEventPayload);
      await invoke<void>("agent_worker_event", { event }).catch(() => undefined);
    }
  } finally {
    clearDeltaTimer(run);
    run.unsubscribe();
    rejectApprovals(new Error("Execution ended before approval settlement"));
    if (active === run) active = null;
  }
}

function handleCommand(command: AgentWorkerCommand) {
  switch (command.type) {
    case "start":
      void start(command);
      return;
    case "cancel": {
      const run = matchesActive(command.executionId);
      if (!run) return;
      run.cancelled = true;
      rejectApprovals(new Error("Execution was cancelled"));
      run.harness.abort();
      return;
    }
    case "steer":
      matchesActive(command.executionId)?.harness.steer(command.text);
      return;
    case "follow-up":
      matchesActive(command.executionId)?.harness.followUp(command.text);
      return;
    case "approval-decision":
      settleApproval(command.approvalId, command.approved);
  }
}

async function connect() {
  const channel = new Channel<AgentWorkerCommand>();
  channel.onmessage = handleCommand;
  await invoke<void>("connect_agent_worker", { channel });
}

if ("__TAURI_INTERNALS__" in window) {
  void connect().catch((error) => console.error("Agent Worker failed to connect", error));
}
