# DESIGN remediation — step 03-01 EXEC-claim evidence

- **Feature:** `netns-density-295`
- **Decision:** `D-295-DELIVER-03-01`
- **Date:** 2026-09-22
- **Status:** Independently approved by solution-architecture review iteration 2
- **Review:** `arch_rev_20260922T152916Z_iteration_2` — zero critical/high/medium/low findings
- **Trigger:** `review-03-01.md` iteration-2 finding D2

## Scope

This decision resolves one contradiction: whether the private
`GuestNetworkExecState.active_claims +1/-1` field delta must be independently
observed by step `03-01`, despite the accepted opaque capability API exposing no
state snapshot. It does not change EXEC admission behavior, claim ownership,
recovery, fail-stop, or the dependency from step `03-01` to `03-02`.

## Observed production facts

- `GuestNetworkExecGate`, `GuestNetworkExecSupervisor`, and
  `GuestNetworkExecClaim` share one private state; `GuestNetworkExecState`
  contains private `state` and `active_claims` fields
  (`crates/overdrive-core/src/guest_network.rs:18-49`).
- `GuestNetworkExecGate::claim_release` increments `active_claims` only when the
  private state is Open, returns the opaque RAII claim, waits in BootClosed or
  Recovering, and returns `None` in FailStop
  (`crates/overdrive-core/src/guest_network.rs:173-200`).
- `GuestNetworkExecClaim::drop` checks and decrements `active_claims`, then
  notifies waiters (`crates/overdrive-core/src/guest_network.rs:320-327`).
- A repository-wide source scan finds no production read of `active_claims`
  outside that Drop debug assertion. The field has one initialization, one
  increment, one assertion, and one decrement. No public projection, admission
  decision, recovery transition, drain, or shutdown decision consumes it.
- The real owner is `VmDriver::release_for_exit_emission`: it awaits
  `claim_release` before taking `pending_exec` or `gate_sender`, binds the claim
  to the release future as `_exec_claim`, and keeps that value in scope across
  the beacon writer acknowledgement
  (`crates/overdrive-worker/src/vm_driver.rs:1828-1865`). Cancelling or completing
  that future drops the opaque claim through ordinary Rust ownership.
- The iteration-2 bounded spike removed only the decrement and left all current
  step-`03-01` selectors green. This proves the field delta is not observable at
  the accepted S-ND295-27 boundary; it does not prove a production failure.

## Contradiction

The accepted public contract intentionally makes all three capabilities opaque
and sanctions no state accessor. S-ND295-27 maps evidence to public method
returns, `recovery_progress`, blocking/wake/refusal, and the closed typed tables.
S-ND295-28 separately maps the real `VmDriver` acknowledgement and cancellation
schedules.

The former Contract Shape row nevertheless required a whole-private-state
assertion of `active_claims +1/-1`. The roadmap repeated that private-field
oracle. Satisfying those words would require a new observation boundary that
the API contract rejects, or an implementation-coupled test of bookkeeping that
no production decision consumes.

## Alternatives

### 1. Add a public or doc-hidden state accessor

This would make the counter directly observable but would widen the exact API,
turn bookkeeping into product/test surface, and violate the accepted opaque
capability boundary. Rejected.

### 2. Add a source-local private snapshot or test-only production seam

A core source-local test could inspect the field, or production could expose a
test-gated snapshot. Either choice would make the field layout an acceptance
contract without a stakeholder-visible or owner-visible consequence. The seam
would exist only to satisfy the contradictory oracle. Rejected.

### 3. Allocate evidence at the existing observable boundaries

S-ND295-27 proves the opaque gate's public return/projection and task-schedule
behavior. S-ND295-28 proves the real `VmDriver` owns one claim lifetime across
writer acknowledgement and that completion/cancellation ends that lifetime
without a detached writer. The private counter may remain as debug bookkeeping,
but its storage delta is not separately asserted. Selected.

## Decision and rationale

The private `active_claims +1/-1` field-level delta is **not** an independently
observed acceptance outcome for step `03-01`.

The complete accepted evidence contract is:

1. **S-ND295-27 / step 03-01:** public return values, `is_boot_closed`,
   `recovery_progress`, pending/wake/refusal behavior, terminal first-request
   behavior, and typed component/cause coverage through the opaque gate and
   supervisor capabilities.
2. **S-ND295-28 / step 03-02:** production `VmDriver` schedules showing claim
   acquisition before pending EXEC ownership, lifetime across beacon-writer
   acknowledgement, pre-detection completion, recovery blocking, FailStop
   refusal, and cancellation-owned lifetime end without a detached writer.

This is not a waiver of claim lifetime or Drop ownership. It removes only the
unsanctioned requirement to inspect one private storage representation. The
accepted Rust ownership and production schedule remain required. Step `03-01`
may therefore be reviewed without a private counter oracle; step `03-02`
remains mandatory before the feature's claim-lifetime evidence is complete.

## Exact contract changes

- `feature-delta.md` § *EXEC-close linearization* now describes an opaque claim
  lifetime and classifies `active_claims` as non-projected private bookkeeping.
- `feature-delta.md` § *Effect isolation and Contract Shape classification*
  declares the observable read-capability universe and assigns S-ND295-27 and
  S-ND295-28 their distinct evidence responsibilities.
- `feature-delta.md` § *Decisions Table* records this decision and its bounded
  authority.
- `distill/test-scenarios.md` makes the S-ND295-27 public evidence boundary and
  the S-ND295-28 production-owner evidence boundary explicit.
- `deliver/roadmap.json` removes the whole-private-state/counter oracle from
  steps `01-01` and `03-01`, pins claim-lifetime evidence to `03-02`, and records
  the independent iteration-2 approval.
- `docs/product/architecture/brief.md` aligns its effect-isolation catalogue to
  the same opaque observable boundary and S-ND295-27/S-ND295-28 evidence split.

## API and architecture impact

There is no public, doc-hidden, crate-private, or test-only API change. No type,
method, parameter, trait, enum variant, owner, dependency edge, persistence
model, consistency protocol, recovery protocol, or task topology changes. The
current private counter may remain in production as uncontracted debug
bookkeeping. The architecture brief receives only the required Contract Shape
prose alignment. Because component boundaries and runtime interactions are
unchanged, ADRs and C4 diagrams require no amendment.

## Acceptance, DISTILL, and roadmap impact

- The active S-ND295-27 bodies are sufficient evidence for the step-`03-01`
  gate contract when their public assertions pass.
- A mutation that changes only private counter bookkeeping is outside the
  accepted S-ND295-27 oracle and is not a step-`03-01` blocker.
- S-ND295-28 is not pulled into step `03-01`; its real-`VmDriver` schedules stay
  in step `03-02` and remain required.
- No acceptance body is waived, deleted, weakened, re-authored, or moved.
- No RED classification changes; the iteration-2 spike remains evidence of the
  former contract contradiction, not a product defect.

## Non-goals

- Exposing gate state or the counter.
- Adding a second gate API, private snapshot, or test hook.
- Removing or changing current production bookkeeping.
- Changing claim, waiter, recovery, fail-stop, writer, or cancellation behavior.
- Pulling step-`03-02` implementation or tests into step `03-01`.
- Redesigning ownership, persistence, restart, recovery, or shutdown.
- Modifying the iteration-2 review artifact or DES execution log.

## Handoff criteria

Independent review iteration 2 approved this remediation after confirming:

1. every private-counter oracle is removed from the authoritative Contract
   Shape and roadmap text;
2. S-ND295-27 still requires every accepted public return, projection,
   blocking/wake/refusal, and terminal behavior;
3. S-ND295-28 still requires the real `VmDriver` claim-lifetime,
   acknowledgement, and cancellation schedules;
4. no API or implementation/test seam was introduced;
5. the architecture brief carries the same opaque observable boundary; and
6. step `03-01` review is instructed to reassess D2 against this evidence split,
   while step `03-02` remains the mandatory dependent evidence step.
