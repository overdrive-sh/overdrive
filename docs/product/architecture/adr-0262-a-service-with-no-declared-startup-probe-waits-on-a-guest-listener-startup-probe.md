# ADR-0262: A Service with no declared startup probe waits on a guest-listener startup probe, so the startup gates carry over unchanged

## Status

**User-approved in direction, 2026-11-03 ("add it. and proceed with recommendations"),
spikes V34 to V39 run (V34, V35, V36, V37, V38 held; V39 partial, see the feature delta
§ Spike table); the corrected D13 contract shape was ACCEPTED by the user on 2026-11-03.
Not yet Accepted as a decision: "user-approved, pending the V41 boot-cost measurement and the
independent re-review"** (a material B-minus-A delta returns D13 to DESIGN). Not implemented. GH #318, GH #334. (The producer's behaviour
itself needs no spike; its inputs are D13's.)
Related:
[ADR-0261](adr-0261-the-guest-reports-each-workload-listener-it-opens-over-the-beacon-channel.md)
(the guest report), [ADR-0265](adr-0265-the-guest-observes-workload-listeners-with-kernel-hooks-not-a-poll.md)
(the kernel observation), [ADR-0263](adr-0263-a-probe-less-service-is-stable-when-the-guest-reports-every-declared-listener-open.md)
(the operator rule), ADR-0054 (probe runner, producer-recorded streaks), ADR-0055,
ADR-0252/0253/0254/0258/0260 (the gates that read the startup row).

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
  (ADR-0263), the probe task writes one `Pass` row immediately. No tick is waited for.
- **Fail is a cadence.** While no `Pass` has been written, the task writes a `Fail` row
  every `interval` after the allocation is registered, naming the declared listeners not
  yet reported. The descriptor's `interval`, `timeout` and `max_attempts` are filled by the
  parser so that `interval x max_attempts` is the platform startup deadline (60 s, the value
  the inferred probe has today); they are not separate knobs for this mechanic. The first
  `Fail` therefore satisfies the existing `attempts >= max_attempts && elapsed >= deadline`
  stop condition, and each later `Fail` is the new row D8 needs to retry a stop that did not
  land. No reconciler logic changes.
- **Source of the events.** The driver that owns the beacon reader holds the reported set
  for the allocation's incarnation (level-readable, edge-notified: a probe task that
  subscribes late reads the current set), and the probe runner's task for this mechanic
  reads it. The set lives and dies with the VM; a control-plane restart that loses the VM
  loses the set with it, and the successor allocation reports afresh.
- **Ordering.** The guest cannot report before the workload runs, the workload runs only
  after `EXEC` is released, and the shim registers the supervisor before it releases
  (ADR-0249) and writes the `Running` row before that. So a report never precedes the
  supervisor or the `Running` row, and `on_alloc_terminal` cancelling the supervisor
  (ADR-0248) means no `Pass` is written after the exit event has been consumed.
- **Everything downstream is the existing gate**: `Stable` (D9/B1), backend eligibility
  (D12), the startup stop and retry (D7/D8), the give-up `StartupProbeFailed` (D7), the
  progress deadline (D10), and the deploy stream.

## Alternatives considered

- **A new observation row family for listener facts.** Needs a new envelope, a new
  `ObservationRowKind`, a new wake and a new reconciler branch to reach the same `Stable`
  and stop semantics the startup row already carries. Rejected: more surface for no new
  behaviour.
- **`ServiceLifecycle` reads the driver's set through a new read-port and decides.** A
  host-backed fact has no row to wake the reconciler, so it also needs a resync cadence,
  and the deadline stop needs a timer; both exist for free in the probe runner. Rejected.
- **A single `Fail` at the deadline.** D8's retry needs a new row per attempt; one row
  strands a stop that did not land. Rejected.
- **Make the mechanic a poll with a short interval that reads the set.** Reintroduces the
  floor the ruling removes. The `Fail` cadence is long on purpose; only the `Pass` is
  immediate.

## Consequences

- One new probe mechanic (operator-visible in `workload describe` and OpenAPI) and one new
  mechanic-specific producer in the probe runner; no new action, port trait, row family or
  reconciler branch.
- The `Pass` row for this mechanic carries an `attempt` of the number of `Fail` rows before
  it (zero for a prompt guest), which the streak arithmetic already accepts.
- Honest bound: a lost `Pass` write holds `Stable` until the next row. The task writes
  `Pass` once per incarnation, so a failed write is retried by the task until it lands
  (same rule as any producer-recorded row); this is a DISTILL scenario (seeded
  simulation with a failing observation write), not a spike.
