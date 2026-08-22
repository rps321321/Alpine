# Local Qwen performance optimization — 2026-08-19

## Follow-up: reproducible profiles and request-local n-gram

The original measurements below explain the path from 4.42 to roughly 7.5 tok/s and remain valid historical evidence. Later work in the same session added a durable benchmark system and isolated the 29.2 tok/s divergence to cross-request `ngram-mod` state. A minimal opt-in b10453 patch resets that table at request begin.

Five-run schema-2 distributions now show:

| Profile | Novel median | Repeated-copy median | 4K prefill median | Structured JSON | Status |
|---|---:|---:|---:|:---:|---|
| Stable-16K | 7.48 tok/s | 10.88 tok/s | 725.92 tok/s | 5/5 | production |
| Turbo-16K | 7.33 tok/s | 64.17 tok/s | 717.94 tok/s | 5/5 | candidate |
| Fast-32K | 7.39 tok/s | 63.98 tok/s | 722.39 tok/s | 5/5 | candidate |

The 64 tok/s number is a perfect-copy ceiling with 100% draft acceptance, not a general coding claim. On an executable OpenCode bug-fix task Stable passed in 85.5 seconds; Fast-32K also passed but took 102.8 seconds because it generated more novel text. Stable therefore remains the general default.

Fast-32K recovered three needles exactly from a 27,851-token prompt twice: 711.3 prefill tok/s median, 39.20-second TTFT, 11,796.5 MiB peak VRAM. Long-64K processed 55,705 tokens at 100.0 tok/s in 557 seconds, peaked at 11,892 MiB VRAM and 23.09 GiB private commit, then emitted EOS without the answer. It remains experimental.

OpenCode request capture also supersedes the earlier prompt estimate: a bounded request is 26,003 bytes. A Fast-32K fresh `hey` used 5,529 input tokens, 11 output tokens and about 1.55 seconds; the final Stable-16K smoke used the same input, produced 64 output tokens in about 9.76 seconds of model time, and did not compact. The prior 24–29K request was foreign/global prompt and skill-catalog contamination.

The private raw runs and machine inventory behind this dated investigation are intentionally excluded from the public source tree. Reproduce any claim through the current Alpine evidence path before relying on it.

## Decision

Use the existing Blackfrost `Q4_K_M` model at 16,384 tokens with one server slot, MTP depth 3, Q8 KV cache, and 16 CPU threads. In text mode, keep attention, DeltaNet/SSM, and recurrent-state tensors on the RTX 5070 while placing only FFN matrices from blocks 0–43 on CPU. Use `--load-mode none`; this is the measured optimum on this 12 GB GPU plus 32 GB single-channel host. Vision remains available through a separate launcher and uses automatic fitting because the projector needs additional VRAM. Override only the GGUF's added Blackfrost execution-bias block with Qwen's official chat template; the Qwen tool, vision, and reasoning structure remains intact.

This keeps the model at 4-bit-or-better precision, preserves the 16K context, and does not remove coding, shell, web, or tool capability. The runtime changes only concurrency, memory placement, and whether the optional vision projector is resident.

## Measured results

All decode results use the same 256-token deterministic completion on the installed RTX 5070 12 GB, i7-13700, 32 GB single-DIMM machine. Each matrix changed one variable at a time.

| Configuration | Decode tok/s | Change from old default |
|---|---:|---:|
| Old default: auto 4 slots, vision, MTP 3 | 4.42 | baseline |
| 1 slot, vision, MTP 3 | 5.45 | +23% |
| 1 slot, text-only, MTP 3, 1024 MiB reserve | 6.32 | +43% |
| Tuned auto-fit: 1 slot, text-only, MTP 3, 512 MiB reserve | 6.84 | +55% |
| Final production: FFN-only CPU placement, MTP 3 | **7.49** | **+69%** |

The vision mode measured 5.60 tok/s with automatic fitting. It keeps the one-slot improvement but pays the expected cost of keeping the projector resident. It remains a deliberate mode, not a removed capability.

### Tensor placement

Automatic fitting moves complete early layers to CPU. That is suboptimal on this hybrid Qwen architecture because it also moves the relatively small attention and recurrent-state work away from CUDA. Tensor metadata showed approximately 10,046.6 MiB of FFN weights, 3,438.5 MiB of attention weights, and 830.2 MiB of SSM weights. The winning override moves only selected FFN matrices:

```text
blk\.([0-9]|[1-3][0-9]|4[0-3])\.ffn_.*=CPU
```

The controlled workload was 4,050 prompt tokens plus 256 deterministic output tokens:

| Placement | Prompt tok/s | Decode tok/s | Combined time | GPU memory |
|---|---:|---:|---:|---:|
| auto-fit, 512 MiB reserve | 322.33 | 6.61 | 51.31 s | 11,058 MiB |
| FFN blocks 0–43 on CPU, mmap | 418.59 | 7.24 | 45.02 s | 11,425 MiB |
| FFN blocks 0–43 on CPU, load none | **691.18** | **7.47** | **40.11 s** | 11,264 MiB |
| FFN blocks 0–42 on CPU, load none | 682.28 | 7.18 | 41.57 s | 11,428 MiB |
| FFN blocks 0–41 on CPU, load none | 701.55 | 6.85 | 43.15 s | 11,572 MiB |

The selected shape cut combined time by about 21.8% and more than doubled prompt ingestion. Two independent runs produced the same output hash. Physical working set fell from about 17.4 GiB under auto-fit to about 8.8–9.1 GiB under `--load-mode none`; private commit is higher, so the machine still needs its page file enabled.

### Speculative decoding

| MTP setting, one slot + vision | Decode tok/s |
|---|---:|
| disabled | 2.78 |
| depth 1 | 4.22 |
| depth 2 | 5.09 |
| depth 3 | **5.45** |
| depth 4 | 5.37 |
| depth 8, p-min 0.8 | 4.40 |
| depth 16, p-min 0.8 | 3.34 |

MTP depth 3 nearly doubled decode speed over no speculation. Deeper drafting regressed on this workload. This agrees with NVIDIA's recommendation to start MTP at depth 3 and benchmark nearby values; higher depth is not automatically faster. NVIDIA also reports the same Qwen3.8 27B MTP mechanism improving an AGX Thor result from about 13 to 35 tok/s, while warning that exact model, quantization, and workload determine acceptance and speed. See [NVIDIA Jetson AI Lab: Speculative Decoding](https://www.jetson-ai-lab.com/tutorials/speculative-decoding/).

There is an upstream correctness caveat for quantized targets: [llama.cpp issue #25618](https://github.com/ggml-org/llama.cpp/issues/25618) reports greedy output divergence with MTP. Local target-only decode was 3.10 tok/s. MTP depth 1 produced the exact same 256-token SHA-256 at 5.03 tok/s; depth 3 produced a different but internally stable continuation at 7.49 tok/s. Three identical depth-3 requests in one process and repeated clean restarts all produced the same hash. Depth 3 remains the performance default because the user's objective prioritizes maximum useful speed, but `mtp_depth: 1` is the measured bit-for-bit target-equivalent setting for work that requires that property.

`ngram-mod` was also tested. By itself it was lossless and raised a repeat-heavy code workload from 3.11 to 12.88 tok/s. Combined with MTP3 it reached 29.20 tok/s, but repeated identical temperature-zero requests began producing different output hashes as the shared n-gram pool learned. MTP1 plus n-gram showed the same interaction. That attractive but non-repeatable combination is disabled in production. MTP3 alone reached 11.32 tok/s on the same repeated-code workload and stayed stable.

### GPU reserve and CPU threads

| Fit reserve | Decode tok/s | Prompt tok/s | GPU memory after load |
|---|---:|---:|---:|
| 1024 MiB | 6.32 | 15.79 | 10,495 MiB |
| 768 MiB | 6.53 | 16.93 | 10,740 MiB |
| 512 MiB | **6.73** | 17.88 | 10,964 MiB |
| 384 MiB | 6.23 | 18.76 | 11,176 MiB |
| 256 MiB | 6.22 | 18.36 | 11,177 MiB |

At 384 MiB, prompt ingestion improved but MTP draft acceptance fell from about 55% to 46%, slowing generation. The 512 MiB reserve was the measured auto-fit knee. It is retained for vision mode and as the fallback if the explicit text placement is removed.

Thread tests at the final memory shape put 12 and 16 threads within noise for decode. Sixteen threads had the best prompt throughput; 20 and 24 threads were slower. CPU affinity and priority hacks are therefore not justified.

### KV cache

Q8 remains the default. Q5 was slower. Q4 produced 6.93 tok/s in one short run, only about 2.7% above Q8, while changing the deterministic output path. That marginal gain does not justify reducing long-context cache fidelity. Quantizing only the MTP draft cache also produced no speed gain.

## Why the old `hey` showed 8K and took 50 seconds

The screenshot belongs to the older profile. OpenCode's database records that exact `Hey` session as 7,951 input tokens and 3 output tokens. It still had the larger generic prompt/tool catalog and automatic title request.

OpenCode 1.18.18's TUI meter sums the provider-reported input, output, reasoning, cache-read, and cache-write token fields; it does not reserve 4,096 output tokens in that displayed number. See the [OpenCode 1.18.18 TUI calculation](https://github.com/anomalyco/opencode/blob/v1.18.18/packages/tui/src/component/prompt/index.tsx#L264-L280). Thus 7,951 / 16,384 correctly appeared as about 49%.

The remaining first-turn path was measured in stages:

| OpenCode `hey` path | Input context | Wall time | Behavior |
|---|---:|---:|---|
| Screenshot profile | 7,951 tokens | 50.1 s | old generic profile |
| Minimal profile + optimized runtime, embedded Blackfrost prompt | about 4,312 tokens | 27.5 s | unnecessary Git/tool detour |
| Official template + auto-fit | 4,038 tokens | 19.1 s | one inference, direct answer |
| Final FFN-placement path | **4,038 tokens** | **10.68 s** | one inference, direct answer |
| Final identical warm path | 4 new + 4,034 cached | **5.6 s** | one inference, direct answer |

The GGUF's template was the official Qwen template plus one 1,389-character block saying that every response is an execution. That block strongly biased trivial conversation toward tools. After removing just that insertion, the remaining template matches [Qwen's official pinned template](https://huggingface.co/Qwen/Qwen3.8-27B/blob/1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0/chat_template.jinja); a real file question still completed the full OpenCode tool loop correctly. This changes harness behavior, not weights, precision, context, tool availability, or the model's low-refusal profile.

Most of the remaining cold prompt is the seven useful core tool schemas. Removing them would make the model a chat bot rather than a coding agent. A live file-read loop completed correctly in 11.43 seconds; its second inference reported 4,093 cached prompt tokens. The right speed path is prefix reuse plus the faster server shape, not stripping file/shell/web capability.

The earlier 4,096-token malformed `write` call was a generation-control failure, not a permission failure. The minimal prompt now tells the model to create minimal files and edit in bounded calls, and to retry parse failures with shorter calls. This preserves capability while avoiding a 15-minute all-or-nothing JSON generation.

A persisted llama.cpp slot cache was also tested and rejected. Saving 4,048 clean prefix tokens produced a 284 MiB file and took 134 ms; restoring it took about 82 ms. With this MTP model, however, the next request still reprocessed all 4,038 input tokens, including for an identical greeting, so wall time remained about 17.9 seconds. The cache and restore plumbing were removed.

## Blackwell-native FP4 assessment

The RTX 5070 reports compute capability 12.0 (Blackwell). The installed llama.cpp CUDA build contains `sm_120` support and the Blackwell NVFP4/MXFP4 kernels, so the runtime is capable of native FP4 tensor-core execution. NVIDIA describes NVFP4 as E2M1 values with an E4M3 scale per 16-value micro-block and an additional per-tensor scale. See [NVIDIA: Introducing NVFP4](https://developer.nvidia.com/blog/introducing-nvfp4-for-efficient-and-accurate-low-precision-inference/) and [llama.cpp](https://github.com/ggml-org/llama.cpp).

The installed `Q4_K_M` weights are not NVFP4. They use standard K-quant kernels and cannot become native Blackwell FP4 through a launch flag.

The official [Blackfrost NVFP4 checkpoint](https://huggingface.co/Blackfrost-AI/Qwen3.8-27B-ABLITERATED-NVFP4) is about 30.26 GB of ModelOpt safetensors, recommends SGLang or another ModelOpt-NVFP4 engine, and was validated on a B200. It cannot fit fully in this 12 GB GPU and is not a straightforward native-Windows llama.cpp replacement.

A third-party [NVFP4 GGUF conversion](https://huggingface.co/qzshch/Qwen3.8-27B-Blackfrost-Abliterated-NVFP4-GGUF) offers a 19,653,898,560-byte `VERY-LOW` file with SHA-256 `18ca45c3dea75f7f7f6f37990024ff00816eaed67c1b6ac46a9b0071cb2c5f72`. Its card says it quantizes 496 tensors, including attention and the stateful DeltaNet/SSM path, to NVFP4. The quality-oriented `ORIG` instead retains those paths in BF16 but is about 29.3 GB. `VERY-LOW` is larger than the current 16.81 GB model, so fewer weights can remain on the 12 GB GPU; CPU-offloaded NVFP4 also cannot use the RTX's native tensor cores. The partial download was therefore paused and the candidate rejected before spending several more hours on a checkpoint that violates the stated no-stupid-quality-compromise criterion.

No NVFP4 model should replace the current Q4_K_M default without beating it on speed, structured tool calls, coding tasks, and long-context checks.

## Current ecosystem findings

- [SGLang's current quantization documentation](https://docs.sglang.io/docs/advanced_features/quantization) supports GGUF on NVIDIA and native `modelopt_fp4` on SM100+; its FP4 backends include `flashinfer_cutlass` and CUDA 13+ cuDNN on SM120. This makes SGLang a credible future engine, but it does not make a 19–29 GB checkpoint fit a 12 GB GPU. CPU spill remains the controlling constraint here.
- [SGLang speculative decoding](https://docs.sglang.io/docs/advanced_features/speculative_decoding) supports MTP, EAGLE, DFlash, and n-gram methods. Its own guidance and NVIDIA's both imply workload benchmarking, which the local matrix confirms.
- Unsloth's release offers compact GGUFs and native NVFP4 options. A community [full quant comparison](https://www.reddit.com/r/LocalLLM/comments/1vr4iqj/i_benchmarked_every_qwen_38_27b_quant_that_fits/) reports Q4_K_M very close to Q8 perplexity and a larger quality drop for smaller IQ4/Q3 variants. Those are different upstream weights, so the numbers are supporting evidence rather than a direct benchmark of Blackfrost.
- Recent 12–16 GB community setups increasingly use FFN-only `--override-tensor` placement. The [dense-model guide](https://www.reddit.com/r/LocalLLM/comments/1vq5oyu/guide_for_running_dense_models_on_16_gb_vram_qwen/) motivated the local tensor-level investigation, but no published cutoff was copied: blocks 0–41, 0–42, 0–43, and 0–47 were measured on this machine.
- The experimental chained-MTP [llama.cpp PR #27173](https://github.com/ggml-org/llama.cpp/pull/27173) was built locally against CUDA 13.2 and native `sm_120a`. Its best tested setting improved combined time by less than 1% while increasing private memory from about 20.9 to 28.1 GiB. Depth 5 was slower and depth 8 fell to 4.32 tok/s. The official b10453 runtime remains production.

## Reproducibility

The historical PowerShell matrix runners were retired after their measured conclusions were captured here. Current reproducible measurements use Alpine's versioned Rust evaluation, benchmark, tuning, and evidence commands.

Raw server logs are machine-local under the generated installation's `logs` directory and are not public evidence.
