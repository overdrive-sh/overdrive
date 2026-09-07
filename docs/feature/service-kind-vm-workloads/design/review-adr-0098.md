# DESIGN review — ADR-0098

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` (GH #257) |
| ADR | ADR-0098 — Terminal netns cleanup recovers an unheld allocation slot from its observed network address |
| Review type | Independent DESIGN review |
| Review history | Iteration 1 — APPROVED |
| Current verdict | **APPROVED** |

## Scope and evidence

This review covers only the proven native-metal E10 terminal-cleanup leak and ADR-0098's narrowly specified correction. It reviewed the accepted ADR, the current VM-workload feature design, ADR-0088 and ADR-0089's C3 addressing and ownership decisions, the E10 black-box expectation and captured product run, and the production action-shim and veth-provisioner paths.

The defect is reachable through the real product owner path, rather than a test-only cancellation or fabricated host artifact. The captured built-product run reaches the failed startup-probe terminal and reports an `ovd-hv-*` veth and `ovd-ns-*` namespace as the two unexpected network residues ([E10 product capture](../../../../verification/expectations/E10-vm-service-http-cross-driver-status/evidence/product-run.out), lines 48–51). The checked-in example starts `overdrive serve`, deploys the checked-in Service, waits for its terminal state, and only then applies its ordinary cleanup; its cleanup oracle only compares before/after host network snapshots and does not create or remove a network fixture ([`run-example.sh`](../../../../examples/service-kind-vm-workloads/run-example.sh), lines 132–171, 350–365, 386–456).

The complete current production path supports the ADR's premise:

1. `provision_and_inject_netns` assigns the allocation slot, derives the existing plan, provisions it before driver start, and stores either the transit `workload_addr` for allocation-network Exec or the guest address for VM ([`action_shim/mod.rs`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs), lines 1188–1250).
2. `FinalizeFailed` reads that prior row, keeps `Stable` non-destructive, and for a genuine terminal awaits intercept stop, invokes network teardown, invokes the driver terminal hook, and only then writes the terminal fence (lines 1544–1769). `StopAllocation` has the same cleanup-before-terminal ordering (lines 2691–2835).
3. The present private teardown helper takes only an allocator snapshot and returns success when that ephemeral `AllocationId -> NetSlot` binding is absent (lines 1391–1417). That is the reached no-op which leaves the provisioned structural complement behind.

## Assessment

### D1 — precise address-to-slot fallback

**Approved.** ADR-0098 retains the allocator binding as the normal source and uses a prior `workload_addr` only when that binding is absent. The two accepted address forms exactly match the existing injection branches: the transit address for allocation-network Exec and the guest address for VM. The existing plans derive those values from the same slot before injection; no declared probe target or operator input is being repurposed as ownership evidence.

The specified crate-private pure helper has the necessary exactness boundary: it accepts only a valid slot in one of the two established /30 carves, at the second usable-host offset, and re-derives the address before accepting it. This aligns with the existing `NetSlot` range validation and with the transit and VM plan derivations ([`veth_provisioner.rs`](../../../../crates/overdrive-control-plane/src/veth_provisioner.rs), lines 711–740, 816–857, 961–1044). It creates no action, persistence, wire, trait, port, or operator API surface; the only changed call shape is the named private teardown helper.

### D2 — cross-allocation safety and teardown ownership

**Approved.** The fallback requires one snapshot to establish all three facts: A has no binding, the observed address inverses exactly to a valid slot, and no different allocation owns that slot. A conflicting binding therefore keeps the existing no-op and cannot call the provisioner or release B. On the eligible path, the existing plan remains the sole source of resource names and the existing provisioner remains the sole structural cleanup owner.

The release ordering is also correct: teardown occurs first and `release(A)` happens only after it succeeds. `NetSlotAllocator::release` is an allocation-keyed idempotent map removal, while existing structural teardown already handles benign absence and cleans the slot's namespace, stranded platform TAP, veth, and resolver complement ([`veth_provisioner.rs`](../../../../crates/overdrive-control-plane/src/veth_provisioner.rs), lines 961–1044, 2191–2212). Thus a fallback for A cannot free B's allocation binding.

### D3 — lifecycle, idempotency, error, and persistence compatibility

**Approved.** The correction is applied only to genuine terminal arms which already have a prior row. `Stable` returns after its non-terminal write and never reaches structural cleanup; pre-Running and unnetworked Exec cases have no accepted observed address and retain the current no-op behavior. Start and restart abort paths pass `None`, so they retain allocator-only cleanup.

ADR-0098 preserves the present awaited order: intercept stop, network cleanup, driver terminal hook, then durable terminal write. An intercept-stop error or cancellation returns before cleanup and the terminal fence; a teardown error does not release a held binding or write the fence, so the existing level-triggered terminal action can re-drive it. A successful teardown followed by a write failure remains governed by the existing replay/authorship behavior. No retry owner, detached task, recovery record, lifecycle state, or persistence schema is introduced. The prior address is read from the existing `Running` row and the resulting genuine-terminal row still deliberately writes no live backend address ([`action_shim/mod.rs`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs), lines 1583–1690).

### D4 — evidence boundary

**Approved.** The required action-shim test exercises the real dispatcher and provisioner seam through the actual `Running -> FinalizeFailed` transition, then models precisely the proven missing ephemeral binding while retaining the existing row's assigned address. It requires both address carves, the exact derived plan, pre-commit structural cleanup, held-slot preservation, Stable, unrecognised address, cross-allocation conflict, teardown failure/redrive, and the existing awaited cancellation behavior. That keeps internal lifecycle proof inside Rust tests.

The required E10 run remains a separate black-box layer: the default-feature binary is driven through `serve` and `deploy` over every existing Exec/VM × 204/302/404/503 cell, while the runner merely observes a zero host-network delta. This preserves the repository's required integration/expectation boundary and does not turn the expectation into a cleanup surrogate.

## Findings

| ID | Severity | Status | Disposition |
| --- | --- | --- | --- |
| None | — | — | No blocking or non-blocking design finding. |

The review considered the snapshot-based conflict check against the actual current action dispatch ownership. Ordinary convergence actions are dispatched sequentially in their owning loop, and no current first-party workflow emitter creates a concurrent allocation-start path. No reachable production path was shown in which another allocation can acquire the derived slot between the specified snapshot and teardown. Under the repository's evidence rule, that is not a current defect or a reason to widen this narrow correction.

## Verification

This is a DESIGN review; no implementation, production test, expectation, runner, proposal, or roadmap was modified. The review verified that ADR-0098 names an exact private API shape, preserves the current owner and terminal ordering, and requires the necessary implementation and black-box evidence in its D4 obligations. The review artifact itself passes whitespace validation with `git diff --check`.

## Iteration 1 disposition and verdict

| Item | Disposition |
| --- | --- |
| Proven E10 cleanup premise | Confirmed. |
| D1 exact private fallback and API shape | Approved. |
| D2 cross-allocation guard and release order | Approved. |
| D3 lifecycle, failure, cancellation, and persistence boundary | Approved. |
| D4 independent test and expectation obligations | Approved. |

**APPROVED.** ADR-0098 corrects the proven missing-ephemeral-binding terminal teardown with the minimum existing ownership evidence. It is sufficiently precise to protect a different allocation's slot, preserves the established lifecycle and error semantics, and does not expand the architecture or public contract.
