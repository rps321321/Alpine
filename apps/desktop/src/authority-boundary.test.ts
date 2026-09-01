import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(path, "utf8");

describe("desktop execution authority", () => {
  it("keeps credentials and durable mutation primitives out of the visible renderer", () => {
    const desktop = read("src/desktop.ts");
    const taskExecution = read("src/task-execution.ts");
    for (const forbidden of [
      "apiKey",
      "resolvePiLaunch",
      "createExecution",
      "transitionExecution",
      "appendTaskMessage",
      "appendTaskEvent",
      "setTaskStatus",
      "requestToolApproval",
      "getToolApproval",
      "editProjectFile",
      "runProjectShell",
    ]) {
      expect(desktop).not.toContain(forbidden);
    }
    expect(taskExecution).not.toContain("@earendil-works/pi-");
    expect(taskExecution).toContain("submitPrompt");
    expect(taskExecution).toContain("subscribeExecutionUpdates");
  });

  it("binds provider execution to the isolated worker and host supervisor", () => {
    const host = read("src-tauri/src/lib.rs");
    const supervisor = read("src-tauri/src/supervisor.rs");
    const worker = read("src/agent-worker.ts");
    expect(host).toContain('"agent-worker"');
    expect(host).toContain("supervisor::submit_prompt");
    expect(host).not.toContain("resolve_pi_launch,");
    expect(supervisor).toContain("require_webview");
    expect(supervisor).toContain("ExecutionState::Cancelling");
    expect(worker).toContain("connect_agent_worker");
    expect(worker).toContain("PiHarness");
  });
});
