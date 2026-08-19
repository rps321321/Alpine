# Git writes are a Harness Policy Boundary, not a prompt

Inside the Selected Project the Harness may inspect Git, create/switch a work branch, stage, and commit. It may not push, force-push, rewrite history, `reset --hard` over uncommitted user work, delete unmerged branches, edit hooks, change remotes, change global/system Git config, read credentials, or operate on another repo merely because it is on disk.

These rules live in OpenCode permissions. That is a Harness Policy Boundary: meaningful, deterministic, and still not kernel isolation. OpenCode runs as the Windows user. We will not add a git wrapper just to look stronger.
