# ADR-0020 — Drop `commit_index` from Phase 1 single-mode

## Status

Accepted. 2026-04-26. (Originally Proposed 2026-04-26; flipped to
Accepted on DELIVER completion per project ADR convention — Status
flips when the change-set lands on `main`, not when the design is
approved. The Phase 1 surgery — wire shape, store, tests, OpenAPI,
ADR amendments, brief edits — landed on `main` in the
`redesign-drop-commit-index` feature commits 01-01 through 01-05.)

Amends:

- ADR-0015 (`HTTP-error-mapping`) §4 *Status-code matrix* — the
  idempotent-success row currently reads "200 OK, same commit_index as
  the original"; revised to "200 OK, same `spec_digest` as the
  original" with a typed `IdempotencyOutcome` enum on the wire (see
  *Decision* below).
- ADR-0008 (`rest-openapi-transport`) endpoint table — `POST /v1/jobs`
  no longer returns `commit_index`; `GET /v1/jobs/{id}` no longer
  returns `commit_index`; `GET /v1/cluster/info` drops `commit_index`
  with no replacement field on `ClusterStatus`.

Does NOT amend:

- ADR-0013 (`reconciler-primitive-runtime`) — Grep audit 2026-04-26:
  zero `commit_index` references in
  `docs/product/architecture/adr-0013-reconciler-primitive-runtime.md`.
  No amendment required.
- ADR-0014 (`CLI-HTTP-client-and-shared-types`) — Grep audit 2026-04-26:
  zero `commit_index` references in
  `docs/product/architecture/adr-0014-CLI-HTTP-client-and-shared-types.md`.
  No amendment required. The briefing's claim that ADR-0014 references
  `commit_index` was incorrect; the CLI's commit-index rendering lives
  in `crates/overdrive-cli/src/commands/cluster.rs`, which is
  implementation, not an ADR-level concern.

## Context

### The structural argument: renaming the same race

The structural finding that drives this ADR — established before any
specific bug-cascade narrative — is that **`commit_index` exposes a
field-name surface whose only honest reading is "an in-memory counter
that resets on process boot," and renaming the field to communicate
that semantic does not eliminate any of the three structural bug
surfaces that produce its failure modes.** The three surfaces are:

1. **The cross-writer race.** The bump call site lives outside redb's
   writer lock; two committers peek the same value before either
   bumps. This is a property of the in-memory counter, not the field
   name.
2. **The conditional-gate sprawl pattern.** Each fix in the cascade
   added a conditional gate around the same bump (the third commit
   added four). The pattern is available to any future contributor
   regardless of what the field is called.
3. **The restart-reset surprise.** The counter resets to zero on
   every process start. A field named `commit_index` hides this; a
   field named `writes_since_boot` advertises it; neither field
   eliminates the semantic, which is present every time a process
   restart truncates what looked like a monotonic counter.

The implication: any architecture that preserves the in-memory
counter, the bump call site, or the field-on-`ClusterStatus` surface
preserves the bug class. The only structurally consistent answer is
to remove the surface that generates the class — to delete, not to
rename. The bug cascade narrative below is the empirical evidence
for this structural finding; it is not the foundation of the
argument.

This framing is load-bearing for the rejection of Alternative D
(`writes_since_boot + process_started_at`) under *Considered
alternatives* below, and for the wave-decisions document's
recommendation that `ClusterStatus` is pure-drop with no replacement
field.

### The bug cascade

`commit_index` has produced three bugs in 26 hours, each correctly
fixed for its target failure mode and each leaving the field's
existence intact:

| Commit | Bug fixed | Bug introduced |
|---|---|---|
| `ba72297` (`fix-commit-index-per-entry`) | Store-wide counter raced against the value returned in the POST response | Doubled the counter surface (per-entry + store-wide); inline `[u64-LE-prefix \|\| value]` row encoding; snapshot frame v2 |
| `97b0069` (`fix-commit-counter-and-watch-doc`) | Phantom bump on no-op deletes / `KeyExists` / empty txn | Added 4 conditional gates around the bump call site |
| `f5b361c` (`fix-commit-counter-and-watch-doc-rca`) | Bump leaked on commit failure | The current cross-writer race — the bump moved outside redb's writer lock; two committers peek the same value before either bumps |

Each fix was a localised correction to a localised symptom. None
stepped back to ask whether the field was earning its keep. The third
bug landed 26 hours ago. The structural question — "should this exist
at all in Phase 1?" — was overdue.

### The investigation found

1. **No reader.** `commit_index` appears in three response shapes
   (`SubmitJobResponse`, `JobDescription`, `ClusterStatus`). Nothing
   in the codebase (CLI, reconcilers, integration tests acting as
   consumers, internal subsystems) reads the surfaced value back as
   input to a subsequent operation. The CLI displays the value;
   nothing consumes the displayed value. Operator workflow surveys
   confirm Ana does not use `commit_index` as a reference point in any
   walking-skeleton journey step. `ClusterStatus.commit_index`
   specifically has *zero* readers in production code or tests beyond
   a `reads-back-as-u64` wiring assertion in
   `tests/integration/cluster_and_node_commands.rs`; the walking-
   skeleton US-04 wiring witness is fully covered by
   `mode + region + reconcilers + broker` (broker.dispatched > 0
   proves the reconciler ran).

2. **Redundant with `spec_digest`.** Idempotent re-submission's
   "this is the same logical record" witness is already
   content-addressed via `spec_digest` (SHA-256 of canonical rkyv
   bytes, ADR-0002 + ADR-0014). `commit_index` is a parallel
   *write-ordered* witness that adds no information the digest
   doesn't carry — and is *strictly weaker*:

   | Property | `commit_index` (Phase 1) | `spec_digest` |
   |---|---|---|
   | Stable across process restart | No | Yes |
   | Deterministic from the spec alone | No | Yes |
   | Stable across `single → HA` migration | No | Yes |
   | Stable across snapshot/restore | No (resets to 0) | Yes |
   | Computable client-side without the server | No | Yes |
   | Identifies "this is the same logical record" | No (only "same insert event") | Yes |
   | Identifies "this is the same insert event" | Yes | No |

   The single property `commit_index` carries that `spec_digest` does
   not — "same insert event" — has no consumer in Phase 1. Phase 2's
   reconcilers will need a real Raft log index for replay; the Phase 1
   in-memory counter is structurally incapable of providing it.

3. **Phase 2 promise unfulfillable.** The docstring at
   `crates/overdrive-store-local/src/redb_backend.rs:138-145` commits
   to Phase 2 `RaftStore` "replacing the implementation while keeping
   the accessor signature stable so handlers stay mode-agnostic." The
   semantics this requires — durable, strictly monotonic across
   restarts, log-entry identity, cluster-wide consensus on order — are
   genuinely Raft properties, not properties of an in-memory counter
   that resets to zero on every process start. Any client that treats
   the Phase 1 value as a stable reference is *already* broken across
   restarts. The "stable signature" framing was a post-hoc
   justification for keeping a placeholder field alive, not a
   load-bearing forward-compat investment. Verification: handlers
   already branch on mode for other Phase-2-only fields (peer status,
   leader identity), so "keep `commit_index` so handlers stay
   mode-agnostic" is not the constraint it appeared to be.

4. **The demand traces to acceptance scenarios, not consumers.**
   `commit_index` is in the API surface because the user stories
   demand it (US-03 / US-04 acceptance criteria explicitly call for
   `LocalStore::commit_index()` accessor + monotonic property), not
   because anything downstream uses the surfaced value. The acceptance
   criteria themselves were written from an "etcd-shaped" reflex —
   "every store has a commit log index, like Postgres LSN or MySQL
   binlog position" — without first asking what Ana actually does with
   the value in single mode. The answer in Phase 1 is: nothing.

### Earned-Trust framing

The bug class is a textbook case of methodology principle 12 (Earned
Trust): *every dependency you do not probe is an act of faith you made
for the user*. The `commit_index` field's existence was a faith claim
— that some future consumer would need it, that the Phase 1
in-memory counter would resemble the Phase 2 Raft log index closely
enough that the signature could be stable, that the monotonicity
property the user stories asserted was a property the *operator* would
notice if it broke. Three production bugs in 26 hours are the
empirical evidence that the faith was misplaced. The field exists
because nobody asked "what happens if the environment lies?" — and
the environment (concurrent writers in redb's writer-lock release
window) lied.

The structural fix is to delete the field, not to add a fourth
conditional gate. A field with no consumer cannot be probed, because
there is nothing to probe *for*.

### Why no `writes_since_boot` replacement on `ClusterStatus`

A `writes_since_boot` field on `ClusterStatus` was considered as an
"honest replacement" for the dropped `commit_index` — same in-memory
counter, same bump on every successful write, same reset-on-process-
start, but with a field name that admits the reset semantics. The
proposal was rejected because it reproduces the bug class verbatim
under a different name:

- The in-memory counter still exists.
- The bump call site still exists.
- The cross-writer race window (peek-then-bump in redb's
  writer-lock release gap) still exists.
- The conditional-gate-sprawl pattern (the third commit in the
  cascade added four conditional gates around the same bump) is
  still available to the next contributor who finds a phantom
  increment.
- The restart-reset surprise is "in the field name" but is still
  a real semantic, present every time a process restart truncates
  what looked like a monotonic counter.

Renaming the field communicates the gotcha to the operator at the
field-name layer; it does not eliminate any of the three structural
bug surfaces (race, conditional gates, restart reset) that produced
the cascade. The next bug in the family — say, a snapshot/restore
interaction that causes the counter to skip values, or a transactional
rollback that double-bumps — would ship under the new name with the
same code shape.

The only structurally consistent answer is to remove the surface that
generates the class. `ClusterStatus` becomes four fields:
`{mode, region, reconcilers, broker}`. If a real activity-rate signal
is needed in the future, it lives in Phase 5's metrics endpoint
(Prometheus-shaped), where it gets durable backing, proper rate
semantics in the query language, and is named honestly — not in a
single u64 on a status RPC where the name is the contract.

## Decision

### 1. Drop `commit_index` from `IntentStore` and `LocalIntentStore`

The trait surface `IntentStore::get` returns `Option<Bytes>` — not
`Option<(Bytes, u64)>`. The `PutOutcome` variants drop their
`commit_index` field:

```rust
pub enum PutOutcome {
    Inserted,
    KeyExists { existing: Bytes },
}
```

`LocalIntentStore` deletes the `commit_counter: AtomicU64` field, the
`peek_next_inside` / `bump_commit_after_commit` helpers, the inline
`[u64-LE-prefix || value]` row encoding (rows are now the
caller-provided bytes verbatim), and the public
`commit_index() -> u64` accessor.

### 2. Drop `commit_index` from API responses

| Response | Was | Now |
|---|---|---|
| `SubmitJobResponse` | `{job_id, commit_index}` | `{job_id, spec_digest, outcome}` where `outcome: IdempotencyOutcome ∈ {"inserted", "unchanged"}` |
| `JobDescription` | `{spec, commit_index, spec_digest}` | `{spec, spec_digest}` |
| `ClusterStatus` | `{mode, region, commit_index, reconcilers, broker}` | `{mode, region, reconcilers, broker}` |

Two replacements on `SubmitJobResponse`, justified by use cases the
dropped field was masking on the per-write surface:

- **`SubmitJobResponse.spec_digest`** — replaces `commit_index` as the
  per-write content witness. Operators hashing their local TOML get
  the same digest the server returned; mismatch is a real bug. The
  digest is stable across restart, deterministic from the spec, and
  identifies the logical record (which `commit_index` did not).
- **`SubmitJobResponse.outcome: IdempotencyOutcome`** — typed enum
  (`"inserted"` | `"unchanged"`) replaces "did the index advance?" as
  the operator-visible signal that distinguishes "your spec landed
  fresh" from "your spec was already there." This is an explicit
  signal where the index was an inferred one (Ana would have had to
  remember the prior index to know whether her re-submit took the
  idempotency branch). 409 Conflict still fires on a different spec
  at the same key; the typed `outcome` field never carries
  `"conflict"` because conflict is an HTTP-status concern, not an
  enumeration value.

`JobDescription` and `ClusterStatus` are pure-drop. No replacement
field is added on either response shape; rationale in the *Why no
`writes_since_boot` replacement* subsection above.

### 3. Snapshot frame: revert v2 → v1

The snapshot frame `Vec<(Vec<u8>, Vec<u8>, u64)>` (v2) reverts to
`Vec<(Vec<u8>, Vec<u8>)>` (v1). The v1 decoder already exists per the
existing module docstring at
`crates/overdrive-store-local/src/snapshot_frame.rs` ("v1 inputs
project the missing index column to `0`"); the encoder reverts to
emitting v1. v2 frames written during the bug-cascade window are not
externally observable (Phase 1 has not shipped), so no upgrade story
is required. Whether v2 is renamed back to v1 or the v2 schema is
kept with the per-entry index column dropped is a DELIVER-lane choice
documented in `upstream-changes.md`; semantically the wire is
identical.

### 4. CLI rendering follows the API

`overdrive job submit` prints `job_id`, `spec_digest`, and `outcome`
(`"created"` | `"unchanged"` for human consumption — the JSON
`outcome` enum is `"inserted"` | `"unchanged"`; the CLI renders the
human form). The "Commit index" line is removed from every CLI output
(`render::job_submit`, `render::alloc_status`, `render::cluster_status`,
`render::node_list`, and any other site identified by
`upstream-changes.md`). `cluster status` output narrows from five
lines to four.

### 5. Phase 2 forward-compat: introduce HA-only fields fresh under Raft semantics

The Phase 2 `RaftStore` design is **not pre-committed by this ADR**.
What this ADR fixes is the principle: **HA-only fields are introduced
fresh under Raft semantics when HA lands, not inherited from Phase 1
placeholders.** Specific Phase 2 field naming (whether the field is
called `log_index`, `raft_log_index`, `commit_index` under genuinely
new semantics, or something else entirely) is deferred to a Phase 2
ADR, written when the `RaftStore` design is in scope.

The forward-compat contract is therefore stated as:

- HA-only response fields are introduced under genuinely Raft
  semantics — durable, strictly monotonic across restart, log-entry
  identity, cluster-wide linearizable order. These are properties
  of openraft's log; they cannot be retrofitted onto an in-memory
  counter.
- Single-mode responses do not include the field — handlers branch
  on mode, exactly as they already do for other HA-only fields
  (peer status, leader identity).
- A Phase 1 placeholder shaped vaguely like the eventual HA field
  — an in-memory counter named `commit_index` that "Phase 2 will
  replace with a real Raft index" — is the architectural drift
  this ADR is correcting. The replacement is not in scope; the
  drift correction is.

Whitepaper §4 (*The Intent / Observation Split*) covers the Raft /
CRDT consistency boundary at the architecture level. It does not
explicitly mandate any specific HA field name on Phase 1 single-mode
response shapes; the principle here is derived from the
Intent/Observation discipline (intent is linearizable, written once
per region; HA fields like leader identity and peer status are
mode-conditional on the wire) rather than from a specific paragraph.

## Considered alternatives

### Alternative A — Keep per-entry only; drop store-wide

Per-entry `commit_index` survives in `SubmitJobResponse` and
`JobDescription` for the idempotency contract; `ClusterStatus.commit_index`
goes. Implementation: redb meta-table counter, durable across restart.

**Rejected.** This addresses one of the three structural problems
(the cross-writer race; closes the restart-reset gap) but preserves
the other two (no consumer; redundant with `spec_digest`). It also
asymmetrically resolves the "what does this field mean" question
(per-entry survives; store-wide dies) without a principled
distinction — the answer to "why per-entry" is "the idempotency
contract is load-bearing," but the idempotency tests' index-equality
assertions are *behavioural* assertions about handler logic ("did the
handler take the idempotency branch and not write?"), not
*operator-visible* assertions about content identity. The behavioural
assertion is preserved by the typed `IdempotencyOutcome` field;
content identity is preserved by `spec_digest`. The per-entry index
adds nothing that those two together do not.

The smaller-scope appeal of Alternative A is also illusory: the trait
surface still changes (return type of `get` and `PutOutcome` variants
both drop their indices on `ClusterStatus`'s side anyway), the
handlers still change, the snapshot frame still changes (the v2 frame
exists *for* the per-entry index — Alternative A keeps it alive). The
deliverable cost is comparable to dropping entirely; the conceptual
cost is higher (two flavours of "commit_index" surviving in different
shapes is a confusion vector for future readers).

### Alternative B — Keep both; fix race only with a redb meta-table counter

The minimal fix from the original bug-fix scope. Replace the
in-memory `AtomicU64` with a redb meta-table counter incremented
inside the same write transaction as the entry insert; the race
closes because the bump is now under redb's writer lock.

**Rejected.** This is a fourth conditional gate around a field that
has no consumer. It addresses the immediate symptom (the race) and
none of the structural problems (no consumer, redundant with
`spec_digest`, Phase 2 placeholder unfulfillable). The bug cascade is
empirical evidence that adding gates around `commit_index` does not
end the bug class — it shifts it. The next bug is whatever the
meta-table counter doesn't quite handle (snapshot/restore semantics,
bootstrap from existing data, transactional rollback interaction).

### Alternative C — Add `Inserted::commit_index` semantics via a redb sequence (preserving the trait surface)

Make the per-entry index a redb-managed sequence (a separate range of
keys whose `len()` is the next index). Atomic inside the writer lock.
Strictly monotonic. Survives restart.

**Rejected.** Same diagnosis as Alternative B — solves a problem
nobody has, at non-trivial implementation cost (now there are two
on-disk encodings to migrate when `RaftStore` lands), without a
consumer.

### Alternative D — Pure-drop on per-write surface, `writes_since_boot` replacement on `ClusterStatus`

The architect's first-pass draft of this ADR. `commit_index` drops
from `SubmitJobResponse` and `JobDescription` (with `spec_digest` +
`outcome` replacing it on submit, exactly as in the accepted Decision);
`ClusterStatus.commit_index` is renamed `writes_since_boot` and paired
with a `process_started_at` field, exposing the reset-on-restart
semantics in the field name. The proposal's framing was that "honest
naming is the smaller-blast-radius answer" — the operator now reads
the field name and understands the counter resets on restart, while
the implementation cost is comparable to the accepted Option A.

**Rejected on user review (2026-04-26 conversation).** The proposal
was rejected because **renaming the field is not probing**, and
preserving the field is preserving the bug class. The full structural
argument:

1. **The in-memory counter still exists.** `LocalIntentStore` retains
   `commit_counter: AtomicU64`. The peek-then-bump dance in
   `redb_backend.rs` — the cross-writer race documented in
   commit `f5b361c`'s RCA — is a property of this counter, not of
   the field name on the wire. A field named `writes_since_boot`
   reads the same atomic the field named `commit_index` did; the
   race window between `peek_next_inside` and
   `bump_commit_after_commit` (outside redb's writer lock) is
   unchanged.

2. **The bump call site still exists.** Every successful write path
   in the impl still calls the bump helper. The four conditional
   gates added in the second commit of the cascade
   (`97b0069`) — guarding against phantom bumps on no-op deletes,
   `KeyExists`, empty txns — survive verbatim. The next contributor
   finding a phantom-increment edge case (snapshot/restore
   interaction, transactional rollback, bootstrap-from-existing-
   data) can add a fifth gate around the same bump under the new
   field name, and the bug cascade pattern repeats.

3. **The cross-writer race window still exists.** The architectural
   choice that produced the third commit in the cascade — moving the
   bump outside redb's writer lock to allow the snapshot frame v2
   migration — is preserved by Alternative D. The race is unchanged;
   only the field name on the wire differs.

4. **The conditional-gate-sprawl pattern is still available.** This
   is the load-bearing point. The cascade is not three unfortunate
   bugs in a row; it is three localised corrections to a surface
   that generates bugs faster than reviewers catch them. Renaming
   the field exposes one aspect of that surface (the restart-reset
   semantic) while leaving the other three (the race, the bump call
   site, the conditional-gate pattern) intact. The next bug in the
   family ships under the new name with the same code shape.

5. **The restart-reset surprise survives in the semantics.** A field
   named `writes_since_boot` advertises the reset to the operator,
   yes — but the value still resets to zero on every process start.
   Any operator workflow that surveys cluster activity over a window
   spanning a restart still sees a discontinuity. The honest naming
   does not eliminate the discontinuity; it just makes it
   non-surprising. The structural fix would eliminate the
   discontinuity, not advertise it.

The methodology principle 12 framing applies directly: every
dependency you do not probe is an act of faith you made for the user.
Alternative D was an attempt to *describe* the unprobed faith claim
honestly rather than *eliminate* it. The cascade is the empirical
evidence that the faith was misplaced — three production bugs in 26
hours, each from a different angle of the same surface. Renaming the
field carries the faith claim forward under a more honest name; only
deletion eliminates it.

The structurally consistent answer is to remove the surface that
generates the class. `ClusterStatus` therefore narrows to four fields
with no replacement; activity-rate signals become a Phase 5 metrics-
endpoint concern, where they get durable backing across restart,
proper rate semantics in the query language, and naming conventions
that match the actual semantics — not a single u64 on a status RPC
where the field name is the contract.

See `docs/feature/redesign-drop-commit-index/design/wave-decisions.md`
*Why `ClusterStatus` is pure-drop* and the *Option D* row in the
options-analysis section for the matching argument in the wave-
decisions document. The two artifacts are deliberately congruent.

### Alternative E — Defer the decision, fix the race, revisit later

**Rejected.** "We'll come back to this" is the disposition that
produced the bug cascade. Three fixes in 26 hours are evidence that
"come back to this" never gets prioritised over the next ticket. The
field has been in the platform for `commit_index_per_entry`'s
lifetime (since 2026-04-25); the consumer-audit gap was visible
throughout. Greenfield, no shipped externals — the cost of doing the
deletion now is one mechanical PR; the cost of deferring is the next
bug, and the next, and reviewer attention permanently spent
re-justifying a vestigial field.

## Consequences

### Positive

- **Bug class eliminated, including under rename.** The cross-writer
  race, the phantom-bump conditional gates, the in-memory counter /
  on-disk row encoding drift, the restart-reset surprise — all become
  unreachable. There is no `commit_counter` to bump, no
  `peek_next_inside` to call before a bump, no inline
  `[u64-LE-prefix || value]` row to decode, no v2 snapshot frame to
  validate, **and no field on `ClusterStatus` whose name implies a
  durable counter that secretly resets**. The surface that generated
  the cascade does not exist, and importantly does not exist under a
  different name with the same code shape.
- **Trait surface narrows.** `IntentStore::get -> Option<Bytes>` is
  the natural shape; `(Bytes, u64)` was awkward at every call site
  (the briefing-flagged "we destructure the bytes side only" pattern
  appears multiple times in the test suite). `PutOutcome` becomes a
  tagged enum with exactly the information the handler needs to
  decide 200-vs-409.
- **API contract honest.** `spec_digest` on `SubmitJobResponse`
  replaces an operator-invisible monotonic counter with a
  content-addressed witness the operator can verify locally.
  `IdempotencyOutcome` makes the 200-with-no-write case explicit
  rather than something the operator has to *infer* from "the index
  didn't advance, but I forget what it was last time."
- **`ClusterStatus` self-documents.** Four fields,
  `{mode, region, reconcilers, broker}`, each answering a question
  the operator demonstrably asks. Activity-rate questions ("how busy
  is this control plane?") are not answered by a status RPC's u64;
  they are answered by metrics with rate semantics in the query
  language. Pushing that signal off the status RPC removes a wrong-
  shape primitive from the wire contract.
- **Phase 2 has a clean introduction path.** Whatever the eventual
  HA-only Raft-log field is named (a Phase 2 ADR decision), it
  lands fresh under genuine Raft semantics — not as a re-purposed
  Phase 1 placeholder.
- **Earned Trust restored.** The `commit_index` field's existence was
  the artifact of an unprobed assumption; deleting it removes the
  unprobed assumption. The bug cascade does not recur because the
  surface that generated it does not exist — and does not exist under
  a different name. Renaming is not probing.

### Negative

- **DELIVER cost is real.** The surgery touches the trait, the
  single-mode impl, the snapshot frame, the sim adapter, every
  handler, the API types, the CLI rendering, ~12 test files, and 3
  documentation surfaces (this ADR, the brief, the user-stories /
  test-scenarios / walking-skeleton amendments). The full surface is
  enumerated in `docs/feature/redesign-drop-commit-index/design/upstream-changes.md`.
  Deliberately accepted: greenfield, no externals, single-cut
  migration discipline.
- **The idempotency tests' "index didn't advance" assertion shape
  changes.** The current
  `triple_resubmit_byte_identical_all_return_same_commit_index`
  asserts `commit_index` equality across N re-submits as a proxy for
  "the handler took the idempotency branch and didn't write." After
  this ADR, the same property is asserted via
  `outcome == "unchanged"` on the second/third response, plus a
  back-door read asserting the stored bytes are byte-equal to the
  original (already in
  `byte_identical_resubmit_returns_original_commit_index_unchanged`).
  Net coverage is preserved; the assertion shape is more direct
  ("the response says `outcome: unchanged`") and matches the
  operator-visible semantics.
- **Phase 2 cannot quietly reuse the `commit_index` field name to
  carry Raft-log semantics.** When HA lands and a real Raft log
  index is appropriate, the field's name is a Phase 2 ADR decision —
  whether it is called `log_index`, `raft_log_index`, or something
  else, the principle is that the new semantics get a fresh name
  rather than an old name with a quiet meaning change. This is
  positive (the new name will carry the new semantics honestly),
  but it does mean any documentation / runbook authored against
  the wire field name needs to track that decision. Phase 1 is
  unshipped, so no external runbooks exist today.
- **`SubmitJobResponse` grows a field.** The wire contract gains
  `spec_digest` (replacing `commit_index`) and `outcome` (new). For
  the operator this is a strict improvement; for the wire-contract
  bytes it is comparable size. The OpenAPI schema diff is not
  trivial but is one-shot.
- **`ClusterStatus` shrinks a field, no replacement.** The pure-drop
  decision means Ana's prior question "is this control plane active?"
  is no longer answered by the status RPC at all — only by the
  reconciler/broker fields (which give *primitive aliveness*, not
  *write-rate*). For Phase 1 this is acceptable: there is no
  walking-skeleton step that asks the write-rate question. Phase 5's
  metrics endpoint is the correct home for write-rate signals when
  they are needed.

### Quality-attribute impact

- **Reliability — fault tolerance / recoverability**: positive. The
  bug class generated by the field is eliminated. Snapshot/restore
  no longer carries an inline per-entry index that has to be reasoned
  about during recovery.
- **Maintainability — analyzability / modifiability**: positive. The
  trait surface is narrower; the call sites are shorter; the storage
  layout is the caller-provided bytes verbatim with no framing. The
  next contributor reading `redb_backend.rs` does not need to
  understand four conditional gates around a counter.
- **Maintainability — testability**: positive. The behavioural
  invariant the per-entry tests were defending ("the handler takes
  the idempotency branch on byte-identical re-submit and does not
  write") is preserved via `outcome == "unchanged"` plus the
  back-door byte-equality check. The deletes are mechanical (whole
  test files); the survivors get clearer assertion shapes.
- **Usability — operability**: positive on the per-write surface
  (`spec_digest` + `outcome` are operator-comprehensible), neutral
  on the status surface (the reconciler/broker fields already answer
  the live walking-skeleton questions; the activity-rate question is
  punted to Phase 5 metrics where it belongs).
- **Security — non-repudiation / accountability**: neutral.
  `commit_index` was never a security artifact (process-local,
  resets-on-restart). `spec_digest` is the cryptographic witness
  that survives.
- **Performance — time behavior / resource utilization**: marginally
  positive. One fewer atomic load per `commit_index()` call (which
  no longer exists); no inline frame decoding per `get`; no v2-frame
  payload size overhead in snapshots. Effects are sub-percent and
  not the motivation.

### Enforcement

- A grep gate (informal — caught by the DELIVER review pass) asserts
  that `commit_index` does not appear in the trait surface,
  handlers, API types, CLI rendering, snapshot frame, or row
  encoding.
- A behavioural test (in `tests/acceptance/submit_job_idempotency.rs`)
  asserts `IdempotencyOutcome::Unchanged` is returned on the second
  byte-identical submit and that the back-door redb read shows the
  stored bytes are byte-equal to the original.
- The OpenAPI schema diff is the contract-level enforcement: the
  generated YAML must match the checked-in `api/openapi.yaml`,
  failing the build if the field set drifts.
- The DST harness invariant `intent_store_returns_caller_bytes`
  (lift from the existing watch-event-shape invariant) asserts that
  `IntentStore::get` returns the bytes as supplied to `put`, with
  no framing overhead — automatic regression guard against a future
  contributor re-introducing inline encoding.

## References

- `docs/feature/redesign-drop-commit-index/design/wave-decisions.md`
  — full options analysis and recommendation, including the
  "renaming the same race" argument that drives the
  `ClusterStatus` pure-drop choice.
- `docs/feature/redesign-drop-commit-index/design/upstream-changes.md`
  — DELIVER target list (every file in the surgery surface with
  specific change shape).
- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
  — US-03, US-04 amendments (Amendment 2026-04-26).
- `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`
  — walking-skeleton 1.1 / 1.2 / 1.3 + §4 + §5 + §6 amendments.
- `docs/feature/phase-1-control-plane-core/distill/walking-skeleton.md`
  — Amendment 2026-04-26.
- ADR-0015 — HTTP error mapping; §4 idempotency-success row revised.
- ADR-0008 — REST/OpenAPI transport; endpoint table revised.
- ADR-0002 — schematic ID canonicalisation; the `spec_digest` story
  this ADR leans on.
- `docs/evolution/2026-04-25-fix-commit-index-per-entry.md`,
  `docs/evolution/2026-04-25-fix-commit-counter-and-watch-doc.md`,
  `docs/evolution/2026-04-26-fix-commit-counter-and-watch-doc.md`
  — historical record of the bug cascade. Reference-only; this ADR
  supersedes the architectural decision they captured.
- Whitepaper §4 — *The Intent / Observation Split*. Grep audit
  2026-04-26: zero `commit_index` references in the whitepaper.
  The field was a Phase 1 amendment, not whitepaper-mandated. The
  whitepaper covers the Raft / CRDT consistency boundary at the
  architecture level; specific Phase 2 HA field naming on Phase 1
  single-mode response shapes is not pre-committed there and is
  deferred to a Phase 2 ADR.
- Methodology principle 12 — *Earned Trust*. The structural framing
  of "every dependency you do not probe is an act of faith you made
  for the user" applies directly here: `commit_index` was the
  unprobed faith claim; the bug cascade was the empirical
  falsification; renaming the same race under a different field name
  would be the next falsification.

## Review (2026-04-26)

Independent peer review of ADR-0020. Verdict: **APPROVED with notes**.
The reviewer found no architectural problems with the decision; five
notes raised on documentation specificity and ADR-convention
compliance — two high, two medium, one low.

### Findings

**High N1 — Status field convention.**
Status currently reads "Proposed. 2026-04-26." Project ADR convention
flips Status from Proposed to Accepted on DELIVER landing; the ADR
should make that transition explicit so a reader landing on the file
mid-flight understands the Proposed-vs-Accepted boundary.
**Fix recommendation**: add a single sentence: "Transitions to
Accepted on DELIVER completion (per project ADR convention)."

**High N2 — Phase 2 `log_index` claim cited as architecture fact.**
The §Decision §5 paragraph presents `log_index: u64` as the field name
the future `RaftStore` will introduce. This is presented as
architecture fact but is not cited against whitepaper §4. Two
options: (a) cite the specific whitepaper §4 paragraph that mandates
Phase 2 introduces a new HA-only field rather than inheriting the
Phase 1 counter, OR (b) reframe as a design principle: "Phase 2
RaftStore design is not pre-committed by this ADR; the principle is
that HA-only fields are introduced fresh under Raft semantics, not
inherited from Phase 1 placeholders. Specific Phase 2 field naming
is deferred to a Phase 2 ADR." The whitepaper §4 doesn't explicitly
name `log_index`; option (b) is honest. **Fix recommendation**: (b).

**Medium N3 — "Renaming the same race" buried in Context.**
The structural argument that drives the entire ADR — "renaming the
field to communicate the gotcha does not eliminate the bug class" —
currently lives in §Context lines 132-167, after the bug-cascade
narrative. Structurally the argument should be the lead, not buried.
**Fix recommendation**: lift the core claim into a separate, earlier
paragraph in §Context, before the bug cascade narrative. The
structural argument should be the lead.

**Medium N4 — Alternative D rejection rationale brevity.**
The Alternative D rejection in §Considered alternatives is currently
12 lines. The wave-decisions document carries the full structural
argument; the ADR currently relies on the cross-reference. For the
ADR to be self-contained as an architectural-decision record, the
rejection rationale should expand to include the "renaming the same
race" argument inline. **Fix recommendation**: expand to enumerate
the four structural surfaces (in-memory counter, bump call site,
race window, conditional-gate-sprawl pattern, restart-reset
semantic) that survive Alternative D's renaming.

**Low N5 — ADR-0013 / ADR-0014 non-amendment claims.**
Currently reads "verified no `commit_index` reference." The reviewer
suggests citing the explicit grep verification: "Grep audit
2026-04-26: zero `commit_index` references in [file]. No amendment
required." This makes the audit step replicable rather than asserted.
**Fix recommendation**: cite the grep audit explicitly in both
non-amendment entries.

### Resolution (2026-04-26)

| Issue | Status | Resolution |
|---|---|---|
| **N1** (Status field convention) | Resolved | Status field updated to "Proposed. 2026-04-26. Transitions to Accepted on DELIVER completion (per project ADR convention — Status flips when the change-set lands on `main`, not when the design is approved)." The Proposed-vs-Accepted boundary is now explicit. |
| **N2** (Phase 2 `log_index` framing) | Resolved | Adopted option (b) — reframed as a design principle. Decision §5 now reads "The Phase 2 `RaftStore` design is **not pre-committed by this ADR**. What this ADR fixes is the principle: HA-only fields are introduced fresh under Raft semantics when HA lands, not inherited from Phase 1 placeholders. Specific Phase 2 field naming (whether the field is called `log_index`, `raft_log_index`, `commit_index` under genuinely new semantics, or something else entirely) is deferred to a Phase 2 ADR." Positive consequence and Negative consequence sections updated to match. References section's whitepaper §4 entry now states explicitly that the whitepaper does not mandate a specific Phase 2 field name; specific HA field naming is deferred to a Phase 2 ADR. |
| **N3** (lift "renaming the same race" earlier) | Resolved | Added a new lead subsection to §Context titled *The structural argument: renaming the same race*, placed before the bug-cascade narrative. The lead enumerates the three structural surfaces (cross-writer race, conditional-gate sprawl pattern, restart-reset surprise) that survive any rename, and frames the bug cascade as empirical evidence for the structural finding rather than as the foundation of the argument. The original *Why no `writes_since_boot` replacement on `ClusterStatus`* subsection survives further down §Context as the load-bearing detail; the new lead is the structural framing. |
| **N4** (expand Alternative D rationale) | Resolved | §Considered alternatives §D expanded from 12 lines to a full structural argument enumerating the five points: (1) the in-memory counter still exists, (2) the bump call site still exists, (3) the cross-writer race window still exists, (4) the conditional-gate-sprawl pattern is still available, (5) the restart-reset surprise survives in semantics. Methodology principle 12 framing applied directly inline. The ADR is now self-contained without requiring cross-reference to wave-decisions.md, while still cross-linking for symmetry. |
| **N5** (grep verification cite) | Resolved | ADR-0013 and ADR-0014 non-amendment entries now cite the explicit grep audit: "Grep audit 2026-04-26: zero `commit_index` references in [file]. No amendment required." The whitepaper §4 reference under §References also now cites the grep audit explicitly. The audit step is replicable rather than asserted. |

No deferred issues. All review findings addressed in this revision pass.
