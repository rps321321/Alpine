from pathlib import Path

root = Path(__file__).resolve().parents[1]

lib = root / "apps/desktop/src-tauri/src/lib.rs"
value = lib.read_text(encoding="utf-8")
old = "pub(crate) struct PiLaunchConfig {"
if value.count(old) != 1:
    raise RuntimeError("unexpected PiLaunchConfig visibility")
lib.write_text(value.replace(old, "pub struct PiLaunchConfig {", 1), encoding="utf-8")

supervisor = root / "apps/desktop/src-tauri/src/supervisor.rs"
value = supervisor.read_text(encoding="utf-8")
old = "        config: PiLaunchConfig,"
if value.count(old) != 1:
    raise RuntimeError("unexpected Start config field")
value = value.replace(old, "        config: Box<PiLaunchConfig>,", 1)
old = "            config: launch,"
if value.count(old) != 1:
    raise RuntimeError("unexpected Start config construction")
value = value.replace(old, "            config: Box::new(launch),", 1)
supervisor.write_text(value, encoding="utf-8")

print("fixed supervisor visibility and command size")
