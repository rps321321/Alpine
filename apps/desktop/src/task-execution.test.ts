import type { AgentEvent, AgentMessage } from "@earendil-works/pi-agent-core";
import { describe, expect, it, vi } from "vitest";
import type { DesktopClient, DesktopTask, TaskMessage } from "./desktop";
import {
  createTaskExecution,
  type AgentRuntime,
  type TaskExecutionUpdate,
} from "./task-execution";

const launch = {
  modelId: "Qwen.gguf",
  baseUrl: "http://127.0.0.1:8100",
  apiKey: "local-token",
  contextWindow: 16_384,
  maxTokens: 2_048,
  temperature: 0.2,
};

function task(status: DesktopTask["status"] = "draft"): DesktopTask {
  return {
    id: "task-1",
    projectId: "project-1",
    title: "Inspect the project",
    status,
    modelRepoId: "local/Qwen",
    modelFilename: "Qwen.gguf",
    profile: "stable-16k",
    error: null,
    createdAtMs: 1,
    updatedAtMs: 1,
  };
}

function desktopDouble() {
  let messageSequence = 0;
  let eventSequence = 0;
  const client = {
    resolvePiLaunch: vi.fn().mockResolvedValue(launch),
    appendTaskMessage: vi.fn(async ({ taskId, role, content }) => ({
      id: `message-${++messageSequence}`,
      taskId,
      sequence: messageSequence,
      role,
      content,
      createdAtMs: messageSequence,
    })) as DesktopClient["appendTaskMessage"],
    appendTaskEvent: vi.fn(async ({ taskId, kind, payload }) => ({
      id: `event-${++eventSequence}`,
      taskId,
      sequence: eventSequence,
      kind,
      payload,
      createdAtMs: eventSequence,
    })) as DesktopClient["appendTaskEvent"],
    setTaskStatus: vi.fn(async (_taskId, status, error = null) => ({
      ...task(status),
      error,
    })) as DesktopClient["setTaskStatus"],
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
  it("streams and persists a Pi run through Alpine-owned updates", async () => {
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
        cost: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          total: 0,
        },
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
    const execution = createTaskExecution(
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

    const result = await execution.run("Inspect the project");

    expect(result).toEqual({
      taskId: "task-1",
      prompt: "Inspect the project",
      response: "Done.",
      state: "done",
    });
    expect(desktop.setTaskStatus).toHaveBeenNthCalledWith(
      1,
      "task-1",
      "running",
    );
    expect(desktop.setTaskStatus).toHaveBeenLastCalledWith(
      "task-1",
      "completed",
    );
    expect(desktop.appendTaskMessage).toHaveBeenCalledWith({
      taskId: "task-1",
      role: "assistant",
      content: "Done.",
    });
    expect(desktop.appendTaskEvent).toHaveBeenCalledWith({
      taskId: "task-1",
      kind: "agent.started",
      payload: {},
    });
    expect(
      updates.some(
        (update) => update.type === "response" && update.response === "Done.",
      ),
    ).toBe(true);
    expect(
      updates.filter(
        (update): update is Extract<TaskExecutionUpdate, { type: "event" }> =>
          update.type === "event",
      ),
    ).toHaveLength(3);
  });

  it("settles cancellation requested while launch readiness is pending", async () => {
    const desktop = desktopDouble();
    let releaseLaunch!: (runtime: AgentRuntime) => void;
    const runtimePromise = new Promise<AgentRuntime>((resolve) => {
      releaseLaunch = resolve;
    });
    const execution = createTaskExecution(
      { desktop, task: task(), history: [], onUpdate: () => undefined },
      {
        createRuntime: () => runtimePromise,
        scheduleFrame: () => 1,
        cancelFrame: () => undefined,
      },
    );

    const run = execution.run("Wait for launch");
    execution.cancel();
    releaseLaunch(runtimeThatEmits([]));

    await expect(run).resolves.toMatchObject({ state: "cancelled" });
    expect(desktop.setTaskStatus).toHaveBeenCalledWith("task-1", "cancelling");
    expect(desktop.setTaskStatus).toHaveBeenLastCalledWith(
      "task-1",
      "cancelled",
    );
  });

  it("restores Alpine messages and reports runtime failures without exposing Pi state", async () => {
    const desktop = desktopDouble();
    const history: TaskMessage[] = [
      {
        id: "message-1",
        taskId: "task-1",
        sequence: 1,
        role: "user",
        content: "Earlier direction",
        createdAtMs: 10,
      },
    ];
    const runtime = runtimeThatEmits([], "Local model rejected the request");
    const createRuntime = vi.fn().mockResolvedValue(runtime);
    const updates: TaskExecutionUpdate[] = [];
    const execution = createTaskExecution(
      {
        desktop,
        task: task(),
        history,
        onUpdate: (update) => updates.push(update),
      },
      {
        createRuntime,
        scheduleFrame: () => 1,
        cancelFrame: () => undefined,
      },
    );

    const result = await execution.run("Continue");

    expect(createRuntime).toHaveBeenCalledWith(
      launch,
      expect.objectContaining({ taskId: "task-1", history }),
    );
    expect(result).toMatchObject({
      state: "error",
      error: "Local model rejected the request",
    });
    expect(desktop.setTaskStatus).toHaveBeenLastCalledWith(
      "task-1",
      "failed",
      "Local model rejected the request",
    );
    expect(
      updates.some(
        (update) =>
          update.type === "error" &&
          update.message === "Local model rejected the request",
      ),
    ).toBe(true);
  });
});
