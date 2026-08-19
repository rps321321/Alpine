from __future__ import annotations

import hashlib
import json
import time
import urllib.request
from typing import Any


def stream_completion(
    base_url: str,
    api_key: str,
    workload: dict[str, Any],
    timeout: int = 900,
) -> dict[str, Any]:
    payload = {
        "prompt": workload["prompt"],
        "n_predict": int(workload["n_predict"]),
        "temperature": 0.0,
        "top_k": 1,
        "seed": 42,
        "ignore_eos": bool(workload.get("ignore_eos", True)),
        "cache_prompt": False,
        "stream": True,
    }
    request = urllib.request.Request(
        f"{base_url}/completion",
        data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
        method="POST",
    )
    started = time.perf_counter()
    first_content_at: float | None = None
    content: list[str] = []
    final: dict[str, Any] | None = None
    with urllib.request.urlopen(request, timeout=timeout) as response:
        for raw in response:
            line = raw.decode("utf-8", errors="replace").strip()
            if not line.startswith("data: "):
                continue
            event = json.loads(line[6:])
            chunk = event.get("content", "")
            if chunk:
                if first_content_at is None:
                    first_content_at = time.perf_counter()
                content.append(chunk)
            if event.get("stop"):
                final = event
                break
    finished = time.perf_counter()
    if final is None:
        raise RuntimeError("Streaming completion ended without a final timing event")
    text = "".join(content)
    timings = final.get("timings", {})
    return {
        "content": text,
        "output_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
        "prompt_tokens": final.get("tokens_evaluated", timings.get("prompt_n")),
        "generated_tokens": final.get("tokens_predicted", timings.get("predicted_n")),
        "prefill_tps": timings.get("prompt_per_second"),
        "decode_tps": timings.get("predicted_per_second"),
        "drafted_tokens": timings.get("draft_n"),
        "accepted_tokens": timings.get("draft_n_accepted"),
        "ttft_ms": None if first_content_at is None else (first_content_at - started) * 1000,
        "latency_ms": (finished - started) * 1000,
        "truncated": final.get("truncated"),
        "stop_type": final.get("stop_type"),
    }
