# ADR 0017: Single-slot text-fast default

## Status

Accepted — 2026-08-19

## Decision

Run the `stable-16k` Qwen text session with one server slot, MTP depth 3, Q8 KV cache, 16 CPU threads, and explicit tensor placement: all GPU layers requested, FFN matrices in blocks 0–43 overridden to CPU, and `--load-mode none`. Keep attention and DeltaNet/SSM on CUDA. Do not load the vision projector by default. Vision remains an explicit launcher option using automatic fitting with a 512 MiB reserve. Use Qwen's official chat template in place of the GGUF's added execution-bias block while preserving Qwen's tool and vision format.

## Rationale

Measured decode improved from 4.42 tok/s in the old four-slot vision configuration to about 7.5 tok/s for novel generation in the final text configuration. A corrected five-run distribution is stable and passes the structured-output checks. MTP depth 3 is the local stable performance optimum. Request-local n-gram modes are now separate candidates rather than part of this rollback profile; lower KV precision, more aggressive VRAM packing, chained-MTP patches, native-FP4 candidates, and persisted slot-cache restoration remain rejected or experimental.

This changes resource allocation, not the model's topic, coding, shell, web, or tool capability. Context remains 16,384 and model weights remain Q4_K_M.
