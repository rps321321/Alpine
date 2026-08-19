# Dedicated CUDA llama-server; 16k context; official Qwen chat template

The Inference Session uses a current official ggml-org Windows CUDA 13 `llama-server` under `%USERPROFILE%\local-models\`, pinned in Session Config. `~\llama-cuda` b9608 stays the Whispering :8090 binary. The Vulkan winget `llama-server` on PATH is not used.

v1 context is 16,384. Raise to 32,768 only after a measured Session stays above the operational floor.

`--jinja` stays on. The artifact was first evaluated as shipped, and its added Blackfrost operational block was measured to turn greetings into unnecessary tool execution. The server therefore uses the upstream Qwen3.8 template pinned in Session Config. This is not a second alignment or refusal prompt: it removes only the vendor insertion while retaining Qwen's native tool, vision, and reasoning structure. See ADR 0017 and the performance report.
