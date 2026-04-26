# Architect briefing — redesign-drop-commit-index

**Wave**: DESIGN (cross-wave revision — touches DISCUSS and DISTILL)
**Mode**: propose (present 2-3 options with trade-offs; user picks)
**Scope**: design + ADR + revise DISCUSS user stories + revise DISTILL
test-scenarios + walking-skeleton (scope (b)). DELIVER follows in a
separate wave after this design is approved.

## Question

Should `commit_index` exist in Phase 1 single mode at all?

## How this came up

A code review on `crates/overdrive-store-local/src/redb_backend.rs`
flagged a cross-writer race: `peek_next_inside` runs inside the redb
write transaction (where redb's writer serialisation guarantees mutual
exclusion), but `bump_commit_after_commit` runs *after*
`write.commit()` returns — by which time redb has released the writer
lock. A second writer's `begin_write()` returns and calls
`peek_next_inside` before the first writer's bump, so both peek the
same counter value and persist on-disk rows with identical
`commit_index` prefixes.

This is the third bug in `commit_index` in 26 hours. Each prior fix
was correct for the bug it targeted; none stepped back to ask whether
the field was earning its keep:

| Commit | Bug fixed | Bug introduced |
|---|---|---|
| `ba72297` (`fix-commit-index-per-entry`) | Store-wide counter raced against POST response value | Doubled the counter surface (per-entry + store-wide); inline `[u64-LE-prefix \|\| value]` row encoding; snapshot frame v2 |
| `97b0069` (`fix-commit-counter-and-watch-doc`) | Phantom bump on no-op deletes / KeyExists / empty txn | Added 4 conditional gates around the bump call site |
| `f5b361c` (`fix-commit-counter-and-watch-doc-rca`, 26h ago) | Bump leaked on commit failure | **The current race** — moved bump outside redb's writer lock |

The investigation that uncovered this race also revealed:

1. **No reader.** `commit_index` appears in API responses
   (`SubmitJobResponse`, `JobDescription`, `ClusterStatus`). Nothing
   in the codebase (CLI, reconcilers, tests-as-consumers, internal
   subsystems) reads it back as input. The CLI displays it. Ana never
   uses the displayed value as input to a subsequent command.
2. **Redundant with `spec_digest`.** Idempotent re-submission's
   "this is the same logical record" witness is already content-
   addressed via `spec_digest` (SHA-256 of canonical rkyv bytes,
   ADR-0014). `commit_index` is a parallel write-ordered witness that
   adds no information the digest doesn't carry — and is *strictly
   weaker* (resets on restart; not deterministic from the spec; not
   stable across migration).
3. **Phase 1 placeholder, Phase 2 promise unfulfillable.** The
   docstring at `crates/overdrive-store-local/src/redb_backend.rs:138-145`
   commits to Phase 2 RaftStore replacing the accessor with the real
   Raft log index "while keeping the accessor signature stable so
   handlers stay mode-agnostic." Raft log index has stronger
   semantics (durability, strict monotonicity per Raft, log-entry
   identity). Phase 1's in-memory counter cannot simulate them. Any
   client that treats the Phase 1 value as a stable reference is
   already broken across restarts.
4. **The demand traces to acceptance scenarios, not consumers.**
   `commit_index` is in the API surface because the user stories
   demand it, not because anything uses it:
   - `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`
     (walking-skeleton 1.1 / 1.2 / 1.3, US-03, US-04 sections):
     "submit output reports a commit index ≥ 1", "second submit
     reports commit index 17", "commit index reported matches what
     the intent store reports", "each submit response's
     `commit_index` is strictly greater than the previous one".
   - `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
     (US-03 / US-04 acceptance, US-04 deliverable):
     "`LocalStore::commit_index()` accessor exists and is strictly
     monotonic".
5. **Etcd-shaped reflex.** "Every store has a commit log index"
   instinct from etcd / Postgres LSN / MySQL binlog position. Familiar
   to operators. Adopted as a Phase 1 acceptance requirement without
   asking *what does Ana do with it in single mode?*

## Surgery surface (informs trade-off analysis)

If `commit_index` is dropped from Phase 1, the affected files are:

| Layer | Files |
|---|---|
| Trait | `crates/overdrive-core/src/traits/intent_store.rs` (return type of `get`, `PutOutcome::{Inserted, KeyExists}::commit_index`, docstrings) |
| Single-mode impl | `crates/overdrive-store-local/src/redb_backend.rs` (delete `commit_counter` field, delete `peek_next_inside` / `bump_commit_after_commit`, delete inline row encoding, simplify all four write paths and `bootstrap_from`) |
| Snapshot frame | `crates/overdrive-store-local/src/snapshot_frame.rs` (v2 → v1; v1 already has zero-prefix forward-compat per existing module docstring) |
| Sim adapter | `crates/overdrive-sim/src/adapters/intent_store.rs` (mirror trait shape) |
| Handlers | `crates/overdrive-control-plane/src/handlers.rs` (drop the field from `SubmitJobResponse`, `JobDescription`; rework `ClusterStatus` accessor) |
| API surface | `api/openapi.yaml` (drop `commit_index` from `SubmitJobResponse`, `JobDescription`, `ClusterStatus` schemas; revise descriptions) |
| API types | `crates/overdrive-control-plane/src/api.rs` (drop the field from response structs) |
| CLI render | `crates/overdrive-cli/src/render.rs`, `commands/job.rs`, `commands/cluster.rs`, `commands/alloc.rs` (drop the column / field from output) |
| Tests | `tests/integration/commit_counter_invariant.rs` (delete entirely), `tests/acceptance/per_entry_commit_index.rs` (delete entirely), `tests/acceptance/commit_index_monotonic.rs` (delete entirely), `tests/acceptance/phantom_writes.rs` (revise — keep the no-emit assertions, drop the no-bump assertions), `tests/acceptance/snapshot_roundtrip.rs` (revise frame v2 → v1), `tests/acceptance/put_if_absent.rs` (drop commit_index assertions, keep KeyExists/Inserted variant assertions), `tests/acceptance/local_store_basic_ops.rs` (drop the tuple unpack), `tests/acceptance/local_store_edges.rs` (drop the tuple unpack), `tests/integration/snapshot_proptest.rs` (drop the per-entry index column), `tests/integration/per_entry_commit_index.rs` (delete entirely), `tests/integration/concurrent_submit_toctou.rs` (verify no commit_index assertion remains), `tests/integration/idempotent_resubmit.rs` (drop the index-equality assertion; keep the digest-equality assertion), `tests/integration/submit_round_trip.rs` (drop the index assertion; keep the digest assertion), `tests/integration/describe_round_trip.rs` (same), `tests/acceptance/submit_job_idempotency.rs` (same), `tests/integration/walking_skeleton.rs` (revise per the new walking-skeleton.md), CLI integration tests (drop commit-index assertions in CLI output captures) |
| ADRs | ADR-0013 (reconciler-primitive-runtime — references `commit_index`), ADR-0014 (CLI-HTTP-client-and-shared-types — references `commit_index`), ADR-0015 (HTTP-error-mapping — references `commit_index` heavily, esp. §4 idempotency contract) — all need amendment |
| Brief | `docs/product/architecture/brief.md` (1 reference) |
| User stories | `docs/feature/phase-1-control-plane-core/discuss/user-stories.md` (US-03 / US-04 revisions) |
| Test scenarios | `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md` (walking-skeleton 1.1 / 1.2 / 1.3 + US-03 / US-04 section revisions) |
| Walking skeleton | `docs/feature/phase-1-control-plane-core/distill/walking-skeleton.md` |
| DISTILL wave-decisions | `docs/feature/phase-1-control-plane-core/distill/wave-decisions.md` |
| Existing roadmaps | `docs/feature/phase-1-control-plane-core/deliver/roadmap.json` (frozen — feature already shipped; reference-only), `docs/feature/fix-commit-index-per-entry/deliver/roadmap.json` (the feature being undone) |
| Feature evolutions | `docs/evolution/2026-04-25-fix-commit-index-per-entry.md`, `docs/evolution/2026-04-25-fix-commit-counter-and-watch-doc.md`, `docs/evolution/2026-04-26-fix-commit-counter-and-watch-doc.md` (reference-only; document the cascade) |

The DELIVER wave for this redesign will be large but mechanical
(delete a vestigial field across the surface). It is its own PR
after this design is settled.

## What the architect should produce (scope (b))

Routed through this architect run:

1. **DESIGN deliverable** at
   `docs/feature/redesign-drop-commit-index/design/wave-decisions.md`:
   options analysis, recommendation, blast radius, migration story
   (none — Phase 1 hasn't shipped externally), rollback story
   (revert is one PR).
2. **ADR** at `docs/product/architecture/adr-NNNN-drop-commit-index-phase-1.md`:
   the structural argument (no consumer; redundant with `spec_digest`;
   Phase 2 RaftStore introduces a real Raft log index field with
   proper semantics, not a Phase 1 placeholder; bug cascade as
   evidence of architectural drift). Status: Proposed → Accepted on
   user approval. Supersedes / amends ADR-0013, ADR-0014, ADR-0015.
3. **DISCUSS amendment** at
   `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`:
   revise US-03 acceptance criteria, US-04 acceptance criteria and
   deliverables. Replace `commit_index` references with `spec_digest`
   where a content-addressed witness is wanted; remove where no
   witness is wanted. Use an explicit `> **Amendment 2026-MM-DD**`
   block (mirrors the existing
   `> **Amendment 2026-04-26.**` precedent for the cluster-init
   removal at `test-scenarios.md:8-17`).
4. **DISTILL amendment** at
   `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`
   and `walking-skeleton.md`: revise scenarios 1.1 / 1.2 / 1.3 and
   US-03 / US-04 sections. Same `> **Amendment**` block style.
5. **Open question for DELIVER** —
   `docs/feature/redesign-drop-commit-index/design/upstream-changes.md`
   listing every file in the surgery-surface table above with the
   specific change shape so the DELIVER crafter has a complete
   target.

## Options to present (architect must analyse and recommend)

The architect should evaluate each on: idempotency contract integrity,
walking-skeleton observability ("did the write commit?"), Phase 2
forward-compat, surgery cost, future ADR coherence, and bug-class
elimination.

### Option A — Drop entirely from Phase 1

Delete `commit_index` from trait, API, row encoding, snapshot frame,
handlers, CLI, tests. `spec_digest` is the sole content-addressed
write witness. `ClusterStatus` exposes write-count-since-boot only
if an operator actually needs it (architect should check; my prior
read is they don't).

Phase 2 RaftStore introduces `log_index` as a *new* field in HA-only
responses, with proper Raft semantics (durable, strictly monotonic,
log-entry identity). Single-mode responses simply don't include it.
Handler code branches on mode for that field — which is fine; HA
already has other fields single mode doesn't (peer status, leader
identity).

### Option B — Keep per-entry only; drop store-wide

Per-entry `commit_index` survives in `SubmitJobResponse` and
`JobDescription` for the idempotency contract; `ClusterStatus.commit_index`
goes. Implementation: redb meta-table counter, durable across
restart. Closes the race; closes the restart-reset gap; preserves
the per-entry idempotency contract; deletes one of the two
counters.

This is the smaller-scope answer. Justification depends on whether
the per-entry idempotency contract is genuinely load-bearing
(architect to evaluate against the actual idempotency-test suite —
do tests assert *index equality* across re-submits, or do they
assert *digest equality*? If the latter, the per-entry index is also
unused).

### Option C — Keep both; fix race only with redb meta-table counter

The minimal fix from the original bug-fix scope. Doesn't address
the no-consumer problem; doesn't address the redundancy with
`spec_digest`. Architect should evaluate whether the option carries
its own weight or is just a "do nothing structural" placeholder.

## Constraints and discipline

- **Phase 1 hasn't shipped externally.** No backwards-compat
  obligation. Single-cut migration per the project's
  `feedback_single_cut_greenfield_migrations` convention — delete
  old paths, no deprecations, no grace periods, no feature-flagged
  coexistence.
- **The architect agent owns DESIGN artifacts** per project memory
  `feedback_delegate_to_architect`. Do not ask the user to inline-
  edit ADRs or wave-decisions.
- **No DELIVER work in this run.** Surface the surgery in
  `upstream-changes.md`; the DELIVER crafter consumes it next.
- **Snapshot frame v1 forward-compat already exists** per
  `crates/overdrive-store-local/src/snapshot_frame.rs` module docstring
  ("v1 frames with zero-padded indices") — Option A's frame
  simplification reverts to v1 cleanly without a new compat layer.
- **No external clients.** `api/openapi.yaml` is the contract; no
  third-party SDKs, no published spec. Schema changes are free.
- **The `commit_index_per_entry` evolution doc** (`docs/evolution/2026-04-25-fix-commit-index-per-entry.md`)
  is reference-only history. Do not amend; the new ADR supersedes
  the architectural decision the evolution captured.

## Project conventions for the architect

- Read `docs/whitepaper.md` (already loaded — referenced in CLAUDE.md).
- Read `docs/product/architecture/brief.md` (architecture SSOT).
- Read `docs/product/commercial.md` (tenancy/tiers SSOT) only if
  commercially relevant — likely irrelevant here.
- Skill loading: `~/.claude/skills/nw-design/SKILL.md` per `nw-design`
  command.

## Success criteria

- [ ] `wave-decisions.md` analyses three options against six criteria
  (idempotency contract, walking-skeleton observability, Phase 2
  forward-compat, surgery cost, ADR coherence, bug-class
  elimination)
- [ ] One option recommended with explicit rationale; alternatives
  documented for traceability
- [ ] ADR drafted (status Proposed) with structural argument, not
  just "we found a bug"
- [ ] User-stories.md amended with `> **Amendment**` block — US-03
  and US-04 revised
- [ ] Test-scenarios.md amended with `> **Amendment**` block —
  walking-skeleton 1.1 / 1.2 / 1.3 and US-03 / US-04 sections revised
- [ ] Walking-skeleton.md amended with `> **Amendment**` block
- [ ] `upstream-changes.md` enumerates every file in the surgery
  surface with specific change shape (not just "modify file X" —
  "in file X, line Y, change Z to W")
- [ ] User reviews and approves before any code changes; DELIVER is
  a separate wave invoked separately
