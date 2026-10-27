# STEP 12-01 approved DESIGN contract — beacon FIFO model identity

## Status and boundary

**Status: BFIFO1 APPROVED by the user 2026-10-26; post-approval fold review,
implementation and evidence remain pending.** Exact authority: “i approve the
rest of the decisions.” Technical review found zero open findings. This
approval authorizes only the bounded contract below; STEP 12-01 remains
non-executable until its remaining gates pass.

BFIFO1 corrects one reproduced abstraction error in the existing
`owner_flows` beacon state machine: `HostAcceptBeacon(b)` may currently select
any connection whose phase is `pending`, while both production and
`SimBeaconIo` accept the head of one FIFO listener queue. The correction adds
one model-internal ordered fact and changes the guard/effect of the existing
open and accept actions. It adds no action, scenario, named run, hazard,
runtime owner, production mechanism, public product API, or observable product
outcome.

The accepted launch-token, live-claim, stale-generation, VMM-exit, and
reclamation contracts remain unchanged. D-UP-19's reclamation-order
abstraction also remains unchanged. Only D-UP-19's claim that every accept
model may choose any pending connection is invalid for the beacon conformance
map.

## Reproduced counterexample

Metal run `50c02457-e6a3-49f4-a701-d22376d53bcc` failed the randomized
`flows_beacon_conf` driver with `QUINT_SEED=0x2331472b`. The complete raw log
is
`.context/netns-density-295/12-01/quint-flows-beacon-randomized-seed-2331472b-verbose1.log`.
Its relevant trace is:

```text
Init
  -> Provision
  -> VmStart
  -> VmStop
  -> Provision
  -> VmStart
  -> GuestOpenBeacon(2)
  -> GuestOpenBeacon(1)
  -> VmStop
  -> HostAcceptBeacon(1)
```

`GuestOpenBeacon(2)` makes connection 2 pending first (raw lines
21707-21741), then `GuestOpenBeacon(1)` makes connection 1 pending (lines
21743-21777). `VmStop` correctly leaves both queued connections pending with
their generation 2 tokens (lines 21779-21812). The model then selects
`HostAcceptBeacon(1)` and closes connection 1 because no driver launch is live
(lines 21814-21848). The real sim queue still has connection 2 at its head, so
the driver refuses the fabricated selection: `connection Some(2) is ahead of
1` (lines 21850-21864).

This is a native falsification of the current D-UP-19 beacon premise, not a
new desired behavior. Filtering the seed, choosing the numerically smallest
connection, or treating connection 2's real accept as connection 1 would hide
the mismatch rather than model the production transition.

## Production and simulation facts

The production path contains one accept owner and one FIFO kernel queue:

1. `BeaconIntake::bind` starts one `run_accept_task`
   (`crates/overdrive-worker/src/beacon_intake.rs:58-68`). That task performs
   one `BeaconIo::accept()` at a time and passes the returned
   `BeaconConnKey` into the per-connection read task (`:128-173`). The
   asynchronously spawned READY reads do not reorder which child `accept`
   returns.
2. `VsockBeaconIo::accept` makes one `accept_with` call and returns the peer
   identity of that accepted child (`crates/overdrive-host/src/beacon_io.rs:128-167`).
3. Linux 7.0 appends each connected child at the tail of the listener's vsock
   accept queue (`vendors/linux/net/vmw_vsock/af_vsock.c:499-510 @ v7.0`),
   removes `accept_queue.next` (`:688-707 @ v7.0`), and calls that dequeue from
   `vsock_accept` (`:1824-1878 @ v7.0`). That is FIFO.
4. `SimBeaconIo` implements the same contract with
   `State.queue: VecDeque<u64>`, `queue_connection`/`push_back`, and
   `try_accept`/`pop_front`
   (`crates/overdrive-sim/src/adapters/beacon_io.rs:112-128`, `:208-242`,
   `:361-383`).

VMM death does not remove an unaccepted child. The current model already
represents K-A2-prime correctly: `VmStop` closes current-generation beacon
connections in `reading` or `live`, while a `pending` connection stays queued
with its buffered READY and original generation
(`specs/quint/guest-flow-owner/owner_flows.qnt:361-383`). The FIFO fact must
therefore cross `VmStop`, `Provision`, and `VmStart`; clearing or sorting it at
a generation boundary would contradict P-37/K-A2-prime and the reproduced
trace.

## Exact approved formal contract

### One model-internal queue

Add exactly one variable to `owner_flows`:

```quint
var bq: List[int]
```

`bq` is the ordered list of beacon connection IDs in the one listener accept
queue. It is model-internal history formed only by the existing
`GuestOpenBeacon` and `HostAcceptBeacon` actions. It is not an owner field, a
runtime queue, a driver-maintained copy, a new sim field, or a new production
API.

The exact transition contract is:

| Existing transition | Required `bq` effect |
|---|---|
| `InitWith` | `bq' = List()` |
| `GuestOpenBeacon(b)` | Preserve every existing guard and state effect, and append once: `bq' = bq.append(b)`. The existing connection-phase guard prevents a duplicate live occurrence of `b`. |
| `HostAcceptBeacon(b)` | Replace the arbitrary-pending guard with `bcn.get(b).st == "pending" && bq.length() > 0 && bq.head() == b`; preserve the existing `reading`/`closed` branch; remove exactly that head with `bq' = bq.tail()`. |
| `VmStop` | Preserve `bq`. Pending children remain queued across VMM death; accepted `reading`/`live` children are already absent from `bq`. |
| `Provision`, `VmStart`, `DriverStop`, `HostReadReady` | Preserve `bq`. None accepts or removes a queued child. |
| Every non-beacon action and helper that currently preserves beacon state | Preserve `bq` alongside `bcn`; non-beacon instances keep it empty. |

No action removes a non-head item. No generation transition rewrites, sorts,
or clears the queue. A stale child at the head is accepted first, then follows
the existing outcome: it closes immediately when `drvLive = false`, or enters
`reading` and is closed by the existing token/live-launch check. Its removal
then exposes the next queued child. A later reuse of the same bounded model ID
is enabled only after its previous occurrence was accepted, removed from
`bq`, and reached `closed`, so the append is unambiguous.

### Structural safety in the existing check

Add one structural clause to the existing `BeaconSafety` conjunction:

```text
BeaconQueueExact:
  - every element of bq belongs to BCONNS;
  - bq contains no duplicate connection ID; and
  - for every b in BCONNS, b is in bq exactly when bcn(b).st is pending.
```

This clause checks that every transition preserves the meaning of the new
state. It does not create a new product outcome, hazard family, check, or
named run; the existing `BeaconSafety` checks exercise it. `bq` must also join
the existing `allVars` tuple used for weak fairness. The current per-connection
fairness remains valid: only the head accept is enabled, and weak fairness for
that existing action eventually removes it before a later queued connection
can become the head.

### Exact conformance handoff

`HostAcceptBeacon(b)` continues to mean **release one real accept**, not
"select connection `b` from the listener." The corrected model makes that
action playable only when `b` is its own modeled FIFO head. After the existing
`release_accept`, the real `SimBeaconIo` pops its actual head and the existing
state projection observes the accepted identity:

- a live launch reports the returned key in `BeaconObservation.unread`, which
  projects that exact `b` as `reading`; or
- no live launch closes the returned socket, and QPROJ-7
  `closed_by_driver` projects that exact `b` as `closed`.

The normal post-step state comparison therefore proves model/adapter identity
without a new read surface. The conformance driver must remove its private
`accept_order: VecDeque<i64>` and must not replace it with another shadow
queue, numeric-ID ordering, candidate filter, or chosen-ID accept seam. The
existing `SimBeaconSocketObservation.queued` map continues to project pending
membership and buffered token; it does not become an ordering oracle.

This is a truthful abstraction because the same ordered action history drives
the model queue and the real sim queue, while the accepted connection's real
identity is observed after every accept. A divergence in either queue changes
which `bcn` entry becomes `reading` or `closed` and fails the existing state
comparison.

## Lifecycle closure

The approved state closes all existing beacon lifecycle transitions without
adding a cleanup path:

- **Queued then VMM exit:** `VmStop` leaves the child in `bq` and leaves
  `bcn(b).gen` unchanged. A later accept removes the head and closes it if no
  launch is live.
- **Queued old generation then new launch:** `Provision` and `VmStart` leave
  the old head in place. A new generation may queue another connection behind
  it. FIFO accepts the stale child first; `HostReadReady` retains the existing
  generation/token rejection.
- **Accepted then VMM exit:** `HostAcceptBeacon` already removed the child
  from `bq`; `VmStop` closes its `reading` or `live` state under the current
  rule, so no queue deletion is needed.
- **Driver stop:** `DriverStop` closes the existing live claim only. It does
  not own pending listener children and preserves `bq`.
- **Claim rejection or READY rejection:** both occur after accept, so
  `HostReadReady` closes a child already absent from `bq`.
- **ID reuse:** a closed ID can be opened again only after its previous accept
  removed it from `bq`; the new open appends a new occurrence at the tail.

These are consequences of the reproduced FIFO identity fact and existing
K-A2-prime/ATTR-TOKEN/ATTR-LIVE-CLAIM transitions. They add no generalized
listener cleanup or runtime ordering policy.

## D-UP-19 replacement boundary

BFIFO1 replaces only the accept-order half of D-UP-19 with:

> Accept order is abstracted away for owner models whose invariants and
> conformance actions are connection-order-insensitive. `owner_flows` beacon
> acceptance is the bounded exception: its existing connection-ID action is
> mapped to one sequential `BeaconIo::accept`, so the model records the
> listener FIFO in `bq`; `GuestOpenBeacon` appends, `HostAcceptBeacon` accepts
> and removes only the head, and lifecycle transitions preserve queued stale
> children. Boot reclamation remains one `Reclaim` action with no per-orphan
> order.

R5-59 remains a per-connection, order-insensitive adapter-equivalence test.
That test's oracle does not authorize the formal driver to select an identity
the real accept call did not return.

## Rejected alternatives

| Alternative | Reason rejected |
|---|---|
| Keep any-pending acceptance and reinterpret the real head as model-selected `b` | Fabricates connection identity and can apply one launch token or close outcome to another child. |
| Enable only the numerically smallest pending ID | Connection IDs are bounded model labels, not enqueue sequence; it removes real schedules such as 2 then 1. |
| Filter randomized traces or retry another action after the driver refusal | Hides a valid model transition instead of reconciling the model with the port. |
| Retain the driver's private `accept_order` as the contract | Duplicates arbiter state in the harness; the model remains over-permissive and the driver becomes the source of truth for a queue it does not own. |
| Add an ordered public sim observation or chosen-ID accept method | Unnecessary surface. The existing action history plus the real accepted identity closes the proof. |
| Clear pending children at `VmStop`, `Provision`, or `VmStart` | Contradicts K-A2-prime/P-37 and the reproduced stale-queue production path. |
| Add a new accept-head action, scenario, hazard, or runtime reorderer | The existing `HostAcceptBeacon` action and runtime FIFO already contain the required transition. |

## Approved implementation and evidence gates

User authority is recorded as BFIFO1 on 2026-10-26 by the exact quote “i
approve the rest of the decisions.” The bounded implementation surface is:

- `specs/quint/guest-flow-owner/owner_flows.qnt`: `bq`, exact existing-action
  effects, `BeaconQueueExact`, and `allVars`;
- `specs/quint/guest-flow-owner/README.md`: the approved D-UP-19 beacon
  exception and variable/action inventory;
- `crates/overdrive-worker/src/vm_driver/quint_flows_beacon.rs`: remove the
  private `accept_order` bookkeeping and let one real accept plus existing
  post-state projection establish identity; and
- generated formal evidence only through the normal record workflow.

No production module or public product interface is in scope. Because the
Quint model input changes, completion requires replay of exact seed
`0x2331472b`, every unchanged named and randomized beacon conformance target,
the complete guest-flow-owner re-record with exactly three jobs,
`verify-evidence`, and independent proof/source review of the replacement
evidence. Until those results are recorded and approved, the feature delta
and roadmap validation remain pending.
