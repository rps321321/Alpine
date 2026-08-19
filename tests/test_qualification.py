from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from localmodel.config import sha256
from localmodel.qualification import qualify_run
from localmodel.store import ResultStore


IDENTITY = {
    "profile": "turbo-16k",
    "model_sha256": "model-1",
    "backend_commit": "backend-1",
    "profile_sha256": "profile-1",
}


def summary() -> dict[str, object]:
    return {
        "all_quality_pass": True,
        "all_deterministic": True,
        "workloads": {
            "prefill-4k": {"decode_tps": {"n": 5, "mean": 0.0}, "prefill_tps": {"n": 5, "mean": 700.0, "stdev": 1.0}},
            "novel-256": {"decode_tps": {"n": 5, "mean": 7.0, "stdev": 0.1}},
            "repeat-code-256": {"decode_tps": {"n": 5, "mean": 20.0, "stdev": 0.2}},
            "structured-json-128": {"decode_tps": {"n": 5, "mean": 8.0, "stdev": 0.1}},
        },
    }


def create_run(store: ResultStore, run_id: str, kind: str, *, identity: dict[str, str] = IDENTITY) -> None:
    benchmark_name = {"micro": "micro", "context": "context-needle", "agent": "golden-agent"}[kind]
    store.create_run({
        "id": run_id,
        "started_at": f"2026-08-19T00:00:0{len(run_id)}+00:00",
        "status": "running",
        "kind": kind,
        "profile": identity["profile"],
        "model_sha256": identity["model_sha256"],
        "backend_commit": identity["backend_commit"],
        "config": {
            "launch": {"profile_sha256": identity["profile_sha256"]},
            "benchmark": {"name": benchmark_name, "schema": 1, "sha256": f"{kind}-suite"},
        },
    })
    result_summary = summary() if kind == "micro" else ({"all_quality_pass": True} if kind == "context" else {"success": True})
    store.finish_run(run_id, "done", "passed", result_summary)


class CompleteQualificationTests(unittest.TestCase):
    def test_production_inherits_candidate_and_validated_gates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = ResultStore(Path(directory) / "results.sqlite3")
            try:
                create_run(store, "micro", "micro")
                result = qualify_run(store, "micro", "production")
                self.assertTrue(result["automated_pass"])
                self.assertEqual(
                    set(result["missing_external_evidence"]),
                    {
                        "same-process-50-request-greedy-stability",
                        "ten-clean-restart-greedy-stability",
                        "near-limit-context-stress",
                        "golden-agent-task-pass",
                        "operator-reviewed-capability-report",
                        "rollback-profile-available",
                    },
                )
            finally:
                store.close()

    def test_production_can_be_complete_with_exact_benchmark_and_human_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = ResultStore(root / "results.sqlite3")
            try:
                create_run(store, "micro", "micro")
                create_run(store, "context", "context")
                create_run(store, "agent", "agent")
                attached = (
                    "same-process-50-request-greedy-stability",
                    "ten-clean-restart-greedy-stability",
                    "operator-reviewed-capability-report",
                    "rollback-profile-available",
                )
                for kind in attached:
                    path = root / f"{kind}.json"
                    payload = {"kind": kind, "identity": IDENTITY, "decision": "pass"}
                    if kind == "operator-reviewed-capability-report":
                        payload["reviewed_by"] = "operator"
                    path.write_text(json.dumps(payload), encoding="utf-8")
                    store.add_artifact("micro", kind, str(path), sha256(path))

                result = qualify_run(store, "micro", "production")

                self.assertTrue(result["promotion_ready"])
                self.assertEqual({item["status"] for item in result["evidence"]}, {"satisfied"})

                capability = root / "operator-reviewed-capability-report.json"
                capability.write_text("changed after review", encoding="utf-8")
                stale = qualify_run(store, "micro", "production")
                item = next(entry for entry in stale["evidence"] if entry["name"] == "operator-reviewed-capability-report")
                self.assertEqual(item["status"], "stale")
                self.assertFalse(stale["promotion_ready"])
            finally:
                store.close()

    def test_wrong_runtime_context_evidence_is_reported_as_identity_mismatched(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = ResultStore(root / "results.sqlite3")
            try:
                create_run(store, "micro", "micro")
                mismatched = {**IDENTITY, "backend_commit": "other-backend"}
                create_run(store, "context", "context", identity=mismatched)
                result = qualify_run(store, "micro", "validated")
                evidence = next(item for item in result["evidence"] if item["name"] == "near-limit-context-stress")
                self.assertEqual(evidence["status"], "identity-mismatched")
            finally:
                store.close()

    def test_capability_artifact_without_explicit_human_reviewer_is_not_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            store = ResultStore(root / "results.sqlite3")
            try:
                create_run(store, "micro", "micro")
                path = root / "operator-reviewed-capability-report.json"
                path.write_text(
                    json.dumps(
                        {
                            "kind": "operator-reviewed-capability-report",
                            "identity": IDENTITY,
                            "decision": "pass",
                        }
                    ),
                    encoding="utf-8",
                )
                store.add_artifact("micro", "operator-reviewed-capability-report", str(path), sha256(path))
                result = qualify_run(store, "micro", "production")
                item = next(
                    entry for entry in result["evidence"]
                    if entry["name"] == "operator-reviewed-capability-report"
                )
                self.assertEqual(item["status"], "missing")
                self.assertIn("human reviewer", item["reason"])
                self.assertFalse(result["promotion_ready"])
            finally:
                store.close()


if __name__ == "__main__":
    unittest.main()
