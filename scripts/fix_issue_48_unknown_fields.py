from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "apps/desktop/src-tauri/src/store/journal.rs"
value = PATH.read_text(encoding="utf-8")

marker = '''pub(super) fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEvent> {'''
helper = '''fn decode_task_journal_event(event_json: &str) -> Result<TaskJournalEvent, StoreError> {
    let value: Value = serde_json::from_str(event_json).map_err(|error| {
        StoreError::message(format!("invalid Task Journal event JSON: {error}"))
    })?;
    let object = value
        .as_object()
        .ok_or_else(|| StoreError::message("Task Journal event must be a JSON object"))?;
    let event_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| StoreError::message("Task Journal event requires a string type"))?
        .to_owned();
    let allowed: &[&str] = match event_type.as_str() {
        "user-prompt-accepted" => &["type", "content"],
        "user-direction-accepted" => &["type", "direction", "content"],
        "execution-queued" => &["type", "executionSpecId"],
        "execution-preparing" => &["type"],
        "execution-started" => &["type"],
        "assistant-message-completed" => &["type", "content"],
        "tool-proposed" => &["type", "approvalId", "toolCallId", "proposal"],
        "execution-waiting-for-approval" => &["type", "approvalId"],
        "approval-decided" => &["type", "approvalId", "approved"],
        "execution-resumed" => &["type", "approvalId"],
        "tool-started" => &["type", "approvalId", "proposal"],
        "tool-result-recorded" => &["type", "approvalId", "succeeded", "result"],
        "tool-settled" => &["type", "approvalId", "state", "detail"],
        "approval-interrupted" => &["type", "approvalId", "detail"],
        "execution-cancelling" => &["type"],
        "execution-finished" => &[
            "type",
            "outcome",
            "failure",
            "durationMs",
            "responseCharacters",
        ],
        "legacy-imported" => &[
            "type",
            "source",
            "sourceId",
            "sourceSequence",
            "sourceOccurredAtMs",
            "causalOrder",
            "data",
        ],
        _ => {
            return Err(StoreError::message(format!(
                "unsupported Task Journal event type '{event_type}'"
            )));
        }
    };
    for key in object.keys() {
        if !allowed
            .iter()
            .any(|allowed_key| key.as_str() == *allowed_key)
        {
            return Err(StoreError::message(format!(
                "Task Journal event '{event_type}' contains unknown field '{key}'"
            )));
        }
    }
    serde_json::from_value(value)
        .map_err(|error| StoreError::message(format!("invalid Task Journal event: {error}")))
}

pub(super) fn record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEvent> {'''
if value.count(marker) != 1:
    raise RuntimeError(f"record decoder marker: expected 1 match, found {value.count(marker)}")
value = value.replace(marker, helper, 1)

old = '''    let event_json: String = row.get(5)?;
    let event = serde_json::from_str::<TaskJournalEvent>(&event_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;'''
new = '''    let event_json: String = row.get(5)?;
    let event = decode_task_journal_event(&event_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })?;'''
if value.count(old) != 1:
    raise RuntimeError(f"record decoder replacement: expected 1 match, found {value.count(old)}")
value = value.replace(old, new, 1)

PATH.write_text(value, encoding="utf-8")
print("strict Task Journal top-level field decoding applied")
