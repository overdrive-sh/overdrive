# fix-commit-index-per-entry — Feature Evolution

**Feature ID**: fix-commit-index-per-entry
**Type**: Bug fix (via `/nw-bugfix` → `/nw-deliver` pipeline)
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Date**: 2026-04-25
**Commits**:
- `4fc5b51e5f5b91babbe2b660167aa22d3515b620` — `test(commit-index): RED scaffold pinning per-entry commit_index contract` (Step 01-01)
- `ba722978cea09924054c83ec81bdb25707726e1b` — `fix(intent-store,control-plane): make commit_index per-entry across PutOutcome and get` (Step 01-02)
- `c33027dd015d6243634ee878bb86271160306c72` — `docs(api): clarify per-entry vs store-wide commit_index in API rustdoc` (reviewer-driven follow-up to 01-02)

**Status**: Delivered (RED scaffold → cohesive GREEN cut → adversarial-review docstring follow-up; 2-step roadmap, 3 commits)

---

## Summary

Two control-plane endpoints (`describe_job` and the `submit_job::Inserted`
arm) populated their `commit_index` JSON field with the *current* store
counter at response-construction time, contradicting the per-entry contract
documented in `JobDescription`'s rustdoc and in
`docs/feature/phase-1-control-plane-core/discuss/user-stories.md:234`. The
bug was structurally invisible in Phase 1's walking-skeleton scope (single
job, no concurrent writers) but would surface immediately under any
distinct-key write following a previous one — `submit A → 17, submit B → 18,
describe A → returned 18 instead of 17`. This fix closes the trait-surface
gap (`PutOutcome::Inserted` and `KeyExists` now carry a `commit_index`
field; `IntentStore::get` now returns `Option<(Bytes, u64)>`), persists each
entry's `commit_index` inline alongside its value in redb as a packed
`[u64-LE-prefix || value]` frame, moves the bump-and-capture inside the
`spawn_blocking` write transaction so per-entry index assignment is atomic
with the redb commit, and bumps the snapshot frame from v1 to v2 to carry
indices through the single → HA migration story (with v1 forward-compat
preserved). The OpenAPI wire shape is unchanged (`u64` fields stay `u64`);
only semantics tighten — clients receive a more stable value.

---

## Root cause

Full RCA at `docs/feature/fix-commit-index-per-entry/deliver/rca.md`. The
multi-causal 5 Whys, reproduced verbatim:

### Branch A — `describe_job` returns wrong index

- **WHY 1A**: Reader returns store-wide current counter, not the index at
  which this entry was committed. *Evidence*: `handlers.rs:180`
  `commit_index: state.store.commit_index()` after `get` returned bytes.
- **WHY 2A**: `IntentStore::get` returns `Option<Bytes>` only — no index.
  *Evidence*: `crates/overdrive-core/src/traits/intent_store.rs:140`.
- **WHY 3A**: No durable per-entry index is stored alongside the value.
  redb table is `entries: &[u8] -> &[u8]`
  (`crates/overdrive-store-local/src/redb_backend.rs:55`); the counter
  is process-local `AtomicU64` (`redb_backend.rs:91`).
- **WHY 4A**: Phase 1 design treats `commit_index` as a "store cursor"
  (a single inherent accessor on `LocalIntentStore`, `redb_backend.rs:139`)
  — there was never a value-attached index in the trait surface; the
  wire field name pretended otherwise.
- **WHY 5A — ROOT CAUSE A**: **Trait-surface gap.** `IntentStore` exposes
  no `(value, index)` read primitive. The trait could not satisfy a
  per-entry contract even if the handler tried.

### Branch B — `submit_job::Inserted` reads outside the txn

- **WHY 1B**: Counter is read on the calling task after `put_if_absent`
  returns. *Evidence*: `handlers.rs:90-94`.
- **WHY 2B**: The `bump_commit` happens INSIDE the `spawn_blocking` body
  (`redb_backend.rs:238`), but `PutOutcome::Inserted` is unit-like
  (`intent_store.rs:75`), carrying no index back to the caller.
- **WHY 3B**: redb serializes write txns, so `put_if_absent` for *the
  same key* is safe; but two writes for *different keys* (A then B)
  interleave freely between commit-N's bump and the handler's read of
  `commit_index()`.
- **WHY 4B**: Atomicity contract was framed around "no double-write at
  the same key" (TOCTOU on the key). It was not framed around "the index
  returned to caller equals the index at which their bytes landed."
- **WHY 5B — ROOT CAUSE B**: **`PutOutcome::Inserted` carries no index.**
  The atomic compare-and-set returns a unit-like marker; the caller has
  no way to obtain the index assigned inside the same transaction.

### Branch C — Doc/code drift not caught by tests

- **WHY 1C**: Existing tests assert global monotonicity
  (`crates/overdrive-store-local/tests/acceptance/commit_index_monotonic.rs`)
  and TOCTOU-single-winner
  (`crates/overdrive-control-plane/tests/integration/concurrent_submit_toctou.rs`);
  none assert `describe(A).commit_index == submit(A).commit_index` after
  intervening writes.
- **WHY 2C**: The user-stories example documenting the exact failure
  shape — "submit A → 17, submit B → 18, describe A → still 17"
  (`docs/feature/phase-1-control-plane-core/discuss/user-stories.md:234`)
  was never lifted into a test fixture.
- **WHY 5C — ROOT CAUSE C**: **Missing regression test for per-entry
  semantics.** The contract is written down in user stories and
  `JobDescription`'s rustdoc (`api.rs:50-52` "the commit index at which
  it was written"); no test mechanically pins it.

---

## Decisions made

### 1. Single-cut additive trait change — no shim, no second variant

Per `feedback_single_cut_greenfield_migrations`. The trait surface is a
clean break inside the workspace; every caller of `PutOutcome` and
`IntentStore::get` is updated in the same commit (`ba72297`). No
`PutOutcome::InsertedV2`, no `get_with_index` companion method, no
`#[deprecated]` aliases. A `git grep -E
'(InsertedV2|get_with_index|#\[deprecated\])' crates/overdrive-{core,store-local,control-plane}`
sweep returns empty post-fix — it is part of the quality gate.

### 2. Per-entry index storage layout — packed frame, single redb table

Two options were on the table per the roadmap notes:

- **(a)** parallel `entry_index: &[u8] -> u64` redb table written in the
  same write txn as `entries`
- **(b)** packed `[u64-LE-prefix || value]` frame in the existing entries
  table

The crafter chose option (b) — packed frame in the existing `ENTRIES_TABLE`.
Rationale (documented in `crates/overdrive-store-local/src/redb_backend.rs`
module docstring, with helper functions `encode_entry` and `decode_entry`
landing alongside the trait change): keeping a single redb table simplifies
the snapshot frame v2 schema (one logical row per key, no cross-table
join during export), and keeps the write txn narrower (one `insert` per
key, not two). The cost — every reader must slice off the 8-byte prefix —
is negligible relative to the redb read cost itself, and is encapsulated
inside `decode_entry` (one call site per read path).

### 3. Bump-and-capture moved INSIDE `spawn_blocking`

Closes Branch B of the RCA. Pre-fix, `put_if_absent` ran the redb commit
inside `spawn_blocking` and `bump_commit()` outside it on the calling task
after `.await`. Post-fix, both happen inside the same closure: the new
helper `Inner::bump_commit_inside` is invoked at the same atomic point
where the redb write transaction commits, and the resulting index is
baked into `PutOutcome::Inserted::commit_index` before the closure returns.
The inherent `LocalIntentStore::commit_index()` accessor (still consumed
by `cluster_status`) keeps tracking the global cursor — it is incremented
by the same `bump_commit_inside` call, so the global cursor and the
per-entry index advance together inside the same transaction. No possible
inversion under concurrent writers. A grep on `redb_backend.rs` post-fix
shows no `bump_commit*` call outside a `spawn_blocking` body for the put
/ put_if_absent / delete / txn paths — this is part of the quality gate.

### 4. `ClusterStatus.commit_index` keeps store-wide cursor semantics

`ClusterStatus.commit_index` is correct as a store-wide commit count and
intentionally NOT switched to per-entry. Only `SubmitJobResponse.commit_index`
and `JobDescription.commit_index` had their semantics tightened. The
adversarial review of Step 01-02 flagged that the rustdoc on
`ClusterStatus.commit_index` did not explicitly distinguish itself from
the per-entry pair — this was addressed in commit `c33027d` with explicit
"per-entry semantics" sections on `SubmitJobResponse` / `JobDescription`,
a "store-wide cursor semantics" section on `ClusterStatus`, and intra-doc
links cross-referencing them so future readers are routed between them.

### 5. Snapshot frame v1 → v2, with v1 forward-compat (decode-only)

Magic stays `OSNP`; version word advances 1 → 2; rkyv payload schema gains
a per-entry `commit_index: u64`. `decode` accepts both v1 (every entry
projected to `commit_index = 0` for forward-compat with any DR backups
predating this fix) and v2; `encode` always emits v2. `bootstrap_from`
raises the global counter to the max imported index so subsequent writes
do not collide with bootstrapped entries. This is the ONLY place v1
compatibility is preserved — every other code path went through the
single-cut migration.

### 6. OpenAPI wire shape unchanged — semantics-only tightening

The JSON shape on `SubmitJobResponse.commit_index` and
`JobDescription.commit_index` remained `u64` fields with the same names;
clients are wire-compatible. What changed is the *value semantics* — the
field now refers to the index at which this entry was written, not the
live store cursor at response-construction time. Clients receive a more
stable value (does not advance when other keys are written). The
docstring tightening in commit `c33027d` propagated through `cargo xtask
openapi-gen` regeneration so `api/openapi.yaml` matches the new rustdoc
one-to-one (utoipa derives JSON-Schema descriptions from rustdoc per
ADR-0009; `openapi-check` is the CI gate).

### 7. Step 01-01 RED scaffolds → flipped GREEN inside Step 01-02

The first commit (`4fc5b51`) lands three failing tests: a store-layer test
that does not even compile against pre-fix code (because it references
`PutOutcome::Inserted { commit_index }` and tuple-destructures `get`'s
return), a control-plane integration test lifting the user-story scenario
verbatim (fails at runtime against pre-fix), and a tightened
`concurrent_submit_toctou.rs` assertion. Per `.claude/rules/testing.md
§"RED scaffolds and intentionally-failing commits"` the commit was made
with `--no-verify` and a body explicitly calling out the intentional RED
state. The cohesive Step 01-02 commit (`ba72297`) flipped all three GREEN
without modification — this is what proves the fix is load-bearing and
that the tests pin the invariant.

---

## Steps completed

2 phases, 2 roadmap steps, 3 commits. RED → GREEN → docstring follow-up.

| Step ID | Phase | Status | Commit | Notes |
|---|---|---|---|---|
| 01-01 | RED scaffold | PASS | `4fc5b51` | Three new failing tests pin the per-entry contract: store-layer (`per_entry_commit_index.rs` won't compile pre-fix), control-plane (`per_entry_commit_index.rs` lifts user-stories.md:234 verbatim, fails at runtime pre-fix), tightened `concurrent_submit_toctou.rs::concurrent_byte_identical_submits_return_single_commit_index` with follow-on describe assertion. Committed `--no-verify` per testing.md §RED scaffolds. |
| 01-02 | GREEN cohesive cut | PASS | `ba72297` | Trait additive change (`PutOutcome::Inserted/KeyExists` carry `commit_index`; `IntentStore::get → Option<(Bytes, u64)>`); per-entry index packed into entries table as `[u64-LE-prefix \|\| value]`; bump-and-capture moved inside `spawn_blocking`; snapshot frame v2 with v1 decode-only forward-compat; handler swap (submit + describe consume from variants/tuple, cluster_status keeps global cursor); 530/530 nextest pass; doctests green; clippy clean. |
| 01-02 (follow-up) | Reviewer-driven docstring tightening | PASS | `c33027d` | Adversarial review APPROVED Step 01-02 with one HIGH note: `ClusterStatus.commit_index` rustdoc did not explicitly distinguish store-wide-cursor semantics from the per-entry pair. `api.rs` rustdocs gain explicit per-entry / store-wide semantics sections with intra-doc cross-references; `api/openapi.yaml` regenerated via `cargo xtask openapi-gen` so the wire descriptions match one-to-one. No code or wire-shape change. |

DES execution log: `docs/feature/fix-commit-index-per-entry/deliver/execution-log.json`
— integrity verifier `All 2 steps have complete DES traces` at finalize time.

### Quality gates

- **Full workspace tests**: `cargo nextest run --workspace --features integration-tests` exits 0; **530/530** pass.
- **Doctests**: `cargo test --doc --workspace` exits 0.
- **Clippy**: `cargo clippy --all-targets --features integration-tests -- -D warnings` clean.
- **OpenAPI drift gate**: `cargo xtask openapi-gen` regenerated `api/openapi.yaml` after `c33027d`; `openapi-check` CI gate green.
- **Single-cut sweep**: `git grep -E '(InsertedV2|get_with_index|#\[deprecated\])' crates/overdrive-{core,store-local,control-plane}` empty.
- **Bump-inside-spawn-blocking**: grep on `redb_backend.rs` confirms no `bump_commit*` outside a `spawn_blocking` body for write paths.

### Mutation testing — skipped at user request

`cargo xtask mutants --diff origin/main` was scoped at finalize time and
identified 38 mutants on the diff (PutOutcome variant assignment, get
tuple's index field, snapshot frame v2 round-trip, handler consumption
sites). Execution was skipped at user request (`kill mutation test and
finalize feature`); no mutated source on disk; no kill-rate gate enforced
for this scope.

This skip is documented for traceability rather than treated as a quality
regression for this scope:

- **Code surface is small** — the per-entry contract change touches four
  production files; the diff is reviewable in a single sitting.
- **Regression tests pin the contract directly** — both Step 01-01 RED
  scaffolds (the user-story scenario and the store-layer per-entry
  assertions) flipped GREEN inside the cohesive Step 01-02 commit. Any
  mutation that breaks per-entry semantics fails the user-story test by
  construction.
- **OpenAPI gate covers the wire shape** — `cargo xtask openapi-gen` +
  `openapi-check` CI catch any rustdoc drift that would propagate into
  the wire schema.

A subsequent change touching `intent_store.rs`, `redb_backend.rs`,
`snapshot_frame.rs`, or the `handlers.rs` submit/describe flows should
trigger a mutation re-run scoped to the diff.

---

## Lessons learned

### 1. Walking-skeleton scope hides contract-violation bugs until concurrency lands

Phase 1's intentional walking-skeleton shape — single job, no concurrent
writers, no multi-key churn — made this bug structurally invisible in CI.
Every existing test asserted *global monotonicity* (`commit_index` strictly
increasing across writes) and *TOCTOU-single-winner* (concurrent
byte-identical submits return one shared `commit_index`); none asserted
the property the wire field's name and rustdoc had been promising all
along — `describe(A).commit_index == submit(A).commit_index` after an
intervening distinct-key write.

The exact failure shape was already documented in
`docs/feature/phase-1-control-plane-core/discuss/user-stories.md:234`:
"submit A → 17, submit B → 18, describe A → still 17". Branch C of the
RCA names this gap as ROOT CAUSE C — the user-story example was never
lifted into a test fixture. **Future Phase 1 features should default to
lifting user-story examples into regression tests at distill time.**
This is the same shape as the `fix-cli-cannot-reach-control-plane`
lesson where every existing integration test passed one `TempDir` as
both `data_dir` and operator-config root, manufacturing a coincidence
the production binary never exhibits — both are "the test suite was
the wrong shape, not the code under test."

### 2. Trait-surface gaps that look cosmetic silently force handlers into wrong-shape behaviour

The visible bug was a handler-side line — `commit_index:
state.store.commit_index()` in `describe_job` (`handlers.rs:180`) reading
the live counter instead of the per-entry index. But the *structural* bug
was at the trait surface: `IntentStore::get` returned `Option<Bytes>` —
not `Option<(Bytes, u64)>` — so even if the handler had wanted to do the
right thing, the trait could not deliver. Same shape on the write side:
`PutOutcome::Inserted` was unit-like, so the index assigned inside the
write txn could not flow back to the caller.

This is a recurring shape on this codebase: a missing tuple element on a
return type, or a missing field on an enum variant, looks cosmetic in
review but silently forecloses the only correct implementation. The
defensive posture is **make the trait surface carry the data the contract
promises** — the handler-side fix becomes mechanical once the trait
shape is right. Both Step 01-01's failure modes (compile error on the
store side, runtime assertion failure on the control-plane side)
followed directly from this — the test simply asked for the data the
trait did not provide.

### 3. The OpenAPI rustdoc IS the wire contract via utoipa derivation

A docstring-only change in commit `c33027d` would have been a no-op in a
codebase where rustdoc and OpenAPI are independent. In this codebase
they are not — utoipa derives JSON-Schema descriptions from rustdoc
(per ADR-0009), and `cargo xtask openapi-gen` regenerates
`api/openapi.yaml` from the rustdoc. A rustdoc tightening without
regeneration drifts the yaml, which the `openapi-check` CI gate catches
post-hoc. The reviewer-driven docstring follow-up (`c33027d`) ran
`openapi-gen` immediately and committed both files together — this is
the correct discipline; future rustdoc changes on `api.rs` types must
follow the same pattern.

### 4. Mutation testing skip is documented for traceability, not minimised

Skipping mutation testing at user request is a *quality decision*, not a
quality regression — but it is a decision that future maintainers need
to see. Recording the skip explicitly (with the reasons it is acceptable
for this scope) gives a follow-up reviewer the option to re-run scoped
to the diff if any subsequent change touches the same files. The pattern
matches the prior `fix-cli-cannot-reach-control-plane` evolution doc,
which also recorded a partial mutation run with the missed mutations
attributed to pre-existing code rather than the bugfix diff.

---

## Risk delta (post-fix)

- **Wire-compatible**: `u64` fields stay `u64`; field names unchanged;
  clients deserialising `SubmitJobResponse` / `JobDescription` continue to
  parse without modification. Only value semantics tighten — the field
  becomes a more stable per-entry pointer rather than the live store
  cursor at response-construction time.
- **Single → HA migration story preserved**: snapshot frame v2 carries
  per-entry indices through `export_snapshot` → `bootstrap_from` →
  `export_snapshot` byte-identically; `bootstrap_from` raises the global
  counter to the max imported index so post-bootstrap writes do not
  collide; v1 frames decode with `commit_index = 0` per-entry for
  forward-compat with DR backups predating this fix. `RaftStore` (Phase
  2) wires real Raft log indices through the same `PutOutcome` shape with
  no further trait change required.
- **DST untouched**: `SimObservationStore` is observation, not intent
  (`crates/overdrive-store-local/tests/integration/snapshot_proptest.rs`
  exercises real `LocalIntentStore`). `LocalStore` IS the sim intent
  store per `.claude/rules/testing.md §Store composition`; it now
  exposes per-entry semantics through the new trait shape, so any future
  DST test asserting on `commit_index` semantics picks up the new
  contract automatically. A grep across `crates/*/tests/` confirmed no
  DST tests asserted on per-entry semantics today (only global
  monotonicity), so no breakage.
- **Watch streams unaffected**: watch events carry `(key, value)` not
  indices (per `IntentStore::watch` trait docstring); no contract change.
- **CLI**: `overdrive job submit` / `overdrive job describe` already
  render the field; printed number changes meaning but the type and
  field name do not.

---

## Files touched

23 files, +820 / −167 across the three commits.

| Crate | Files | Notes |
|---|---|---|
| `overdrive-core` (src) | 1 | `traits/intent_store.rs` — `PutOutcome::Inserted/KeyExists` carry `commit_index`; `IntentStore::get → Option<(Bytes, u64)>`; rustdoc tightened. |
| `overdrive-store-local` (src) | 2 | `redb_backend.rs` — packed `[u64-LE \|\| value]` frame, `encode_entry`/`decode_entry` helpers, `bump_commit_inside` invoked in `spawn_blocking`. `snapshot_frame.rs` — v2 schema with per-entry indices, v1 decode-only forward-compat. |
| `overdrive-store-local` (tests) | 6 | NEW `acceptance/per_entry_commit_index.rs`; MOD `acceptance/{put_if_absent, commit_index_monotonic, snapshot_roundtrip, local_store_basic_ops, local_store_error_paths}.rs`; MOD `tests/local_store_edges.rs`; MOD `integration/snapshot_proptest.rs`. |
| `overdrive-control-plane` (src) | 2 | `handlers.rs` — `submit_job` consumes `commit_index` from `PutOutcome` variants; `describe_job` consumes from `get` tuple; `cluster_status` keeps global cursor. `api.rs` (commit `c33027d`) — per-entry vs store-wide rustdoc with intra-doc links. |
| `overdrive-control-plane` (tests) | 5 | NEW `integration/per_entry_commit_index.rs`; MOD `integration/{concurrent_submit_toctou, idempotent_resubmit, submit_round_trip}.rs`; MOD `acceptance/submit_job_idempotency.rs`. |
| `api/openapi.yaml` | 1 | Regenerated via `cargo xtask openapi-gen` after `c33027d`'s rustdoc tightening. |
| `docs/feature/...` | (preserved) | `deliver/{rca.md, roadmap.json, execution-log.json}` — preserved per nw-finalize. |

---

## Notes on discarded workspace artifacts

This is a bugfix, not a feature. **No DESIGN, DISTILL, or DEVOPS waves
exist for this fix** — only `deliver/`. There are no lasting artefacts
to migrate to `docs/architecture/`, `docs/adrs/`, `docs/scenarios/`, or
`docs/ux/`. No new ADRs (the bug was a conformance violation against
existing trait contracts and existing wire-field rustdoc; the fix
restored conformance).

The full audit trail lives in:

- This evolution document — canonical post-mortem
- `docs/feature/fix-commit-index-per-entry/deliver/rca.md` — authoritative
  RCA from `/nw-bugfix` Phase 1, user-approved at Phase 2; source
  specification for the commits
- `docs/feature/fix-commit-index-per-entry/deliver/roadmap.json` —
  2-step Outside-In TDD plan
- `docs/feature/fix-commit-index-per-entry/deliver/execution-log.json` —
  DES phase log
- The three commit messages on `marcus-sa/phase-1-control-plane-core`

The `deliver/` directory is preserved per nw-finalize rules so the wave
matrix can derive feature status. Session markers
(`.nwave/des/{deliver-session.json, des-task-active,
des-task-active-fix-commit-index-per-entry--}` and
`docs/feature/fix-commit-index-per-entry/deliver/.develop-progress.json`)
were cleaned at finalize time.

---

## Related

- **User-story example** — `docs/feature/phase-1-control-plane-core/discuss/user-stories.md:234`
  ("submit A → 17, submit B → 18, describe A → still 17") — the exact
  failure shape, documented but not lifted into a test fixture until
  this fix.
- **`JobDescription` rustdoc** — `crates/overdrive-control-plane/src/api.rs:50-52`
  — "the commit index at which it was written"; the contract this fix
  finally honours.
- **ADR-0009** — utoipa-derived OpenAPI; rustdoc IS the wire contract.
- **ADR-0015** — Phase 1 IntentStore contract.
- **Prior evolution**: `2026-04-25-fix-cli-cannot-reach-control-plane.md`
  — the prior bugfix on the same Phase 1 walking-skeleton scope; same
  shape of "test suite manufactured a coincidence the production binary
  never exhibits."
- **Memory**: `feedback_single_cut_greenfield_migrations` — single-cut
  applied to the trait surface change; no compatibility shim, no second
  variant, no `#[deprecated]` aliases.
