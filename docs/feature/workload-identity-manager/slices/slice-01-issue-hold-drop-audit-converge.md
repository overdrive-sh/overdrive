# Slice 01 — Issue-on-start, hold, drop-on-stop, audit, converge

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
  NO `Ca`/`ObservationStore` handle, wall-clock only via `tick.now`). Emits
  `Action::IssueSvid` when an alloc reaches Running, `Action::DropSvid` on Stop.
  Trait/behaviour pinned in docstrings.
- New typed `Action::IssueSvid` / `Action::DropSvid` variants (exact field set is
  a DESIGN call — see feature-delta Open-Questions #1).
- Action-shim executor `action_shim/issue_svid.rs` (mirroring
  `dataplane_update_service.rs`): on `IssueSvid` calls
  `ca_issuance::issue_and_audit(ca, observation, clock, node, request)` and
  writes the returned `SvidMaterial` into `Arc<IdentityMgr>`; on `DropSvid`
  removes the `AllocationId` from the held map.
- `IdentityMgr` struct in `overdrive-control-plane`: held-SVID
  `BTreeMap<AllocationId, SvidMaterial>` (`BTreeMap` mandatory — iterated by the
  invariant) behind the concurrency primitive DESIGN pins. Drop-on-stop removes
  the entry so the leaf private key (`SvidMaterial::leaf_key`) is no longer held.
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
- Restart-idempotence (recompute held state on boot from persisted inputs) →
  Slice 03 (Slice 01's View may be minimal/empty; durability is Slice 03).
- The #40 rotation seam (near-expiry branch) → Slice 03.
- The dataplane consumers themselves (#26 sockops / gateway / telemetry).
- `SpiffeId::for_allocation` derivation shape → DESIGN handoff (#1).

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

- [ ] `SvidLifecycle::reconcile` is pure (no `.await`, no CA/observation handle, wall-clock via `tick.now` only) and passes dst-lint; emits `IssueSvid` on Running, `DropSvid` on Stop.
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
proven in-tree. The one DESIGN-pinned uncertainty (Action field set + `SpiffeId`
derivation) is resolved by the architect before crafting, not by a spike.

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
