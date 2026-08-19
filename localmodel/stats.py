from __future__ import annotations

import math
import statistics
from typing import Iterable


def percentile(values: list[float], probability: float) -> float:
    if not values:
        return math.nan
    ordered = sorted(values)
    position = (len(ordered) - 1) * probability
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def describe(values: Iterable[float | int | None]) -> dict[str, float | int | None]:
    data = [float(value) for value in values if value is not None and math.isfinite(float(value))]
    if not data:
        return {"n": 0, "min": None, "max": None, "mean": None, "median": None, "stdev": None, "p50": None, "p90": None, "p95": None}
    return {
        "n": len(data),
        "min": min(data),
        "max": max(data),
        "mean": statistics.fmean(data),
        "median": statistics.median(data),
        "stdev": statistics.stdev(data) if len(data) > 1 else 0.0,
        "p50": percentile(data, 0.50),
        "p90": percentile(data, 0.90),
        "p95": percentile(data, 0.95),
    }
