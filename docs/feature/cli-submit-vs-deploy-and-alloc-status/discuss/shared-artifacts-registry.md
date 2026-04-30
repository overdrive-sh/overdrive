# Shared artifacts registry — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISCUSS / Phase 2.5
**Owner**: Luna
**Date**: 2026-04-30

Single source of truth per artifact. Drift across consumers is a
defect.

---

## Inherited unchanged

| Artifact | Source | Consumers (this feature) |
|---|---|---|
| `spec_digest` | `ContentHash::of(rkyv_archived_job_bytes)` — `overdrive-core` | Streaming submit first event (1a), terminal summary (1c), `alloc status` snapshot (4) |
| `intent_key` | overdrive-core canonical form — already shipped | Streaming submit first event (1a), `--detach` JSON response |
| `alloc_id` | `overdrive-core::aggregate::Allocation::id` — emitted by scheduler in `Action::StartAllocation` | LifecycleTransition events (1b), terminal events (1c), `alloc status` rows (4) |
| `alloc_state` | `overdrive-core::traits::driver::AllocationState` (Pending / Running / Draining / Terminated / Failed) | LifecycleTransition events (1b), `alloc status` STATE column (4) |
| `outcome` | `IdempotencyOutcome` per ADR-0020 (Inserted / Unchanged) | First NDJSON event (1a), `--detach` JSON response |

---

## New in this feature (DESIGN owns precise types)

### `convergence_event` — typed NDJSON line

Source: typed enum in `overdrive-control-plane::api` per ADR-0014.
Variants (DESIGN names; this is the journey-level shape):

| Variant | Carries | Emitted when |
|---|---|---|
| `Accepted` | `spec_digest`, `intent_key`, `outcome` | First line, after IntentStore commit. |
| `LifecycleTransition` | `alloc_id`, `from`, `to`, `reason`, `source`, `at` | Each ObservationStore AllocStatusRow transition. |
| `ConvergedRunning` | `alloc_id`, `started_at` | Allocation reaches Running and replicas_running ≥ desired. |
| `ConvergedFailed` | `alloc_id`, `terminal_reason`, `error` | Backoff exhausted, server wall-clock cap hit, or unrecoverable driver error. |

`source` is structured (`reconciler` | `driver(process)` | future
driver kinds), not a free string.

Consumers:
- Server: streaming endpoint emits as NDJSON.
- CLI: line-delimited reqwest reader, renders to stdout, maps
  terminal variants to exit codes.
- Future TUI mode (out of scope) consumes the same stream.

### `alloc_status_snapshot` — typed snapshot struct

Source: typed struct in `overdrive-control-plane::api` per ADR-0014.
Likely extension of the existing `AllocStatusResponse` (DESIGN
decides whether to extend in place or version). Fields (journey
level):

| Field | Sourced from |
|---|---|
| `job_id`, `spec_digest`, `replicas_desired`, `replicas_running` | DescribeJob (existing) |
| `allocations: Vec<AllocSummary>` | Each summary: `alloc_id`, `state`, `resources`, `started_at`, `exit_code` |
| `last_transition: Option<TransitionRecord>` | `from`, `to`, `reason`, `source`, `at` — same shape as the streaming `LifecycleTransition` event |
| `restart_budget: Option<RestartBudget>` | `used`, `max`, `exhausted: bool` from lifecycle reconciler private libSQL view |

Consumers:
- Server: `GET /v1/alloc/status?job=...` handler.
- CLI: `alloc status` renderer.

---

## Cross-cutting: `transition_reason`

The `reason` string is the load-bearing artifact for the journey's
"told the truth" promise.

- **One source**: emitted by the lifecycle reconciler (when reason
  is reconciler-domain, e.g. `scheduling on local`,
  `backoff_exhausted`) or by the ProcessDriver via the action shim
  (when reason is driver-domain, e.g. the verbatim
  `stat /usr/local/bin/payments: no such file or directory`).
- **Two consumers**: streaming `LifecycleTransition` and
  `ConvergedFailed` events; snapshot `last_transition` and per-row
  `error` field.

Drift between the two consumption surfaces is a defect — covered by
explicit AC and an `integration_validation.shared_artifact_consistency`
entry on the journey YAML.

---

## Cross-cutting: `restart_budget`

Source: lifecycle reconciler's private libSQL `view` (NextView) per
the §18 reconciler primitive contract. Already exists internally —
phase-1-first-workload step 5 introduced it. This feature exposes it
on the wire shape of `alloc_status_snapshot` (and the snapshot is the
ONLY surface it crosses; the streaming event does not need to carry
the full budget structure on every transition — the terminal
`ConvergedFailed` event with `terminal_reason: backoff_exhausted` is
the streaming surface, and the operator pivots to `alloc status` for
the count if curious).

---

## Out-of-band but referenced

| Artifact | Source | Why mentioned |
|---|---|---|
| `commit_index` | (DROPPED per ADR-0020) | Not used in any wire shape this feature touches. |
| `cgroup_path` | ProcessDriver — phase-1-first-workload step 6 | Not surfaced in streaming or snapshot. Visible via `systemctl status` for debugging. Out of scope here. |
