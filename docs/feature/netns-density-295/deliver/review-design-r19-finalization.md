# DESIGN Review — R19 native-falsification FINALIZATION

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Review | `arch_rev_20261003_netns295_r19_finalization` |
| Reviewer | Fresh independent `nw-solution-architect-reviewer` (continuation of `arch_rev_20261003_netns295_r19_native_falsification`) |
| Model | Opus 4.8 (1M) |
| Iteration | 1 (finalization) |
| Date | 2026-10-03 |
| Source snapshot | HEAD `eaa9a2bc` on `marcus-sa/spike-netns-density-295-v1`, with the nine integrated DESIGN artifacts modified in the working tree; native receipt `.context/named-service-e14e-run-08-01.log` (nextest `f03c43d5-ffcc-4bec-8bd3-9b98f90e334a`, 2 passed) |
| Scope | The finalized R19 decision + its integration across nine operative DESIGN artifacts; the killed-mode/E14(e) determination the prior review held pending; adversarial security-weakening gate |
| **VERDICT** | **CHANGES_REQUESTED** — narrow, surgical documentary reconciliation only. The core decision is SOUND. |
| Findings | 0 critical, 1 high, 1 medium, 1 low |
| Killed-mode residual | **CLOSED** — independently verified from test source + native log |
| Semantic-weakening gate | **PASS** — no security outcome removed, moved, synonym-swapped, or relaxed |
| New material user decision required | **NO** |

I verified every claim against the test source and the native log rather than taking the architect's assertions on faith. The single most important security claim — that the E14(e) door test is *itself* a killed-mode scenario, closing the killed-mode residual without any runtime-quiescence remedy — holds. The verdict is CHANGES_REQUESTED **solely** because the finalization's own normative-diff gate ("every operative artifact must carry the precise new status and wording") is not fully met: two operative DESIGN clauses still assert the *withdrawn* R19 reorder as the current fail-closure mechanism and label the listener fail-closure "conditional on native RED." These are cheap inline fixes, not decision defects.

---

## Evidence verification (checked against test source + `f03c43d5` log)

| Claim to verify | Source of truth | Verified? |
|---|---|---|
| Door test itself kills the serve owner (~line 2558) | `intercept_mark_fail_closed.rs:2558` `kill_serve_owner(handle).await.expect("killed-mode serve abandons its owner")` | **YES** — exact line 2558 |
| `kill_serve_owner` is genuine owner-death, no quiescence | `serve_lifetime_support.rs:132-139` — delivers `ServeSignal::Kill`, runs `ServeLifetime::run(handle)`, requires `Ok(ServeExit::Killed)`; doc: "the single in-process process-loss mechanism" | **YES** — no graceful stop, no quiesce step on this path |
| TAP up after the kill | `:2560` `assert!(tap_is_up(&guest_tap), "the guest's TAP stays up after killed mode")`; re-asserted `:2593` | **YES** |
| Program/members/policy-route survive the kill | `:2561-2565` `program_at_fault == program_before`; `:2662-2666` `program_after == program_before`; `assert_r19_program_live` (`:936-963`) checks program present + managed + outbound + fwmark 0x1/table 100 + local route | **YES**; log lines 52-53,56 show `managed=true, outbound=true, fwmark_rule=true, local_route=true` |
| TIME_WAIT entry survives the kill *before* any reconnect | `:2566-2571` reads the exact 4-tuple via `ss -tan state time-wait` after the kill and asserts it still contains the guest addr | **YES** |
| True TIME_WAIT on the Service-VIP tuple (named-Service journey, not direct-peer) | `:2547-2551`; log line 51: `tuple=10.98.0.1:18951<-100.95.0.3:60366, peer_allocation=100.95.0.2` — VIP `10.98.0.1` ≠ peer `100.95.0.2` ≠ guest `100.95.0.3` | **YES** — genuine named-Service resolution to the VIP, the exact gap the prior direct-peer variant lacked |
| Named-Service resolution `server.svc.overdrive.local` → VIP | `:2409` `destination = format!("{MESH_NAME}:{SERVICE_PORT}")`; `MESH_NAME="server.svc.overdrive.local"` (`guest_stack_mtls_egress.rs:79`); `:2466-2468` reads the guest's real DNS-resolved destination | **YES** |
| Ordering gate `pre_fault_crafted=0` | `:2612-2615, :2635-2638` `assert_eq!(pre_fault_crafted, 0, ...)`; log line 55 `pre_fault_crafted=0` | **YES** |
| Door probe non-vacuous (`crafted ≥ 1`) | `:2639-2642` `assert!(crafted >= 1, ...)`; log line 55 `crafted=100` | **YES** |
| Door oracle `reopened=false`, `accepts=0` | `:2667-2671` `accepts==0`; `:2672-2676` `assert!(!reopened, ...)`; log line 54 `reopened=false ... accepts=0` | **YES** |
| Independent control calibrates the oracle (neg=BareAck, pos=SynAck) | `both_time_wait_controls_prove_the_substate_and_sequence_gates` (`:2115`) + `prove_time_wait_controls()` at `:2363`; log lines 50, 82 | **YES** — proves the apparatus *can* detect a reopen; the negative is meaningful |
| Current production order is mark → TPROXY → accept | `nft.rs:679-690` `tproxy_and_mark_and_accept`: `meta mark set 0x1` (`:682-683`) → load addr/port + `tproxy` (`:685-688`) → `accept` (`:689`); both rule 1 (`:757`) and rule 3 (`:770`) call it | **YES** — matches ADR-0140's "mark `0x1`, then `tproxy`, then `accept`" exactly |
| Native run is real metal/KVM, real CH guests | Log lines 6-18 (bootstrap to `ubuntu@151.115.99.251`, metal lease, KVM preflight) | **YES** |

---

## Per-check findings

### Check 1 — Listener-loss NO-QUIESCENCE classification: genuinely evidence-supported, not over-extrapolated — PASS

The classification is anchored to reproduced native conditions and is explicitly bounded:

- ADR-0140:17-28 derives "a pure leg-F or leg-C listener failure does not quiesce managed TAPs" from (a) the ordinary absent-listener outbound path failing closed (E14(c)/(d)) and (b) the TIME_WAIT side door failing closed on the named-Service E14(e) killed-mode result — **and** closes with "This is recorded on that bounded native evidence and is not extrapolated to untested kernels or host configurations." The evidence limit is preserved, not sold as a universal product contract.
- **Leg-C coverage is structural, not extrapolated.** ADR-0140:85-88 (Context) establishes the inbound/leg-C path was *already* fail-closed by the managed-guest drop rule (shared rule 4, `nft.rs:773-779`), independent of listener state. So "leg-C does not quiesce" rests on the pre-existing drop-rule structure, not on new native evidence it lacks. Consistent, not over-reached.
- The killed-mode case is the **strictly stronger** proof for the door: in both ordinary pure-leg-F loss and killed mode, the kernel state the door depends on is identical (no leg-F transparent listener + a TIME_WAIT transparent socket), but killed mode additionally has *no owner alive to help*. If the door fails closed with no owner (E14(e)), it fails closed in the ordinary case too. The architect's use of the killed case to subsume the ordinary door case is sound.

No `never quiesce`/fail-closed outcome is asserted more broadly than the evidence supports.

### Check 2 — Killed-mode residual = CLOSED: independently determined SOUND — PASS

This is the load-bearing claim and I scrutinized it hardest. My independent basis:

1. The test establishes a **true TIME_WAIT** on the Service-VIP tuple (`:2476-2495`, `:2547-2551`; log line 51), with explicit FIN/seq/tsval newer-than gates (`:2527-2539`).
2. It then **kills the serve owner** (`:2558` → `kill_serve_owner` → `ServeExit::Killed`, `serve_lifetime_support.rs:132-139`). There is no quiescence on this path — it is the abrupt process-loss seam.
3. With the owner dead, it verifies the **TAP is up** (`:2560`), the **program/members/route survive** (`:2561-2565`), and the **exact TIME_WAIT entry survives the kill before any reconnect** (`:2566-2571`).
4. Only then does the guest emit **100 crafted newer-sequence reconnect SYNs** (`crafted=100`, `pre_fault_crafted=0`), proving the door probe is non-vacuous and strictly post-fault.
5. The door oracle returns **`reopened=false`, `accepts=0`** (`:2667-2676`; log line 54), and the program is re-verified unchanged through the full window (`:2662-2666`).
6. The independent control test proves the oracle is calibrated to detect a reopen (positive=SynAck) and a surviving entry (negative=BareAck) — so `reopened=false` is a meaningful negative, not a measurement artifact.

The door-closed result is therefore produced *with no owner alive, the TAP up, the program intact, and the TIME_WAIT entry live* — i.e. it is itself the killed-mode residual scenario. The kernel path fails the reopen closed with no live owner, so runtime quiescence is not needed to preserve the outcome. **The killed-mode residual is closed.** No gap: the kill precedes every reconnect, the TAP is not down, the entry survived, and the capture is lossless (`:2584-2588`).

ADR-0140:148-158 and ADR-0124:28-34/61-74 record exactly this and correctly conclude "no killed-`serve` residue exposure is surfaced to the user." Accurate.

### Check 3 — Semantic weakening gate — PASS

No `never`/`must`/`forbidden`/`zero` security outcome was removed, moved, or synonym-swapped:

- "A pure listener failure does not quiesce TAPs or existing commands" is **preserved verbatim** at ADR-0124:202; the feature-delta register row (line 94) states "The 'never quiesce' clause is unchanged."
- The single-loss outcome is **preserved verbatim** — ADR-0140:25-26 ("no SYN-ACK, no wildcard accept, no reopen to the user — is preserved; nothing here relaxes it"), ADR-0139:133 (R18 forwarding + host-local zero outcomes), feature-delta:105-114, c4:2071.
- The only removals are the **contingent** R19-induced ones the conditional branch sanctions: no R19 canonical-identity change and no R19-only stale-table cleanup (ADR-0140:138-144; ADR-0125:42-50; feature-delta register row 99). Genuine foreign/malformed/noncanonical startup refusal is explicitly retained (ADR-0140:28; `.context` contract lines 176-178).
- No evidence LIMIT is presented as a weaker product contract — the opposite: every RECORDED annotation explicitly scopes to "bounded native evidence."

The two stale clauses in Finding F1 below do **not** weaken a security outcome (they over-credit a withdrawn mechanism for a *correct* fail-closed outcome) — so they are a consistency defect, not a weakening. Gate passes.

### Check 4 — No manufactured amendment history — PASS

The ADRs state the current order as the one present-tense decision. ADR-0140's Decision (lines 99-110) is "This decision is withdrawn… keep their existing tail," in present tense; the historical conditional acceptance is cleanly quarantined under "Historical conditional acceptance, retained as provenance rather than current execution authority" (lines 38-53). ADR-0139:54-56 and :123-131 state the current order and record the cross-reference supersession without narrating a reorder-then-revert. No artifact narrates "reordered then reverted/amended." The reorder was never implemented, and the artifacts correctly do not pretend it was.

### Check 5 — R18 preserved exactly — PASS

ADR-0139 decision (lines 67-82) is intact: one separate `ip overdrive-mtls-guard` table, one prerouting chain at filter priority, one rule dropping intercept-marked (`0x295a`) TCP. The R19 cross-reference annotation (lines 123-131) explicitly states "This changes no R18 decision, field, error, effect, guard rule, outcome, or ownership: R18's IP-program-loss exposure independently reproduces and its decision stands." Nine-IP-rule cardinality preserved (ADR-0139:138; `.context` contract lines 199-201). `InterceptMarkGuardAbsent`, the two doc-hidden effects (`observe_intercept_mark_guard`/`converge_intercept_mark_guard`, scaffolded at `nft.rs:3087`/`:3099`), and E14(a)/(b)'s 1,972-forwarded-SYN RED are all preserved. No descope.

### Check 6 — Cross-artifact consistency — PARTIAL (see Findings F1, F3)

The security OUTCOMES are consistent across all nine artifacts. However, two residual-phrasing defects remain:

- The decisions table (feature-delta:746), the R19 contract (`:4064-4066`), the reuse analysis (`:6765`), and all five ADRs + brief + c4 state the withdrawal and the retained mark → TPROXY → accept order correctly. c4:1652 ("mark-before-TPROXY rules retained (ADR-0140 reorder withdrawn on native E14 fail-closed)"), c4:1600, and c4:2071 are fully reconciled.
- **But** two operative DESIGN clauses still carry the withdrawn mechanism and a pre-resolution status (Finding F1), and the register/brief notes about the C4 are now stale (Finding F3).

Finding L1 from the prior review (controls completed + direct-peer variant + named-Service E14(e) RESOLVED fail-closed) IS incorporated — register row 103 and ADR-0140:148-158 record the named-Service E14(e) killed-mode door-closed result explicitly.

### Check 7 — ADR discipline — PASS

ADR-0140 records exactly one decision (the reorder, now withdrawn); status/decision are coherent. No Rust signatures/variants/wire structs are embedded in the ADRs — ADR-0124 explicitly routes exact types/signatures to the feature delta ("Exact types and signatures remain single-sourced in the #295 feature delta," line 149; "Exact public request/method/error shapes live only in the feature delta," line 211). ADR-0139 carries no code surface. The two Rust effect signatures in the `.context` proposal (lines 189-192) are the pre-existing R18 contract and live in the feature-delta register, not in any ADR.

### Check 8 — design.md material-decision surfacing — PASS (no new user decision)

The R19 ordinary withdrawal rode the pre-approved conditional (ADR-0140:41-52, user acceptance 2026-09-24, with the self-withdrawal clause verbatim). The no-quiescence classification **preserves** the original single-loss outcome (no SYN-ACK / no wildcard accept / no reopen) — it does not relax it. The killed-mode residual is **closed by evidence**, not **accepted as a residual** — so there is no new security-outcome reduction. No new material security decision requiring fresh user approval is present. (Had E14(e) *opened* the door, a measured residual would have gone to the user — ADR-0140 and the `.context` gate correctly reserve that path; it did not arise.)

---

## FINDINGS

### F1 (HIGH) — Two operative DESIGN clauses still assert the WITHDRAWN R19 reorder as the current fail-closure mechanism and label it "conditional on native RED"

**Location:** `docs/feature/netns-density-295/feature-delta.md`
- **Line 7772** (Functional requirements, scope item 2, under the live `## Wave: DESIGN / [REF] Requirements, Quality Attributes, and Capacity`): "…or a TPROXY target listener is absent **(PROPOSED D-295-R19)**."
- **Line 7806** (Ranked quality attributes, **#1 Security / confidentiality**): "…and a missing TPROXY listener fails closed **through the PROPOSED D-295-R19 rule order**; **both conditional on native RED**."

**Why this is a finding, not a nit.** These are *operative* Tier-1 `[REF]` DESIGN clauses (not under any historical-provenance banner — the banner at feature-delta:116 governs the overall status line, and lines 82-83 declare only that "historical text *elsewhere* is retained as provenance"). The feature-delta inline-reconciled its *other* R19 clauses (decisions table 746, R19 contract 4064, reuse 6765, boot-ordering references) but missed these two. The ranked quality attribute #1 is exactly the text DISTILL reads to derive security scenarios; leaving it asserting (a) the **withdrawn** reorder as the mechanism and (b) status "conditional on native RED" (i.e. unresolved) directly contradicts the recorded decision (withdrawn; fail-closure via the retained order on recorded native evidence) and will propagate the wrong mechanism/status downstream. This is precisely the propagation the design's own normative-diff gate exists to stop (`.context`:274-281: "every operative artifact must carry the precise new status and wording, with the old contradicted assertions marked historical or withdrawn").

**This is NOT a security-outcome weakening** — both clauses correctly assert the listener fails *closed*; the defect is mechanism attribution + status label. Hence HIGH (operative security-attribute text feeding DISTILL), not critical.

**Surgical fix (clear):** reconcile both clauses to the recorded decision, e.g. line 7806 → "…a missing TPROXY listener fails closed under the retained mark → TPROXY → accept order (D-295-R19 reorder withdrawn 2026-10-03 on native E14 evidence)"; line 7772 → drop "(PROPOSED D-295-R19)" or annotate "(listener-absent fail-closure recorded on native E14(c)/(d)/(e); D-295-R19 reorder withdrawn)." No security wording changes.

### F2 (MEDIUM) — Register RECORDED status for the security/evidence projections is ahead of the inline text

**Location:** feature-delta register rows 94 and 100 (`:94`, `:100`). Row 100 marks "FD… application/**security**/evidence projections… RECORDED," but the security projection at line 7806 (F1) was not updated inline. The register's RECORDED claim is therefore slightly ahead of the artifact's actual state for those projections.

**Disposition:** resolved automatically once F1 is fixed. Folded into F1's remediation. Flagged separately so the register's "RECORDED" is not read as "fully reconciled inline" before F1 lands.

### F3 (LOW) — Stale "C4 still needs reconciliation" note; the C4 is in fact reconciled

**Location:** feature-delta register row 100 ("The `c4-diagrams.md` L2 order/listener prose is outside this task's edit ownership and still needs the same wording reconciliation (flagged for the C4 edit owner)") and brief.md:950-952 (same note). But `c4-diagrams.md` HAS been reconciled: line 1600 records the withdrawal, line 1652 states "mark-before-TPROXY rules retained (ADR-0140 reorder withdrawn on native E14 fail-closed)," and line 2071 states the no-quiescence classification correctly. The notes are stale.

**Disposition:** LOW; no security or correctness impact (the C4 content is correct). Update the two notes to reflect that the C4 was reconciled. Documentation reconciliation only.

---

## Killed-mode determination (independent basis)

**CLOSED.** The E14(e) door test is itself a killed-mode scenario: `kill_serve_owner` (`:2558`, a genuine `ServeExit::Killed` with no quiescence) abandons the owner; the TAP remains up (`:2560`), the program/members/route survive (`:2561-2565`), and the true Service-VIP TIME_WAIT entry survives the kill before any reconnect (`:2566-2571`). Only then do 100 strictly-post-fault crafted newer-sequence reconnects (`pre_fault_crafted=0`, `crafted=100`) meet a door that returns `reopened=false`, `accepts=0`, with the oracle independently calibrated (neg=BareAck/pos=SynAck). With no owner alive, the kernel path fails the reopen closed. The residual the prior review held open ("quiescence alone cannot preserve killed-mode security after owner death") is discharged *without* a quiescence remedy. I found no gap: the kill is not after quiescence, the TAP is not down, the entry survived, and the capture is lossless.

## Semantic-weakening gate

**PASS.** Every `never`/`must`/`forbidden`/`zero` security clause is preserved verbatim; the only removals are the contingent R19-induced canonical-identity change and R19-only cleanup; genuine foreign/malformed/noncanonical startup refusal is retained; no evidence limit is dressed up as a weaker product contract. The F1 clauses over-credit a withdrawn mechanism for a correct fail-closed outcome — a consistency defect, not a relaxation.

## New material security decision requiring the user

**NONE.** The withdrawal rode the pre-approved 2026-09-24 conditional; the no-quiescence classification preserves the original single-loss outcome; the killed-mode residual is closed by evidence rather than accepted as a residual. No fresh user approval is needed. (If any future kernel/host configuration ever opened this door, ADR-0140's and `.context`'s gates correctly reserve that for an outcome-preserving design + explicit user approval — not triggered here.)

---

## FINAL VERDICT

**CHANGES_REQUESTED** — documentary reconciliation only; the finalized decision is sound.

The core of this finalization is **correct and fully evidenced**: the R19 reorder is validly withdrawn under its pre-approved native condition; R18 stands exactly; the listener-loss NO-QUIESCENCE classification and the killed-mode residual closure are independently verified from the test source and the `f03c43d5` native log; the semantic-weakening gate passes; there is no manufactured amendment history; and no new material user decision is required. Eight of the nine artifacts (ADR-0140, 0124, 0139, 0125, 0088, 0089, brief, c4) are correctly reconciled and RECORDED.

The finalized DESIGN may **not** proceed to roadmap reconciliation until the following land, because they are operative security-attribute text that feeds DISTILL and contradicts the recorded decision:

**Blocking items:**
1. **(HIGH, F1)** Reconcile feature-delta:7772 and :7806 so neither attributes listener-absent fail-closure to the *withdrawn* "PROPOSED D-295-R19 rule order" nor labels it "conditional on native RED." State the recorded disposition: fails closed under the retained mark → TPROXY → accept order on native E14(c)/(d)/(e) evidence; D-295-R19 reorder withdrawn 2026-10-03.

**Non-blocking (fix in the same pass):**
2. **(MEDIUM, F2)** Confirm register rows 94/100 "RECORDED" is accurate once F1 lands (the security projection is then truly reconciled inline).
3. **(LOW, F3)** Update the stale "C4 still needs reconciliation" notes at feature-delta register row 100 and brief.md:950-952 — the C4 is already reconciled (c4:1600, 1652, 2071).

These are surgical inline edits with no security-wording change. Once they land, the finalized DESIGN is approvable and may proceed to roadmap revalidation (itself a separate downstream gate, as every artifact correctly states). The original 08-01 crafter remains before GREEN; this review authorizes no DES phase, GREEN, or COMMIT.
