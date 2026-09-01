import { describe, expect, it } from "vitest";
import { ApprovalContinuations } from "./approval-continuations";

describe("ApprovalContinuations", () => {
  it("delivers an approval that arrives before the worker starts waiting", async () => {
    const continuations = new ApprovalContinuations();
    continuations.settle("approval-1", true);
    await expect(continuations.wait("approval-1")).resolves.toBeUndefined();
  });

  it("delivers an approval to an existing waiter", async () => {
    const continuations = new ApprovalContinuations();
    const waiting = continuations.wait("approval-1");
    continuations.settle("approval-1", true);
    await expect(waiting).resolves.toBeUndefined();
  });

  it("preserves denial regardless of delivery order", async () => {
    const early = new ApprovalContinuations();
    early.settle("approval-early", false);
    await expect(early.wait("approval-early")).rejects.toThrow(
      "Operator denied",
    );

    const waiting = new ApprovalContinuations();
    const denied = waiting.wait("approval-waiting");
    waiting.settle("approval-waiting", false);
    await expect(denied).rejects.toThrow("Operator denied");
  });

  it("rejects all live continuations when an Execution ends", async () => {
    const continuations = new ApprovalContinuations();
    const first = continuations.wait("approval-1");
    const second = continuations.wait("approval-2");
    continuations.rejectAll(new Error("Execution ended"));
    await expect(first).rejects.toThrow("Execution ended");
    await expect(second).rejects.toThrow("Execution ended");
  });
});
