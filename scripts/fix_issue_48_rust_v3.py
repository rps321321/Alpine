from pathlib import Path

script = Path(__file__).with_name("fix_issue_48_rust_v2.py")
source = script.read_text(encoding="utf-8")
source = source.replace(
    'if v.count(success_old) != 2:\n    raise RuntimeError(f"successful tool settlements: expected 2 matches, found {v.count(success_old)}")',
    'if v.count(success_old) != 1:\n    raise RuntimeError(f"successful tool settlements: expected 1 match, found {v.count(success_old)}")',
)
exec(compile(source, str(script), "exec"), {"__file__": str(script), "__name__": "__main__"})

supervisor = Path(__file__).resolve().parents[1] / "apps/desktop/src-tauri/src/supervisor.rs"
value = supervisor.read_text(encoding="utf-8")
old = '''            let settled = store
                .settle_tool_effect(
                    &approval_id,
                    succeeded,
                    ToolResult::from(result.clone()),
                    Some(&detail),
                )
                .map_err(|error| error.to_string())?;'''
new = '''            let settled = journal_or_fail(
                &store,
                &supervisor,
                &execution_id,
                store.settle_tool_effect(
                    &approval_id,
                    succeeded,
                    ToolResult::from(result.clone()),
                    Some(&detail),
                ),
            )?;'''
count = value.count(old)
if count != 1:
    raise RuntimeError(f"shell success settlement: expected 1 match, found {count}")
supervisor.write_text(value.replace(old, new, 1), encoding="utf-8")
print("issue #48 Rust hardening v3 applied")
