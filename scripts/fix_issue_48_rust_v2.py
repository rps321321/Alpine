from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]

def load(path):
    return (ROOT / path).read_text(encoding="utf-8")

def save(path, value):
    (ROOT / path).write_text(value, encoding="utf-8")

def once(value, old, new, label):
    count = value.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected 1 match, found {count}")
    return value.replace(old, new, 1)

def regex_once(value, pattern, new, label):
    value, count = re.subn(pattern, new, value, count=1, flags=re.S)
    if count != 1:
        raise RuntimeError(f"{label}: expected 1 match, found {count}")
    return value

# Remove superseded #46 mutation helpers.
p = "apps/desktop/src-tauri/src/store/execution.rs"
v = load(p)
v = regex_once(v, r'\n#\[derive\(Clone, Debug, Deserialize\)\]\n#\[serde\(rename_all = "camelCase", deny_unknown_fields\)\]\npub struct CreateExecution \{\n    pub task_id: String,\n    pub specification: NewExecutionSpecification,\n\}\n', '\n', 'CreateExecution')
v = regex_once(v, r'\npub\(super\) fn ensure_execution_for_task\(.*?\n\}\n\npub\(super\) fn task_has_executions', '\npub(super) fn task_has_executions', 'ensure_execution_for_task')
save(p, v)

# Fix strict-Clippy and borrow-safe replay matching.
p = "apps/desktop/src-tauri/src/store/journal.rs"
v = load(p)
v = v.replace("matches!(record.event,", "matches!(&record.event,")
v = once(v, "            source_sequence,\n            source_occurred_at_ms,", "            source_sequence: _,\n            source_occurred_at_ms,", "legacy source_sequence binding")
save(p, v)

# Fail-stop exact active Execution when authoritative journal mutations fail.
p = "apps/desktop/src-tauri/src/supervisor.rs"
v = load(p)
v = once(
    v,
    "        TaskEvent, TaskMessage, ToolApproval, ToolApprovalDecision, ToolProposal, ToolResult,\n        UserDirection,\n",
    "        StoreError, TaskEvent, TaskMessage, ToolApproval, ToolApprovalDecision, ToolProposal,\n        ToolResult, UserDirection,\n",
    "StoreError import",
)
v = once(
    v,
    "fn terminalize(\n    store: &DesktopStore,\n",
    '''fn journal_or_fail<T>(
    store: &DesktopStore,
    supervisor: &TaskSupervisor,
    execution_id: &str,
    result: Result<T, StoreError>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let message = error.to_string();
            let _ = fail_execution(store, supervisor, execution_id, message.clone());
            Err(message)
        }
    }
}

fn terminalize(
    store: &DesktopStore,
''',
    "journal_or_fail helper",
)
v = once(
    v,
    '''        if let Ok(decision) = store.decide_tool_approval_recorded(&approval.id, false) {
            broadcast_records(&supervisor, &decision.records);
            supervisor.broadcast(state_update(decision.execution));
            let _ = supervisor.send_worker(AgentWorkerCommand::ApprovalDecision {
                approval_id: approval.id,
                approved: false,
            });
        }''',
    '''        let decision = journal_or_fail(
            &store,
            &supervisor,
            &execution_id,
            store.decide_tool_approval_recorded(&approval.id, false),
        )?;
        broadcast_records(&supervisor, &decision.records);
        supervisor.broadcast(state_update(decision.execution));
        let _ = supervisor.send_worker(AgentWorkerCommand::ApprovalDecision {
            approval_id: approval.id,
            approved: false,
        });''',
    "cancel pending approval",
)
v = once(
    v,
    '''        let outcome = store
            .record_execution_state(&execution_id, ExecutionState::Cancelling)
            .map_err(|error| error.to_string())?;''',
    '''        let outcome = journal_or_fail(
            &store,
            &supervisor,
            &execution_id,
            store.record_execution_state(&execution_id, ExecutionState::Cancelling),
        )?;''',
    "cancel journal state",
)
v = once(
    v,
    '''    let recorded = store
        .record_direction(execution_id, direction, &text)
        .map_err(|error| error.to_string())?;''',
    '''    let recorded = journal_or_fail(
        store,
        supervisor,
        execution_id,
        store.record_direction(execution_id, direction, &text),
    )?;''',
    "direction journal write",
)
v = once(
    v,
    '''    let decision = store
        .decide_tool_approval_recorded(&approval_id, approved)
        .map_err(|error| error.to_string())?;''',
    '''    let decision = journal_or_fail(
        &store,
        &supervisor,
        &execution_id,
        store.decide_tool_approval_recorded(&approval_id, approved),
    )?;''',
    "approval decision journal write",
)
v = once(
    v,
    '''    supervisor.verify_active(&input.task_id, input.execution_id.as_str())?;
    let outcome = store
        .propose_tool(input)
        .map_err(|error| error.to_string())?;''',
    '''    supervisor.verify_active(&input.task_id, input.execution_id.as_str())?;
    let execution_id = input.execution_id.to_string();
    let outcome = journal_or_fail(
        &store,
        &supervisor,
        &execution_id,
        store.propose_tool(input),
    )?;''',
    "tool proposal journal write",
)
claim_old = '''    let claim = store
        .claim_tool_effect(&approval_id, &proposal)
        .map_err(|error| error.to_string())?;'''
claim_new = '''    let claim = journal_or_fail(
        &store,
        &supervisor,
        &execution_id,
        store.claim_tool_effect(&approval_id, &proposal),
    )?;'''
if v.count(claim_old) != 2:
    raise RuntimeError(f"tool claim writes: expected 2 matches, found {v.count(claim_old)}")
v = v.replace(claim_old, claim_new)

success_old = '''            let settled = store
                .settle_tool_effect(
                    &approval_id,
                    true,
                    ToolResult::from(result.clone()),
                    Some(&detail),
                )
                .map_err(|error| error.to_string())?;'''
success_new = '''            let settled = journal_or_fail(
                &store,
                &supervisor,
                &execution_id,
                store.settle_tool_effect(
                    &approval_id,
                    true,
                    ToolResult::from(result.clone()),
                    Some(&detail),
                ),
            )?;'''
if v.count(success_old) != 2:
    raise RuntimeError(f"successful tool settlements: expected 2 matches, found {v.count(success_old)}")
v = v.replace(success_old, success_new)

failure_old = '''            let settled = store
                .settle_tool_effect(
                    &approval_id,
                    false,
                    ToolResult::Failure {
                        message: message.clone(),
                    },
                    Some(&message),
                )
                .map_err(|cause| cause.to_string())?;'''
failure_new = '''            let settled = journal_or_fail(
                &store,
                &supervisor,
                &execution_id,
                store.settle_tool_effect(
                    &approval_id,
                    false,
                    ToolResult::Failure {
                        message: message.clone(),
                    },
                    Some(&message),
                ),
            )?;'''
if v.count(failure_old) != 2:
    raise RuntimeError(f"failed tool settlements: expected 2 matches, found {v.count(failure_old)}")
v = v.replace(failure_old, failure_new)

v = once(
    v,
    '''    let result = tauri::async_runtime::spawn_blocking(move || {
        workspace::execute_shell(&store_for_shell, &task_for_shell, shell)
    })
    .await
    .map_err(|error| format!("workspace shell worker failed: {error}"))?;''',
    '''    let result = match tauri::async_runtime::spawn_blocking(move || {
        workspace::execute_shell(&store_for_shell, &task_for_shell, shell)
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let message = format!("workspace shell worker failed: {error}");
            let _ = fail_execution(&store, &supervisor, &execution_id, message.clone());
            return Err(message);
        }
    };''',
    "shell worker join failure",
)
save(p, v)

# Tests match through references so the typed union is never moved out of borrowed rows.
p = "apps/desktop/src-tauri/tests/desktop_store.rs"
v = load(p)
v = v.replace("matches!(record.event,", "matches!(&record.event,")
v = v.replace("matches!(detail.events[0].event,", "matches!(&detail.events[0].event,")
v = v.replace("matches!(detail.events[1].event,", "matches!(&detail.events[1].event,")
v = v.replace("matches!(\n        detail.events.last().unwrap().event,", "matches!(\n        &detail.events.last().unwrap().event,")
save(p, v)

print("issue #48 Rust hardening v2 applied")
