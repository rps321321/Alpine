from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, artifact_manifest, git_commit, install_profile, install_session, powershell, read_json, tree_sha256
from .stats import describe
from .store import ResultStore
from .telemetry import GpuTelemetry, process_memory


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def http_ok(url: str) -> bool:
    try:
        with urllib.request.urlopen(url, timeout=3) as response:
            return 200 <= response.status < 300
    except (OSError, urllib.error.URLError):
        return False


def listener_process_path(port: int) -> str | None:
    command = (
        f"$c=Get-NetTCPConnection -LocalPort {port} -State Listen -ErrorAction SilentlyContinue | "
        "Select-Object -First 1; if($c){(Get-Process -Id $c.OwningProcess -ErrorAction Stop).Path}"
    )
    result = subprocess.run(
        [powershell(), "-NoProfile", "-Command", command], capture_output=True, text=True, check=False
    )
    return result.stdout.strip() or None


def run_powershell(script: Path, *arguments: str, timeout: int = 900) -> subprocess.CompletedProcess[str]:
    command = [powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script), *arguments]
    # A detached Windows child can retain inherited pipe handles and prevent
    # subprocess.run(capture_output=True) from observing EOF. Real files keep
    # the launcher observable without coupling its lifetime to llama-server.
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as stdout, tempfile.TemporaryFile(mode="w+", encoding="utf-8") as stderr:
        result = subprocess.run(command, stdout=stdout, stderr=stderr, text=True, timeout=timeout, check=False)
        stdout.seek(0)
        stderr.seek(0)
        return subprocess.CompletedProcess(command, result.returncode, stdout.read(), stderr.read())


def load_workloads(selected: list[str] | None = None) -> list[dict[str, Any]]:
    root = REPO_ROOT / "benchmarks" / "micro"
    suite = read_json(root / "workloads.json")
    workloads: list[dict[str, Any]] = []
    for definition in suite["workloads"]:
        if selected and definition["id"] not in selected:
            continue
        prompt = (root / definition["prompt_file"]).read_text(encoding="utf-8")
        item = dict(definition)
        item["prompt"] = prompt * int(definition.get("repeat", 1))
        workloads.append(item)
    if not workloads:
        raise ValueError("No microbenchmark workloads matched the selection")
    return workloads


def suite_identity() -> dict[str, Any]:
    root = REPO_ROOT / "benchmarks" / "micro"
    definition = read_json(root / "workloads.json")
    files = [root / "workloads.json"] + [root / item["prompt_file"] for item in definition["workloads"]]
    return {
        "name": "micro",
        "schema": int(definition["schema"]),
        "sha256": tree_sha256(root, files),
        "files": [path.relative_to(root).as_posix() for path in files],
    }


def quality_pass(content: str, mode: str) -> bool:
    if mode == "nonempty":
        return bool(content.strip())
    if mode == "json":
        try:
            value = json.loads(content)
            return (
                isinstance(value, dict)
                and set(value) == {"safe", "files", "reason"}
                and isinstance(value["safe"], bool)
                and isinstance(value["files"], list)
                and len(value["files"]) == 2
                and all(isinstance(item, str) for item in value["files"])
                and isinstance(value["reason"], str)
            )
        except json.JSONDecodeError:
            return False
    raise ValueError(f"Unknown quality check: {mode}")


def stream_completion(base_url: str, api_key: str, workload: dict[str, Any], timeout: int = 900) -> dict[str, Any]:
    payload = {
        "prompt": workload["prompt"], "n_predict": int(workload["n_predict"]),
        "temperature": 0.0, "top_k": 1, "seed": 42,
        "ignore_eos": bool(workload.get("ignore_eos", True)),
        "cache_prompt": False, "stream": True,
    }
    request = urllib.request.Request(
        f"{base_url}/completion",
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
        method="POST",
    )
    started = time.perf_counter()
    first_content_at: float | None = None
    content: list[str] = []
    final: dict[str, Any] | None = None
    with urllib.request.urlopen(request, timeout=timeout) as response:
        for raw in response:
            line = raw.decode("utf-8", errors="replace").strip()
            if not line.startswith("data: "):
                continue
            event = json.loads(line[6:])
            chunk = event.get("content", "")
            if chunk:
                if first_content_at is None:
                    first_content_at = time.perf_counter()
                content.append(chunk)
            if event.get("stop"):
                final = event
                break
    finished = time.perf_counter()
    if final is None:
        raise RuntimeError("Streaming completion ended without a final timing event")
    text = "".join(content)
    timings = final.get("timings", {})
    return {
        "content": text,
        "output_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
        "prompt_tokens": final.get("tokens_evaluated", timings.get("prompt_n")),
        "generated_tokens": final.get("tokens_predicted", timings.get("predicted_n")),
        "prefill_tps": timings.get("prompt_per_second"),
        "decode_tps": timings.get("predicted_per_second"),
        "drafted_tokens": timings.get("draft_n"),
        "accepted_tokens": timings.get("draft_n_accepted"),
        "ttft_ms": None if first_content_at is None else (first_content_at - started) * 1000,
        "latency_ms": (finished - started) * 1000,
        "truncated": final.get("truncated"),
        "stop_type": final.get("stop_type"),
    }


def summarize(samples: list[dict[str, Any]]) -> dict[str, Any]:
    measured = [sample for sample in samples if not sample["warmup"]]
    result: dict[str, Any] = {"workloads": {}}
    for workload in sorted({sample["workload"] for sample in measured}):
        rows = [sample for sample in measured if sample["workload"] == workload]
        hashes = sorted({sample["output_sha256"] for sample in rows})
        result["workloads"][workload] = {
            "decode_tps": describe(sample["decode_tps"] for sample in rows),
            "prefill_tps": describe(sample["prefill_tps"] for sample in rows),
            "ttft_ms": describe(sample["ttft_ms"] for sample in rows),
            "latency_ms": describe(sample["latency_ms"] for sample in rows),
            "vram_peak_mib": describe(sample["telemetry"].get("vram_peak_mib") for sample in rows),
            "gpu_util_mean": describe(sample["telemetry"].get("gpu_util_mean") for sample in rows),
            "gpu_memory_util_mean": describe(sample["telemetry"].get("gpu_memory_util_mean") for sample in rows),
            "gpu_power_mean_w": describe(sample["telemetry"].get("gpu_power_mean_w") for sample in rows),
            "process_private_mib": describe(sample.get("process_private_mib") for sample in rows),
            "draft_acceptance_rate": describe(
                (sample["accepted_tokens"] / sample["drafted_tokens"])
                if sample.get("drafted_tokens") else None for sample in rows
            ),
            "quality_pass_rate": sum(bool(sample["quality_pass"]) for sample in rows) / len(rows),
            "unique_output_hashes": hashes,
            "deterministic": len(hashes) == 1,
        }
    result["all_quality_pass"] = all(sample["quality_pass"] for sample in measured)
    result["all_deterministic"] = all(item["deterministic"] for item in result["workloads"].values())
    return result


def run_microbenchmark(
    install_root: Path,
    profile_name: str,
    runs: int = 10,
    warmups: int = 1,
    selected_workloads: list[str] | None = None,
    keep_server: bool = False,
    notes: str | None = None,
) -> tuple[str, dict[str, Any]]:
    install_root = install_root.resolve()
    session = install_session(install_root)
    profile = install_profile(install_root, profile_name)
    artifacts = artifact_manifest()
    suite = suite_identity()
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    result_root = REPO_ROOT / "results"
    raw_dir = result_root / "runs" / run_id
    raw_dir.mkdir(parents=True, exist_ok=False)
    store = ResultStore(result_root / "results.sqlite3")
    hardware_files = sorted((REPO_ROOT / "inventory").glob("hardware-*.json")) + sorted((REPO_ROOT / "inventory").glob("hardware-*.json"))
    record = {
        "id": run_id, "started_at": utc_now(), "status": "running", "kind": "micro",
        "profile": profile_name, "git_commit": git_commit(),
        "hardware_manifest": str(hardware_files[-1].relative_to(REPO_ROOT)) if hardware_files else None,
        "model_sha256": artifacts["model"]["sha256"], "backend_commit": artifacts["llama_cpp"]["commit"],
        "config": {
            "profile": profile,
            "benchmark": {
                **suite,
                "workloads": selected_workloads or "all",
                "warmups": warmups,
                "runs": runs,
                "sampler": {"temperature": 0.0, "top_k": 1, "seed": 42},
                "default_ignore_eos": True,
                "cache_prompt": False,
            },
        },
        "notes": notes,
    }
    store.create_run(record)
    (raw_dir / "run.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
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
                raise RuntimeError(f"Port {session['port']} is healthy but owned by another runtime: {listener}")
            state = read_json(Path(session["state_file"]))
            if state.get("profile") != profile_name:
                raise RuntimeError(f"Running profile is {state.get('profile')}, requested {profile_name}")
        else:
            result = run_powershell(install_root / "scripts" / "start-session.ps1", "-Profile", profile_name)
            (raw_dir / "start.stdout.log").write_text(result.stdout, encoding="utf-8")
            (raw_dir / "start.stderr.log").write_text(result.stderr, encoding="utf-8")
            if result.returncode != 0:
                raise RuntimeError(f"Server start failed: {result.stderr or result.stdout}")
            started_server = True

        api_key = Path(session["api_key_file"]).read_text(encoding="utf-8-sig").strip()
        state = read_json(Path(session["state_file"]))
        record["config"]["launch"] = {
            "runtime": state.get("runtime"),
            "server": state.get("server"),
            "profile_sha256": state.get("profile_sha256"),
            "arguments": state.get("arguments"),
            "environment": state.get("environment"),
            "fallback": state.get("fallback"),
        }
        runtime_dir = Path(state["server"]).parent
        build_manifest = runtime_dir / "build-manifest.json"
        if build_manifest.is_file():
            record["config"]["runtime_build"] = read_json(build_manifest)
        (raw_dir / "run.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
        store.update_config(run_id, record["config"])
        pid = int(state["pid"])
        workloads = load_workloads(selected_workloads)
        jsonl_path = raw_dir / "samples.jsonl"
        with jsonl_path.open("w", encoding="utf-8") as jsonl:
            for workload in workloads:
                total = warmups + runs
                for offset in range(total):
                    is_warmup = offset < warmups
                    iteration = offset + 1 if is_warmup else offset - warmups + 1
                    telemetry = GpuTelemetry()
                    telemetry.start()
                    try:
                        sample = stream_completion(f"http://{session['host']}:{session['port']}", api_key, workload)
                    finally:
                        sample_telemetry = telemetry.stop()
                    memory = process_memory(pid)
                    sample.update({
                        "workload": workload["id"], "iteration": iteration, "warmup": is_warmup,
                        "quality_pass": quality_pass(sample["content"], workload["quality"]),
                        "telemetry": sample_telemetry,
                        "process_working_set_mib": memory["working_set_mib"],
                        "process_private_mib": memory["private_mib"],
                        "process_page_faults": memory["page_faults"],
                    })
                    samples.append(sample)
                    store.add_sample(run_id, sample)
                    jsonl.write(json.dumps(sample, sort_keys=True) + "\n")
                    jsonl.flush()
                    phase = "warmup" if is_warmup else "run"
                    print(f"{workload['id']} {phase} {iteration}: decode={sample['decode_tps']:.3f} tok/s ttft={sample['ttft_ms']:.1f} ms pass={sample['quality_pass']}", flush=True)

        summary = summarize(samples)
        (raw_dir / "summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
        for name in ("session-out.log", "session-err.log"):
            source = install_root / "logs" / name
            if source.exists():
                shutil.copy2(source, raw_dir / name)
        store.finish_run(run_id, utc_now(), "passed" if summary["all_quality_pass"] else "failed-quality", summary)
        return run_id, summary
    except BaseException as exc:
        failure = {"error": type(exc).__name__, "message": str(exc)}
        (raw_dir / "failure.json").write_text(json.dumps(failure, indent=2) + "\n", encoding="utf-8")
        store.finish_run(run_id, utc_now(), "error", failure)
        raise
    finally:
        if started_server and not keep_server:
            result = run_powershell(install_root / "scripts" / "stop-session.ps1")
            (raw_dir / "stop.stdout.log").write_text(result.stdout, encoding="utf-8")
            (raw_dir / "stop.stderr.log").write_text(result.stderr, encoding="utf-8")
        store.close()
