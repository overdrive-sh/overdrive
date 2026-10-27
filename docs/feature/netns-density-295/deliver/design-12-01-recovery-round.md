# STEP 12-01 DESIGN proposal — recovery attempt-round state

## Status and decision boundary

**Status: RECROUND1 APPROVED by the user on 2026-10-27 after independent
DESIGN review APPROVED FOR USER DECISION.** Exact user quote: **“proceed”**.
Context: this was the direct reply to the orchestrator's concrete RECROUND1
approval request, after the proposal and independent verdict were presented.
Reviewer:
`/root/design_12_01_recovery_round/review_recovery_round`. Persisted review:
[`review-design-12-01-recovery-round.md`](review-design-12-01-recovery-round.md).
STEP 12-01 remains paused at the attachment cut and `deliver/roadmap.json`
remains `validation.status = pending`.

RECROUND1 adds one model-only status map to `quiescence.qnt`. It represents
the runtime supervisor's already-existing attempt-round membership and result
barrier so the formal generator does not request a result from a mid-round
joiner and does not publish a multi-member count/restore effect before the
round completes.

It adds no production state, port parameter, public or internal method, error
variant, observation accessor, fixture seam, action, recovery phase, test,
lane, or product outcome. QPROJ-6 remains unchanged.

The exact decision to surface after independent review is:

> **RECROUND1:** add the model-only `roundState: str -> str` state and exact
> guards/barrier below. Keep QPROJ-6, production repair APIs, typed errors,
> fixtures, actions, and observable outcomes unchanged.

## Reproduced defect

The full default union ran 2,124 tests with 2,123 passing and only
`quint_quiescence::quiescence_quiesce_conf_randomized` failing
(`.context/netns-density-295/12-01/default-union2124-after-quiescence92-nextfit88-20261026.log:18483-18484`).
The retained replay uses `QUINT_SEED=0x2355dc58` and reaches:

```text
InitWith(0) -> Damage(C1) -> Damage(C2)
-> RecQuiesce(C1) -> RecQuiesce(C2) -> RepFail(C2)
```

Immediately before `RepFail(C2)`, QPROJ-6 projects C1 and C2 identically as
`Held` at attempt zero. Production has a parked C1 repair and no C2 repair.
The existing driver correctly returns `NotEnabled` because
`ControlListener`'s repair is not parked
(`.context/netns-density-295/12-01/quiescence-seed-2355dc58-after-selector93-20261026.log:15320-15416`).

The production ordering that causes the distinction is established through
the real composed supervisor:

- a round starts only when both `attempts` and `round_pending` are empty;
- every recovery present at that instant is inserted into `round_pending` and
  gets one spawned attempt task (`crates/overdrive-control-plane/src/lib.rs:1702-1730`);
- a later audit may add a distinct recovery and hold while that set remains
  nonempty, but does not add the joiner to the active round (`:1743-1787`);
- a task result is removed from `round_pending` and recorded in
  `round_results` (`:1806-1811`); and
- only when the set becomes empty are successful holds restored and
  `complete_attempt` called (`:1846-1923`). The EXEC gate leaves a
  `joined_since_attempt` component uncharged (`crates/overdrive-core/src/guest_network.rs:314-352`).

The reverse schedule is reachable through the same entry points:

```text
Damage(C2) -> RecQuiesce(C2) -> Damage(C1) -> RecQuiesce(C1)
```

C2 is then the participant and C1 the joiner. Component order,
`conf_step`, `RecoveryPhase`, and completed attempts cannot derive the fact.
Filtering all overlap results, choosing by component order, or completing the
participant inside the joiner's requested action each loses accepted coverage
or changes projected state.

## RECROUND1 — exact formal representation

### One model-only state map

Add exactly:

```quint
var roundState: str -> str
val roundOutside: str -> str = COMPONENTS.mapBy(c => "outside")
def inRound(c: str): bool = roundState.get(c) != "outside"
def resultPending(c: str): bool = roundState.get(c) == "pending"
```

The closed value set and meaning are:

| Value | Meaning |
|---|---|
| `outside` | No active-round result belongs to this component. It is idle, or it joined after the active round began. |
| `pending` | The component belongs to the active round and its result has not returned. With `rec == held`, its repair result is pending; with `rec == repaired`, its post-repair audit result is pending. |
| `keep` | The round collected an unhealthy result for the component and is waiting for another member before publishing the barrier. |
| `restore` | The round collected a healthy result and is waiting for another member before restoring its hold at the barrier. |

`pending` corresponds to the component remaining in private
`round_pending`; `keep` / `restore` correspond to its false / true entry in
private `round_results` after it has left `round_pending`. The map is formal
scheduler state, not a QPROJ-6 phase or observation.

### Round start and barrier

The model collapses the retry timer's state-only delay, as it already does for
the attempt cadence. Define these exact semantics; private Quint helper shape
is implementation-owned:

1. `beginRound(nextRec)` maps every `nextRec(c) == "held"` component to
   `pending` and every other component to `outside`.
2. A round is complete only when it has at least one `keep` or `restore`
   member and no `pending` member.
3. At completion, apply one barrier:
   - each `keep` member becomes `rec = "held"` and increments `att` once;
   - each `restore` member consumes one hold, becomes `rec = "idle"`, and
     resets `att` to zero;
   - each `outside` joiner keeps its phase, hold, and attempt count;
   - the existing `boolLatch` hazard retains its current first-restore defect
     so the same Safety counterexample remains load-bearing; and
   - if held recoveries remain, `beginRound(postBarrierRec)` represents the
     next retry round; otherwise every value becomes `outside`.

For a normal instance, the barrier decrements `cnt` by exactly the number of
`restore` members. A finding started during a result action has already taken
its hold, is `outside`, and is neither restored nor charged by that barrier.

### Action transitions

No action is added, removed, renamed, or given a new production call.
Single-member projected post-states remain the same. Multi-member changes
linearize at the existing production barrier.

| Action | Exact `roundState` effect |
|---|---|
| `InitDown`, `InitWith` | `roundState' = roundOutside`. |
| `Damage`, `OpenFlow`, `LeakHold`, `ForeignRestore`, `RestartWith` | Preserve `roundState` (`RestartWith` can only preserve the stopped all-`outside` value). |
| `Crash`, `FailStop` | Clear to `roundOutside` with the other process-scoped state. |
| `RecQuiesce(c)` | Keep the existing guard/hold effects. If a round exists, preserve it and leave new `c` `outside`; otherwise start `beginRound(rec.set(c, "held"))`. |
| `RecRepair(c)` | Require `resultPending(c)` and `rec(c) == "held"`; keep the existing repair-success effects and leave `c` `pending` through its post-repair audit. |
| `RepFail(c)` | Require `resultPending(c)` and `rec(c) == "held"`; decrement existing `repLeft`, mark `c` `keep`, and do not change projected `att`, `rec`, `cnt`, or `latch` until the barrier. If it was the last pending member, apply the barrier in this action. |
| `RecRedo(c)` | Require `resultPending(c)`, `rec(c) == "repaired"`, and the existing blocking finding. Start a different idle finding under one new hold exactly as today and leave it `outside`; mark `c` `keep`. Apply the barrier only if this was the last pending member. |
| `RecRestore(c)` | Require `resultPending(c)`, `rec(c) == "repaired"`, and the existing non-blocking finding. Start a later idle finding under one new hold exactly as today and leave it `outside`; mark `c` `restore`. Apply the barrier only if this was the last pending member. |

The four result guards become:

```quint
RecRepair(c):  up and resultPending(c) and rec.get(c) == "held"
               and att.get(c) < MAXATT
RepFail(c):    up and resultPending(c) and rec.get(c) == "held"
               and att.get(c) < MAXATT and repLeft != 0
RecRedo(c):    up and resultPending(c) and rec.get(c) == "repaired"
               and findingBlocks(c, auditFinding)
RecRestore(c): up and resultPending(c) and rec.get(c) == "repaired"
               and not(findingBlocks(c, auditFinding))
```

This disables the reproduced `RepFail(C2)` because C2 is `outside`. The C1
result remains available. When C1's round completes, C1 and C2 are both
eligible in the next round if both remain held, preserving independent failure
coverage for either component without component-order inference.

### Per-component liveness

ADR-0169:135-138 requires each recovery to finish under fairness of its own
steps, never fairness against another recovery. Production continues checking
every recovery's elapsed time while it waits at a round barrier
(`crates/overdrive-control-plane/src/lib.rs:1654-1674`). The formal model has
no elapsed clock and already represents the bound through `FailStop`.

Define:

```quint
def waitingOnRound(c: str): bool = and {
  holding(c),
  not(resultPending(c)),
  COMPONENTS.exists(d => resultPending(d)),
}
```

Extend the canonical `FailStop(c)` guard with `waitingOnRound(c)`. Selecting
it represents the existing per-component deadline expiring while `c` is an
`outside` joiner or a collected `keep` / `restore` result waiting for another
pending task. Keep `OwnFair(c)` and every temporal outcome; include
`roundState` in the state tuples of actions whose enabledness reads it.

The recording conformance selector does **not** gain this branch in
RECROUND1. The existing conformance `FailStop` handler remains limited to its
already-drivable spent-attempt/leak cases; seeded R5-58 simulation remains the
production evidence for the elapsed deadline. No driver handler or clock
behavior changes.

### Representation invariant

Add to `Safety`:

```quint
val RoundStateIsRecovery: bool = and {
  COMPONENTS.forall(c =>
    roundState.get(c).in(Set("outside", "pending", "keep", "restore"))),
  COMPONENTS.forall(c => inRound(c).implies(holding(c))),
  COMPONENTS.forall(c =>
    (roundState.get(c) == "restore").implies(rec.get(c) == "repaired")),
  not(COMPONENTS.exists(c => roundState.get(c).in(Set("keep", "restore"))))
    or COMPONENTS.exists(c => resultPending(c)),
}
```

The final term states that a collected result is observable as model-only
intermediate state only while another round result remains pending; otherwise
the barrier applies in the same action.

Add expectations to the existing overlap run, without a new run/check:

- after `RecQuiesce(C1)` then `RecQuiesce(C2)`, C1 is `pending` and C2 is
  `outside`;
- after C1 repairs/restores and its round completes, C2 is `pending` in the
  next round.

The existing repair-failure run continues to prove that a single-member
failure counts once and stays held.

## Projection and implementation boundary

`roundState` is explicitly model-only. It is omitted from
`QuiescenceState`, `State::from_spec`, and `State::from_driver`. The driver
must not maintain a shadow map or infer it from component order. This follows
the repository rule that a model variable may be marked model-only and not
projected.

The existing driving-point check remains unchanged: a selected
`RecRepair(c)` or `RepFail(c)` must find the real composed fixture's existing
`held_repair(c)` effect. A mismatch still fails with the existing typed
`DriverError::NotEnabled`; no action is filtered, auto-completed, or
fabricated.

QPROJ-6 remains byte-for-byte and semantically unchanged:

- no `RecoveryEpisode` field;
- no `RecoveryPhase` variant;
- no `SharedGuestNetworkRecovery` or `recovery_progress()` change;
- no exposure of `round_pending`, `round_results`, or
  `joined_since_attempt`; and
- no `ComposedNode`/supervisor observation accessor.

The bounded implementation surface after approval is:

- `specs/quint/guest-flow-owner/quiescence.qnt` — one state map, guards,
  barrier effects, invariant, fairness tuples, and existing-run expectations;
- `crates/overdrive-control-plane/src/quint_quiescence.rs` — documentation
  only, classifying the new variable as model-only; no handler or projection
  code change;
- `specs/quint/guest-flow-owner/README.md` — exact abstraction and action map;
- `docs/feature/netns-density-295/feature-delta.md` — after approval only,
  formal variable/action/projection wording while QPROJ-6 stays unchanged;
  and
- regenerated existing guest-flow-owner evidence after validation.

No production Rust, fixture, public/internal API, typed error, executable
test, expectation, lane, check inventory, acceptance criterion, or DES history
change is authorized.

## Unproven next-round hypotheses — no requirement or blocker

Source review raises two questions after a round contains multiple members:

1. concurrent attempt tasks call unkeyed `converge_shared(&self)` while the
   owner holds one internal `repair_component` slot; and
2. the composed fixture's `release_audits()` releases every parked audit.

Neither has a retained failing seeded `overdrive-sim` invariant through the
current composition for the claimed task-owner/repair-target or multi-audit
divergence. They are therefore hypotheses only. RECROUND1 does not mandate a
component parameter, new method, typed-error expansion, serialization policy,
observation accessor, scheduler control, or test seam for them.

The status-map proposal is evaluated against observable supervisor episode
and barrier outcomes; it does not promise an internal per-task repair-target
identity that the canonical contract has not independently proven necessary.
If implementation/validation reaches a new failure, retain its seed and exact
production-composition path, return only that proven dependency to DESIGN,
and apply the native falsification gate. Do not pre-design a remedy from the
source hypothesis.

An attempted candidate replay on 2026-10-26 was interrupted at bootstrap
immediately after Metal lease acquisition when isolation was corrected; no
test started and it is not evidence. The temporary canonical Quint hunks were
removed. Any future candidate execution must route from isolated
`spike-scratch` source without changing the canonical spec; current tooling's
macro path is fixed, so inability to route such a candidate is a tooling
limitation, not authority to patch the canonical oracle.

## Lifecycle Gate Ownership

Not applicable. RECROUND1 represents existing result enabledness and barrier
linearization; it adds or moves no production gate.

## Typed error contract

No typed product error changes. Existing repair, restore, and fail-stop
mappings remain unchanged. `DriverError::NotEnabled { action, why }` remains
the conformance diagnostic for a selected action the production composition
cannot drive. No semantic distinction is encoded in a string or added to an
error enum.

## Reuse analysis

| Existing component | Overlap | Decision |
|---|---|---|
| QPROJ-6 `rec` / `att` projection | Episode phase and completed attempts | Keep unchanged; add orthogonal model-only round status. |
| Supervisor `round_pending` / `round_results` | Membership and barrier | Represent their observable scheduling semantics; do not expose them. |
| EXEC `joined_since_attempt` | Protects a mid-round joiner from charge | Reuse exact semantics, unchanged. |
| `ComposedNode::held_repair` | Existing action drivability proof | Reuse at the driving point; do not turn it into state projection. |
| Existing `FailStop` abstraction | Per-component attempt/deadline bound | Extend canonical guard for waiting state; do not change conformance handler. |

No new component is created.

## Rejected alternatives

| Alternative | Why it loses |
|---|---|
| Boolean membership only | Prevents the first false action but publishes multi-member attempts/restores before production's barrier. |
| Add round fields to QPROJ-6 | Adds observation surface for model scheduler state without solving a product outcome. |
| Infer membership by component order | Reverse start order is reachable. |
| Filter result actions whenever recoveries overlap | Removes accepted overlap-plus-failure coverage. |
| Auto-complete the participant inside a joiner's action | Changes phase/count before the requested action. |
| Serialize production recoveries | Reopens ADR-0169's rejected global serialization and changes per-component bounds. |
| Add a component-targeted repair API now | The claimed need has no failing seeded production-composition invariant; adding it would be unapproved scope and invented surface. |
| Add a test scheduler/seam to prove the hypothesis | Manufacturing the schedule violates the production-reachability gate; any missing testability boundary must be approved separately. |

## Normative contract diff

RECROUND1 adds model-only state and tightens result enabledness. It moves
multi-member formal count/restore effects to their already-existing production
barrier. It does not remove or relax `zero`, `never`, `must`, `exactly`,
fail-closed, forbidden, or required language. It preserves counted holds,
overlap, independent per-component failure, the no-charge joiner rule,
per-component deadlines, restore-at-zero, and every fail-stop outcome.

## Validation and cost

After review and user approval, validation is exactly:

1. `cargo xtask quint typecheck` and all existing guest-flow-owner checks and
   hazard controls;
2. replay `QUINT_SEED=0x2355dc58` against the randomized quiescence driver;
3. every existing named/randomized quiescence conformance target;
4. `cargo xtask conformance-lint`;
5. the complete default union that previously produced 2,123/2,124; and
6. the existing three-job guest-flow-owner evidence record followed by
   `cargo xtask quint verify-evidence`.

No mutation, new test, new check, new hazard, new lane, KVM, or real-kernel
job is added. Runtime/API/wire/storage cost is zero. The model-state cost is
one two-component four-value map (16 raw valuations, constrained by
`RoundStateIsRecovery`) inside existing jobs. Any material budget growth must
be surfaced; checks or bounds must not be weakened.

## Approval and downstream gates

Independent review checked both correctness and necessity and approved the
proposal for user decision; the user approval quoted above authorizes this
exact canonical fold. STEP 12-01 may resume only after the fold itself is
independently reviewed. Roadmap validation and replacement formal/conformance
evidence remain pending throughout.
