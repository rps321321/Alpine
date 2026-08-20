# Minimal OpenCode is capability-preserving and effect-gated

The 16K Local Model uses the double-clickable `%USERPROFILE%\local-models\Open Local Qwen.exe`, with `Open Minimal OpenCode.cmd` retained as a transparent fallback. The EXE only selects and remembers a project and forwards reviewed arguments to `alpine.exe opencode`. Rust owns policy verification, the full-lifetime inference lease, Session acquisition/restoration, Ctrl-C cleanup, crash recovery, and redacted failure records. Native OpenCode output remains attached to the terminal instead of being redirected into a potentially sensitive transcript.

The minimal profile removes global skills, external plugins, the inherited global Claude prompt, project OpenCode configuration, and Convex MCP by default. It replaces OpenCode's large generic agent manual with a short neutral coding prompt and disables automatic session-title generation while explicitly retaining core read, edit, write/patch, search, shell, web, task, and todo tools. These are context and trust-surface controls, not topic or technique filters. Convex and trusted project OpenCode configuration are explicit opt-ins, and their merged effective policy must still pass every locked safety invariant.

The Harness Policy Boundary is effect-based:

- reasoning, code generation, local project work, research, reverse engineering, scanning, fuzzing, and exploitation techniques are not filtered;
- representative destructive filesystem and Git commands, personal-identity use, publication/deployment, and remote writes ask for operator consent;
- harmless Git inspection remains allowed;
- direct Read-tool access to personal credential stores is denied;
- credential-like environment variables are removed before OpenCode starts.

This supersedes ADR 0008 where it used permanent denial for operator-authorizable Git effects. Those operations now ask. Skill denial is separately classified as a reversible 16K context choice. OpenCode permissions are useful consent tripwires for ordinary tool calls, not adversarial mediation of every shell effect: interpreters, aliases, APIs, and custom executables can bypass string rules, and session persistence is not DLP. Hostile-agent containment belongs to an OS/VM isolation boundary.
