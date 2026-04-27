# Outcome KPIs — phase-1-first-workload

## Feature: phase-1-first-workload

### Objective

Overdrive platform engineers can run `overdrive job submit <spec>`
against a local single-mode control plane (control plane and worker
co-located on one machine), see the lifecycle reconciler converge to
declared replica count via a real OS process running under
cgroup-isolated supervision — with self-healing on crash, clean
stop-and-drain via `overdrive job stop`, and structural
control-plane-vs-workload isolation — by the end of the first
walking-skeleton-extension release for this feature.

> **Phase 1 is single-node.** Multi-node, multi-region, taints, and
> tolerations are explicit Phase 1 non-goals. KPIs that only made
> sense in a multi-node context (e.g. "taint-respect rate") were
> pulled per the 2026-04-27 scope correction (see `wave-decisions.md`).

### Outcome KPIs (feature level)

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | Overdrive platform engineer (and DST harness) | Calls `schedule(...)` and gets a deterministic placement decision | 100% of identical-input calls return identical results across N (≥1024) randomised inputs | N/A (greenfield — no scheduler exists today) | Proptest in `overdrive-control-plane::scheduler::tests` (or wherever DESIGN places it) | Leading — primary |
| K2 | Overdrive platform engineer | Runs a real workload via ProcessDriver under cgroup-isolated supervision | 100% of integration-tests-gated `Driver::start` calls produce a process whose `/proc/<pid>/cgroup` matches the AllocationHandle's cgroup path; 0 zombie processes after `Driver::stop` | N/A (greenfield — no production driver exists today) | Linux-only integration test in `crates/overdrive-host/tests/integration/process_driver.rs` (gated `integration-tests`) | Leading — primary |
| K3 | Overdrive platform engineer (and DST harness) | Submits a job, watches the lifecycle reconciler converge to declared replica count, and runs `overdrive job stop` to drain it cleanly | 100% of submitted 1-replica jobs reach Running within N reconciler ticks under DST; 100% of `job stop` calls drive the allocation to Terminated within N+M ticks; the existing `ReconcilerIsPure` invariant continues to pass with `JobLifecycle` added | N/A (greenfield — no lifecycle reconciler exists today; no `job stop` exists today) | Three new DST invariants — `JobScheduledAfterSubmission`, `DesiredReplicaCountConverges`, `NoDoubleScheduling` — gated on every PR; `job stop` integration test | Leading — primary |
| K4 | Overdrive platform engineer | Watches `overdrive cluster status` stay responsive while a workload bursts CPU on the same host | 100% of integration test runs show `cluster status` returning under 100 ms during a 100% CPU workload burst | N/A (greenfield — no slice creation exists today) | Linux-only integration test in `crates/overdrive-control-plane/tests/integration/cgroup_isolation.rs` (gated `integration-tests`) | Leading — primary |

### Metric Hierarchy

- **North Star**: an end-to-end run where Ana submits a process job, sees a real OS process come up under cgroup isolation, kills the process, watches the platform converge back to Running, then runs `overdrive job stop` and watches it drain cleanly to Terminated — all without any operator intervention beyond the initial `job submit` and final `job stop`. **K1 ∧ K2 ∧ K3 ∧ K4** all green simultaneously.
- **Leading Indicators**: K1 (scheduler determinism — derisks DST invariants); K3 covers both the convergence loop and the stop-and-drain affordance.
- **Guardrail Metrics**:
  - The phase-1-foundation guardrails remain in force (DST wall-clock < 60s, lint-gate false-positive rate at 0, snapshot round-trip byte-identical).
  - The phase-1-control-plane-core guardrails remain in force (CLI round-trip < 100ms on localhost, OpenAPI schema-drift gate green).
  - **NEW guardrail**: lifecycle-reconciler purity. The existing `ReconcilerIsPure` invariant must continue to pass with `JobLifecycle` in the catalogue. A regression here is a platform-team alert.
  - **NEW guardrail**: `dst-lint` clean on the lifecycle reconciler — no banned API in `reconcile`. (The reconciler lives in `overdrive-control-plane` which is `adapter-host` class and not scanned by `dst-lint`; if DESIGN moves it to a `core` crate, this becomes a hard gate. Either way, the team commitment is the reconciler doesn't reach for wall-clock or randomness.)

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|---|---|---|---|---|
| K1 | Proptest in the scheduler module | `proptest!` block over arbitrary `(BTreeMap<NodeId, NodeView>, JobView, Vec<AllocStatusRow>)` inputs; assert determinism | Every PR touching the scheduler | CI |
| K2 | Linux integration test in `overdrive-host/tests/integration/process_driver.rs` (gated `integration-tests`) | Real process spawn against `/bin/sleep`; read `/proc/<pid>/cgroup`; stop; assert scope removed | Every PR touching `overdrive-host` driver code; on the Linux Tier 3 matrix per `.claude/rules/testing.md` | CI |
| K3 | DST invariant catalogue — three new evaluators in `overdrive-sim::invariants::evaluators`; `job stop` integration test | `cargo xtask dst` runs the invariants on every PR; integration test exercises stop-and-drain end-to-end | Every PR | CI |
| K4 | Linux integration test in `overdrive-control-plane/tests/integration/cgroup_isolation.rs` (gated `integration-tests`) | Submit CPU-burst job; measure `cluster status` round-trip during burst | Every PR touching slice-creation / pre-flight code | CI |

### Hypothesis

We believe that shipping a deterministic scheduler + ProcessDriver
with cgroup placement + the lifecycle reconciler with the action shim
(including `overdrive job stop`) + control-plane cgroup isolation as
a single walking-skeleton-extension release will achieve an
execution-layer foundation that every subsequent Phase 1+ feature
(real Raft, multi-node + taint/toleration, microVM driver, WASM
driver, §14 right-sizing, §13 dual policy) can build on with
confidence.

We will know this is true when **a Overdrive platform engineer can
submit a process job to a single-node co-located dev cluster, see
the platform converge to a Running allocation under cgroup-isolated
supervision that survives a SIGKILL of the workload process, and
then drain it cleanly with `overdrive job stop` — all on a single
DST seed plus a single Linux integration test run, both gated in
CI**.

### Smell Tests

| Check | Status | Note |
|---|---|---|
| Measurable today? | Yes | Every KPI has an automated measurement path in CI or in the DST harness. |
| Rate not total? | K1, K2, K3 are rate-shaped (% of round-trips / runs / inputs); K4 is a percentile-shaped property over runs. Acceptable for a greenfield walking-skeleton extension. |
| Outcome not output? | K1 targets engineer behaviour against the platform; K2, K3, K4 target the convergence loop's actual behaviour against real or simulated workloads. Not feature-delivery checkboxes. |
| Has baseline? | Greenfield — every KPI's baseline row is explicit. |
| Team can influence? | Yes — every KPI is a direct consequence of code the platform team writes in this feature. |
| Has guardrails? | The phase-1-foundation + phase-1-control-plane-core guardrails remain. New guardrails: lifecycle-reconciler purity (existing invariant covers); `dst-lint` clean on the reconciler body. |

## Handoff to DEVOPS

The platform-architect needs these from this document to plan
instrumentation:

1. **Data collection requirements**: CI job logs capturing the three
   new DST invariants' pass/fail per tick; integration test wall-clock
   for the cgroup-isolation `cluster status` round-trip during burst
   (the 100 ms bound is the gate); proptest output for scheduler
   determinism; `dst-lint` clean output on reconciler bodies.
2. **Dashboard/monitoring needs**: CI dashboards tracking the three
   new DST invariants over time; flakiness signal on the
   integration-tests-gated tests (should be 0% flake — if higher,
   investigate kernel-version dependencies on the Tier 3 matrix).
3. **Alerting thresholds**: any regression on a new DST invariant is
   a platform-team alert. `cluster status` round-trip > 100 ms during
   workload burst is a regression alert (slice machinery has
   degraded).
4. **Baseline measurement**: none new — the prior features' baselines
   continue to apply. The integration tests gated `integration-tests`
   establish the baseline for cgroup-related properties.

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial KPIs for `phase-1-first-workload` DISCUSS wave. |
| 2026-04-27 | Scope correction — Phase 1 is single-node. Removed the K1 (node-registration round-trip) and K5 (taint-respect / job-stop combined) KPIs. The `overdrive job stop` portion of the prior K5 is folded into the new K3 (lifecycle reconciler — start AND stop are the same convergence loop). Re-numbered the remaining KPIs to K1-K4. Removed every multi-node phrasing. |
