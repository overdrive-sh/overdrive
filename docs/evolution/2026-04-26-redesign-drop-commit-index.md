# 2026-04-26 — redesign-drop-commit-index

A scoped, full-wave nWave redesign that **deletes** `commit_index` from
the Phase 1 `IntentStore` trait surface, the `LocalIntentStore`
implementation, and the public REST/CLI wire contract. The field is
structurally redundant with `spec_digest` on the per-write surface,
has no consumer in the codebase, cannot meet its Phase 2 forward-compat
promise, and produced three production bugs in 26 hours. Phase 1 is
unshipped; greenfield single-cut migration discipline applies (see
project memory `feedback_single_cut_greenfield_migrations`).

- **Branch**: `marcus-sa/phase-1-control-plane-core`
- **Architect**: Morgan (DESIGN), Apex (DEVOPS)
- **Crafter**: Casey (DELIVER)
- **DESIGN landed**: 2026-04-26 (`ff86291`, ADR-0020 Proposed)
- **DELIVER landed**: 2026-04-26 — 2026-04-27 (commits `f6b21b8` →
  `d17c490`, ADR-0020 flipped to Accepted)
- **Phase 4 review**: APPROVED — zero defects, zero testing theater
- **ADR**: [ADR-0020](../product/architecture/adr-0020-drop-commit-index.md)
  (Accepted)

## Outcome

`commit_index` is gone from Phase 1. Three response types changed shape.
One snapshot frame version reverted. One DST invariant added.
Four reconciler/store integration tests deleted; fourteen tests revised
to pin the new shape; zero behavioural tests weakened. All gates green.

| Surface | Before | After |
|---|---|---|
| `IntentStore::commit_index()` trait method | present, returned `u64` | **deleted** |
| `LocalIntentStore::commit_counter` field | `AtomicU64`, peek + bump helpers | **deleted** |
| Snapshot frame `LOCAL_STORE_FRAME_V2` | embedded `commit_counter` | **reverted to v1** (no counter) |
| `ClusterStatus` JSON | `{mode, region, reconcilers, broker, commit_index}` | `{mode, region, reconcilers, broker}` (**pure drop, no replacement**) |
| `SubmitJobResponse` JSON | `{job_id, commit_index}` | `{job_id, spec_digest, outcome}` (replacement fields per ADR) |
| `JobDescription` JSON | `{spec, commit_index}` | `{spec, spec_digest}` |
| `IdempotencyOutcome` enum | absent | new, `"inserted" \| "unchanged"`, `Copy + Eq` |
| DST invariant catalogue | 10 invariants | 11 (added `intent-store-returns-caller-bytes`) |

## Bug cascade context — "renaming the same race"

This is the redesign that ends a 26-hour bug cascade. Three production
defects against `commit_index`, in chronological order:

1. **2026-04-25 (`fix-commit-index-per-entry`)** — the helper attached
   the same `commit_counter` value to every row in a multi-op `txn`,
   collapsing observability of in-transaction order.
2. **2026-04-25 (`fix-commit-counter-and-watch-doc`)** — `bump_commit_inside`
   advanced the counter *before* the redb commit, so a failed write
   left `commit_index` ahead of durable state.
3. The third in-flight investigation surfaced during Phase 0 of this
   redesign: even with both fixes, `commit_index` cannot serve a Phase 2
   Raft cluster — the field's documented forward-compat promise
   ("operators rely on this monotonic counter across single → HA
   migration") is impossible to satisfy because Raft log indices are
   committed-at-quorum, not committed-at-leader-write, and the two
   spaces have no canonical mapping.

The structural argument in ADR-0020 is **"renaming the same race."**
Each previous fix renamed the visible artifact of a deeper redundancy
without removing it. `spec_digest` (already on the wire per ADR-0008)
is the per-write content witness operators actually need; `commit_index`
duplicates the same signal at a coarser granularity, in a way that
cannot survive Phase 2. Pure deletion is the only fix that doesn't
re-cycle the same bug class under a different name.

## Commits

```
ff86291  docs(architecture): ADR-0020 drop commit_index from Phase 1
         — DESIGN landing: ADR + DISCUSS user-story revisions + DISTILL
           scenario revisions + walking-skeleton (Proposed)

f6b21b8  feat!(intent-store): drop commit_index — RED scaffold (ADR-0020)
         — Step 01-01. Deleted IntentStore::commit_index() trait method.
           RED_ACCEPTANCE pin: walking-skeleton compile-fail counter-test.
           RED_UNIT pin: trait-surface compile-fail.

2b12030  refactor(store-local): drop commit_counter and revert snapshot
         frame to v1 (ADR-0020)
         — Step 01-02. Deleted commit_counter: AtomicU64, peek_next_inside
           and bump_commit_after_commit, the inline row-encoding prefix.
           Reverted LOCAL_STORE_FRAME_V2 → v1. Added DST invariant
           intent-store-returns-caller-bytes as the structural-regression
           guard for byte-identity on the read path.

7388ef5  refactor(cli,api): regenerate OpenAPI + update CLI rendering
         for ADR-0020 wire shape (RED scaffold for 01-04)
         — Step 01-03. Regenerated openapi.json. Added IdempotencyOutcome
           enum (Copy + Eq + Serialize + Deserialize + ToSchema,
           rename_all = "lowercase"). Reshaped SubmitJobResponse,
           JobDescription, ClusterStatus per ADR. CLI render
           updated to display spec_digest + outcome on submit, drop
           commit_index column from cluster status.

34b6a44  test: drop commit_index test surface and revise survivors
         for ADR-0020
         — Step 01-04. Deleted 4 tests entirely:
           commit_counter_invariant.rs (the proptest from the prior
           bugfix — its invariant is gone with the field), and three
           per-feature tests pinning commit_index propagation.
           Revised 14 surviving tests to assert against the new wire
           shape (spec_digest + IdempotencyOutcome on submit;
           four-field cluster_status; two-field describe_job).
           Zero assertions weakened.

d17c490  docs(architecture): amend ADRs 0008/0015 + brief + flip
         ADR-0020 to Accepted
         — Step 01-05. Amended ADR-0008 (spec_digest is the sole
           per-write content witness) and ADR-0015 (write-acknowledgement
           shape no longer carries commit_index). Updated brief.md
           Phase 1 surface table. Flipped ADR-0020 Status: Proposed
           → Accepted with the implementation digest. ADRs 0013/0014
           confirmed unaffected by audit.
```

## Test surface delta

**Deleted (4 tests):**
- `crates/overdrive-store-local/tests/integration/commit_counter_invariant.rs`
  — the proptest from `fix-commit-counter-and-watch-doc`, asserting
  `commit_index() == effective_committed_op_count`. The invariant has
  no referent post-deletion; the test is preserved in git history for
  the audit trail.
- 3 reconciler/store tests pinning `commit_index` propagation through
  the API surface.

**Revised (14 tests, all assertion-strengthening):**
- Submit job flow: assert `{job_id, spec_digest, outcome: "inserted"}`
  on first write; `{job_id, spec_digest, outcome: "unchanged"}` on
  byte-equal idempotent retry; 409 Conflict (no body field) on
  byte-different retry.
- Describe job: assert `{spec, spec_digest}` shape, no `commit_index`.
- Cluster status: assert exactly four fields
  `{mode, region, reconcilers, broker}` and absence of `commit_index`
  / `writes_since_boot` / any replacement.
- Walking-skeleton acceptance test (`xtask/tests/acceptance/`)
  reshaped to the new wire contract end-to-end.

**Added (1 DST invariant):**
- `intent-store-returns-caller-bytes` in
  `crates/overdrive-sim/src/invariants/evaluators.rs` — exercises
  both `put` and `put_if_absent` paths with empty values, LE-looking
  prefixes, and general payloads; asserts byte-identity:
  `get(k)` returns *exactly* the bytes passed to `put(k)`. This is
  the structural-regression guard that prevents any future encoder
  from re-introducing an inline row-prefix the caller never sees.

## Implementation surface delta

**Deleted from `crates/overdrive-store-local/src/redb_backend.rs`:**
- `commit_counter: AtomicU64` field on `LocalIntentStore`
- `peek_next_inside()` helper (returned `commit_counter + 1` for
  the per-entry index prefix)
- `bump_commit_after_commit()` helper (advanced the counter post-commit)
- `encode_entry` inline `[u8; 8]` index prefix on every value
- All `commit_index()` trait method impls

**Reverted in `crates/overdrive-store-local/src/snapshot.rs`:**
- `LOCAL_STORE_FRAME_V2` (embedded `commit_counter` in the frame)
  → `LOCAL_STORE_FRAME_V1` (no counter; canonical pre-v2 shape)
- v2 encoder, decoder branch, and `pub const VERSION: u16 = 2`
  deleted
- v1 decoder preserved verbatim
- Module docstring rewritten to describe v1 as canonical

**Deleted from `crates/overdrive-core/src/store.rs`:**
- `IntentStore::commit_index() -> u64` trait method
- All implementations across `core`, `store-local`, `sim`, harness fakes

## Architectural records

| ADR | Status | Notes |
|---|---|---|
| **ADR-0020 — Drop commit_index from Phase 1** | **Accepted** (this feature) | Created in DESIGN, flipped to Accepted in step 01-05 with implementation digest |
| ADR-0008 — Per-write content witness | **Amended** | `spec_digest` is now sole content witness on the per-write surface |
| ADR-0015 — Write-acknowledgement shape | **Amended** | Acknowledgement no longer carries `commit_index`; carries `spec_digest` + optional `IdempotencyOutcome` |
| ADR-0010 — `LocalIntentStore` semantics | Audited, no change | `commit_counter` was an implementation detail not codified in ADR-0010 |
| ADR-0013 — Snapshot frame versioning | Audited, no change | Reversion to v1 stays inside the additive-only discipline (single-cut greenfield, no in-flight v2 snapshots exist) |
| ADR-0014 — DST invariant catalogue | Audited, no change | Adding `intent-store-returns-caller-bytes` follows ADR-0014's additive process |

## Lessons

- **"Renaming the same race" is a falsifiable structural test.** When
  three fixes against the same field address visible artifacts of a
  deeper redundancy, the right move is to ask whether the field
  itself is the bug. The structural argument in ADR-0020 — that
  `spec_digest` already carries the operator-visible signal at a
  finer granularity, and that `commit_index` cannot survive Phase 2
  Raft semantics — is the kind of reasoning that the bug-fix lane
  cannot reach. Promotion from bugfix → redesign requires this
  shape of argument.
- **Pure-drop is cheaper than replacement-field for an unshipped
  surface.** The briefing initially proposed replacement fields on
  `ClusterStatus` (`writes_since_boot`, `process_started_at`).
  DESIGN rejected this: the same race that motivated the deletion
  re-applies to any monotonic-counter replacement. `ClusterStatus`
  is pure-drop; `SubmitJobResponse` gets replacement fields
  (`spec_digest`, `outcome`) only because the per-write surface has
  legitimate operator-visible signals that *aren't* counter-shaped.
  The asymmetry is principled, not pragmatic.
- **DST invariants generalise behavioural pins.** The deleted
  `commit_counter_invariant.rs` proptest pinned a specific helper
  pair against a specific field. The new
  `intent-store-returns-caller-bytes` invariant is broader — it
  prevents *any* future encoder from re-introducing a hidden
  per-row prefix, regardless of the field name. This is the right
  shape of guard for a deletion: the invariant is about what the
  *contract* says, not about the implementation that was removed.
- **Test-budget discipline pays dividends in deletion features.**
  G8 budget was 8 tests for 4 behaviours; 6 tests delivered. The
  walking-skeleton acceptance test pinned the new wire shape
  end-to-end, so per-behaviour unit pins were redundant for
  steps 01-02, 01-03, 01-04. RED_UNIT was correctly skipped on
  three of five steps with structural rationale recorded in
  `execution-log.json`.

## Outstanding items

- **Phase 5 (mutation testing) deferred to CI nightly.** Per explicit
  user direction 2026-04-26: the inner-loop background mutation runs
  were killed mid-baseline by the runtime cap (the strategy in
  `CLAUDE.md` is per-feature, but the per-PR diff-scoped run
  exceeded the practical wall-clock budget for this branch). The
  nightly `cargo xtask mutants --workspace` job in the CI pipeline
  is the gate that closes this loop; the per-feature strategy is
  honoured by that pipeline rather than by per-PR execution for
  this feature.
- **Push to remote pending user authorization.** This finalize commit
  is staged locally only; the user issues `git push` themselves per
  the project rule.

## Migrated artifacts

None. `docs/feature/redesign-drop-commit-index/` is preserved in full
as the wave-matrix history per the standard finalize protocol —
ADR-0020 lives at its permanent path
`docs/product/architecture/adr-0020-drop-commit-index.md` (committed
in `ff86291`), the briefing/wave-decisions/upstream-changes are
preserved under `design/` for the audit trail, and the deliver/
artifacts (execution-log, roadmap, Phase 4 review) are committed
into the workspace by this finalize commit. No file moves are
required.

## Supersedes

This note formally closes the `commit_index` saga that began with
`docs/evolution/2026-04-25-fix-commit-index-per-entry.md` and
continued through both
`docs/evolution/2026-04-25-fix-commit-counter-and-watch-doc.md` and
`docs/evolution/2026-04-26-fix-commit-counter-and-watch-doc.md`.
Those three notes document the bug-fix lane's attempts; this note
documents the redesign that closed the underlying race by deletion.
