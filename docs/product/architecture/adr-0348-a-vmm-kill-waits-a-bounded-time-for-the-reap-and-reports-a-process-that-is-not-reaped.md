# ADR-0348 — A VMM kill waits a bounded time for the reap and reports a process that is not reaped

## Status

**Accepted** 2026-11-23 (decision RB-1 of the node-local persistent volume feature, GH #348; the user's ruling of 2026-11-23, decision 11,
option a). Not implemented, so the text is in the present tense. The interface contract is pinned in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "A VMM kill's reap wait is bounded"; this ADR holds only
the decision.

## Context

`Vmm::terminate` sends SIGKILL to the hypervisor process and then waits for the process to be reaped, with no bound. A process whose
thread is blocked in uninterruptible I/O is never reaped: the leader is a zombie and the sibling threads sit in D-state until the I/O
completes. SIGKILL is already pending on every thread of the group (`complete_signal`, `kernel/signal.c:1003-1025 @ v7.0`), but a thread in
an uninterruptible wait does not act on it until the wait ends. The case is reachable on Overdrive without any foreign software: the guest's
flush is held on a failing disk, or on a device that is suspended or draining (the volume feature's held-claim case, S-VOL-T3-12).

The unbounded wait propagates up. `VmDriver::stop` never returns; the action shim's `StopAllocation` arm never reaches its terminal row
write; the allocation's row stays `Running` and the reconciler's lane for that workload stays held (ADR-0102), so nothing for that workload is
ever evaluated again. A failing disk in production reaches the same state. This predates the volume feature, which only makes the
draining-writer case reachable on a healthy node.

## Decision

**After SIGKILL, `Vmm::terminate` waits for the reap for at most a fixed bound. A process not reaped by then is reported as a typed
"killed, not yet reaped" outcome, and the allocation ends there.** `VmDriver::stop` treats that outcome as a completed stop: it cleans up
what it can and returns, and the shim writes the terminal row as for any stop. The adapter's background reaper keeps waiting for the
process. What cannot be removed while the stuck threads exist (the cgroup scope) is left to the existing reclamation, which converges
at boot and on every resync pass. The volume claim stays held until the I/O drains, and a replacement waits for it as ADR-0317 already
states.

The terminal state is truthful because a killed VMM can never run guest code again: the fatal signal is pending on every thread, and a thread
leaves its uninterruptible wait only to exit.

**The guest-address lease is held until the VMM is reaped.** The stuck process still holds `/dev/vhost-vsock` with its guest CID, and the CID is
`GUEST_CID_BASE` plus the lease offset (ADR-0156). Freeing the offset at the stop would let next-fit hand the CID to a new launch, which the
kernel refuses with `EADDRINUSE`. So a stop of an unreaped VMM retires the lease and does not release it: a Retiring lease keeps its offset
held and next-fit skips it. Release has one added precondition, observed rather than inferred from the stop's outcome: the allocation's cgroup
scope is absent, and absence is a proof only because every removal of a VM scope first waits for the scope to report empty (`populated 0`,
ADR-0349). A scope directory can be removed under a live, stuck task on the running kernel, so absence alone is not trusted; absence after
the populated-first removal means every task of the VMM has exited and dropped its descriptors. The release happens on the
existing network-reclaim path, which retries a finished allocation that still holds a lease until it is released. The lease costs one
attachment slot until the reap, the same cost this decision already accepts for the scope.

(The lease clause is approved: user ruling 2026-11-23, decision 12 of GH #348, option a: the lease is released only once the allocation's
cgroup scope is observed absent; while the scope is present the lease is retired and held, and the stop still completes. It works out
decision 11 and changes no other decision. User ruling 2026-11-23, decision 16: "absent" means removed after the scope reported empty; the
removal order is ADR-0349.)

## Alternatives considered

- **b. Return an error from `terminate` and leave the row `Running`.** Rejected by the user. A `Running` row for a VM that can never run
  again is false, and it keeps the workload's lane held, so the workload stays unmanageable. A typed error also conflates "the kernel
  refused the kill" (`VmmError`) with "the kill is done and the reap is slow".
- **c. Change only the test, leaving the unbounded wait.** Rejected by the user. A failing disk in production reaches the same defect, so
  the test would hide a reachable failure.
- **Drop the `terminate` future at the bound, from the driver.** Rejected: the outcome belongs to the port, so every caller (stop,
  start-failure cleanup, boot teardown) gets it, and a driver-side timeout cannot tell the caller the process is still alive.
- **Release the lease at the stop, as for a reaped VMM.** Rejected: a later next-fit wrap onto that offset launches a VMM whose CID the stuck
  process still holds; the launch fails with `EADDRINUSE` and is charged as a start failure. ADR-0156's "no production path produces that
  state" would be false.
- **Skip held CIDs in next-fit by a separate set, or have `VmReclamation` own the release.** Rejected: a second record of the same fact drifts
  from the lease, and `VmReclamation` observes host artifacts, not leases, so a lease whose scope was already gone would never be visited. The
  network-reclaim path already converges "finished allocation that holds a lease".
- **Never wait for the reap.** Rejected: the common case reaps in milliseconds, and an immediate return would leave the scope, run
  directory and clone to the reclamation sweep on every stop for no gain.

## Consequences

- **State meaning.** `Terminated`, for an allocation ended by a stop, previously implied that the hypervisor process had been reaped. It now
  means that the VMM was killed and can run no guest code; its reap may still be pending. This is the one state-meaning change, and it is
  inside the user's ruling.
- A stop of a VMM that is not reaped takes the existing grace window plus the bound, then completes. The lane is released by the end of that
  time instead of never.
- The stuck process keeps its descriptors until it is reaped, including the volume's kernel claim and the VMM's vsock device. Both are
  released by the kernel at exit; neither is released by Overdrive. The guest-address lease that names the device's CID is held, Retiring,
  for the same time, and is released only after the scope is gone.
- A Retiring lease of an unreaped VMM counts against the attachment cap until the reap. If the process never reaps, the slot stays consumed
  until the node reboots (the lease is process-local and not persisted, ADR-0118). The cap is node-wide, so enough unreaped VMMs refuse every
  new launch and every restart's successor on the node (`AdmissionCapReached`) until they reap. This is the same class of cost as the scope
  the decision already accepts, and it is accepted with decision 12.
- Release latency after a late reap is the sum of two existing periods: the `VmReclamation` pass that removes the scope, then the next
  network-reclaim attempt (its backoff). Neither is on a hot path.
- Across a `serve` restart no lease exists: the pool is empty. Boot reclamation removes every unsupervised VMM's scope it can within the
  bound and reserves the guest-address offset of each one that is still stuck, before the first assign (ADR-0350, ADR-0156).
- Once the successor row exists, the predecessor's lease is eligible for network reclaim: the restart path does not mark the predecessor
  handled, and the network-reclaim retry has a constant backoff and no attempt ceiling, so a lease held for an unreaped VMM is retried until
  the scope is gone. Terminal rows stay in `VmReclamation`'s managed targets, so the resync pass keeps revisiting them.
- The scope removal that releases the lease is bounded by [ADR-0349](adr-0349-a-scope-kill-waits-a-bounded-time-for-the-scope-to-settle-and-reports-a-scope-that-is-busy.md).
- Leftovers (the scope) persist until the process is reaped; `VmReclamation` retries each pass. A stuck process that is never reaped leaves
  one scope per such allocation until the node reboots. There is no operator to remove it, and none is needed for correctness.
- The bound is a policy value, revisited if a production `KilledNotReaped` is observed for a healthy disk.
- `Vmm` gains one outcome variant and a bound constant; every exhaustive match on the outcome gains one arm.
- The `node-local-volume` Quint model needs no change. The `guest-flow-owner` model (`cid_lease.qnt`) gains the unreaped stop, the late
  release and the scope's removal after it is empty (feature-delta, same section, item 11).
- ADR-0156 is amended: the lease is released after the VMM is reaped, not merely after the stop.
- After a `KilledNotReaped` stop `VmDriver::stop` does not remove the scope (it is known to be populated); `VmReclamation` removes it after
  the reap. On a reaped stop, and in the start-failure cleanup, the driver removes the scope through `kill_scope`, the populated-first
  removal of ADR-0349 (user ruling 2026-11-23, N-1, option a), so `kill_scope` is the only remover of a VM scope. A `ResourceBusy` from it
  does not change the stop: it completes, and the scope is left to `VmReclamation`.
