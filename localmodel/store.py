from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from typing import Any, Iterable

SCHEMA = """
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
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
        self.connection = sqlite3.connect(path)
        self.connection.row_factory = sqlite3.Row
        self.connection.executescript(SCHEMA)

    def close(self) -> None:
        self.connection.close()

    def create_run(self, record: dict[str, Any]) -> None:
        self.connection.execute(
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
        self.connection.commit()

    def add_sample(self, run_id: str, sample: dict[str, Any]) -> None:
        self.connection.execute(
            """INSERT INTO samples VALUES
            (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            (
                run_id, sample["workload"], sample["iteration"], int(sample.get("warmup", False)),
                sample.get("prompt_tokens"), sample.get("generated_tokens"), sample.get("prefill_tps"),
                sample.get("decode_tps"), sample.get("ttft_ms"), sample.get("latency_ms"),
                sample.get("output_sha256"), None if sample.get("quality_pass") is None else int(sample["quality_pass"]),
                sample.get("telemetry", {}).get("vram_peak_mib"),
                sample.get("telemetry", {}).get("gpu_util_mean"),
                sample.get("telemetry", {}).get("gpu_power_mean_w"),
                sample.get("telemetry", {}).get("gpu_temp_max_c"),
                sample.get("process_working_set_mib"), json.dumps(sample, sort_keys=True),
            ),
        )
        self.connection.commit()

    def update_config(self, run_id: str, config: dict[str, Any]) -> None:
        self.connection.execute(
            "UPDATE runs SET config_json=? WHERE id=?",
            (json.dumps(config, sort_keys=True), run_id),
        )
        self.connection.commit()

    def finish_run(self, run_id: str, finished_at: str, status: str, summary: dict[str, Any]) -> None:
        self.connection.execute(
            "UPDATE runs SET finished_at=?, status=?, summary_json=? WHERE id=?",
            (finished_at, status, json.dumps(summary, sort_keys=True), run_id),
        )
        self.connection.commit()

    def runs(self, profiles: Iterable[str] | None = None) -> list[sqlite3.Row]:
        if profiles:
            names = list(profiles)
            placeholders = ",".join("?" for _ in names)
            return list(self.connection.execute(
                f"SELECT * FROM runs WHERE profile IN ({placeholders}) ORDER BY started_at DESC", names
            ))
        return list(self.connection.execute("SELECT * FROM runs ORDER BY started_at DESC"))

    def run(self, run_id: str) -> sqlite3.Row | None:
        return self.connection.execute("SELECT * FROM runs WHERE id=?", (run_id,)).fetchone()
