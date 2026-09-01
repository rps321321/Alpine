import type { AgentEvent, AgentMessage } from "@earendil-works/pi-agent-core";
import type {
  DesktopClient,
  DesktopTask,
  Execution,
  ExecutionState,
  TaskEvent,
  TaskMessage,
  ToolApproval,
} from "./desktop";
import type { PiHarnessDependencies, PiLocalModelConfig } from "./harness/pi";

export interface AgentRuntime {
  readonly errorMessage?: string;
  prompt(text: string): Promise<void>;
  subscribe(listener: (event: AgentEvent) => void | Promise<void>): () => void;
  abort(): void;
  steer(text: string): void;
  followUp(text: string): void;
}

export type TaskExecutionUpdate =
  | { type: "response"; taskId: string; prompt: string; response: string }
  | { type: "message"; message: TaskMessage }
  | { type: "event"; event: TaskEvent }
  | { type: "approval"; approval: ToolApproval }
  | { type: "inspector"; tab: "changes" | "terminal" }
  | { type: "error"; scope: "persistence" | "run"; message: string };

export interface TaskExecutionInput {
  desktop: DesktopClient;
  task: DesktopTask;
  history: TaskMessage[];
  onUpdate: (update: TaskExecutionUpdate) => void;
  measurePerformance?: boolean;
}

export interface TaskExecutionResult {
  taskId: string;
  executionId?: string;
  prompt: string;
  response: string;
  state: "done" | "cancelled" | "error";
  error?: string;
}

export interface TaskExecutionDependencies {
  createRuntime(
    config: PiLocalModelConfig,
    dependencies: PiHarnessDependencies,
  ): Promise<AgentRuntime>;
  scheduleFrame(callback: () => void): number;
  cancelFrame(frame: number): void;
}

const defaultDependencies: TaskExecutionDependencies = {
  async createRuntime(config, dependencies) {
    const { PiHarness } = await import("./harness/pi");
    return new PiHarness(config, dependencies);
  },
  scheduleFrame: (callback) => window.requestAnimationFrame(callback),
  cancelFrame: (frame) => window.cancelAnimationFrame(frame),
};

export function createTaskExecution(
  input: TaskExecutionInput,
  dependencies: Partial<TaskExecutionDependencies> = {},
) {
  return new TaskExecution(input, { ...defaultDependencies, ...dependencies });
}

export class TaskExecution {
  private runtime: AgentRuntime | null = null;
  private execution: Execution | null = null;
  private cancelled = false;
  private running = false;
  private promptText = "";
  private response = "";
  private frame: number | null = null;
  private persistenceQueue: Promise<void> = Promise.resolve();
  private persistenceFailure: Error | null = null;
  private cancelStatus: Promise<unknown> = Promise.resolve();
  private transitionQueue: Promise<void> = Promise.resolve();
  private runStartedAt = 0;

  constructor(
    private readonly input: TaskExecutionInput,
    private readonly dependencies: TaskExecutionDependencies,
  ) {}

  async run(prompt: string): Promise<TaskExecutionResult> {
    if (this.running) throw new Error("Task execution is already running");
    this.running = true;
    this.promptText = prompt;
    this.response = "";
    this.runStartedAt = Date.now();
    this.mark("alpine:pi-launch:start");
    let unsubscribe: (() => void) | undefined;

    try {
      const launch = await this.input.desktop.resolvePiLaunch();
      this.execution = await this.input.desktop.createExecution({
        taskId: this.input.task.id,
        specification: launch.specification,
      });
      if (this.cancelled) return await this.settleCancelled();
      await this.moveTo("preparing");

      this.runtime = await this.dependencies.createRuntime(launch, {
        taskId: this.input.task.id,
        executionId: this.execution.id,
        desktop: this.input.desktop,
        history: this.input.history,
        onApproval: async (approval) => {
          if (!this.cancelled && this.execution?.state === "running")
            await this.moveTo("waiting-for-approval");
          this.input.onUpdate({ type: "approval", approval });
        },
      });
      if (this.cancelled) return await this.settleCancelled();
      await this.moveTo("running");

      this.measure(
        "alpine:pi-launch",
        "alpine:pi-launch:start",
        "alpine:pi-launch:ready",
      );
      let firstDelta = true;
      unsubscribe = this.runtime.subscribe((event) => {
        if (
          event.type === "message_update" &&
          event.assistantMessageEvent.type === "text_delta"
        ) {
          if (firstDelta) {
            firstDelta = false;
            this.measure(
              "alpine:stream:first-event",
              "alpine:stream:start",
              "alpine:stream:first-event:ready",
            );
          }
          this.response += event.assistantMessageEvent.delta;
          this.scheduleResponse();
          return;
        }
        this.persistenceQueue = this.persistenceQueue.then(() => this.persist(event));
      });

      this.mark("alpine:stream:start");
      await this.runtime.prompt(prompt);
      await this.persistenceQueue.catch((error: unknown) => {
        this.persistenceFailure = asError(error);
        this.input.onUpdate({
          type: "error",
          scope: "persistence",
          message: `Task history could not be saved: ${this.persistenceFailure.message}`,
        });
        throw this.persistenceFailure;
      });
      this.measure(
        "alpine:stream:duration",
        "alpine:stream:start",
        "alpine:stream:end",
      );
      if (this.runtime.errorMessage) throw new Error(this.runtime.errorMessage);
      if (this.cancelled) return await this.settleCancelled();

      await this.recordMetricEvent("completed");
      await this.moveTo("completed");
      this.flushResponse();
      return this.result("done");
    } catch (error) {
      const message = asError(error).message;
      if (this.cancelled) return await this.settleCancelled();
      if (this.execution && !isTerminal(this.execution.state)) {
        await this.moveTo("failed", message).catch(() => undefined);
      }
      this.input.onUpdate({ type: "error", scope: "run", message });
      this.flushResponse();
      return this.result("error", message);
    } finally {
      unsubscribe?.();
      this.clearFrame();
      this.runtime = null;
      this.running = false;
    }
  }

  steer(text: string) {
    this.runtime?.steer(text);
  }

  followUp(text: string) {
    this.runtime?.followUp(text);
  }

  cancel() {
    if (this.cancelled) return;
    this.cancelled = true;
    this.runtime?.abort();
    this.cancelStatus = this.execution
      ? this.moveTo("cancelling").catch((error: unknown) => {
          this.input.onUpdate({
            type: "error",
            scope: "persistence",
            message: `Cancellation state could not be saved: ${asError(error).message}`,
          });
        })
      : Promise.resolve();
  }

  private async settleCancelled(): Promise<TaskExecutionResult> {
    await this.cancelStatus;
    if (this.execution && !isTerminal(this.execution.state)) {
      await this.moveTo("cancelled").catch(() => undefined);
    }
    await this.recordMetricEvent("cancelled").catch(() => undefined);
    this.flushResponse();
    return this.result("cancelled");
  }

  private result(
    state: TaskExecutionResult["state"],
    error?: string,
  ): TaskExecutionResult {
    return {
      taskId: this.input.task.id,
      ...(this.execution ? { executionId: this.execution.id } : {}),
      prompt: this.promptText,
      response: this.response,
      state,
      ...(error ? { error } : {}),
    };
  }

  private moveTo(state: ExecutionState, failure: string | null = null) {
    const operation = this.transitionQueue.then(async () => {
      if (!this.execution) throw new Error("Execution identity is unavailable");
      if (this.execution.state === state) return this.execution;
      if (isTerminal(this.execution.state)) return this.execution;
      this.execution = await this.input.desktop.transitionExecution(
        this.execution.id,
        state,
        failure,
      );
      return this.execution;
    });
    this.transitionQueue = operation.then(
      () => undefined,
      () => undefined,
    );
    return operation;
  }

  private scheduleResponse() {
    if (this.cancelled || this.frame != null) return;
    this.frame = -1;
    const frame = this.dependencies.scheduleFrame(() => {
      this.frame = null;
      if (!this.cancelled) this.emitResponse();
    });
    if (this.frame != null) this.frame = frame;
  }

  private flushResponse() {
    this.clearFrame();
    this.emitResponse();
  }

  private emitResponse() {
    this.input.onUpdate({
      type: "response",
      taskId: this.input.task.id,
      prompt: this.promptText,
      response: this.response,
    });
  }

  private clearFrame() {
    if (this.frame != null && this.frame >= 0)
      this.dependencies.cancelFrame(this.frame);
    this.frame = null;
  }

  private async persist(event: AgentEvent) {
    const executionId = this.execution?.id;
    if (!executionId) throw new Error("Execution identity is unavailable");
    if (
      event.type === "tool_execution_start" &&
      this.execution?.state === "waiting-for-approval"
    ) {
      await this.moveTo("running");
    }
    if (
      event.type === "message_end" &&
      (event.message.role === "user" || event.message.role === "assistant")
    ) {
      const content = agentMessageText(event.message);
      if (content) {
        const message = await this.input.desktop.appendTaskMessage({
          taskId: this.input.task.id,
          executionId,
          role: event.message.role,
          content,
        });
        this.input.onUpdate({ type: "message", message });
      }
    }

    const normalized = normalizeAgentEvent(event);
    if (!normalized) return;
    const persisted = await this.input.desktop.appendTaskEvent({
      taskId: this.input.task.id,
      executionId,
      ...normalized,
    });
    this.input.onUpdate({ type: "event", event: persisted });
    if (persisted.kind === "tool.finished") {
      const payload = persisted.payload as { toolName?: string };
      if (payload.toolName === "edit_file")
        this.input.onUpdate({ type: "inspector", tab: "changes" });
      if (payload.toolName === "run_command")
        this.input.onUpdate({ type: "inspector", tab: "terminal" });
    }
  }

  private async recordMetricEvent(outcome: "completed" | "cancelled") {
    if (!this.execution) return;
    const event = await this.input.desktop.appendTaskEvent({
      taskId: this.input.task.id,
      executionId: this.execution.id,
      kind: "execution.metrics",
      payload: {
        outcome,
        durationMs: Math.max(0, Date.now() - this.runStartedAt),
        responseCharacters: this.response.length,
      },
    });
    this.input.onUpdate({ type: "event", event });
  }

  private mark(name: string) {
    if (this.input.measurePerformance === false) return;
    try {
      performance.mark(name);
    } catch {
      // Performance marks are best-effort local diagnostics.
    }
  }

  private measure(name: string, start: string, end: string) {
    if (this.input.measurePerformance === false) return;
    try {
      performance.mark(end);
      performance.measure(name, start, end);
    } catch {
      // Performance marks are best-effort local diagnostics.
    }
  }
}

function isTerminal(state: ExecutionState) {
  return ["completed", "cancelled", "failed", "interrupted"].includes(state);
}

export function normalizeAgentEvent(
  event: AgentEvent,
): { kind: string; payload: unknown } | null {
  switch (event.type) {
    case "agent_start":
      return { kind: "agent.started", payload: {} };
    case "agent_end":
      return {
        kind: "agent.finished",
        payload: { messageCount: event.messages.length },
      };
    case "turn_start":
      return { kind: "turn.started", payload: {} };
    case "turn_end":
      return {
        kind: "turn.finished",
        payload: { toolResultCount: event.toolResults.length },
      };
    case "message_start":
      return { kind: "message.started", payload: { role: event.message.role } };
    case "message_end":
      return event.message.role === "assistant"
        ? {
            kind: "message.finished",
            payload: {
              role: "assistant",
              stopReason: event.message.stopReason,
              usage: event.message.usage,
              error: event.message.errorMessage ?? null,
            },
          }
        : { kind: "message.finished", payload: { role: event.message.role } };
    case "tool_execution_start":
      return {
        kind: "tool.started",
        payload: {
          toolCallId: event.toolCallId,
          toolName: event.toolName,
          args: boundedPayload(event.args),
        },
      };
    case "tool_execution_update":
      return {
        kind: "tool.updated",
        payload: {
          toolCallId: event.toolCallId,
          toolName: event.toolName,
          details: boundedPayload(event.partialResult?.details ?? null),
        },
      };
    case "tool_execution_end":
      return {
        kind: "tool.finished",
        payload: {
          toolCallId: event.toolCallId,
          toolName: event.toolName,
          isError: event.isError,
          details: boundedPayload(event.result?.details ?? null),
        },
      };
    case "message_update":
      return null;
  }
}

function boundedPayload(value: unknown): unknown {
  const encoded = JSON.stringify(value);
  if (!encoded || encoded.length <= 256_000) return value;
  return { truncated: true, preview: encoded.slice(0, 256_000) };
}

function agentMessageText(message: AgentMessage): string {
  if (message.role === "user") {
    if (typeof message.content === "string") return message.content;
    return message.content
      .filter((part) => part.type === "text")
      .map((part) => part.text)
      .join("\n");
  }
  if (message.role === "assistant")
    return message.content
      .filter((part) => part.type === "text")
      .map((part) => part.text)
      .join("");
  return "";
}

function asError(error: unknown) {
  return error instanceof Error ? error : new Error(String(error));
}
