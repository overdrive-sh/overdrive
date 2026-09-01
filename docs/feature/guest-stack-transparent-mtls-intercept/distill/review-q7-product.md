# DISTILL Product-Owner Review — Q7/Q9 remediation

**Reviewer:** `nw-product-owner-reviewer` (Eclipse)
**Date:** 2026-08-29
**Reviewed commit:** `7255c68e64b7f0f15da5a2ed8a033806a2939e6a`
**Compared with:** `29ab0bf71ef8c178a04035010d0ad72084b9ce7b`
**Verdict:** **REJECTED_PENDING_REVISIONS**

## Finding counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 3 |
| Medium | 2 |
| Low | 0 |

Approval requires zero Blocker and High findings. The remediation is therefore
not ready for DELIVER handoff.

## Scope and evidence reviewed

- The complete feature delta, approved Q7/Q9 design remediation and review,
  ADR-0088, ADR-0089, the normative architecture-brief amendment, both product
  journeys, the existing microVM DISTILL contract, the backend-instance
  replacement contract, and the current DISTILL scenario/red-classification
  artifacts.
- The public CLI declarations and dispatch for deploy, workload describe,
  workload restart, and job stop.
- The current Job natural-exit branch in `WorkloadLifecycle`.
- Commit scope: three documentation files, 318 insertions and 116 deletions.
  `git diff --check 7255c68e^ 7255c68e` passed.
- DISCUSS artifacts are acknowledged as missing. This review derives the
  product contract from the approved architecture/design and the live
  cross-feature lifecycle contracts; it does not add a new public surface.

## What is sound

- Q7 now has a deterministic boundary: every platform initialization stage
  precedes READY; failure powers off before READY, emits no guest `EXIT`, never
  reaches Running or EXEC, and cannot run the operator command
  (`test-scenarios.md:59,237-251`).
- S-GTI-08 preserves the exact VMM `Option<i32>` in terminal `Failed`, emits
  only `FinalizeFailed`, and holds both private and durable restart counts
  unchanged. The supporting classifier, suppression, and bounded-diagnostic
  matrices are concrete and falsifiable (`test-scenarios.md:245-295`).
- S-GTI-02 materially closes the born-captured promise: capture is armed before
  the real VMM, identity is correlated to the exact allocation, uncertain
  observations fail conservatively, the interval covers all guest L2 traffic,
  and the first operator tuple is followed through rule increment, leg-F,
  TLS/kTLS, and zero cleartext (`test-scenarios.md:180-194`).
- Beacon and public describe shapes remain unchanged, and the design correctly
  uses the existing describe projection rather than inventing a field
  (`test-scenarios.md:105-110`; `design/wave-decisions.md:171-179`).
- Fresh-install failure, restart-install failure, and teardown are all intended
  to fail closed; the defects below concern the reachability and operator
  wording of two of those proofs, not the security objective.

## Findings

### HIGH-1 — S-GTI-06 does not name a route that actually proves the same-allocation restart gate

**Dimension:** Cross-wave lifecycle coherence / scenario reachability /
restart safety
**Evidence:** `distill/test-scenarios.md:221-228`,
`design/wave-decisions.md:18`,
`crates/overdrive-reconcilers/src/workload_lifecycle.rs:823-865`,
`docs/feature/backend-instance-replacement/distill/test-scenarios.md:34-39,264-281`,
`docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md:1687-1704`

S-GTI-06 says a `RestartAllocation` reuses the allocation id through
“crash-recovery, restart budget, or `overdrive workload restart`” and then
claims this proves the `:1880` restart install gate. Those triggers are not
interchangeable for the executable `[vm]+[job]` surface:

- a Job natural exit, clean or crashed, is finalized run-once before the
  restart-budget branch and emits no restart attempt;
- `overdrive workload restart <id>` is the ADR-0073 generation-replacement
  verb: it mints a **new** AllocationId and emits a fresh `StartAllocation`, so
  it exercises the fresh-start path rather than same-id `RestartAllocation`;
- the established reachable same-allocation VM Job re-drive is platform
  reclamation while intent still stands.

As written, the scenario can be implemented through a fresh allocation and
leave the `:1880` path unproved, or it can invite a forbidden Job crash/restart
loop. That defeats the exact regression lock S-GTI-06 exists to provide.

**Required remediation:** Drive S-GTI-06 with the established
platform-reclamation transition that emits same-allocation
`RestartAllocation`. Assert the reused allocation identity and the post-restart
wire outcome. Remove restart-budget, natural-crash, and `workload restart` as
equivalent triggers from the DISTILL artifacts and embedded feature-delta
summary. Back-propagate the correction to D6's stale “all live for VM” wording.
State explicitly that pre-READY failure and Job natural exit finalize without
restart, while the operator restart verb creates a fresh allocation and is
covered by the fresh-start gate.

### HIGH-2 — S-GTI-12 invokes a public command that does not exist

**Dimension:** Operator journey executability / command traceability
**Evidence:** `distill/test-scenarios.md:253-260`,
`feature-delta.md:719-722`, `crates/overdrive-cli/src/cli.rs:64-70,94-109`,
`crates/overdrive-cli/src/main.rs:139-153`,
`docs/product/journeys/run-a-vm-workload.yaml:157-167`

The teardown scenario calls `overdrive workload stop` and the embedded
walking-skeleton summary advertises `workload {describe,restart,stop}`. The CLI
has only `workload restart` and `workload describe`; stopping a Job is
`overdrive job stop <id>`. The internal handler happens to remain
`commands::deploy::stop`, but that is not an operator command.

This makes the claimed real driving-port scenario non-executable and leaves
the teardown promise untraceable from the actual operator surface.

**Required remediation:** Use `overdrive job stop <id>` in S-GTI-12 and list
`overdrive workload {describe,restart}` plus `overdrive job stop` in the
feature-delta walking-skeleton summary. Keep the internal handler mapping in
the RED plan, but label it as implementation wiring rather than a public verb.

### HIGH-3 — the rejected status-78 sentinel can survive because the post-READY complement is not an acceptance example

**Dimension:** Lifecycle boundary completeness / mutation resistance /
operator-exit honesty
**Evidence:** `design/wave-decisions.md:117-127,181-187`,
`feature-delta.md:188-211`, `distill/test-scenarios.md:237-251,340-365`,
`docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md:120-137`

The amendment was required because setup failure once used `EXIT 78`, and the
design explicitly rejects reserving 78 (or any status) because an operator
command may return the same status. S-GTI-08 proves only the new pre-READY
failure half. The existing cross-feature operator-exit example uses status 7,
not 78. Therefore an implementation can retain a special post-READY `EXIT 78`
classification and still satisfy every named example. The completeness audit
asserts that post-READY `EXIT` is operator-only, but no acceptance example
kills this exact historical mutant.

**Required remediation:** Add a traceable real-VM complement, either in this
package or by explicitly strengthening and reusing S-VM-02: network setup
succeeds, READY precedes EXEC, the operator command exits **78**, and
`overdrive workload describe <id>` reports the ordinary operator/guest result
with exit code 78, never a setup rejection or `VmGuestExitUnreported`. This
requires no Beacon, enum, or describe-shape addition.

### MEDIUM-1 — S-GTI-08 hides its operator outcome behind private test state and implementation actions

**Dimension:** User-observable acceptance language / hexagonal boundary
**Evidence:** `distill/test-scenarios.md:237-251,263-296`,
`design/wave-decisions.md:153-175`

The Tier-3 scenario's only operator action is deploy, yet its Given injects
nonzero private and durable restart state and its Then asserts
`WorkloadLifecycleView`, `FinalizeFailed`, and exact internal reason shapes.
It never has the operator run `overdrive workload describe`, even though the
approved design identifies describe as the unchanged surface that renders the
terminal state, reason, selected detail, exact exit code, and restart count.

The internal checks are valuable, but putting them in the metal Gherkin makes
the acceptance outcome implementation-shaped and invites test-only state
injection into a production-path scenario.

**Required remediation:** Split the two layers. Let the metal scenario drive
deploy and describe and assert the operator-visible `Failed` result,
actionable selected detail, exact exit code, and unchanged restart count. Keep
the seeded private View, `FinalizeFailed`-only action set, and View equality in
the already-planned source-local reconciler example. Preserve the unchanged
public describe schema.

### MEDIUM-2 — “0 contradictions” conceals a live restart-policy conflict

**Dimension:** Wave-decision reconciliation / product-contract traceability
**Evidence:** `distill/test-scenarios.md:28-38`, `feature-delta.md:651-662`,
`docs/product/journeys/run-a-vm-workload.yaml:110-123`,
`docs/feature/microvm-driver-cloud-hypervisor/distill/test-scenarios.md:1583-1598`

The reconciliation says it read `run-a-vm-workload.yaml` and found zero
contradictions. That journey still promises that an unreported guest death is
retried under backoff. The live Job implementation and the later microVM
DISTILL clarification instead make a VM Job run-once and explicitly call the
backoff wording inapplicable. The new Q7 contract also requires its pre-READY
unreported exit to finalize with unchanged restart counts.

The effective no-restart behavior is clear enough to implement, so this is not
an additional High finding, but the reconciliation record is not truthful and
can reintroduce the restart loop through a future reader.

**Required remediation:** Record the conflict and its authoritative
resolution instead of claiming zero contradictions. Correct the stale product
journey, or cite the later ratified Job run-once decision as superseding that
sentence and identify a follow-up owner for the journey cleanup. Re-run the
contradiction count after HIGH-1 is resolved.

## Product-contract disposition

| Contract area | Disposition |
|---|---|
| Deterministic pre-READY init failure | Sound |
| Terminal `Failed` observability and exact VMM code | Sound internally; operator describe proof needs MEDIUM-1 |
| No operator command / no Running / no EXEC on setup failure | Sound |
| Unchanged private and durable restart counts | Sound for S-GTI-08 |
| No restart loop | Sound for S-GTI-08; contradicted by S-GTI-06 trigger wording and reconciliation |
| Beacon / public describe shape unchanged | Sound |
| Born-captured first connection | Sound and materially strengthened |
| Restart re-enrollment | Not proved through a reachable same-allocation trigger |
| Teardown | Intended behavior sound; public command invalid |
| Post-READY operator exit honesty | Missing the historical status-78 complement |

## Verdict

**REJECTED_PENDING_REVISIONS.** The core Q7/Q9 correction is strong, but three
High defects leave the restart branch, teardown driving port, and operator-exit
boundary inadequately or incorrectly specified. Remediate the five findings,
refresh the scenario/traceability counts, and repeat product-owner review before
DELIVER resumes.

---

## Iteration 2 — 2026-08-29

**Reviewed commit:** `a5bd158edb76308a44ccf99597d349950898f0f7`
**Compared with:** `7255c68e64b7f0f15da5a2ed8a033806a2939e6a`
**Verdict:** **REJECTED_PENDING_REVISIONS**

### Iteration-2 finding counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 1 |
| Medium | 0 |
| Low | 0 |

### Prior-finding dispositions

| Iteration-1 finding | Disposition | Evidence |
|---|---|---|
| HIGH-1 — S-GTI-06 did not reach the same-allocation restart gate | **PARTIALLY RESOLVED; HIGH remains** | S-GTI-06a/06b now correctly drive unclean serve restart → platform reclamation → same-`AllocationId` re-drive, and explicitly exclude natural Job exit and generation replacement (`test-scenarios.md:73-79,243-260`). However the authoritative DESIGN decision, ADR, and DESIGN section of the feature delta still claim that restart budget, crash recovery, and `overdrive workload restart` are all live same-allocation VM routes (`design/wave-decisions.md:18`; ADR-0089 `:56-62`; `feature-delta.md:570-577`). |
| HIGH-2 — S-GTI-12 used a nonexistent stop command | **CLOSED** | S-GTI-12a/12b use the real `overdrive job stop <target-id>` port and preserve exact siblings (`test-scenarios.md:315-330`). The feature-delta operator map now distinguishes workload describe from Job stop (`feature-delta.md:891-898,920-922,1008-1010`). E09 uses the same real command. |
| HIGH-3 — no post-READY status-78 complement | **CLOSED** | S-GTI-08b now drives READY → EXEC → operator exit 78 and asserts describe reports the ordinary exit-78 result, never setup rejection or unreported guest exit, with no restart attempt (`test-scenarios.md:282-288`). `C-GTI-08-EXIT78` and E08 independently anchor the same discriminator (`:391-392,474`; E08 README `:7-12,41-50`). |
| MEDIUM-1 — S-GTI-08 mixed operator and private component facts | **CLOSED** | S-GTI-08a observes only deploy/describe state, diagnostic, durable count, forbidden effects, cleanup, and sibling preservation (`test-scenarios.md:269-280`). The seeded View, exact finalization action, View equality, and no-restart facts moved to `C-GTI-08-RECONCILE` (`:380-389`). No public describe shape was added. |
| MEDIUM-2 — reconciliation falsely claimed zero restart-policy contradictions | **CLOSED for implementation; journey cleanup remains explicitly downstream** | The reconciliation now records the stale product-journey retry sentence, selects the later Job run-once/platform-reclamation rule as authoritative, and assigns journey cleanup to Product/Journey ownership (`test-scenarios.md:26-36`; `feature-delta.md:824-831`). It no longer reports zero contradictions. |

### HIGH-1 — the corrected restart route is still contradicted by authoritative DESIGN text

**Dimension:** Cross-wave coherence / authoritative contract integrity /
restart-loop safety
**Evidence:** `design/wave-decisions.md:18`,
`docs/product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md:56-62`,
`feature-delta.md:570-577`, contrasted with
`distill/test-scenarios.md:26-36,73-79,243-260` and
`feature-delta.md:810,824-831,895-898,992-994`

The DISTILL remediation now has the right product behavior: a VM Job's natural
exit finalizes run-once; `overdrive workload restart` mints a fresh allocation;
only platform reclamation with standing intent exercises the same-allocation
restart install path. But three effective DESIGN statements still say the
opposite:

- D6 says restart budget, crash recovery, and `overdrive workload restart` are
  “all live for VM” and are the reason the restart gate must flip;
- ADR-0089 repeats that exact route list as the accepted architectural reason;
- the DESIGN portion of `feature-delta.md` says every listed route applies to
  VM kind.

This is not harmless history: these are the sources DISTILL itself names as
authoritative, and the same current feature delta contains both claims. A
crafter can follow the accepted D6/ADR language and introduce a Job
restart-budget loop, or use generation replacement and exercise the fresh gate
while believing the same-id restart proof is satisfied. The downstream
scenario is now correct, but the cross-wave contract remains contradictory.

**Required remediation:** Amend D6, ADR-0089 §1, and the DESIGN D6 narrative in
`feature-delta.md` to name platform reclamation with standing intent as the
reachable same-`AllocationId` VM Job route. State that natural Job exit/crash
finalizes without restart and that `overdrive workload restart` creates a fresh
AllocationId and therefore uses the fresh-start gate. Preserve the requirement
to flip both install sites and the S-GTI-06a/06b outcome; only its route
rationale changes. Re-run the contradiction scan after all three authoritative
locations agree.

### Product and EDD regression check

- No previously accepted operator or security outcome was weakened in the
  remediated DISTILL scenarios. Pre-READY failure remains terminal and
  non-executing, restart counts remain unchanged, post-READY exit 78 remains an
  ordinary Job result, Beacon/describe schemas remain unchanged, and the D7
  witness strengthens rather than narrows the born-captured promise.
- E07 reflects the real built-binary serve → deploy → describe → Job-stop
  journey and requires exact lifecycle, wire, kernel, and cleanup evidence.
- E08 reflects two real and discriminating deploy/describe journeys: a
  deploy-selected resolver-failure rootfs and a READY/EXEC command exiting 78.
- E09 reflects the production-reachable unclean serve restart on the same data
  directory, same-id platform reclamation, successful/failed reinstall, and
  exact `overdrive job stop` behavior with sibling preservation.
- E07/E08/E09 are honestly marked `pending`; their README contracts require
  built commands and native evidence, while their runner files explicitly say
  no command or evidence exists. Implementing the native runner, global lease,
  and evidence capture is a valid roadmap/DEVOPS downstream obligation and is
  not counted as an unresolved DISTILL Medium.

### Iteration-2 verdict

**REJECTED_PENDING_REVISIONS.** Four of five product-owner findings are closed,
and the new EDD stubs describe real operator journeys without claiming evidence.
HIGH-1 remains because the corrected S-GTI-06 route is contradicted by the
effective D6, ADR-0089, and DESIGN feature-delta text. Align those three sources
with the already-correct DISTILL contract, then repeat product-owner review.

---

## Iteration 3 — 2026-08-29

**Reviewer:** `nw-product-owner-reviewer` (GPT 5.6 Luna, maximum thinking)
**Reviewed commit:** `ed332f8972c6285fb067d995d73f51ee63a5ff01`
**Compared with:** `85550e4a267cbd53ac266fa54f4d8cda164910af`
**Verdict:** **APPROVED**

### Iteration-3 finding counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

### Prior-finding disposition

| Prior finding | Disposition | Evidence |
|---|---|---|
| Iteration-2 HIGH-1 — the same-allocation route was contradicted by authoritative DESIGN text | **CLOSED** | Effective parent commit `85550e4a267cbd53ac266fa54f4d8cda164910af` aligns D6, ADR-0089, and the DESIGN feature-delta narrative. All three now state that unclean control-plane restart with standing intent produces boot-epoch Platform Reclamation and same-`AllocationId` `RestartAllocation`; natural VM Job result/crash finalizes run-once, while `overdrive workload restart` mints a fresh allocation (`design/wave-decisions.md:18,260-279`; ADR-0089 `:57-73`; `feature-delta.md:570-586,823-825`). S-GTI-06a/06b and E09 use that exact route. |

### Product and operator coherence

| Outcome | Iteration-3 disposition |
|---|---|
| Fresh guard-install rejection | **COHERENT AND REAL.** S-GTI-05/E08 drive an initial built-binary deploy against a test-owned regular, hookless `prerouting` chain. The production ensure and production TPROXY append run; the kernel rejects the actual append. No injection seam substitutes for the failing production port, and fixture restoration is separately proved against the saved nft/FIB baseline. |
| Same-allocation reinstall rejection | **COHERENT, DISTINCT, AND REAL.** S-GTI-06b/E09 first establish a Running VM Job, terminate `overdrive serve` uncleanly, restart against the same durable data, observe Platform Reclamation and reuse of the allocation id, and require the real production reinstall to receive a deterministic kernel rejection. They explicitly exclude natural Job crash, restart budget, and `overdrive workload restart`, so this cannot collapse into either the fresh path or a new allocation. |
| Pre-READY resolver failure | **COHERENT AND OPERATOR-VISIBLE.** S-GTI-08a/E08 use a deploy-selected custom rootfs that makes the production resolver write fail, then observe one terminal Failed attempt, detail, exact available VMM exit code, unchanged durable counts, forbidden READY/Running/EXEC effects, cleanup, and sibling preservation through deploy/describe. Private View/action-vector assertions remain in `C-GTI-08-RECONCILE`. |
| Post-READY status 78 | **COHERENT AND DISCRIMINATING.** S-GTI-08b/E08 drive a built deploy whose successful lifecycle orders READY before EXEC and EXIT, and `overdrive workload describe <id>` must report ordinary Job exit code 78 rather than setup rejection or unreported guest exit. This is the real complement to the two pre-READY failure journeys. |
| Describe and canonical address | **COHERENT.** S-GTI-05/06a/06b/07/08a/08b and E07/E08/E09 use the existing `overdrive workload describe <id>` port. They add no Published Language or describe-schema field; S-GTI-07 reads the existing canonical address as the guest /30 address rather than the transit hop. |
| Job stop and inverse | **COHERENT AND REAL.** S-GTI-12a/12b and E07/E08/E09 use the actual `overdrive job stop <id>` verb, not the nonexistent `workload stop`. E09 requires exact target deletion, sibling sequence equality after filtering the target handle, and the observable Stopped → AlreadyStopped idempotent result for the no-rule inverse. |
| Born-captured stakeholder outcome | **COHERENT.** S-GTI-01 remains stakeholder language; S-GTI-02/E07 own the exact D7 lifecycle, capture, kernel, TLS/no-cleartext, and cleanup mechanics. No test-only identity or production wiring was reintroduced into the walking skeleton. |

### EDD journey separation and evidence honesty

- E07 is the successful built serve → deploy → describe → Job-stop journey and
  owns born-captured/D7/TLS evidence.
- E08 contains three deliberately discriminating built-binary journeys: fresh
  production guard-install rejection, custom-rootfs pre-READY resolver failure,
  and successful READY/EXEC followed by ordinary exit 78.
- E09 owns the different same-id lifecycle: unclean serve restart on the same
  data, platform reclamation, successful or failed production reinstall, and
  exact Job-stop preservation/idempotency.
- All three remain honestly marked `pending`. Their READMEs require native,
  non-virtualized x86_64 KVM, the pre-sync outer lease, built commands, bounded
  state/wire/kernel/cleanup evidence, and independent review. The runner stubs
  explicitly say that no command ran and no evidence exists; implementation and
  capture remain the roadmap/DEVOPS obligation, not a DISTILL pass claim.

### Verification

- Reviewed the complete target delta and the effective upstream D6/ADR-0089
  correction at parent `85550e4a267cbd53ac266fa54f4d8cda164910af`.
- `git diff --check 85550e4a267cbd53ac266fa54f4d8cda164910af..ed332f8972c6285fb067d995d73f51ee63a5ff01`
  passed.
- No source or runtime test was executed for this documentation-only review;
  `red-classification.md` and E07/E08/E09 truthfully remain `NOT_EXECUTED` /
  `pending`.

### Iteration-3 verdict

**APPROVED.** The sole iteration-2 High is closed in every effective
authoritative source. The DISTILL contract preserves the accepted security and
operator outcomes, uses the production-reachable same-allocation route, and
keeps fresh failure, pre-READY resolver failure, failed reinstall, post-READY
exit 78, describe, and Job stop as distinct executable journeys. There are zero
Blocker, High, Medium, or Low findings.

---

## Iteration 4 — 2026-08-29

**Reviewer:** `nw-product-owner-reviewer` (GPT 5.6 Luna, maximum thinking)
**Reviewed commit:** `558589a7a3ee14ebea0b8cdb6496b65a5830f777`
**Compared with:** `ed332f8972c6285fb067d995d73f51ee63a5ff01`
**Verdict:** **APPROVED**

### Iteration-4 finding counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

### Approved-outcome regression check

| Changed area | Product-owner disposition |
|---|---|
| Fresh guard-install fixture | **APPROVED; now executable as specified.** S-GTI-05 returns to stakeholder language while E08 owns the mechanics. The EDD contract starts from a clean baseline, preflights the pinned appliance kernel, lets production observe `EEXIST` for its production-named `prerouting` chain, and requires the real production `append-egress` / `append-rule` to fail with typed `-EOPNOTSUPP` on an INPUT-hook base chain. The linked upstream Linux implementation confirms that TPROXY validates only a prerouting-reachable base-chain hook and returns `-EOPNOTSUPP` for the wrong base-chain hook. No injected result or test-owned success path replaces the production port. |
| Install-failure lifecycle | **APPROVED; corrected without weakening fail-closed behavior.** The artifacts now acknowledge the production ordering in which a durable Running row can briefly precede the installer and then be superseded by terminal Failed. The user-facing invariant remains the meaningful one: final Failed with typed detail, no EXEC release or operator marker, no guest frame/cleartext, and bounded cleanup. The separate resolver failure remains pre-READY and therefore still forbids READY and Running. |
| Same-id failed reinstall | **APPROVED; distinct and production-reachable.** E09 establishes a Running target and durable intent, ends serve uncleanly, installs the wrong-hook fixture, restarts against unchanged durable data without another deploy, and requires Platform Reclamation plus `RestartAllocation` for the same allocation id before the real restart-arm append receives `-EOPNOTSUPP`. Natural crash, restart budget, generation replacement, and a fresh allocation remain invalid substitutes. |
| Successful reclamation and Job stop | **APPROVED; preserved separately from destructive failure setup.** E09 keeps successful same-id re-enrolment, first-flow D7/TLS protection, exact `overdrive job stop <id>` target deletion, sibling-sequence preservation, and Stopped → AlreadyStopped idempotency in its non-destructive journey. The failed-reinstall fixture runs sibling-free and cannot masquerade as sibling-preservation evidence. |
| Describe and exit 78 | **APPROVED; unchanged.** Fresh failure, resolver failure, reclamation, canonical address, and ordinary command results remain observable through the existing bounded `overdrive workload describe <id>` port. E08 still proves READY → EXEC → EXIT 78 and reports 78 as an ordinary Job result, not setup rejection, with no restart consumption. No public describe or Beacon shape is added. |
| Canonical completeness | **APPROVED; more honest.** The audit now records C4a as the one unmapped duplicate-create gap and reports 14/15 rather than a false 15/15. Fourteen covered categories satisfy the canonical completeness threshold; the missing attempt-owned-resource replay AT is explicitly carried as `AT_GAP_IN_DELIVERY_SCOPE` and is not represented as executed evidence. This disclosure does not remove or blur any stakeholder outcome. |
| Native EDD evidence boundary | **APPROVED as a downstream prerequisite.** E07/E08/E09 now require the canonical host-global lease across every Run, Sync, and supported direct-bootstrap writer before shared-tree mutation, with Run ownership through final probes. They explicitly invalidate runtime claims until that boundary and the native runner exist. This strengthens evidence integrity without changing the public product journey. |

### EDD journey integrity

- E07 remains the successful built serve → deploy → describe → Job-stop proof
  for the born-captured first mesh dial, exact D7 accounting, TLS/no-cleartext,
  and cleanup.
- E08 remains three product-distinct built-binary journeys: fresh production
  guard-install rejection, deploy-selected pre-READY resolver failure, and a
  successful READY/EXEC command whose ordinary result is exit 78.
- E09 now makes its two responsibilities operationally explicit: a
  non-destructive successful reclamation/stop journey with siblings, and a
  sibling-free failed-reinstall journey with same-id restart-arm evidence and
  assertion-safe restoration to the target-filtered nft/FIB baseline.
- All expectations remain `pending`, all runner files state that no command ran
  and no evidence was fabricated, and `red-classification.md` continues to
  report `NOT_EXECUTED`. The fixture, lifecycle, lease, and completeness edits
  therefore refine the DELIVER contract without self-certifying it.

### Verification

- Reviewed the complete commit delta, effective upstream lifecycle/design
  artifacts, DISTILL scenarios/classification, E07/E08/E09 contracts and stubs,
  expectation index, and the linked upstream Linux TPROXY hook-validation path.
- `git diff --check ed332f8972c6285fb067d995d73f51ee63a5ff01..558589a7a3ee14ebea0b8cdb6496b65a5830f777`
  passed.
- No source or runtime test was executed for this documentation-only product
  review.

### Iteration-4 verdict

**APPROVED.** The fixture correction is a real production-port kernel failure,
the transient-Running clarification matches the existing lifecycle while
preserving no-EXEC/final-Failed behavior, the failed-reinstall and successful
reclamation/stop journeys remain distinct, and the completeness and EDD changes
are explicit about their gaps and prerequisites. Every previously approved
product and operator outcome remains intact. There are zero Blocker, High,
Medium, or Low findings.
