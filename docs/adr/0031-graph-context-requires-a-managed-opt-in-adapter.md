# ADR 0031: Graph context requires a managed, opt-in adapter

## Status

Accepted — 2026-08-24

## Context

Local coding models have materially smaller useful context windows than hosted
frontier models. A scoped dependency and symbol graph could reduce raw file
search and repeated context ingestion. Graphify 0.8.46 is installed on the
development machine and can extract a repository, query a bounded subgraph and
use an OpenAI-compatible local endpoint with concurrency limited to one.

The reviewed distribution is an MIT-licensed external Python CLI, not a stable
library contract owned by Alpine. Several installation commands edit AGENTS.md,
agent settings or Git hooks. Extraction can launch worker processes, write a
`graphify-out` tree and optionally call an LLM. Invoking an ambient executable
inside every Selected Project would therefore create hidden writes, resource
use and executable-identity drift at the same boundary ADR 0028 keeps explicit.

## Decision

Graph context is a planned Agent Runtime capability, but this release does not
run `graphify install`, add hooks, edit a Selected Project or expose the ambient
Graphify binary to Pi. Settings reports the capability as unavailable rather
than implying it is active.

A future Graph Context Adapter must be explicitly enabled per Desktop Project
Record and must:

- pin and report the exact Graphify distribution and digest;
- invoke an executable and argument vector directly, never through a shell;
- keep graph output in Alpine-managed app-local storage rather than the project;
- deny installer, hook and agent-configuration commands;
- use the selected project as read-only input and the active verified local
  Inference Session with concurrency one when semantic extraction is requested;
- bound elapsed time, process count, output bytes and model tokens, support
  cancellation and retain redacted local metrics;
- expose typed build, query and freshness results through the Desktop Interface;
- require Tool Approval before any requested project write or external network
  access.

Only bounded query results enter the Task context. The generated graph is a
replaceable cache, not durable Task truth, source truth or authorization.

## Alternatives considered

### Run `graphify install` for every project

Rejected. It mutates project instructions and hooks outside Alpine's typed
consent and recovery model.

### Let Pi call Graphify through `run_command`

Rejected as a product integration. An operator may still approve that exact
command, but it does not establish version pinning, cache ownership, freshness
or bounded context semantics.

### Ignore graph context entirely

Rejected. The context-efficiency need is real and the query model is promising;
the adapter remains a declared release slice with explicit safety gates.

## Consequences

The current app makes no false context-savings claim and gains no ambient Python
or hook authority. Shipping graph context requires a deliberate adapter and its
own lifecycle tests, but that work can reuse the existing Selected Project,
Inference Session, Task Event, metrics and Tool Approval boundaries.
