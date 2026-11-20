# ADR-0336 — A workload waiting for guest-network capacity is derived at read time

## Status

**Accepted** 2026-11-18 (decisions W-1, W-2 and W-4 of the node-local persistent volume feature, GH #348; user rulings of 2026-11-18: option A
for W-1 and W-2 and option 2 for W-4; the visible waiting state is the user's ruling of the same day on the admission handling of ADR-0338).
The derivation of item 1 is **Accepted** 2026-11-19 by user ruling (H2, option A). This ADR is not implemented and is stated in the present
tense.

## Context

The guest-network pool caps the node at 16,384 attached guests. When the pool is full, `WorkloadLifecycle` emits no placement for a first
placement or a restart (the occupancy gate), and an admission that races past the gate is refused with no allocation row and no charge
(ADR-0338, which states what a refusal does; this ADR states how the wait is shown). In both cases the workload has no allocation row, no
shim runs, and the only signal is the `guest_network.admission_refusal` log event, fired for the race alone. On the appliance there is no operator shell, so the operator cannot see
why a workload that intent says should run is not running. Kubernetes keeps such a pod `Pending` with a named reason.

## Decision

**`overdrive workload describe` reports that a workload is waiting for guest-network attachment capacity, with the held count and the cap. The
state is derived at read time from the stop sentinel, the workload's allocation rows, the reconciler's View and the committed generation, and
the pool's occupancy; nothing is persisted for it.**

1. **Derivation.** The wait holds iff the workload's start is blocked **only** by capacity: intent says run, nothing is running or launching,
   the current allocation holds no guest-network lease, the pool is at its cap, and the reconciler would otherwise emit a placement or a
   restart. "Would otherwise emit" is one pure predicate, shared with the reconciler's own rules and built from the same helpers
   (`current_alloc`, the restart decision, the generation-pending rule of ADR-0335 with its finished-Job exception, the operator-stop veto and
   the Job natural-exit fence), so the display cannot call a workload waiting that the reconciler has finished with, or the reverse. Blocked for
   good, and never a wait: a Job whose rows carry a terminal claim, a `BackoffExhausted` workload, and an operator-stopped workload, each
   with no generation pending. Waiting at the cap: a restarted workload (a generation is pending), `Stopped{LivenessProbe}`,
   `Stopped{SystemGc}` and a crashed row under budget. The Draining test reads the current row only, as the reconciler's does
   (`workload_lifecycle.rs:1009`). The generation-pending input comes from the committed generation and the reconciler's View (feature delta
   § Operator surface: a workload waiting for capacity). The cap comparison is one definition, `GuestAttachmentOccupancy::at_cap`, shared with
   the reconciler's occupancy gate and the pool's admission. The predicate reads the pool only: while the pool is at its cap, `describe` can
   also show the wait for a workload that another limit blocks (no healthy node, CPU or memory `NoCapacity`), which a freed slot does not
   place. A withheld workload is placed by the 30 s resync pass after a slot frees (no row change wakes it), not at once.
2. **Source: a read-time projection in the handler (chosen).** The handler already reads the intent, the rows and, in one call, the pool
   snapshot. The projection adds no action, row, envelope, wake or View field, and cannot be stale.
3. **W-1: no start time and no history.** The line states the live held count and the cap. How long a workload has waited, and a history of
   waits, are not shown. This is a scope limitation, tracked by GH #265 (durable, queryable operational events; scope comment
   https://github.com/overdrive-sh/overdrive/issues/265#issuecomment-6988095924). The wait is a derived current state with no recorded
   start; a refusal charges nothing, so no budget or audit depends on its occurrence.
4. **W-2: guest-network capacity only.** A placement refused for CPU or memory is not shown. This is a scope limitation, tracked by GH #261
   (real node capacity and the `NoCapacity` emit site; scope comment
   https://github.com/overdrive-sh/overdrive/issues/261#issuecomment-6988096862). `CapacityResource` is `#[non_exhaustive]` and has one
   variant, `GuestNetworkAttachments`.
5. **W-4: the wait is shown only in `describe`.** `overdrive workload list` does not exist (no verb, no endpoint, no wire type; `job list` is
   parsed and unwired), so it is its own feature, tracked by GH #367 (https://github.com/overdrive-sh/overdrive/issues/367). When it lands it
   consumes the same pure derivation. This is a scope limitation.

Exact wire, rendering, derivation and test obligations: feature delta § "Operator surface: a workload waiting for capacity".

## Alternatives considered

- **A durable condition row written by the shim or the reconciler.** Rejected: the common case (a placement withheld at the cap) emits no
  action, so the shim never runs; a reconciler cannot write a row, so it would need a new action, row kind, envelope and gossip merge rule;
  and any written marker must be cleared on every exit path (placed, stopped, deleted, a crash in between), which makes the marker the thing
  consulted instead of the pool (`.claude/rules/reconcilers.md` § "A View decides when to act, never whether an effect is in place").
- **Leave the wait to the `guest_network.admission_refusal` log event.** Rejected: the operator has no shell on the appliance and the event
  does not fire for a placement the occupancy check withheld.
- **A handler-local conjunct list ("the current allocation carries no terminal", and so on).** Rejected: it restates the reconciler's rules in a
  second place and drifts. A restarted workload whose current row is an operator-stopped or exhausted row waits at the cap, and a
  conjunct about `terminal` calls it finished; the shared predicate removes the class.
- **Exclude issued allocation ids that hold a lease from the over-report window.** Rejected: more handler reads for a window that is bounded,
  self-clearing and only at the cap (Consequences).
- **A "since" shown only where an input exists (a restart).** Rejected (W-1): right for a restart, absent for a first placement.
- **A durable condition with a `since` and an event history.** Rejected (W-1): a new row, writer, clearing rules and gossip semantics for a
  fact no budget depends on; durable operational events are #265's.
- **Include CPU and memory waits now (W-2).** Rejected: `scheduler::schedule` would have to name the refusing resource and the handler would
  need the node's capacity, which #261 builds.
- **Build `workload list` in this feature (W-4).** Rejected: a new operator verb, endpoint and wire type unrelated to volumes, and a status
  vocabulary for the list that needs its own decision; it is #367's.

## Consequences

- Positive: a workload that cannot be placed because the node's guest-network attachments are full is visible in `describe`, where on `main`
  it is not visible at all, and the display cannot go stale.
- Positive: no new persisted state, action, wake or View field.
- Negative: the wait has no start time and no history (W-1, #265).
- Negative: only guest-network capacity is shown; a CPU or memory refusal stays silent (W-2, #261).
- Negative: the wait is visible only in `describe`; `workload list` does not exist (W-4, #367).
- Negative: the derivation can over-report in three bounded cases, all only while the pool is at its cap. (1) A restart's backoff window is
  still open: the workload is capacity-blocked and would also be backed off. (2) Another limit also blocks the workload (no healthy node, CPU
  or memory `NoCapacity`): the display names the pool, and a freed slot does not place it. (3) A launch is in flight: the launch holds its
  guest-network lease before its allocation row exists (up to the launch's own bound, about 30 s for a VM, about 90 s with a volume), so when
  it takes the last slot `describe` can show `Waiting` for a workload that is starting. Window (3) is bounded and clears when the row is
  written. Outside these cases the display never reports a wait for a workload the reconciler would place if a slot were free.
- Negative: after a slot frees, a withheld workload waits for the 30 s resync pass before it is placed and the line clears.
- Negative: the handler reads the rows, the pool, the stop key, the committed generation and the View separately, so one `describe` can
  combine reads from adjacent instants; the next `describe` is correct.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Operator surface: a workload waiting for capacity
- [ADR-0338](adr-0338-an-admission-refusal-by-the-guest-network-pool-writes-no-row-and-charges-nothing.md) (the admission handling this makes visible),
  [ADR-0335](adr-0335-a-restart-generation-is-handled-when-its-successor-is-observed.md) (the generation-pending rule the predicate reuses),
  [ADR-0134](adr-0134-placement-reads-held-attachment-count-through-read-port.md) (the `GuestAttachmentView` read port),
  `.claude/rules/reconcilers.md`
