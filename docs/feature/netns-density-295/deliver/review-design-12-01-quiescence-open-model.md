# DESIGN review: 12-01 quiescence OpenFlow enabledness

## Current disposition

| Field | Value |
|---|---|
| Current iteration | 2 — post-approval authority and fold reconciliation |
| Decision | `QUIESCENCEOPEN1` (reviewed as `QUIESCENCE-OPEN-1`) |
| User authority | **APPROVED 2026-10-26** — exact quote: “i approve the rest of the decisions.” |
| Canonical authority | `feature-delta.md` authority-table row `BFIFO1, PABORTERR1, ACTPRIORITY1, ADMISSIONREFUSAL1, QUIESCENCEOPEN1, CHCGROUP55RUNNER1` |
| Technical findings | 0 |
| Final verdict | **APPROVED** |
| Roadmap disposition | `deliver/roadmap.json` validation remains **pending evidence** with `approved_at = null` |

The Iteration 1 metadata and verdict below record the pre-approval review state.
Iteration 2 at the end of this artifact supersedes its authority gate and
corrects its non-normative Trace 1 label.

## Review metadata

| Field | Value |
|---|---|
| Proposal | `docs/feature/netns-density-295/deliver/design-12-01-quiescence-open-model.md` |
| Canonical destination | `docs/feature/netns-density-295/feature-delta.md`, pending `QUIESCENCE-OPEN-1` amendment |
| Roadmap | `docs/feature/netns-density-295/roadmap.md`, validation remains pending |
| Review iteration | 1 |
| Scope | Model-only correction to `canOpenFlow` after fixed `FlowListeners` damage |
| Technical findings | 0 |
| Authority gates | 1 — explicit user decision required |
| Technical disposition | Approved for presentation as the exact `QUIESCENCE-OPEN-1` decision below |
| Verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |

## Proposal reviewed

The proposal changes only the existing `canOpenFlow` predicate in
`specs/quint/guest-flow-owner/quiescence.qnt`:

```quint
def canOpenFlow =
  up and
  not(latch) and
  flowsOpen == 0 and
  not(dmg.get("C1"))
```

It adds no model state, action, effect, scenario, driver filter, production API,
or production recovery mechanism. `OpenFlow` retains its existing effect and
the driver retains its existing production operation and projection.

## Reproduced premise

The bounded native failure is established.

- Seed: `0x8703c2cd`.
- Recorded Metal run: `f4e6751b-6063-4ba3-b8fb-579adcaf1e21`.
- Raw trace: `RestartWith(true) -> Damage(C1) -> OpenFlow`.
- Immediately before `OpenFlow`, both sides agree on `up = true`,
  `latch = false`, `flowsOpen = 0`, and `dmg.C1 = true`.
- The `OpenFlow` step is the sole mismatch: the current model reaches
  `flowsOpen = 1`, while the production projection remains `flowsOpen = 0`.

This is an enabledness mismatch. It is not evidence for changing the
`OpenFlow` effect, its result projection, or the production implementation.

## Production reachability and owner path

`C1` is the fixed `FlowListeners` component. The conformance driver damages it
through the real composed-node failure port and waits until the stream-listener
accept task has exited. The subsequent `OpenFlow` driver operation queues a new
stream request and settles the production composition. With the sole stream
accept task gone, that request cannot enter the admitted guest-flow owner path,
so the production `flowsOpen` projection remains zero.

The projection counts owner-held guest flows, dials, and datagrams. It does not
count a child merely queued at the damaged listener. The model action therefore
means that a newly accepted request has entered the admitted/read/attached/
paired flow path. It does not mean that an unaccepted child exists in the
listener queue. Disabling that action while fixed `C1` damage is present is the
smallest correction to the false abstraction.

An acceptance that occurred before `Damage(C1)` is not lost by the new guard.
Once the production accept task has registered the flow owner, the projection
already reports an open flow, so the corresponding model order is
`OpenFlow -> Damage(C1)`. That retained order represents the legitimate case in
which a registered flow remains while the listener accept task is down.

## Exactness of the selector

The selector is exact for the reproduced defect:

- `dmg.C1` is set only after the real fixed `FlowListeners` accept task has
  exited.
- `dmg.C1` is cleared by the existing repair action only after the production
  repair path restores that component.
- The other existing preconditions — node up, quiescence latch clear, and no
  currently open flow — retain their meanings.
- The proposal does not infer listener health from a watch value or from an
  adjacent lifecycle state. It uses the existing component-damage fact that
  drove the real failure.

No broader damage predicate is justified by the reproduced trace.

## Retained lifecycle behavior

| Production/model order | Result after the amendment | Review disposition |
|---|---|---|
| `OpenFlow -> Damage(C1)` | The existing registered flow remains represented; later quiescence can close it. | Retained and required. |
| `Damage(C1) -> OpenFlow` | `OpenFlow` is disabled until repair. | Corrects the reproduced false transition. |
| `Damage(C1) -> RecQuiesce -> RecRepair` | Repair clears `dmg.C1`, but the latch still prevents opening. | Retained. |
| `Damage(C1) -> RecQuiesce -> RecRepair -> RecRestore -> OpenFlow` | Restore clears the latch after repair, so opening becomes possible again. | Retained. |
| crash or restart recovery | Existing recovery resets damage and flow state according to its current action. | Retained. |
| undamaged running node | Existing `OpenFlow` enabledness is unchanged. | Retained. |

The proposal does not change any recovery action or make repair alone sufficient
to reopen admission. It preserves the existing repair-then-restore order.

## Why `C2` is excluded

`C2` is the fixed control-listener accept task. Its failure prevents a new
control session from being accepted, but it does not destroy an existing live
control session. A guest flow may therefore still be opened through that live
session after `Damage(C2)`. Adding `not(dmg.C2)` would remove that legitimate
production behavior and would overgeneralize the C1 finding.

The current conformance fixture lazily creates a control session when
`OpenFlow` first needs one. A trace that damages `C2` before the fixture has
created the session can consequently be unplayable even though the abstract
production state with an already live session is valid. Establishing that live
session in the applicable test fixture is DISTILL-owned test support. It is not
a reason to add a C2 guard, new model state, or a driver-side trace filter.

## Invariant and scenario assessment

The amendment strengthens action enabledness without changing the state update.
It preserves the accepted safety and lifecycle meanings:

- a quiesced node holds no open flow;
- the latch and recovery count remain aligned;
- no flow reopens before the existing repair and restore sequence completes;
- damage of a listener does not silently close an already registered flow;
- recovery remains explicit rather than being inferred from a later open.

The existing named recovery runs open flows only when `C1` is healthy. The
existing disabled-flow witness remains disabled under its prior latch condition.
No scenario deletion, trace filter, or weakened check is required. The
amendment should be verified by rerunning the exact seed and the complete
jobs-3 formal evidence after it is implemented.

The proposal's Trace 1 handling is also correct. The fixture-side lazy-session
precondition is a DISTILL concern and does not justify broadening this model
change. The reproduced Trace 2 mismatch is independently sufficient to require
the C1 enabledness correction.

## Scope and material cost

The proposal has no runtime resource, API, wire, persistence, production
ownership, or operational cost. Its material effect is confined to the formal
contract: the model no longer permits a new `OpenFlow` transition while the
fixed `FlowListeners` component is known to be down.

That change is still a material internal action-contract amendment. The
applicable `.claude/rules/design.md` rule states: “Before a material design
decision is marked accepted, carried into DISTILL, or used to authorize
DELIVER, the architect or orchestrator MUST surface it to the user in the
conversation and obtain an explicit approval.” It expressly applies to “a
public or internal action/port contract.” Technical review cannot supply that
authority.

## Exact user decision

### QUIESCENCE-OPEN-1

Approve the following bounded formal-model amendment:

> Add `not(dmg.get("C1"))` to the existing `canOpenFlow` predicate. Keep the
> existing `OpenFlow` action, effect, driver operation, production projection,
> state variables, scenarios, APIs, and recovery mechanisms unchanged. Do not
> add a C2 guard: a dead control-listener accept task does not invalidate an
> existing live control session. Treat any lazy-session fixture preparation as
> DISTILL-owned test support.

Approval of `QUIESCENCE-OPEN-1` authorizes only this amendment. It does not
authorize a global damage guard, a queue-state model, a new control-session
state variable, a driver trace filter, or production behavior changes.

## Roadmap and validation disposition

The feature delta and roadmap must remain pending until the user explicitly
approves `QUIESCENCE-OPEN-1`. After approval and implementation, the exact
seed must be replayed and the complete jobs-3 evidence must be recorded before
the canonical delta or roadmap validation can be approved.

## Final verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** The proposal has technical
review zero: the C1 predicate is necessary, exact, complete for the reproduced
failure, and preserves the accepted lifecycle and safety behavior. No
implementation authority is granted. The reviewable user decision is the exact
`QUIESCENCE-OPEN-1` text above.

---

# Review iteration 2 — user authority and canonical fold

## Authority reconciliation

The decision sequence is explicit in the canonical authority table. The user
first said “Approve 240-second exception,” which the immediately preceding
`FLOWQUOTA240` row limits to that timeout decision. After the remaining
reviewed decisions were presented, the user said “i approve the rest of the
decisions.” The 2026-10-26 authority row names `QUIESCENCEOPEN1` among those
remaining decisions and maps it to the exact `QUIESCENCE-OPEN-1` reviewed
here. Authority is no longer inferred or pending.

## Canonical fold comparison

The accepted `QUIESCENCEOPEN1` clause in `feature-delta.md` under the
quiescence action map matches the reviewed proposal:

| Reviewed contract | Accepted fold | Result |
|---|---|---|
| Existing `canOpenFlow` gains only `not(dmg.get("C1"))` | Exact predicate `up && not(latch) && flowsOpen == 0 && not(dmg.get("C1"))` | Exact |
| C1 retains fixed `FlowListeners` identity | Same component identity | Exact |
| Existing `OpenFlow` admitted/read/attached/paired meaning and driver call remain | Same action meaning and call | Exact |
| No C2 or global-damage guard | Expressly excluded | Exact |
| No queue state, action, driver filter, API, or runtime mechanism | Expressly excluded | Exact |
| Exact reproduced seed `0x8703c2cd`, Trace 2 | Same premise | Exact |

The fold preserves every previously reviewed lifecycle branch: an already
registered flow may survive C1 damage until the existing quiescence path
closes it; repair alone cannot reopen while the latch remains set; restore
after clean repair enables the existing action; and C2 damage does not forbid
flow admission over an already live control session.

## Trace 1 record correction

Iteration 1's last invariant paragraph called Trace 1 a “lazy-session
precondition.” That label was incorrect. Trace 1 is the separate old-child
postmortem failure caused by the conformance fixture reusing one Tokio runtime
across simulated process death; real process death destroys that runtime. Its
disposition remains the same: private DISTILL-owned test support, with no
production reachability and no authority for a production cleanup mechanism.

The lazy control-session setup discussed in the C2 analysis is a separate
fixture concern. It continues to justify neither a C2 guard nor new formal
state. This correction changes no `QUIESCENCEOPEN1` normative term and reveals
no canonical fold mismatch.

## Evidence disposition

`deliver/roadmap.json` validation remains `pending` with `approved_at = null`.
Approval and this fold review do not replace exact seed `0x8703c2cd`, every
unchanged named/randomized quiescence target, the complete three-job
guest-flow-owner record, `verify-evidence`, and independent proof/source
review. This iteration changes only this review artifact.

## Iteration 2 verdict

**APPROVED.** User authority for `QUIESCENCEOPEN1` is exact and dated
2026-10-26, and the canonical fold matches the zero-finding
`QUIESCENCE-OPEN-1` proposal. No broader damage guard, API, state, action,
scenario, driver filter, production mechanism, or observable outcome was
introduced. The prior `USER_DECISION_REQUIRED` disposition is closed. Roadmap
validation remains pending replacement evidence.
