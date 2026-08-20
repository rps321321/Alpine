# Qualification and deployment operations

Profiles, Qualification, and Deployment answer different questions:

- A Profile is immutable inference material such as runtime, context, batching, cache, and speculative-decoding settings.
- Qualification is an evidence-backed fact about one exact Evidence Identity.
- Deployment assigns the mutable `daily_default` and `rollback_profile` roles through append-only events.

Running an evaluation or Qualification never changes deployment roles.

## Default and one-session override

Omitting `--profile` uses the current deployment `daily_default`:

```powershell
alpine opencode --project C:\path\to\project
```

Supplying `--profile stable-16k` is a one-session override. It does not edit Session Config, alter deployment history, demote another Profile, or create an incident.

## Promotion

Promotion re-runs current production Qualification under an exclusive deployment lock, checks the exact current Profile bytes and expected current default, refuses unresolved suspensions, then atomically appends a hash-chained Promotion event:

```powershell
alpine promote --profile turbo-16k --expected-daily-default stable-16k `
  --final-run-id <final> --tuning-run <baseline> `
  --operator <operator> --reason <substantive-reason>
```

Turbo must not be promoted until the user has completed and recorded the substantive Capability Review and a fresh production Qualification passes. Promotion is an explicit operator action; evaluation cannot invoke it.

## Rollback

Rollback is a consequential deployment change. It requires the current default and exact Promotion event ID, restores the configured rollback Profile as `daily_default`, and appends a suspension or revocation:

```powershell
alpine rollback --expected-daily-default turbo-16k `
  --promotion-event-id <event-id> --disposition suspended `
  --operator <operator> --reason <reason>
```

Historical Qualification remains historical evidence; rollback does not rewrite it. A suspended/revoked Profile cannot be promoted again until the suspension is explicitly resolved and fresh current evidence passes. There is no automatic re-promotion.

## Incident

An Incident records contradictory operational evidence and opens a suspension without inherently changing `daily_default`:

```powershell
alpine incident --profile turbo-16k --promotion-event-id <event-id> `
  --operator <operator> --reason <observed-contradiction>
```

Testing, maintenance, and an ordinary temporary Profile override are not incidents. Resolve an investigated suspension explicitly with `alpine resolve-incident`; resolution does not promote the Profile.

Deployment event JSON files live under the generated installation's `deployment/events/` directory. Alpine validates sequence, filenames, event IDs, role transitions, references, and the previous-event hash chain whenever it derives state. These controls make Alpine fail closed against accidental corruption and application-level mutation; they are not a cryptographic defense against an administrator rewriting the final local event and all later anchors.
