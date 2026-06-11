# DESIGN — make the duplicate `IssuanceOrdinal` unrepresentable

**Architect:** Morgan (Solution Architect, DESIGN wave)
**Date:** 2026-06-11
**Mode:** Propose (autonomous analysis; fix direction "Option B" pre-chosen by user)
**Input:** `docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md` (validated RCA — every supplied data point confirmed against source).
**Scope:** PIN the API surface for the chosen fix. NOT implementation.

This amends a shipped design (ADR-0063 D6). It does **not** introduce a new
architectural style, store, or pattern — it adds **one** `ObservationStore`
port method and rewires a single call site. The whole exercise is governed by
`.claude/rules/development.md` § "Check-and-act must be atomic (no TOCTOU)"
→ *type the racy surface away / make the collision unrepresentable*.

---

## 1. The defect, in one line

`ca_issuance.rs:209-210` stamps the audit row's `IssuanceOrdinal` from
`observation.issued_certificate_rows().await?.len()` — a read-len → mint →
write-row TOCTOU whose uniqueness depends on two *ambient* invariants
(serialized dispatch + append-only) that live only in rustdoc. Two concurrent
`issue_and_audit` calls for one store both read `len()==N`, both stamp ordinal
`N`; the consumer's `max_by_key(issuance_ordinal)` (`handlers.rs:1046`) then
resolves the tie by CSPRNG-serial store order → a stale serial renders as
"current". The collision is **representable**; the fix makes it
**unrepresentable** by sourcing the ordinal from a durable, atomically-allocated
monotonic counter instead of `len()`.

---

## 2. Candidate evaluation — B1 (separate atomic allocation) vs B2 (store-assigns-at-write)

Both candidates dissolve the TOCTOU. They differ in blast radius and in how much
of the existing typed-row / codec contract they disturb.

### B1 — separate atomic allocation (a `next_issuance_ordinal()` port method)

- **Shape.** A new `ObservationStore` method atomically allocates-and-persists
  the next ordinal from a durable monotonic sequence and returns it. `ca_issuance.rs`
  calls it in place of the `len()` read; the row's `issuance_ordinal` field stays
  **caller-set** from the returned value. Construction of `IssuedCertificateRow`
  is unchanged.
- **Blast radius.** Minimal. One new trait method + two adapter impls + one
  call-site swap. **No** change to `IssuedCertificateRow`, its struct-literal
  construction, the rkyv `V1` payload, the envelope, the golden-bytes fixture,
  or the append-only serial guard. The schema-evolution surface is untouched.
- **Gap semantics (stated, accepted).** Allocation and the subsequent row write
  are two operations. If the mint or the row write fails *after* the ordinal is
  allocated, that ordinal number is **burned** (allocated but never written to a
  row) — the sequence is left with a hole. This is **fine**: the consumer
  projection needs strict *ordering*, not *density*. A burned ordinal never
  collides (the counter never re-issues it), and `max_by_key` over a
  gap-containing-but-strictly-increasing set is still recency-correct. The
  contract states this explicitly so a future reader does not mistake the gap
  for a bug.
- **TOCTOU posture.** The allocate-and-act is atomic *within* the counter
  (the counter's read-increment-write is one transaction). The *row* write is a
  separate transaction, but it no longer participates in any check-then-act on
  the ordinal — the ordinal is already decided, uniquely, before the row exists.
  The only thing the two steps share is the *value*, which is immutable once
  allocated. No window remains.

### B2 — store-assigns-the-ordinal-at-write

- **Shape.** The store stamps `issuance_ordinal` *inside* the
  `IssuedCertificate` write transaction; the caller no longer computes or sets it.
- **Blast radius — materially larger, and it fights the existing contract:**
  1. **`IssuedCertificateRow` construction contract changes.** Today the row is
     built by struct literal with `issuance_ordinal: ordinal` set by the caller
     (`ca_issuance.rs:240-249`). B2 means the caller must construct the row
     *without* an ordinal — but `issuance_ordinal` is a non-`Option` field on the
     `V1` payload. Making it caller-omittable means either (a) an `Option<IssuanceOrdinal>`
     field that the store fills in (re-introducing a "not-yet-assigned" sentinel
     state — the exact anti-pattern `development.md` § "Sum types over sentinels"
     forbids, and a representable-invalid-state the rkyv envelope would have to
     carry), or (b) a separate "row-without-ordinal" input type the store widens
     into the persisted row — a second type for one field.
  2. **The codec/persistence boundary changes.** `IssuedCertificateRow::archive_for_store`
     (`development.md` § "rkyv schema evolution") is the single canonical-bytes
     site, co-located with `spec_digest`-style hashing. B2 makes the store mutate
     the row's `issuance_ordinal` *after* the caller built it but *before* (or as
     part of) archiving — the canonical bytes are now produced inside the write
     txn from a store-mutated value, not from the caller's value. That splits the
     "one source-of-truth byte sequence" the rule depends on.
  3. **The append-only guard interacts.** `apply_issued_certificate`
     (`observation_backend.rs:1089`) keys on serial and is a pure
     insert-if-absent. B2 would have it *also* read-increment-write a counter and
     mutate the incoming row — coupling the serial-dedup guard to ordinal
     allocation, two concerns in one transaction arm.
  4. **The generic `write(ObservationRow)` surface is wrong-shaped for it.**
     `write` takes a fully-formed `ObservationRow` and returns `()`. B2 needs the
     store to *return* the assigned ordinal (the caller may need it, e.g. for the
     held `SvidMaterial` or future observability), which `write`'s signature does
     not carry — so B2 *also* needs a new method or a changed return type. It does
     not actually save the surface change B1 spends.

### Decision — **B1 is pinned.**

B1 is the smaller, more honest change. It adds exactly one port method, leaves
the typed-row / codec / envelope / golden-bytes / append-only contracts entirely
intact, and keeps the ordinal a caller-set field (no sentinel, no second
"row-without-ordinal" type). B2's claim to be "more unrepresentable" is illusory:
it would still need a method to return the assigned value, *and* it would
introduce a not-yet-assigned representable state and split the canonical-bytes
codec — trading a real, contained surface addition for a diffuse contract
disturbance. **The collision is made unrepresentable in B1 by the counter's
atomicity, not by where the field is set.**

---

## 3. PINNED SURFACE SPEC

> The crafter is **forbidden** from improvising any of the following
> (CLAUDE.md § "Implement to the design — never invent API surface"). The exact
> signature, the trait-doc contract clauses, the two adapter behaviours, and the
> call-site shape are pinned here. A primitive not named here is **not** to be
> added on the crafter's initiative — surface a blocker instead.

### 3.1 New `ObservationStore` port method (`overdrive-core/src/traits/observation_store.rs`)

Add ONE method to the `ObservationStore` trait. No other method signature
changes.

```rust
/// Atomically allocate the next global issuance ordinal from a durable,
/// strictly-monotonic counter, and return it.
///
/// This is the SSOT for the `IssuanceOrdinal` stamped on every
/// `issued_certificates` audit row (ADR-0063 D6, rev 8). It REPLACES the
/// former `issued_certificate_rows().await?.len()` derivation
/// (`ca_issuance.rs`, pre-rev-8), which was a check-then-act TOCTOU
/// (`.claude/rules/development.md` § "Check-and-act must be atomic"): two
/// concurrent issuances read the same `len()` and stamped DUPLICATE
/// ordinals. Drawing from an atomically-incremented durable counter makes
/// that collision UNREPRESENTABLE — two concurrent callers receive two
/// distinct, strictly-increasing values.
///
/// # Preconditions
///
/// None. The method is safe to call concurrently for the same store from
/// any number of callers — that is the whole point. It requires no
/// serialization, no single-writer discipline, and no append-only
/// invariant on any other table.
///
/// # Postconditions
///
/// On `Ok(n)`:
/// * `n` is strictly greater than every ordinal this method has previously
///   returned for this store, INCLUDING across process restart (the counter
///   is durable). The first call on a fresh store returns the initial
///   value (see "Edge cases").
/// * The counter has been durably advanced past `n` BEFORE this method
///   returns `Ok(n)` — a crash immediately after return cannot re-issue
///   `n` to a later caller.
/// * No `issued_certificates` row is read or written by this call. The
///   ordinal is allocated independently of the audit table's contents; it
///   is NOT a function of `issued_certificate_rows().len()`.
///
/// # Edge cases
///
/// * **Fresh store.** The first call on a never-before-allocated store
///   returns the initial ordinal value. The initial value is pinned at
///   `IssuanceOrdinal::new(0)` to match the pre-fix `len()`-of-empty-table
///   semantics (an empty audit log derived ordinal 0 for its first row);
///   this keeps existing consumers' "ordinals start at 0" expectation and
///   the golden-bytes V1 fixtures valid. Greenfield single-cut migration
///   (`feedback_single_cut_greenfield_migrations.md`): a fresh store starts
///   the counter at 0; "delete the redb file" is the upgrade path. No
///   migration code reconstructs a counter from a pre-fix store.
/// * **Allocated-but-unused ordinal (gap semantics).** This method commits
///   the counter advance unconditionally on `Ok`. If the caller's
///   subsequent mint or audit-row write fails, the allocated ordinal is
///   BURNED — the sequence is left with a hole. This is by design and
///   CORRECT: the consumer projection
///   (`handlers::issued_certificates_for_rows`) maxes over the ordinal for
///   strict ORDERING, not density. A burned ordinal never collides and
///   never re-issues. Callers MUST NOT attempt to "return" or "reclaim" an
///   allocated-but-unused ordinal.
/// * **Counter saturation.** `u64` ordinals do not realistically saturate
///   (2^64 issuances). Adapters do NOT special-case overflow; a wrapping or
///   saturating add is unnecessary, and `u64::MAX` issuances is out of any
///   operational envelope.
///
/// # Observable invariants
///
/// * **Monotonic-and-unique across concurrency.** For any interleaving of N
///   concurrent calls, the N returned values are N distinct, totally-ordered
///   `IssuanceOrdinal`s. No two calls — ever, on the same store — return the
///   same value.
/// * **Durable across restart.** The value returned after a restart is
///   strictly greater than every value returned before the restart. The
///   counter survives process death (host: persisted; sim: in-memory for
///   the sim process lifetime — sufficient, as DST does not restart the
///   sim process mid-run; see § 4).
/// * **Independent of the audit table.** Deleting, compacting, or GCing
///   `issued_certificates` rows does NOT rewind the counter (this is the
///   #226 delete-survival property — see § 6). The counter is a separate
///   durable datum.
///
/// # Errors
///
/// [`ObservationStoreError::Io`] on a backing-store read/write/commit
/// failure while allocating. On error, NO ordinal is allocated and the
/// counter is unchanged (the advance is committed atomically or not at all).
async fn next_issuance_ordinal(&self) -> Result<IssuanceOrdinal, ObservationStoreError>;
```

**Import note.** `IssuanceOrdinal` is already imported in this module
(used on `IssuedCertificateRow`); no new import is required. The method
returns the `overdrive_core::id::IssuanceOrdinal` newtype directly — NOT a
bare `u64` (newtype discipline, `development.md` § "Newtypes — STRICT by
default").

### 3.2 Call-site shape (`overdrive-control-plane/src/ca_issuance.rs`)

Replace the `len()`-derivation block (lines 200-211) with a single atomic
allocation. The exact replacement:

```rust
// Global monotonic issuance ordinal — atomically allocated from the
// store's durable counter (ADR-0063 D6 rev 8). REPLACES the former
// `issued_certificate_rows().await?.len()` derivation, which was a
// read-len → mint → write check-then-act TOCTOU (RCA
// docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md): two
// concurrent issuances read the same len() and stamped duplicate
// ordinals. The atomic counter makes that collision unrepresentable —
// no serialization precondition is relied on. DST-deterministic (the
// sim adapter's counter advances under the same Mutex the audit writes
// take). See § D1-AMEND-2 (superseded by this rev).
let ordinal = observation
    .next_issuance_ordinal()
    .await
    .map_err(CaIssuanceError::audit)?;
```

- The row construction (`ca_issuance.rs:240-249`) is **unchanged** —
  `issuance_ordinal: ordinal` still sets the field from the local `ordinal`,
  which is now the allocated value rather than the `len()`-derived one.
- The `CaIssuanceError::audit` mapping is reused (an allocation failure is an
  audit-path failure — issuance refuses, no unaudited cert escapes; same
  fail-closed posture as the existing read-failure mapping). **No new
  `CaIssuanceError` variant** is added (B1 needs none; the RCA's Option-A
  `OrdinalCollision` variant is NOT part of this fix — there is no collision to
  detect).
- The `# Errors` rustdoc on `issue_and_audit` is updated to cite the ordinal
  allocation (not the old "count read") as the `CaIssuanceError::Audit` source —
  see § 5.

---

## 4. Adapter contracts (BOTH implementations)

Per `development.md` § "Trait definitions specify behavior, not just signature",
both adapters MUST observe the contract in § 3.1 identically. The DST
equivalence requirement is assessed below.

### 4.1 Host adapter — `overdrive-store-local/src/observation_backend.rs` (redb)

- **Durable counter.** Add a dedicated single-key redb table, e.g.
  `const ISSUANCE_ORDINAL_COUNTER_TABLE: TableDefinition<&[u8], u64> =
  TableDefinition::new("issuance_ordinal_counter");` (name pinned;
  additive-only, never alters existing tables). Materialize it in `open()`
  alongside the other tables (the `write.open_table(...)` block at lines
  321-343). The table holds at most one entry under a fixed key (e.g. the
  empty-slice key `b""` or a `b"next"` constant — crafter picks one literal
  key and documents it; the table is single-row by construction).
- **Atomic allocate.** `next_issuance_ordinal` runs inside ONE
  `begin_write` → read-current → insert(current+1 or initial) → `commit`,
  on the `spawn_blocking` pool (matching the `write` path's structure at lines
  381-382; no blocking redb call on a tokio worker thread,
  `development.md` § "No blocking `std::fs::*` inside `async fn`"). redb's
  serializable isolation linearises concurrent writers, so the
  read-increment-write is collision-free — the SAME TOCTOU-safety the existing
  `apply_alloc_status_lww` / `apply_issued_certificate` read-before-write rely
  on (`observation_backend.rs:1094-1098` documents exactly this property).
  - On an absent counter entry (fresh store): return `IssuanceOrdinal::new(0)`
    and persist `1` as the next value. (Equivalently: persist the returned
    value `v` and store `v+1`; the crafter pins the encoding so "first call
    returns 0" holds — match the § 3.1 edge-case clause.)
- **Durable across restart.** The committed counter survives process death;
  the next boot's first allocation reads the persisted value. Satisfied by the
  redb commit.
- **No interaction with the audit table.** The counter table is independent of
  `ISSUED_CERTIFICATES_TABLE`. Deleting audit rows does not touch the counter
  (the #226 delete-survival property).

### 4.2 Sim adapter — `overdrive-sim/src/adapters/observation_store.rs`

- **In-memory counter.** Add a field to `SimObservationStore`'s inner struct,
  e.g. `next_issuance_ordinal: Mutex<u64>` (initialized to `0`), guarded by the
  same `parking_lot::Mutex` discipline the sibling indices use (lines 102-145).
- **Atomic allocate.** `next_issuance_ordinal` locks the mutex,
  reads-increments-returns under the lock (`let mut n = self.inner.next.lock();
  let v = *n; *n += 1; IssuanceOrdinal::new(v)` — shape, not literal). Single
  critical section, no `.await` held across the lock
  (`development.md` § "Never hold a lock across `.await`"). This is the
  determinism contract: under a seeded DST run the sim's allocation order
  follows the (deterministic) call order, so the ordinal trajectory is
  bit-identical across replays.
- **Durable-across-restart caveat (explicit).** The sim counter is in-memory
  and resets if the sim store is reconstructed. This is SUFFICIENT and matches
  the trait contract's sim clause: DST does not reconstruct the
  `SimObservationStore` mid-run (a fresh sim store IS a fresh logical store, and
  starting its counter at 0 is the correct fresh-store behaviour). The host
  adapter carries the real durable-across-OS-restart guarantee; the sim adapter
  carries durable-across-sim-process-lifetime, which is the analog DST needs.

### 4.3 DST equivalence — REQUIRED

Per `development.md` § "Trait definitions specify behavior" → "The DST
equivalence test is the structural guard", and because the two adapters now
share a non-trivial concurrency contract, a DST/conformance test driving BOTH
adapters through the same sequence and asserting observable equivalence is
**required**. Concretely it must assert:

1. **Monotonic-and-unique under concurrency.** Drive N concurrent (or
   interleaved-by-the-harness) `next_issuance_ordinal()` calls against one
   store; assert the N returned ordinals are N distinct, strictly-increasing
   values, for BOTH adapters.
2. **Independent of the audit table.** Allocate an ordinal, write an
   `issued_certificate` row, allocate again; assert the second ordinal is
   strictly greater — and that it does NOT equal `issued_certificate_rows().len()`
   after a (hypothetical) row gap. (This pins the "not derived from len()"
   property and the gap semantics in one assertion.)
3. **Host-only: durable across reopen.** For the host adapter, allocate,
   drop+reopen the `LocalObservationStore` on the same redb path, allocate
   again; assert strictly greater. (Sim is exempt — its durability domain is the
   process lifetime; the conformance harness gates this sub-assertion on the
   host adapter.)

The existing trait-conformance harness home is
`overdrive_core::testing::observation_store::run_lww_conformance` (cited in the
`write` trait doc, line 1182-1184). The new ordinal-allocation conformance
should extend that harness family (a `run_issuance_ordinal_conformance`
sibling) so BOTH adapters are exercised through one definition. The crafter
wires the host-durability sub-case as a host-adapter-specific integration test
(gated `integration-tests` per `testing.md` § "Integration vs unit gating",
since it reopens a real redb file).

---

## 5. `issue_and_audit` `# Preconditions` rustdoc — what changes

The two-precondition block (`ca_issuance.rs:153-176`) is rewritten:

- **Precondition 1 (single-writer / serialized issuance) — RETIRED.** The
  ordinal no longer derives from a `len()` read, so there is no check-then-act
  for serialization to protect. `issue_and_audit` MAY now be called
  concurrently for the same store; the atomic counter guarantees distinct
  ordinals. The rustdoc states this explicitly: *"The former single-writer
  precondition is retired as of ADR-0063 rev 8 — the ordinal is allocated from a
  durable atomic counter (`ObservationStore::next_issuance_ordinal`), so
  concurrent issuance against one store no longer collides."*
- **Precondition 2 (append-only audit log) — DOWNGRADED / DECOUPLED.** The
  append-only property of `issued_certificates` is **no longer load-bearing for
  ordinal monotonicity** — the ordinal comes from the counter, not from the
  row count. The rustdoc records that the append-only contract on the audit
  table remains for its OWN reasons (audit immutability, the serial-dedup guard,
  the ADR-0067 D10 restart-recovery marker), but the ordinal's correctness no
  longer depends on it. This is the explicit answer to the task's deliverable-3
  question: **yes — the durable counter ALSO satisfies #226**, because the
  ordinal now survives row deletion (it is not a function of `len()`). A future
  delete/GC/revocation path on `issued_certificates` can prune rows freely; the
  counter does not rewind. See § 6.

The `# Errors` block updates: `CaIssuanceError::Audit` is now sourced by either
the **ordinal allocation** failure (`next_issuance_ordinal`) OR the audit-row
**write** failure — both are audit-path failures that refuse issuance and drop
the leaf. The old wording "(or the pre-write issuance-ordinal count read
failed)" becomes "(or the issuance-ordinal allocation failed)".

---

## 6. #226 disposition — RECOMMENDATION FOR THE ORCHESTRATOR TO RELAY (agent files/edits nothing)

> **No GitHub issue is created, edited, or closed by this design.** Per CLAUDE.md
> § "Deferrals require GitHub issues — AND user approval BEFORE creation" and
> MEMORY (incident #161), issue actions are user-gated. The following is a
> recommendation only.

**Finding.** #226 (read with `--comments` by the orchestrator) tracks ONLY the
delete/GC precondition (append-only precondition 2): *if* a future delete path
prunes `issued_certificates` rows, the `len()`-derived ordinal would rewind and
collide. The fix #226 prescribes is "re-source the ordinal from a persisted
monotonic counter a delete cannot rewind" — which is **exactly** the durable
atomic counter B1 introduces.

**Disposition: this design CLOSES #226's technical content.** The `len()`
derivation that #226 was filed against is REMOVED entirely. A durable
atomic counter that survives row deletion is the precise mechanism #226 asked
for. Because the ordinal is no longer a function of the audit table's contents:

- A future delete/GC/revocation path on `issued_certificates` can prune rows
  without rewinding the ordinal (#226's acceptance — **satisfied**).
- The concurrency precondition (untracked today — the RCA notes precondition 1
  has no issue) is ALSO satisfied by the same counter (**bonus** — one fix,
  both preconditions).

**Recommended user action (pick one — agent acts on none):**
- **(i, recommended)** Close #226 as resolved-by-this-fix when the fix lands,
  citing this design and the ADR-0063 rev 8 amendment. The durable counter is
  the mechanism #226 specified; the delete-survival acceptance is met.
- **(ii)** If the user prefers to keep #226 open until a delete/GC path actually
  *exists* (the trigger #226's title names), narrow #226's body to "verify the
  counter survives the delete path once a delete path lands" and note that the
  ordinal-source half is already done by this fix. (Weaker; the structural fix
  is complete, only the not-yet-existing delete path is unverified.)

Either way, the rustdoc precondition-2 citation to #226 is updated to reflect
that the ordinal no longer depends on append-only (it cites the counter, and —
per the user's choice above — either drops the #226 ref or re-scopes it to "the
delete path, when it lands, must not touch the counter").

---

## 7. Decision record home — ADR-0063 rev 8 amendment (proposal, NOT committed)

This adds an `ObservationStore` **port** method — squarely ADR-0063 D6's domain
(D6 owns the `issued_certificates` audit row AND its typed reader on the port).
The amendment is a dated changelog entry that reopens the Accepted ADR (the
ADR's own convention — § Status "each reopens this Accepted ADR via a dated
entry, not a silent rewrite"), plus a tightening of the D6 prose.

Cross-checks performed:
- **ADR-0067** (SVID lifecycle): the D10 restart-recovery marker depends on
  `audit-row-exists ⟹ minted-and-audited` (the audit-before-hold ordering),
  which this fix does NOT touch — the row write is unchanged, only the ordinal's
  *source* changes. No ADR-0067 conflict; no ADR-0067 amendment needed.
- **Archived feature-delta § D1-AMEND-2** (`docs/evolution/2026-06-11-built-in-ca-operator-composition.md`):
  that is the *archived* record of the original `len()`-derivation decision.
  It is finalized history and is NOT edited; the ADR-0063 rev 8 changelog entry
  is the live supersession pointer (it names D1-AMEND-2 as the superseded
  derivation).

The draft amendment text is in § 7.1. **Per house rule, ADR edits go through the
architect (me) — but this is a PROPOSAL for user approval; nothing is committed
by this design task.**

### 7.1 DRAFT — ADR-0063 D6 amendment (rev 8) — for user approval

> Append to ADR-0063 § Status:
> *"and 2026-06-11 (rev 8 — issuance ordinal sourced from a durable atomic
> counter (`ObservationStore::next_issuance_ordinal`) instead of
> `issued_certificate_rows().len()`; retires the single-writer issuance
> precondition and decouples ordinal monotonicity from the append-only audit
> log — resolves the TOCTOU RCA at
> `docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md` and the #226
> delete-survival precondition)"*

> Append to ADR-0063 D6 (after the "Issuance is never silent" paragraph):
>
> **D6 rev 8 — issuance ordinal is counter-allocated, not `len()`-derived.**
> The `IssuanceOrdinal` stamped on each `issued_certificates` row is allocated
> by a new additive `ObservationStore` port method
> `next_issuance_ordinal(&self) -> Result<IssuanceOrdinal, ObservationStoreError>`,
> which atomically advances a durable, strictly-monotonic counter and returns the
> next value. This SUPERSEDES the original derivation (feature-delta § D1-AMEND-2:
> "ordinal = count of already-persisted rows, read immediately before the audit
> write"), which was a read-len → mint → write check-then-act TOCTOU
> (`.claude/rules/development.md` § "Check-and-act must be atomic"): two concurrent
> issuances read the same `len()` and stamped duplicate ordinals, and the
> consumer's `max_by_key(issuance_ordinal)` then surfaced a stale serial as
> "current". The counter makes the collision UNREPRESENTABLE.
>
> Consequences of rev 8:
> - The **single-writer / serialized-issuance precondition is RETIRED** —
>   `issue_and_audit` is now safe to call concurrently for one store.
> - **Ordinal monotonicity no longer depends on the append-only audit log.**
>   The counter is a separate durable datum that row deletion cannot rewind —
>   this satisfies the #226 delete/GC precondition as a side effect (a future
>   `issued_certificates` GC may prune rows without breaking ordinals).
> - The method is mandatory on BOTH adapters (`LocalObservationStore` host:
>   single-key redb counter table, atomic under redb's serializable isolation;
>   `SimObservationStore` sim: in-memory counter under the existing Mutex). A DST
>   conformance case drives both through the same allocation sequence and asserts
>   monotonic-and-unique-under-concurrency equivalence.
> - Gap semantics: an allocated-but-unwritten ordinal (mint/write fails after
>   allocation) is burned; the sequence may have holes. This is correct —
>   selection needs strict ordering, not density.
> - **No row / envelope / golden-bytes change.** `issuance_ordinal` stays a
>   caller-set field on the `V1` payload; only its *source* changes. The rkyv
>   schema-evolution surface and the append-only serial guard are untouched.
> - Greenfield single-cut migration: a fresh store starts the counter at 0;
>   "delete the redb file" is the upgrade path. No migration code reconstructs a
>   counter from a pre-rev-8 store.

(Earned Trust note for the amendment's probe contract: ADR-0063 already carries a
§ "Earned Trust (probe contract)"; the new counter table participates in the
`ObservationStore` adapter's existing `probe()` discipline — the host adapter's
boot-time table materialization (`open()`) is the wire-then-probe gate that
fails startup if the counter table cannot be opened. No new probe surface is
required; the counter table rides the existing observation-store probe.)

---

## 8. Scope boundaries / out of scope

- **Multi-node gossip ordinal uniqueness (GH #36) — UNCHANGED, out of scope.**
  Today's `len()` is equally node-local; the new counter is equally node-local.
  This fix does not improve OR regress cross-node ordinal uniqueness — when
  `issued_certificates` becomes gossiped (#36), cross-node ordinal allocation is
  a separate concern (a per-node counter still collides across nodes; a global
  allocator is a #36 design question). **Do not expand into #36.** The fix's
  contract is explicitly *per-store* (= per-node today).
- **No migration code.** Greenfield single-cut per
  `feedback_single_cut_greenfield_migrations.md`: fresh store → counter at 0;
  delete-the-redb-file is the upgrade path. **Confirmed: no migration code is
  needed** — there is no "reconstruct the counter from existing rows" path (that
  would re-introduce a `len()`-shaped derivation). A pre-rev-8 store is discarded,
  not migrated.
- **No new error variant.** B1 needs none. The RCA's Option-A `OrdinalCollision`
  detection variant is explicitly NOT part of this fix — there is no collision to
  detect once it is unrepresentable.
- **No change to `issue_and_audit`'s signature**, the row construction, the
  consumer projection (`handlers::issued_certificates_for_rows` keeps
  `max_by_key(issuance_ordinal)` — now collision-free), or the append-only guard.

---

## 9. Quality gates (self-check)

- [x] Requirements traced to surface (TOCTOU → atomic counter port method).
- [x] Single new component boundary, clear responsibility (ordinal allocation
      = `ObservationStore`'s concern; it owns the durable audit surface).
- [x] Technology choice in the amendment with the rejected alternative (B2) and
      rejection rationale (§ 2).
- [x] Dependency-inversion preserved: the new method is a port-trait method;
      `ca_issuance` depends on the trait, not an adapter.
- [x] Adapter contract specified for BOTH host and sim, with the DST equivalence
      requirement (§ 4).
- [x] Trait-doc contract: preconditions / postconditions / edge-cases /
      observable-invariants per `development.md` § "Trait definitions specify
      behavior" (§ 3.1).
- [x] Earned Trust: the counter table rides the existing `ObservationStore`
      probe (boot-time table materialization is the wire-then-probe gate).
- [x] Simplest solution: B1 over B2; no new store, no new pattern, no new error
      variant, no migration code.
- [x] OSS-only (redb already in graph; no new dependency).
- [x] No GitHub issue created/edited/closed; #226 disposition surfaced as text
      (§ 6).
- [x] No implementation code or tests written — pinned signatures + contracts +
      ADR amendment proposal only.
