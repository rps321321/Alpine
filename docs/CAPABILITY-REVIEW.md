# Capability Review contract

The Capability Review is human evidence about whether the complete production workflow is useful and trustworthy. It complements deterministic automated gates; it is not a fixed-answer benchmark and Alpine does not automate the reviewer's judgment.

## Required structure

The input is a schema-1 JSON document with:

- `reviewer_role`: a stable role label suitable for public structural export; private reviewer identity is supplied separately with `--reviewed-by`.
- `scenarios`: 8 to 64 reviewer-chosen realistic scenarios. The eight required categories are `trivial-conversation`, `repository-orientation`, `diagnosis`, `scoped-code-change`, `longer-horizon-work`, `web-research`, `permission-boundary`, and `session-lifecycle`.
- `accepted_residual_risks`: explicitly identified risks with a description and rationale.
- `final_decision`: `approved`, `rejected`, or `inconclusive`.
- `final_rationale`: the reviewer's substantive conclusion.

Every scenario records a unique ID, category, actual task, expected capability, observed behavior, `pass`/`fail`/`inconclusive` outcome, limitations, and optional supporting-artifact SHA-256. A passing scenario has no disposition. A failed or inconclusive scenario must be either `blocking` or `accepted-risk` and include a rationale. An accepted-risk disposition must reference declared residual-risk IDs. An approved review cannot contain a blocking scenario.

The validator checks schema, completeness, internal consistency, artifact-digest format, required-category coverage, and exact Evidence Identity binding. It does not decide whether a limitation is acceptable. That decision remains with the named human reviewer.

## Private record and attestation

The reviewer writes the document after exercising the exact candidate production workflow and records it with:

```powershell
cargo run --release --bin alpine -- record-evidence <final-run-id> `
  --kind operator-reviewed-capability-report `
  --evidence C:\private\capability-review.json `
  --reviewed-by "private reviewer label"
```

Do not put the private review document, prompts, transcripts, repository content, or reviewer identity into Git. Recording a review is a human attestation action; agents must not submit or sign it on the reviewer's behalf.

## Public projection

After the production Qualification is complete, generate an allowlisted projection:

```powershell
cargo run --release --bin alpine -- public-evidence `
  --final-run-id <final-run-id> --tuning-run <tuning-run-id> `
  --output public-evidence.json
```

This command re-runs current production Qualification and emits only the qualification identity, reviewer role, category coverage, outcome/disposition counts, final decision, and verified artifact digests. It does not copy and redact the private document. Tasks, prompts, observations, limitations, risk narratives, transcripts, repository content, and the private reviewer label have no field in the public schema. Any public narrative about a limitation or accepted risk must be authored separately and explicitly approved.
