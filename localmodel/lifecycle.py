from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Protocol

from .config import powershell, read_json
from .locking import FileLease, LeaseBusyError
from .stats import describe
from .store import ResultStore


class ActiveRunError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def run_powershell(script: Path, *arguments: str, timeout: int = 900) -> subprocess.CompletedProcess[str]:
    command = [powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script), *arguments]
    with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as stdout, tempfile.TemporaryFile(
        mode="w+", encoding="utf-8"
    ) as stderr:
        result = subprocess.run(command, stdout=stdout, stderr=stderr, text=True, timeout=timeout, check=False)
        stdout.seek(0)
        stderr.seek(0)
        return subprocess.CompletedProcess(command, result.returncode, stdout.read(), stderr.read())


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.{uuid.uuid4().hex}.tmp")
    try:
        with temporary.open("w", encoding="utf-8", newline="\n") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()


class SessionAdapter(Protocol):
    def acquire(self, profile: str) -> dict[str, Any]: ...

    def release(self, acquisition: dict[str, Any], keep_server: bool = False) -> None: ...


class PowerShellSessionAdapter:
    def __init__(self, install_root: Path):
        self.install_root = install_root.resolve()
        self.module = self.install_root / "scripts" / "inference-session.ps1"

    @staticmethod
    def _quote(value: str) -> str:
        return "'" + value.replace("'", "''") + "'"

    def _invoke_json(self, expression: str) -> dict[str, Any]:
        command = f". {self._quote(str(self.module))}; {expression}"
        result = subprocess.run(
            [powershell(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", command],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode:
            raise RuntimeError((result.stderr or result.stdout).strip())
        lines = [line for line in result.stdout.splitlines() if line.strip()]
        if not lines:
            raise RuntimeError("Inference Session adapter returned no result")
        return json.loads(lines[-1])

    def acquire(self, profile: str) -> dict[str, Any]:
        selected = self._quote(profile)
        expression = (
            f"Enter-InferenceSession -InstallRoot {self._quote(str(self.install_root))} "
            f"-Profile {selected} 6>$null | ConvertTo-Json -Depth 8 -Compress"
        )
        acquisition = self._invoke_json(expression)
        if acquisition.get("profile") != profile:
            raise RuntimeError(f"running Profile is {acquisition.get('profile')}, requested {profile}")
        return acquisition

    def release(self, acquisition: dict[str, Any], keep_server: bool = False) -> None:
        if keep_server or not acquisition.get("changed"):
            return
        encoded = self._quote(json.dumps(acquisition, separators=(",", ":")))
        expression = f"""
        $acquisition = {encoded} | ConvertFrom-Json
        Exit-InferenceSession -InstallRoot {self._quote(str(self.install_root))} -Acquisition $acquisition 6>$null
        $after = Get-InferenceSessionStatus -InstallRoot {self._quote(str(self.install_root))}
        [pscustomobject]@{{ active=$after.Active; healthy=$after.Healthy; profile=$after.Profile; vision=$after.Vision }} | ConvertTo-Json -Compress
        """
        result = self._invoke_json(expression)
        prior = acquisition.get("prior") or {}
        if prior.get("active") and (
            not result.get("healthy")
            or result.get("profile") != prior.get("profile")
            or bool(result.get("vision")) != bool(prior.get("vision"))
        ):
            raise RuntimeError("pre-benchmark Inference Session restoration failed verification")


def summarize_samples(samples: list[dict[str, Any]]) -> dict[str, Any]:
    measured = [sample for sample in samples if not sample.get("warmup")]
    result: dict[str, Any] = {"workloads": {}}
    for workload in sorted({str(sample["workload"]) for sample in measured}):
        rows = [sample for sample in measured if sample["workload"] == workload]
        hashes = sorted({sample.get("output_sha256") for sample in rows if sample.get("output_sha256")})
        result["workloads"][workload] = {
            "decode_tps": describe(sample.get("decode_tps") for sample in rows),
            "prefill_tps": describe(sample.get("prefill_tps") for sample in rows),
            "ttft_ms": describe(sample.get("ttft_ms") for sample in rows),
            "latency_ms": describe(sample.get("latency_ms") for sample in rows),
            "vram_peak_mib": describe(sample.get("telemetry", {}).get("vram_peak_mib") for sample in rows),
            "gpu_util_mean": describe(sample.get("telemetry", {}).get("gpu_util_mean") for sample in rows),
            "gpu_memory_util_mean": describe(sample.get("telemetry", {}).get("gpu_memory_util_mean") for sample in rows),
            "gpu_power_mean_w": describe(sample.get("telemetry", {}).get("gpu_power_mean_w") for sample in rows),
            "process_private_mib": describe(sample.get("process_private_mib") for sample in rows),
            "draft_acceptance_rate": describe(
                sample.get("accepted_tokens") / sample["drafted_tokens"]
                if sample.get("drafted_tokens") else None for sample in rows
            ),
            "quality_pass_rate": sum(bool(sample.get("quality_pass")) for sample in rows) / len(rows),
            "unique_output_hashes": hashes,
            "deterministic": len(hashes) <= 1,
        }
    result["all_quality_pass"] = bool(measured) and all(bool(sample.get("quality_pass")) for sample in measured)
    result["all_deterministic"] = all(item["deterministic"] for item in result["workloads"].values())
    return result


def parse_agent_events(text: str) -> dict[str, Any]:
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


class BenchmarkLifecycle:
    def __init__(
        self,
        result_root: Path,
        record: dict[str, Any],
        session_adapter: SessionAdapter,
        *,
        keep_server: bool = False,
        inference_lease_path: Path | None = None,
        session_log_root: Path | None = None,
    ):
        self.result_root = result_root.resolve()
        self.record = record
        self.session_adapter = session_adapter
        self.keep_server = keep_server
        self.session_log_root = session_log_root
        self.raw_dir = self.result_root / "runs" / str(record["id"])
        self.database = self.result_root / "results.sqlite3"
        self.run_lease = FileLease(self.raw_dir / "run.lease", {"kind": "benchmark-run", "run_id": record["id"]})
        self.inference_lease = FileLease(
            inference_lease_path or self.result_root / "inference.lease",
            {"kind": "inference-capacity", "run_id": record["id"], "profile": record["profile"]},
        )
        self.store: ResultStore | None = None
        self.acquisition: dict[str, Any] | None = None
        self.completed = False
        self.summary: dict[str, Any] | None = None
        self.status: str | None = None
        self._entered = False
        self._lease_environment_prior = os.environ.get("LOCALMODEL_INFERENCE_LEASE_ID")
        self._lease_environment_had_value = "LOCALMODEL_INFERENCE_LEASE_ID" in os.environ

    def _event(self, event: str, **details: Any) -> None:
        payload = {"at": utc_now(), "event": event, **details}
        with (self.raw_dir / "lifecycle.jsonl").open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(json.dumps(payload, sort_keys=True) + "\n")
            handle.flush()

    def __enter__(self) -> "BenchmarkLifecycle":
        self.raw_dir.mkdir(parents=True, exist_ok=False)
        try:
            self.run_lease.acquire()
            self.store = ResultStore(self.database)
            self.store.create_run(self.record)
            write_json(self.raw_dir / "run.json", self.record)
            self._event("run-created", status="running")
            self.inference_lease.acquire()
            os.environ["LOCALMODEL_INFERENCE_LEASE_ID"] = str(self.inference_lease.owner["lease_id"])
            self.acquisition = self.session_adapter.acquire(str(self.record["profile"]))
            self.inference_lease.update_owner(
                session_identity=self.acquisition.get("session_identity"),
                runtime=self.acquisition.get("runtime"),
            )
            self.record.setdefault("config", {})["launch"] = {
                key: self.acquisition.get(key)
                for key in ("runtime", "server", "profile_sha256", "session_identity", "arguments", "environment", "fallback")
            }
            self.record["config"]["inference_lease"] = {
                "lease_id": self.inference_lease.owner["lease_id"],
                "session_identity": self.acquisition.get("session_identity"),
                "contention": False,
            }
            self.store.update_config(str(self.record["id"]), self.record["config"])
            write_json(self.raw_dir / "run.json", self.record)
            self._event("session-acquired", session_identity=self.acquisition.get("session_identity"))
            self._entered = True
            return self
        except BaseException as exc:
            if self.store is not None:
                contention = isinstance(exc, LeaseBusyError)
                self.record.setdefault("config", {})["inference_lease"] = {
                    "lease_id": self.inference_lease.owner["lease_id"],
                    "contention": contention,
                }
                self.store.update_config(str(self.record["id"]), self.record["config"])
                failure = {"error": type(exc).__name__, "message": str(exc), "contention": contention}
                write_json(self.raw_dir / "failure.json", failure)
                write_json(self.raw_dir / "run.json", self.record)
                status = "blocked-contention" if contention else "error"
                self.store.finish_run(str(self.record["id"]), utc_now(), status, failure)
                self._event("run-failed", status=status, error=type(exc).__name__)
            self._release_resources(release_session=True)
            raise

    def record_sample(self, sample: dict[str, Any]) -> None:
        if not self._entered or self.store is None:
            raise RuntimeError("BenchmarkLifecycle is not active")
        self.store.add_sample(str(self.record["id"]), sample)
        with (self.raw_dir / "samples.jsonl").open("a", encoding="utf-8", newline="\n") as handle:
            handle.write(json.dumps(sample, sort_keys=True) + "\n")
            handle.flush()
        self._event("sample-recorded", workload=sample.get("workload"), iteration=sample.get("iteration"), warmup=sample.get("warmup"))

    def flush_samples(self) -> None:
        if self.store is not None:
            self.store.flush_samples()

    def complete(self, summary: dict[str, Any], status: str) -> None:
        if status not in {"passed", "failed-quality"}:
            raise ValueError(f"unsupported completed benchmark status: {status}")
        self.summary = summary
        self.status = status
        self.completed = True

    def _copy_session_logs(self) -> None:
        if self.session_log_root is None:
            return
        for name in ("session-out.log", "session-err.log"):
            source = self.session_log_root / name
            if source.is_file():
                shutil.copy2(source, self.raw_dir / name)

    def __exit__(self, exc_type: Any, exc: BaseException | None, _: Any) -> bool:
        if self.store is None:
            self._release_resources(release_session=True)
            return False
        if exc is not None:
            failure = {"error": type(exc).__name__, "message": str(exc)}
            write_json(self.raw_dir / "failure.json", failure)
            self.summary = failure
            self.status = "interrupted" if isinstance(exc, KeyboardInterrupt) else "error"
            self._event("run-failed", status=self.status, error=type(exc).__name__)
        elif not self.completed or self.summary is None or self.status is None:
            failure = {"error": "IncompleteLifecycle", "message": "benchmark exited without complete()"}
            write_json(self.raw_dir / "failure.json", failure)
            self.summary, self.status = failure, "error"
            self._event("run-failed", status=self.status, error="IncompleteLifecycle")
        else:
            write_json(self.raw_dir / "summary.json", self.summary)
            self._event("run-completed", status=self.status)
        self._copy_session_logs()
        self.store.finish_run(str(self.record["id"]), utc_now(), self.status, self.summary)
        release_error: BaseException | None = None
        try:
            if self.acquisition is not None:
                self.session_adapter.release(self.acquisition, self.keep_server)
        except BaseException as caught:
            release_error = caught
            write_json(self.raw_dir / "cleanup-failure.json", {"error": type(caught).__name__, "message": str(caught)})
        finally:
            self._release_resources(release_session=False)
        if release_error is not None and exc is None:
            raise release_error
        return False

    def _release_resources(self, *, release_session: bool) -> None:
        if release_session and self.acquisition is not None:
            try:
                self.session_adapter.release(self.acquisition, self.keep_server)
            finally:
                self.acquisition = None
        if self.store is not None:
            self.store.close()
            self.store = None
        self.inference_lease.release()
        if self._lease_environment_had_value:
            os.environ["LOCALMODEL_INFERENCE_LEASE_ID"] = self._lease_environment_prior or ""
        else:
            os.environ.pop("LOCALMODEL_INFERENCE_LEASE_ID", None)
        self.run_lease.release()
        self._entered = False

    def abandon_for_test(self) -> None:
        """Simulate process death after raw evidence publication without finalization."""
        if self.acquisition is not None:
            self.session_adapter.release(self.acquisition, self.keep_server)
            self.acquisition = None
        self._release_resources(release_session=False)


def reconcile_run(result_root: Path, run_id: str) -> dict[str, Any]:
    result_root = result_root.resolve()
    raw_dir = result_root / "runs" / run_id
    if not raw_dir.is_dir():
        raise ValueError(f"raw evidence directory not found: {raw_dir}")
    lease = FileLease(raw_dir / "run.lease", {"kind": "reconciler", "run_id": run_id})
    try:
        lease.acquire()
    except LeaseBusyError as exc:
        raise ActiveRunError(f"run is still active: {run_id}") from exc
    store = ResultStore(result_root / "results.sqlite3")
    try:
        row = store.run(run_id)
        if row is None:
            raise ValueError(f"run not found: {run_id}")
        if row["status"] in {"passed", "failed-quality"} and row["summary_json"]:
            return {
                "run_id": run_id,
                "status": row["status"],
                "summary": json.loads(row["summary_json"]),
                "already_finalized": True,
            }
        prior: dict[str, Any] = {"status": row["status"]}
        failure_path = raw_dir / "failure.json"
        if failure_path.is_file():
            prior["failure"] = read_json(failure_path)
        summary_path = raw_dir / "summary.json"
        if summary_path.is_file():
            summary = read_json(summary_path)
            if row["kind"] == "agent":
                event_path = raw_dir / "opencode.stdout.jsonl"
                if event_path.is_file():
                    summary.update(parse_agent_events(event_path.read_text(encoding="utf-8")))
            summary["reconciled_from"] = prior
            quality = summary.get("success") if row["kind"] == "agent" else summary.get("all_quality_pass")
            status = "passed" if quality else "failed-quality"
        else:
            samples_path = raw_dir / "samples.jsonl"
            samples = []
            if samples_path.is_file():
                samples = [json.loads(line) for line in samples_path.read_text(encoding="utf-8").splitlines() if line.strip()]
            if samples:
                store.restore_samples(run_id, samples)
                summary = summarize_samples(samples)
                summary["reconciled_from"] = prior
                status = "passed" if summary["all_quality_pass"] else "failed-quality"
            else:
                summary = {"reconciled_from": prior, "error": "interrupted before any sample completed"}
                status = "interrupted"
        write_json(summary_path, summary)
        store.finish_run(run_id, utc_now(), status, summary)
        return {"run_id": run_id, "status": status, "summary": summary}
    finally:
        store.close()
        lease.release()
