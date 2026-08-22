from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import tempfile
import threading
import unittest
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from pathlib import Path
from unittest.mock import patch

from localmodel.lifecycle import (
    ActiveRunError,
    AlpineSessionAdapter,
    BenchmarkLifecycle,
    FileLease,
    LeaseBusyError,
    reconcile_run,
    write_json,
)
from localmodel.store import ResultStore


class FakeSessionAdapter:
    def __init__(self) -> None:
        self.acquired = 0
        self.released = 0

    def acquire(self, profile: str) -> dict[str, object]:
        self.acquired += 1
        return {
            "profile": profile,
            "runtime": "fixture",
            "server": "fixture-server",
            "profile_sha256": "profile-hash",
            "session_identity": "session-1",
            "changed": True,
        }

    def release(self, acquisition: dict[str, object], keep_server: bool = False) -> None:
        self.released += 1


class FailingSessionAdapter(FakeSessionAdapter):
    def acquire(self, profile: str) -> dict[str, object]:
        raise RuntimeError("startup failed")


class FailingReleaseAdapter(FakeSessionAdapter):
    def release(self, acquisition: dict[str, object], keep_server: bool = False) -> None:
        self.released += 1
        raise RuntimeError("restore failed")


def record(run_id: str, kind: str = "micro") -> dict[str, object]:
    return {
        "id": run_id,
        "started_at": "2026-08-19T00:00:00+00:00",
        "status": "running",
        "kind": kind,
        "profile": "stable-16k",
        "config": {"benchmark": {"name": "fixture"}},
    }


def sample(iteration: int) -> dict[str, object]:
    return {
        "workload": "fixture",
        "iteration": iteration,
        "warmup": False,
        "prompt_tokens": 1,
        "generated_tokens": 1,
        "prefill_tps": 1.0,
        "decode_tps": 1.0,
        "ttft_ms": 1.0,
        "latency_ms": 1.0,
        "output_sha256": f"hash-{iteration}",
        "quality_pass": True,
        "telemetry": {},
    }


class BenchmarkLifecycleTests(unittest.TestCase):
    def test_alpine_adapter_uses_file_backed_output_instead_of_pipes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "alpine.exe").write_bytes(b"fixture")
            adapter = AlpineSessionAdapter(root)

            def complete(command: list[str], **options: object) -> subprocess.CompletedProcess[str]:
                self.assertNotEqual(options.get("stdout"), subprocess.PIPE)
                self.assertNotEqual(options.get("stderr"), subprocess.PIPE)
                self.assertFalse(options.get("capture_output", False))
                stdout = options["stdout"]
                assert hasattr(stdout, "write")
                stdout.write('{"ok":true}\n')
                stdout.flush()
                return subprocess.CompletedProcess(command, 0)

            with patch("localmodel.lifecycle.subprocess.run", side_effect=complete):
                result = adapter._invoke_json(["session", "status", "--compact"])

            self.assertEqual(result, {"ok": True})

    def test_workload_modules_use_only_shared_operational_seams(self) -> None:
        package = Path(__file__).resolve().parents[1] / "localmodel"
        for name in ("microbench.py", "contextbench.py"):
            source = (package / name).read_text(encoding="utf-8")
            self.assertIn("BenchmarkLifecycle", source, name)
            self.assertNotIn("ResultStore(", source, name)
            self.assertNotIn("Start-InferenceSession", source, name)
            self.assertNotIn("Stop-InferenceSession", source, name)
        self.assertNotIn("from .microbench import", (package / "contextbench.py").read_text(encoding="utf-8"))

    def test_success_owns_complete_evidence_and_releases_session(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            adapter = FakeSessionAdapter()
            with BenchmarkLifecycle(result_root, record("run-ok"), adapter) as lifecycle:
                lifecycle.record_sample(sample(1))
                lease_owner = json.loads(lifecycle.inference_lease.metadata_path.read_text(encoding="utf-8"))
                self.assertEqual(lease_owner["session_identity"], "session-1")
                lifecycle.complete({"all_quality_pass": True}, "passed")

            self.assertEqual((adapter.acquired, adapter.released), (1, 1))
            self.assertTrue((result_root / "runs" / "run-ok" / "run.json").is_file())
            self.assertTrue((result_root / "runs" / "run-ok" / "summary.json").is_file())
            store = ResultStore(result_root / "results.sqlite3")
            try:
                row = store.run("run-ok")
                self.assertEqual(row["status"], "passed")
                self.assertEqual(store.sample_count("run-ok"), 1)
            finally:
                store.close()

    def test_failure_records_error_and_releases_session(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            adapter = FakeSessionAdapter()
            with self.assertRaisesRegex(RuntimeError, "boom"):
                with BenchmarkLifecycle(result_root, record("run-error"), adapter) as lifecycle:
                    lifecycle.record_sample(sample(1))
                    raise RuntimeError("boom")

            failure = json.loads((result_root / "runs" / "run-error" / "failure.json").read_text(encoding="utf-8"))
            self.assertEqual(failure["error"], "RuntimeError")
            store = ResultStore(result_root / "results.sqlite3")
            try:
                self.assertEqual(store.run("run-error")["status"], "error")
                self.assertEqual(store.sample_count("run-error"), 1)
            finally:
                store.close()
            self.assertEqual(adapter.released, 1)

    def test_quality_failure_and_interruption_finalize_without_leaking_session(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            quality_adapter = FakeSessionAdapter()
            with BenchmarkLifecycle(result_root, record("run-quality"), quality_adapter) as lifecycle:
                failing = sample(1)
                failing["quality_pass"] = False
                lifecycle.record_sample(failing)
                lifecycle.complete({"all_quality_pass": False}, "failed-quality")

            interrupt_adapter = FakeSessionAdapter()
            with self.assertRaises(KeyboardInterrupt):
                with BenchmarkLifecycle(result_root, record("run-interrupt"), interrupt_adapter):
                    raise KeyboardInterrupt()

            store = ResultStore(result_root / "results.sqlite3")
            try:
                self.assertEqual(store.run("run-quality")["status"], "failed-quality")
                self.assertEqual(store.run("run-interrupt")["status"], "interrupted")
            finally:
                store.close()
            self.assertEqual(quality_adapter.released, 1)
            self.assertEqual(interrupt_adapter.released, 1)

    def test_database_finalization_failure_still_releases_session_and_both_leases(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            adapter = FakeSessionAdapter()
            lifecycle = BenchmarkLifecycle(result_root, record("finish-fails"), adapter)
            with self.assertRaisesRegex(RuntimeError, "database finish failed"):
                with lifecycle:
                    lifecycle.record_sample(sample(1))
                    lifecycle.complete({"all_quality_pass": True}, "passed")
                    assert lifecycle.store is not None
                    lifecycle.store.finish_run = lambda *_args, **_kwargs: (_ for _ in ()).throw(
                        RuntimeError("database finish failed")
                    )
            self.assertEqual(adapter.released, 1)
            FileLease(lifecycle.run_lease.path, {"kind": "probe"}).acquire().release()
            FileLease(lifecycle.inference_lease.path, {"kind": "probe"}).acquire().release()

    def test_restoration_failure_marks_run_error_and_releases_leases(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            adapter = FailingReleaseAdapter()
            lifecycle = BenchmarkLifecycle(result_root, record("restore-fails"), adapter)
            with self.assertRaisesRegex(RuntimeError, "restore failed"):
                with lifecycle:
                    lifecycle.record_sample(sample(1))
                    lifecycle.complete({"all_quality_pass": True}, "passed")
            store = ResultStore(result_root / "results.sqlite3")
            try:
                row = store.run("restore-fails")
                self.assertEqual(row["status"], "error")
                self.assertIn("BenchmarkFinalizationError", row["summary_json"])
            finally:
                store.close()
            FileLease(lifecycle.inference_lease.path, {"kind": "probe"}).acquire().release()

    def test_reconciliation_refuses_live_writer_then_repairs_abandoned_run(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            adapter = FakeSessionAdapter()
            lifecycle = BenchmarkLifecycle(result_root, record("run-live"), adapter)
            lifecycle.__enter__()
            lifecycle.record_sample(sample(1))
            try:
                with self.assertRaises(ActiveRunError):
                    reconcile_run(result_root, "run-live")
            finally:
                lifecycle.abandon_for_test()

            result = reconcile_run(result_root, "run-live")
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["summary"]["reconciled_from"]["status"], "running")
            self.assertEqual(result["summary"]["reconciled_from"]["last_lifecycle_event"]["event"], "sample-recorded")

    def test_reconciliation_ignores_only_a_torn_trailing_sample_without_duplicates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            lifecycle = BenchmarkLifecycle(result_root, record("torn-sample"), FakeSessionAdapter())
            lifecycle.__enter__()
            lifecycle.record_sample(sample(1))
            with (lifecycle.raw_dir / "samples.jsonl").open("a", encoding="utf-8") as handle:
                handle.write('{"workload":"novel-256"')
            lifecycle.abandon_for_test()

            result = reconcile_run(result_root, "torn-sample")
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["summary"]["raw_evidence"]["complete_sample_records"], 1)
            self.assertEqual(result["summary"]["raw_evidence"]["ignored_trailing_incomplete_records"], 1)
            store = ResultStore(result_root / "results.sqlite3")
            try:
                self.assertEqual(store.sample_count("torn-sample"), 1)
            finally:
                store.close()

    def test_session_acquisition_failure_is_finalized(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            with self.assertRaisesRegex(RuntimeError, "startup failed"):
                with BenchmarkLifecycle(result_root, record("run-start-failed"), FailingSessionAdapter()):
                    self.fail("unreachable")
            store = ResultStore(result_root / "results.sqlite3")
            try:
                self.assertEqual(store.run("run-start-failed")["status"], "error")
            finally:
                store.close()
            self.assertTrue((result_root / "runs" / "run-start-failed" / "failure.json").is_file())

    def test_reconciliation_repairs_context_agent_and_post_summary_interruptions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)

            context = BenchmarkLifecycle(result_root, record("context-run", "context"), FakeSessionAdapter())
            context.__enter__()
            context.record_sample(sample(1))
            context.abandon_for_test()
            context_result = reconcile_run(result_root, "context-run")
            self.assertEqual(context_result["status"], "passed")

            agent = BenchmarkLifecycle(result_root, record("agent-run", "agent"), FakeSessionAdapter())
            agent.__enter__()
            write_json(agent.raw_dir / "summary.json", {"success": True})
            (agent.raw_dir / "opencode.stdout.jsonl").write_text(
                json.dumps({"type": "step_finish", "part": {"tokens": {"total": 12}}}) + "\n",
                encoding="utf-8",
            )
            agent.abandon_for_test()
            agent_result = reconcile_run(result_root, "agent-run")
            self.assertEqual(agent_result["status"], "passed")
            self.assertEqual(agent_result["summary"]["step_count"], 1)

            summarized = BenchmarkLifecycle(result_root, record("summary-run"), FakeSessionAdapter())
            summarized.__enter__()
            write_json(summarized.raw_dir / "summary.json", {"all_quality_pass": True, "workloads": {}})
            summarized.abandon_for_test()
            summary_result = reconcile_run(result_root, "summary-run")
            self.assertEqual(summary_result["status"], "passed")

            store = ResultStore(result_root / "results.sqlite3")
            try:
                self.assertEqual(store.sample_count("context-run"), 1)
            finally:
                store.close()

    def test_simultaneous_reconcilers_serialize_or_report_active_owner(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            lifecycle = BenchmarkLifecycle(result_root, record("race-reconcile"), FakeSessionAdapter())
            lifecycle.__enter__()
            lifecycle.record_sample(sample(1))
            lifecycle.abandon_for_test()
            barrier = threading.Barrier(2)

            def reconcile() -> str:
                barrier.wait(timeout=10)
                try:
                    return reconcile_run(result_root, "race-reconcile")["status"]
                except ActiveRunError:
                    return "active-owner"

            with ThreadPoolExecutor(max_workers=2) as pool:
                results = [future.result(timeout=20) for future in (pool.submit(reconcile), pool.submit(reconcile))]
            self.assertIn("passed", results)
            self.assertTrue(set(results) <= {"passed", "active-owner"})
            store = ResultStore(result_root / "results.sqlite3")
            try:
                self.assertEqual(store.run("race-reconcile")["status"], "passed")
                self.assertEqual(store.sample_count("race-reconcile"), 1)
            finally:
                store.close()

    def test_writer_exiting_during_reconciliation_is_never_partially_finalized(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            lifecycle = BenchmarkLifecycle(result_root, record("writer-exit-race"), FakeSessionAdapter())
            lifecycle.__enter__()
            lifecycle.record_sample(sample(1))
            barrier = threading.Barrier(2)

            def inspect() -> str:
                barrier.wait(timeout=10)
                try:
                    return reconcile_run(result_root, "writer-exit-race")["status"]
                except ActiveRunError:
                    return "active-owner"
            with ThreadPoolExecutor(max_workers=1) as pool:
                inspection = pool.submit(inspect)
                barrier.wait(timeout=10)
                lifecycle.abandon_for_test()
                observed = inspection.result(timeout=20)
            self.assertIn(observed, {"active-owner", "passed"})
            final = reconcile_run(result_root, "writer-exit-race")
            self.assertEqual(final["status"], "passed")
            store = ResultStore(result_root / "results.sqlite3")
            try:
                self.assertEqual(store.sample_count("writer-exit-race"), 1)
            finally:
                store.close()

    def test_inference_contention_is_visible_and_recorded(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result_root = Path(directory)
            lease_path = result_root / "shared-inference.lease"
            first = BenchmarkLifecycle(
                result_root, record("run-first"), FakeSessionAdapter(), inference_lease_path=lease_path
            )
            first.__enter__()
            try:
                with self.assertRaises(LeaseBusyError):
                    with BenchmarkLifecycle(
                        result_root, record("run-blocked"), FakeSessionAdapter(), inference_lease_path=lease_path
                    ):
                        self.fail("unreachable")
            finally:
                first.abandon_for_test()
            store = ResultStore(result_root / "results.sqlite3")
            try:
                row = store.run("run-blocked")
                self.assertEqual(row["status"], "blocked-contention")
                self.assertTrue(json.loads(row["config_json"])["inference_lease"]["contention"])
            finally:
                store.close()


class ResultStoreConcurrencyTests(unittest.TestCase):
    def test_failed_constructor_closes_partially_opened_database(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "broken.sqlite3"
            with closing(sqlite3.connect(database)) as connection:
                connection.execute("CREATE TABLE samples (wrong TEXT)")
            with self.assertRaises(sqlite3.OperationalError):
                ResultStore(database)
            renamed = database.with_name("renamed.sqlite3")
            database.rename(renamed)
            self.assertTrue(renamed.is_file())

    def test_failed_sample_batch_rolls_back_and_never_marks_run_complete(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "results.sqlite3"
            store = ResultStore(database)
            try:
                store.create_run(record("batch-failure"))
                store.add_sample("batch-failure", sample(1))
                store.add_sample("batch-failure", sample(1))
                with self.assertRaises(sqlite3.IntegrityError):
                    store.finish_run("batch-failure", "done", "passed", {"all_quality_pass": True})
                self.assertEqual(store.run("batch-failure")["status"], "running")
                self.assertEqual(store.sample_count("batch-failure"), 0)
            finally:
                store.close()

    def test_first_use_is_safe_under_true_concurrent_initialization(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "results.sqlite3"
            barrier = threading.Barrier(8)

            def initialize(index: int) -> None:
                barrier.wait(timeout=10)
                store = ResultStore(database)
                try:
                    run_id = f"concurrent-{index}"
                    store.create_run(record(run_id))
                    store.add_sample(run_id, sample(index + 1))
                    store.finish_run(run_id, "done", "passed", {"all_quality_pass": True})
                finally:
                    store.close()

            with ThreadPoolExecutor(max_workers=8) as pool:
                futures = [pool.submit(initialize, index) for index in range(8)]
                for future in futures:
                    future.result(timeout=40)

            observer = ResultStore(database)
            try:
                ids = {row["id"] for row in observer.runs()}
                self.assertEqual(ids, {f"concurrent-{index}" for index in range(8)})
                self.assertEqual(sum(observer.sample_count(run_id) for run_id in ids), 8)
            finally:
                observer.close()

    def test_initialization_is_safe_and_samples_commit_as_one_batch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "results.sqlite3"
            stores = [ResultStore(database) for _ in range(4)]
            try:
                stores[0].create_run(record("batched"))
                stores[0].add_sample("batched", sample(1))
                with closing(sqlite3.connect(database)) as observer:
                    count = observer.execute("SELECT COUNT(*) FROM samples WHERE run_id='batched'").fetchone()[0]
                self.assertEqual(count, 0)
                stores[0].finish_run("batched", "done", "passed", {"ok": True})
                self.assertEqual(stores[1].sample_count("batched"), 1)
            finally:
                for store in stores:
                    store.close()

if __name__ == "__main__":
    unittest.main()
