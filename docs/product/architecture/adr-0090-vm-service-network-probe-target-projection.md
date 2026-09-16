# ADR-0090: Resolve VM Service network-probe targets at allocation registration

## Status

**Accepted** (2026-09-06), user-selected during guided DESIGN and approved by
independent architecture review iteration 2. GH #257. **Amended 2026-09-16 by
the user-authorized GH #295 solution-review F-01 remediation:** the probe target
policy remains, while the stale six-argument `VmDriver` constructor contract is
removed in favor of the #295 feature delta's complete post-cut constructor.
**Amended again 2026-09-16 by user-authorized S2-F04:** netns/veth evidence is
strictly historical; the live proof uses the shared bridge/direct host TAP.

Extends ADR-0054 (ProbeRunner), ADR-0055/0080 (probe-role lifecycle), and
ADR-0088/0089 (VM guest addressing and provision-before-start ordering).

## Context

At the time of the original decision, `ProbeRunner` translated omitted HTTP
hosts and the TCP wildcard `0.0.0.0` to host loopback inside every probe tick;
that matched the then-live Exec/process Service path. GH #293 deleted that
driver and its probe surface, so this fact is historical rationale only. The
operative problem is VM-only: the listener runs inside the guest and the action
shim materializes its routed guest address in the grouped network assignment.

The following production owner path is **pre-#295 historical context**, retained
to explain why this decision originally selected the allocation address:

1. `provision_and_inject_netns` always assigns a network for `DriverType::Vm`;
2. `inject_workload_network` writes `Some(tap.guest_addr)` to the allocation
   spec;
3. the selected VM driver starts and completes its Beacon contract;
4. the action shim commits the Running observation; and
5. only then it calls `Driver::on_alloc_running(&spec)`.

Native-metal spike H6 independently proved that a host-originated TCP
connection reaches this address through the production veth/netns/TAP path.
That spike establishes historical reachability, not the post-#295 substrate or
component ownership.

After #295, the same semantic value arrives through the grouped guest-network
assignment. The action shim provisions one direct host-netns TAP on the shared
bridge before `VmDriver::start`; `overdrive-init` applies that assignment before
READY; the action shim commits Running and calls the unchanged probe-registration
hook afterward. The persisted observation remains `AllocStatusRow.workload_addr`.
No netns, veth, `/30`, `NetSlot`, or `host_veth` participates in the live target
path.

## Decision

`ProbeRunner` resolves each HTTP/TCP effective target once when the allocation
is registered at the existing `on_alloc_running` supervision boundary. Its
existing public registration method becomes:

```rust
pub fn start_alloc(&self, spec: &AllocationSpec) -> CancellationToken;
```

The projection is immutable for the task lifetime:

| Declaration | Allocation driver | Effective destination |
|---|---|---|
| Non-wildcard explicit host | VM | Explicit host unchanged |
| Omitted HTTP host or `0.0.0.0` | VM | That allocation's grouped guest-network assignment address; the same value persists as `AllocStatusRow.workload_addr` |

The declared `ProbeDescriptor` remains persisted intent. Effective runtime
targets are private task inputs and are not written back to intent or
observation schemas.

The already-trusted production `Arc<ProbeRunner>` remains a mandatory
`VmDriver` constructor dependency immediately before the GH #295 EXEC-gate
capability; host layout remains last. GH #295 adds that gate so the
control-plane shared-owner supervisor and worker release path linearize on one
dependency-neutral capability. The exact complete constructor is intentionally
single-sourced in
`docs/feature/netns-density-295/feature-delta.md` § *EXEC-close
linearization*; the prior six-argument code block here is removed rather than
left as a competing stale signature.

`VmDriver` uses the existing three `Driver` hooks: Running registers all roles,
Stable stops Startup only, and terminal stops the whole allocation supervisor.
No new `Driver` method, optional builder, or VM-specific probe runner is
introduced. The old comparison to `ExecDriver` is historical and carries no
live branch or compatibility requirement.

### VM address precondition

`Some(network)` on `AllocationSpec`, with the address carried inside the
grouped assignment, is a precondition for VM probe registration, guaranteed by
the post-#295 production path. `Vm + None` remains representable only because
the shared transient specification supports a not-yet-provisioned value; it has
no producer at the registration point on the real VM `serve` + `deploy` path.

Consequently this ADR defines no fallback, scheduled probe failure, silent
skip, allocation transition, public error/state, or test for `Vm + None`.
Converting that internal invariant into a probe `Fail` would falsely blame
guest health. Any future defense requires a production-path reproduction that
first disproves this premise.

### Lifecycle Gate Ownership

**Not applicable:** target projection changes only where an already-scheduled
HTTP/TCP attempt connects; it adds, removes, and moves no lifecycle gate.

- Allocation `Running` retains the action-shim/VM-driver Beacon meaning.
- Startup results alone gate Service `Stable`.
- Readiness results alone control `Backend.healthy`.
- `ServiceLifecycle` liveness results feed only the existing threshold and
  liveness `StopAllocation` termination.
- `WorkloadLifecycle` remains the sole restart authority: it observes that
  terminated allocation row and decides restart versus finalization under the
  unified budget.

A VM can therefore be Running while its guest port is closed and its startup
probe is failing. Probe health never delays, revokes, or reinterprets Running.

## Alternatives considered

1. **Resolve on every tick — rejected.** The allocation address is immutable
   for the registered supervisor; repeating the policy broadens every
   long-lived task without improving correctness.
2. **Rewrite descriptors during hydration — rejected.** The runtime address is
   unavailable at declared-intent hydration and rewriting would collapse
   intent into runtime projection.
3. **Global allocation-address registry — rejected.** The value is already at
   the registration handoff; a registry adds mutable ownership, lookup errors,
   and teardown races.
4. **VM-specific runner or new driver hook — rejected.** Existing runner and
   lifecycle hooks own the exact responsibility.
5. **Define behavior for `Vm + None` — rejected as an ungrounded premise.** No
   real production path produces it; this repeats the failure class documented
   by the GH #248 precedent in `.claude/rules/development.md`.

## Consequences

- Positive: HTTP/TCP probes reach the guest through the address the platform
  already provisions and exposes.
- Positive: explicit VM hosts remain unchanged; omitted/wildcard VM targets use
  the provisioned guest address.
- Positive: no new storage, adapter, protocol, lifecycle state, or dependency.
- Negative: `ProbeRunner::start_alloc` now accepts the full allocation spec,
  and every bounded call site must migrate in one cut.
- Negative: the 2026-09-06 six-argument constructor record is no longer
  complete after GH #295; the feature delta pins the additional mandatory gate
  and bounded compiler fallout while preserving the `Driver` trait.

## Evidence obligations

- Pure projection properties over the complete live production input set:
  explicit VM host and VM default with provisioned grouped-assignment address.
- Component evidence that both TCP and HTTP adapters receive the projected
  destination while the stored descriptor is unchanged.
- Lifecycle evidence that Running is unaffected by failing guest probes and
  Stable/readiness/liveness retain their existing owners.
- Native-metal, built-default-feature `serve` + `deploy` evidence through the
  production shared bridge and direct host-TAP path; no workload netns/veth and
  no test-installed address, route, TAP, classifier, or listener.
- Existing terminal-wins-over-late-probe invariant. No `Vm + None` scenario.

## References

- `docs/feature/service-kind-vm-workloads/feature-delta.md`
- `docs/feature/service-kind-vm-workloads/design/wave-decisions.md`
- `docs/feature/service-kind-vm-workloads/spike/findings.md` (H6)
- ADR-0054, ADR-0055, ADR-0080, ADR-0088, ADR-0089

## Amendment — 2026-09-07

**Historical after GH #293 removed the Exec driver.** The text below records the
then-valid Exec/process refinement; it is not a live post-#293 branch.

ADR-0097 supersedes this ADR's `Exec/process -> 127.0.0.1` default/wildcard
row only when the existing action-shim composition has provisioned an Exec
allocation network and supplied `Some(workload_addr)`. Such an Exec probe is
host-originated while its process is in that allocation netns, so the
registration-time effective target is its provisioned transit address. An
unnetworked Exec allocation retains this ADR's loopback normalization, and all
non-wildcard explicit hosts remain unchanged.

## Amendment — 2026-09-16 (GH #295 application composition)

The VM target-projection policy is unchanged: an omitted/wildcard HTTP/TCP
target resolves once to that allocation's provisioned guest address when probe
supervision starts. The live producer is now the grouped network assignment on
the shared-bridge/direct-host-TAP path, not the historical netns/veth path.
Native evidence must drive the default-feature product through real `serve` +
`deploy` and observe the shared bridge/TAP/classifier path the product creates;
a test-created address, route, TAP, TCX attachment, nft rule, or listener is not
substitute evidence. This amendment changes no probe state, health owner, or
lifecycle gate.
