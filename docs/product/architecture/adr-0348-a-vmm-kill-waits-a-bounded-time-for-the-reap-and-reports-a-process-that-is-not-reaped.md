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

## Alternatives considered

- **b. Return an error from `terminate` and leave the row `Running`.** Rejected by the user. A `Running` row for a VM that can never run
  again is false, and it keeps the workload's lane held, so the workload stays unmanageable. A typed error also conflates "the kernel
  refused the kill" (`VmmError`) with "the kill is done and the reap is slow".
- **c. Change only the test, leaving the unbounded wait.** Rejected by the user. A failing disk in production reaches the same defect, so
  the test would hide a reachable failure.
- **Drop the `terminate` future at the bound, from the driver.** Rejected: the outcome belongs to the port, so every caller (stop,
  start-failure cleanup, boot teardown) gets it, and a driver-side timeout cannot tell the caller the process is still alive.
- **Never wait for the reap.** Rejected: the common case reaps in milliseconds, and an immediate return would leave the scope, run
  directory and clone to the reclamation sweep on every stop for no gain.

## Consequences

- **State meaning.** `Terminated`, for an allocation ended by a stop, previously implied that the hypervisor process had been reaped. It now
  means that the VMM was killed and can run no guest code; its reap may still be pending. This is the one state-meaning change, and it is
  inside the user's ruling.
- A stop of a VMM that is not reaped takes the existing grace window plus the bound, then completes. The lane is released by the end of that
  time instead of never.
- The stuck process keeps its descriptors until it is reaped, including the volume's kernel claim and the VMM's vsock device. Both are
  released by the kernel at exit; neither is released by Overdrive.
- Leftovers (the scope) persist until the process is reaped; `VmReclamation` retries each pass. A stuck process that is never reaped leaves
  one scope per such allocation until the node reboots. There is no operator to remove it, and none is needed for correctness.
- The bound is a policy value, revisited if a production `KilledNotReaped` is observed for a healthy disk.
- `Vmm` gains one outcome variant and a bound constant; every exhaustive match on the outcome gains one arm.
- The `node-local-volume` Quint model needs no change (feature-delta, same section).
