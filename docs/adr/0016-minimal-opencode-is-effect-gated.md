# Minimal OpenCode is capability-preserving and effect-gated

The 16k Local Model uses the double-clickable `%USERPROFILE%\local-models\Open Local Qwen.exe`, with `Open Minimal OpenCode.cmd` retained as a transparent fallback. The EXE only selects and remembers one project before calling the reviewed PowerShell policy engine. The launcher starts the configured Inference Session when needed, opens OpenCode on one Selected Project, and restores the previous inference/cleanup state when it exits.

The minimal profile removes global skills, external plugins, the inherited global Claude prompt, project OpenCode configuration, and Convex MCP by default. It replaces OpenCode's large generic agent manual with a short neutral coding prompt, disables automatic session-title generation, and hides Task/Todo orchestration schemas while retaining core file, search, shell, and web tools. These are context and trust-surface controls, not topic or technique filters. Convex and trusted project OpenCode configuration are explicit `-WithConvex` and `-WithProjectConfig` opt-ins.

The Harness Policy Boundary is effect-based:

- reasoning, code generation, local project work, research, reverse engineering, scanning, fuzzing, and exploitation techniques are not filtered;
- common destructive filesystem and Git actions, personal-identity use, publication/deployment, and remote writes ask for operator consent;
- harmless Git inspection remains allowed;
- direct Read-tool access to personal credential stores is denied;
- credential-like environment variables are removed before OpenCode starts.

This supersedes ADR 0008 where it used permanent denial for operator-authorizable Git effects. Those operations now ask. It also narrows ADR 0011's description: OpenCode permissions are useful consent tripwires for ordinary tool calls, not adversarial mediation of every shell effect. Hostile-agent containment still belongs to the later Attack-Lab Isolation Boundary.
