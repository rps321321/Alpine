from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]


def update(path: str, transform):
    file = root / path
    before = file.read_text(encoding="utf-8")
    after = transform(before)
    if after == before:
        raise RuntimeError(f"no change applied to {path}")
    file.write_text(after, encoding="utf-8")


def remove_preview_effects(value: str) -> str:
    value, count = re.subn(
        r'\n  async editProjectFile\(_taskId, approvalId, edit\) \{.*?\n  \},\n  async runProjectShell\(_taskId, approvalId, shell\) \{.*?\n  \},',
        "",
        value,
        count=1,
        flags=re.S,
    )
    if count != 1:
        raise RuntimeError(f"expected obsolete preview effects once, found {count}")
    return value


update("apps/desktop/src/desktop.ts", remove_preview_effects)


def trim_host_imports(value: str) -> str:
    old = '''use workspace::{
    WorkspaceEdit, WorkspaceEditResult, WorkspaceEntry, WorkspaceRead, WorkspaceSearchMatch,
    WorkspaceShell, WorkspaceShellResult,
};'''
    new = "use workspace::{WorkspaceEntry, WorkspaceRead, WorkspaceSearchMatch};"
    if value.count(old) != 1:
        raise RuntimeError("unexpected workspace import shape")
    return value.replace(old, new, 1)


update("apps/desktop/src-tauri/src/lib.rs", trim_host_imports)


def rename_worker_effects(value: str) -> str:
    value = value.replace("editProjectFile(", "executeApprovedEdit(")
    value = value.replace("runProjectShell(", "executeApprovedShell(")
    return value


update("apps/desktop/src/harness/pi.ts", rename_worker_effects)
update("apps/desktop/src/agent-worker.ts", rename_worker_effects)

print("issue #47 cleanup applied")
