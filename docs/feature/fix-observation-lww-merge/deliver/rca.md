# RCA — `LocalObservationStore::write` lacks LWW merge

**Feature ID**: `fix-observation-lww-merge`
**Reported via**: code review on `crates/overdrive-store-local/src/observation_backend.rs:127-161`
**RCA produced**: 2026-04-25
**User review**: APPROVED 2026-04-25 (Phase 2 of `/nw-bugfix`).

## Symptom

`LocalObservationStore::write` (production redb adapter) accepts every incoming row and overwrites the prior bytes by arrival order alone. `SimObservationStore::apply_alloc_status` rejects an incoming row whose `updated_at: LogicalTimestamp` does not dominate the existing entry. An older `AllocStatusRow` or `NodeHealthRow` arriving after a newer one at the production store silently regresses the value to the older state.

Phase 1 has no real observation writers (`noop-heartbeat` emits only `Action::Noop`), so this is latent today. The moment Phase 2 writers land, this becomes a silent data-loss path under out-of-order delivery.

## 5 Whys (three composing causes)

### Cause A — Trait under-specification

- **WHY 1A** — Production `write` overwrites without comparing `updated_at`. *Evidence: `observation_backend.rs:138-140` calls `table.insert(key, bytes)` unconditionally.*
- **WHY 2A** — ADR-0012 (rev. 2026-04-24) chose "no LWW merge" — "last write wins by the trivial 'most recent redb transaction commits' rule". *Evidence: ADR-0012 §"No CRDT machinery, by design".*
- **WHY 3A** — ADR-0012 conflated *Phase 1 single-writer* (one node, monotonic local writes; LWW vacuous) with *trait contract* (shared with `SimObservationStore` today, shipping to Corrosion in Phase 2).
- **WHY 4A** — The trait `ObservationStore` does not specify ordering semantics for `write`. Two adapters claiming the same trait converge differently. ADR-0011 (intent vs observation non-substitutability) is enforced by *type*, not by *ordering contract*.
- **WHY 5A — ROOT CAUSE A** — `LogicalTimestamp` is part of the row schema (every writer fills it) but the trait never names it as the merge key. The contract is under-specified: row shape is type-checked, semantics are folklore.

### Cause B — Test coverage shape

- **WHY 1B** — Acceptance tests didn't catch this: `overwrite_same_key_replaces_first_row` writes counters `1 → 2` (monotonic) and asserts the second wins. That holds equally for "blind overwrite" and for "LWW".
- **WHY 2B** — The acceptance suite mirrors ADR-0012's six ACs verbatim; none cover out-of-order delivery.
- **WHY 3B** — DST/proptest LWW coverage in `sim_observation_lww_converges.rs` exists *only* against `SimObservationStore` — production `LocalObservationStore` is structurally absent from the convergence test surface.
- **WHY 4B** — The two adapters were tested under different harness shapes (Tier 1 DST vs Tier 3 acceptance) with no shared property test asserting they honour the same merge order on the same input.
- **WHY 5B — ROOT CAUSE B** — No invariant is asserted at the trait level — only at the implementation level. Conformance tests against `dyn ObservationStore` would have caught this on day one.

### Cause C — Comparator in the wrong crate

- **WHY 1C** — `lww_dominates` lives in `overdrive-sim` (adapter-sim) — `overdrive-store-local` (adapter-host) cannot import it. *Evidence: there is no shared dependency direction.*
- **WHY 5C — ROOT CAUSE C** — A semantically load-bearing total-ordering primitive lives in a leaf crate that only the sim depends on. Two adapters of the same trait *cannot* call the same comparator. Both *necessarily* re-implement merge semantics — or, as in production, omit them.

## Approved fix (single-cut PR)

1. **Promote** `lww_dominates` to `crates/overdrive-core/src/traits/observation_store.rs` as `LogicalTimestamp::dominates(&self, &Self) -> bool`. dst-lint clean (pure `Ord` over `(u64, NodeId)`, no I/O). Sim deletes its local copy and imports the core method.

2. **Document** the LWW contract on the `ObservationStore::write` trait method docstring — losers MUST NOT mutate state and MUST NOT emit on subscriptions.

3. **`LocalObservationStore::write`** reads the prior row inside the existing `begin_write` transaction, decodes `updated_at`, compares via `dominates`, and skips both `table.insert` AND the post-commit `self.emit(row)` on loss. Same shape for `AllocStatusRow` and `NodeHealthRow`. Commit on every code path (the read-then-conditional-insert is one txn).

4. **Trait conformance suite** — generic over `T: ObservationStore`, exposed from `overdrive-core::testing::observation_store::run_lww_conformance` (gated behind a new `test-utils` feature so it does not pollute production code paths). Both adapters' test suites instantiate the harness:
   - `crates/overdrive-store-local/tests/integration/lww_conformance.rs` (gated `integration-tests`).
   - `crates/overdrive-sim/tests/acceptance/lww_conformance.rs` (default lane — sim is in-memory).

5. **ADR-0012 third revision** (architect-dispatched): strike "no CRDT machinery, by design"; LWW domination becomes mandatory at the trait level even though Phase 1's single writer makes it trivially satisfied today. Per user-memory `feedback_delegate_to_architect`, this edit goes through the architect agent — not inline.

## Files affected

| File | Change |
|---|---|
| `crates/overdrive-core/Cargo.toml` | Add `test-utils` feature; conditionally pub-export `testing` module. |
| `crates/overdrive-core/src/lib.rs` | `#[cfg(any(test, feature = "test-utils"))] pub mod testing;` |
| `crates/overdrive-core/src/traits/observation_store.rs` | Add `LogicalTimestamp::dominates`; document LWW on `write`. |
| `crates/overdrive-core/src/testing/observation_store.rs` | New: `run_lww_conformance<T: ObservationStore>(store: &T)`. |
| `crates/overdrive-store-local/src/observation_backend.rs` | LWW-guarded `write`; suppress emit on loss; read prior row inside txn. |
| `crates/overdrive-store-local/Cargo.toml` | Add `overdrive-core/test-utils` to dev-deps. |
| `crates/overdrive-store-local/tests/integration/lww_conformance.rs` | New: invokes the harness against `LocalObservationStore`. |
| `crates/overdrive-store-local/tests/integration.rs` | Wire new module. |
| `crates/overdrive-store-local/tests/acceptance/local_observation_store.rs` | Add `out_of_order_alloc_status_does_not_regress` + `out_of_order_node_health_does_not_regress`. |
| `crates/overdrive-sim/src/adapters/observation_store.rs` | Replace local `lww_dominates` with `LogicalTimestamp::dominates`. |
| `crates/overdrive-sim/Cargo.toml` | Add `overdrive-core/test-utils` to dev-deps. |
| `crates/overdrive-sim/tests/acceptance/lww_conformance.rs` | New: invokes the harness against `SimObservationStore::single_peer`. |
| `crates/overdrive-sim/tests/acceptance.rs` | Wire new module. |
| `docs/product/architecture/adr-0012-observation-store-server-impl.md` | Third revision via architect. |

## Risk

- **Transaction shape**: read-then-conditional-insert inside one `begin_write` is standard redb usage; serialisable isolation already provided. No deadlock risk (one table per row variant, no cross-table ordering).
- **Performance**: one extra `table.get` + small rkyv decode per `write`. μs-scale; well inside the 100 ms REST budget per ADR-0012.
- **Emission semantics**: suppressing `emit` on LWW reject is the right choice — subscribers must never see a row the store will then refuse to return on read. Matches `SimObservationStore::apply` semantics.
- **Existing tests**: `overwrite_same_key_replaces_first_row` keeps passing (`2.dominates(1) == true` under monotonic counters).
- **Mutation testing**: `dominates` is a high-value target — branch flips on counter (`>` vs `>=`) and tiebreak inversion each get a dedicated proptest case via the conformance suite. Verify via `cargo xtask mutants --diff origin/main --package overdrive-core --file crates/overdrive-core/src/traits/observation_store.rs`.
- **Single-cut migration**: per `feedback_single_cut_greenfield_migrations`, sim's `lww_dominates` is deleted in the same commit as core's `dominates` lands. No `#[deprecated]` shim, no parallel comparator.

## User review record

| Date | Reviewer | Verdict | Notes |
|---|---|---|---|
| 2026-04-25 | user (Marcus) | **APPROVED** | Confirmed root cause match; approved promotion of `dominates` to core, suppressed emit on LWW reject, trait-generic conformance suite under `integration-tests` in both adapters, and ADR-0012 third revision. No scope-narrowing — both row variants in this PR. |
