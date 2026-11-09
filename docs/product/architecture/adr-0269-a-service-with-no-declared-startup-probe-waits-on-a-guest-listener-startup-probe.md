# ADR-0269: A Service with no declared startup probe waits on a guest-listener startup probe, so the startup gates carry over unchanged

## Status

**Accepted, 2026-11-09, after user rulings and independent full DESIGN review
iteration 4** ([review](../../feature/service-allocation-readiness-and-ending/design/review.md)).
PR-3, PR-4 and P1 are in the [feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md)
(D14). The user approved D14-A1's exact TCP-view signature and D14-E2's typed
input-error contract on 2026-11-09 (“Approve all three exact contracts”);
bounded technical review iteration 2 and the full review approved their shape.
Not implemented. GH #318, GH #334.
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
  observed streak on it (`same_status_runs`, the count of consecutive written rows of that
  status) has reached the gate's attempts threshold and the elapsed time has reached the
  derived deadline (`service_lifecycle.rs:1168-1210`). Today the threshold is the descriptor's
  `max_attempts`, because a dial probe writes one counted row per interval from its first
  tick.
- D8 retries a stop that did not land once per new startup row, so the producer must keep
  writing rows while the allocation is `Running` and has not passed.

A guest that never opens its listener sends nothing, so the report cannot itself produce
the `Fail`.

## Decision

**The probe runner remains the only writer of startup `ProbeResultRow`s. A Service whose
startup gate is the guest-listener gate has a platform-supplied startup descriptor with a
new mechanic whose producer is the guest's listener reports rather than a dial.**

- **Rows are ordinary and honest.** The task writes through the dial task's run state, so
  `run_seq` and `same_status_runs` obey the existing run-count contract: a row's
  `same_status_runs` is the count of consecutive written rows of its status. No row field is
  added, and no row carries a counter the producer did not observe.
- **Pass is an event.** When the host has seen every declared listener reported open
  (ADR-0270), the probe task writes one `Pass` row immediately.
- **Fail starts at the deadline.** If no `Pass` has been written when the platform startup
  deadline (60 s, the descriptor's `interval x max_attempts`) is reached, the task writes the
  first `Fail` (`same_status_runs` 1), naming the declared listeners not yet reported; it
  then writes one `Fail` per `interval` while no `Pass` has been written (2, 3, …). Each
  later `Fail` is the new row D8 needs to retry a stop that did not land. No `Fail` precedes
  the deadline.
- **The stop gate takes its attempts threshold from the descriptor's mechanic** (P1,
  accepted 2026-11-04). The reconciler already reads the Service's startup descriptor when it
  derives the deadline; it puts the threshold on the fact: the descriptor's `max_attempts` for
  `Tcp` and `Http`, and 1 for `GuestListeners`, whose only clock is the deadline. A first
  `Fail` at the deadline then satisfies the gate, and an explicit startup probe's rule is
  unchanged.
- **Everything else downstream is the existing gate**: `Stable` (D9/B1), backend eligibility
  (D12), the startup stop and retry (D7/D8), the give-up `StartupProbeFailed` (D7), the
  progress deadline (D10), and the deploy stream. The one reconciler change is the gate's
  threshold input above.
- **The platform-only descriptor is admitted at its exact boundary.** It is
  valid only as the sole Service startup descriptor; Service API admission
  returns distinct typed causes for an invalid role or a non-sole startup
  descriptor, and operator TOML rejects the known platform-only mechanic with
  a distinct typed parse cause (D14-E2). Raw Job/Schedule payloads have no probe
  field. The TCP-only TPROXY consumers read the owned TCP view of the same
  declared-listener set (D14-A1). The feature delta owns the exact signatures,
  variants, payloads and ingress mappings.

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
  tcp:8080") before the deadline and needs no gate change, at about 29 extra observation
  writes, router wakes and evaluations per never-ready attempt, for rows only that display
  reads. Rejected (the user's PR-3 trade-off).
- **A first `Fail` that carries `max_attempts` as its streak.** It would let the gate stay
  unchanged, but the streak is the producer's observed count of consecutive written rows
  (`ProbeRunState::next_streak`), pinned by the run-count tests; a forged value breaks the
  invariant every reader of the field relies on. Rejected.
- **A counter field on the row.** A new row version for a number the reconciler can derive
  from the descriptor it already reads. Rejected.
- **Make the mechanic a poll with a short interval that reads the set.** Reintroduces the
  floor the ruling removes.

## Consequences

- One new probe mechanic (operator-visible in `workload describe` and OpenAPI) and one new
  mechanic-specific producer in the probe runner; no new action, row family or reconciler
  branch.
- Until `Pass` or the deadline, an attempt has no startup row, so `describe` shows no startup
  result for the first 60 s.
- One in-memory State field of the reconciler changes (`ServiceAllocFact.max_attempts` is
  replaced by `startup_attempts_required`, the gate's attempts threshold); no row, wire, archived or CBOR change.
- A lost `Pass` write holds `Stable` until the next row; the task retries a failed `Pass`
  write until it lands (a seeded-simulation scenario with a failing observation write).
