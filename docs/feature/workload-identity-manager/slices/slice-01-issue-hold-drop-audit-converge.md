# Slice 01 — Issue-on-start, hold, drop-on-stop, audit, converge

> **DESIGN RESOLVED THE OPEN SURFACES — implement ADR-0067 (rev 2), NOT the
> "DESIGN call" text below.** The pinned decisions: the two Actions are
> `IssueSvid { alloc_id, spiffe_id, node_id, correlation }` / `DropSvid { alloc_id,
> correlation }` (ADR-0067 D2); the reconciler converges **`desired` = running
> allocs vs `actual` = the `IdentityMgr` held set** (held-set-as-`actual`, D1/D4 —
> the runtime's `hydrate_actual` reads `state.identity.held_snapshot()`, mirroring
> the `WorkflowLifecycle`/`live_instances()` arm, `reconciler_runtime.rs:2206-2209`);
> `SpiffeId::for_allocation` is the **canonical extraction** of `mint_alloc_identity`
> (`backend_discovery_bridge.rs:424`) + `mint_identity` (`workload_lifecycle.rs:808`)
> — migrate BOTH call sites (D5); and **`SvidLifecycle` is level-triggered via
> `Action::EnqueueEvaluation`** from `WorkloadLifecycle::reconcile` (`:181`) + the
> exit observer (`:230-256`) keyed `job/<workload_id>` (D5b — without this the
> reconciler builds but never ticks). Implement ADR-0067 rev 2, not the
> Open-Questions "DESIGN call" wording.

**Job**: J-SEC-002 | **Feature**: workload-identity-manager (GH #35) | **Stories**: US-WIM-01 (core), US-WIM-03
**Walking skeleton**: YES — the feature's own thinnest end-to-end cut (D2 brownfield)

## Goal (one sentence)

Bind a live, chain-verifiable SVID to the exact set of running allocations — a
standalone `SvidLifecycle` reconciler emits `Action::IssueSvid` on Running /
`Action::DropSvid` on Stop, an action-shim executor mints via the shipped
`ca_issuance::issue_and_audit` and holds the `SvidMaterial` in a shared
`Arc<IdentityMgr>`, drop removes it (leaf key no longer held), and the binding
converges under a DST `assert_eventually!` invariant.

## IN scope

- `SvidLifecycle` reconciler in `overdrive-core` (core class, pure
  `reconcile(desired, actual, view, tick) -> (Vec<Action>, View)` — NO `.await`,
  NO `Ca`/`ObservationStore` handle, wall-clock only via `tick.now`). Converges
  **`desired` = running allocs vs `actual` = the held set**: `running ∧ ¬held →
  IssueSvid`, `¬running ∧ held → DropSvid`, `running ∧ held(valid) → Noop`.
  Builds the `SpiffeId` (pure). Trait/behaviour pinned in docstrings.
- New typed `Action::IssueSvid { alloc_id, spiffe_id, node_id, correlation }` /
  `Action::DropSvid { alloc_id, correlation }` variants (ADR-0067 D2 — `node_id`
  KEPT; correlation via `CorrelationKey::derive`). Plain-enum +2 variants + the
  dispatch triple.
- **`SpiffeId::for_allocation`** (ADR-0067 D5) — the canonical extraction of the
  two existing private helpers; **migrate `mint_alloc_identity`
  (`backend_discovery_bridge.rs:424`) + `mint_identity` (`workload_lifecycle.rs:808`)
  to it in the same slice** (single-cut — no third implementation).
- **Enqueue/handoff** (ADR-0067 D5b): a third `Action::EnqueueEvaluation` in
  `WorkloadLifecycle::reconcile` (`:181` alloc-mutating block, ungated by kind,
  target `job/<workload_id>`) + a sibling `broker().submit` in the exit observer
  (`:230-256`). Without it the reconciler never ticks. Regression test: Running AND
  Stopped transitions tick `SvidLifecycle` with no manual broker poke.
- **`hydrate_actual` `SvidLifecycle` arm** (`reconciler_runtime.rs:2190`): one new
  `AnyReconciler::SvidLifecycle(_)` arm reading `state.identity.held_snapshot()`
  (sync, in-process) → `SvidLifecycleState{desired, actual}` — identical shape to
  the `WorkflowLifecycle` arm (`:2206-2209`).
- Action-shim executor `action_shim/issue_svid.rs` (mirroring
  `dataplane_update_service.rs`): on `IssueSvid` calls
  `ca_issuance::issue_and_audit(ca, observation, clock, node, request)` and
  writes the returned `SvidMaterial` into `Arc<IdentityMgr>`; on `DropSvid`
  removes the `AllocationId` from the held map.
- `IdentityMgr` struct in `overdrive-control-plane`: held-SVID
  `BTreeMap<AllocationId, SvidMaterial>` (`BTreeMap` mandatory — iterated by the
  invariant + `held_snapshot`) behind `parking_lot::RwLock` (ADR-0067 D4); mutators
  `hold`/`drop_svid`/`set_bundle` + `held_snapshot()` (the sync `actual`-projection
  reader). Drop-on-stop removes the entry so the leaf private key
  (`SvidMaterial::leaf_key`) is no longer held.
- Reuse of `issue_and_audit` writes the `issued_certificates` row per issuance
  and refuses issuance on audit-write failure (US-WIM-03 O5 — no silent issuance).
- DST `assert_eventually!("running allocs hold a valid SVID")` invariant over the
  held map vs the running set, plus a teeth test (a broken executor fails it).
- Acceptance test (TEST tier — #35 is a FOUNDATION feature, F2): the
  `issued_certificates` row is WRITTEN per issuance and read back via the
  ObservationStore; `openssl verify -CAfile <root> -untrusted <intermediate>
  <svid.pem>` exits 0 on the minted leaf (built-in-ca's `rcgen_ca_chain_verify`
  shape). The operator `alloc status` render of that row + the deployed-SVID
  operator-verify flow are **#215's** O05/E03 (blocked on #35), NOT this slice's.

## OUT scope

- The `IdentityRead` consumer read surface → Slice 02.
- Restart **recovery** (re-issue every still-Running alloc on boot via the
  `running ∧ ¬held` branch — same branch this slice builds, re-running on an empty
  held set) + the retry-memory View backoff → Slice 03. Slice 01's View may be
  minimal/empty (no retry-backoff path yet); the recovery story is Slice 03.
- The #40 rotation seam (near-expiry branch) → Slice 03.
- The dataplane consumers themselves (#26 sockops / gateway / telemetry).
- (`SpiffeId::for_allocation` is IN scope here — ADR-0067 D5 pins it; the two
  private-helper call sites migrate in this slice.)

## Learning hypothesis

- **Disproves if it fails**: "identity warrants its own convergence target — a
  standalone `SvidLifecycle` reconciler + `IssueSvid`/`DropSvid` actions +
  executor can bind the held-SVID set to the running-alloc set, drop the leaf key
  on stop, audit each issuance, and the binding converges (`assert_eventually!`)."
  If the loop cannot converge cleanly or drop genuinely purge the held key, the
  LOCKED Option 1 is wrong and must be reconsidered before any read surface or
  durability is built.
- **Confirms if it succeeds**: the walking skeleton is COMPLETE — the running set
  holds live, audited, chain-verifiable identity and a stopped workload holds
  none; Slices 02/03 are additive on this surface.

## Acceptance criteria

- [ ] `SvidLifecycle::reconcile` is pure (no `.await`, no CA/observation handle, wall-clock via `tick.now` only) and passes dst-lint; converges `desired` = running allocs vs `actual` = the held set: `running ∧ ¬held → IssueSvid`, `¬running ∧ held → DropSvid`, `running ∧ held(valid) → Noop`.
- [ ] `hydrate_actual` gains a `SvidLifecycle` arm reading `state.identity.held_snapshot()` (sync, in-process) into `actual` — the same shape as the `WorkflowLifecycle` arm (`reconciler_runtime.rs:2206-2209`).
- [ ] `SvidLifecycle` is enqueued via `Action::EnqueueEvaluation` (from `WorkloadLifecycle::reconcile` + the exit observer, keyed `job/<workload_id>`); a regression test proves a Running transition AND a Stopped transition each tick `SvidLifecycle` with no manual broker poke.
- [ ] `SpiffeId::for_allocation` exists (infallible, `#[must_use]`) and the two existing private helpers (`mint_alloc_identity`, `mint_identity`) are migrated to it (no third implementation remains).
- [ ] The executor mints via the shipped `ca_issuance::issue_and_audit` (NOT re-implemented) and writes/removes the `SvidMaterial` in the `Arc<IdentityMgr>` held `BTreeMap`.
- [ ] After Running: the `issued_certificates` row is written for the issuance (read back via the ObservationStore in a gated `integration-tests` test) and `openssl verify -CAfile <root> -untrusted <intermediate> <svid.pem>` exits 0 on the minted leaf at the TEST tier (built-in-ca's `rcgen_ca_chain_verify` shape). (The operator `alloc status` render + the deployed-SVID operator-verify flow are deferred to **#215** — its O05/E03, blocked on #35 — NOT this slice's AC.)
- [ ] After Stop: the held map no longer contains the allocation (the leaf private key is no longer reachable in the held set).
- [ ] DST: `assert_eventually!("running allocs hold a valid SVID")` holds across Running/Stopped churn at a fixed seed; a deliberately broken hold/drop fails the invariant (it has teeth).
- [ ] DST: identity-lifecycle scenario reproduces bit-identically twin-run at a seed (`BTreeMap` iteration + serials via `Entropy` + fixture keys).
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the new acceptance test.

## Dependencies

- The shipped `Ca` port + `ca_issuance::issue_and_audit` (exist:
  `overdrive-core/src/traits/ca.rs`, `overdrive-control-plane/src/ca_issuance.rs`).
- The reconciler runtime + action-shim executor pattern (exist).
- `SimObservationStore` for the audit row under DST (exists).

## Effort estimate

~1 day (≤6h). Reference class: the executor mirrors
`action_shim/dataplane_update_service.rs`; the reconciler mirrors
`ServiceMapHydrator`; the audit binding is wholly reused from `issue_and_audit`.
The new parts are the held `IdentityMgr` map + the two actions + the
`assert_eventually!` invariant.

## Pre-slice SPIKE

Not needed — the shipped `Ca` port + `issue_and_audit` remove the crypto/audit
uncertainty; the reconciler + executor + `assert_eventually!` patterns are all
proven in-tree; the held-set-as-`actual` + enqueue/handoff are grounded against the
`WorkflowLifecycle` precedent (`reconciler_runtime.rs:2206-2209`,
`workload_lifecycle.rs:181`). All previously "DESIGN-pinned" surfaces (Action field
set, `SpiffeId` derivation, held-set-as-`actual`, the enqueue trigger) are RESOLVED
in ADR-0067 rev 2 before crafting, not by a spike.

## Taste-test note

The walking skeleton — touches every backbone activity (notice → bind/unbind →
hold → prove/audit). Ships a reconciler + 2 actions + 1 executor + the held
struct: at the "thin" boundary, justified because a convergence loop is not
demonstrable without all four (a reconciler with no executor emits unhandled
actions; an executor with no invariant cannot prove convergence). Production-data
ACs (real rcgen `openssl verify` + the `issued_certificates` row via the
ObservationStore, NOT synthetic — the operator `alloc status` render is #215's,
blocked on #35). Disproves a real
pre-commitment (the locked Option-1 thesis that identity warrants its own
convergence target). Carries two value stories (US-WIM-01 core + US-WIM-03) — NOT
infra-only.
