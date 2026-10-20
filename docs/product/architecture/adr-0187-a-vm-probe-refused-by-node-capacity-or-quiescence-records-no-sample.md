# ADR-0187 — A VM probe refused by node capacity or quiescence records no sample

## Status

**Accepted — PROBE-NOSAMPLE approved by user 2026-10-13.**
GH #295. Recorded in the #295 feature delta,
`docs/feature/netns-density-295/feature-delta.md` § *[REF] VM probe runner*
(the probe outcome per dial result) and § *[REF] Driving port — guest stream
dial `GuestStreamDial`* (the dial's order of decision).

Related decisions, each its own ADR: how the VM probe runner reaches a VM
port (ADR-0173, INB-1, accepted), the probe-socket mark for exec and process
targets (ADR-0094), the per-role probe supervisor (ADR-0080), node forwarding
quiescence (ADR-0169), the flow quota and node flow-slot capacity
(ADR-0208), the single restart authority that acts on liveness
(ADR-0087), how a probe sample is counted (ADR-0191), and the event a
probe run that records no sample emits (ADR-0192). Decision 1's bound — one
probe dial per probe task — holds because ADR-0209 releases a dial's places
when its stream is dropped or aborted, so no probe flow is held past its
stream.

## Context

A VM probe dials the guest through the guest-flow owner (ADR-0173). Two of
the dial's typed refusals can describe a condition of the node rather than of
the VM being probed:

- **Capacity.** A dial takes one slot of the node's flow capacity, and a
  relay dial also counts against its allocation's flow quota (ADR-0208,
  ADR-0173). The node's slots are shared by every VM on it: sixteen VMs at
  their per-allocation quota fill the node, and half-closed flows held open
  by a guest that withholds credit keep their slots until they end. A
  dial at a full node is refused although the probed VM may hold few flows.
- **Quiesced.** Node forwarding is quiesced while Overdrive recovers its own
  shared network state (ADR-0169). Every dial on the node is refused for the
  duration, whichever VM it targets.

The probe outcome drives three lifecycle decisions owned elsewhere: the
startup probe gates `Stable` and, after enough failures, `StartupProbeFailed`
(ServiceLifecycle); liveness failures restart the workload under the single
restart authority (WorkloadLifecycle, on ServiceLifecycle's liveness
terminate); readiness decides backend eligibility. If either refusal counted
as a probe failure, a node-wide condition, or another tenant's load, would
fail a healthy VM's startup, restart it, or withdraw it as a backend. A
direct TCP probe of an exec workload is not affected by other workloads'
flows or by Overdrive's flow quiescence, so the same workload run as an exec
workload would stay healthy. Probe failure would put the failure in the wrong
owning domain (`.claude/rules/design.md` § "Gate placement rules").

The allocation's own flow quota is a different case. A probe exists to detect
an application that has stopped serving; an application that is hung holds
its flows open and reaches its quota exactly when it stops serving. If a
probe dial counted against that quota, a hung application would refuse its
own probes for capacity, and treating that refusal as no sample would hide
the hang from its liveness probe indefinitely. Treating it as a failure
instead would fail a busy, healthy VM whose listener still accepts. The probe
dial and the relay dial have different purposes — one measures the
application, the other carries a client's connection — and only the relay
dial is client load the quota exists to bound.

## Decision

**A VM probe dial does not count against its allocation's flow quota, and a
VM probe attempt whose dial is refused for node flow-slot capacity or because
node forwarding is quiesced records no sample.**

1. **Probe dials stand outside the allocation quota.** A probe dial is
   distinguished from a relay dial by its purpose, which the caller states
   when it dials (feature delta). A probe dial still takes one node forwarder
   slot; it does not count against, and is not refused by, the allocation's
   flow quota. Each probe task holds at most one dial in flight, so the
   in-flight probe dials of an allocation are bounded by the probe count its
   spec declares, one dial in flight per probe task. The spec grammar sets no
   limit on that count (each role's probes are indexed by their position in
   the role's array, `crates/overdrive-core/src/aggregate/workload_spec.rs:1108-1119`),
   so the bound is the spec's own count. ServiceLifecycle reads only the
   first probe of each role (`crates/overdrive-reconcilers/src/service_lifecycle.rs:899-904`),
   as it does today.
2. **No sample for node conditions.** For a probe dial, a capacity refusal
   can only mean the node's flow slots are full. That refusal, and a
   quiesced refusal, record no sample: the probe runner writes no probe
   result for that attempt, consumes no startup-probe attempt, advances no
   success or failure threshold, and runs the probe again at its next
   period. It emits the not-sampled event of ADR-0192.
3. **Every other dial result keeps its meaning under ADR-0173.** A refused,
   timed out, unreachable, not-serving or no-session dial, and a dial that
   misses its pairing deadline, is a probe failure with the result as its
   detail. A hung application at its own quota is reached by its probe dial
   (which stands outside the quota) and fails its HTTP probe when the
   request times out; a busy healthy VM at its quota passes.

A missing sample therefore leaves every streak and the last recorded verdict
where they are; the absence of a sample is not itself a verdict, and a period
with no sample neither advances nor resets the startup gate. How each
recorded sample is counted toward a threshold is ADR-0191.

## Alternatives considered

- **Count both node refusals as probe failures.** A node-wide condition or
  another VM's load would fail, restart or de-register healthy VMs, and
  Overdrive's own recovery would cascade into workload restarts. Rejected.
- **Probe dials count against the allocation quota, and a quota refusal is a
  probe failure.** A busy, healthy VM whose listener still accepts would fail
  its probes and be restarted, which a direct TCP probe of the same workload
  would not do. Rejected.
- **Probe dials count against the allocation quota, and a quota refusal
  records no sample.** A hung application holds its flows and reaches its
  quota exactly when it stops serving; its probes would then record nothing,
  and its liveness probe would never fail. Rejected.
- **Reserve a share of each allocation's quota for probe dials.** Needs a
  sizing rule for the reserve and keeps probe and relay load in one budget;
  the purpose exemption is bounded by construction (one dial in flight per
  declared probe) and needs no sizing. Rejected.
- **Reserve node flow slots for probe dials outside the node capacity.** Adds
  a second sizing rule for the slot space, and does not cover quiescence,
  under which every dial is refused regardless of slots. Rejected.

## Consequences

- No VM is restarted, failed at startup or withdrawn as a backend because
  the node is full or quiesced.
- While the node stays full or quiesced, a VM's probes record nothing: a VM in
  startup stays in startup until a sample is recorded, and liveness and
  readiness keep their streaks and last verdict. The exposure lasts as long
  as the condition; the not-sampled event makes it visible per probe and
  period.
- A hung VM at its own flow quota is detected by its liveness probe; a busy
  healthy VM at its quota is not reported unhealthy, and its load is visible
  through its flow counts.
- Probe dials can exceed an allocation's flow quota by at most its declared
  probe count; they still count against the node's flow slots.
