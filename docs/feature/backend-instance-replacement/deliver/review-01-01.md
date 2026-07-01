# Adversarial Review — backend-instance-replacement step 01-01

**Reviewer**: Codex (`/nw-review`, adversarial)
**Date**: 2026-06-30
**Step**: `01-01` — `TxnOp::IncrementU64` store primitive
**Verdict**: `approved`

## Scope Reviewed

- `docs/feature/backend-instance-replacement/deliver/roadmap.json`
- `docs/feature/backend-instance-replacement/deliver/mutants-01-01.md`
- `docs/feature/backend-instance-replacement/deliver/execution-log.json`
- `crates/overdrive-core/src/traits/intent_store.rs`
- `crates/overdrive-store-local/src/redb_backend.rs`
- `crates/overdrive-store-local/tests/acceptance.rs`
- `crates/overdrive-store-local/tests/acceptance/txn_increment_u64.rs`
- `crates/overdrive-control-plane/src/action_shim/mod.rs`
- `crates/overdrive-control-plane/tests/acceptance/workflow_emit_action_lands_in_raft_channel.rs`

## Findings

No blocking issues found.

The prior stale blockers for this review are now closed:

- Mutation evidence is present in `docs/feature/backend-instance-replacement/deliver/mutants-01-01.md:15`, recording 11/11 caught whole-file mutants and 100% kill rate. The same evidence honestly documents that cargo-mutants produces no `txn` / `IncrementU64` mutant for the load-bearing arm (`:73-88`) and records a manual `+1 -> +0` mutation proof where `S-BIR-TXN-02` fails as required (`:95-122`).
- The named test doubles no longer silently ignore the new variant. `FaultInjectingIntentStore` applies `TxnOp::IncrementU64` with length-guarded BE-u64 decode and saturating increment (`crates/overdrive-control-plane/src/action_shim/mod.rs:1914-1949`), and `CountingIntentStore` has an exhaustive match that includes `IncrementU64` (`crates/overdrive-control-plane/tests/acceptance/workflow_emit_action_lands_in_raft_channel.rs:147-158`).

## Acceptance Coverage

- `S-BIR-TXN-01`: `single_txn_bumps_generation_and_clears_present_stop_sentinel` covers absent generation + present stop sentinel, then asserts generation `1` and stop deletion (`crates/overdrive-store-local/tests/acceptance/txn_increment_u64.rs:45-77`).
- `S-BIR-TXN-02`: `concurrent_restart_txns_never_lose_a_bump_final_generation_equals_n` runs 32 concurrent txns, asserts all commit, post-commit reads remain in range, and final generation equals `N` (`:107-183`).
- `S-BIR-TXN-03`: `absent_keys_bump_to_one_and_delete_of_absent_stop_is_a_noop` covers the no-generation/no-stop edge (`:193-226`).
- `S-BIR-TXN-04`: `corrupt_short_row_decodes_as_zero_then_bumps_to_one` covers non-8-byte row handling without panic (`:236-270`).
- Extra useful coverage pins saturation (`:281-316`) and deterministic sequential monotonicity (`:329-372`).
- The acceptance module includes the new test file under the `integration-tests` gate (`crates/overdrive-store-local/tests/acceptance.rs:26-28`).

## Implementation Assessment

The production arm performs the read-modify-write inside one redb `begin_write` / `commit` span (`crates/overdrive-store-local/src/redb_backend.rs:321-376`). The generation value is decoded with `<[u8; 8]>::try_from(...)`, defaults absent/corrupt rows to `0`, increments with `saturating_add(1)`, and writes canonical 8-byte BE bytes (`:357-370`). That satisfies the roadmap's atomic monotonic bump-and-clear contract (`docs/feature/backend-instance-replacement/deliver/roadmap.json:16-22`).

The trait surface carries the required behavior contract for `TxnOp::IncrementU64` and `IntentStore::txn`: preconditions, postconditions, edge cases, saturation, atomic sibling ops, and concurrent no-lost-increment invariant (`crates/overdrive-core/src/traits/intent_store.rs:71-120`, `:257-288`).

## Non-Blocking Notes

- `docs/feature/backend-instance-replacement/deliver/execution-log.json` still does not list a mutation phase; the mutation evidence is in `mutants-01-01.md`. That is acceptable for this review because the evidence artifact is present and specific, but keeping the execution log in sync would reduce future review ambiguity.
- `git diff --check origin/main...HEAD` still reports trailing whitespace in backend-instance-replacement design review markdown files. I did not count this against step `01-01` because it is outside the store primitive files.

## Verification Run

```bash
cargo test -p overdrive-store-local --test acceptance txn_increment_u64 --features integration-tests
# 6 passed

cargo test -p overdrive-store-local --test acceptance --features integration-tests
# 42 passed; 1 ignored
```

## Decision

Approve step `01-01`. The production primitive, trait contract, acceptance tests, named doubles, and mutation evidence satisfy the step contract.
