# 2026-04-25 — fix-commit-counter-and-watch-doc

Two code-review defects against `crates/overdrive-store-local/`, both
landed as direct bugfix commits without a roadmap-driven DELIVER
cycle. This note is the post-mortem for the audit trail.

## Defects

### Bug 1 — Phantom `commit_index` advancement on failed writes

`bump_commit_inside` called `fetch_add` *before* `table.insert` /
`write.commit()` in every write path. A failure between the bump and a
successful commit advanced `commit_counter` without a corresponding
on-disk row, contradicting the "commit_index reflects committed writes"
invariant in the module-header docstring.

**Fix.** Split the helper into a peek + bump pair. `peek_next_inside`
returns `commit_counter + 1` without advancing — used as the per-entry
index prefix in `encode_entry`. `bump_commit_after_commit` advances the
counter and is called only after `write.commit()` returns `Ok` *and*
the op had observable effect (a successful insert; a remove that
returned `Some(_)`; a `txn` with at least one effective op). redb's
write serialisation guarantees the peeked and bumped values match;
`debug_assert_eq!` pins the invariant in dev builds.

Applied to `put`, `put_if_absent::Inserted`, `delete`, and `txn`.

**Regression test.** `tests/integration/commit_counter_invariant.rs`
asserts `commit_index() <= entries_on_disk_count()` after arbitrary
sequences of write ops, catching phantom advancement under any failure
source.

### Bug 2 — Subscription lag-drop docstring mismatch

Both backends use `BroadcastStream::filter_map` to drop `Err(Lagged)`
silently and keep the stream alive. `redb_backend.rs` described this
truthfully; `observation_backend.rs:30-35` claimed lag "is signalled
end-of-stream," contradicting the implementation.

**Fix.** Doc-only — aligned `observation_backend.rs` module docstring
with the actual `filter_map(Result::ok)` behaviour and the
intent-backend's truthful description. Phase 2's Corrosion gossip
catch-up remains the recovery path of record. A behaviour change to
actually signal end-of-stream was rejected as Phase 2-shaped scope.

## Commits

- `f5b361c` — `fix(store-local): defer commit_counter bump until after redb commit`
- `df6d8c2` — `docs(store-local): align observation subscription lag-drop docstring with code`

## Lessons

- The reviewer's analysis was sufficient — a fresh troubleshooter
  dispatch would have repeated their work. For well-scoped review
  defects, jump straight to the user-review phase with a concise fix
  proposal.
- The "commit_index reflects committed writes" invariant is a
  module-header claim that needs a regression test, not just a
  docstring. Bug 1 was discoverable only because a careful reviewer
  cross-checked the helper against the header invariant; the
  invariant test now closes that loop.
- When two backends share a behaviour but their docstrings drift,
  the right Phase 1 call is to align the docs to the implementation,
  not the other way around. Behavioural alignment becomes the
  Phase 2 replacement's job (gossip-driven catch-up subsumes the
  "should the stream close on lag?" question entirely).

## Migrated artifacts

None — this feature produced no design, scenario, or UX artifacts.
The RCA scratch note (`deliver/rca-context.md`) is process scaffolding
and is captured here.
