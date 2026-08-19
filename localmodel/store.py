from __future__ import annotations

import json
import sqlite3
import time
from pathlib import Path
from typing import Any, Iterable

SCHEMA = """
CREATE TABLE IF NOT EXISTS runs (
    id TEXT PRIMARY KEY,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    status TEXT NOT NULL,
    kind TEXT NOT NULL,
    profile TEXT NOT NULL,
    git_commit TEXT,
    hardware_manifest TEXT,
    model_sha256 TEXT,
    backend_commit TEXT,
    config_json TEXT NOT NULL,
    summary_json TEXT,
    notes TEXT
);
CREATE TABLE IF NOT EXISTS samples (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    workload TEXT NOT NULL,
    iteration INTEGER NOT NULL,
    warmup INTEGER NOT NULL DEFAULT 0,
    prompt_tokens INTEGER,
    generated_tokens INTEGER,
    prefill_tps REAL,
    decode_tps REAL,
    ttft_ms REAL,
    latency_ms REAL,
    output_sha256 TEXT,
    quality_pass INTEGER,
    vram_peak_mib REAL,
    gpu_util_mean REAL,
    gpu_power_mean_w REAL,
    gpu_temp_max_c REAL,
    process_working_set_mib REAL,
    raw_json TEXT NOT NULL,
    PRIMARY KEY (run_id, workload, iteration, warmup)
);
CREATE INDEX IF NOT EXISTS samples_profile_lookup ON samples(run_id, workload, warmup);
CREATE TABLE IF NOT EXISTS artifacts (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    sha256 TEXT,
    PRIMARY KEY (run_id, kind, path)
);
"""


class ResultStore:
    def __init__(self, path: Path):
        path.parent.mkdir(parents=True, exist_ok=True)
        self.path = path
        self.connection: sqlite3.Connection | None = None
        self._pending_samples: list[tuple[Any, ...]] = []
        try:
            connection = sqlite3.connect(path, timeout=30.0)
            connection.row_factory = sqlite3.Row
            connection.execute("PRAGMA busy_timeout=30000")
            connection.execute("PRAGMA foreign_keys=ON")
            deadline = time.monotonic() + 30.0
            while True:
                try:
                    journal_mode = str(connection.execute("PRAGMA journal_mode").fetchone()[0]).lower()
                    if journal_mode != "wal":
                        connection.execute("PRAGMA journal_mode=WAL")
                    connection.executescript(SCHEMA)
                    break
                except sqlite3.OperationalError as error:
                    connection.rollback()
                    if (
                        not any(marker in str(error).lower() for marker in ("locked", "busy"))
                        or time.monotonic() >= deadline
                    ):
                        raise
                    time.sleep(0.05)
            self.connection = connection
        except BaseException:
            if self.connection is not None:
                self.connection.close()
            elif "connection" in locals():
                connection.close()
            raise

    def _connection(self) -> sqlite3.Connection:
        if self.connection is None:
            raise RuntimeError("ResultStore is closed")
        return self.connection

    def close(self) -> None:
        if self.connection is not None:
            self.connection.rollback()
            self._pending_samples.clear()
            self.connection.close()
            self.connection = None

    def create_run(self, record: dict[str, Any]) -> None:
        connection = self._connection()
        connection.execute(
            """INSERT INTO runs
            (id, started_at, status, kind, profile, git_commit, hardware_manifest,
             model_sha256, backend_commit, config_json, notes)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                record["id"], record["started_at"], record["status"], record["kind"],
                record["profile"], record.get("git_commit"), record.get("hardware_manifest"),
                record.get("model_sha256"), record.get("backend_commit"),
                json.dumps(record["config"], sort_keys=True), record.get("notes"),
            ),
        )
        connection.commit()

    def add_sample(self, run_id: str, sample: dict[str, Any]) -> None:
        self._pending_samples.append(self._sample_row(run_id, sample))

    @staticmethod
    def _sample_row(run_id: str, sample: dict[str, Any]) -> tuple[Any, ...]:
        return (
            run_id, sample["workload"], sample["iteration"], int(sample.get("warmup", False)),
            sample.get("prompt_tokens"), sample.get("generated_tokens"), sample.get("prefill_tps"),
            sample.get("decode_tps"), sample.get("ttft_ms"), sample.get("latency_ms"),
            sample.get("output_sha256"), None if sample.get("quality_pass") is None else int(sample["quality_pass"]),
            sample.get("telemetry", {}).get("vram_peak_mib"),
            sample.get("telemetry", {}).get("gpu_util_mean"),
            sample.get("telemetry", {}).get("gpu_power_mean_w"),
            sample.get("telemetry", {}).get("gpu_temp_max_c"),
            sample.get("process_working_set_mib"), json.dumps(sample, sort_keys=True),
        )

    def restore_samples(self, run_id: str, samples: list[dict[str, Any]]) -> None:
        """Idempotently restore durable rows from append-only raw evidence."""
        if not samples:
            return
        connection = self._connection()
        try:
            connection.executemany(
                """INSERT OR IGNORE INTO samples VALUES
                (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                [self._sample_row(run_id, sample) for sample in samples],
            )
            connection.commit()
        except BaseException:
            connection.rollback()
            raise

    def flush_samples(self) -> None:
        if not self._pending_samples:
            return
        connection = self._connection()
        pending = list(self._pending_samples)
        try:
            connection.executemany(
                """INSERT INTO samples VALUES
                (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                pending,
            )
            connection.commit()
        except BaseException:
            connection.rollback()
            raise
        else:
            del self._pending_samples[: len(pending)]

    def update_config(self, run_id: str, config: dict[str, Any]) -> None:
        connection = self._connection()
        connection.execute(
            "UPDATE runs SET config_json=? WHERE id=?",
            (json.dumps(config, sort_keys=True), run_id),
        )
        connection.commit()

    def finish_run(self, run_id: str, finished_at: str, status: str, summary: dict[str, Any]) -> None:
        connection = self._connection()
        pending = list(self._pending_samples)
        try:
            if pending:
                connection.executemany(
                    """INSERT INTO samples VALUES
                    (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
                    pending,
                )
            connection.execute(
                "UPDATE runs SET finished_at=?, status=?, summary_json=? WHERE id=?",
                (finished_at, status, json.dumps(summary, sort_keys=True), run_id),
            )
            connection.commit()
        except BaseException:
            connection.rollback()
            raise
        else:
            del self._pending_samples[: len(pending)]

    def runs(self, profiles: Iterable[str] | None = None) -> list[sqlite3.Row]:
        if profiles:
            names = list(profiles)
            placeholders = ",".join("?" for _ in names)
            return list(self._connection().execute(
                f"SELECT * FROM runs WHERE profile IN ({placeholders}) ORDER BY started_at DESC", names
            ))
        return list(self._connection().execute("SELECT * FROM runs ORDER BY started_at DESC"))

    def run(self, run_id: str) -> sqlite3.Row | None:
        return self._connection().execute("SELECT * FROM runs WHERE id=?", (run_id,)).fetchone()

    def sample_count(self, run_id: str) -> int:
        row = self._connection().execute("SELECT COUNT(*) FROM samples WHERE run_id=?", (run_id,)).fetchone()
        return int(row[0])

    def samples(self, run_id: str) -> list[sqlite3.Row]:
        return list(self._connection().execute(
            "SELECT * FROM samples WHERE run_id=? ORDER BY workload, warmup DESC, iteration",
            (run_id,),
        ))

    def add_artifact(self, run_id: str, kind: str, path: str, digest: str | None = None) -> None:
        connection = self._connection()
        connection.execute(
            "INSERT OR REPLACE INTO artifacts(run_id, kind, path, sha256) VALUES (?, ?, ?, ?)",
            (run_id, kind, path, digest),
        )
        connection.commit()

    def artifacts(self, run_id: str) -> list[sqlite3.Row]:
        return list(self._connection().execute(
            "SELECT * FROM artifacts WHERE run_id=? ORDER BY kind, path", (run_id,)
        ))
