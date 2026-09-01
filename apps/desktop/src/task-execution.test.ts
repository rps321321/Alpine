import { describe, expect, it, vi } from "vitest";
import type {
  DesktopClient,
  DesktopTask,
  Execution,
  ExecutionUpdate,
  SubmitPromptResult,
  TaskMessage,
} from "./desktop";
import {
  createTaskExecution,
  type TaskExecutionUpdate,
} from "./task-execution";

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

function execution(state: Execution["state"] = "preparing"): Execution {
  return {
    id: "execution-1",
    taskId: "task-1",
    executionSpecId: "spec-1",
    specification: {
      id: "spec-1",
      taskId: "task-1",
      modelRegistryId: "model-1",
      modelRepoId: "local/Qwen",
      modelRevision: null,
      modelFilename: "Qwen.gguf",
      modelSha256: "a".repeat(64),
      sessionConfigSha256: "b".repeat(64),
      profileName: "stable-16k",
      profileSha256: "c".repeat(64),
      runtimeName: "official",
      runtimeIdentity: "d".repeat(64),
      adapterIdentity: "pi-agent-core@0.84.2",
      policyIdentity: "alpine-desktop-project-tools-v1",
      contextWindow: 16_384,
      maxTokens: 2_048,
      temperatureMillis: 200,
      legacyUnverified: false,
      createdAtMs: 1,
    },
    state,
    failure: null,
    queuedAtMs: 1,
    startedAtMs: state === "preparing" ? 2 : null,
    finishedAtMs: null,
    updatedAtMs: 2,
  };
}

function promptMessage(): TaskMessage {
  return {
    id: "message-user",
    taskId: "task-1",
    executionId: "execution-1",
    sequence: 1,
    role: "user",
    content: "Inspect the project",
    createdAtMs: 2,
  };
}

function desktopDouble() {
  const listeners = new Set<(update: ExecutionUpdate) => void>();
  const emit = (update: ExecutionUpdate) => {
    for (const listener of listeners) listener(update);
  };
  const accepted: SubmitPromptResult = {
    execution: execution(),
    promptMessage: promptMessage(),
  };
  const client = {
    subscribeExecutionUpdates: vi.fn(async (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    }),
    submitPrompt: vi.fn().mockResolvedValue(accepted),
    cancelExecution: vi.fn().mockResolvedValue(execution("cancelling")),
    steerExecution: vi.fn().mockResolvedValue(promptMessage()),
    queueFollowUp: vi.fn().mockResolvedValue(promptMessage()),
  } as unknown as DesktopClient;
  return { client, emit, accepted };
}

const immediateFrames = {
  scheduleFrame(callback: () => void) {
    callback();
    return 1;
  },
  cancelFrame() {},
};

describe("Task execution renderer proxy", () => {
  it("submits intent and renders host-owned updates", async () => {
    const { client, emit } = desktopDouble();
    const updates: TaskExecutionUpdate[] = [];
    const proxy = createTaskExecution(
      {
        desktop: client,
        task: task(),
        history: [],
        onUpdate: (update) => updates.push(update),
      },
      immediateFrames,
    );

    const run = proxy.run("Inspect the project");
    await vi.waitFor(() => expect(client.submitPrompt).toHaveBeenCalled());
    emit({
      type: "state",
      taskId: "task-1",
      executionId: "execution-1",
      execution: execution("running"),
    });
    emit({
      type: "delta",
      taskId: "task-1",
      executionId: "execution-1",
      delta: "Done.",
    });
    const assistant: TaskMessage = {
      id: "message-assistant",
      taskId: "task-1",
      executionId: "execution-1",
      sequence: 2,
      role: "assistant",
      content: "Done.",
      createdAtMs: 3,
    };
    emit({
      type: "message",
      taskId: "task-1",
      executionId: "execution-1",
      message: assistant,
    });
    emit({
      type: "terminal",
      taskId: "task-1",
      executionId: "execution-1",
      execution: { ...execution("completed"), finishedAtMs: 4 },
      outcome: "completed",
      error: null,
    });

    await expect(run).resolves.toEqual({
      taskId: "task-1",
      executionId: "execution-1",
      prompt: "Inspect the project",
      response: "Done.",
      state: "done",
    });
    expect(client.submitPrompt).toHaveBeenCalledWith(
      "task-1",
      "Inspect the project",
    );
    expect(
      updates.some(
        (update) => update.type === "response" && update.response === "Done.",
      ),
    ).toBe(true);
    expect(
      updates.some(
        (update) =>
          update.type === "message" && update.message.id === assistant.id,
      ),
    ).toBe(true);
  });

  it("settles cancellation requested while host acceptance is pending", async () => {
    const { client, emit, accepted } = desktopDouble();
    let accept!: (value: SubmitPromptResult) => void;
    vi.mocked(client.submitPrompt).mockReturnValue(
      new Promise<SubmitPromptResult>((resolve) => {
        accept = resolve;
      }),
    );
    const proxy = createTaskExecution(
      {
        desktop: client,
        task: task(),
        history: [],
        onUpdate: () => undefined,
      },
      immediateFrames,
    );

    const run = proxy.run("Wait for host");
    proxy.cancel();
    accept(accepted);
    await vi.waitFor(() =>
      expect(client.cancelExecution).toHaveBeenCalledWith("execution-1"),
    );
    emit({
      type: "terminal",
      taskId: "task-1",
      executionId: "execution-1",
      execution: { ...execution("cancelled"), finishedAtMs: 4 },
      outcome: "cancelled",
      error: null,
    });

    await expect(run).resolves.toMatchObject({
      executionId: "execution-1",
      state: "cancelled",
    });
  });

  it("reports host submission failures without importing provider state", async () => {
    const { client } = desktopDouble();
    vi.mocked(client.submitPrompt).mockRejectedValue(
      new Error("Local runtime is unavailable"),
    );
    const updates: TaskExecutionUpdate[] = [];
    const proxy = createTaskExecution(
      {
        desktop: client,
        task: task(),
        history: [],
        onUpdate: (update) => updates.push(update),
      },
      immediateFrames,
    );

    await expect(proxy.run("Continue")).resolves.toMatchObject({
      state: "error",
      error: "Local runtime is unavailable",
    });
    expect(
      updates.some(
        (update) =>
          update.type === "error" &&
          update.message === "Local runtime is unavailable",
      ),
    ).toBe(true);
  });
});
