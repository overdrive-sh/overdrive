# ADR-0260: A failed startup stop is retried once per new startup probe row

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D8 and D8
item 3). Not implemented. GH #318, GH #334. The exact contract (D8), the
lifecycle gate (G-5), the formal protocol model and the DISTILL obligations live
in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Companion: [ADR-0259](adr-0259-a-startup-probe-failure-stops-the-allocation-and-the-restart-authority-decides.md)
(the stop this decision retries). It is a separate record because it is reversible
without ADR-0259: the stop is complete without it, and reverting restores the
single-shot dedup and removes one persisted field.

## Context

ADR-0259 makes `ServiceLifecycle` stop a live allocation whose startup probes are
exhausted. The reconciler records the allocation as acted on at emission and
skips it from then on. If the action shim's stop fails (a driver stop error other
than not-found, or a mTLS or network cleanup error), the allocation stays
`Running` with exhausted probes (and not a backend, ADR-0267), and no reconciler acts
again. The liveness stop does not have this property: it acts once per crossing,
keyed by the probe row's run sequence, and acts again on a newer crossed row while
the allocation is `Running`.

The startup probe keeps writing a row every interval until `Stable` or the
terminal hook cancels it, so a newer row arrives at the probe interval after the
deadline. Kubernetes recomputes the kill from the stored probe result at every
sync and so retries by construction; Nomad retries the kill, not the trigger.

## Decision

**The startup stop is emitted once per new written startup probe row while the
allocation is live, using the same persisted marker shape the liveness stop uses.**

- The reconciler's persisted view gains one additive field recording the run
  sequence of the startup row a stop last acted on, evolved as an additive CBOR
  field with a serde default (ADR-0035). Absent means never acted.
- The stop is emitted when the D7 gates and the ADR-0257 guard hold and the latest
  startup row is newer than the recorded one.
- Membership of the allocation in the reconciler's "acted on" set no longer
  suppresses a live allocation: the stop rule is evaluated for it on every newer
  row until the row leaves `Running`.
- A startup `Pass` recorded after a stop that did not land contradicts the
  decision the stop acted on: no further stop is emitted, `Stable` is announced
  and the allocation leaves the acted-on set, so liveness (which skips an allocation
  in the set) acts on it once it is `Stable`.

## Alternatives considered

- **Leave the stop single-shot.** The startup path would be strictly weaker than
  the liveness path for the same stop; the user included the retry.
- **Retry inside the action shim.** It moves a decision into the executor and
  cannot cover the cleanup errors that are not the driver's.
- **Recompute with no marker (emit every evaluation while the gates hold).** It
  repeats a driver call at the evaluation cadence instead of once per observation;
  the repository's liveness precedent paces by the probe row.
- **Clear the acted-on marker when the stop fails.** The reconciler never observes
  the shim's error; it would need a new failure-observation row.

## Consequences

- Positive: a transient stop failure no longer strands a live allocation; the
  retry is paced by the probe interval and ends when the row leaves `Running`.
- Positive: one persisted-view field, additive, mirroring an existing one; no
  archived, wire or intent change.
- Negative: a persistent stop failure retries at probe-interval pace without
  limit while the allocation is `Running`, the liveness path's bound taken as is.
- Negative: the persisted view grows by one entry per startup stop emitted, the
  same growth as the liveness marker.
- Neutral: a late `Pass` ends the retry instead of racing it, so a serving
  instance is never stranded in the acted-on set, never `Stable` and never stopped.
- Dependency: the retry trigger is a new startup probe row, which exists only while
  the allocation's probe supervisor is alive. A stop whose terminal row write fails
  after the supervisor was released leaves no further row, so the retry never fires.
  [ADR-0266](adr-0266-the-shim-releases-a-stops-probe-supervision-after-its-terminal-row-is-durable.md)
  (accepted 2026-11-03) decides the shim ordering that keeps the trigger alive until the row
  is durable.
