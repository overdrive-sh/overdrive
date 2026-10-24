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
