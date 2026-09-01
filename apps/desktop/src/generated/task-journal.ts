// GENERATED FILE. DO NOT EDIT.
// Source: src-tauri/src/store/journal.rs via scripts/generate-task-journal-contract.mjs

export const TASK_JOURNAL_VERSION = 1 as const;

export type UserDirection = "steer" | "follow-up";
export type ToolSettlementState = "completed" | "failed" | "interrupted";
export type ExecutionOutcome = "completed" | "cancelled" | "failed" | "interrupted";
export type LegacySource = "execution" | "message" | "event" | "approval";

export type ToolProposal =
  | { type: "edit"; path: string; oldText: string; newText: string }
  | { type: "shell"; command: string; timeoutSeconds: number };

export type ToolResult =
  | { type: "edit"; path: string; replacements: number; diff: string }
  | {
      type: "shell";
      command: string;
      exitCode: number;
      stdout: string;
      stderr: string;
      durationMs: number;
      truncated: boolean;
    }
  | { type: "failure"; message: string };

export type TaskJournalEvent =
  | { type: "user-prompt-accepted"; content: string }
  | { type: "user-direction-accepted"; direction: UserDirection; content: string }
  | { type: "execution-queued"; executionSpecId: string }
  | { type: "execution-preparing" }
  | { type: "execution-started" }
  | { type: "assistant-message-completed"; content: string }
  | {
      type: "tool-proposed";
      approvalId: string;
      toolCallId: string;
      proposal: ToolProposal;
    }
  | { type: "execution-waiting-for-approval"; approvalId: string }
  | { type: "approval-decided"; approvalId: string; approved: boolean }
  | { type: "execution-resumed"; approvalId: string }
  | { type: "tool-started"; approvalId: string; proposal: ToolProposal }
  | {
      type: "tool-result-recorded";
      approvalId: string;
      succeeded: boolean;
      result: ToolResult;
    }
  | {
      type: "tool-settled";
      approvalId: string;
      state: ToolSettlementState;
      detail: string | null;
    }
  | { type: "approval-interrupted"; approvalId: string; detail: string }
  | { type: "execution-cancelling" }
  | {
      type: "execution-finished";
      outcome: ExecutionOutcome;
      failure: string | null;
      durationMs: number | null;
      responseCharacters: number | null;
    }
  | {
      type: "legacy-imported";
      source: LegacySource;
      sourceId: string;
      sourceSequence: number | null;
      sourceOccurredAtMs: number;
      causalOrder: "unverified";
      data: unknown;
    };
