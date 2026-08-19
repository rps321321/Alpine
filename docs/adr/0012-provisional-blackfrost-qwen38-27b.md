# Provisional v1 Local Model: Blackfrost Qwen3.8-27B-ABLITERATED Q4_K_M

Capability first. The provisional primary is the Blackfrost 3.8-27B Abliterated Q4_K_M GGUF. Same-family fallback is Q3_K_M. Switch families only if 27B violates the operational floor (sustained <<1 tok/s, pathological paging, OOM/crashes, unusable context, tool-loop collapse). Runner-up is heretic-org ARA. Huihui 3.8 is not the day-one file. 35B-A3B is not chosen for speed.

This is a model-selection decision, not a download. Session Config must record immutable artifact identity (repo, revision, filename, SHA256, size), not a mutable marketing name.

Verified from Hugging Face on 2026-08-17 and installed/checksummed locally afterward:

- GGUF repo: `Blackfrost-AI/Qwen3.8-27B-ABLITERATED-GGUF`
- Repo revision: `4f6732ce2123ced5d9df722a934f700619e066a8` (lastModified 2026-08-16T07:20:42Z)
- Parent: `Blackfrost-AI/Qwen3.8-27B-ABLITERATED-BF16`
- Upstream base: `Qwen/Qwen3.8-27B` (Apache-2.0)
- Method: weight-level refusal-surface edit; card says not SFT, DPO, LoRA, merge, or prune
- Extra (not a fine-tune): Blackfrost operational system prompt is embedded in the GGUF chat template; ADR 0017 overrides only this added block with Qwen's official template after it was measured to cause unnecessary tool calls
- Primary file: `Qwen3.8-27B-ABLITERATED-Q4_K_M.gguf`
- Size: 16,810,716,384 bytes (16.8 GB decimal)
- SHA256: `5d53637a59cfcd3a4d8354e254ffd44943e5a693da2405a3e228c62962355509`
- Same-family fallback: `Qwen3.8-27B-ABLITERATED-Q3_K_M.gguf`, 13,500,738,784 bytes, SHA256 `18580f97cc179c2042ccca8ec52fa32921255ade624b5e28b69733c36ce4de2f`
- MTP: vendor claims native MTP head rebuilt into each main GGUF on 2026-08-16; smoke-tested as 65-block model on B200, not on a 5070
- Local runtime: verified on official llama.cpp b10453 CUDA 13.3 with MTP depth 3 and 16,384-token context
