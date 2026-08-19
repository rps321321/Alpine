from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any

from .config import REPO_ROOT, read_json


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
