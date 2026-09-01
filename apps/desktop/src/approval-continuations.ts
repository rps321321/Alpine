type ApprovalWaiter = {
  resolve: () => void;
  reject: (error: Error) => void;
  removeAbortListener: () => void;
};

export class ApprovalContinuations {
  private readonly waiters = new Map<string, ApprovalWaiter>();
  private readonly earlyDecisions = new Map<string, boolean>();

  wait(approvalId: string, signal?: AbortSignal): Promise<void> {
    if (this.waiters.has(approvalId)) {
      return Promise.reject(
        new Error(`Tool Approval ${approvalId} already has a waiting continuation`),
      );
    }
    if (this.earlyDecisions.has(approvalId)) {
      const approved = this.earlyDecisions.get(approvalId) === true;
      this.earlyDecisions.delete(approvalId);
      return approved
        ? Promise.resolve()
        : Promise.reject(new Error("Operator denied the proposed operation"));
    }
    if (signal?.aborted) {
      return Promise.reject(new Error("Tool Approval wait was cancelled"));
    }
    return new Promise<void>((resolve, reject) => {
      const onAbort = () => {
        this.waiters.delete(approvalId);
        reject(new Error("Tool Approval wait was cancelled"));
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      this.waiters.set(approvalId, {
        resolve,
        reject,
        removeAbortListener: () =>
          signal?.removeEventListener("abort", onAbort),
      });
    });
  }

  settle(approvalId: string, approved: boolean) {
    const waiter = this.waiters.get(approvalId);
    if (!waiter) {
      this.earlyDecisions.set(approvalId, approved);
      return;
    }
    this.waiters.delete(approvalId);
    waiter.removeAbortListener();
    if (approved) waiter.resolve();
    else waiter.reject(new Error("Operator denied the proposed operation"));
  }

  rejectAll(error: Error) {
    for (const [approvalId, waiter] of this.waiters) {
      this.waiters.delete(approvalId);
      waiter.removeAbortListener();
      waiter.reject(error);
    }
    this.earlyDecisions.clear();
  }
}
