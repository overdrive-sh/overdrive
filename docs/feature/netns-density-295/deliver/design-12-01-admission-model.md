# STEP 12-01 approved DESIGN contract — guest-flow admission/refusal action semantics

## Status and boundary

**Status: ADMISSIONREFUSAL1 and ACTPRIORITY1 APPROVED by the user 2026-10-26;
post-approval fold review, implementation and evidence remain pending.** Exact
authority: “i approve the rest of the decisions.” Independent review iteration
3 found zero open technical findings. Approval is limited to the exact two
contracts below; STEP 12-01 remains non-executable until its remaining gates
pass.

The approved decisions reconcile two reproduced `owner_flows`
action-contract gaps:

1. a guest-opened flow that arrives while forwarding is quiesced must never
   become an admitted/registered flow, while the current launch's live control
   session must still receive the accepted `Refused { Quiesced }`; and
2. when a Provisioned CID is both quiesced and has no live control session,
   the existing `activate` call returns `QuiescenceLatched`, while the model
   currently also enables `ActivatePending`.

Both corrections use existing public types, wire messages, actions, and
owners. They add no public API, wire field, enum variant, scenario, formal
action, or persistent state. The separate pre-Paired guest-`Abort` errno
contract remains in `design-12-01-paired-abort-error.md` and is not duplicated
here.

## Reproduced admission contradiction — seed `0xafd56990`

The exact log
`.context/netns-density-295/12-01/owner-flows-afd56990-verbose1.log`
replays the existing named run
`QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore`:

```text
... -> HostConnectDone(1)
    -> Quiesce
    -> GuestOpenFlow(2)
    -> HostAcceptFlow(2)
```

The first hold closes flow 1 and latches admission. The guest then opens flow
2 with its request buffered. The model's existing `HostAcceptFlow(2)` closes
it immediately because `admissible` is false. Current production instead
registers it as `GuestFlowPhase::Registered { request: Nothing }`; the exact
post-state differs only at flow 2 (`closed` versus `accepted`, raw lines
473-629).

A temporary source gate requiring Active + live session + not-quiesced before
registration made the formal replay pass
(`owner-flows-afd56990-verbose1-after-admission-gate.log`), but it closed the
socket without reading its request or sending a control response. That
contradicts the unchanged accepted owner cell
`quiescence_aborts_a_connecting_flow_and_its_late_completion_attaches_nothing`:
after the first hold, its second guest-opened flow must close **and** the live
session must carry exactly the existing
`ControlMessage::Refused { flow, refusal: FlowRefusal::Quiesced }`
(`crates/overdrive-control-plane/src/guest_network/owner_acceptance/admission.rs:131-180`).
The temporary gate is therefore invalid and was correctly reverted.

### Current caller path

`spawn_flow` currently inserts every accepted connection into
`entry.guest_flows` when a session exists and the CID is not Retiring; it does
not consult the admission latch (`owner_runtime.rs:364-401`). The flow task
later reads the request and `handle_flow` checks, in order, current launch and
session, Active, quiesced, then capacity. Its quiesced branch sends the exact
`FlowRefusal::Quiesced`, removes the registered flow, and closes the socket
(`owner_runtime.rs:523-633`). With automatic request delivery the accepted
cell observes only the eventual refusal. With the conformance seam holding the
request after accept, seed `0xafd56990` exposes the intermediate registered
flow.

That intermediate is incompatible with two accepted contracts:

- U-1/per-kind step 3 says the admission check and registration are one step
  relative to quiescence, so the losing side of the race cannot become an
  admitted flow; and
- QHOLD's observable invariant says every CID's `guest_flows`, `dials`, and
  `datagrams` are empty while admission is Quiesced
  (`flow_owner.rs:300-315`).

Closing at accept without a response preserves the state invariant but loses
the accepted typed refusal. Registering first and refusing after a held read
preserves the response but violates the state invariant. The replacement must
preserve both.

## Exact approved admission contract — iteration-2 remediation

### ADMISSIONREFUSAL1: reuse the existing task owner, with one explicit bound

The refusal reader is unavoidable. The owner cannot send `Refused` until it
has read the request's `flow` and authenticated its launch token, while U-1
and QHOLD forbid registering the socket as an admitted flow first. A deadline
alone does not bound concurrent readers. ADMISSIONREFUSAL1 therefore makes
one material resource decision, approved explicitly on 2026-10-26:

No accepted bound already covers this class. `held_flows` and the
per-allocation quota deliberately exclude a refusal reader to preserve zero
admitted flows/places; forwarder inventory counts only pairs/watches; the
kernel backlog bounds children not yet accepted, not tasks after accept; and
`FLOW_REQUEST_DEADLINE` bounds duration rather than arrival-rate concurrency.
The reader cap is therefore a genuinely new internal resource bound, not a
rename of an existing quota. It is recorded explicitly instead of being
smuggled into `reserve` or inferred by the crafter.

- **Owner:** reuse the existing `CidState::flow_tasks` map, `FlowTask`
  cancellation token and join handle, keyed by the accepted `GuestConnKey`.
  A refusal reader is identified exactly by the existing fields
  `task.owner_launch == Some(snapshotted_launch) && task.flow == None` while
  the task is in `flow_tasks`. Ordinary stream tasks keep
  `owner_launch = None`; after a datagram request is decoded its task has
  `flow = Some(flow)`, so neither class is counted. The snapshotted launch is
  also the attribution input used before response. No second collection,
  queue, task supervisor, public observation phase, or API is added.
- **Global bound:** at most `GUEST_FLOW_MAX_PAIRS` (65,536) refusal readers
  may exist node-wide, counted by that exact predicate over every CID's
  `flow_tasks`. It is separate from the maximum number of admitted flows: a
  refusal reader holds no quota place, forwarder pair/watch, or `held_flows`
  count.
- **FD budget:** each reader holds one accepted vsock and one Tokio task,
  cancellation token, and at most one 32-byte request buffer for at most
  `FLOW_REQUEST_DEADLINE`. Three values must not be conflated:
  - current source checks
    `4 * GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION + 16,384 + 1 + 32,767 = 65,536`
    (`guest_network.rs:821-827`);
  - the already-accepted canonical target is
    `4 * GUEST_FLOW_MAX_PAIRS + 16,384 + 1 + 32,767 = 311,296`; current
    source's use of the per-allocation constant is a pre-existing STEP 12-01
    source divergence, not a cost of this decision; and
  - ADMISSIONREFUSAL1 adds exactly `GUEST_FLOW_MAX_PAIRS = 65,536`, so the
    accepted `GuestNetworkError::FdBudget.required` is exactly
    **376,832**. The incremental cost presented to the user is 65,536 fds,
    not the full accepted baseline and not a symbolic “four-to-five” claim.
- **Concurrency:** readers remain concurrent, as existing `flow_tasks` are.
  The stream accept loop does not await a request read, so one incomplete
  request does not head-of-line block attributable refusals from other CIDs.
- **Pool saturation:** while the refusal-reader count equals the bound, the
  stream accept loop stops calling `accept`; it creates no task and consumes
  no userspace fd for another child. Established children stay in the existing
  AF_VSOCK accept queue. Production requests `listen(i32::MAX)`; Linux clamps
  it to the appliance network namespace's `net.core.somaxconn` and stores that
  effective value as the vsock backlog (`net/socket.c:1918-1924`,
  `net/vmw_vsock/af_vsock.c:1920-1941 @ v7.0`). Call that existing environment
  input **B**. The decision configures no new backlog and adds zero backlog
  slots; at saturation it can keep at most the existing B connected children
  queued. When those B slots are full, Linux sends RST for a new request
  (`virtio_transport_common.c:1544-1546 @ v7.0`), so the guest's `connect`
  fails and no control refusal exists because the owner never accepted or read
  a flow ID. B is not assumed to equal a spike host's value and no benchmark
  lane or new sysctl contract is introduced.

### Lossless saturation rearm

The bounded accept loop must implement this observable liveness/atomicity
contract; the private notification primitive remains the crafter's HOW:

1. reader-count check and refusal-task insertion use the same owner-state lock,
   so the count never exceeds 65,536;
2. when full, the loop registers interest in reader completion **before** its
   final locked recheck, then waits only if the recheck is still full;
3. every state mutation that removes refusal-reader entries — ordinary
   `finish_flow_task`, bulk take by quiescence/teardown, or root cancellation
   cleanup — publishes capacity after the removal; spurious wakes are allowed;
4. the wait selects root shutdown with priority over capacity, and otherwise
   loops to the locked predicate before calling `accept`; and
5. therefore no completion between “full” and wait is lost, and if any reader
   completes/cancels while root shutdown has not won, the stream accept loop
   eventually calls `accept` again.

This reuses existing task completion and cancellation/drain sites. It requires
only a private wake/recheck primitive; it adds no userspace socket queue,
public observation field, or recovery subsystem.

The global bound prevents unbounded tasks and descriptors. It does not claim a
new per-allocation isolation guarantee: while forwarding is globally
quiesced, one guest can occupy the node-wide refusal-reader pool until those
reads complete or time out. That exposure and the additional fd/task budget
are accepted costs of ADMISSIONREFUSAL1 under the user's exact approval. They
must not be hidden as private implementation.

The nearest isolation alternative is a per-CID cap of
`GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION` (4,096) combined with the same global
cap. Its worst-case global task/fd cost is unchanged, but one CID could not
fill the pool. Once that CID cap is full, however, the owner must either close
later accepted sockets without the required typed refusal, block the single
FIFO accept loop behind that CID, or add per-CID queues/scheduling. No accepted
outcome requires a new refusal-fairness policy, and the first two alternatives
weaken response or cross-CID progress. ADMISSIONREFUSAL1 therefore selects
the global-only cap and records single-guest global occupancy for explicit
approval; it does not grow a per-CID quota/queue/fairness subsystem merely to
make the model pass.

### Linearization and admitted-state ownership

Under the same owner lock that serializes quiescence and per-CID lifecycle,
accept observes exactly one of these states:

1. unknown CID, no live session, or Retiring: close unattributed immediately;
   create no reader and send no message;
2. a typed admission failure, in existing priority `NotActive`, `Quiesced`,
   then `Capacity`: snapshot the CID, launch, exact live session identity and
   failure, insert only the bounded refusal reader in `flow_tasks`, and never
   enter `guest_flows`/`datagrams` or take a flow place; or
3. no admission failure: perform the existing atomic registration/place
   acquisition and run the ordinary admitted task.

For the reproduced order, if accept/admit wins before the first hold, the
first hold finds and aborts the registered flow. If the first hold wins, the
connection takes branch 2 with snapshotted `Quiesced`; it never becomes an
admitted flow. Thus `QuiescedHoldsNoFlow` remains true at every owner step.

### Exact refusal-reader lifecycle

The existing `flow_tasks` ownership supplies every cancellation path:

| Later event | Required reader/socket outcome |
|---|---|
| Stable owner/CID/session through a complete read | Apply the response table below once, then close; `finish_flow_task` removes the handle and wakes accept backpressure. |
| First quiescence hold for a reader created before that hold | The existing quiesce drain takes the task, cancels and awaits it; close silently. A reader whose cause is `Quiesced` is created only after the latch and is allowed to finish. |
| Additional quiescence hold | No new lifecycle effect; the already-latched state remains, and refusal readers continue under their deadline. |
| Restore | Does not change a snapshotted refusal; a matching current session still receives it. The connection is never admitted after restore. |
| Teardown/Retiring | Existing teardown takes the CID's `flow_tasks`, cancels and awaits them before returning; close silently and send nothing after teardown. |
| Control-session loss/replacement | Existing session-loss cancellation cancels every CID `flow_task`; the reader's biased cancellation branch closes silently and cannot redirect to a replacement session. |
| Owner/root shutdown | The reader is a child of the existing `OwnerTasks.shutdown`; root cancellation closes it. Its join handle remains owned in `flow_tasks` until task exit, and serve/runtime termination cannot retain the socket beyond the owner runtime. |
| Request deadline, EOF, or read error | Close silently, remove the task, and send no response. |

Every cancel/read race selects cancellation first. A response is attempted
only after a complete request and a final check that the snapshotted launch
and exact session identity are still the CID's current launch/session. A
failed session send is not retried and is never redirected; the socket closes.

### Complete typed response precedence (F2)

Decode classification precedes the snapshotted admission refusal, as current
production does; attribution precedes every response. The complete table is:

| Reader outcome | Attribution/current-session check | Wire outcome | Socket/state outcome |
|---|---|---|---|
| Cancelled; incomplete at EOF; read error; request deadline | Not applicable | None | Close; no admitted state/place |
| Complete request with token absent/invalid or unequal to snapshotted launch | Fails | None (`StaleLaunch`) | Close; no admitted state/place |
| Complete request, `UnsupportedVersion`, extractable flow/token | Token matches and the exact snapshotted session is still current | `Refused { flow, Unsupported }` | One attempt, then close |
| Complete request, any other `FlowWireError`, extractable flow/token | Same | `Refused { flow, Malformed }` | One attempt, then close |
| Valid request; snapshotted cause `NotActive` | Same | `Refused { flow, NotActive }` | One attempt, then close |
| Valid request; snapshotted cause `Quiesced` | Same | `Refused { flow, Quiesced }` | One attempt, then close |
| Valid request; snapshotted cause `Capacity` | Same | `Refused { flow, Capacity }` | One attempt, then close |
| Any complete row above after CID removal, Retiring, launch change, or session loss/replacement | Fails final check | None | Close; never send across lifecycle/launch/session |
| No snapshotted failure | Not a refusal reader | Ordinary admitted branch | Existing registration, request read and pairing path |

For a decode error, the existing fixed layout supplies the raw flow field and
launch bytes. If they cannot form a valid attributable current-launch request,
the connection closes silently. Malformed/Unsupported therefore cannot be
masked by Quiesced/NotActive/Capacity, but neither can an unauthenticated
request elicit a control response. Later restore/activation/capacity release
does not change a valid snapshotted refusal; later teardown/session/launch
change suppresses the response for attribution safety.

### Exact formal and conformance action meaning

No new Quint state or action is required. `GuestOpenFlow(f)` already means
that the guest connected with its complete, valid, current-launch request
buffered, so malformed/incomplete/overload cases remain production-boundary
semantics rather than new formal traces. Existing `HostAcceptFlow(f)` has:

1. **Admitted branch:** when `admissible`, release accept only and transition
   `queued -> accepted`; existing `HostReadReqN` later releases the request.
2. **Quiesced refusal branch:** release accept and the already-buffered request
   as one formal owner action, require the production owner itself to emit
   `Refused { flow, Quiesced }`, close, and transition `queued -> closed`.

The existing named run remains unchanged. Its
`GuestOpenFlow(2) -> HostAcceptFlow(2)` suffix ends closed, while the handler
observes the real refusal through the existing sim seam. It must not inject a
message, shadow the result, or accept socket close alone. No formal
run/action/state/filter is added. Resource-bound and malformed-precedence
validation are part of the accepted ADMISSIONREFUSAL1 decision; DISTILL must
specify them without expanding the approved mechanism or scenario inventory.

## Reproduced activation priority — seed `0xf23a4188`

The requested first divergent trace is present in
`.context/netns-density-295/12-01/owner-flows-f23a4188-verbose1.log`. Its
relevant state is a Provisioned CID with `quiesced = true` and no live control
session:

```text
Provision -> VmStart -> Quiesce
  -> ActivateQuiesced
  -> ActivatePending
```

`ActivateQuiesced` agrees on `QuiescenceLatched`. The next
`ActivatePending`, without any intervening state change, makes the model expect
`ControlSessionPending`, while the real owner returns `QuiescenceLatched`
(raw lines 1030-1179). Production checks the global admission latch before
the Active/session branches (`crates/overdrive-control-plane/src/guest_network.rs:919-958`).

The exact existing-outcome priority for a valid Provisioned CID is therefore:

1. if forwarding is quiesced: `QuiescenceLatched`;
2. otherwise, if its control session is absent: `ControlSessionPending`;
3. otherwise: `Activated`.

The formal correction is only enabledness:

```quint
ActivateQuiesced: alloc == "Provisioned" and quiesced
ActivatePending:  alloc == "Provisioned" and not(quiesced) and not(sessLive)
```

Both actions retain their current no-state-change effect. `Activate`,
`ActivateRetiring`, their public variants, retry budget, and caller behavior
remain unchanged. The priority does not weaken either typed outcome; it makes
the overlap deterministic and matches the real call instead of asking the
driver to shadow the action result. No new named run is required: exact seed
`0xf23a4188` is the reproduced overlap trace, while the existing named runs
retain each non-overlap outcome.

## Rejected alternatives

| Alternative | Reason rejected |
|---|---|
| Close a quiesced connection immediately at accept | Loses the accepted `Refused { Quiesced }` outcome because the flow ID is in the unread request. |
| Register it, then wait for/read the request and refuse | Makes a quiesced CID observably hold a registered flow and place; violates QHOLD/U-1 under a delayed or partial request. |
| Treat `accepted` as an unadmitted phase and weaken `QuiescedHoldsNoFlow` | Relaxes an accepted `must`/empty-state invariant and changes quota ownership. No approval permits that weakening. |
| Send `Refused` before validating the launch token | Can send a current session a response for a stale launch, violating ATTR-TOKEN/ATTR-LIVE-CLAIM. |
| Add a public refusal port, wire field, owner observation phase, or Quint action | Existing accept/read/session and `Refused` surfaces already carry the exact outcome. |
| Make the conformance driver inject `Refused` or ignore the missing message | Tests a driver-created consequence instead of the production owner. |
| Leave refusal tasks outside `flow_tasks` | Provides no per-CID cancellation/join owner and permits sockets to outlive teardown/session loss. |
| Reuse `flow_tasks` without a bound or fd-budget term | A deadline bounds each task's duration, not concurrent count; arrival rate can exhaust tasks/descriptors. |
| Await every refused request inline in the stream accept loop | Bounds concurrency at one but head-of-line blocks every CID for up to `FLOW_REQUEST_DEADLINE`, including below resource saturation. |
| Add a second refusal-task map/queue/supervisor | Duplicates the existing `flow_tasks` owner and kernel accept queue; the required ownership and backpressure fit those existing mechanisms. |
| Add a per-CID 4,096-reader cap beside the global cap | Same worst-case node fd/task budget, but saturation requires silent close, global FIFO head-of-line blocking, or new per-CID queues/scheduling. No accepted outcome requires that new fairness policy; the global-only exposure is explicit in the approved user decision. |
| Hide malformed/unsupported requests behind the snapshotted admission cause | Changes the existing typed security/error precedence and lets quiescence mask malformed traffic. |
| Leave both activation results enabled in the overlap | The real `activate` call has one result; randomized conformance must not choose an unplayable alternative. |
| Change production to prefer `ControlSessionPending` or normalize both outcomes | Contradicts the reproduced caller path and erases an existing typed result without an accepted reason. |

## Approval and downstream gates

The user approved these two separate decisions together through the quoted
authority; neither decision's scope is broadened by approval of the other:

### ADMISSIONREFUSAL1 — approved bounded refusal reader

The existing `flow_tasks` owner applies to non-admitted refusal readers, with the
node-wide bound `GUEST_FLOW_MAX_PAIRS`, kernel-accept-queue backpressure at the
bound with the lossless rearm invariant, the exact accepted-baseline to
post-decision fd-budget change **311,296 -> 376,832** (while current source's
65,536 check is separately corrected to the accepted baseline), the
lifecycle/response table above, and decode priority
`Unsupported/Malformed` before the snapshotted `NotActive/Quiesced/Capacity`
for an attributable current-launch request. This preserves exact QHOLD/U-1,
the accepted Quiesced refusal, and launch/session security. Runtime cost is up
to 65,536 extra short-lived Tokio tasks, accepted sockets, cancellation
tokens, and 32-byte buffers for at most `FLOW_REQUEST_DEADLINE`; one guest can
occupy that global pool during quiescence. At pool saturation userspace stops
accepting and the existing effective AF_VSOCK backlog B (the listener's
`somaxconn` clamp, unchanged by this decision) supplies at most B queued
children; backlog overflow resets new connects with no control reply because
no flow ID was read. The rejected per-CID-plus-global alternative has the same
node fd/task maximum but requires a new fairness/degraded-response choice.

Validation cost after approval: extend the existing R5-26/admission source
cell and private owner tests, without a new scenario id or public seam, to
prove (a) stable Quiesced emits the exact refusal with zero admitted
flow/place; (b) malformed/unsupported/stale priority; (c) cancellation under
teardown/session loss/root shutdown; and (d) the pure private reader-count
boundary/backpressure decision. Exact seed `0xafd56990` remains the formal
replay.

### ACTPRIORITY1 — approved activation overlap priority

For a valid Provisioned CID, quiescence precedes missing-session:
`QuiescenceLatched`, else `ControlSessionPending`, else `Activated`.
`ActivatePending` gains only `not(quiesced)`. Runtime/API/wire/check-inventory
cost is zero; exact seed `0xf23a4188` and the existing named activation runs
are the evidence.

Both are accepted material internal action/resource contracts. Post-approval
fold review must verify exact correspondence before implementation dispatch.

The bounded implementation/alignment surface is:

- the private production accept/refusal path needed to keep refused sockets
  outside admitted owner state while emitting the existing typed response,
  reusing `flow_tasks`, plus its bounded backpressure and fd-budget formula;
- `specs/quint/guest-flow-owner/owner_flows.qnt`: comments/action mapping,
  `ActivatePending`'s non-quiesced guard, and no new action/state/run;
- `specs/quint/guest-flow-owner/README.md` and the feature delta: the exact
  admission/refusal and activation-priority clauses; and
- the existing private `owner_flows` conformance handler: drive and verify the
  real refusal side effect without a driver-owned result.

No public API or wire change is permitted. Any model input change requires
exact replay of seeds `0xafd56990` and `0xf23a4188`, every unchanged named and
randomized `owner_flows` target, the complete guest-flow-owner re-record with
exactly three jobs, `verify-evidence`, and independent proof/source review.
Roadmap validation remains pending until every STEP 12-01 correction and
evidence gate is complete.
