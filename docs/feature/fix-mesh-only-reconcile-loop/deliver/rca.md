# RCA — Mesh-only service causes a perpetual empty reconcile loop in `ServiceMapHydrator`

**Method:** Toyota 5 Whys (multi-causal, evidence at every level).
**Scope:** `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs` —
`reconcile` / `should_dispatch` / the line-426 retry-bump guard.
**Mode:** read-only static investigation; no tests run; no production source modified.
**Investigator:** Rex (RCA specialist).
**Date:** 2026-06-24.

---

## 1. Problem statement (scoped)

When **every** backend of a service falls inside the Path-A/mesh workload
subnet (`10.99.0.0/16`), the D-GATE three-way subnet partition filters all
backends out of **both** load-balancer paths. The service therefore emits
**zero actions** every tick, never records anything in its `RetryMemory`
View, never produces a `service_hydration_results` observation row, and so
`should_dispatch` re-enters its dispatch arm on every subsequent tick **in
perpetuity**. The service's View never reflects "settled," so any caller
inspecting View state to reason about convergence sees a perpetually-pending
service.

This is a **re-litigation of finding D3** from the 02-02 adversarial review
(`docs/feature/canonical-workload-address-inbound-tproxy/deliver/02-02-review.md`),
which traced the identical loop, ratified the guard as "strictly more honest,"
and pinned it with a ≥2-tick test. D3 was assessed **only** on the I/O-cost
axis. This RCA confirms D3's I/O-cost trace is correct **and** surfaces the
axis D3 did not weigh: **convergence-state observability** (the View never
settles) and a **separate latent teardown gap** (Finding 2).

---

## 2. Grounding verification (the submitted trace, confirmed against source)

The submitted loop trace is **correct in every step**. Evidence:

**Step 1 — first tick dispatches.** `should_dispatch` is called with
`actual_status = None` (no row written), `retry = None`
(`service_map_hydrator.rs:322-328`):

```rust
let actual_status = actual.actual.get(service_id);
let need_dispatch = should_dispatch(
    actual_status,
    desired_svc.fingerprint,
    view.retries.get(service_id),
    tick.now_unix,
);
```

The `None | Some(Pending)` arm (`:548-553`) returns `true` because
`retry.and_then(|r| r.last_attempted_fingerprint)` is `None`, so the
`_ => true` sub-arm fires:

```rust
None | Some(ServiceHydrationStatus::Pending) => {
    match retry.and_then(|r| r.last_attempted_fingerprint) {
        Some(last) if last == desired_fingerprint => backoff_window_elapsed(retry, now),
        _ => true,
    }
}
```

**Step 2 — mesh filter empties both slices.** `:374-388`:

```rust
let is_mesh_backend = |b: &&Backend| match b.addr.ip() {
    std::net::IpAddr::V4(v4) => workload_subnet.contains(&v4),
    std::net::IpAddr::V6(_) => false,
};
let (local, remote): (Vec<&Backend>, Vec<&Backend>) =
    desired_svc.backends.iter().filter(|b| !is_mesh_backend(b)).partition(...);
let remote_is_empty = remote.is_empty();
let local_is_empty = local.is_empty();
```

For an all-mesh set, `filter(|b| !is_mesh_backend(b))` removes every backend,
so `local` and `remote` are both empty.

**Step 3 — no action emitted.** `DataplaneUpdateService` is gated on
`if !remote_is_empty` (`:390`); `push_register_local_backend_actions` iterates
an empty `local` slice (`:405-416`, `:486` `for backend in local`). Both
no-op → `actions` stays empty.

**Step 4 — View never updated.** The retry-bump guard (`:426-431`):

```rust
if !(local_is_empty && remote_is_empty) {
    let entry = next_view.retries.entry(*service_id).or_default();
    entry.attempts = entry.attempts.saturating_add(1);
    entry.last_failure_seen_at = tick.now_unix;
    entry.last_attempted_fingerprint = Some(desired_svc.fingerprint);
}
```

With `local_is_empty && remote_is_empty == true`, the condition is `false`;
the body is skipped. `next_view.retries` gets no entry for this service.

**Step 5 — no observation row, so `actual_status` stays `None` forever.** The
`actual` side is hydrated from `service_hydration_results` rows
(`architecture.md § 8`, Hydration shape table). Those rows are written by the
action-shim **after a dataplane call returns**. No action → no dataplane call
→ no row → `actual.actual.get(service_id) == None` on the next tick. Loop
returns to Step 1.

**Backoff is a degenerate constant — confirms the "recording an attempt would
lie" point.** `backoff_for_attempt` ignores its argument
(`workload_lifecycle.rs:59-61`):

```rust
pub const fn backoff_for_attempt(_attempt: u32) -> Duration {
    RESTART_BACKOFF_DURATION   // = Duration::from_secs(1)  (:33)
}
```

So even if `last_attempted_fingerprint` **were** recorded, the
`Some(last) if last == desired_fingerprint => backoff_window_elapsed(...)`
sub-arm would re-fire once per 1 s window — the service would still **never
fully settle**, and the View would carry an affirmative lie ("programmed at
fp") that the line-418-425 doc comment explicitly forbids. **Recording an
attempt is NOT a valid fix.**

---

## 3. Five Whys — multi-causal

### Branch A — the perpetual empty-dispatch loop (the reported symptom)

```
WHY 1A: A mesh-only service re-enters the dispatch block on every tick forever.
  [Evidence: should_dispatch :548-553 returns true whenever
   retry.and_then(|r| r.last_attempted_fingerprint) != Some(desired_fingerprint);
   for an all-mesh service that value is permanently None.]

  WHY 2A: last_attempted_fingerprint is permanently None because the View is
          never written for an all-mesh service.
    [Evidence: the retry-bump guard :426 `if !(local_is_empty && remote_is_empty)`
     is false for an all-mesh set, so :427-430 (the only writer of
     last_attempted_fingerprint on the non-V6 path) never runs.]

    WHY 3A: should_dispatch's "no status row" arm treats the absence of a
            recorded attempt as "needs dispatch," with no notion of "this
            service legitimately programs nothing."
      [Evidence: should_dispatch :539-564 branches ONLY on
       (actual_status, last_attempted_fingerprint, backoff). It has no input
       describing whether the desired state is a no-op. `_ => true` is the
       catch-all for "no prior attempt at this fingerprint."]

      WHY 4A: The convergence model assumes every dispatched service eventually
              produces a `service_hydration_results` observation row, which is
              the ONLY signal that lets actual.fingerprint reach
              desired.fingerprint and stop the loop.
        [Evidence: architecture.md § 8 — "Retries are driven by fingerprint
         mismatch, not by re-emitting on every tick" (:651); the View "reset
         on confirmed convergence (actual.fingerprint == desired.fingerprint)"
         (:686-687). Convergence is defined ENTIRELY through the `actual`
         observation row. A service that emits no action produces no row, so
         actual is never populated and the model's terminating condition is
         structurally unreachable.]

        WHY 5A (ROOT CAUSE A): The D-GATE three-way partition (added in step
              02-02 of canonical-workload-address-inbound-tproxy) introduced a
              NEW service class — "programs nothing, owned by nft-TPROXY" —
              that the phase-2-xdp-service-map convergence model
              (architecture.md § 8) never contemplated. That model has exactly
              two terminating shapes: (i) a dispatch produces a Completed/Failed
              observation row, or (ii) the service leaves `desired` (GC at :441).
              A mesh-only service hits NEITHER: it dispatches nothing (so no
              row) yet stays in `desired` (so no GC). The gate closed the
              action paths but left should_dispatch's terminating condition
              unchanged, so the loop has no exit.
        [Evidence: D-GATE / D-GATE-PRED (wave-decisions.md :23-24, :169-184)
         specify only that mesh backends "program NEITHER the cgroup LOCAL path
         NOR the XDP remote path." Neither the design nor the ADR-0053/0071
         amendments addresses what the hydrator's View/dispatch state machine
         records for a service that programs nothing. The retry guard
         (:418-431) and its self-justifying comment were authored in 02-02 as a
         silent decision over exactly this gap — see 02-02-review.md D3.]

        -> ROOT CAUSE A: A subnet-derived no-op service class was bolted onto a
           convergence state machine whose only terminating conditions are
           "observation row arrives" or "service removed" — neither of which a
           no-op service reaches. The gate suppressed the actions but not the
           dispatch decision that depends on those actions' downstream
           observation.
```

### Branch B — why the loop was shipped (the process/design branch)

```
WHY 1B: The loop landed in main despite an adversarial review that traced it.
  [Evidence: 02-02-review.md D3 (:87-112) traces the exact loop:
   "should_dispatch(None, fp, None, now) hits _ => true → need_dispatch == true
   every 100ms tick forever."]

  WHY 2B: D3 was assessed solely on the I/O-cost axis and judged benign.
    [Evidence: 02-02-review.md :97-103 — "the non-convergence is benign — a
     fully-gated mesh service has no convergence target ... With the guard:
     zero actions, zero ViewStore I/O, no phantom fingerprint — only a cheap
     pure recompute." 02-02.md :120-142 ratifies the guard: "strictly more
     honest, not worse."]

    WHY 3B: The I/O-cost trace IS correct — the only axis weighed — but the
            convergence-observability axis (View never settles) was not weighed
            because no consumer of View settledness existed at the time.
      [Evidence: persist_view's Eq-diff skip (reconciler_runtime.rs :674-676)
       elides BOTH fsync and the in-memory insert when current == view; with
       the guard, next_view == view every tick, so the loop is genuinely
       zero-fsync — D3's I/O claim holds. Grep for production readers of
       `.retries` / ServiceMapHydratorView outside the reconciler returns ONLY
       self-writes (service_map_hydrator.rs) and DST invariant machinery
       (overdrive-sim/src/invariants/); no production component reads
       `view.retries` to reason about convergence today. The
       reconciler_runtime.rs `.retries` matches at :2888/:2924 are an unrelated
       `attempt_gate.retries` in the workflow runtime.]

      WHY 4B: The ratification converted the guard into a pinned contract with
              a test that asserts the loop's behavior (retries stay empty),
              cementing "View never settles" as intended rather than flagging
              it as a model gap.
        [Evidence: 02-02.md :138-142 — test
         `all_mesh_service_emits_nothing_and_keeps_retries_empty_across_ticks`
         asserts `actions.is_empty() AND next_view.retries.is_empty()` on both
         of two ticks. This pins the no-action / empty-View behavior, but does
         NOT assert "the service is observably settled" — that property has no
         representation, so the test cannot guard it.]

        WHY 5B (ROOT CAUSE B): The review's cost model was I/O-only; it lacked
              a convergence-observability requirement to test the guard
              against. With no caller depending on View settledness and no DST
              liveness invariant exercising a mesh-only service, the gap was
              invisible to every gate — there was nothing to make the
              "perpetually-pending" property fail.
        [Evidence: the DST eventual-convergence invariant
         (overdrive-sim/src/invariants/service_map_hydrator.rs :81-167) checks
         `actions.is_empty() && all_converged(&state)` (:153) where
         all_converged tests actual.fingerprint == desired.fingerprint (:284).
         For a mesh-only service actual is never populated, so all_converged
         is false and the invariant would FAIL within CONVERGENCE_TICK_BUDGET
         (:50, 8 ticks) — BUT the invariant's fixtures use only non-mesh
         backends, so the mesh-only path is never exercised. The one gate that
         would have caught the perpetual-pending property is not pointed at the
         service class that exhibits it.]

        -> ROOT CAUSE B: The defect is invisible to every existing gate because
           (i) no production consumer reads View settledness, and (ii) the DST
           liveness invariant that encodes "must converge" is not exercised
           against a mesh-only service — the exact class that cannot converge
           under the current model.
```

### Cross-validation

- **Root Cause A + Root Cause B are consistent, not contradictory.** A is the
  *mechanism* (a no-op service class with no terminating condition in the
  dispatch state machine); B is the *why it shipped* (the only axis the review
  weighed was I/O cost, and no gate exercises the failing class). A produces
  the loop; B explains why the loop survived review and CI.
- **All symptoms explained.** The reported symptom ("perpetual empty reconcile
  loop; View never settles; a convergence-reasoning caller sees
  perpetually-pending") is fully explained by A. The "shipped anyway despite a
  review that saw it" puzzle is fully explained by B. No residual symptom.
- **Completeness check.** A third contributing factor was surfaced during the
  investigation and is reported separately below (Finding 2 — the
  non-mesh→all-mesh teardown gap), which is a *distinct* defect that the
  line-390 gate also causes but which is NOT part of the reported loop.

---

## 4. Boundary of the defect (impact assessment)

### Is the loop purely wasted CPU, or does it have a correctness consequence?

**Today: it is bounded wasted CPU — there is NO live correctness consequence,
because no production reader of View settledness exists.**

- **Zero I/O per tick.** `persist_view`'s Eq-diff skip
  (`reconciler_runtime.rs:674-676`) returns early when `current == view`. With
  the guard, `next_view == view` every tick, so the loop performs **no fsync
  and no in-memory map mutation** — only the pure `reconcile` recompute (a
  partition over the backend Vec) and an empty action-shim dispatch. D3's
  I/O-cost trace is correct.
- **No production reader of `view.retries`.** Grep across `**/src/**/*.rs` for
  `.retries` and `ServiceMapHydratorView` finds the field read ONLY by the
  reconciler itself (`service_map_hydrator.rs`) and by DST invariant evaluators
  (`overdrive-sim/src/invariants/`, test machinery). **No control-plane
  component, CLI command, or operator surface inspects `view.retries` to reason
  about convergence.** The "any caller inspecting View state sees a
  perpetually-pending service" consequence is therefore **latent /
  hypothetical today** — it becomes real the moment such a reader is added
  (e.g. an `overdrive service status` command, a readiness gate, a
  reconciler-health probe).

**Verdict on severity:** **P3 today** (systemic improvement; bounded CPU,
latent observability hazard), with a clear escalation path to **P1** if a
convergence-reasoning reader of the hydrator View is introduced. The cost is
small at Phase-1 N (single-node, few services), but it scales linearly with
the number of mesh-only services × tick rate (100 ms) and is **unbounded in
time** — it never stops on its own.

### Finding 2 (SEPARATE — non-mesh → all-mesh teardown gap; NOT the reported loop)

**This is a distinct latent defect surfaced during the completeness check. Do
NOT fold it into the Branch-A fix.**

When a service **transitions** from a state with non-mesh backends (which
programmed a dataplane VIP) to all-mesh (every backend now in `10.99.0.0/16`),
the old dataplane program is **never torn down**.

- **Evidence — the teardown mechanism exists but is gated off.** The
  `Dataplane::update_service` contract (`traits/dataplane.rs:197-204`)
  specifies: *"`backends.is_empty()` ⇒ per-proto purge (ADR-0060 D4). The
  adapter removes the prior `frontend.proto` `REVERSE_NAT` keys for this
  VIP..."* So an empty-backends `DataplaneUpdateService` is exactly the purge
  signal.
- **Evidence — the gate prevents the purge from ever being emitted.** `:390`
  emits `DataplaneUpdateService` only `if !remote_is_empty`. On a non-mesh→mesh
  transition `remote` becomes empty, so **no** `DataplaneUpdateService` is
  emitted — neither a populated one nor the empty-backends purge. The
  previously-programmed `REVERSE_NAT` / `SERVICE_MAP` entries for that VIP are
  **stranded** in the dataplane.
- **Why this is separate from Branch A.** Branch A is about a service that was
  *always* mesh-only (programs nothing, loops). Finding 2 is about a service
  that *was* programmed and then *became* mesh-only (programmed something,
  never un-programs it). The line-390 gate causes both, but they are different
  defects with different blast radii: Finding 2 is a **stale-dataplane-state /
  mis-rewrite hazard** (a connect to the VIP could still hit a stale
  reverse-NAT entry), not merely wasted CPU.
- **Caveat on live exposure.** Whether a real service can transition non-mesh →
  all-mesh in Phase-1 depends on whether a service's backend set can migrate
  from host-netns/non-Path-A allocs to all-Path-A allocs without changing
  `ServiceId`. This RCA does not establish that such a transition is reachable
  on a production path today; it establishes that **if** it occurs, the
  teardown does not happen. Recommend a follow-up scope decision (surface to
  the user; do NOT create an issue unilaterally per CLAUDE.md).

---

## 5. Proposed fixes — OPTIONS (no unilateral pick)

The design-sensitivity axes that bind hard here:

- **`RetryMemory` / `ServiceMapHydratorView` shape is design-pinned** to
  exactly three fields (`attempts`, `last_failure_seen_at`,
  `last_attempted_fingerprint`) by `architecture.md § 8` (:666-672) and the
  in-code doc comment (`service_map_hydrator.rs:150-165`). Adding a field is
  **new persisted surface** → a design change requiring architect ratification
  (repo CLAUDE.md § "Implement to the design — never invent API surface").
- **Persist inputs, not derived state** (`.claude/rules/development.md`).
  "Mesh-only" is deterministically derivable from
  `(backends, workload_subnet, host_ipv4)` — all already available in
  `reconcile`. Anything recorded must be an INPUT, never a cached "is no-op"
  classification.

### Option A — make `should_dispatch` no-op-aware (no View surface change) — RECOMMENDED for evaluation

**Mechanism.** In `reconcile`, before calling `should_dispatch`, compute the
no-op disposition from inputs already in hand:

```rust
// Pseudocode — derived from inputs, not persisted:
let desired_is_noop = desired_svc.backends.iter().all(|b| is_mesh_backend(&b));
```

Thread `desired_is_noop: bool` into `should_dispatch` (a pure-function
parameter, recomputed every tick — NOT persisted) and short-circuit
`need_dispatch` to `false` when the service programs nothing:

```rust
fn should_dispatch(actual_status, desired_fingerprint, retry, now, desired_is_noop) -> bool {
    if desired_is_noop { return false; }   // nft-TPROXY owns delivery; nothing to converge
    // ... existing arms unchanged ...
}
```

- **Touches design-pinned View shape?** **NO.** No field added. `should_dispatch`
  gains a parameter, but it is a private free function — its signature is
  **not** part of the persisted/operator/ADR surface. Per the architecture
  doc, `reconcile` is described as a skeleton with "key invariants," not a
  pinned signature; the View is the pinned artifact, and it is untouched.
- **`should_dispatch` becomes mesh-aware?** YES — via the derived
  `desired_is_noop` boolean computed in `reconcile`. This keeps the gate logic
  (the `is_mesh_backend` predicate) as the single source and lifts only its
  *boolean conclusion* into the dispatch decision.
- **Persist-inputs compliance?** PASS — `desired_is_noop` is recomputed from
  `(backends, workload_subnet)` every tick; nothing derived is persisted.
- **Blast radius.** Two private surfaces in one core file: `reconcile` (compute
  the bool) and `should_dispatch` (one param + one early return). No View, no
  ADR, no observation-store, no action surface. DST-pure (no clock, no I/O).
- **Risk.** **Low–medium**, with ONE thing that MUST be verified before
  shipping: the `should_dispatch` early-return MUST NOT break a service that
  *was* programmed and then became all-mesh (Finding 2's teardown transition).
  If `desired_is_noop` short-circuits to `false` (no dispatch), the purge that
  Finding 2 needs is *also* suppressed — i.e. Option A as written **preserves**
  the Finding-2 teardown gap (does not worsen it, but does not fix it).
  Finding 2 must be handled as its own scope decision, NOT by widening Option A
  to emit a purge (that would be inventing convergence behavior the design
  didn't specify). **Verification required:** confirm that the non-mesh→mesh
  teardown is genuinely out of Option A's scope and tracked separately, so
  Option A is not silently shipping past Finding 2.
- **Design-fidelity verdict.** Whether Option A is "implementing the design" or
  "inventing past a gap": the design (architecture.md § 8 + D-GATE) **did NOT
  anticipate the mesh-only no-op case** — § 8 predates the gate and assumes
  every service converges via an observation row; D-GATE specified the action
  suppression but was silent on the dispatch state machine. So Option A is
  **filling a gap the design left underspecified**, not implementing a named
  behavior. Per repo CLAUDE.md, "when the design specifies a model but not the
  exact signature ... STOP and surface the gap." **Option A's `should_dispatch`
  parameter shape MUST be surfaced to the orchestrator/architect for
  ratification before a crafter builds it** — do not let a crafter improvise
  the `desired_is_noop` threading.

### Option B — record a settled marker in the View (NEW SURFACE — design change)

**Mechanism.** Add a field to `RetryMemory` (or `ServiceMapHydratorView`) — e.g.
`last_noop_fingerprint: Option<BackendSetFingerprint>` — recording the
fingerprint for which the service was determined to be a no-op, and gate
`should_dispatch`'s `None`-arm on it.

- **Touches design-pinned View shape?** **YES — this is new persisted surface.**
  `architecture.md § 8` pins `RetryMemory` to three fields; this adds a fourth.
  Per repo CLAUDE.md § "Implement to the design," this is a **design change, not
  a bug-fix** — it requires an architect dispatch and an `architecture.md § 8`
  amendment (and, because the View is rkyv/CBOR-persisted, a schema-evolution
  consideration per `.claude/rules/development.md` § "Reconciler I/O → Schema
  evolution" — additive `#[serde(default)]` is the likely shape, but it is
  still a schema touch).
- **Persist-inputs compliance?** **BORDERLINE / likely FAIL.** A
  `last_noop_fingerprint` is a *cached classification* — it records the
  *output* of applying the mesh predicate to a fingerprint, not an input. The
  fingerprint itself is an input, but "this fingerprint is a no-op" is derived
  from `(backends, workload_subnet)` and would go stale if `workload_subnet`
  ever changed (the #239 tunable-base hazard the design already flagged). This
  is exactly the "persist the inputs, recompute the derived value" rule.
  Recording the no-op-ness violates it; recording only the fingerprint adds a
  field that duplicates `last_attempted_fingerprint` for a service that was
  never attempted (semantically confusing).
- **Blast radius.** Larger: View struct + schema-evolution + architect
  amendment + every test that constructs `RetryMemory` by literal.
- **Risk.** **Medium–high.** New persisted surface, a persist-inputs tension,
  and a schema touch — all to encode a property Option A derives for free.
- **Verdict.** **Not recommended** unless a production reader genuinely needs
  the settled-ness *persisted across restarts* (no such reader exists today).
  If that reader appears, this becomes the right answer **with** an architect
  amendment — but it must be flagged as a design change, never shipped as a
  silent bug-fix.

### Option C — GC a mesh-only service's evaluation differently / skip enqueue

**Mechanism.** Prevent the evaluation broker from re-enqueuing a mesh-only
service, or have `reconcile` signal "no further work" so the runtime stops
ticking this target until its inputs change.

- **Touches design-pinned View shape?** NO (View untouched).
- **Feasibility.** **Uncertain / likely out of reach for a core-only fix.** The
  evaluation broker and re-enqueue logic live in `overdrive-control-plane`
  (`reconciler_runtime.rs` / `eval_broker.rs`), and the reconciler trait is a
  pure function with no "I am quiescent" return channel (`reconcile` returns
  `(Vec<Action>, View)` only). Adding a quiescence signal is a **runtime/trait
  change** with cross-reconciler blast radius — far larger than Option A, and
  it touches ADR-0035/0036 trait surface.
- **Risk.** **High.** Trait-surface change affecting every reconciler;
  level-triggered re-evaluation is a load-bearing property (a backend-set change
  must re-trigger). Suppressing re-enqueue risks missing the mesh→non-mesh
  transition. **Not recommended** for this defect's scope.

### Recommendation (for the user/architect to decide — NOT a unilateral pick)

**Option A** is the smallest correct change, touches no design-pinned surface,
satisfies persist-inputs, and keeps `reconcile` pure/DST-replayable. Its one
required guard is verifying it does not silently ship past **Finding 2**
(teardown), which must be tracked as its own scope decision. Because Option A
fills a design gap (the mesh-only no-op case was never modeled), its
`should_dispatch` parameter shape should be **surfaced to the
orchestrator/architect for ratification** before a crafter builds it, per repo
CLAUDE.md.

---

## 6. Files affected (per option)

| Option | Files | Surface touched |
|---|---|---|
| **A** | `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs` (only) | private `reconcile` (compute `desired_is_noop`) + private `should_dispatch` (one param + early return). No View, no ADR, no schema. |
| **B** | `service_map_hydrator.rs` (View struct + reconcile + should_dispatch) + `docs/feature/phase-2-xdp-service-map/design/architecture.md` § 8 amendment (architect) + every `RetryMemory` literal in tests + schema-evolution consideration | **design-pinned View shape** + schema + ADR doc. |
| **C** | `crates/overdrive-control-plane/src/reconciler_runtime.rs`, `eval_broker.rs`, `crates/overdrive-core/src/reconcilers/mod.rs` (trait) + ADR-0035/0036 | **reconciler trait surface** (cross-reconciler). |
| **Finding 2** (separate) | `service_map_hydrator.rs:390` (gate) — requires emitting an empty-backends `DataplaneUpdateService` purge on non-mesh→mesh transition; **design decision needed** (the design did not specify teardown for the gate). | dataplane action emission + a transition-detection input. |

---

## 7. Risk assessment summary

| Option | Severity addressed | Risk | Design change? | Persist-inputs |
|---|---|---|---|---|
| A — no-op-aware `should_dispatch` | Branch A (the loop) | Low–medium (must verify Finding-2 boundary) | No (gap-fill; surface signature for ratification) | PASS |
| B — settled marker in View | Branch A | Medium–high | **YES — new persisted surface** | Borderline/FAIL |
| C — broker quiescence | Branch A | High (trait surface) | Yes (trait/ADR) | N/A |
| Finding 2 — teardown purge | Stale-dataplane hazard | Medium (correctness, not CPU) | Yes (gate teardown unspecified) | N/A — handle as SEPARATE scope |

---

## 8. Prevention strategy (Kaizen)

1. **Early detection — extend the DST liveness invariant to a mesh-only
   service.** The `HydratorEventuallyConverges` invariant
   (`overdrive-sim/src/invariants/service_map_hydrator.rs:81-167`) would FAIL
   on a mesh-only service today (`all_converged` is `false` because `actual` is
   never populated), but its fixtures use only non-mesh backends, so the class
   is never exercised. Adding a mesh-only scenario to that invariant would have
   caught this at PR time and will catch the next regression of this class.
   This is the structural gate the defect slipped past (Root Cause B).
2. **Define what "settled" means for a no-op service before adding a reader.**
   The latent-to-real escalation (P3→P1) happens the moment a
   convergence-reasoning reader of the hydrator View is introduced. Whoever adds
   that reader must first decide whether a mesh-only service is "settled" and
   how that is represented — choosing Option A's derived-on-read model over
   Option B's persisted-marker model unless cross-restart persistence is
   genuinely required.
3. **Review-cost models must weigh observability, not just I/O.** Root Cause B
   is a review that weighed only the I/O-cost axis of a non-converging loop.
   When a gate suppresses a state machine's outputs, the review must explicitly
   ask "does the state machine still have a terminating condition?" — not only
   "does it cost I/O?".
4. **When a gate is bolted onto an existing state machine, re-examine the state
   machine's terminating conditions in the same change.** D-GATE closed the
   action paths but left `should_dispatch`'s exit condition (which depends on
   those actions' downstream observation) unchanged. A gate that removes the
   only input that lets a loop terminate is incomplete without adjusting the
   loop's exit.

---

## 9. Evidence index (file:line)

- Loop entry / dispatch decision: `service_map_hydrator.rs:322-328`, `:548-553`.
- Mesh filter + empty slices: `service_map_hydrator.rs:374-388`.
- Action gates: `service_map_hydrator.rs:390` (remote), `:405-416`+`:486` (local).
- Retry-bump guard (the no-View-write): `service_map_hydrator.rs:418-431`.
- Degenerate backoff: `workload_lifecycle.rs:33`, `:59-61`.
- View shape pinned (design): `phase-2-xdp-service-map/design/architecture.md` § 8 `:653-676`; in-code `service_map_hydrator.rs:150-165`.
- Convergence model (observation-row-driven): `architecture.md` § 8 `:645-651`, `:686-687`.
- Eq-diff skip (zero-I/O proof): `reconciler_runtime.rs:674-676`.
- No production reader of View: grep `.retries`/`ServiceMapHydratorView` over `**/src/**/*.rs` → reconciler self-writes + DST invariants only.
- Teardown mechanism (Finding 2): `traits/dataplane.rs:197-204` (`is_empty()` ⇒ purge), suppressed by `service_map_hydrator.rs:390`.
- DST liveness invariant (would-fail-if-exercised): `overdrive-sim/src/invariants/service_map_hydrator.rs:81-167`, `:153`, `:284`, `CONVERGENCE_TICK_BUDGET :50`.
- D-GATE design: `canonical-...-tproxy/design/wave-decisions.md:23-24`, `:50-58`, `:169-184`.
- Prior review of this exact loop (Root Cause B): `canonical-...-tproxy/deliver/02-02-review.md:87-112`; ratification `02-02.md:120-142`.

---

## 10. ADDENDUM (2026-06-24) — executed-evidence confirmation: the all-mesh loop is one of THREE faces of a fingerprint-domain mismatch

The orchestrator extended the investigation past the all-mesh scope after
noticing the action-shim computes the hydration-result fingerprint over a
DIFFERENT backend set than the hydrator's convergence comparison uses. A
throwaway probe drove the REAL `ServiceMapHydrator::reconcile` + the REAL
action-shim (`dataplane_update_service::dispatch` /
`register_local_backend::dispatch`) through a 6-tick loop, feeding `actual`
from the rows the shim ACTUALLY wrote (not the faithless `desired.fingerprint`
echo the DST harness uses). Executed under Lima; output captured verbatim.

### 10.1 Root Cause A is broader than "all-mesh" — it is a fingerprint-DOMAIN mismatch

- The hydrator's **desired** fingerprint is `fingerprint(vip, row.backends)` over
  the FULL backend set (`service_map_hydrator.rs:126`).
- The action-shim's **observed** `Completed{fingerprint}` is
  `fingerprint(vip, backends_in_action)` over the PROGRAMMED SUBSET only
  (`dataplane_update_service.rs:111,138`); the `DataplaneUpdateService` action
  carries only the post-gate remote survivors (`service_map_hydrator.rs:396`).
- The `Completed` arm compares `*fingerprint != desired_fingerprint` with NO
  backoff (`service_map_hydrator.rs:554-556`).
- The `RegisterLocalBackend` shim writes NO observation row at all
  (`action_shim/register_local_backend.rs:17-20`, confirmed) — the cgroup path
  has no hydration-result surface.

Therefore `actual.fingerprint == desired.fingerprint` (the architecture.md § 8
convergence condition) is **structurally unreachable whenever gating drops any
backend OR the only emitted action is `RegisterLocalBackend`.** Only a
remote-only service (no mesh, no local) converges.

### 10.2 Executed evidence (Lima, 6 ticks/shape, verbatim)

```
=== shape: remote-only  backends=[10.96.0.50]  desired_fp=14908175023571825766 ===
  tick 0: actions=1 (dus=1 rlb=0)  actual_fp=Some(14908175023571825766)  converged=false
  tick 1: actions=0 (dus=0 rlb=0)  actual_fp=Some(14908175023571825766)  converged=true
  RESULT[remote-only]: dus_ticks=1/6 rlb_ticks=0/6 converged_tick=Some(1)

=== shape: all-mesh  backends=[10.99.0.6, 10.99.0.10]  desired_fp=1754204326311495833 ===
  tick 0..5: actions=0 (dus=0 rlb=0)  actual_fp=None  converged=false
  RESULT[all-mesh]: dus_ticks=0/6 rlb_ticks=0/6 converged_tick=None

=== shape: mixed-mesh+remote  backends=[10.99.0.6, 10.96.0.50]  desired_fp=14223179022246209482 ===
  tick 0..5: actions=1 (dus=1 rlb=0)  actual_fp=Some(14908175023571825766)  converged=false
  RESULT[mixed-mesh+remote]: dus_ticks=6/6 rlb_ticks=0/6 converged_tick=None

=== shape: local-only  backends=[10.0.0.1]  desired_fp=7308524915020491227 ===
  tick 0..5: actions=1 (dus=0 rlb=1)  actual_fp=None  converged=false
  RESULT[local-only]: dus_ticks=0/6 rlb_ticks=6/6 converged_tick=None
```

**Smoking gun:** the mixed service's `actual_fp` (`14908175023571825766`) is
byte-identical to the remote-only service's `desired_fp` — both are
`fingerprint(vip, [10.96.0.50])`. This proves the hydration row carries the
SUBSET fingerprint, while the mixed service's `desired_fp`
(`14223179022246209482`, over `[10.99.0.6, 10.96.0.50]`) is over the full set.
They never match → re-dispatch every tick.

### 10.3 The three non-converging faces (all confirmed)

| Shape | Emitted | Hydration row | Result |
|---|---|---|---|
| remote-only | `DataplaneUpdateService{full}` | `Completed{full_fp}` == desired | ✅ converges tick 1 |
| **all-mesh** (reported) | nothing | none → `actual=None` | ❌ `None`-arm, never settles |
| **mixed** (mesh+remote) | `DataplaneUpdateService{subset}` | `Completed{subset_fp}` ≠ desired | ❌ `Completed`-arm TIGHT spin, **6/6 ticks** (re-dispatch + new row every tick — worse than all-mesh's zero-I/O loop) |
| **local-only** | `RegisterLocalBackend` only | none (cgroup path writes no row) | ❌ `None`-arm, never settles |

### 10.4 Why every gate missed it — the DST harness is a faithless simulation of the action-shim

`overdrive-sim/src/invariants/service_map_hydrator.rs:112-135` simulates the
action-shim by writing back `Completed{ fingerprint: desired.fingerprint }`
(line 129) — NOT `fingerprint(vip, action.backends)` as the real shim does
(`dataplane_update_service.rs:111`). Because the harness always echoes the
DESIRED fingerprint, `all_converged` becomes true after one dispatch for ANY
shape, so `HydratorEventuallyConverges` is **structurally incapable** of
detecting the subset-mismatch spin. This is the simulation-fidelity cousin of
`.claude/rules/development.md` § "Production code is not shaped by simulation":
here the SIM does not faithfully model the production effect, yielding false
convergence confidence. The corrected invariant MUST compute the write-back
fingerprint the same way the real shim does (over the action's backends), or
drive the real shim — and exercise all four shapes.

### 10.5 Consequence for the fix (supersedes § 5's Option A recommendation)

**Option A (no-op-aware `should_dispatch`) is now disqualified as "the fix."**
It patches only the all-mesh `None`-arm face; it leaves the mixed `Completed`-arm
tight spin (the worst face — re-dispatch + obs-write every tick) and the
local-only non-settle untouched, and cements Finding 2. The correct fix is a
**fingerprint-domain realignment**: the fingerprint the hydrator dispatches on
and converges against must be over the SAME backend set the dataplane is
actually driven to program and that the observation row reports back — with the
empty programmed set (all-mesh) round-tripping as a settled/no-op state (and, if
the empty-backends `DataplaneUpdateService` purge is emitted, also closing
Finding 2). Because architecture.md § 8 *defines* convergence as
`actual.fingerprint == desired.fingerprint` over the service's backend set, and
because the no-row `RegisterLocalBackend`/cgroup path's participation in
convergence is unspecified, this is a **design-level change routed through
@nw-solution-architect** (amend § 8), NOT a localized bug-fix a crafter may
improvise (repo CLAUDE.md § "Implement to the design — never invent API
surface").
