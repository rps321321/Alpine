from __future__ import annotations

import os
import shutil
import subprocess
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, artifact_manifest, git_commit, read_json, resolve_session, sha256, tree_sha256
from .lifecycle import BenchmarkLifecycle, PowerShellSessionAdapter, parse_agent_events, run_powershell, utc_now


def run_agentbenchmark(
    install_root: Path,
    profile_name: str,
    task_id: str,
    keep_server: bool = False,
    notes: str | None = None,
) -> tuple[str, dict[str, Any]]:
    install_root = install_root.resolve()
    task_root = REPO_ROOT / "benchmarks" / "golden" / task_id
    task = read_json(task_root / "task.json")
    fixture_source = task_root / "fixture"
    if not fixture_source.is_dir():
        raise ValueError(f"fixture missing: {fixture_source}")
    resolved = resolve_session(install_root, profile_name, require_runtime=True)
    profile = resolved.profile
    artifacts = artifact_manifest()
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    result_root = REPO_ROOT / "results"
    suite_files = [path for path in task_root.rglob("*") if path.is_file()]
    record = {
        "id": run_id, "started_at": utc_now(), "status": "running", "kind": "agent",
        "profile": profile_name, "git_commit": git_commit(), "hardware_manifest": None,
        "model_sha256": artifacts["model"]["sha256"], "backend_commit": artifacts["llama_cpp"]["commit"],
        "config": {
            "profile": profile,
            "benchmark": {
                "name": "golden-agent", "schema": int(task["schema"]), "task_id": task_id,
                "suite_sha256": tree_sha256(task_root, suite_files), "timeout_seconds": int(task["timeout_seconds"]),
            },
        },
        "notes": notes,
    }
    adapter = PowerShellSessionAdapter(install_root)
    with BenchmarkLifecycle(
        result_root,
        record,
        adapter,
        keep_server=keep_server,
        inference_lease_path=install_root / "logs" / "inference.lease",
        session_log_root=install_root / "logs",
    ) as lifecycle:
        worktree = lifecycle.raw_dir / "worktree"
        shutil.copytree(fixture_source, worktree)
        protected_before = {name: sha256(worktree / name) for name in task["protected_paths"]}
        launcher = install_root / "scripts" / "open-local-opencode.ps1"
        started_at = time.perf_counter()
        agent = run_powershell(
            launcher, "-Profile", profile_name, "-Project", str(worktree),
            "-RunPrompt", task["prompt"], "-KeepServer", timeout=int(task["timeout_seconds"]),
        )
        agent_wall_ms = (time.perf_counter() - started_at) * 1000
        (lifecycle.raw_dir / "opencode.stdout.jsonl").write_text(agent.stdout, encoding="utf-8")
        (lifecycle.raw_dir / "opencode.stderr.log").write_text(agent.stderr, encoding="utf-8")
        parsed = parse_agent_events(agent.stdout)

        environment = {key: value for key, value in os.environ.items() if not any(word in key.upper() for word in ("TOKEN", "SECRET", "PASSWORD", "API_KEY", "CREDENTIAL"))}
        tests_started = time.perf_counter()
        tests = subprocess.run(
            [str(item) for item in task["test_command"]], cwd=worktree, env=environment,
            capture_output=True, text=True, timeout=120, check=False,
        )
        tests_wall_ms = (time.perf_counter() - tests_started) * 1000
        (lifecycle.raw_dir / "tests.stdout.log").write_text(tests.stdout, encoding="utf-8")
        (lifecycle.raw_dir / "tests.stderr.log").write_text(tests.stderr, encoding="utf-8")
        protected_after = {name: sha256(worktree / name) for name in task["protected_paths"]}
        protected_ok = protected_before == protected_after
        source_files = {path.relative_to(worktree).as_posix() for path in worktree.rglob("*") if path.is_file() and "__pycache__" not in path.parts}
        allowed = set(task["allowed_changed_paths"]) | set(task["protected_paths"])
        unexpected_files = sorted(source_files - allowed)
        success = agent.returncode == 0 and tests.returncode == 0 and protected_ok and not unexpected_files
        summary = {
            "task_id": task_id, "success": success, "agent_exit_code": agent.returncode,
            "tests_exit_code": tests.returncode, "protected_paths_unchanged": protected_ok,
            "unexpected_files": unexpected_files, "agent_wall_ms": agent_wall_ms,
            "tests_wall_ms": tests_wall_ms, **parsed,
        }
        lifecycle.complete(summary, "passed" if success else "failed-quality")
    return run_id, summary
