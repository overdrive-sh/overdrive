# ADR-0269: A Service with no declared startup probe waits on a guest-listener startup probe, so the startup gates carry over unchanged

## Status

User-approved, 2026-11-03; `declared_listeners` field (PR-4) and `Fail` cadence (PR-3)
accepted 2026-11-04; the D13 hook scope it depends on (PR-2) is accepted in contract shape
(V42 held 2026-11-04, [findings](../../feature/service-allocation-readiness-and-ending/spike/findings-workload-cgroup.md));
not yet Accepted (remaining gates: model check, design review round 4). Not implemented. GH #318, GH #334. Contract
and status: [feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md) (D14).
Related:
[ADR-0268](adr-0268-the-guest-reports-each-workload-listener-it-opens-over-the-beacon-channel.md)
(the guest report), [ADR-0272](adr-0272-the-guest-observes-workload-listeners-with-kernel-hooks-not-a-poll.md)
(the kernel observation), [ADR-0270](adr-0270-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md)
(the operator rule), ADR-0054 (probe runner, producer-recorded streaks), ADR-0055,
ADR-0259/0260/0261/0265/0267 (the gates that read the startup row).

## Context

D5 to D12 of this feature, ADR-0055, ADR-0095 and ADR-0096 all consume one fact: the
latest `ProbeResultRow` of the startup role (`Pass`, or a `Fail` with an attempt count),
its derived deadline, and the committed `Stable` it leads to. Today the only producer is
the probe runner polling the guest. Replacing that producer with a guest report must not
fork those gates. Two properties of the existing gate matter:

- `Stable` needs a `Pass` row. A `Fail` row stops the instance only when the producer's
  attempt count has reached the descriptor's `max_attempts` and the elapsed time has
  reached the derived deadline (`service_lifecycle.rs:1168-1210`).
- D8 retries a stop that did not land once per new startup row, so the producer must keep
  writing rows while the allocation is `Running` and has not passed.

A guest that never opens its listener sends nothing, so the report cannot itself produce
the `Fail`.

## Decision

**The probe runner remains the only writer of startup `ProbeResultRow`s. A Service whose
startup gate is the guest-listener gate has a platform-supplied startup descriptor with a
new mechanic whose producer is the guest's listener reports rather than a dial.**

- **Pass is an event.** When the host has seen every declared listener reported open
  (ADR-0270), the probe task writes one `Pass` row immediately.
- **Fail starts at the deadline.** If no `Pass` has been written when the platform startup
  deadline (60 s, the descriptor's `interval x max_attempts`) is reached, the task writes the
  first `Fail`, naming the declared listeners not yet reported, with the attempt count
  exhausted; it then writes one `Fail` per `interval` while no `Pass` has been written. The
  first `Fail` satisfies the existing stop condition, and each later `Fail` is the new row D8
  needs to retry a stop that did not land. No `Fail` precedes the deadline.
- **Everything downstream is the existing gate**: `Stable` (D9/B1), backend eligibility
  (D12), the startup stop and retry (D7/D8), the give-up `StartupProbeFailed` (D7), the
  progress deadline (D10), and the deploy stream. No reconciler logic changes.

## Alternatives considered

- **A new observation row family for listener facts.** Needs a new envelope, a new
  `ObservationRowKind`, a new wake and a new reconciler branch to reach the same `Stable`
  and stop semantics the startup row already carries. Rejected: more surface for no new
  behaviour.
- **`ServiceLifecycle` reads the host's reported set through a new read-port and decides.**
  A host-backed fact has no row to wake the reconciler, so it also needs a resync cadence,
  and the deadline stop needs a timer; both exist for free in the probe runner. Rejected.
- **A single `Fail` at the deadline.** D8's retry needs a new row per attempt; one row
  strands a stop that did not land. Rejected.
- **A `Fail` every interval from the start.** It gives the operator progress ("missing
  tcp:8080") before the deadline, at about 29 extra observation writes, router wakes and
  evaluations per never-ready attempt, for rows only that display reads. Rejected.
- **Make the mechanic a poll with a short interval that reads the set.** Reintroduces the
  floor the ruling removes.

## Consequences

- One new probe mechanic (operator-visible in `workload describe` and OpenAPI) and one new
  mechanic-specific producer in the probe runner; no new action, row family or reconciler
  branch.
- Until `Pass` or the deadline, an attempt has no startup row, so `describe` shows no startup
  result for the first 60 s; the first `Fail`'s attempt count is `max_attempts` by definition
  rather than a count of failed attempts.
- The `Pass` row carries an `attempt` of 0, which the streak arithmetic already accepts.
- A lost `Pass` write holds `Stable` until the next row; the task retries a failed `Pass`
  write until it lands (a seeded-simulation scenario with a failing observation write).
