# ADR-0199 — Forwarder pairs live and die with the control context that attached them

## Status

**Accepted — EVMOD-CTL, approved by the user 2026-10-17.** GH #295.
Follows from ADR-0197 (accepted 2026-10-16) and the user's design input of
that ruling: "a real control interface and pair-completion event in place of
the spike ioctl and /proc polling". This ADR decides only the pairs' and
watches' lifetime. Related decisions, each separately reversible: how a pair
reports its end (ADR-0202, EVMOD-REC); what an owner's abort of a watch does
to the watched socket (ADR-0211, WATCH-ABORT); how a dialled stream's holder
learns its outcome (the `GuestStream` contract, EVMOD-STREAM-END, feature
delta). The device operations, the attach preconditions and the core port
trait are in the #295 feature delta, § *[REF] vsock data path — the
in-kernel event-driven forwarder (EVMOD)*, "Control interface", "Context
lifetime and owner failure" and "Driven port — `FlowForwarder`".

## Context

The evmod spike attached a pair with an `ioctl` on a global `/dev/evmod` and
detached it by a global id (`spike/findings-evmod.md`, caveat 7). A pair
held its own file references to both sockets, so nothing ended it when the
process that attached it died.

Three existing contracts constrain the lifetime:

- **Fail closed on owner death** (ADR-0159, D19): when the process that owns
  forwarding exits, every forwarded flow ends with a reset at its far ends.
  The module's own file references keep the sockets open after the owner's
  descriptors close, so the owner's exit does not end a pair by itself.
- **One owner per side** (ADR-0151): one guest-flow owner holds every host
  pair; `overdrive-init` holds every guest pair. Nothing else attaches.
- **Bounded in-process recovery** (ADR-0124): the host owner's `Forwarder`
  component is repaired inside the running `serve` under a quiescence hold
  whose first hold aborts every flow (ADR-0160, ADR-0169). A repair that
  opens new forwarding state while the old state still forwards would leave
  pairs with no owner.

`serve` spawns child processes (the VMMs). A descriptor without
`FD_CLOEXEC` is inherited by every child, and a file is released only at its
last close (`fs/file_table.c:528-530 @ v7.0`).

## Decision

- **A forwarder context is one open file of the module's character device**,
  opened with `O_CLOEXEC`. Every pair attached and every vsock watched
  through a context belongs to it.
- **The context's release aborts every live pair and watch of the context
  before the release returns**: TCP halves are reset, vsock halves — a
  watched `V_h` included — are shut down in both directions, the forwarder's
  callbacks are removed and its references dropped, and no end record is
  queued. A watched socket's holder keeps its descriptor. Owner death is
  therefore fail-closed without the owner acting. Measured on Linux 7.0
  (P-70, `spike/findings-ev-ctl.md` § 3.2, § 3.3, § 3.10): release by
  `close()`, `kill -9` and `exit` reset 48 of 48 far TCP ends of every run
  at release return, before the parent's `waitpid` returned, and queued no
  record; a `kill -9` with 16,384 pairs reset all of them by `waitpid`
  return; 120 kills landing inside attach, abort and abort-all ioctls left no
  unreset attached pair, no reset of a never-attached pair and no leaked
  object.
- **One context per owner, held by value.** The owner is the only holder of
  its context's descriptor; the context is never duplicated or passed to
  another process. `O_CLOEXEC` is load-bearing: without it an exec'd child
  held the context after the owner was killed and its pairs kept
  forwarding (8 of 8 far ends open, bytes still moving; with it, 0 of 8,
  P-70 § 3.11).
- **A context is replaced only after the old one is released.** The one
  in-process path that replaces a context is the host owner's `Forwarder`
  component recovery: its first hold aborts every flow; the repair then
  closes the old context (whose release aborts any pair the owner's abort
  did not reach) and only afterwards opens and probes a new one. No path
  opens a second context while the first is open.
- **Any other end of the owner is the end of its process.** The host owner
  is not recovered in-process; the supervisor's end is already a fail-stop
  of `serve` (ADR-0124), and the process exit releases the context. The guest
  owner is PID 1.

## Alternatives considered

- **Pairs that outlive the owner process** (a global device, as in the
  spike). Forwarding would continue while `serve` is down, against ADR-0159.
  Rejected.
- **Reopen a context beside the failed one during recovery.** The old pairs
  keep forwarding with no owner and their ends are never released. Rejected.
- **A context descriptor inheritable by children.** A VMM child would keep
  the context alive after `serve` died, and the pairs with it. Rejected:
  `O_CLOEXEC` is part of the contract.
- **An owner-liveness token per pair** (each pair holding a reference to an
  owner object that a watchdog revokes). Adds a second liveness mechanism
  beside the file the kernel already releases at process exit. Rejected.

## Consequences

- A `serve` crash resets every forwarded flow's far ends, as before
  (V-13, P-42); a guest PID-1 crash ends the VM and the host side aborts on
  the device reset.
- The forwarder's in-process recovery costs every live flow, which the
  first quiescence hold aborts in any case.
- After a release shuts a watched `V_h`, its holder's `read` may return 0
  or fail `ENOTCONN`: `vsock_shutdown` clears `SOCK_DONE`
  (`net/vmw_vsock/af_vsock.c:1220 @ v7.0`), and once the socket has left
  `ESTABLISHED` without it `recvmsg` returns `-ENOTCONN` (`:2522-2531`)
  (measured: 10 of 400, 4 of 400 and 1,916 of 4,096, P-70 § 3.5). In
  production a release either ends the process that holds the streams or,
  in the `Forwarder` recovery, follows the owner's abort of every stream
  (which leaves the holder's socket unchanged, ADR-0211). How a holder's
  stream reports either case is the `GuestStream` contract of the feature
  delta (EVMOD-STREAM-END), not this decision.
- The Tier-3 suite of the module step keeps the spike's release cases: a
  context closed with live pairs and watches resets every far end, and a
  child spawned while the context is open does not keep it alive. The
  recovery order (abort-all, close the old context, open the new one) is a
  composition of measured facts — the release completes before it returns,
  and one context's release leaves another's pairs running (P-70 § 3.8) —
  and is exercised through the production owner in DELIVER (V-8).
