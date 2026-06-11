# Wave decisions — fix-issuance-ordinal-toctou (DESIGN)

**Architect:** Morgan · **Date:** 2026-06-11 · **Mode:** Propose

| # | Decision | Rationale | Rejected alternative |
|---|----------|-----------|----------------------|
| D1 | **B1 — separate atomic allocation** via a new `ObservationStore::next_issuance_ordinal()` port method. | Smallest honest surface: one port method + two adapter impls + one call-site swap. Leaves `IssuedCertificateRow`, its rkyv `V1` payload, the envelope, golden-bytes fixture, codec, and append-only serial guard ENTIRELY intact. Ordinal stays a caller-set field. | **B2 (store-assigns-at-write):** introduces a not-yet-assigned sentinel state on the row (forbidden by § "Sum types over sentinels"), splits the canonical-bytes codec (`archive_for_store`), couples the serial-dedup guard to counter allocation, AND still needs a method to return the assigned value. More disturbance, no net surface saving. |
| D2 | Method returns `IssuanceOrdinal` newtype, not `u64`. | Newtype discipline (`development.md` § "Newtypes — STRICT"). Already imported in the trait module. | bare `u64` return — rejected. |
| D3 | Fresh-store counter starts at **0**; first call returns `IssuanceOrdinal::new(0)`. | Matches pre-fix `len()`-of-empty-table semantics; keeps existing "ordinals start at 0" consumer expectation and V1 golden fixtures valid. | start at 1 — would shift all existing semantics for no benefit. |
| D4 | **Gap semantics accepted:** allocated-but-unwritten ordinals are burned (sequence may have holes). | Consumer projection (`max_by_key(issuance_ordinal)`) needs strict ORDERING, not density. A burned ordinal never collides. | reclaim/return-on-failure — adds complexity, re-introduces a check-then-act. |
| D5 | Host counter = single-key redb table, atomic in one `begin_write`/`commit` under redb serializable isolation (on `spawn_blocking`). | Same TOCTOU-safety the existing LWW / append-only read-before-write guards rely on. Durable across OS restart. | a separate file / external sequence — unnecessary; redb already linearises writers. |
| D6 | Sim counter = in-memory `Mutex<u64>`, increment under the existing `parking_lot::Mutex` discipline. | DST-deterministic (allocation follows deterministic call order); durable-across-sim-process-lifetime is the analog DST needs (sim store is never reconstructed mid-run). | persisting sim state — out of sim's contract; in-memory is the sanctioned sim durability domain. |
| D7 | **DST equivalence test REQUIRED** — drive both adapters through one allocation sequence; assert monotonic-and-unique-under-concurrency + independence-from-audit-table; host-only durable-across-reopen sub-case. | The two adapters now share a non-trivial concurrency contract; § "The DST equivalence test is the structural guard" mandates it. | unit-test-each-adapter-separately — would not pin observable equivalence. |
| D8 | Precondition 1 (single-writer) **RETIRED**; precondition 2 (append-only) **DECOUPLED** from ordinal monotonicity. | The ordinal no longer derives from `len()` — serialization and append-only are no longer load-bearing for it. | keep them as documentation — rejected; the rule (§ Check-and-act) explicitly disprefers narrated invariants over structural ones. |
| D9 | **No new `CaIssuanceError` variant.** Allocation failure reuses `CaIssuanceError::audit`. | B1 needs no collision detection (collision is unrepresentable). The RCA Option-A `OrdinalCollision` variant is NOT part of this fix. | add a detection variant — that is Option A (dispreferred detect-after-the-fact), not the chosen B. |
| D10 | Decision recorded as **ADR-0063 rev 8** dated changelog entry + D6 prose tightening. | The method is `ObservationStore`-port surface = D6's domain. ADR's own convention: amend via dated entry reopening the Accepted ADR. ADR-0067 untouched (audit-before-hold ordering unchanged); archived feature-delta D1-AMEND-2 not edited (finalized history; rev 8 is the live supersession pointer). | a new standalone ADR — overkill for one additive port method amending an existing D-point. |
| D11 | **#226 disposition: this fix CLOSES #226's technical content** (durable counter is the exact mechanism #226 prescribed; ordinal now survives row deletion). Surfaced as a recommendation; agent files/edits/closes NOTHING. | The `len()` derivation #226 was filed against is removed; delete-survival acceptance met. User decides close-now vs keep-open-until-delete-path-lands. | unilaterally close/edit #226 — forbidden (CLAUDE.md; incident #161). |
| D12 | **No migration code.** Greenfield single-cut: fresh store → counter at 0; delete-the-redb-file upgrade path. | `feedback_single_cut_greenfield_migrations.md`. A "reconstruct counter from existing rows" path would re-introduce a `len()`-shaped derivation — explicitly avoided. | back-fill the counter from `len()` of existing rows — re-creates the defect. |
| D13 | **#36 (multi-node gossip) out of scope, UNCHANGED.** | Today's `len()` and the new counter are equally node-local; this fix neither improves nor regresses cross-node uniqueness. | extend the counter to a global allocator — a separate #36 design question. |

## Pinned signature (front and centre)

```rust
// overdrive-core/src/traits/observation_store.rs — additive method on `trait ObservationStore`
async fn next_issuance_ordinal(&self) -> Result<IssuanceOrdinal, ObservationStoreError>;
```

Call site (`overdrive-control-plane/src/ca_issuance.rs`, replacing lines 200-211):

```rust
let ordinal = observation
    .next_issuance_ordinal()
    .await
    .map_err(CaIssuanceError::audit)?;
```

Row construction, consumer projection, append-only guard, rkyv envelope, golden
bytes — all UNCHANGED.

## Handoff annotation

- **External integrations:** none. This is purely internal port-trait surface;
  no contract-test annotation for platform-architect.
- **Development paradigm:** object-oriented (per project CLAUDE.md) — the new
  method is a trait method; adapters are the concrete impls.
- **Crafter constraint:** the signature in § "Pinned signature" is the contract.
  Do NOT invent additional surface (no error variant, no second method, no row
  field change). If a primitive seems missing, surface a blocker — do not
  improvise (CLAUDE.md § "Implement to the design").
