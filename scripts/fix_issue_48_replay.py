from pathlib import Path

path = Path("apps/desktop/src-tauri/src/store/journal.rs")
value = path.read_text(encoding="utf-8")
old = '''    transaction.execute("DELETE FROM tool_approvals WHERE task_id = ?1", [task_id])?;
    transaction.execute("DELETE FROM task_messages WHERE task_id = ?1", [task_id])?;
    transaction.execute("DELETE FROM executions WHERE task_id = ?1", [task_id])?;
    for record in &records {
        apply_record_projection(&transaction, record)?;
    }
'''
new = '''    transaction.execute("DELETE FROM tool_approvals WHERE task_id = ?1", [task_id])?;
    transaction.execute("DELETE FROM task_messages WHERE task_id = ?1", [task_id])?;
    transaction.execute("DELETE FROM executions WHERE task_id = ?1", [task_id])?;

    // Prompt acceptance is deliberately journaled before ExecutionQueued so a crash can
    // never expose a provider launch without its accepted user intent. Projection replay
    // therefore creates every Execution first, then replays the remaining causal facts.
    for record in &records {
        if is_execution_seed(record) {
            apply_record_projection(&transaction, record)?;
        }
    }
    for record in &records {
        if !is_execution_seed(record) {
            apply_record_projection(&transaction, record)?;
        }
    }
'''
if value.count(old) != 1:
    raise RuntimeError("rebuild loop shape changed")
value = value.replace(old, new, 1)
marker = '''fn apply_record_projection(
    transaction: &Transaction<'_>,
    record: &TaskEvent,
) -> Result<(), StoreError> {
'''
helper = '''fn is_execution_seed(record: &TaskEvent) -> bool {
    matches!(record.event, TaskJournalEvent::ExecutionQueued { .. })
        || matches!(
            &record.event,
            TaskJournalEvent::LegacyImported {
                source: LegacySource::Execution,
                ..
            }
        )
}

fn apply_record_projection(
    transaction: &Transaction<'_>,
    record: &TaskEvent,
) -> Result<(), StoreError> {
'''
if value.count(marker) != 1:
    raise RuntimeError("projection function marker changed")
value = value.replace(marker, helper, 1)
path.write_text(value, encoding="utf-8")
