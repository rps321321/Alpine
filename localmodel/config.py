from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import time
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .locking import FileLease
from .io import write_json_atomic

REPO_ROOT = Path(__file__).resolve().parents[1]
SESSION_SCHEMAS = {3, 4, 5}


class ConfigError(ValueError):
    """A rendered Session Config or selected Profile is not usable."""


@dataclass(frozen=True)
class ResolvedSession:
    install_root: Path
    session: dict[str, Any]
    profile_name: str
    profile: dict[str, Any]
    runtime_name: str
    server: Path
    model: Path
    mmproj: Path
    chat_template: Path
    api_key_file: Path
    base_url_file: Path
    state_file: Path

    @property
    def host(self) -> str:
        return str(self.session["host"])

    @property
    def port(self) -> int:
        return int(self.session["port"])

    @property
    def base_url(self) -> str:
        return f"http://{self.host}:{self.port}"


def read_json(path: Path) -> dict[str, Any]:
    for attempt in range(200):
        try:
            value = json.loads(path.read_text(encoding="utf-8-sig"))
            break
        except PermissionError:
            if attempt == 199:
                raise
            time.sleep(0.01)
        except FileNotFoundError as exc:
            raise ConfigError(f"JSON file missing: {path}") from exc
        except json.JSONDecodeError as exc:
            raise ConfigError(f"Malformed JSON in {path}: {exc.msg}") from exc
    if not isinstance(value, dict):
        raise ConfigError(f"Expected a JSON object in {path}")
    return value


def _required_string(value: dict[str, Any], name: str, source: Path) -> str:
    candidate = value.get(name)
    if not isinstance(candidate, str) or not candidate.strip():
        raise ConfigError(f"{source}: required value '{name}' must be a non-empty string")
    return candidate


def _validate_profile(profile: dict[str, Any], name: str, source: Path) -> None:
    if _required_string(profile, "name", source) != name:
        raise ConfigError(f"{source}: Profile name does not match selected name '{name}'")
    _required_string(profile, "runtime", source)
    _required_string(profile, "kv_cache", source)
    positive = ("context", "output", "parallel", "threads", "batch_size", "ubatch_size", "mtp_depth")
    for field in positive:
        value = profile.get(field)
        if not isinstance(value, int) or isinstance(value, bool) or value < 1:
            raise ConfigError(f"{source}: Profile value '{field}' must be a positive integer")
    block = profile.get("tensor_cpu_through_block")
    if not isinstance(block, int) or isinstance(block, bool) or block < 0:
        raise ConfigError(f"{source}: Profile value 'tensor_cpu_through_block' must be a non-negative integer")


def resolve_session(
    install_root: Path,
    profile_name: str | None = None,
    *,
    require_runtime: bool = False,
) -> ResolvedSession:
    """Resolve and validate the selected Session Config and Profile."""
    root = install_root.expanduser().resolve()
    publication_marker = root / ".setup-publishing.json"
    if publication_marker.exists():
        raise ConfigError(
            f"Setup publication is incomplete: {publication_marker}. "
            "Re-run setup to restore the prior installation before using it."
        )
    session_path = root / "config" / "session.json"
    session = read_json(session_path)
    schema = session.get("schema")
    if schema not in SESSION_SCHEMAS:
        raise ConfigError(
            f"{session_path}: unsupported Session Config schema {session.get('schema')!r}; "
            "expected 3, 4 or 5"
        )
    configured_root = Path(_required_string(session, "root", session_path)).expanduser().resolve()
    if configured_root != root:
        raise ConfigError(f"{session_path}: root resolves to {configured_root}, expected {root}")
    _required_string(session, "host", session_path)
    port = session.get("port")
    if not isinstance(port, int) or isinstance(port, bool) or not 1 <= port <= 65535:
        raise ConfigError(f"{session_path}: 'port' must be an integer between 1 and 65535")
    if profile_name:
        selected = profile_name
    elif schema == 3:
        selected = _required_string(session, "active_profile", session_path)
    else:
        alpine = root / "alpine.exe"
        if not alpine.is_file():
            raise ConfigError(
                f"schema {schema} default selection is Rust-owned and requires the installed alpine.exe"
            )
        result = subprocess.run(
            [str(alpine), "deployment-status", "--install-root", str(root), "--compact"],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            raise ConfigError("Alpine could not derive the deployment daily_default")
        try:
            deployment = json.loads(result.stdout)
            selected = str(deployment["roles"]["daily_default"])
        except (json.JSONDecodeError, KeyError, TypeError) as exc:
            raise ConfigError("Alpine returned incomplete deployment roles") from exc
    profile_path = root / "profiles" / f"{selected}.json"
    if not profile_path.is_file():
        raise ConfigError(f"Profile missing: {profile_path}")
    profile = read_json(profile_path)
    _validate_profile(profile, selected, profile_path)
    profile.pop("status", None)
    runtime_name = str(profile["runtime"])
    runtimes = session.get("runtimes")
    runtime_value = runtimes.get(runtime_name) if isinstance(runtimes, dict) else None
    if not isinstance(runtime_value, str) or not runtime_value.strip():
        raise ConfigError(f"Runtime '{runtime_name}' is unavailable for Profile '{selected}'")
    server = Path(runtime_value).expanduser().resolve()
    if require_runtime and not server.is_file():
        raise ConfigError(f"Runtime '{runtime_name}' is unavailable at {server}")
    path_fields = {
        "model": "model",
        "mmproj": "mmproj",
        "chat_template": "chat_template",
        "api_key_file": "api_key_file",
        "base_url_file": "base_url_file",
        "state_file": "state_file",
    }
    paths = {
        target: Path(_required_string(session, source, session_path)).expanduser().resolve()
        for source, target in path_fields.items()
    }
    return ResolvedSession(
        install_root=root,
        session=session,
        profile_name=selected,
        profile=profile,
        runtime_name=runtime_name,
        server=server,
        model=paths["model"],
        mmproj=paths["mmproj"],
        chat_template=paths["chat_template"],
        api_key_file=paths["api_key_file"],
        base_url_file=paths["base_url_file"],
        state_file=paths["state_file"],
    )


def artifact_manifest() -> dict[str, Any]:
    return read_json(REPO_ROOT / "config" / "artifacts.json")


def profiles() -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for path in sorted((REPO_ROOT / "config" / "profiles").glob("*.json")):
        profile = read_json(path)
        _validate_profile(profile, path.stem, path)
        result[path.stem] = profile
    return result


def install_session(install_root: Path) -> dict[str, Any]:
    return resolve_session(install_root).session


def install_profile(install_root: Path, name: str) -> dict[str, Any]:
    return resolve_session(install_root, name).profile


def select_active_profile(install_root: Path, name: str) -> Path:
    """Validate and atomically select an installed Profile, returning the backup path."""
    root = install_root.expanduser().resolve()
    schema = read_json(root / "config" / "session.json").get("schema")
    if schema in {4, 5}:
        raise ConfigError(
            f"schema {schema} has no active_profile; use Alpine Promotion or a one-session --profile override"
        )
    resolve_session(root, name, require_runtime=True)
    path = root / "config" / "session.json"
    with FileLease(root / ".setup.lock", {"kind": "profile-selection", "profile": name}):
        with FileLease(path.with_suffix(".lock"), {"kind": "session-config", "profile": name}):
            session = read_json(path)
            # Revalidate while holding both mutation locks so setup cannot publish
            # a replacement Session Config under this selection transaction.
            resolve_session(root, name, require_runtime=True)
            backup = path.with_name(
                "session.json.backup-"
                + datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S-%f")
                + "-"
                + uuid.uuid4().hex[:8]
            )
            shutil.copy2(path, backup)
            session["active_profile"] = name
            write_json_atomic(path, session, sort_keys=False)
    return backup


def sha256(path: Path, chunk_size: int = 8 * 1024 * 1024) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(chunk_size):
            digest.update(chunk)
    return digest.hexdigest()


def hardware_manifest_identity(root: Path = REPO_ROOT) -> dict[str, str] | None:
    candidates = sorted((root / "inventory").glob("hardware-*.json"))
    if not candidates:
        return None
    manifest = candidates[-1]
    return {"path": manifest.relative_to(root).as_posix(), "sha256": sha256(manifest)}


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
