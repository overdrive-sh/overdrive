# ADR-0098 — Terminal netns cleanup recovers an unheld allocation slot from its observed network address

## Status

**Accepted** (2026-09-07). Focused, user-authorized DESIGN correction for the
proven E10 terminal-cleanup defect. It extends the existing C3 terminal
teardown implementation; it does not introduce a recovery subsystem, a new
owner, persistence, an action, a terminal condition, a lifecycle state, or a
retry protocol.

Depends on ADR-0088 (the two slot-derived address carves), ADR-0089 (C3 owns
structural provisioning and teardown), ADR-0096 (the existing
`StartupProbeFailed` terminal), and ADR-0097 (the E10 status matrix).

## Context

The built-default-feature native-metal E10 product run reached the existing
`StartupProbeFailed` / `FinalizeFailed` path and then failed the existing
zero-delta cleanup oracle. Its captured output names the residual resources:

```text
E08 teardown deltas: vm=0 probe=0 network=2 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0
ovd-hv-0000@<metal-host-redacted> ...
ovd-ns-0000 (id: 0)
```

(`verification/expectations/E10-vm-service-http-cross-driver-status/evidence/product-run.out:48-51`.)
The failure is reached through the real product owner path: E10 starts an
isolated `overdrive serve`, deploys its checked-in failing Service, waits for
the existing `StartupProbeFailed` terminal, then the checked-in case cleanup
waits for no owned runtime resources before it stops `serve`
(`examples/service-kind-vm-workloads/run-example.sh:386-456`, `350-365`).
The runner does not install a network artifact; it merely compares the
post-cleanup host network snapshot with its pre-run snapshot
(`run-example.sh:132-171`).

The code establishes that the allocation acquired the resource before its
driver started: `provision_and_inject_netns` calls the existing
`NetSlotAllocator::assign(spec.alloc.clone())`, derives the plan, provisions
it, and injects the allocation address (`action_shim/mod.rs:1188-1222`). The
same allocator is an in-memory `AllocationId -> NetSlot` map
(`veth_provisioner.rs:961-1044`).

The Service lifecycle's existing terminal action reaches the action shim in
the same process. `FinalizeFailed` reads its prior row, distinguishes only
`Stable` as non-destructive, and for every genuine terminal awaits
`stop_alloc`, calls `teardown_and_release_netns`, invokes the driver terminal
hook, and only then writes the durable terminal fence
(`action_shim/mod.rs:1544-1769`). `dispatch_with_network_provisioner` awaits
actions in vector order and records an action error while continuing a batch
(`action_shim/mod.rs:904-957`).

The defect is at the existing structural teardown helper. It looks only in
the allocator snapshot and returns `Ok(())` if that ephemeral map contains no
binding for the allocation (`action_shim/mod.rs:1391-1407`). In the proven
E10 terminal, that branch is taken even though start assigned slot zero; the
existing named netns and host veth consequently remain. This is not a
hypothetical cancellation state: the real product evidence above observes the
residue after the terminal path and its ordinary cleanup wait.

The prior `AllocStatusRow` still carries the allocation's observed
`workload_addr` while it is `Running`; the terminal row deliberately removes
that address because dead allocations are not backends
(`action_shim/mod.rs:1583-1690`). That pre-terminal value is already a
slot-derived C3 output, not an operator-supplied destination: Exec receives
the transit address and VM receives the guest address
(`action_shim/mod.rs:1214-1250`; ADR-0088 §§1-3). Each is an exact inverse of
one of ADR-0088's disjoint /30 carves. It is therefore sufficient existing
ownership evidence for this narrow terminal cleanup, where the ephemeral
allocator record is absent.

## Decision

### D1 — Resolve the terminal teardown slot from the allocation binding first, then from the pre-terminal observed address

The existing private action-shim teardown helper keeps its allocator binding
as the first and ordinary source of the slot. When that binding is absent, the
helper receives the already-read prior row's `workload_addr` and performs one
crate-internal, pure inverse projection:

```rust
fn slot_from_assigned_workload_addr(address: Ipv4Addr) -> Option<NetSlot>
```

This is a `pub(crate)` veth-provisioner helper only; it is neither re-exported
nor a port, trait, operator API, wire type, persistence type, or new public
surface. It returns a slot only when `address` is exactly the second usable
address of one valid slot in either existing address carve:

| Existing allocation form | Accepted observed address | Derived teardown plan |
|---|---|---|
| allocation-network Exec | `derive_workload_netns_plan(slot, responder).workload_addr` | the existing transit netns/veth plan |
| VM | `derive_vm_tap_plan(slot, responder).guest_addr` | the same existing transit netns/veth plan; its existing teardown reaps the slot's TAP too |
| any other address or `None` | none | no fallback teardown |

The inverse must validate the fixed base, the relevant carve, the exact
second-usable-host offset, range `0..=NET_SLOT_MAX`, and the re-derived
address. It must not accept a network address, gateway, broadcast, arbitrary
mesh address, explicit probe target, or a rounded/truncated value. The
existing plan derivation remains the sole source of names, subnet, addresses,
TAP name, route, and resolver directory; the correction adds no plan field or
parallel derivation.

The private teardown call shape is extended only with this existing transient
input:

```rust
fn teardown_and_release_netns_raw(
    alloc_id: &AllocationId,
    prior_workload_addr: Option<Ipv4Addr>,
    net_slot_allocator: &NetSlotAllocator,
    network_provisioner: &dyn WorkloadNetworkProvisioner,
) -> Result<(), VethProvisionError>
```

`FinalizeFailed` and `StopAllocation` already read the prior row before
terminal cleanup and pass its `workload_addr`; start/restart-abort and
post-assignment provision-failure callers pass `None` and retain their current
allocator-only behavior. No `Action` shape changes.

### D2 — Fallback is allocation-safe and does not clean another allocation's slot

The fallback is eligible only when all of the following hold:

1. the snapshot has no binding for `alloc_id`;
2. the prior row's address passes D1's exact inverse projection; and
3. the same snapshot has no binding of that derived slot to a different
   allocation.

If another allocation holds the slot, or if the address cannot be proved to
be a C3-assigned address, the helper retains the current benign no-op. It must
not call the network provisioner, release the other allocation, or synthesize
a slot from an allocation id. This is the boundary that makes cross-allocation
teardown impossible: a missing record for A cannot authorize deletion of a
slot known to be held by B.

On an eligible fallback, the helper invokes the existing
`WorkloadNetworkProvisioner::teardown` with the re-derived plan and calls the
existing `NetSlotAllocator::release(alloc_id)` only after that teardown
succeeds. `release(A)` remains an idempotent removal of A's possibly-absent
binding; it never frees or mutates B. The existing provisioner retains its
complete structural cleanup semantics: it deletes the netns, the stranded
platform TAP when applicable, host veth, and resolver directory, with benign
absence idempotence (`veth_provisioner.rs:2191-2212`).

### D3 — Existing terminal ordering, failure, cancellation, and state rules remain unchanged

This correction applies only to a genuine terminal. `Stable` remains a
non-terminal success that keeps `Running`, its address, the slot, and its
network intact; it does not enter the fallback. An uninitialized/pre-Running
allocation has no prior running address and preserves the existing no-op
behavior. An unnetworked Exec allocation likewise carries no accepted C3
address and preserves existing behavior.

The existing order remains binding:

```text
await existing intercept stop (when composed)
  -> terminal netns/veth/TAP teardown
  -> existing driver terminal hook
  -> durable terminal row write
```

If `stop_alloc` returns an error or is cancelled at its existing `await`, the
function returns before structural teardown and before the terminal write;
this ADR adds no detached task, timeout, cancellation branch, or false
completion. If network teardown returns its existing typed error, the slot is
not released and the durable terminal fence is not written, exactly as today;
the level-triggered existing action path can re-drive the same terminal and
the same address-derived cleanup. A successful teardown followed by a terminal
row write failure retains the existing authorship/replay behavior. No new
retry, recovery, adoption, garbage collection, or persistence mechanism is
created.

### D4 — Evidence obligations

DELIVER must provide these independent evidence layers:

1. An action-shim acceptance test drives the real dispatcher/provisioner seam
   through a `Running` allocation and a genuine `FinalizeFailed`, then models
   the proven missing allocator binding while retaining that row's observed
   assigned address. It proves the existing network provisioner is called once
   with the exact slot-derived plan and its owned netns/veth/TAP complement is
   gone before the terminal row commits. Cover one allocation-network Exec
   transit address and one VM guest address; every transitioned test carries
   its required Contract Shape declaration.
2. The same test boundary proves the normal held-slot case is unchanged,
   `Stable` performs no structural cleanup, an unassigned/unrecognised address
   performs no cleanup, and an address resolving to a slot held by a different
   allocation cannot trigger a teardown or release for that other allocation.
3. A teardown failure at the existing provisioner seam leaves the terminal
   uncommitted and does not release a held binding; a subsequent ordinary
   re-drive is the only retry behavior asserted. Existing cancellation behavior
   remains covered by the awaited `MtlsInterceptLifecycle::stop_alloc` path;
   no test may manufacture a detached cleanup completion.
4. The built default-feature E10 expectation reruns all eight existing
   Exec/VM × 204/302/404/503 cells through `overdrive serve` and
   `overdrive deploy`. The existing case cleanup's host-network delta is zero
   for each cell, including each `StartupProbeFailed` cell; every E10 ledger
   row retains `cleanup=zero-delta`. The runner remains black-box and installs
   no netns, veth, TAP, route, address, mark, listener, or cleanup surrogate.

## Alternatives considered

1. **Persist a slot-to-allocation binding or add a recovery record — rejected.**
   The terminal already has an observed, exact C3 address sufficient for this
   path. Durable ownership state would expand the lifecycle and recovery model
   to solve one missing ephemeral lookup.
2. **Scan host namespaces at every terminal — rejected.** The boot-only
   adopt-or-GC observer has a different owner and an intentionally broader
   crash/restart contract. Reusing it here would add filesystem observation,
   PID correlation, and a second cleanup authority to a normal terminal.
3. **Derive a slot from `AllocationId` or delete by name without evidence —
   rejected.** Allocation ids do not encode slot ownership. Such a guess can
   delete another allocation's resources.
4. **Release before teardown — rejected.** It recreates the existing
   released-but-undestroyed leak window. The existing teardown-before-release
   order remains unchanged.
5. **Change E10 cleanup to remove residual artifacts — rejected.** The
   expectation is an observer. Removing the artifacts there would mask the
   missing production cleanup and violate its black-box boundary.

## Consequences

The terminal path can complete its existing allocation-owned structural
cleanup when its sole ephemeral ownership entry has gone missing but the prior
Running row still proves the exact assigned address. Healthy `Stable`
allocations, unnetworked allocations, unmatched addresses, and slots held by
another allocation retain their current behavior. The implementation remains a
small extension of the existing action shim and veth provisioner, with no new
component, C4 edge, persistence migration, or operator contract.

## References

- ADR-0088, ADR-0089, ADR-0096, ADR-0097
- `crates/overdrive-control-plane/src/action_shim/mod.rs:1188-1222,1391-1417,1544-1769,2691-2819`
- `crates/overdrive-control-plane/src/veth_provisioner.rs:961-1044,711-740,816-857,2191-2212`
- `examples/service-kind-vm-workloads/run-example.sh:132-171,350-365,386-456,511-547`
- `verification/expectations/E10-vm-service-http-cross-driver-status/runner.sh`
