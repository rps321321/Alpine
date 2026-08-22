from __future__ import annotations

import json
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .config import (
    REPO_ROOT,
    artifact_manifest,
    git_commit,
    hardware_manifest_identity,
    read_json,
    resolve_session,
    tree_sha256,
)
from .inference import stream_completion
from .lifecycle import AlpineSessionAdapter, BenchmarkLifecycle, summarize_samples, utc_now
from .telemetry import GpuTelemetry, process_memory


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
    resolved = resolve_session(install_root, profile_name, require_runtime=True)
    session = resolved.session
    profile = resolved.profile
    artifacts = artifact_manifest()
    suite = suite_identity()
    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    result_root = REPO_ROOT / "results"
    hardware = hardware_manifest_identity()
    record = {
        "id": run_id, "started_at": utc_now(), "status": "running", "kind": "micro",
        "profile": profile_name, "git_commit": git_commit(),
        "hardware_manifest": hardware["path"] if hardware else None,
        "model_sha256": artifacts["model"]["sha256"], "backend_commit": artifacts["llama_cpp"]["commit"],
        "config": {
            "hardware": hardware,
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
    build_manifest = resolved.server.parent / "build-manifest.json"
    if build_manifest.is_file():
        record["config"]["runtime_build"] = read_json(build_manifest)
    samples: list[dict[str, Any]] = []
    adapter = AlpineSessionAdapter(install_root)
    with BenchmarkLifecycle(
        result_root,
        record,
        adapter,
        keep_server=keep_server,
        inference_lease_path=install_root / "logs" / "inference.lease",
        session_log_root=install_root / "logs",
    ) as lifecycle:
        api_key = resolved.api_key_file.read_text(encoding="utf-8-sig").strip()
        state = read_json(Path(session["state_file"]))
        pid = int(state["pid"])
        workloads = load_workloads(selected_workloads)
        for workload in workloads:
            total = warmups + runs
            for offset in range(total):
                is_warmup = offset < warmups
                iteration = offset + 1 if is_warmup else offset - warmups + 1
                telemetry = GpuTelemetry()
                telemetry.start()
                try:
                    sample = stream_completion(resolved.base_url, api_key, workload)
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
                lifecycle.record_sample(sample)
                phase = "warmup" if is_warmup else "run"
                print(f"{workload['id']} {phase} {iteration}: decode={sample['decode_tps']:.3f} tok/s ttft={sample['ttft_ms']:.1f} ms pass={sample['quality_pass']}", flush=True)
            lifecycle.flush_samples()
        summary = summarize_samples(samples)
        lifecycle.complete(summary, "passed" if summary["all_quality_pass"] else "failed-quality")
    return run_id, summary
