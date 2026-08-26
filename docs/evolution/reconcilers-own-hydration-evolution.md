# Evolution — reconcilers-own-hydration (ADR-0087 + ADR-0086)

**Finalized:** 2026-08-25. **Feature slug:** `reconcilers-own-hydration`.
**Decision records:** ADR-0087 (precursor — single restart authority) then
ADR-0086 (hydration crate-move, 4 read-ports); both accepted, **not amended by
this record**. ADR-0086 amends ADR-0036 in part and supersedes ADR-0055 §7
(LivenessRestartGovernor). **Waves run:** DESIGN → DISTILL → DELIVER → FINALIZE
(no DISCUSS — the feature entered at DESIGN with option A locked; the decision
predated the wave).

---

## 1. What this is — a reconciler-primitive refactor, sequenced precursor-first

Two ADRs, landed as one arc:

- **ADR-0087 (precursor — single restart authority).** `WorkloadLifecycle`
  becomes the **sole** restart authority: one `restart_counts` budget spans
  **both** crash and liveness restart (the kubelet shape —
  `.claude/rules/reconcilers.md` § "Single restart authority"). `ServiceLifecycle`
  demotes to readiness/membership + liveness-**terminate** (it emits
  `StopAllocation { Stopped { by: LivenessProbe } }` and reads no budget). This
  **dissolves at its root** the `ServiceLifecycle`→`WorkloadLifecycle` cross-read
  that the original ADR-0086 draft was going to preserve behind a fifth read-port.

- **ADR-0086 (hydration crate-move).** Reconciler **hydration** (impure, formerly
  a ~1100-line central `match` free function in
  `overdrive-control-plane/src/reconciler_runtime.rs`) moves **onto the reconciler
  impls**, reading through an injected `HydrationContext`. The impls + the 3
  dispatch enums extract into a **new `overdrive-reconcilers` crate**; the
  resulting Cargo cycle is broken by **4 narrow read-ports in core**. This
  partially reverses the ADR-0036 "runtime owns all hydration" ruling for the
  intent+observation half: reconcilers own their hydration; the contract stays in
  core.

ADR-0086 is **behaviour-preserving** (proven by a byte-identical characterization
golden + full-suite green). ADR-0087 is a **deliberate, cause-carrying behaviour
change** confined to the liveness terminal reason.

### What shipped

| Component | Home (after) | Disposition |
|---|---|---|
| `overdrive-reconcilers` crate (`crate_class = "adapter-host"`) — 8 reconciler impls + 3 dispatch enums (`AnyReconciler`/`AnyState`/`AnyReconcilerView`) + `service_lifecycle` + per-reconciler `*State`/`*View` + pure helpers + the moved `hydrate_*` bodies | `crates/overdrive-reconcilers/` | **NEW** — depends only DOWN on `overdrive-core` |
| 4 driven read-port traits — `ListenerFacts::fact_for` (async), `ServiceVipView::assigned_vip` (async), `WorkflowLiveSet::live_instances` (sync), `HeldSvidView::held_snapshot` (sync) | `crates/overdrive-core/` | **NEW** — narrow read projections; **no `RestartBudgetView`** (ADR-0087 removed the cross-read) |
| `HydrationContext<'_>` + `HydrateError` (`IntentRead`/`ObservationRead`/`IntentDecode`, `#[from]`→`ConvergenceError`); `HeldSvidFacts` relocated to core; `Reconciler::hydrate_desired`/`hydrate_actual` async trait methods | `crates/overdrive-core/` | **NEW / RELOCATE / EXTEND** |
| `StoppedBy::LivenessProbe` (fieldless rkyv **tail** variant, discriminant 5) + `is_liveness_killed` dual-field predicate + cause-aware exhaustion terminal (`ServiceFailed{LivenessProbeFailed}` vs `BackoffExhausted`) | `crates/overdrive-core/src/transition_reason.rs` (+ reconcilers) | **EXTEND** (ADR-0087) |
| `ReconcilerRuntime` builds a `HydrationContext` per tick, calls `AnyReconciler::hydrate_*`; view persistence unchanged (ADR-0035). Implements **no** read-port (former `RestartBudgetView` gone) | `crates/overdrive-control-plane/src/reconciler_runtime.rs` | **EXTEND** |
| 3 control-plane read-port impls — `ListenerFactStore` (`ListenerFacts`), `WorkflowEngine` (`WorkflowLiveSet`, narrow view only — engine NOT relocated), `IdentityMgr` (`HeldSvidView`) | `crates/overdrive-control-plane/` | **EXTEND** |
| `PersistentServiceVipAllocator` implements `ServiceVipView` | `crates/overdrive-dataplane/` | **EXTEND** |
| 4 `Sim*` read-port impls — `SimListenerFacts`/`SimServiceVipView`/`SimWorkflowLiveSet`/`SimHeldSvidView` | `crates/overdrive-sim/` | **NEW** — hydration boundary now DST-injectable |
| dst-lint whole-crate purity clause over `overdrive-reconcilers/src/**` + additive compile-guard (`hydrate_*` carry no `&dyn Clock`) | `xtask/src/dst_lint.rs` | **EXTEND** |

### What was deleted (single-cut, with tests, per CLAUDE.md deletion discipline)

- The central `hydrate_*` free fns + **~11 helpers** in `reconciler_runtime.rs` —
  deleted the moment their bodies moved onto the impls (ADR-0086 D9 / S3). Absence
  proven by green build; no second hydration path survives.
- The ADR-0087 cross-read: the `restart_status_for_alloc` **CALL** in
  `hydrate_service_alloc_facts`, plus `ServiceAllocFact.{restart_count,restart_spec}`,
  `RestartReason`, and `Action::RestartAllocation.reason`. **KEPT** the
  `restart_status_for_alloc` **method** — 4 live streaming callers
  (`streaming.rs`) render the operator attempt-index.

---

## 2. Key decisions (the ones with lasting value)

| ID | Decision | ADR |
|---|---|---|
| D0 | **Precursor: single restart authority.** `WorkloadLifecycle` owns crash+liveness under one budget; `ServiceLifecycle` → readiness/membership + liveness-terminate; the cross-read is dissolved, not ported. Lands BEFORE the crate-move. | ADR-0087 |
| D1 | Reconcilers own `hydrate_*` as impure async trait methods; `AnyReconciler` forwards + wraps `Self::{State,View}` at the boundary (clone of the existing `reconcile` enum-forwarding shape). | ADR-0086 D1 |
| D2 | Keep the 3 dispatch enums (pragmatic cut; enum-collapse #272 out of scope). | ADR-0086 D2 |
| D3 | New crate `overdrive-reconcilers`, `crate_class = "adapter-host"` — contract stays in core, impls move out. | ADR-0086 D3 |
| D4 | Cycle broken: the new crate depends only **DOWN** on core; read-ports are impl'd **UP** by control-plane/dataplane. `overdrive-core → overdrive-reconcilers` exists **dev-dep only**. | ADR-0086 D4 |
| D5 | **Exactly 4** new read-ports (`VmHostState` + `DriverRegistry` already core; VIP allocator added). `RestartBudgetView` **removed** — ADR-0087 eliminates the cross-read. | ADR-0086 D5 (amended) |
| D6 | `HeldSvidFacts` relocates to core (crosses the `HeldSvidView` trait signature). | ADR-0086 D6 |
| D7 | Purity firewall restored via a targeted dst-lint `reconcile`-body scan over the new crate; `ReconcilerIsPure` as compile backstop. | ADR-0086 D7 |
| D8 | Pure-sync `reconcile` unchanged ⇒ **DST replay survives**; the Sim read-ports are a **net injectability gain** (the hydration boundary was previously invisible to the harness). | ADR-0086 D8 |
| D9 | Single-cut migration; the old central free fns deleted in the same arc. | ADR-0086 D9 |

### The one contested-but-forced sub-decision (LOCKED)

`LivenessProbeFailed.attempts` value-semantics shift (ADR-0087 D4 / DESIGN OQ-1 /
S-ROH-A-07). Under single authority `WorkloadLifecycle` stamps
`attempts = restart-budget count consumed` (= `CEILING`), **not**
`consecutive liveness failures`. This is **forced** — preserving
`consecutive_failures` would require the very cross-read being eliminated — and is
the more consistent reading (parallels `BackoffExhausted.attempts`, same
"attempts consumed" meaning). **User-confirmed / locked.**

---

## 3. Delivery — 7 DES-monitored steps, precursor-first

Every step RED/GREEN/COMMIT logged; DES integrity verified.

| Step | Slice | Commit(s) |
|---|---|---|
| 01-01 | ADR-0087 single-cut: liveness-terminate + `WorkloadLifecycle` sole authority; delete the cross-read + liveness-restart vocabulary | `0d9d0b73`, `3cc79247`, `52df8e64` |
| 01-02 | Harden: rkyv schema-evolution golden `FIXTURE_LIVENESS_PROBE_V1` for `StoppedBy::LivenessProbe` (existing fixtures untouched) | `60b35e57` |
| 02-01 | S1 — add the 4 core read-port traits + `HydrationContext`/`HydrateError`; relocate `HeldSvidFacts` to core | `a2b4cdb3` |
| 02-02 | S2 — create `overdrive-reconcilers` (`adapter-host`); move the 8 impls + 3 enums + `service_lifecycle` + State/View + pure helpers | `96d2f90b` |
| 02-03 | **S2-gate — capture + COMMIT the characterization golden** (pre-move hydrated `AnyState` per reconciler + fixed-seed reconcile trajectory) | `90a53bc3` |
| 02-04 | S3 (single-cut) — move the hydrate bodies onto the impls; add `AnyReconciler::hydrate_*` forwarding; **delete** the central free fns | `ead31eda` |
| 02-05 | S4 — add the 4 `Sim*` read-port impls; point the DST invariant catalogue at `overdrive_reconcilers`; whole-crate dst-lint purity clause | `4162677e` |

Review remediation (D1/D2/D4): `b77d5786` / `688c7567` / `f5459f19`.

---

## 4. Sequencing — the two prerequisites that made the arc correct

1. **ADR-0087 landed first.** Removing the `restart_status_for_alloc` cross-read
   as a **precursor** meant ADR-0086's hydration move never had to port a
   `RestartBudgetView` — the fifth read-port the original draft carried simply
   ceased to exist. The right fix was upstream (dissolve the cross-read), not a
   cleaner way to relocate it.

2. **The characterization golden preceded the single cut (HARD DELIVER GATE).**
   ADR-0086 S3 is single-cut: deleting the central `hydrate_*` free fns leaves **no
   live old-vs-new diff**. The behaviour-preservation claim (B-01/B-02/B-03
   equivalence) therefore has a baseline **only because** the golden — the pre-move
   hydrated `AnyState` per reconciler, plus the pre-move reconcile trajectory under
   a fixed seed — was captured and committed at 02-03, **before** 02-04 removed the
   free fns. DELIVER blocked S3 until the golden was committed.

---

## 5. Quality-gate outcomes (how we know it works)

- **Full workspace: 2777/2777 green.**
- **Behaviour-preservation (ADR-0086):** byte-identical characterization golden
  (hydrated `AnyState`) + DST replay-equivalence (same seed → bit-identical
  trajectory), both green against the pre-cut baseline.
- **Behaviour-change surface (ADR-0087):** the cause-carrying terminal reason
  (`ServiceFailed{LivenessProbeFailed}` ≠ `BackoffExhausted`) is pinned by the rkyv
  schema-evolution golden + Tier-1 DST trajectory tests — the four-tier gate.
- **Adversarial Opus review: PASSED** after one remediation pass (D1/D2/D4).
- **Mutation:** `transition_reason.rs` **6/6 = 100%**; plus manual temp-break
  flip-proofs for `is_liveness_killed` and the 3 `ServiceLifecycle` guards, where
  the extraction made clean diff-scoped mutation impractical (the pattern from the
  `cargo-mutants` blind-spot memory — a manual flip/revert proof stands in for a
  vacuous diff-scoped pass).
- **DES integrity: verified.**
- **dst-lint purity firewall:** the whole-crate `reconcile`-body scan over
  `overdrive-reconcilers/src/**` + the `hydrate_*`-no-`&dyn Clock` compile-guard both
  green — the pure-sync `reconcile` contract survives the move, which is what keeps
  DST replay intact (D8).

---

## 6. Verification catalogue — N/A (no expectations, none graduate)

This feature authored **no `verification/` expectations**, and that is correct.
It is an `@infrastructure` reconciler-framework refactor with no
operator-observable / qualitative surface: ADR-0086 is behaviour-preserving, and
ADR-0087's one operator-visible surface (the terminal reason) is covered by the
four-tier gate (rkyv golden + DST trajectory), not the executed-evidence catalogue
(`.claude/rules/verification.md`). The `distill/test-scenarios.md` scenarios are
all Tier-1/structural — none tagged `@driving_port` / `@walking_skeleton` — so none
graduate to `verification/expectations/`. No expectations were invented at finalize.

---

## 7. Evidence pointers

- **Decision records:**
  `docs/product/architecture/adr-0087-single-restart-authority-workload-lifecycle-owns-crash-and-liveness.md`,
  `docs/product/architecture/adr-0086-reconcilers-own-hydration-overdrive-reconcilers-crate.md`.
- **Feature workspace (preserved):**
  `docs/feature/reconcilers-own-hydration/` — `feature-delta.md` (DESIGN + DISTILL +
  DELIVER waves), `distill/test-scenarios.md` (24 scenarios, Buckets A/B),
  `deliver/{roadmap.json,execution-log.json}`.
- **The behaviour-lock tests:** the rkyv schema-evolution golden
  `FIXTURE_LIVENESS_PROBE_V1`, the pre-move characterization golden (hydrated
  `AnyState` + fixed-seed trajectory), and the Bucket-A DST trajectory + Bucket-B
  equivalence tests exercised via `Sim*` adapters and the 4 new `Sim*` read-ports.
- **Cross-feature SSOT:** `docs/product/architecture/brief.md` §
  "Shipped — Component Inventory (FINALIZE 2026-08-25) — reconcilers-own-hydration".

---

## 8. Follow-ups (tracked, not deferred here)

- **Enum collapse (#272)** — the 3 dispatch enums (`AnyReconciler`/`AnyState`/
  `AnyReconcilerView`) were kept as a pragmatic cut (ADR-0086 D2); collapsing them
  is explicitly out of this feature's scope. Not a deferral introduced here — a
  pre-existing scope boundary recorded for the next reader. No new issue filed at
  finalize.
