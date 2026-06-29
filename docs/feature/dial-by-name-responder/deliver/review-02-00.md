# Adversarial review — step 02-00 (MtlsResolve re-key: `by_frontend` + three-way fail-closed classify)

- **Artifact:** roadmap step `02-00` — *MtlsResolve re-key — by_frontend (FrontendKey=(SocketAddrV4,Proto)) translation + three-way classify + ordered drain (1b-A)*
- **Commit:** `29a5f6dc`
- **Reviewer:** `nw-software-crafter-reviewer` (Opus, adversarial "different-fox" posture); all six hypotheses independently traced against source, not trusted from the dispatch.
- **Verdict:** **NEEDS_REVISION** — 2 blocking (D1 EQUIV-01 vacuous, D2 COHERENCE-01 tests the wrong ordering), 1 blocking-process (D3 no mutation gate), 2 non-blocking (N1 over-exposed `pub`, N2 truncated doc).

> Note: REKEY-01/02/03/04 and FAILCLOSED-01 are all genuinely non-vacuous and well-targeted (verified below). The two **blocking** test defects are confined to EQUIV-01 and COHERENCE-01 — both of which claim to enforce a contract they structurally cannot. This is a narrow-but-real "green suite over an unenforced criterion" rejection (CLAUDE.md § "Implement to the design — verify against the design, not just 'tests pass'"), not a wholesale failure.

---

## Per-criterion verdicts

| Criterion | Verdict | Defending test | Litmus |
|---|---|---|---|
| **REKEY-01** (frontend → first-by-Ord healthy) | **MET** | `rekey_01_frontend_hit_selects_first_by_ord_healthy_backend` (`mtls_resolve_rekey.rs:90`) | Independently recomputes `min` over the healthy set (`:110-118`) and asserts `classify == Mesh(min)`; a last-by-Ord or pick-unhealthy mutant flips it. `first_healthy_backend_for` uses `.min()` over the per-service healthy-filtered addrs (`mtls_resolve_adapter.rs:369-381`), scoped to `service_id`. |
| **REKEY-02** (frontend hit, zero-healthy → MeshUnreachable) | **MET** | `rekey_02_frontend_hit_zero_healthy_is_mesh_unreachable` (`:146`) | Generates `0..=3` all-unhealthy backends; asserts `MeshUnreachable`. Kills a mutant returning `Mesh`/`NonMesh` on the empty-healthy arm. |
| **REKEY-03** (proto axis discriminates) | **MET** | `rekey_03_frontend_key_discriminates_proto` (`:188`) | Two distinct services on `(F,P,Tcp)` and `(F,P,Udp)`; each resolves its own backend. A `Proto`-drop mutant collapses both keys → one assertion flips. `FrontendKey` carries `proto` (`:235`); `classify` keys `FrontendKey::new(orig_dst, proto)` (`:412`). |
| **REKEY-04** (by_addr preserved + NonMesh fall-through) | **MET** | `rekey_04_by_addr_path_preserved_and_nonmesh_fall_through` (`:244`) | Drives arm-3 only; asserts Mesh/MeshUnreachable per `healthy` and NonMesh for a TEST-NET-3 dst. `classify_by_addr` (`:453-461`) preserved verbatim. "running-AND-healthy" correctly maps to the single `Backend.healthy` bit — no separate `running` field (`dataplane.rs:155-160`). |
| **FAILCLOSED-01** (subnet-miss → MeshUnreachable; general-miss → NonMesh) | **MET** | `failclosed_01_subnet_miss_fails_closed_general_miss_nonmesh` (`:301`) | Empty index; Property 1 → MeshUnreachable on `10.98/16` miss, Property 2 → NonMesh out-of-subnet. Kills both flatten mutants. Arm-2 uses the single pinned `WORKLOAD_FRONTEND_BASE.contains` (`:427`) — no broader helper. |
| **COHERENCE-01** (write-time ordering barrier) | **NOT-MET (theater — wrong direction)** | `coherence_01_by_frontend_updated_before_name_index_exposes_f` (`dns_name_index.rs:1046`) | See **D2**. The fixture exposes `name_index` for ALL jobs *before* `by_frontend` is bound for ANY — the exact forbidden state — and the loop only checks the *current* job, never the cross-job lagging window. The roadmap-named mutant cannot exist in 02-00 code (the single ordered drain is 02-01). |
| **EQUIV-01** (DST host-vs-in-memory equivalence) | **NOT-MET (vacuous — `x == x`)** | `equiv_01_two_constructions_agree_on_every_classify` (`mtls_resolve_rekey.rs:412`) | See **D1**. Two `BackendIndex::default()` driven through the SAME `drive()` calls; `BackendIndex` is deterministic over `BTreeMap`s, so `verdict_a == verdict_b` is true by construction. No second implementation/oracle exists; the `prop_assert_eq!` can never fail. |

---

## Blocking issues

### D1 — `issue (blocking)`: S-DBN-EQUIV-01 is vacuous — it compares a struct against itself (`x == x`)

`mtls_resolve_rekey.rs:412-427`. The criterion (roadmap `criteria[6]`) demands proof that **"the host build path AND the in-memory test-double path"** — two distinct implementations honouring the same contract — agree at every step. The test does not do this:

- `index_a = BackendIndex::default()` and `index_b = BackendIndex::default()` (`:415-416`) are the **same type, same construction**. There is exactly ONE `BackendIndex` and ONE construction path; `drive()` calls only `apply_row` + `bind_frontend` + `classify` (`:372-398`), never `replace_from_snapshot`, so even the two intra-type write paths aren't contrasted.
- `BackendIndex` is a pure deterministic structure over `BTreeMap`s (`mtls_resolve_adapter.rs:274-300`) with **no injected nondeterminism** (no `Clock`/`Entropy`/`HashMap`-iteration). Driving two identical instances through an identical call sequence makes `verdict_a == verdict_b` **true by construction** — the `prop_assert_eq!` at `:421` is `assert_eq!(f(steps), f(steps))` and can never fail.

**Litmus (deletion test):** introduce ANY logic bug into `classify` / `first_healthy_backend_for` (swap `.min()` for `.max()`, flatten an arm, drop the proto). **`equiv_01` stays GREEN** — both `index_a` and `index_b` execute the *same* buggy code and still agree. It catches zero mutants that REKEY-01..04/FAILCLOSED-01 don't already catch; zero independent kill power. This is exactly the failure `.claude/rules/development.md` § "Trait definitions specify behavior" → "The DST equivalence test is the structural guard" warns against: *if there is only one implementation, an "equivalence" test of it against itself enforces nothing.*

The criterion's second clause — **"the verdict trajectory is deterministic (seed → bit-identical)"** — is also not isolated: agreement-of-a-thing-with-itself is not a determinism proof.

**Required (one of):**
- (a) Stand up a genuinely-independent **reference oracle** for the three-way classify (a hand-written `fn oracle(steps) -> Vec<MtlsResolution>` that re-derives the expected trajectory *without* calling `BackendIndex`), and assert `BackendIndex`'s trajectory equals the oracle's. That is the only shape that gives EQUIV-01 independent kill power. **OR**
- (b) If no second implementation is intended at this slice, **surface to the orchestrator that EQUIV-01 cannot be satisfied at 02-00** and either re-scope the criterion to a determinism-replay property with a real falsifier, or defer host-vs-sim equivalence to where a second `MtlsResolve` implementation exists. Do not ship a tautology under an equivalence criterion (CLAUDE.md § "Implement to the design — STOP and surface the gap").

### D2 — `issue (blocking)`: S-DBN-COHERENCE-01 tests the REVERSE of the ordering barrier and never observes the cross-job lagging window

`dns_name_index.rs:1046-1148` + the `index_listing` fixture (`:100-117`). The criterion (roadmap `criteria[5]`) demands: *"there is NO observable moment where `name_index` answers F for `<job>` while `by_frontend` does NOT yet hold the (F, port, proto) entry"* — i.e. **`by_frontend` FIRST (STEP A), `name_index` exposes F SECOND (STEP B)**. Three independent defects, each fatal:

**(a) The production ordering barrier does not exist in 02-00.** The test header is explicit (`:1014`): *"the single shared drain is COMPOSED in 02-01 (`run_server`…)"*, and the commit message agrees. The roadmap-named mutant — *"a mutant that exposes F to name_index before binding by_frontend"* — lives in the 02-01 drain that is **not yet written**. There is no 02-00 production code that orders the two projection writes, so there is nothing at this slice for COHERENCE-01 to gate. (The CLAUDE.md "vertical slice" tell, inverted: a test asserting an ordering invariant for a mechanism the step defers to a later slice.)

**(b) The fixture establishes the forbidden state, then asserts around it.** `index_listing` (`:100-117`) pre-`assign`s **every** `<job>` into the shared allocator (`:108`) and seeds the `NameIndex` with **all** rows (`:112`) *before* `probe()` (`:115`). By the time the loop begins, `name_index.frontend_for` answers F for **every** job; meanwhile `by_frontend` starts **empty** and is bound incrementally *inside* the loop at STEP A (`:1102-1103`). This is the **exact reverse** of the claimed order: name_index exposes F first (in the fixture), by_frontend learns it second (in the loop).

**(c) The loop only checks the current job, so the forbidden window is invisible.** Trace `{j0, j1}`. After `index_listing`, `name_index` answers F for both. Iteration `i=0`: STEP A binds `by_frontend` for `j0`, then the assertions (`:1109-1110`) query **only j0**. At that instant `name_index` already answers F for `j1` while `by_frontend` does NOT hold `j1` — *the precise forbidden moment* — but **no assertion observes `j1`** in iteration 0. `j1` is only checked in iteration `i=1`, after its own STEP A. The cross-job window is structurally invisible.

**Litmus:** there is no 02-00 production mutant this test would catch (per (a)); and it would stay GREEN even against a correct implementation of the *opposite* ordering, because (b)+(c) mean the only state ever asserted is "current job, post-STEP-A," coherent by the loop's own construction. The `else`/NxDomain branch (`:1127-1144`) asserts a `by_addr` arm-3 backend match — unrelated to the frontend-ordering barrier.

**Required (one of):**
- (a) If the barrier belongs to the 02-01 drain (the honest reading), **surface that COHERENCE-01 cannot be enforced at 02-00** and move it to 02-01 against the real drain — a test that drives the actual ordered drain and asserts, at the inter-projection observation point, that no job's `name_index` answer precedes its `by_frontend` bind. **OR**
- (b) If a Tier-1 projection-level property is wanted now, rebuild the fixture to interleave `name_index` exposure and `by_frontend` binding per the modeled drain, and assert the barrier across the **whole batch at every inter-step point** (after binding `j0` but before `j1`, assert *no* job whose `name_index` answers F lacks a `by_frontend` entry — quantify over all jobs, not just the current one).

Either way the criterion is claimed-MET but unenforced — a rejection.

### D3 — `issue (blocking)`: the mandated per-step mutation gate was never run or logged

Roadmap `criteria[9]` mandates a per-step diff-scoped run — `cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-control-plane --file crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` — at kill-rate ≥80%, naming the three-way classify arms, the first-by-Ord rule, and the ordered-drain barrier as primary kill targets. `.nwave/des-config.json` has `mutation_enabled=true`.

`execution-log.json` for `sid: 02-00` records only **PREPARE / RED_ACCEPTANCE / RED_UNIT(SKIPPED) / GREEN / COMMIT** (`:251-284`) — **no MUTATION event, no kill-rate, no `mutants-summary.json`**. This is the same gate gap flagged as D3 in `review-01-05.md` (still unresolved there), recurring at 02-00. The gate is exactly what would have surfaced D1 (EQUIV-01's lack of independent kill power). REKEY-01..04/FAILCLOSED-01 look strong enough to likely clear ≥80% on `classify`/`first_healthy_backend_for`, but the gate must *prove* it and confirm the `bind_frontend` insert path and the per-service eviction in `apply_row` are mutation-covered.

**Required:** run the mandated `--file mtls_resolve_adapter.rs` diff-scoped mutation gate, achieve ≥80% over the re-key call sites, add tests for any survivor, and log a MUTATION phase in `execution-log.json` with the recorded kill-rate (read the **guest** summary on macOS Lima — the host `target/xtask/mutants-summary.json` is stale).

---

## Non-blocking findings

### N1 — `suggestion (non-blocking)`: `BackendIndex::replace_from_snapshot` is `pub` beyond the minimal surface the tests require

`mtls_resolve_adapter.rs:341`. The pinned surface (roadmap `criteria[7]`) names the 02-00 EXTEND as `by_frontend`, `FrontendKey`, and the three-way `classify`; `apply_row`/`bind_frontend`/`classify` are `pub` because the acceptance test (a separate crate) calls them — **required**. But `replace_from_snapshot` is consumed **only inside the module** (by `relist_into`, `:543`) — no cross-crate consumer (the only `tests/` matches are `NameIndex`'s own, separate `replace_from_snapshot`). The sibling `name_index.rs` keeps the analogous methods **private** (`name_index.rs:173,227`). `BackendIndex::replace_from_snapshot` can be `pub(crate)`/private without breaking any caller — widening it to `pub` exports crate API the design did not pin. Low severity (additive, inert), but "Invent NO surface beyond this" governs visibility too. Fix: narrow `replace_from_snapshot` to the minimum, matching the `name_index` precedent.

### N2 — `nitpick (non-blocking)`: `FrontendKey::addr` doc is a truncated sentence

`mtls_resolve_adapter.rs:230-231`: *"…F drawn from [`WORKLOAD_FRONTEND_BASE`] (`10.98.0.0/16`), the port the listener's."* — ends mid-clause. Fix: "…the port is the listener's." (or drop the dangling clause). The field doc is part of the contract surface (`.claude/rules/development.md` § "Trait definitions specify behavior").

### H4 — `question` → **resolved (correct as-is)**: `replace_from_snapshot` does not clear `by_frontend`

Confirmed: it clears `by_addr` + `addrs_by_service` only (`:342-343`), leaving `by_frontend` intact. This is **correct** for the relist/recovery path: `replace_from_snapshot` is driven by `relist_into` on a `service_backends` **liveness** snapshot (`:537-545`), which carries no frontend-binding information — the `<job> → F` bindings are owned by the `FrontendAddrAllocator` (DDN-2) and projected into `by_frontend` by the (02-01) ordered drain, not by the liveness relist. Wiping `by_frontend` on a liveness relist would drop every frontend mapping on the first `Lagged` and break withhold-not-release. No fix needed; the docstring (`:341-347`) could note `by_frontend` is intentionally untouched (sub-nitpick).

---

## Praise

- **`praise:` FAILCLOSED-01 is a genuinely strong two-property structural defense.** `failclosed_01_…` (`mtls_resolve_rekey.rs:301-328`) pins BOTH directions over an empty index — subnet-miss → MeshUnreachable (kills the fail-open footgun) AND general-miss → NonMesh (kills the break-legitimate-egress mutant) — with the subnet boundary as the sole discriminator. Neither mutant survives, and `WORKLOAD_FRONTEND_BASE.contains` (`:427`) honours the "no broader helper" constraint precisely.
- **`praise:` REKEY-01 independently recomputes the oracle.** The test derives `expected_addr = min over healthy` from the generated backends *itself* (`:110-118`) rather than re-using production's selection — a real oracle for the first-by-Ord rule, killing both last-by-Ord and pick-unhealthy mutants. This is the shape EQUIV-01 *should* have borrowed.
- **`praise:` clean additive-EXTEND, correct three-way arm ordering.** Arm-1 (`by_frontend` HIT) → arm-2 (subnet `contains`) → arm-3 (`classify_by_addr` verbatim) is the pinned order (`:407-434`); the pre-REV-2 `by_addr` path is preserved unchanged (`:453-461`); `first_healthy_backend_for` is correctly scoped to `service_id` (`:369-381`) with a deterministic `.min()` tie-break.

---

## Quality-gate summary

- **G1 (one acceptance test active):** PASS.
- **G2 (AT fails for valid reason):** PASS for REKEY-01..04/FAILCLOSED-01 (production `classify` arms landed RED via `todo!`). **Suspect** for EQUIV-01/COHERENCE-01 — a tautological/wrong-direction test cannot have failed "for the right reason" in RED.
- **G6/G7 (green before/at commit):** PASS — GREEN (`19:12:57`) precedes COMMIT (`19:15:40`).
- **G8 (mutation gate ≥80%):** **FAIL** — no mutation run (D3).
- **G9 (no test weakening to fit impl):** PASS — RED→GREEN in one commit; production `mtls_resolve_adapter.rs` is in the diff alongside the tests; no fixture theater.
- **Test integrity:** **EQUIV-01 vacuous (D1); COHERENCE-01 wrong-direction theater (D2).** Two of seven criteria claimed-MET but unenforced.
- **External validity:** N/A at this slice — 02-00 is Tier-1 data-structure; the production-drivable loop is 02-01/02-02 (correctly deferred, not overclaimed).

---

## Blocking issues to resolve before approval

1. **D1** — EQUIV-01 compares `BackendIndex::default()` against itself (`x == x`, never fails). Add a genuine reference oracle for the classify trajectory, OR surface that host-vs-sim equivalence cannot be satisfied at 02-00 and re-scope/defer the criterion. No tautology under an equivalence criterion.
2. **D2** — COHERENCE-01 tests the reverse of the ordering barrier and only checks the current job, never the cross-job forbidden window; the single ordered drain it claims to gate is 02-01 code. Move COHERENCE-01 to 02-01 against the real drain, OR rebuild the Tier-1 fixture to interleave the projections and quantify the barrier over the whole batch at every inter-step point.
3. **D3** — Run + log the mandated ≥80% diff-scoped mutation gate over `mtls_resolve_adapter.rs` (guest summary on macOS Lima); add tests for any survivor; log a MUTATION phase in `execution-log.json`.

Non-blocking: **N1** (narrow `replace_from_snapshot` visibility), **N2** (fix truncated `FrontendKey::addr` doc). **H4** resolved.

---

## Summary

REKEY-01..04 and FAILCLOSED-01 are genuinely non-vacuous, well-targeted, and confirm the three-way re-key works. But the two tests the dispatch flagged are both unenforced: **EQUIV-01 is a tautology** (two identical deterministic `BackendIndex` instances driven through identical calls — `assert_eq!(x, x)`, zero independent kill power, no second implementation to compare), and **COHERENCE-01 tests the opposite of the ordering it claims** (the fixture pre-exposes `name_index` for every job before `by_frontend` is bound for any — the exact forbidden state — and the loop only observes the current job, so the cross-job lagging window the criterion forbids is structurally invisible; the single ordered drain it would gate is deferred to 02-01). Compounding both, the **mandated per-step mutation gate was never run or logged** (D3) — the gate that would have flagged EQUIV-01's lack of kill power. Two of seven criteria claimed-MET-but-unenforced, plus a missing mandated gate → **NEEDS_REVISION**.

---

## Re-review — corrective commit 69948303 (2026-06-26)

- **Scope:** adversarial re-verification of the `02-00` correctives against current HEAD — D1 (EQUIV-01 vacuous), D2 (COHERENCE-01 wrong-direction), D3 (mutation gate), N1/N2/H4, plus the REV-4 process question. All claims traced to source, not trusted from the dispatch.
- **Updated verdict:** **NEEDS_REVISION** — one blocking issue remains (**D3** — the mandated per-step mutation gate is still un-run and un-logged for 02-00). D1 and D2 are genuinely resolved (not re-greened, not goalpost-moved). One new non-blocking finding (N3) and one process finding to surface for sign-off (P1/REV-4).

### Prior-finding status

- **D1 (EQUIV-01 vacuous `x == x`) — RESOLVED.** `praise:` The rebuild is the correct shape. `ClassifyOracle` (`mtls_resolve_rekey.rs:455-549`) folds its OWN `std::collections::BTreeMap` model (`by_addr` / `addrs_by_service` / `by_frontend`) and re-derives every classify verdict WITHOUT calling `BackendIndex` / `FrontendKey` / `classify` (`:507-531`); the 10.98/16 membership is re-derived from raw octets `octets()[0] == 10 && octets()[1] == 98` (`:520`), NOT the prod const, so a `WORKLOAD_FRONTEND_BASE` const-drift mutant in production is caught; the deterministic prelude (`:580-584`) binds a frontend key to a service holding three healthy backends at distinct addrs then probes it, STRUCTURALLY forcing the arm-1 first-by-Ord tie-break every run so a `min→max` mutant diverges the trajectories. No path was found by which the oracle is secretly coupled to production — it is a genuine independent reference with real kill power (`equiv_01_classify_matches_independent_reference_oracle`, `:565-596`). This is the shape D1 demanded.

- **D2 (COHERENCE-01 wrong-direction theater) — RESOLVED (via a legitimate REV-4 simplification, security argument independently verified).** The author resolved D2 by superseding the ordering barrier in the design (REV-4) and reframing COHERENCE-01 to byte-identity (P1) + timing-independent fail-closed (P2). The security argument was independently traced and it HOLDS:
  - **No cleartext window exists.** `smallest_free_addr` (`frontend_addr_allocator.rs:114-127`) scans strictly `WORKLOAD_FRONTEND_BASE.network()+1 ..= broadcast()-1` — every DNS-answered `F` is provably inside `10.98.0.0/16`. Production arm-2 (`mtls_resolve_adapter.rs:463-465`) returns `MeshUnreachable` for ANY `by_frontend` miss whose `orig_dst.ip() ∈ 10.98.0.0/16`, regardless of inter-drain timing. The only fail-OPEN (NonMesh) path is arm-3, reachable ONLY for an IP *outside* the subnet — which a DNS-answered F can never be. So a frontend-subnet dial that races ahead of the `by_frontend` bind fails closed, never cleartext. REV-4 is a genuine over-specification removal (the barrier was an availability nicety), not a gap-hiding rationalization.
  - **P1 is non-vacuous.** P1 (`dns_name_index.rs:1108-1113`) asserts `answer_for(&mesh, A, &name_index) == records_of(&allocator, &mesh)`. `frontend_for` reads `self.allocator.snapshot().get(name)` (`name_index.rs:340`) — the SAME `Arc`-shared allocator. A mutant that made `name_index` derive `F` from the row / store / a re-derivation (a second `<job> → F` source) diverges the two and flips P1. It pins the *single-source* invariant specifically (not redundant with IDX-04).
  - **P2 kills the arm-2-flatten mutant.** P2 (`:1134-1139`) drives the last (unbound) job and asserts `classify(f_endpoint, Tcp) == MeshUnreachable`. A mutant flattening arm-2 to `NonMesh` flips it. It is the byte-identity-context companion to FAILCLOSED-01, not a pure restatement.

- **D3 (mandated mutation gate) — NOT RESOLVED (STILL BLOCKING).** `issue (blocking):` The second `02-00` round in `execution-log.json` (events at `2026-06-25T20:33–20:44`) records `PREPARE / RED_ACCEPTANCE(SKIPPED) / RED_UNIT(SKIPPED) / GREEN / COMMIT` — **no MUTATION phase, no kill-rate, no `mutants-summary.json`**; the first round had none either. No mutants artifact exists anywhere under `docs/feature/dial-by-name-responder/`. Roadmap criterion #10 (`roadmap.json:118`) STILL mandates `cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-control-plane --file crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` at kill-rate ≥80%, and `.nwave/des-config.json` has `mutation_enabled=true`. This is now doubly load-bearing: the corrective's central claim — that the new oracle has "real kill power (a `min→max` bug diverges)" — is UNPROVEN without the gate. **Required:** run the mandated `--file mtls_resolve_adapter.rs` diff-scoped gate (read the GUEST summary on macOS Lima), add tests for any survivor, and log a MUTATION phase with the recorded kill-rate.

### New findings

- **N3 — `nitpick (non-blocking)`: the separate determinism clause `equiv_01_verdict_trajectory_is_deterministic` (`mtls_resolve_rekey.rs:606-622`) is harmless-but-near-vacuous.** It drives two fresh `BackendIndex::default()` through identical calls and asserts the trajectories are equal. Its comment claims it would catch "a `HashMap` swap that reorders the first-by-`Ord` scan." It would not: `first_healthy_backend_for` uses `.copied().min()` (`mtls_resolve_adapter.rs:416`) — order-independent — and arm-1 is a point `by_frontend.get(&FrontendKey…)` (`:448`) — also order-independent. A `HashMap`-for-`BTreeMap` swap on either map would NOT change the verdict, so the determinism clause cannot kill the mutant it names; against a single deterministic `BTreeMap` structure it is `x == x` in spirit (the genuine determinism guard against `HashMap` smuggling is the `core` dst-lint gate, not this test). Not blocking — the oracle property carries the real correctness coverage. Suggestion: drop the clause, or restate its kill claim to something `.min()`/point-`get` are actually sensitive to (there isn't one at this structure shape — itself the tell that the clause adds no coverage).

- **P1 / REV-4 — `question (process — surface for explicit sign-off)`: was the security-invariant removal architect-authored and user-ratified?** `praise:` first — the REV-4 audit trail is exemplary: the superseded contract is retained VERBATIM (`adr-0072-…:533-547`), the supersession note is dated and cross-linked (`:524-532`), and the reconciliation honestly states "design reconciled to implemented reality." The security reasoning is sound (verified above). BUT two process concerns remain, and "design changed to match implementation" is the precedent-laden divergence risk CLAUDE.md § "Implement to the design" warns about:
  1. **Inline ADR edit.** REV-4 amends an accepted ADR (`adr-0072-…:133-215`) inline. The project's standing rule (CLAUDE.md / memory `feedback_delegate_to_architect`) routes ADR / design-SSOT edits through the architect agent — *"even small review-resolution edits."*
  2. **No REV-4 ratification recorded.** The amendment and commit cite REV-2 as user-ratified but say nothing about REV-4 sign-off. Removing a documented *security-relevant* invariant (Finding-3(i)) to close a review finding needs explicit user ratification even when the change is technically correct. **Required (gates the ADR change, not the tests):** confirm REV-4 was produced via the architect agent AND that the user ratified superseding the ordering barrier; if neither, route the amendment through the architect and obtain sign-off.

### Resolved non-blocking (R5)

- **N1 — RESOLVED.** `BackendIndex::replace_from_snapshot` is now private — `fn replace_from_snapshot` with no `pub` (`mtls_resolve_adapter.rs:355`), with a docstring recording the narrowing. Matches the sibling `name_index.rs` precedent; no cross-crate caller broken.
- **N2 — RESOLVED.** `FrontendKey::addr` doc is now a complete sentence: "…`F` drawn from [`WORKLOAD_FRONTEND_BASE`] (`10.98.0.0/16`); the port is the listener's." (`:230-231`).
- **H4 — RESOLVED.** The `replace_from_snapshot` docstring now explicitly states `by_frontend` is INTENTIONALLY left untouched on a liveness relist and explains why (the liveness snapshot carries no frontend-binding info; wiping it would break withhold-not-release) (`:343-349`).

### Quality-gate summary (delta from prior review)

- **G2 (AT fails for valid reason):** the second round logs `RED_ACCEPTANCE: SKIPPED` justified as "hardening existing-GREEN tests against reframed criteria — no new production behavior." Acceptable for a test-hardening + doc + visibility corrective (no production behavior changed).
- **G8 (mutation gate ≥80%):** **STILL FAIL** — no mutation run (D3).
- **G9 (no test weakening to fit impl):** PASS — the EQUIV/COHERENCE tests were STRENGTHENED (vacuous → independent oracle; wrong-direction → byte-identity + fail-closed), not weakened. The legitimate "rebuild a vacuous test into a real one" change, not a G9 violation.
- **Test integrity:** EQUIV-01 and COHERENCE-01 are now genuinely non-vacuous (D1/D2 resolved). The only residual is the near-vacuous determinism sub-clause (N3, non-blocking).
- **External validity:** N/A at this slice (Tier-1 data structure; the production-drivable loop is 02-01/02-02), unchanged.

### Remaining blocking issues

1. **D3** — Run + log the mandated ≥80% diff-scoped mutation gate over `mtls_resolve_adapter.rs` (guest summary on macOS Lima); add tests for any survivor; log a MUTATION phase in `execution-log.json`. Sole remaining blocker on test/gate grounds.

### To surface for user sign-off (gates the ADR change, not the tests)

- **P1 / REV-4** — confirm the security-invariant removal (Finding-3(i) ordering barrier) was architect-authored and user-ratified.

### Summary (re-review)

The correctives are strong, honest work — `praise:` the EQUIV-01 oracle rebuild is exactly the independent-reference shape D1 demanded (the `min→max`/arm-flatten/proto-drop mutants all diverge), and the REV-4 audit trail (verbatim retention of the superseded contract + dated supersession note + honest "reconciled to implemented reality" framing) is model discipline. The COHERENCE-01 reframe is a *legitimate* design simplification, not goalpost-moving: it was independently traced that `10.98.0.0/16` is the sole address source for every DNS-answered F and that arm-2 fail-closes every in-subnet `by_frontend` miss regardless of timing — there is no cleartext window, so the ordering barrier genuinely was an availability nicety. D1 and D2 are RESOLVED; N1/N2/H4 are RESOLVED. **But D3 is STILL OPEN** — the mandated per-step mutation gate over `mtls_resolve_adapter.rs` has never been run or logged for 02-00, and it is the gate that would *prove* the very kill-power the corrective claims. One new non-blocking test nitpick (N3) and one process item (REV-4 needs architect authorship + user ratification of the security-invariant removal). Verdict: **NEEDS_REVISION** — resolve D3 (run + log the mutation gate) and obtain REV-4 sign-off.
