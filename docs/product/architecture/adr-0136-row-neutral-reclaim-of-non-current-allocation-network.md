# ADR-0136 — Reclaim a non-current or unowned allocation's network residue through a row-neutral lifecycle action

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R11. Proposed 2026-09-23 and revised 2026-09-24; reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN.
Depends on ADR-0133 (the counting policy of user ruling D-295-R7, 2026-09-24),
ADR-0134, and ADR-0135. ADR-0106 recorded a retrying cleanup owner as an
independently decidable expansion that needs explicit approval and its own
exact design. This ADR is that design, scoped to guest-network residue. The
retry-forever policy is **user-approved on 2026-09-24**, which discharges
ADR-0106's approval requirement. Exact signatures live only in the #295 feature
delta.

## Context

Retry-retaining cleanup (ADR-0135) needs a level-triggered owner to retry it.
Two cleanup paths already have one:

- **Stop:** the terminal row is written only after cleanup, so the reconciler
  re-emits `StopAllocation` until cleanup succeeds.
- **Genuine-terminal finalize:** the terminal claim is the commit fence, so the
  reconciler replays `FinalizeFailed`, and only the missing effect runs again.

Other leased allocations have none:

- **A restart predecessor** whose cleanup, run after the successor's outcome
  (ADR-0106), fails once the successor is Running. It is no longer the current
  allocation, and nothing re-drives it.
- **A Failed or Terminated current allocation of a workload that is then stopped
  or deleted.** The Stop and GC branches stop only Running rows.
- **An at-cap replacement predecessor.** Under ADR-0133 its lease counts until
  its cleanup finishes, so at the cap its successor cannot be admitted until
  that cleanup runs, and no restart dispatch runs to perform it.

Under ADR-0133 each such lease also holds a slot against the cap. The
reconciler returns early from several branches (the Stop branch, the Absent/GC
branch, and the Run branch's Job fence, Running, Draining, operator-stop, and
Job natural-exit guards), so any retry emission computed only on the restart
path would never run for these cases.

Reusing `StopAllocation` or `FinalizeFailed` would rewrite the allocation's
recorded outcome: a crash would become an operator stop, or gain a new terminal
claim.

The shape is established even though no system uses this exact action: the
kubelet's pod worker retries a terminated pod's sandbox teardown independently
of the replacement the controller already created, and finalizers retain a
deleted object until its controller finishes. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 5.1, 6.1 and 6.4.)

## Decision

`WorkloadLifecycle` emits a row-neutral reclaim action for every allocation of
its workload that meets all of these conditions:

- it holds a guest-network lease (ADR-0134's read-port);
- its latest row is Failed or Terminated;
- no other action in the same evaluation names it;
- it is not the current allocation of a workload whose restart is pending and
  either not yet due, or due while the node has room, because that restart's
  own one-shot cleanup owns it (ADR-0106). Only a due restart at the cap hands
  its predecessor to reclaim.

This covers restart predecessors, allocations of stopped and deleted
workloads, and, at the cap, the current replacement predecessor. The reclaim set
is computed on every evaluation path, including every early return.

The action shim executes it in this order. It retires the lease and quiesces any
VMM, tolerating NotFound. It stops the intercept, tears down the attachment
effect-first, and releases the lease last. It writes no allocation row and
emits no lifecycle event.

The action is idempotent and level-triggered: an allocation with no lease is a
no-op, and a part that is already gone counts as removed. It retries without an
attempt limit until the lease is released or the next boot sweeps the residue.
Re-emission after a failure follows the existing restart backoff policy,
computed from persisted inputs, so a persistently failing reclaim backs off
rather than re-dispatching immediately. That policy is a constant one second
today, until
[GH #137](https://github.com/overdrive-sh/overdrive/issues/137) makes it
configurable. It converges when the lease disappears.
Cleanup of a Running allocation stays with Stop, and a finalize stays with
Finalize replay, so each allocation has exactly one cleanup owner at a time.

One case has no in-process retry owner: an allocation whose first row was never
written, because the observation store rejected it, and whose same-arm cleanup
then also failed. The reconciler cannot see it without a row. It holds no
intercept element and its TAP is down, but its Retiring lease counts against
the cap until the next process boot's reclamation and sweep remove it. The
retiring count in every admission refusal makes it visible.

## Alternatives considered

### Extend the host-backed `vm-reclamation` reconciler

Rejected. `vm-reclamation` owns VMM process residue discovered by host
observation. Network residue is lifecycle-owned. Extending it would need new
pool and registry read-ports and would create a second cleanup authority.

### Reuse `StopAllocation` or `FinalizeFailed`

Rejected. Either rewrites the allocation's row and falsifies what happened.

### Compute the reclaim set only on the restart path

Rejected. The reconciler's early returns would skip it for stopped workloads,
deleted workloads, Job terminals, and the other guards, so their leases would
never be reclaimed.

### Leave the residue until process restart

Rejected. It leaks kernel attachments for the process lifetime, and under
ADR-0133 it would hold slots against the cap.

## Consequences

Positive: every retained retirement of an allocation that has a row has exactly
one level-triggered retry owner, recorded outcomes stay truthful, and the at-cap
recreate ordering has an owner.

Negative:

- The core `Action` enum gains one variant, with an output-validator rule and a
  shim arm.
- `WorkloadLifecycle` gains one hydrated fact and one emission computed on every
  path.
- Retrying forever at a constant one-second cadence means a large population of
  persistently failing cleanups competes for the reconciliation runtime's
  bounded concurrency until the policy becomes progressive. A failure common to
  every allocation is a shared-network fault that recovery repairs or
  fail-stops within seconds, so sustained load comes only from per-allocation
  failures.
