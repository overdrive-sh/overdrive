# fix-issuance-ordinal-toctou — Feature Evolution

**Finalized**: 2026-06-11 · **Type**: Bug fix / latent-invariant hardening
(code-review comment → `/nw-bugfix` → DESIGN → DELIVER) · **Fix SHA**:
`dedc432b` (`fix(ca-issuance): allocate issuance ordinal from a durable atomic
counter`, Step-ID 01-01) · **Docs SHA**: `2512c9d1` (ADR-0063 rev 8 + research
doc) · **ADR**:
[ADR-0063](../product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md)
rev 8 (D6 — issuance ordinal counter-allocated, not `len()`-derived) ·
**Builds on**:
[2026-06-11-built-in-ca-operator-composition](2026-06-11-built-in-ca-operator-composition.md)
(the feature-delta § D1-AMEND-2 `len()`-derivation this fix supersedes) ·
**Research**:
[issuance-ordinal-allocation-approaches](../research/pki/issuance-ordinal-allocation-approaches.md)

---

## Summary

The `IssuanceOrdinal` stamped on every `issued_certificates` audit row was
derived from `observation.issued_certificate_rows().await?.len()` immediately
before the audit write (`ca_issuance.rs:209-210`) — a read-len → mint →
write-row **check-then-act TOCTOU**. Its uniqueness held only by two *ambient*
invariants that lived solely in rustdoc: "issuance is serialized through the
single action-shim executor" (precondition 1) and "the audit log is
append-only" (precondition 2). Two concurrent `issue_and_audit` calls for one
store both read `len()==N` and both stamp ordinal `N`; the consumer projection
`handlers::issued_certificates_for_rows` then resolves the `max_by_key(
issuance_ordinal)` tie by store iteration order (CSPRNG-serial-keyed, no
recency relation) and surfaces a **stale serial as "current"** — verbatim the
bug the ordinal exists to prevent. Correct today (the only production caller is
the strictly-sequential action-shim `dispatch` loop), latent the moment a
parallel executor lands; permanent and silent when it does (append-only ⇒
non-deletable, no error/panic).

The fix (design candidate **B1**) makes the collision **unrepresentable**:
the ordinal is sourced from a durable, atomically-allocated monotonic counter
that the `ObservationStore` owns, independent of the audit-table contents. One
additive port method — `async fn next_issuance_ordinal(&self) ->
Result<IssuanceOrdinal, ObservationStoreError>` — a one-line call-site swap in
`ca_issuance.rs`, two adapter impls (host: single-key redb counter table,
atomic under serializable isolation; sim: `Mutex<u64>`), retired/decoupled
precondition rustdoc, and a `run_issuance_ordinal_conformance` DST equivalence
harness driving both adapters. **No** row, envelope, or golden-bytes change; no
new error variant; no migration code (greenfield single-cut). This is the
**workload-identity** CA's `issued_certificates` table, NOT the operator /
control-plane HTTPS CA (CLAUDE.md § "Two distinct certificate authorities").

---

## Origin and verdict

A code-review comment flagged the seam: the serialized-execution invariant
guarding the ordinal derivation is **documentation-only**, neither type- nor
runtime-enforced. The comment routed through `/nw-bugfix` (RCA + DESIGN) into
DELIVER.

The validated RCA (`deliver/rca.md`, every supplied data point adversarially
re-checked against source) classified this as a **LATENT-INVARIANT HARDENING
task, not a present bug** — **P2**, not P0/P1:

- Behaviour is correct **today**. The sole production caller is the action-shim
  `dispatch_issue` (`issue_svid.rs:101`), reached only from the sequential
  `for action in actions { … .await … }` loop (`action_shim/mod.rs:416-440`).
  No production path issues two SVIDs concurrently against one store, so the
  `len()` read and the row write never interleave.
- A red regression test against *current* behaviour is **not constructible**:
  the precondition *is* "do not call `issue_and_audit` concurrently for one
  store," so any test that makes the ordinal collide must violate precondition 1
  by construction (a `join!` of two issuances, or a hand-rolled interleave). Such
  a test asserts on behaviour outside the contract — it proves the seam is
  unguarded, which is the reviewer's point, not that today's code is wrong.

The two root causes compose, not contradict: **A** (why a slip corrupts —
ordinal modeled as a derived `len()` projection, collision representable) and
**B** (why a slip is silent — no post-write uniqueness check; the append-only
*serial* guard returns green and is irrelevant to ordinals; the consumer is
tie-blind). Fixing A (make the ordinal unrepresentable-to-collide) dissolves B
as a side effect — no collision means nothing to detect.

---

## What shipped — B1, the make-unrepresentable fix

Governed end-to-end by `.claude/rules/development.md` § "Check-and-act must be
atomic (no TOCTOU)" → *type the racy surface away / make the collision
unrepresentable* (the `ClaimSet` / `RaceOnceCell`-class move).

1. **One additive `ObservationStore` port method**
   (`crates/overdrive-core/src/traits/observation_store.rs`):

   ```rust
   async fn next_issuance_ordinal(&self) -> Result<IssuanceOrdinal, ObservationStoreError>;
   ```

   Atomically allocates the next ordinal from a durable strictly-monotonic
   counter and returns the `IssuanceOrdinal` newtype (not a bare `u64` —
   newtype discipline). The full trait-doc contract pins preconditions (**none**
   — safe to call concurrently for one store from any number of callers),
   postconditions (`n` strictly greater than every prior return *including
   across restart*; the counter is durably advanced past `n` *before* `Ok(n)`
   returns; no audit row read or written), edge cases (fresh store returns
   `IssuanceOrdinal::new(0)` to match pre-fix `len()`-of-empty semantics and keep
   V1 golden fixtures valid; allocated-but-unwritten ordinals are **burned** —
   gaps are by design), and observable invariants (monotonic-and-unique under
   any concurrent interleaving; durable across restart; **independent of the
   audit table** — deleting/compacting/GCing rows does not rewind the counter).

2. **One-line call-site swap**
   (`crates/overdrive-control-plane/src/ca_issuance.rs`) — the `len()`
   derivation block (lines 200-211) is replaced by:

   ```rust
   let ordinal = observation
       .next_issuance_ordinal()
       .await
       .map_err(CaIssuanceError::audit)?;
   ```

   Row construction is **unchanged** — `issuance_ordinal: ordinal` still sets the
   caller-set field, now from the allocated value rather than the `len()`-derived
   one. The `CaIssuanceError::audit` mapping is reused (an allocation failure is
   an audit-path failure — issuance refuses fail-closed, no unaudited cert
   escapes). **No new `CaIssuanceError` variant** (B1 needs none; the RCA's
   Option-A `OrdinalCollision` detection variant is explicitly NOT part of this
   fix — there is no collision to detect once it is unrepresentable).

3. **Two adapter implementations:**
   - **Host** (`overdrive-store-local/src/observation_backend.rs`): a dedicated
     single-key redb table `issuance_ordinal_counter`, materialized at `open()`
     alongside the other tables; `next_issuance_ordinal` does read-current →
     insert(current+1) → `commit` inside ONE `begin_write` on `spawn_blocking`.
     redb's serializable single-writer isolation linearises concurrent writers,
     so the read-increment-write is collision-free — the same TOCTOU-safety the
     existing LWW / append-only read-before-write guards already rely on.
     Durable across OS restart; independent of `ISSUED_CERTIFICATES_TABLE`.
   - **Sim** (`overdrive-sim/src/adapters/observation_store.rs`): an in-memory
     `Mutex<u64>` (init `0`) under the existing `parking_lot::Mutex` discipline;
     read-increment-return in one critical section, no `.await` held across the
     lock. Allocation follows deterministic call order → bit-identical ordinal
     trajectory across seeded replays. Durable-across-sim-process-lifetime is the
     analog DST needs (the sim store is never reconstructed mid-run).

4. **Precondition rustdoc retired/decoupled** (`ca_issuance.rs`): precondition 1
   (single-writer / serialized issuance) is **RETIRED** — `issue_and_audit` may
   now be called concurrently for one store. Precondition 2 (append-only audit
   log) is **DECOUPLED** from ordinal monotonicity — append-only remains for its
   own reasons (audit immutability, the serial-dedup guard, the ADR-0067 D10
   restart-recovery marker) but the ordinal's correctness no longer depends on
   it. The `# Errors` block updates: `CaIssuanceError::Audit` is now sourced by
   the ordinal **allocation** failure (or the audit-row write failure), not the
   old "count read."

5. **DST equivalence harness** — `run_issuance_ordinal_conformance` extends the
   existing `overdrive_core::testing::observation_store` conformance family,
   driving BOTH adapters through one allocation sequence and asserting:
   (i) monotonic-and-unique under concurrency; (ii) independence from the audit
   table (allocate → write a row → allocate again, second strictly greater, not
   equal to `len()`); (iii) host-only durable-across-reopen (allocate,
   drop+reopen the `LocalObservationStore` on the same redb path, allocate again,
   strictly greater — gated `integration-tests`, sim exempt).

### Why the collision is now unrepresentable

The ordinal is a **separate durable datum**, allocated **atomically**,
**independent of the audit rows**. Two concurrent callers receive two distinct,
strictly-increasing values; the type system and the counter's atomicity — not a
narrated precondition — guarantee it. Both root causes dissolve: A (no
collision) and B (nothing to detect).

---

## The mid-DELIVER design challenge — why a separate counter, not derive-from-rows

Mid-DELIVER the user asked the right question: *"why a separate counter —
shouldn't the row just hold it, read+increment, retry on conflict?"* This is
candidate **(B)** — `max(ordinal)+1` over the audit rows plus an optimistic-retry
loop. The research (`docs/research/pki/issuance-ordinal-allocation-approaches.md`,
16 sources, confidence High) was commissioned to settle it adversarially and
**confirmed KEEP (A)** — the dedicated atomic counter — one-sided across all
seven evaluation axes.

The **deciding axis is deletion-survival (GH#226)**, and it is exactly where the
user's intuition breaks. The concurrency *half* of the instinct is sound — an
atomic read-increment with conflict handling does prevent concurrent collision,
and (A) already does precisely that. The *derive-from-rows* half does not
survive deletion, and **retry cannot save it**: with rows at ordinals {0,1,2},
pruning the max-ordinal row (the exact #226 trigger) makes `max(surviving)+1 =
2` re-issue the just-retired ordinal — and because the colliding row was
*deleted*, the uniqueness check has nothing to conflict against, the insert
succeeds on the **first** attempt, and the retry loop **never fires**. Retry
defends only against a conflict with a *surviving* row; deletion removes that
row. (B) therefore fixes only the concurrency half and re-opens #226 that (A)
closes.

Load-bearing evidence the research pinned:

- **`SELECT max()+1` is the 40-year RDBMS concurrency anti-pattern**; a
  dedicated sequence/counter is the standard remedy. Its **gaps are the
  intended, documented behaviour** — PostgreSQL's docs state verbatim that
  sequences "cannot be used if 'gapless' assignment is needed," and our
  requirement is explicitly *ordering, not density*, so the burned-ordinal gaps
  are correct, not a defect (Findings 1, 2, 8).
- **Certificate serials are a different field governed by an opposite rule.**
  RFC 5280 §4.1.2.2 requires serials to be unique and (with CA/B Forum §7.1)
  **non-sequential** CSPRNG draws — which is *why* the recency ordinal must exist
  at all (the serial is useless for ranking). The entire PKI ecosystem (SPIRE,
  step-ca, Vault, cfssl, Boulder, EJBCA) allocates *serials* via CSPRNG; that
  advice must NOT be imported as guidance for the *ordinal*, and when those
  systems need recency ordering they delegate to their datastore's native
  sequence, never application-level `max()+1` (Findings 3, 4, 5).
- **redb gives serializable single-writer isolation** ("all writes applied
  sequentially") — (A)'s counter is linearised for free; the original bug existed
  only because the read and the write were two separate transactions (Finding 7).

Secondary axes all agree: (B) is OCC and degrades to documented livelock/retry-
storm under exactly the high-contention parallel-executor future this fix
targets; (B) re-couples the ordinal to the audit table, violating "persist
inputs, not derived state" that (A) honours; (B)'s schedule-dependent retry
count threatens DST replay-equivalence that (A)'s Mutex-serialized counter
preserves; and (A) is the clean migration base for the multi-node future (GH#36 —
out of scope, unchanged: both `len()` and the counter are node-local today).

---

## #226 disposition — CLOSED as resolved-by-this-fix

This fix **resolves GH#226**. #226 tracked the delete/GC/revocation precondition:
*if* a future delete path prunes `issued_certificates` rows, the `len()`-derived
ordinal would rewind and collide. The remedy #226 prescribed — "re-source the
ordinal from a persisted monotonic counter a delete cannot rewind" — is
**exactly** the durable atomic counter B1 introduces. Because the ordinal is no
longer a function of the audit table's contents:

- A future delete/GC/revocation path on `issued_certificates` can prune rows
  without rewinding the ordinal (#226's acceptance — **satisfied**).
- The concurrency precondition (untracked: the RCA confirmed precondition 1 had
  **no** issue, only precondition 2 cited #226) is **also** satisfied by the same
  counter — one fix, both preconditions.

Per user-approved option **(a)**, #226 is **CLOSED as resolved-by-this-fix**.
Per CLAUDE.md § "Deferrals require GitHub issues — AND user approval BEFORE
creation" / MEMORY (incident #161), no agent touched the issue: the orchestrator
performs the actual close. The RCA and DESIGN both surfaced the disposition as a
*recommendation only*; the user chose (a).

---

## Decisions made (DESIGN D1–D13)

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **B1 — separate atomic allocation** via `next_issuance_ordinal()` port method | Smallest honest surface: one method + two adapters + one call-site swap. Leaves `IssuedCertificateRow`, its rkyv V1 payload, envelope, golden-bytes, codec, and append-only serial guard ENTIRELY intact. **B2 (store-assigns-at-write)** rejected: introduces a not-yet-assigned sentinel state (forbidden by § "Sum types over sentinels"), splits the canonical-bytes codec (`archive_for_store`), couples the serial-dedup guard to allocation, AND still needs a method to return the value — more disturbance, no net saving. |
| D2 | Returns `IssuanceOrdinal` newtype, not `u64` | Newtype discipline; already imported in the trait module. |
| D3 | Fresh-store counter starts at **0** | Matches pre-fix `len()`-of-empty semantics; keeps "ordinals start at 0" consumer expectation and V1 golden fixtures valid. |
| D4 | **Gap semantics accepted** — burned ordinals leave holes | Consumer `max_by_key` needs strict ORDERING, not density; a burned ordinal never collides. Reclaim/return-on-failure would re-introduce a check-then-act. |
| D5 | Host counter = single-key redb table, atomic in one `begin_write`/`commit` under serializable isolation (`spawn_blocking`) | Same TOCTOU-safety the existing LWW / append-only read-before-write guards rely on; durable across OS restart. |
| D6 | Sim counter = in-memory `Mutex<u64>` under existing `parking_lot::Mutex` | DST-deterministic; durable-across-sim-process-lifetime is the analog DST needs. |
| D7 | **DST equivalence test REQUIRED** — both adapters, one sequence | The adapters now share a non-trivial concurrency contract; § "The DST equivalence test is the structural guard" mandates it. |
| D8 | Precondition 1 (single-writer) **RETIRED**; precondition 2 (append-only) **DECOUPLED** | The ordinal no longer derives from `len()`; § "Check-and-act" disprefers narrated invariants over structural ones. |
| D9 | **No new `CaIssuanceError` variant** — allocation failure reuses `::audit` | B1 needs no collision detection (unrepresentable). The RCA Option-A `OrdinalCollision` variant is NOT part of this fix. |
| D10 | Recorded as **ADR-0063 rev 8** dated changelog entry + D6 prose tightening | The method is `ObservationStore`-port surface = D6's domain. ADR-0067 untouched (audit-before-hold ordering unchanged); archived feature-delta D1-AMEND-2 not edited (finalized history; rev 8 is the live supersession pointer). |
| D11 | **#226 CLOSED** — durable counter is the exact mechanism #226 prescribed | The `len()` derivation #226 was filed against is removed; delete-survival acceptance met. User chose close-now (option a); agent files/edits/closes nothing. |
| D12 | **No migration code** — greenfield single-cut, fresh store → counter at 0 | A "reconstruct counter from existing rows" path would re-introduce a `len()`-shaped derivation — explicitly avoided. "Delete the redb file" is the upgrade path. |
| D13 | **#36 (multi-node gossip) out of scope, UNCHANGED** | Today's `len()` and the new counter are equally node-local; neither improves nor regresses cross-node uniqueness. |

---

## ADR record — ADR-0063 rev 8

The decision lives at
[ADR-0063](../product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md)
as a dated **rev 8** changelog entry reopening the Accepted ADR plus a D6 prose
tightening (docs SHA `2512c9d1`). D6 rev 8 records: the `IssuanceOrdinal` is
counter-allocated via `next_issuance_ordinal`, **superseding** feature-delta
§ D1-AMEND-2's `len()` derivation; the single-writer precondition is retired;
ordinal monotonicity no longer depends on the append-only audit log (satisfying
#226 as a side effect); the method is mandatory on both adapters with a DST
conformance case; gap semantics are correct; no row / envelope / golden-bytes
change; greenfield single-cut migration. The new counter table rides the existing
`ObservationStore` `probe()` discipline (boot-time table materialization is the
wire-then-probe gate) — no new probe surface.

---

## Quality gates

- **Lima compile / nextest / doctest** — green (the canonical Linux compile +
  run environment; not `--no-run`).
- **dst-lint** — exits 0: the new core method is pure trait surface; the host
  adapter's redb counter is on `spawn_blocking`, no blocking I/O on a tokio
  worker; the sim counter holds no lock across `.await`.
- **clippy** — `-D warnings` clean across the touched crates.
- **Adversarial peer review** — **APPROVED**: verified the implementation against
  the pinned design's exact API shape (one method, the pinned signature, no
  invented surface — no second method, no error variant, no row-field change),
  zero defects, Testing Theater check passed (the conformance harness asserts on
  real observable equivalence, not narrated behaviour).
- **DES integrity** — verifier exit 0; all phases EXECUTED/SKIPPED with valid
  transitions.
- **Mutation testing (Phase 5)** — **SKIPPED by user direction.** Recorded
  honestly: the per-feature mutation gate was not run for this fix. It did not
  execute; no kill-rate is claimed.

---

## Steps completed

| Step ID | Phase | Status | Commit | Notes |
|---|---|---|---|---|
| 01-01 | GREEN fix | DONE | `dedc432b` | `next_issuance_ordinal` lands on the `ObservationStore` trait with the full behaviour-contract rustdoc; host (single-key redb counter table, atomic `begin_write`/`commit`) and sim (`Mutex<u64>`) adapters implement it; `ca_issuance.rs` swaps the `len()` derivation for the atomic allocation; precondition-1 retired / precondition-2 decoupled in rustdoc; `run_issuance_ordinal_conformance` DST equivalence harness drives both adapters (monotonic-and-unique-under-concurrency + independence-from-audit-table + host-durable-across-reopen). No row/envelope/golden-bytes change; no new error variant; no migration code. |
| (docs) | ADR + research | DONE | `2512c9d1` | ADR-0063 rev 8 dated amendment (D6 — counter-allocated ordinal, supersedes feature-delta D1-AMEND-2) via architect; research doc `issuance-ordinal-allocation-approaches.md` validating KEEP-A over the user's proposed derive-from-rows+retry (B). |

---

## Lessons learned

1. **A documentation-only invariant on a representable collision is the bug,
   even when the code is correct today.** The visible defect was a missing
   atomicity guarantee; the structural defect was that the ordinal's uniqueness
   was delegated to two ambient preconditions living only in rustdoc, with the
   collision **representable** by the type system. § "Check-and-act must be
   atomic" ranks *make-unrepresentable* above *detect-and-recover*; the fix that
   closes the cause is sourcing the value from a structurally-collision-free
   counter, not adding a post-write tripwire (the RCA's dispreferred Option A,
   which only converts silent corruption into a loud `Err` *after* a permanent
   duplicate row already exists, and taxes the hot path with an unbounded `O(N)`
   re-read).

2. **The user's "shouldn't the row just hold the counter and retry?" was worth a
   research dispatch, and the answer sharpened the design.** The instinct was
   right about *mechanism* (an atomic read-increment is correct — and (A) already
   IS that) but wrong about *source* (deriving from `max(rows)` re-couples to the
   audit table and rewinds on deletion, which retry structurally cannot fix). The
   deciding evidence was a deductive certainty from `max()` semantics, not a
   judgment call — exactly the kind of question where a 16-source research pass
   converts an intuition into a pinned, defensible decision.

3. **Serial ≠ ordinal.** The PKI ecosystem's uniform CSPRNG-serial practice is
   evidence about a *different field* governed by the *opposite* rule
   (RFC 5280 / CA/B Forum mandate non-sequential serials). Conflating "how CAs
   allocate serials" with "how to allocate a recency ordinal" is a category
   error the research existed to prevent — the ordinal is our own construct, and
   the serial is precisely *why* it must exist.

4. **A latent-invariant hardening with no constructible red test is still worth
   shipping before the regime that triggers it arrives.** No production caller
   violates the precondition today, so no in-contract regression test reproduces
   the collision — but the cost of a slip is high (permanent, silent,
   append-only) and rising (any future parallel executor, fan-out reconciler, or
   careless test introduces it). Hardening *before* a parallel executor lands is
   the correct trade; the DST conformance harness is the durable proof that both
   adapters honour the atomic contract going forward.

---

## Risk delta (post-fix)

- **Wire-compatible.** No `ObservationStore::write` / `issue_and_audit` signature
  change, no row / envelope / golden-bytes change. `issuance_ordinal` stays a
  caller-set field on the V1 payload; only its *source* changes. The consumer
  `max_by_key(issuance_ordinal)` is unchanged — now collision-free.
- **No client-visible behaviour change in the current single-node sequential
  regime** (ordinals were already unique under serialized dispatch). The
  structural protection activates the moment a concurrent issuance path lands.
- **#226 closed.** A future `issued_certificates` delete/GC/revocation path may
  prune rows without rewinding the ordinal — the counter is a separate durable
  datum.
- **Performance.** The host path trades one unbounded `O(N)` `issued_certificate_
  rows().len()` read per issuance for one single-key redb counter read-increment-
  commit (`O(1)`, bounded) — a net improvement that also removes the growth-with-
  log-size tax the RCA flagged on Option A.
- **DST.** The sim counter advances under the same `parking_lot::Mutex` the audit
  writes take; allocation order follows deterministic call order, so the ordinal
  trajectory is bit-identical across seeded replays. The conformance harness pins
  monotonic-and-unique-under-concurrency for both adapters.
- **Two-CA discipline intact.** This touches ONLY the **workload-identity** CA's
  `issued_certificates` audit path (`ca_issuance.rs` / `lib.rs:1595` lineage).
  The operator / control-plane HTTPS CA (`mint_ephemeral_ca`, `lib.rs:1237`) is
  untouched.

---

## Related

- **`ObservationStore::next_issuance_ordinal`** —
  `crates/overdrive-core/src/traits/observation_store.rs` — the durable atomic
  allocator; SSOT for the `IssuanceOrdinal` (ADR-0063 D6 rev 8). Replaces the
  former `issued_certificate_rows().len()` derivation.
- **`issue_and_audit`** — `crates/overdrive-control-plane/src/ca_issuance.rs` —
  the call site swapped from `len()`-derivation to atomic allocation;
  precondition 1 retired, precondition 2 decoupled.
- **`run_issuance_ordinal_conformance`** —
  `crates/overdrive-core/src/testing/observation_store.rs` — the DST equivalence
  harness driving both adapters through one allocation sequence; the durable hook
  for any future `ObservationStore` impl.
- **ADR-0063 rev 8** —
  [adr-0063](../product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md)
  — D6 amendment: counter-allocated ordinal, supersedes feature-delta
  D1-AMEND-2.
- **Research** —
  [issuance-ordinal-allocation-approaches](../research/pki/issuance-ordinal-allocation-approaches.md)
  — KEEP-A vs derive-from-rows+retry (B); deletion-survival is dispositive.
- **Builds on** —
  [2026-06-11-built-in-ca-operator-composition](2026-06-11-built-in-ca-operator-composition.md)
  — the feature whose § D1-AMEND-2 `len()` derivation this fix supersedes.
- **Doctrine** — `.claude/rules/development.md` § "Check-and-act must be atomic
  (no TOCTOU)" (make-unrepresentable) and § "Persist inputs, not derived state"
  (the rule (B) would violate and (A) honours).
- **GH#226** — closed as resolved-by-this-fix (durable counter is the prescribed
  mechanism). **GH#36** (multi-node gossip ordinal uniqueness) — out of scope,
  unchanged; (A) is the clean migration base.

---

## Notes on workspace artifacts

This bugfix carries DESIGN-wave artifacts (the `/nw-bugfix` pipeline produced an
RCA + a full DESIGN pass pinning the API surface). The lasting records are: this
evolution document, ADR-0063 rev 8, and the research doc (already in its
permanent `docs/research/` home — no migration needed). The
`docs/feature/fix-issuance-ordinal-toctou/` workspace
(`deliver/rca.md`, `design/architecture.md`, `design/wave-decisions.md`,
`deliver/roadmap.json`, `deliver/execution-log.json`) is **preserved** per
repo convention (feature dirs persist after finalize so the wave matrix can
derive feature status) — not deleted. The full audit trail:

- This evolution document — canonical post-mortem.
- `docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md` — the validated
  Toyota-5-Whys RCA (every supplied data point re-checked against source).
- `docs/feature/fix-issuance-ordinal-toctou/design/architecture.md` — the pinned
  B1 surface spec.
- `docs/feature/fix-issuance-ordinal-toctou/design/wave-decisions.md` — D1–D13.
- `docs/research/pki/issuance-ordinal-allocation-approaches.md` — the KEEP-A
  research.
- The two commits on `marcus-sa/cert-rotation-workflow` (`dedc432b`, `2512c9d1`).
