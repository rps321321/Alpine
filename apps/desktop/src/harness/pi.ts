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
import type {
  DesktopClient,
  MessageRole,
  TaskMessage,
  ToolApproval,
} from "../desktop";

export interface PiLocalModelConfig {
  modelId: string;
  baseUrl: string;
  apiKey: string;
  contextWindow: number;
  maxTokens: number;
  temperature: number;
}

export interface PiHarnessDependencies {
  streamFn?: StreamFn;
  taskId?: string;
  executionId?: string;
  desktop?: DesktopClient;
  history?: Array<Pick<TaskMessage, "role" | "content" | "createdAtMs">>;
  onApproval?: (approval: ToolApproval) => void | Promise<void>;
  onApprovalSettled?: (approval: ToolApproval) => void | Promise<void>;
}

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
      dependencies.taskId && dependencies.executionId && dependencies.desktop
        ? createProjectTools(
            dependencies.taskId,
            dependencies.executionId,
            dependencies.desktop,
            dependencies.onApproval,
            dependencies.onApprovalSettled,
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

  subscribe(listener: (event: AgentEvent) => void | Promise<void>) {
    return this.agent.subscribe((event) => listener(event));
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
  desktop: DesktopClient,
  onApproval?: (approval: ToolApproval) => void | Promise<void>,
  onApprovalSettled?: (approval: ToolApproval) => void | Promise<void>,
): AgentTool[] {
  const listFilesTool: AgentTool<typeof listFilesSchema> = {
    name: "list_files",
    label: "List project files",
    description:
      "List project-relative files and directories. Generated dependency/build directories are omitted.",
    parameters: listFilesSchema,
    async execute(_toolCallId, { limit }) {
      const entries = await desktop.listProjectFiles(taskId, limit ?? 2_000);
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
      const result = await desktop.readProjectFile(taskId, path, offset, limit);
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
      const matches = await desktop.searchProjectFiles(
        taskId,
        query,
        limit ?? 200,
      );
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
      const proposal = {
        path: edit.path,
        oldText: edit.oldText,
        newText: edit.newText,
      };
      const approval = await desktop.requestToolApproval({
        taskId,
        executionId,
        toolCallId,
        operation: "edit",
        proposal,
      });
      await onApproval?.(approval);
      await waitForApproval(
        desktop,
        approval.id,
        signal,
        onApprovalSettled,
      );
      const result = await desktop.editProjectFile(
        taskId,
        approval.id,
        proposal,
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
      const approval = await desktop.requestToolApproval({
        taskId,
        executionId,
        toolCallId,
        operation: "shell",
        proposal: shell,
      });
      await onApproval?.(approval);
      onUpdate?.({
        content: [{ type: "text", text: "Waiting for operator approval…" }],
        details: { approvalId: approval.id, state: "pending" },
      });
      await waitForApproval(
        desktop,
        approval.id,
        signal,
        onApprovalSettled,
      );
      onUpdate?.({
        content: [{ type: "text", text: "Approved. Running command…" }],
        details: { approvalId: approval.id, state: "executing" },
      });
      const result = await desktop.runProjectShell(taskId, approval.id, shell);
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

async function waitForApproval(
  desktop: DesktopClient,
  approvalId: string,
  signal?: AbortSignal,
  onSettled?: (approval: ToolApproval) => void | Promise<void>,
) {
  for (;;) {
    if (signal?.aborted) throw new Error("Tool Approval wait was cancelled");
    const approval = await desktop.getToolApproval(approvalId);
    if (!approval) throw new Error("Tool Approval no longer exists");
    if (approval.state === "approved") {
      await onSettled?.(approval);
      return;
    }
    if (approval.state === "denied") {
      await onSettled?.(approval);
      throw new Error("Operator denied the proposed operation");
    }
    if (["interrupted", "failed", "completed"].includes(approval.state)) {
      await onSettled?.(approval);
      throw new Error(`Tool Approval settled as ${approval.state}`);
    }
    await delay(200, signal);
  }
}

function delay(milliseconds: number, signal?: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const timer = window.setTimeout(resolve, milliseconds);
    signal?.addEventListener(
      "abort",
      () => {
        window.clearTimeout(timer);
        reject(new Error("Operation aborted"));
      },
      { once: true },
    );
  });
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
