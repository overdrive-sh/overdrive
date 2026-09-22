# Design review — step 03-01 EXEC-claim evidence remediation

- **Feature:** `netns-density-295`
- **Decision:** `D-295-DELIVER-03-01`
- **Review ID:** `arch_rev_20260922T154018Z_iteration_2_durable`
- **Reviewer:** fresh isolated `nw-solution-architect-reviewer`
- **Reviewed range:** `a829de84e32b7add5498b9d00a4929a60bee5223..078e00db5cf6fe6df0e1662bcf041878aa9f89b1`
- **Design commit:** `078e00db5cf6fe6df0e1662bcf041878aa9f89b1`
- **Review date:** 2026-09-22
- **Final verdict:** **APPROVED**

## Review boundary and authority

This is the durable independent review of the bounded DESIGN remediation
triggered by `deliver/review-03-01.md` iteration-2 finding D2. The reviewed
question is only whether private
`GuestNetworkExecState.active_claims +1/-1` storage must be independently
observed in step `03-01`, or whether S-ND295-27's opaque-capability evidence
plus S-ND295-28's production-`VmDriver` schedules form the complete accepted
evidence contract.

The authority reviewed was:

- `deliver/design-remediation-03-01.md`;
- the exact API and Contract Shape clauses in `feature-delta.md`;
- S-ND295-27 and S-ND295-28 in `distill/test-scenarios.md`;
- steps `01-01`, `03-01`, and `03-02`, their dependency, and roadmap
  validation in `deliver/roadmap.json`;
- the aligned effect-isolation catalogue in
  `docs/product/architecture/brief.md`;
- the D2 trigger in `deliver/review-03-01.md`; and
- the necessary current production facts in
  `crates/overdrive-core/src/guest_network.rs` and
  `crates/overdrive-worker/src/vm_driver.rs`.

No production Rust, test, ADR, C4 diagram, DES log, or prior review artifact is
part of this review's write scope.

## Prior chat-review non-reliance

The prior child review returned only chat YAML. It is not a completed repository
review and was not used as evidence. In particular, the approval text already
present in the design remediation, feature delta, and roadmap was treated as a
claim to verify, not as proof. This review independently read the bounded diff,
the trigger, the current authoritative clauses, and the two production files
before reaching its verdict. This Markdown artifact is the durable review
record.

## Findings and severity

| Severity | Count | Disposition |
|---|---:|---|
| Critical | 0 | None |
| High | 0 | None |
| Medium | 0 | None |
| Low | 0 | None |

There are no blocking or advisory findings.

## Production-fact verification

The remediation's premise matches current production:

- `GuestNetworkExecGate`, `GuestNetworkExecSupervisor`, and
  `GuestNetworkExecClaim` share private state; `active_claims` is a private
  field (`guest_network.rs:18-49`).
- Open admission increments that field and returns an opaque claim;
  BootClosed/Recovering wait and FailStop refuses (`guest_network.rs:173-200`).
- claim `Drop` contains the sole non-update inspection, a debug assertion,
  followed by the decrement and waiter notification
  (`guest_network.rs:320-327`). A bounded
  repository source scan found only declaration, initialization, increment,
  debug assertion, and decrement occurrences. No admission, recovery, drain,
  shutdown, or public projection consumes the count.
- `VmDriver::release_for_exit_emission` awaits `claim_release` before reading or
  taking `pending_exec` and `gate_sender`, and its `_exec_claim` local remains
  in the enclosing future across `BeaconWriter::release_exec` acknowledgement
  (`vm_driver.rs:1836-1895`).
- The production writer is not an unowned test surrogate: `BeaconWriter`
  retains its task handle, `release_exec` transfers the command before awaiting
  acknowledgement, and its cancellation guard signals the retained writer
  (`vm_driver.rs:576-673,728-811`).

These facts establish that the counter is private bookkeeping rather than a
production decision boundary. Requiring an independent field oracle would
contract the storage representation, not add stakeholder-visible or
owner-visible coverage. No runtime failure is asserted by this remediation, so
there is no unproved defect being converted into a design requirement.

## Contract Shape and evidence allocation

The revised Contract Shape is internally consistent and complete at the
accepted boundaries:

1. `GuestNetworkExecWiring::new` remains pure construction of the paired opaque
   capabilities.
2. `GuestNetworkExecGate::claim_release` plus claim Drop remains
   **bounded-change**, but its observable universe is now exact: Open returns
   one opaque lifetime, BootClosed/Recovering remains pending, FailStop returns
   `None`, wake/refusal is task-observable, and supervisor-owned state and
   projections retain their separate ownership. Unrelated claims, pending EXEC
   values, rows, request ownership, and recovery receipts remain the stated
   complement.
3. S-ND295-27 retains every accepted public return, `recovery_progress`
   projection, blocking/wake/refusal schedule, terminal first-request behavior,
   invalid-state table, all six typed causes, and the roadmap's exhaustive
   twelve-component by six-cause typed table. No Display/Debug oracle replaces
   typed evidence.
4. S-ND295-28 remains mandatory and distinct. Its Gherkin, technical mapping,
   step `03-02` criteria, and real `VmDriver`/beacon-writer selectors still
   require claim-before-detection completion, recovery backpressure, writer
   acknowledgement, cancellation ownership, claim-lifetime end, FailStop
   refusal, and absence of a detached writer. Unobserved counter mutation is
   explicitly insufficient.

The result complies with effect isolation: the read capability does not gain a
write method, the bounded universe and complement are declared, and no
unbounded-preservation operation or Plan-returning API is introduced.

## API, architecture, and scope assessment

The reviewed commit changes five documentation files and no source or test
file. It adds no public, doc-hidden, crate-private, or test-only method, type,
trait, enum variant, parameter, accessor, snapshot, hook, or alternate owner.
It also adds no persistence/recovery mechanism, consistency protocol,
dependency edge, task topology, or process owner.

The three alternatives are evaluated against the exact accepted API: a public
or doc-hidden accessor is rejected, a private/test-only snapshot seam is
rejected, and evidence is allocated to the existing public gate boundary and
existing production owner. That is the smallest authorized correction to the
contradiction. Existing counter prose describes current implementation
bookkeeping but no longer creates a private-counter or whole-state oracle.

No ADR or C4 amendment is required because component boundaries, ownership,
runtime sequencing, and public surface do not change. The architecture brief's
only change is the necessary evidence-boundary alignment.

## Cross-artifact consistency

| Artifact | Assessment |
|---|---|
| Design remediation | Selects the observable-boundary split and explicitly preserves claim ownership, recovery, fail-stop, and `03-02`. |
| Feature delta | Exact API fence is unchanged; EXEC-close linearization and Contract Shape rows distinguish opaque lifetime behavior from non-projected bookkeeping. The decisions table records the same bounded decision. |
| DISTILL | S-ND295-27 is the complete opaque-capability contract; S-ND295-28 retains production owner/writer acknowledgement and cancellation evidence. |
| Roadmap | `03-01` retains S-ND295-27 and its transition/typed-table criteria. `03-02` depends directly on `03-01`, retains S-ND295-28, and still requires the real `VmDriver` writer and cancellation schedules. |
| Architecture brief | Uses the same public-outcome/private-bookkeeping distinction and the same S-ND295-27/S-ND295-28 split. |

No stale authoritative whole-private-state or independent-counter oracle remains
in these artifacts. The roadmap's `validation.status = approved` and the
design-remediation approval status are substantively justified by this fresh
independent review; the pre-existing chat-only review identifier was not relied
upon.

## Roadmap review checks

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | PASS | `03-01` drives the real public core capabilities; dependent `03-02` drives the real production `VmDriver` and beacon writer. |
| Acceptance-criterion coupling | PASS for this remediation | The change preserves the accepted exact capability surface and named scenario bodies without introducing a new implementation mechanism or API. |
| Step decomposition ratio | PASS | The affected evidence split remains two sequential steps over the two owning production files (`guest_network.rs`, `vm_driver.rs`), ratio 1.0. No duplicate step is added. |
| Implementation code in roadmap | PASS | The remediation adds no implementation snippet or algorithm; its roadmap edits state behavioral evidence ownership and prohibitions. |
| Concision and precision | PASS | Step count, estimates, and criteria count are unchanged. The changed notes precisely resolve one oracle contradiction. |
| Test boundary | PASS | S-ND295-27 stays at the public opaque capability boundary; S-ND295-28 stays at the production owner/writer boundary. No private module import, state hook, or counter snapshot is authorized. |

## Architecture review dimensions

| Dimension | Assessment |
|---|---|
| Architectural bias | PASS — no technology or mechanism is added; two broader observation seams are explicitly rejected. |
| Decision quality | PASS — context, trigger, three alternatives, rationale, consequences, non-goals, and exact handoff criteria are present. |
| Completeness | PASS — public behavior, typed evidence, owner schedule, dependency, API prohibition, and scope consequences are all covered. |
| Implementation feasibility | PASS — the evidence maps to existing capabilities and the current production owner; no unavailable testability surface is assumed. |
| Priority validation | PASS — the remediation addresses only the reproduced contract/evidence contradiction and leaves unrelated architecture closed. |

## Verification

| Check | Result |
|---|---|
| `jq -e . docs/feature/netns-density-295/deliver/roadmap.json` | PASS |
| Roadmap `03-01`/`03-02` uniqueness, scenario mapping, and `03-02 -> 03-01` dependency assertions | PASS |
| S-ND295-27/S-ND295-28 heading and decision-row cross-reference counts | PASS — exactly one of each |
| Referenced scoped files and remediation link target | PASS |
| Cross-document evidence-split phrase checks | PASS |
| Bounded production `active_claims` source scan | PASS — five production occurrences, with no consumer outside Drop's debug assertion |
| Production/source diff in the reviewed range | PASS — none |
| Changed-file scope | PASS — exactly the five authorized documentation files |
| `git diff --check a829de84e32b7add5498b9d00a4929a60bee5223..078e00db5cf6fe6df0e1662bcf041878aa9f89b1` | PASS |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate |

## Remediation dispositions

| Trigger | Disposition |
|---|---|
| D2 — private active-claim decrement was unobservable under the accepted opaque API | RESOLVED by the approved evidence allocation. Private storage is not an independent step-`03-01` outcome; public capability behavior remains S-ND295-27 and production owner/writer lifetime behavior remains mandatory S-ND295-28. |

No implementation remediation or DESIGN expansion is authorized by this
review. Step `03-01` may be re-reviewed against the revised evidence contract;
step `03-02` remains the next mandatory dependent evidence step.

# APPROVED
