import { PiHarness } from "../src/harness/pi.ts";

const required = [
  "ALPINE_LIVE_BASE_URL",
  "ALPINE_LIVE_API_KEY",
  "ALPINE_LIVE_MODEL_ID",
] as const;

for (const name of required) {
  if (!process.env[name]?.trim()) {
    throw new Error(`${name} is required for the live Pi smoke test`);
  }
}

const modelId = process.env.ALPINE_LIVE_MODEL_ID!.trim();
const harness = new PiHarness({
  modelId,
  baseUrl: process.env.ALPINE_LIVE_BASE_URL!.trim(),
  apiKey: process.env.ALPINE_LIVE_API_KEY!.trim(),
  contextWindow: 16_384,
  maxTokens: 32,
  temperature: 0,
});

let response = "";
const unsubscribe = harness.subscribe((event) => {
  if (
    event.type === "message_update" &&
    event.assistantMessageEvent.type === "text_delta"
  ) {
    response += event.assistantMessageEvent.delta;
  }
});

try {
  await harness.prompt("Reply with exactly ALPINE_OK and nothing else.");
} finally {
  unsubscribe();
}

if (harness.errorMessage) {
  throw new Error(`Pi reported a runtime error: ${harness.errorMessage}`);
}

if (response.trim() !== "ALPINE_OK") {
  throw new Error(
    `Pi returned unexpected visible output (${response.trim().length} characters)`,
  );
}

console.log(
  JSON.stringify({
    ok: true,
    adapter: "Pi SDK/core",
    model: modelId,
    visibleCharacters: response.trim().length,
  }),
);
