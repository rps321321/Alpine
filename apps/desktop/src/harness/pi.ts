import { Agent, type AgentEvent, type StreamFn } from "@earendil-works/pi-agent-core";
import { streamSimple } from "@earendil-works/pi-ai/api/openai-completions";
import type { Model } from "@earendil-works/pi-ai";

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
}

export function localPiModel(config: PiLocalModelConfig): Model<"openai-completions"> {
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
  readonly agent: Agent;

  constructor(config: PiLocalModelConfig, dependencies: PiHarnessDependencies = {}) {
    this.agent = new Agent({
      initialState: {
        model: localPiModel(config),
        systemPrompt:
          "You are Alpine, a local-first coding and model-analysis assistant. Distinguish estimates from measured evidence.",
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
}
