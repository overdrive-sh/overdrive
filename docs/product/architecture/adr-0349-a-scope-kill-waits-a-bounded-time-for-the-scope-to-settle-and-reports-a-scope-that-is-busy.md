# ADR-0349 — A scope kill waits a bounded time for the scope to settle and reports a scope that is busy

## Status

**Accepted** 2026-11-23 (decision RB-3 of the node-local persistent volume feature, GH #348; user rulings of 2026-11-23, decision 13 option a, and
decision 16). Not implemented. The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "A scope kill's settle wait is bounded". The evidence is
[`spike/findings-cgroup-events.md`](../../feature/node-local-persistent-volume/spike/findings-cgroup-events.md).

## Context

`VmHostState::kill_scope` writes `cgroup.kill` and then removes the allocation's cgroup scope, and its port contract says it does not return until
the removal has settled. A scope whose VMM is killed but not reaped (ADR-0348) cannot settle: the VMM's last task is stuck in exit, and it still holds
the device and the guest CID.

Two facts about the running kernel shape the decision (guest kernel `7.0.0-38-generic`, spike of 2026-11-23):

- `cgroup.events` reports `populated 1` for the whole time the stuck task exists, and `populated 0` only after it is gone (0/20 early, 0/20 missed).
  `rmdir` of a scope after `populated 0` never blocked and never failed (60/60).
- `rmdir` of a scope whose last task is stuck in exit succeeded in about 1 ms (20/20): the directory was removed while the task was alive and still
  held the device. On the vendored `v7.0` source the same `rmdir` is refused with `-EBUSY` or sleeps (`cgroup_drain_dying`,
  `kernel/cgroup/cgroup.c:6254-6297 @ v7.0`). The two readings differ, and the design depends on neither.

So "the scope directory is absent" does not mean "no task of the scope is left" unless the removal first saw the scope empty.

## Decision

**`kill_scope` writes `cgroup.kill`, waits for the scope to report empty (`populated 0` in `cgroup.events`), and only then removes the directory. It
never removes a scope that reports `populated 1`.** The wait is bounded by a fixed bound measured over the whole call. A scope that is not empty by
then returns the existing `io::ErrorKind::ResourceBusy`: the kill is delivered, the scope is left in place, and no removal was attempted. The port
keeps its `io::Result` return, so no new error type is added. `ResourceBusy` is the one error kind that means "retry later"; every other kind is a
refusal and is returned at once.

Because the removal is entered only when the scope is empty, it neither sleeps (the `v7.0` reading) nor succeeds early under a live task (the
observed reading). `populated` is cleared in the same section that ends `cgroup_drain_dying`'s wait (`do_cgroup_task_dead`,
`kernel/cgroup/cgroup.c:7072-7102 @ v7.0`). The wait is a read of a file and holds no thread in the kernel.

A busy return takes the full bound, never less: the convergence loop re-submits a target whose tick emitted an action immediately, and the busy wait
is the only pacing on that retry.

Callers treat busy as "not settled, try again later" and take no further step on that scope. The exceptions and each caller's handling are in the
feature delta; the boot executor step is [ADR-0354](adr-0354-a-busy-boot-scope-writes-its-terminal-row.md). That a removed VM scope proves an empty
scope depends on this wait being the only removal: [ADR-0351](adr-0351-a-vm-scope-has-one-remover-the-populated-first-scope-kill.md).

## Alternatives considered

- **Leave the wait unbounded.** Rejected by the user (decision 13). The stall is reachable on a healthy node by the volume feature's held-flush case
  and on a failing disk.
- **Retry `rmdir` up to a count or a deadline.** Rejected (decision 16): on the running kernel a retried `rmdir` succeeds under a live task, so the
  call returns `Ok` while the VMM holds the device. It also needs a blocking `rmdir` on the blocking pool under a deadline, with an in-flight claim per
  scope. Waiting on `populated` first removes all of it.
- **Bound only the retry count, as today.** Rejected: it does not bound a `rmdir` that blocks inside the kernel, and it removes a scope under a live
  task.
- **A new typed busy outcome (an error enum or an `Ok` variant).** Rejected: `ResourceBusy` already names the kernel's own answer, the port is
  `io::Result` for every method, and no caller needs more than the kind.
- **Return busy at once when the scope is not empty.** Rejected: the busy return becomes instantaneous, and with the immediate requeue the `node/<id>`
  target spins.
- **A pacing field in `VmReclamation`'s View.** Rejected: it adds state to defend a retry that the bounded wait already paces, and a View field never
  decides whether the scope is gone (`reconcilers.md` § "A View decides when to act").

## Consequences

- A stuck scope costs the `VmReclamation` target one bound of waiting per tick, repeated until the process is reaped; actions for other allocations
  in the same tick are delayed by up to that bound for each stuck scope.
- A stuck scope costs one open file and one waiter for the length of the call, and no blocked thread.
- The bound is a policy value, revisited if a busy return is observed for a healthy disk.
- The kernel facts rest on a distro build of the 7.0 series, not on the vendored tag. The decision holds for either behaviour of `rmdir`.
- The `guest-flow-owner` model changes: the scope becomes a state of its own, removed only when empty (feature delta, § "Quint changes").
