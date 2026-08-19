# Capability first; about 1 tok/s is the usability floor

On 12 GB + 32 GB, a denser Abliterated Model may generate at ~5 tok/s and still be the right default if it is meaningfully stronger. A faster MoE must not win on tokens/sec alone. Hard failures are OOM, severe pagefile thrash, inability to hold the needed context, tool-loop instability, crashes, or sustained generation materially below ~1 tok/s. Exact checkpoint (27B vs 35B-A3B vs smaller) stays open until abliterated artifacts are compared on those terms.
