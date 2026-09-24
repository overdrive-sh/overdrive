# ADR-0131 — Raise an allocation TAP only after its intercept is live and before guest command release

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R5. Proposed 2026-09-23 and revised 2026-09-24; reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN.
This carries forward the ordering and owner-state contract of the committed,
user-directed first revision of D-295-DELIVER-04-01. It replaces that
revision's falsified premise, that Cloud Hypervisor could attach a down TAP by
name, with ADR-0127 and ADR-0128. Exact signatures live only in the #295
feature delta.

## Context

Once ADR-0127 lets the TAP stay down through VMM start and READY, some owner
must decide when it goes up. The accepted states keep distinct meanings:

- **Guest READY:** guest platform initialization is complete and the guest is
  blocked awaiting EXEC.
- **Allocation Running:** READY plus the durable row.
- **Intercept-live:** the allocation's `2 + P` shared IP elements and exact
  generation capability are active and read back. The synchronous
  `mtls.intercept.install.success` event receipts it.
- **Guest command release:** the gate on `release_for_exit_emission`.

None of those states promises host TAP forwarding.

The shared guest-network owner already owns every TAP mutation, read-back,
runtime quiescence, recovery, and teardown. A second owner of TAP
administrative state would split one kernel object's lifecycle.

The principle that no workload traffic flows before enforcement is live and
read back is established: Cilium's CNI ADD regenerates the endpoint
synchronously, Istio's ambient ADD fails pod creation without its node agent,
and the kubelet creates the sandbox and network before any container. The
mechanism of holding the host TAP administratively down across guest boot has
no external precedent; it rests on the native spikes and on kernel semantics.
(Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 2.1, 3.1, 3.2 and 3.3.)

ADR-0124 already defines how new guest command release behaves while the
shared network is recovering: release attempts wait under the EXEC gate and
proceed only after recovery, or refuse at FailStop. It also forbids
infrastructure conditions from consuming restart budget. TAP activation sits
immediately before command release, so it must follow the same rule rather
than turn a recovery interval into an allocation failure.

## Decision

1. Provision ends with the complete guarded, classified attachment read back
   while the TAP is administratively **down**.
2. After the durable Running row, the action shim awaits `start_alloc` and emits
   the exact success event.
3. The shim then waits on the node's existing EXEC gate: while the gate is
   BootClosed or Recovering it waits, and at FailStop it withholds activation
   and command release and writes no further row.
4. With the gate open, the shim awaits the owner's allocation `activate`
   operation. That operation re-reads the attachment's protection, raises the
   TAP, and reads back up-state and bridge master.
5. Only then does the shim call the existing EXEC-release hook. The success
   event therefore precedes the only up-transition, so every possible guest
   frame is strictly later than the event.

Activation and runtime TAP quiescence serialize in private owner state:

- An activation that linearizes first is included in a later quiescence.
- An activation that observes a latched quiescence raises nothing and reports
  that outcome. It is not a failure: the shim returns to waiting on the gate
  and retries once recovery reopens it. It consumes no restart budget and
  writes no row.
- A runtime restore raises only attachments whose activation had completed,
  never a still-pending one. When recovery calls that restore is decided by the
  recovery owner (ADR-0124 and the #295 feature delta), not here.
- An allocation whose VMM the recovery owner has killed is never raised:
  activation refuses it and follows the failure path below.

Activation failure that is not a recovery condition is fail-closed and reuses
the existing failure reason. When the VMM quiesces, the shim stops the
intercept, tears down the attachment, and releases the lease last. It then
writes a dominating Failed row with the existing guest-network failure reason
and a closed activation stage. EXEC is never released on this path.

No lifecycle state changes meaning. The action shim gains the node's existing
EXEC-gate capability, used only to wait; the owner port gains the awaited
`activate` operation.

## Alternatives considered

### Refuse activation while quiescence is latched and fail the allocation

Rejected. A shared-network recovery would become an allocation `Failed` row
that consumes restart budget, contradicting ADR-0124's wait-during-Recovering
rule and ADR-0121's rule that infrastructure conditions consume no restart
budget.

### Leave the allocation with the TAP down and EXEC withheld, and activate it when recovery reopens

Rejected. The dispatch would return with neither activation nor release done,
so a new retry owner and an observable "activation pending" state would be
needed to finish both later. Waiting on the existing gate inside the dispatch
needs neither, and the wait is bounded by the supervisor's recovery window.

### Gate on carrier instead of administrative state (`IFF_NO_CARRIER` or `TUNSETCARRIER`)

Rejected. The kernel offers both (research Finding 2.1). `TUNSETCARRIER` is an
ioctl on a
queue descriptor, and the confined VMM holds that descriptor, so a carrier gate
would be under the VMM's control.
Raising administrative state needs `CAP_NET_ADMIN`, which the confined VMM
lacks; native run `f1a15668` shows exactly that `EPERM`. The spikes also show
zero frames and zero counters through READY with the TAP administratively down.

### Activate inside the intercept owner's `start_alloc`

Rejected. The intercept owner would mutate TAP state that the switch owns. That
splits TAP ownership and defeats serialization with quiescence.

### `VmDriver` activates at EXEC release

Rejected. It gives the VMM owner authority to mutate host links, for the same
split-ownership reason.

### Activate before emitting the success event

Rejected. The first frame could then precede the timestamp claimed as the
barrier.

### Keep the TAP up from provision

Rejected by the native counterexample recorded in ADR-0127.

## Consequences

Positive: the zero-frame barrier has one owner and one ordering. Quiescence and
recovery reason about an explicit per-allocation phase. A recovery interval
delays activation instead of failing the allocation. That holds for recovery
that the allocation merely observes; a VM the recovery owner kills has crashed
and follows the ordinary crash path.

Negative:

- Every start adds one netlink round trip and one read-back.
- The owner carries private phase state, which boot never persists or adopts.
- Activation failure adds a cleanup branch to the post-Running path.
- An activation arriving during recovery holds its dispatch for up to the
  recovery window.
