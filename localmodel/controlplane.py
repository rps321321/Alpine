from __future__ import annotations

from pathlib import Path
from typing import Any, Iterator

from .config import ConfigError, git_commit, read_json, sha256
from .io import write_json_atomic


IDENTITY_SCHEMA = 1
IDENTITY_PATH = Path("config/control-plane.json")


def control_plane_files(repo_root: Path) -> Iterator[tuple[Path, Path]]:
    mappings = (
        (repo_root / "runtime" / "scripts", Path("scripts")),
        (repo_root / "runtime" / "launcher", Path("launcher")),
        (repo_root / "config" / "profiles", Path("profiles")),
    )
    for source_root, install_root in mappings:
        for source in sorted(path for path in source_root.glob("*") if path.is_file()):
            yield source, install_root / source.name
    artifact_manifest = repo_root / "config" / "artifacts.json"
    if artifact_manifest.is_file():
        yield artifact_manifest, Path("config/artifacts.json")
    profile_capabilities = repo_root / "config" / "profile-capabilities.json"
    if profile_capabilities.is_file():
        yield profile_capabilities, Path("config/profile-capabilities.json")


def expected_control_plane(repo_root: Path) -> dict[str, str]:
    return {
        installed.as_posix(): sha256(source)
        for source, installed in control_plane_files(repo_root.resolve())
    }


def write_control_plane_identity(
    repo_root: Path,
    install_root: Path,
    *,
    source_commit: str | None = None,
) -> dict[str, Any]:
    expected = expected_control_plane(repo_root)
    missing = [relative for relative in expected if not (install_root / relative).is_file()]
    if missing:
        raise ConfigError(f"Cannot record control-plane identity; installed files are missing: {', '.join(missing)}")
    mismatched = [
        relative for relative, source_hash in expected.items()
        if sha256(install_root / relative) != source_hash
    ]
    if mismatched:
        raise ConfigError(f"Cannot record control-plane identity; copied files differ: {', '.join(mismatched)}")
    identity = {
        "schema": IDENTITY_SCHEMA,
        "source_commit": source_commit if source_commit is not None else git_commit(),
        "files": [{"path": path, "sha256": digest} for path, digest in sorted(expected.items())],
    }
    write_json_atomic(install_root / IDENTITY_PATH, identity)
    return identity


def verify_control_plane(repo_root: Path, install_root: Path) -> dict[str, Any]:
    identity_path = install_root / IDENTITY_PATH
    identity = read_json(identity_path)
    if identity.get("schema") != IDENTITY_SCHEMA:
        raise ConfigError(
            f"{identity_path}: unsupported control-plane identity schema {identity.get('schema')!r}; "
            f"expected {IDENTITY_SCHEMA}"
        )
    files = identity.get("files")
    if not isinstance(files, list):
        raise ConfigError(f"{identity_path}: 'files' must be a list")
    recorded: dict[str, str] = {}
    generated: set[str] = set()
    for entry in files:
        if not isinstance(entry, dict) or not isinstance(entry.get("path"), str) or not isinstance(entry.get("sha256"), str):
            raise ConfigError(f"{identity_path}: invalid file identity entry")
        recorded[entry["path"]] = entry["sha256"].lower()
        if entry.get("generated") is True:
            generated.add(entry["path"])
    expected = expected_control_plane(repo_root)
    missing: list[str] = []
    modified: list[str] = []
    stale: list[str] = []
    for relative in sorted(set(expected) | set(recorded)):
        source_hash = expected.get(relative)
        recorded_hash = recorded.get(relative)
        installed = install_root / relative
        if source_hash != recorded_hash and relative not in generated:
            stale.append(relative)
        if not installed.is_file():
            if relative in expected or relative in recorded:
                missing.append(relative)
            continue
        actual_hash = sha256(installed)
        if actual_hash != recorded_hash and actual_hash != source_hash:
            modified.append(relative)
    return {
        "exact_match": not (missing or modified or stale),
        "source_commit": identity.get("source_commit"),
        "missing": missing,
        "modified": modified,
        "stale": stale,
        "identity_path": str(identity_path),
    }
