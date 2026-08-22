from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from localmodel import contextbench
from localmodel.store import ResultStore
from tests.test_config import write_fixture
from tests.test_lifecycle import FakeSessionAdapter, FailingSessionAdapter


class WrongProfileAdapter(FakeSessionAdapter):
    def acquire(self, profile: str) -> dict[str, object]:
        raise RuntimeError(f"running Profile is wrong-profile, requested {profile}")


class FakeTelemetry:
    def start(self) -> None:
        return None

    def stop(self) -> dict[str, float]:
        return {}


def prepare_install(root: Path) -> None:
    write_fixture(root)
    (root / "models").mkdir()
    (root / "models" / "model.gguf").write_bytes(b"model")
    (root / "models" / "mmproj.gguf").write_bytes(b"vision")
    (root / "config" / "chat.jinja").write_text("template", encoding="utf-8")
    (root / "config" / "api-key.txt").write_text("fixture-key", encoding="utf-8")
    (root / "logs").mkdir()
    (root / "logs" / "session-state.json").write_text(json.dumps({"pid": os.getpid()}), encoding="utf-8")


def artifact_identity() -> dict[str, object]:
    return {"model": {"sha256": "model-hash"}, "llama_cpp": {"commit": "backend-commit"}}


def last_status(result_root: Path) -> str:
    store = ResultStore(result_root / "results.sqlite3")
    try:
        rows = store.runs()
        if not rows:
            raise AssertionError("expected a persisted benchmark run")
        return str(rows[0]["status"])
    finally:
        store.close()


class WorkloadLifecycleIntegrationTests(unittest.TestCase):
    def run_context(self, lab_root: Path, install: Path, adapter: FakeSessionAdapter, *, quality: bool) -> str:
        sample = {
            "content": "|".join(contextbench.NEEDLES) if quality else "wrong",
            "prompt_tokens": 100,
            "generated_tokens": 3,
            "prefill_tps": 100.0,
            "decode_tps": 10.0,
            "ttft_ms": 5.0,
            "latency_ms": 10.0,
            "output_sha256": "output",
        }
        with (
            patch.object(contextbench, "REPO_ROOT", lab_root),
            patch.object(contextbench, "AlpineSessionAdapter", return_value=adapter),
            patch.object(contextbench, "artifact_manifest", return_value=artifact_identity()),
            patch.object(contextbench, "hardware_manifest_identity", return_value={"path": "hardware.json", "sha256": "hardware"}),
            patch.object(contextbench, "git_commit", return_value="commit"),
            patch.object(contextbench, "prompt_near_tokens", return_value=("prompt", 100)),
            patch.object(contextbench, "stream_completion", return_value=sample.copy()),
            patch.object(contextbench, "GpuTelemetry", FakeTelemetry),
            patch.object(
                contextbench,
                "process_memory",
                return_value={"working_set_mib": 1.0, "private_mib": 1.0, "page_faults": 0},
            ),
        ):
            contextbench.run_contextbenchmark(install, "stable-16k", runs=1)
        return last_status(lab_root / "results")

    def test_context_success_and_workload_failure_use_shared_finalization(self) -> None:
        for quality, expected in ((True, "passed"), (False, "failed-quality")):
            with self.subTest(quality=quality), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                install, lab = root / "install", root / "lab"
                prepare_install(install)
                adapter = FakeSessionAdapter()
                self.assertEqual(self.run_context(lab, install, adapter, quality=quality), expected)
                self.assertEqual(adapter.released, 1)

    def test_context_wrong_profile_and_startup_failure_are_persisted(self) -> None:
        for adapter, message in (
            (WrongProfileAdapter(), "wrong-profile"),
            (FailingSessionAdapter(), "startup failed"),
        ):
            with self.subTest(message=message), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                install, lab = root / "install", root / "lab"
                prepare_install(install)
                with self.assertRaisesRegex(RuntimeError, message):
                    self.run_context(lab, install, adapter, quality=True)
                self.assertEqual(last_status(lab / "results"), "error")

if __name__ == "__main__":
    unittest.main()
