# DESIGN review — STEP 12-01 recovery attempt-round state

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `12-01` |
| Decision | `RECROUND1` — independently approved for user decision, then user-approved on 2026-10-27 (post-review record below) |
| Review date | 2026-10-27 |
| Reviewer | `/root/design_12_01_recovery_round/review_recovery_round` |
| Role | Fresh independent DESIGN reviewer |
| Model / reasoning | Exact model identifier and reasoning setting were not exposed to this reviewer |
| Scope | Reopened `design-12-01-recovery-round.md`; production supervisor, EXEC gate, Quint model, conformance driver, retained seed, accepted per-component liveness contract, and workspace design/reproduction rules |
| Final verdict | **APPROVED FOR USER DECISION** |
| Open proven findings | 0 |

This review was the technical approval to present the exact proposed decision
to the user. The post-review record below captures the later approval; neither
the review nor that approval makes the pending roadmap executable.

## Evidence and review boundary

- `.context/netns-density-295/12-01/quiescence-seed-2355dc58-after-selector93-20261026.log:15320-15416` records the retained `QUINT_SEED=0x2355dc58` failure: after C1 and C2 become `Held` at attempt zero, `RepFail(C2)` asks for a repair that is not parked.
- `docs/feature/netns-density-295/deliver/review-distill-12-01-alignment.md`, iteration 94, classifies the first-round joiner and reverse-order schedule against the composed supervisor and QPROJ-6.
- `crates/overdrive-control-plane/src/lib.rs:1702-1730,1743-1787,1806-1811,1846-1923` establishes round snapshotting, later recovery insertion, result collection, and the barrier before successful restores and `complete_attempt`.
- `crates/overdrive-core/src/guest_network.rs:267-299,314-352` establishes that a mid-round joiner is not charged by that round.
- `specs/quint/guest-flow-owner/quiescence.qnt:249-329,458-475` supplies the current recovery actions, attempt effects, fail-stop, and `OwnFair(c)` temporal premise.
- `crates/overdrive-control-plane/src/quint_quiescence.rs:314-365,548-596` supplies the existing driving-point repair check, audit handler, and QPROJ-6 projection. `crates/overdrive-control-plane/src/shared_network_test_ports.rs:1219-1256` supplies the parked-repair evidence and audit release behavior.
- `docs/product/architecture/adr-0169-forwarding-quiescence-is-a-counted-move-only-hold.md:135-143`, `AGENTS.md`, and `.claude/rules/design.md` supply the accepted liveness, evidence, API, error, and user-decision gates.

The review was source and retained-log inspection only. I did not edit code or
design, run tests or Metal, execute a model checker, or produce new native
evidence.

## Prior iterations and dispositions

| Review stage | Earlier finding or question | Final disposition |
|---|---|---|
| Boolean membership proposal | It disabled the false joiner result but published a member's attempt count or restore before a multi-member production round reached its barrier. | **Resolved.** `roundState` now distinguishes `pending`, collected `keep`, and collected `restore`; projected count and restore effects apply only when no member remains pending. |
| Boolean membership proposal | A joiner could wait behind another component while `OwnFair(c)` had no enabled own result action. | **Resolved.** The proposed canonical `FailStop(c)` guard includes a recovery waiting outside or at a collected-result barrier, representing its existing per-component deadline. `OwnFair(c)` and its eventual outcomes remain. |
| Four-value drafts | A proposed conformance selector and deadline handler were assessed against possible multi-audit and mixed foreign-restore paths. | **Withdrawn from RECROUND1.** The final proposal adds neither selector nor handler branch. Existing driving-point checks remain, and the recording conformance fail-stop selector does not gain the new canonical deadline case. |
| Source review of a later multi-member round | The unkeyed owner repair slot and fixture-wide audit release could cause another divergence. | **Unproven hypotheses, not accepted blockers or requirements.** No failing seeded invariant through the current composition establishes the claimed later task-to-target or audit divergence. The proposal adds no port, error, fixture, scheduler, or observation surface for them. A new failure must be retained and routed through the workspace falsification gate before a remedy is designed. |
| Interim `SPECIFICATION_AMBIGUITY` artifact | Its first reproduction gate and proposed typed-error examples were reviewed, then corrected. | **Superseded by the reopened bounded proposal.** They impose no implementation requirement here. If a later production-composition failure is proven, the seeded `overdrive-sim` invariant and DESIGN-owned exact API/error contract gates apply. |

## Final correctness and necessity assessment

The reproduced failure needs a component-specific distinction between a
recovery holding a `QuiescenceHold` and a recovery whose task belongs to the
current attempt round. `RecoveryEpisode { phase, failed_attempts }` cannot make
that distinction: both episodes are `Held` at zero in the retained trace, but
only C1 has a parked result. The reverse start order rules out deriving round
membership from component ordering. The proposed `roundState` map is therefore
necessary to remove this false formal result action without suppressing all
overlap failures or changing the QPROJ-6 contract.

The four values are also necessary for the supervisor's observable
multi-member barrier. `round_pending` loses each member as its task result is
collected, while `round_results` retains whether that member stays or restores;
the supervisor delays successful restores and `complete_attempt` until the
last member returns. A Boolean membership map alone cannot preserve both
pending membership and an already collected result. The `keep` and `restore`
values preserve the compared phase, count, and hold until the barrier, then
apply the existing outcome once. A later joiner remains uncharged by that
barrier, matching the EXEC gate's `joined_since_attempt` rule.

The proposed guards require `resultPending(c)` for the four repair-result
actions. Thus the retained `RepFail(C2)` is disabled while C2 is outside the
active C1 round, and C1's result remains enabled. A subsequent round may
include both held recoveries. `waitingOnRound(c)` applies to a held joiner or
a collected member while another task remains pending; the canonical
`FailStop(c)` extension represents the already-accepted elapsed deadline and
keeps per-component liveness under `OwnFair(c)`. The recording conformance
selector does not add that branch, so this decision does not claim a new
driver-observed deadline case.

`roundState` is expressly model-only and absent from `QuiescenceState`,
`State::from_spec`, and `State::from_driver`. The existing `held_repair(c)`
check continues to reject an unplayable action as `DriverError::NotEnabled`.
No production port, public or internal API, typed product error, fixture,
handler, action inventory, check, or lane is added. The stated `boolLatch`,
leak, and foreign-hold hazards remain in the existing formal action set, and
the validation plan retains their checks. No normative outcome or invariant is
relaxed to close the retained seed.

## Verification and verdict

| Check | Review result |
|---|---|
| Retained first-round seed and reverse-order reachability | Supported by the retained log, iteration-94 review, and supervisor source. |
| Multi-member count/restore timing | The proposed four-value barrier matches the source-level supervisor and EXEC ordering. |
| Per-component liveness | The proposed canonical waiting/deadline guard preserves `OwnFair(c)` and ADR-0169's eventual outcome. |
| Projection, API, and typed-error scope | QPROJ-6 and product interfaces stay unchanged; `roundState` is explicitly model-only. |
| Later repair-slot and multi-audit concerns | Not promoted without a failing seeded current-composition invariant. |
| Tests, Metal, model checks, evidence record | Not run in this independent source review; required after user approval and implementation. |

**APPROVED FOR USER DECISION.** The orchestrator may present the exact
`RECROUND1` decision in the proposal to the user. Only explicit user approval
authorizes canonical amendment and resumed STEP 12-01 delivery.

## Post-review user decision

On 2026-10-27, after the orchestrator presented the concrete RECROUND1
approval request with this review verdict, the user replied exactly
**“proceed”**. That approves RECROUND1; it does not approve replacement
formal/conformance evidence or change `deliver/roadmap.json` validation from
`pending`. The canonical fold remains subject to independent fold review
before STEP 12-01 implementation resumes.

## Iteration 2 — post-approval canonical fold review

### Metadata and authority

| Field | Value |
|---|---|
| Review date | 2026-10-27 |
| Reviewer / role | `/root/design_12_01_recovery_round/review_recovery_round` / independent DESIGN reviewer |
| Reviewed decision | User-approved `RECROUND1`, exact reply **“proceed”** on 2026-10-27 to the concrete request after independent review |
| Scope | Fold from `deliver/design-12-01-recovery-round.md` into `feature-delta.md` and bounded STEP 12-01 entries in `deliver/roadmap.json` |
| Verdict | **APPROVED — CANONICAL FOLD** |
| Open findings | 0 |

The proposal records the exact user reply and context at lines 5-13. The
canonical decision note at `feature-delta.md:10769-10775`, its STEP 12-01
summary at `:13301-13317`, and the roadmap protocol entry at
`deliver/roadmap.json:53` retain that authority without treating it as
replacement-proof validation. The Iteration 1 verdict and findings above
remain historical; this iteration reviews the later canonical fold only.

### Exact contract comparison

| Approved proposal | Canonical fold | Assessment |
|---|---|---|
| `roundState: str -> str`; `outside`, `pending`, `keep`, `restore` and their membership/result meanings (`design-12-01-recovery-round.md:79-100`) | `feature-delta.md:10779-10803`; projection table `:10744` | **Exact.** The four closed values and relation to private `round_pending` / `round_results` are preserved. |
| `beginRound`, collected-result barrier, per-member attempt/restore effects, and unchanged outside joiner (`design:102-124`) | `feature-delta.md:10801-10818` | **Exact.** Count falls by the number of restored members; findings begun in a result action remain outside and uncharged. |
| Existing action identities plus `resultPending(c)` guards and same-action last-member barrier (`design:126-159`) | `feature-delta.md:10820-10843` | **Exact.** The four guards retain their phase, attempt, failure-budget and audit-finding conditions. `RecRepair` remains pending through audit; the three result actions publish at the barrier. |
| `waitingOnRound(c)`, canonical `FailStop(c)`, retained `OwnFair(c)` and unchanged recording selector/handler (`design:161-189`) | `feature-delta.md:10845-10861` | **Exact.** Per-component deadline meaning and the conformance boundary are retained. |
| `RoundStateIsRecovery` and two expectations added to the existing overlap run (`design:191-219`) | `feature-delta.md:10863-10879` | **Exact.** The value, holding, `restore`-phase and pending-member clauses are present without a new run/check. |
| Model-only state; no QPROJ-6, projection, API, typed error, fixture or handler change (`design:221-259`) | `feature-delta.md:9837-9840,10744,10879-10882`; roadmap `:53,1062` | **Exact.** Existing `held_repair(c)` / `NotEnabled` remains the driving-point check. |
| Unkeyed repair-slot and multi-audit questions remain unproven hypotheses (`design:261-281`) | `feature-delta.md:10888-10891,13313-13314` | **Exact.** Neither is promoted into a production/API/error/testability requirement. |

The fold retains the existing `boolLatch` first-restore hazard, counted holds,
independent overlapping recovery failures, no-charge mid-round joiner, and
restore-at-zero outcome. Its RECROUND1 additions do not remove or relax a
`zero`, `never`, `must`, `exactly`, fail-closed, forbidden, or required
condition. The existing action, `ConfStep`, named-run, hazard, and check
inventories are not expanded by this fold. Other concurrent feature-delta
edits were outside this bounded review and were not attributed to RECROUND1.

### Roadmap and verification

The STEP 12-01 criterion at `deliver/roadmap.json:1062` directs the crafter to
implement the exact feature-delta fold, replay seed `0x2355dc58`, run every
unchanged named/randomized quiescence target, and retain the full three-job
evidence and independent replacement-proof gates. The delivery protocol entry
at `:53` repeats the bounded surface. The parsed roadmap remains
`validation.status = pending` and `validation.approved_at = null`
(`:1702-1705`); user approval and this fold review do not approve the
replacement evidence or make the roadmap executable on their own.

Verification was a read-only comparison of the approved proposal, prior review,
canonical feature-delta sections, roadmap entries, and their textual diff;
`roadmap.json` parsed successfully. No production source, Quint model, test,
fixture, driver, DES log, or other design artifact was edited in this review.
No tests, Metal job, model check, or evidence record was run.

**APPROVED — CANONICAL FOLD.** The recorded user approval and the exact
canonical/roadmap fold are consistent. Replacement formal/conformance proof
and roadmap validation remain pending.

