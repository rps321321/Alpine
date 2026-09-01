from pathlib import Path
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


# Remove #46 writer helpers that are no longer part of the journal-owned path.
path = "apps/desktop/src-tauri/src/store/execution.rs"
value = read(path)
value = sub_once(
    value,
    r"\n#\[derive\(Clone, Debug, Deserialize\)\]\n#\[serde\(rename_all = \"camelCase\", deny_unknown_fields\)\]\npub struct CreateExecution \{\n    pub task_id: String,\n    pub specification: NewExecutionSpecification,\n\}\n",
    "\n",
    "obsolete CreateExecution",
)
value = sub_once(
    value,
    r"\npub\(super\) fn ensure_execution_for_task\(.*?\n\}\n\npub\(super\) fn task_has_executions",
    "\npub(super) fn task_has_executions",
    "obsolete ensure_execution_for_task",
)
write(path, value)

# Tighten journal projection matching and remove an intentionally unused legacy binding.
path = "apps/desktop/src-tauri/src/store/journal.rs"
value = read(path)
value = value.replace("matches!(record.event,", "matches!(&record.event,")
value = replace_once(
    value,
    "            source_sequence,\n            source_occurred_at_ms,",
    "            source_sequence: _,\n            source_occurred_at_ms,",
    "unused legacy source sequence",
)
write(path, value)

# Ensure every authoritative journal failure stops/fails the exact active Execution.
path = "apps/desktop/src-tauri/src/supervisor.rs"
value = read(path)
value = value.replace(
    "        TaskEvent, TaskMessage, ToolApproval, ToolApprovalDecision, ToolProposal, ToolResult,\n        UserDirection,\n",
    "        StoreError, TaskEvent, TaskMessage, ToolApproval, ToolApprovalDecision, ToolProposal,\n        ToolResult, UserDirection,\n",
)
marker = '''fn terminalize(
    store: &DesktopStore,
'''
helper = '''fn journal_or_fail<T>(
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
'''
value = replace_once(value, marker, helper, "journal failure helper")

value = replace_once(
    value,
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
    "cancellation approval persistence",
)
value = replace_once(
    value,
    '''        let outcome = store
            .record_execution_state(&execution_id, ExecutionState::Cancelling)
            .map_err(|error| error.to_string())?;''',
    '''        let outcome = journal_or_fail(
            &store,
            &supervisor,
            &execution_id,
            store.record_execution_state(&execution_id, ExecutionState::Cancelling),
        )?;''',
    "cancellation journal failure",
)
value = replace_once(
    value,
    '''    let recorded = store
        .record_direction(execution_id, direction, &text)
        .map_err(|error| error.to_string())?;''',
    '''    let recorded = journal_or_fail(
        store,
        supervisor,
        execution_id,
        store.record_direction(execution_id, direction, &text),
    )?;''',
    "direction journal failure",
)
value = replace_once(
    value,
    '''    let decision = store
        .decide_tool_approval_recorded(&approval_id, approved)
        .map_err(|error| error.to_string())?;''',
    '''    let decision = journal_or_fail(
        &store,
        &supervisor,
        &execution_id,
        store.decide_tool_approval_recorded(&approval_id, approved),
    )?;''',
    "approval decision journal failure",
)
value = replace_once(
    value,
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
    "tool proposal journal failure",
)
value = replace_once(
    value,
    '''    let claim = store
        .claim_tool_effect(&approval_id, &proposal)
        .map_err(|error| error.to_string())?;''',
    '''    let claim = journal_or_fail(
        &store,
        &supervisor,
        &execution_id,
        store.claim_tool_effect(&approval_id, &proposal),
    )?;''',
    "edit claim journal failure",
)
# Second occurrence is shell claim.
value = replace_once(
    value,
    '''    let claim = store
        .claim_tool_effect(&approval_id, &proposal)
        .map_err(|error| error.to_string())?;''',
    '''    let claim = journal_or_fail(
        &store,
        &supervisor,
        &execution_id,
        store.claim_tool_effect(&approval_id, &proposal),
    )?;''',
    "shell claim journal failure",
)

# Convert all settlement writes in the edit/shell handlers to fail-stop behavior.
value = value.replace(
    '''            let settled = store
                .settle_tool_effect(''',
    '''            let settled = journal_or_fail(
                &store,
                &supervisor,
                &execution_id,
                store.settle_tool_effect(''',
)
value = value.replace(
    '''                    Some(&detail),
                )
                .map_err(|error| error.to_string())?;''',
    '''                    Some(&detail),
                ),
            )?;''',
)
value = value.replace(
    '''                    Some(&message),
                )
                .map_err(|cause| cause.to_string())?;''',
    '''                    Some(&message),
                ),
            )?;''',
)

# A failure to join the shell worker also has to stop the owning Execution.
old = '''    let result = tauri::async_runtime::spawn_blocking(move || {
        workspace::execute_shell(&store_for_shell, &task_for_shell, shell)
    })
    .await
    .map_err(|error| format!("workspace shell worker failed: {error}"))?;'''
new = '''    let result = match tauri::async_runtime::spawn_blocking(move || {
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
    };'''
value = replace_once(value, old, new, "shell worker join failure")
write(path, value)

# Borrow journal events in tests rather than attempting to move non-Copy event enums.
for path in [
    "apps/desktop/src-tauri/tests/desktop_store.rs",
]:
    value = read(path)
    value = value.replace("matches!(record.event,", "matches!(&record.event,")
    value = value.replace("matches!(detail.events[0].event,", "matches!(&detail.events[0].event,")
    value = value.replace("matches!(detail.events[1].event,", "matches!(&detail.events[1].event,")
    value = value.replace(
        "matches!(\n        detail.events.last().unwrap().event,",
        "matches!(\n        &detail.events.last().unwrap().event,",
    )
    write(path, value)

print("issue #48 Rust cleanup and fail-stop hardening applied")
