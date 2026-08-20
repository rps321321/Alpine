# Support and qualification

Project Alpine separates eligibility from demonstrated production behavior.

## Support Envelope

The versioned Support Envelope in `config/support-envelope.json` describes an environment that Alpine knows how to inspect and evaluate. For v0.1, that envelope is Windows x86-64 with the declared NVIDIA/OpenCode probes. Passing envelope inspection means **eligible to evaluate**. It does not establish production support, model quality, compatibility with every NVIDIA GPU, or safe operation.

## Verified Deployment

A Verified Deployment is one exact seven-dimensional Evidence Identity—hardware, Alpine software, model, runtime, workload suite, material configuration, and policy—that passed the applicable automated evidence gates and the substantive human Capability Review. Any change to a material identity dimension can stale the evidence.

Project Alpine currently provides an evaluation path for its declared Windows/NVIDIA environment. Production qualification may be claimed only for explicitly recorded deployments that passed the current qualification path. Other machines and configurations remain unverified until independently qualified.

The repository does not currently claim general Windows + NVIDIA production support, hardware-vendor certification, or guaranteed support for systems that merely resemble a qualified deployment.

## Getting help

Use GitHub Discussions or an issue for reproducible setup and behavior questions once those facilities are public. Include Alpine version/commit, non-sensitive profile settings, command output with secrets and personal paths removed, and whether the environment merely passed the Support Envelope or has its own qualification evidence.

Use the private process in `SECURITY.md` for vulnerabilities.
