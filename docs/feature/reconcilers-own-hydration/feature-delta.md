# Feature Delta — `reconcilers-own-hydration`

**Wave**: DESIGN (no DISCUSS — entered at DESIGN with option A locked).
**Authoritative ADRs**: ADR-0087 (precursor — single restart authority)
then ADR-0086 (hydration crate-move, 4 read-ports). **Amends**: ADR-0036
(in part); **supersedes** ADR-0055 §7 (LivenessRestartGovernor).
**Mode**: guide. **Density**: lean (Tier-1 `[REF]`).

**Re-design note (2026-08-25).** The original ADR-0086 introduced a fifth
read-port, `RestartBudgetView`, as a behaviour-preserving cycle-break for
`ServiceLifecycle`'s cross-read of the `WorkloadLifecycle` restart budget.
That cross-read is now **eliminated at its root** by ADR-0087 (single
restart authority, kubelet shape): `WorkloadLifecycle` owns crash **and**
liveness restart under one budget; `ServiceLifecycle` demotes to
readiness/membership + liveness-*terminate*. ADR-0087 lands as a precursor
slice; ADR-0086 then proceeds with **4** read-ports and no cross-read.

`docs/feature/reconcilers-own-hydration/discuss/*` — ⊘ not found
(expected; the decision was made before this wave).

---

## [REF] Problem

The reconciler diff (pure, `overdrive-core`) and its hydration
(impure, a ~1100-line central `match` free function in
`overdrive-control-plane/src/reconciler_runtime.rs`) live in different
crates. Hydration reads five control-plane/dataplane/runtime surfaces as
**concrete** `AppState` fields, so DST cannot substitute them — the
hydration boundary is invisible to the harness. Reverse the ADR-0036
"runtime owns all hydration" ruling for the intent+observation half:
reconcilers own their hydration; keep the dispatch enums; extract a new
crate; break the resulting Cargo cycle with narrow read-ports in core.

## [REF] Domain / DDD

Not a business-domain feature — an architectural refactor of the
**reconciler primitive** bounded context (whitepaper §18). No new
aggregates, no ubiquitous-language additions. The one modelling move:
promote the reconciler **contract** (trait + vocabulary) as the core
SSOT and confine reconciler **impls** (+ impure hydration) to an
adapter crate — the same ports-in-core / adapters-out split the
platform already holds for `IntentStore` / `Driver` / `ObservationStore`.

## [REF] Component decomposition

| Component | Home (after) | Responsibility |
|---|---|---|
| `Reconciler` trait (+ new async `hydrate_*`) | `overdrive-core` | reconciler contract: pure `reconcile` + impure `hydrate_desired`/`hydrate_actual` + `resync_schedule`/`interests` |
| `HydrationContext<'_>`, `HydrateError` | `overdrive-core` | borrow-bundle of injected read-ports + plain data; typed hydration error (`IntentRead`/`ObservationRead`/`IntentDecode`, `#[from]`→`ConvergenceError`) |
| 4 read-port traits (D5) | `overdrive-core` | narrow read projections of control-plane/dataplane state (no `RestartBudgetView` — ADR-0087 removes the cross-read) |
| `AnyReconciler` / `AnyState` / `AnyReconcilerView` | `overdrive-reconcilers` (NEW) | enum dispatch; forwards `reconcile` + `hydrate_*`, wraps `Self::{State,View}` at the boundary |
| 8 reconciler impls + per-reconciler `*State`/`*View` + `service_lifecycle` + pure helpers | `overdrive-reconcilers` (NEW) | the diffs + the moved hydration bodies |
| `ReconcilerRuntime` | `overdrive-control-plane` | builds a `HydrationContext` per tick; calls `AnyReconciler::hydrate_*`; owns view persistence (unchanged, ADR-0035). **Implements no read-port** (the former `RestartBudgetView` is gone) |
| 4 read-port impls | `overdrive-control-plane` / `overdrive-dataplane` | `ListenerFactStore`, `WorkflowEngine`, `IdentityMgr` (control-plane) + `PersistentServiceVipAllocator` (dataplane) implement the core read-ports |
| 4 `Sim*` read-port impls | `overdrive-sim` | make the hydration boundary DST-injectable |
| **ADR-0087 precursor** — `StoppedBy::LivenessProbe`, `WorkloadLifecycle::is_liveness_killed`, `ServiceLifecycle` liveness-terminate | `overdrive-core` | single restart authority; deletes the `restart_status_for_alloc` **hydration call** (keeps the method for streaming), `ServiceAllocFact.{restart_count,restart_spec}`, `RestartReason`. Lands BEFORE the crate-move |

## [REF] Ports

**Driven ports read during hydration (all read-only — Principle-12
read/write split satisfied by construction):**

- **Already core, no change**: `IntentStore`, `ObservationStore`,
  `VmHostState`, `DriverRegistry`+`Driver::live_allocations`.
- **NEW core read-ports** (ADR-0086 D5, full contracts there) — 4,
  all **driven** read-only ports: `ListenerFacts::fact_for` (async),
  `ServiceVipView::assigned_vip` (async),
  `WorkflowLiveSet::live_instances` (sync),
  `HeldSvidView::held_snapshot` (sync). **No `RestartBudgetView`** — the
  cross-reconciler restart-budget read is deleted by ADR-0087, not
  ported.

**Driving surface**: `AnyReconciler::hydrate_desired` /
`hydrate_actual` (called by `ReconcilerRuntime` per tick) and
`AnyReconciler::reconcile` (unchanged). `reconcile` stays pure-sync.

## [REF] Technology choices

No new dependencies. `#[async_trait]` for the read-ports + hydrate
methods (already the platform's async-trait convention, e.g.
`IntentStore`). `crate_class = "adapter-host"` for the new crate
(ADR-0086 D3 rationale). Enforcement: extend `xtask::dst_lint` (an AST
text scanner, no `overdrive-*` dep) with a targeted `reconcile`-body
clause over `overdrive-reconcilers` (ADR-0086 D7). No proprietary
tech; no external integrations (no contract-test annotation needed).

## [REF] Decisions

| # | Decision | Where |
|---|---|---|
| D1 | Reconcilers own `hydrate_*` as impure async trait methods; `AnyReconciler` forwards+wraps | ADR-0086 D1 |
| D2 | Keep the 3 enums (pragmatic cut; #272 out of scope) | ADR-0086 D2 |
| D3 | New crate `overdrive-reconcilers`, `crate_class = "adapter-host"`; contract stays in core, impls move | ADR-0086 D3 |
| D4 | Cycle broken: new crate depends only DOWN on core; read-ports impl'd UP | ADR-0086 D4 |
| D5 | Exactly 4 new read-ports (VmHostState + DriverRegistry already core; VIP allocator added). `RestartBudgetView` **removed** — ADR-0087 eliminates the cross-read | ADR-0086 D5 (amended) |
| D0 | **Precursor** — single restart authority: `WorkloadLifecycle` owns crash+liveness restart; `ServiceLifecycle` → readiness/membership + liveness-terminate; cross-read dissolved | ADR-0087 |
| D6 | `HeldSvidFacts` relocates to core (crosses a core trait signature) | ADR-0086 D6 |
| D7 | Purity firewall restored via targeted dst-lint `reconcile`-body scan; `ReconcilerIsPure` as backstop | ADR-0086 D7 |
| D8 | Pure-sync `reconcile` unchanged ⇒ DST replay survives; Sim read-ports = net injectability gain | ADR-0086 D8 |
| D9 | Single-cut migration; old free fns deleted in the same arc | ADR-0086 D9 |

## [REF] Reuse Analysis (mandatory gate)

Every overlapping surface cites contract shape (pure-function /
bounded-change / unbounded-preservation), universe, and the assertion
mechanism the crafter uses.

| Existing surface | Action | Contract shape | Universe | Assertion mechanism |
|---|---|---|---|---|
| `AnyReconciler::reconcile` enum forwarding | **Reuse pattern** — clone its exact shape for `hydrate_*` | pure-function (return-only): `reconcile`; the new `hydrate_*` are bounded-change (read-only over injected ports, mutate nothing) | reconciler dispatch enum | `reconciler_trait_signature...` compile guard + dst-lint `reconcile`-body scan (D7) |
| `IntentStore` / `ObservationStore` (core) | **Reuse** — read via `HydrationContext`, no new trait | pure-function (read); returns rows | intent/observation stores | existing trait contracts + DST sim adapters |
| `VmHostState` (core) | **Reuse** — already a core trait; no new trait | bounded-change (`observe()` reads host) | VM host observation | existing `SimVmHostState` |
| `DriverRegistry` + `Driver::live_allocations` (core) | **Reuse** — already core; no new trait | pure-function (read) over the registry | driver supervision set | existing `SimDriver` |
| `ListenerFactStore.fact_for` (control-plane) | **Extract** read-port `ListenerFacts` (core) | pure-function (read); `Option<ListenerRow>` | per-`ServiceId` listener facts | trait doc-contract + new `SimListenerFacts` |
| `PersistentServiceVipAllocator.get` (dataplane) | **Extract** read-port `ServiceVipView` (core) | pure-function (read); `Option<ServiceVip>` | VIP memo | trait doc-contract + new `SimServiceVipView` |
| `WorkflowEngine.live_instances` (control-plane) | **Extract** read-port `WorkflowLiveSet` (core) — narrow view ONLY, engine NOT relocated | pure-function (read); snapshot set | live-task correlation keys | trait doc-contract + new `SimWorkflowLiveSet` |
| `IdentityMgr.held_snapshot` (control-plane) | **Extract** read-port `HeldSvidView` (core) | pure-function (read); global held map | node-held SVID set | trait doc-contract + new `SimHeldSvidView` |
| `restart_status_for_alloc` **call** in `hydrate_service_alloc_facts` (`reconciler_runtime.rs:3419`) + `ServiceAllocFact.{restart_count,restart_spec}` + `RestartReason` + `Action::RestartAllocation.reason` | **Delete** (ADR-0087; the hydration cross-read + liveness-restart vocabulary are dead under single authority). **KEEP** the `restart_status_for_alloc` method (`:499`) — 4 live streaming callers (`streaming.rs:398/438/492/544`) render the operator attempt-index | n/a | n/a | absence of the hydration join proven by green build; no reconciler-hydration site left for the `.claude/rules/reconcilers.md` "single restart authority" symptom |
| `StoppedBy` (core `transition_reason`) | **Extend** — add `LivenessProbe` tail variant (ADR-0087) | value type (rkyv-additive) | terminal disposition | schema-evolution golden fixture + exhaustive-match compile-forcing (`by_reclaims_platform`) |
| `Action::StopAllocation` + action-shim executor + `is_restartable`/`is_intentionally_stopped` (core) | **Reuse** — ServiceLifecycle emits `StopAllocation { terminal: Stopped { by: LivenessProbe } }`; WorkloadLifecycle sees it restartable (ADR-0087) | bounded-change (emit-only; no new action/executor) | alloc lifecycle | preserved-liveness-terminal AT + DST trajectory test |
| `HeldSvidFacts` (core `svid_lifecycle`) | **Relocate** to a core module (stays in core) | value type (no behaviour) | held-SVID value | crosses `HeldSvidView` signature — compile-enforced |
| central `hydrate_*` free fns + 9 helpers (control-plane) | **Delete** (moved onto impls, single-cut) | n/a | n/a | absence proven by green build after S3 |
| `overdrive_core::reconcilers::{impls,enums,*State,*View}` imports (control-plane, sim ×8, tests) | **Rewrite** to `overdrive_reconcilers::` | n/a | n/a | green compile across workspace |

No component is created without an existing surface justification; the
only genuinely new artifacts are the 4 read-port traits + their Sim
impls + `HydrationContext`/`HydrateError` (ADR-0086), and the ADR-0087
`StoppedBy::LivenessProbe` variant + `is_liveness_killed` predicate —
each justified by a concrete `hydrate_*`-body read that must become
injectable, or by the single-authority behaviour change.

## [REF] Migration slices

**Precursor (ADR-0087) — single restart authority, cross-read-free.**
One single-cut arc, BEFORE the crate-move: add `StoppedBy::LivenessProbe`
+ `is_liveness_killed` + cause-aware exhaustion terminal in
`WorkloadLifecycle`; switch `ServiceLifecycle`'s liveness branch to
`StopAllocation { Stopped { by: LivenessProbe } }`; delete the
`restart_status_for_alloc` **hydration call** (keep the method — 4
streaming callers), `ServiceAllocFact.{restart_count,restart_spec}`,
`RestartReason`, `Action::RestartAllocation.reason`. Tests: preserved
liveness terminal (`ServiceFailed { LivenessProbeFailed }` ≠
`BackoffExhausted`), budget unification (one pool spans crash+liveness),
liveness-not-exempt (consumes budget, unlike platform-reclaim), a Tier-1
DST trajectory (`SimClock`/`SimDriver`/`SimObservationStore`), and a
mutation gate on `is_liveness_killed` + the terminal-selection branch.

**Then ADR-0086** § "Migration slice sketch" (S1–S4, single-cut, 4
read-ports, no `RestartBudgetView`). Not a roadmap.json — that is a later
`/nw-roadmap` step.

## [REF] Open questions / blockers for the user

1. **`LivenessProbeFailed.attempts` value-semantics shift** (ADR-0087
   D4; the one contested-but-forced sub-decision). Today
   `ServiceLifecycle` stamps `attempts = consecutive liveness failures`
   at exhaustion; under single authority `WorkloadLifecycle` stamps
   `attempts = restart-budget count consumed` (= `CEILING`). This is
   **forced** — preserving `consecutive_failures` would require the very
   cross-read being eliminated — and is the more consistent reading
   (parallels `BackoffExhausted.attempts` — same "attempts consumed"
   meaning). Recommendation: adopt the restart-count reading. Surfaced for
   the user to confirm; not a blocker (the alternative reintroduces the
   cross-read).
2. **Crate-class taxonomy fit** (non-blocking, resolved to
   `adapter-host` per the task hint). The new crate *consumes* ports
   rather than *binding* a host primitive — a loose but correct fit
   among ADR-0003's four classes. Flagged, not blocking; a 5th class
   would need an ADR-0003 amendment (out of scope).
3. No other contested sub-decision surfaced — the move/stay boundary,
   the 4-trait set, `HeldSvidFacts` relocation (ADR-0086 D3/D5/D6), the
   liveness→terminate mechanism, and the cause-carrying surface
   (`StoppedBy::LivenessProbe` on the observed row, ADR-0087 D3) are all
   pinned from code.

---

## Wave: DISTILL

**Mode**: guide. **Density**: lean (Tier-1 `[REF]`). Acceptance scenarios are
**specification prose** — GIVEN/WHEN/THEN companion at
`distill/test-scenarios.md`, never parsed/executed (`.claude/rules/testing.md`:
no `.feature`, no pytest-bdd). The DELIVER crafter translates each into a Rust
`#[test]` / `#[tokio::test]` (Tier-1 DST via `Sim*` adapters, default-lane unit,
rkyv schema-evolution, or trybuild/dst-lint structural, per each scenario's tag).
**No new driving port** — this is an internal reconciler-framework refactor; the
exercised entry is the **reconciler runtime tick** (observation rows + hydrated
`State` → `reconcile` / `hydrate_*` → emitted `Action` / next `State` / DST
trajectory).

### [REF] Test scenarios (Tier-1 primary)

**Bucket A — ADR-0087 single restart authority (BEHAVIOUR CHANGE; new scenarios).**

| ID | Scenario | Tier | Kind | Traces |
|---|---|---|---|---|
| S-ROH-A-01 | Liveness threshold emits `StopAllocation{Stopped{by:LivenessProbe}}`, reads no budget | Tier-1 DST | happy | ADR-0087 D1/D2/D3 |
| S-ROH-A-02 | `WorkloadLifecycle` restarts the liveness-terminated row under its single `restart_counts` | Tier-1 DST | happy | ADR-0087 D4/D5 |
| S-ROH-A-03 | Budget exhaustion → `ServiceFailed{LivenessProbeFailed}` ≠ `BackoffExhausted` | Tier-1 DST | error | ADR-0087 D4 (HC-1) |
| S-ROH-A-04 | Liveness consumes budget; only genuine platform-reclaim is exempt | Tier-1 DST | edge | ADR-0087 D5 |
| S-ROH-A-05 | Operator/SystemGc stop is NEVER restarted; `LivenessProbe` doesn't widen the set | default-lane unit | error/regression | ADR-0087 D4 (HC-2) |
| S-ROH-A-06 | Full DST trajectory → `assert_eventually!` `ServiceFailed{LivenessProbeFailed}` + `restart_counts==CEILING` | Tier-1 DST | e2e trajectory | ADR-0087 D6 |
| S-ROH-A-07 | `LivenessProbeFailed.attempts == CEILING` (restart-consumed), not `consecutive_failures` | Tier-1 DST | contested-decided | ADR-0087 D4 / OQ-1 |
| S-ROH-A-08 | Budget unification: interleaved crash+liveness draw ONE pool; last-cause-wins | Tier-1 DST | edge | ADR-0087 D5 |
| S-ROH-A-09 | ServiceLifecycle liveness-terminate idempotent while stop in flight (counter-reset) | Tier-1 DST | edge/idempotency | ADR-0087 D2 |
| S-ROH-A-10 | WorkloadLifecycle exhaustion idempotent across BOTH terminal kinds | Tier-1 DST | edge/idempotency | ADR-0087 D4 |
| S-ROH-A-11 | `is_liveness_killed` dual-field (terminal primary, reason defensive) | default-lane unit | edge (mutation-gate) | ADR-0087 D4 |
| S-ROH-A-12 | `StoppedBy::LivenessProbe` additive rkyv tail; existing `FIXTURE_Vn` decode unchanged + 1 new | schema-evolution | regression | ADR-0087 §Compliance |
| S-ROH-A-13 | Cross-read gone (green-build absence); streaming method survives | structural | regression/absence | ADR-0087 D7 |

**Bucket B — ADR-0086 hydration move (BEHAVIOUR-PRESERVING; equivalence / regression / structural).**

| ID | Scenario | Tier | Kind | Traces |
|---|---|---|---|---|
| S-ROH-B-01 | Each of 4 `Sim*` read-ports reproduces the pre-move hydrated `State` (characterization) | Tier-1 DST | equivalence | ADR-0086 D5/D8 |
| S-ROH-B-02 | DST replay-equivalence: same seed → bit-identical trajectory | Tier-1 DST | equivalence | ADR-0086 D8 |
| S-ROH-B-03 | `AnyReconciler::hydrate_*` forwarding → same `AnyState` variant as deleted free fns | Tier-1 DST | equivalence | ADR-0086 D1/D2 |
| S-ROH-B-04 | Purity firewall dst-lint scan fires on planted violation; `ReconcilerIsPure` backstop | structural + DST | negative/purity | ADR-0086 D7 |
| S-ROH-B-05 | Empty/stale `SimWorkflowLiveSet` → crash-resume trigger (injectability WIN) | Tier-1 DST | edge | ADR-0086 D5/D8 |
| S-ROH-B-06 | `SimListenerFacts` None → hydrator SKIPS, never defaults `Proto::Tcp` | Tier-1 DST | edge/error | ADR-0086 D5 (C3) |
| S-ROH-B-07 | `SimServiceVipView` memo-absent → defer tick, log `allocator_memo_absent` | Tier-1 DST | edge/error | ADR-0086 D5 (§4) |
| S-ROH-B-08 | `HeldSvidView` global set; hydrator filters by `SpiffeId::for_allocation` | Tier-1 DST | edge/equivalence | ADR-0086 D5 (D5b) |
| S-ROH-B-09 | `HydrationContext` S1 audit — every read surface represented; no unrepresented `state.*` | structural (S1 gate) | audit | ADR-0086 D5 / S1 |
| S-ROH-B-10 | Compile guard — `reconcile` sync; `hydrate_*` carry no `&dyn Clock` | structural (compile) | audit | ADR-0086 D1 |
| S-ROH-B-11 | Central `hydrate_*` free fns gone (green-build absence); no second hydration path survives | structural | regression/absence | ADR-0086 D9/S3 |

### [REF] Tier mapping

- **Tier-1 DST (primary)** — `Sim*` adapters + the 4 new `Sim*` read-ports;
  `assert_always!` / `assert_eventually!`; seed-reproducible. 16 of 24 scenarios.
- **default-lane unit** — pure-Rust predicate / terminal-selection tests (A-05,
  A-11). Tier-1-class fast lane.
- **schema-evolution** — rkyv golden-bytes fixture (A-12); default lane; existing
  `FIXTURE_Vn` untouched, one new fixture added.
- **structural** — compile-guard / dst-lint AST scan / green-build absence / S1
  read-surface audit (A-13, B-04, B-09, B-10, B-11). Not a runtime tier.
- **Tier-3** — **none.** Both ADRs are net-positive for DST-testability (ADR-0087
  D6: liveness path becomes purely observation-row-driven; ADR-0086 D8: hydration
  boundary becomes injectable). The real `probe_runner` is an unchanged
  observation-producer, out of scope — reconciler logic is fed `ProbeResultRow`s /
  `AllocStatusRow`s via `SimObservationStore`, so no real kernel/cgroup test is
  needed for the changed behaviour.
- **Mutation-gate targets** (`.claude/rules/testing.md` — reconciler logic is
  mandatory): `is_liveness_killed` (A-11) and the `WorkloadLifecycle`
  terminal-selection branch (A-03).

### [REF] Error / edge coverage

15 of 24 scenarios are error / edge / regression / negative (A: 03,04,05,08,09,10,
11,12,13; B: 04,05,06,07,08,11) ≈ **63%** — above the ≥40% `.claude/rules/testing.md`
discipline bar.

### [REF] Prerequisites

- **ADR-0087 lands first** (precursor slice) — removes the
  `restart_status_for_alloc` cross-read so ADR-0086's hydration move never ports a
  `RestartBudgetView`.
- Bucket B needs the 4 `Sim*` read-port impls + `HydrationContext` / `HydrateError`
  core types (ADR-0086 S1/S4).
- **HARD DELIVER GATE** — Bucket B equivalence (B-01/02/03) REQUIRES a
  **characterization golden** (the pre-move hydrated `AnyState`, plus the pre-move
  reconcile trajectory under a fixed seed for B-02) captured and committed **at/before
  S2, before the single-cut S3 deletes the central `hydrate_*` free fns**. No live
  old-vs-new diff exists after S3, so without the golden B-01/02/03 have no expected
  baseline. DELIVER blocks S3 until it is committed (OQ-2).

### [REF] DISTILL open questions + one hard DELIVER gate

1. **`LivenessProbeFailed.attempts` semantics** (S-ROH-A-07) — **CONFIRMED /
   LOCKED**. The scenario pins the ADR-0087 D4 value (`attempts == CEILING`,
   restart-consumed); the user has confirmed it as the locked reading. The
   alternative (`consecutive_failures`) reintroduces the eliminated cross-read.
   Mirrors DESIGN Open Question 1. Not open.
2. **Equivalence-baseline capture — HARD DELIVER GATE** (S-ROH-B-01/02/03). The
   move is single-cut: ADR-0086 S3 deletes the central `hydrate_*` free fns, so
   there is no live old-vs-new diff after the cut. B-01/B-02/B-03 therefore REQUIRE
   a **characterization golden** — the pre-move hydrated `AnyState` (and, for B-02,
   the pre-move reconcile trajectory under a fixed seed) — **captured and committed
   at/before S2, before S3 removes the free fns**. Without it, B-01/B-02/B-03 have
   no expected baseline. This is a hard sequencing gate, not a soft note: DELIVER
   blocks S3 until the golden is committed.

**No hard DESIGN blockers** — every scenario's design detail is pinned by ADR-0086
or ADR-0087; nothing required inventing un-specified surface. The single hard gate
is the DELIVER sequencing prerequisite in item 2 (capture the characterization
golden before the S3 deletion).

---

## Wave: DELIVER

**Mode**: guide. **Density**: lean (Tier-1 `[REF]`). Landed in **7 DES-monitored
steps** (01-01…01-02 for ADR-0087, 02-01…02-05 for ADR-0086); every step
RED/GREEN/COMMIT logged; DES integrity verified. Full workspace **2777/2777 green**.
Adversarial Opus review **passed after one remediation pass** (D1/D2/D4). Evolution
record: `docs/evolution/reconcilers-own-hydration-evolution.md`.

### [REF] Implementation summary

A two-ADR refactor of the reconciler primitive, sequenced precursor-first: ADR-0087
consolidates restart authority so `WorkloadLifecycle` owns crash **and** liveness
restart under one `restart_counts` budget (kubelet shape), dissolving the
`ServiceLifecycle`→`WorkloadLifecycle` cross-read at its root; ADR-0086 then moves
reconciler hydration off the central ~1100-line control-plane `match` free fns onto
the reconciler impls, extracts the impls + dispatch enums into a new
`overdrive-reconcilers` crate, and breaks the resulting Cargo cycle with 4 narrow
core read-ports. ADR-0086 is behaviour-preserving (byte-identical characterization
golden + full suite green); ADR-0087 is a deliberate, cause-carrying behaviour
change on the liveness terminal reason only.

### [REF] Files modified (categorized — one line per group)

| Group | Change |
|---|---|
| **NEW crate `overdrive-reconcilers`** (`crate_class = "adapter-host"`) | the 8 reconciler impls + 3 dispatch enums (`AnyReconciler`/`AnyState`/`AnyReconcilerView`) + `service_lifecycle` + per-reconciler `*State`/`*View` + pure helpers + the moved `hydrate_*` bodies — all OUT of `overdrive-core`; depends only DOWN on core |
| **`overdrive-core` additions** | 4 driven read-port traits (`ListenerFacts`/`ServiceVipView`/`WorkflowLiveSet`/`HeldSvidView` — **no `RestartBudgetView`**); `HydrationContext<'_>`; `HydrateError`; `HeldSvidFacts` relocated to core; `Reconciler::hydrate_desired`/`hydrate_actual` async methods; `StoppedBy::LivenessProbe` (fieldless rkyv tail, discriminant 5) + `is_liveness_killed` + cause-aware exhaustion terminal |
| **`overdrive-control-plane`** | central `hydrate_*` free fns + ~11 helpers DELETED (single-cut); `ReconcilerRuntime` builds a `HydrationContext` per tick + calls `AnyReconciler::hydrate_*`; 3 read-port impls (`ListenerFactStore`/`WorkflowEngine`/`IdentityMgr`); ADR-0087 cross-read (`restart_status_for_alloc` CALL) + `ServiceAllocFact.{restart_count,restart_spec}` + `RestartReason` + `Action::RestartAllocation.reason` DELETED (the `restart_status_for_alloc` **method** KEPT — 4 live streaming callers) |
| **`overdrive-dataplane`** | `PersistentServiceVipAllocator` implements the core `ServiceVipView` read-port |
| **`overdrive-sim`** | 4 `Sim*` read-port impls — hydration boundary is now DST-injectable |
| **`xtask` (dst-lint)** | whole-crate purity clause over `overdrive-reconcilers/src/**`; additive compile-guard (`hydrate_*` carry no `&dyn Clock`) |
| **Tests** | rkyv schema-evolution golden `FIXTURE_LIVENESS_PROBE_V1` (existing fixtures untouched); the pre-move **characterization golden** (hydrated `AnyState` per reconciler + fixed-seed reconcile trajectory) captured at 02-03 BEFORE the S3 cut; Bucket-A DST trajectory + Bucket-B equivalence tests |

### [REF] DoD / quality gates

- All **7 steps green** — RED/GREEN/COMMIT PASS each; **DES integrity verified**.
- Full workspace **2777/2777 green**.
- **Adversarial Opus review passed** after remediation of D1/D2/D4 (commits
  `b77d5786` / `688c7567` / `f5459f19`).
- **Mutation**: `transition_reason.rs` **6/6 = 100%**; plus manual temp-break
  flip-proofs for `is_liveness_killed` and the 3 `ServiceLifecycle` guards, where
  the extraction made clean diff-scoped mutation impractical.
- **Cargo cycle broken**: `overdrive-reconcilers → overdrive-core` (down);
  `overdrive-core → overdrive-reconcilers` is **dev-dep only**; the contract stays in
  core (green compile across the workspace).

### [REF] Prerequisites (satisfied in order)

1. **ADR-0087 precursor landed BEFORE ADR-0086** (01-01/01-02 before 02-*) — so the
   hydration move never ports a `RestartBudgetView`; the cross-read was dissolved at
   its root, not relocated.
2. **Characterization golden captured/committed at 02-03 (S2-gate), BEFORE the S3
   single-cut (02-04)** deleted the central `hydrate_*` free fns — the HARD DELIVER
   GATE from DISTILL OQ-2. No live old-vs-new diff exists after S3, so B-01/B-02/B-03
   equivalence has a baseline only because the golden predates the cut.

### [REF] Verification catalogue — N/A (no expectations authored, none graduate)

`verification/` expectations are **N/A** for this feature. It is an
`@infrastructure` reconciler-framework refactor with **no operator-observable /
qualitative surface**: ADR-0086 is behaviour-preserving, and ADR-0087's one
operator-visible surface — the terminal reason (`ServiceFailed{LivenessProbeFailed}`
≠ `BackoffExhausted`) — is covered by the rkyv schema-evolution golden + the DST
trajectory tests, which are the **four-tier gate** (`.claude/rules/testing.md`), not
the executed-evidence catalogue (`.claude/rules/verification.md`). No DISTILL
expectations were authored (the `distill/test-scenarios.md` scenarios are all
Tier-1/structural, none tagged `@driving_port`/`@walking_skeleton`), so none
graduate to `verification/expectations/`. No verification expectations were invented.
