# DESIGN Wave-Decisions — backend-instance-replacement (GH #249)

> Authored by Morgan (nw-solution-architect), 2026-06-29. GUIDE mode (the guided
> session was pre-run by the orchestrator; this records the locked decision set
> formalized into signatures + the reconciler edit + ADR-0073). Lean density.
> Closes the DISCUSS `[D1]` hard gate. Full record: **ADR-0073**.
>
> **DDD-14 addendum (Morgan, 2026-07-01).** DDD-1..13 are the DESIGN-wave decision
> set (ADR-0073). DDD-14 is a later DELIVER-surfaced pin: the error model for the
> A1 pump half-close-forward's failed `shutdown(SHUT_WR)`, closing the gap the
> A1 amendment left silent. Its full record is the **ADR-0070 A1 amendment
> addendum (2026-07-01) § "The failed-`SHUT_WR` error model"**, NOT ADR-0073.

- **`design/review-design-iteration-4.md` is the CURRENT DESIGN review handoff**
  (2026-06-30, verdict `conditionally_approved`). Its only finding is a High
  handoff correction: the ADR-0073 index row in `brief.md` still summarizes the
  reconciler fix as generation-gating only and should mention the current-
  instance-scoped veto predicate pinned in DDD-6. The
  authoritative ADR/design handoff resolves the iteration-3 stale-veto Critical
  and has no remaining correctness blocker.
- **`design/review-design-iteration-3.md` is iteration-3 history
  (SUPERSEDED)** — its single Critical (superseded Operator-stop rows re-arm the
  stale veto after fresh placement) was resolved by the 2026-06-30 iteration-3
  revision (current-instance-scoped veto via `current_alloc`, R1-crash row, and
  regression acceptance case); do not read its verdict as current.
- **`design/review-design-iteration-2.md` is iteration-2 history (SUPERSEDED)** —
  its Critical (cardinality contract vs. coalescing) was resolved by the
  2026-06-30 iteration-2 revision (Option B level-triggered coalescing); do not
  read its verdict as current.
- **`design/review-design.md` is iteration-1 history (SUPERSEDED)** — its
  Critical (the unproduceable `TxnOutcome::Conflict` atomicity blocker) was
  resolved by the iteration-1 `TxnOp::IncrementU64` revision; do not read its
  verdict as current.
- Convention: a further revision adds `review-design-iteration-5.md` and updates
  this pointer; the review files themselves are the reviewer's artifacts and
  are not edited by the architect.

## Key Decisions (with rationale + source)

| # | Decision | Rationale | Source |
|---|---|---|---|
| DDD-1 | Verb = `overdrive workload restart <id>` (new top-level `workload` namespace) | Operator-mandated; aligns with #220's planned `workload describe`. `job` namespace stays `list`/`stop` only. | Locked decision 1; #220 |
| DDD-2 | One verb, rollout-restart breadth (running → stop-then-start; stopped → start) | Covers both US-BIR-1 happy paths with a single verb; matches `kubectl rollout restart`. | Locked decision 2; feature-delta `[D1]` |
| DDD-3 | Mechanism = minimal desired-run `generation: u64` precursor; reconciler places when `observed_generation < generation` | Supersedes the stale line-520 operator-stop observation-veto by making placement intent-driven; the forward-compat seam #64/#253/#254 reuse. | Locked decision 3; ADR-0050 OQ-1 |
| DDD-4 | Seam is THIN — only `generation`/`observed_generation`; NO revision rows / `RevisionId` / retention / status reporting | Those stay deferred to #180 (ADR-0050 OQ-1 "Option β"); pulling them forward is the rejected Alt-C over-build. | Locked decision 4; ADR-0050 |
| DDD-5 | Generation = standalone sibling key `workloads/<id>/generation`, 8-byte big-endian u64 | NOT an rkyv aggregate field ⇒ no ADR-0048 envelope bump / golden fixture; sibling-key precedent (`/stop` empty sentinel, `/kind` single byte). Folds into `workloads/<id>/current` when #180 lands. | ADR-0073 item 4; ADR-0050 § Consequences |
| DDD-6 | Reconciler edit gates the line-520/485 veto on `restart_pending = observed_generation < desired.generation` **AND scopes it to the current instance** (`!restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped)` — NOT `any(...)` across history); stamps `observed_generation = desired.generation` on placement | Clearing the sentinel alone is necessary-but-NOT-sufficient — the observed `payments-0` Terminated/Operator row persists. The `any(...)` form let that superseded row re-arm the veto after the fresh placement and wedge the fresh instance's later crash (iteration-3 Critical). | ADR-0073 § Context + § 5; grounded `workload_lifecycle.rs:520` |
| DDD-13 | Veto scoped to the CURRENT instance (iteration-3) — fires only when `current_alloc(&allocs_vec)` (latest-placed alloc by numeric `mint_alloc_id` suffix max) is itself operator-stopped; superseded prior-generation Operator-stop rows NEVER veto | Resolves the iteration-3 Critical. Reuses the existing alloc-id-suffix monotonicity (rows never deleted) — **NO `generation` field on the rkyv `AllocStatusRow`, NO ADR-0048 envelope bump**; the lightest of the review's three acceptable shapes (no View bookkeeping, no per-row marker). NOTE: `current_alloc` uses the NUMERIC suffix max, not `BTreeMap`/`.values()` order (which is LEXICAL: `alloc-x-10 < alloc-x-2`). | ADR-0073 § 5 → "Why the veto must be scoped to the current instance" + R1-crash; **iteration-3 review Critical** |
| DDD-7 | Bug 3 preserved — only `restart` bumps; `deploy` never does | After a same-spec deploy `observed == desired` ⇒ veto stands ⇒ deploy cannot resurrect an operator-stopped workload. | `fix-exec-driver-exit-watcher` Bug 3; locked decision 5 |
| DDD-8 | HTTP route `POST /v1/jobs/:id/restart` (mirror `stop`), NOT `/v1/workloads/:id` | Consistency with the live `/v1/jobs` family; the `jobs/` HTTP prefix is an independent concern from the `workloads/` IntentKey prefix + the `workload` CLI verb — the same split already shipped for `job stop`. Relocating `/v1/jobs → /v1/workloads` is out of scope. | ADR-0073 item 2 |
| DDD-9 | TOCTOU-safe: generation bump + sentinel delete in ONE `IntentStore::txn` via the NEW `TxnOp::IncrementU64` (read-modify-write *inside* the write txn) + `Delete`; NO `Conflict` retry | `development.md` § "Check-and-act must be atomic" — redb serializes writers ⇒ the increment is atomic + monotonic; no window between clear and bump. **Revised post-review** — the prior `Put`-gen + retry-on-`Conflict` relied on a `Conflict` `LocalIntentStore::txn` never produces (it returns `Committed` unconditionally), which lost a bump under concurrency and could drive `generation` backwards. | Locked decision 7; review Critical |
| DDD-10 | `restart` idempotency posture = **level-triggered coalescing**: generation advances monotonically per call (audited); the reconciler converges to **one** fresh instance for the latest generation. Sequential restarts each cycle the workload; concurrent / pre-placement restarts coalesce into one cycle. Only 404 refuses. | Correct rollout-restart posture — a "replace the instance" op is definitionally a *level*, not a command queue; coalescing is the architecturally correct contract (per-generation consumption would graft an edge-triggered replay queue onto the reconciler, the anti-pattern ADR-0064's two-primitive doctrine rejects). Differs from `stop`'s sticky-sentinel idempotency. | ADR-0073 item 1 / § "Idempotency posture: level-triggered coalescing"; **iteration-2 review Critical** |
| DDD-11 | `RestartOutcome` PINNED (was open): two variants, classified from the check-exists read's `/stop` lookup (present ⇒ `Resumed`, absent ⇒ `Restarted`), before the bump; label cosmetic, placement is the reconciler's | Resolves review Finding 2 — no residual open question. | ADR-0073 item 1 |
| DDD-12 | Running-origin sequencing PINNED as the R1–R5 state-transition table (stamp `observed_generation` on the placement tick, never the stop tick; exactly one `StopAllocation` across draining ticks) | Resolves review Finding 4 — the sequencing is load-bearing; DELIVER pins only the existing-branch wiring, not the machine. | ADR-0073 item 5 |
| DDD-14 | **A1 failed-`SHUT_WR` error model = Option B (invariant tripwire).** The half-close forward's `libc::shutdown(dst_fd, SHUT_WR)` return is inspected, NOT discarded: silent on `{0, ENOTCONN}` (success + the documented harmless no-op), `tracing::error!(name: "mtls.pump.half_close_forward_failed", errno, …)` + `debug_assert!(false, …)` on any other errno (the `EBADF`/`ENOTSOCK` family — unreachable unless the join-before-close leg-ownership invariant is broken, i.e. a platform bug). Pure diagnostic, NO behavior change (the connection still reclaims via the sibling `TransportDeath` backstop + terminal-teardown). NO new API surface — a two-arm `match` mirroring `set_best_effort_tcp_opt` (`mod.rs:544-577`). Options A (`let _`) and C (escalate to `mark_exited(TransportDeath)`) rejected. | Closes the error-model gap the A1 amendment left silent (it pinned SUCCESS, not FAILURE) after a code review flagged the discarded `c_int`. `EBADF` is structurally unreachable-unless-invariant-violated (`reclaim_connection` joins every pump before `drop(state.legs)`), `ENOTCONN` is the documented no-op → a tripwire that surfaces a platform bug is right-sized; A over-absorbs (`development.md` § "Errors"), C over-engineers an unreachable-errno escalation redundant with the backstop. D-MTLS-16 is NOT violated by the error path (it governs the *clean* half-close, not a failed one). | **ADR-0070 A1 amendment addendum (2026-07-01) § "The failed-`SHUT_WR` error model"**; RCA Root Cause A; code review of `splice.rs:252-269` |

## Architecture Summary

A pure extension of the shipped hexagonal + reconciler/intent topology. The
`overdrive workload restart <id>` verb is the new driving port → `POST
/v1/jobs/:id/restart` → `restart_workload` handler (mirrors `stop_workload`). The
handler reads `workloads/<id>` (404 if absent) plus `workloads/<id>/stop` (for
the cosmetic `RestartOutcome` label) in one read, then commits an atomic
`IntentStore::txn` that **increments** the desired-run generation at the new
sibling key `workloads/<id>/generation` via the NEW `TxnOp::IncrementU64`
(read-modify-write inside the write txn — atomic + monotonic) and deletes the
`workloads/<id>/stop` sentinel, then enqueues a `job-lifecycle` evaluation. The `WorkloadLifecycle` reconciler (pure
sync) hydrates `desired.generation` from the key and holds `observed_generation`
in its CBOR View; its Run branch gates the line-520 operator-stop veto on
`observed_generation < generation` **and scopes it to the current instance**
(`!restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped)`
— a superseded prior-generation Operator-stop row never vetoes; overridable
while a restart is pending, overriding only for a *current* operator-stop —
Bug 3), emits a `StopAllocation` for a running-origin current instance, then
places a fresh instance (new AllocationId + new `workload_addr` via
`mint_alloc_id`'s `attempt = allocs_vec.len()`) and stamps
`observed_generation = desired.generation`. A later crash of the fresh instance
converges via the existing `is_restartable` branch (the stale superseded
operator-stop row no longer wedges it — iteration-3 fix). The dial-by-name `F`-binding stays
byte-stable (the `FrontendAddrAllocator`'s idempotent `assign` — withhold-not-
release — untouched); in-flight churn fails fast via the existing intercept-worker
`TCP_USER_TIMEOUT`/keepalive legs (no `sock_destroy`).

### The six pinned signatures (DELIVER builds only these)

1. **CLI:** `Command::Workload(WorkloadCommand)` + `WorkloadCommand::Restart { id }`; `commands::workload::restart(RestartArgs{id, config_path}) -> Result<RestartOutput{workload_id, outcome, endpoint}, CliError>`; `RestartOutcome ∈ { Restarted, Resumed }`.
2. **HTTP:** `POST /v1/jobs/:id/restart` → `restart_workload(State<AppState>, Path<String>) -> Result<Json<RestartWorkloadResponse{workload_id, outcome}>, ControlPlaneError>`; 200 / 404 (`NotFound{resource: workloads/<id>}`) / 400 / 500.
3. **http-client:** `ApiClient::restart_workload(&self, id: &str) -> Result<RestartWorkloadResponse, CliError>` → `POST v1/jobs/{id}/restart`.
4. **Generation key + store primitive:** `IntentKey::for_workload_generation(&WorkloadId) -> Self` → `workloads/<id>/generation`; value = `u64::to_be_bytes` (absent/short ⇒ 0). Bump = the **NEW `TxnOp::IncrementU64 { key }`** variant — read the BE u64 (absent ⇒ 0), write `+1` (saturating) *inside the same write txn*. Carries a trait behavior contract (preconditions/postconditions/edge/monotonic-atomic invariant) + a `tests/acceptance/txn_increment_u64.rs` concurrency test (N concurrent ⇒ final == N).
5. **Reconciler:** add `WorkloadLifecycleState.generation: u64` + `WorkloadLifecycleView.observed_generation: u64` (`#[serde(default)]`); add the pure `current_alloc(&[&AllocStatusRow]) -> Option<&AllocStatusRow>` helper (latest-placed alloc by numeric `mint_alloc_id` suffix max, co-located with `mint_alloc_id`); gate `let veto = !restart_pending && current_alloc(&allocs_vec).is_some_and(is_operator_stopped); if veto { ... }` where `restart_pending = view.observed_generation < desired.generation` — **scoped to the current instance, NOT `allocs_vec.iter().any(is_operator_stopped)`** (the iteration-3 fix: a superseded prior-generation Operator-stop row must not veto the current instance's lifecycle, incl. crash-restart of the fresh alloc); stamp `next_view.observed_generation = desired.generation` **on the placement tick only** (R3/R4 of the ADR-0073 R1–R5 table), never on the stop tick (R2/R5). NO rkyv `AllocStatusRow` change.
6. **Handler sequence:** parse → get(for_workload) [+ get(for_workload_stop) for the label] else 404 → `txn[IncrementU64 gen, Delete stop]` (returns `Committed`; NO read-current-gen, NO Conflict retry) → enqueue eval → 200 `{ workload_id, outcome }` (`resumed` if `/stop` was present at the read, else `restarted`). Atomicity = single `txn` commit with the increment read-modify-write inside it.

## Reuse Analysis (HARD GATE — zero unjustified CREATE NEW)

| Overlapping component | Verdict | Evidence |
|---|---|---|
| `stop_workload` handler shape | **EXTEND** | `restart_workload` mirrors it 1:1 (parse→404→atomic-mutate→enqueue→respond); reuses `parse_workload_id_path`, `ControlPlaneError::NotFound`, `enqueue_workload_lifecycle_eval`. |
| `WorkloadLifecycle` reconciler | **EXTEND** | Two additive fields + a current-instance-scoped gate on existing line-520/485 branches + a stamp on the existing placement arm. No new reconciler. |
| `current_alloc` pure helper | **CREATE NEW (minimal, pure)** | No "latest-placed instance" helper exists; needed to scope the veto to the current instance (iteration-3 fix). Pure fn over `&[&AllocStatusRow]` returning the numerically-highest `mint_alloc_id`-suffix row; co-located with `mint_alloc_id`. NO new per-row state, NO rkyv schema change — reuses the existing alloc-id-suffix monotonicity. |
| `IntentKey` | **EXTEND** | New `for_workload_generation` alongside `for_workload_stop`/`for_workload_kind` — same `workloads/<id>/…` family. |
| `ApiClient` http-client | **EXTEND** | `restart_workload` reuses `post_typed`; identical shape to `stop_workload`. |
| `StopWorkloadResponse`/`StopOutcome` (api.rs) | **EXTEND** | `RestartWorkloadResponse`/`RestartOutcome` are siblings; `StopOutcome`'s docstring already anticipates "future verbs (start, restart, cancel) can extend the enum additively". |
| `hydrate_desired` | **EXTEND** | Add a generation read sibling to `stop_intent_present`. |
| `Command::Workload(WorkloadCommand)` namespace | **CREATE NEW (minimal)** | No `workload` namespace exists; `job` is list/stop only. Verb operator-mandated NOT under `job` (#220). Extending `JobCommand` is impossible without violating the locked verb decision. |
| `restart_workload` handler + route | **CREATE NEW (minimal)** | No restart handler/route exists; the new driving port. Mirrors `stop_workload`. |
| `workloads/<id>/generation` key + BE codec | **CREATE NEW (minimal)** | No generation surface exists pre-#249 (ADR-0050 deferred it to #180). The mandated forward-compat seam; extending an existing key is impossible (none carries a generation). |
| `TxnOp::IncrementU64` store primitive | **CREATE NEW (minimal, on the `IntentStore` port)** | **Post-review (atomicity Critical).** `Put` (blind write — TOCTOU), `put_if_absent` (insert-if-absent, no increment), and `get`+`Put` (the rejected race) cannot express atomic monotonic increment. Minimal: one `TxnOp` arm + one redb match arm (the `put_if_absent` get-then-insert-in-one-write-txn shape). NOT throwaway — #180's generation model reuses it verbatim. Carries a trait behavior contract + the `txn_increment_u64` concurrency acceptance test. |

## Technology Stack

| Concern | Tech | License | Note |
|---|---|---|---|
| Generation codec | `u64::to_be_bytes` (std) | — | Branch-free, hex-debuggable, no envelope bump |
| HTTP | axum (existing) | MIT | Mirror `stop_workload` |
| CLI | clap (existing) | MIT/Apache-2.0 | New `WorkloadCommand` |
| Atomic mutation | `IntentStore::txn` + NEW `TxnOp::IncrementU64` variant | — | Atomic monotonic bump+clear in one commit (redb serializes writers) |

No new dependency; no external integration; OSS-first stack unchanged. The one
new surface is the `TxnOp::IncrementU64` port variant (no new crate, no new dep).

## Constraints Established

- Single-node, Phase 2. `replicas = 1`, end-then-bring-up (a brief no-live-instance gap is acceptable — confirmed by #253).
- Intent stays declared (`workloads/<id>` present before AND after — distinct from #211 deletion).
- New instance identity (new AllocationId + new `workload_addr` — distinct from crash-restart's slot reuse).
- `F`-binding byte-stability is a guardrail (allocator's idempotent `assign` — must not regress).
- In-flight churn fail-fast via existing `TCP_USER_TIMEOUT`/keepalive (NO `sock_destroy` — #61 scope).
- Reconciler stays pure-sync (ADR-0035/0036); generation is intent input, `observed_generation` is View memory (persist inputs, not derived state).

## Upstream Changes

- **Product SSOT:** none required by DESIGN — the DISCUSS SSOT diffs (J-OPS-003 extend, journey step 4, Ana persona note) were already operator-ratified 2026-06-29.
- **ADRs:** new ADR-0073; **no existing ADR amended** — ADR-0050 OQ-1's #180 deferral is *consistent with* (not changed by) this thin seam; ADR-0027's stop-HTTP shape is the surface mirrored, unchanged. The NEW `TxnOp::IncrementU64` is an **additive** variant on the `IntentStore` port (ADR-0020 byte-passthrough lineage); it amends no prior decision — the port already exposes `txn` as a batch-of-ops surface, and an additive read-modify op is the same shape extension as any prior `TxnOp` variant would be.
- **Trait surface (DELIVER lands):** the `TxnOp::IncrementU64` variant on `crates/overdrive-core/src/traits/intent_store.rs`, its behavior contract in the trait/variant rustdoc (per `development.md` § "Trait definitions specify behavior, not just signature"), its `LocalIntentStore::txn` match arm (`crates/overdrive-store-local/src/redb_backend.rs`, the `put_if_absent` get-then-insert-in-one-write-txn shape), and the `tests/acceptance/txn_increment_u64.rs` concurrency test. The two test-double `impl IntentStore` blocks (`FaultInjectingIntentStore`, `CountingIntentStore`) gain the arm via exhaustive-match (RED scaffold if unimplemented).
- **No `jobs.yaml` / journey / persona edit by DESIGN** (architect does not edit product SSOT in DESIGN; DISCUSS already applied them).

## DISCUSS assumptions changed (back-propagation)

**None.** Every locked decision was confirmed against the live code:
`is_operator_stopped` at `workload_lifecycle.rs:520` reads the observed row (so
the reconciler edit is mandatory — clearing the sentinel alone is insufficient,
exactly as DISCUSS's gap table stated); no no-sentinel direct-operator-stop path
exists in production (`intentional_stop` → `StoppedBy::Operator` is set only by
`Driver::stop`, driven only by the reconciler Stop branch — the
`workload_lifecycle.rs:502-505` comment anticipates one but it is not built);
`mint_alloc_id`'s `attempt = allocs_vec.len()` yields `payments-1` for free;
`IntentStore::txn`/`get`/`delete` are the existing surface. **Iteration-3
verification:** `AllocationId` is `pub struct AllocationId(String)` with
`#[derive(Ord)]` (`crates/overdrive-core/src/id.rs:139-214`) → `BTreeMap<AllocationId, _>`
iteration (`actual.allocations`) is **LEXICAL on the raw string**, so
`alloc-payments-10` sorts BEFORE `alloc-payments-2` — "last in `.values()`" is
NOT reliably the latest placement. The `current_alloc` helper therefore takes the
NUMERIC max of the parsed `mint_alloc_id` suffix, not iteration order. The
never-delete invariant (the feature retains `payments-0` to achieve `A1 ≠ A2`)
makes attempt indices a strictly-increasing series, so the numeric max is
unambiguously the current instance. No rkyv `AllocStatusRow` field was needed. **Post-review
correction:** the existing `TxnOp` (blind `Put`/`Delete`) + the unconditional
`Committed` return of `LocalIntentStore::txn` cannot produce the `Conflict` the
original draft's bump retried on — the atomic monotonic bump requires a NEW
`TxnOp::IncrementU64` read-modify variant (verified against
`crates/overdrive-core/src/traits/intent_store.rs:62-72` and
`crates/overdrive-store-local/src/redb_backend.rs:302-358`). This is the one
genuinely-new surface the atomicity fix adds; see the Reuse Analysis. The DISCUSS
DoR finding-2 staleness ("all three stories") and persona finding-3 (body-level
amendment) are DISCUSS-wave hygiene items, not DESIGN concerns — flagged here for
the orchestrator but not changed by this wave.

## Open questions deferred to DISTILL/DELIVER

- **Concrete existing-branch *wiring* of the R1–R5 running-origin state machine.** The machine itself is PINNED (ADR-0073 item 5 — resolves Finding 4); DELIVER pins only which existing reconciler branch each row maps to as it exercises the three-AT oracle.
- ~~Whether the handler reads obs to label `RestartOutcome`~~ **CLOSED (Finding 2).** Two variants ship; classified from the check-exists read's `/stop` lookup (present ⇒ `Resumed`, absent ⇒ `Restarted`), before the bump; cosmetic — placement is the reconciler's. ADR-0073 item 1 / DDD-11.
- Whether a stray `workloads/<id>/generation` key needs explicit reaping on #211 deletion (harmless today — reconciler GCs on `job.is_none()`).
