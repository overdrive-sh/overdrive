<!-- markdownlint-disable MD024 -->

# Test Scenarios — service-vip-allocator

**Wave**: DISTILL
**Feature**: service-vip-allocator
**Date**: 2026-05-14
**SSOT**: ADR-0049 + `docs/feature/service-vip-allocator/discuss/user-stories.md`

All scenarios below are specification-level GIVEN/WHEN/THEN. The DELIVER
crafter translates them into Rust `#[test]` / `#[tokio::test]` functions
per project convention. No `.feature` files.

---

## Driving Ports

Every scenario names its driving port — the entry point through which the
behavior is exercised. This enables port-to-port acceptance tests per the
DISTILL methodology.

| Port | Kind | Location | Exercises |
|---|---|---|---|
| `submit_workload` HTTP handler | Driving (user-facing) | `crates/overdrive-control-plane/src/handlers.rs` | AC-01, AC-02, AC-04, AC-06 |
| `WorkloadLifecycle` reconciler tick | Driving (internal) | `crates/overdrive-control-plane/src/reconcilers/` | AC-03 |
| `PoolAllocator<T>` API | Driven (library) | `crates/overdrive-dataplane/src/allocators/pool.rs` | AC-05 |
| `IntentBackedAllocator<T>` API | Driven (persistence) | `crates/overdrive-dataplane/src/allocators/intent_backed.rs` | AC-01, AC-02, AC-03 |
| TOML parser (serde deserializer) | Driving (user-facing) | `crates/overdrive-core/src/aggregate/` | AC-06 |
| Operator config boot path | Driving (operator-facing) | `crates/overdrive-control-plane/` | Config validation |

---

## AC-01: Platform allocates VIP on submit (happy path)

### S-VIP-01: Submit Service spec without VIP — platform allocates and echoes

**Driving port**: `submit_workload` handler
**Tags**: `@happy_path` `@ac-01` `@kpi:K1`

```gherkin
Given a single-node control plane is running with VIP pool
  configured as `ranges = ["10.96.0.0/24"]` (256 addresses)
And the VIP pool has at least one available address
And a Service spec `frontend.toml` with one `[[listener]]` block
  declaring `port = 8080`, `protocol = "tcp"`, and no `vip` field
When the operator submits the spec via `submit_workload` handler
Then the response status is 200 (or 201)
And the submit echo contains an allocated VIP from the 10.96.0.0/24 range
And the VIP is a valid IPv4 address within the configured range
And the VIP is not in the reserved set
```

**Crafter notes**: Exercise through the `submit_workload` HTTP handler
with a real `LocalStore` (TempDir-backed) and a real
`IntentBackedAllocator<ServiceVip>`. The allocator must be wired into
`AppState` and probed at boot before the handler is exercised.

### S-VIP-02: alloc status renders the same VIP as submit echo

**Driving port**: `submit_workload` handler + `alloc_status` handler
**Tags**: `@happy_path` `@ac-01`

```gherkin
Given the operator has submitted `frontend.toml` via S-VIP-01
And the submit echo returned VIP `V`
When the operator queries `alloc status --job frontend` via the
  alloc-status handler
Then the Service-level VIP in the alloc-status response equals `V`
And the per-listener lines render as `<port>/<protocol>` (no VIP
  per listener — the VIP is Service-level)
```

**Crafter notes**: This is a byte-equality assertion between the
submit-echo VIP and the alloc-status VIP. Both consult
`ServiceVipAllocator::get(&spec_digest)`.

### S-VIP-03: Multi-listener Service gets one VIP shared across listeners

**Driving port**: `submit_workload` handler
**Tags**: `@happy_path` `@ac-01`

```gherkin
Given a Service spec with two `[[listener]]` blocks:
  | port | protocol |
  | 8080 | tcp      |
  | 8443 | tcp      |
When the operator submits the spec via `submit_workload` handler
Then exactly one VIP is allocated at the Service level
And both listeners render as `<port>/<protocol>` without per-listener VIPs
And the Service-level VIP appears once in the response
```

**Crafter notes**: Validates ADR-0049 § 5a — one VIP per Service,
shared across listeners.

---

## AC-02: Idempotent allocation on resubmit

### S-VIP-04: Resubmit byte-identical spec returns same VIP

**Driving port**: `submit_workload` handler
**Tags**: `@happy_path` `@ac-02` `@kpi:K1`

```gherkin
Given the operator submitted `frontend.toml` and received VIP `V`
And `frontend.toml` is byte-identical (same spec digest)
When the operator re-submits `frontend.toml` via `submit_workload`
Then the response returns VIP `V` (same as prior)
And the submit echo is byte-identical to the prior submission's echo
```

**Crafter notes**: Two `submit_workload` calls with the same spec
body. The idempotency is observable through the return value (same
VIP). Internal allocator memo behavior (hit vs fresh allocation) is
verified by property test S-VIP-P03, not here.

### S-VIP-05: Resubmit after restart returns same VIP (persistence)

**Driving port**: `IntentBackedAllocator<ServiceVip>` (bulk_load)
**Tags**: `@happy_path` `@ac-02`

```gherkin
Given an IntentBackedAllocator has allocated VIP `V` for spec
  digest `D` and the allocation is persisted in IntentStore
When the allocator is reconstructed via `bulk_load` from the same
  IntentStore (simulating restart)
Then `allocator.get(&D)` returns `Some(V)`
And `allocator.allocate(D)` returns `V` (same VIP as before restart)
```

**Crafter notes**: Exercises the `bulk_load` → memo reconstruction
path with a real `LocalStore` backed by a `TempDir`. The observable
behavior is that `get` and `allocate` return the same VIP after
restart. Internal counter state is verified by property test
S-VIP-P03.

---

## AC-03: Terminal-state reclamation

### S-VIP-06: Terminal-state transition releases VIP

**Driving port**: `WorkloadLifecycle` reconciler tick
**Tags**: `@happy_path` `@ac-03` `@kpi:K3`

```gherkin
Given a Service `frontend` is running with allocated VIP `V`
And the WorkloadLifecycle reconciler is registered
When the workload transitions to a terminal state (observation row
  updated)
And the reconciler ticks
Then the reconciler emits `Action::ReleaseServiceVip { spec_digest }`
And after action-shim dispatch, `allocator.get(&spec_digest)`
  returns `None`
And the VIP `V` is available for reallocation
```

**Crafter notes**: Requires wiring `WorkloadLifecycle` reconciler
with a `SimObservationStore` that can be driven to terminal state.
The test must observe the `Action::ReleaseServiceVip` emission and
the subsequent allocator state change.

### S-VIP-07: Released VIP is reusable by a different spec

**Driving port**: `submit_workload` handler + reconciler tick
**Tags**: `@happy_path` `@ac-03`

```gherkin
Given a pool with exactly 1 address (e.g., `ranges = ["10.96.0.1/32"]`)
And Service `A` was allocated VIP `10.96.0.1` and then transitioned
  to terminal state (VIP released)
When a different Service spec `B` (different spec digest) is
  submitted via `submit_workload`
Then Service `B` receives VIP `10.96.0.1` (the released address)
```

**Crafter notes**: Uses a minimal pool (single-address CIDR) to
force reuse. The test exercises the full lifecycle: allocate → release
→ reallocate.

---

## AC-04: Pool exhaustion

### S-VIP-08: Pool exhausted — typed rejection

**Driving port**: `submit_workload` handler
**Tags**: `@error_path` `@ac-04` `@kpi:K4`

```gherkin
Given a pool configured as `ranges = ["10.96.0.1/32"]` (1 address)
And 1 VIP is already allocated to Service `A`
When a different Service spec `B` is submitted via `submit_workload`
Then the response is a typed error (HTTP 503)
And the error body names pool exhaustion as the cause
And the error includes `allocated` and `capacity` counts
```

**Crafter notes**: `PoolError::Exhausted { allocated: 1, capacity: 1 }`
surfaces as `AllocatorError::Exhausted` → `ControlPlaneError` →
HTTP 503 with `ProblemDetails`. Verify the error shape, not just the
status code.

### S-VIP-09: Pool exhaustion leaves existing allocations unaffected

**Driving port**: `submit_workload` handler
**Tags**: `@error_path` `@ac-04`

```gherkin
Given a pool with 2 addresses and both are allocated to Services
  `A` and `B`
When Service `C` submission is rejected with pool exhaustion
Then `allocator.get(&digest_A)` still returns VIP for `A`
And `allocator.get(&digest_B)` still returns VIP for `B`
And alloc-status for `A` and `B` still renders their VIPs
```

**Crafter notes**: Verify no partial state mutation on the exhaustion
path. The rejected submission must not corrupt the allocator's memo
table or IntentStore entries.

### S-VIP-10: No partial state persisted on exhaustion

**Driving port**: `IntentBackedAllocator<ServiceVip>` API
**Tags**: `@error_path` `@ac-04`

```gherkin
Given a pool with 1 address, fully allocated
When `allocator.allocate(new_digest)` returns `Err(Exhausted { .. })`
Then `allocator.get(&new_digest)` returns `None`
And `allocator.get(&existing_digest)` still returns the prior VIP
```

**Crafter notes**: Exercises the "no partial state" invariant from
ADR-0049 § 4 via the allocator's observable interface (`get`). The
failed allocation must not appear in the allocator, and the
existing allocation must not be corrupted. Internal store-level
verification (IntentStore queries, memo_len) belongs in property
tests.

---

## AC-05: Shared allocator primitive

### S-VIP-11: PoolAllocator serves BackendId allocations (API stability)

**Driving port**: `PoolAllocator<BackendId>` API
**Tags**: `@happy_path` `@ac-05`

```gherkin
Given a PoolAllocator<BackendId> constructed with BackendIdRange
When `allocate((ip, port, proto))` is called with a unique endpoint
Then a unique BackendId is returned
And calling `allocate` again with the same endpoint returns the
  same BackendId (memo-hit)
And `release(&endpoint)` removes the memo entry
```

**Crafter notes**: The refactored `BackendIdAllocator` wraps
`PoolAllocator<BackendId>`. This scenario validates that the existing
`BackendIdAllocator` API (`allocate(ip, port, proto)`, `release(id)`,
`memo_len()`) is signature-stable after the single-cut migration to
`allocators/backend_id.rs`. The existing proptest and collision-witness
tests move with the file.

### S-VIP-12: PoolAllocator serves ServiceVip allocations

**Driving port**: `PoolAllocator<ServiceVip>` API
**Tags**: `@happy_path` `@ac-05`

```gherkin
Given a PoolAllocator<ServiceVip> with VipRange from
  `ranges = ["10.96.0.0/24"]`
When `allocate(spec_digest)` is called with a unique digest
Then a ServiceVip within 10.96.0.0/24 is returned
And `allocate` with the same digest returns the same VIP (memo-hit)
And `release(&digest)` frees the VIP
And `get(&digest)` returns None after release
```

**Crafter notes**: Exercises the `Token` impl for `ServiceVip` —
specifically `Token::nth(n, range)` mapping counter index to CIDR
address, skipping reserved.

---

## AC-06: Operator-supplied VIP rejection (parser-level)

### S-VIP-13: Parser rejects `vip` field with named guidance

**Driving port**: TOML parser (serde deserializer)
**Tags**: `@error_path` `@ac-06`

```gherkin
Given a Service spec TOML with a `[[listener]]` block containing:
  | field    | value         |
  | port     | 8080          |
  | protocol | tcp           |
  | vip      | 10.96.42.17   |
When the spec is parsed via the standard Job/ServiceSpec parser
Then parsing fails with a typed error
And the error message names the `vip` field (or `unknown field`)
And the error message guides the operator to remove the field
```

**Crafter notes**: Per ADR-0049 § 5, the `vip` field is removed from
`Listener`. `#[serde(deny_unknown_fields)]` (or TOML deserializer
equivalent) produces `unknown field 'vip'`. The crafter ensures the
error message includes named guidance — the exact wording is the
crafter's call; the load-bearing property is that operators know what
to do.

### S-VIP-14: Parser rejection causes no state mutation

**Driving port**: `submit_workload` handler
**Tags**: `@error_path` `@ac-06`

```gherkin
Given a Service spec with a `[[listener]]` block containing
  `vip = "10.96.42.17"` (matching S-VIP-13 setup)
When the spec is submitted via `submit_workload` handler
Then the handler returns a validation error (HTTP 400 or 422)
And a subsequent `alloc status` query shows no allocation for this spec
And the allocator has no VIP assigned for this spec's digest
```

**Crafter notes**: The parse error fires before the admission path
reaches the allocator. The "no state mutation" invariant is verified
through the observable interface: alloc-status shows nothing, and the
allocator's `get(&digest)` returns `None`. This exercises the same
parser-level rejection as S-VIP-13 but asserts handler-level side
effects.

---

## Config and Boot Validation

### S-VIP-15: Missing `[dataplane.vip_allocator]` section — boot refuses

**Driving port**: Control-plane boot / composition root
**Tags**: `@error_path` `@config` `@boot`

```gherkin
Given an operator config TOML with a `[dataplane]` section but no
  `[dataplane.vip_allocator]` subsection
When the control plane attempts to boot
Then boot fails with a typed `VipAllocatorConfigError::Missing` error
And the error message names the missing section
And a structured `health.startup.refused` event is emitted
```

### S-VIP-16: Overlapping CIDR ranges — boot refuses

**Driving port**: VipRange constructor
**Tags**: `@error_path` `@config` `@boot`

```gherkin
Given config `ranges = ["10.96.0.0/24", "10.96.0.0/16"]`
  (overlapping CIDRs)
When the VipRange is constructed from this config
Then construction fails with
  `VipAllocatorConfigError::OverlappingRanges`
And the error names the overlapping CIDRs
```

### S-VIP-17: Reserved address outside ranges — boot refuses

**Driving port**: VipRange constructor
**Tags**: `@error_path` `@config` `@boot`

```gherkin
Given config `ranges = ["10.96.0.0/24"]` and
  `reserved = ["192.168.1.1"]` (outside the range)
When the VipRange is constructed
Then construction fails with a typed config error naming the
  out-of-range reserved address
```

### S-VIP-18: Zero-capacity range — boot refuses

**Driving port**: VipRange constructor
**Tags**: `@error_path` `@config` `@boot`

```gherkin
Given config `ranges = ["10.96.0.1/32"]` and
  `reserved = ["10.96.0.1"]` (the only address is reserved)
When the VipRange is constructed
Then `capacity()` is 0
And boot refuses with a typed error naming zero capacity
```

### S-VIP-19: Persisted state outside shrunk CIDR — probe refuses

**Driving port**: `IntentBackedAllocator::bulk_load` (Earned Trust probe)
**Tags**: `@error_path` `@boot`

```gherkin
Given an IntentStore containing a persisted allocator entry with
  VIP `10.96.1.100`
And the current config has `ranges = ["10.96.0.0/24"]` (the
  persisted VIP is outside the new range)
When `IntentBackedAllocator::bulk_load` runs
Then bulk_load returns `Err(AllocatorBootError::...)` naming the
  inconsistency
And the control plane refuses to start
```

**Crafter notes**: Exercises Earned Trust probe check 3 from
ADR-0049 § 8: "every persisted (key, token) projects back to a
token within `range`."

---

## Idempotency and Edge Cases

### S-VIP-20: Release of already-released key is idempotent

**Driving port**: `IntentBackedAllocator<ServiceVip>` API
**Tags**: `@edge_case`

```gherkin
Given VIP `V` was allocated for digest `D` and then released
When `allocator.release(&D)` is called again
Then the call succeeds (no error)
And `allocator.get(&D)` still returns `None`
And the pool state is unchanged
```

### S-VIP-21: Reserved addresses are skipped during allocation

**Driving port**: `PoolAllocator<ServiceVip>` API
**Tags**: `@edge_case`

```gherkin
Given a pool with `ranges = ["10.96.0.0/30"]` (4 addresses:
  .0, .1, .2, .3) and `reserved = ["10.96.0.0", "10.96.0.3"]`
  (network + broadcast)
When two distinct Service specs are submitted
Then the first receives an address from {10.96.0.1, 10.96.0.2}
And the second receives the other address from that set
And neither receives 10.96.0.0 or 10.96.0.3
And a third submission returns `PoolError::Exhausted { allocated: 2,
  capacity: 2 }`
```

**Crafter notes**: Validates `Token::nth(n, range)` correctly skips
reserved addresses. The capacity is 2 (4 CIDR minus 2 reserved).

---

## Property-Based Tests

These are mandated by `.claude/rules/testing.md` and exercise invariants
beyond the BDD scenarios above.

### S-VIP-P01: ServiceVip newtype roundtrip

**Tags**: `@property` `@mandatory:newtype_roundtrip`

```
For all valid IPv4 addresses `a`:
  ServiceVip::from_str(&ServiceVip::new(a).to_string()) == Ok(ServiceVip(a))
  serde_json::from_str(&serde_json::to_string(&ServiceVip(a))) == Ok(ServiceVip(a))
```

**Crafter notes**: Per `.claude/rules/development.md` § "Newtype
completeness" — `Display`/`FromStr`/serde roundtrip.

### S-VIP-P02: AllocatorEntry rkyv envelope golden-bytes roundtrip

**Tags**: `@property` `@mandatory:schema_evolution`

```
For AllocatorEntryV1 with hand-pinned fields:
  hex_decode(FIXTURE_V1) → rkyv-deserialise → into_latest() →
  assert_eq(canonical Latest projection)
```

**Crafter notes**: Per `.claude/rules/testing.md` § "Archive
schema-evolution roundtrip" — golden-bytes fixture in
`crates/overdrive-dataplane/tests/schema_evolution/allocator_entry.rs`.

### S-VIP-P03: PoolAllocator never assigns duplicate tokens

**Tags**: `@property` `@mandatory:allocator_invariant`

```
For all sequences of N allocate() calls with distinct keys (N ≤ capacity):
  all returned tokens are distinct
  memo_len() == N
  releasing key K and reallocating K returns a token (possibly
    different from the original if counter advanced)
```

**Crafter notes**: Proptest with `PROPTEST_CASES=1024`. Exercises
both `BackendId` and `ServiceVip` token types.

### S-VIP-P04: VipRange capacity equals CIDR size minus reserved count

**Tags**: `@property`

```
For all valid (cidr, reserved_set) where reserved ⊆ cidr:
  VipRange::new(cidr, reserved).capacity() == cidr.size() - reserved.len()
```

---

## Adapter Coverage Table

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `IntentStore` (redb via `LocalStore`) | YES | S-VIP-05 (bulk_load persistence roundtrip), S-VIP-10 (no partial state on exhaustion), S-VIP-19 (probe with shrunk CIDR) |
| TOML parser (serde) | YES | S-VIP-13 (parser rejects `vip` field), S-VIP-14 (no state mutation on parse error) |
| `submit_workload` HTTP handler | YES | S-VIP-01 (submit and echo), S-VIP-04 (idempotent resubmit), S-VIP-08 (pool exhaustion) |
| `alloc_status` HTTP handler | YES | S-VIP-02 (render same VIP as echo) |
| `WorkloadLifecycle` reconciler | YES | S-VIP-06 (terminal-state reclamation) |
| VipRange config parser | YES | S-VIP-16/17/18 (overlapping/out-of-range/zero-capacity) |

Zero "NO — MISSING" rows.

---

## KPI Traceability

| KPI | Scenario(s) | What it exercises |
|---|---|---|
| K1 (successful-allocation rate) | S-VIP-01, S-VIP-04 | Happy-path allocation success; idempotent resubmit success |
| K3 (VIP reclamation lag) | S-VIP-06 | Terminal-state → Action::ReleaseServiceVip → allocator.release() |
| K4 (pool exhaustion) | S-VIP-08 | Typed rejection when pool is full; counter/capacity in error |
| K2 (allocator-induced latency) | — | Not directly testable in acceptance tests; instrumentation concern for DEVOPS. Latency is structural: in-memory O(log N) + single fsync. |

---

## AC-to-Scenario Traceability

| AC | Scenario(s) | Coverage |
|---|---|---|
| AC-01 | S-VIP-01, S-VIP-02, S-VIP-03 | Submit echo, alloc-status, multi-listener |
| AC-02 | S-VIP-04, S-VIP-05 | Idempotent resubmit, persistence across restart |
| AC-03 | S-VIP-06, S-VIP-07 | Reclamation, VIP reuse |
| AC-04 | S-VIP-08, S-VIP-09, S-VIP-10 | Typed rejection, existing unaffected, no partial state |
| AC-05 | S-VIP-11, S-VIP-12 | BackendId API stability, ServiceVip allocations |
| AC-06 | S-VIP-13, S-VIP-14 | Parser rejection with guidance, no state mutation |

All 6 ACs covered. No AC without at least 2 scenarios.

---

## Self-Review Checklist

- [x] 1. WS strategy declared in wave-decisions.md (DWD-01: SKIP — brownfield)
- [x] 2. WS scenarios tagged correctly (N/A — no WS)
- [x] 3. Every driven adapter has at least one `@real-io` scenario (adapter coverage table: 0 MISSING)
- [x] 4. For InMemory doubles: documented what they CANNOT model (DWD-02: Sim adapters for ObservationStore/Driver/Dataplane — these are not under test for the allocator feature)
- [x] 5. Container preference documented (N/A — no containers; all local)
- [x] 6. **Mandate 7**: Scaffolding deferred to DELIVER per Rust project convention (DWD-05)
- [x] 7. **Driving Adapter**: Every handler/reconciler in DESIGN has at least one scenario exercising it via its protocol (`submit_workload` → S-VIP-01/04/08; `WorkloadLifecycle` → S-VIP-06; parser → S-VIP-13)
- [x] 8. Error path coverage ≥ 40% (48% — 10 of 21 scenarios)
- [x] 9. No contradictions in wave-decision reconciliation (0 found)
- [x] 10. AC-to-scenario traceability complete (all 6 ACs covered, ≥ 2 scenarios each)
- [x] 11. KPI traceability documented (K1, K3, K4 covered; K2 structural)
- [x] 12. Property tests specified for mandatory call sites (newtype roundtrip, schema evolution, allocator invariant)
