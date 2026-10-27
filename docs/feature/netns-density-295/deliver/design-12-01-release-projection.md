# STEP 12-01 approved DESIGN contract — released lease versus retained control-reader cleanup

## Status and decision boundary

**Status: RELEASEPROJ1 APPROVED by the user 2026-10-26; bounded conformance
alignment and replacement evidence remain pending.** Independent DESIGN/formal
review found zero open technical findings. Exact authority: “Approve
RELEASEPROJ1”. The decision changes the accepted QPROJ-3 meaning of the
existing `owner_flows.alloc` projection and clarifies the meaning of an
existing `CidPhase::Retiring` observation. It adds no product behavior, public
API shape, wire value, runtime owner, formal state, action, run, scenario,
hazard, or check.

Roadmap validation remains `pending`. Approval authorizes only the exact
private conformance projection and documentation alignment below; it does not
approve replacement evidence or roadmap validation.

## Reproduced contradiction

Metal run `b7880b3e-1cb9-48f6-9e25-fee60bcaa742` replayed
`flows_conf_randomized_traces` with `QUINT_SEED=0xf23a4188`. Trace 44 is:

```text
InitWith("g")
  -> Provision
  -> Quiesce
  -> ActivateQuiesced
  -> VmStart
  -> Restore
  -> DialNotServing
  -> GuestOpenSession(1)
  -> VmStop
  -> HostAcceptSession(1)
  -> DialNotServing
  -> ActivatePending
  -> DialNotServing
  -> ActivatePending
  -> ActivatePending
  -> DialNotServing
  -> Teardown
  -> Quiesce
  -> Release
```

The raw log is
`.context/netns-density-295/12-01/owner-flows-f23a4188-verbose1-final-current.log`.
Trace 44 starts at line 33086; the accepted-unread session is still `reading`
after `Teardown` and `Quiesce` at lines 33676-33745; `Release` sets the model
allocation to `Absent` while preserving that `reading` session at lines
33748-33781. The state comparison at lines 33783-33808 differs only in the
combined allocation projection:

```text
model:          alloc = Absent,   lease_gen = None
implementation: alloc = Retiring, lease_gen = Some(1)
```

`quiesced`, the session states, claims, flows, dials, and typed results all
match. This is not an action-order, refusal, socket, or cleanup difference.

## Complete production path

The mismatch is produced by the normal release owner path:

1. `retire_guest_network_lease` changes the pool lease from `Admitted` to
   `Retiring` before the VMM is stopped
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:1719-1731`).
2. The terminal dispatch stops and releases VMM supervision, stops the mTLS
   allocation, then enters `teardown_for_dispatch`
   (`action_shim/mod.rs:1614-1647`).
3. `teardown_guest_network` awaits the existing
   `GuestNetworkProvisioner::teardown`; only after `Ok` does it call
   `GuestAddressPool::release` (`action_shim/mod.rs:1587-1602`). This is the
   production entry point for the formal `Release` action.
4. An accepted-unread control connection is not a claimed control session.
   Teardown marks the CID `Retiring`, clears any claim, drains every serving
   flow/dial/datagram, but deliberately retains a control task whose key is in
   `unread` (`guest_network.rs:985-1044`). Its final residue check excludes
   exactly those accepted-unread tasks; it returns `Ok` and retains the row
   while `unread`/`control_tasks` are nonempty (`:1090-1118`).
5. `GuestAddressPool::release` removes the lease and frees its offset
   (`guest_network.rs:610-623`). `lease_observation().leases` therefore no
   longer contains the allocation. This is the pool's accepted release fact,
   not a test inference.
6. The late control read is generation-safe. The task carries its
   `accepted_launch` (`owner_runtime.rs:185-220`); after read, a launch mismatch
   closes it without a claim (`:242-300`). `finish_connection` removes only
   the task whose `owner_launch` matches and cannot clear a newer session
   unless the CID row's launch also matches (`:1477-1515`).
7. If the one-CID pool assigns a next launch before the old task finishes,
   `provision` replaces only a serving-state-empty `Retiring` row containing those
   old accepted-unread tasks, moves the task handles into the new CID row, and
   retains their old launch identities (`guest_network.rs:874-938`). The late
   finish therefore cannot claim, clear, or remove the new launch.

The owner row is consequently a process-local cleanup tombstone after pool
release. It is not a lease, does not admit a flow, cannot hold a control
session claim, and owns no flow, dial, datagram, pair, or watch.

## Accepted contracts that must all remain true

| Contract | Required outcome | Evidence |
|---|---|---|
| Release-last / pool truth | After `pool.release`, the lease is absent and its CID offset is free; a Retiring lease exists only until `release`. | Feature delta QPROJ-2 and `lease_observation` contract (`feature-delta.md:9510-9524`, `:6276-6290`); pool source above. |
| Formal `Release` | With the VMM down and allocation Retiring, existing `Release` sets `alloc = "Absent"` and preserves session state. | `owner_flows.qnt:457-465`. |
| Accepted-unread teardown | `Teardown` does not touch a control connection in `reading`; after teardown it closes at `HostReadOpen` without taking a claim. | `owner_flows.qnt:437-456`, `:1251-1260`; `TeardownBetweenAcceptAndReadClosesWithoutClaim`. |
| Owner teardown postcondition | `teardown` returns after every flow and claimed control session is absent. An accepted-unread task is not a claimed session and remains fail-closed. | `GuestNetworkProvisioner::teardown` (`guest_network.rs:134-142`) and the source path above. |
| Same-CID next launch | A dead launch's queued or accepted-unread connection may finish under the next lease of the same CID, but its old token never claims or poisons the new launch. | K-A2-prime / ATTR-LIVE-CLAIM runs `StaleLaunchSessionIsClosedAtReadAndDoesNotPoisonClaim` and `TeardownBetweenAcceptAndReadClosesWithoutClaim` (`owner_flows.qnt:1241-1260`). |
| No process survivor | No row, task, session, or lease survives a `serve` process exit. | NO-SURVIVE; feature delta decision table and owner-death contract. |
| Honest projection | Driver bookkeeping cannot replace owner/pool observations; QPROJ-2 is the pool lease arbiter and QPROJ-3 is the owner-table arbiter. | Feature delta conformance rules (`feature-delta.md:9493-9504`, `:9510-9524`, `:9566-9590`). |

Changing `Release` to leave the model allocation `Retiring` violates the first
two contracts. Cancelling, closing, or awaiting the reader in teardown
violates the accepted-unread contract. Hiding the task or deriving it from
driver action history violates the last contract.

## RELEASEPROJ1 — exact approved contract

### Allocation lifecycle projection

`owner_flows.alloc` denotes the allocation's current lease/admission lifecycle,
not the presence of process-local reader cleanup. Its projection is exactly:

1. Locate the driver allocation in the real QPROJ-2
   `GuestAddressPool::lease_observation().leases` snapshot.
2. If no lease exists, project `alloc = "Absent"` and no compared lease
   generation, even if QPROJ-3 still contains the bounded cleanup tombstone
   described below.
3. If the pool lease is `Retiring`, project `alloc = "Retiring"` and its
   launch token. The owner row may already be absent after its last task
   finished; QPROJ-2 remains authoritative for the held lease.
4. If the pool lease is `Admitted`, require QPROJ-3 to contain the same CID,
   allocation, and launch. Project only its `Provisioned` or `Active` phase
   and that same launch. An Admitted pool lease with no matching owner row, or
   with an owner `Retiring` phase, is a conformance failure.

This replaces only the current owner-row-first/fallback rule for `alloc` and
its compared generation. All other QPROJ-2 and QPROJ-3 fields keep their
existing arbiters.

### Retained cleanup tombstone

QPROJ-3 `cids[cid]` may remain `CidPhase::Retiring` after teardown returns and
after the pool lease is released only when all of these are true:

- `session`, `session_sender`, and `session_cancel` are absent;
- `guest_flows`, `flow_tasks`, `flow_ids`, `dials`, `dial_replies`,
  `dial_cancels`, `dial_requests`, and `datagrams` are empty;
- no pair or dial watch belongs to the CID;
- `control_tasks.len() == unread.len()`; and
- every retained control task has `owner_launch = Some(the tombstone launch)`.

The tombstone is not an allocation lease and takes no attachment or flow
place. It is fail-closed: new accepts cannot register against it, activation
cannot succeed, and it cannot claim a session. The existing
`sess(c).st = "reading"` projection continues to come from its `unread` set.
The final task finish removes the tombstone when no newer launch replaced it.

### Same-CID replacement and late finish

Existing `Provision` may replace that exact tombstone with the next lease of
the same CID and transfer `unread` and their `control_tasks`. Every transferred
task retains its original
`owner_launch`. Its read compares the old accepted launch and wire token with
the new row's launch, closes stale, takes no claim, and removes only its own
task/key. It cannot clear the new launch's claim or remove its row. Private
non-resource result caches are not a RELEASEPROJ1 projection input and retain
their existing implementation-owned semantics.

### Formal and conformance surface

The existing formal state and actions already express the required result:
`Teardown` preserves a `reading` connection; `Release` changes only `alloc` to
`Absent`; a later `HostReadOpen` closes the old connection; `Provision` may
start the next generation. No Quint state, action, effect, guard, run,
scenario, hazard, check, or trace filter changes.

The existing driver changes only the `alloc`/compared-generation projection
to the four-case QPROJ-2/QPROJ-3 rule above. It continues projecting
`sess(c).st = "reading"` from the real owner `unread` set and `closed` from the
existing socket arbiter. It adds no shadow state or action-derived result.

## Lifecycle Gate Ownership

Not applicable. RELEASEPROJ1 changes no admission, activation, teardown,
release, claim, or read gate. It distinguishes the existing pool lease fact
from the existing owner cleanup fact at the conformance projection boundary.

## Rejected alternatives

| Alternative | Why it loses |
|---|---|
| Keep owner-row-first `alloc = Retiring` after release | Claims a lease and compared generation that QPROJ-2 proves absent; contradicts the accepted formal `Release`. |
| Change formal `Release` to preserve `Retiring` | Weakens release-last/offset reuse and makes the model disagree with the real pool action it maps. |
| Cancel, close, or await accepted-unread tasks during teardown | Removes the accepted `reading`-after-teardown state and the late fail-closed ATTR-LIVE-CLAIM check. |
| Drop the retained task or its handle when teardown returns | Loses explicit process ownership and cleanup/join responsibility while the socket and task remain live. |
| Add an owner release callback, a second public release method, or shared pool/owner state | Adds a runtime/API/ownership mechanism only to mirror a fact the existing QPROJ-2 observation already supplies. |
| Add a separate public tombstone type or observation map | Adds public surface without changing any required result; the existing bounded Retiring row already carries the exact task ownership and launch identity. |
| Infer `reading` from the action sequence or a driver flag | Compares the model with driver bookkeeping and hides a missing production task. |
| Add formal tombstone state or a new cleanup action | Enlarges the model even though its existing `sess` and `alloc` variables already express the reproduced production outcomes exactly. |

## Authorized alignment, cost, and validation

- **Runtime/API/wire/storage cost:** zero. RELEASEPROJ1 retains current
  production effects and public type shapes.
- **Formal cost:** zero new state, transitions, checks, runs, scenarios, or
  evidence jobs.
- **Authorized implementation scope:** the private `owner_flows`
  conformance projection and precise comments/README/feature-delta wording.
  Production changes are out of scope unless review disproves one of the
  cited source facts.
- **Required evidence:** exact replay of seed `0xf23a4188` including Trace 44;
  the unchanged teardown/stale-launch named runs; every unchanged
  `owner_flows` named and randomized target; the already-required complete
  three-job guest-flow-owner record and `verify-evidence`; and independent
  source/proof review. Roadmap validation remains pending throughout.

## Approval record and downstream gates

Independent DESIGN/formal review approved RELEASEPROJ1 for presentation with
zero open technical findings. On 2026-10-26 the user explicitly said “Approve
RELEASEPROJ1”. This authorizes only this projection/state-meaning correction.
It does not authorize a new release API, runtime callback, task owner, formal
state/action, scenario, cleanup policy, or product behavior.

DISTILL may align the existing private driver and exact contract documentation.
Completion still requires exact replay of seed `0xf23a4188`, including Trace
44; every unchanged teardown, stale-launch, named and randomized `owner_flows`
target; the complete three-job guest-flow-owner record; `verify-evidence`;
and independent source/proof review. Roadmap validation stays pending until
those gates pass.
