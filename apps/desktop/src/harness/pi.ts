import {
  Agent,
  type AgentEvent,
  type AgentMessage,
  type AgentTool,
  type StreamFn,
} from "@earendil-works/pi-agent-core";
import { streamSimple } from "@earendil-works/pi-ai/api/openai-completions";
import type { Model } from "@earendil-works/pi-ai";
import { Type } from "typebox";
import type { ToolProposal } from "../generated/task-journal";
import type {
  MessageRole,
  TaskMessage,
  ToolApproval,
  WorkspaceEdit,
  WorkspaceEditResult,
  WorkspaceEntry,
  WorkspaceRead,
  WorkspaceSearchMatch,
  WorkspaceShell,
  WorkspaceShellResult,
} from "../desktop";

export interface PiLocalModelConfig {
  modelId: string;
  baseUrl: string;
  apiKey: string;
  contextWindow: number;
  maxTokens: number;
  temperature: number;
}

export interface PiToolClient {
  listProjectFiles(taskId: string, limit: number): Promise<WorkspaceEntry[]>;
  readProjectFile(
    taskId: string,
    path: string,
    offset?: number,
    limit?: number,
  ): Promise<WorkspaceRead>;
  searchProjectFiles(
    taskId: string,
    query: string,
    limit: number,
  ): Promise<WorkspaceSearchMatch[]>;
  proposeEffectApproval(input: {
    taskId: string;
    executionId: string;
    toolCallId: string;
    proposal: ToolProposal;
  }): Promise<ToolApproval>;
  waitForApproval(approvalId: string, signal?: AbortSignal): Promise<void>;
  executeApprovedEdit(
    taskId: string,
    executionId: string,
    approvalId: string,
    edit: WorkspaceEdit,
  ): Promise<WorkspaceEditResult>;
  executeApprovedShell(
    taskId: string,
    executionId: string,
    approvalId: string,
    shell: WorkspaceShell,
  ): Promise<WorkspaceShellResult>;
}

export interface PiHarnessDependencies {
  streamFn?: StreamFn;
  taskId?: string;
  executionId?: string;
  tools?: PiToolClient;
  history?: Array<Pick<TaskMessage, "role" | "content" | "createdAtMs">>;
}

export type PiHarnessEvent =
  | { type: "delta"; delta: string }
  | { type: "message"; role: "assistant"; content: string }
  | { type: "event"; kind: string; payload: unknown };

const listFilesSchema = Type.Object({
  limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 10_000 })),
});
const readFileSchema = Type.Object({
  path: Type.String({ description: "Project-relative path" }),
  offset: Type.Optional(Type.Integer({ minimum: 1 })),
  limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 2_000 })),
});
const searchFilesSchema = Type.Object({
  query: Type.String({
    description: "Exact text to find inside project files",
  }),
  limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 500 })),
});
const editFileSchema = Type.Object({
  path: Type.String({
    description: "Project-relative path to an existing UTF-8 file",
  }),
  oldText: Type.String({ description: "Exact unique text to replace" }),
  newText: Type.String({ description: "Replacement text" }),
});
const runCommandSchema = Type.Object({
  command: Type.String({
    description: "PowerShell command to run at the Selected Project root",
  }),
  timeoutSeconds: Type.Optional(Type.Integer({ minimum: 1, maximum: 3_600 })),
});

export function localPiModel(
  config: PiLocalModelConfig,
): Model<"openai-completions"> {
  const endpoint = config.baseUrl.replace(/\/$/, "");
  return {
    id: config.modelId,
    name: config.modelId,
    api: "openai-completions",
    provider: "alpine-local",
    baseUrl: endpoint.endsWith("/v1") ? endpoint : `${endpoint}/v1`,
    reasoning: false,
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: config.contextWindow,
    maxTokens: config.maxTokens,
    samplingParams: { temperature: config.temperature },
    compat: {
      supportsDeveloperRole: false,
      supportsReasoningEffort: false,
      maxTokensField: "max_tokens",
    },
  };
}

export class PiHarness {
  private readonly agent: Agent;
  readonly descriptor: Readonly<{
    modelId: string;
    toolNames: string[];
    steeringMode: "one-at-a-time";
    followUpMode: "one-at-a-time";
  }>;

  constructor(
    config: PiLocalModelConfig,
    dependencies: PiHarnessDependencies = {},
  ) {
    const tools =
      dependencies.taskId && dependencies.executionId && dependencies.tools
        ? createProjectTools(
            dependencies.taskId,
            dependencies.executionId,
            dependencies.tools,
          )
        : [];
    this.descriptor = Object.freeze({
      modelId: config.modelId,
      toolNames: tools.map((tool) => tool.name),
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
    });
    this.agent = new Agent({
      initialState: {
        model: localPiModel(config),
        systemPrompt:
          "You are Alpine, a local-first coding and model-analysis assistant. Work only inside the Selected Project through the provided tools. Read/list/search tools are project-scoped. Exact edits and shell commands pause for explicit operator approval. Distinguish estimates from measured evidence and summarize validation honestly.",
        messages: restoreMessages(dependencies.history ?? [], config),
        tools,
      },
      streamFn: dependencies.streamFn ?? (streamSimple as StreamFn),
      getApiKey: () => config.apiKey,
      toolExecution: "parallel",
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
    });
  }

  prompt(text: string) {
    return this.agent.prompt(text);
  }

  subscribe(listener: (event: PiHarnessEvent) => void | Promise<void>) {
    return this.agent.subscribe(async (event) => {
      for (const normalized of normalizePiEvent(event)) await listener(normalized);
    });
  }

  abort() {
    this.agent.abort();
  }

  steer(text: string) {
    this.agent.steer(userMessage(text));
  }

  followUp(text: string) {
    this.agent.followUp(userMessage(text));
  }

  get errorMessage() {
    return this.agent.state.errorMessage;
  }
}

function createProjectTools(
  taskId: string,
  executionId: string,
  tools: PiToolClient,
): AgentTool[] {
  const listFilesTool: AgentTool<typeof listFilesSchema> = {
    name: "list_files",
    label: "List project files",
    description:
      "List project-relative files and directories. Generated dependency/build directories are omitted.",
    parameters: listFilesSchema,
    async execute(_toolCallId, { limit }) {
      const entries = await tools.listProjectFiles(taskId, limit ?? 2_000);
      const text = entries.length
        ? entries
            .map(
              (entry) =>
                `${entry.kind === "directory" ? "dir " : "file"}\t${entry.path}`,
            )
            .join("\n")
        : "The Selected Project is empty.";
      return { content: [{ type: "text", text }], details: { entries } };
    },
  };
  const readFileTool: AgentTool<typeof readFileSchema> = {
    name: "read_file",
    label: "Read project file",
    description:
      "Read an existing UTF-8 file inside the Selected Project. Use offset and limit to continue large files.",
    parameters: readFileSchema,
    async execute(_toolCallId, { path, offset, limit }) {
      const result = await tools.readProjectFile(taskId, path, offset, limit);
      const continuation = result.truncated
        ? `\n\n[Showing lines ${result.startLine}-${result.endLine} of ${result.totalLines}. Continue with offset=${result.endLine + 1}.]`
        : "";
      return {
        content: [{ type: "text", text: `${result.content}${continuation}` }],
        details: result,
      };
    },
  };
  const searchFilesTool: AgentTool<typeof searchFilesSchema> = {
    name: "search_files",
    label: "Search project files",
    description:
      "Search for exact text inside UTF-8 files in the Selected Project.",
    parameters: searchFilesSchema,
    async execute(_toolCallId, { query, limit }) {
      const matches = await tools.searchProjectFiles(taskId, query, limit ?? 200);
      const text = matches.length
        ? matches
            .map((match) => `${match.path}:${match.line}: ${match.preview}`)
            .join("\n")
        : `No matches for ${JSON.stringify(query)}.`;
      return { content: [{ type: "text", text }], details: { matches } };
    },
  };
  const editFileTool: AgentTool<typeof editFileSchema> = {
    name: "edit_file",
    label: "Edit project file",
    description:
      "Replace one exact unique text block in an existing UTF-8 project file. The operator must approve the exact path and change before it runs.",
    parameters: editFileSchema,
    executionMode: "sequential",
    async execute(toolCallId, edit, signal) {
      const proposal: ToolProposal = {
        type: "edit",
        path: edit.path,
        oldText: edit.oldText,
        newText: edit.newText,
      };
      const approval = await tools.proposeEffectApproval({
        taskId,
        executionId,
        toolCallId,
        proposal,
      });
      await tools.waitForApproval(approval.id, signal);
      const result = await tools.executeApprovedEdit(
        taskId,
        executionId,
        approval.id,
        edit,
      );
      return {
        content: [
          { type: "text", text: `Edited ${result.path}.\n\n${result.diff}` },
        ],
        details: result,
      };
    },
  };
  const runCommandTool: AgentTool<typeof runCommandSchema> = {
    name: "run_command",
    label: "Run project command",
    description:
      "Run a PowerShell command at the Selected Project root and capture stdout, stderr, exit code, and duration. The operator must approve the exact command first.",
    parameters: runCommandSchema,
    executionMode: "sequential",
    async execute(toolCallId, input, signal, onUpdate) {
      const shell = {
        command: input.command,
        timeoutSeconds: input.timeoutSeconds ?? 120,
      };
      const proposal: ToolProposal = { type: "shell", ...shell };
      const approval = await tools.proposeEffectApproval({
        taskId,
        executionId,
        toolCallId,
        proposal,
      });
      onUpdate?.({
        content: [{ type: "text", text: "Waiting for operator approval…" }],
        details: { approvalId: approval.id, state: "pending" },
      });
      await tools.waitForApproval(approval.id, signal);
      onUpdate?.({
        content: [{ type: "text", text: "Approved. Running command…" }],
        details: { approvalId: approval.id, state: "executing" },
      });
      const result = await tools.executeApprovedShell(
        taskId,
        executionId,
        approval.id,
        shell,
      );
      const text = [
        `$ ${result.command}`,
        result.stdout,
        result.stderr ? `[stderr]\n${result.stderr}` : "",
        `[exit ${result.exitCode}; ${result.durationMs} ms${result.truncated ? "; output truncated" : ""}]`,
      ]
        .filter(Boolean)
        .join("\n");
      if (result.exitCode !== 0) throw new Error(text);
      return { content: [{ type: "text", text }], details: result };
    },
  };
  return [
    listFilesTool,
    readFileTool,
    searchFilesTool,
    editFileTool,
    runCommandTool,
  ];
}

export function normalizePiEvent(event: AgentEvent): PiHarnessEvent[] {
  switch (event.type) {
    case "agent_start":
      return [{ type: "event", kind: "agent.started", payload: {} }];
    case "agent_end":
      return [
        {
          type: "event",
          kind: "agent.finished",
          payload: { messageCount: event.messages.length },
        },
      ];
    case "turn_start":
      return [{ type: "event", kind: "turn.started", payload: {} }];
    case "turn_end":
      return [
        {
          type: "event",
          kind: "turn.finished",
          payload: { toolResultCount: event.toolResults.length },
        },
      ];
    case "message_start":
      return [
        {
          type: "event",
          kind: "message.started",
          payload: { role: event.message.role },
        },
      ];
    case "message_update":
      return event.assistantMessageEvent.type === "text_delta"
        ? [{ type: "delta", delta: event.assistantMessageEvent.delta }]
        : [];
    case "message_end": {
      const completed: PiHarnessEvent =
        event.message.role === "assistant"
          ? {
              type: "event",
              kind: "message.finished",
              payload: {
                role: "assistant",
                stopReason: event.message.stopReason,
                usage: event.message.usage,
                error: event.message.errorMessage ?? null,
              },
            }
          : {
              type: "event",
              kind: "message.finished",
              payload: { role: event.message.role },
            };
      if (event.message.role !== "assistant") return [completed];
      const content = agentMessageText(event.message);
      return content
        ? [{ type: "message", role: "assistant", content }, completed]
        : [completed];
    }
    case "tool_execution_start":
      return [
        {
          type: "event",
          kind: "tool.started",
          payload: {
            toolCallId: event.toolCallId,
            toolName: event.toolName,
            args: boundedPayload(event.args),
          },
        },
      ];
    case "tool_execution_update":
      return [
        {
          type: "event",
          kind: "tool.updated",
          payload: {
            toolCallId: event.toolCallId,
            toolName: event.toolName,
            details: boundedPayload(event.partialResult?.details ?? null),
          },
        },
      ];
    case "tool_execution_end":
      return [
        {
          type: "event",
          kind: "tool.finished",
          payload: {
            toolCallId: event.toolCallId,
            toolName: event.toolName,
            isError: event.isError,
            details: boundedPayload(event.result?.details ?? null),
          },
        },
      ];
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

function userMessage(content: string): AgentMessage {
  return { role: "user", content, timestamp: Date.now() };
}

function restoreMessages(
  history: Array<{ role: MessageRole; content: string; createdAtMs: number }>,
  config: PiLocalModelConfig,
): AgentMessage[] {
  return history.flatMap((message): AgentMessage[] => {
    if (message.role === "system") return [];
    if (message.role === "user") {
      return [
        {
          role: "user",
          content: message.content,
          timestamp: message.createdAtMs,
        },
      ];
    }
    return [
      {
        role: "assistant",
        content: [{ type: "text", text: message.content }],
        api: "openai-completions",
        provider: "alpine-local",
        model: config.modelId,
        usage: {
          input: 0,
          output: 0,
          cacheRead: 0,
          cacheWrite: 0,
          totalTokens: 0,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        },
        stopReason: "stop",
        timestamp: message.createdAtMs,
      },
    ];
  });
}
