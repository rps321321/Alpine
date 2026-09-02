from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "apps/desktop/src-tauri/src/store/journal.rs"
value = PATH.read_text(encoding="utf-8")
old = '''    for key in object.keys() {
        if !allowed
            .iter()
            .any(|allowed_key| key.as_str() == *allowed_key)
        {'''
new = '''    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {'''
if value.count(old) != 1:
    raise RuntimeError(f"manual contains replacement: expected 1 match, found {value.count(old)}")
PATH.write_text(value.replace(old, new, 1), encoding="utf-8")
print("Clippy-safe Task Journal field lookup applied")
