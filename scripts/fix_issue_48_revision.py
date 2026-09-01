from pathlib import Path

path = Path("apps/desktop/src-tauri/src/store/execution.rs")
value = path.read_text(encoding="utf-8")
old = '''    let model_revision = specification
        .model_revision
        .as_deref()
        .map(|value| validate_optional_identifier("model revision", Some(value), 160))
        .transpose()?
        .flatten();'''
new = '''    let model_revision = specification
        .model_revision
        .as_deref()
        .map(|value| validate_optional_identifier("model revision", Some(value), 160))
        .transpose()?
        .flatten()
        .map(str::to_owned);'''
count = value.count(old)
if count != 1:
    raise RuntimeError(f"model revision validator shape changed: found {count}")
path.write_text(value.replace(old, new, 1), encoding="utf-8")
print("execution model revision now owns its validated identifier")
