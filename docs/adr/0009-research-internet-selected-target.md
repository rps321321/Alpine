# v1 does not gate destinations; techniques are not filtered

Ordinary research Internet is open. v1 also does **not** implement a Selected Target allowlist. Destination control is operator judgment, not a Harness Policy rule. OpenCode runs as the Windows user and cannot provide a network namespace; we will not fake one with a host list we cannot enforce.

Technique-looking-offensive is not a deny reason. Scanning, fuzzing, exploitation, reverse engineering, and similar stay available. Hostile-environment containment is the Attack Lab.

SSH still must not silently inherit personal `~/.ssh`. Any SSH before the Attack Lab uses an explicit Test Credential. That is identity isolation, not a destination allowlist.
