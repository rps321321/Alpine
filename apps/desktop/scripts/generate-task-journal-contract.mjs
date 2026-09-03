import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const rustPath = resolve(root, "src-tauri/src/store/journal.rs");
const outputPath = resolve(root, "src/generated/task-journal.ts");

const variants = [
  "UserPromptAccepted",
  "UserDirectionAccepted",
  "ExecutionQueued",
  "ExecutionPreparing",
  "ExecutionStarted",
  "AssistantMessageCompleted",
  "ToolProposed",
  "ExecutionWaitingForApproval",
  "ApprovalDecided",
  "ExecutionResumed",
  "ToolStarted",
  "ToolResultRecorded",
  "ToolSettled",
  "ApprovalInterrupted",
  "ExecutionCancelling",
  "ExecutionFinished",
  "LegacyImported",
];

function enumVariants(source, enumName) {
  const marker = `pub enum ${enumName} {`;
  const start = source.indexOf(marker);
  if (start < 0) throw new Error(`missing Rust enum ${enumName}`);
  let depth = 1;
  let cursor = start + marker.length;
  let body = "";
  while (cursor < source.length && depth > 0) {
    const char = source[cursor++];
    if (char === "{") depth += 1;
    if (char === "}") depth -= 1;
    if (depth > 0) body += char;
  }
  if (depth !== 0) throw new Error(`unterminated Rust enum ${enumName}`);
  const result = [];
  depth = 0;
  let token = "";
  for (const char of body) {
    if (char === "{") depth += 1;
    if (char === "}") depth -= 1;
    if (depth === 0 && char === ",") {
      const match = token.trim().match(/^([A-Z][A-Za-z0-9_]*)/);
      if (match) result.push(match[1]);
      token = "";
    } else token += char;
  }
  const match = token.trim().match(/^([A-Z][A-Za-z0-9_]*)/);
  if (match) result.push(match[1]);
  return result;
}

const rust = readFileSync(rustPath, "utf8");
const actual = enumVariants(rust, "TaskJournalEvent");
if (JSON.stringify(actual) !== JSON.stringify(variants)) {
  throw new Error(
    `TaskJournalEvent variants drifted. Rust=${JSON.stringify(actual)} generator=${JSON.stringify(variants)}`,
  );
}

const output = `// GENERATED FILE. DO NOT EDIT.\n// Source: src-tauri/src/store/journal.rs via scripts/generate-task-journal-contract.mjs\n\nexport const TASK_JOURNAL_VERSION = 1 as const;\n\nexport type UserDirection = "steer" | "follow-up";\nexport type ToolSettlementState = "completed" | "failed" | "interrupted";\nexport type ExecutionOutcome = "completed" | "cancelled" | "failed" | "interrupted";\nexport type LegacySource = "execution" | "message" | "event" | "approval";\n\nexport type ToolProposal =\n  | { type: "edit"; path: string; oldText: string; newText: string }\n  | { type: "shell"; command: string; timeoutSeconds: number };\n\nexport type ToolResult =\n  | { type: "edit"; path: string; replacements: number; diff: string }\n  | {\n      type: "shell";\n      command: string;\n      exitCode: number;\n      stdout: string;\n      stderr: string;\n      durationMs: number;\n      truncated: boolean;\n    }\n  | { type: "failure"; message: string };\n\nexport type TaskJournalEvent =\n  | { type: "user-prompt-accepted"; content: string }\n  | { type: "user-direction-accepted"; direction: UserDirection; content: string }\n  | { type: "execution-queued"; executionSpecId: string }\n  | { type: "execution-preparing" }\n  | { type: "execution-started" }\n  | { type: "assistant-message-completed"; content: string }\n  | {\n      type: "tool-proposed";\n      approvalId: string;\n      toolCallId: string;\n      proposal: ToolProposal;\n    }\n  | { type: "execution-waiting-for-approval"; approvalId: string }\n  | { type: "approval-decided"; approvalId: string; approved: boolean }\n  | { type: "execution-resumed"; approvalId: string }\n  | { type: "tool-started"; approvalId: string; proposal: ToolProposal }\n  | {\n      type: "tool-result-recorded";\n      approvalId: string;\n      succeeded: boolean;\n      result: ToolResult;\n    }\n  | {\n      type: "tool-settled";\n      approvalId: string;\n      state: ToolSettlementState;\n      detail: string | null;\n    }\n  | { type: "approval-interrupted"; approvalId: string; detail: string }\n  | { type: "execution-cancelling" }\n  | {\n      type: "execution-finished";\n      outcome: ExecutionOutcome;\n      failure: string | null;\n      durationMs: number | null;\n      responseCharacters: number | null;\n    }\n  | {\n      type: "legacy-imported";\n      source: LegacySource;\n      sourceId: string;\n      sourceSequence: number | null;\n      sourceOccurredAtMs: number;\n      causalOrder: "unverified";\n      data: unknown;\n    };\n`;

if (process.argv.includes("--check")) {
  const current = readFileSync(outputPath, "utf8").replace(/\r\n/g, "\n");
  if (current !== output) {
    console.error("Generated Task Journal TypeScript contract is stale.");
    process.exit(1);
  }
} else {
  writeFileSync(outputPath, output);
}
