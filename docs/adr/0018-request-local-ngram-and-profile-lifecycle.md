# ADR 0018: Request-local n-gram is a workload candidate

## Status

Accepted — 2026-08-19

## Decision

Maintain a minimal patch against pinned `llama.cpp` b10453 that optionally resets `ngram-mod` state at request begin. Enable it only in `turbo-16k`, `fast-32k` and explicitly experimental profiles. Keep the official MTP-only runtime as `stable-16k` production and rollback.

Profiles follow `experimental → candidate → validated → production`. Microbenchmark speed alone can reach candidate, but validated/production require same-process and restart stability, near-limit context where applicable, executable agent tasks, capability review and a rollback profile.

## Rationale

The unpatched process-wide n-gram table made identical greedy requests history-dependent. The request-local patch restored stable hashes while retaining 28–30 tok/s on medium repeated code and 61–65 tok/s on a perfect-copy probe. Novel decode was about 2% slower, and Fast-32K took 20% longer than Stable on the first executable OpenCode task. The optimization is therefore real but workload-specific.

## Consequences

The patch and build manifest are versioned and independently verifiable. Upstream quantized-MTP and recurrent rollback concerns still apply, so the candidate must fail closed to Stable on unexplained hash splits, malformed tool calls, stalls or agent regressions.
