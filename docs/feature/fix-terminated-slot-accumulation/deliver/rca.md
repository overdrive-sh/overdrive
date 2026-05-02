# RCA — `Terminated` slot accumulation in `ExecDriver::live`

**Defect site**: `crates/overdrive-worker/src/driver.rs:341`

## Problem

`Driver::stop` re-inserts `LiveAllocation::Terminated` into `self.live`
after teardown so that `Driver::status` can return `Terminated` rather
than `NotFound`. The slot is never evicted; over a long-running node
session every allocation that completes its lifecycle (without a
subsequent `start` overwriting the slot) leaves a permanent `BTreeMap`
entry. Restart cycles overwrite (`Terminated → Running` at line 270),
so the leak is bounded to "one entry per finally-terminated alloc."

## Root cause (selected branch)

The `Driver` trait conflates "what's running right now" with "what was
the terminal state of a once-known alloc" into one in-memory map, with
no eviction primitive in the trait surface. The `status() == Terminated
post-stop` contract is asserted by three acceptance tests but is **not
read by any production caller** — `action_shim.rs` calls `start`/`stop`
only, and the reconciler reads `AllocStatusRow` from the
ObservationStore for terminal-state truth.

The §18 state-layer split puts durable terminal state in the
ObservationStore, not in driver-private memory; the current shape
duplicates obs's job.

## Decision: Candidate A — evict in `stop()`

Drop `LiveAllocation::Terminated`, evict the slot in `stop()`, simplify
`status()`/`resize()`/`stop()` matches, rewrite three test assertions
to expect `Err(DriverError::NotFound)` post-stop, document the relaxed
contract on the trait. Apply symmetrically to `SimDriver` so the two
adapters stay aligned with the shared trait contract.

## Scope of change

| File | Change |
|---|---|
| `crates/overdrive-core/src/traits/driver.rs` | Rustdoc on `Driver::status`: post-stop returns `Err(NotFound)`; durable terminal state lives in obs. |
| `crates/overdrive-worker/src/driver.rs` | Drop `LiveAllocation::Terminated` (collapse to a struct or 1-variant enum); remove the re-insert at L341; simplify `stop`/`status`/`resize` matches. |
| `crates/overdrive-sim/src/adapters/driver.rs` | Symmetric: drop terminal-slot retention; same contract. |
| `crates/overdrive-worker/tests/integration/exec_driver/stop_with_grace.rs` | L64-65 — expect `Err(NotFound)` post-stop. |
| `crates/overdrive-worker/tests/integration/exec_driver/stop_escalates_to_sigkill.rs` | L189-190 — expect `Err(NotFound)` post-stop. |
| `crates/overdrive-sim/tests/acceptance/sim_adapters_deterministic.rs` | L298-303 — expect `Err(NotFound)` post-stop. |

## Behavioral changes

- `status()` after `stop()` → `Err(DriverError::NotFound)` instead of
  `Ok(AllocationState::Terminated)`. Documented on the trait.
- Double-`stop()` → `Err(NotFound)` instead of `Ok(())`. Already
  absorbed by `action_shim.rs:140, 187` (`let _ = driver.stop(...)`).
  No production behavior change.

## Regression test

A new test in the worker integration suite asserting that, after `N`
start+stop cycles against distinct allocation IDs, `ExecDriver`'s live
map cardinality is bounded (i.e. equals the number of currently-running
allocations, which is zero). Equivalent test for `SimDriver`. Test
fails against current code (slot retained); passes after fix.

## Risk

- 3 acceptance assertions to update (mechanical).
- Trait rustdoc clarification.
- Symmetric `SimDriver` change.
- No DST invariant currently asserts `status==Terminated post-stop`.
- No production caller depends on the removed contract.

## References

- Whitepaper §18 — Reconciler / Workflow primitives; three-layer state taxonomy.
- ADR-0026 — `ExecDriver` Phase 1 production driver.
- ADR-0029 — `exec` driver naming.
- `.claude/rules/development.md` § State-layer hygiene.
- Memory: `feedback_single_cut_greenfield_migrations.md`,
  `feedback_delete_dont_gate.md`, `feedback_phase1_single_node_scope.md`.
