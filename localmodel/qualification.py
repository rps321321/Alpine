from __future__ import annotations

import json
import hashlib
import math
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, read_json, sha256, tree_sha256
from .store import ResultStore


def _inherited_policy(policy: dict[str, Any], target: str) -> dict[str, Any]:
    gates = policy["gates"]
    if target not in gates:
        raise ValueError(f"unknown qualification target: {target}")
    chain: list[dict[str, Any]] = []
    current = target
    seen: set[str] = set()
    while current:
        if current in seen:
            raise ValueError("promotion policy contains an inheritance cycle")
        seen.add(current)
        gate = gates[current]
        chain.append(gate)
        current = gate.get("inherits")
    merged: dict[str, Any] = {}
    evidence: list[str] = []
    for gate in reversed(chain):
        merged.update({key: value for key, value in gate.items() if key not in {"inherits", "requires_external_evidence"}})
        evidence.extend(gate.get("requires_external_evidence", []))
    merged["requires_external_evidence"] = list(dict.fromkeys(evidence))
    return merged


def qualify_summary(summary: dict[str, Any], target: str = "candidate") -> dict[str, Any]:
    policy = read_json(REPO_ROOT / "config" / "promotion-policy.json")
    gate = _inherited_policy(policy, target)
    checks: list[dict[str, Any]] = []

    def add(name: str, passed: bool, observed: Any, required: Any) -> None:
        checks.append({"name": name, "passed": bool(passed), "observed": observed, "required": required})

    if gate.get("require_quality_pass"):
        add("quality", bool(summary.get("all_quality_pass")), summary.get("all_quality_pass"), True)
    if gate.get("require_deterministic_outputs"):
        add("determinism", bool(summary.get("all_deterministic")), summary.get("all_deterministic"), True)

    present = set(summary.get("workloads", {}))
    required_workloads = set(gate.get("required_workloads", []))
    missing_workloads = sorted(required_workloads - present)
    add("required-workloads", not missing_workloads, sorted(present), sorted(required_workloads))

    minimum = int(gate.get("minimum_measured_samples_per_workload", 0))
    max_cv = float(gate.get("maximum_decode_coefficient_of_variation", math.inf))
    for name, workload in sorted(summary.get("workloads", {}).items()):
        decode = workload.get("decode_tps", {})
        prefill = workload.get("prefill_tps", {})
        distribution = decode if (decode.get("mean") or 0) > 0 else prefill
        metric = "decode" if distribution is decode else "prefill"
        count = int(distribution.get("n") or 0)
        add(f"{name}:sample-count", count >= minimum, count, f">={minimum}")
        mean = distribution.get("mean")
        stdev = distribution.get("stdev")
        cv = None if not mean else float(stdev or 0.0) / float(mean)
        add(f"{name}:{metric}-cv", cv is not None and cv <= max_cv, cv, f"<={max_cv}")

    missing = gate.get("requires_external_evidence", [])
    return {
        "target": target,
        "automated_pass": all(check["passed"] for check in checks),
        "promotion_ready": all(check["passed"] for check in checks) and not missing,
        "checks": checks,
        "missing_external_evidence": missing,
    }


def qualify_run_row(run: Any, target: str = "candidate") -> dict[str, Any]:
    if not run:
        raise ValueError("run not found")
    if run["status"] not in {"passed", "failed-quality"} or not run["summary_json"]:
        raise ValueError(f"run is not qualifiable in status {run['status']}")
    result = qualify_summary(json.loads(run["summary_json"]), target)
    result.update({"run_id": run["id"], "profile": run["profile"], "run_status": run["status"]})
    return result


def _run_identity(run: Any) -> dict[str, Any]:
    config = json.loads(run["config_json"])
    launch = config.get("launch", {})
    return {
        "profile": run["profile"],
        "model_sha256": run["model_sha256"],
        "backend_commit": run["backend_commit"],
        "profile_sha256": launch.get("profile_sha256"),
        "runtime": launch.get("runtime"),
        "server_sha256": launch.get("server_sha256"),
        "runtime_identity": launch.get("runtime_build_sha256") or launch.get("server_sha256"),
    }


def _identity_matches(expected: dict[str, Any], observed: dict[str, Any]) -> bool:
    return all(value not in (None, "") and observed.get(key) == value for key, value in expected.items())


def _expected_benchmark_identity(kind: str, benchmark: dict[str, Any]) -> dict[str, Any]:
    if kind == "micro":
        from .microbench import suite_identity

        return suite_identity()
    if kind == "context":
        from .contextbench import NEEDLES

        return {
            "name": "context-needle",
            "schema": 1,
            "generator_sha256": sha256(REPO_ROOT / "localmodel" / "contextbench.py"),
            "needles_sha256": hashlib.sha256("|".join(NEEDLES).encode()).hexdigest(),
        }
    if kind == "agent":
        task_id = benchmark.get("task_id")
        if not isinstance(task_id, str) or not task_id:
            raise ValueError("agent benchmark identity is missing task_id")
        task_root = REPO_ROOT / "benchmarks" / "golden" / task_id
        task = read_json(task_root / "task.json")
        files = [path for path in task_root.rglob("*") if path.is_file()]
        return {
            "name": "golden-agent",
            "schema": int(task["schema"]),
            "task_id": task_id,
            "suite_sha256": tree_sha256(task_root, files),
        }
    raise ValueError(f"unsupported benchmark kind: {kind}")


def _benchmark_identity_matches(kind: str, benchmark: dict[str, Any]) -> bool:
    try:
        expected = _expected_benchmark_identity(kind, benchmark)
    except (OSError, ValueError):
        return False
    return all(benchmark.get(key) == value for key, value in expected.items())


def _benchmark_evidence(
    store: ResultStore,
    expected_identity: dict[str, Any],
    name: str,
    kind: str,
    benchmark_name: str,
) -> dict[str, Any]:
    candidates = [
        row for row in store.runs([str(expected_identity["profile"])])
        if row["kind"] == kind and row["status"] == "passed"
    ]
    relevant: list[Any] = []
    for row in candidates:
        config = json.loads(row["config_json"])
        if config.get("benchmark", {}).get("name") == benchmark_name:
            relevant.append(row)
    stale: list[Any] = []
    for row in relevant:
        if _identity_matches(expected_identity, _run_identity(row)):
            benchmark = json.loads(row["config_json"]).get("benchmark", {})
            if not _benchmark_identity_matches(kind, benchmark):
                stale.append(row)
                continue
            return {
                "name": name,
                "status": "satisfied",
                "run_id": row["id"],
                "benchmark_identity": {
                    key: benchmark.get(key) for key in ("name", "schema", "sha256", "suite_sha256") if benchmark.get(key) is not None
                },
            }
    if stale:
        return {
            "name": name,
            "status": "stale",
            "run_ids": [row["id"] for row in stale],
            "reason": "benchmark suite identity does not match the current versioned suite",
        }
    if relevant:
        return {
            "name": name,
            "status": "identity-mismatched",
            "run_ids": [row["id"] for row in relevant],
            "expected_identity": expected_identity,
        }
    return {"name": name, "status": "missing"}


def _artifact_evidence(
    artifacts: dict[str, Any],
    expected_identity: dict[str, Any],
    name: str,
) -> dict[str, Any]:
    artifact = artifacts.get(name)
    if artifact is None:
        return {"name": name, "status": "missing"}
    path = Path(artifact["path"])
    if not path.is_file() or not artifact["sha256"] or sha256(path) != artifact["sha256"]:
        return {"name": name, "status": "stale", "path": str(path)}
    try:
        payload = read_json(path)
    except (OSError, ValueError):
        return {"name": name, "status": "stale", "path": str(path)}
    if payload.get("kind") != name or payload.get("decision") != "pass":
        return {"name": name, "status": "stale", "path": str(path)}
    if not isinstance(payload.get("identity"), dict) or not _identity_matches(expected_identity, payload["identity"]):
        return {"name": name, "status": "identity-mismatched", "path": str(path)}
    if name == "operator-reviewed-capability-report" and not payload.get("reviewed_by"):
        return {"name": name, "status": "missing", "reason": "explicit human reviewer is required"}
    return {"name": name, "status": "satisfied", "path": str(path), "sha256": artifact["sha256"]}


def qualify_run(store: ResultStore, run_id: str, target: str = "candidate") -> dict[str, Any]:
    run = store.run(run_id)
    base = qualify_run_row(run, target)
    expected_identity = _run_identity(run)
    identity_complete = all(value not in (None, "") for value in expected_identity.values())
    anchor_config = json.loads(run["config_json"])
    anchor_benchmark = anchor_config.get("benchmark", {})
    benchmark_current = _benchmark_identity_matches(str(run["kind"]), anchor_benchmark)
    base["checks"].extend(
        [
            {"name": "exact-run-identity", "passed": identity_complete, "observed": expected_identity, "required": "all fields"},
            {"name": "current-benchmark-identity", "passed": benchmark_current, "observed": anchor_benchmark, "required": "current versioned suite"},
        ]
    )
    base["automated_pass"] = bool(base["automated_pass"] and identity_complete and benchmark_current)
    required = list(base["missing_external_evidence"])
    if not required:
        base["evidence"] = []
        base["promotion_ready"] = base["automated_pass"]
        base["identity"] = expected_identity
        return base
    artifacts = {row["kind"]: row for row in store.artifacts(run_id)}
    benchmark_requirements = {
        "near-limit-context-stress": ("context", "context-needle"),
        "golden-agent-task-pass": ("agent", "golden-agent"),
    }
    evidence: list[dict[str, Any]] = []
    for name in required:
        if name in benchmark_requirements:
            kind, benchmark_name = benchmark_requirements[name]
            evidence.append(_benchmark_evidence(store, expected_identity, name, kind, benchmark_name))
        else:
            evidence.append(_artifact_evidence(artifacts, expected_identity, name))
    incomplete = [item["name"] for item in evidence if item["status"] != "satisfied"]
    base.update({
        "promotion_ready": bool(base["automated_pass"] and not incomplete),
        "missing_external_evidence": incomplete,
        "evidence": evidence,
        "identity": expected_identity,
    })
    return base
