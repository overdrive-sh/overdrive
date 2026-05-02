# C4 Level 1 — System Context

**Wave**: DESIGN
**Date**: 2026-04-30

This is a feature-scoped re-render of the brief.md system-context
diagram with the new actors (CI / pipeline calling `submit --detach`)
made explicit. Nothing on this diagram is new at the *system* level;
the boundary is preserved per brief.md §1.

```mermaid
C4Context
  title System Context — Overdrive (cli-submit-vs-deploy-and-alloc-status)

  Person(operator, "Platform Operator (Ana)", "Senior SRE; runs `overdrive job submit` from a TTY for the inner-loop deploy-observe-fix cycle")
  Person(ci, "CI / automation script", "Runs `overdrive job submit --detach` (or relies on pipe auto-detach) for fire-and-forget commits in workflows")
  System(overdrive, "Overdrive (single-node, Phase 1)", "One binary; control-plane + worker subsystems co-located. Streaming `POST /v1/jobs` over HTTP/2+rustls; `GET /v1/allocs?job=...` snapshot endpoint")
  System_Ext(redb, "Local filesystem (redb)", "Backs LocalStore (intent) and LocalObservationStore (observation)")
  System_Ext(kernel, "Linux kernel", "cgroup v2 hierarchy + tokio::process child for ExecDriver workloads")
  System_Ext(workload, "Workload process", "The fork/execed binary the operator submitted (e.g. /usr/local/bin/payments)")

  Rel(operator, overdrive, "Submits jobs via streaming `POST /v1/jobs`; reads `GET /v1/allocs?job=...` snapshots", "HTTPS / NDJSON")
  Rel(ci, overdrive, "Submits jobs via `POST /v1/jobs` with `Accept: application/json` (auto-detach or `--detach`)", "HTTPS / single JSON")
  Rel(overdrive, redb, "Persists job intent + observation rows to")
  Rel(overdrive, kernel, "Manages `overdrive.slice/workloads.slice/<alloc>.scope` cgroup; spawns/stops workload processes")
  Rel(kernel, workload, "Hosts as child of overdrive-worker")
```

## Notes

- The two human/agent actors (operator and CI) are functionally
  identical from the server's perspective — they're both clients of
  `POST /v1/jobs`. They're shown distinctly because the **default
  experience differs**: the operator sees streaming NDJSON; CI sees a
  single JSON object. This reflects [D5] (CLI-side TTY detection)
  and US-03 / US-04.
- No external system was added by this feature. The boundary is
  unchanged from brief.md §C4 Level 1; this re-render highlights the
  two consumer profiles for the streaming endpoint.
- The redb and kernel external systems are inherited unchanged from
  brief.md C4 Level 2; they're shown here as system-level
  dependencies because the streaming and snapshot endpoints both
  ultimately read from / write to them.
