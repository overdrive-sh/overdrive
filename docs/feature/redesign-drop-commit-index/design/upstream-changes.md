# Upstream changes — redesign-drop-commit-index

**Wave**: DESIGN deliverable, consumed by DELIVER
**Status**: Design-locked. The DELIVER crafter consumes this file as
            the target list. Every entry below is a specific change
            shape, not a "modify file X" gesture.
**Decision**: Option A — pure-drop. ADR-0020.

## How to read this file

Each entry has the shape:

> **`{absolute path from workspace root}`**
> *Current state*: what is in the file today.
> *New state*: what the DELIVER PR lands.
> *Rationale tag*: one of `drop-field` / `simplify-impl` /
> `revise-test` / `revise-doc` / `revise-adr` / `delete-file` /
> `regenerate`.
> *Notes*: anything the crafter needs that is not obvious from the
> rationale tag.

Where line ranges are cited, the citation is informational — the
DELIVER crafter is expected to re-grep at the time of editing because
the surrounding code may have moved between this DESIGN run and the
DELIVER PR. Where a choice between alternatives might otherwise have
required architect adjudication, this DESIGN run has locked the
choice explicitly (see e.g. §2 *snapshot_frame.rs* — revert to v1,
locked, not a DELIVER-lane choice; §3 *api.rs* — `IdempotencyOutcome`
canonically defined in `api.rs`, locked).

The single-cut migration discipline (project memory
`feedback_single_cut_greenfield_migrations`) applies to every entry:
delete the old paths and land the new ones in one PR. No deprecations,
grace periods, or feature-flagged old paths.

---

## 1. Trait surface

### `crates/overdrive-core/src/traits/intent_store.rs`

*Current state* (verified by grep audit 2026-04-26):

- **Line 193**: `async fn get(&self, key: &[u8]) -> Result<Option<(Bytes, u64)>, IntentStoreError>;` — the `u64` is the per-entry `commit_index`.
- **Lines 99-104**: `PutOutcome::Inserted { commit_index: u64 }` — the per-entry index assigned inside the same write transaction.
- **Lines 112-118**: `PutOutcome::KeyExists { existing: Bytes, commit_index: u64 }` — the per-entry index of the prior `Inserted` that placed `existing` at this key.
- **Lines 69-92**: type-level docstring on `PutOutcome` carrying the per-entry vs store-wide distinction.
- **Lines 178-192**: docstring on `get` carrying the per-entry semantics framing and the "store-wide cursor" cross-reference to `LocalIntentStore::commit_index()`.
- **Lines 217-228**: docstring on `watch` referencing the inline framing implementation detail.

*New state*:

```rust
pub enum PutOutcome {
    Inserted,
    KeyExists { existing: Bytes },
}

#[async_trait]
pub trait IntentStore: Send + Sync + 'static {
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, IntentStoreError>;
    // …all other methods unchanged in arity but with `(Bytes, u64)`
    //   tuple references replaced with `Bytes`…
}
```

Specific edits:

- Line 193: change return type from `Result<Option<(Bytes, u64)>, IntentStoreError>` to `Result<Option<Bytes>, IntentStoreError>`.
- Lines 99-104: drop the `commit_index: u64` field from `Inserted`. The variant becomes a unit variant: `Inserted,`.
- Lines 112-118: drop the `commit_index: u64` field from `KeyExists`. The variant becomes `KeyExists { existing: Bytes },`.
- Lines 69-92: rewrite the type-level docstring — drop the per-entry-vs-store-wide distinction (vacuous post-drop). Keep the documentation of the variant semantics (which caller wins, what `existing` carries).
- Lines 178-192: rewrite `get`'s docstring — drop the per-entry semantics framing; drop the cross-reference to `LocalIntentStore::commit_index()` (the accessor no longer exists).
- Lines 217-228: rewrite `watch`'s docstring — the parenthetical "as `LocalIntentStore` does" reference to inline framing is dropped (no inline framing post-drop). Keep the underlying semantics ("watch events carry caller-provided bytes").

The new public type `IdempotencyOutcome` is **not** added to this
trait surface — it is an API-layer concern (see §3 / `api.rs`). The
trait stays at the storage-semantic layer (`PutOutcome::Inserted` vs
`PutOutcome::KeyExists`); the handler maps these to
`IdempotencyOutcome::Inserted` / `IdempotencyOutcome::Unchanged` on
the wire.

*Rationale tag*: `drop-field`.

*Notes*: every docstring mentioning the per-entry index is
rewritten or deleted. The "Phase 2 RaftStore replaces the
implementation while keeping the accessor signature stable" framing
in the docstring is deleted in full — see ADR-0020 §Decision §5 for
the forward-compat principle (HA-only fields are introduced fresh
under Raft semantics, not inherited from Phase 1 placeholders;
specific Phase 2 field naming is deferred to a Phase 2 ADR).

---

## 2. Single-mode implementation

### `crates/overdrive-store-local/src/redb_backend.rs`

*Current state*: this file carries the bulk of the
`commit_index`-related machinery. The briefing identifies the
following surfaces (line ranges informational; verify at edit time):

- `commit_counter: AtomicU64` field on the backend struct.
- `peek_next_inside` helper — peeks the next index inside the redb
  write transaction.
- `bump_commit_after_commit` helper — bumps the in-memory counter
  *after* `write.commit()` returns (the cross-writer race documented
  in `f5b361c`'s RCA).
- Public `commit_index() -> u64` accessor.
- `encode_entry` / `decode_entry` helpers wrapping the inline
  `[u64-LE-prefix || value]` row encoding (lines ~1-50 of the
  module docstring).
- `debug_assert_eq!` calls cross-checking peek and bump. **Verified
  by grep audit 2026-04-26: there are exactly 3 `debug_assert_eq!`
  calls in `redb_backend.rs` at lines 312, 392, and 525.** Each
  asserts peek/bump alignment. Post-deletion the file contains zero
  `debug_assert_eq!` calls; the DELIVER crafter re-greps at edit
  time and deletes every match. (The original briefing referenced
  "four" calls, but live grep audit confirms three. If a fourth has
  appeared by edit time, the rule "delete every match" still
  applies.)
- Peek/bump call sites in `put`, `put_if_absent`, `delete`, `txn`.
- The `bootstrap_from` `fetch_max(commit_index)` call site.

*New state*: every item in the list above is deleted. After the cut:

- No `commit_counter` field on the backend struct.
- No `peek_next_inside` / `bump_commit_after_commit` helpers; no
  call sites referencing them.
- No public `commit_index()` accessor.
- No inline row encoding. `encode_entry` and `decode_entry` are
  deleted (or replaced by trivial pass-through and inlined out, at
  the crafter's discretion — semantically the row stored equals the
  caller-provided bytes verbatim).
- All four `debug_assert_eq!` calls deleted.
- `put` / `put_if_absent` / `delete` / `txn` simplified to the
  shapes that do not include the bump/peek dance.
- `bootstrap_from` no longer reads or writes a counter; it clears
  the `entries` table and replays the snapshot rows verbatim.

The `watch` broadcast-channel implementation is unaffected by this
ADR — the `(key, value)` event payload was already free of the
commit index per the existing module docstring.

*Rationale tag*: `simplify-impl`.

*Notes*: the docstring for the file is rewritten end-to-end. The
"Per-entry `commit_index` storage" section, the "Why packed inline
rather than a parallel index table" subsection, and the
"Phase 2 RaftStore" forward-compat framing are all deleted; what
remains is a short docstring describing the `entries` table layout
(key bytes → value bytes verbatim) and the `watch` channel.

### `crates/overdrive-store-local/src/snapshot_frame.rs`

*Current state* (verified by grep audit 2026-04-26):

- `pub const VERSION: u16 = 2;` — current encoder writes v2.
- `pub const VERSION_V1: u16 = 1;` — v1 decoder support.
- v2 payload is rkyv-archived `Vec<(Vec<u8>, Vec<u8>, u64)>`; the `u64` is the per-entry `commit_index`.
- v1 decoder accepts `Vec<(Vec<u8>, Vec<u8>)>` and projects the missing index column to `0` (forward-compat for DR snapshots).

*New state*: **revert to v1**. Decision-locked per
wave-decisions.md §Decision and ADR-0020 §Decision §3 — this is
not a DELIVER-lane choice; the DESIGN wave already chose.

The encoder reverts to emitting `Vec<(Vec<u8>, Vec<u8>)>` under the
v1 schema discriminator. The v1 decoder already exists per the
existing module docstring; it is unchanged. The v2 frame definition,
the v2 encoder path, and the v2 decoder branch are deleted entirely.
`pub const VERSION: u16` becomes `1` (or the constant is renamed to
match the v1 spelling and the `_V1` alias removed; either is
mechanically equivalent — the DELIVER crafter picks the cleaner
mechanical edit at edit time).

After the cut:

- `pub const VERSION: u16 = 1;` (or `pub const VERSION: u16 = VERSION_V1;` reduced to a single constant).
- The encoder emits `Vec<(Vec<u8>, Vec<u8>)>` under v1.
- The decoder accepts only v1 frames; the `VERSION_V1` decode branch is preserved verbatim, the `VERSION` (v2) branch is deleted.
- The "v1 inputs project the missing index column to `0`" framing in the module docstring is rewritten — there is no missing column anymore; the v1 frame is canonical.

Snapshot proptests in `tests/integration/snapshot_proptest.rs`
revise to drop the index column from the property generators (see
§7 below for the test-side change).

v2 frames written during the bug-cascade window are not externally
observable — Phase 1 has not shipped. No upgrade story is required
for v2-on-disk data; development databases are scratch.

*Rationale tag*: `simplify-impl`.

*Notes*: this entry was previously framed as "architect adjudication
required at DELIVER time." The peer review of upstream-changes.md
(2026-04-26) flagged this as a critical issue — wave-decisions and
ADR-0020 already chose, and re-opening the choice in DELIVER risks
the crafter making a different decision than the design records.
The deferred framing is removed; the decision is locked here to
match.

### Sim adapter — there is no separate sim adapter for `IntentStore`

*Verified by ls + grep audit 2026-04-26*: the `crates/overdrive-sim`
crate has adapters for `Clock`, `Transport`, `Entropy`, `Dataplane`,
`Driver`, `Llm`, and `ObservationStore` (all under
`crates/overdrive-sim/src/adapters/`). It does **not** contain a
sim adapter for `IntentStore`. The DST harness
(`crates/overdrive-sim/src/harness.rs`) instead reuses the **real**
`LocalIntentStore` from `crates/overdrive-store-local` on a per-host
tempdir — see the harness module docstring (lines 1-21 / 313-318):

```rust
// Real LocalIntentStore on a per-host tempdir — shared with evaluator
overdrive_store_local::LocalIntentStore::open(&store_path)
    .map_err(|source| HarnessError::LocalIntentStoreOpen { index, source })?,
```

The implication: **all `commit_index`-related changes in the sim
crate are downstream consequences of the trait edit (§1) and the
single-mode impl edit (`redb_backend.rs` above), not a separate sim
adapter rewrite.** The sim crate compiles against the same
`IntentStore` trait and the same `LocalIntentStore` impl as
production; once the trait drops `(Bytes, u64)` and `PutOutcome`
drops `commit_index`, any sim-side call site that destructures the
tuple or matches against the variant fields needs the same
mechanical edits.

*New state*: the DELIVER crafter greps `crates/overdrive-sim/` for
`commit_index`, `(Bytes, u64)`, `Option<(Bytes, u64)>`, and any
match arms that destructure `PutOutcome::Inserted { commit_index }`
or `PutOutcome::KeyExists { commit_index, .. }`. Each match site
becomes the same mechanical edit as the production-side equivalent
(drop the tuple unpack on `get` returns, drop the field bind on
`PutOutcome` matches, drop any assertion on the index value). The
audit at edit time is the safety net.

The previous DESIGN-run framing ("verify the adapter file path at
edit time — if the path differs, the change shape is the same") was
incorrect: there is no sim adapter file to edit, and the change
shape is downstream-of-trait, not sim-adapter-mirroring. This entry
is corrected here to reflect on-disk reality.

*Rationale tag*: `simplify-impl` (downstream consequence; no
standalone sim file edit).

*Notes*: this correction is one of the issues raised in the peer
review of upstream-changes.md (2026-04-26) — the prior framing
quoted a file path that does not exist. The grep audit at edit time
remains the correctness check for sim-side downstream consequences.

---

## 3. Control-plane handlers

### `crates/overdrive-control-plane/src/handlers.rs`

*Current state* (three sites):

1. `submit_job` — calls `IntentStore::put`, destructures the
   `PutOutcome` to extract `commit_index`, writes it into
   `SubmitJobResponse`.
2. `describe_job` — calls `IntentStore::get`, destructures
   `(Bytes, u64)`, writes the `u64` into `JobDescription`.
3. `cluster_status` — calls `LocalStore::commit_index()` (the public
   accessor), writes the value into `ClusterStatus`.

*New state*:

1. `submit_job` — calls `IntentStore::put`, matches the
   `PutOutcome` variant, computes `IdempotencyOutcome` from the
   match (`Inserted` → `IdempotencyOutcome::Inserted`,
   `KeyExists { existing }` → `IdempotencyOutcome::Unchanged`).
   Computes `spec_digest = ContentHash::of(archived_bytes)` already
   on the rkyv-archived value (this computation already exists for
   `JobDescription`; the handler reuses it). Writes
   `{job_id, spec_digest, outcome}` into `SubmitJobResponse`. The
   conflict path (different spec at same key) is detected by
   comparing the digest of the existing bytes with the digest of
   the candidate bytes; mismatched digest under
   `KeyExists { existing }` returns 409 Conflict (this is the
   pre-existing behaviour; only the field set on the 200 response
   changes).
2. `describe_job` — calls `IntentStore::get`, takes the `Bytes`
   directly (no tuple destructure), recomputes the digest, writes
   `{spec, spec_digest}` into `JobDescription`.
3. `cluster_status` — no longer calls `LocalStore::commit_index()`
   (the accessor is deleted). Writes
   `{mode, region, reconcilers, broker}` into `ClusterStatus`. No
   replacement field for the dropped commit index.

   **Pure-drop rationale (self-contained):** the dropped
   `commit_index` field on `ClusterStatus` was an in-memory counter
   that resets on process boot. Renaming the field to
   `writes_since_boot` was considered (Alternative D in
   wave-decisions and ADR-0020) and rejected because the rename
   communicates the reset semantic to the operator without
   eliminating the bug surface that produces the cascade — the
   in-memory counter, the bump call site, and the cross-writer
   race window all survive verbatim under the new name. Pure-drop
   removes the surface that generates the class. The walking-
   skeleton US-04 wiring witness ("did the reconciler primitive
   run?") is preserved by `broker.dispatched > 0` plus the
   `reconcilers` list. Activity-rate signals belong in Phase 5's
   metrics endpoint, not on the status RPC. See ADR-0020
   §Considered alternatives §D for the full structural argument.

The `IdempotencyOutcome` mapping in `submit_job` references the
canonical type definition in `crates/overdrive-control-plane/src/api.rs`
(see the `api.rs` entry below for the locked derive set and wire
shape `"inserted"` | `"unchanged"`).

*Rationale tag*: `drop-field` (`submit_job`, `describe_job`,
`cluster_status`) + the new `IdempotencyOutcome` mapping in
`submit_job`.

### `crates/overdrive-control-plane/src/api.rs`

*Current state* (verified by grep audit 2026-04-26): the file ships
the three API response types with `commit_index: u64` fields —
- **Line 62**: `pub commit_index: u64,` on `SubmitJobResponse`.
- **Line 86**: `pub commit_index: u64,` on `JobDescription`.
- **Line 112**: `pub commit_index: u64,` on `ClusterStatus`.

The fields are documented across lines 39-58, 65-86, 92-112 with
"per-entry" vs "store-wide" framing.

*New state*:

```rust
#[derive(Serialize, Deserialize, ToSchema)]
pub struct SubmitJobResponse {
    pub job_id: JobId,
    pub spec_digest: ContentHash,
    pub outcome: IdempotencyOutcome,
}

/// Outcome of an idempotent `POST /v1/jobs` submission.
///
/// Distinguishes "your spec landed fresh" from "your spec was already
/// there." Conflict (different spec at same key) is an HTTP-status
/// concern (409), never an enumeration value here.
///
/// Wire shape: `"inserted"` | `"unchanged"` (lowercase JSON via
/// `#[serde(rename_all = "lowercase")]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum IdempotencyOutcome {
    /// The handler took the insert branch — `IntentStore::put_if_absent`
    /// returned `PutOutcome::Inserted`.
    Inserted,
    /// The handler took the idempotency branch —
    /// `IntentStore::put_if_absent` returned
    /// `PutOutcome::KeyExists { existing }` and the candidate bytes
    /// were byte-equal to `existing`. No write occurred.
    Unchanged,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct JobDescription {
    pub spec: Job,
    pub spec_digest: ContentHash,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct ClusterStatus {
    pub mode: ClusterMode,
    pub region: Region,
    pub reconcilers: Vec<ReconcilerName>,
    pub broker: BrokerCounters,
}
```

**Decision-locked: `IdempotencyOutcome` is defined in
`crates/overdrive-control-plane/src/api.rs`, colocated with the
other API response types.** The peer review of upstream-changes.md
(2026-04-26) flagged the previous "crafter choice; both are
defensible" framing as ambiguous — the DELIVER crafter must not
re-open a settled decision. Rationale for `api.rs` colocation:
- Other API response types live there; the enum is a wire-contract
  type and shares their lifecycle.
- The OpenAPI schema is generated by walking the `api.rs` items;
  colocation simplifies the `ToSchema` derive surface.
- A crafter searching for "where does the wire shape of
  `IdempotencyOutcome` live" reaches `api.rs` naturally.

The exact derives on the type are also locked here:
`Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema`,
plus `#[serde(rename_all = "lowercase")]`. `Copy` is included because
the enum has no payload-bearing variants and is cheap to copy at
match sites; `PartialEq` + `Eq` are required for the test assertions
(`outcome == IdempotencyOutcome::Unchanged`).

`IdempotencyOutcome` is a typed enum at the API layer; it never
carries a `Conflict` variant (conflict is an HTTP-status concern,
not an enumeration value — see ADR-0015 §4 amendment). The handler
(§3, `handlers.rs::submit_job`) maps `PutOutcome::Inserted` →
`IdempotencyOutcome::Inserted`, and the byte-equality check inside
the `KeyExists { existing }` arm decides between
`IdempotencyOutcome::Unchanged` (return 200) and 409 Conflict
(different spec at same key — no `outcome` field returned at all
on the 409 path).

*Rationale tag*: `drop-field` + new enum type.

---

## 4. OpenAPI schema

### `api/openapi.yaml`

*Current state*: the three component schemas (`SubmitJobResponse`,
`JobDescription`, `ClusterStatus`) carry `commit_index` properties
with descriptions that distinguish per-entry vs store-wide
semantics. The descriptions are several paragraphs long.

*New state*: regenerate from the new Rust types. The generated YAML
is the contract; the DELIVER crafter does not hand-edit
`openapi.yaml`. The expected diff:

- `SubmitJobResponse` schema: drop `commit_index` property, add
  `spec_digest` (`type: string`, formatted as a content hash) and
  `outcome` (`enum: [inserted, unchanged]`).
- `JobDescription` schema: drop `commit_index` property. `spec` and
  `spec_digest` properties unchanged.
- `ClusterStatus` schema: drop `commit_index` property. `mode`,
  `region`, `reconcilers`, `broker` properties unchanged.
- New component schema: `IdempotencyOutcome` (string enum with
  values `inserted` | `unchanged`).

The previous long description text explaining per-entry vs
store-wide semantics is gone (the field it described no longer
exists).

*Rationale tag*: `regenerate`.

*Notes*: regenerate via `cargo xtask openapi`, then verify
`cargo xtask openapi-check` passes. The regenerated YAML is
checked in; the DELIVER crafter does NOT hand-edit
`api/openapi.yaml`. The `cargo xtask openapi-check` gate will
fail on the first DELIVER step that lands the trait change
without the matching schema regeneration; this is the desired
behaviour (structural drift is caught at the gate). The DELIVER
lane regenerates the schema as part of the same PR.

---

## 5. CLI rendering

### `crates/overdrive-cli/src/render.rs`

*Current state* (verified by grep audit 2026-04-26): three "Commit
index:" lines in this file —
- **Line 32**: `let _ = writeln!(s, "Commit index:  {}", out.commit_index);` (in `job_submit` block).
- **Line 81**: `let _ = writeln!(s, "Commit index:  {}", out.commit_index);` (in `alloc_status` block).
- **Line 100**: `let _ = writeln!(s, "Commit index:  {}", out.commit_index);` (in `cluster_status` block).

*New state*:

- `render::job_submit` — line 32 deleted; replaced with a "Spec
  digest:" line plus an "Outcome:" line. The outcome enum renders
  as the human form `created` (for `inserted`) or `unchanged` (for
  `unchanged`); the CLI does not surface the raw lowercase JSON form
  to the operator.
- `render::alloc_status` — line 81 deleted. The "Spec digest" line
  (which already exists adjacent) is unchanged.
- `render::cluster_status` — line 100 deleted. The output narrows
  from five lines to four: `Mode: …`, `Region: …`, `Reconcilers: …`,
  `Broker: …`.
- `render::node_list` — grep audit 2026-04-26 found no "Commit index"
  line in any node-list rendering site. No change required here.

*Rationale tag*: `drop-field`.

### `crates/overdrive-cli/src/cli.rs`

*Current state* (verified by grep audit 2026-04-26):
- **Line 91**: rustdoc comment on a CLI subcommand reading `/// Read
  canonical \`spec_digest\` + \`commit_index\` for a job and the …`.

*New state*: line 91 rustdoc updated to drop the `\`commit_index\``
mention. The reading becomes `/// Read canonical \`spec_digest\` for
a job and the …` — the help-text reflects the new wire contract.

*Rationale tag*: `revise-doc` (rustdoc-only; help text auto-derived
by `clap`).

### `crates/overdrive-cli/src/commands/cluster.rs`

*Current state*: `ClusterStatusOutput` struct (or equivalent)
carries a `commit_index: u64` field that mirrors the API response
shape; the print logic reads this field.

*New state*: `ClusterStatusOutput` drops the `commit_index` field.
Mirrors the new `ClusterStatus` API shape — four fields:
`{mode, region, reconcilers, broker}`. The print logic is updated
to reflect the four-field shape.

*Rationale tag*: `drop-field`.

### `crates/overdrive-cli/src/commands/job.rs`

*Current state*: the submit handler prints the commit index from
the `SubmitJobResponse`; the describe handler prints the commit
index from the `JobDescription`.

*New state*: the submit handler prints the spec digest and outcome
from the new `SubmitJobResponse`; the describe handler prints only
the spec and spec digest from the new `JobDescription`. Any
shared-types crate surfaces (e.g. response struct mirrors per
ADR-0014) follow the new API field set.

*Rationale tag*: `drop-field`.

### `crates/overdrive-cli/src/commands/alloc.rs`

*Current state*: the alloc-status handler may print a commit index
line. Verify at edit time.

*New state*: drop any "Commit index" line. The spec-digest line is
unchanged.

*Rationale tag*: `drop-field`.

---

## 6. Tests — delete in full

The following test files exist solely to assert properties of
`commit_index`. They are deleted in full. Where the same test
binary contains tests for adjacent properties, only the
`commit_index`-asserting tests are deleted; the binary survives.

### `tests/integration/commit_counter_invariant.rs`

*Current state*: integration test asserting the commit counter
invariant under concurrent writers.

*New state*: deleted.

*Rationale tag*: `delete-file`.

### `tests/acceptance/per_entry_commit_index.rs`

*Current state*: acceptance test asserting per-entry index
assignment.

*New state*: deleted.

*Rationale tag*: `delete-file`.

### `tests/integration/per_entry_commit_index.rs`

*Current state*: integration test mirroring the acceptance one
above.

*New state*: deleted.

*Rationale tag*: `delete-file`.

### `tests/acceptance/commit_index_monotonic.rs`

*Current state*: asserts the commit index is strictly monotonic
across submits.

*New state*: deleted.

*Rationale tag*: `delete-file`.

*Notes*: the monotonic property has no consumer; ADR-0020 *Decision*
§5 documents why preserving it is wrong shape for Phase 1.

---

## 7. Tests — revise

### `tests/acceptance/phantom_writes.rs`

*Current state*: asserts both
(a) no watch event is emitted on a no-op write, and
(b) the commit counter is not bumped on a no-op write.

*New state*: assertion (a) is preserved verbatim; assertion (b) is
deleted (there is no counter to assert against). The test name
likely changes from `phantom_writes_dont_bump_counter` (or similar)
to `phantom_writes_dont_emit_events`.

*Rationale tag*: `revise-test`.

### `tests/acceptance/snapshot_roundtrip.rs`

*Current state*: asserts the v2 frame round-trips key + value +
per-entry index.

*New state*: asserts the v1 (or v2-without-index, per §2 architect
adjudication) frame round-trips key + value. The per-entry index
column assertion is dropped.

*Rationale tag*: `revise-test`.

### `tests/acceptance/put_if_absent.rs`

*Current state*: asserts the `PutOutcome` variant *and* the
commit index value on each variant.

*New state*: asserts the `PutOutcome` variant (`Inserted` vs
`KeyExists { existing }`) only. Variant assertions are preserved;
index assertions are deleted.

*Rationale tag*: `revise-test`.

### `tests/acceptance/local_store_basic_ops.rs`

*Current state*: destructures `(bytes, idx) = store.get(...).unwrap();`.

*New state*: takes `bytes = store.get(...).unwrap();` directly. No
tuple unpack.

*Rationale tag*: `revise-test`.

### `tests/acceptance/local_store_edges.rs`

*Current state*: same tuple-unpack pattern as
`local_store_basic_ops.rs`.

*New state*: same revision — drop the tuple unpack.

*Rationale tag*: `revise-test`.

### `tests/integration/snapshot_proptest.rs`

*Current state*: proptest generates `Vec<(Vec<u8>, Vec<u8>, u64)>`
frames; assertion includes the per-entry index column round-trip.

*New state*: proptest generates `Vec<(Vec<u8>, Vec<u8>)>` frames;
the per-entry index column is dropped from the generator and from
the assertion.

*Rationale tag*: `revise-test`.

### `tests/integration/concurrent_submit_toctou.rs`

*Current state*: verify whether this test asserts on `commit_index`.
The briefing flags it as needing an audit.

*New state*: if the test asserts on `commit_index`, drop the
assertion; the TOCTOU property the test is defending (idempotency
under concurrent submits of the same spec) is preserved by the
`outcome == "unchanged"` assertion plus the back-door byte-equality
check. If the test does not assert on `commit_index` at all, no
revision needed.

*Rationale tag*: `revise-test`.

### `tests/integration/idempotent_resubmit.rs`

*Current state*: asserts `commit_index` equality across N re-submits
of the byte-identical spec.

*New state*: asserts `outcome == "unchanged"` on every re-submit
after the first, plus `spec_digest` equality across all submits.
The pre-existing back-door byte-equality assertion (per ADR-0020
*Decision* §2 reference to
`byte_identical_resubmit_returns_original_commit_index_unchanged`)
remains; the test is renamed to drop the `commit_index` reference
in the test name (e.g.
`byte_identical_resubmit_returns_outcome_unchanged_and_same_digest`).

*Rationale tag*: `revise-test`.

### `tests/integration/submit_round_trip.rs`

*Current state*: asserts the submit returns a `commit_index` ≥ 1.

*New state*: asserts the submit response carries `outcome ==
"inserted"` and a `spec_digest` equal to the locally-computable
digest of the rkyv archive. The "≥ 1" assertion is dropped (no
counter exists; the per-write witness is the digest).

*Rationale tag*: `revise-test`.

### `tests/integration/describe_round_trip.rs`

*Current state*: asserts the describe response carries the same
`commit_index` the submit returned.

*New state*: asserts the describe response carries the same
`spec_digest` the submit returned. The `commit_index` assertion is
dropped.

*Rationale tag*: `revise-test`.

### `tests/acceptance/submit_job_idempotency.rs`

*Current state*: asserts `commit_index` equality across re-submits.

*New state*: asserts `outcome == "unchanged"` on the second/third
submit plus `spec_digest` equality across all submits. Same
behavioural property; clearer assertion shape.

*Rationale tag*: `revise-test`.

### `tests/integration/walking_skeleton.rs` (and `tests/acceptance/walking_skeleton.rs` if both exist)

*Current state*: asserts on the WS-1, WS-2, WS-3 scenarios as
written in `distill/test-scenarios.md` §1.1, §1.2, §1.3 — including
`commit_index` Then-lines.

*New state*: revise per the test-scenarios amendment block. WS-1
asserts on spec-digest round-trip plus `outcome == "inserted"`.
WS-2 asserts on the four-field cluster-status output (no Commit-
index line) plus `broker.dispatched > 0` after a tick. WS-3 asserts
on `outcome == "unchanged"` plus digest equality across re-submits.

*Rationale tag*: `revise-test`.

### CLI integration tests (output captures) — enumeration

*Current state* (verified by grep audit 2026-04-26): the following
CLI test files reference `commit_index` directly. Each entry below
specifies the assertion shape change and the lines to revise:

#### `crates/overdrive-cli/tests/integration/http_client.rs`

- **Line 141**: `assert!(submit_resp.commit_index > 0, "commit_index must be > 0");` — drop assertion; replace with `assert_eq!(submit_resp.outcome, IdempotencyOutcome::Inserted);` plus a digest-equality assertion against the locally-computed digest.
- **Line 146**: `assert_eq!(description.commit_index, submit_resp.commit_index);` — drop assertion; replace with `assert_eq!(description.spec_digest, submit_resp.spec_digest);`.

*Rationale tag*: `revise-test`.

#### `crates/overdrive-cli/tests/integration/job_submit.rs`

- **Lines 10, 15**: rustdoc references in the file header (`commit_index`, `commit_index >= 1`) — revise to reference `spec_digest` and `outcome` instead.
- **Lines 129-131**: `assert!(output.commit_index >= 1, …);` — drop assertion; replace with `assert_eq!(output.outcome, IdempotencyOutcome::Inserted)` plus digest equality assertion.

*Rationale tag*: `revise-test`.

#### `crates/overdrive-cli/tests/integration/endpoint_from_config.rs`

- **Lines 94-96**: `assert!(output.commit_index >= 1, …);` — drop assertion; same replacement as `job_submit.rs` above.

*Rationale tag*: `revise-test`.

#### `crates/overdrive-cli/tests/integration/cluster_and_node_commands.rs`

- **Line 12**: rustdoc reference in file header noting `ClusterStatusOutput` carries `commit_index`. Revise to drop the `commit_index` mention; the four-field shape (`mode`, `region`, `reconcilers`, `broker`) is the new contract. (No assertion-level test code currently references `commit_index` in this file — the change is rustdoc-only.)

*Rationale tag*: `revise-doc`.

#### `crates/overdrive-cli/tests/acceptance/render_job_submit.rs`

- **Lines 12, 28, 43, 59**: test fixture and assertion sites. Specifically:
  - Line 12: rustdoc enumerates the rendered labels including `Commit index:`. Revise to enumerate the new label set: `Accepted.`, `Job ID:`, `Intent key:`, `Spec digest:`, `Outcome:`, `Endpoint:`, `Next:`.
  - Line 28: test-fixture struct literal carries `commit_index: 17`. Drop the field; the fixture now carries `spec_digest` and `outcome` instead.
  - Line 43: assertion-loop label list includes `"Commit index:"`. Drop; replace with `"Spec digest:"` and `"Outcome:"`.
  - Line 59: assertion `"rendered block must contain commit_index 17; got:\n{rendered}"`. Drop; replace with assertions on the new label values.

*Rationale tag*: `revise-test`.

#### `crates/overdrive-cli/tests/acceptance/render_alloc_status.rs`

- **Line 12**: rustdoc references `spec_digest` + `commit_index`. Revise to drop `commit_index`.
- **Lines 20, 32, 110, 143**: four test-fixture struct literals carrying `commit_index: <N>`. Drop the field from each fixture.

*Rationale tag*: `revise-test`.

#### `crates/overdrive-cli/tests/acceptance/render_cluster_and_node.rs`

- **Lines 12, 28, 44, 62**: cluster-status rendering test. Specifically:
  - Line 12: rustdoc enumerates the rendered labels including `Commit index:`. Revise to drop.
  - Line 28: test-fixture struct literal `commit_index: 42`. Drop the field.
  - Line 44: label list iteration `["Mode:", "Region:", "Commit index:", "Reconcilers:", "Broker counters:"]`. Drop `"Commit index:"`; the four-label list becomes the contract.
  - Line 62: assertion `"rendered cluster-status must contain commit_index value; got:\n{rendered}"`. Drop; replace with the four-field assertion shape.

*Rationale tag*: `revise-test`.

#### `crates/overdrive-cli/tests/integration/walking_skeleton.rs`

- **Lines 120-122**: `assert!(submit_output.commit_index >= 1, …);` on the WS-1 step. Drop; replace with `outcome == IdempotencyOutcome::Inserted` + digest equality.
- **Lines 135-137**: `assert!(status_output.commit_index >= 1, …);` on the WS-2 step. Drop; the WS-2 wiring witness becomes `broker.dispatched > 0` per the test-scenarios amendment in `distill/test-scenarios.md`.

*Rationale tag*: `revise-test`. (This file is also referenced in
the §7 *walking_skeleton.rs* entry above; this is the same file
with the line-level enumeration added.)

#### `crates/overdrive-control-plane/tests/acceptance/runtime_registers_noop_heartbeat.rs`

- **Line 202**: `assert_eq!(body.commit_index, state.store.commit_index(), "commit_index from store");`. Drop the assertion; the store accessor `commit_index()` is deleted in the impl edit (§2). The test asserts on the four-field `ClusterStatus` shape instead — the noop-heartbeat property the test defends is "the runtime registers and the heartbeat reconciler advances `broker.dispatched`," which is preserved on the new shape.

*Rationale tag*: `revise-test`.

#### `crates/overdrive-control-plane/tests/acceptance/api_type_shapes.rs`

- **Lines 43, 48**: `SubmitJobResponse { job_id: "payments".to_string(), commit_index: 42 }` and round-tripping assertion `assert_eq!(round_tripped.commit_index, 42);`. Drop the field from the fixture; replace the round-trip assertion with `assert_eq!(round_tripped.spec_digest, …);` + `assert_eq!(round_tripped.outcome, IdempotencyOutcome::Inserted);`.
- **Lines 55, 62**: `JobDescription { …, commit_index: 7, … }` fixture and assertion. Drop the field; the round-trip assertion becomes `spec_digest` equality only.
- **Lines 71, 80**: `ClusterStatus { …, commit_index: 11, … }` fixture and assertion. Drop the field; the round-trip asserts on the four-field shape.

*Rationale tag*: `revise-test`.

#### Catch-all grep audit

After the per-file enumeration above is applied, the DELIVER PR
runs a final `grep -rln 'commit_index\|Commit index' crates/ tests/
api/ | grep -v target/ | grep -v evolution/` audit. The expected
result is zero matches outside `docs/`. Any remaining match is
either (a) a missed file the enumeration above did not catch, in
which case the per-file shape applies (drop the assertion / drop
the field / drop the rustdoc reference), or (b) a docs/ surface
that is correctly preserved as historical record.

The catch-all audit is the safety net, not a substitute for the
per-file enumeration. The peer review of upstream-changes.md
(2026-04-26) flagged the previous "the DELIVER crafter greps
`tests/` for `commit_index` and `Commit index`" framing as a
gesture rather than an enumeration — the per-file listing above
closes that gap.

---

## 8. ADRs

### `docs/product/architecture/adr-0008-rest-openapi-transport.md`

*Current state*: §endpoint table lists the three responses
(`SubmitJobResponse`, `JobDescription`, `ClusterStatus`) with
`commit_index` fields.

*New state*: amendment block at the head of the file (matching the
project's existing amendment-block style), revising the endpoint
table:
- `POST /v1/jobs` returns `{job_id, spec_digest, outcome}`.
- `GET /v1/jobs/{id}` returns `{spec, spec_digest}`.
- `GET /v1/cluster/info` returns `{mode, region, reconcilers, broker}`.

The amendment block cites ADR-0020 as the source.

*Rationale tag*: `revise-adr`.

### `docs/product/architecture/adr-0015-http-error-mapping.md`

*Current state*: §4 status-code matrix's idempotent-success row
reads "200 OK, same commit_index as the original."

*New state*: amendment block at the head, revising §4 idempotent-
success row to read "200 OK, same `spec_digest` as the original,
with `outcome: IdempotencyOutcome::Unchanged`." The 409 Conflict
row is unaffected. The amendment block cites ADR-0020 as the
source.

*Rationale tag*: `revise-adr`.

### ADRs that are NOT amended

- `docs/product/architecture/adr-0013-reconciler-primitive-runtime.md`
  — verified zero `commit_index` references; no amendment needed.
- `docs/product/architecture/adr-0014-CLI-HTTP-client-and-shared-types.md`
  — verified zero `commit_index` references; no amendment needed.
  (The CLI's commit-index rendering is implementation in
  `commands/cluster.rs`, not an ADR-level concern.)

The briefing's surgery surface table claimed both ADR-0013 and
ADR-0014 reference `commit_index`. Direct grep audit (during the
DESIGN run) confirmed neither does; both are unaffected by this
ADR.

*Rationale tag*: `revise-adr` (negative — no edit needed; this entry
documents the verified non-amendment).

---

## 9. Architecture brief

### `docs/product/architecture/brief.md`

*Current state*: three references to `commit_index` (per the
briefing's audit).

*New state*: each reference is revised inline. Where the brief
discusses the per-entry idempotency contract, the reference is
replaced with `spec_digest` + `IdempotencyOutcome`. Where the brief
discusses cluster-status fields, the reference is removed (the
field is dropped with no replacement). The brief carries no
strikethrough markup — it is the architecture SSOT, not an
amendable artifact; the references are deleted/replaced cleanly.

*Rationale tag*: `revise-doc`.

*Notes*: this is the only doc surface where strikethrough markup is
NOT used. The brief is rewritten cleanly to reflect the post-ADR
architecture; the project memory `feedback_delegate_to_architect`
applies (the architect agent owns the brief's edits).

---

## 10. Feature documents (already amended in this DESIGN run)

These four documents were amended in this DESIGN run with
`> **Amendment 2026-04-26.**` blocks and `~~strikethrough~~` markup
on the original prose. They are listed here for completeness so the
DELIVER crafter knows not to re-amend them.

- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
  — Amendment block + revisions to US-03, US-04, US-05.
- `docs/feature/phase-1-control-plane-core/distill/test-scenarios.md`
  — Amendment block + revisions to §1.1, §1.2, §1.3, §3.1, §4.5
  (deleted), §4.6 (deleted), §4.9, §6.1, §6.2; adapter coverage
  table revised; scenario count table revised.
- `docs/feature/phase-1-control-plane-core/distill/walking-skeleton.md`
  — Amendment block + revisions to demo script step 2 and step 5,
  WS-1, WS-2, WS-3, traceability footer.
- `docs/product/architecture/adr-0020-drop-commit-index-phase-1.md`
  — Authored in this DESIGN run; status Proposed; promoted to
  Accepted on DELIVER landing.

---

## 11. Out of scope (reference only)

The following surfaces reference `commit_index` historically and are
NOT edited by the DELIVER PR:

- `docs/evolution/2026-04-25-fix-commit-index-per-entry.md` —
  historical record of the bug-cascade first fix. Reference-only.
- `docs/evolution/2026-04-25-fix-commit-counter-and-watch-doc.md` —
  historical record of the bug-cascade second fix. Reference-only.
- `docs/evolution/2026-04-26-fix-commit-counter-and-watch-doc.md` —
  historical record of the bug-cascade third fix (the RCA that
  surfaced the cross-writer race). Reference-only.
- `docs/feature/phase-1-control-plane-core/deliver/roadmap.json` —
  frozen feature roadmap; reference-only.
- `docs/feature/fix-commit-index-per-entry/deliver/roadmap.json` —
  the feature being undone; reference-only.

These documents capture the historical decision the new ADR
supersedes. They are not amended; the new ADR's *References*
section cites them.

---

## 12. Verification gates the DELIVER PR must pass

These gates exist in CI today; they will fail mid-PR if the surgery
is incomplete. The crafter runs them as part of the standard DELIVER
flow:

- `cargo nextest run --workspace` — every test in §6 must be
  deleted and every test in §7 revised before this gate passes.
- `cargo test --doc --workspace` — any rustdoc example referencing
  `commit_index` must be updated.
- `cargo xtask openapi-check` — the regenerated YAML must match
  what the new Rust types produce; any drift fails this gate.
- `cargo xtask dst` — the harness invariant
  `intent_store_returns_caller_bytes` (per ADR-0020 *Enforcement*)
  catches a regression of the inline row encoding. If this
  invariant is not yet present, it lands as part of the DELIVER PR.
- `cargo xtask mutants --diff origin/main` — kill-rate ≥ 80% per
  the workspace policy.
- A `grep -r commit_index crates/ tests/ api/` audit at the end of
  the PR must show zero matches outside `docs/evolution/` and
  `docs/feature/.../deliver/roadmap.json`.

The grep audit is the definitive completeness check. Anything it
flags is either (a) a production code site missed in the surgery
or (b) a documentation site correctly preserved as historical
reference. Each match is justified explicitly in the PR
description.

---

## Review (2026-04-26)

Independent peer review of `upstream-changes.md`. Verdict:
**NEEDS_REVISION**. Four critical/blocking issues + two high-severity
issues + three medium-severity issues + one low-severity issue
raised. The reviewer found no architectural problems with the
file's strategy; the gaps are documentation precision: deferred
decisions that wave-decisions and ADR-0020 already settled,
incomplete file enumerations, missing line citations, and one
file path that does not exist on disk.

### Findings

**Blocking U1 (CRITICAL) — Snapshot frame v2/v1 decision deferred.**
The §2 entry for `snapshot_frame.rs` framed the v2-vs-v1 choice as
"architect adjudication required at DELIVER time" with two options
(a) and (b). But wave-decisions.md §Decision §3 and ADR-0020
§Decision §3 both already chose "revert v2 → v1" — the deferred
framing risks the DELIVER crafter making a different choice than
the design records. **Fix recommendation**: replace the deferred
framing with a single unambiguous statement: "Revert to v1: encoder
emits `Vec<(Vec<u8>, Vec<u8>)>` under schema discriminator v1. The
v1 decoder already exists per the module docstring; it is unchanged.
The v2 frame definition and encoder are deleted entirely. Snapshot
proptests in `tests/integration/snapshot_proptest.rs` revise to drop
the index column from the property generators."

**Blocking U2 (CRITICAL) — `IdempotencyOutcome` enum location ambiguous.**
The §3 `api.rs` entry framed the location of the new
`IdempotencyOutcome` enum as "crafter choice; both are defensible."
This is a documentation gap — the DELIVER crafter must not face an
ambiguous design. **Fix recommendation**: decide now. Define in
`crates/overdrive-control-plane/src/api.rs` colocated with the other
response types. Specify the derives explicitly:
`#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]`,
plus `#[serde(rename_all = "lowercase")]`. Wire shape: `"inserted"`
| `"unchanged"`. Update §3 (handlers.rs) and §1 (api.rs) entries to
reference this canonical definition.

**Blocking U3 (CRITICAL) — Test file enumeration incomplete.**
Grep audit shows 37 production files reference `commit_index`; the
upstream-changes file enumerated ~15. The reviewer specifically
flagged the following missing CLI test files:
- `crates/overdrive-cli/tests/integration/http_client.rs`
- `crates/overdrive-cli/tests/integration/job_submit.rs`
- `crates/overdrive-cli/tests/integration/endpoint_from_config.rs`
- `crates/overdrive-cli/tests/acceptance/render_job_submit.rs` (and adjacent render tests)
- `crates/overdrive-cli/tests/acceptance/render_alloc_status.rs`
- `crates/overdrive-cli/tests/acceptance/render_cluster_and_node.rs`
- `crates/overdrive-control-plane/tests/acceptance/runtime_registers_noop_heartbeat.rs`
- `crates/overdrive-cli/src/cli.rs` (CLI source — help-text references)

**Fix recommendation**: enumerate each file explicitly with line
numbers, the assertion-shape change, and the rationale tag. The
catch-all grep audit at §12 is the safety net, not a substitute for
enumeration.

**Blocking U4 (CRITICAL) — Sim adapter path verification.**
The §2 entry for `crates/overdrive-sim/src/adapters/intent_store.rs`
deferred path verification ("verify the adapter file path at edit
time"). On-disk audit shows the file does not exist — the sim crate
has no `IntentStore` adapter; the DST harness reuses the real
`LocalIntentStore` from `overdrive-store-local` on a per-host
tempdir. **Fix recommendation**: state the actual path situation.
Specify the change shape: drop `commit_index` references
downstream-of-trait throughout the sim crate; the entry is
"downstream consequence of the trait edit," not a standalone sim
adapter rewrite.

**High U5 — Trait surface line numbers missing.**
The §1 entry for `intent_store.rs` listed the change shape but
without line citations. **Fix recommendation**: add explicit line
citations:
- Line 193: `async fn get(...)` return type changes from
  `Option<(Bytes, u64)>` to `Option<Bytes>`.
- Lines 99-104: drop `commit_index: u64` field from
  `PutOutcome::Inserted`.
- Lines 112-118: drop `commit_index: u64` field from
  `PutOutcome::KeyExists`.
Also enumerate the docstring lines that need to be revised or
deleted (lines 69-92 carry the per-entry vs store-wide distinction
explanation that becomes vacuous post-drop).

**High U6 — `debug_assert_eq!` deletion sites in `redb_backend.rs`.**
The §2 entry referenced "Four `debug_assert_eq!` calls cross-checking
peek and bump" without grep verification. **Fix recommendation**:
add note: "Grep `redb_backend.rs` for `debug_assert_eq!`; delete
every match. There are exactly 3 in the file as of this DESIGN run
(lines 312, 392, 525 — verified by grep audit 2026-04-26). Each
asserts peek/bump alignment; post-deletion there are zero
`debug_assert_eq!` calls in the file." (The reviewer's prompt
suggested 4 calls plus "one verified by re-grep"; live grep audit
shows 3.)

**Medium — `ClusterStatus` pure-drop self-justification.**
The §3 handlers entry for `cluster_status` removed the field with no
inline rationale, requiring cross-reference to wave-decisions.md to
justify the choice. **Fix recommendation**: add the structural
rationale ("renaming the same race") inline at §3 cluster_status
entry so the entry is self-justifying.

**Medium — Snapshot proptest revision detail.**
The §7 entry for `snapshot_proptest.rs` was thin on which file,
which functions, what the generator currently produces vs what it
should produce. **Fix recommendation**: now folded into U1 (revert
to v1; proptests drop the index column from the property generator).

**Medium — OpenAPI schema regeneration command.**
The §4 entry for `openapi.yaml` did not name the regeneration
command. **Fix recommendation**: add: "Regenerate via
`cargo xtask openapi`, then verify `cargo xtask openapi-check`
passes. The regenerated YAML is checked in; do not hand-edit."

**Low — CLI help-text cleanup.**
`crates/overdrive-cli/src/cli.rs` line 91 carries a rustdoc reference
to `commit_index` that becomes vacuous post-drop. Now folded into U3
(`crates/overdrive-cli/src/cli.rs` enumeration).

### Resolution (2026-04-26)

| Issue | Status | Resolution |
|---|---|---|
| **U1** (snapshot frame v2/v1 decision) | Resolved | §2 `snapshot_frame.rs` entry rewritten. The deferred (a)/(b) framing is replaced with the locked decision: revert to v1, emit `Vec<(Vec<u8>, Vec<u8>)>`, delete v2 encoder/decoder branch. The constant `pub const VERSION: u16` becomes 1; the v1 decoder is preserved verbatim. Cross-references wave-decisions §Decision §3 and ADR-0020 §Decision §3 inline. The "deferred framing is removed" note documents the correction. |
| **U2** (`IdempotencyOutcome` location) | Resolved | §3 `api.rs` entry rewritten. `IdempotencyOutcome` is canonically defined in `crates/overdrive-control-plane/src/api.rs` colocated with the other API response types. The full `#[derive(...)]` set is locked: `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema` plus `#[serde(rename_all = "lowercase")]`. Wire shape `"inserted"` | `"unchanged"`. Inline rustdoc on the type body is also specified. The "crafter choice" framing is removed. The §3 handlers entry references the canonical api.rs definition. |
| **U3** (test file enumeration) | Resolved | §7 *CLI integration tests* section expanded from a single gesture entry to a per-file enumeration covering all 8 missing CLI files plus two missing control-plane test files. Each entry cites line numbers from grep audit 2026-04-26 and specifies the assertion-shape change. Added: `http_client.rs` (lines 141, 146), `job_submit.rs` (lines 10, 15, 129-131), `endpoint_from_config.rs` (lines 94-96), `cluster_and_node_commands.rs` (line 12 rustdoc), `render_job_submit.rs` (lines 12, 28, 43, 59), `render_alloc_status.rs` (lines 12, 20, 32, 110, 143), `render_cluster_and_node.rs` (lines 12, 28, 44, 62), `walking_skeleton.rs` CLI (lines 120-122, 135-137), `runtime_registers_noop_heartbeat.rs` (line 202), `api_type_shapes.rs` (lines 43, 48, 55, 62, 71, 80). The catch-all grep audit at §12 remains as the safety net. |
| **U4** (sim adapter path) | Resolved | §2 sim entry rewritten. The previous entry quoted a path (`crates/overdrive-sim/src/adapters/intent_store.rs`) that does not exist on disk — verified by ls audit of `crates/overdrive-sim/src/adapters/` (which contains clock, dataplane, driver, entropy, llm, transport, observation_store — but no `intent_store`). The DST harness at `crates/overdrive-sim/src/harness.rs` reuses the real `LocalIntentStore`. The new entry explains the architectural reality and specifies the sim-side change shape as "downstream consequence of the trait edit": grep `crates/overdrive-sim/` for `commit_index` / `(Bytes, u64)` / `PutOutcome::Inserted { commit_index }` / `PutOutcome::KeyExists { commit_index, .. }` and apply the same mechanical edits as the production-side equivalent. |
| **U5** (trait surface line numbers) | Resolved | §1 entry for `intent_store.rs` now cites all line numbers from grep audit 2026-04-26. Line 193 (`get` return type), lines 99-104 (`Inserted` field), lines 112-118 (`KeyExists` field), lines 69-92 (type-level docstring), lines 178-192 (`get` docstring), lines 217-228 (`watch` docstring). Each line is paired with the specific edit shape (change return type, drop field, rewrite docstring, etc.). |
| **U6** (`debug_assert_eq!` count) | Resolved | §2 entry now cites the exact count from grep audit: "There are exactly 3 `debug_assert_eq!` calls in `redb_backend.rs` at lines 312, 392, and 525." The reviewer's prompt suggested 4 plus "one verified by re-grep"; live grep audit shows 3. The entry instructs the DELIVER crafter to re-grep at edit time and delete every match, with the rule "delete every match" surviving any future drift. |
| **Medium** (`ClusterStatus` self-justifying) | Resolved | §3 handlers entry for `cluster_status` now carries the structural rationale inline: the in-memory counter, bump call site, and cross-writer race window all survive verbatim under any rename; pure-drop removes the surface that generates the class; activity-rate signals belong in Phase 5's metrics endpoint. Cross-references ADR-0020 §Considered alternatives §D for the full structural argument. |
| **Medium** (snapshot proptest detail) | Resolved | Folded into U1. The §7 `snapshot_proptest.rs` entry already specifies the generator change (drop the index column); U1's revert-to-v1 framing makes the proptest revision shape explicit. |
| **Medium** (OpenAPI regeneration command) | Resolved | §4 `openapi.yaml` entry now states explicitly: "Regenerate via `cargo xtask openapi`, then verify `cargo xtask openapi-check` passes. The regenerated YAML is checked in; the DELIVER crafter does NOT hand-edit `api/openapi.yaml`." |
| **Low** (CLI help-text cleanup) | Resolved | Folded into U3. New §5 entry for `crates/overdrive-cli/src/cli.rs` cites line 91 explicitly with the rustdoc revision. |

No deferred issues. All review findings addressed in this revision pass.
