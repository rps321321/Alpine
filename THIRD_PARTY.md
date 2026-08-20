# Third-party boundaries

Project Alpine's Apache-2.0 license applies to Alpine's own source. It does not relicense model weights, model metadata, `llama.cpp`, CUDA, OpenCode, downloaded runtimes, or dependencies.

## Referenced and patched projects

- `llama.cpp` is Copyright (c) 2023-2026 The ggml authors and is licensed under the MIT License. Alpine carries a source patch in `patches/` and preserves the upstream notice in `third_party/llama.cpp-LICENSE`; any distribution of `llama.cpp` or a derived runtime must retain it.
- Qwen models, chat templates, projectors, and metadata are external artifacts. Their applicable upstream model and repository licenses must be reviewed independently before redistribution. Alpine v0.1 does not redistribute them.
- NVIDIA CUDA libraries and tools are external software governed by NVIDIA's terms. Alpine v0.1 does not redistribute them.
- OpenCode is an external program. Alpine integrates through its command-line and configuration interfaces and does not redistribute it in v0.1.

## Source dependencies

`Cargo.lock` pins the resolved Rust dependency graph. Direct Rust dependencies and the license expressions reported by their locked package metadata are:

| Package | Locked version | License expression |
| --- | ---: | --- |
| clap | 4.6.6 | MIT OR Apache-2.0 |
| ctrlc | 3.5.2 | MIT/Apache-2.0 |
| fs2 | 0.4.3 | MIT/Apache-2.0 |
| getrandom | 0.4.3 | MIT OR Apache-2.0 |
| regex | 1.13.1 | MIT OR Apache-2.0 |
| rusqlite | 0.40.2 | MIT |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| sha2 | 0.11.0 | MIT OR Apache-2.0 |
| sysinfo | 0.36.1 | MIT |
| tempfile | 3.27.0 | MIT OR Apache-2.0 |
| thiserror | 2.0.20 | MIT OR Apache-2.0 |
| ureq | 3.4.0 | MIT OR Apache-2.0 |
| uuid | 1.24.1 | Apache-2.0 OR MIT |

Transitive dependencies retain their upstream notices and license files in the Cargo source distribution. Before any binary release, regenerate a complete dependency notice/SBOM from the exact locked graph and audit the binary's complete redistributed closure. This source-only v0.1 does not claim that future binary redistribution has already passed that audit.
