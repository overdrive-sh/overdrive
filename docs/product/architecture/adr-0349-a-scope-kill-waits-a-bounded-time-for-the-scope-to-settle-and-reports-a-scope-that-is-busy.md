# ADR-0349 — A scope kill waits a bounded time for the scope to settle and reports a scope that is busy

## Status

**Accepted** 2026-11-23 (decision RB-3 of the node-local persistent volume feature, GH #348; the user's ruling of 2026-11-23, decision 13,
option a). Not implemented, so the text is in the present tense. The interface contract is pinned in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "A scope kill's settle wait is bounded".

## Context

`VmHostState::kill_scope` writes `cgroup.kill` and then removes the allocation's cgroup scope, and its port contract says it does not return
until the removal has settled. A scope whose VMM is killed but not reaped (ADR-0348) cannot settle: the kernel refuses `rmdir` of a cgroup that
still has a visible task with `-EBUSY` (`cgroup_drain_dying`, `kernel/cgroup/cgroup.c:6254-6272 @ v7.0`), and when every remaining task is
already exiting, `rmdir` itself sleeps uninterruptibly until the last of them passes `cgroup_task_dead` (`:6274-6297 @ v7.0`). The second case is
the held-flush case: a thread that has begun to exit and is held in the close of a suspended device.

The host adapter already bounds its retry on `EBUSY` by an attempt count, but each attempt is a blocking `rmdir` on the runtime's blocking pool.
An attempt in the second case never returns, so the count never advances and `kill_scope` never returns. Two callers stall on it. The
`VmReclamation` executors run inside the convergence tick for the node's single `node/<id>` target, so one stuck scope stops reclamation for
every VM on the node. Boot reclamation (`vm_reclamation_boot::converge`) never finishes, so `serve` never boots after a crash that left an
unreaped VMM. A retry that gives up also returns the raw last error, so a caller cannot tell "the scope is not settled yet" from "the substrate
refuses".

## Decision

**`kill_scope` waits for the scope to settle for at most a fixed bound, measured over the whole call. A scope that has not settled by then
returns the existing `io::ErrorKind::ResourceBusy`: the kill is delivered and the scope is not yet removed.** The port keeps its `io::Result`
return, so no new error type is added. `ResourceBusy` is the one error kind that means "retry later"; every other kind is a refusal and is
returned at once, not retried.

The bound covers a blocked `rmdir`, not only a refused one: the adapter does not let a kernel sleep in `rmdir` hold the call. A removal still
in flight when the bound passes finishes by itself when the process is reaped, and the adapter starts at most one removal per scope at a time,
so a stuck scope costs one blocked thread, not one per call.

A busy return takes the full bound, never less. The convergence loop re-submits a target whose tick emitted an action immediately, and the busy
wait is the only pacing on that retry.

Callers treat busy as "not settled, try again later" and take no further step on that scope (no artifact discard, no row write):

- **`VmReclamation`** (the steady-state tick) returns the error as it does for any substrate failure; the target is re-evaluated by the
  existing requeue and the 30 s resync pass. Other allocations' actions in the same tick are still dispatched.
- **Boot reclamation** keeps its contract: it is fail-closed, and a busy scope refuses the boot with the typed executor error. What changes is
  that the refusal now arrives after the bound instead of never.
- **Guest-address lease release** (ADR-0348, ADR-0156) is gated on the scope being observed absent, so a busy scope keeps its lease held.

## Alternatives considered

- **Leave the wait unbounded.** Rejected by the user (decision 13). The stall is reachable on a healthy node by the volume feature's held-flush
  case and on a failing disk.
- **Bound only the retry count, as today.** Rejected: it does not bound a `rmdir` that blocks inside the kernel, which is the case that
  matters.
- **A new typed busy outcome (an error enum or an `Ok` variant).** Rejected: `ResourceBusy` already names the kernel's own answer, the port is
  `io::Result` for every method, and no caller needs more than the kind.
- **Return busy at once when a removal is in flight.** Rejected: it makes the busy return instantaneous, and with the immediate requeue the
  `node/<id>` target spins.
- **A pacing field in `VmReclamation`'s View.** Rejected: it adds state to defend a retry that the bounded wait already paces, and a View field
  never decides whether the scope is gone (`reconcilers.md` § "A View decides when to act").
- **Let boot proceed past a busy scope.** Not adopted: after a restart no lease exists (ADR-0118), so a launch could be handed the CID that
  the unreaped VMM still holds and fail `EADDRINUSE`. ADR-0156's guarantee that reclamation completes before the first assign depends on it.

## Consequences

- A stuck scope costs the `VmReclamation` target one bound of waiting per tick, repeated until the process is reaped. Actions for other
  allocations in the same tick are delayed by up to that bound for each stuck scope.
- A boot after a crash that left an unreaped VMM is refused after the bound, and the supervisor's restart retries it. The node boots once the
  process is reaped. This is the existing fail-closed contract, now terminating.
- The adapter holds one blocked thread per stuck scope until the reap.
- The bound is a policy value, revisited if a busy return is observed for a healthy disk.
- `VmHostState::kill_scope`'s rustdoc is rewritten; the signature is unchanged. Test doubles of the port that wait for a kill (the
  boot-reclamation test double) gain the busy case.
- The `guest-flow-owner` model needs no new action: a refused boot is the model's `Reclaim` not being enabled while an orphan is unreaped
  (feature-delta, item 11).
