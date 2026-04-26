# KPI Instrumentation — phase-1-control-plane-core

**Status**: DEVOPS wave artifact. Maps each outcome KPI from
`discuss/outcome-kpis.md` (K1–K7) to a named `tracing` span with
structured fields, suitable for Phase 2 DuckLake ingestion without
re-instrumentation.

**Scope in Phase 1**: instrument only. The subscriber is `tracing`'s
default `fmt::Layer` writing to stderr. No external collector. No
Prometheus, OTLP export, or dashboards.

**Phase 2 hook**: field names and span boundaries chosen so the
DuckLake ingestion work (whitepaper §12) can consume them unchanged.
A Phase 2 DuckLake Parquet schema would use the span names as table
names and the field names as columns.

**Not in scope**: dashboards, SLOs, alerting thresholds,
burn-rate alerts, retention policies — all Phase 2+ DuckLake work
per wizard decision.

---

## Instrumentation conventions

- **Span naming**: `control_plane.<subsystem>.<operation>` or
  `reconciler.<operation>`. Two-dot segments; snake_case.
- **Level**: `INFO` for operation boundaries; `DEBUG` for intra-span
  detail; `WARN` for KPI guardrail breaches (e.g., non-monotonic
  commit index); `ERROR` for handler failures that the CLI surfaces.
- **Field types**: match the Rust newtype shape. `job_id: &JobId`
  (Display impl), `commit_index: u64`, `spec_digest: &ContentHash`
  (hex Display impl), `duration_ms: u64`.
- **Error fields**: `error.class` (enum variant name),
  `error.message` (Display), `error.field` (Option<String> for
  validation errors).
- **Span relationship**: the CLI emits a `cli.<command>` span that
  wraps the HTTP call; the server emits a `control_plane.<handler>`
  span inside the request. DuckLake can correlate via
  `trace_id` (std `tracing` span context) without any additional
  machinery.

---

## Per-KPI instrumentation design

### K1 — Round-trip spec-digest byte-identity

**Observable**: SHA-256 digest of the submitted spec matches what
`alloc status` returns. Measured by acceptance test subprocess +
proptest variant.

**Tracing design**:
- Span: `control_plane.job_submit`
- Level: INFO
- Fields: `job_id` (SpiffeId Display), `intent_key` (str),
  `commit_index` (u64), `spec_digest` (ContentHash hex),
  `spec_bytes_len` (u64), `idempotent_hit` (bool — true if same
  digest re-submitted per ADR-0015)

- Span: `control_plane.alloc_status`
- Level: INFO
- Fields: `job_id` (SpiffeId Display), `spec_digest`
  (ContentHash hex), `resolved_from` (str — "intent" | "observation"),
  `rows_returned` (u64)

**Instrumentation site**:
- `overdrive-control-plane::handlers::submit_job` (axum handler,
  ADR-0008) — emits `control_plane.job_submit`.
- `overdrive-control-plane::handlers::alloc_status` — emits
  `control_plane.alloc_status`.

**Phase 2 ingestion**: DuckLake `control_plane_ops` table. K1 query:
any `job_id` whose `spec_digest` varies across submit vs describe
rows is a K1 violation.

---

### K2 — Invalid spec rejected before IntentStore write (400)

**Observable**: validating-constructor failure → HTTP 400 → no store
side effect.

**Tracing design**:
- Span: `control_plane.job_submit` (same span as K1)
- Additional fields on rejected requests:
  - `error.class` = enum variant name from `ControlPlaneError`
    (e.g. `"Validation"`)
  - `error.field` = optional field name from `ErrorBody`
  - `status_code` = 400
  - `store_write_attempted` = false (proves no IntentStore call)

**Instrumentation site**:
- `overdrive-control-plane::handlers::submit_job` — fields added on
  the early-return error branch.
- `overdrive-control-plane::error::to_response` (ADR-0015) — emits
  `control_plane.error_mapping` span tagged with variant + status
  mapping.

**Phase 2 ingestion**: K2 query = count rows where
`error.class = "Validation"` AND `store_write_attempted = true`.
Expected zero; any non-zero is a K2 regression.

---

### K3 — `commit_index` strictly monotonic

**Observable**: N successive submits produce strictly-increasing
`commit_index` values. Proptest in `overdrive-store-local` or server
crate.

**Tracing design**:
- Span: `control_plane.job_submit` (same span as K1/K2)
- Additional field: `prev_commit_index` (Option<u64>) —
  the index observed immediately before this submit
- Invariant check: if `commit_index <= prev_commit_index`, emit
  `WARN` with `kpi.k3.violation = true`

**Instrumentation site**:
- `overdrive-control-plane::handlers::submit_job` after
  `IntentStore::txn` returns the new commit index; reads the
  previous via `LocalStore::commit_index()` accessor (DESIGN
  wave-decisions §Reuse Analysis).

**Phase 2 ingestion**: K3 query = any span whose
`commit_index <= prev_commit_index` across the same data dir. The
span field suffices; no separate "monotonic violation" event.

---

### K4 — DST invariants for reconciler primitive

**Observable**: three new DST invariants
(`at_least_one_reconciler_registered`,
`duplicate_evaluations_collapse`, `reconciler_is_pure`) pass on every
run of `cargo xtask dst`.

**Tracing design**: the DST harness produces
`target/xtask/dst-summary.json` per invariant (phase-1-foundation
ADR-0006). K4 does NOT need additional `tracing` spans — the existing
DST summary file IS the structured signal.

Spans emitted by the reconciler runtime at runtime (not DST):
- Span: `reconciler.register`
- Level: INFO
- Fields: `reconciler_name` (ReconcilerName Display),
  `libsql_path` (str)

- Span: `reconciler.evaluation_enqueued`
- Level: DEBUG
- Fields: `reconciler_name`, `target_resource` (str),
  `correlation_key` (str), `collapsed_into_pending` (bool — broker
  collapse signal)

**Instrumentation site**:
- `overdrive-control-plane::reconciler_runtime::ReconcilerRuntime::
  register` — emits `reconciler.register`.
- `overdrive-control-plane::eval_broker::EvaluationBroker::submit` —
  emits `reconciler.evaluation_enqueued` with `collapsed_into_pending`
  for cancelable-eval-set behaviour.

**Phase 2 ingestion**: K4's observability in Phase 2 becomes a
DuckLake table of broker events; ratio of `collapsed_into_pending =
true` to total submissions is the storm-mitigation-working metric.

---

### K5 — `cluster status` surfaces reconciler registry + broker counters

**Observable**: `overdrive cluster status` prints the
noop-heartbeat reconciler + broker counters. Acceptance test asserts
the section layout.

**Tracing design**:
- Span: `control_plane.cluster_status`
- Level: INFO
- Fields: `reconcilers_registered_count` (u64),
  `broker_pending_count` (u64),
  `broker_total_enqueued` (u64),
  `broker_total_collapsed` (u64),
  `broker_total_reaped` (u64)

**Instrumentation site**:
- `overdrive-control-plane::handlers::cluster_status` — reads from
  `ReconcilerRuntime::registered()` and
  `EvaluationBroker::counters()` (DISTILL DWD-06 scaffold surface).

**Phase 2 ingestion**: counters become time-series in DuckLake.
K5's stable-integer fields (not just counts-at-query-time) let Phase
2 compute broker efficiency over time without re-instrumenting.

---

### K6 — Error paths answer "what / why / how to fix"

**Observable**: 100% of error paths (connection refused, invalid
spec, unknown JobId, server internal) render actionable CLI output.

**Tracing design**:
- Span: `cli.<command>` (e.g. `cli.job_submit`, `cli.alloc_status`)
- Level: INFO; ERROR on failure
- Fields on failure: `error.class` (CliError variant name),
  `error.message` (Display),
  `error.remediation` (static str — the "how to fix" hint),
  `exit_code` (u8)

**Instrumentation site**:
- CLI command handlers in `crates/overdrive-cli/src/commands/*.rs` —
  one span per top-level CLI subcommand; error arms set the
  `error.*` fields before `std::process::exit`.
- `overdrive-cli::client::CliError::remediation()` — a method
  that returns the static remediation string per variant, also
  printed to stderr for the operator. The `tracing` field is the
  same string.

**Phase 2 ingestion**: K6 does not primarily live in DuckLake; it
lives in the CLI acceptance tests. The `tracing` field exists so
that Phase 2 can grep real-world usage for
`error.remediation IS NULL` — a variant that added an
error-class without a remediation hint.

---

### K7 — Empty observations render explicit empty state

**Observable**: zero-nodes, zero-allocations, zero-history cases
produce explicit empty-state text, not a blank table.

**Tracing design**:
- Span: `control_plane.alloc_status` | `control_plane.node_list` |
  `control_plane.cluster_status` (reuses K1, K5 spans above)
- Additional field on empty-result branches: `rows_returned = 0`,
  `empty_state_rendered = true` (on CLI side)

CLI side:
- Span: `cli.node_list` | `cli.alloc_status` | `cli.cluster_status`
- Fields: `rows_rendered` (u64), `empty_state_path_taken` (bool)

**Instrumentation site**:
- Server: existing handler spans (K1, K5) — `rows_returned` field
  already present.
- CLI: `overdrive-cli::commands::<command>` — emits
  `empty_state_path_taken` on the zero-row render branch.

**Phase 2 ingestion**: K7 query = count spans where
`rows_returned = 0` AND `empty_state_path_taken = false`.
Expected zero; any non-zero means the CLI printed a blank table
against a zero-row response — a K7 regression.

---

## Guardrail spans (inherited, not new)

Phase-1-foundation guardrails apply unchanged:

- **DST wall-clock <60s on reference laptop** — enforced by the `dst`
  CI job's 10-minute timeout + local K1 contract. No new span.
- **Lint-gate false-positive rate = 0** — the `dst-lint` job in CI.
  No new span.
- **Snapshot round-trip byte-identical** — integration-gated
  proptest in `overdrive-store-local`. No new span; the proptest
  outcome is the gate.

---

## Subscriber configuration (Phase 1)

The `overdrive serve` binary initialises `tracing-subscriber` with:

- `tracing_subscriber::fmt::Layer` → stderr, compact format
- `EnvFilter` → defaults to `info,overdrive=debug` (env override
  via `RUST_LOG` / `OVERDRIVE_LOG`)
- No OTLP exporter, no Prometheus exporter, no DuckLake exporter

The CLI binary (`overdrive`) uses the same subscriber shape so
`cli.<command>` spans appear in operator stderr identically to how
server spans appear in `serve` stderr. This is intentional —
correlation across the CLI/server boundary is manual
(timestamps + correlation keys) in Phase 1; Phase 2 DuckLake work
owns the machine-correlated view.

---

## Phase 2 handoff

When Phase 2 lands DuckLake:

1. The span names above become Parquet table names (one table per
   span, or one table with a `span_name` column — a Phase 2 schema
   decision).
2. The field names above become columns, with type hints from their
   Rust types.
3. A DuckLake subscriber (`tracing-subscriber`-compatible) replaces
   the `fmt::Layer` in `overdrive serve`.
4. Phase 2 re-validates each K1–K7 query against real DuckLake
   data — any field name change required at that point is an
   instrumentation change, not a KPI redefinition.

**Nothing in this file pins a Phase 2 schema.** The goal is
"instrument once, ingest later, don't re-instrument."

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial KPI-to-tracing-span map for phase-1-control-plane-core. K1–K7 each mapped to a named span + structured fields. Phase 2 DuckLake ingestion path noted but not implemented. |
