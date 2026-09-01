import type {
  DesktopClient,
  DesktopTask,
  ExecutionUpdate,
  TaskEvent,
  TaskMessage,
  ToolApproval,
} from "./desktop";

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
  scheduleFrame(callback: () => void): number;
  cancelFrame(frame: number): void;
}

const defaultDependencies: TaskExecutionDependencies = {
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
  private running = false;
  private cancelled = false;
  private cancelRequested = false;
  private executionId: string | null = null;
  private promptText = "";
  private response = "";
  private frame: number | null = null;
  private firstDelta = true;
  private terminalResolve: ((result: TaskExecutionResult) => void) | null = null;
  private terminalSettled = false;
  private readonly seenMessages = new Set<string>();
  private readonly seenEvents = new Set<string>();

  constructor(
    private readonly input: TaskExecutionInput,
    private readonly dependencies: TaskExecutionDependencies,
  ) {}

  async run(prompt: string): Promise<TaskExecutionResult> {
    if (this.running) throw new Error("Task execution is already running");
    this.running = true;
    this.cancelled = false;
    this.cancelRequested = false;
    this.executionId = null;
    this.promptText = prompt;
    this.response = "";
    this.firstDelta = true;
    this.terminalSettled = false;
    this.seenMessages.clear();
    this.seenEvents.clear();
    this.mark("alpine:host-execution:start");

    const buffered: ExecutionUpdate[] = [];
    const terminal = new Promise<TaskExecutionResult>((resolve) => {
      this.terminalResolve = resolve;
    });
    let unsubscribe: (() => void) | undefined;

    try {
      unsubscribe = await this.input.desktop.subscribeExecutionUpdates((update) => {
        if (update.taskId !== this.input.task.id) return;
        if (!this.executionId) {
          buffered.push(update);
          return;
        }
        if (update.executionId === this.executionId) this.apply(update);
      });

      const accepted = await this.input.desktop.submitPrompt(
        this.input.task.id,
        prompt,
      );
      this.executionId = accepted.execution.id;
      this.acceptMessage(accepted.promptMessage);
      for (const update of buffered) {
        if (update.executionId === this.executionId) this.apply(update);
      }

      this.measure(
        "alpine:host-execution",
        "alpine:host-execution:start",
        "alpine:host-execution:accepted",
      );
      this.mark("alpine:stream:start");

      if (this.cancelRequested) {
        await this.input.desktop.cancelExecution(this.executionId);
      }
      return await terminal;
    } catch (error) {
      const message = asError(error).message;
      if (!this.terminalSettled) {
        this.input.onUpdate({ type: "error", scope: "run", message });
        this.flushResponse();
        return this.result("error", message);
      }
      return await terminal;
    } finally {
      unsubscribe?.();
      this.clearFrame();
      this.terminalResolve = null;
      this.running = false;
    }
  }

  steer(text: string) {
    const executionId = this.executionId;
    if (!executionId) return;
    void this.input.desktop.steerExecution(executionId, text).catch((error) => {
      this.input.onUpdate({
        type: "error",
        scope: "run",
        message: asError(error).message,
      });
    });
  }

  followUp(text: string) {
    const executionId = this.executionId;
    if (!executionId) return;
    void this.input.desktop.queueFollowUp(executionId, text).catch((error) => {
      this.input.onUpdate({
        type: "error",
        scope: "run",
        message: asError(error).message,
      });
    });
  }

  cancel() {
    if (this.cancelRequested || this.terminalSettled) return;
    this.cancelRequested = true;
    this.cancelled = true;
    const executionId = this.executionId;
    if (!executionId) return;
    void this.input.desktop.cancelExecution(executionId).catch((error) => {
      this.input.onUpdate({
        type: "error",
        scope: "persistence",
        message: `Cancellation could not be requested: ${asError(error).message}`,
      });
    });
  }

  private apply(update: ExecutionUpdate) {
    switch (update.type) {
      case "state":
        return;
      case "delta":
        if (this.firstDelta) {
          this.firstDelta = false;
          this.measure(
            "alpine:stream:first-event",
            "alpine:stream:start",
            "alpine:stream:first-event:ready",
          );
        }
        this.response += update.delta;
        this.scheduleResponse();
        return;
      case "message":
        this.acceptMessage(update.message);
        if (update.message.role === "assistant") {
          this.response = update.message.content;
          this.flushResponse();
        }
        return;
      case "event":
        if (this.seenEvents.has(update.event.id)) return;
        this.seenEvents.add(update.event.id);
        this.input.onUpdate({ type: "event", event: update.event });
        return;
      case "approval":
        this.input.onUpdate({ type: "approval", approval: update.approval });
        return;
      case "inspector":
        this.input.onUpdate({ type: "inspector", tab: update.tab });
        return;
      case "error":
        this.input.onUpdate({
          type: "error",
          scope: update.scope,
          message: update.message,
        });
        return;
      case "terminal": {
        if (this.terminalSettled) return;
        this.terminalSettled = true;
        this.measure(
          "alpine:stream:duration",
          "alpine:stream:start",
          "alpine:stream:end",
        );
        this.flushResponse();
        if (update.outcome === "completed") {
          this.resolveTerminal(this.result("done"));
          return;
        }
        if (update.outcome === "cancelled") {
          this.cancelled = true;
          this.resolveTerminal(this.result("cancelled"));
          return;
        }
        const message = update.error || "The host-owned Execution failed";
        this.input.onUpdate({ type: "error", scope: "run", message });
        this.resolveTerminal(this.result("error", message));
      }
    }
  }

  private acceptMessage(message: TaskMessage) {
    if (this.seenMessages.has(message.id)) return;
    this.seenMessages.add(message.id);
    this.input.onUpdate({ type: "message", message });
  }

  private resolveTerminal(result: TaskExecutionResult) {
    const resolve = this.terminalResolve;
    this.terminalResolve = null;
    resolve?.(result);
  }

  private result(
    state: TaskExecutionResult["state"],
    error?: string,
  ): TaskExecutionResult {
    return {
      taskId: this.input.task.id,
      ...(this.executionId ? { executionId: this.executionId } : {}),
      prompt: this.promptText,
      response: this.response,
      state,
      ...(error ? { error } : {}),
    };
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

function asError(error: unknown) {
  return error instanceof Error ? error : new Error(String(error));
}
