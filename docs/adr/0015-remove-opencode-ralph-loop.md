# Remove Ralph Loop from OpenCode

The OpenCode Ralph Loop plugin is removed from the v1 Harness. Its installed plugin repeatedly fails against OpenCode 1.18.18 with `content.match is not a function`, and its loop behavior is not required for the Repo Loop.

Remove the active OpenCode plugin configuration, the global `opencode-ralph-loop` npm package, and the Ralph-only OpenCode skills (`ralph-loop`, `cancel-ralph`, and its generic `help` skill). This supersedes the part of ADR 0005 that retained Ralph Loop. Convex MCP and the 16,384-token Inference Session context remain unchanged.
