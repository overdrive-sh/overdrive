# Definition of Ready Validation — phase-1-first-workload

9-item DoR checklist applied to each of the four user stories in
`user-stories.md`. Per the product-owner skill's hard-gate rule,
DESIGN wave does not start until every item passes with evidence —
EXCEPT where a hard DESIGN dependency is explicitly flagged (US-03).

> **Phase 1 is single-node.** The DoR was originally validated against
> a 6-story plan that included a node-registration story (US-01) and
> a taint/toleration story (US-05). Both were pulled per the
> 2026-04-27 scope correction (see `wave-decisions.md`) — Phase 1 has
> exactly one local node (the host the control plane runs on),
> implicit and not operator-facing. The DoR below covers the
> tightened 4-story plan.

---

## Story: US-01 — First-fit scheduler scaffold

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the determinism-load-bearing argument; ties to DST replay correctness. Calls out Phase 1 single-node as a degenerate case the proptest still covers. |
| 2. User/persona with specific characteristics | PASS | Ana wiring the lifecycle reconciler; motivation explicit. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) local-node placement; (b) capacity accounting under partial pre-allocation; (c) capacity exhausted with structured PlacementError. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: determinism, single-node accept, NoCapacity, capacity accounting, NoHealthyNode. Within band. |
| 5. AC derived from UAT | PASS | 7 AC bullets each trace to a scenario or to the System Constraint (BTreeMap-only iteration). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~1 day; 5 scenarios; single concern (one pure function + one error enum + one helper). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address NodeView/JobView projection types (DESIGN owns), Resources arithmetic, the Phase 1 single-node degenerate-case shape. |
| 8. Dependencies resolved or tracked | PASS | Depends on prior feature's aggregates only. |
| 9. Outcome KPIs with measurable targets | PASS | K1 row targets 100% determinism on randomised inputs; measurement via proptest. |

### DoR Status: **PASSED**

---

## Story: US-02 — ProcessDriver — tokio::process + cgroups v2

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the §6 commitment; ties to whitepaper claims about workload types being first-class. |
| 2. User/persona with specific characteristics | PASS | Ana on a Linux dev host; explicit Linux scope. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) `/bin/sleep 60` in a real cgroup scope with concrete alloc_id; (b) clean stop with grace; (c) binary-not-found error. Real binaries, real PIDs, real paths. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 6 scenarios: spawn + scope, /proc/<pid>/cgroup match, BinaryNotFound, SIGTERM clean, SIGKILL escalation, default-lane no-real-process. Within band. |
| 5. AC derived from UAT | PASS | 9 AC bullets each trace to a scenario or System Constraint (integration-tests gating). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~1 day on a Linux dev workstation; ~2 days if developer needs a Linux VM in the loop. 6 scenarios. Single concern (Driver impl). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address `cgroups-rs` vs direct cgroupfs (DESIGN owns), resource enforcement on cgroup scope (DESIGN owns), macOS / Windows scope. |
| 8. Dependencies resolved or tracked | PASS | Depends on phase-1-foundation Driver trait + AllocationSpec + Resources — all shipped. |
| 9. Outcome KPIs with measurable targets | PASS | K2 row targets 100% Driver::start success under integration-tests; 0 zombies. |

### DoR Status: **PASSED**

---

## Story: US-03 — Job-lifecycle reconciler + action shim (and `job stop`)

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the §18 architectural commitment; explicit linkage to convergence-loop closure AND to the operator-facing `job stop` affordance. |
| 2. User/persona with specific characteristics | PASS | Four personas named: Phase 2+ reconciler authors, DST harness, Ana via alloc status, operator running `job stop`. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) submit-schedule-start-Running with concrete alloc_id, then `job stop` to Terminated; (b) crash recovery with restart count tracking; (c) backoff exhausted after M failures. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 7 scenarios: convergence, purity, dst-lint clean on reconciler, crash restart, backoff exhausted, stop-and-drain, stop-on-unknown-job. At the upper band but each scenario covers a distinct behaviour and the bundle is justified by `job stop` being the inverse of start through the same lifecycle path. |
| 5. AC derived from UAT | PASS | 13 AC bullets each trace to a scenario or invariant. The bundle of start + stop is dense but each AC is checkable in isolation. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | **WARN** — marked ~1-2 days in the slice brief (upper end). 7 scenarios. The slice is dense: new variants on Action enum + new variants on AnyReconciler/View + lifecycle reconciler + action shim + AppState extension + three new DST invariants + `overdrive job stop` end-to-end. **Acceptable but at the upper limit; if effort exceeds 2 days during DESIGN / DELIVER, a split is: 3A = lifecycle reconciler skeleton + StartAllocation only + JobScheduledAfterSubmission invariant; 3B = StopAllocation + RestartAllocation + backoff + DesiredReplicaCountConverges + NoDoubleScheduling + `job stop` CLI + handler.** Flagged but not blocking. |
| 7. Technical notes: constraints/dependencies | PASS | Notes address the **HARD DESIGN dependency on State shape** (codebase research's flagged structural blocker), action shim placement, `tick.now` rule, MigrateAllocation deferred, `job stop` HTTP shape (DESIGN-owned). |
| 8. Dependencies resolved or tracked | **CONDITIONAL PASS** — Depends on US-01, US-02 (both in this feature's pipeline). **HARD DESIGN dependency: the `State` shape MUST be clarified by DESIGN before DELIVER can begin on this story.** Three options listed in technical notes (parameterised, concrete with BTreeMap, or per-reconciler typed via `AnyState` matching `AnyReconciler`); recommendation is option (c). |
| 9. Outcome KPIs with measurable targets | PASS | K3 row covers the three new DST invariants + ReconcilerIsPure regression check + `job stop` integration test. |

### DoR Status: **PASSED — with one HARD DESIGN dependency flagged on item 8 (State shape) and one right-sizing WARN on item 6**

This is the one story that DOES NOT clear DoR by ordinary means. The
`State` shape decision is not a DISCUSS-wave problem to solve — it
belongs in DESIGN's first ADR for this feature. DELIVER must NOT
start on US-03 before that ADR lands. The other DoR items pass; this
single dependency is the gate.

---

## Story: US-04 — Control-plane cgroup isolation

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the §4 structural-defence claim; explicit positioning relative to single-node co-location making the cgroup story the structural backstop. |
| 2. User/persona with specific characteristics | PASS | Linux-host engineer; future systemd-unit operator; explicit DST-not-applicable note. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) server boots, enrols, stays responsive under `stress --cpu 4` burst; (b) idempotent re-boot detects existing slice; (c) actionable error on missing cgroup v2 delegation with full multi-line message. Real systemd flags. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: responsive under burst, smoke test, missing delegation, cgroup v1 host, missing controller. Within band. |
| 5. AC derived from UAT | PASS | 7 AC bullets each trace to a scenario or System Constraint. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~1 day on a Linux dev workstation. 5 scenarios. Single concern (slice creation + pre-flight + responsive-under-burst test). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address Linux-only scope, dev escape hatch, deferred resource limits on the slice, the GH #20 split (taint/toleration half deferred). |
| 8. Dependencies resolved or tracked | PASS | Depends on US-02 (cgroup machinery) and US-03 (workloads to assert against). |
| 9. Outcome KPIs with measurable targets | PASS | K4 row targets 100% < 100ms under burst; integration test on Tier 3 Linux matrix. |

### DoR Status: **PASSED**

---

## Summary

| Story | Status | Note |
|---|---|---|
| US-01 | PASSED | — |
| US-02 | PASSED | — |
| US-03 | PASSED, with HARD DESIGN dependency on `State` shape | DELIVER blocked on a DESIGN ADR. |
| US-04 | PASSED | — |

**Net status**: 3/4 stories cleanly READY. US-03 is READY pending one
DESIGN ADR (the `State` shape decision); this is a structural blocker
the codebase research surfaced and Luna has flagged explicitly rather
than papering over. **The DESIGN wave should treat the State shape
ADR as priority-zero work — without it, US-03 cannot start, and
without US-03 the convergence loop stays open and `overdrive job stop`
cannot be wired.**

## Cross-cutting

- All 4 stories use real persona names, real numeric values, real
  binary paths (no `user123`, no `test@test.com`, no `Foo`/`Bar`).
- All 4 stories' acceptance criteria are testable through DST,
  proptest, integration tests gated `integration-tests`, or in-process
  axum fixture acceptance — no AC requires manual inspection.
- Scenario titles describe operator-observable outcomes
  ("Job-lifecycle reconciler converges to declared replica count")
  not internal mechanics ("Action shim consumes Vec<Action>").
- No banned anti-pattern detected: no "Implement X" titles, no
  generic data, no technical AC, no oversized stories beyond the US-03
  WARN noted above (which carries an explicit pre-described split).
- **Phase 1 single-node** — every story implicitly or explicitly
  acknowledges the precondition; no story assumes multi-node placement
  choice or operator-facing node registration.

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial DoR validation for `phase-1-first-workload`. |
| 2026-04-27 | Scope correction — Phase 1 is single-node. Removed the prior US-01 (node-registration) and US-05 (taint/toleration) DoR sections entirely. Re-numbered the remaining four stories. The HARD DESIGN dependency on the `State` placeholder (now US-03) is preserved; that's the single structural blocker for DELIVER. The right-sizing WARN on the lifecycle-reconciler story is retained and updated with the new pre-described split (3A start-side / 3B stop-side + restart-side + backoff). |
