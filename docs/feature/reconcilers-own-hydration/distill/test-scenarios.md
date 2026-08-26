# Test Scenarios — `reconcilers-own-hydration` (DISTILL)

**Specification prose only.** Per `.claude/rules/testing.md` these GIVEN/WHEN/THEN
blocks are a specification companion — they are **never parsed or executed**. The
DELIVER crafter translates each into a Rust `#[test]` / `#[tokio::test]` (Tier-1
DST via `Sim*` adapters, or a default-lane unit / schema-evolution / trybuild test
as tagged). **No `.feature` files, no pytest-bdd** — this project forbids both.

**Authoritative design**: ADR-0087 (precursor — single restart authority) then
ADR-0086 (hydration crate-move, 4 read-ports). Amends ADR-0036 in part.

**Driving surface**: there is **no** CLI / HTTP / hook driving port in this
feature — it is an internal reconciler-framework refactor. The exercised entry is
the **reconciler runtime tick**: observation rows + hydrated `State` → `reconcile`
(and, for ADR-0086, `hydrate_*`) → emitted `Action` / next `State` / (under DST)
an eventual observation-row trajectory. Every scenario is framed at that boundary.

**Tier legend** (`.claude/rules/testing.md`):

- **Tier-1 DST** — in-process, `Sim*` adapters (`SimClock`, `SimDriver`,
  `SimObservationStore`, and the 4 new `Sim*` read-ports), `assert_always!` /
  `assert_eventually!`, seed-reproducible. **Primary tier for this feature.**
- **default-lane unit** — pure-Rust `#[test]` (predicate logic, terminal
  selection), no `Sim*` harness needed. Still Tier-1-class (fast lane).
- **schema-evolution** — rkyv golden-bytes fixture (`.claude/rules/testing.md`
  § "Archive schema-evolution roundtrip"), default lane.
- **structural** — compile-guard (trybuild-shaped) / dst-lint AST scan / green-build
  absence proof / `xtask` test. Not a runtime tier.
- **Tier-3** — real kernel / cgroup / probe. **None required** — see the Tier-3
  note at the foot.

**Observable universe** (Rust analogue of the state-delta Universe): assertions
observe only port-exposed surfaces — emitted `Action`s, `AllocStatusRow.{state,
terminal, reason, restart_count}`, the reconciler's next `View` (`restart_counts`,
`liveness_consecutive_failures`), the hydrated `AnyState` variant, and the
observation-row trajectory. Never a private struct field.

`RESTART_BACKOFF_CEILING` = **5** throughout.

---

## Bucket A — ADR-0087 single restart authority (BEHAVIOUR CHANGE)

New behaviour; needs genuine new scenarios. `WorkloadLifecycle` becomes the sole
restart authority; `ServiceLifecycle` demotes to readiness/membership +
liveness-**terminate**. The cause travels on the shared observed `AllocStatusRow`
via a new `StoppedBy::LivenessProbe` disposition.

### S-ROH-A-01 — Liveness threshold emits a terminate, reads no budget

**Tier-1 DST** · traces ADR-0087 D1/D2/D3 · covers task-bucket-A #1

```
GIVEN a Service allocation in state Running with a liveness probe declared
  AND its liveness consecutive-failure counter sits one below liveness_failure_threshold
  AND a fresh liveness-Fail ProbeResultRow arrives, taking the counter to the threshold
WHEN the ServiceLifecycle reconciler ticks
THEN it emits exactly Action::StopAllocation { alloc_id, terminal:
     Some(TerminalCondition::Stopped { by: StoppedBy::LivenessProbe }) }
  AND it emits NO RestartAllocation and NO FinalizeFailed on the liveness path
  AND its reconcile reads no restart budget and makes no restart-vs-finalize decision
  AND the readiness -> Backend.healthy -> ServiceBackendRow membership branch is unaffected
```

Universe: emitted `Action` set; `ServiceLifecycleView.liveness_consecutive_failures`.
Note: the reconcile signature must not reach any restart-budget surface — this is
the ADR-0087 property that lets ADR-0086 drop `RestartBudgetView`.

### S-ROH-A-02 — WorkloadLifecycle restarts the liveness-terminated row under its single budget

**Tier-1 DST** · traces ADR-0087 D4/D5 · covers task-bucket-A #2

```
GIVEN an AllocStatusRow in state Terminated with terminal =
      Stopped { by: StoppedBy::LivenessProbe }
  AND the workload's intent still stands (Job present)
  AND view.restart_counts for this alloc is below RESTART_BACKOFF_CEILING
WHEN the WorkloadLifecycle reconciler ticks (woken by the alloc_status change)
THEN is_restartable(row) is true (LivenessProbe is NOT an intentional stop)
  AND it emits Action::RestartAllocation (reason: None — the cause is the prior
      row's observable Stopped { by: LivenessProbe } terminal, not a field on the action)
  AND next_view.restart_counts for this alloc is incremented by exactly one
  AND the increment lands at WorkloadLifecycle's single existing increment site
      (crash and liveness share ONE counter)
  AND on the resulting restart the observed AllocStatusRow.restart_count (CrashFacts —
      COMPLETED restarts, ADR-0078 §D3) ALSO advances on this liveness path
  AND that observed restart_count stays a DISTINCT number from view.restart_counts
      (attempts): the two ADR-0078 §D3 quantities remain separate across the new
      single-authority liveness path — completed-restarts is never conflated with attempts
```

Universe: emitted `Action`; `WorkloadLifecycleView.restart_counts` (attempts); the
observed `AllocStatusRow.restart_count` (CrashFacts completed-restarts) — asserted to
advance AND to stay distinct from `restart_counts`, per ADR-0078 §D3.

### S-ROH-A-03 — Budget exhaustion on a liveness loop finalises as `ServiceFailed{LivenessProbeFailed}`, not `BackoffExhausted`

**Tier-1 DST** · **error path** · traces ADR-0087 D4 (Hard Constraint 1) · covers task-bucket-A #3

```
GIVEN an AllocStatusRow in state Terminated with terminal =
      Stopped { by: StoppedBy::LivenessProbe }
  AND view.restart_counts for this alloc == RESTART_BACKOFF_CEILING (5)
  AND the row is not already carrying a finalised terminal
WHEN the WorkloadLifecycle reconciler ticks
THEN is_liveness_killed(row) is true
  AND it emits FinalizeFailed { terminal: Some(ServiceFailed {
      reason: LivenessProbeFailed { probe_idx: 0, attempts } }) }
  AND it does NOT emit BackoffExhausted
  AND a crash loop on an identically-shaped alloc (terminal NOT LivenessProbe) at
      the same ceiling instead finalises as BackoffExhausted { attempts }
      (the two are distinguished on the same alloc shape)
```

Universe: emitted `Action` terminal variant. Mutation-gate target (terminal-selection branch).

### S-ROH-A-04 — A liveness kill consumes budget; only genuine platform-reclaim is exempt

**Tier-1 DST** · **edge** · traces ADR-0087 D5 · covers task-bucket-A #4

```
GIVEN an alloc that has failed its liveness probe N times, each producing a
      Stopped { by: StoppedBy::LivenessProbe } Terminated row and a restart
WHEN N reaches RESTART_BACKOFF_CEILING
THEN the budget is exhausted and the terminal fires (liveness is crash-class:
     by_reclaims_platform(LivenessProbe) == false)
  AND by contrast, an alloc reclaimed by the platform N times
      (Stopped { by: PlatformReclaimed }) does NOT exhaust the budget —
      is_platform_reclaimed exempts the ceiling CHECK, and it is re-driven each cycle
```

Universe: emitted `Action`; `restart_counts`. Pins the `LivenessProbe => false`
arm of `by_reclaims_platform` against the platform-reclaim exemption.

### S-ROH-A-05 — Operator / SystemGc stop is NEVER restarted (discriminator regression guard)

**default-lane unit** · **error path / regression** · traces ADR-0087 D4 (Hard Constraint 2) · covers task-bucket-A #5

```
GIVEN a Terminated AllocStatusRow with terminal = Stopped { by: StoppedBy::Operator }
   OR terminal = Stopped { by: StoppedBy::SystemGc }
WHEN is_intentionally_stopped and is_restartable are evaluated
THEN is_intentionally_stopped is true and is_restartable is false (never restarted)
  AND adding StoppedBy::LivenessProbe does NOT widen the intentional-stop set —
      is_intentionally_stopped still matches ONLY Operator | SystemGc
  AND a Stopped { by: LivenessProbe } row is is_restartable == true
```

Universe: predicate return values. Guards against the new tail variant accidentally
leaking into the intentional-stop discriminator.

### S-ROH-A-06 — Full liveness restart-loop trajectory converges to `ServiceFailed{LivenessProbeFailed}` (end-to-end DST)

**Tier-1 DST** · traces ADR-0087 D6 + § Migration "DST trajectory" · covers task-bucket-A #6

```
GIVEN SimClock, SimDriver, SimObservationStore and a Running Service alloc with a liveness probe
WHEN liveness-Fail ProbeResultRows are injected repeatedly and the clock is advanced
     across each backoff window
THEN the trajectory is: liveness-Fail rows -> ServiceLifecycle StopAllocation ->
     shim writes Stopped { by: LivenessProbe } -> WorkloadLifecycle RestartAllocation
     (restart_counts++) -> ... -> exhaust ->
     assert_eventually!(final AllocStatusRow.terminal == ServiceFailed { LivenessProbeFailed })
  AND assert_eventually!(restart_counts == RESTART_BACKOFF_CEILING)
  AND the run is replay-equivalent under its seed (seed printed on failure)
```

Universe: observation-row trajectory; `restart_counts`. This is the `assert_eventually!`-shaped
liveness property the ADR mandates.

### S-ROH-A-07 — `LivenessProbeFailed.attempts` reads restart-budget-consumed (= CEILING), not consecutive-failures

**Tier-1 DST** · **contested-but-decided** · traces ADR-0087 D4 (attempts-semantics) + feature-delta Open Question 1 · covers task-bucket-A #7

```
GIVEN a liveness loop that exhausts the shared budget with restart_counts == CEILING
  AND the consecutive-liveness-failure streak at the deciding tick is some value F (F != CEILING)
WHEN WorkloadLifecycle stamps the ServiceFailed { LivenessProbeFailed { attempts } } terminal
THEN attempts == RESTART_BACKOFF_CEILING (restart-budget count consumed)
  AND attempts is NOT the consecutive-liveness-failure streak F
  AND this parallels BackoffExhausted { attempts } (same "attempts consumed" meaning)
```

Universe: the `attempts` field on the finalised terminal. **Pinned to the locked
(user-confirmed) ADR-0087 D4 semantics** so DELIVER implements the restart-count
reading. See Open Question 1 (now CONFIRMED / LOCKED) — the alternative (streak F)
would reintroduce the eliminated cross-read.

### S-ROH-A-08 — Budget unification: interleaved crash + liveness draw ONE pool (kubelet shape)

**Tier-1 DST** · **edge** · traces ADR-0087 D5 + § Consequences (last-cause-wins) · ADR-mandated "budget-unification" test

```
GIVEN one alloc that both crashes and fails liveness before the shared budget is spent
WHEN the restarts interleave through WorkloadLifecycle's single increment site
THEN total restarts across BOTH causes are capped at RESTART_BACKOFF_CEILING
     (one budget, not two)
  AND at exhaustion the terminal reflects the MOST RECENT kill's cause
     (is_liveness_killed reads the latest row: last-cause-wins, kubelet single-RESTARTS shape)
```

Universe: `restart_counts`; final terminal variant. Distinguishes the unified
budget from the old split-by-cause behaviour.

### S-ROH-A-09 — ServiceLifecycle liveness-terminate is idempotent while the stop is in flight

**Tier-1 DST** · **edge / idempotency** · traces ADR-0087 D2 (counter-reset retained)

```
GIVEN a Running Service alloc whose liveness counter has reached the threshold
  AND ServiceLifecycle has already emitted StopAllocation { Stopped { by: LivenessProbe } }
      on the prior tick (counter reset-on-emit; row still Running because the shim's
      stop is in flight)
WHEN ServiceLifecycle ticks again before the row leaves Running
THEN no second StopAllocation is emitted for the same alloc
  AND once the row leaves Running (state != Running) the predicate is false by construction
  AND after restart the fresh Running alloc starts with a clean consecutive-failure counter
```

Universe: emitted `Action` set; `liveness_consecutive_failures`. Guards the
double-terminate race.

### S-ROH-A-10 — WorkloadLifecycle exhaustion is idempotent across BOTH terminal kinds

**Tier-1 DST** · **edge / idempotency** · traces ADR-0087 D4 (idempotency guard extended)

```
GIVEN an alloc at the ceiling whose row already carries terminal =
      ServiceFailed { LivenessProbeFailed { .. } }  OR  BackoffExhausted { .. }
WHEN WorkloadLifecycle ticks again (level-triggered re-enqueue)
THEN it re-emits NO FinalizeFailed for either terminal kind
     (the idempotency guard covers both, not just BackoffExhausted)
```

Universe: emitted `Action` set (must be empty). Extends the pre-existing
`BackoffExhausted`-only guard to the new `LivenessProbeFailed` terminal.

### S-ROH-A-11 — `is_liveness_killed` is dual-field (terminal primary, reason defensive)

**default-lane unit** · **edge** · traces ADR-0087 D4 · mutation-gate target

```
GIVEN a Terminated row carrying Stopped { by: LivenessProbe } on `terminal`
      (the live shim path — StopAllocation hardcodes reason = Reconciler)
WHEN is_liveness_killed(row) is evaluated
THEN it returns true via the `terminal` branch
  AND a row carrying the marker on `reason` instead ALSO returns true (defensive
      branch, mirrors is_platform_reclaimed's dual-field shape — not a live path today)
  AND a non-Terminated row, or one with any other StoppedBy, returns false
```

Universe: predicate return value. Mutation-gate target (per ADR-0087 § Migration).

Regression guard (existing coverage — NOT a new scenario): the `StopAllocation`
action-shim executor still writes `reason = Stopped { by: Reconciler }` for a liveness
stop — the `LivenessProbe` cause travels on `terminal` ONLY. Changing the shim reason
would regress wire-side `last_transition.reason` for EVERY stop (not just liveness).
This invariant is already covered by the unchanged shim-executor test
(`action_shim/mod.rs` ~:1967-1991); coverage is explicitly that existing test — no
new scenario is added here.

### S-ROH-A-12 — `Stopped{by:LivenessProbe}` is an additive rkyv tail variant; existing fixtures decode unchanged

**schema-evolution** · **regression** · traces ADR-0087 § Compliance (rkyv) + § Migration step 1

```
GIVEN the existing StoppedBy golden-bytes fixtures (FIXTURE_V1 .. FIXTURE_Vn)
WHEN StoppedBy::LivenessProbe is appended as a fieldless tail variant (discriminant 5)
THEN every existing FIXTURE_Vn decodes UNCHANGED through the envelope (no layout growth —
     fieldless variant, no max-variant size change; existing FIXTURE_Vn are NEVER re-minted)
  AND ONE new golden fixture is added covering an AllocStatusRow carrying the
      LivenessProbe disposition, round-tripping archive -> access -> into_latest
```

Universe: archived bytes -> decoded value. Default lane, pure-Rust. NOT Tier-3.

### S-ROH-A-13 — The cross-read is gone; the streaming method survives (structural)

**structural (green-build absence)** · **regression** · traces ADR-0087 D7 + § Migration "Cross-read-gone"

```
GIVEN the single-authority arc has landed
WHEN the reconciler-hydration path is inspected
THEN the restart_status_for_alloc CALL in hydrate_service_alloc_facts is absent,
     ServiceAllocFact.{restart_count, restart_spec} are absent, RestartReason and
     Action::RestartAllocation.reason are absent (all proven by green build)
  AND the `.claude/rules/reconcilers.md` "single restart authority" symptom
     (a reconciler reading another's View during hydration) has NO site left
  AND the ReconcilerRuntime::restart_status_for_alloc METHOD survives with its four
     live streaming.rs callers (:398/:438/:492/:544 — operator attempt-index),
     because those are the streaming/event layer, not a reconciler-hydration read
```

Universe: presence/absence of named surfaces (compile + grep). Structural, not
runtime; but the surviving-method half is a genuine regression guard.

---

## Bucket B — ADR-0086 hydration move (BEHAVIOUR-PRESERVING — equivalence / regression)

**All Bucket B scenarios are equivalence / regression / structural bars, not new
behaviour.** The refactor moves `hydrate_*` off the central free functions and onto
the `Reconciler` trait, extracts `overdrive-reconcilers`, and injects 4 narrow
read-ports. The behaviour the reconcilers exhibit must be **identical** after the
move; the net gain is that the hydration boundary becomes DST-injectable.

**Equivalence-baseline note (crafter prerequisite):** the migration is single-cut —
the old central `hydrate_*` free fns are DELETED in the same arc (ADR-0086 D9 / S3).
There is therefore **no live old-vs-new A/B diff** available after the cut. The
equivalence bar is expressed as a **characterization** bar: pin the pre-move
hydrated `AnyState` for a representative row set as the expected golden (captured at
or before S2, while both paths still exist), then assert the port-driven
`hydrate_*` reproduces it. See Open Question 2.

### S-ROH-B-01 — Each of the 4 read-ports has a `Sim*` impl reproducing the pre-move hydrated `State`

**Tier-1 DST** · **equivalence** · traces ADR-0086 D5/D8 · covers task-bucket-B #8

```
GIVEN a SimListenerFacts / SimServiceVipView / SimWorkflowLiveSet / SimHeldSvidView
      seeded to the same facts the concrete AppState field held pre-move
WHEN the owning reconciler hydrates its State via HydrationContext (through the port)
THEN the hydrated State equals the characterization golden the deleted central
     hydrate_* produced for the same inputs
  AND this holds for each of the 4 read-ports (parameterised: one case per port)
```

Universe: hydrated `AnyState` variant. Equivalence characterization, per port.

### S-ROH-B-02 — DST replay-equivalence survives the move (same seed → bit-identical trajectory)

**Tier-1 DST** · **equivalence** · traces ADR-0086 D8 · covers task-bucket-B #9

```
GIVEN a DST scenario driving reconcile + hydration through the Sim read-ports under a fixed seed
WHEN the run is executed twice with the same seed
THEN the reconcile trajectory (emitted Actions + View evolution + observation rows)
     is bit-identical across the two runs
  AND it matches the trajectory the pre-move central-hydration path produced for the
     same seed (single-loop / single-clock replay-equivalence model unchanged)
```

Universe: full reconcile trajectory under the seed. The `assert_replay_equivalent!`-shaped
guard; pure-sync `reconcile` is unchanged, so replay survives by construction.

### S-ROH-B-03 — `AnyReconciler::hydrate_*` forwarding yields the same `AnyState` variant as the deleted free fns

**Tier-1 DST** · **equivalence** · traces ADR-0086 D1/D2 · covers task-bucket-B #10

```
GIVEN each AnyReconciler variant in turn
WHEN AnyReconciler::hydrate_desired / hydrate_actual is called
THEN it forwards to the concrete impl's trait method and wraps Self::State into the
     MATCHING AnyState variant (one arm per variant, mirroring AnyReconciler::reconcile)
  AND the wrapped AnyState variant is the same variant the deleted central
     `match reconciler { .. } -> AnyState` free fn produced for that reconciler
```

Universe: the `AnyState` discriminant per reconciler. Per-reconciler equivalence of
the enum forwarding.

### S-ROH-B-04 — Purity firewall fires on a planted violation (dst-lint scan + `ReconcilerIsPure` backstop)

**structural (negative) + Tier-1 DST backstop** · traces ADR-0086 D7 · covers task-bucket-B #11

```
GIVEN the extended xtask::dst_lint clause scanning ALL of overdrive-reconcilers/src/**
      for banned symbols (Instant::now, SystemTime::now, tokio::, rand::, raw HashMap),
      with a narrow allowlist for ONLY the async hydrate_* methods
WHEN a banned symbol is planted in a pure `reconcile` body or a pure helper
     (backoff_for_attempt, plan_reclamation, classify_backend_address, project_*)
THEN the dst-lint scan FAILS the build (covers reconcile AND its transitive pure helpers,
     not just reconcile bodies)
  AND planting the same violation and running the ReconcilerIsPure DST twin-invocation
     invariant is retained as the behavioural BACKSTOP (not sufficient alone — it shares
     one TickContext across both calls, so a wall-clock bypass need not diverge)
  AND a banned symbol inside an allow-listed async hydrate_* body does NOT fail
     (that is the one legitimately-impure surface)
```

Universe: dst-lint pass/fail verdict. The negative test — the scanner must actually
fire, not vacuously pass.

### S-ROH-B-05 — Injectable crash-resume: empty/stale `SimWorkflowLiveSet` triggers convergence

**Tier-1 DST** · **edge / injectability WIN** · traces ADR-0086 D5 (`WorkflowLiveSet` edge) + D8

```
GIVEN a SimWorkflowLiveSet returning an EMPTY live-instance set (models a post-restart engine)
  AND a workflow instance that is running-in-intent with no terminal observation row
WHEN WorkflowLifecycle hydrates and reconciles
THEN the empty set is treated as legitimate (not an error) and the
     running-in-intent + no-live-task + no-terminal condition IS the crash-resume
     trigger (ADR-0064 §5) — it re-emits StartWorkflow
  AND this DST case was impossible under the pre-move concrete AppState (net new coverage)
```

Universe: emitted `Action`. New DST coverage the ADR explicitly enables.

### S-ROH-B-06 — Injectable `ListenerFacts` miss: hydrator SKIPS, never defaults `Proto::Tcp`

**Tier-1 DST** · **edge / error path** · traces ADR-0086 D5 (`ListenerFacts` edge, ADR-0060 C3)

```
GIVEN a SimListenerFacts returning None for a given ServiceId
WHEN the hydrator hydrates the service's State via ListenerFacts::fact_for
THEN the service is SKIPPED (no listener fact) and the hydrator NEVER defaults to Proto::Tcp
  AND a subsequent seeding of the fact makes the same service hydrate normally
```

Universe: hydrated `State` (service present/absent). Pins the ADR-0060 C3 "never
default proto" contract as a now-injectable edge.

### S-ROH-B-07 — Injectable `ServiceVipView` memo-absent: defer the tick, log `allocator_memo_absent`

**Tier-1 DST** · **edge / error path** · traces ADR-0086 D5 (`ServiceVipView` edge, ADR-0049 §4)

```
GIVEN a persisted Service intent whose spec digest has no memoised VIP
  AND a SimServiceVipView returning None for that ContentHash
WHEN the hydrator hydrates via ServiceVipView::assigned_vip
THEN (PRIMARY) the tick is DEFERRED: the hydrator does NOT hydrate the service's State
     and the reconciler emits NO Action for it — None is treated as the ADR-0049 §4
     structural-invariant-violation signal (not a panic, not a default VIP)
  AND (secondary) `allocator_memo_absent` is logged — a supporting signal, NOT the
     sole assertion (do not write a log-string-only test)
  AND the adapter maps the core ContentHash to the allocator's ServiceSpecDigest
```

Universe: PRIMARY = tick outcome (deferred; no State hydrated for the service; no
Action emitted). Secondary = `allocator_memo_absent` log signal. Now-injectable error
path — the deferred/no-Action outcome is the load-bearing assertion, the log is a check.

### S-ROH-B-08 — `HeldSvidView` returns the GLOBAL set; the hydrator filters to the target workload

**Tier-1 DST** · **edge / equivalence** · traces ADR-0086 D5 (`HeldSvidView` edge, ADR-0067 D5b)

```
GIVEN a SimHeldSvidView returning the unfiltered GLOBAL node-held SVID map
      (keyed by AllocationId, several workloads present)
WHEN the svid-lifecycle reconciler hydrates its State
THEN the hydrator filters the global set to the TARGET workload by
     SpiffeId::for_allocation equality (the trait returns the global set by contract;
     filtering is the hydrator's job — ADR-0067 D5b)
  AND presence in the (filtered) set means "held"
```

Universe: hydrated held-SVID `State` for the target. Equivalence + the filter contract.

### S-ROH-B-09 — `HydrationContext` S1 audit: every read surface is represented; nothing reaches an unrepresented `state.*`

**structural (S1 acceptance gate)** · traces ADR-0086 D5 (S1 acceptance invariant) + § Migration S1

```
GIVEN the "read every hydrate_* body" audit over the moved hydration bodies
WHEN each body's reads are enumerated
THEN HydrationContext carries a handle for EVERY surface any hydrate_* body reads
     (IntentStore, ObservationStore, VmHostState, DriverRegistry, the 4 read-ports,
      + plain data node_id/host_ipv4/intent_redb_path)
  AND NO hydrate_* body reaches a state.* field not represented on HydrationContext
  AND post-ADR-0087, NO hydrate_* body reaches any restart-budget surface (the
     cross-read is gone; the audit confirms it)
```

Universe: the set of surfaces each body reads vs the `HydrationContext` field set.
Structural S1 gate — the primary ADR-0086 acceptance evidence.

### S-ROH-B-10 — Compile guard: `reconcile` stays sync; `hydrate_*` carry no `&dyn Clock`

**structural (compile-guard / trybuild-shaped)** · traces ADR-0086 D1 (compile guard additive assertion)

```
GIVEN the reconciler_trait_signature_is_synchronous_no_async_no_clock_param compile guard
WHEN the trait surface is checked
THEN reconcile is still pinned synchronous (no async, no clock param)
  AND the guard gains one ADDITIVE assertion: the new async hydrate_desired /
      hydrate_actual methods carry no &dyn Clock parameter
  AND a planted `async fn reconcile` or a `&dyn Clock` on hydrate_* fails to compile
```

Universe: compile pass/fail. Keeps `reconcile` purity structurally enforced while
`hydrate_*` is the only impure surface.

### S-ROH-B-11 — The central `hydrate_*` free fns are gone; no second hydration path survives (structural)

**structural (green-build absence)** · **regression** · traces ADR-0086 D9 + § Migration S3

```
GIVEN the single-cut hydration move has landed (S3 complete)
WHEN the reconciler-hydration path in reconciler_runtime.rs is inspected
THEN the central hydrate_desired / hydrate_actual free fns are absent, and their
     hydrate_*_* helper fns (the ~9 per-reconciler hydration helpers) are absent
     (all proven by green build)
  AND the ONLY surviving hydration entry is AnyReconciler::hydrate_* forwarding to the
     per-impl trait methods — no second (old central-match) hydration path co-exists
  AND ReconcilerRuntime builds a HydrationContext per tick and calls
     AnyReconciler::hydrate_* as the single post-move hydration dispatch
```

Universe: presence/absence of named surfaces (compile + grep). Structural, not runtime;
the single-path half is a genuine regression guard against a stale duplicate hydrator
surviving the cut (the Bucket-B mirror of A-13's cross-read-gone absence proof).

---

## Tier mapping summary

| Scenario | Tier | Kind |
|---|---|---|
| S-ROH-A-01 | Tier-1 DST | behaviour (happy) |
| S-ROH-A-02 | Tier-1 DST | behaviour (happy) |
| S-ROH-A-03 | Tier-1 DST | error path |
| S-ROH-A-04 | Tier-1 DST | edge |
| S-ROH-A-05 | default-lane unit | error / regression |
| S-ROH-A-06 | Tier-1 DST | end-to-end trajectory |
| S-ROH-A-07 | Tier-1 DST | contested-but-decided |
| S-ROH-A-08 | Tier-1 DST | edge |
| S-ROH-A-09 | Tier-1 DST | edge / idempotency |
| S-ROH-A-10 | Tier-1 DST | edge / idempotency |
| S-ROH-A-11 | default-lane unit | edge (mutation-gate) |
| S-ROH-A-12 | schema-evolution | regression |
| S-ROH-A-13 | structural | regression / absence |
| S-ROH-B-01 | Tier-1 DST | equivalence |
| S-ROH-B-02 | Tier-1 DST | equivalence (replay) |
| S-ROH-B-03 | Tier-1 DST | equivalence |
| S-ROH-B-04 | structural + DST backstop | negative / purity |
| S-ROH-B-05 | Tier-1 DST | edge (injectability) |
| S-ROH-B-06 | Tier-1 DST | edge / error |
| S-ROH-B-07 | Tier-1 DST | edge / error |
| S-ROH-B-08 | Tier-1 DST | edge / equivalence |
| S-ROH-B-09 | structural (S1 gate) | audit |
| S-ROH-B-10 | structural (compile guard) | audit |
| S-ROH-B-11 | structural | regression / absence |

**Counts**: Bucket A = 13, Bucket B = 11, total = 24.
**Error / edge / regression / negative**: A = {03,04,05,08,09,10,11,12,13} = 9;
B = {04,05,06,07,08,11} = 6; total 15 / 24 ≈ **63%** (target ≥ 40%).

## Tier-3 note — none required

**Zero scenarios are Tier-3-only.** Both ADRs are, by design, net-positive for
DST-testability: ADR-0087 D6 makes the liveness restart path *purely
observation-row-driven* (no cross-read to inject), and ADR-0086 D8 turns the
previously-concrete hydration boundary *into* an injectable one. The reconciler
decision logic is fed by `ProbeResultRow`s / `AllocStatusRow`s injected through
`SimObservationStore` — the real `probe_runner` subsystem is an unchanged
observation-producer and is **out of scope** for this feature, so no real
kernel/cgroup/probe integration test is needed to exercise the changed behaviour.
The only non-DST scenarios are **structural** (compile-guard, dst-lint AST scan,
green-build absence, S1 read-surface audit) and **schema-evolution** (rkyv golden
fixture) — all default-lane, none Tier-3.

## Prerequisites

- ADR-0087 lands as a **precursor** (single restart authority) BEFORE ADR-0086's
  crate-move — its slice removes the `restart_status_for_alloc` cross-read so the
  hydration move never ports a `RestartBudgetView`.
- Bucket B needs the 4 `Sim*` read-port impls (`SimListenerFacts`,
  `SimServiceVipView`, `SimWorkflowLiveSet`, `SimHeldSvidView`) in `overdrive-sim`
  (ADR-0086 S4) and the `HydrationContext` / `HydrateError` core types (S1).
- **HARD DELIVER GATE** — Bucket B equivalence (B-01/B-02/B-03) REQUIRES a
  **characterization golden** (a snapshot of the pre-move hydrated `AnyState` for the
  representative row set, plus the pre-move reconcile trajectory under a fixed seed for
  B-02) captured and committed **at/before S2, before the single-cut S3 deletes the
  central `hydrate_*` free fns**. There is no live old-vs-new diff after S3, so without
  this golden B-01/B-02/B-03 have no expected baseline. Not a soft note — DELIVER
  blocks S3 until the golden is committed. See Open Question 2.

## Open questions (surfaced) + one hard DELIVER gate

1. **`LivenessProbeFailed.attempts` value-semantics** (S-ROH-A-07) — **CONFIRMED /
   LOCKED**. ADR-0087 D4 pins `attempts = restart_counts` (= CEILING), superseding
   today's `attempts = consecutive_failures`. The user has confirmed this as the
   locked ADR-0087 D4 value; the scenario pins it. Not open — the alternative
   (streak `F`) would reintroduce the eliminated cross-read. (feature-delta Open
   Question 1.)
2. **Equivalence-baseline capture is a HARD DELIVER GATE** (S-ROH-B-01/02/03).
   The migration is single-cut — ADR-0086 S3 DELETES the central `hydrate_*` free
   fns, so there is **no live old-vs-new A/B diff** after the cut. The Bucket-B
   equivalence scenarios (B-01/B-02/B-03) therefore REQUIRE a **characterization
   golden** — a snapshot of the pre-move hydrated `AnyState` for the representative
   row set (and, for B-02, the pre-move reconcile trajectory under a fixed seed) —
   **captured and committed at/before S2, before S3 removes the free fns**. This is
   a hard sequencing gate, not a soft note: without the golden, B-01/B-02/B-03 have
   **no expected baseline** and cannot be authored as equivalence bars. DELIVER MUST
   land the golden before executing the S3 deletion.

**No hard DESIGN blockers.** Every scenario's design detail is pinned by ADR-0086 or
ADR-0087; nothing required inventing un-specified surface. The single hard gate is the
DELIVER sequencing prerequisite in item 2 above — capture the characterization golden
before the S3 deletion.
