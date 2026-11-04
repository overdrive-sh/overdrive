# ADR-0249: The shim registers probe supervision before it releases the exit-event gate

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D3). GH #318,
GH #334. Not implemented. The exact call-order text, the lifecycle gate (G-2), the
formal protocol model and the DISTILL obligations live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Companion: [ADR-0248](adr-0248-the-component-that-observes-an-allocations-end-fires-the-terminal-hook.md)
(who fires the terminal hook); this ADR can be adopted or reversed without it.
Related: ADR-0054 § 2, ADR-0083, ADR-0100.

## Context

`release_for_exit_emission` is what lets the VM driver's exit watcher emit an
`ExitEvent`; the gate send is the last effect of the method. Today the action
shim calls it before `on_alloc_running`, at both `Running` sites (a start and a
restart successor). The two calls share one piece of state, the allocation's
probe supervisor, and have no happens-before edge.

Once the exit observer fires the terminal hook per `ExitEvent` (ADR-0248), an
event consumed before `on_alloc_running` runs the hook against an unregistered
allocation id as a no-op. `on_alloc_running` then registers a supervisor for an
allocation that already ended, and nothing ever stops it: the shim never fires
the hook for a naturally exited allocation. The window is narrow, because the
watcher must finish its artifact cleanup first, but it is an unordered pair of
operations on shared state. Probe tasks tick before they dial, and
`on_alloc_running` does not depend on the exec release or the gate, so moving it
earlier changes no other behaviour.

## Decision

**At both `Running` sites the action shim registers probe supervision
(`on_alloc_running`) before it releases the exit-event gate
(`release_for_exit_emission`).** An `ExitEvent` for an allocation can therefore
only be emitted after its supervisor is registered, and the exit observer's hook
always finds it.

## Alternatives considered

- **Keep the current order and add a registry tombstone.** The probe runner
  remembers every stopped allocation id and refuses `start_alloc` for it. It
  compensates for a missing ordering with an unbounded set, and it is a fallback:
  it repairs a race instead of removing it.
- **Keep the current order and accept the window.** A permanent orphan supervisor
  per lost race, with probes running against a dead guest for the process
  lifetime. The exposure is small but unbounded in time and has a trivial fix.
- **Have the observer register on first sight of an event.** The observer would
  need the allocation's probe descriptors, which travel on the start spec and
  not on the event. It moves start-side knowledge into the observer.

## Consequences

- Positive: the race is removed rather than bounded; the model checks it as
  `NoRegisterAfterEnd`.
- Positive: no new state, type, signature or error; two statements swap order.
- Negative: ordering is a convention between two shim statements. The
  recording-driver trace test and the Quint hazard variant are the enforcement;
  no lint is proposed.
- Neutral: when the shim errors before reaching either call (mTLS install,
  guest activation), neither runs, as today, so the hook has nothing to stop.
