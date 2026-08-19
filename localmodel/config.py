from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parents[1]


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8-sig"))


def artifact_manifest() -> dict[str, Any]:
    return read_json(REPO_ROOT / "config" / "artifacts.json")


def profiles() -> dict[str, dict[str, Any]]:
    return {
        path.stem: read_json(path)
        for path in sorted((REPO_ROOT / "config" / "profiles").glob("*.json"))
    }


def install_session(install_root: Path) -> dict[str, Any]:
    return read_json(install_root / "config" / "session.json")


def install_profile(install_root: Path, name: str) -> dict[str, Any]:
    return read_json(install_root / "profiles" / f"{name}.json")


def sha256(path: Path, chunk_size: int = 8 * 1024 * 1024) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(chunk_size):
            digest.update(chunk)
    return digest.hexdigest()


def tree_sha256(root: Path, paths: list[Path]) -> str:
    """Hash both relative names and bytes so a benchmark suite has one identity."""
    digest = hashlib.sha256()
    for path in sorted(paths, key=lambda item: item.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix().encode("utf-8")
        digest.update(len(relative).to_bytes(4, "big"))
        digest.update(relative)
        digest.update(path.read_bytes())
    return digest.hexdigest()


def git_commit() -> str | None:
    result = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "rev-parse", "HEAD"],
        capture_output=True,
        text=True,
        check=False,
    )
    return result.stdout.strip() if result.returncode == 0 else None


def powershell() -> str:
    return "powershell.exe"
