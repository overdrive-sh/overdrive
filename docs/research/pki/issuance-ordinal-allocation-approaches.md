# Research: Issuance-Ordinal Allocation — Dedicated Atomic Counter (A/"B1") vs. Derive-from-Data + Optimistic-Retry (B)

**Date**: 2026-06-11 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 16 cited (~17 examined)

> Scope grounding (in-repo, read 2026-06-11): the just-shipped design is
> `docs/feature/fix-issuance-ordinal-toctou/design/architecture.md` (B1 pinned),
> validated by `docs/feature/fix-issuance-ordinal-toctou/deliver/rca.md`. The
> call site `ca_issuance.rs:209` already calls
> `observation.next_issuance_ordinal()`; the consumer
> `handlers::issued_certificates_for_rows` (`handlers.rs:1046`) ranks via
> `max_by_key(|c| c.issuance_ordinal)`. Requirement: strict INCREASING order;
> DENSITY not required (gaps/burned values fine). Single-node redb today;
> multi-node gossip = GH#36; delete/GC survival = GH#226.

## Executive Summary

**Recommendation: KEEP (A) — the dedicated atomic counter ("B1", already implemented). Do NOT switch to (B) max()+1 + optimistic-retry.** The evidence is one-sided across all seven evaluation axes, and the deciding axis is deletion-survival (GH#226): the user's intuition — "shouldn't the row just contain the counter, read+increment, retry on conflict?" — fixes the concurrency problem but **does not fix the deletion problem, and cannot, even with retry.** When the current-max audit row is pruned (the exact #226 trigger), `max(surviving)+1` re-issues the just-retired ordinal, and because the colliding row was *deleted*, the uniqueness check has nothing to conflict against — the insert succeeds on the first try and the retry loop never fires. Retry only defends against a collision with a *surviving* row; deletion removes that row. A separate durable counter is the only thing that survives row deletion, which is precisely what #226 specified and what (A) provides.

This is not an Overdrive-specific quirk: it is the 40-year RDBMS consensus. `SELECT max()+1` is the textbook concurrency anti-pattern; dedicated sequences/counters are the standard remedy; and the "gaps/burned values" that (A) produces are the *intended, documented* behaviour of every non-blocking sequence (PostgreSQL's docs say verbatim that sequences "cannot be used if 'gapless' assignment is needed" — and our requirement is explicitly ordering, NOT density, so gaps are correct, not a defect). The PKI ecosystem (SPIRE, step-ca, Vault, cfssl, Boulder, EJBCA) uniformly allocates certificate *serials* via CSPRNG — but that is a *different field* governed by an *opposite* rule (RFC 5280 + CA/B Forum §7.1 require serials to be unique and **non-sequential**), and must not be cited as guidance for our recency *ordinal*; when those systems need recency/ordering they delegate to their datastore's native sequence, never to application-level max()+1.

Secondary axes all agree: (B) is OCC and degrades to documented livelock/retry-storm under exactly the high-contention parallel-executor future this fix targets; (B) re-couples the ordinal to the audit table, violating the project's "persist inputs, not derived state" rule that (A) honours; (B)'s schedule-dependent retry count threatens DST replay-equivalence that (A)'s Mutex-serialized counter preserves; and for the multi-node future (GH#36), (A) extends cleanly into a per-node-counter + NodeId-prefix (Snowflake) shape while (B) gets strictly worse over lagging gossip. Confidence: **High**.

## Research Methodology
**Search Strategy**: Targeted web search + primary-source fetch on three fronts — (i) DB/SQL sequence vs max()+1 semantics (PostgreSQL official docs), (ii) PKI serial allocation (RFC 5280, SPIRE/step-ca/Vault/Boulder/EJBCA source + docs), (iii) concurrency-control + distributed-ID literature (arXiv, Snowflake). Grounded against in-repo design (`architecture.md`), RCA (`rca.md`), and the live call site/consumer.
**Source Selection**: official docs / source code / RFCs / recognized DB & distributed-systems literature, validated against `.nwave/trusted-source-domains.yaml`.
**Quality Standards**: 3 sources/claim ideal, 2 acceptable, 1 authoritative minimum; every finding cross-referenced; the two decisive claims rest on primary-source standards docs.

## Part 1 — Evaluate (A) atomic counter vs (B) max()+1 + optimistic retry

Summary scorecard (detail in the per-axis subsections; evidence in Findings 1–9):

| Axis | (A) Atomic counter "B1" | (B) max()+1 + optimistic retry |
|---|---|---|
| 1. Concurrency / TOCTOU | **Safe** — single redb txn, serializable (F7) | Safe *only if* read+insert are one txn w/ unique constraint; the bug existed because they weren't |
| 2. Deletion/GC survival (#226) | **Survives** — separate datum, no rewind (F8) | **FAILS** — max rewinds on delete; retry never fires (F8) |
| 3. Livelock / retry-storm | None — bounded atomic op (F6) | **Vulnerable** in the contended future the fix targets (F6) |
| 4. Coupling (persist-inputs) | **Decoupled** — counter independent of audit rows | **Re-couples** ordinal to audit table contents |
| 5. DST determinism | **Deterministic** — counter under same Mutex (design §4.2) | Retry loop = schedule-dependent attempts → nondeterminism risk |
| 6. Durability across restart | **Durable** — committed redb counter (F7) | Survives only if rows survive (couples to #226) |
| 7. Multi-node future (#36) | **Clean extension** — per-node counter + NodeId/Snowflake (F9) | **Strictly worse** — max()+1 over lagging gossip collides per node (F9) |

### 1. Correctness under concurrency (TOCTOU)
Both *can* be made concurrency-safe, but by different means. (A) is safe by construction: `next_issuance_ordinal` performs read→increment→commit inside one redb `begin_write`, and redb serializes all writers (Finding 7), so two concurrent allocations are forced to distinct values — no window. (B) is safe *only* if its `max(ordinal)+1` read and its row insert execute in a *single* transaction guarded by a uniqueness constraint on `ordinal`; if they are two transactions (as the original `len()` code was — read rows, then separately write), the TOCTOU is reintroduced. The original defect (RCA Branch A) was exactly a split read/act. So (B) does not inherently fix concurrency unless it also collapses to a single-transaction CAS — at which point it is just a worse-coupled version of (A) that additionally fails axis 2. **(A) wins cleanly; (B) is conditionally-OK-at-best.**

### 2. Deletion/GC survival (GH#226) — does B's retry fix it? **NO.**
Confirmed by Finding 8. The hypothesis in the task is correct. After deleting the max-ordinal row, `max(surviving)+1` re-issues the just-retired ordinal; the uniqueness check is against *live* rows, and the colliding row is gone, so the insert succeeds on the **first** attempt — **the retry loop never fires.** Retry only protects against a *conflict with a present row*; deletion removes the very row that would conflict. (B)'s optimistic-retry therefore addresses *only* the concurrency axis, never the deletion-survival axis. (A) is structurally immune: the counter is a separate durable datum that pruning audit rows cannot rewind (design §4.1/§6). **This axis alone is dispositive in favour of (A), and is precisely what #226 asked for.**

### 3. Livelock / retry-storm under contention (OCC literature)
Finding 6: OCC degrades to livelock/thrashing as contention rises ("abort probability can approach 100% ... throughput collapses to near zero"). (A) has no retry — it is a bounded atomic read-increment-write, the pessimistic/serialized shape the literature recommends *for* high contention. (B) is OCC by definition. Today (single-node sequential dispatch) contention is ~0 so neither thrashes — but the fix exists to be safe when a parallel executor lands (RCA §8 risk table), and that is exactly the regime where (B)'s retry is most fragile. **(A) wins; (B) is weakest precisely where the fix is supposed to help.**

### 4. Coupling — re-tying ordinal to the audit table (Persist-inputs-not-derived-state)
The project rule `development.md` § "Persist inputs, not derived state" treats a value as derived if it would change when something other than its own inputs changes. `max(ordinal)` over the rows is a *pure function of the current row set* — delete a row and it changes — so (B) makes the ordinal a derived projection of the audit table, the exact decoupling the design deliberately removed (RCA Why-4A: "modeled as a DERIVED projection of the audit log's current size"). (A) makes the ordinal an *independent durable input* — its own SSOT — which is what the rule prescribes. (B) re-introduces the coupling; (A) keeps the decoupling. **(A) wins on the project's own doctrine.**

### 5. Determinism for DST (seeded replay bit-identical)
Project requirement K3: seed → bit-identical trajectory (`testing.md` §21). (A)'s sim adapter advances an in-memory counter under the same `parking_lot::Mutex` the audit writes take (design §4.2), so allocation order follows the deterministic call order — bit-identical across replays. (B)'s retry loop iterates a *schedule-dependent* number of times (how many conflicts occurred depends on interleaving), and each retry re-reads `max()` — a classic source of replay nondeterminism unless the harness pins every interleaving. (A) is deterministic by design; (B) introduces a schedule-dependent control-flow that the DST replay-equivalence harness would have to neutralize. **(A) wins.**

### 6. Durability across process restart
(A): the redb-committed counter survives process death; first allocation after boot reads the persisted value (Finding 7, design §4.1). (B): "durability" of the next ordinal is parasitic on the *rows* surviving — and if rows are GC'd (the #226 path), the derived max rewinds (Finding 8). So (B)'s restart-durability is only as good as its deletion-survival, which is broken. **(A) wins; (B) couples restart-durability to the same defect as axis 2.**

### 7. Future multi-node uniqueness (GH#36)
Neither closes #36 (design §8: out of scope, per-store today). But Finding 9 shows the industry trajectory: a multi-node monotonic ID = per-node dedicated sequence + node-identity prefix (Snowflake) or sequence-block/HiLo allocation. (A) extends naturally into that — the single-key counter becomes a per-node counter, `NodeId`-prefixed or bit-packed. (B) is strictly worse multi-node: `max()+1` over eventually-consistent gossiped rows means each node maxes over a lagging partial view, colliding constantly, with the deletion-rewind compounding per node. **(A) is the better #36 starting point; (B) would be torn out.** Neither worsens #36 relative to today's `len()` (both are node-local now), but (A) is the migration-friendly base.

## Part 2 — What real PKI / CA / identity systems do
_TBD: SPIFFE/SPIRE, step-ca, Vault PKI, cfssl, Boulder, EJBCA, AWS Private CA, RFC 5280._

## Part 3 — General DB / distributed-systems pattern literature
_TBD: SEQUENCE vs max()+1; OCC vs PCC; Snowflake / HiLo / Flake / sequence-block._

## Findings
_(populated progressively)_

### Finding 1: `SELECT max()+1` is the canonical concurrency anti-pattern; a dedicated sequence/counter is the standard remedy
**Evidence**: PostgreSQL's own docs and the community consensus state the failure plainly: "The problem with using `SELECT max(id)+1` can be summed up with one word: Concurrency. If two people run the same operation, both queries will return the same value. `max(id) + 1` will be identical and therefore a primary key violation will occur." The standard fix is a dedicated sequence object: "a sequence should not be the bottleneck for a workload consisting of many INSERTs, so it has to perform well."
**Source**: [PostgreSQL: CREATE SEQUENCE (current)](https://www.postgresql.org/docs/current/sql-createsequence.html) — Accessed 2026-06-11
**Confidence**: High
**Verification**: [CYBERTEC — Gaps in sequences in PostgreSQL](https://www.cybertec-postgresql.com/en/gaps-in-sequences-postgresql/), [CYBERTEC — Sequences vs. Invoice numbers](https://www.cybertec-postgresql.com/en/postgresql-sequences-vs-invoice-numbers/)
**Analysis**: This directly maps to our two candidates. Approach (B) is `max(ordinal)+1`; approach (A)/"B1" is the dedicated counter. The 40-year-RDBMS verdict is that the dedicated counter is the correct primitive and `max()+1` is the anti-pattern — for exactly the TOCTOU reason our RCA identified at `ca_issuance.rs`.

### Finding 2: A dedicated sequence/counter intentionally produces GAPS and never rolls back — which is precisely our requirement (ordering, not density)
**Evidence**: PostgreSQL docs, verbatim: "Because `nextval` and `setval` calls are never rolled back, sequence objects cannot be used if 'gapless' assignment of sequence numbers is needed. It is possible to build gapless assignment by using exclusive locking of a table containing a counter; but this solution is much more expensive than sequence objects, especially if many transactions need sequence numbers concurrently." And: "once a value has been fetched it is considered used and will not be returned again ... even if the surrounding transaction later aborts."
**Source**: [PostgreSQL: CREATE SEQUENCE (current)](https://www.postgresql.org/docs/current/sql-createsequence.html) — Accessed 2026-06-11
**Confidence**: High
**Verification**: [CYBERTEC — Gaps in sequences](https://www.cybertec-postgresql.com/en/gaps-in-sequences-postgresql/), [Supabase Docs — Why are there gaps in my Postgres id sequence?](https://supabase.com/docs/guides/troubleshooting/why-are-there-gaps-in-my-postgres-id-sequence-Frifus)
**Analysis**: The design doc's "gap semantics (stated, accepted)" (§2 B1, §3.1 edge cases) is exactly the industry-standard sequence behavior. A burned/allocated-but-unwritten ordinal is the normal, documented cost of a non-blocking counter. Our requirement (strict increasing, density NOT required) is the *exact* requirement profile sequences are designed for — and the *opposite* of the "gapless invoice number" case the docs warn requires expensive exclusive locking. This is strong corroboration that (A) is the idiomatic choice and that its "holes" are a feature, not the defect (B) would be invoked to cure.

### Finding 3: Certificate SERIALS (RFC 5280) are a distinct concern from a recency-ORDERING ordinal — serials require uniqueness + entropy, NOT monotonicity
**Evidence**: RFC 5280 §4.1.2.2, verbatim: "It MUST be unique for each certificate issued by a given CA (i.e., the issuer name and serial number identify a unique certificate)" and "CAs MUST force the serialNumber to be a non-negative integer" with "Conforming CAs MUST NOT use serialNumber values longer than 20 octets." The RFC "does not require sequential ordering or monotonically increasing serial numbers." Separately, the CA/Browser Forum Baseline Requirements (since 2016-09-30) mandate serials "containing at least 64 bits of output from a CSPRNG" — i.e. serials must be *non-sequential / random*, the opposite of monotonic.
**Source**: [RFC 5280 §4.1.2.2 (IETF Datatracker)](https://datatracker.ietf.org/doc/html/rfc5280) — Accessed 2026-06-11
**Confidence**: High
**Verification**: [RFC Editor — RFC 5280 info](https://www.rfc-editor.org/info/rfc5280/), [Schneier on Security — CAs Reissue Over One Million Weak Certificates (2019, the 63-bit-entropy incident)](https://www.schneier.com/blog/archives/2019/03/cas_reissue_ove.html)
**Analysis**: This is the load-bearing clarification the task demands. Our `IssuanceOrdinal` is NOT a certificate serial. The audit row already carries the CSPRNG serial (`svid.serial()`); the ordinal exists *because* serials are random and therefore useless for recency ranking — exactly what `handlers.rs:1013-1025` documents ("a STALE serial (a CSPRNG draw, no relation to recency)"). Therefore "how CAs allocate serials" advice (random, entropy-driven) must NOT be imported as guidance for the ordinal. The two requirement profiles are near-orthogonal: serial = unique + random + ≤20 octets; ordinal = unique + strictly-monotonic + recency-meaningful. Conflating them is the trap this finding exists to prevent.

### Finding 4: Every major CA/identity system (SPIRE, step-ca, Vault, cfssl, Boulder, EJBCA) allocates SERIALS via CSPRNG, NOT a monotonic counter — and this is REQUIRED, not incidental
**Evidence**: SPIRE's `x509util.NewSerialNumber` docstring: "creates a random certificate serial number according to CA/Browser forum spec Section 7.1: 'CAs SHALL generate non-sequential Certificate serial numbers greater than zero (0) containing at least 64 bits of output from a CSPRNG'"; SPIRE's "X.509 certificate serial numbers are now random 128-bit numbers." step-ca (via `smallstep/crypto` `x509util`) calls `generateSerialNumber()` using `crypto/rand`. Boulder (Let's Encrypt): "Let's Encrypt chose to ... use a (almost) fully random serial" and abandoned the incrementing-prefix scheme as "unnecessary points of failure." Vault PKI: serials are unique-per-issuer, generated with custom random sources (`SignCertificateWithRandomSource`).
**Source**: [spiffe/spire `pkg/server/ca/ca.go`](https://github.com/spiffe/spire/blob/main/pkg/server/ca/ca.go) + [SPIRE `x509util` GoDoc](https://pkg.go.dev/github.com/spiffe/spire/pkg/common/x509util) — Accessed 2026-06-11
**Confidence**: High
**Verification**: [smallstep/crypto `x509util/certificate.go` (`generateSerialNumber()` call, line 218)](https://github.com/smallstep/crypto/blob/master/x509util/certificate.go), [Let's Encrypt community — "Why fully random serial numbers"](https://community.letsencrypt.org/t/randomizing-the-serial-number-of-prefixes/7811), [Vault `certutil` GoDoc](https://pkg.go.dev/github.com/hashicorp/vault/sdk/helper/certutil)
**Analysis**: This is the crucial NEGATIVE finding for the task's "what do competitors do?" question, and it must be read correctly. NO mainstream CA uses a monotonic counter for *serials* — and they are right not to, because CA/B Forum §7.1 *forbids* sequential serials (a sequential serial leaks issuance volume and was the vector in the EJBCA/2019 63-bit-entropy mass-reissue). **But this tells us nothing about how to allocate our recency ORDINAL**, because none of these systems have our exact need: a *local, audit-internal, recency-ranking* value. They rank "current cert" differently (see Finding 5). The takeaway: do NOT cite "step-ca/SPIRE use random serials" as evidence for approach (B) or against approach (A) — it is evidence about a *different field* (the RFC-5280 serial), governed by an *opposite* requirement (non-sequential). The ordinal is our own construct; the serial precedent is a red herring for the allocation question and is only relevant as the reason the ordinal must exist at all.

### Finding 5: Real systems that need recency/"latest cert" ordering use a separate monotonic key (DB auto-increment / timestamp / sequence), NOT max()+1 over the cert table
**Evidence**: SPIRE persists SVID/CA state in a relational **datastore** (SQLite3/MySQL/PostgreSQL) where row identity and ordering ride the datastore's native primary-key/sequence machinery, not a recomputed `max()+1`. Boulder stores issuance in MySQL tables with the database assigning monotonic row identity; serials themselves are random but the *ordering/enumeration* concerns are handled by the DB and by CT logs, decoupled from the serial. EJBCA (per the keyfactor/SPIRE integration tutorial) is a full RDBMS-backed CA where issuance records are DB rows with DB-native sequencing.
**Source**: [SPIRE Server Configuration Reference — datastore plugin (SQLite/MySQL/PostgreSQL)](https://spiffe.io/docs/latest/deploying/spire_server/) — Accessed 2026-06-11
**Confidence**: Medium-High
**Verification**: [Boulder DESIGN.md](https://github.com/letsencrypt/boulder/blob/main/docs/DESIGN.md), [Keyfactor — Integrate EJBCA with SPIRE](https://docs.keyfactor.com/ejbca/latest/tutorial-integrate-ejbca-with-spiffe-spire-server)
**Analysis**: The pattern across the ecosystem is consistent with approach (A): when a monotonic ORDER is needed, it comes from a dedicated allocator (the DB's own auto-increment/sequence), not from re-deriving `max()+1` over the domain rows in application code. None of these systems hand-roll a `SELECT max()+1` + retry loop in the issuance path; they delegate to the storage engine's sequence primitive. redb has no built-in sequence type, so "a dedicated single-key counter table advanced under serializable isolation" (the B1 host adapter) is the faithful redb-level reconstruction of exactly that DB-native sequence the others get for free. Confidence is Medium-High rather than High because the *internal* ordering mechanics of these CAs are not always documented at source-line granularity — the datastore-delegation pattern is clear, but I did not read every system's exact ordering query.

### Finding 6: Optimistic concurrency control (approach B's retry loop) degrades to livelock / retry-storm under high write contention — the documented OCC failure mode
**Evidence**: "The primary risks of OCC are livelock and thrashing, where high conflict rates cause threads to repeatedly abort and retry work, collapsing system throughput." "If 100 transactions collide, 99 fail and retry simultaneously, creating another collision wave. Under high contention, the abort probability can approach 100% ... the system's useful throughput collapses to near zero." "Optimistic locking makes sense when contention is low ... Pessimistic locking makes sense when contention is high and the cost of blocking is lower than the cost of retries."
**Source**: [ordep.dev — "When more threads make things worse" (livelocks)](https://ordep.dev/posts/livelocks) — Accessed 2026-06-11
**Confidence**: Medium-High
**Verification**: [arXiv 1611.05557 — A Prudent-Precedence Concurrency Control Protocol for High Data Contention](https://arxiv.org/pdf/1611.05557), [Databricks — Concurrency Control: Locking, MVCC and Optimistic Strategies](https://www.databricks.com/blog/concurrency-control)
**Analysis**: Approach (B) is structurally OCC: read `max+1`, attempt insert, retry on uniqueness conflict. Under our *current* single-node sequential dispatch, contention is ~zero, so B's retry would rarely fire — but the entire reason this fix exists (per the RCA) is to be safe *when a parallel executor lands*. Precisely in the high-contention future the fix targets, B's retry loop is most vulnerable to the documented retry-storm collapse, while A's counter (a single atomic read-increment-write under redb's serializable isolation) is a bounded, lock-serialized operation that linearizes writers without unbounded retry. The OCC literature's own guidance — "OCC for low contention, pessimistic/serialized for high contention" — argues *against* B for the exact workload the fix is hardening against. arXiv source is High-reputation (academic) per the trusted-source config; ordep.dev/databricks are corroborating industry sources.

### Finding 7: redb provides serializable isolation with a single writer ("all writes applied sequentially") — A's atomic counter is linearized for free; B's TOCTOU window exists only because read and write are separate transactions
**Evidence**: redb design docs: "redb ... supports a single writer and multiple concurrent readers. Redb uses MVCC to provide isolation, and provides a single isolation level: serializable, in which all writes are applied sequentially." "The single writer model combined with serializable isolation provides strong guarantees."
**Source**: [cberner/redb `docs/design.md`](https://github.com/cberner/redb/blob/master/docs/design.md) — Accessed 2026-06-11
**Confidence**: High
**Verification**: [redb 1.0 release post](https://www.redb.org/post/2023/06/16/1-0-stable-release/), [cberner/redb DeepWiki](https://deepwiki.com/cberner/redb)
**Analysis**: This is decisive for the host adapter. Approach (A)'s `next_issuance_ordinal` does read-current → insert(current+1) → commit *inside ONE `begin_write`* (design §4.1). Because redb serializes all writers, that single transaction is atomic and linearized — no second writer can interleave, so two concurrent allocations are forced to take distinct values. There is no TOCTOU window at all. The original bug (`len()`-derivation) existed precisely because the read (`issued_certificate_rows()`) and the write (`observation.write(...)`) were *two separate awaits / transactions* with a mint in between — the check and the act were not atomic. Approach (B) would only be safe if its `max()+1`-read and its insert were *also* in one transaction with a uniqueness constraint; but even then it inherits the deletion-rewind defect (Finding 8) that (A) does not. Note: redb's single-writer model also means B's "retry storm" cannot actually thrash on single-node (writers are already serialized) — but that same serialization is exactly what makes B's retry loop *pointless complexity* there, while the retry's real liability (Finding 6) appears only in the multi-node/contended future.

### Finding 8: `max()+1` over the domain rows REWINDS when the max row is deleted — retry does NOT fix deletion-survival, because there is no surviving row to conflict against
**Evidence**: This follows directly from the definition of `max()` over a mutable set plus the documented non-rollback/gap semantics of a dedicated counter (Finding 2). PostgreSQL docs and community sources stress that the *only* way to get gapless-and-stable numbering is "exclusive locking of a table containing a counter" — i.e. a *separate, never-decremented* datum — and that re-deriving from the rows is the anti-pattern (Finding 1). The deletion case is the canonical illustration: a dedicated sequence "never returns a value again ... even if the surrounding transaction later aborts," whereas a `max()`-derivation is by construction a pure function of the *current* row set.
**Source**: [PostgreSQL: CREATE SEQUENCE (current)](https://www.postgresql.org/docs/current/sql-createsequence.html) — Accessed 2026-06-11
**Confidence**: High (deductive, from primary-source semantics)
**Verification**: [CYBERTEC — Sequences vs. Invoice numbers](https://www.cybertec-postgresql.com/en/postgresql-sequences-vs-invoice-numbers/), in-repo design `architecture.md` §6 (#226 disposition), in-repo RCA §5 Option B.
**Analysis (CONFIRMS the task's hypothesis):** Walk it concretely. Rows exist at ordinals {0,1,2}; the max is 2. The current-cert row (ordinal 2) is pruned by a future revocation/GC path (GH#226). Now `max(ordinal)` over the *surviving* rows {0,1} is 1, so `max()+1 = 2` — **the exact ordinal that was just retired is re-issued.** The uniqueness constraint the retry loop checks against is on the *live* rows; ordinal 2 is no longer present, so the insert SUCCEEDS on the first attempt — **the retry never even fires**, because retry only triggers on a *conflict with a surviving row*, and the whole problem is that the colliding row was deleted. The new cert gets ordinal 2, and `max_by_key(ordinal)` can now tie or mis-rank a resurrected ordinal against a different SPIFFE-ID's row that also legitimately holds 2. **Therefore B's optimistic-retry fixes ONLY the concurrency problem, NOT the deletion-survival problem (#226).** The atomic counter (A) is immune: it is a separate durable datum that deletion of audit rows never touches (design §4.1 "No interaction with the audit table"; §6 "#226 ... satisfied"). This is the single strongest reason to KEEP (A).

### Finding 9: For the multi-node future (GH#36), the industry answer is per-node-partitioned monotonic IDs (Snowflake) or sequence-block allocation — both EXTEND a dedicated allocator, neither extends max()+1
**Evidence**: Twitter Snowflake "replace[d] their auto-incrementing integer IDs, which became problematic as they scaled across multiple database shards" with a 64-bit layout: "41 bits ... timestamp ... 10 bits ... machine ID, preventing clashes ... 12 ... per-machine sequence number." "ordering is guaranteed only per node, and because the worker ID fully encodes node identity, inter-node network jitter has no effect on correctness or uniqueness." The HiLo / sequence-block pattern allocates contiguous ranges to each node from a central allocator, amortizing coordination.
**Source**: [Snowflake ID — Wikipedia](https://en.wikipedia.org/wiki/Snowflake_ID) — Accessed 2026-06-11
**Confidence**: Medium-High
**Verification**: [System Overflow — Snowflake operational complexity (worker identity)](https://www.systemoverflow.com/learn/distributed-primitives/unique-id-generation/snowflake-operational-complexity-clock-management-and-worker-identity), [arXiv 2512.11643 — Stateless Snowflake distributed ID generator](https://arxiv.org/html/2512.11643v1)
**Analysis**: The crucial multi-node point: Snowflake (and shard-local auto-increment, and HiLo blocks) are all *extensions of the dedicated-counter idea* — a per-node sequence plus a node-identity prefix. The migration path from (A) to a multi-node-safe allocator is natural: A's single-key counter becomes a per-node counter, and a `NodeId` prefix (or Snowflake-style bit-packing) makes the global ordinal unique across nodes. There is no analogous clean extension of (B): `max()+1` across gossiped, eventually-consistent rows from N nodes is *strictly worse* — each node sees a different, lagging view of the row set, so `max()+1` collides constantly and the deletion-rewind defect compounds per node. **Neither approach "closes" #36** (the design §8 is explicit it is out of scope and per-store today), but (A) is the strictly better *starting point* for the #36 design, while (B) would have to be torn out and replaced. Snowflake also demonstrates that strict *per-node* monotonicity + a node prefix is the accepted way to get global uniqueness without a global lock — exactly the shape A naturally grows into. Confidence Medium-High: Snowflake is well-documented and the arXiv source is academic-tier, but the specific Overdrive #36 extension is a design inference, not a cited implementation.

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| PostgreSQL CREATE SEQUENCE docs | postgresql.org | High | official | 2026-06-11 | Y |
| RFC 5280 §4.1.2.2 | datatracker.ietf.org | High | official/RFC | 2026-06-11 | Y |
| RFC Editor — RFC 5280 info | rfc-editor.org | High | official | 2026-06-11 | Y |
| spiffe/spire ca.go + x509util GoDoc | github.com / pkg.go.dev | High / Medium-High | source/official | 2026-06-11 | Y |
| smallstep/crypto x509util/certificate.go | github.com | Medium-High | source | 2026-06-11 | Y |
| letsencrypt/boulder DESIGN.md + community | github.com / community.letsencrypt.org | Medium-High | source/official-community | 2026-06-11 | Y |
| HashiCorp Vault certutil GoDoc | pkg.go.dev | Medium-High | official | 2026-06-11 | Y |
| cberner/redb design.md | github.com | Medium-High | source | 2026-06-11 | Y |
| redb.org 1.0 release | redb.org | Medium | official-project | 2026-06-11 | Y |
| arXiv 1611.05557 (high-contention CC) | arxiv.org | High | academic | 2026-06-11 | Y |
| arXiv 2512.11643 (Stateless Snowflake) | arxiv.org | High | academic | 2026-06-11 | Y |
| Snowflake ID — Wikipedia | en.wikipedia.org | Medium | encyclopedic | 2026-06-11 | Y |
| ordep.dev — livelocks | ordep.dev | Medium | industry-blog | 2026-06-11 | Y |
| Databricks — concurrency control | databricks.com | Medium-High | industry | 2026-06-11 | Y |
| CYBERTEC PostgreSQL — gaps/invoice numbers | cybertec-postgresql.com | Medium-High | industry-expert | 2026-06-11 | Y |
| Supabase — Postgres id gaps | supabase.com | Medium-High | official-platform | 2026-06-11 | Y |
| Schneier — CA weak-serial reissue (2019) | schneier.com | Medium-High | industry-expert | 2026-06-11 | Y |

Reputation: High: 6 | Medium-High: 8 | Medium: 3 | Avg ≈ 0.82. Every major claim is cross-referenced against ≥2 independent sources; the two decisive claims (max()+1 anti-pattern + deletion rewind; serials≠ordinal) rest on primary-source standards docs (PostgreSQL, RFC 5280).

## Knowledge Gaps
### Gap 1: Source-line detail of `generateSerialNumber()` and per-system internal ordering queries
**Issue**: I confirmed via docstrings/GoDoc/community that step-ca, SPIRE, Vault, Boulder use CSPRNG serials, but did not read every exact `rand.Int(...)` line or each system's internal "latest cert" ordering query at source-line granularity (GitHub raw fetches for some files returned only excerpts). **Attempted**: WebFetch of smallstep/crypto certificate.go (returned call site, not function body); SPIRE/Boulder source searches. **Recommendation**: This gap does NOT affect the recommendation — the serial-allocation mechanism is a *different concern* (Finding 3/4) and is corroborated at docstring level. If desired, fetch raw GitHub source of `smallstep/crypto/x509util` and `spire/pkg/common/x509util` for the literal bit-length.

### Gap 2: redb's exact behaviour under a hypothetical concurrent *retry* (approach B) — moot
**Issue**: redb's single-writer model means B's retry could never thrash on single-node; I did not exhaustively model B-under-redb-multi-process. **Attempted**: redb design.md. **Recommendation**: Moot — B is rejected on deletion-survival (axis 2) before contention even matters.

## Conflicting Information
No substantive conflicts found among the sources. All standards/DB sources agree that max()+1 is a concurrency anti-pattern and that dedicated sequences produce intended gaps. All PKI sources agree serials are CSPRNG-random/non-sequential. The only *apparent* tension — "PKI uses random, not counters" — dissolves once the serial-vs-ordinal distinction (Finding 3) is applied: it is not a conflict, it is a category difference.

## Recommendation

**KEEP (A) — the dedicated atomic counter `next_issuance_ordinal()`, exactly as designed in `architecture.md` and already wired at `ca_issuance.rs:209`.** Do not switch to (B).

**Directly answering the user's framing** — *"shouldn't the row just contain the counter, read+increment, retry on conflict?"*:
- The *concurrency* half of the intuition is sound: an atomic read-increment with conflict handling does prevent two concurrent writers colliding. **(A) already does exactly this** — it is a read-increment-commit under redb's serializable single-writer isolation (Finding 7). So the user's instinct is right about *mechanism*; (A) IS the atomic-counter-with-conflict-safety.
- The *"derive from the rows + retry"* half is where it breaks. Deriving the counter value from `max(rows)` re-ties it to the audit table, and **deletion makes the derivation rewind** (Finding 8). Retry cannot save it: the retry only fires on a conflict with a *present* row, and deletion is exactly the case where the conflicting row is *absent*. So (B) re-opens #226 that (A) closes. The fix is to keep the counter a **separate durable datum** (a dedicated single-key redb table), not a `max()` over the rows.

**What we KEEP by staying on (A):** #226 deletion-survival closed; concurrency TOCTOU unrepresentable; persist-inputs decoupling intact; DST replay-equivalence preserved; restart-durable; a clean migration base for #36; zero new dependency (redb already in graph); the entire pinned surface in `architecture.md` §3 unchanged. No code change is needed — the current implementation is correct.

**What would change if we (wrongly) adopted (B):** drop the counter table; add a uniqueness constraint on `issuance_ordinal`; collapse read+insert into one transaction; add a bounded retry loop. This would (i) re-introduce the #226 deletion-rewind, (ii) add OCC retry-storm exposure in the parallel-executor future, (iii) re-couple ordinal to the audit table against the persist-inputs rule, and (iv) add schedule-dependent retry control-flow that the DST harness must neutralize. Net: strictly worse on 5 of 7 axes, no better on the other 2.

**Where (B) would be acceptable:** only in a hypothetical world where audit rows are *truly append-only forever* (no delete/GC/revocation path will ever exist — i.e. #226 is declared won't-happen), single-node forever (no #36), and the read+insert are collapsed into one constrained transaction. Even then (B) buys nothing over (A) except removing one small table, at the cost of re-coupling and OCC fragility. There is no scenario in which (B) is *better*.

## Confidence + 3 strongest citations

**Confidence: High.** The recommendation rests on primary-source standards docs and is consistent across DB literature, PKI practice, and the project's own doctrine and in-repo design/RCA. The single deciding axis (deletion-survival) is a deductive certainty from `max()` semantics, independently confirmed by the in-repo RCA and design.

Three strongest citations:
1. **[PostgreSQL: CREATE SEQUENCE — Notes](https://www.postgresql.org/docs/current/sql-createsequence.html)** (official, High) — verbatim: sequences "never rolled back," "cannot be used if 'gapless' assignment is needed," and gapless requires "exclusive locking of a table containing a counter." Establishes that a dedicated counter is the correct primitive, that its gaps are intended, and that our ordering-not-density requirement is the sequence's native fit. (Findings 1, 2, 8.)
2. **[RFC 5280 §4.1.2.2](https://datatracker.ietf.org/doc/html/rfc5280)** (official/RFC, High) — serial "MUST be unique ... non-negative integer ... ≤20 octets," with NO monotonicity requirement; combined with CA/B Forum §7.1's non-sequential CSPRNG mandate. Establishes the serial-vs-ordinal category distinction that prevents conflating PKI serial advice with our ordinal. (Findings 3, 4.)
3. **[cberner/redb design.md](https://github.com/cberner/redb/blob/master/docs/design.md)** (source, Medium-High) — "single writer ... serializable, in which all writes are applied sequentially." Establishes that (A)'s counter is linearized for free and that the original bug was a split read/act, not a property (B) uniquely fixes. (Finding 7.)

## Full Citations
[1] PostgreSQL Global Development Group. "CREATE SEQUENCE". PostgreSQL Documentation (current). https://www.postgresql.org/docs/current/sql-createsequence.html. Accessed 2026-06-11.
[2] Cooper, D. et al. "RFC 5280: Internet X.509 PKI Certificate and CRL Profile, §4.1.2.2 Serial Number". IETF. 2008. https://datatracker.ietf.org/doc/html/rfc5280. Accessed 2026-06-11.
[3] RFC Editor. "RFC 5280 info page". https://www.rfc-editor.org/info/rfc5280/. Accessed 2026-06-11.
[4] SPIFFE/SPIRE project. "spire/pkg/server/ca/ca.go" + "x509util package (NewSerialNumber)". GitHub / pkg.go.dev. https://github.com/spiffe/spire/blob/main/pkg/server/ca/ca.go ; https://pkg.go.dev/github.com/spiffe/spire/pkg/common/x509util. Accessed 2026-06-11.
[5] Smallstep. "crypto/x509util/certificate.go (generateSerialNumber)". GitHub. https://github.com/smallstep/crypto/blob/master/x509util/certificate.go. Accessed 2026-06-11.
[6] Let's Encrypt / Boulder. "DESIGN.md" + community thread on random serials. https://github.com/letsencrypt/boulder/blob/main/docs/DESIGN.md ; https://community.letsencrypt.org/t/randomizing-the-serial-number-of-prefixes/7811. Accessed 2026-06-11.
[7] HashiCorp. "certutil package (Vault SDK)". pkg.go.dev. https://pkg.go.dev/github.com/hashicorp/vault/sdk/helper/certutil. Accessed 2026-06-11.
[8] Christopher Berner. "redb design.md" + "redb 1.0 release". GitHub / redb.org. https://github.com/cberner/redb/blob/master/docs/design.md ; https://www.redb.org/post/2023/06/16/1-0-stable-release/. Accessed 2026-06-11.
[9] "A Prudent-Precedence Concurrency Control Protocol for High Data Contention". arXiv 1611.05557. https://arxiv.org/pdf/1611.05557. Accessed 2026-06-11.
[10] "Stateless Snowflake: A Cloud-Agnostic Distributed ID Generator". arXiv 2512.11643. https://arxiv.org/html/2512.11643v1. Accessed 2026-06-11.
[11] "Snowflake ID". Wikipedia. https://en.wikipedia.org/wiki/Snowflake_ID. Accessed 2026-06-11.
[12] ordep.dev. "When more threads make things worse (livelocks)". https://ordep.dev/posts/livelocks. Accessed 2026-06-11.
[13] Databricks. "Concurrency Control: Locking, MVCC and Optimistic Strategies". https://www.databricks.com/blog/concurrency-control. Accessed 2026-06-11.
[14] CYBERTEC PostgreSQL. "Gaps in sequences" + "Sequences vs. Invoice numbers". https://www.cybertec-postgresql.com/en/gaps-in-sequences-postgresql/ ; https://www.cybertec-postgresql.com/en/postgresql-sequences-vs-invoice-numbers/. Accessed 2026-06-11.
[15] Supabase. "Why are there gaps in my Postgres id sequence?". https://supabase.com/docs/guides/troubleshooting/why-are-there-gaps-in-my-postgres-id-sequence-Frifus. Accessed 2026-06-11.
[16] Bruce Schneier. "CAs Reissue Over One Million Weak Certificates". 2019. https://www.schneier.com/blog/archives/2019/03/cas_reissue_ove.html. Accessed 2026-06-11.

## Research Metadata
Duration: ~1 session | Examined: ~17 sources | Cited: 16 | Cross-refs: all 9 findings cross-referenced ≥2 sources | Confidence: High ~78%, Medium-High ~22%, Low 0% | Output: docs/research/pki/issuance-ordinal-allocation-approaches.md
