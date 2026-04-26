# Wave Decisions — redesign-drop-commit-index (DESIGN)

**Wave**: DESIGN
**Mode**: propose (architect analyses; user approves)
**Date**: 2026-04-26
**Architect**: Morgan
**Scope**: design + ADR + revise DISCUSS user-stories + revise DISTILL test-scenarios + walking-skeleton (scope (b) — DELIVER follows separately)

## Question

Should `commit_index` exist in Phase 1 single mode at all?

## Recommendation

**Option A — Pure-drop. No replacement field on `ClusterStatus`.**

The field is structurally redundant with `spec_digest` on the per-write
surface, has no consumer in the codebase, cannot meet its Phase 2
forward-compat promise, and has generated three production bugs in
26 hours. Phase 1 is unshipped; greenfield discipline applies. The
DELIVER cost is real but mechanical; the right time to delete is now,
not after a fourth bug.

The recommendation deviates from the briefing's framing in one specific
way: where the briefing proposed dropping the field outright, Option A
as recommended *also adds two replacement fields to `SubmitJobResponse`*
to preserve the operator-visible signals that `commit_index` was masking
on the per-write surface:

- `SubmitJobResponse` gains `spec_digest` (replaces `commit_index` as the
  per-write content witness — operators verify locally) AND
  `outcome: IdempotencyOutcome ∈ {"inserted", "unchanged"}` (replaces
  "did the index advance?" as the operator-visible idempotency signal).

`ClusterStatus` is **pure-drop**. Four fields remain:
`{mode, region, reconcilers, broker}`. No `writes_since_boot`. No
`process_started_at`. The "renaming the same race" argument forces this
choice — see the next subsection.

### Why `ClusterStatus` is pure-drop

A `writes_since_boot` replacement was considered in the architect's
first-pass draft and is rejected on the structural argument the user
flagged on review:

> A `writes_since_boot` field carries the **same race conditions and
> restart-reset gap as today's `commit_index`, just renamed**. The
> in-memory counter that resets on every process boot is precisely the
> failure mode the field name `commit_index` was hiding. Renaming the
> field to `writes_since_boot` exposes the reset semantics in the name
> but does not eliminate the bug class — the next contributor adding a
> conditional gate around the bump call site, the next snapshot/restore
> interaction, the next cross-writer race in the bump-after-commit
> window, all reappear under the new name. We would relitigate the
> same three bugs in three months under a different field name.

The walking-skeleton US-04 wiring witness is already covered by
`mode + region + reconcilers + broker` — the broker's `dispatched`
counter advancing past zero proves the reconciler primitive ran, and
the `reconcilers` list proves it is registered. There is no operator
question that *only* a writes-since-boot counter answers; if real
activity-rate signals are ever needed, Phase 5's metrics endpoint
(Prometheus-shaped) brings them with proper semantics — durable across
restart, named honestly, scrapeable, with rate functions in the query
language rather than in the field name.

`ClusterStatus.writes_since_boot` is therefore not "the smaller-blast-
radius answer" the architect's first draft framed it as. It is a
restatement of the same unprobed-faith claim that produced the bug
cascade, dressed in honest field naming. Pure-drop is the only option
that eliminates the surface that generates the bug class, on this
response shape as well as on the per-write surface.

Final shapes:

- `SubmitJobResponse`: `{job_id, spec_digest, outcome: IdempotencyOutcome}` —
  three fields; `commit_index` gone; two replacements added.
- `JobDescription`: `{spec, spec_digest}` — two fields; `commit_index`
  gone; no replacements added.
- `ClusterStatus`: `{mode, region, reconcilers, broker}` — four fields;
  `commit_index` gone; no replacements added.

## Options analysis

### Six criteria

| Criterion | What it asks |
|---|---|
| **Idempotency contract integrity** | Does the option preserve "byte-identical re-submit returns the same logical answer; different spec at same key returns 409"? |
| **Walking-skeleton observability** | Can Ana confirm "did the write commit?" from the API response alone? |
| **Phase 2 forward-compat** | When `RaftStore` lands, does the wire contract evolve cleanly without re-purposing Phase 1 placeholders? |
| **Surgery cost** | How many files / ADRs / docs change? How mechanical is the DELIVER lane? |
| **ADR coherence** | After the option lands, do the existing ADRs (0008, 0013, 0014, 0015) read consistently with the new field set? |
| **Bug-class elimination** | Does the option remove the *class* of bug (cross-writer race, phantom bumps, conditional-gate sprawl, restart-reset surprise), or shift it under a different name? |

### Option A — Pure-drop (recommended)

**Shape.** Delete `commit_index` from the trait, the impl, the snapshot
frame, the API responses, the CLI rendering, and the test surface that
asserted on it. On the per-write surface (`SubmitJobResponse`) replace
operator-visible signals with `spec_digest` (content witness) and
`IdempotencyOutcome` (typed enum). On `JobDescription` and
`ClusterStatus`, no replacement field is added — the surface narrows.

| Criterion | Score | Reasoning |
|---|---|---|
| Idempotency contract integrity | **Strong** | `IdempotencyOutcome` makes the "did the handler take the idempotency branch?" signal explicit; the back-door byte-equality test (already present) covers "the stored bytes did not change." 409 Conflict on different-spec-at-same-key is unaffected. |
| Walking-skeleton observability | **Strong** | `spec_digest` lets Ana verify the spec landed correctly *and* that her local hash matches the server's — a stronger signal than "the index is ≥ 1." `outcome: "inserted" or "unchanged"` is more direct than inferring idempotency from index equality. WS-04's wiring witness ("did the reconciler primitive run?") is preserved by `broker.dispatched > 0`. |
| Phase 2 forward-compat | **Strong** | `RaftStore` introduces a NEW field `log_index` with proper Raft semantics, on HA-only response shapes. Single-mode responses do not carry the field. Handler code branches on mode (already does so for other HA-only fields). No re-purposing of a Phase 1 placeholder, and no Phase 1 field whose name implies a counter that isn't there. |
| Surgery cost | **High** | Touches the trait, the impl, snapshot frame, sim adapter, every handler, API types, CLI rendering, ~12 test files, ADR-0008 + ADR-0015 + brief.md. Mechanical but extensive. Single-cut PR per project convention. |
| ADR coherence | **Strong** | ADR-0015 §4 idempotency row is amended (`commit_index` → `spec_digest` + `outcome`); ADR-0008 endpoint table is amended (responses listed without `commit_index`; `ClusterStatus` listed with four fields). ADR-0013 / ADR-0014 unaffected (verified — neither references `commit_index`). The new ADR-0020 carries the structural rationale; downstream ADRs reference it. |
| Bug-class elimination | **Strong** | The cross-writer race is unreachable (no counter to bump). The phantom-bump conditional gates are unreachable (no bump call site). The inline `[u64-LE-prefix \|\| value]` row encoding is gone (rows are caller bytes verbatim). The v2 snapshot frame is gone. The restart-reset surprise is gone (no field whose name implies a durable counter that resets). The bug class does not recur because the surface that generated it does not exist. |

### Option B — Keep per-entry only; drop store-wide

**Shape.** Per-entry `commit_index` survives in `SubmitJobResponse` and
`JobDescription` (idempotency contract). `ClusterStatus.commit_index`
goes. Implementation: redb meta-table counter, durable across restart,
incremented in the same write transaction as the entry insert.

| Criterion | Score | Reasoning |
|---|---|---|
| Idempotency contract integrity | **Strong** | Per-entry index survives; the existing `triple_resubmit_byte_identical_all_return_same_commit_index` assertion shape is preserved as-is. |
| Walking-skeleton observability | **Adequate** | Operators still see a numeric witness on submit; whether they *use* it is unchanged from today (they don't). |
| Phase 2 forward-compat | **Weak** | The per-entry index promise stays alive: "Phase 2's `RaftStore` will replace this with the real Raft log index, signature stable so handlers stay mode-agnostic." This is the same unfulfillable promise the existing docstring carries. The Phase 1 redb-meta-table counter cannot acquire log-entry identity, cluster-wide consensus on order, or replication semantics. The placeholder remains a placeholder — Phase 2 will either invent a different field name (rendering the Phase 1 surface vestigial) or stretch the existing field's semantics (silently breaking external consumers who relied on the Phase 1 shape). The choice is just deferred. |
| Surgery cost | **Medium** | Still touches the trait, the impl (redb meta-table is non-trivial), the snapshot frame (v2 stays), the sim adapter, half the handlers, half the API types. Net surface delta vs Option A is ~40% smaller; conceptual cost (two flavours of "commit_index" surviving in different shapes) is higher. |
| ADR coherence | **Weak** | ADR-0008 / ADR-0015 keep most of their `commit_index` references; the per-entry semantics remain documented; the per-store-wide semantics get retired. The "two `commit_index` flavours" framing was always confusing — the briefing flagged this in framing the original RCA. Option B preserves the confusion. |
| Bug-class elimination | **Partial** | Closes the cross-writer race (the redb meta-table counter is bumped inside the writer lock). Closes the restart-reset gap (durable in redb). Does NOT close the no-consumer / redundant-with-`spec_digest` / Phase-2-promise-unfulfillable structural problems. The next bug in the family ships when the next contributor wonders why the counter sometimes doesn't advance (snapshot/restore semantics, bootstrap-from-existing-data, transactional rollback interaction). |

**Why rejected.** Option B addresses one structural problem (the race)
and ducks the other three. The smaller-scope appeal is illusory: the
trait surface still changes, the snapshot frame still has v2 (with the
per-entry index payload), the handlers still change for `ClusterStatus`,
the conceptual cost (two `commit_index` flavours surviving with different
semantics) is higher than Option A's clean break.

The principled question Option B doesn't answer: *what does the
per-entry index actually witness?* The idempotency tests assert
`commit_index` equality across re-submits — but the property they're
defending is "the handler took the idempotency branch and did not
write," which is a *behavioural* property of handler logic, not an
*operator-visible* property of content identity. The behavioural
property is preserved by `IdempotencyOutcome::Unchanged` (Option A);
content identity is preserved by `spec_digest` (Option A). The
per-entry index adds nothing those two together don't.

### Option C — Keep both; fix race only with redb meta-table counter

**Shape.** Replace the in-memory `AtomicU64` with a redb meta-table
counter incremented inside the write transaction. Both `commit_index`
flavours survive on the wire.

| Criterion | Score | Reasoning |
|---|---|---|
| Idempotency contract integrity | **Strong** | Unchanged from today. |
| Walking-skeleton observability | **Adequate** | Unchanged from today. |
| Phase 2 forward-compat | **Weak** | Same diagnosis as Option B — the placeholder remains a placeholder. |
| Surgery cost | **Low** | Smallest change set. The DELIVER lane is "replace the `AtomicU64` with a redb sequence and bump it inside the writer lock." |
| ADR coherence | **Adequate** | Existing ADRs unchanged. |
| Bug-class elimination | **Weak** | Closes the immediate race; preserves the conditional-gate-sprawl pattern (the next bug ships when the next contributor adds the fifth conditional gate). The no-consumer / redundant-with-`spec_digest` / Phase-2-placeholder structural problems are entirely untouched. The restart-reset surprise survives if `ClusterStatus.commit_index` is preserved; eliminating only the per-store-wide value while keeping per-entry collapses to Option B. |

**Why rejected.** Option C is the disposition that produced the bug
cascade. Three fixes in 26 hours, each correct for its target failure
mode and each leaving the field's existence intact, are the empirical
evidence that "fix the immediate bug; defer the structural question"
does not actually defer the structural question — it just guarantees
the next bug ships. Option C is the choice if Phase 1 had shipped to
external consumers; it is the wrong choice when the cost of Option A
is one mechanical PR.

### Option D — Pure-drop on per-write surface, `writes_since_boot` replacement on `ClusterStatus`

**Shape.** The architect's first-pass draft of this ADR.
`commit_index` drops from `SubmitJobResponse` and `JobDescription`
(with `spec_digest` + `outcome` replacing it on submit, exactly as in
Option A). On `ClusterStatus`, `commit_index` is renamed
`writes_since_boot` and paired with a `process_started_at` field —
the in-memory counter survives, but its reset-on-restart semantics
are now communicated honestly in the field name.

| Criterion | Score | Reasoning |
|---|---|---|
| Idempotency contract integrity | **Strong** | Identical to Option A on the per-write surface — `IdempotencyOutcome` makes the idempotency-branch signal explicit; `spec_digest` is the content witness. The `ClusterStatus` field rename does not interact with the idempotency contract. |
| Walking-skeleton observability | **Adequate** | Per-write surface is the same as Option A. On the status surface, `writes_since_boot` answers "is this control plane processing writes?" more directly than `broker.dispatched`. But the walking-skeleton US-04 wiring witness is already covered by `mode + region + reconcilers + broker` (broker.dispatched > 0 proves the reconciler ran), and Phase 1 has no walking-skeleton step that asks the write-rate question. The marginal observability gain over Option A is not load-bearing. |
| Phase 2 forward-compat | **Adequate** | Per-write surface is the same as Option A — `RaftStore` introduces `log_index` as a new HA-only field. `writes_since_boot` on `ClusterStatus` is honest about being a single-mode-only counter (the Raft log index is the right answer for HA), so the rename does not produce a Phase-2 placeholder problem. But it does produce a different problem: `writes_since_boot` survives into Phase 2 as a single-mode-only field that HA mode does not populate, which is the same "branch on mode" surface area Option A introduces — without the bug-class elimination payoff. |
| Surgery cost | **High** | Comparable to Option A — touches the same trait, impl, snapshot frame, sim adapter, handlers, API types, CLI rendering, ~12 test files. The `ClusterStatus` rename is one more field touch (`commit_index → writes_since_boot`, plus `process_started_at` introduced fresh) but does not change the order-of-magnitude cost. |
| ADR coherence | **Adequate** | ADR-0008 endpoint table is amended (`ClusterStatus` lists `writes_since_boot, process_started_at` instead of `commit_index`); ADR-0015 §4 idempotency row is amended identically to Option A. Coherent on paper; the criticism is below at *bug-class elimination* — the rename communicates the gotcha but does not eliminate the bug class, so the coherence is for an architecture that still ships the bug surface. |
| Bug-class elimination | **Weak** | The renaming is the load-bearing failure of Option D. The in-memory counter still exists. The bump call site still exists. The cross-writer race window (peek-then-bump in redb's writer-lock release gap) still exists. The conditional-gate-sprawl pattern that produced the third commit in the cascade still exists — the next contributor finding a phantom increment can still add a fourth gate around the same bump. The restart-reset surprise is "in the field name" but is still a real semantic, present every time a process restart truncates what looked like a monotonic counter. The next bug in the family — say, a snapshot/restore interaction that causes the counter to skip values, or a transactional rollback that double-bumps — would ship under the new name with the same code shape. **Renaming the field communicates the gotcha to the operator; it does not remove the surface that generates the class.** |

**Why rejected.** The `writes_since_boot` rename closes zero of the
three structural bug surfaces (race, conditional gates, restart reset)
that produced the cascade. The in-memory counter, the bump call site,
the cross-writer race window, and the conditional-gate-sprawl pattern
all survive verbatim under the new name. Renaming is not probing
— principle 12 of the methodology applies directly here, and the
bug class would re-surface in three months under a different field
name. Option A removes the surface that generates the class on this
response shape; Option D preserves it.

The criterion that distinguishes Option D from Option A is
*bug-class elimination*, and Option A scores Strong where Option D
scores Weak. Every other criterion ties or favours Option A by a
margin that does not justify the structural cost of preserving the
bug surface. See *Why `ClusterStatus` is pure-drop* above for the
full structural argument; ADR-0020 §Considered alternatives §D
restates it inline so the ADR is self-contained.

## Recommendation rationale

Option A wins on five of six criteria (the only criterion it loses on
is *surgery cost*, where Option C is smaller). The single-cut migration
discipline (project memory `feedback_single_cut_greenfield_migrations`)
makes surgery cost a one-time concern: the DELIVER PR is mechanical, the
target list is enumerated in `upstream-changes.md`, no external consumer
is broken because there is no external consumer.

The bug-class-elimination criterion is the load-bearing one, and it is
the one a `writes_since_boot` replacement on `ClusterStatus` would have
quietly compromised: three production bugs in 26 hours is not "an
unlucky run." It is empirical evidence that the surface generates bugs
faster than reviewers can catch them. Option B closes one of the three
failure modes; Option C closes one. **Renaming the field on
`ClusterStatus` to `writes_since_boot` would close zero** — the
in-memory counter, the bump call site, the cross-writer race window,
the restart-reset surprise, all survive verbatim under the new name.
Only pure-drop removes the surface that generates the class.

The "Earned Trust" framing (methodology principle 12, surfaced explicitly
in CLAUDE.md) lands here as the structural argument: every dependency
you do not probe is an act of faith. `commit_index` was the unprobed
faith claim — that some future consumer would need it, that the Phase 1
counter would resemble the Phase 2 Raft index closely enough for the
signature to be stable, that the monotonicity property the user stories
demanded was a property the operator would notice if it broke. Three
production bugs are the falsification. Renaming is not probing.

## Where the briefing was wrong (deviations)

The briefing was structurally correct in its diagnosis (no consumer,
redundant with `spec_digest`, Phase 2 promise unfulfillable, bug cascade
as evidence). Two specific framing errors were corrected during the
audit:

1. **ADR-0013 and ADR-0014 do NOT reference `commit_index`.** The
   briefing's surgery-surface table claims they do; verified by grep —
   neither does. Only ADR-0015 and `brief.md` carry the references.
   The new ADR-0020 amends ADR-0015 §4 and ADR-0008 endpoint table; it
   does not amend ADR-0013 or ADR-0014. This narrows the ADR-amendment
   surface and removes a fictitious dependency.

2. **`ClusterStatus` replacement: rejected on the architect's first
   draft.** The first draft proposed `writes_since_boot +
   process_started_at` as honest replacements for the dropped
   `commit_index`. On user review, the proposed replacement was
   recognised as the same in-memory counter, the same restart-reset
   gap, and the same bump-call-site bug surface — renamed honestly,
   yes, but not eliminated. The bug class survives. Pure-drop is the
   structurally consistent choice; the `SubmitJobResponse` replacements
   (`spec_digest`, `outcome`) are kept because they answer different
   operator questions (per-write content witness, idempotency-branch-
   taken signal) that are not race-prone or restart-resetting in their
   own right.

## Blast radius

| Layer | Surface |
|---|---|
| Trait | `IntentStore::get` return type; `PutOutcome` variants |
| Single-mode impl | `LocalIntentStore` — drop `commit_counter`, drop `peek_next_inside` / `bump_commit_after_commit`, drop inline frame, drop public `commit_index()` accessor, simplify all four write paths and `bootstrap_from` |
| Snapshot frame | v2 → v1 (forward-compat already exists; the v2 encoder/decoder is deleted) |
| Sim adapter | `SimIntentStore` mirrors trait shape |
| Handlers | `submit_job`, `describe_job`, `cluster_status` |
| API types | `SubmitJobRequest` unchanged; `SubmitJobResponse` field set changes (drop `commit_index`; add `spec_digest`, `outcome`); `JobDescription` field set changes (drop `commit_index`); `ClusterStatus` field set changes (drop `commit_index`, add nothing); new `IdempotencyOutcome` enum |
| OpenAPI | `api/openapi.yaml` regenerated; component schemas changed for three responses + one new enum |
| CLI render | `commands/job.rs` (submit + describe), `commands/cluster.rs`, `commands/alloc.rs`, `render.rs` |
| Tests | Delete `commit_counter_invariant.rs`, `per_entry_commit_index.rs` (×2), `commit_index_monotonic.rs`. Revise `phantom_writes.rs` (preserve no-emit assertions, drop no-bump assertions), `snapshot_roundtrip.rs` (frame v2 → v1 expectations), `put_if_absent.rs` (drop index assertions, keep variant assertions), `local_store_basic_ops.rs` (drop tuple unpack), `local_store_edges.rs` (drop tuple unpack), `snapshot_proptest.rs` (drop per-entry index column), `concurrent_submit_toctou.rs` (verify no `commit_index` assertion remains), `idempotent_resubmit.rs` (replace index-equality with `outcome` + `spec_digest` assertions), `submit_round_trip.rs`, `describe_round_trip.rs`, `submit_job_idempotency.rs`, `walking_skeleton.rs` (×2 — CLI + control-plane), CLI render tests. |
| ADRs | New ADR-0020 (Proposed; Accepted on DELIVER landing); ADR-0015 §4 amended; ADR-0008 endpoint table amended. ADR-0013 / ADR-0014 unaffected (verified). |
| Briefs | `brief.md` (3 references) |
| User stories | `discuss/user-stories.md` US-03 + US-04 (Amendment block) |
| Test scenarios | `distill/test-scenarios.md` walking-skeleton 1.1 / 1.2 / 1.3 + §4 + §5 + §6 (Amendment block) |
| Walking skeleton | `distill/walking-skeleton.md` (Amendment block) |

The full enumeration with file paths, line ranges, and specific change
shape is in
`docs/feature/redesign-drop-commit-index/design/upstream-changes.md`.

## Migration story

**None.** Phase 1 has not shipped externally. No backwards-compat
obligation. Single-cut migration discipline per project memory
`feedback_single_cut_greenfield_migrations`. Delete the old paths,
land the new ones, no deprecations / grace periods / feature flags.

The redb on-disk format does change (rows are no longer prefixed with
the inline `[u64-LE-prefix || value]` frame); a redb file written by
the current code is not readable by the post-ADR code. This is
deliberately accepted — Phase 1 development databases are scratch,
no production data exists to migrate.

The OpenAPI schema bytes change. The `cargo xtask openapi-check` gate
will fail on the first DELIVER step that lands the trait change without
the matching schema regeneration; this is the desired behaviour
(structural drift is caught at the gate). The DELIVER lane regenerates
the schema as part of the same PR.

## Rollback story

One PR revert. The DELIVER PR for this redesign is a single mechanical
change set; reverting it restores the field across the surface.

Rollback after the DELIVER PR has merged would re-introduce the
unsolved bug class (cross-writer race, phantom bumps, conditional-gate
sprawl, restart-reset surprise). If a downstream subsystem turns out
to need a numeric witness the field was providing — the architect's
audit says no such subsystem exists, but if it surfaces — the right
answer is *not* to revive `commit_index`; it is to introduce the new
field with the shape the actual consumer needs (which the audit will
then have specified). Rollback is therefore not a forward-compat
hedge; it is the literal "undo this PR" tool.

## Open questions

None. The four user decisions for this run have been made and are
recorded in this document and in ADR-0020:

1. Pure-drop, no `ClusterStatus` replacement.
2. `ClusterStatus` is four fields: `{mode, region, reconcilers, broker}`.
3. Prior-run drafts (this file and ADR-0020) revised, not preserved.
4. ADR amends narrowed to ADR-0008 + ADR-0015.

## Success criteria — checklist

Each `[x]` below is verified against an artifact present on disk at
the time this checklist was revised (2026-04-26 review-resolution
pass). Items pending DELIVER are honestly `[ ]` regardless of how
near to ready they appear; the checkbox flips when the artifact
actually lands, not when the design says it should.

- [x] Four options analysed against six criteria (this document — A,
      B, C, D in the *Options analysis* section).
- [x] One option recommended with explicit rationale (Option A pure-drop;
      *Recommendation rationale* section, with the "renaming the same race"
      argument made load-bearing).
- [x] Alternatives documented with rejection rationale (Options B, C, D
      above; ADR-0020 §Considered alternatives also lists A/B/C/D/E for
      symmetry — the ADR adds Option E "defer" which is implicit here).
- [x] ADR drafted (Proposed) with structural argument, not just "we found
      a bug" (`docs/product/architecture/adr-0020-drop-commit-index-phase-1.md` exists on disk).
- [x] User-stories.md amendment authored (Amendment 2026-04-26 block
      present in `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`).
- [x] Test-scenarios.md amendment authored (Amendment 2026-04-26 block
      present in `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`).
- [x] Walking-skeleton.md amendment authored (Amendment 2026-04-26 block
      present in `docs/feature/phase-1-control-plane-core/distill/walking-skeleton.md`).
- [x] `upstream-changes.md` enumerates every file in the surgery surface
      with specific change shape (file present on disk; per-file enumeration
      verified against live grep audit on 2026-04-26).
- [ ] User has reviewed all DESIGN-wave artifacts produced under this
      decision and approved before invoking `/nw-deliver`. (Pending — this
      review-resolution pass closes the reviewer-flagged gaps; user
      approval flips this checkbox.)
- [ ] DELIVER PR opened against `main`. (Pending — separate wave.)
- [ ] DELIVER wave has landed the change-set across the trait, impl,
      snapshot frame, handlers, API, CLI, tests, and ADRs. (Pending —
      separate wave.)
- [ ] ADR-0020 status flipped from Proposed to Accepted on DELIVER
      landing. (Pending — separate wave.)

## Changelog

| Date | Change |
|---|---|
| 2026-04-26 | Initial DESIGN-wave decisions for the redesign. Architect: Morgan. First draft recommended pure-drop on per-write surface + `writes_since_boot` replacement on `ClusterStatus`. |
| 2026-04-26 | Revision per **explicit user decision on review (this conversation, 2026-04-26)**: `ClusterStatus` is pure-drop with no replacement field. The architect's first draft proposed `writes_since_boot + process_started_at`; user response, paraphrased: *"that's renaming the same race"* — the proposed `writes_since_boot` field keeps the in-memory counter, the bump call site, the cross-writer race window, and the conditional-gate surface intact, with only the field name changing. Renaming is not probing (methodology principle 12). ADR-0020 revised to match; *Considered alternatives* §D documents the rejection inline. Success-criteria checklist revised to honest `[ ]` / `[x]` (the prior run's `[x]` marks across the board over-claimed completion of items not actually authored by that run). |
| 2026-04-26 | Review-resolution pass per three independent peer reviews (`wave-decisions.md`, `adr-0020-drop-commit-index-phase-1.md`, `upstream-changes.md`). Findings appended to each artifact's `## Review (2026-04-26)` section with `### Resolution (2026-04-26)` subsections enumerating which review issues were resolved and which were deferred. This file changes: (1) Alternative D added to the *Options analysis* section as a fourth row evaluated against all six criteria, with bug-class-elimination scored Weak inline; (2) Changelog entry above gains explicit user-decision date and quote attribution; (3) Success-criteria checklist revised to verify each `[x]` against artifacts present on disk (DELIVER-pending items honestly `[ ]`). |

## Review (2026-04-26)

Independent peer review of `wave-decisions.md`. Verdict:
**NEEDS_REVISION**. Three blocking issues + one high-severity issue
+ two suggestions raised, all on documentation specificity rather
than architectural correctness. The reviewer found no architectural
problems with Option A pure-drop; the gaps are completeness and
audit-trail precision.

### Findings

**Blocking B1 — Six-criteria table omits Alternative D.**
The *Options analysis* section evaluates A, B, C against the six
criteria but Alternative D (`writes_since_boot` replacement) was
discussed prose-only in a separate "Why `ClusterStatus` is pure-drop"
subsection. The architect's first draft proposed Option D and the
user's review rejected it; the rejection rationale therefore needs
to live in the same options-analysis structure as the other rejected
alternatives. **Fix recommendation**: add a fourth row to the
options-analysis table evaluating Alternative D against all six
criteria. Score every criterion `Weak` or `Adequate`, with bug-class-
elimination explicitly `Weak` and the rationale "the in-memory
counter, bump call site, and cross-writer race window all survive
verbatim under the renamed field."

**Blocking B2 — Rejected-alternatives discipline; A/B/C ≠ A/B/C/D/E.**
ADR-0020 §Considered alternatives lists five options (A/B/C/D/E);
wave-decisions options-analysis listed three (A/B/C). The
alternative sets must be congruent — adding Alternative D to
wave-decisions explicitly with the same rejection rationale
ADR-0020 carries closes the gap. ADR-0020 also lists Option E
(defer); wave-decisions captures the equivalent argument under
*Recommendation rationale* — Option E is implicit here.
**Fix recommendation**: add Alternative D row matching ADR-0020;
note Option E equivalence inline.

**Blocking B3 — User decision authority paraphrased, not dated/cited.**
The Changelog entry "Revision per user decision on review:
`ClusterStatus` is pure-drop with no replacement field" carries
the rationale but does not anchor the decision to a citable event.
For the audit trail to survive — especially given that the only
record of the user's decision is this conversation — the Changelog
must explicitly attribute the decision: date, conversation thread,
direct user attribution. **Fix recommendation**: in the Changelog
(line 319) and the recommendation rationale, add an explicit
anchor: "Decision: User explicitly rejected Alternative D on
review 2026-04-26 (this conversation; the architect's first
draft proposed `writes_since_boot + process_started_at`; user
response: 'renaming the same race')."

**High — Success-criteria checklist accuracy.**
Prior-run `[x]` marks were inaccurate. Verify each `[x]` against
artifacts that actually exist on disk:
- ADR-0020: exists ✓
- wave-decisions.md: exists ✓
- user-stories.md amendment: exists ✓
- test-scenarios.md amendment: exists ✓
- walking-skeleton.md amendment: exists ✓
- upstream-changes.md: exists ✓
Mark `[x]` only those that are actually present and complete.
Items pending DELIVER (e.g. "DELIVER PR opened", "code changes
landed") stay `[ ]`. **Fix recommendation**: revise checklist to
honest `[x]` / `[ ]` against on-disk verification.

**Suggestion — Decision-record date in Changelog.**
Cite the conversation thread that produced the decision. Combined
with B3.

**Suggestion — Cross-link to ADR-0020 alternatives table.**
The two alternatives sections (this file and ADR-0020) should
be cross-referenced so a reader landing in either can navigate
to the other. ADR-0020 already references this file under
*References*; this file's *Options analysis* section should
add an inline note that ADR-0020 §Considered alternatives is
the canonical inline form.

### Resolution (2026-04-26)

| Issue | Status | Resolution |
|---|---|---|
| **B1** (six-criteria table omits Alt D) | Resolved | Added *Option D — Pure-drop on per-write surface, `writes_since_boot` replacement on `ClusterStatus`* as a fourth row in the options-analysis section, with all six criteria scored. Bug-class-elimination scored **Weak** explicitly with the inline rationale "in-memory counter still exists, bump call site still exists, cross-writer race window still exists, conditional-gate-sprawl pattern still available, restart-reset surprise survives in semantics." Idempotency contract integrity scored Strong (per-write surface unchanged from Option A). Walking-skeleton observability, Phase 2 forward-compat, ADR coherence all scored Adequate. Surgery cost High (comparable to Option A). |
| **B2** (alternative sets congruent A/B/C/D/E) | Resolved | Adding Option D row matches ADR-0020's Considered alternatives list. Option E (defer) is implicit in the *Recommendation rationale* section's "the cost of doing the deletion now is one mechanical PR; the cost of deferring is the next bug" framing — explicit cross-reference to ADR-0020 §Considered alternatives §E added in the Option D rejection rationale ("ADR-0020 §Considered alternatives §D restates it inline so the ADR is self-contained"). |
| **B3** (user decision authority cited) | Resolved | Changelog entry revised to explicitly cite the conversation thread (2026-04-26, this conversation) as the decision authority. Direct user-quote attribution included: *"renaming the same race"*. The architect's first-draft proposal of `writes_since_boot + process_started_at` is named explicitly. The Changelog now reads as a citable audit trail. |
| **High** (success-criteria checklist) | Resolved | Checklist revised to honest `[x]` / `[ ]` against on-disk verification. Each `[x]` annotated with the on-disk path that justifies it. DELIVER-pending items (PR opened, code landed, ADR-0020 status flipped) are honestly `[ ]`. The header note explains the verification discipline. |
| **Suggestion** (decision-record date) | Resolved | Combined with B3 — Changelog now carries explicit date and conversation citation. |
| **Suggestion** (cross-link to ADR-0020) | Resolved | Option D rejection rationale now cross-references ADR-0020 §Considered alternatives §D inline; ADR-0020 already cross-references this file under *References*. |

No deferred issues. All review findings addressed in this revision pass.
