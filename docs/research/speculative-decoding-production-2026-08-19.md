# Speculative decoding and production deployment for Qwen3.8-27B

**Research date:** 2026-08-19
**Scope:** Qwen3.8-27B on one Windows RTX 5070 12 GB system, with `llama.cpp` as the current runtime and SGLang as a possible future runtime. This note separates upstream facts from inferences based on the local b10453 measurements.

## Decision summary

### Local follow-up after source isolation

The upstream analysis below remains the reason not to ship **unmodified shared** `ngram-mod`. A subsequent local negative-control experiment proved that b10453's process-wide table carried proposal state across requests. The repository now carries a minimal opt-in patch that resets only that table at request begin. With the patch enabled, four identical novel requests kept one hash; three medium-repeat requests retained the MTP3 hash at 28.0–29.6 tok/s; and the five-run perfect-copy microbenchmark kept the Stable hash at a 64.17 tok/s median.

That is enough to reclassify request-local n-gram from “do not use” to **candidate**, not production. It does not resolve the upstream quantized-MTP or recurrent rollback issues described below. Stable remains official-runtime MTP3 without n-gram, and the patched candidate must pass the broader restart and agent gates in `config/promotion-policy.json`.

- Keep the current **16,384-token, Q8 KV, `draft-mtp=3`, no n-gram** profile as the production default. It is the best currently measured balance of speed, memory and repeatability on this machine.
- Do **not** promote unmodified `draft-mtp + ngram-mod` merely because it reached 29.2 tok/s on repeated code. `ngram-mod` deliberately uses a mutable table shared by all requests, while upstream has a still-open bug showing that quantized target models can diverge under MTP even at greedy settings. The request-local patch removes the measured cross-request state source, but the remaining verification/rollback path is not yet proven lossless across the full validation matrix.
- Treat `ngram-simple + MTP3`, `ngram-mod` alone, and any newer `llama.cpp` build as experiments behind an exact-output and agent-task regression gate. `ngram-simple` is the most defensible next experiment because it searches the sequence history instead of a process-wide mutable table.
- Increase context in separate profiles, not by changing the production default globally. At Q8 KV, this model consumes about **544 MiB at 16K, 1,088 MiB at 32K, 2,176 MiB at 64K, 4,352 MiB at 128K and 8,704 MiB at 262K**. Each profile needs its own layer/tensor placement and near-limit stress test.
- Pin the whole serving artifact, not just the model name: `llama.cpp` commit/release, CUDA DLL set, build architecture, compiler/CMake versions, model revision and hash, GGUF hash, chat template, launch arguments and sampler settings. For native Blackwell builds, explicitly target `120a-real`.
- There is no released post-b10453 change that can presently be cited as fixing this combined divergence. b10502 is newer, but the relevant MTP issue remains open and the rollback repair in PR #27173 remains unmerged.

## 1. Why MTP plus `ngram-mod` can make greedy requests diverge

### What should happen

Speculative decoding is intended to change performance, not the verified target-model result. The draft proposes several tokens; the target model verifies them and corrects the first rejection. With greedy sampling and identical target computation, the final tokens should match target-only decoding. The [llama.cpp speculative decoding guide](https://github.com/ggml-org/llama.cpp/blob/master/docs/speculative.md) explicitly recommends greedy sampling when exact output matching is required.

That intended property is not currently reliable for every quantized draft-model path. [Issue #25618](https://github.com/ggml-org/llama.cpp/issues/25618), still open on the research date, reproduces different greedy outputs with `draft-mtp`/DSpark on Q4 targets while BF16 matches. Target-only Q4 is repeatable, `ngram-simple` and `ngram-mod` match the target on the same Q4 setup, and disabling Flash Attention does not remove the MTP/DSpark failure. The issue therefore narrows the fault class to the draft-model path—draft context/KV, embedding injection, multi-token target evaluation, or related path-dependent quantized computation—but does **not** establish one final root cause.

### Why `ngram-mod` changes the execution path between otherwise identical requests

At b10453, `ngram-mod` is not a pure function of the current prompt:

1. Its rolling hash maps n-grams into a fixed-size table and overwrites the entry on collision ([`ngram-mod.cpp` at b10453](https://github.com/ggml-org/llama.cpp/blob/3cb7ffb1a/common/ngram-mod.cpp)).
2. The speculative implementation owns one table **shared across all sequences/requests**, feeds each prompt back into it, resets it based on occupancy/acceptance history, and gives draftless methods precedence when methods are combined ([`speculative.cpp` at b10453](https://github.com/ggml-org/llama.cpp/blob/3cb7ffb1a/common/speculative.cpp); [design guide](https://github.com/ggml-org/llama.cpp/blob/master/docs/speculative.md)).
3. Consequently, prior requests, hash collisions, acceptance history and reset timing can change the n-gram proposal presented for the same later prompt. This is deterministic mutable state, but it is not request-isolated state.

A correct target verifier should reject any wrong proposal, so the shared table alone does **not** prove output corruption. It does change proposal lengths, verification batches and rollback points. That matters because upstream has separately documented recurrent rollback defects in hybrid models: [issue #26695](https://github.com/ggml-org/llama.cpp/issues/26695) shows that fully undoing a decoded batch can leave Qwen hybrid recurrent state modified, and open [PR #27173](https://github.com/ggml-org/llama.cpp/pull/27173) includes a gated-DeltaNet convolution-state rollback repair plus a regression test.

**Best-supported explanation of the local result:** the shared `ngram-mod` pool changes which token batches enter the speculative verifier; the quantized MTP and recurrent-state path is sensitive to those batches/rollback points; a small path-dependent logit difference can change a greedy argmax and all later tokens. This is an inference from the source and upstream bug reports, not an upstream-confirmed single cause. It fits the local evidence: MTP3 alone was internally stable, n-gram alone matched the target, but their combination varied across identical requests.

There is a second operational warning. [PR #25819](https://github.com/ggml-org/llama.cpp/pull/25819) describes an `ngram-mod` verification-failure loop in which restoring a checkpoint can recreate the same draft and fail again. The proposed loop detector is explicitly described as mitigation rather than the root fix and was still not a production-ready merged fix on the research date.

### Production interpretation of 29.2 tok/s

The 29.2 tok/s result was a repeated-code best case: precisely the workload in which an n-gram predictor can copy long runs. It measures a useful ceiling, not general agent throughput. Cross-request output variation at temperature zero is a larger reliability signal than the benchmark gain because coding agents rely on valid JSON/tool calls, reproducible edits and bounded retry behavior.

Recommended modes:

| Mode | Production status | Reason |
|---|---|---|
| Target only | Reference profile | Slow but establishes the canonical target behavior. |
| MTP1 | Strict speculative candidate | Locally matched the target hash; retain for correctness-sensitive comparisons. |
| MTP3 | Current default | Locally repeatable and materially faster, although its text differed from target-only; judge it on task success as well as hashes. |
| `ngram-simple + MTP3` | Best next experiment | Request-history search avoids the process-wide mutable hash pool; still exercises the MTP verification/rollback path. |
| `ngram-mod` alone | Isolated experiment | Locally target-equivalent, but has shared mutable state and the open stuck-loop report. |
| `ngram-mod + MTP` | Do not use in production | Local greedy divergence plus unresolved upstream MTP/rollback risks. |

An alternative process-per-session design can bound `ngram-mod` contamination, but it sacrifices cross-request warming and adds restart/model-load cost. It also cannot cure an MTP verifier or recurrent rollback error inside that session.

### Acceptance gate for any speculative change

Do not accept a mode from one prompt or average tokens/s. Compare against a target-only canonical run with:

- at least 50 identical greedy requests in one process and 10 clean server restarts;
- cold, warm and prefix-cache paths, plus deliberately different preceding requests to exercise the shared n-gram pool;
- repeated code, novel code edits, prose, exact JSON/tool schemas, long prompts, context-checkpoint reuse and generation near the context limit;
- exact token/output hashes, parse success, task success, retries, timeouts/stalls, accepted-draft counts, TTFT, generation rate, private RAM, VRAM and recurrent-state allocation;
- concurrency matching production (`--parallel 1` now; repeat the gate if it changes).

Fail closed to the last stable profile on any malformed tool call, stalled request, unexplained hash split or recurrent-state mismatch. A newer build is not a fix until it passes this matrix on the actual GGUF and GPU.

## 2. Context scaling for the hybrid DeltaNet/attention model

### Architecture facts

The pinned [Qwen3.8-27B configuration](https://huggingface.co/Qwen/Qwen3.8-27B/blob/1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0/config.json) has 64 layers in a repeating three-linear-attention/one-full-attention pattern: 48 gated-DeltaNet recurrent layers and 16 full-attention layers. Full-attention layers use four KV heads with head dimension 256. The recurrent SSM state is configured as FP32. The official [model card](https://huggingface.co/Qwen/Qwen3.8-27B/blob/1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0/README.md) declares a native 262,144-token context and recommends YaRN only beyond that; it also warns that current framework implementations generally apply static YaRN and that this can degrade shorter-context performance.

Only the 16 full-attention layers grow a conventional KV cache with sequence length:

```text
16 layers × 2 (K and V) × 4 KV heads × 256 values = 32,768 cached values/token
```

At F16 that is 65,536 bytes/token (64 KiB/token). llama.cpp Q8_0 stores 32 int8 values plus one FP16 scale in 34 bytes ([Q8_0 type source](https://github.com/ggml-org/llama.cpp/blob/master/ggml/src/ggml.c), [quant block definition](https://github.com/ggml-org/llama.cpp/blob/master/ggml/src/ggml-quants.h)), or 1.0625 bytes/value. This gives 34,816 bytes/token, exactly 34 KiB/token, before small allocator overhead.

| Context | Q8_0 KV | F16 KV | Increment over current 16K Q8 |
|---:|---:|---:|---:|
| 16,384 | 544 MiB | 1,024 MiB | — |
| 32,768 | 1,088 MiB | 2,048 MiB | +544 MiB |
| 65,536 | 2,176 MiB | 4,096 MiB | +1,632 MiB |
| 131,072 | 4,352 MiB | 8,192 MiB | +3,808 MiB |
| 262,144 | 8,704 MiB (8.5 GiB) | 16,384 MiB | +8,160 MiB |

The 544 MiB 16K Q8 allocation is also present in the local b10453 logs, so the formula agrees with the runtime allocation. These figures are cache capacity, not total VRAM: weights, CUDA graphs/work buffers, embeddings, recurrent state and the display driver remain additional consumers.

### Recurrent state is context-constant but speculation-dependent

DeltaNet layers avoid a token-by-token KV cache, but their recurrent state still needs a cell per live sequence and rollback/checkpoint state. On the exact local Qwen3.8-27B b10453 configuration with `--parallel 1`, logs in the [local performance report](../performance-optimization-2026-08-19.md) measured:

| Speculation depth | Recurrent state |
|---:|---:|
| none | 149.62 MiB |
| MTP1 | 299.25 MiB |
| MTP3 | 598.50 MiB |
| MTP5 | 897.75 MiB |
| MTP8 | 1,346.62 MiB |

The observed relationship is approximately `(draft depth + 1) × 149.62 MiB`. This is a local implementation measurement, not a general architectural constant. Parallel slots and context checkpoints can add more recurrent cells. The merged [hybrid recurrent context-checkpoint work in PR #19408](https://github.com/ggml-org/llama.cpp/pull/19408) improves OpenCode-style multi-turn reuse, but checkpoint counts should be chosen deliberately because each retained recurrent snapshot is large.

### Safe scaling plan on 12 GB VRAM

The 16K profile already used roughly 11.26 GB of dedicated GPU memory in the local test. Therefore context growth competes directly with GPU-resident weights/workspace:

1. Keep 16K as the normal profile.
2. Add a separate 32K candidate. Its extra 544 MiB Q8 KV may fit only after rechecking headroom and may require slightly more FFN tensor placement on CPU.
3. Treat 64K as an extended profile. It needs about 1.63 GiB more KV than 16K, so additional CPU spill and lower generation speed are expected.
4. Treat 128K and 262K as research profiles on this 12 GB card. Native model support does not imply that this quantized, partially offloaded deployment can serve them efficiently.
5. Do not enable YaRN at or below 262K. Beyond 262K, evaluate the official YaRN guidance in a separate long-context quality benchmark.

For every context profile, retune tensor placement after KV allocation, then test prompt ingestion near the advertised maximum, a long generation afterward, multi-turn prefix reuse, and recovery after cancellation/rollback. A server that merely starts at a large `--ctx-size` has not demonstrated usable long-context operation.

## 3. Reproducible Windows Blackwell (`sm_120`) builds and runtime pins

The current official b10453 package is a reasonable production baseline because its identity is unambiguous: [release b10453](https://github.com/ggml-org/llama.cpp/releases/tag/b10453), commit `3cb7ffb1a1f612d5e4a46244ae5a3c77ad934a70`, Windows x64 CUDA 13 executable package plus the matching CUDA 13.3 DLL package.

For a source build, the b10453 [Windows/CUDA build instructions](https://github.com/ggml-org/llama.cpp/blob/3cb7ffb1a/docs/build.md) require a supported Visual Studio C++ environment and CMake. The b10453 [CUDA CMake logic](https://github.com/ggml-org/llama.cpp/blob/3cb7ffb1a/ggml/src/ggml-cuda/CMakeLists.txt) recognizes Blackwell compute capability 120 only with CUDA 12.8 or later, uses architecture-specific `a` targets for FP4 instructions because they are not forward-compatible, and supports `120a-real`. Use an explicit architecture rather than relying on workstation-dependent native detection:

```powershell
cmake -S . -B build-sm120 `
  -DGGML_CUDA=ON `
  -DGGML_NATIVE=OFF `
  -DCMAKE_CUDA_ARCHITECTURES=120a-real `
  -DCMAKE_BUILD_TYPE=Release
cmake --build build-sm120 --config Release --parallel
```

`GGML_NATIVE` and every other performance flag must be explicit in the recorded manifest. If portability means “this exact RTX 5070/Blackwell class,” `120a-real` is appropriate. If one package must run on other GPU generations, produce and test a separate multi-architecture artifact rather than assuming an architecture-specific binary is portable.

Record the following beside every deployable bundle:

- llama.cpp tag **and full Git commit**; clean/dirty source status and patch IDs if not clean;
- official asset filenames or exact CMake command; SHA-256 of every EXE/DLL/archive;
- CUDA toolkit version, CUDA runtime DLL set, NVIDIA driver, CMake version, Visual Studio Build Tools/MSVC version and Windows build;
- `CMAKE_CUDA_ARCHITECTURES=120a-real`, `GGML_CUDA`, `GGML_NATIVE` and all non-default GGML options;
- output of `llama-server --version` and startup `system_info`, including compiled CUDA architectures and Blackwell feature flags;
- official Qwen model revision `1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0`, source file hashes, conversion/quantization command, GGUF SHA-256 and any abliterated-model provenance;
- exact chat template, context/KV types, tensor overrides, batch/ubatch, threads, parallelism, sampler settings, MTP/n-gram settings and environment variables;
- a golden-prompt result set and the speculative acceptance gate results from the same artifact.

Ship the executable and all dependent DLLs from the same release/build directory. PATH-order substitution of a different CUDA runtime is not a reproducible deployment. NVIDIA's [Blackwell compatibility guide](https://docs.nvidia.com/cuda/archive/13.0.0/blackwell-compatibility-guide/index.html) explains why native cubins and PTX/JIT compatibility are distinct; record which one the artifact actually contains.

## 4. What has changed after llama.cpp b10453

The newest release on the research date is [b10502](https://github.com/ggml-org/llama.cpp/releases/tag/b10502), commit `0adcc3b`, also offering Windows CUDA 13 executables with CUDA 13.3 DLLs. The [b10453-to-b10502 comparison](https://github.com/ggml-org/llama.cpp/compare/3cb7ffb1a...0adcc3bb5) contains 49 commits. An audit of the commit/file set found no merged change that claims to fix quantized MTP greedy divergence, the combined n-gram/MTP behavior, or the gated-DeltaNet rollback defect described above. This is a scoped source-history finding, not proof that no unrelated change could affect the symptom.

More decisive evidence is the tracker state:

- quantized target divergence [issue #25618](https://github.com/ggml-org/llama.cpp/issues/25618) remains open;
- `ngram-mod` loop mitigation [PR #25819](https://github.com/ggml-org/llama.cpp/pull/25819) is not a completed root fix;
- chained MTP and the explicit gated-DeltaNet rollback repair [PR #27173](https://github.com/ggml-org/llama.cpp/pull/27173) remain open and unmerged.

Therefore b10502 can be benchmarked as a clean candidate, but it should not replace b10453 on the assumption that it fixes the 29.2 tok/s mode. PR #27173 is also not automatically the answer: its author reports gains on larger multi-GPU Blackwell hardware, while the isolated local single-5070 build showed less than 1% combined improvement and much higher private memory. If its rollback fix is evaluated, run a focused correctness build and the complete regression gate before considering the performance branch.

## 5. SGLang as an alternative runtime

SGLang has first-class documentation for this exact family. Its [Qwen3.8-27B cookbook](https://docs.sglang.io/cookbook/autoregressive/Qwen/Qwen3.8-27B.md) describes the 48-GDN/16-attention architecture, native 262K context, FlashInfer requirements on SM120/121, and MTP with linear replay of SSM state. It also exposes the central memory trade-off: a GDN state slot is approximately 153.9 MB in FP32 or 78.4 MB in BF16, so state-pool capacity can limit concurrency before the attention KV pool. Changing the SSM state dtype is a speed/memory/quality choice and needs its own evaluation.

The same cookbook's practical single-GPU checkpoints are about 16.5 GB for NVFP4 and 28.5 GB for FP8, aimed at a 32 GB RTX 5090. They do not remove the 12 GB capacity constraint. SGLang's [speculative decoding documentation](https://docs.sglang.io/docs/advanced_features/speculative_decoding) also notes that n-gram speculation is CUDA-only, disables some overlap/chunked-prefill combinations, and that deeper speculation can cause rejection cascades and higher memory use.

Finally, temperature zero alone is not a reproducibility guarantee in a dynamic batching runtime. SGLang's [deterministic inference guide](https://docs.sglang.io/docs/advanced_features/deterministic_inference) explains that reduction order and batching can vary results and provides dedicated deterministic flags/tests with backend constraints. A serious SGLang evaluation should therefore use WSL2/Linux or a pinned container/commit, pin the compatible FlashInfer version/backend, start with concurrency one, and pass the same output/tool-call gate. The official [installation guide](https://docs.sglang.io/docs/get-started/install) is Linux/container-oriented; there is no equivalent supported native-Windows production recipe to prefer over the current llama.cpp deployment.

## Bottom line

The production problem is not that n-gram speculation is inherently unsafe or that greedy sampling is random. It is that the fastest local mode combines a **shared, history-dependent draft source** with a **quantized hybrid-model verification and rollback path that upstream has not yet proven lossless**. Keep that mode experimental. Preserve 16K/MTP3 as the stable daily profile, retain a target-only or MTP1 reference profile, and make every speed or context increase earn promotion through reproducibility, agent-task correctness and memory stress tests on the exact pinned artifact.
