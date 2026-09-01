import type { AgentEvent, AgentMessage } from "@earendil-works/pi-agent-core";
import { describe, expect, it, vi } from "vitest";
import type {
  DesktopClient,
  DesktopTask,
  Execution,
  ExecutionState,
  TaskMessage,
} from "./desktop";
import {
  createTaskExecution,
  type AgentRuntime,
  type TaskExecutionUpdate,
} from "./task-execution";

const specification = {
  modelRegistryId: "model-1",
  modelRepoId: "Qwen/Qwen-GGUF",
  modelRevision: "a".repeat(40),
  modelFilename: "Qwen.gguf",
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
};

const launch = {
  modelId: "Qwen.gguf",
  baseUrl: "http://127.0.0.1:8100",
  apiKey: "local-token",
  contextWindow: 16_384,
  maxTokens: 2_048,
  temperature: 0.2,
  specification,
};

function task(): DesktopTask {
  return {
    id: "task-1",
    projectId: "project-1",
    title: "Inspect the project",
    status: "draft",
    summary: "ready",
    activeExecutionId: null,
    latestExecutionId: null,
    modelRepoId: "local/Qwen",
    modelFilename: "Qwen.gguf",
    profile: "stable-16k",
    error: null,
    createdAtMs: 1,
    updatedAtMs: 1,
  };
}

function execution(state: ExecutionState = "queued"): Execution {
  return {
    id: "execution-1",
    taskId: "task-1",
    executionSpecId: "spec-1",
    specification: {
      id: "spec-1",
      taskId: "task-1",
      ...specification,
      modelRegistryId: specification.modelRegistryId,
      modelSha256: specification.modelSha256,
      sessionConfigSha256: specification.sessionConfigSha256,
      profileSha256: specification.profileSha256,
      legacyUnverified: false,
      createdAtMs: 1,
    },
    state,
    failure: null,
    queuedAtMs: 1,
    startedAtMs: state === "queued" ? null : 2,
    finishedAtMs: ["completed", "cancelled", "failed", "interrupted"].includes(
      state,
    )
      ? 3
      : null,
    updatedAtMs: 2,
  };
}

function desktopDouble() {
  let messageSequence = 0;
  let eventSequence = 0;
  let current = execution();
  const client = {
    resolvePiLaunch: vi.fn().mockResolvedValue(launch),
    createExecution: vi.fn(async () => current) as DesktopClient["createExecution"],
    transitionExecution: vi.fn(async (_executionId, state, failure = null) => {
      current = {
        ...current,
        state,
        failure,
        startedAtMs: current.startedAtMs ?? Date.now(),
        finishedAtMs: ["completed", "cancelled", "failed", "interrupted"].includes(
          state,
        )
          ? Date.now()
          : null,
      };
      return current;
    }) as DesktopClient["transitionExecution"],
    appendTaskMessage: vi.fn(async ({ taskId, executionId, role, content }) => ({
      id: `message-${++messageSequence}`,
      taskId,
      executionId,
      sequence: messageSequence,
      role,
      content,
      createdAtMs: messageSequence,
    })) as DesktopClient["appendTaskMessage"],
    appendTaskEvent: vi.fn(async ({ taskId, executionId, kind, payload }) => ({
      id: `event-${++eventSequence}`,
      taskId,
      executionId,
      sequence: eventSequence,
      kind,
      payload,
      createdAtMs: eventSequence,
    })) as DesktopClient["appendTaskEvent"],
  } as unknown as DesktopClient;
  return client;
}

function runtimeThatEmits(
  events: AgentEvent[],
  errorMessage?: string,
): AgentRuntime {
  const listeners: Array<(event: AgentEvent) => void | Promise<void>> = [];
  return {
    errorMessage,
    subscribe(listener) {
      listeners.push(listener);
      return () => undefined;
    },
    async prompt() {
      for (const event of events) {
        for (const listener of listeners) await listener(event);
      }
    },
    abort: vi.fn(),
    steer: vi.fn(),
    followUp: vi.fn(),
  };
}

describe("Task execution", () => {
  it("creates one durable Execution and binds streaming history to its identity", async () => {
    const desktop = desktopDouble();
    const assistantMessage: AgentMessage = {
      role: "assistant",
      content: [{ type: "text", text: "Done." }],
      api: "openai-completions",
      provider: "alpine-local",
      model: "Qwen.gguf",
      usage: {
        input: 1,
        output: 1,
        cacheRead: 0,
        cacheWrite: 0,
        totalTokens: 2,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
      },
      stopReason: "stop",
      timestamp: 2,
    };
    const runtime = runtimeThatEmits([
      { type: "agent_start" } as AgentEvent,
      {
        type: "message_update",
        message: assistantMessage,
        assistantMessageEvent: {
          type: "text_delta",
          contentIndex: 0,
          delta: "Done.",
        },
      } as AgentEvent,
      { type: "message_end", message: assistantMessage } as AgentEvent,
      { type: "agent_end", messages: [assistantMessage] } as AgentEvent,
    ]);
    const updates: TaskExecutionUpdate[] = [];
    const executionController = createTaskExecution(
      {
        desktop,
        task: task(),
        history: [],
        onUpdate: (update) => updates.push(update),
      },
      {
        createRuntime: vi.fn().mockResolvedValue(runtime),
        scheduleFrame: (callback) => {
          callback();
          return 1;
        },
        cancelFrame: () => undefined,
      },
    );

    const result = await executionController.run("Inspect the project");

    expect(result).toMatchObject({
      taskId: "task-1",
      executionId: "execution-1",
      response: "Done.",
      state: "done",
    });
    expect(desktop.createExecution).toHaveBeenCalledWith({
      taskId: "task-1",
      specification,
    });
    expect(desktop.transitionExecution).toHaveBeenNthCalledWith(
      1,
      "execution-1",
      "preparing",
      null,
    );
    expect(desktop.transitionExecution).toHaveBeenCalledWith(
      "execution-1",
      "completed",
      null,
    );
    expect(desktop.appendTaskMessage).toHaveBeenCalledWith({
      taskId: "task-1",
      executionId: "execution-1",
      role: "assistant",
      content: "Done.",
    });
    expect(desktop.appendTaskEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        taskId: "task-1",
        executionId: "execution-1",
        kind: "agent.started",
      }),
    );
    expect(updates.some((update) => update.type === "message")).toBe(true);
  });

  it("settles cancellation against the exact active Execution", async () => {
    const desktop = desktopDouble();
    let releaseLaunch!: (runtime: AgentRuntime) => void;
    const runtimePromise = new Promise<AgentRuntime>((resolve) => {
      releaseLaunch = resolve;
    });
    const controller = createTaskExecution(
      { desktop, task: task(), history: [], onUpdate: () => undefined },
      {
        createRuntime: () => runtimePromise,
        scheduleFrame: () => 1,
        cancelFrame: () => undefined,
      },
    );

    const run = controller.run("Wait for launch");
    await vi.waitFor(() => expect(desktop.createExecution).toHaveBeenCalled());
    controller.cancel();
    releaseLaunch(runtimeThatEmits([]));

    await expect(run).resolves.toMatchObject({
      executionId: "execution-1",
      state: "cancelled",
    });
    expect(desktop.transitionExecution).toHaveBeenCalledWith(
      "execution-1",
      "cancelling",
      null,
    );
    expect(desktop.transitionExecution).toHaveBeenCalledWith(
      "execution-1",
      "cancelled",
      null,
    );
  });

  it("does not claim cancellation when its durable transition fails", async () => {
    const desktop = desktopDouble();
    let releaseLaunch!: (runtime: AgentRuntime) => void;
    const runtimePromise = new Promise<AgentRuntime>((resolve) => {
      releaseLaunch = resolve;
    });
    const createRuntime = vi.fn(() => runtimePromise);
    const controller = createTaskExecution(
      { desktop, task: task(), history: [], onUpdate: () => undefined },
      {
        createRuntime,
        scheduleFrame: () => 1,
        cancelFrame: () => undefined,
      },
    );

    const run = controller.run("Wait for launch");
    await vi.waitFor(() => expect(createRuntime).toHaveBeenCalled());
    vi.mocked(desktop.transitionExecution).mockRejectedValueOnce(
      new Error("state store unavailable"),
    );
    controller.cancel();
    releaseLaunch(runtimeThatEmits([]));

    await expect(run).resolves.toMatchObject({
      executionId: "execution-1",
      state: "error",
      error: "Cancellation state could not be saved: state store unavailable",
    });
    expect(desktop.transitionExecution).toHaveBeenCalledWith(
      "execution-1",
      "cancelling",
      null,
    );
    expect(desktop.transitionExecution).toHaveBeenCalledWith(
      "execution-1",
      "failed",
      "Cancellation state could not be saved: state store unavailable",
    );
    expect(desktop.transitionExecution).not.toHaveBeenCalledWith(
      "execution-1",
      "cancelled",
      null,
    );
  });

  it("fails a queued Execution when preparation persistence fails", async () => {
    const desktop = desktopDouble();
    const transition = vi.mocked(desktop.transitionExecution);
    transition
      .mockRejectedValueOnce(new Error("state store unavailable"))
      .mockResolvedValueOnce(execution("failed"));
    const createRuntime = vi.fn().mockResolvedValue(runtimeThatEmits([]));
    const controller = createTaskExecution(
      { desktop, task: task(), history: [], onUpdate: () => undefined },
      {
        createRuntime,
        scheduleFrame: () => 1,
        cancelFrame: () => undefined,
      },
    );

    const result = await controller.run("Continue");

    expect(createRuntime).not.toHaveBeenCalled();
    expect(result).toMatchObject({
      executionId: "execution-1",
      state: "error",
      error: "state store unavailable",
    });
    expect(transition).toHaveBeenNthCalledWith(
      2,
      "execution-1",
      "failed",
      "state store unavailable",
    );
  });

  it("records failure on the Execution without mutating Task lifecycle fields", async () => {
    const desktop = desktopDouble();
    const history: TaskMessage[] = [];
    const runtime = runtimeThatEmits([], "Local model rejected the request");
    const createRuntime = vi.fn().mockResolvedValue(runtime);
    const controller = createTaskExecution(
      { desktop, task: task(), history, onUpdate: () => undefined },
      {
        createRuntime,
        scheduleFrame: () => 1,
        cancelFrame: () => undefined,
      },
    );

    const result = await controller.run("Continue");

    expect(createRuntime).toHaveBeenCalledWith(
      launch,
      expect.objectContaining({
        taskId: "task-1",
        executionId: "execution-1",
        history,
      }),
    );
    expect(result).toMatchObject({
      executionId: "execution-1",
      state: "error",
      error: "Local model rejected the request",
    });
    expect(desktop.transitionExecution).toHaveBeenCalledWith(
      "execution-1",
      "failed",
      "Local model rejected the request",
    );
  });
});
