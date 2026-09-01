import type { StreamFn } from "@earendil-works/pi-agent-core";
import { describe, expect, it } from "vitest";
import { PiHarness, localPiModel } from "./pi";
import type { DesktopClient } from "../desktop";

const config = {
  modelId: "Qwen3.5-9B-Q4_K_M.gguf",
  baseUrl: "http://127.0.0.1:8080/",
  apiKey: "local-test-token",
  contextWindow: 16_384,
  maxTokens: 2_048,
  temperature: 0.2,
};

describe("Pi harness adapter", () => {
  it("maps the selected local model to Pi's OpenAI-compatible provider contract", () => {
    expect(localPiModel(config)).toMatchObject({
      id: "Qwen3.5-9B-Q4_K_M.gguf",
      provider: "alpine-local",
      api: "openai-completions",
      baseUrl: "http://127.0.0.1:8080/v1",
      contextWindow: 16_384,
      maxTokens: 2_048,
      input: ["text"],
    });
  });

  it("does not duplicate the v1 path from an Alpine base-url file", () => {
    expect(
      localPiModel({ ...config, baseUrl: "http://127.0.0.1:8080/v1" }).baseUrl,
    ).toBe("http://127.0.0.1:8080/v1");
  });

  it("constructs the embedded Pi agent with the launch-time model", () => {
    const neverCalled = (() => {
      throw new Error("test stream should not run");
    }) as StreamFn;
    const harness = new PiHarness(config, { streamFn: neverCalled });

    expect(harness.descriptor).toMatchObject({
      modelId: "Qwen3.5-9B-Q4_K_M.gguf",
      steeringMode: "one-at-a-time",
      followUpMode: "one-at-a-time",
    });
    expect(harness.errorMessage).toBeUndefined();
  });

  it("binds Alpine-owned coding tools and queues steering through Pi", () => {
    const neverCalled = (() => {
      throw new Error("test stream should not run");
    }) as StreamFn;
    const harness = new PiHarness(config, {
      streamFn: neverCalled,
      taskId: "task-1",
      executionId: "execution-1",
      desktop: {} as DesktopClient,
    });

    expect(harness.descriptor.toolNames).toEqual([
      "list_files",
      "read_file",
      "search_files",
      "edit_file",
      "run_command",
    ]);
    harness.steer("Focus on the failing test.");
    harness.followUp("Then summarize the diff.");
  });
});
