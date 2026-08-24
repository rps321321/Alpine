import { describe, expect, it } from "vitest";
import { PI_RUNTIME_CAPABILITIES } from "./capabilities";

describe("Pi runtime capability report", () => {
  it("reports the implemented task controls without claiming unsupported harness parity", () => {
    expect(PI_RUNTIME_CAPABILITIES.filter((capability) => capability.status === "available").map((capability) => capability.id)).toEqual([
      "prompt-stream",
      "cancel",
      "steer",
      "follow-up",
      "history-restore",
      "project-tools",
      "tool-approval",
    ]);
    expect(PI_RUNTIME_CAPABILITIES.find((capability) => capability.id === "images")?.status).toBe("unavailable");
    expect(PI_RUNTIME_CAPABILITIES.find((capability) => capability.id === "compaction")?.status).toBe("unavailable");
    expect(PI_RUNTIME_CAPABILITIES.find((capability) => capability.id === "graph-context")?.status).toBe("unavailable");
    expect(new Set(PI_RUNTIME_CAPABILITIES.map((capability) => capability.id)).size).toBe(PI_RUNTIME_CAPABILITIES.length);
  });
});
