# STEP 12-01 guest-network error variants — independent DESIGN review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-GUEST-NETWORK-ERROR-VARIANTS-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Iteration | 1 |
| Scope | The bounded post-attachment-cut `GuestNetworkError` and `GuestNetworkFact` reconciliation, ADR-0122, affected producer/caller paths, and STEP 12-01 roadmap consistency |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 0 critical, 1 high |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

The reviewer definition and the `nw-sar-critique-dimensions` and
`nw-roadmap-review-checks` skills were loaded. `AGENTS.md` and the applicable
repository DESIGN, development, workflow, and testing rules govern this
review, including exact public API shape, the typed-error contract gate,
contract-weakening review, production-path evidence, and no speculative
mechanism.

This review changes only this native Markdown review artifact. It changes no
production source, test body, reviewed design artifact, roadmap validation
field, DES event, execution log, model, lane, or kernel state. It performs no
Metal run, mutation run, commit, or implementation review.

## Artifacts and evidence reviewed

- `docs/feature/netns-density-295/feature-delta.md`, especially the bridge-era
  deletion inventory, production composition, `GuestFlowIo` contract, typed
  error taxonomy, Contract Impact Matrix, and Earned Trust probe contract.
- `docs/product/architecture/adr-0122-guest-network-operation-error-family.md`.
- `docs/feature/netns-density-295/deliver/roadmap.json`, especially STEP
  12-01's exact deletion criterion and validation state.
- The current pre-cut public types and test-only references in
  `crates/overdrive-control-plane/src/guest_network.rs`,
  `guest_network/owner_acceptance/errors.rs`, and
  `tests/acceptance/netns_density_guest_network.rs`.
- The phase-1 caller path in `crates/overdrive-control-plane/src/lib.rs`, the
  `GuestFlowIo` boundary in `guest_network/flow_io.rs`, and the existing RED
  scaffold in `guest_network.rs`.
- The approved forwarder vocabulary in
  `crates/overdrive-core/src/guest_transport/forwarder.rs` and the producer
  implementation in `crates/overdrive-fwd/src/lib.rs`.
- The complete dirty diff of the feature delta, ADR-0122, and roadmap
  validation block. `roadmap.json` parses and the reviewed diff passes
  `git diff --check`.

## Bounded reconciliation assessment

Deleting `ScratchCleanupIncomplete` and `StartupProbeCleanup` at the
attachment cut is architecturally correct. STEP 12-01 already deletes the
bridge/TAP scratch seam, `GuestNetworkProbeStage`, `GuestNetworkScratch*`, and
the scratch `GuestNetworkFact` variants with their only owner and tests
(`roadmap.json:1058`). The production inventory identifies those types and
variants only as bridge-era code to delete (`feature-delta.md:1920`). In the
current pre-cut tree, the two error variants have no production producer or
branch consumer: outside their definitions, their live references are an
exhaustive acceptance-test taxonomy name and bridge-era acceptance helpers.
Those test surfaces are in the same deletion boundary.

The replacement does not need a compatibility variant. `GuestNetworkError`
is a process-local orchestration error and neither it nor `GuestNetworkFact`
has a wire or persisted representation. The exact post-cut set at
`feature-delta.md:8554-8569` is closed and supplies every retained variant's
payload. It retains the allocation-scoped `CleanupComplement`; only the
deleted node-level scratch complement and startup-probe fact disappear
(`feature-delta.md:8623-8632`).

### Producer and caller paths

The affected paths have one coherent owner and outer mapping:

1. Phase 1 calls `probe_guest_flow_startup`, which performs the four retained
   host checks and then `GuestFlowIo::probe_forwarder`. The seam owns a
   process-local forwarder context and releases it before return on success or
   failure (`feature-delta.md:7380-7404,7483-7497,10742-10762`; current port
   declaration `flow_io.rs:279-296`).
2. `run_server`'s phase-1 wrapper converts the returned
   `GuestNetworkError` into `ControlPlaneError` and emits
   `health.startup.refused` with reason `guest_network.probe`
   (`lib.rs:2672-2695`). The observable refusal point and boot ordering do not
   change.
3. Boot step 9 and the supervisor's `Forwarder` recovery call
   `open_forwarder`; the seam opens and probes the serving context before it
   is used and returns `ForwarderError` on failure. The accepted `#[from]`
   wrapper is `GuestNetworkError::Forwarder { source }`
   (`feature-delta.md:7459-7472,10755-10762`).
4. `ForwarderProbeStage` already contains `ContextRelease` and `Cleanup`, and
   `ForwarderError` already contains the exact `Probe { stage, source }` and
   `ProbeMismatch { stage, observed }` carriers
   (`forwarder.rs:437-483,503-595`). No new public variant, payload, stage, or
   conversion is required.

The current host producer matches that design. `probe_wrap` preserves an
already staged `Probe` or `ProbeMismatch` and wraps every other typed
`ForwarderError` with the active stage (`overdrive-fwd/src/lib.rs:481-495`).
The context-release probe classifies operation failures at
`ContextRelease` and a far end that was not reset as
`ProbeMismatch { ContextRelease, FarEndNotReset }` (`:989-1004`). The final
inventory and queued-end checks classify operation failure at `Cleanup` and
semantic residue as
`ProbeMismatch { Cleanup, ResidueRemains { pairs } }` (`:1007-1044`).

### Exact semantic error mapping

| Designed condition | Post-cut typed representation | Owner / projection | Assessment |
|---|---|---|---|
| Bridge/TAP scratch reverse cleanup reports a non-empty complement | Deleted with `GuestNetworkScratchComplement` and its producer; no replacement condition exists | Deleted scratch owner | Correct deletion |
| Bridge/TAP startup primary failure plus independent reverse-cleanup failure | Deleted with the independent scratch cleanup owner; no aggregate or string substitute | Deleted scratch owner | Correct deletion |
| Forwarder cleanup operation fails | `GuestNetworkError::Forwarder { source: ForwarderError::Probe { stage: Cleanup, source } }` | Phase 1 or serving-context open; existing startup/converge refusal | Complete |
| Forwarder cleanup inventory or end queue is non-zero | `GuestNetworkError::Forwarder { source: ForwarderError::ProbeMismatch { stage: Cleanup, observed: ResidueRemains { pairs } } }` | Same | Complete |
| Context-release operation fails | `GuestNetworkError::Forwarder { source: ForwarderError::Probe { stage: ContextRelease, source } }` | Same | Complete |
| Released test context does not reset its far end | `GuestNetworkError::Forwarder { source: ForwarderError::ProbeMismatch { stage: ContextRelease, observed: FarEndNotReset } }` | Same | Complete |
| Any other forwarder probe stage fails or mismatches | The same `ForwarderError::Probe` / `ProbeMismatch` with that exact stage, carried unchanged by `GuestNetworkError::Forwarder` | Same | Complete |

The mapping does not flatten a typed cause, invent an operation string, or
collapse a semantic mismatch into generic I/O. The old simultaneous
primary-plus-cleanup payload protected an independently mutable bridge/TAP/map/
pin/guard scratch universe. That universe and its second cleanup owner are
gone. The forwarder's process-local context is the ownership boundary: its
release ends its pairs and watches, while the explicit `Cleanup` probe still
requires zero inventory before a successful return. Removing the old aggregate
therefore removes obsolete machinery; it does not relax the zero-residue,
fail-before-boot, source-preservation, retry, cleanup, or operator-visible
outcomes.

No new spike, regression test, simulation seam, model state, lane, retry,
recovery protocol, persistence boundary, or runtime cleanup subsystem is
warranted for this consistency correction. The existing forwarder contract
and producer already supply the replacement distinctions.

## Finding

### F-01 — The Contract Impact Matrix and STEP 12-01 still require a different public error variant

- **Severity:** High; blocks DESIGN approval under the exact API shape and
  typed-error contract gates.
- **Claim:** The newly reconciled authoritative taxonomy says the post-cut
  public carrier is exactly
  `GuestNetworkError::Forwarder { #[from] source: ForwarderError }` with no
  `operation` field (`feature-delta.md:8554-8569`). Two implementation-facing
  declarations still require `Flow { operation, source }` instead.
- **Evidence:** Contract Impact Matrix row 11 names the replacement as
  “`Flow { operation, source }`” (`feature-delta.md:10622`). STEP 12-01 repeats
  “`GuestNetworkOperation/Flow{operation, source}` taxonomy” in its step-7
  criterion (`roadmap.json:1038`). The repository contains no post-cut
  `GuestNetworkError::Flow` contract, while the affected phase-1 and
  serving-context ports return `ForwarderError` and the reviewed producer
  emits its own closed `ForwarderProbeStage`.
- **Consequence:** A STEP 12-01 crafter cannot satisfy both contracts without
  inventing or choosing public API surface: either add the undocumented
  `Flow { operation, source }` variant and operation payload, or implement the
  exact `Forwarder { source }` set and violate the matrix and roadmap. Green
  tests cannot resolve that design-owned choice.
- **Required revision:** Reconcile Contract Impact Matrix row 11 and the
  STEP 12-01 criterion to the same exact
  `Forwarder { #[from] source: ForwarderError }` carrier pinned at
  `feature-delta.md:8554-8569`. Preserve the reviewed stage mappings and make
  no new variant, operation discriminator, compatibility alias, mechanism,
  test, lane, or model.
- **Disposition:** Open.

## Architecture review dimensions

| Dimension | Assessment | Evidence |
|---|---|---|
| Architectural bias | Pass | The correction removes a deleted mechanism and reuses the already-approved forwarder taxonomy. |
| ADR quality | Pass for the bounded correction | ADR-0122 states context, decision, consequence, ownership, and the authoritative feature-delta home; the pre-cut paragraphs are explicitly scoped and superseded for the post-cut API. |
| Completeness | **Fail pending F-01** | Exact variants, payloads, facts, mappings, owner, and outcomes are complete in the taxonomy, but two downstream contract declarations still prescribe another variant. |
| Feasibility and testability | Pass | The existing forwarder types and producer implement every required stage and mismatch without new surface; current RED scaffolds identify the implementation boundary. |
| Priority and data justification | Pass | This is a bounded consistency defect in the active STEP 12-01 contract, not a new production-failure hypothesis. |
| Effect isolation / Contract Shape | Pass | The probe boundary is process-local and releases its context before return; retained failure data is typed and no new capability or write surface is added. |

## Roadmap review

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass for this correction | Phase 1 is invoked by `run_server`; serving-context open is invoked at boot step 9 and recovery. |
| Acceptance-criterion coupling | **Fail pending F-01** | Exact public error shape is DESIGN-owned and must be singular; the criterion currently names the conflicting `Flow` shape. |
| Step decomposition | Pass | Both obsolete variants and facts are deleted with their sole bridge/TAP scratch owner inside STEP 12-01. |
| Implementation code | Pass | The reconciliation pins public variants and observable mappings without prescribing private helper structure. |
| Concision and precision | **Fail pending F-01** | The bounded reconciliation is precise, but the matrix and roadmap give the crafter two incompatible public contracts. |
| Test boundary | Pass | Existing source-local acceptance coverage observes the typed phase-1 result; forwarder integration coverage owns the adapter stages. No new cell or lane is needed. |
| Mechanical validity | Pass | `roadmap.json` parses; the design/roadmap diff passes `git diff --check`; validation correctly remains pending. |

## Iteration 1 verdict

**CHANGES_REQUESTED.** The proposed deletion of
`ScratchCleanupIncomplete`, `StartupProbeCleanup`, their scratch payload, and
the obsolete scratch facts is correct. The replacement
`ForwarderError::Probe` / `ProbeMismatch` stage mapping is exact,
source-preserving, reachable through the existing phase-1 and serving-context
ports, and preserves every accepted outcome and invariant without new
mechanism or validation work.

Approval is blocked only by F-01: the Contract Impact Matrix and STEP 12-01
still prescribe `Flow { operation, source }` while the authoritative exact set
prescribes `Forwarder { source: ForwarderError }`. Until those two declarations
are reconciled and independently re-reviewed, roadmap validation must remain
`pending` and ADR-0122's bounded reconciliation must remain proposed. After
F-01 closes with the already-pinned `Forwarder` carrier, the roadmap may be
restored to `approved` and ADR-0122 may fold the proposed reconciliation into
its accepted status; no additional user decision is required.

---

# Review iteration 2 — exact carrier reconciliation

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-GUEST-NETWORK-ERROR-VARIANTS-ARCH-REVIEW-02` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | F-01 remediation in Contract Impact Matrix row 11 and STEP 12-01; iteration 1's deletion, mapping, ownership, invariant, and scope assessments remain incorporated |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This iteration changes only this review artifact. It changes no reviewed
design artifact, roadmap validation field, production source, test body, DES
event, execution log, model, lane, or kernel state. No new Metal run, mutation
run, spike, or implementation test was needed: F-01 was an exact-contract
documentation conflict, and iteration 1 already verified the affected
producer and caller paths.

## F-01 remediation verification

F-01 is closed. The two formerly contradictory declarations now name the
same carrier as the authoritative typed-error taxonomy:

- Contract Impact Matrix row 11 now replaces the pre-cut
  `GuestNetworkError::Tcx` surface with
  `Forwarder { #[from] source: ForwarderError }`
  (`feature-delta.md:10622`).
- STEP 12-01 now requires the exact
  `GuestNetworkError::Forwarder { source: ForwarderError }` taxonomy and
  explicitly states that this variant has no `operation` field
  (`roadmap.json:1038`).
- Repository-wide searches of the reviewed feature delta, ADR-0122, and
  roadmap find no remaining `Flow { operation, source }`,
  `GuestNetworkOperation/Flow`, or `GuestNetworkError::Flow` declaration.

The remediation does not add or remove any other variant, field, conversion,
operation discriminator, fact, owner, caller, outcome, or validation item. It
therefore preserves iteration 1's verified mapping:

| Failure | Exact carrier after remediation |
|---|---|
| Cleanup operation failure | `GuestNetworkError::Forwarder { source: ForwarderError::Probe { stage: Cleanup, source } }` |
| Cleanup residue | `GuestNetworkError::Forwarder { source: ForwarderError::ProbeMismatch { stage: Cleanup, observed: ResidueRemains { pairs } } }` |
| Context-release operation failure | `GuestNetworkError::Forwarder { source: ForwarderError::Probe { stage: ContextRelease, source } }` |
| Context-release semantic mismatch | `GuestNetworkError::Forwarder { source: ForwarderError::ProbeMismatch { stage: ContextRelease, observed: FarEndNotReset } }` |

The complete post-cut `GuestNetworkError` set at
`feature-delta.md:8554-8569`, the fact deletion at `:8623-8632`, the Contract
Impact Matrix, and STEP 12-01 now provide one exact public API contract. A
crafter can delete `ScratchCleanupIncomplete`, `StartupProbeCleanup`, their
payload/support types, and the obsolete scratch facts without inventing a
replacement carrier or choosing between incompatible signatures.

## Finding disposition

| Finding | Iteration 1 | Iteration 2 evidence | Disposition |
|---|---|---|---|
| F-01 — matrix and roadmap require `Flow { operation, source }` while the exact set requires `Forwarder { source }` | High, blocking | Both declarations now require the exact `ForwarderError` carrier; stale `Flow` declarations are absent | **CLOSED** |

No new critical, high, or medium finding is present. The bounded correction
still removes only error and fact surface whose bridge/TAP scratch owner is
deleted by the already-accepted attachment cut. It preserves the phase-1 and
serving-context refusal boundaries, typed causes, zero-residue requirement,
context release, boot ordering, retry behavior, wire and persistence formats,
and operator-visible outcomes. It introduces no mechanism or contract
weakening.

## Iteration 2 roadmap recheck

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | `run_server` invokes phase 1, and boot step 9 plus recovery invoke serving-context open. |
| Acceptance-criterion coupling | Pass | STEP 12-01 names the design-owned exact public carrier while leaving private implementation structure to the crafter. |
| Step decomposition | Pass | The variants and scratch facts leave with their sole bridge/TAP scratch owner in STEP 12-01. |
| Implementation code | Pass | The artifacts pin interface and error semantics without prescribing private bodies. |
| Concision and precision | Pass | The exact set, matrix, and criterion agree on `Forwarder { source: ForwarderError }` with no operation field. |
| Test boundary | Pass | Existing phase-1 acceptance and forwarder integration layers remain sufficient; no new test or lane is authorized. |
| Mechanical validity | Pass | `roadmap.json` parses; the reviewed design/roadmap diff passes `git diff --check`; validation remains pending until the approved reconciliation is folded. |

## Iteration 2 verdict

**APPROVED.** The attachment-cut deletion of
`ScratchCleanupIncomplete`, `StartupProbeCleanup`, their scratch payload and
support types, and the obsolete scratch facts is exact and complete. Every
post-cut startup and serving-context probe failure is carried through the
single approved `GuestNetworkError::Forwarder { source: ForwarderError }`
variant with the exact `Cleanup` or `ContextRelease` `Probe` /
`ProbeMismatch` distinction. No accepted outcome, invariant, owner, refusal
boundary, retry rule, wire format, persisted format, or security boundary is
weakened, and no new mechanism, spike, test, lane, or model is required.

The roadmap is eligible to be restored from `pending` to `approved` with this
independent DESIGN approval recorded. ADR-0122's bounded attachment-cut
reconciliation is eligible to be folded from proposed/pending-review wording
into the ADR's accepted status. No additional user decision is required.

---

# Supplement — fail-stop causes and obsolete supervisor VM-kill surface

## Supplement metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-SHARED-NETWORK-FAIL-STOP-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The bounded post-attachment-cut `SharedGuestNetworkFailStopCause` set, deletion of the shared-network supervisor's VM-kill surface, supplied-owner signature fallout, and the corresponding STEP 12-01 roadmap reconciliation |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This supplement changes only this review artifact. It changes no reviewed
feature delta, roadmap field, production source, test body, DES event,
execution log, model, lane, or kernel state. It performs no Metal run,
mutation run, spike, commit, or implementation test.

## Accepted authority and final outcome

ADR-0124's accepted 2026-10-11 amendment makes the correction mandatory. The
vsock attachment removes every per-VM TAP, TCX link, endpoint-map entry and
bridge-guard member that formerly determined the supervisor's VM-kill scope.
The amendment explicitly replaces TAP quiescence with counted forwarding
quiescence, replaces per-TAP damage with shared guest-flow components, and
replaces both per-VM and whole-workloads-slice kills with flow abort: a VM with
no forwarded flow emits nothing, shared-network recovery kills no VM, and a
failed recovery ends in the existing typed fail-stop
(`adr-0124-bounded-shared-network-owner-recovery.md:70-105`).

The reviewed correction does not choose a new recovery mechanism or weaken a
security outcome. It removes the unimplemented bridge-era mechanism that the
later accepted amendment already superseded. The retained sequence remains:
the first counted hold refuses admission and aborts every live flow; each
component repairs and audits under its own hold; a clean recovery restores
that hold; an exhausted, leaked, foreign, or abnormal supervisor outcome
fail-stops `serve`. VM lifecycle stop and boot reclamation remain separate
owners of allocation-scoped VM termination.

Earlier roadmap steps describe the bridge-era intermediate implementation
that STEP 12-01 explicitly reworks at the attachment cut. The final STEP
12-01 criterion and the accepted 2026-10-11 ADR amendment control the
post-cut landing reviewed here.

## Exact fail-stop vocabulary and mappings

The post-cut `SharedGuestNetworkFailStopCause` set is exactly eight unit
variants:

| Semantic condition | Exact variant | Production owner / mapping |
|---|---|---|
| Twenty failed attempts or the five-second component-recovery bound | `RecoveryDeadlineExceeded` | Shared-network supervisor's bounded recovery path |
| Owner reports a non-zero leaked quiescence-hold count at an audit tick | `LeakedQuiescenceHold` | Shared-network supervisor; forwarding remains quiesced |
| `restore` returns `QuiescenceRestoreError::ForeignHold` | `ForeignQuiescenceHold` | Shared-network supervisor maps the typed owner error immediately |
| Supervisor task returns `Ok(())` without a request | `SupervisorReturned` | `SharedNetworkSupervisorHandle::shutdown_requested` |
| Supervisor task returns a typed error | `SupervisorFailed` | `shutdown_requested` |
| Supervisor task panics | `SupervisorPanicked` | `shutdown_requested` from Tokio `JoinError` |
| Supervisor task is unexpectedly cancelled | `SupervisorCancelled` | `shutdown_requested` from Tokio `JoinError` |
| The fail-stop request channel closes | `RequestChannelClosed` | `shutdown_requested` |

Intentional shutdown remains the separate cancellation-and-join path and
produces no fail-stop cause. The five task/channel mappings exist in the
current production owner (`overdrive-control-plane/src/lib.rs:1288-1331`),
the typed foreign-hold mapping exists at `:1492-1526`, and the recovery-bound
mapping exists at `:1556-1588`. The leaked-hold mapping is already pinned by
QHOLD and the owner observation contract for the STEP 12-01 supervisor
rework. None of the eight variants carries an underspecified payload, string
classification, wire representation, or persisted representation.

`TapQuiescenceUndetermined` and `VmKillFailed` have no post-cut semantic
condition. In the current pre-cut tree they have no production producer or
branch consumer; outside the enum declaration they appear only in the
exhaustive acceptance cause table that DISTILL must align before DELIVER.
Deleting them therefore closes the public enum over reachable post-cut
outcomes without folding a semantic failure into a generic cause.

## Supervisor capability and ownership audit

The deleted VM-kill surface is entirely bridge-era and currently
unimplemented:

- `SHARED_NETWORK_VM_KILL_CALL_BOUND` is an unused RED-scaffold constant.
- `SharedNetworkSupervisorPorts::vm_kill` is the only supervisor field that
  grants VM-kill authority.
- `vm_kill::VmKillCapability::{kill_allocation, kill_workloads_slice}` are RED
  scaffold bodies. No production composition constructs the capability, and
  the supervisor body has no live call to either method.
- `CgroupPath::workloads_slice()` is a RED scaffold used by no caller outside
  that capability design.
- The `vm_cgroups` value is threaded through
  `run_server_with_obs_and_driver`, `run_server_with_obs_and_drivers`, and
  private `boot_server`, where the current code only assigns it to `_` for the
  future capability.

Removing these items restores the owner boundary required by the accepted
amendment: the shared-network supervisor can quiesce and abort flows, repair
components, audit, restore holds, and fail-stop; it cannot terminate VMs.
This is subtractive and leaves no dead compatibility capability.

## Retained VM lifecycle ownership

The correction explicitly retains the VM lifecycle surface that remains
reachable and required:

- `CgroupManager::cgroup_kill(&CgroupPath)` remains the VM driver's
  allocation-scoped cleanup mechanism for failed start, stop, and artifact
  disposal (`overdrive-worker/src/vm_driver.rs:1069-1074,1398-1405`).
- `CgroupPath::for_alloc` remains the canonical allocation scope used by the
  VM driver, boot reclamation, action-shim reclamation, simulations, and
  integration evidence.
- Boot reclamation still kills every unsupervised VMM before the first lease
  assignment and waits for its allocation-scoped host cleanup.
- Production `run_server` still creates the workloads slice and composes the
  VM driver over its own cgroup manager. Only the second manager reference
  formerly reserved for the shared-network supervisor disappears.

The deletion therefore does not weaken VM stop, crash cleanup, boot
reclamation, NO-SURVIVE, lease-release ordering, or the VM driver's ownership
of its process and allocation scope.

## Interface and compiler fallout

The feature delta now pins the two supplied-owner entry points without a
`CgroupManager` parameter:

```text
run_server_with_obs_and_drivers(config, boot, drivers,
    shared_guest_network, guest_network_exec)

run_server_with_obs_and_driver(config, boot, driver,
    shared_guest_network, guest_network_exec)
```

Private `boot_server` likewise loses `vm_cgroups`. The fallout is mechanical
and bounded: the single-driver helper stops forwarding the argument, the
registry helper and ordinary `run_server` stop passing it to `boot_server`,
and the repository's supplied-owner test/simulation callers remove their
neutral `CgroupManager::new(...)` argument and stale signature prose. Their
supplied `Driver` or `DriverRegistry` already owns VM lifecycle effects; the
parameter was not a capability those entry points otherwise used.

Deleting the two enum variants requires the existing exhaustive core
acceptance table to move from ten causes to the exact eight-cause set before
DELIVER compilation. The roadmap correctly assigns that test-body alignment
to DISTILL and allows only the necessary compiler fallout in STEP 12-01; it
does not authorize the crafter to invent a replacement variant or rewrite an
acceptance outcome.

## Invariant and scope audit

| Concern | Assessment |
|---|---|
| Admission during recovery | Preserved: the counted hold latches admission closed. |
| Live forwarded flows | Preserved: the first hold aborts all of them with the accepted reset semantics. |
| Bounded recovery | Preserved: 250 ms retry, twenty attempts and five seconds remain. |
| Terminal recovery failure | Preserved through `RecoveryDeadlineExceeded`; no VM-kill sub-operation remains to fail. |
| Leaked or foreign hold | Preserved as distinct typed fail-stop causes. |
| Abnormal supervisor ownership | Preserved as five distinct typed causes; intentional shutdown remains separate. |
| VM lifecycle stop and reclamation | Preserved under the VM driver and reclamation owners with allocation-scoped cgroups. |
| Wire / persistence / CLI configuration | Unchanged; the cause travels only in the existing in-process typed shutdown request. |
| Tests, lanes and models | No new item required; only DISTILL alignment of the existing exhaustive cause table and caller fallout. |
| Scope | Subtractive consistency correction required by the accepted attachment-cut ADR; no new mechanism or authority. |

The normative diff removes bridge-era kill requirements but does not relax an
accepted post-cut `zero`, `never`, `must`, `exactly`, fail-closed, cleanup, or
security invariant. “Recovery kills no VM” is itself the accepted post-cut
contract; retaining the old kill capability would violate ownership and add
unreachable machinery.

## Roadmap review

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The retained supervisor remains composed through `run_server`; supplied-owner entry points retain the same driver, owner, EXEC wiring, and boot path without the unused cgroup capability. |
| Acceptance-criterion coupling | Pass | Exact public causes and supplied-owner signatures are DESIGN-owned contracts; private supervisor implementation remains with the crafter. |
| Step decomposition | Pass | The cause/capability deletions occur with the attachment cut that removes their TAP-based semantic inputs. |
| Implementation code | Pass | The delta pins variants, ownership and signatures without prescribing private helper bodies. |
| Concision and precision | Pass | The exact eight variants, deleted VM-kill surface, retained allocation-scoped surface, and compiler fallout are unambiguous. |
| Test boundary | Pass | Existing exhaustive cause coverage is aligned by DISTILL; no new test, seam, lane, or model is required. |
| Mechanical validity | Pass | `roadmap.json` parses, the reviewed diff passes `git diff --check`, and validation remains pending until this approval is folded. |

## Supplement verdict

**APPROVED.** The exact eight-variant
`SharedGuestNetworkFailStopCause` set, deletion of
`TapQuiescenceUndetermined` and `VmKillFailed`, deletion of the supervisor-only
`VmKillCapability` / whole-workloads-slice path and kill-write deadline, and
removal of the unused supplied-owner `vm_cgroups` parameter are consistent
with ADR-0124's accepted 2026-10-11 amendment and the real producer/caller
graph. The bounded compiler fallout is identified, while allocation-scoped VM
stop and boot-reclamation ownership remain intact.

The roadmap is eligible to return from `pending` to `approved` with this
supplemental independent DESIGN approval recorded alongside the already
approved GuestNetworkError reconciliation. The feature delta's pending
exact-set wording may be folded into the accepted contract. No additional
user decision is required.

---

# Supplement — `VipRange::first_overlapping_range`

## Supplement metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-VIP-RANGE-OVERLAP-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The bounded public `VipRange` overlap query, phase-1 caller and typed refusal mapping, exact all-configured-ranges invariant, deterministic offending-CIDR representation, and STEP 12-01 roadmap impact |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This supplement changes only this review artifact. It changes no reviewed
feature delta, roadmap field, production source, test body, DES event,
execution log, model, lane, or kernel state. It performs no Metal run,
mutation run, spike, commit, or implementation test.

## Necessity and accepted outcome

ADR-0185 requires startup to refuse before any boot effect when **any** CIDR
of the configured service VIP range overlaps the guest prefix, and requires
the typed refusal to name the overlapping range
(`adr-0185-the-guest-prefix-holds-no-node-address-and-no-vip.md:35-55`). The
feature delta restates the same approved D8-PREFIX-VIP invariant and R5-40
oracle. The existing `VipRange` surface cannot implement both parts honestly:

- `first_range()` exposes only the first operator-supplied CIDR, so it can
  miss an overlap in every later CIDR.
- `contains(Ipv4Addr)` deliberately returns false for reserved VIPs and
  cannot return the CIDR needed by `GuestPrefixOverlapsVipRange`.
- `allocatable_addrs()` expands allocatable host addresses, skips reserved
  addresses, and discards each address's prefix length. It can take work
  proportional to the configured address space, including a `/0`.
- The backing `ranges: Vec<Ipv4Net>` is private, correctly preventing the
  control plane from inspecting or copying allocator internals.

A cross-crate query owned by `VipRange` is therefore necessary. The proposed
surface is the smallest one that preserves both the refusal predicate and the
exact typed cause:

```rust
pub fn first_overlapping_range(&self, other: Ipv4Net) -> Option<Ipv4Net>;
```

It adds no iterator, slice, cloned collection, reserved-set accessor, mutable
capability, runtime probe, or guest-network dependency to the dataplane crate.
The method is a pure read over an existing `Vec<Ipv4Net>` and can scan CIDRs
directly in O(configured ranges) time with no allocation or host-address
expansion. `Ipv4Net` is already the type stored and returned by the existing
`first_range()` API, so the signature adds no dependency or representation.

## Exact contract and deterministic representation

The method contract is complete:

| Input/state | Result |
|---|---|
| The first operator-ordered configured CIDR intersects `other` | `Some(that_cidr)` |
| One or more later CIDRs intersect `other` while earlier CIDRs are disjoint | `Some(first_later_overlap)` |
| Multiple configured CIDRs intersect `other` | `Some(first_overlap_in_operator_order)` |
| Every configured CIDR is disjoint | `None` |
| The intersection contains only addresses marked reserved | Still `Some(cidr)`; reservation does not change configured-range overlap |

`VipRange` already preserves operator order structurally in its private
`Vec<Ipv4Net>` (`vip_range.rs:38-47`), and its constructor rejects overlap
between configured ranges while retaining that order (`:66-100`). Selecting
the first overlapping CIDR is therefore bounded and deterministic.

This ordering does not add a new startup outcome. The approved outcome is
already binary: refuse iff any configured CIDR overlaps, otherwise continue.
When several disjoint configured CIDRs lie within or cross the guest prefix,
the existing typed error has room for one `Ipv4Net`. Operator order chooses a
stable representative for that already-required payload; it does not permit a
previously forbidden configuration, reject a previously valid one, or add a
second error or retry path.

## Producer and caller mapping

The phase-1 mapping is exact and remains at the current owner boundary:

1. `probe_guest_flow_startup` calls
   `vip_range.first_overlapping_range(node_prefix)` before the node-address,
   owner-placement, fd-budget and forwarder checks.
2. `Some(overlap)` returns exactly
   `GuestNetworkError::GuestPrefixOverlapsVipRange { prefix: node_prefix,
   vip_range: overlap }`.
3. `None` continues through the existing checks in their accepted order.
4. The owner entry point delegates to the same phase-1 function over its own
   seam and mandatory `VipRange`, so the supplied-owner and boot-seam paths
   cannot disagree.
5. `run_server` retains the existing `health.startup.refused` event with
   reason `guest_network.probe`; no boot effect has run.

The `GuestPrefixOverlapsVipRange` variant and both `Ipv4Net` payloads were
already accepted and are unchanged. No generic error, string-carried CIDR,
new operation discriminator, alternate outer error, or fallback mapping is
introduced.

## Reserved-address and all-ranges invariant

Ignoring `reserved` in this query is required. ADR-0185 rejects overlap of the
**configured range**, because boot route/address construction and mesh VIP
classification are defined over that configured range. Allowing an overlap
because the intersecting addresses happen to be unavailable to the allocator
would reintroduce the rejected “exclude overlapping addresses from leasing”
alternative and would weaken D8-PREFIX-VIP.

The existing P4-S14 property already drives both phase-1 entry points and
asserts refusal iff some configured CIDR overlaps. Its bounded DISTILL
alignment must additionally generate reserved addresses and assert the exact
first operator-ordered overlapping payload. This strengthens the existing
property's oracle; it adds no scenario, lane, fixture, seam, or model. The
STEP 13 marker remains because STEP 13 owns activation of the guest-prefix
boot-check cells. STEP 12-01 only lands the required public primitive and its
production caller; it does not edit the DISTILL-owned test body or activate
the later step's marker.

## Alternatives and scope

| Alternative | Disposition |
|---|---|
| Use `first_range()` | Invalid: misses later configured CIDRs. |
| Use `contains()` over selected addresses | Invalid: reservation-sensitive, address-shaped, and cannot recover the typed CIDR. |
| Use `allocatable_addrs()` | Invalid: reservation-sensitive, potentially address-space-sized, and loses prefix length. |
| Expose `ranges()` / a slice or iterator | Excess surface: exposes the whole private collection when the caller needs one closed query. |
| Clone/copy configured ranges into the control plane | Duplicates ownership and widens the constructor/state contract. |
| Add a guest-specific overlap method or guest-prefix constant to dataplane | Couples the allocator to a consumer-specific policy unnecessarily. |
| Add a runtime route/address probe | Changes the approved configuration check into a later effect/observation mechanism. |

The proposed query is a narrow semantic projection from the owner of the
private collection. It is general only over the comparison `Ipv4Net` needed
to keep the dataplane type independent of the guest-network constant; it does
not create a general collection-inspection API.

## Invariant and roadmap audit

| Concern | Assessment |
|---|---|
| Every configured CIDR checked | Preserved by the full ordered scan; no first-range shortcut. |
| Exact typed payload | Preserved as the actual first overlapping configured CIDR. |
| Check ordering | Preserved: VIP overlap remains before node address, placement, fd budget and forwarder probe. |
| Refusal timing | Preserved before every boot effect. |
| Reserved addresses | Correctly irrelevant to configured-range intersection. |
| Allocation/performance | Bounded by configured CIDR count; no address expansion or allocation. |
| Public surface | One exact read-only method; no iterator, collection, reserved-set or mutation surface. |
| Wire, persistence and configuration | Unchanged. |
| Test architecture | Existing P4-S14 property strengthened by DISTILL; STEP 13 marker retained; no new scenario or lane. |
| Scope | Necessary interface completion of the already-approved D8-PREFIX-VIP outcome. |

The roadmap passes external-validity, decomposition, implementation-code,
concision and test-boundary review for this correction. It names the exact
public signature and result semantics because those are DESIGN-owned API and
typed-error contracts, while leaving the private overlap implementation to the
crafter. `roadmap.json` parses and the reviewed feature-delta/roadmap diff
passes `git diff --check`.

## Supplement verdict

**APPROVED.** `VipRange::first_overlapping_range(&self, other: Ipv4Net) ->
Option<Ipv4Net>` is necessary and is the minimum public surface that can
preserve the approved all-configured-ranges D8-PREFIX-VIP refusal and its exact
offending-CIDR payload. First operator order is a bounded deterministic
representation of an existing typed refusal, not a new user-visible outcome.
The reserved set correctly has no effect on configured-range overlap, and the
phase-1 mapping, ordering and pre-effect refusal remain exact.

The roadmap is eligible to return from `pending` to `approved` with this
supplemental independent DESIGN approval recorded alongside the earlier STEP
12-01 reconciliations. The feature delta's new interface wording may be folded
into the accepted D8-PREFIX-VIP contract. No additional user decision is
required.
