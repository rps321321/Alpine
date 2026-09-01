from pathlib import Path
import json
import re

ROOT = Path(__file__).resolve().parents[1]


def read(path):
    return (ROOT / path).read_text(encoding="utf-8")


def write(path, value):
    (ROOT / path).write_text(value, encoding="utf-8")


def replace_once(value, old, new, label):
    count = value.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one literal match, found {count}")
    return value.replace(old, new, 1)


def sub_once(value, pattern, replacement, label):
    updated, count = re.subn(pattern, replacement, value, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{label}: expected one regex match, found {count}")
    return updated


def patch_desktop():
    path = "apps/desktop/src/desktop.ts"
    value = read(path)
    value = replace_once(
        value,
        'import { listen } from "@tauri-apps/api/event";\n',
        'import { listen } from "@tauri-apps/api/event";\nimport type { TaskJournalEvent } from "./generated/task-journal";\n',
        "Task Journal generated import",
    )
    value = replace_once(
        value,
        '''export interface TaskEvent {
  id: string;
  taskId: string;
  executionId: string;
  sequence: number;
  kind: string;
  payload: unknown;
  createdAtMs: number;
}''',
        '''export interface TaskEvent {
  id: string;
  taskId: string;
  executionId: string | null;
  sequence: number;
  version: number;
  event: TaskJournalEvent;
  createdAtMs: number;
}''',
        "typed Task Journal record",
    )
    value = replace_once(
        value,
        '''export interface ToolApprovalDecision {
  approval: ToolApproval;
  event: TaskEvent;
}''',
        '''export interface ToolApprovalDecision {
  approval: ToolApproval;
  execution: Execution;
  records: TaskEvent[];
}''',
        "Tool Approval journal decision",
    )
    value = replace_once(
        value,
        '''function previewExecutionUpdate(update: ExecutionUpdate) {
  for (const listener of previewExecutionListeners) listener(update);
}

function previewExecution(executionId: string) {''',
        '''function previewExecutionUpdate(update: ExecutionUpdate) {
  for (const listener of previewExecutionListeners) listener(update);
}

function nextPreviewTaskSequence(detail: TaskDetail) {
  return Math.max(
    0,
    ...detail.messages.map((message) => message.sequence),
    ...detail.events.map((event) => event.sequence),
  ) + 1;
}

function previewJournalRecord(
  detail: TaskDetail,
  executionId: string,
  event: TaskJournalEvent,
  createdAtMs = Date.now(),
) {
  const record: TaskEvent = {
    id: previewId("journal"),
    taskId: detail.task.id,
    executionId,
    sequence: nextPreviewTaskSequence(detail),
    version: 1,
    event,
    createdAtMs,
  };
  detail.events.push(record);
  previewExecutionUpdate({
    type: "event",
    taskId: detail.task.id,
    executionId,
    event: record,
  });
  return record;
}

function previewDirection(
  executionId: string,
  text: string,
  direction: "steer" | "follow-up",
) {
  const found = previewExecution(executionId);
  if (!found) throw new Error("Preview Execution does not exist");
  const record = previewJournalRecord(found.detail, executionId, {
    type: "user-direction-accepted",
    direction,
    content: text,
  });
  const message: TaskMessage = {
    id: record.id,
    taskId: found.execution.taskId,
    executionId,
    sequence: record.sequence,
    role: "user",
    content: text,
    createdAtMs: record.createdAtMs,
  };
  found.detail.messages.push(message);
  previewExecutionUpdate({
    type: "message",
    taskId: message.taskId,
    executionId,
    message,
  });
  return message;
}

function previewExecution(executionId: string) {''',
        "preview journal helpers",
    )
    value = sub_once(
        value,
        r"  async submitPrompt\(taskId, prompt\) \{.*?\n  async cancelExecution\(executionId\) \{",
        '''  async submitPrompt(taskId, prompt) {
    const detail = previewDetails.get(taskId);
    if (!detail) throw new Error("Preview task does not exist");
    if (
      (detail.executions ?? []).some((execution) =>
        ["queued", "preparing", "running", "waiting-for-approval", "cancelling"].includes(
          execution.state,
        ),
      )
    )
      throw new Error("Preview task already has an active Execution");
    const now = Date.now();
    const executionId = previewId("execution");
    const specificationId = previewId("execution-spec");
    const execution: Execution = {
      id: executionId,
      taskId,
      executionSpecId: specificationId,
      specification: {
        id: specificationId,
        taskId,
        modelRegistryId: "preview-model-1",
        modelRepoId: detail.task.modelRepoId,
        modelRevision: "0123456789abcdef0123456789abcdef01234567",
        modelFilename: detail.task.modelFilename,
        modelSha256: "a".repeat(64),
        sessionConfigSha256: "b".repeat(64),
        profileName: detail.task.profile,
        profileSha256: "c".repeat(64),
        runtimeName: "official",
        runtimeIdentity: "d".repeat(64),
        adapterIdentity: "pi-agent-core@0.84.2",
        policyIdentity: "alpine-desktop-project-tools-v1",
        contextWindow: 16_384,
        maxTokens: 2_048,
        temperatureMillis: 200,
        legacyUnverified: false,
        createdAtMs: now,
      },
      state: "preparing",
      failure: null,
      queuedAtMs: now,
      startedAtMs: now,
      finishedAtMs: null,
      updatedAtMs: now,
    };
    detail.executions = [...(detail.executions ?? []), execution];
    detail.task.status = "running";
    detail.task.summary = "active";
    detail.task.activeExecutionId = execution.id;
    detail.task.latestExecutionId = execution.id;

    const promptRecord = previewJournalRecord(
      detail,
      executionId,
      { type: "user-prompt-accepted", content: prompt },
      now,
    );
    const promptMessage: TaskMessage = {
      id: promptRecord.id,
      taskId,
      executionId,
      sequence: promptRecord.sequence,
      role: "user",
      content: prompt,
      createdAtMs: now,
    };
    detail.messages.push(promptMessage);
    previewJournalRecord(
      detail,
      executionId,
      { type: "execution-queued", executionSpecId: specificationId },
      now,
    );
    previewJournalRecord(detail, executionId, { type: "execution-preparing" }, now);
    previewExecutionUpdate({
      type: "message",
      taskId,
      executionId,
      message: promptMessage,
    });
    previewExecutionUpdate({ type: "state", taskId, executionId, execution });

    window.setTimeout(() => {
      if (execution.state !== "preparing") return;
      execution.state = "running";
      execution.updatedAtMs = Date.now();
      previewJournalRecord(detail, executionId, { type: "execution-started" });
      previewExecutionUpdate({ type: "state", taskId, executionId, execution });
      const content = "Preview mode accepted the host-owned Execution.";
      previewExecutionUpdate({ type: "delta", taskId, executionId, delta: content });
      const assistantRecord = previewJournalRecord(detail, executionId, {
        type: "assistant-message-completed",
        content,
      });
      const assistant: TaskMessage = {
        id: assistantRecord.id,
        taskId,
        executionId,
        sequence: assistantRecord.sequence,
        role: "assistant",
        content,
        createdAtMs: assistantRecord.createdAtMs,
      };
      detail.messages.push(assistant);
      previewExecutionUpdate({ type: "message", taskId, executionId, message: assistant });
      const finishedAt = Date.now();
      execution.state = "completed";
      execution.finishedAtMs = finishedAt;
      execution.updatedAtMs = finishedAt;
      detail.task.status = "completed";
      detail.task.summary = "done";
      detail.task.activeExecutionId = null;
      previewJournalRecord(
        detail,
        executionId,
        {
          type: "execution-finished",
          outcome: "completed",
          failure: null,
          durationMs: Math.max(0, finishedAt - now),
          responseCharacters: content.length,
        },
        finishedAt,
      );
      previewExecutionUpdate({ type: "state", taskId, executionId, execution });
      previewExecutionUpdate({
        type: "terminal",
        taskId,
        executionId,
        execution,
        outcome: "completed",
        error: null,
      });
    }, 10);
    return { execution, promptMessage };
  },
  async cancelExecution(executionId) {''',
        "preview prompt journal",
    )
    value = sub_once(
        value,
        r"  async cancelExecution\(executionId\) \{.*?\n  async steerExecution\(executionId, text\) \{.*?\n  async queueFollowUp\(executionId, text\) \{\n    return this\.steerExecution\(executionId, text\);\n  \},",
        '''  async cancelExecution(executionId) {
    const found = previewExecution(executionId);
    if (!found) throw new Error("Preview Execution does not exist");
    const { detail, execution } = found;
    execution.state = "cancelling";
    execution.updatedAtMs = Date.now();
    previewJournalRecord(detail, executionId, { type: "execution-cancelling" });
    previewExecutionUpdate({
      type: "state",
      taskId: execution.taskId,
      executionId,
      execution,
    });
    window.setTimeout(() => {
      const finishedAt = Date.now();
      execution.state = "cancelled";
      execution.finishedAtMs = finishedAt;
      execution.updatedAtMs = finishedAt;
      detail.task.status = "cancelled";
      detail.task.summary = "ready";
      detail.task.activeExecutionId = null;
      previewJournalRecord(
        detail,
        executionId,
        {
          type: "execution-finished",
          outcome: "cancelled",
          failure: null,
          durationMs: null,
          responseCharacters: null,
        },
        finishedAt,
      );
      previewExecutionUpdate({ type: "state", taskId: execution.taskId, executionId, execution });
      previewExecutionUpdate({
        type: "terminal",
        taskId: execution.taskId,
        executionId,
        execution,
        outcome: "cancelled",
        error: null,
      });
    }, 0);
    return execution;
  },
  async steerExecution(executionId, text) {
    return previewDirection(executionId, text, "steer");
  },
  async queueFollowUp(executionId, text) {
    return previewDirection(executionId, text, "follow-up");
  },''',
        "preview cancel and direction journal",
    )
    value = sub_once(
        value,
        r"  async decideToolApproval\(approvalId, approved\) \{.*?\n    return \{ approval, event \};\n  \},",
        '''  async decideToolApproval(approvalId, approved) {
    const approval = previewApprovals.get(approvalId);
    if (!approval || approval.state !== "pending")
      throw new Error("Approval already settled");
    approval.state = approved ? "approved" : "denied";
    approval.decidedAtMs = Date.now();
    const detail = previewDetails.get(approval.taskId);
    const found = previewExecution(approval.executionId);
    if (!detail || !found) throw new Error("Preview task execution does not exist");
    const decision = previewJournalRecord(
      detail,
      approval.executionId,
      { type: "approval-decided", approvalId: approval.id, approved },
      approval.decidedAtMs,
    );
    found.execution.state = "running";
    found.execution.updatedAtMs = approval.decidedAtMs;
    const resumed = previewJournalRecord(
      detail,
      approval.executionId,
      { type: "execution-resumed", approvalId: approval.id },
      approval.decidedAtMs,
    );
    previewExecutionUpdate({
      type: "state",
      taskId: approval.taskId,
      executionId: approval.executionId,
      execution: found.execution,
    });
    return { approval, execution: found.execution, records: [decision, resumed] };
  },''',
        "preview approval journal",
    )
    write(path, value)


def patch_app():
    path = "apps/desktop/src/App.tsx"
    value = read(path)
    value = replace_once(
        value,
        '''    setTaskDetail((current) =>
      !current || current.task.id !== decision.event.taskId
        ? current
        : { ...current, events: [...current.events, decision.event] },
    );''',
        '''    setTaskDetail((current) =>
      !current || current.task.id !== decision.approval.taskId
        ? current
        : { ...current, events: [...current.events, ...decision.records] },
    );''',
        "approval decision journal records",
    )
    value = replace_once(
        value,
        '''function latestToolDetails(events: TaskEvent[], toolName: string) {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index];
    if (event.kind !== "tool.finished") continue;
    const payload = event.payload as { toolName?: string; details?: unknown };
    if (payload.toolName === toolName) return payload.details ?? null;
  }
  return null;
}''',
        '''function latestToolDetails(events: TaskEvent[], toolName: string) {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const record = events[index];
    if (record.event.type !== "tool-result-recorded") continue;
    const result = record.event.result;
    if (toolName === "edit_file" && result.type === "edit") return result;
    if (toolName === "run_command" && result.type === "shell") return result;
  }
  return null;
}''',
        "typed Changes and Terminal journal projection",
    )
    write(path, value)


def patch_package():
    path = "apps/desktop/package.json"
    data = json.loads(read(path))
    scripts = data["scripts"]
    scripts["generate:journal"] = "node scripts/generate-task-journal-contract.mjs"
    scripts["check:journal"] = "node scripts/generate-task-journal-contract.mjs --check"
    write(path, json.dumps(data, indent=2) + "\n")


def patch_verify():
    path = ".github/workflows/verify.yml"
    value = read(path)
    value = replace_once(
        value,
        '''      - name: Test renderer
        working-directory: apps/desktop
        run: npm test
''',
        '''      - name: Verify generated Task Journal contract
        working-directory: apps/desktop
        run: npm run check:journal
      - name: Test renderer
        working-directory: apps/desktop
        run: npm test
''',
        "Task Journal generation verification",
    )
    write(path, value)


def patch_authority_test():
    path = "apps/desktop/src/authority-boundary.test.ts"
    value = read(path)
    value = replace_once(
        value,
        '''    expect(taskExecution).toContain("submitPrompt");
    expect(taskExecution).toContain("subscribeExecutionUpdates");''',
        '''    expect(taskExecution).toContain("submitPrompt");
    expect(taskExecution).toContain("subscribeExecutionUpdates");
    expect(desktop).toContain("TaskJournalEvent");
    expect(desktop).not.toContain("kind: string");
    expect(desktop).not.toContain("payload: unknown");''',
        "renderer typed journal boundary",
    )
    write(path, value)


patch_desktop()
patch_app()
patch_package()
patch_verify()
patch_authority_test()
print("issue #48 renderer cutover applied")
