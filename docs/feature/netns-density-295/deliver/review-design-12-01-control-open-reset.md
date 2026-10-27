# Independent DESIGN/formal review — STEP 12-01 buffered ControlOpen reset

## Current disposition

| Field | Value |
|---|---|
| Current iteration | 2 — post-approval authority and fold reconciliation |
| Decision | `CONTROLOPENRESET1` |
| User authority | **APPROVED 2026-10-26** — exact quote: “Approve CONTROLOPENRESET1” |
| Canonical authority | `feature-delta.md` authority row `CONTROLOPENRESET1` |
| Canonical contract | `owner_flows` `HostReadOpen(c)` action map and accepted `CONTROLOPENRESET1` clause |
| Technical findings | 0 |
| Final verdict | **APPROVED** |
| Roadmap disposition | Validation remains **pending evidence** with `approved_at = null` |

The Iteration 1 metadata and verdict below record the pre-approval review
state. Iteration 2 at the end of this artifact supersedes only its user-
authority disposition.

## Review metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_control_open_reset` |
| Date | 2026-10-26 |
| Proposal | `docs/feature/netns-density-295/deliver/design-12-01-control-open-reset.md` |
| Decision | `CONTROLOPENRESET1` |
| Scope | Stable `HostReadOpen(c)` post-state for a valid buffered `ControlOpen` followed by its already-queued reset |
| Technical findings | 0 |
| Authority finding | 1 blocking user-decision gate |
| Technical disposition | **APPROVED FOR PRESENTATION** |
| Verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |
| Roadmap disposition | Validation remains **pending / non-executable** with `approved_at = null` |

This review is bounded to the reproduced writer-down `HostReadOpen` branch and
its existing session-loss composition. It reads the raw trace, current formal
branches, production control-task path, Sim buffer/end ordering, driver effect
boundary, and accepted K-A2-prime/ATTR-LIVE-CLAIM/session-loss contracts. It
creates only this native Markdown review artifact. It changes no proposal,
canonical design, roadmap, model, driver, production/test source, evidence,
DES state, branch, or commit, and runs no model, test, Metal, build, mutation,
or evidence job.

## Reproduced premise

The native contradiction is exact:

- Metal run: `0ca90d92-a15a-468a-b49b-7a22d03b1e8c`.
- Seed: `0x6539f0dd`.
- Trace 33 reaches `VmStop -> Quiesce -> HostAcceptSession(2) ->
  ActivateQuiesced -> HostReadOpen(2)`.
- The listener child was queued while launch 1 was live, with a complete valid
  `ControlOpen`; VMM death attached a reset without discarding the child or its
  buffered record.
- Before `HostReadOpen`, model and production agree that the connection is
  `reading`, the allocation is `Provisioned`, generation is 1, the VM is down,
  the claim is free, and forwarding is quiesced.
- The step's only mismatch is:

```text
model:          claim = 2, sess(2) = dead
implementation: claim = 0, sess(2) = closed
```

Every allocation, flow, dial, activation, quiescence, and other session value
matches. This is an existing formal-action post-state omission, not a token,
identity, projection, or production cleanup defect.

## Complete production causal path

The proposal follows the current production owner and existing Sim transport
semantics without inserting a schedule:

1. `GuestOpenSession(2)` queues a connected control child with its complete
   current-launch opening record.
2. `VmStop` preserves the queued record and attaches `Reset` to that child.
   This is K-A2-prime: VMM death does not dequeue the child or discard already
   buffered bytes.
3. `HostAcceptSession(2)` releases the real owner accept. The existing Sim
   adapter moves both the buffered record and the attached reset to the
   accepted record socket; the owner registers the key in `unread` and starts
   the existing control task.
4. `HostReadOpen(2)` invokes the driver's one existing
   `release_read(key, None)` effect. It moves every buffered opening byte to
   ready and, because the buffer is then empty and an end already exists,
   makes the reset readable in the same release.
5. `read_record` returns ready bytes before the released end, so the production
   task decodes and validates the complete `ControlOpen` rather than rejecting
   it early.
6. The current launch matches and no claim is held, so the task briefly installs
   the session holder, sender, and cancellation token.
7. The task immediately enters the existing control read loop. No bytes remain;
   the already-readable reset returns `ConnectionReset`, so the loop exits.
8. `session_lost` clears the holder/sender/cancellation state and performs the
   existing `NoSession` flow/dial/message cleanup. The socket closes and
   `finish_connection` removes the task.
9. The driver settles after the single `release_read` call. There is no QPROJ
   comparison boundary between the brief claim and its mandatory cleanup.

The stable production state is therefore `claim = 0`, session `closed`. Adding
a claim barrier or splitting byte and reset delivery would manufacture a new
test effect rather than observe the current production call.

## Exact branch partition

The proposed order matches the production checks and preserves every existing
branch:

| `HostReadOpen` condition | Required result | Review result |
|---|---|---|
| buffered token is not the current live token | Close this connection; leave any current holder unchanged | Unchanged and exact. |
| another session claim is held | Close this connection; retain the existing holder | Unchanged and exact. |
| valid token, free claim, `writerUp` | `live`, `claim = c` | Unchanged and exact. |
| valid token, free claim, writer down | compose the already-pending reset and existing session-loss effects: `closed`, `claim = 0` | Exact correction. |

The order matters. Token invalidity is still decided before the held-claim
case, and both precede writer liveness. The new branch therefore cannot clear a
different connection's valid claim or turn a stale connection into a session.

## Why writer-down is the exact selector

Within the accepted model, a valid-token, free-claim connection can be
`reading` with its writer down only through the queued-child ordering addressed
here:

- a connection already `reading` when `VmStop` occurs is closed by the existing
  `VmStop` branch before `HostReadOpen` can run;
- a session already live at `VmStop` becomes `dead` and remains owned for the
  separate `HostDeadSessionLoss` action;
- a child from an older generation fails the live-token comparison before the
  writer-down branch; and
- a current-generation child queued before VMM death carries the already-
  attached reset into accept, exactly as the reproduced trace shows.

The selector consequently does not generalize every dead-session path. It
identifies the only `reading`/valid/free state whose driver effect inseparably
delivers the opening record and terminal reset.

## Session-loss effect equivalence

The writer-down branch must reuse the existing `sessionLoss(c)` state effects,
not define a second cleanup meaning:

- close `sess(c)`;
- clear the claim that was only transiently installed;
- abort every registered guest flow;
- end host dials with the existing `NoSession` mapping;
- clear pending paired/refused/abort message sets as the current session-loss
  transition does;
- preserve the allocation, generation, VMM, quiescence, guest-side stimulus,
  beacon, activation, and unrelated session entries; and
- retain the existing closed-terminal and no-cross-launch ghost semantics.

These are semantically identical to the production sequence “claim valid open,
observe reset, run `session_lost`.” The model composes them because the current
driver exposes only the stable boundary after both effects.

Under accepted safety, the branch normally has no registered flows or dials:
a free claim means no live session, and admitted/open flows require that live
session. Reusing the total session-loss cleanup is still correct and prevents a
residue if the branch is reached in a wider model state.

## Claim and stable session meaning

`claim` is the current ClaimSet holder identity: production QPROJ-3 exposes
`Option<GuestConnKey>`, and the model relabels the current holder to connection
`c` or zero. It is not a monotonic acceptance counter or evidence that a
connection held the claim transiently.

Similarly, model `dead` means that the owner still holds this session while the
socket arbiter reports the peer gone. Here the owner has already processed the
reset, cleared the holder, and closed the socket. Projecting `dead`/2 after the
step would assert ownership that does not exist at the comparison boundary.
Projecting `closed`/0 is exact and strengthens the fail-closed state; it does
not erase an observable current claim.

## Retained lifecycle branches

| Lifecycle case | Result after the amendment |
|---|---|
| writer still up at valid free `HostReadOpen` | Session becomes `live` and holds the claim, unchanged. |
| invalid/stale opening record | Connection closes without a claim, unchanged. |
| another connection holds the claim | New connection closes and the holder remains, unchanged. |
| session was live before `VmStop` | `VmStop` produces `dead`; `HostDeadSessionLoss` later closes and clears it, unchanged. |
| connection was already `reading` at `VmStop` | `VmStop` closes it without a claim, unchanged. |
| queued child survives `VmStop`, then accepts and reads | `HostReadOpen` ends `closed` with no claim, corrected. |

`HostDeadSessionLoss` remains enabled and necessary for a previously live
session. No state or action is removed, and no production session-loss gate is
moved.

## Security and invariant preservation

The proposal preserves or strengthens every relevant invariant:

- at most one live session remains true;
- stale/dead launch work never holds a claim against a later generation;
- a closed/reset connection cannot admit a flow or carry a control message;
- current-generation attribution still requires the same token comparison;
- a held claim remains protected from competing connections;
- session loss still aborts every flow and dial through its existing typed
  mapping;
- K-A2-prime still preserves the queued child's buffered record until the real
  accept/read path consumes it; and
- a live session's VMM-death state is not collapsed into this branch.

No `zero`, `never`, `must`, fail-closed, launch-isolation, claim, cleanup, or
typed-result outcome is weakened. The formal action ceases to expose a
production-unobservable transient claim.

## Formal and implementation scope

The amendment changes only the existing `HostReadOpen(c)` valid/free branch
when `writerUp(sess(c).gen)` is false. It adds no model state, action, run,
scenario, hazard, witness, temporal property, check, driver filter, test seam,
API, wire value, or production mechanism. The driver and its one
`release_read` effect remain unchanged.

The existing exact seed supplies a bounded regression. Requiring a new named
run or split driver effect would expand the user's requested scope without
changing a required outcome.

## Materiality and user authority

Runtime/API/wire/storage cost is zero. Formal state-space cost is zero new
state or transitions; one existing transition reaches its actual stable
post-state. Validation cost is the already-required exact replay and complete
replacement evidence.

The change is still material because it amends an accepted internal action
contract. `.claude/rules/design.md` states that a proposal is not evidence of
user acceptance and requires explicit approval before a material decision is
marked accepted or carried into DISTILL/DELIVER; that rule expressly includes
the meaning of an existing state or internal action/port contract. Technical
review cannot supply that authority.

## Exact reviewable user decision

### CONTROLOPENRESET1

Approve this bounded formal-action correction:

> In existing `HostReadOpen(c)`, after the existing valid-live-token and
> free-claim checks, keep the writer-up result `sess(c) = live`, `claim = c`.
> When that same connection's writer is down, compose its immediately queued
> reset and the existing `NoSession` session-loss cleanup in the same action,
> producing `sess(c) = closed` and `claim = 0`. Preserve invalid-token and
> held-claim branches, previously-live `VmStop -> HostDeadSessionLoss`, the
> already-`reading` `VmStop` branch, K-A2-prime, ATTR-LIVE-CLAIM, all existing
> state/action/run/check inventory, and production/API/wire behavior.

Approval authorizes no barrier, byte/reset split, projection shadow, production
gate, public seam, new state/action/scenario, driver filter, or broader session
semantic change.

## Roadmap and evidence disposition

`deliver/roadmap.json` validation remains `pending` with `approved_at = null`.
If the user approves `CONTROLOPENRESET1`, completion still requires:

- exact replay of seed `0x6539f0dd`, including Trace 33;
- every unchanged named and randomized `owner_flows` target;
- the complete guest-flow-owner record with exactly three jobs;
- `verify-evidence`; and
- independent source/proof review.

No canonical acceptance or roadmap-validation approval follows from this
technical review.

## Final verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** The proposal has zero open
technical findings. The reproduced production path makes the buffered opening
record and queued reset one inseparable driver effect; the writer-down selector
is exact; the composed result is semantically identical to the existing
session-loss cleanup; and all live-writer, stale-token, held-claim,
previously-live, already-accepted, security, and generation-isolation branches
remain intact. `CONTROLOPENRESET1` is approved for presentation exactly as
written above, but is not yet accepted or implementation authority.

---

# Review iteration 2 — user authority and canonical fold

## Authority reconciliation

After the zero-finding independent review presented the exact writer-down
branch, the user explicitly said “Approve CONTROLOPENRESET1” on 2026-10-26.
The canonical authority table records that exact quote and decision
identifier. The authority gate is closed without inference from any other
decision.

## Canonical fold comparison

The accepted `CONTROLOPENRESET1` clause and `HostReadOpen(c)` action-map entry
in `feature-delta.md` match the reviewed proposal:

| Reviewed contract | Accepted fold | Result |
|---|---|---|
| Existing action first requires `reading` | Same existing precondition | Exact |
| Existing current-live-token comparison occurs before the new branch | Same order | Exact |
| Invalid token closes without taking a claim | Same retained branch | Exact |
| An already-held claim closes this connection and retains its holder | Same retained branch | Exact |
| Valid/free plus writer up becomes `live`, `claim = c` | Same retained result | Exact |
| Valid/free plus writer down composes the immediately queued reset and existing `NoSession` session-loss effects | Same bounded correction | Exact |
| Writer-down stable result is `closed`, claim 0 | Same exact state | Exact |
| Registered flows/dials and in-flight message/result state use existing session-loss cleanup | Same existing effects; no second cleanup meaning | Exact |
| A session already live at `VmStop` remains `dead` until `HostDeadSessionLoss` | Same separate lifecycle branch | Exact |
| A connection already `reading` at `VmStop` closes in `VmStop` | Same retained branch | Exact |
| K-A2-prime, ATTR-LIVE-CLAIM, production/Sim/driver gates, and unrelated formal semantics remain | Same preserved contracts | Exact |
| No API, wire, production behavior, mechanism, state, action, run, scenario, hazard, witness, check, or filter | Canonical clause expressly excludes each | Exact |

The fold adds no effect beyond the reviewed action composition. In particular,
it introduces no claim barrier, byte/reset split, projection shadow, driver
filter, new scheduling point, or broader writer-down/session-dead rule.

## Evidence disposition

The downstream source alignment has received its separate independent review
without extra effects. That does not replace the complete replacement-proof
gate. `deliver/roadmap.json` validation remains `pending` with
`approved_at = null`, and completion still requires:

- exact replay of seed `0x6539f0dd`, including Trace 33;
- every unchanged named and randomized `owner_flows` target;
- the complete guest-flow-owner record with exactly three jobs;
- `verify-evidence`; and
- independent source/proof review of the complete replacement evidence.

This iteration changes only this review artifact.

## Iteration 2 verdict

**APPROVED.** User authority for `CONTROLOPENRESET1` is exact and dated
2026-10-26. The canonical fold matches the zero-finding reviewed branch order,
stable state, cleanup effects, and exclusions without extra API, wire, model,
runtime, scheduling, lifecycle, or outcome changes. The prior
`USER_DECISION_REQUIRED` disposition is closed. Roadmap validation remains
pending full replacement proof.
