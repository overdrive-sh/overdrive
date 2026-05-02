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
Top-level variants (locked by ADR-0032 §3 Amendment 2026-04-30):

| Variant | Carries | Emitted when |
|---|---|---|
| `Accepted` | `spec_digest`, `intent_key`, `outcome` | First line, after IntentStore commit. |
| `LifecycleTransition` | `alloc_id`, `from`, `to`, `reason: TransitionReason`, `source`, `at` | Each ObservationStore AllocStatusRow transition. |
| `ConvergedRunning` | `alloc_id`, `started_at` | Allocation reaches Running and replicas_running ≥ desired. |
| `ConvergedFailed` | `alloc_id`, `terminal_reason: TerminalReason`, `error: Option<String>` | Backoff exhausted, server wall-clock cap hit, or unrecoverable driver error. |

`reason` is the cause-class `TransitionReason` enum from
`overdrive-core` per ADR-0032 §3 Amendment 2026-04-30. Phase 1 variants:

- **5 progress markers** — `Scheduling`, `Starting`, `Started`,
  `BackoffPending { attempt }`, `Stopped { by: StoppedBy }`.
- **9 cause-class failure variants** — `ExecBinaryNotFound { path }`,
  `ExecPermissionDenied { path }`, `ExecBinaryInvalid { path, kind }`,
  `CgroupSetupFailed { kind, source }`, `DriverInternalError { detail }`,
  `RestartBudgetExhausted { attempts, last_cause_summary }`,
  `Cancelled { by: CancelledBy }`, `NoCapacity { requested, free }`.
- **2 Phase 2 emit-deferred** (declared for forward-compat; ExecDriver
  Phase 1 does not currently observe these) — `OutOfMemory { peak_bytes,
  limit_bytes }`, `WorkloadCrashedImmediately { exit_code, signal,
  stderr_tail }`.

`terminal_reason` is the structured `TerminalReason` enum:
`BackoffExhausted { attempts, cause: TransitionReason }`,
`DriverError { cause: TransitionReason }`, `Timeout { after_seconds }`.

`source` is structured (`reconciler` | `driver(exec)` | future
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

The cause-class `TransitionReason` enum is the load-bearing artifact
for the journey's "told the truth" promise — typed payloads, not
free-form strings.

- **One source**: emitted by the lifecycle reconciler (for
  reconciler-domain causes — `Scheduling`, `Starting`, `BackoffPending`,
  `BackoffExhausted` (terminal), `NoCapacity { requested, free }`) or
  written by the action shim (for driver-domain causes — the shim
  classifies `DriverError::StartRejected.reason` text into a cause-class
  variant via a small prefix matcher per ADR-0032 §4 amended
  classification table; e.g. `"No such file or directory"` →
  `ExecBinaryNotFound { path }`, `"Permission denied"` →
  `ExecPermissionDenied { path }`, etc.). The verbatim driver text is
  preserved unchanged in `AllocStatusRow.detail` for audit.
- **Two consumers**: streaming `LifecycleTransition.reason` and
  `ConvergedFailed.terminal_reason.cause`; snapshot
  `last_transition.reason` and per-row `error` field.

Both surfaces serialise from the **same Rust enum** — byte-equality
across surfaces extends to the cause-class typed payload (`data: { path }`
on `ExecBinaryNotFound`, `data: { attempts, cause: { kind, data } }` on
`BackoffExhausted`, etc.), not just the `kind` discriminator. Drift
between the two consumption surfaces is structurally impossible by
construction; an integration test asserts byte-equality of the typed
payload as the regression-target — covered by explicit AC and an
`integration_validation.shared_artifact_consistency` entry on the
journey YAML.

---

## Cross-cutting: `restart_budget`

Source: lifecycle reconciler's private libSQL `view` (NextView) per
the §18 reconciler primitive contract. Already exists internally —
phase-1-first-workload step 5 introduced it. This feature exposes it
on the wire shape of `alloc_status_snapshot` (and the snapshot is the
ONLY surface it crosses; the streaming event does not need to carry
the full budget structure on every transition — the terminal
`ConvergedFailed { terminal_reason: BackoffExhausted { attempts,
cause } }` event carries the count and the structural cause on the
streaming surface, and the operator pivots to `alloc status` for the
human-readable `5 / 5 used (backoff exhausted)` rendering if curious).

---

## Out-of-band but referenced

| Artifact | Source | Why mentioned |
|---|---|---|
| `commit_index` | (DROPPED per ADR-0020) | Not used in any wire shape this feature touches. |
| `cgroup_path` | ProcessDriver — phase-1-first-workload step 6 | Not surfaced in streaming or snapshot. Visible via `systemctl status` for debugging. Out of scope here. |

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-30 | Initial DISCUSS shared-artifacts registry. |
| 2026-04-30 | Cause-class refactor of `TransitionReason` per ADR-0032 §3 Amendment 2026-04-30. `### convergence_event` variant catalogue updated from journey-level deferral ("DESIGN names") to the locked Phase 1 cause-class shape (5 progress markers + 9 cause-class + 2 Phase 2 emit-deferred). `## Cross-cutting: transition_reason` paragraph rewritten — typed payloads, not free-form strings; classifier described per ADR-0032 §4 amended; byte-equality assertion now covers the typed payload, not just the `kind` discriminator. |
