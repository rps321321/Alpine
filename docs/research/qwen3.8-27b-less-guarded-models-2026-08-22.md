# Current less-guarded Qwen3.8-27B releases

**Checked:** 2026-08-22  
**Target:** Windows, RTX 5070 12 GB, 32 GB system RAM, current `llama.cpp`/OpenCode stack  
**Scope:** read-only review of publisher model cards and live Hugging Face repository metadata. No weights were downloaded and no runtime configuration was changed.

## Bottom line

There is no proven replacement for the current qualified baseline yet. The approximately 16.8 GB **Elder Plinius / OBLITERATUS Q4_K_M** the user identified is now a materially stronger candidate than its original V1 release, but its published operating assumptions conflict with OpenCode.

- **First direct quality challenger: Elder Plinius OBLITERATUS V2 Q4_K_M.** The current 16.81 GB GGUF uses a complementary SVD/LEACE blend. The publisher's expanded results report MMLU 84.32% versus stock 84.6%, 2/842 refusals, 100% usable output, and 7/8 real-world tasks. These are encouraging publisher measurements, not an independent coding-agent qualification.
- **The OpenCode conflict is blocking evidence, not a footnote.** OBLITERATUS explicitly recommends an empty system prompt and thinking off, warning that system prompts can reintroduce refusals. OpenCode necessarily supplies a coding-agent system prompt, environment, and tool contract. Alpine cannot reproduce the card's “naked” operating condition without ceasing to test the intended OpenCode product.
- **First memory-aware challenger: 0bserverx RVN, preferably `RVN-Q3_K_S-multilingual.gguf` (12.07 GB decimal / 11.24 GiB) without embedded MTP for the first test.** It is the most interesting combination of a newer ARA-derived weight edit, code/tool-aware quant calibration, and a materially smaller footprint. Its claimed 0–1/100 refusal result is publisher evidence, not a capability result, and three ARA passes could still harm long-horizon behavior.
- **Second challenger: Jonathan Coletti Q4_K_M.** This is the most auditable capability-preservation release: the card discloses the Heretic search, base-relative benchmarks, quant perplexities, MTP graft verification, and important limitations. At 16.81 GB it has essentially the same memory problem as Blackfrost, so it tests the *weight edit*, not efficiency.
- **Treat HauhauCS Aggressive as experimental, not the production favorite.** Its publisher reports zero refusals, but the card itself positions the aggressive cut below a future Balanced release for reliability-critical long-context agent work. Its Q4_K_P is larger than the current model; IQ3_M is small enough to test but adds a large quantization trade-off.
- **“Ultra” was probably Unsloth. Unsloth's Qwen3.8-27B GGUF and NVFP4 releases are quantizations of the official Qwen model, not abliterated or uncensored checkpoints.** They are useful clean controls, not candidates for Alpine's less-guarded-model role.

The word *best* cannot be resolved from refusal counts or download popularity. Alpine should compare candidates on new coding-agent scenarios, especially context/compaction survival, tool selection, structured output, and completion of multi-file work.

## Shortlist

| Release | What was changed | Useful files for this machine | Evidence and caveats | Assessment |
|---|---|---|---|---|
| [Blackfrost Qwen3.8-27B Abliterated GGUF](https://huggingface.co/Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF/tree/4f6732ce2123ced5d9df722a934f700619e066a8) | Weight-level refusal-surface modification; publisher says no SFT, merge, LoRA, pruning, or coding fine-tune. | Q3_K_M 13.50 GB; Q4_K_S 15.83 GB; current Q4_K_M 16.81 GB. All include the native MTP head. | Card reports 11 residual refusals out of 450, but explicitly says this was a sequential funnel on its NVFP4 derivative, not a fresh full evaluation of every final GGUF. Experimental release. | **Known baseline; retain.** No new evidence that another ordinary Q4 is better for Alpine. |
| [Elder Plinius / OBLITERATUS Qwen3.8-27B V2](https://huggingface.co/OBLITERATUS/Qwen3.8-27B-OBLITERATED/tree/e335d239dbdfae590687e24b800e81a18d070ebe) | V2 blends 60% of a LEACE-based surgery with 40% of an aggressive/SVD surgery, then restores stock MTP and vision tensors. This supersedes the original five-direction V1. | IQ4_XS 15.42 GB; Q4_K_M 16.81 GB; Q5_K_M 19.54 GB. Compact projector 0.93 GB. | V1 achieved 0/842 refusals but lost about six MMLU points and produced only 80% usable output. Current V2 reports 2/842 refusals, 100% usable output, MMLU 84.32% versus stock 84.6% over 2,850 questions, and 7/8 advanced tasks. Full MMLU remains in progress. The card requires thinking off and recommends no system prompt. | **Strongest direct new challenger by disclosed publisher evidence, but not proven in OpenCode.** Same Q4 footprint as Blackfrost; it will not solve paging. |
| [0bserverx RVN Heretic Abliterated GGUF](https://huggingface.co/0bserverx/Qwen3.8-27B-Heretic-Abliterated-Uncensored-GGUF/tree/20b94f0613b632b4848bbe3b1e05d9ee0c2b1608) | Starts from Tim Rohrbaugh's ARA checkpoint, then applies two more full-weight ARA passes. Publisher claims KL about 0.0085 and 0–1 refusals per 100. | Plain multilingual Q3_K_S 12.07 GB; IQ3_S 12.42 GB; Q3_K_M 13.30 GB. MTP twins add about 0.45 GB. | Quant calibration includes code, reasoning, tools, and multiple languages; all files reportedly passed a tool-call API gate. Huge, rapidly changing file surface; prior corrupt IQ3_M was replaced. Refusal/KL claims do not establish coding quality. | **Best new memory-aware challenger**, pinned to an exact file and revision. Start without MTP to isolate model quality. |
| [JonathanColetti Qwen3.8-27B Uncensored GGUF](https://huggingface.co/JonathanColetti/Qwen3.8-27B-Uncensored-GGUF/tree/dee0a3164d9e11bbbebf5b63f52ba99443d14fc3) | Heretic search over refusal count and KL; BF16 edit followed by merge. Stock MTP tensors are grafted back and verified. | IQ2_M 10.62 GB; IQ4_XS 15.31 GB; Q4_K_M 16.81 GB. | Base-relative general benchmarks show a reported mean change of -0.5 points; Q4/IQ4 perplexities remain close to BF16 while IQ2_M degrades clearly. Card reports 12/100 harmful-prompt refusals and explicitly says code, math, multilingual, vision, and generative ability were not evaluated. | **Best-documented weight-edit challenger.** Use Q4_K_M for a fair quality comparison; IQ2_M is too compromised to represent the checkpoint fairly. |
| [HauhauCS Aggressive MTP GGUF](https://huggingface.co/HauhauCS/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF/tree/993a5971fda8f30dd1b7eb2654792ba4415c7460) | Aggressive refusal removal with direct-answer behavior; embedded MTP. | IQ3_M 12.79 GB; IQ4_XS 15.71 GB; Q4_K_P 17.92 GB. | Publisher claims 0/465 refusals. FastMTP figures come from an RTX Pro 6000 Blackwell 96 GB and require a pinned runtime/patch, so they do not transfer to the RTX 5070. The card recommends a Balanced cut for reliability-critical long-context agents when available. | **Interesting experiment, not first production candidate.** Avoid adding its custom FastMTP path before ordinary-model qualification. |
| [Huihui Qwen3.8-27B Abliterated GGUF](https://huggingface.co/huihui-ai/Huihui-Qwen3.8-27B-abliterated-GGUF/tree/2379b9294c14c0e55bd2ea5ac84d8bb9ffdfd98b) | Directional abliteration; first 15 layers retained; MTP and vision unchanged. | Standard Q4_K 16.81 GB; fidelity-preserving Q4_K_L 20.92 GB. | The author calls the implementation a crude proof of concept. K_L keeps ablation-sensitive tensors at Q8/BF16, increasing footprint substantially. No comparative coding-agent evidence. | **Lower priority.** K_L is a poor fit for 12 GB VRAM/32 GB RAM. |
| [AEON-based 3.69 bpw 12 GB MTP GGUF](https://huggingface.co/soyaakinohara/qwen3.8-27b-abliterated-3.69bpw-12GB-MTP.gguf/tree/5fa37c47bf58d013ab60588ddd021a4ed664b5b0) | Independent mixed quant of AEON-7's early-access Abliterix checkpoint. Preserves sensitive Gated-DeltaNet/SSM, attention, embedding/output, and MTP tensors at higher precision. | One 12.60 GB decimal / 11.73 GiB file. | Attractive size and thoughtful tensor placement, but the parent explicitly warns of loops and coherence loss on very long responses. Reported speed used two GPUs with 24 GB combined VRAM. No comparative coding benchmark. | **Useful diagnostic for memory pressure, not a production favorite.** It changes both the weight edit and quantization at once. |
| [Unsloth Qwen3.8-27B GGUF](https://huggingface.co/unsloth/Qwen3.8-27B-GGUF/tree/4ca720788d1e01f1bff70c033e0d0028fd02e502) | Dynamic/importance-aware quantization of official `Qwen/Qwen3.8-27B`; no refusal removal. | UD-Q3_K_XL 13.15 GB; UD-IQ4_XS 14.25 GB; UD-Q4_K_M 16.46 GB. | Unsloth claims improved quant accuracy, but it remains the guarded official checkpoint. The separate NVFP4 release is also official-base and too large for this machine's memory envelope. | **Clean control only**, not a better abliterated model. |

All listed repositories declare Apache-2.0. That metadata does not make every publisher's added usage clause or third-party artifact automatically equivalent; pin and review the selected repository's card and files before redistribution.

## Hardware interpretation

The RTX 5070 has 12 GB dedicated VRAM, but a 12 GB model file is not a 12 GB deployment. Runtime buffers, CUDA allocations, Gated-DeltaNet recurrent state, context state, and optional vision/MTP weights need additional memory. Therefore:

- 15.8–17.9 GB Q4-class files still require CPU offload and substantial system RAM, just like the current Blackfrost Q4_K_M.
- 11–13 GB Q3/IQ3 files are more plausible for reducing host spill, but they still should not be assumed to fully fit in VRAM at 16K context.
- 8–10 GB IQ2 files fit more comfortably but introduce the largest quality risk. Both Jonathan's and RVN's cards warn or imply that extreme low-bit quants are not faithful quality representatives.
- Blackfrost NVFP4 is not a 12 GB shortcut. Its four safetensor shards total about 30.24 GB, and its card validates SGLang on a B200. It would increase host-memory pressure here.
- Vision and MTP should initially be omitted for challenger qualification. Add each separately only after the plain text checkpoint passes capability and lifecycle gates.

The current failure evidence also cautions against blaming Blackfrost alone: Stable reproduced context/compaction degradation, and the last cold evaluation stalled under host paging before producing a sample. A smaller quant may improve memory behavior, but it cannot repair OpenCode context overhead or compaction semantics by itself.

## Elder Plinius / OBLITERATUS: the important distinction

The repository was replaced with **V2 weights** at the current pinned revision. The older public claims and many discussions describe V1, so they should not be mixed:

| | V1 | Current V2 |
|---|---:|---:|
| Method | Five-direction aggressive/SVD surgery | 60/40 blend of LEACE and aggressive/SVD surgeries |
| Refusal result | 0/842 | 2/842 (0.24%) |
| Usable output | 80% | 100% |
| MMLU | 81.4% on 285 questions | 84.32% on 2,850 questions; stock 84.6% |
| Advanced real-world tasks | Not tested | 7/8, tied with stock in the publisher matrix |

This is a real reported improvement in the capability/refusal trade-off, but it is not conclusive: the full MMLU suite is still in progress, the eight practical tasks are a small publisher-designed matrix, and no OpenCode long-horizon/compaction test is published.

The model card's operating contract matters even more for Alpine:

- temperature 0 and repetition penalty 1.15;
- thinking **off**;
- no/empty system prompt, because the publisher reports that system prompts can reintroduce refusals;
- the V2 GGUF chat template defaults thinking off, but inference tools can override it.

Alpine already disables thinking, which is compatible. OpenCode's system prompt is not optional in the intended coding-agent workflow: it carries the agent role, environment, tool definitions, and authority boundary. Removing it merely to preserve OBLITERATUS's benchmark refusal rate would test a different product and weaken tool behavior. The correct experiment is therefore to run V2 **with Alpine's normal OpenCode prompt**, record whether legitimate authorized tasks are refused, and judge capability normally. A refusal rate measured with no system prompt must not be presented as Alpine's expected result.

## Proposed comparison order

No artifact change is recommended yet. If a controlled acquisition is later approved:

1. Preserve the current installed/qualified artifact as the rollback baseline and verify its exact repository/hash before any acquisition; the repository configuration inspected during this research names Blackfrost, while the user identifies the approximately 16.8 GB model as Elder Plinius OBLITERATUS.
2. If OBLITERATUS V2 is not already the exact installed artifact, test **OBLITERATUS V2 Q4_K_M** first with Alpine's normal OpenCode system prompt, thinking off, MTP off, and identical placement. This directly compares weight behavior at the same Q4 size.
3. Test **RVN Q3_K_S multilingual, plain/no-MTP**, pinned by repository revision and file SHA. It is the clearest test of whether a roughly 4.7 GB smaller model improves paging and usable agent behavior enough to offset Q3 quality loss.
4. Test **JonathanColetti Q4_K_M** with identical runtime, template, context, and no speculative decoding if a second same-size ablation-method comparison is useful.
5. Consider HauhauCS IQ3_M only if the first challengers fail; do not introduce FastMTP or a patched runtime into the same experiment.
6. Use new realistic coding-agent scenarios. Record throughput and memory, but gate on task completion, post-compaction correctness, tool routing, structured output, and refusal behavior on legitimate authorized work.

Downloads and likes were checked only as weak ecosystem signals and were not used to rank quality. Repository cards are publisher claims unless the report explicitly says otherwise.

## Primary sources

- [Official Qwen3.8-27B model card](https://huggingface.co/Qwen/Qwen3.8-27B)
- [Elder Plinius / OBLITERATUS V2 card and exact files](https://huggingface.co/OBLITERATUS/Qwen3.8-27B-OBLITERATED/tree/e335d239dbdfae590687e24b800e81a18d070ebe)
- [Elder Plinius OBLITERATUS source repository](https://github.com/elder-plinius/OBLITERATUS)
- [Blackfrost GGUF card and exact files](https://huggingface.co/Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF/tree/4f6732ce2123ced5d9df722a934f700619e066a8)
- [Blackfrost NVFP4 card](https://huggingface.co/Blackfrost-AI/Qwen3.8-27B-ABLITERATED-NVFP4/tree/faf7945020c138c8ef864ab1644273f3158f85fa)
- [RVN card and exact files](https://huggingface.co/0bserverx/Qwen3.8-27B-Heretic-Abliterated-Uncensored-GGUF/tree/20b94f0613b632b4848bbe3b1e05d9ee0c2b1608)
- [Tim Rohrbaugh ARA parent card](https://huggingface.co/trohrbaugh/Qwen3.8-27B-heretic-ara)
- [Jonathan Coletti card and exact files](https://huggingface.co/JonathanColetti/Qwen3.8-27B-Uncensored-GGUF/tree/dee0a3164d9e11bbbebf5b63f52ba99443d14fc3)
- [HauhauCS Aggressive card and exact files](https://huggingface.co/HauhauCS/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF/tree/993a5971fda8f30dd1b7eb2654792ba4415c7460)
- [Huihui card and exact files](https://huggingface.co/huihui-ai/Huihui-Qwen3.8-27B-abliterated-GGUF/tree/2379b9294c14c0e55bd2ea5ac84d8bb9ffdfd98b)
- [AEON 3.69 bpw quant card](https://huggingface.co/soyaakinohara/qwen3.8-27b-abliterated-3.69bpw-12GB-MTP.gguf/tree/5fa37c47bf58d013ab60588ddd021a4ed664b5b0)
- [AEON-7 BF16 parent card](https://huggingface.co/AEON-7/Qwen3.8-27B-AEON-ULTIMATE-UNCENSORED-BF16)
- [Unsloth official-base GGUF card and files](https://huggingface.co/unsloth/Qwen3.8-27B-GGUF/tree/4ca720788d1e01f1bff70c033e0d0028fd02e502)
