# issue-141 — JobLifecycleView: persist backoff inputs, not derived deadline

**Date:** 2026-05-03
**Issue:** [#141](https://github.com/overdrive-sh/overdrive/issues/141)
**PR:** [#143](https://github.com/overdrive-sh/overdrive/pull/143)
**Branch:** `marcus-sa/reconciler-view-cache-comment`
**Status:** Implemented

## Summary

Replaced `JobLifecycleView::next_attempt_at: BTreeMap<AllocationId, Instant>` —
a derived deadline persisted as the reconciler's memory of "when may this
allocation restart" — with `last_failure_seen_at: BTreeMap<AllocationId,
UnixInstant>`, the *observation timestamp* of the last failure. The backoff
deadline is now recomputed on every reconcile tick from
`last_failure_seen_at + backoff_for_attempt(restart_count)`. The persisted
field is an input to the active policy; the deadline is an output recomputed
from the live policy plus the persisted inputs.

Introduced `overdrive_core::wall_clock::UnixInstant` — a `Duration`-since-
`UNIX_EPOCH` newtype with `rkyv::Archive`/`Serialize`/`Deserialize` derives,
saturating arithmetic, and full newtype-completeness (`Display` / `FromStr`
/ serde matching exactly, with proptest roundtrips). `TickContext` gained a
`now_unix: UnixInstant` field alongside the existing `now: Instant`; the
runtime populates it once per tick from `UnixInstant::from_clock(&*state.clock)`.
`Instant` stays for in-process budget / deadline arithmetic; `UnixInstant`
is the persistable wall-clock primitive.

## Business Context

Two structural concerns drove the shape of this change.

**Persist inputs, not derived state.** `Instant` is a process-local
monotonic clock reading whose origin is unspecified and system-boot-relative
— Rust's std docs explicitly state that there is no method to get "the
number of seconds" from an `Instant`. Persisting it across a control-plane
restart cannot preserve original semantic meaning. Issue #139 wires a libSQL
hydrate path for the view; if that work inherited the `Instant`-typed
deadline field, it would either (a) define a wire format that papers over
`Instant`'s non-portability with a hack, or (b) block on this redesign.

Beyond portability, the deeper concern is that a precomputed deadline is a
stale cache of the policy that produced it. Operator-configurable backoff
policy (Kubernetes `restartPolicy`-shape, Nomad `restart` stanza-shape) is a
stated future requirement. Phase 1 hardcodes `RESTART_BACKOFF_DURATION =
Duration::from_secs(1)`; Phase 2+ lifts this into per-job operator-supplied
policy. If the deadline were persisted, every change to the constant — and
every per-job override that lands later — would silently no-op against
allocations whose `next_attempt_at` was written under the old policy. The
operator would change the configuration boundary and observe the change at
the read site for new allocations only; existing rows would carry the old
policy until they aged out. Recovery would require a column migration
(re-deriving the missing inputs from a value that has already lost them) or
an explicit drain.

This is precisely the failure mode the project rule
`.claude/rules/development.md` § "Persist inputs, not derived state"
codifies. The rule was added in `de36b89` (predecessor of this branch); this
feature is the first place in the codebase to apply it as a refactor of an
existing field. The codebase precedent is
`crates/overdrive-control-plane/src/worker/exit_observer.rs:291`, where the
persisted input is `attempts` and the policy lookup is the indexed
`RETRY_BACKOFFS` table — swap the table, the next attempt picks up the new
schedule with no migration, no drain, no inconsistency window.

**Portable wall-clock representation.** `UnixInstant` fills a missing
primitive. The codebase had no persistable wall-clock type; reconciler
memory needed one and #139's libSQL hydrate path will consume it directly.
The newtype satisfies the project's strict completeness rules from
`.claude/rules/development.md` § "Newtype completeness": validating
constructors returning `Result`, `FromStr`/`Display`/serde matching exactly,
mandatory proptest roundtrips for both Display/FromStr and rkyv archive →
access → deserialise → equal.

## Key Architectural Decisions

**Option E over Option B.** The research doc evaluated five candidate
representations. Option B (precomputed deadline as `UnixInstant`) was the
minimal-diff choice — same field, just a portable type. Option E (persist
inputs, recompute on every read) was the explicitly chosen shape because
operator-configurable backoff is a stated requirement and Option B locks the
active policy into the persisted value at write time. The research doc's
"Recommendation" section captures the trade-off; this PR ships Option E.

**`UnixInstant` is additive, not a replacement for `Instant`.** The
`TickContext.now` field stays `Instant` because in-process budget timing
(per-tick deadline arithmetic, the reconciler runtime's tick budget) is
exactly what `Instant` is for. `UnixInstant` is added alongside as the
persistable wall-clock primitive. Reconcile reads `tick.now_unix` for any
comparison whose right-hand side is a persisted timestamp; reads `tick.now`
for any comparison whose right-hand side is a per-tick budget. The two
clocks have different semantics and the type system enforces the boundary.

**`backoff_for_attempt(_attempt: u32) -> Duration` as a const fn.** The
function is a degenerate lookup in Phase 1 — it ignores `_attempt` and
returns `RESTART_BACKOFF_DURATION` regardless. The leading underscore is
deliberate: it documents the unused-parameter intent at the call site so
operator-configurable policy can land in Phase 2+ without changing call
shape. This is the codified pattern: the function is the policy lookup
function the rule talks about; the constant is the table entry. Today the
table has one row; tomorrow it has many.

**Canonical `fresh_tick(now: Instant, now_unix: UnixInstant)` signature.**
Test fixtures across five files (overdrive-sim, overdrive-core,
overdrive-control-plane) previously had a `fresh_tick(now: Instant)` helper
with subtle per-file variation. Step 03-01 swept all of them to one uniform
signature taking both clocks explicitly. Tests that don't exercise wall-clock
pass `UnixInstant::from_unix_duration(Duration::from_secs(0))` at the call
site so the helper signature stays uniform — no shared-default fallback,
no per-file variation. This makes the next test-author's job mechanical: one
shape, one import, no thinking.

**Restart-survival as the load-bearing acceptance bullet.** Issue #141's
acceptance section pinned the property "a no-op policy change in code
produces identical reconcile output across restart." Step 03-02 extended
`runtime_convergence_loop.rs` with an explicit DST assertion: simulate a
restart by dropping the `JobLifecycleView`, rehydrate from
`(restart_counts, last_failure_seen_at)`, run the next reconcile tick
against the same `TickContext`, and assert the returned Actions are
bit-identical to the pre-restart tick. This is the property the new shape
enables — and the property a precomputed deadline could not provide.

## Steps Completed

All 6 steps reached COMMIT/PASS per `execution-log.json` (DES schema 3.0).
RED_UNIT was SKIPPED (NOT_APPLICABLE) on every step — for newtype work the
public signature IS the driving port and acceptance proptests cover all
completeness branches; for the reconciler retype the public `reconcile`
signature IS the driving port and acceptance tests cover the boundary +
restart-survival behaviours at that port; for the fixture sweep the gate is
the existing acceptance suite plus an invariant grep.

| Step | Commit | Phase | Outcome |
|------|--------|-------|---------|
| 01-01 | `5d76498` | UnixInstant newtype + arithmetic | PASS — type + constructors + Add/Sub impls land; `cargo check -p overdrive-core` green |
| 01-02 | `89256af` | Display/FromStr/Serde completeness + proptest roundtrips | PASS — canonical 9-digit nanos form; structured ParseError; mutation kill rate 100% |
| 02-01 | `5736caf` | TickContext.now_unix + backoff_for_attempt + runtime construction | PASS — additive field; const fn signature stable for Phase 2+ |
| 02-02 | `282c0a3` | JobLifecycleView retype + reconcile read/write rewrite | PASS — `last_failure_seen_at` shape; deadline recomputed each tick; mutation kill rate 100% |
| 03-01 | `5161337` | Canonical `fresh_tick(now, now_unix)` signature + fixture sweep | PASS — five test files swept; zero `next_attempt_at` references remain |
| 03-02 | `62abeb9` + `83414e0` | DST suite + per-PR mutation gate end-to-end | PASS — restart-idempotence DST assertion lands; mutation kill rate 100% after kill-fix commit |

A trailing rules-alignment commit `a4aafbf` updated
`.claude/rules/development.md` § "Reconciler I/O" example to use the
`hydrate` / `reconcile` split + `tick.now_unix` shape introduced by this
feature.

## Quality Gates

- **Workspace tests** — 831/831 passing under
  `cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`.
- **Mutation testing** — kill rate 100% (19/19 viable mutants caught) on the
  per-PR diff scope after step 03-02's kill-fix follow-up. CI gate
  `cargo xtask mutants --diff origin/main --features integration-tests`
  passes.
- **Quality gates G1–G9** — verified per the quality-framework skill:
  - G1: exactly one acceptance test active during RED_ACCEPTANCE per step.
  - G2: every RED_ACCEPTANCE failure was a business-logic failure, not an
    import / syntax / setup error.
  - G3: unit-level proptest failures triggered on assertion, not panic.
  - G4: no mocks inside the hexagon; only `Sim*` adapters at port boundaries
    (`SimClock` for time).
  - G5: business language in test names — `unix_instant_arithmetic_and_clock_construction`,
    `tick_context_carries_now_unix_and_runtime_constructs_from_clock`,
    `job_lifecycle_recomputes_deadline_from_persisted_inputs`,
    `dst_suite_green_and_mutation_gate_passes_with_restart_idempotence`.
  - G6: GREEN at every step; G7: 100% passing before COMMIT (acceptance +
    unit + proptest; fmt clean; clippy `-D warnings` clean).
  - G8: test budget honoured (≤ 2x distinct behaviours per step).
  - G9: zero test modifications during GREEN/REFACTOR.
- **dst-lint** — clean. No `Instant::now()` / `SystemTime::now()` reachable
  from inside any reconcile method body in `overdrive-core` or
  `overdrive-control-plane`.
- **DES integrity** — `cargo xtask verify_deliver_integrity` exit 0; all 6
  steps have complete DES traces.

## Lessons Learned

**The persist-inputs-not-derived-state pattern is now codified as a project
rule.** The rule landed in `de36b89` ahead of this feature; this PR is the
first refactor of an existing persisted field to apply it. Every subsequent
reconciler memory field added to the codebase will be reviewed against this
rule. The two symptoms-during-review the rule documents — a persisted field
whose name describes a future event (`next_attempt_at`, `expires_at`,
`scheduled_for`), and a read site that uses a persisted field directly in a
comparison without consulting a policy table or function — are exactly the
shape this PR removed. Reviewers can grep for the silhouette
`if now < persisted.deadline` and propose the structural fix mechanically.

**`UnixInstant` fills a missing primitive that #139 will consume directly.**
The libSQL hydrate path for the JobLifecycleView (issue #139) was the
forcing function for this work — it could not have shipped a sensible wire
format on top of `Instant`. With `UnixInstant` in place, #139's hydrate
path is one rkyv-roundtrip away from working. The newtype's full
completeness (FromStr/Display/serde + proptest roundtrips) means it is
ready for the libSQL `TEXT` and `INTEGER` representations, the JSON
operator-API representation, and the rkyv archive bytes the IntentStore
will eventually carry — no second pass needed.

**Time as injected state, not a side channel.** The `Clock` trait already
controlled wall-clock under DST (`SystemClock` in production, `SimClock`
under simulation). `TickContext.now_unix` is the natural extension: the
runtime snapshots `clock.unix_now()` once per evaluation and threads it
through the pure `reconcile` function, where the type is `UnixInstant` and
the only operations are saturating arithmetic. No production `reconcile`
body reads wall-clock directly; no test can forget to inject one. The
contract continues to be enforced by the type system rather than by
discipline.

## Out of Scope (Deferred)

- **libSQL hydrate path for `JobLifecycleView`** — issue #139. The shape is
  ready; the hydrate implementation is a separate piece of work.
- **Operator-configurable per-job restart policy** — Phase 2+. The
  `backoff_for_attempt` const fn signature and the persisted-inputs shape
  are both stable for the policy surface to land into; today's degenerate
  one-row table becomes tomorrow's per-job lookup with no further reshaping.
- **`Instant`-typed fields other than `next_attempt_at`** — none exist; nothing
  else needs migrating.

## Links

- PR: [#143](https://github.com/overdrive-sh/overdrive/pull/143)
- Issue: [#141](https://github.com/overdrive-sh/overdrive/issues/141)
- Research: `docs/research/control-plane/issue-139-followup-portable-deadline-representation-research.md`
- Codebase precedent: `crates/overdrive-control-plane/src/worker/exit_observer.rs:291`
- Project rule: `.claude/rules/development.md` § "Persist inputs, not derived state"
- ADR-0013 §2b (runtime hydrate-then-reconcile contract)
- Whitepaper §17 (per-primitive libSQL), §18 (reconciler I/O purity)
- Related: #139 (libSQL view_cache replacement; this PR prepares the field shape)
