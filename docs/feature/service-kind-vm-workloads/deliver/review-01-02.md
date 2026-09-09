# DELIVER review — 01-02 ProbeRunner TCP projection

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Step | `01-02` — ProbeRunner TCP projection |
| Reviewed commit | `058344a9bb271911655fac9ba6d270ad05942843` |
| Base commit | `de34682c7fb9f9500c1757a994a285c580fa3813` |
| Iterations | 1–2 |
| Reviewer | Fresh independent implementation reviewer |
| Verdict | **APPROVED** |

## Scope and accepted contract

The review is limited to the 01-02 commit and ADR-0090 / DESIGN ACD-1. The required public shape is exactly `ProbeRunner::start_alloc(&AllocationSpec) -> CancellationToken`; the former `(alloc_id, descriptors)` form must be absent. VM TCP wildcard targets are projected once to the provisioned `workload_addr`; Exec defaults, explicit hosts, and persisted descriptors retain their existing semantics. `Vm + None` has no behavior or test. A TCP health observation must not own `Running`.

The dirty worktree contains feature design/roadmap updates and unrelated files. They were not reviewed as part of commit `058344a9` and were not modified by this review.

## Iteration 1 evidence

### Design and API shape

- `ProbeRunner::start_alloc` has the accepted single-argument signature at `crates/overdrive-worker/src/probe_runner/mod.rs:298`; repository call-site search found no retained old ProbeRunner registration form.
- `ExecDriver::on_alloc_running` forwards the complete spec at `crates/overdrive-worker/src/driver.rs:828-831`; the core Driver-hook documentation was updated consistently at `crates/overdrive-core/src/traits/driver.rs:856-873`.
- The registration loop clones the declared descriptors, changes only a VM TCP `0.0.0.0` host, and passes that private clone to the task at `probe_runner/mod.rs:319-364`. The caller-owned/persisted descriptor therefore remains unchanged.
- The explicit-host and Exec paths do not satisfy the VM-wildcard guard. Existing tick dispatch continues to normalize non-VM wildcard TCP to loopback at `probe_runner/mod.rs:513-521`; HTTP's established omitted/wildcard loopback mapping remains at `probe_runner/mod.rs:448-458` and is outside this TCP-only change.
- The code has no fallback, row, state, or test for `Vm + None`; its `expect` records the accepted production precondition at `probe_runner/mod.rs:324-331`.

### Lifecycle and production-path check

The relevant owner path is action shim `StartAllocation`: it writes the successful `Running` row before calling `Driver::on_alloc_running` (`crates/overdrive-control-plane/src/action_shim/mod.rs:2004-2027`, `2146-2203`). The existing Exec driver then invokes `ProbeRunner::start_alloc` (`crates/overdrive-worker/src/driver.rs:819-831`), whose task only calls `probe_tick` and writes `ProbeResultRow` (`crates/overdrive-worker/src/probe_runner/mod.rs:351-364`, `491-595`). It has no allocation-status writer or lifecycle action. Thus TCP pass/fail cannot own `Running` on the current production owner path.

VM-driver delegation and composition-root injection are deliberately step 01-03 work. Their absence from this commit is not a 01-02 finding; the projection is correctly placed at the existing registration boundary for that subsequent wiring.

### Tests and DES record

- The 01-02 RED, GREEN, and COMMIT events are present and PASS in `deliver/execution-log.json`.
- The two transitioned scenario tests replaced only their RED scaffolds with behavioral assertions; comparison to the parent commit found no weakened or deleted assertion.
- Test budget: two roadmap scenarios (S-SVM-12 and S-SVM-13), maximum four tests; this commit transitions two scenario tests and adds one property, for three tests total.
- The scoped required command passed: `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(exec_default_network_probe_targets_remain_loopback) or test(vm_tcp_probe_records_guest_connect_outcome_without_owning_running)'` — 2 passed.
- The added TCP projection property also passed independently — 1 passed.
- `git diff 058344a9^ 058344a9 --check` returned clean.

## Findings

| ID | Severity | Evidence | Required remediation |
|---|---|---|---|
| R01 | Blocker — Contract Shape declaration | `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:105-148` adds the live proptest `vm_wildcard_tcp_target_projects_each_provisioned_guest_addr`, but its rustdoc contains no `CONTRACT_SHAPE` declaration. The repository rule requires every new or transitioned test to declare its contract shape; the scenario is a stateful/projection test, not a source-local pure function. | Add the exact declaration matching the accepted scenario contract: `/// CONTRACT_SHAPE: bounded-change.` immediately before the property test. Do not alter production behavior, add API, or introduce a `Vm + None` case. |

### Finding reachability

R01 is a mechanical test-contract finding, not a claim of a new runtime failure. The property it leaves unclassified exercises the live registration API at `ProbeRunner::start_alloc` (`probe_runner/mod.rs:298`) and captures the host passed to the TCP adapter after its supervised task executes (`service_kind_vm_workloads.rs:105-148`). That API is reached in production after the action-shim `Running` write through `Driver::on_alloc_running` as traced above. No speculative lifecycle, shutdown, retry, or VM-without-address path is asserted.

## Remediation disposition

R01 is necessary and bounded: it restores the mandated metadata on an already-added test. It does not change the accepted API, target-projection mechanism, persistent descriptor, lifecycle ownership, or architecture. No other remediation is requested.

## Iteration 1 conclusion

The implementation matches the accepted API shape and target-projection/lifecycle boundaries, and the focused verification is green. It cannot be approved yet because the new live property lacks its required per-test Contract Shape declaration. Return R01 to the original 01-02 crafter; after that metadata-only correction, rerun the scoped test and request iteration-2 review.

## Iteration 2 — remediation review

### Evidence

- Remediation commit `1d3e5b0383ab284d7dc1142d7eb9c65916e4cbae` changes exactly one line in `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs`; it introduces no production, API, lifecycle, descriptor, or `Vm + None` behavior.
- The live property now has the exact required declaration at `service_kind_vm_workloads.rs:105-108`: `/// CONTRACT_SHAPE: bounded-change.`
- The combined focused verification passed: `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test acceptance -E 'test(exec_default_network_probe_targets_remain_loopback) or test(vm_tcp_probe_records_guest_connect_outcome_without_owning_running) or test(vm_wildcard_tcp_target_projects_each_provisioned_guest_addr)'` — 3 passed.
- `git diff 1d3e5b03^ 1d3e5b03 --check` returned clean.

### Remediation disposition

| Finding | Disposition | Evidence |
|---|---|---|
| R01 | Resolved | The required bounded-change declaration is immediately adjacent to the property test, and the test remains green. The commit is metadata-only and remains within the original 01-02 contract. |

## Final verdict

**APPROVED.** Iteration 2 resolves the sole bounded blocker without scope expansion. The 01-02 implementation and its remediation now satisfy the accepted API shape, target-projection boundary, lifecycle ownership constraint, and test Contract Shape requirement.
