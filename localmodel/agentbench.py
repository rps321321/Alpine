from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, artifact_manifest, git_commit, install_profile, install_session, powershell, read_json, sha256, tree_sha256
from .microbench import http_ok, listener_process_path, run_powershell, utc_now
from .store import ResultStore


def _parse_events(text: str) -> dict[str, Any]:
    events: list[dict[str, Any]] = []
    for line in text.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(event, dict) and "type" in event:
            events.append(event)
    finishes = [event for event in events if event.get("type") == "step_finish"]
    token_totals = {"new_input": 0, "output": 0, "reasoning": 0, "context_peak": 0, "cache_read_reused": 0}
    for event in finishes:
        tokens = event.get("part", {}).get("tokens", {})
        token_totals["new_input"] += int(tokens.get("input") or 0)
        token_totals["output"] += int(tokens.get("output") or 0)
        token_totals["reasoning"] += int(tokens.get("reasoning") or 0)
        token_totals["context_peak"] = max(token_totals["context_peak"], int(tokens.get("total") or 0))
        token_totals["cache_read_reused"] += int(tokens.get("cache", {}).get("read") or 0)
    tool_events = [event for event in events if event.get("type") in {"tool_use", "tool_result", "tool"}]
    return {
        "event_count": len(events),
        "step_count": len(finishes),
        "tool_event_count": len(tool_events),
        "tokens": token_totals,
        "session_ids": sorted({event.get("sessionID") for event in events if event.get("sessionID")}),
    }


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
    session = install_session(install_root)
    profile = install_profile(install_root, profile_name)
    artifacts = artifact_manifest()
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    raw_dir = REPO_ROOT / "results" / "runs" / run_id
    worktree = raw_dir / "worktree"
    raw_dir.mkdir(parents=True, exist_ok=False)
    shutil.copytree(fixture_source, worktree)
    suite_files = [path for path in task_root.rglob("*") if path.is_file()]
    protected_before = {name: sha256(worktree / name) for name in task["protected_paths"]}
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
    store = ResultStore(REPO_ROOT / "results" / "results.sqlite3")
    store.create_run(record)
    started_server = False
    try:
        health = f"http://{session['host']}:{session['port']}/health"
        configured_value = session.get("runtimes", {}).get(profile.get("runtime")) or session["llama_server"]
        configured_server = str(Path(configured_value).resolve()).casefold()
        listener = listener_process_path(int(session["port"]))
        if http_ok(health):
            if not listener or str(Path(listener).resolve()).casefold() != configured_server:
                raise RuntimeError(f"port {session['port']} belongs to another runtime: {listener}")
        else:
            started = run_powershell(install_root / "scripts" / "start-session.ps1", "-Profile", profile_name)
            (raw_dir / "start.stdout.log").write_text(started.stdout, encoding="utf-8")
            (raw_dir / "start.stderr.log").write_text(started.stderr, encoding="utf-8")
            if started.returncode:
                raise RuntimeError(f"server start failed: {started.stderr or started.stdout}")
            started_server = True

        state = read_json(Path(session["state_file"]))
        if state.get("profile") != profile_name:
            raise RuntimeError(f"running profile is {state.get('profile')}, requested {profile_name}")
        record["config"]["launch"] = {
            "runtime": state.get("runtime"), "server": state.get("server"),
            "profile_sha256": state.get("profile_sha256"), "arguments": state.get("arguments"),
            "environment": state.get("environment"), "fallback": state.get("fallback"),
        }
        store.update_config(run_id, record["config"])
        (raw_dir / "run.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")

        launcher = install_root / "scripts" / "open-local-opencode.ps1"
        started_at = time.perf_counter()
        agent = run_powershell(
            launcher, "-Profile", profile_name, "-Project", str(worktree),
            "-RunPrompt", task["prompt"], "-KeepServer", timeout=int(task["timeout_seconds"]),
        )
        agent_wall_ms = (time.perf_counter() - started_at) * 1000
        (raw_dir / "opencode.stdout.jsonl").write_text(agent.stdout, encoding="utf-8")
        (raw_dir / "opencode.stderr.log").write_text(agent.stderr, encoding="utf-8")
        parsed = _parse_events(agent.stdout)

        environment = {key: value for key, value in os.environ.items() if not any(word in key.upper() for word in ("TOKEN", "SECRET", "PASSWORD", "API_KEY", "CREDENTIAL"))}
        tests_started = time.perf_counter()
        tests = subprocess.run(
            [str(item) for item in task["test_command"]], cwd=worktree, env=environment,
            capture_output=True, text=True, timeout=120, check=False,
        )
        tests_wall_ms = (time.perf_counter() - tests_started) * 1000
        (raw_dir / "tests.stdout.log").write_text(tests.stdout, encoding="utf-8")
        (raw_dir / "tests.stderr.log").write_text(tests.stderr, encoding="utf-8")
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
        (raw_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        store.finish_run(run_id, utc_now(), "passed" if success else "failed-quality", summary)
        return run_id, summary
    except BaseException as exc:
        failure = {"error": type(exc).__name__, "message": str(exc)}
        (raw_dir / "failure.json").write_text(json.dumps(failure, indent=2) + "\n", encoding="utf-8")
        store.finish_run(run_id, utc_now(), "error", failure)
        raise
    finally:
        if started_server and not keep_server:
            stopped = run_powershell(install_root / "scripts" / "stop-session.ps1")
            (raw_dir / "stop.stdout.log").write_text(stopped.stdout, encoding="utf-8")
            (raw_dir / "stop.stderr.log").write_text(stopped.stderr, encoding="utf-8")
        store.close()
