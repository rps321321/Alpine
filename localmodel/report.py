from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from .store import ResultStore


def _number(value: Any, digits: int = 2) -> str:
    return "—" if value is None else f"{float(value):.{digits}f}"


def latest_profile_rows(store: ResultStore, profiles: list[str]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for profile in profiles:
        candidates = [
            row for row in store.runs([profile])
            if row["kind"] == "micro" and row["status"] in ("passed", "failed-quality")
        ]
        if not candidates:
            rows.append({"profile": profile, "run_id": None})
            continue
        run = candidates[0]
        summary = json.loads(run["summary_json"])
        repeat = summary.get("workloads", {}).get("repeat-code-256", {})
        prefill = summary.get("workloads", {}).get("prefill-4k", {})
        rows.append({
            "profile": profile,
            "run_id": run["id"],
            "decode_median": repeat.get("decode_tps", {}).get("median"),
            "decode_p90": repeat.get("decode_tps", {}).get("p90"),
            "prefill_median": prefill.get("prefill_tps", {}).get("median"),
            "ttft_median": repeat.get("ttft_ms", {}).get("median"),
            "vram_median": repeat.get("vram_peak_mib", {}).get("median"),
            "quality": summary.get("all_quality_pass"),
            "deterministic": summary.get("all_deterministic"),
            "status": run["status"],
        })
    return rows


def comparison_markdown(rows: list[dict[str, Any]]) -> str:
    lines = [
        "# Local-model profile comparison", "",
        "Headline throughput is the measured median, never the single best sample.", "",
        "| Profile | Run | Repeat decode median | p90 | 4K prefill median | TTFT median | VRAM | Quality | Deterministic |",
        "|---|---|---:|---:|---:|---:|---:|:---:|:---:|",
    ]
    for row in rows:
        if not row.get("run_id"):
            lines.append(f"| {row['profile']} | — | — | — | — | — | — | — | — |")
            continue
        lines.append(
            f"| {row['profile']} | `{row['run_id']}` | {_number(row['decode_median'])} tok/s | "
            f"{_number(row['decode_p90'])} | {_number(row['prefill_median'])} tok/s | "
            f"{_number(row['ttft_median'])} ms | {_number(row['vram_median'], 0)} MiB | "
            f"{'pass' if row['quality'] else 'FAIL'} | {'yes' if row['deterministic'] else 'NO'} |"
        )
    lines += ["", "Engine performance and task capability remain separate gates; this table does not claim agent-task success.", ""]
    return "\n".join(lines)


def write_comparison(database: Path, profiles: list[str], output: Path) -> None:
    store = ResultStore(database)
    try:
        markdown = comparison_markdown(latest_profile_rows(store, profiles))
    finally:
        store.close()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(markdown, encoding="utf-8")
