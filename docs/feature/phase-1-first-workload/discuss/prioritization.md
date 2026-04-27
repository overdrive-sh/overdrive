# Prioritization — phase-1-first-workload

> **Phase 1 is single-node.** Control plane and worker run co-located on
> one machine. There is exactly one node — the local host — and it is
> implicit. There is no operator-facing node-registration verb, no
> taint, no toleration, no multi-node placement choice. The earlier
> 6-slice plan that included a node-registration slice and a
> taint/toleration slice was pulled per the 2026-04-27 scope
> correction (see `wave-decisions.md`).

## Release Priority

| Priority | Release | Target Outcome | KPI | Rationale |
|---|---|---|---|---|
| 1 | Walking Skeleton extension (Slices 1-4) | Operator submits a process job, scheduler places it on the local node, ProcessDriver starts it inside a cgroup scope, lifecycle reconciler converges, control plane stays isolated and responsive | K1, K2, K3, K4 (see `outcome-kpis.md`) | This feature is the execution-layer extension of `phase-1-control-plane-core`'s walking skeleton. All four internal slices ship together to close the "submit → run" loop. |

There is no Release 2 for this feature. Subsequent waves (Phase 2+) add: real Raft + multi-node, microVM/WASM drivers, **scheduler taint/toleration support** (the deferred half of GH #20), the §14 right-sizing reconciler reading memory pressure, eBPF dataplane, and the §13 dual-policy engine.

## Backlog Suggestions

| Story | Release | Priority | Outcome Link | Dependencies | GH Issue |
|---|---|---|---|---|---|
| US-01 — First-fit scheduler scaffold | WS-ext | P1 (parallel with US-02) | K1 (scheduler determinism) | phase-1-control-plane-core (Resources, aggregates) | #15 |
| US-02 — ProcessDriver + cgroup scope | WS-ext | P1 (parallel with US-01) | K2 (driver Linux integration) | phase-1-foundation Driver trait | #14 |
| US-03 — Job-lifecycle reconciler + action shim + `job stop` | WS-ext | P2 | K3 (convergence + purity) | US-01, US-02; **DESIGN must clarify State shape** | #21 |
| US-04 — Control-plane cgroup isolation | WS-ext | P3 | K4 (control plane stays responsive) | US-02, US-03 | #20 (cgroup half only — taint/toleration half DEFERRED) |

## Priority ordering within the walking-skeleton extension

1. **US-01 and US-02 in parallel** — they share the precondition that aggregates exist, but they don't share code directly. If resource-constrained to one developer, **US-01 first** because it is the higher-uncertainty pure-function piece (determinism is load-bearing for DST), and US-02 is Linux-host-coupled and can be staged behind the `integration-tests` feature gate without blocking the default lane.
2. **US-03 next** — the convergence loop. Cannot start until US-01 (scheduler to call) and US-02 (ProcessDriver to dispatch into) are landed. **HARD DESIGN DEPENDENCY** on the `State` shape — see DoR. This story also bundles `overdrive job stop` end-to-end since stop is the inverse of start through the same lifecycle path.
3. **US-04 last** — control-plane cgroup isolation. Depends on US-02 (ProcessDriver wiring) and US-03 (lifecycle reconciler producing real workloads to assert against in the integration test).

## Tie-break rules

Applied only if Slices 1 and 2 cannot run in parallel:

1. **Walking-skeleton coverage first** — both slices are on the skeleton extension, so this doesn't break the tie.
2. **Riskiest assumption first** — Slice 1 (scheduler determinism) is the highest-uncertainty pure-function piece, and DST replay correctness depends on it. Slice 2 (ProcessDriver) is well-understood mechanically — `tokio::process::Command` + `cgroups-rs` is established art. **Slice 1 jumps ahead under contention.**
3. **Highest-value outcome** — if both could land first, Slice 1 is preferred because Slice 3 (the convergence loop) depends on calling Slice 1's `schedule(...)` function from within the lifecycle reconciler's pure body. Slice 2 only needs to be present by the time Slice 3 wires the action shim — which is the last step of Slice 3 itself.

Under no contention, **parallel for Slices 1 and 2 remains the fastest** (assumes one developer per slice; if a single developer is doing both serially, do Slice 1 first, then Slice 2).

## Sequencing summary — dependency chain

The 4-slice plan dependency chain is **#15 (Slice 1) → #14 (Slice 2) [parallel] → #21 (Slice 3) → #20-cgroup (Slice 4)**. This matches the codebase research's pure type-dependency chain directly — no learning-leverage trade is needed once the taint/toleration half of #20 is removed from Phase 1 scope.

**GH #20 split note**: Issue #20 is *"Control-plane cgroup isolation + scheduler taint/toleration support"*. The 4-slice plan delivers only the cgroup-isolation half (Slice 4); the taint/toleration half is deferred to a later phase alongside multi-node + Raft. The user is expected to split GH #20 into two issues:

- (Phase 1) Control-plane cgroup isolation — covered by Slice 4 / US-04.
- (Phase 2+) Scheduler taint/toleration support — needs multi-node placement choice to be meaningful.

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial prioritization for `phase-1-first-workload`. |
| 2026-04-27 | Scope correction — Phase 1 is single-node. Removed the prior US-01 (node registration) and US-05 (taint/toleration) entries. Re-numbered to a 4-story plan: US-01 (scheduler) → US-02 (driver) → US-03 (reconciler + `job stop`) → US-04 (cgroup isolation). Updated dependency chain to drop the taint/toleration leg and noted the GH #20 split. |
