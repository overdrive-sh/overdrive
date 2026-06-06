# RCA — `apply_issued_certificate` does not enforce its append-only contract

**Feature ID**: `fix-issued-cert-append-only`
**Reported via**: code review on `crates/overdrive-store-local/src/observation_backend.rs:957-965`
**RCA produced**: 2026-06-06
**User review**: PENDING (Phase 2 of `/nw-bugfix`).

## Symptom

`apply_issued_certificate` (the `issued_certificates` audit-table write path,
ADR-0063 D6) documents an **append-only** surface: the function docstring
(`observation_backend.rs:944-956`) and the trait contract on
`ObservationStore::issued_certificate_rows`
(`overdrive-core/src/traits/observation_store.rs:1185-1189`) both assert
"one row per distinct issued serial … a key collision is the issuance bug,
not an LWW case." The implementation, however, performs an **unconditional**
`table.insert()` with no existence guard:

```rust
let key = incoming.serial.as_str().as_bytes().to_vec();
let bytes = incoming.archive_for_store()?;
table.insert(key.as_slice(), bytes.as_ref())?;   // redb insert == upsert
Ok(true)
```

`redb::Table::insert` is an upsert. A second write at an already-present
serial **silently replaces** the earlier audit row with the newer bytes and
returns `Ok(true)` — which drives a post-commit broadcast
(`observation_backend.rs:377-379`). The exact outcome the docstring rules
out: the audit row is overwritten, and subscribers see a second fan-out for
a serial they already observed.

## Finding the report missed — both adapters share the defect

The bug report names only the host adapter. The **sim adapter has the
identical bug**:

```rust
// crates/overdrive-sim/src/adapters/observation_store.rs:246-250
fn apply_issued_certificate(&self, incoming: &IssuedCertificateRow) -> bool {
    let key = incoming.serial.clone();
    self.by_issued_certificate.lock().insert(key, incoming.clone()); // BTreeMap upsert
    true
}
```

Both `LocalObservationStore` (host) and `SimObservationStore` (sim) overwrite
on collision and return `true`. They are **bug-compatible** today, so a naive
fix to only the host adapter would *introduce* a host/sim divergence — exactly
the failure class `development.md` § "Trait definitions specify behavior"
warns about (the SimDataplane-vs-EbpfDataplane drift precedent). The fix must
land in **both** adapters in the same change.

## 5 Whys (two composing causes)

### Cause A — invariant documented, never enforced

- **WHY 1A** — A duplicate serial overwrites the prior audit row. *Evidence:
  `observation_backend.rs:963` unconditional `table.insert`; redb insert is an
  upsert.*
- **WHY 2A** — The code defends no precondition before inserting — no
  `table.get(key)` existence check. *Evidence: the sibling LWW writers
  (`apply_*_lww`) all read-before-write at `:926`; this function does not.*
- **WHY 3A** — The author treated "serials are CSPRNG-drawn" as proof a
  collision *cannot* happen, so no guard was written. The reasoning is in the
  docstring itself: "a key collision is the issuance bug, not an LWW case."
- **WHY 4A** — "Can't happen" was conflated with "need not be defended." But a
  duplicate can arrive from a message replay, an issuance-path retry/bug, or —
  once `issued_certificates` is gossiped (GH #36) — the **normal** idempotent
  re-delivery case that every other observation row already handles.
- **WHY 5A — ROOT CAUSE A** — The append-only invariant lives only in prose
  (function docstring + trait docstring). Nothing in the type system or the
  code body enforces it, so the contract is aspirational, not mechanical.

### Cause B — no test exercises the collision path

- **WHY 1B** — No test writes the same serial twice. *Evidence: the only
  `apply_issued_certificate` references are the two adapter definitions and
  their call sites; no duplicate-serial test exists in either crate.*
- **WHY 2B** — Existing issued-cert coverage asserts the happy path (distinct
  serials → distinct rows), which holds equally for "append-only" and for
  "blind overwrite" — the two behaviours are indistinguishable without a
  same-key write.
- **WHY 3B — ROOT CAUSE B** — The invariant is asserted nowhere at the adapter
  level, and there is no shared conformance/equivalence test driving both
  adapters through a duplicate-serial sequence. The gap between documented
  contract and implementation was therefore invisible — and was copied
  verbatim into the sim adapter.

## Contributing factor — gossip makes the collision the *expected* case

The trait docstring frames a duplicate serial as "the issuance bug." That is
true only single-node. Per GH #36, `issued_certificates` becomes gossiped like
every other observation row, at which point a peer that already holds serial
`S` **will** receive `S` again — the same idempotent re-delivery that LWW rows
treat as a no-op. The missing guard is not just a defence against a rare bug;
it is the absence of the idempotency every other row already has.

## Proposed fix

Enforce the append-only invariant in code, mirroring the LWW-reject pattern
(`Ok(false)` → caller suppresses the broadcast), in **both** adapters.

1. **Host** — `crates/overdrive-store-local/src/observation_backend.rs:957`:
   read the key first; if present, return `Ok(false)` (no insert, no
   overwrite, no emit); else insert and return `Ok(true)`.

   ```rust
   fn apply_issued_certificate(
       table: &mut Table<'_, &[u8], &[u8]>,
       incoming: &IssuedCertificateRow,
   ) -> Result<bool, ObservationStoreError> {
       let key = incoming.serial.as_str().as_bytes().to_vec();
       // Append-only: a serial already in the audit table is never
       // overwritten. Ok(false) suppresses the post-commit emit, mirroring
       // the LWW-reject path. (Duplicate = issuance replay or, post-GH #36,
       // idempotent gossip re-delivery.)
       if table.get(key.as_slice()).map_err(map_to_io)?.is_some() {
           return Ok(false);
       }
       let bytes = incoming.archive_for_store().map_err(ObservationStoreError::from)?;
       table.insert(key.as_slice(), bytes.as_ref()).map_err(map_to_io)?;
       Ok(true)
   }
   ```

2. **Sim** — `crates/overdrive-sim/src/adapters/observation_store.rs:246`:
   `if contains_key → false`, else insert + `true`. Keeps the two adapters
   observably equivalent on the duplicate path.

3. **Docstrings** — correct both function docstrings (currently "Returns
   `true` (always accepted)") to state the collision contract: a duplicate
   serial is a no-op that returns `false` and is not re-broadcast. A one-line
   strengthening of the trait `write` / `issued_certificate_rows` contract to
   name duplicate-serial-write as a no-op is in scope.

4. **Regression test (primary deliverable)** — write serial `S` with body A,
   then serial `S` with body B; assert (a) the second `write` does not
   overwrite — `issued_certificate_rows()` still returns body A — and (b) the
   second write produces no second broadcast. Mirror for the sim adapter (or a
   shared duplicate-serial assertion across both), so the parity is pinned.

## Files affected

- `crates/overdrive-store-local/src/observation_backend.rs` (fix + docstring)
- `crates/overdrive-sim/src/adapters/observation_store.rs` (fix + docstring)
- `crates/overdrive-core/src/traits/observation_store.rs` (contract docstring, minimal)
- Regression tests in both crates' test trees

## Risk assessment

**Low.** The behaviour change is confined to the collision path, which the
codebase currently treats as can't-happen; the happy path (distinct serials)
is byte-for-byte unchanged. The `accepted == false` branch is already a
first-class, tested path in both callers (host commits a read-only no-op and
suppresses emit at `:363-379`; sim's gossip enqueue is unaffected). No schema
change, no envelope-version bump, no migration. The only observable
difference is the correct one: a duplicate serial no longer destroys the
original audit row and no longer double-fans-out.
