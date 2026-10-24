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

---

# Supplement — host-local route convergence and IPv4 FIB route type

## Supplement metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-NETLINK-ROUTE-INTERFACES-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The bounded `overdrive-netlink` host-local-route convergence and IPv4 FIB route-type interfaces, exact public types/errors, `GuestFlowIo` mappings, route ownership and failure preservation, and existing GW-LOCAL / D26 outcomes |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 0 critical, 2 high |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This supplement changes only this review artifact. It changes no reviewed
feature delta, roadmap field, production source, test body, DES event,
execution log, model, lane, or kernel state. It performs no Metal run,
mutation run, spike, commit, or implementation test.

## Necessity and API-shape assessment

Both primitives are necessary. The accepted GW-LOCAL contract requires boot
to keep, replace, or add exactly one Overdrive-tagged
`local <gateway>/32 dev lo table local` route and to leave the prior route
intact on a failed mutation. The existing `Client::add_local_route(table,
oif)` owns a different object: a table-parameterized local default route
(`0.0.0.0/0`) used by the mark-routing path. Reusing or generalizing it would
either write the wrong prefix or expose table/prefix/interface/type/scope
choices that GW-LOCAL does not own.

The proposed mutation surface is correspondingly narrow:

```rust
pub async fn converge_ipv4_host_local_route(
    &self,
    destination: Ipv4Addr,
    protocol: u8,
) -> Result<(), NetlinkError>;
```

It fixes IPv4, `/32`, `RT_TABLE_LOCAL`, `lo`, `RouteType::Local`,
`RouteScope::Host`, and no gateway. The sole production caller supplies the
guest gateway and `GUEST_PREFIX_ROUTE_PROTOCOL` 247. The destination and
protocol parameters are the two values the netlink crate cannot own without a
reverse dependency on guest-network constants; no table, prefix, interface,
kind, scope, gateway, priority, or replace-policy parameter is exposed.

The accepted D26 owner check also needs the kernel's route decision for one
destination. Current `HostGuestFlowIo::route_type` is an explicit DESIGN
blocker, and the private `HostRouteType` duplicates the semantic projection.
The proposed query and re-export remove that duplicate:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FibRouteType {
    Unicast,
    Local,
    Broadcast,
    Multicast,
    Anycast,
    Other,
}

pub async fn ipv4_fib_route_type(
    &self,
    destination: Ipv4Addr,
) -> Result<FibRouteType, NetlinkError>;
```

This is a destination-specific query, not a table dump or raw-route escape.
The five route types D26 names remain distinct; all other kernel types collapse
to `Other` because the owner branches only on `Unicast`, the gateway:53
exception, and every other type. Exposing the complete `RouteMessage` or a raw
route-type byte would add surface the owner cannot use.

## Kernel and library feasibility

The lookup contract matches Linux 7.0 and the checked-in rtnetlink version:

- A `RouteMessageBuilder::<Ipv4Addr>` request carrying a destination attribute
  causes rtnetlink 0.23's `RouteGetRequest` to send `RTM_GETROUTE` without
  `NLM_F_DUMP`; the dump flag is added only when no destination attribute is
  present (`rtnetlink-0.23.0/src/route/get.rs`).
- Linux validates `/32` destination queries, runs the output lookup when no
  input interface is supplied, and returns a unicast netlink reply
  (`route.c:3290-3402,3440-3455 @ v7.0`). `rt_fill_info` copies the selected
  route's `rt_type` to `rtm_type` (`route.c:2949-2973 @ v7.0`).
- A normal no-route result is a typed kernel/netlink error. A successful
  response stream containing no route message is therefore structurally
  distinct and needs the source-less
  `NetlinkError::FibLookupMissing { destination }` variant.
- rtnetlink 0.23's `RouteAddRequest::replace()` emits one `RTM_NEWROUTE` with
  `NLM_F_CREATE | NLM_F_REPLACE | NLM_F_ACK`. The kernel allocates the desired
  replacement before changing the FIB alias and rolls back the alias if the
  replacement notifier fails. A missing-route insertion likewise removes the
  newly inserted alias if its notifier fails.

The public error set is otherwise unchanged. Kernel/decoder failures stay in
`NetlinkError::Route` with exact operation labels
`"gateway-local-replace"` and `"fib-lookup"`; link resolution retains
`LinkAbsent { iface: "lo" }`; `FibLookupMissing` carries the queried
destination and maps to `errno() == None`. No string is used as a semantic
substitute.

## Ownership and failure mappings

| Condition | Exact representation | Projection |
|---|---|---|
| `lo` cannot be resolved | `GuestNetworkError::Netlink { operation: LocalRouteConverge, source: NetlinkError::LinkAbsent { iface: "lo" } }` | Boot refuses at `guest_network.converge`; no route request was issued |
| Atomic create-or-replace returns a route error | `GuestNetworkError::Netlink { operation: LocalRouteConverge, source: NetlinkError::Route { op: "gateway-local-replace", source } }` | Boot refuses; the kernel mutation leaves the prior route state |
| FIB lookup returns a kernel/decoder error | `NetlinkError::Route { op: "fib-lookup", source }` | Owner returns flow-local `Refused(HostInternal)` before `open_peer` |
| Successful FIB exchange returns no route message | `NetlinkError::FibLookupMissing { destination }` | Same fail-closed refusal |
| Lookup returns `Unicast` | `FibRouteType::Unicast` | Direct peer-open path may continue |
| Lookup returns local/broadcast/multicast/anycast/other | Corresponding semantic variant or `Other` | `HostInternal`, except the already-approved gateway:53 exception |

The route mutator remains boot-only. It is absent from runtime audit and
repair, never removes the route, and has one production caller. The FIB query
is read-only and occurs before `open_peer`, so no outbound peer socket,
destination connect, or forwarder attach can occur on lookup failure. The
existing firewall rule remains the atomic enforcement against the check/connect
race; this read does not replace it.

The existing R5-11 and D26 evidence can be aligned to these adapter primitives,
including the `FibLookupMissing` mapping, without a new scenario, lane, model,
runtime hook, command invocation, or generalized netlink port.

## Findings

### N-01 — The lookup-error projection contradicts the real owner path

- **Severity:** High; blocks approval under the exact outcome and production-
  path rules.
- **Claim:** The new taxonomy row and exact-netlink prose say a lookup error
  produces `Refused(HostInternal)` before `open_peer` and that “no socket or
  other flow-side effect” has occurred. The first half is correct; the second
  is false for the real production path.
- **Evidence:** `tcp_connect_pair` and `datagram_associate` receive an already
  accepted guest `RecordSocket` before entering their policy path
  (`owner_runtime.rs:700-708,846-858`). They perform `route_type` before
  `open_peer` (`:790-799,868-882`), but on any resulting refusal their common
  cleanup closes the existing guest-side socket (`:836-843`; the datagram
  path has the same ownership pattern), and the owner emits the typed
  `Refused(HostInternal)` control outcome. Thus no **outbound peer socket or
  destination-side effect** occurs, while flow-local refusal and guest-socket
  cleanup necessarily do occur.
- **Consequence:** A crafter cannot simultaneously implement the specified
  typed refusal and “no socket or other flow-side effect.” Interpreting the
  latter literally would suppress the required refusal/cleanup or leak the
  accepted guest socket.
- **Required revision:** In the taxonomy row and exact-netlink prose, replace
  “no socket or other flow-side effect” with the precise boundary: no outbound
  peer socket is opened, no destination connect and no forwarder attach occur;
  the already-accepted guest flow is refused `HostInternal` and its guest-side
  socket/slot are closed and released by the ordinary refusal path. Do not
  change the error type, owner ordering, refusal, test count, or mechanism.
- **Disposition:** Open.

### N-02 — The atomic failure-preservation claim cites lines before replacement and rollback

- **Severity:** High under the repository's mandatory Linux-source citation
  gate; the underlying mechanism is valid.
- **Claim:** The feature delta says Linux's atomic insertion is why an error
  preserves the prior route, but cites only
  `net/ipv4/fib_trie.c:1194-1290 @ v7.0`. That range ends before the actual
  alias replacement and does not include either rollback path.
- **Evidence:** Replacement occurs at `fib_trie.c:1292`; a notifier failure
  restores the prior alias at `:1299-1305`, and success releases the old alias
  at `:1309-1318`. For a missing route, insertion occurs at `:1332-1355`, a
  notifier failure removes the new alias at `:1365-1388`, and other failures
  return through `:1389-1392`. These are the lines that establish the claimed
  old-or-new result.
- **Consequence:** The conclusion is correct, but the cited evidence does not
  substantiate it. Repository DESIGN rules require a kernel claim to cite the
  exact Linux 7.0 path that proves it.
- **Required revision:** Replace the incomplete range with citations covering
  replacement allocation/swap/rollback/success and missing-route insertion/
  rollback, for example `fib_trie.c:1212-1216,1263-1318,1332-1392 @ v7.0`.
  Preserve the one-request mechanism and all API/error mappings.
- **Disposition:** Open.

## Roadmap assessment

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | GW-LOCAL is called by the production boot owner; D26 lookup is in both direct TCP and non-VIP datagram owner paths. |
| Acceptance-criterion coupling | Pass | Fixed route shape, semantic enum and typed errors are DESIGN-owned interface contracts. |
| Step decomposition | Pass | Both methods close explicit RED blockers in the STEP 12-01 host seam and reuse existing R5-11/D26 evidence. |
| Implementation code | Pass | The design pins messages and semantics while leaving builder/helper structure to the crafter. |
| Concision and precision | **Fail pending N-01** | The intended no-peer-open boundary is contradicted by the broader no-socket/no-flow-effect statement. |
| Test boundary | Pass | Existing R5-11 and D26 cells can align to the adapters; no new scenario, lane or model is needed. |
| Mechanical validity | Pass | `roadmap.json` parses and the reviewed feature-delta/roadmap diff passes `git diff --check`. |
| Kernel evidence | **Fail pending N-02** | The source supports atomic old-or-new mutation, but the artifact's cited range omits the swap and rollback lines. |

## Supplement verdict

**CHANGES_REQUESTED.** The two exact `Client` methods, `FibRouteType`,
`FibLookupMissing`, direct `NetlinkError` seam, `LocalRouteConverge` wrapper,
and fail-closed `HostInternal` mapping are necessary, feasible, narrowly owned,
and preserve the accepted GW-LOCAL and D26 outcomes. The fixed route shape and
destination-specific semantic query add no generalized routing surface or new
scenario.

Approval is blocked by two bounded documentation defects: N-01 overstates the
lookup-error universe despite the required flow-local refusal and guest-socket
cleanup, and N-02 cites a kernel range that ends before the replacement and
rollback that prove failure preservation. Until both are corrected and
re-reviewed, roadmap validation must remain `pending` and this netlink
supplement must not be folded into the accepted contract. Neither remediation
requires a new user decision, API, error variant, mechanism, test, lane, model,
or production change.

## Netlink supplement review iteration 2

### Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-NETLINK-ROUTE-INTERFACES-ARCH-REVIEW-02` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | N-01 and N-02 remediation only; iteration 1's necessity, API-shape, typed-error, ownership, feasibility and roadmap assessments remain incorporated |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This iteration changes only this review artifact. It changes no reviewed
feature delta, roadmap field, production source, test body, DES event,
execution log, model, lane, or kernel state. No new Metal run, mutation run,
spike, or implementation test was needed; both findings concerned exact
contract prose and primary-source citation over mechanisms already verified in
iteration 1.

### N-01 remediation verification

N-01 is closed. The taxonomy row now states the complete real flow-local
outcome (`feature-delta.md:8713`):

- a lookup failure maps to `Refused(HostInternal)`;
- no outbound peer socket is opened;
- no destination connect or forwarder attach occurs; and
- the ordinary refusal path sends the typed refusal and closes/releases the
  already-accepted guest-side socket or datagram slot.

The exact-netlink section carries the same boundary
(`feature-delta.md:11006-11014`). It no longer claims that the flow had no
socket or effect. The wording now matches `tcp_connect_pair` and
`datagram_associate`: `route_type` precedes `open_peer`, while their common
error cleanup still closes the accepted guest socket and their owner path
emits the required control refusal. The correction changes no ordering,
error, refusal, cleanup, quota/slot release, test, or implementation
mechanism.

### N-02 remediation verification

N-02 is closed. The atomic failure-preservation claim now cites every Linux
7.0 transition on which it relies (`feature-delta.md:10988-10996`):

- desired `fib_info` construction at `fib_trie.c:1212-1216`;
- replacement allocation, alias swap, notifier rollback and old-alias release
  only after success at `:1263-1318`; and
- missing-route insertion, notifier rollback, cleanup and return at
  `:1332-1392`.

Those citations match the vendored v7.0 source. A replacement error before the
swap leaves the prior alias untouched; a notifier error after the tentative
swap restores it; a missing-route notifier error removes the new alias. The
one `RTM_NEWROUTE` create-or-replace request therefore provides the claimed
old-or-new mutation boundary without delete/add exposure.

### Finding disposition

| Finding | Iteration 1 | Iteration 2 evidence | Disposition |
|---|---|---|---|
| N-01 — lookup failure claims no socket/flow effect despite ordinary refusal cleanup | High, blocking | Prose now distinguishes the absent outbound peer/connect/attach from the existing accepted guest socket, typed refusal and cleanup | **CLOSED** |
| N-02 — failure-preservation citation ends before swap and rollback | High, blocking | Citation now covers desired-info creation, replacement swap/rollback/success and missing-route insertion/rollback | **CLOSED** |

No new critical, high, or medium finding is present. The exact public surface
remains unchanged from iteration 1:

- `Client::converge_ipv4_host_local_route(destination, protocol)` owns one
  fixed `/32` host-local route shape and one atomic create-or-replace;
- `Client::ipv4_fib_route_type(destination)` performs one destination-specific
  FIB lookup;
- `FibRouteType::{Unicast, Local, Broadcast, Multicast, Anycast, Other}` is
  the closed semantic projection;
- `NetlinkError::FibLookupMissing { destination }` distinguishes an empty
  successful response, and `errno()` returns `None` for it;
- `GuestFlowIo::route_type` retains the typed `NetlinkError` directly; and
- gateway convergence maps every netlink failure through
  `GuestNetworkError::Netlink { operation: LocalRouteConverge, source }`.

### Iteration 2 verdict

**APPROVED.** The bounded netlink interfaces, semantic route-type vocabulary,
typed failure mappings, atomic route replacement, boot-only route ownership,
and fail-closed pre-peer-open owner behavior are exact and supported by the
actual production caller path, rtnetlink request behavior, and vendored Linux
7.0 source. They preserve GW-LOCAL and D26 without adding a generalized route
API, new outcome, runtime repair, scenario, lane, model, or invariant
weakening.

The roadmap is eligible to return from `pending` to `approved` with this
iteration-2 independent DESIGN approval recorded alongside the earlier STEP
12-01 supplements. The netlink interface wording may be folded into the
accepted GW-LOCAL / D26 contract. No additional user decision is required.

---

# Supplement — QHOLD leaked-count audit interface

## Supplement metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-QHOLD-LEAKED-COUNT-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The exact `SharedGuestNetworkOwner::leaked_quiescence_holds` interface, owner/ledger identity, implementation fallout, audit-tick ordering, `LeakedQuiescenceHold` mapping, and existing R5-32 evidence |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This supplement changes only this review artifact. It changes no reviewed
feature delta, roadmap field, production source, test body, DES event,
execution log, model, lane, or kernel state. It performs no Metal run,
mutation run, spike, commit, or implementation test.

## Necessity and accepted contract

ADR-0169 already requires the supervisor to treat a leaked-hold count above
zero as an exhausted recovery and issue ADR-0124's typed fail-stop at the next
audit tick. The hold's drop increments the ledger that issued it, leaves the
owner's hold count raised and performs no I/O. Forwarding therefore remains
quiesced until process exit. This supplement supplies the missing interface by
which a supervisor holding only `Arc<dyn SharedGuestNetworkOwner>` can read
that exact owner's ledger.

The existing surfaces do not provide an honest substitute:

- `HostSharedGuestNetworkOwner::flow_observation()` includes the value, but is
  an inherent, async, full diagnostic/conformance snapshot unavailable on the
  owner trait object.
- Downcasting the owner would exclude supplied-owner and simulated
  compositions and couple the supervisor to concrete types.
- A default trait body returning zero would silently suppress fail-stop for an
  external owner that omitted the implementation.
- A second supervisor ledger would observe different holds and break the
  issuing-owner identity that makes QHOLD fail closed.
- Folding a leak into `audit_shared` would conflate an exhausted recovery
  defect with component damage and could start repair before the required
  immediate fail-stop.

The proposed method is therefore the minimum interface that realizes the
accepted outcome:

```rust
#[must_use]
fn leaked_quiescence_holds(&self) -> u32;
```

It is synchronous and infallible because it is one atomic read of the owner's
existing `QuiescenceLedger::leaked()` counter. It performs no I/O, lock-taking
protocol, state transition, audit, repair, restore, or allocation. The value
starts at zero for each owner instance, counts holds of that owner's ledger
dropped unredeemed, and never decreases during that owner's process lifetime.
No new failure requires an error mapping.

## Owner identity and implementation fallout

The current repository has ten `SharedGuestNetworkOwner` implementations. The
fallout is complete and ownership-preserving:

| Implementation class | Implementations | Required mapping |
|---|---|---|
| Ledger owners | `HostSharedGuestNetworkOwner`, `SimSharedGuestNetworkOwner`, `TestSharedOwner` | Return `self`'s existing ledger `leaked()` value |
| Delegating owner adapters | `ActivationFaultOwner`; the mTLS journal owner; `RecordingGuestNetwork`; the activation-order owner; the retiring-cleanup journal owner; `LeaseLedger`; `RecordingOwner` | Delegate to the same inner `SharedGuestNetworkOwner` |

The seven adapters already delegate their startup, convergence, audit,
quiescence and restore behavior to one inner owner. Delegating the leaked
count preserves the same identity across every supplied-owner and simulation
path. None may create a ledger, cache a count, return a fixture constant, or
derive the value from its own journal.

The three direct owners already hold the required state:

- The production owner's `flow_observation()` reads `self.ledger.leaked()`.
- `SimSharedGuestNetworkOwner` owns `SimQuiescence { ledger, holds }` and its
  existing inherent leaked-count accessor reads that ledger.
- `TestSharedOwner` owns one `QuiescenceLedger`, uses it for `quiesce` and
  `restore`, and already exposes its leaked count to the R5-32 fixture.

No constructor, hold type, ledger type, persistence shape, wire format,
component vocabulary, error variant, or public diagnostic snapshot changes.
The absence of a default trait body makes compiler fallout the enforcement
that all ten owners preserve the contract.

## Audit ordering and exact mapping

The runtime ordering is exact:

1. The one-second audit tick becomes ready unless intentional shutdown wins.
2. Before `audit_shared`, component detection, `converge_shared`, repair, or
   restore, the supervisor calls
   `shared_guest_network.leaked_quiescence_holds()` through the trait object.
3. Zero continues into the existing audit/recovery work.
4. A value above zero calls the existing EXEC supervisor fail-stop with
   `SharedGuestNetworkFailStopCause::LeakedQuiescenceHold`, sends the existing
   `ServeShutdownRequest::SharedGuestNetwork`, and performs no audit, repair,
   attempt increment, or restore on that tick.
5. Forwarding stays quiesced until the process ends; the supervised restart
   constructs a fresh owner whose counter begins at zero and converges on boot.

The mapping reuses the already-approved cause and shutdown path. It does not
create a second deadline, retry rule, component, recovery episode, or
supervisor task. A leak is terminal at observation; it is not counted as a
failed repair attempt and does not wait for the five-second recovery budget.

Intentional shutdown remains distinct. Cancelling the supervisor drops any
in-progress hold, but the cancelled task runs no later audit tick, so that
drop produces no `LeakedQuiescenceHold` request. Forwarding remains quiesced
until the already-requested process exit. The new read does not change this
accepted safe-direction behavior.

## Outcome and evidence assessment

The existing R5-32 leaked-hold cell already supplies the required observable
journey: a supplied/composed owner issues and leaks a hold, its leaked count
rises while forwarding remains quiesced, a completed recovery cannot reopen
the leaked hold, and the next audit tick produces exactly
`LeakedQuiescenceHold`; the restart begins with zero and converges. Aligning
that cell to drive the new trait method verifies the external-owner path
without adding a scenario, seam, model variable, lane, downcast, or full owner
snapshot.

The existing R5-58 invariant remains complementary: ordinary recoveries keep
their holds, so `leaked_quiescence_holds` stays zero and each episode ends only
by restore or its existing budget fail-stop. The new check does not alter
attempt counts or repair cadence in the no-leak universe.

## Invariant and scope audit

| Concern | Assessment |
|---|---|
| Exact owner | Preserved: direct owners read their own ledger; wrappers delegate to the same inner owner. |
| Fail-closed hold semantics | Preserved: leaking never decrements the hold count or reopens admission. |
| Detection bound | Preserved: the next one-second audit tick checks before any other audit/repair work. |
| Typed failure | Preserved through the existing `LeakedQuiescenceHold` cause and shutdown request. |
| Recovery budgets | Unchanged; a detected leak bypasses repair and the five-second budget as already accepted. |
| Intentional shutdown | Unchanged; cancellation-and-join runs no subsequent audit tick and emits no leak cause. |
| Interface size | One `u32` read; no full observation, downcast, error, default, ledger or mutation surface. |
| Persistence / wire / configuration | Unchanged. |
| Tests and models | Existing R5-32 alignment only; no new scenario, lane, seam or model state. |
| Compiler fallout | Exactly the ten current trait implementations; three direct ledger reads and seven delegations. |

## Roadmap assessment

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The runtime supervisor holds the production/supplied owner as `Arc<dyn SharedGuestNetworkOwner>` and reads the method at its actual audit boundary. |
| Acceptance-criterion coupling | Pass | Exact trait signature, owner identity and fail-stop ordering are DESIGN-owned interface/lifecycle contracts. |
| Step decomposition | Pass | The method lands with the STEP 12-01 supervisor rework that must implement the accepted QHOLD audit. |
| Implementation code | Pass | The design pins observable semantics and delegation; private field/access mechanics remain with the crafter. |
| Concision and precision | Pass | Return type, monotonic meaning, no-default rule, owner identity, ordering and mapping each have one interpretation. |
| Test boundary | Pass | The existing R5-32 external-owner cell covers the behavior; no new scenario or lane is introduced. |
| Mechanical validity | Pass | `roadmap.json` parses and the reviewed feature-delta/roadmap diff passes `git diff --check`. |

## Supplement verdict

**APPROVED.** The required
`#[must_use] fn leaked_quiescence_holds(&self) -> u32` method is the minimum
owner-port surface that lets the runtime supervisor implement the already-
accepted QHOLD fail-stop through concrete, simulated and supplied owners. Its
direct-ledger/delegating implementation boundary preserves one ledger per
owner; its audit-first ordering maps nonzero directly to the existing
`LeakedQuiescenceHold` shutdown request and performs no repair work on that
tick. No outcome, mechanism, error, persistence, wire format, test scenario,
lane, or model is added or weakened.

The roadmap is eligible to return from `pending` to `approved` with this
supplemental independent DESIGN approval recorded alongside the earlier STEP
12-01 supplements. The QHOLD audit-interface wording may be folded into the
accepted ADR-0169 / ADR-0124 contract. No additional user decision is
required.

---

# Supplement — per-CID control-session loss and `ControlListener` recovery

## Supplement metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-CONTROL-SESSION-LOSS-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The roadmap-only correction separating per-CID control-session loss from shared `ControlListener` accept-task audit and repair, including exact loss detection, flow/dial outcomes, reconnect ordering, and the closed component taxonomy |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This supplement changes only this review artifact. It changes no reviewed
roadmap field, feature delta, ADR, production source, test body, DES event,
execution log, model, lane, or kernel state. It performs no Metal run,
mutation run, spike, commit, or implementation test.

## Canonical ownership distinction

The corrected criterion restores the ownership already accepted by ADR-0166,
ADR-0213, and the feature delta:

- `ControlListener` is the one shared host vsock listener and its accept task.
  The closed `SharedGuestNetworkComponent` vocabulary therefore contains one
  `ControlListener` entry, alongside `FlowListeners`, `Forwarder`, `LegF`,
  `LegC`, `IpRules`, `IpSets`, `Dns`, and `Supervisor`; it contains no
  per-CID session component (`overdrive-core/src/guest_network.rs:75-101`).
- A live control session is allocation state owned by one CID. ADR-0166 pins
  at most one live session per CID, an attributed atomic claim, direct loss
  handling, and the rule that loss aborts every flow of that VM
  (`adr-0166:26-55`). ADR-0213 makes flow and dial admission/registration one
  per-CID step relative to that loss (`adr-0213:16-40`).
- The feature delta retains the corresponding observable runs:
  `SessionLossAbortsFlowsAndReconnectRestores`,
  `GuestReopensSessionOnlyAfterObservingItsLoss`,
  `SessionLossAbortsAllFlowsBeforeReconnect`,
  `DialAfterObservedSessionLossWithVmGoneReturnsNoSession`, and
  `DialAfterReconnectPairs` (`feature-delta.md:13700-13720`). It explicitly
  assigns standalone loss without a VM fault to seeded simulation and the
  in-guest session cells because no `serve` + deploy stimulus exists
  (`:12569-12573,12734,14590-14592`).

The removed roadmap wording treated absence of one per-CID connection as
damage to a shared component and required the supervisor to repair it. That
would conflate two different owners and invent a tenth component/failure
mapping. The correction removes that contradiction; it does not remove loss
detection or recovery behavior.

## Actual production paths

### Shared listener audit and repair

`HostSharedGuestNetworkOwner::audit_shared` compares the forwarder inventory
with the owner's pair/watch tables and checks the two held accept tasks. A
finished stream accept task maps to `FlowListeners`; a finished control accept
task maps to `ControlListener` (`guest_network.rs:1154-1221`). No branch audits
whether each CID currently has a session.

`ControlListener` repair calls `bind_listener(Control, control_port)`, then
`restart_listener(Control)` (`guest_network.rs:1087-1096`). The port contract
states that, when the accept task ended while the held socket still listens,
`bind_listener` verifies that socket and `accept(listener)` serves it again;
nothing is closed or rebound (`guest_network/flow_io.rs:275-300`). The revised
criterion's “verify the held listener and re-spawn its accept task” is
therefore the exact existing shared-component recovery boundary.

### Per-CID loss detection and cleanup

Each accepted control connection runs `control_session`. Its read loop treats
EOF, read failure, write failure, sender closure, or cancellation as the end
of that connection. If its key still owns the CID's live claim, it calls
`session_lost` with `ControlSessionLost` before finishing and closing the
connection (`owner_runtime.rs:199-331`). Detection is thus immediate in the
connection owner; it does not wait for the one-second shared audit tick.

The loss transition locks the CID state, verifies that the ending connection
still owns the claim, clears `session`, `session_sender`, and
`session_cancel`, snapshots every registered pair/task/dial, records
`GuestDialError::NoSession` for every awaiting reply, cancels every dial and
flow task, and aborts every registered pair and paired dial watch
(`owner_runtime.rs:1072-1124`). The ordinary end-record path preserves the
same typed mapping when a dial is still awaiting its result:
`ControlSessionLost` becomes `GuestDialError::NoSession`
(`owner_runtime.rs:910-963`). A new dial with no live session is also rejected
as `NoSession` before opening its guest vsock (`guest_network/dial.rs:68-90`).

The claim and all CID admission checks use the same owner-state lock. A new
session may claim only the live launch token and only after the old claim has
been cleared; a second live claim is refused (`owner_runtime.rs:228-258`). A
guest-opened flow checks the same session state before registration and is
closed unattributed when no session exists (`:489-529`). Together with the
guest contract that holds the old session until it observes loss and reopens
only afterwards, this preserves the accepted sequence: the old session's
registered work is ended, the attributed replacement claims the CID, and a
subsequent dial can pair. The correction neither creates a supervisor repair
episode nor permits an unattributed or concurrent replacement session.

## Exact outcome mapping

| Condition | Owner and action | Typed / observable outcome |
|---|---|---|
| Shared stream accept task ends | Shared audit returns `FlowListeners`; supervisor verifies the held stream listener and re-spawns its accept task | Existing `SharedGuestNetworkComponent::FlowListeners` recovery |
| Shared control accept task ends | Shared audit returns `ControlListener`; supervisor verifies the held control listener and re-spawns its accept task | Existing `SharedGuestNetworkComponent::ControlListener` recovery |
| Forwarder inventory read or owner/inventory comparison fails | Shared audit returns `Forwarder` with the existing typed source | Existing `SharedGuestNetworkComponent::Forwarder` recovery |
| One CID's live control connection ends | That connection's owner clears the CID claim and aborts its registered flows and dials | No shared component failure and no audit/recovery episode |
| A pending dial exists when that session is lost | Per-CID loss transition resolves it | `GuestDialError::NoSession`; its guest-side vsock/watch is closed or aborted |
| A dial starts while the CID has no live session | Per-CID dial admission refuses before opening a vsock | `GuestDialError::NoSession` with no admitted dial state |
| Guest observes loss and opens a correctly attributed replacement | Per-CID atomic claim accepts the matching live launch token after the old claim is free | One live session; a later dial pairs normally |

No semantic failure is flattened into a diagnostic string, and no new public
variant, payload, trait method, repair operation, audit fact, persistence
field, or wire record is required.

## Invariant and scope audit

| Concern | Assessment |
|---|---|
| Loss detection | Preserved: the connection task observes its own end and invokes the per-CID transition directly. |
| Abort completeness | Preserved: the transition snapshots and ends all registered pairs, flow tasks, pending dials, and paired dial watches for that CID. |
| Typed dial result | Preserved: observed session loss maps pending and newly attempted dials to `NoSession`. |
| Attribution | Preserved: replacement requires the CID's live launch token and a free claim. |
| Reconnect | Preserved: accepted model and guest-owner runs require observation of loss before reopen, then prove a dial after reconnect pairs. |
| Shared repair taxonomy | Preserved at exactly nine variants; no per-CID connection is promoted to `SharedGuestNetworkComponent`. |
| Listener ownership | Preserved: `ControlListener` means the held listener plus shared accept task, whose repair verifies and re-spawns. |
| Failure bound | Preserved and correctly attributed: per-CID loss is handled on connection end rather than delayed until an audit tick. |
| Tests and model | Existing named Quint/seeded-sim and in-guest cells remain the evidence; no new scenario, seam, lane, or model state is introduced. |

## Roadmap assessment

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The actual accept-task, control-connection, per-CID state, dial, and repair paths have the ownership and ordering named by the corrected criterion. |
| Acceptance-criterion coupling | Pass | The criterion now maps the shared task failures and per-CID connection loss to their accepted owners and outcomes. |
| Step decomposition | Pass | This is a single stale criterion correction inside STEP 12-01's owner/audit rework; it adds no implementation dependency. |
| Implementation code | Pass | It pins lifecycle meanings and outcomes while leaving private task and state-threading mechanics to the crafter. |
| Concision and precision | Pass | `FlowListeners`, `ControlListener`, `Forwarder`, per-CID session loss, `NoSession`, and reconnect each have one exact meaning. |
| Test boundary | Pass | Existing model and in-guest cells cover disconnect/reconnect; the correction does not manufacture a production stimulus that the accepted evidence says does not exist. |
| Mechanical validity | Pass | The reviewed documentation diff is confined to the roadmap criterion and validation block; `roadmap.json` parses and `git diff --check` passes. |

## Supplement verdict

**APPROVED.** The corrected roadmap criterion matches the accepted component
taxonomy and the actual production ownership split. Shared audit and repair
continue to cover the `FlowListeners` and `ControlListener` accept tasks plus
`Forwarder` damage. Each per-CID control connection detects its own end and
drives the existing serialized CID transition, preserving complete flow/dial
abort, the exact `NoSession` mapping, launch-token attribution, loss-before-
reopen behavior, and successful dialing after reconnect. No accepted outcome
or invariant is weakened, and no new API, error, supervisor mechanism,
scenario, lane, model, or spike is warranted.

The roadmap is eligible to return from `pending` to `approved` with this
supplemental independent DESIGN approval recorded alongside the earlier STEP
12-01 supplements. The corrected criterion may be folded into the accepted
ADR-0166 / ADR-0213 / shared-network recovery contract. No additional user
decision is required.

---

# Supplement — typed beacon-bind boot refusal

## Supplement metadata

| Field | Value |
|---|---|
| Review ID | `STEP12-01-BEACON-BIND-TYPED-REFUSAL-ARCH-REVIEW-01` |
| Date | 2026-10-24 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The exact `VmmBootError::BeaconBind` public variant, nested typed-cause chain, Optional/Required disposition, `run_server` refusal event, composition ordering, and beacon-bind failure cleanup |
| Verdict | **APPROVED** |
| Open findings | 0 critical, 0 high, 0 medium |
| Roadmap state during review | **PENDING / NON-EXECUTABLE** |

This supplement changes only this review artifact. It changes no reviewed
feature delta, roadmap field, production source, test body, DES event,
execution log, model, lane, or kernel state. It performs no Metal run,
mutation run, spike, commit, or implementation test.

## Necessity and exact public contract

The existing DG-4 contract already requires a failed node beacon-listener bind
to refuse boot with `health.startup.refused` reason
`vm_driver.beacon_bind`. The production composition currently reaches the
fallible `BeaconIntake::bind` after the VM and cgroup-accounting probes, but
its error arm is an explicit RED scaffold because no control-plane carrier was
previously pinned (`overdrive-control-plane/src/lib.rs:3343-3354`). A typed
carrier is therefore necessary to implement the accepted refusal without
flattening the existing `BeaconIntakeError::Bind` into `Internal(String)`,
misclassifying it as a probe failure, or inventing an unapproved generic
envelope.

The supplement closes that gap with exactly one public addition:

```rust
#[error("VM driver beacon intake bind refused: {source}")]
BeaconBind {
    #[source]
    source: overdrive_worker::BeaconIntakeError,
}
```

It is a variant of the existing public `VmmBootError`. Its field name, field
type, source annotation, and `Display` text are pinned. The existing
`Probe`, `CgroupAccountingProbe`, `KernelHeaderRead`, and `KernelFormat`
variants retain their shapes and meanings, and the existing transparent
`ControlPlaneError::VmmBoot(#[from] VmmBootError)` wrapper remains the outer
carrier. No `From<BeaconIntakeError>` conversion is declared, so the
composition boundary remains the one explicit semantic mapping point.

The public enum's implementation rustdoc currently describes four causes and
says `ControlPlaneError::VmmBoot` is reached only for Required-capability
probe failures (`error.rs:709-729,746-808`). Updating those counts and scope
when the new variant lands is bounded documentation fallout from the pinned
contract, not another API choice or a reason to alter any existing variant.

## Actual producer and caller path

The reviewed production path has one unambiguous owner and effect order:

1. `run_server` completes the cgroup bootstrap, guest-network phase-1 checks,
   and probe-runner gate, creates an empty `DriverRegistry`, and calls
   `compose_vm_driver` before `boot_server` (`lib.rs:2961-3020,3044-3053`).
2. `compose_vm_driver` probes the exact supplied `Vmm` and
   `CgroupAccounting` adapters before constructing the host layout and calling
   `BeaconIntake::bind(beacon_io, beacon_port)` (`:3207-3251,3312-3354`). A
   beacon bind failure is consequently downstream of successful capability
   probes; it is evidence that the selected VM composition reached its
   mandatory intake, not evidence that the optional VM capability is absent.
3. On success only, the intake exposes its bound port, is moved into
   `VmDriver::new`, and the resulting driver is returned (`:3355-3369`). On
   error, none of those steps occurs.
4. `run_server` inserts the `Vm` driver and stores the bound port only in the
   `Ok` arm (`:3021-3024`). A refused compose result logs the one startup
   refusal and returns before `boot_server` (`:3032-3044`).

This path supports the supplement's hard-refusal rule for both
`VmCapability::Optional` and `VmCapability::Required`. Capability controls
the interpretation of a failed VM/cgroup capability probe. It cannot turn a
later mandatory listener-bind failure into “not available” after those probes
have succeeded. Mapping the bind error directly to
`VmComposeError::Refused(VmmBootError::BeaconBind { source })` preserves that
distinction without adding a public error or retry path. `VmComposeError` is
crate-private, so its internal match mechanics remain the crafter's choice.

## Typed cause and observable mapping

| Layer / condition | Exact representation | Preserved information or outcome |
|---|---|---|
| `BeaconIo::bind(configured_port)` fails | `std::io::Error` | Original error kind, raw OS error when present, and diagnostic message |
| `BeaconIntake::bind` projects that failure | `BeaconIntakeError::Bind { port, source }` | Exact configured `VsockListenPort` plus the original `io::Error` as `#[source]` (`beacon_intake.rs:58-71,256-268`) |
| VM composition classifies the mandatory intake failure | `VmmBootError::BeaconBind { source }` | The complete `BeaconIntakeError::Bind` remains the typed source; no probe or capability-absence reinterpretation |
| `run_server` refuses the node boot | `ControlPlaneError::VmmBoot(VmmBootError::BeaconBind { .. })` | Existing outer boot-error family; no new `ControlPlaneError` variant |
| Operator observation | Exactly one `health.startup.refused`, reason `vm_driver.beacon_bind` | The bind-specific reason is distinct from `vmm.probe`; the returned value carries the same typed chain |

Neither `BeaconIntake::bind` nor the `BeaconIo` adapters emit a startup-refusal
event. The single event therefore belongs at the existing `run_server`
disposition boundary. Selecting `vm_driver.beacon_bind` for `BeaconBind` and
retaining `vmm.probe` for the existing probe family preserves the current
one-event boot refusal pattern without a logging helper, second event, or new
error discriminator.

## Failure atomicity and cleanup ownership

`BeaconIo::bind` already promises `Err: nothing bound`
(`overdrive-core/src/vm/beacon.rs:420-440`). The host adapter does not publish
its bound port or listener into adapter state until socket creation, bind,
listen, `getsockname`, and `AsyncFd` construction all succeed; any earlier
owned descriptor drops on return (`overdrive-host/src/beacon_io.rs:93-123`).
The simulation adapter likewise consumes its one-shot fault and returns before
setting `bound` (`overdrive-sim/src/adapters/beacon_io.rs:462-475`).

`BeaconIntake::bind` constructs its state, cancellation token, and accept task
only after `io.bind` succeeds (`beacon_intake.rs:58-71`). A bind `Err`
therefore leaves:

- no bound beacon listener or handle;
- no `BeaconIntake` value or accept task;
- no `VmDriver` value and no `Vm` registry entry;
- no bound beacon port published on the server handle; and
- no call to `boot_server`, so no later boot phase begins.

The already-completed earlier preflight/probe work is not rolled back by this
mapping and is not claimed to be. No beacon cleanup complement, secondary
error, retry, delayed task, or compensating mechanism is needed because the
port contract already supplies failure atomicity.

## Accepted outcome and evidence alignment

The existing `a_failed_beacon_bind_refuses_boot` composition scenario drives
`run_server` with a `SimBeaconIo` one-shot bind fault and already observes the
accepted external outcome: no listener is bound, one refusal reason equals
`vm_driver.beacon_bind`, and `boot_server`'s later shared-table write never
occurs (`shared_guest_network_startup.rs:442-456`). Aligning that scenario to
assert the nested `ControlPlaneError::VmmBoot` →
`VmmBootError::BeaconBind` → `BeaconIntakeError::Bind { port, source }`
chain, and exercising its existing composition with both capability values,
strengthens the oracle for the newly pinned representation. It does not add a
new user journey, failure stimulus, test seam, lane, or model state.

## Invariant and scope audit

| Concern | Assessment |
|---|---|
| Exact public API | One `VmmBootError::BeaconBind { source: BeaconIntakeError }` addition with pinned `#[source]` and `Display`; all existing variants and the outer wrapper keep their shapes. |
| Semantic error set | Complete: configured port and original `io::Error` survive in `BeaconIntakeError::Bind`, which survives in the boot-specific outer variant. |
| Optional capability | Preserved: only a failed capability probe soft-skips composition; a mandatory intake bind failure after successful probes refuses boot. |
| Required capability | Preserved: bind failure remains a hard refusal, with the bind-specific reason instead of the probe reason. |
| Event cardinality | Exactly one refusal event at `run_server`; no producer below it emits another. |
| Driver publication | None on failure: intake construction, driver construction, registry insertion, server boot, and bound-port publication all remain success-only. |
| Cleanup | Existing `BeaconIo::bind` failure atomicity is sufficient; no new cleanup or retry mechanism is introduced. |
| Persistence / wire / configuration | Unchanged; the configured beacon port is carried as typed failure context only. |
| Tests and model | Existing failed-bind scenario alignment only; no new scenario, lane, seam, or model. |

## Roadmap assessment

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The RED scaffold is reached by the real `run_server` → `compose_vm_driver` → `BeaconIntake::bind` production path, and the existing composition scenario drives that boundary. |
| Acceptance-criterion coupling | Pass | Exact variant, payload, mapping, capability disposition, event reason, and cleanup outcome are all observable or caller-branching contract. |
| Step decomposition | Pass | The public variant and its bounded `error.rs`/`lib.rs` caller fallout land with STEP 12-01's step-6 beacon composition cut. |
| Implementation code | Pass | The design pins the public and cross-crate error contract and outcomes while leaving private match structure to the crafter. |
| Concision and precision | Pass | There is one producer, one semantic variant, one existing outer wrapper, and one refusal reason; forbidden substitutes are explicit. |
| Test boundary | Pass | The existing integration scenario can assert the chain and both capability dispositions without another scenario or mechanism. |
| Mechanical validity | Pass | The reviewed feature-delta/roadmap diff passes `git diff --check`; `roadmap.json` parses and validation correctly remains pending during this review. |

## Supplement verdict

**APPROVED.** `VmmBootError::BeaconBind { source: BeaconIntakeError }` is the
minimum exact public carrier needed to implement DG-4's already-accepted
beacon-bind refusal. The complete typed chain preserves the configured port
and original `io::Error`; `compose_vm_driver` hard-refuses after successful
probes for both capability modes; `run_server` emits exactly one refusal with
reason `vm_driver.beacon_bind` and returns the existing
`ControlPlaneError::VmmBoot` wrapper. Existing bind atomicity leaves no
listener, intake, accept task, driver entry, server boot, or published port.
No accepted outcome is weakened, and no generalized error, cleanup, retry,
scenario, lane, model, or architectural mechanism is added.

The roadmap is eligible to return from `pending` to `approved` with this
supplemental independent DESIGN approval recorded alongside the earlier STEP
12-01 supplements. The typed beacon-bind mapping may be folded into the
accepted DG-4 composition contract. No additional user decision is required.
