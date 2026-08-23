import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const args = new Map();
for (let index = 2; index < process.argv.length; index += 2) {
  args.set(process.argv[index], process.argv[index + 1]);
}
const candidateRoot = args.get("--candidate-root");
const requestPath = args.get("--request");
if (!candidateRoot || !requestPath) process.exit(64);

const request = JSON.parse(await readFile(requestPath, "utf8"));
const fixture = JSON.parse(await readFile(request.fixture, "utf8"));
const policy = request.effective_policy;
const apiKey = process.env.ALPINE_BAKEOFF_API_KEY;
if (!apiKey) process.exit(65);
if (
  policy?.schema !== 1 ||
  policy.id !== request.prompt_tool_policy ||
  policy.system !== fixture.system ||
  policy.target_file !== fixture.target_file ||
  policy.temperature !== 0 ||
  policy.max_input_tokens !== request.budget.max_input_tokens ||
  policy.max_output_tokens !== request.budget.max_output_tokens ||
  policy.request_timeout_ms !== request.budget.request_timeout_ms ||
  policy.max_event_queue < request.budget.max_event_queue ||
  JSON.stringify(policy.allowed_tools) !== JSON.stringify(["read"]) ||
  JSON.stringify(policy.denied_tools) !== JSON.stringify(["write", "edit", "shell", "network"])
) process.exit(67);

const empty = () => ({
  schema: 1,
  candidate: request.candidate,
  scenario: request.scenario,
  requests_used: 0,
  max_input_tokens_observed: 0,
  max_output_tokens_observed: 0,
  retries_used: 0,
  worker_restarts_used: 0,
  events: [],
  errors: [],
});

const fail = (kind, code, retryable = false, requests = 0) => ({
  ...empty(),
  requests_used: requests,
  errors: [{ kind, code, retryable }],
});

const safeFailure = (code = "adapter-native-failure", requests = 1) =>
  fail("protocol-violation", code, false, requests);

const budgetFailure = (requests = 0) =>
  fail("budget-exceeded", "adapter-runtime-budget-exceeded", true, requests);

const failureFromReceipt = (receipt, kind, code, retryable = false) => ({
  ...receipt,
  events: [],
  errors: [{ kind, code, retryable }],
});

const safeFailureFromReceipt = (receipt, code = "adapter-native-failure") =>
  failureFromReceipt(receipt, "protocol-violation", code);

const budgetFailureFromReceipt = (receipt) =>
  failureFromReceipt(receipt, "budget-exceeded", "adapter-runtime-budget-exceeded", true);

function addEvents(receipt, ...events) {
  if (receipt.events.length + events.length > request.budget.max_event_queue) {
    return false;
  }
  receipt.events.push(...events);
  return true;
}

function needsProviderRequests(minimum) {
  return request.budget.requests >= minimum && request.budget.max_event_queue > 0;
}

const packageEntry = (name, entry = "dist/index.js") =>
  join(candidateRoot, "node_modules", ...name.split("/"), entry);

const importEntry = (name, entry) =>
  import(pathToFileURL(packageEntry(name, entry)).href);

const prompt = fixture.prompts[request.scenario];
if (typeof prompt !== "string") process.exit(66);

const project = join(request.state_root, "fixture");
await mkdir(project, { recursive: true });
await writeFile(join(project, fixture.target_file), `${fixture.target_text}\n`, "utf8");
const exactTargetText = `${fixture.target_text}\n`;

function usageFromPiMessage(message, receipt) {
  const usage = message?.usage;
  if (!usage) return;
  const input = Number(usage.input ?? 0) + Number(usage.cacheRead ?? 0) + Number(usage.cacheWrite ?? 0);
  receipt.max_input_tokens_observed = Math.max(receipt.max_input_tokens_observed, input);
  receipt.max_output_tokens_observed = Math.max(receipt.max_output_tokens_observed, Number(usage.output ?? 0));
}

function textFromPiMessage(message) {
  if (message?.role !== "assistant" || !Array.isArray(message.content)) return "";
  return message.content
    .filter((part) => part.type === "text")
    .map((part) => part.text ?? "")
    .join("");
}

function localPiModel() {
  return {
    id: request.model_id,
    name: "Alpine local bake-off",
    api: "openai-completions",
    provider: "alpine-local",
    baseUrl: `${request.base_url}/v1`,
    reasoning: false,
    input: ["text"],
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
    contextWindow: policy.max_input_tokens,
    maxTokens: policy.max_output_tokens,
    samplingParams: { temperature: policy.temperature },
    compat: {
      supportsDeveloperRole: false,
      supportsReasoningEffort: false,
      maxTokensField: "max_tokens",
    },
  };
}

async function runPiSdk() {
  const core = await importEntry("@earendil-works/pi-agent-core");
  const ai = await importEntry("@earendil-works/pi-ai");
  const api = await importEntry("@earendil-works/pi-ai", "dist/api/openai-completions.js");
  if (request.scenario === "worker-restart" || request.scenario === "continuation-recovery") {
    return fail("unsupported-capability", "no-versioned-restart-state");
  }
  if (request.scenario === "retry") {
    return fail("unsupported-capability", "no-controlled-retry-injection");
  }
  if (request.scenario === "compaction") {
    return fail("unsupported-capability", "no-versioned-compaction-state");
  }
  if (request.scenario === "normalized-errors") {
    if (request.budget.max_event_queue < 1) return budgetFailure();
    const agent = new core.Agent({
      initialState: { model: localPiModel(), systemPrompt: fixture.system },
      streamFn: api.streamSimple,
      getApiKey: () => apiKey,
    });
    try {
      await agent.continue();
      return safeFailure("empty-continuation-was-accepted", 0);
    } catch {
      const receipt = empty();
      receipt.events.push({
        type: "error-normalized",
        kind: "protocol-violation",
        code: "empty-continuation-rejected",
      });
      receipt.errors.push({
        kind: "protocol-violation",
        code: "empty-continuation-rejected",
        retryable: false,
      });
      return receipt;
    }
  }

  const minimumRequests = ["tools", "steering", "follow-up"].includes(request.scenario) ? 2 : 1;
  if (!needsProviderRequests(minimumRequests)) return budgetFailure();
  const receipt = empty();
  let exactReadCompleted = false;
  const readTool = {
    name: "read",
    label: "Read fixture",
    description: "Read only target.txt from the bounded fixture.",
    parameters: ai.Type.Object({ path: ai.Type.String() }),
    execute: async (_callId, input) => {
      if (input.path !== fixture.target_file) throw new Error("path denied");
      const text = await readFile(join(project, fixture.target_file), "utf8");
      if (text !== exactTargetText) throw new Error("fixture mismatch");
      exactReadCompleted = true;
      return {
        content: [{ type: "text", text }],
        details: {},
      };
    },
  };
  const agent = new core.Agent({
    initialState: {
      model: localPiModel(),
      systemPrompt: fixture.system,
      tools: request.scenario === "tools" ? [readTool] : [],
    },
    streamFn: api.streamSimple,
    getApiKey: () => apiKey,
    shouldStopAfterTurn: () => receipt.requests_used >= request.budget.requests,
  });
  let deltas = 0;
  let controlled = false;
  let nativeEvents = 0;
  let nativeBudgetExceeded = false;
  let requestBudgetExceeded = false;
  let cancellationAborted = false;
  let finalAssistantText = "";
  const toolCalls = new Map();
  agent.subscribe(async (event) => {
    nativeEvents += 1;
    if (nativeEvents > request.budget.max_event_queue) {
      nativeBudgetExceeded = true;
      agent.abort();
      return;
    }
    if (event.type === "turn_start") {
      receipt.requests_used += 1;
      if (receipt.requests_used > request.budget.requests) {
        requestBudgetExceeded = true;
        agent.abort();
      }
    }
    if (event.type === "message_update" && event.assistantMessageEvent?.type === "text_delta") {
      deltas += Buffer.byteLength(event.assistantMessageEvent.delta ?? "", "utf8");
      if (!controlled && request.scenario === "steering") {
        controlled = true;
        agent.steer({ role: "user", content: "Reply with exactly ALPINE-STEERED.", timestamp: Date.now() });
      } else if (!controlled && request.scenario === "follow-up") {
        controlled = true;
        agent.followUp({ role: "user", content: "Reply with exactly ALPINE-FOLLOWED-UP.", timestamp: Date.now() });
      } else if (!controlled && request.scenario === "cancellation") {
        controlled = true;
        agent.abort();
      }
      if (request.scenario === "backpressure") {
        controlled = true;
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    }
    if (event.type === "tool_execution_start" && request.scenario === "tools") {
      toolCalls.set(event.toolCallId, { tool: event.toolName, succeeded: false });
    }
    if (event.type === "tool_execution_end" && request.scenario === "tools") {
      const call = toolCalls.get(event.toolCallId);
      if (call && call.tool === event.toolName) call.succeeded = !event.isError && exactReadCompleted;
    }
    if (event.type === "message_end" && event.message?.role === "assistant") {
      usageFromPiMessage(event.message, receipt);
      finalAssistantText = textFromPiMessage(event.message).trim();
      cancellationAborted ||= event.message.stopReason === "aborted";
    }
  });
  try {
    await agent.prompt(prompt);
  } catch {
    if (request.scenario !== "cancellation") return safeFailureFromReceipt(receipt, "pi-sdk-run-failed");
  }
  if (nativeBudgetExceeded || requestBudgetExceeded) return budgetFailureFromReceipt(receipt);
  if (request.scenario === "streaming" && deltas > 0) {
    if (!addEvents(receipt, { type: "stream-started" }, { type: "stream-delta", bytes: deltas }, { type: "stream-finished" })) {
      return budgetFailureFromReceipt(receipt);
    }
  } else if (request.scenario === "tools") {
    const completed = [...toolCalls.entries()].filter(([, call]) => call.tool === "read" && call.succeeded);
    if (completed.length === 1 && toolCalls.size === 1) {
      const [[callId]] = completed;
      if (!addEvents(
        receipt,
        { type: "tool-started", call_id: callId, tool: "read" },
        { type: "tool-finished", call_id: callId, tool: "read", succeeded: true },
      )) return budgetFailureFromReceipt(receipt);
    }
  } else if (request.scenario === "steering" && controlled && finalAssistantText === "ALPINE-STEERED.") {
    if (!addEvents(receipt, { type: "steering-accepted" })) return budgetFailureFromReceipt(receipt);
  } else if (request.scenario === "follow-up" && controlled && finalAssistantText === "ALPINE-FOLLOWED-UP.") {
    if (!addEvents(receipt, { type: "follow-up-accepted" })) return budgetFailureFromReceipt(receipt);
  } else if (request.scenario === "cancellation" && controlled && cancellationAborted) {
    if (!addEvents(receipt, { type: "cancellation-requested" }, { type: "cancellation-confirmed" })) {
      return budgetFailureFromReceipt(receipt);
    }
  } else if (request.scenario === "backpressure" && controlled) {
    if (!addEvents(receipt, { type: "backpressure-applied", queue_capacity: 1 })) {
      return budgetFailureFromReceipt(receipt);
    }
  }
  if (!receipt.events.length && request.budget.max_event_queue === 0) return budgetFailureFromReceipt(receipt);
  return receipt.events.length ? receipt : safeFailureFromReceipt(receipt, "pi-sdk-scenario-not-observed");
}

async function runOpenCode() {
  return fail("unsupported-capability", "no-exact-system-prompt-override");
}

async function runPiProcess() {
  return fail("unsupported-capability", "rpc-read-cannot-enforce-exact-path");
}

async function runCline() {
  if (["steering", "follow-up", "retry", "compaction", "worker-restart", "continuation-recovery"].includes(request.scenario)) {
    const codes = {
      steering: "no-steering-queue",
      "follow-up": "no-follow-up-queue",
      retry: "no-explicit-retry-hook",
      compaction: "no-explicit-compaction-lifecycle",
      "worker-restart": "no-versioned-restart-state",
      "continuation-recovery": "no-versioned-continuation-import",
    };
    return fail("unsupported-capability", codes[request.scenario]);
  }
  const cline = await importEntry("@cline/agents");
  if (request.scenario === "normalized-errors") {
    if (request.budget.max_event_queue < 1) return budgetFailure();
    const agent = new cline.Agent({ providerId: "openai-compatible", modelId: request.model_id, apiKey, baseUrl: `${request.base_url}/v1` });
    try {
      await agent.continue();
      return safeFailure("empty-continuation-was-accepted", 0);
    } catch {
      const receipt = empty();
      receipt.events.push({ type: "error-normalized", kind: "protocol-violation", code: "empty-continuation-rejected" });
      receipt.errors.push({ kind: "protocol-violation", code: "empty-continuation-rejected", retryable: false });
      return receipt;
    }
  }
  const minimumRequests = request.scenario === "tools" ? 2 : 1;
  if (!needsProviderRequests(minimumRequests)) return budgetFailure();
  const receipt = empty();
  let exactReadCompleted = false;
  const readTool = {
    name: "read",
    description: "Read only target.txt from the bounded fixture.",
    inputSchema: { type: "object", properties: { path: { type: "string" } }, required: ["path"], additionalProperties: false },
    execute: async (input) => {
      if (input.path !== fixture.target_file) throw new Error("path denied");
      const text = await readFile(join(project, fixture.target_file), "utf8");
      if (text !== exactTargetText) throw new Error("fixture mismatch");
      exactReadCompleted = true;
      return text;
    },
  };
  let deltas = 0;
  let controlled = false;
  let nativeEvents = 0;
  let nativeBudgetExceeded = false;
  let requestBudgetExceeded = false;
  const toolCalls = new Map();
  let agent;
  const hooks = {
    onEvent: async (event) => {
      nativeEvents += 1;
      if (nativeEvents > request.budget.max_event_queue) {
        nativeBudgetExceeded = true;
        agent.abort("event-budget-exceeded");
        return;
      }
      if (event.type === "turn-started") {
        receipt.requests_used += 1;
        if (receipt.requests_used > request.budget.requests) {
          requestBudgetExceeded = true;
          agent.abort("request-budget-exceeded");
        }
      }
      if (event.type === "assistant-text-delta") {
        deltas += Buffer.byteLength(event.text ?? "", "utf8");
        if (!controlled && request.scenario === "cancellation") {
          controlled = true;
          agent.abort("bounded-cancellation");
        }
        if (request.scenario === "backpressure") {
          controlled = true;
          await new Promise((resolve) => setTimeout(resolve, 5));
        }
      } else if (event.type === "tool-started" && request.scenario === "tools") {
        toolCalls.set(event.toolCall.toolCallId, { tool: event.toolCall.toolName, finished: false });
      } else if (event.type === "tool-finished" && request.scenario === "tools") {
        const call = toolCalls.get(event.toolCall.toolCallId);
        if (call && call.tool === event.toolCall.toolName) call.finished = true;
      } else if (event.type === "usage-updated") {
        receipt.max_input_tokens_observed = Math.max(receipt.max_input_tokens_observed, Number(event.usage.inputTokens ?? 0));
        receipt.max_output_tokens_observed = Math.max(receipt.max_output_tokens_observed, Number(event.usage.outputTokens ?? 0));
      }
    },
  };
  agent = new cline.Agent({
    providerId: "openai-compatible",
    modelId: request.model_id,
    apiKey,
    baseUrl: `${request.base_url}/v1`,
    systemPrompt: fixture.system,
    tools: request.scenario === "tools" ? [readTool] : [],
    hooks,
    maxIterations: request.budget.requests,
    modelOptions: {
      maxTokens: policy.max_output_tokens,
      temperature: policy.temperature,
    },
  });
  let result;
  try {
    result = await agent.run(prompt);
  } catch {
    return safeFailureFromReceipt(receipt, "cline-run-failed");
  }
  if (nativeBudgetExceeded || requestBudgetExceeded) return budgetFailureFromReceipt(receipt);
  if (request.scenario === "streaming" && deltas > 0 && result.status === "completed") {
    if (!addEvents(receipt, { type: "stream-started" }, { type: "stream-delta", bytes: deltas }, { type: "stream-finished" })) {
      return budgetFailureFromReceipt(receipt);
    }
  } else if (request.scenario === "tools") {
    const completed = [...toolCalls.entries()].filter(([, call]) => call.tool === "read" && call.finished && exactReadCompleted);
    if (completed.length === 1 && toolCalls.size === 1) {
      const [[callId]] = completed;
      if (!addEvents(
        receipt,
        { type: "tool-started", call_id: callId, tool: "read" },
        { type: "tool-finished", call_id: callId, tool: "read", succeeded: true },
      )) return budgetFailureFromReceipt(receipt);
    }
  } else if (request.scenario === "cancellation" && controlled && result.status === "aborted") {
    if (!addEvents(receipt, { type: "cancellation-requested" }, { type: "cancellation-confirmed" })) {
      return budgetFailureFromReceipt(receipt);
    }
  } else if (request.scenario === "backpressure" && controlled) {
    if (!addEvents(receipt, { type: "backpressure-applied", queue_capacity: 1 })) {
      return budgetFailureFromReceipt(receipt);
    }
  }
  if (!receipt.events.length && request.budget.max_event_queue === 0) return budgetFailureFromReceipt(receipt);
  return receipt.events.length ? receipt : safeFailureFromReceipt(receipt, "cline-scenario-not-observed");
}

let receipt;
try {
  if (request.candidate === "opencode-process") receipt = await runOpenCode();
  else if (request.candidate === "pi-sdk-core") receipt = await runPiSdk();
  else if (request.candidate === "pi-process-rpc") receipt = await runPiProcess();
  else if (request.candidate === "cline-agents") receipt = await runCline();
  else receipt = fail("adapter-unavailable", "unknown-candidate");
} catch {
  receipt = fail("adapter-unavailable", "candidate-adapter-unavailable", false, 0);
}
process.stdout.write(JSON.stringify(receipt));
