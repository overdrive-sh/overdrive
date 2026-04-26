# 2026-04-26 — fix-commit-counter-and-watch-doc

Two code-review defects against `crates/overdrive-store-local/`, both
landed as direct bugfix commits without a roadmap-driven DELIVER cycle
on branch `marcus-sa/phase-1-control-plane-core`. The git history is
the execution record — there is no `roadmap.json`, no
`execution-log.json`, and no `.develop-progress.json` for this feature.
This note is the post-mortem for the audit trail.

## Defects

### Bug 1 — Phantom `commit_index` advancement on failed writes

`bump_commit_inside` in `crates/overdrive-store-local/src/redb_backend.rs`
called `fetch_add` *before* `table.insert` / `write.commit()` on every
write path (`put`, `put_if_absent::Inserted`, `delete`, `txn`). A
failure between the bump and the durable redb commit advanced
`commit_counter` without a corresponding row on disk, contradicting the
"commit_index reflects committed writes" invariant in the module-header
docstring. Downstream callers — most notably the `cluster_status`
handler — rely on the counter being a strict upper bound on committed
state.

**Fix.** Split the helper into a peek + bump pair, both invoked inside
the `spawn_blocking` body that holds the redb write lock.
`peek_next_inside` returns `commit_counter + 1` without advancing — used
as the per-entry index prefix in `encode_entry`.
`bump_commit_after_commit` advances the counter and is called only
after `write.commit()` returns `Ok` *and* the operation had observable
effect (a successful insert; a remove that returned `Some(_)`; a `txn`
with at least one effective op gated by `effective.iter().any(...)`).
redb serialises writers, so the peek load is race-free without extra
locking; a `debug_assert_eq!` pins peek == bump in dev builds.

**Regression test.**
`crates/overdrive-store-local/tests/integration/commit_counter_invariant.rs`
asserts `commit_index() == effective_committed_op_count` after every
operation in arbitrary `put` / `delete` / `txn` sequences. The minimal
counter-example proptest's shrinker found against the unfixed code —
`Txn { ops: [Delete { key: [0] }] }` — is pinned in the checked-in
`.proptest-regressions` file so every future run exercises it first.

### Bug 2 — Subscription lag-drop docstring mismatch

Both `redb_backend.rs` and `observation_backend.rs` wrap
`BroadcastStream` in `filter_map(Result::ok)`, silently dropping
`Err(Lagged)` and keeping the stream alive. The intent backend
described this truthfully; the observation backend's module docstring
at `observation_backend.rs:30-35` claimed lag "is signalled
end-of-stream," contradicting the implementation.

**Fix.** Doc-only — aligned `observation_backend.rs` module docstring
with the actual `filter_map(Result::ok)` behaviour and the
intent-backend's truthful description. A behaviour change to actually
signal end-of-stream was rejected as Phase 2-shaped scope: every Phase 1
subscriber would need to grow reconnect logic for a recovery path the
platform docs already say belongs to Phase 2 gossip catch-up
(Corrosion subsumes the "should the stream close on lag?" question
entirely).

## Commits

- `f5b361c` — `fix(store-local): defer commit_counter bump until after redb commit`
- `df6d8c2` — `docs(store-local): align observation subscription lag-drop docstring with code`

## Lessons

- The reviewer's analysis was sufficient to scope both fixes — a fresh
  troubleshooter dispatch would have repeated their work. For
  well-scoped review defects, jump straight to a concise fix proposal.
- The "commit_index reflects committed writes" invariant was a
  module-header claim with no regression test. Bug 1 was only
  discoverable because a careful reviewer cross-checked the helper
  against the header invariant; the new proptest closes that loop and
  becomes the load-bearing artifact for future refactors of the write
  paths.
- When two backends share a behaviour but their docstrings drift, the
  right Phase 1 call is to align the docs to the implementation, not
  the other way around. Behavioural alignment becomes the Phase 2
  replacement's job.

## Migrated artifacts

- `docs/feature/fix-commit-counter-and-watch-doc/deliver/rca-context.md`
  → `docs/research/fix-commit-counter-and-watch-doc-rca.md`
  (original retained in `deliver/` for the wave matrix; copy migrated
  for long-term research reference).

## Supersedes

This note supersedes the placeholder
`docs/evolution/2026-04-25-fix-commit-counter-and-watch-doc.md` written
the day the fixes landed. The 2026-04-25 file is retained as-is for
audit continuity; this 2026-04-26 entry is the formal finalization
written through the standard wave-finalize workflow that matches the
`fix-xtask-mutants-zero-mutant-crash` precedent.
