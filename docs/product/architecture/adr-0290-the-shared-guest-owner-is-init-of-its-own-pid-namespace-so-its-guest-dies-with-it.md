# ADR-0290 — The shared-guest owner is init of its own PID namespace so its guest dies with it

## Status

**Accepted — approved by the user, 2026-11-14 (DDD-8 in the feature delta).**

## Context

A shared guest's VMM must never outlive the owner that serves it: nextest
leaks, crashed owners and killed owners would otherwise leave a VMM running on
the shared Metal box until the run ends. A parent-death signal on the VMM would
need a change to the production launcher, which a test need must not shape.

## Decision

The owner calls `unshare(CLONE_NEWPID)` and forks; the child is the owner proper
and so is init of its namespace. The kernel kills every other task of a PID
namespace when its init dies and waits for them, so the VMM dies with the owner
and the production launcher is unchanged. Kernel basis, read in `vendors/linux`
at v7.0: the first child becomes the namespace's init (`kernel/fork.c:2239,
2425-2428`); init exit reaches `zap_pid_ns_processes` (`kernel/exit.c`), which
SIGKILLs every other task (`kernel/pid_namespace.c:225-234`) and waits until
all are freed (`:270-275`). Measured on the Metal kernel: no VMM survived the
owner (gone within 38.6 ms over 13 cycles), and the production launcher worked
inside the namespace. The run is root, so a refusal of the namespace is not
designed for; an absent owner binary is an unprepared kit, refused by the
electing client as `LaneNotPrepared` before it takes the election lock. The
owner calls `setsid` before it serves, so a group signal to the electing test's
process group does not kill it.

## Alternatives

- **A parent-death signal on the VMM**: needs a launcher change; rejected.
- **Supervision by the owner's own watcher process**: a second process that must
  itself outlive and be reaped; the kernel already provides the guarantee;
  rejected.

## Consequences

The owner is a separate process kind with its own lifetime; when it exits is
ADR-0293. Crafter notes for running inside the namespace are in the feature
delta (DDD-8).

## Links

- [Decision detail and kernel citations](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-8--the-owner-question-3)
- [Owner lifetime findings](../../feature/metal-test-placement-attribute/spike/findings-owner-lifetime.md)
- [ADR-0283](adr-0283-the-tests-of-a-module-share-one-guest-through-a-per-module-owner-process.md)
