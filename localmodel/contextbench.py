from __future__ import annotations

import gzip
import json
import shutil
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, artifact_manifest, git_commit, install_profile, install_session, read_json, sha256
from .microbench import http_ok, listener_process_path, run_powershell, stream_completion, summarize, utc_now
from .store import ResultStore
from .telemetry import GpuTelemetry, process_memory


NEEDLES = ("CEDAR-48291", "ORBIT-73064", "VIOLET-19538")


def token_count(base_url: str, api_key: str, content: str) -> int:
    request = urllib.request.Request(
        f"{base_url}/tokenize",
        data=json.dumps({"content": content, "add_special": True}, separators=(",", ":")).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        return len(json.load(response)["tokens"])


def make_prompt(lines: int) -> str:
    positions = {max(0, int(lines * ratio)): NEEDLES[index] for index, ratio in enumerate((0.10, 0.50, 0.90))}
    parts = [
        "You are checking a long immutable ledger. Remember every line marked IMPORTANT.\n",
        "Most records are filler and must not change the answer.\n",
    ]
    for index in range(lines):
        if index in positions:
            parts.append(f"IMPORTANT checkpoint {len([key for key in positions if key <= index])}: {positions[index]}\n")
        parts.append(f"Record {index:05d}: alpha beta gamma delta epsilon zeta eta theta.\n")
    parts.append(
        "\nReturn exactly the three checkpoint values in order, separated by a single vertical bar. "
        "Do not explain. Answer:"
    )
    return "".join(parts)


def prompt_near_tokens(base_url: str, api_key: str, target_tokens: int) -> tuple[str, int]:
    low, high = 1, max(32, target_tokens // 4)
    while token_count(base_url, api_key, make_prompt(high)) < target_tokens:
        low, high = high, high * 2
    best_prompt = make_prompt(low)
    best_count = token_count(base_url, api_key, best_prompt)
    while low <= high:
        middle = (low + high) // 2
        prompt = make_prompt(middle)
        count = token_count(base_url, api_key, prompt)
        if count <= target_tokens:
            best_prompt, best_count = prompt, count
            low = middle + 1
        else:
            high = middle - 1
    return best_prompt, best_count


def run_contextbenchmark(
    install_root: Path,
    profile_name: str,
    ratio: float = 0.85,
    runs: int = 3,
    warmups: int = 0,
    keep_server: bool = False,
    notes: str | None = None,
) -> tuple[str, dict[str, Any]]:
    if not 0.25 <= ratio <= 0.95:
        raise ValueError("ratio must be between 0.25 and 0.95")
    if runs < 1 or warmups < 0:
        raise ValueError("runs must be positive and warmups non-negative")
    install_root = install_root.resolve()
    session = install_session(install_root)
    profile = install_profile(install_root, profile_name)
    artifacts = artifact_manifest()
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    raw_dir = REPO_ROOT / "results" / "runs" / run_id
    raw_dir.mkdir(parents=True, exist_ok=False)
    store = ResultStore(REPO_ROOT / "results" / "results.sqlite3")
    record = {
        "id": run_id,
        "started_at": utc_now(),
        "status": "running",
        "kind": "context",
        "profile": profile_name,
        "git_commit": git_commit(),
        "hardware_manifest": None,
        "model_sha256": artifacts["model"]["sha256"],
        "backend_commit": artifacts["llama_cpp"]["commit"],
        "config": {
            "profile": profile,
            "benchmark": {
                "name": "context-needle",
                "schema": 1,
                "generator_sha256": sha256(Path(__file__)),
                "ratio": ratio,
                "runs": runs,
                "warmups": warmups,
                "needles_sha256": __import__("hashlib").sha256("|".join(NEEDLES).encode()).hexdigest(),
            },
        },
        "notes": notes,
    }
    store.create_run(record)
    started_server = False
    samples: list[dict[str, Any]] = []
    try:
        health = f"http://{session['host']}:{session['port']}/health"
        runtime_name = profile.get("runtime")
        configured_value = session.get("runtimes", {}).get(runtime_name) or session["llama_server"]
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
        api_key = Path(session["api_key_file"]).read_text(encoding="utf-8-sig").strip()
        base_url = f"http://{session['host']}:{session['port']}"
        target = int(int(profile["context"]) * ratio)
        prompt, actual_tokens = prompt_near_tokens(base_url, api_key, target)
        with gzip.open(raw_dir / "prompt.txt.gz", "wt", encoding="utf-8") as output:
            output.write(prompt)
        expected = "|".join(NEEDLES)
        workload = {"prompt": prompt, "n_predict": 64, "ignore_eos": False}
        jsonl_path = raw_dir / "samples.jsonl"
        with jsonl_path.open("w", encoding="utf-8") as jsonl:
            for offset in range(warmups + runs):
                warmup = offset < warmups
                iteration = offset + 1 if warmup else offset - warmups + 1
                telemetry = GpuTelemetry()
                telemetry.start()
                try:
                    sample = stream_completion(base_url, api_key, workload)
                finally:
                    sample_telemetry = telemetry.stop()
                memory = process_memory(int(state["pid"]))
                sample.update({
                    "workload": f"context-needle-{int(ratio * 100)}",
                    "iteration": iteration,
                    "warmup": warmup,
                    "quality_pass": sample["content"].strip() == expected,
                    "target_prompt_tokens": target,
                    "actual_prompt_tokens": actual_tokens,
                    "expected_sha256": __import__("hashlib").sha256(expected.encode()).hexdigest(),
                    "telemetry": sample_telemetry,
                    "process_working_set_mib": memory["working_set_mib"],
                    "process_private_mib": memory["private_mib"],
                    "process_page_faults": memory["page_faults"],
                })
                samples.append(sample)
                store.add_sample(run_id, sample)
                jsonl.write(json.dumps(sample, sort_keys=True) + "\n")
                prefill_text = "n/a" if sample["prefill_tps"] is None else f"{sample['prefill_tps']:.2f}"
                ttft_text = "n/a" if sample["ttft_ms"] is None else f"{sample['ttft_ms']:.1f}"
                print(
                    f"context {actual_tokens}/{profile['context']} run {iteration}: "
                    f"prefill={prefill_text} tok/s ttft={ttft_text} ms pass={sample['quality_pass']}",
                    flush=True,
                )
        summary = summarize(samples)
        summary["target_prompt_tokens"] = target
        summary["actual_prompt_tokens"] = actual_tokens
        (raw_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        for name in ("session-out.log", "session-err.log"):
            source = install_root / "logs" / name
            if source.exists():
                shutil.copy2(source, raw_dir / name)
        status = "passed" if summary["all_quality_pass"] else "failed-quality"
        store.finish_run(run_id, utc_now(), status, summary)
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
