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
