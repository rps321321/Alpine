from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from localmodel.config import tree_sha256
from localmodel.contextbench import NEEDLES, make_prompt
from localmodel.qualification import qualify_summary
from localmodel.report import latest_profile_rows
from localmodel.stats import describe, percentile
from localmodel.store import ResultStore


class StatisticsTests(unittest.TestCase):
    def test_percentile_interpolates(self) -> None:
        self.assertEqual(percentile([1, 2, 3, 4], 0.5), 2.5)
        self.assertEqual(percentile([1, 2, 3, 4], 0.95), 3.8499999999999996)

    def test_describe_ignores_none(self) -> None:
        result = describe([None, 1, 2, 3])
        self.assertEqual(result["n"], 3)
        self.assertEqual(result["median"], 2.0)


class IdentityTests(unittest.TestCase):
    def test_tree_hash_includes_path_and_content(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "a.txt"
            second = root / "b.txt"
            first.write_text("same", encoding="utf-8")
            second.write_text("same", encoding="utf-8")
            self.assertNotEqual(tree_sha256(root, [first]), tree_sha256(root, [second]))
            original = tree_sha256(root, [first, second])
            second.write_text("changed", encoding="utf-8")
            self.assertNotEqual(original, tree_sha256(root, [first, second]))

    def test_context_prompt_contains_each_needle_once(self) -> None:
        prompt = make_prompt(100)
        for needle in NEEDLES:
            self.assertEqual(prompt.count(needle), 1)
        self.assertTrue(prompt.endswith("Answer:"))


class StoreTests(unittest.TestCase):
    def test_round_trip_and_config_update(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = ResultStore(Path(directory) / "results.sqlite3")
            try:
                store.create_run({
                    "id": "run-1", "started_at": "now", "status": "running", "kind": "micro",
                    "profile": "test", "config": {"old": True},
                })
                store.update_config("run-1", {"new": True})
                row = store.run("run-1")
                self.assertIsNotNone(row)
                self.assertEqual(json.loads(row["config_json"]), {"new": True})
            finally:
                store.close()

    def test_profile_report_ignores_newer_agent_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            store = ResultStore(Path(directory) / "results.sqlite3")
            try:
                for run_id, kind, started in (("micro-1", "micro", "2026-01-01"), ("agent-1", "agent", "2026-01-02")):
                    store.create_run({
                        "id": run_id, "started_at": started, "status": "running", "kind": kind,
                        "profile": "stable-16k", "config": {},
                    })
                    summary = {"all_quality_pass": True, "all_deterministic": True, "workloads": {}}
                    if kind == "micro":
                        summary["workloads"] = {"repeat-code-256": {}, "prefill-4k": {}}
                    store.finish_run(run_id, started, "passed", summary)
                row = latest_profile_rows(store, ["stable-16k"])[0]
                self.assertEqual(row["run_id"], "micro-1")
            finally:
                store.close()


class QualificationTests(unittest.TestCase):
    def test_candidate_gate_passes_stable_five_sample_workload(self) -> None:
        summary = {
            "all_quality_pass": True,
            "all_deterministic": True,
            "workloads": {
                "prefill-4k": {"decode_tps": {"n": 5, "mean": 0.0}, "prefill_tps": {"n": 5, "mean": 700.0, "stdev": 2.0}},
                "novel-256": {"decode_tps": {"n": 5, "mean": 7.0, "stdev": 0.1}},
                "repeat-code-256": {"decode_tps": {"n": 5, "mean": 10.0, "stdev": 0.2}},
                "structured-json-128": {"decode_tps": {"n": 5, "mean": 8.0, "stdev": 0.1}},
            },
        }
        result = qualify_summary(summary, "candidate")
        self.assertTrue(result["automated_pass"])
        self.assertTrue(result["promotion_ready"])

    def test_prefill_only_workload_uses_prefill_variance(self) -> None:
        summary = {
            "all_quality_pass": True,
            "all_deterministic": True,
            "workloads": {
                "prefill-4k": {
                    "decode_tps": {"n": 5, "mean": 0.0, "stdev": 0.0},
                    "prefill_tps": {"n": 5, "mean": 700.0, "stdev": 7.0},
                },
                "novel-256": {"decode_tps": {"n": 5, "mean": 7.0, "stdev": 0.1}},
                "repeat-code-256": {"decode_tps": {"n": 5, "mean": 10.0, "stdev": 0.2}},
                "structured-json-128": {"decode_tps": {"n": 5, "mean": 8.0, "stdev": 0.1}},
            },
        }
        result = qualify_summary(summary, "candidate")
        self.assertTrue(result["automated_pass"])
        self.assertTrue(any(check["name"] == "prefill-4k:prefill-cv" for check in result["checks"]))

    def test_production_never_skips_external_evidence(self) -> None:
        summary = {
            "all_quality_pass": True,
            "all_deterministic": True,
            "workloads": {
                "prefill-4k": {"decode_tps": {"n": 5, "mean": 0.0}, "prefill_tps": {"n": 5, "mean": 700.0, "stdev": 0.0}},
                "novel-256": {"decode_tps": {"n": 5, "mean": 7.0, "stdev": 0.0}},
                "repeat-code-256": {"decode_tps": {"n": 5, "mean": 10.0, "stdev": 0.0}},
                "structured-json-128": {"decode_tps": {"n": 5, "mean": 8.0, "stdev": 0.0}},
            },
        }
        result = qualify_summary(summary, "production")
        self.assertTrue(result["automated_pass"])
        self.assertFalse(result["promotion_ready"])
        self.assertIn("golden-agent-task-pass", result["missing_external_evidence"])


if __name__ == "__main__":
    unittest.main()
