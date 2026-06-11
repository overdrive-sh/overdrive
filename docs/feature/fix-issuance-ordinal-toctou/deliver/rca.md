# RCA — `IssuanceOrdinal` TOCTOU at the `issue_and_audit` seam

**Analyst:** Rex (Toyota 5 Whys RCA)
**Date:** 2026-06-11
**Subject:** code-review defect — serialized-execution invariant for the
issuance ordinal is documentation-only, not type- or runtime-enforced.
**File:** `crates/overdrive-control-plane/src/ca_issuance.rs:209-210`
(`issue_and_audit`).
**Config:** investigation_depth=5, multi_causal=true, evidence_required=true.

---

## 1. Problem definition and scope

### Problem statement (scoped)

`issue_and_audit` derives the `IssuanceOrdinal` stamped on each
`issued_certificates` audit row from
`observation.issued_certificate_rows().await.len()` immediately before
the audit write (`ca_issuance.rs:209-210`). This is a read-len → mint →
write-row **check-then-act** whose correctness depends on a precondition
("issuance is serialized through the single action-shim executor",
feature-delta § D1-AMEND-2) that exists **only in rustdoc**
(`ca_issuance.rs:153-176`). There is no type-level or runtime guard. If
any future code path issues two SVIDs concurrently against the same
`ObservationStore`, both calls read the same `len()` and stamp
**duplicate ordinals**, and the consumer-side "current cert" projection
(`handlers.rs:1034-1055`) then resolves the tie incorrectly — re-opening
the exact stale-cert-selection bug the ordinal was introduced to close.

### In scope

- The ordinal derivation site (`ca_issuance.rs:209-210`) and its two
  documented preconditions (`ca_issuance.rs:153-176`).
- The single caller path (action-shim `dispatch_issue`) and the
  serialization basis it relies on.
- The consumer projection that the ordinal feeds (`handlers.rs:1034-1055`).
- The append-only adapter guard and whether it defends the ordinal.

### Out of scope (distinct, do not conflate)

- **Precondition 2 (append-only / delete-GC).** Tracked by GH#226. A
  delete path is a *separate* trigger from concurrency; #226's literal
  acceptance is scoped to the delete path (see § Contributing factors).
- The two CAs (operator HTTPS vs. workload-identity). This is purely the
  workload-identity audit path; the operator CA is irrelevant here.

### Initial evidence gathered (all four adversarially confirmed below).

---

## 2. Adversarial validation of the supplied evidence

Each supplied claim independently checked against source — **confirm /
refute**, not taken on faith.

| # | Supplied claim | Verdict | Evidence read |
|---|---|---|---|
| 1 | TOCTOU site is `len()`-derived ordinal, read→mint→write; 2 preconditions documented | **CONFIRMED** | `ca_issuance.rs:209-210` is `IssuanceOrdinal::new(observation.issued_certificate_rows().await…len() as u64)`; preconditions rustdoc at `ca_issuance.rs:153-176` names exactly (1) single-writer/serialized and (2) append-only. |
| 2 | Sole caller is action-shim `dispatch_issue`; `issue_and_audit` is a free async fn holding no per-store state | **CONFIRMED** | Grep for `issue_and_audit` shows the only **production** caller is `action_shim/issue_svid.rs:101`; all other hits are tests, the host `RcgenCa`, and docs. Signature `issue_and_audit(ca: &dyn Ca, observation: &dyn ObservationStore, clock: &dyn Clock, node, spiffe_id)` (`ca_issuance.rs:184-190`) — no `self`, no lock, no counter. Nothing to hang a guard on. |
| 2b | The serialization basis is the runtime's sequential per-action dispatch | **CONFIRMED** | `action_shim/mod.rs:416-440` `dispatch` is `for action in actions { … dispatch_single(…).await … }` — strictly sequential, one action at a time, awaited in order. No `join!`/`spawn`/parallel fan-out inside a dispatch call. The serialization is real **today**. |
| 3 | Impact is real: `max_by_key(issuance_ordinal)` returns the LAST among equal-max; tie resolves by store iteration order → stale serial surfaces | **CONFIRMED** | `handlers.rs:1046` `.max_by_key(|c| c.issuance_ordinal)`. Rust's `Iterator::max_by_key` documents it returns the **last** element if several are equally maximum. With duplicate ordinals the "winner" is whichever the `issued_cert_rows` slice yields last — serial-keyed audit-store order, a CSPRNG draw, no recency relation. This is verbatim the bug `IssuanceOrdinal` exists to prevent (`id.rs:593-601`, `handlers.rs:1017-1025`). |
| 4 | GH#226 is scoped to delete/GC/revocation (precondition 2), NOT concurrency (precondition 1) | **CONFIRMED (by the supplied `--comments` read; I did not re-fetch)** | The rustdoc itself cites #226 ONLY against precondition 2 ("A future delete/GC path … MUST re-source the ordinal then … Tracked: overdrive-sh/overdrive#226", `ca_issuance.rs:168-176`). Precondition 1 (concurrency) has **no** issue citation anywhere in the source. Consistent with the supplied #226 scope. **The concurrency TOCTOU has no tracking issue today.** |
| 5 | Append-only guard rejects duplicate SERIALS, not ordinals; concurrent issuances mint DISTINCT serials so both rows write | **CONFIRMED** | `observation_backend.rs:1089-1105` `apply_issued_certificate` keys on `incoming.serial.as_str().as_bytes()` and returns `Ok(false)` only when **that serial** is already present. Distinct serials → both `table.insert` succeed → two rows, duplicate ordinals, both committed (and append-only ⇒ non-deletable). The serial guard is **no defense** against the ordinal TOCTOU. Refutes any hope that the landed append-only fix already covers this. |

**Validation result:** every supplied data point holds against source.
The reviewer's diagnosis is accurate. One nuance the validation
sharpens: the impact (Why-3 below) is *worse* than a plain duplicate —
because the audit log is append-only, a duplicate ordinal written under
a concurrency slip is **permanent**; there is no later correction once
both rows commit.

---

## 3. Toyota 5 Whys — multi-causal

**PROBLEM:** A documented-only serialization precondition guards the
issuance-ordinal derivation; any future concurrency that violates it
produces permanent duplicate ordinals and incorrect "current cert"
selection, as a silent correctness bug rather than a loud failure.

### Branch A — the data-correctness chain (why a slip corrupts)

```
WHY 1A: A future concurrent caller of issue_and_audit stamps duplicate ordinals.
  [Evidence: ca_issuance.rs:209-210 reads len() then writes a row; two
   interleaved calls both observe len()==N and both write ordinal N.]

  WHY 2A: The ordinal is derived by a check-then-act (read-len, act-write)
          with no atomicity across the two steps.
    [Evidence: the read (issued_certificate_rows().await) and the write
     (observation.write(...).await, ca_issuance.rs:256-259) are two
     separate awaits on the &dyn ObservationStore; a mint happens between
     them (ca.issue_svid, :238). development.md § "Check-and-act must be
     atomic" names exactly this silhouette.]

    WHY 3A: The seam holds no per-store state on which a claim/lock could
            hang; the only thing serializing it is the caller.
      [Evidence: issue_and_audit is a free async fn over borrowed trait
       objects (ca_issuance.rs:184-190) — no self, no Mutex, no atomic
       counter. The ObservationStore trait surface exposes write + read
       (observation_store.rs:1196, :1265) but no atomic
       read-and-append / next-ordinal primitive.]

      WHY 4A: The ordinal was modeled as a DERIVED projection of the audit
              log's current size (len()) rather than an INDEPENDENT durable
              monotonic source.
        [Evidence: rustdoc ca_issuance.rs:200-208 — "read from the durable
         audit log itself (the count of rows already persisted)". The design
         chose to re-derive rank from len() on every issuance, which is only
         monotonic-and-unique under BOTH a serialized writer AND an
         append-only log. This is a "persist/derive from a live count"
         decision, not "draw from a counter."]

        WHY 5A (ROOT CAUSE A): The ordinal's uniqueness was made to depend on
              two ambient environmental invariants (serialized dispatch +
              append-only) instead of being structurally guaranteed by the
              value's own source. The collision is REPRESENTABLE — the type
              system permits two rows with the same ordinal — so correctness
              is delegated to documentation and caller discipline.
        [Evidence: IssuanceOrdinal::new(u64) is infallible and unconstrained
         (id.rs:638-640); nothing in the type or the write path prevents two
         rows sharing a value. development.md § "Check-and-act must be atomic"
         doctrine: prefer making the collision UNREPRESENTABLE over relying on
         an invariant that "lives entirely in documentation."]
```

### Branch B — the silence chain (why a slip is invisible)

```
WHY 1B: If the precondition is violated, the failure is silent corruption,
        not a panic or surfaced error.
  [Evidence: the duplicate write succeeds (distinct serials pass the
   append-only guard, observation_backend.rs:1099); the consumer just
   picks the wrong row (handlers.rs:1046). No error, no log, no panic.]

  WHY 2B: There is no post-condition verification that the ordinal it just
          stamped is unique in the log.
    [Evidence: issue_and_audit returns Ok(svid) immediately after
     observation.write (ca_issuance.rs:256-261); it never re-reads to
     confirm uniqueness.]

    WHY 3B: The append-only adapter guard that DOES exist defends a
            DIFFERENT invariant (serial uniqueness), and its success masks
            ordinal duplication.
      [Evidence: observation_backend.rs:1089-1105 keys on serial; two
       distinct serials both return Ok(true). The guard's green return is
       actively misleading here — "the write succeeded" is true and
       irrelevant to the ordinal.]

      WHY 4B: The "current cert" projection silently tolerates ties — it
              applies max_by_key and takes whatever falls out, with no
              assertion that the max is unique.
        [Evidence: handlers.rs:1046 .max_by_key(...) — Rust returns the
         last equal-max element; there is no debug_assert / dedup / tie
         detection. The projection cannot tell a tie from a clean win.]

        WHY 5B (ROOT CAUSE B): The system was designed to be correct under
              the precondition, with no defense-in-depth for the case where
              the precondition is later violated — no detection turns the
              latent corruption into a loud signal. "Correct today" was
              treated as "safe forever."
        [Evidence: the only protection is the rustdoc MUST-NOT
         (ca_issuance.rs:159-167); CLAUDE.md § "Implement to the design"
         and development.md § "Check-and-act" both argue the invariant
         should be structural, not narrated.]
```

### Cross-validation

- **Root Cause A + Root Cause B are consistent, not contradictory.** A is
  *why a slip corrupts* (derived-from-len + representable collision); B is
  *why a slip is invisible* (no post-write check, misleading guard, tie-blind
  consumer). They are the two faces of one design posture: *guard by
  precondition, not by construction*. Fixing A (unrepresentable ordinal)
  dissolves B as a side effect (no collision ⇒ nothing to detect). Fixing B
  alone (detection) leaves A's collision possible but loud.
- **All symptoms explained.** The reviewer's three concrete harms —
  (i) duplicate ordinals, (ii) wrong `max_by_key` resolution, (iii) silent
  rather than panic-time — map to A1/A2 (i), A1+B4 (ii), and B1/B2/B3 (iii)
  respectively. No symptom is unexplained; no branch contradicts another.
- **Completeness check.** No third independent branch found. The DST-replay
  angle (could a duplicate ordinal also break replay-equivalence?) collapses
  into Branch B: under serialized SimClock dispatch the duplicate never
  arises, so replay is unaffected *today*; the concern is identical to
  Branch B's "violated-precondition" hypothetical, not a separate cause.

---

## 4. Verdict — present bug or latent-invariant hardening?

**This is a LATENT-INVARIANT HARDENING task, not a present bug.**

Behaviour is correct **today**:

- The sole production caller is the action-shim `dispatch_issue`
  (`issue_svid.rs:101`), reached only from the sequential
  `for action in actions { … .await … }` dispatch loop
  (`action_shim/mod.rs:416-440`). No production path issues two SVIDs
  concurrently against the same store. The `len()` read and the row write
  are therefore never interleaved, ordinals are unique, and `max_by_key`
  selects correctly.

**Is a failing regression test constructible WITHOUT violating the
documented precondition?** **No.** The precondition *is* "do not call
`issue_and_audit` concurrently for the same store." Any test that makes
the ordinal collide must call it concurrently (or two-phase-interleave a
hand-driven read/write) for one store — i.e. it must **violate
precondition 1 by construction**. There is no in-contract input that
produces the collision. Concretely:

- A `join!` of two `issue_and_audit(... same observation ...)` futures, or
- A hand-rolled interleave (call A's `issued_certificate_rows()`, hold the
  `len()`, run B fully, then complete A's write) — i.e. the
  `concurrent_submit_toctou.rs` shape adapted to this seam,

would deterministically produce two rows at ordinal N. But such a test is
**asserting on behaviour outside the contract** — it proves the seam is
unguarded, which is precisely the reviewer's point, not that today's code
is wrong. So a *red regression test against current behaviour* is not
constructible; a *test that pins a chosen fix* is (see Option A/B below).

This distinction governs prioritization: **P2** (prevention for a
potential issue), not P0/P1 (no active incident, no user impact today).
The cost of a slip is high (permanent, silent, append-only) which argues
for hardening *before* a parallel executor lands — but it is not a fire.

---

## 5. Fix options — scored against project rules

For each: public-API-surface impact, GitHub-issue/approval impact, files
touched, and rule alignment. **No option here is executed** — this RCA
recommends; the orchestrator/user decides.

### Option A — runtime detection guard at the seam (post-write ordinal-uniqueness check)

**Shape.** After the audit write succeeds, re-read
`issued_certificate_rows()` and verify the ordinal just stamped appears
**exactly once**; if it appears more than once, return a new
`CaIssuanceError` variant (e.g. `OrdinalCollision`) instead of `Ok(svid)`.
Uses only the **existing** `ObservationStore::write` +
`issued_certificate_rows` surface — no new trait methods, no new types
beyond an error-enum variant.

- **Inventing public API surface?** Borderline. A new
  `CaIssuanceError::OrdinalCollision` variant is a new *public enum
  variant* on an already-public error type. It is **not** new trait
  surface and not a new primitive — it is the typed-error discipline
  (development.md § Errors: "distinct failure modes get distinct
  variants") applied to a newly-surfaced mode. Defensible as
  *implementing the existing design's error contract*, not inventing API.
  Flag to orchestrator regardless, since CLAUDE.md § "never invent API
  surface" binds even error-enum additions if the design did not name them.
- **GitHub issue / approval needed?** No issue required to *land* A.
- **Detects AFTER the duplicate row is already committed** (append-only,
  non-deletable). So A does **not** prevent the corrupt row — it converts
  *silent* corruption into a *surfaced `Err`* at issuance time. The
  consumer projection could still, on a subsequent read before the failing
  issuance is retried, observe the duplicate. A is **mitigation
  (loud-fail), not cure**.
- **Cost.** One extra `O(N)` read per issuance (N = total issued certs,
  unbounded-append → grows forever). On the workload-start hot path. This
  is a real steady-state tax for a defense that only fires off-contract.
- **Doctrine fit.** development.md § "Check-and-act" explicitly **ranks
  detect-and-recover BELOW make-unrepresentable** ("prefer making the
  collision UNREPRESENTABLE over detect-and-recover-after-the-fact"). A is
  the dispreferred shape by the very rule that governs this defect.
- **Files affected:** `crates/overdrive-control-plane/src/ca_issuance.rs`
  (add variant + post-write verify); its acceptance/integration tests
  (`tests/integration/ca_boot_and_audit.rs`,
  `tests/acceptance/issue_svid_action_shim.rs`) to assert the new error
  fires under a forced collision.

### Option B — make it unrepresentable: durable/atomic monotonic ordinal source (the development.md-preferred shape)

**Shape.** Replace `len()`-derivation with an atomic
read-and-increment-or-append against a durable monotonic source the
adapter owns, so two concurrent issuances cannot draw the same ordinal
even when interleaved. This is the `ClaimSet`/`RaceOnceCell`-class move
(development.md § "Check-and-act" → "type the racy surface away"): the
collision becomes structurally impossible, dissolving **both** root
causes (A: no collision; B: nothing to detect).

- **Inventing public API surface?** **YES — materially.** An atomic
  next-ordinal requires either (a) a **new `ObservationStore` trait
  method** (e.g. `next_issuance_ordinal()` or an
  `append_issued_certificate_with_ordinal` that allocates atomically), or
  (b) a new durable counter primitive. Either is **new port-trait
  surface** that the accepted design (ADR-0063 D6 / feature-delta
  D1-AMEND-2) did **not** name — it modeled the ordinal as derived from
  `len()`. Per CLAUDE.md § "Implement to the design — never invent API
  surface," a crafter MUST NOT add this on its own initiative; the exact
  signature must be pinned by the architect/user first. **B cannot be
  built by a crafter without a design amendment.**
- **GitHub issue / approval needed?** B's *mechanism* (a durable monotonic
  source surviving deletion) is **exactly** what GH#226 prescribes for
  precondition 2. So B overlaps #226 even though #226's literal *trigger*
  is the delete path, not concurrency. The right move is **not** to
  unilaterally widen #226 or file a sibling — both are user-gated actions
  (CLAUDE.md § "Deferrals require GitHub issues — AND user approval BEFORE
  creation"; MEMORY: "Never create GH issues without user approval,
  incident #161"). **Recommendation to surface (see § 7):** either widen
  #226's scope to explicitly cover the *concurrency* precondition (it is
  the same durable-counter fix), or file a sibling issue — **user
  decides**; agent must not act.
- **Detects vs. prevents:** prevents (cure). No corrupt row is ever
  written.
- **Files affected:** `crates/overdrive-core/src/traits/observation_store.rs`
  (new method, with the full behaviour-contract rustdoc development.md §
  "Trait definitions specify behavior" demands);
  `crates/overdrive-store-local/src/observation_backend.rs` (host adapter:
  atomic redb txn);
  `crates/overdrive-sim/src/adapters/observation_store.rs` (sim adapter:
  matching atomic semantics + the DST equivalence contract);
  `crates/overdrive-control-plane/src/ca_issuance.rs` (drop the `len()`
  read, call the new primitive); a
  `tests/integration/<...>_equivalence` driving both adapters; schema/
  golden-bytes are unaffected (ordinal already on the row). This is a
  multi-crate, design-amendment-scoped change — **not a one-commit
  in-scope fix.**

### Option C — accept the precondition as sufficient; close won't-fix-now

**Shape.** Behaviour is correct today; the precondition is documented and
load-bearing; no production caller violates it. Close the review comment
as won't-fix-now, **optionally** recommending a tracking issue for the
concurrency precondition so a future parallel-executor author is warned.

- **Inventing public API surface?** No (no code change).
- **GitHub issue / approval needed?** Only if a tracking issue is filed —
  which is **user-gated** (same rule as B). Agent surfaces; does not file.
- **Doctrine fit.** Tolerated by the two-bar discipline
  (reconcilers.md/workflows.md "ship the floor, defer the destination
  behind a tracked issue") — but **weaker here** because the floor (the
  documented MUST-NOT) is not a *converge-on-boot* self-healing floor; it
  is a naked invariant a future change silently breaks. development.md §
  "Check-and-act" calls this exact posture out as the failure mode to
  prevent.
- **Files affected:** none (code); the rustdoc could be tightened to cite
  a (user-approved) issue against precondition 1 — currently it cites
  #226 only against precondition 2.

### Scoring summary

| | Cures? | Invents API? | Needs issue+approval? | Doctrine fit | Effort |
|---|---|---|---|---|---|
| **A** detect | No (loud-fail) | Borderline (1 error variant) | No (to land) | **Dispreferred** by § Check-and-act | Low, but hot-path O(N) tax |
| **B** unrepresentable | **Yes** | **Yes (new port-trait method)** | Yes (widen #226 / sibling) | **Preferred** by § Check-and-act | High (multi-crate + design amendment) |
| **C** accept | No | No | Only if issue filed | Tolerated, weakest | None |

---

## 6. Recommendation

**Primary: Option B is the doctrinally correct fix** — it is the
`make-the-collision-unrepresentable` move development.md § "Check-and-act"
mandates, and its mechanism is the *same durable monotonic counter* GH#226
already prescribes for the delete-path precondition. One durable atomic
ordinal source closes **both** preconditions at once.

**But B requires a design amendment the agent must NOT improvise.** A new
`ObservationStore` atomic-ordinal method is public port-trait surface the
accepted design did not name (CLAUDE.md § "Implement to the design"). The
honest path is:

1. **Surface to the user (see § 7)** that the concurrency precondition has
   **no tracking issue today** (#226 covers only the delete path), and that
   B's fix overlaps #226's mechanism. Let the user decide: **widen #226** to
   name the concurrency precondition, or **file a sibling issue**. Agent
   files nothing without explicit approval.
2. **If B is not scheduled now,** the interim posture is **C with a
   tracked issue** (not bare C): keep today's correct behaviour, but the
   rustdoc precondition-1 MUST-NOT should cite a real issue number so the
   future parallel-executor author is warned at the seam — symmetric with
   how precondition 2 already cites #226.

**Option A is NOT recommended as the primary fix.** It is the dispreferred
detect-after-the-fact shape, taxes the hot path with an unbounded `O(N)`
re-read, and only converts silence into a loud failure *after* a permanent
duplicate row already exists. Its single legitimate use is as a **cheap
interim tripwire** *if* the user wants a loud signal before B lands AND
judges the new error variant in-contract — but even then it is mitigation,
not cure, and the O(N) cost on every issuance is a poor trade for a defense
that only fires off-contract.

---

## 7. Deferral / issue recommendation — FOR THE ORCHESTRATOR TO RELAY (agent files nothing)

> **No GitHub issue created.** Per CLAUDE.md § "Deferrals require GitHub
> issues — AND user approval BEFORE creation" and MEMORY (incident #161),
> the agent must NOT run `gh issue create`. Surfacing only:
>
> **Gap:** the *concurrency* precondition (precondition 1 in
> `ca_issuance.rs:159-167`) has **no tracking issue**. GH#226 is scoped to
> the *delete/GC/revocation* precondition (precondition 2) only — confirmed
> by the rustdoc citing #226 against precondition 2 exclusively, and by the
> supplied `--comments` read of #226. A durable monotonic ordinal source
> (Option B) fixes both, but #226's literal trigger is deletion, not
> concurrency.
>
> **Decision needed from the user (pick one):**
> - **(i)** Widen GH#226's scope to explicitly include the concurrency
>   precondition (recommended — same durable-counter fix), then cite #226
>   against *both* preconditions in the rustdoc; or
> - **(ii)** File a *sibling* issue dedicated to the concurrency
>   precondition, cross-linked to #226; or
> - **(iii)** Accept Option C as won't-fix-now with no new issue (weakest;
>   leaves precondition 1 uncited).
>
> On approval of (i) or (ii), the issue is created by whoever the user
> directs — not by this agent.

---

## 8. Risk assessment

| Risk | Likelihood (today) | Impact if it fires | Mitigation |
|---|---|---|---|
| A parallel executor / test utility calls `issue_and_audit` concurrently for one store | **Low today** (sequential dispatch is the only caller) but **rising** — any future multi-threaded action dispatch, a fan-out reconciler, or a careless test introduces it | **High & permanent**: duplicate ordinals are append-only ⇒ non-deletable; wrong "current cert" surfaces a stale CSPRNG serial indefinitely; silent (no error/panic) | Option B (cure) or, interim, Option A (loud-fail) + the precondition-1 issue citation |
| Option B's new trait method diverges from the accepted design | Medium if a crafter improvises | Wrong contract propagates (the ADR-0065 precedent in CLAUDE.md) | Pin the exact signature via architect/user **before** any crafter touches it; forbid inventing surface in the dispatch |
| Option A's hot-path `O(N)` re-read regresses issuance latency as the audit log grows | Certain if A ships as-is | Steady-state issuance slowdown, unbounded with log size | Don't ship A as primary; if interim, bound or gate the re-read |
| Closing as C without an issue (bare C) | — | The unguarded seam is silently inherited by the next author; the precondition rots exactly as development.md § "Check-and-act" warns | C **must** carry a tracked precondition-1 issue, not bare close |

---

## 9. Backwards-chain validation

- **Root Cause A → symptoms.** *If* the ordinal is a representable
  `len()`-derived value guarded only by serialized dispatch, *then* a
  concurrent caller reads equal `len()` and writes duplicate ordinals
  (symptom i), which `max_by_key` resolves by iteration order (symptom ii).
  **Holds.**
- **Root Cause B → symptoms.** *If* there is no post-write uniqueness check,
  a serial-keyed guard that ignores ordinals, and a tie-blind consumer,
  *then* the corruption surfaces silently rather than as an error/panic
  (symptom iii). **Holds.**
- **No contradiction.** A and B compose; fixing A subsumes B. Every
  reviewer-named symptom is reachable from a root cause, and no root cause
  predicts a symptom that contradicts observed (correct) behaviour today.
