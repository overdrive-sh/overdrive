# ADR-0303 — A replacement waits for the volume's claim to clear, not for the old VMM process

## Status

**Proposed** (decision VD-8, GH #348). Not accepted until the user approves it.

## Context

Single writer is enforced by the kernel's exclusive claim, taken by opening the block device `O_EXCL`
(ADR-0305; `block/bdev.c:598-611, 949-957 @ v7.0`), and by Cloud Hypervisor's OFD write lock. Neither
is released at VMM process exit. With io_uring the process was gone 28 ms after SIGKILL while an
in-flight write kept the open file, and so the lock and claim, for 2.4 s (`fs/locks.c:2764-2776 @ v7.0`;
spike B2b). A replacement started in that window fails to open the disk (`EBUSY`). ADR-0109 hands off on
a terminal predecessor row, which is earlier than release, and ADR-0110 keeps replacement policy out of
the driver. The restart budget is five attempts one second apart: a drain longer than that would exhaust
it for a reason that is not a crash.

## Decision

**The launch path of an allocation that has a volume waits for the volume to be free before it launches
the VMM. "Free" means one non-blocking attempt to take the volume's exclusive claim (an `O_EXCL` open)
succeeds and is released again. The wait polls on the injected clock for at most 30 seconds. If the bound
passes, the launch fails with the typed volume start failure (cause `Held`), recorded in the allocation's
`Failed` row and charged to the restart budget like any start failure.**

The 30 seconds is more than ten times the longest drain measured (2.4 s, with a 3 s injected device
delay). The kernel offers no event for claim release (only an in-progress claim has a wait queue), so a
poll is the only mechanism. The wait is not the correctness arbiter: Cloud Hypervisor's own open remains
the atomic verdict, and a lost race is an ordinary start failure. The wait exists so a drain is not
charged as a crash. The OFD lock is not polled: for one open file description the kernel removes the lock
before it releases the device (`fs/file_table.c:461,468 @ v7.0`), so a free claim implies a free lock.

## Alternatives considered

- **Treat VMM process exit as "volume free".** Rejected: measured false (2.4 s).
- **Let the launch fail and rely on the restart budget.** Rejected: five attempts at one-second backoff
  would give up inside a longer drain and finalize the workload as failed.
- **A reconciler-side wait that defers the restart.** Rejected: every deferred attempt would still mint an
  allocation, a network lease and an identity.
- **Pass an already-open descriptor to the VMM.** Rejected: a second `O_EXCL` open of the same device by a
  different open file description is refused even in one process.
- **A fork change returning a distinct exit status for "disk busy".** Rejected as unnecessary: the
  pre-launch attempt yields a typed errno in Overdrive's own code.
- **Also take a whole-device OFD write lock in the attempt.** Rejected: redundant with the claim for one
  open file description (above), and the wait is not the arbiter.

## Consequences

- Positive: a replacement attaches as soon as the old VMM's I/O drains, with no budget spent, and an
  unreleased volume produces a named failure.
- Negative: a launch may hold its convergence lane up to 30 s for the claim wait, in addition to the
  `ensure` deadline and the existing boot wait (ADR-0102), so a `workload stop` or `restart` for that
  workload is evaluated only after the launch ends. A claim that never drains charges each of the
  restart-limit attempts and ends the workload as `BackoffExhausted` with the `Held` cause as the last
  reason.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Lifecycle Gate Ownership G-2
- [ADR-0109](adr-0109-replacement-requires-terminal-predecessor-handoff.md), [ADR-0110](adr-0110-microvm-only-live-driver-contract.md),
  [ADR-0305](adr-0305-a-writable-block-device-is-opened-exclusively-by-the-vmm.md), spike findings B1 to B4
