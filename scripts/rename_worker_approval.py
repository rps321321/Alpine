from pathlib import Path

root = Path(__file__).resolve().parents[1]
for relative in [
    "apps/desktop/src/harness/pi.ts",
    "apps/desktop/src/agent-worker.ts",
]:
    path = root / relative
    value = path.read_text(encoding="utf-8")
    count = value.count("requestToolApproval")
    if count == 0:
        raise RuntimeError(f"{relative}: approval method was not found")
    path.write_text(
        value.replace("requestToolApproval", "proposeEffectApproval"),
        encoding="utf-8",
    )
print("renamed worker approval proposal method")
