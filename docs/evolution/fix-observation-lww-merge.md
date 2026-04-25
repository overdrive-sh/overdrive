# fix-observation-lww-merge — Feature Evolution

**Feature ID**: fix-observation-lww-merge
**Type**: Bug fix (via `/nw-bugfix` → `/nw-deliver` pipeline)
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Date**: 2026-04-25
**Commits**:
- `26d5f60` — `test(observation-store): pin LWW contract on out-of-order writes (RED scaffold)` (Step 01-01)
- `a4e6527` — `fix(observation-store): apply LWW guard to LocalObservationStore::write` (Step 01-02 GREEN)
- `aa13c2e` — `test(observation-store): add function-level dominates tests for mutation gate` (Step 01-02 mutation-gate fixup)
- `6d66b41` — `docs(adr-0012): codify ObservationStore::write LWW contract (third revision)` (Step 01-02 architect-dispatched ADR revision)

**Status**: Delivered (RED scaffold → cohesive GREEN cut → mutation-gate fixup → ADR third revision; 2-step roadmap, 4 commits)

---

## Summary

`LocalObservationStore::write` (the production redb adapter on
`crates/overdrive-store-local/src/observation_backend.rs`) performed
an unconditional `table.insert` keyed by `AllocationId` / `NodeId` —
no comparison on `updated_at: LogicalTimestamp`. `SimObservationStore`
already enforced LWW via a sim-local `lww_dominates`. An older
`AllocStatusRow` or `NodeHealthRow` arriving after a newer one at the
production store silently regressed state and emitted the loser on the
broadcast subscription. Latent in Phase 1 (no real observation
writers; `noop-heartbeat` emits only `Action::Noop`); silent
data-loss path the moment Phase 2 writers land. Fix: promote the
comparator to `overdrive_core::traits::observation_store::LogicalTimestamp::dominates`,
add a trait-generic conformance harness (`overdrive_core::testing::observation_store::run_lww_conformance<T>`)
exposed via a new `test-utils` feature on `overdrive-core`, replace
sim's local `lww_dominates` (single-cut delete), apply
read-then-conditional-insert in `LocalObservationStore::write` inside
the existing `begin_write` transaction with the post-commit
`self.emit(row)` suppressed on loss, and codify the new trait contract
in ADR-0012's third revision. 100% mutation kill rate on the comparator
(6/6 caught) via diff-scoped `cargo xtask mutants`.

---

## Root cause

Full RCA at `docs/feature/fix-observation-lww-merge/deliver/rca.md`. Three
composing causes (5 Whys reproduced inline):

### Cause A — Trait under-specification

- **WHY 1A** — Production `write` overwrites without comparing `updated_at`. *Evidence: `observation_backend.rs:138-140` calls `table.insert(key, bytes)` unconditionally.*
- **WHY 2A** — ADR-0012 (rev. 2026-04-24) chose "no LWW merge" — "last write wins by the trivial 'most recent redb transaction commits' rule".
- **WHY 3A** — ADR-0012 conflated *Phase 1 single-writer* (one node, monotonic local writes; LWW vacuous) with *trait contract* (shared with `SimObservationStore` today, shipping to Corrosion in Phase 2).
- **WHY 4A** — The trait `ObservationStore` did not specify ordering semantics for `write`. Two adapters claiming the same trait converged differently. ADR-0011 (intent vs observation non-substitutability) was enforced by *type*, not by *ordering contract*.
- **WHY 5A — ROOT CAUSE A** — `LogicalTimestamp` was part of the row schema (every writer fills it) but the trait never named it as the merge key. Row shape was type-checked; semantics were folklore.

### Cause B — Test coverage shape

- **WHY 1B** — Acceptance `overwrite_same_key_replaces_first_row` writes counters `1 → 2` (monotonic) and asserts the second wins — holds equally for "blind overwrite" and "LWW".
- **WHY 2B** — The acceptance suite mirrored ADR-0012's six ACs verbatim; none covered out-of-order delivery.
- **WHY 3B** — DST/proptest LWW coverage in `sim_observation_lww_converges.rs` exercised *only* `SimObservationStore` — production `LocalObservationStore` was structurally absent from the convergence test surface.
- **WHY 4B** — The two adapters were tested under different harness shapes (Tier 1 DST vs Tier 3 acceptance) with no shared property test asserting they honoured the same merge order on the same input.
- **WHY 5B — ROOT CAUSE B** — No invariant was asserted at the trait level — only at implementation level. Conformance tests against `dyn ObservationStore` would have caught this on day one.

### Cause C — Comparator in the wrong crate

- **WHY 1C** — `lww_dominates` lived in `overdrive-sim` (adapter-sim) — `overdrive-store-local` (adapter-host) could not import it. *No shared dependency direction.*
- **WHY 5C — ROOT CAUSE C** — A semantically load-bearing total-ordering primitive lived in a leaf crate that only the sim depended on. Two adapters of the same trait *could not* call the same comparator — both *necessarily* re-implemented merge semantics, or, as in production, omitted them.

---

## Decisions made

### 1. Comparator promoted to `overdrive-core` as `LogicalTimestamp::dominates`

The total-ordering primitive lives on the type that carries it. The
new method `overdrive_core::traits::observation_store::LogicalTimestamp::dominates(&self, &Self) -> bool`
mirrors the prior sim free function `lww_dominates` exactly: lex order
on `(counter, writer)` with `Display`-string tiebreak on `writer`;
equal timestamps return `false` (preserves the LWW idempotency case).
Pure `Ord` over `(u64, NodeId)` — no `Instant`, no `tokio::net`, no
`rand`, no `std::time`; `cargo xtask dst-lint` continues to pass on
`overdrive-core` (`crate_class = "core"`).

### 2. Trait-generic conformance harness behind `test-utils` feature

`overdrive_core::testing::observation_store::run_lww_conformance<T: ObservationStore + ?Sized>(store: &T)`
is exposed via a new `test-utils = []` feature on `overdrive-core`
(`#[cfg(any(test, feature = "test-utils"))] pub mod testing;` in
`crates/overdrive-core/src/lib.rs`). Both adapter crates pull
`overdrive-core/test-utils` into `[dev-dependencies]`. The harness
exercises the full LWW property surface — newer dominates older,
older arriving after newer is rejected, equal-timestamp idempotency,
counter-tie tiebreak on `writer`, subscriber-silence on loss — for
both `AllocStatusRow` and `NodeHealthRow`. Both adapters' test suites
instantiate it: store-local from
`crates/overdrive-store-local/tests/integration/lww_conformance.rs`
(gated `integration-tests` because the adapter touches real redb files);
sim from `crates/overdrive-sim/tests/acceptance/lww_conformance.rs`
(default lane — sim is in-memory). RCA §Cause B (test coverage
shape) is closed by this single shared call site reaching both
adapters.

### 3. Read-then-conditional-insert inside the existing `begin_write`

`LocalObservationStore::write` opens the redb table, calls
`table.get(key)?` for prior bytes, decodes the prior row's
`updated_at` via rkyv, computes
`accepted = prior.is_none() || incoming.updated_at.dominates(&prior_unwrapped.updated_at)`,
and only on `accepted` does it call `table.insert(key, bytes)`. The
transaction commits on every code path (rejected branches commit a
no-op transaction so the redb txn lifecycle stays balanced). The
`accepted` boolean is returned out of the `spawn_blocking` closure;
`self.emit(row)` after `.await??` is conditional on
`accepted == true`. Same shape for `AllocStatusRow` and
`NodeHealthRow`. Storage choice — full prior-row decode rather than
a separate `LwwHeader` rkyv slice — is documented inline in
`observation_backend.rs` near the new branch; μs-scale per the RCA
risk note, well inside the 100ms REST budget. No new internal helper
extracted; the in-line code is short enough not to warrant abstraction
per `.claude/rules/development.md` ("Don't add features, refactor, or
introduce abstractions beyond what the task requires").

### 4. Single-cut deletion of sim's `lww_dominates`

Per `feedback_single_cut_greenfield_migrations`. The sim's local
`lww_dominates` (lines 229-238 of
`crates/overdrive-sim/src/adapters/observation_store.rs`) is **deleted**
in the same commit as core's `LogicalTimestamp::dominates` lands. The
sim's `dominates_for_merge` keeps its shape (it still wraps the
comparator with the `canary-bug` feature flip) but its delegate is now
`a.dominates(b)` against the new method. No `#[deprecated]` shim, no
parallel comparator, no two-phase trait migration. A
`git grep 'lww_dominates' crates/overdrive-sim` returns no production
matches post-fix.

### 5. Suppressed emit on LWW reject — subscription semantics

When LWW rejects an incoming row, BOTH the redb `table.insert` AND the
post-commit `self.emit(row)` are skipped. Subscribers must never
observe a row the store will then refuse to return on read. This
matches `SimObservationStore::apply_alloc_status` semantics (the
fan-out send happens only inside the dominate branch, see
`crates/overdrive-sim/src/adapters/observation_store.rs:170-176`). The
two new acceptance cases on `local_observation_store.rs` assert
subscriber silence directly via `tokio::time::timeout` over
`BroadcastStream::next()`.

### 6. Sim's `apply_node_health` added in parallel to `apply_alloc_status`

Sim's existing `apply_alloc_status` honoured LWW; an `apply_node_health`
counterpart did not exist (Phase 1's acceptance suite was unconstrained
on NodeHealth ordering). The conformance harness pins both row variants
identically across both adapters, so sim's `apply_node_health` was added
in parallel with `apply_alloc_status` and `node_health_rows()` now
returns LWW winners only via deterministic BTreeMap iteration. This is
in-scope because the conformance harness exercises both row variants
against both adapters; partial closure on AllocStatus alone would have
left the same gap on NodeHealth that the bug exhibited on
`LocalObservationStore`.

### 7. Function-level mutation coverage as a follow-up commit

The cohesive GREEN commit (`a4e6527`) landed the comparator and
exercised it through the trait-conformance harness in adapter test
suites. `cargo xtask mutants --diff origin/main --package overdrive-core --file crates/overdrive-core/src/traits/observation_store.rs`
defaults `--test-workspace=false` per the wrapper's behaviour, so the
mutants saw only `overdrive-core`'s own test suite — where `dominates`
had no caller. First diff-scoped pass: 5/6 missed (the `>` vs `>=`
flip, the `Less`/`Greater` arm swap, and the tiebreak `>` vs `<`
inversion all survived). Commit `aa13c2e` adds
`crates/overdrive-core/tests/acceptance/logical_timestamp_dominates.rs`
— a function-level acceptance file with table-driven coverage of
every branch (`Greater`, `Less`, `Equal` tiebreak with `<` / `>` /
`==` discriminators, plus the idempotency case for the `>` vs `>=`
flip). Re-run: **6/6 caught, 100% kill rate, gate PASS**. The
trait-level conformance harness remains the integration-side proof
of the LWW contract; the function-level file is the comparator-side
proof. Both layers are necessary because mutation testing only runs
the owning crate's suite.

### 8. ADR-0012 third revision via architect dispatch

Per `feedback_delegate_to_architect` (user memory) — the ADR edit was
NOT made inline. The architect was dispatched (commit `6d66b41`) and:

- Struck the §"No CRDT machinery, by design" subsection's claim that
  LWW merge is unnecessary.
- Codified the new trait contract: `ObservationStore::write` applies
  LWW under `LogicalTimestamp::dominates`; losers MUST NOT mutate
  state and MUST NOT emit on subscriptions.
- Kept the §"Restart semantics" rationale unchanged (Phase 1 single
  writer makes the guard trivially-satisfied today; the structural
  protection is for Phase 2 Corrosion).
- Added a third revision header note linking to the RCA at
  `docs/feature/fix-observation-lww-merge/deliver/rca.md`.

The ADR third revision is in the same step (01-02) as the GREEN code
commit, separated from it because the GREEN commit was made by a
subagent that did not have the Agent tool available — the architect
dispatch happens at parent-session level. The trait contract was
already documented inline on the `ObservationStore::write` rustdoc and
on `LogicalTimestamp::dominates` rustdoc within `a4e6527`; `6d66b41`
codified the same change at architecture-document level so the ADR is
no longer in contradiction with the source.

---

## Steps completed

2 phases, 2 roadmap steps, 4 commits. RED → cohesive GREEN cut →
mutation-gate fixup → ADR third revision.

| Step ID | Phase | Status | Commit | Notes |
|---|---|---|---|---|
| 01-01 | RED scaffold | PASS | `26d5f60` | Four failing test artefacts pin the LWW contract: two new `#[tokio::test]` cases on `tests/acceptance/local_observation_store.rs` (`out_of_order_alloc_status_does_not_regress`, `out_of_order_node_health_does_not_regress`) — counter=5 then counter=2, assert subscriber silence and read-returns-newer (fail at runtime pre-fix); two new conformance-harness invocation files (`tests/integration/lww_conformance.rs` on store-local, `tests/acceptance/lww_conformance.rs` on sim) referencing `overdrive_core::testing::observation_store::run_lww_conformance` (fail to compile pre-fix because `overdrive-core::testing` does not exist). Committed `--no-verify` per `.claude/rules/testing.md` §RED scaffolds. |
| 01-02 | GREEN cohesive cut | PASS | `a4e6527` | `LogicalTimestamp::dominates` lands in `overdrive-core`; `test-utils` feature exposes `testing::observation_store::run_lww_conformance<T>`; sim's local `lww_dominates` deleted, `dominates_for_merge` delegates to the new method (preserves `canary-bug` flip); `LocalObservationStore::write` performs read-then-conditional-insert inside the existing `begin_write` txn with `self.emit(row)` gated on `accepted`; sim gains `apply_node_health` in parallel with `apply_alloc_status`; both adapters' Cargo.toml dev-deps add `overdrive-core/test-utils`; Step 01-01's four tests flip RED → GREEN within this commit; full workspace nextest + doctests + clippy clean. |
| 01-02 (mutation fixup) | Mutation-gate close | PASS | `aa13c2e` | `cargo xtask mutants --diff origin/main --package overdrive-core --file crates/overdrive-core/src/traits/observation_store.rs` reported 5/6 missed on the first pass (mutants only see the owning crate's suite per wrapper default `--test-workspace=false`; `dominates` had no caller in `overdrive-core`'s own tests). New function-level acceptance file `tests/acceptance/logical_timestamp_dominates.rs` with table-driven coverage of every branch — re-run reports **6/6 caught, 100% kill rate, gate PASS**. |
| 01-02 (architect dispatch) | ADR third revision | PASS | `6d66b41` | ADR-0012 third revision via `@nw-solution-architect`: strikes §"No CRDT machinery, by design"; codifies `ObservationStore::write` LWW contract under `LogicalTimestamp::dominates`; keeps §"Restart semantics" rationale; adds revision header linking to RCA. |

DES execution log: `docs/feature/fix-observation-lww-merge/deliver/execution-log.json`
— integrity verifier reports all phases EXECUTED/SKIPPED with valid
transitions at finalize time.

### Quality gates

- **Full workspace tests**: `cargo nextest run --workspace --features integration-tests` exits 0 — Step 01-01's four RED tests flip GREEN within commit `a4e6527` without modification, and zero pre-existing tests regress.
- **Doctests**: `cargo test --doc --workspace` exits 0.
- **Clippy**: `cargo clippy --all-targets --features integration-tests -- -D warnings` clean across `overdrive-core`, `overdrive-store-local`, `overdrive-sim`.
- **dst-lint**: `cargo xtask dst-lint` exits 0 — `overdrive-core` additions stay banned-API-clean (`LogicalTimestamp::dominates` is pure `Ord` over `(u64, NodeId)`; the `testing` module is gated `#[cfg(any(test, feature = "test-utils"))]` and uses only the `ObservationStore` trait surface plus standard async).
- **Mutation gate**: `cargo xtask mutants --diff origin/main --package overdrive-core --file crates/overdrive-core/src/traits/observation_store.rs` reports **6/6 caught, 100% kill rate** post-`aa13c2e`. No surviving mutations on the `>` vs `>=` counter flip, the `Less`/`Greater` arm swap, or the tiebreak `>` vs `<` inversion in `LogicalTimestamp::dominates`. Gate threshold is ≥80%; 100% exceeds it.
- **Single-cut sweep**: `git grep 'lww_dominates' crates/overdrive-sim` returns no production matches; `git grep -E '#\[deprecated' crates/overdrive-{core,sim,store-local}` returns no matches in the diff.
- **Subscription emission semantics**: `git grep 'self.emit' crates/overdrive-store-local/src/observation_backend.rs` shows `emit` conditional on `accepted` post-spawn-blocking — not unconditional after `.await??`.

---

## Lessons learned

### 1. Trait under-specification IS the bug, even when implementations look correct

The visible bug was a missing `dominates` call on the production write
path. The *structural* bug was at the trait level: `ObservationStore`
did not specify ordering semantics for `write`. Two adapters claimed
the same trait and converged differently — sim enforced LWW; production
performed blind overwrite — and both were "correct" against the
unstated contract. The fix that closes the bug is not "add the
comparison call" — that closes the symptom. The fix that closes the
*cause* is "make the merge order part of the trait contract, in
rustdoc and in a generic conformance harness exercised against every
adapter." Same shape as the prior `fix-commit-index-per-entry`
evolution doc's lesson: trait-surface gaps that look cosmetic silently
foreclose the only correct implementation.

### 2. Conformance harnesses against `dyn Trait` belong in `core`, not in the adapter crates

`lww_dominates` lived in `overdrive-sim` because that's where the only
caller initially needed it. The moment a second implementation of the
same trait shipped (`LocalObservationStore`), the comparator was
unreachable from the place it was needed. The general pattern: any
total-ordering primitive (or any conformance harness) that's keyed off
a trait method belongs in the crate that owns the trait, not in the
crate that owns the first implementation. The new `test-utils` feature
on `overdrive-core` is the durable hook for any future
`ObservationStore` impl (Phase 2 `CorrosionStore`); the harness already
covers the new adapter without code change. Same hook will exist for
`IntentStore` once a second non-DST adapter lands.

### 3. Diff-scoped mutation runs need the owning crate's test suite to actually exercise the diff

`cargo xtask mutants` defaults `--test-workspace=false` per the
wrapper's per-package mode (large wall-clock win — only that crate's
test suite is rerun per mutation). That meant the comparator landed in
`overdrive-core` but its only callers were in `overdrive-store-local`
and `overdrive-sim` test suites — out of mutation scope. First pass
reported 5/6 missed not because the harness was weak but because the
mutants never reached it. The fix was to add a function-level
acceptance file in the owning crate's own suite (`aa13c2e`); the
trait-level harness still runs against both adapters under their own
test runners. The two layers are not redundant — they prove different
things. **Future patterns**: any new total-ordering primitive or pure
function landed in `overdrive-core` should ship with a function-level
test file in `overdrive-core/tests/acceptance/` even when the
"natural" exercise is from an adapter crate. Without it, the
mutation gate is structurally blind.

### 4. ADR third revisions go through the architect even when the code is already correct

The trait contract was documented inline on the
`ObservationStore::write` rustdoc and on `LogicalTimestamp::dominates`
within `a4e6527` — the source-of-truth at the code level was correct
the moment GREEN landed. ADR-0012's stale §"No CRDT machinery, by
design" claim still contradicted the source until `6d66b41`. Per
`feedback_delegate_to_architect` the ADR edit was dispatched to
`@nw-solution-architect`, not made inline by the crafter. Splitting it
into a separate commit (versus folding into the cohesive GREEN commit)
was a consequence of subagent capability — the GREEN-commit subagent
did not have the Agent tool available, so the architect dispatch
happened at parent-session level. Future bug-fix cycles should expect
the ADR-revision commit to be separate when the GREEN crafter is a
subagent without Agent dispatch capability; this is not a process
violation, just a tooling consequence.

---

## Risk delta (post-fix)

- **Wire-compatible**: `ObservationStore::write` signature unchanged;
  callers see no API surface change. Only the *acceptance criterion* for
  a write tightens — older rows arriving after newer rows are silently
  rejected and not emitted on subscriptions. No client-visible behaviour
  change in Phase 1 (no real writers); the structural protection
  activates the moment Phase 2 Corrosion writers land.
- **Phase 2 readiness**: when `CorrosionStore` (Phase 2 adapter) lands,
  it picks up the same trait contract automatically. The conformance
  harness is the load-bearing test surface — `CorrosionStore`'s test
  suite calls `run_lww_conformance(&store)` and inherits the full LWW
  property assertion set. No further trait change required.
- **DST untouched**: `SimObservationStore` is unchanged in observable
  behaviour — it already had LWW; the only change is that its
  comparator now lives in core. All existing DST tests
  (`sim_observation_lww_converges.rs`, `sim_observation_gossip_mechanics.rs`,
  etc.) continue to pass without modification (one minor mechanical
  edit in `sim_observation_gossip_mechanics.rs` to track the renamed
  delegate). The harness-determinism property is preserved because
  `LogicalTimestamp::dominates` is a pure function with no hidden state.
- **Performance**: one extra `table.get` + one rkyv `updated_at` decode
  per `write` on `LocalObservationStore`. μs-scale, well inside the
  100ms REST budget per ADR-0012.
- **Transaction shape**: read-then-conditional-insert inside one
  `begin_write` is standard redb usage; serialisable isolation already
  provided. No deadlock risk (one table per row variant, no cross-table
  ordering). The redb txn commits on every code path including LWW
  rejection; the lifecycle stays balanced.
- **Subscription semantics**: subscribers never see a row the store
  will refuse to return on read — closes the silent invariant violation
  where a stale row could be fanned out, returned to the subscriber,
  then refused on a subsequent `get`.
- **Existing acceptance tests**: `overwrite_same_key_replaces_first_row`
  (counters 1→2 monotonic) keeps passing under the new LWW guard since
  `2.dominates(&1) == true`.

---

## Files touched

15 files, +1167 / −121 across the four commits.

| Crate | Files | Notes |
|---|---|---|
| `overdrive-core` (src) | 4 | NEW `Cargo.toml` `[features]` adds `test-utils = []`; `src/lib.rs` exposes `#[cfg(any(test, feature = "test-utils"))] pub mod testing;`; `src/traits/observation_store.rs` adds `LogicalTimestamp::dominates` and tightens `ObservationStore::write` rustdoc with the LWW contract; NEW `src/testing/mod.rs` and `src/testing/observation_store.rs` expose `run_lww_conformance<T: ObservationStore + ?Sized>`. |
| `overdrive-core` (tests) | 2 | NEW `tests/acceptance.rs` entrypoint and NEW `tests/acceptance/logical_timestamp_dominates.rs` (mutation-gate fixup, commit `aa13c2e`) — table-driven coverage of every branch in `LogicalTimestamp::dominates`. |
| `overdrive-store-local` (src) | 1 | `src/observation_backend.rs` — read-then-conditional-insert inside `begin_write`; `accepted` boolean returned out of `spawn_blocking`; `self.emit(row)` gated on `accepted`. Both `AllocStatusRow` and `NodeHealthRow` paths. |
| `overdrive-store-local` (Cargo.toml) | 1 | `[dev-dependencies]` adds `overdrive-core/test-utils`. |
| `overdrive-store-local` (tests) | 3 | MOD `tests/integration.rs` wires new module; NEW `tests/integration/lww_conformance.rs` invokes `run_lww_conformance` against `LocalObservationStore`; MOD `tests/acceptance/local_observation_store.rs` appends `out_of_order_alloc_status_does_not_regress` + `out_of_order_node_health_does_not_regress`. |
| `overdrive-sim` (src) | 1 | `src/adapters/observation_store.rs` — DELETE local `fn lww_dominates`; `dominates_for_merge` delegates to `LogicalTimestamp::dominates` (preserves `canary-bug` flip); NEW `apply_node_health` parallel to `apply_alloc_status`; `node_health_rows()` returns LWW winners. |
| `overdrive-sim` (Cargo.toml) | 1 | `[dev-dependencies]` adds `overdrive-core/test-utils`. |
| `overdrive-sim` (tests) | 3 | MOD `tests/acceptance.rs` wires new module; NEW `tests/acceptance/lww_conformance.rs` invokes `run_lww_conformance` against `SimObservationStore::single_peer`; MOD `tests/acceptance/sim_observation_gossip_mechanics.rs` (mechanical update tracking renamed delegate). |
| `docs/product/architecture/` | 1 | ADR-0012 third revision via architect dispatch (commit `6d66b41`): strike §"No CRDT machinery, by design"; codify `write` LWW contract; keep §"Restart semantics"; add revision header. |
| `docs/feature/...` | (preserved) | `deliver/{rca.md, roadmap.json, execution-log.json}` — preserved per nw-finalize. |

---

## Notes on discarded workspace artifacts

This is a bugfix, not a feature. **No DESIGN, DISTILL, or DEVOPS waves
exist for this fix** — only `deliver/`. There are no lasting artefacts
to migrate to `docs/architecture/` or `docs/scenarios/`. The ADR-0012
third revision (commit `6d66b41`) is the architecture-document update
captured in the same iteration; no new ADR was required (the bug was a
conformance violation against the existing trait contract, not a new
architectural decision).

The full audit trail lives in:

- This evolution document — canonical post-mortem
- `docs/feature/fix-observation-lww-merge/deliver/rca.md` — authoritative
  RCA from `/nw-bugfix` Phase 1, user-approved at Phase 2; source
  specification for the four commits
- `docs/feature/fix-observation-lww-merge/deliver/roadmap.json` —
  2-step Outside-In TDD plan
- `docs/feature/fix-observation-lww-merge/deliver/execution-log.json` —
  DES phase log
- The four commit messages on `marcus-sa/phase-1-control-plane-core`

The `deliver/` directory is preserved per nw-finalize rules so the
wave matrix can derive feature status.

---

## Related

- **`ObservationStore::write` trait docstring** — `crates/overdrive-core/src/traits/observation_store.rs` — the LWW contract that this fix codified ("incoming rows whose `updated_at` does not dominate the existing row at the same primary key MUST NOT mutate state and MUST NOT be emitted on subscriptions").
- **`LogicalTimestamp::dominates`** — `crates/overdrive-core/src/traits/observation_store.rs` — the comparator at its new authoritative location; previously lived in `overdrive-sim` as the free function `lww_dominates`.
- **`run_lww_conformance<T>`** — `crates/overdrive-core/src/testing/observation_store.rs` — trait-generic conformance harness exposed via the `test-utils` feature; the durable hook for Phase 2 `CorrosionStore` and any future `ObservationStore` impl.
- **ADR-0012** — `docs/product/architecture/adr-0012-observation-store-server-impl.md` — third revision codifies the LWW trait contract; second revision (2026-04-24) had explicitly chosen "no CRDT machinery, by design" — that claim is struck.
- **ADR-0011** — Intent vs observation non-substitutability; the type-level discipline this fix complements with an ordering-level discipline.
- **Prior evolution**: `2026-04-25-fix-commit-index-per-entry.md` — same shape of "trait-surface gap silently forecloses the only correct implementation"; same mutation-gate discipline.
- **Memory**: `feedback_single_cut_greenfield_migrations` — single-cut applied to sim's local `lww_dominates`; deleted in the same commit as core's `dominates` lands.
- **Memory**: `feedback_delegate_to_architect` — ADR-0012 third revision dispatched to `@nw-solution-architect`, not edited inline.
