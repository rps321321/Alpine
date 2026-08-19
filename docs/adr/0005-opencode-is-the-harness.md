# OpenCode is the v1 Harness

v1 uses the already-installed OpenCode 1.18.16. The local llama-server is added as a provider that reads Session Config, without disturbing the existing Ralph-loop plugin or Convex MCP unless a conflict is demonstrated. We will not install Cline, Roo, Continue, Aider, or an Anthropic-protocol bridge to create optionality. The abstraction is “OpenCode + configurable local OpenAI-compatible endpoint,” not a hard-coded port.
