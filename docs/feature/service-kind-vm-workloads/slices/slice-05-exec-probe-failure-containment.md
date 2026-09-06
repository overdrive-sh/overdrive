# Slice 05 — Exec probe failure containment

**Story:** US-SVM-5
**Priority:** P4, release safety gate
**Effort:** 5 hours maximum after accepted DESIGN
**Dependencies:** Slice 04; accepted H3–H5 evidence

## Goal

Ana observes bounded Exec-probe failure without leaked descendants, ambiguous
replay, unbounded queuing, or loss of the Service/VMM.

## IN

- Deadline kills and reaps the probe process tree only.
- Connection loss performs bounded cleanup and never replays ambiguous work.
- Reconnect begins a new session; the next scheduled tick is fresh work.
- Capacity exhaustion returns prompt overload and keeps memory bounded.
- Stop refuses new work and drains active probes before guest shutdown.

## OUT

- Exactly-once semantics for arbitrary side-effecting commands.
- Snapshot/restore continuity for an in-flight probe session.
- Adversary-resistant truth against guest root.
- Operator-tunable concurrency or protocol controls.

## Learning hypothesis

Failure disproves that the selected in-guest mechanism is safe enough for
repeated health work and blocks Exec enablement under that design. Success
confirms probe failures remain smaller than the Service failure they detect.

## Acceptance

- A two-second command that forks `sleep 30` leaves no shell or child one second
  after timeout; the workload and VMM still serve traffic.
- Socket loss after ambiguous delivery yields one execution, no replay, and a
  fresh next-tick identity across 100 seeded schedules.
- Capacity exhaustion fails promptly as overload with bounded memory.
- Stop leaves no active probe process before guest shutdown completes.

## Dogfood

Ana runs the timeout, disconnect, and overload fixtures while continuously
requesting `reports-vm`; responses continue from the same allocation.

## Reference class

Kubernetes' leaked-exec warning, CRI's bounded ExecSync contract, Kata's
explicit exec/wait/signal lifecycle, and the existing host Exec timeout intent.

## Pre-slice spike

H5 must show bounded cleanup on disconnect and new-session/no-replay behavior
on native metal before DESIGN may mark this slice ready.
