# Walking-Skeleton — phase-2-xdp-service-map

**Decision**: NO new walking skeleton in this feature. The existing
Phase 1 + Phase 2.1 skeleton is the inheritance boundary; Phase 2.2
fills the body of one hexagonal port (`Dataplane::update_service`)
and adds one reconciler that drives it.

---

## Inheritance source

Phase 1's walking skeleton —
`customer-submits-a-job-and-watches-it-run` — already shipped the
end-to-end user-observable loop:

1. Operator runs `overdrive job submit` (REST `POST /v1/jobs`).
2. Control plane writes intent to `IntentStore`; reconciler converges
   the desired-vs-actual gap.
3. `JobLifecycle` reconciler emits `Action::StartAllocation`; the
   action shim invokes `Driver::start`; allocation reaches `Running`.
4. Eventually-routed traffic via the kernel-side dataplane hits the
   workload — at Phase 1 time this was a *no-op stub*; at Phase 2.1
   time it was a *no-op `xdp_pass` program attached to `lo`*.

Phase 2.2 fills the gap between step 3 and step 4: the kernel-side
dataplane now actually *routes* via SERVICE_MAP / Maglev /
REVERSE_NAT instead of being a no-op. **The user-observable loop
from Phase 1 does not change shape.** The same `overdrive job
submit` call still produces a running allocation reachable on the
configured VIP. Phase 2.2 is purely a substrate extension.

---

## Why no new WS scenario

Per `nw-test-design-mandates` § "Walking Skeleton Litmus Test":

1. *Title describes user goal, not technical flow* — the only
   user-goal change Phase 2.2 could plausibly express is "customer
   reaches the workload via VIP" — which is already implicit in the
   Phase 1 WS.
2. *Given/When describe user actions/context, not system state
   setup* — there is no new user action this feature introduces.
3. *Then describes user observations, not internal side effects* —
   the user-observable outcome is unchanged from Phase 1.
4. *Non-technical stakeholder confirmation* — a stakeholder cannot
   tell the difference between Phase 1's no-op-routing-via-stub and
   Phase 2.2's real-routing-via-XDP unless they care about
   throughput, multi-backend distribution, or zero-drop swaps —
   and those are *quality attribute scenarios* (ASR-2.2-01..04),
   not user-goal scenarios.

Adding a "WS: customer reaches a multi-backend service via VIP"
scenario today would either (a) duplicate the Phase 1 skeleton or
(b) embed quality-attribute claims (zero-drop, ≤ 1 % churn) into
something framed as a user goal, conflating Mandate 5 (user-centric
WS) with Mandate 6 (adapter integration). The right shape is what
this feature actually does:

- Phase 1's WS continues to pass with Phase 2.2's substrate behind
  it (regression-tested by the existing `JobLifecycle` ESR
  invariants, which now run through a real `EbpfDataplane` body in
  CI's `ubuntu-latest` integration job).
- Phase 2.2 ships **focused boundary scenarios** (S-2.2-01..30)
  exercising the new substrate at every tier.
- ASR-2.2-04 (`HydratorEventuallyConverges` /
  `HydratorIdempotentSteadyState`) is the structural backstop that
  proves the new substrate stays converged in DST.

---

## Strategy mapping (per orchestrator brief, confirmation-only)

Per DISCUSS Decision 2 (locked, user-ratified):

- **Tier 1 DST** — `SimDataplane` + `SimObservationStore` (Strategy A
  — InMemory). Property-shaped invariants live in
  `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`.
- **Tier 3 real-kernel** — real eBPF programs in Lima (developer
  macOS) or `ubuntu-latest` (CI) against real veth (Strategy C —
  real local).

The two compose: Tier 1 catches concurrency / ordering / partition;
Tier 3 catches kernel verifier / packet rates / NIC drivers / BPF
map format. Neither substitutes for the other.

---

## Phase 1 WS regression coverage in this feature

Phase 1's WS is regression-tested by Phase 2.2 by virtue of:

- The existing `ReconcilerIsPure` invariant in
  `crates/overdrive-sim/src/invariants/mod.rs` continuing to pass
  with `ServiceMapHydrator` added to the catalogue (tested via
  S-2.2-30).
- The existing `JobLifecycle` ESR invariants
  (`JobsEventuallyReachRunning`, `RestartLoopBudgetHonored`, etc.)
  continuing to pass when the action shim's dispatcher fans out into
  `EbpfDataplane::update_service` (Phase 2.2 body) instead of the
  Phase 2.1 no-op stub.
- The existing `IntentStoreReturnsCallerBytes` and
  `SnapshotRoundtripBitIdentical` invariants continuing to pass
  with the additive `service_hydration_results` ObservationStore
  table — the new table is observation-class per architecture.md
  § 12, so it cannot cross into intent.

DELIVER's gate: the Tier 1 DST suite (`cargo xtask dst`) must remain
green at every commit; a regression in any of the existing
invariants signals a Phase 1 WS regression and blocks merge.

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial walking-skeleton inheritance documentation for `phase-2-xdp-service-map`. — Quinn (Atlas). |
