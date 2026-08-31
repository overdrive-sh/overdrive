# DISTILL test scenarios — bounded lifecycle/network correction

**Feature:** `guest-stack-transparent-mtls-intercept` (GH #222)

**Design base:** `465b96c39083984a1d2d470caff918a723b9301f`

**Scope:** BTR-1, BTR-2, and BTR-3 only.

This is specification prose; the repository forbids executable `.feature`
files. The executable handoff is three Rust RED scaffolds in
`crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs`.
They will exercise the existing production `action_shim::dispatch` boundary.
No expectation, example bundle, shell/Python runner, public API, or test-only
production observer is added.

## Prior-wave reading and reconciliation

| Input | Result |
|---|---|
| `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml` | Read; prior interception must remain fail-closed and allocation-owned. |
| `docs/product/journeys/hold-identity-for-the-running-set.yaml` | Read; stopping still drops allocation identity through the existing lifecycle. |
| `docs/product/journeys/run-a-vm-workload.yaml` | Read; its older retry wording remains a separately owned stale product statement and is not changed by this internal correction. |
| `docs/product/architecture/brief.md` | Read at the guest-stack extension and lifecycle/network amendment; no new driving adapter or public observable is introduced. |
| `docs/product/kpi-contracts.yaml` | Read; it is scoped to `docs-platform`, so no KPI tag or edit applies here. |
| `discuss/{user-stories,story-map,wave-decisions}.md` | Not found; warning only. This feature entered through SPIKE and DESIGN. |
| `spike/findings.md`, `spike/wave-decisions.md` | Read; the spike verdict is WORKS and the probe was discarded into DESIGN, so no walking skeleton was promoted for DISTILL to alter. |
| `design/wave-decisions.md`, `design/upstream-changes.md` | Read; the latter is the exact downstream contract for these scenarios. |
| `devops/wave-decisions.md` | Not found; warning only. Default clean/stale/pre-commit environment variants are irrelevant to these deterministic in-process ordering tests. |
| `.nwave/des-config.json` | Read; deliverable type is unresolved and therefore routes as `application`. User direction disables reviewer, mutation, roadmap, expectation, and example work for this pass. |

**Reconciliation passed — 0 contradictions within BTR-1..3.** The three
corrections preserve the existing product journeys and public behavior while
closing internal termination/resource-ordering gaps. The stale VM-journey
retry sentence predates this amendment and remains outside the approved scope;
it does not create ambiguity in any BTR scenario.

## Scope fence

The scenarios may change only these existing production paths:

1. the post-cleanup terminal proposal loop in `StopAllocation`;
2. the unwind after `NetSlotAllocator::assign` succeeds and provisioning fails;
3. the same-id `RestartAllocation` order between prior-driver stop and
   replacement provisioning.

They must not require WorkloadLifecycle or ServiceLifecycle replay, terminal
tail targets, generation fences, route views/hydration, broker self-enqueue or
relist, liveness-attempt state, `ProbeResultRow` V2, probe/driver signature
propagation, arbitrary cancellation recovery, receipts/outboxes, detached
completion, expanded boot GC, or `RestartNetworkDisposition`.

## Scenario list

| ID | Tags | Contract shape | Rust handoff |
|---|---|---|---|
| S-GTI-BTR-01 | `@driving_port @in-memory @error` | bounded-change | `stop_allocation_second_lww_rejection_completes_without_event` |
| S-GTI-BTR-02 | `@driving_port @in-memory @error @cleanup` | bounded-change | `post_assignment_provision_failure_tears_down_before_slot_release` |
| S-GTI-BTR-03 | `@driving_port @in-memory @error @ordering` | bounded-change | `same_id_restart_removes_prior_protection_before_replacement_provision` |

This bug-fix amendment adds no walking skeleton. The feature's checked-in VM
example and sole pending E07 expectation remain byte-for-byte untouched. A
built-binary or real-kernel test would add cost without improving proof of
these action-shim ordering contracts.

## S-GTI-BTR-01 — two losses exhaust the Stop proposal bound

```gherkin
@driving_port @in-memory @error @contract-shape:bounded-change
Scenario: Stop finishes after the exit observer wins both terminal proposals
  Given StopAllocation has completed driver, mTLS, and structural-network cleanup
    And the allocation has one process-local driver route
    And the observation boundary rejects two fresh terminal proposals as LWW losers
  When action_shim dispatches the StopAllocation action
  Then exactly two terminal proposals are attempted
    And each proposal was derived from a fresh current row with a dominating timestamp
    And dispatch returns success after the second rejection
    And driver supervision is released exactly once
    And the process-local driver route is removed
    And no lifecycle occurrence is broadcast
    And the competing accepted row remains the durable truth
```

The Rust case table also pins the total outcomes:

| Partition | Required result |
|---|---|
| first proposal `Ok(Some)` | existing accepted tail: one release, route removal, one best-effort event |
| first `Ok(None)`, second `Ok(Some)` | one rebase, then the same accepted tail |
| fresh read already equals the requested terminal | no proposal/event; one release and route removal |
| two `Ok(None)` results | success; one release, route removal, no event; no third proposal |
| point-read or compound-write error | existing typed error and atomic no-partial-commit behavior |

The fixture is a test-owned `ObservationStore` decorator over
`SimObservationStore`. It may script the two legal `Ok(None)` responses and
fail immediately if a third proposal reaches the port. It does not introduce a
retry clock, sleep, configuration knob, cancellation partition, or production
seam.

## S-GTI-BTR-02 — post-assignment provision failure is fully unwound

```gherkin
@driving_port @in-memory @error @cleanup @contract-shape:bounded-change
Scenario: A provision failure cleans its assigned structural network before closure
  Given a network slot is available for an allocation
    And the production allocator assigns it before the network provisioner fails
  When action_shim dispatches StartAllocation or RestartAllocation
  Then the existing allocation-keyed structural teardown is attempted once for that failed assignment
    And the Failed row keeps the original WorkloadNetnsProvisionFailed cause
    And successful teardown releases the slot only after teardown returns
    And failed teardown retains the slot and returns its existing typed cleanup error
```

The Rust case table covers both action arms and these complements:

| Partition | Call/order and result |
|---|---|
| slot exhaustion before assignment | no provision teardown; existing failure disposition |
| assigned, provision fails, teardown succeeds | `assign -> provision(Err) -> teardown(Ok) -> Failed write`; slot absent; `Ok(())` after the write |
| assigned, provision fails, teardown fails | `assign -> provision(Err) -> teardown(Err) -> Failed write`; slot retained; cleanup `ShimError` after a successful write |
| cleanup fails and Failed write fails | both attempted; the existing observation/store error has return precedence |

The port-observable universe is the provisioner call trace, allocator snapshot,
dispatch result, current allocation row, and lifecycle occurrence. The test
must not infer filesystem residue from an invented path-state model. Benign
absence remains whatever the existing provisioner already defines as success.

## S-GTI-BTR-03 — prior protection ends before replacement work

```gherkin
@driving_port @in-memory @error @ordering @contract-shape:bounded-change
Scenario: A same-id replacement starts only after all prior protection is gone
  Given a running allocation owns driver supervision, mTLS interception, and a structural network slot
  When action_shim dispatches RestartAllocation for the same allocation id
  Then every resolved prior driver stop completes or reports typed NotFound
    And prior mTLS listener, rule, and connection teardown completes next
    And prior structural network teardown completes and releases its slot next
    And only then may replacement provisioning begin
    And replacement identity work follows provisioning
    And replacement driver start follows identity work
```

The activated Rust test records the existing port calls in one ordered trace:

```text
prior-driver-stop(s)
  < prior-mtls-stop-complete
  < prior-network-teardown-and-slot-release
  < replacement-provision
  < replacement-identity
  < replacement-driver-start
```

Test-owned implementations of the existing `Driver`, mTLS driven ports, and
`WorkloadNetworkProvisioner` record the trace. Listener/rule ownership is
observed through the existing port guards and a deliberately delayed mTLS
teardown future; no `*_for_test` method or new public accessor is allowed.

| Partition | Required result |
|---|---|
| prior driver returns a non-`NotFound` error | return existing driver error; no mTLS/network/replacement event |
| prior mTLS teardown fails | return existing mTLS error; no structural teardown or replacement event |
| prior structural teardown fails | retain slot, return existing network error; no replacement event |
| all prior cleanup succeeds | replacement provision/identity/start follow the exact order above |
| replacement fails after assignment | reuse S-GTI-BTR-02's teardown/release/error-precedence contract |

## Test architecture and placement

| Boundary | Treatment | Reason |
|---|---|---|
| Driving port: `action_shim::dispatch` / `dispatch_with_network_provisioner` | real in-process production dispatcher | The defect is call order inside the action executor; helper-only tests could pass while wiring remains wrong. |
| `ObservationStore` | `SimObservationStore` plus a test-local delegating script for exact LWW outcomes | Deterministic compound-write contention without inventing production recovery. |
| `NetSlotAllocator` | real in-memory production allocator | Its public snapshot is the existing slot-ownership observable. |
| `WorkloadNetworkProvisioner` | test-owned implementation of the existing port | Records provision/teardown order and returns sanctioned typed failures. |
| Driver and mTLS boundaries | test-owned implementations of existing ports composed through the real worker/registry | Records completion order; no public or cfg(test) production observation surface. |
| `AllocDriverIndex`, lifecycle bus, durable rows | existing port-visible/local ownership surfaces | Proves the bounded tail and absence of fabricated events. |

The scaffolds live in the closest existing action-shim acceptance module,
`action_shim_crash_observability.rs`, which already owns terminal LWW
contention, network cleanup, driver routing, and real dispatcher fixtures.
Creating another module would duplicate that composition. The tests are
example-based, not property-based: each case is a concrete causal partition
with a fixed error-precedence/order oracle, and generated schedules would make
the failure less diagnostic.

## RED-scaffold handoff

All three Rust functions use the repository's exact
`#[should_panic(expected = "RED scaffold")]` convention and a single
`panic!("... RED scaffold ...")` body. Imports and production signatures
already exist, so the suite is RED-ready rather than broken. DELIVER must:

1. replace each panic body with the specified real-dispatch composition;
2. remove `#[should_panic]` as the corresponding implementation turns green;
3. transition or delete inherited tests that assert the superseded
   `RestartNetworkDisposition::RetainForRetry` behavior; and
4. add no public/test-only seam to make the assertions possible.

`red-classification.md` records the immutable baseline and targeted runner
evidence. No mutation run, reviewer, expectation, example, or roadmap is part
of this user-approved DISTILL pass.

## Canonical AT-completeness audit

This audit is scoped to the three bounded corrections, not to the already
delivered guest-stack feature.

| Item | Verdict | Evidence |
|---|---|---|
| C1a zero/min | PASS | BTR-1 covers zero proposals on exact-terminal read and one accepted proposal; BTR-2 covers no cleanup before assignment. |
| C1b boundary | PASS | BTR-1 pins exactly two proposals and makes a third illegal; BTR-2 distinguishes pre/post assignment. |
| C2a state/order model | PASS | Each scenario names the legal mutation sequence; BTR-3 provides the complete happens-before trace. |
| C2b illegal transition | PASS | BTR-3 forbids provision/identity/start after each earlier failure; BTR-1 forbids a third proposal/event. |
| C3 0/1/N cardinality | PASS | BTR-1 covers 0/1/2 proposals; BTR-3 covers all resolved driver stops, including typed absence. |
| C4a apply twice/idempotency | PASS | BTR-1's exact-terminal read is the replay/no-op partition; allocation-keyed teardown retains existing benign-absence semantics. |
| C4b inverse without prerequisite | PASS | BTR-2 slot exhaustion makes cleanup a no-op before ownership exists. |
| C5a mode combinations | N/A | The correction introduces no mode or feature flag. |
| C5b orthogonality | N/A | No independent mode axes exist. |
| C6a malformed input | N/A | No new input or decoding boundary is introduced. |
| C6b each declared error | PASS | Read/write, driver, mTLS, provision, teardown, slot-exhaustion, and Failed-write partitions are mapped. |
| C6c closed error set | PASS | Existing typed errors only; scenarios reject aggregate/new recovery errors. |
| C7a degraded resource | PASS | Provision, teardown, mTLS, driver, and observation failures each fail at their existing boundary. |
| C7b interruption mid-operation | N/A | Production shutdown joins active dispatch; arbitrary cancellation was explicitly rejected by DESIGN. |
| C7c concurrent actors | PASS | BTR-1 models the concrete exit-observer writer that contends with Stop. |

**Result: 11 PASS, 4 justified N/A, 0 gaps for the approved scope.**
