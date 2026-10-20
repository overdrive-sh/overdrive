# ADR-0192 — A probe run that writes no row emits one not-sampled event

## Status

**Accepted — PROBE-NOSAMPLE-EVENT approved by user 2026-10-13**, under two
rulings of that date: "Probe threshold counting (round-2 H-A)" ("No new
sample = no change (no failure, no success, no reset); each no-row case
(adapter error, write failure) emits an event") for the adapter-error and
failed-write cases, and the approval of PROBE-NOSAMPLE (ADR-0187, whose
decision 2 includes the not-sampled event) for the node-capacity and
quiesced cases. One event name for all four cases, its warning level and
the removal of the `probe_tick.error` warning are architect elaborations
recorded under those rulings. GH #295. The event's name, fields, reasons and
level are in the #295 feature delta,
`docs/feature/netns-density-295/feature-delta.md` § *[REF] VM probe runner*,
*The not-sampled event*; that section is the only home of its fields.

Related decisions, each its own ADR: the probe runner subsystem and the probe
row (ADR-0054), the VM probe outcomes that record no sample (ADR-0187), and
how a written probe run counts toward a threshold (ADR-0191).

## Context

A probe run that writes no probe row is invisible in the row: ADR-0191 makes
such a run consume no run number and leave every verdict where it is, so the
stored row looks the same whether the probe was not due or failed to record
three periods in a row. Four causes write no row, in three paths:

- a VM probe dial refused because the node's flow slots are full, or because
  node forwarding is quiesced (ADR-0187);
- the prober adapter returns an error
  (`crates/overdrive-worker/src/probe_runner/mod.rs:496-517`);
- the observation write fails (`:533-539`).

Today the two error paths log one bare `probe_tick.error` warning
(`mod.rs:585-592`) and the two VM refusals have no event of their own. An
operator looking at a workload whose startup does not progress, or whose
liveness does not fire, has no single signal that its probes are recording
nothing, nor why.

## Decision

**Every probe run that writes no row emits one structured not-sampled event,
naming the probe and the reason, for every workload kind.**

1. One event name for all four causes; the reason distinguishes them (node
   capacity, quiesced, adapter error, write failed), and an adapter error or
   failed write carries the error itself.
2. A probe runs once per period, so the event fires at most once per probe
   per period.
3. It is emitted at the warning level of the event it replaces. It replaces
   the bare `probe_tick.error` warning; there is no separate VM-only event.
4. The event is a diagnostic emission, not state: it changes no row, no
   verdict and no reconciler view.

## Alternatives considered

- **No event for a no-sample run.** A node-capacity or quiescence window, or
  an observation store that rejects writes, would stall a VM's startup and
  freeze its liveness and readiness with no signal. Rejected.
- **Separate events per cause** (a VM-only not-sampled event beside the
  existing error warning). Two names for one fact — "this probe recorded
  nothing this period" — that an operator must join; the reason field already
  separates the causes. Rejected.
- **A durable no-sample marker** (a field or counter in the probe row, or a
  row of its own). A run that writes no row would then write a row, which
  ADR-0191 and ADR-0187 exclude: the probe row records samples, and its
  consumer would have to tell markers from samples. A failed write cannot
  record a durable marker in the store that rejected it. The conditions that
  cause the other two reasons are durable elsewhere: node quiescence is
  Overdrive's own recovery state (ADR-0169) and node capacity is visible in
  the node's flow counts (ADR-0208). Rejected.

## Consequences

- A probe that records nothing is visible per probe and per period, with its
  cause, for every workload kind.
- The bare `probe_tick.error` warning is removed; log queries that matched it
  match the not-sampled event with reason adapter error or write failed.
- The event is not durable: it is lost with the process's log stream, and
  nothing reconstructs a past no-sample window from the store.
