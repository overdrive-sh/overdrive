# Reuse Analysis — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DESIGN
**Date**: 2026-04-30

Hard gate per `nw-architecture-patterns`: every CREATE NEW row carries
a justification that documents what was searched for, what was found,
and why extending was not viable. The default disposition is EXTEND.

---

## Existing surfaces consulted

Searched (Glob/Grep on the workspace):

- `crates/overdrive-control-plane/src/api.rs` — wire types
- `crates/overdrive-control-plane/src/handlers.rs` — `submit_job`,
  `alloc_status` handlers
- `crates/overdrive-control-plane/src/action_shim.rs` — `dispatch`,
  `dispatch_single`, `build_alloc_status_row`
- `crates/overdrive-control-plane/src/reconciler_runtime.rs` — tick
  loop, broker, driver injection
- `crates/overdrive-core/src/traits/observation_store.rs` — `AllocStatusRow`,
  `AllocState`, `LogicalTimestamp`, `ObservationStore`
- `crates/overdrive-core/src/traits/driver.rs` — `Driver`,
  `DriverError`, `DriverType`, `Resources`, `AllocationSpec`,
  `AllocationHandle`
- `crates/overdrive-core/src/reconciler.rs` — `JobLifecycleView`,
  `Reconciler` trait, `Action`, `TickContext`, `AnyState`,
  `AnyReconcilerView`
- `crates/overdrive-cli/src/client.rs` (and CLI submit handler) — HTTP
  client; existing `submit` command shape

Searched for prior art on:
- `TransitionReason`, `transition_reason` — **none found**.
- `LifecycleEvent`, `subscribe_to_alloc` — **none found**.
- `Failed` variant on `AllocState` — **not present**; `Terminated`
  used as today's catch-all.
- NDJSON, `application/x-ndjson` — **none found**.
- `tokio::sync::broadcast` — used elsewhere (e.g.
  `LocalStore::watch`, the existing intent-watch path); the pattern
  is in-house already.
- `IsTerminal` / TTY detection — not used in any current CLI command.

---

## Reuse table

| Surface | Disposition | Rationale | Affected slice |
|---|---|---|---|
| `POST /v1/jobs` HTTP path | **EXTEND** (no path change; gain a second response media type) | [C2] back-compat: no Accept → existing JSON ack. New endpoint would split traffic; ADR-0008 §versioning rule already mandates additive `/v1` evolution. | Slice 02 |
| `submit_job` axum handler signature | **EXTEND** (split into `submit_job_dispatch` that branches on `Accept` and calls the existing JSON path or a new streaming path) | The handler already does the IntentStore commit + spec_digest derivation. Both lanes need that prefix; duplicating is the antipattern. | Slice 02 |
| `SubmitJobRequest`, `SubmitJobResponse`, `IdempotencyOutcome` | **REUSE** unchanged | Same JSON ack body shape on the `application/json` lane; first NDJSON event on the streaming lane carries the same three fields (just inside `SubmitEvent::Accepted`). | Slice 02 |
| `AllocStatusResponse`, `AllocStatusRowBody` | **EXTEND in place** ([D2]) | New fields are pure additions; existing fields unchanged. Single-cut migration discipline ([C9]) means no v1/v2. The existing `pending_with_reason` constructor extends mechanically. | Slice 01 |
| `AllocStatusRow` (`overdrive-core`) | **EXTEND** (gain `reason: Option<TransitionReason>` and `detail: Option<String>`) | [C6] is unimplementable without a single source of truth that flows through both surfaces; the row IS that lineage. rkyv archive shape is additive. | Slice 01 |
| `AllocState` enum | **EXTEND** (gain `Failed` variant) | Today, the action shim collapses driver failures to `Terminated`, conflating "stopped" with "could not start." Adding `Failed` lets the CLI render distinct terminal status. Three-site change. | Slice 01 |
| `DriverError::StartRejected { reason: String }` | **REUSE** unchanged; consume the previously-discarded `reason` | The `reason: _` pattern in `dispatch_single` (action_shim.rs L117, L148) is a current-state code smell — the field exists but its value is dropped. Capturing it into `AllocStatusRow.detail` is a one-line amendment. | Slice 01 |
| `JobLifecycleView.restart_counts: BTreeMap<AllocationId, u32>` | **REUSE** as-is | Already tracks per-alloc attempt counts. Snapshot's `restart_budget.used` reads `restart_counts.values().sum::<u32>()`; `exhausted` derives from `>= RESTART_BUDGET_MAX`. No new state needed. | Slice 01 |
| `RESTART_BUDGET_MAX = 5` constant | **REUSE** as-is, surface as `RestartBudget.max` on the wire | Existing literal in `JobLifecycle::reconcile`; the snapshot exposes it instead of duplicating. | Slice 01 |
| `tokio::sync::broadcast` channel pattern | **REUSE** of the workspace pattern | `LocalStore::watch` and the existing `noop-heartbeat` invariant evaluator both use broadcast channels. Adding one for `LifecycleEvent` is consistent with the in-house pattern, not a new dependency. | Slice 02 |
| `clock.sleep(...)` via `Arc<dyn Clock>` | **REUSE** of the ADR-0013 §2c trait | The wall-clock cap timer rides on the same `Clock` injection path the reconciler runtime uses. Production = `SystemClock`; DST = `SimClock`. No new time source, no new DST seam. | Slice 02 |
| `xtask openapi-check` CI gate | **REUSE** unchanged | The new media-type entry on `submit_job` is captured by `utoipa`; the existing CI gate catches drift. No new xtask command. | Slice 02 |
| `dst-lint` banned-API gate | **REUSE** unchanged | Already enforces `Instant::now()` / `tokio::time::sleep` bans on core-class crates. The streaming handler uses `clock.sleep`; `dst-lint` is the structural enforcer. | Slice 02 |
| ADR-0014 shared-types pattern | **REUSE** unchanged | All new types live in `overdrive-control-plane::api`; CLI imports directly. Pattern is the contract this feature ratifies, not extends. | Slices 01, 02, 03 |
| ADR-0015 `ControlPlaneError` + `ErrorBody` | **REUSE** unchanged | HTTP-level errors (400 on bad TOML, 409 on conflict) flow through the existing `to_response` path on both `application/json` and `application/x-ndjson` lanes. Streaming-mid-stream errors become NDJSON `ConvergedFailed`, NOT `ErrorBody` (per [C2] / DISCUSS [D1] rationale). | Slice 02 |
| `IsTerminal` from `std::io` | **REUSE** stdlib (Rust 1.70+, in workspace) | No `atty`/`isatty` dependency; the stdlib stabilised the API. | Slice 03 |
| `submit_job_post` CLI command in `overdrive-cli` | **EXTEND** (gain `--detach`, gain NDJSON consumption logic, gain `IsTerminal` branch) | The command already exists; it does the JSON ack today. Streaming consumption + `--detach` are the new branches. | Slices 02, 03 |
| `alloc_status` CLI renderer | **REPLACE in place** ([D2]) | The current renderer literally prints `Allocations: N`. The journey TUI mockup specifies the target. Replace is the right shape for "the existing surface tells the operator nothing." | Slice 01 |
| `AppState` struct (axum router state) | **EXTEND** (gain `lifecycle_events: Arc<broadcast::Sender<LifecycleEvent>>`; promote `clock: Arc<dyn Clock>` to a direct field if not already) | One additional `Arc<...>` field; `Clone` impl is preserved (every field is `Arc<...>`). | Slice 02 |

---

## CREATE NEW (with justification)

| New surface | Why not extend | Smallest possible cut |
|---|---|---|
| `enum SubmitEvent` (4 variants) | The streaming wire shape does not exist. There is no `SubmitJobResponse`-shaped surface that could absorb four-variant polymorphism without breaking existing single-object consumers (which is precisely the back-compat the [C2] decision protects). | Smallest possible: one `enum` with `#[serde(tag = "kind")]` for line-by-line parsing legibility. |
| `enum TransitionReason` (8 variants Phase 1) | Codebase-wide search for `TransitionReason` returned nothing. The structured-reason concept is genuinely new. The closest existing type is `DriverError`, which is a thiserror error type intended for log-line `Display`, not a wire-typed enum (no `Serialize` / `ToSchema` derives, no canonical wire form, no rkyv archive shape). Trying to extend `DriverError` to play this role would conflate "this is an error type" with "this is a state-transition reason on the wire" and would cross the §4 intent/observation/error-class boundaries. | One `#[non_exhaustive]` enum, additive going forward; lives in `overdrive-core` so both the action shim and reconciler can construct it; re-exported through `overdrive-control-plane::api` for the wire derive. |
| `enum TerminalReason` (3 variants) | The streaming `ConvergedFailed` event needs a structured terminal-cause discriminator that does not collapse with `TransitionReason` (the cause of *the* failure may be a `TransitionReason`, but the terminal disposition — `BackoffExhausted` vs `Timeout` vs `DriverError` — is a higher-level concept). Could in principle extend `TransitionReason` with these variants, but that would let the *per-transition* `reason` field on `LifecycleTransition` legally carry `Timeout` (a server-side cap concept), which is a category error: a single transition cannot "have" a server timeout. Two enums, two concerns. | Three variants, additive. |
| `enum TransitionSource` (2 variants Phase 1: `Reconciler`, `Driver(DriverType)`) | The journey YAML names `source` as `reconciler` \| `driver(process)` — a structured discriminator. `String` would work but invites drift. `DriverType` already exists (`overdrive_core::traits::driver::DriverType`); reuse it inside the `Driver` variant. | Smallest possible. |
| `enum AllocStateWire` (5 variants mirroring `AllocState`) | The internal `AllocState` enum derives `rkyv::*` for the observation store. The wire shape needs `Serialize` / `Deserialize` / `ToSchema` and a stable lowercase string repr. Adding all those derives to the internal type would entangle storage and wire concerns. | Mirror enum with `#[serde(rename_all = "lowercase")]`; conversion impls. Same pattern the internal `AllocState`'s own `Display` already follows (lowercase strings). |
| `struct TransitionRecord` (snapshot last-transition block) | `last_transition` is a per-allocation block on the snapshot envelope — a struct-shaped composition of `from`, `to`, `reason`, `detail`, `source`, `at`. Could be a tuple but the field count and the `Option`-vs-required distinction warrant a named struct. | Six-field struct with `Option<String>` for `detail`, `at: String` (RFC 3339). |
| `struct RestartBudget { used: u32, max: u32, exhausted: bool }` | Tuple `(u32, u32, bool)` is the alternative; the named struct is the better wire shape (the JSON renders `{used, max, exhausted}` instead of `[3, 5, false]` which is unreadable). | Three-field struct. |
| `struct ResourcesBody { cpu_milli: u32, memory_bytes: u64 }` | The internal `Resources` derives `rkyv::*` for storage; rationale identical to `AllocStateWire`. The conversion from `Resources → ResourcesBody` is mechanical. | Two-field struct mirror. |
| `struct LifecycleEvent { … }` (broadcast channel payload, NOT on wire) | The broadcast subscriber needs the join keys (`alloc_id`, `job_id`) plus the from/to states plus the reason and source. Could pass `AllocStatusRow` directly, but the row does not carry `from` (only `to` is the post-transition state) — the action shim is the only site that knows `from` (via `find_prior_alloc_row`). The event captures both. | Eight-field struct, internal-only, lives in `overdrive-core` next to the trait surface. |

**Total CREATE NEW**: 5 enums + 3 structs (1 internal, 2 wire). Every
one carries a justification that names what was searched for and why
extending was structurally wrong (intent/observation boundary,
error-vs-wire conflation, derive-set divergence) — not just
inconvenient.

---

## What was rejected

- **Reusing `DriverError` as the `reason` enum on the wire.** Searched
  for. Found. Rejected: it's an error type (`thiserror::Error +
  Display`), not a state-transition reason. The shapes overlap on the
  driver-domain reasons but diverge on the reconciler-domain reasons
  (`Scheduling`, `BackoffExhausted`, `NoCapacity` are not driver
  errors); making it carry both would be a category error.
- **Adding NDJSON consumer code to a new `overdrive-api-client`
  crate.** Searched for ADR-0014 §Considered alternatives D. Rejected
  per the ADR's own rationale: a separate crate is a Phase 2+
  promotion when a second binary needs the types. Phase 1 has one
  consumer; the CLI imports directly.
- **Reusing `subscribe_all()` on `ObservationStore` for the streaming
  handler.** Searched for. Found. Rejected: it returns a flat row
  stream filtered by nothing (Phase 1 has no prefix-filter primitive),
  which means the streaming handler would receive every alloc row for
  every job and have to filter client-side. The broadcast channel from
  the action shim is the natural job-scoped surface. (Phase 2+ may
  promote `ObservationStore::subscribe_filtered(predicate)` if a
  second consumer wants the same shape.)

---

## Summary

| Category | Count |
|---|---|
| EXTEND | 14 |
| REUSE unchanged | 8 |
| REPLACE in place | 1 (`alloc_status` CLI renderer) |
| CREATE NEW (with justification) | 8 |
| **Total** | **31** |

EXTEND is the dominant disposition. Every CREATE NEW carries a
single-paragraph rationale; reviewers can challenge any of them
in-line. The new surfaces are minimal and additive.
