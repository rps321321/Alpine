from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def update(path: str, transform) -> None:
    file = ROOT / path
    before = file.read_text(encoding="utf-8")
    after = transform(before)
    if after == before:
        raise RuntimeError(f"{path}: transformation made no change")
    file.write_text(after, encoding="utf-8")


def replace_once(value: str, old: str, new: str, label: str) -> str:
    count = value.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one match, found {count}")
    return value.replace(old, new, 1)


def patch_store(value: str) -> str:
    old = '''        transaction.execute(
            "UPDATE tasks SET updated_at_ms = ?2 WHERE id = ?1",
            params![approval.task_id, decided_at_ms],
        )?;
'''
    new = '''        transaction.execute(
            "UPDATE executions
             SET state = 'running', updated_at_ms = ?2
             WHERE id = ?1 AND state = 'waiting-for-approval'",
            params![approval.execution_id.as_str(), decided_at_ms],
        )?;
        transaction.execute(
            "UPDATE tasks SET updated_at_ms = ?2 WHERE id = ?1",
            params![approval.task_id, decided_at_ms],
        )?;
'''
    return replace_once(value, old, new, "atomic approval execution transition")


def patch_supervisor(value: str) -> str:
    old = '''        supervisor.activate(&task_id, execution.id.as_str())?;
        supervisor.broadcast(ExecutionUpdate::Message {
'''
    new = '''        if let Err(error) = supervisor.activate(&task_id, execution.id.as_str()) {
            let _ = fail_execution(
                &store,
                &supervisor,
                execution.id.as_str(),
                error.clone(),
            );
            return Err(error);
        }
        supervisor.broadcast(ExecutionUpdate::Message {
'''
    value = replace_once(value, old, new, "activation failure settlement")

    old = '''    require_webview(&webview, MAIN_WEBVIEW)?;
    let decision = store
        .decide_tool_approval_with_event(&approval_id, approved)
        .map_err(|error| error.to_string())?;
    let execution_id = decision.approval.execution_id.to_string();
    let task_id = decision.approval.task_id.clone();
    supervisor.verify_active(&task_id, &execution_id)?;
    let current = store
        .get_execution(&execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if current.state == ExecutionState::WaitingForApproval {
        let running = store
            .transition_execution(&execution_id, ExecutionState::Running, None)
            .map_err(|error| error.to_string())?;
        supervisor.broadcast(state_update(running));
    }
'''
    new = '''    require_webview(&webview, MAIN_WEBVIEW)?;
    let approval = store
        .get_tool_approval(&approval_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Tool Approval does not exist".to_owned())?;
    let execution_id = approval.execution_id.to_string();
    let task_id = approval.task_id.clone();
    supervisor.verify_active(&task_id, &execution_id)?;
    let current = store
        .get_execution(&execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if current.state != ExecutionState::WaitingForApproval {
        return Err(format!(
            "Execution {execution_id} is not waiting for this approval"
        ));
    }
    let decision = store
        .decide_tool_approval_with_event(&approval_id, approved)
        .map_err(|error| error.to_string())?;
    let running = store
        .get_execution(&execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution no longer exists".to_owned())?;
    if running.state != ExecutionState::Running {
        let error = "approval decision did not atomically resume its Execution".to_owned();
        let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
        return Err(error);
    }
    supervisor.broadcast(state_update(running));
'''
    value = replace_once(value, old, new, "prevalidated atomic approval decision")

    old = '''    require_webview(&webview, AGENT_WORKER_WEBVIEW)?;
    let (_, execution_id) = event.identity();
    let execution_id = execution_id.to_owned();
    match handle_worker_event(&store, &supervisor, event) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
            Err(error)
        }
    }
'''
    new = '''    require_webview(&webview, AGENT_WORKER_WEBVIEW)?;
    let (_, execution_id) = event.identity();
    let execution_id = execution_id.to_owned();
    let current = store
        .get_execution(&execution_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the Execution does not exist".to_owned())?;
    if current.state.is_terminal() {
        return Ok(());
    }
    supervisor.verify_active(&current.task_id, &execution_id)?;
    match handle_worker_event(&store, &supervisor, event) {
        Ok(()) => Ok(()),
        Err(error) => {
            let current = store
                .get_execution(&execution_id)
                .map_err(|cause| cause.to_string())?
                .ok_or_else(|| "the Execution no longer exists".to_owned())?;
            if current.state.is_terminal() {
                return Ok(());
            }
            let _ = fail_execution(&store, &supervisor, &execution_id, error.clone());
            Err(error)
        }
    }
'''
    return replace_once(value, old, new, "idempotent late worker event handling")


def patch_worker(value: str) -> str:
    value = replace_once(
        value,
        'import { Channel, invoke } from "@tauri-apps/api/core";\n',
        'import { Channel, invoke } from "@tauri-apps/api/core";\nimport { ApprovalContinuations } from "./approval-continuations";\n',
        "approval continuation import",
    )
    waiter_pattern = r'''type ApprovalWaiter = \{.*?\};\n\n'''
    value, count = re.subn(waiter_pattern, "", value, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"worker waiter type: expected one match, found {count}")
    value = replace_once(
        value,
        "const approvalWaiters = new Map<string, ApprovalWaiter>();\n",
        "const approvalContinuations = new ApprovalContinuations();\n",
        "approval registry",
    )
    functions_pattern = r'''function rejectApprovals\(error: Error\) \{.*?\n\}\n\nfunction settleApproval\(approvalId: string, approved: boolean\) \{.*?\n\}\n\nfunction waitForApproval\(approvalId: string, signal\?: AbortSignal\) \{.*?\n\}\n'''
    replacement = '''function rejectApprovals(error: Error) {
  approvalContinuations.rejectAll(error);
}

function settleApproval(approvalId: string, approved: boolean) {
  approvalContinuations.settle(approvalId, approved);
}

function waitForApproval(approvalId: string, signal?: AbortSignal) {
  return approvalContinuations.wait(approvalId, signal);
}
'''
    value, count = re.subn(functions_pattern, replacement, value, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"worker approval functions: expected one match, found {count}")
    return value


def patch_store_test(value: str) -> str:
    marker = '''#[test]
fn deleting_a_task_cascades_all_execution_owned_records() {
'''
    test = '''#[test]
fn approval_decision_atomically_resumes_its_waiting_execution() {
    let (_temporary, store, task_id) = setup();
    let execution = store
        .create_execution(CreateExecution {
            task_id: task_id.clone(),
            specification: specification('a'),
        })
        .unwrap();
    store
        .transition_execution(execution.id.as_str(), ExecutionState::Preparing, None)
        .unwrap();
    store
        .transition_execution(execution.id.as_str(), ExecutionState::Running, None)
        .unwrap();
    store
        .transition_execution(
            execution.id.as_str(),
            ExecutionState::WaitingForApproval,
            None,
        )
        .unwrap();
    let approval = store
        .request_tool_approval(NewToolApproval {
            task_id,
            execution_id: execution.id.clone(),
            tool_call_id: "call-atomic".to_owned(),
            operation: "shell".to_owned(),
            proposal: serde_json::json!({"command": "cargo test", "timeoutSeconds": 30}),
        })
        .unwrap();

    let decision = store
        .decide_tool_approval_with_event(&approval.id, true)
        .unwrap();

    assert_eq!(decision.approval.state, ApprovalState::Approved);
    assert_eq!(decision.event.execution_id, execution.id);
    assert_eq!(
        store
            .get_execution(execution.id.as_str())
            .unwrap()
            .unwrap()
            .state,
        ExecutionState::Running
    );
}

'''
    return replace_once(value, marker, test + marker, "atomic approval test")


update("apps/desktop/src-tauri/src/store.rs", patch_store)
update("apps/desktop/src-tauri/src/supervisor.rs", patch_supervisor)
update("apps/desktop/src/agent-worker.ts", patch_worker)
update("apps/desktop/src-tauri/tests/desktop_store.rs", patch_store_test)
print("issue #47 hardening applied")
