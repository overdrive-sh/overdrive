# ADR-0090: Resolve VM Service network-probe targets at allocation registration

## Status

**Proposed** (2026-09-06), user-selected during guided DESIGN; pending
independent architecture review. GH #257.

Extends ADR-0054 (ProbeRunner), ADR-0055/0080 (probe-role lifecycle), and
ADR-0088/0089 (VM guest addressing and provision-before-start ordering).

## Context

`ProbeRunner` currently translates omitted HTTP hosts and the TCP wildcard
`0.0.0.0` to host loopback inside every probe tick. That is correct for the
existing Exec/process Service path. It is wrong for a VM Service: the listener
runs inside the guest and the action shim has already materialized its routed
guest address as `AllocationSpec.workload_addr`.

The production owner path is:

1. `provision_and_inject_netns` always assigns a network for `DriverType::Vm`;
2. `inject_workload_network` writes `Some(tap.guest_addr)` to the allocation
   spec;
3. the selected VM driver starts and completes its Beacon contract;
4. the action shim commits the Running observation; and
5. only then it calls `Driver::on_alloc_running(&spec)`.

Native-metal spike H6 independently proved that a host-originated TCP
connection reaches this address through the production veth/netns/TAP path.
The spike establishes reachability, not component ownership.

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
| Non-wildcard explicit host | Any | Explicit host unchanged |
| Omitted HTTP host or `0.0.0.0` | VM | That allocation's provisioned `workload_addr` |
| Omitted HTTP host or `0.0.0.0` | Exec/process | `127.0.0.1` |

The declared `ProbeDescriptor` remains persisted intent. Effective runtime
targets are private task inputs and are not written back to intent or
observation schemas.

The already-trusted production `Arc<ProbeRunner>` is shared with `VmDriver` as
a required constructor dependency in this exact order:

```rust
pub fn new(
    vmm: Arc<dyn Vmm>,
    clock: Arc<dyn Clock>,
    fs: Arc<dyn CgroupFs>,
    cgroup_accounting: Arc<dyn CgroupAccounting>,
    probe_runner: Arc<ProbeRunner>,
    layout: VmHostLayout,
) -> Self;
```

`VmDriver` overrides the same three existing `Driver` hooks as `ExecDriver`:
Running registers all roles, Stable stops Startup only, and terminal stops the
whole allocation supervisor. No new `Driver` method, optional builder, or
VM-specific probe runner is introduced.

### VM address precondition

`Some(workload_addr)` is a precondition for VM probe registration, guaranteed
by the production path above. `Vm + None` is representable only because the
shared `AllocationSpec` also models allocations for which the field can be
absent; it has no producer on the real VM `serve` + `deploy` path.

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
- Positive: explicit-host and Exec/process behavior remain unchanged.
- Positive: no new storage, adapter, protocol, lifecycle state, or dependency.
- Negative: `ProbeRunner::start_alloc` now accepts the full allocation spec,
  and every bounded call site must migrate in one cut.
- Negative: `VmDriver::new` gains one mandatory dependency, requiring neutral
  compiler fallout at its existing construction sites.

## Evidence obligations

- Pure projection properties over supported production inputs: explicit host,
  VM default with provisioned address, and Exec/process default.
- Component evidence that both TCP and HTTP adapters receive the projected
  destination while the stored descriptor is unchanged.
- Lifecycle evidence that Running is unaffected by failing guest probes and
  Stable/readiness/liveness retain their existing owners.
- Native-metal, built-default-feature `serve` + `deploy` evidence through the
  production TAP/netns path; no test-installed address, route, or listener.
- Existing terminal-wins-over-late-probe invariant. No `Vm + None` scenario.

## References

- `docs/feature/service-kind-vm-workloads/feature-delta.md`
- `docs/feature/service-kind-vm-workloads/design/wave-decisions.md`
- `docs/feature/service-kind-vm-workloads/spike/findings.md` (H6)
- ADR-0054, ADR-0055, ADR-0080, ADR-0088, ADR-0089
