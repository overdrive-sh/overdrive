# SLOT-ATTACH-REFUSAL-SHAPE / STEP 11-06 independent DESIGN review

## Review metadata

| Field | Value |
|---|---|
| Review ID | `SLOT-ATTACH-REFUSAL-SHAPE-STEP11-06-ARCH-REVIEW-01` |
| Date | 2026-10-23 |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The crate-private guest UDP-slot attach-refusal carrier, exact `ForwarderError` mapping, resident-slot lifecycle, bounded caller/compiler fallout, one proposed source-local test and reconciled STEP 11-06 roadmap |
| Verdict | **CHANGES_REQUESTED** |
| Blocking findings | 2 high |
| Non-blocking findings | 1 medium |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

This review read the proposed correction in
`.context/netns-density-295/11-06-design/slot-attach-refusal.md`, the normative
feature-delta and ADR-0150/ADR-0210 changes, STEP 11-06's roadmap delta, the
current `GuestOwnerIo`, `GuestFlowOwner`, kernel and sim adapters, the public
`FlowForwarder` error contract, the existing UDP-slot tests and the accepted
Quint evidence. It made no production, test, roadmap, ADR, feature-delta, DES
or execution-log change.

## Reproduced path and necessity

The gap is concrete and belongs to the interface contract:

1. `GuestFlowOwner::handle_control(Paired)` removes the pending datagram record
   and calls `attach_slot(datagram.slot, datagram.vsock)`
   (`guest_flow/owner.rs:504-540`).
2. The adapter calls `FlowForwarder::attach(S, V_g, Datagram { udp: A })`, with
   resident UDP S as side A and the consumed flow vsock V_g as side B
   (`kernel_io.rs:841-870`; `sim.rs:1815-1859`).
3. `SimFlowForwarder` checks both halves after validating the datagram kinds and
   returns `ForwarderError::HalfEnded { side: B, errno }` for an ended V_g
   (`overdrive-sim/src/adapters/flow_forwarder.rs:429-475`). The public port
   requires every refused attach to change neither half
   (`overdrive-core/.../forwarder.rs:351-376`).
4. Both current adapters panic on the Err branch because
   `GuestAttachRefused { error, vsock, outer }` requires two owned socket
   handles while `attach_slot` owns only V_g. S is borrowed by slot identity and
   correctly has no `GuestOwnedSocket` arm (`sim.rs:1851-1857`;
   `kernel_io.rs:864-870`; `guest_flow/mod.rs:185-212`).

The same accepted scheduling fact was executed on Linux in the repository Lima
VM: `a_v_g_reset_before_paired_is_refused_half_ended_and_i_is_reset` passed in
nextest run `0b44046b-0b99-4890-b941-36ffda892950`. The common
`SimFlowForwarder` ended-half classifier passed in run
`b077f904-633a-44f0-b56e-f2183cdff295`. The existing datagram success path
passed in run `fe240d06-1b73-4e62-b4ed-1d67766dec56`. The datagram refusal
composition cannot currently complete because its exact adapter branch is the
panic above. This is the original defect isolated from the proposed carrier,
not a test-only invented state.

The proposed interface split is necessary and proportionate. A private helper
cannot return consumed V_g through the existing two-owned-half error type, and
fabricating or generalizing an outer handle would violate resident-slot
ownership. The design appropriately leaves the public `FlowForwarder` and
`ForwarderError` contracts unchanged.

## Findings

### F-01 — “no end record” is unqualified even though the already-attached host pair produces one

- **Severity:** High.
- **Claim:** The proposal says no pair or end record exists after the refused
  guest attach. That is true only in the refusing guest forwarder. The host has
  already attached V_h–H and sent `Paired`; closing returned V_g ends that host
  pair through its normal host forwarder end record. An unqualified global
  “no end record” conflicts with the reachable two-owner order.
- **Reachable path:** The feature's datagram order attaches V_h–H and sends
  `Paired` at step 4, then the guest attempts S–V_g at step 5
  (`feature-delta.md:2949-2964`). The proposed guest refusal closes V_g so “the
  host learns the guest-side abort” (`slot-attach-refusal.md:80-85`). Under
  EVMOD-REC/EVMOD-END, the already-live host pair consequently queues its one
  end record. Only the failed guest `attach_slot` created no guest pair/id and
  therefore no guest forwarder record.
- **Conflicting wording:** `.context` lines 84-85; feature delta lines
  2959-2960 and 11457-11461. ADR-0210's generic “no end record follows” also
  needs to remain explicitly local to the forwarder that refused the attach
  when applied to this guest-on-`Paired` case.
- **Risk:** DISTILL or DELIVER can assert that neither owner receives an end
  record, suppress the required host record, or leave the host pair's end
  unspecified.
- **Required revision:** State exactly: no pair/id or end record is created in
  the **refusing guest forwarder**; closing V_g ends the already-attached host
  pair through its normal host forwarder record; the guest sends no control
  message. Apply the same owner-qualified wording across the handoff, feature
  contract and ADR-0210.
- **Disposition:** Open.

### F-02 — ADR-0210's cause projection contradicts the exact all-error cleanup mapping

- **Severity:** High.
- **Claim:** The exact correction maps every `FlowForwarder::attach` error into
  `GuestSlotAttachRefused`, closes returned V_g and ends the local association.
  ADR-0210 still says owner-defect errors are not projected into a flow abort
  while only `ECONNABORTED` is. Without distinguishing classification/counter
  projection from cleanup, the ADR has two interpretations for non-`HalfEnded`
  variants.
- **Evidence:** The exact seam maps every error and performs close plus
  `end_association(slot, release=false)` for every Err
  (`feature-delta.md:11452-11461`; `slot-attach-refusal.md:73-85`).
  `ForwarderError` also includes `UnsupportedSocket`, `UpperLayerProtocol`,
  `NotConnected`, `AlreadyHooked`, `BufferTooSmall` and `Io`, with existing
  payloads unchanged (`forwarder.rs:503-575`). ADR-0210 lines 94-101 says only
  the second class is projected into a flow abort.
- **Required revision:** Pin the distinction in ADR-0210: every refused
  `attach_slot` ends that local association and returns V_g for close; only
  `HalfEnded` is classified/counted as the accepted ended-half flow event and
  increments `forwarder_attach_ended`. All other variants retain their existing
  owner-defect meanings and payloads without changing cleanup ownership.
- **Disposition:** Open. The typed semantic set is complete in the feature
  contract but inconsistent across its owning ADR.

### F-03 — the handoff describes a fake kernel handle that current source no longer creates

- **Severity:** Medium.
- **Claim:** The reproduced-gap section says the current kernel draft
  fabricates `HostConn(SocketId(0))`. Current `kernel_io.rs` instead panics and
  explicitly states that no fake handle is manufactured.
- **Evidence:** `slot-attach-refusal.md:31-33` versus
  `kernel_io.rs:864-870`. The sim has the analogous panic at
  `sim.rs:1851-1857`.
- **Required revision:** Record the actual current fact: both adapter drafts
  panic on a refused slot attach because the interface cannot carry the owned
  V_g while retaining resident S. Mention a fake handle only as abandoned
  history if it is clearly labelled and evidenced.
- **Disposition:** Open, non-blocking by itself.

## Exact interface and ownership assessment

Subject to the cross-document corrections above, the proposed shape is sound:

```rust
pub(crate) struct GuestSlotAttachRefused {
    pub(crate) error: ForwarderError,
    pub(crate) vsock: HostConn,
}

fn attach_slot(
    &mut self,
    slot: SocketAddrV4,
    vsock: HostConn,
) -> Result<ForwardId, GuestSlotAttachRefused>;
```

| Contract question | Assessment |
|---|---|
| Is V_g ownership returned on every Err? | Yes. `HostConn` is the exact consumed input type and remains move-only. |
| Is resident S falsely transferred? | No. S stays in the slot table and is referenced only by the caller's existing slot identity. |
| Is the stream contract weakened? | No. `GuestAttachRefused { error, vsock, outer }` and both TCP call sites stay unchanged. |
| Is public API added? | No. The new carrier and `GuestOwnerIo` method are crate-private; public `FlowForwarder`, `ForwarderError`, wire and persistence stay unchanged. |
| Is the typed cause preserved? | Yes. Every existing `ForwarderError` variant and payload is moved unchanged into `error`; no generic string, optional outer or new public variant is introduced. |
| Can an unknown/recycled slot become a runtime error? | No. It remains a violated per-kind precondition; the owner supplies the configured resident slot retained by the current association. |
| Does an Err hook either half? | No. The public forwarder contract already guarantees an attach Err changes neither half. |

The exact state delta for the guest refusal is bounded: remove the pending
datagram attempt, increment `forwarder_attach_ended` only for `HalfEnded`, close
returned V_g, discard and disconnect resident S, clear its association/attach
state, retain its `(application socket, destination)` owner, emit no guest
control message, and allow a later application datagram to create the next
association. No guest pair/id/end record is created. Every unrelated slot,
flow, counter, public error and stream refusal remains unchanged.

## Test necessity and authorization boundary

The proposed UDP-slot cell is genuinely new coverage rather than a neutral
compiler correction:

- `udp_slots.rs` currently has 12 tests and none drives an attach refusal;
- the existing stream owner test proves the V_g-reset/`Paired` schedule but has
  an owned TCP outer half, so it cannot prove resident S is retained;
- the `overdrive-sim` forwarder test proves `HalfEnded` and no hooks, but does
  not compose `GuestFlowOwner`, the datagram carrier, slot cleanup or
  re-association; and
- the new cell alone observes the user-relevant complement: V_g closed,
  `forwarder_attach_ended` incremented once, S discarded/disconnected, owner
  retained, no guest pair/control message, and a later datagram re-associates.

Changing `udp_slots` from 12 to 13 is therefore technically justified and is
the minimum honest regression. It uses the existing source-local owner rig and
real `SimFlowForwarder`; it needs no new lane, fixture, public test support or
production seam. The user has not yet explicitly approved this additional
test, so this review does not authorize DISTILL to add it. The orchestrator must
ask the user for that one exact coverage addition after the documentation
findings are closed.

No Quint change is justified. The accepted `fwd_pair` model already covers
ended-half refusal and flow abort; the gap is the Rust port's ownership/error
representation and resident-slot cleanup composition. Any separately pending
Quint expansion remains outside this correction and still requires its own
authority.

## Bounded fallout review

The named fallout is complete: the crate-private carrier/import, exact
`GuestOwnerIo::attach_slot` result, kernel and sim adapters, the datagram
`Paired` Err arm, exhaustive matches, and—only after explicit approval—the one
source-local UDP-slot cell. The public forwarder/error vocabulary, TCP attach
methods, stream refusal carrier, `GuestOwnedSocket`, owner event vocabulary,
wire, persistence, model and lane remain unchanged. No fake handle, optional
stream outer or slot-owned generic close capability is permitted.

## Roadmap checks

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | STEP 11-06 already invokes the guest owner through its source-local production/sim composition; the proposed cell reaches `handle_control(Paired)` and the real `SimFlowForwarder`. |
| Acceptance-criterion coupling | Pass for an interface correction | The crate-private carrier/signature are DESIGN-owned because the old port cannot represent its owned error result; the criterion also asserts observable slot lifecycle outcomes. |
| Step decomposition | Pass | This is one correction inside the existing guest-owner/transport step, not another implementation step. |
| Implementation code | Pass | The roadmap pins ownership, state deltas and typed mapping while leaving helper structure to the crafter. |
| Concision and precision | Changes requested | The unqualified end-record and ADR cause-projection wording must be corrected per F-01/F-02. |
| Test boundary | Pass, authority pending | One source-local owner cell covers the new behavior; the crafter remains forbidden to author/edit it. DISTILL cannot proceed until explicit user approval. |

`roadmap.json` parses and `git diff --check` passes for the reviewed design and
roadmap files. Validation correctly remains `pending`; STEP 11-06 is not
executable. The roadmap must not be marked approved until F-01/F-02 are closed,
the correction is re-reviewed, and its validation record explicitly preserves
the unresolved user/DISTILL gate for the one new test.

## Verdict

**CHANGES_REQUESTED.** The new crate-private datagram refusal carrier is the
smallest correct interface and preserves the accepted EVMOD-ATTACH-END and
resident-slot invariants. The current fold is not yet approval-ready because
it is imprecise about the already-attached host pair's end record and
contradicts itself about cleanup of non-`HalfEnded` error variants. The handoff
also contains one stale source claim. After those bounded documentation fixes,
return this exact contract for re-review. If technically approved, the one new
UDP-slot cell must then receive explicit user authorization before DISTILL;
the separate Quint-model question remains untouched. No implementation, test,
DES, roadmap approval or commit is authorized by this iteration.

---

# Review iteration 2 — remediated ownership and end-record contract

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `SLOT-ATTACH-REFUSAL-SHAPE-STEP11-06-ARCH-REVIEW-02` |
| Date | 2026-10-23 |
| Reviewed at | `2026-10-23T13:27:31Z` |
| Reviewer | `nw-solution-architect-reviewer` |
| Technical verdict | **APPROVED FOR USER DECISION** |
| Workflow verdict | **USER_DECISION_REQUIRED** |
| Blocking technical findings | 0 |
| Roadmap disposition | **PENDING / NON-EXECUTABLE until the one new test is explicitly approved and DISTILL-reviewed** |

This iteration re-read the remediated handoff, feature contract, ADR-0210 and
STEP 11-06 roadmap against the same current production/sim paths and executed
Lima evidence recorded in iteration 1. It did not edit or run production code,
tests, DES, models, the roadmap or kernel state.

## Iteration 1 finding dispositions

| Finding | Remediation | Disposition |
|---|---|---|
| F-01 — unqualified “no end record” | Every canonical location now says the refusing **guest** forwarder creates no guest pair/id/end record or guest control message, while closing V_g ends the already-attached host pair through its normal host end-record path | **Closed.** Both owner contexts and their distinct records are explicit. |
| F-02 — cause projection contradicted all-error cleanup | ADR-0210 now says every guest `attach_slot` Err closes V_g and ends the local association; only `HalfEnded` is classified/counted as the ended-half event, while all other `ForwarderError` variants retain their owner-defect meanings/payloads | **Closed.** Cleanup ownership and semantic classification no longer conflict. |
| F-03 — stale fake-handle source claim | The handoff now records the actual current fact: both sim and kernel adapters panic because the old interface cannot return V_g while retaining S | **Closed.** No fabricated `SocketId` is attributed to current source. |

The remediation changes no approved outcome, carrier field, error variant, test
count or mechanism.

## Exact approved technical contract

The corrected interface is complete and has one interpretation:

```rust
pub(crate) struct GuestSlotAttachRefused {
    pub(crate) error: ForwarderError,
    pub(crate) vsock: HostConn,
}

fn attach_slot(
    &mut self,
    slot: SocketAddrV4,
    vsock: HostConn,
) -> Result<ForwardId, GuestSlotAttachRefused>;
```

The stream carrier remains exactly
`GuestAttachRefused { error, vsock, outer }`. `GuestOwnedSocket` gains no slot
arm. The public `FlowForwarder` and `ForwarderError` APIs, variants and payloads
remain unchanged.

For every guest datagram attach Err:

1. `attach_slot` returns the consumed move-only V_g as `HostConn` and moves the
   unchanged `ForwarderError` into `error`;
2. neither half was hooked and resident S remains in the slot table;
3. the owner closes returned V_g;
4. the owner runs `end_association(slot, release=false)`: discard S's receive
   queue, disconnect S, clear association/attach state and retain the slot's
   `(application socket, destination)` owner;
5. only `HalfEnded` increments `forwarder_attach_ended` and carries the
   accepted ended-half flow-event classification; other variants keep their
   owner-defect semantics and exact payloads; and
6. the refusing guest forwarder creates no guest pair/id/end record and sends
   no guest control message. V_g close drives the already-attached host pair's
   ordinary host end record.

An unknown or recycled `slot` remains an unreachable violation of the
per-kind owner order. It is not a new runtime error, optional result or guessed
handle.

## Reachability and invariant review

The current path remains concretely reachable:

- `handle_control(Paired)` removes the pending datagram and invokes
  `attach_slot(slot, V_g)`;
- datagram mode supplies S as side A and V_g as side B;
- the common `SimFlowForwarder` locked validation returns
  `HalfEnded { side: B, errno }` when V_g reset/closed in the accepted race;
- both current adapters panic precisely at that Err branch because the old
  carrier cannot represent its ownership; and
- the existing Lima receipts prove the V_g-reset-before-`Paired` schedule, the
  common ended-half classifier and the composed datagram success path.

The remediation preserves EVMOD-ATTACH-END, EVMOD-REC, EVMOD-DGRAM and U-2:
the ended half is refused before hooks, the guest has no id/record for a pair it
never created, the already-live host pair still yields its one normal record,
queued slot datagrams are discarded before disconnect, and a later application
datagram may re-associate through the still-owned slot. No invariant was
weakened to fit the current implementation.

## Typed-error completeness

| Semantic condition | Exact representation and handling | Assessment |
|---|---|---|
| Ended V_g | `GuestSlotAttachRefused { error: HalfEnded { side: B, errno }, vsock: V_g }`; count once, close V_g, end association | Complete. |
| Other forwarder refusal (`UnsupportedSocket`, `UpperLayerProtocol`, `NotConnected`, `AlreadyHooked`, `BufferTooSmall`, `Io`) | Same carrier with unchanged variant/payload; no ended-half counter, but identical ownership cleanup | Complete. No string encoding or swallowed cause. |
| Unknown/recycled slot | Violated typed/order precondition; unreachable, not a refusal variant | Complete under the existing owner order. |
| Successful attach | Existing `ForwardId`; S and V_g become the forwarder pair and the slot records the id | Unchanged. |

The semantic set is exhaustive at the relevant crate-private interface. No new
public error, optional outer, fake socket identity or broad enum is needed.

## Bounded fallout and scope

Required compiler fallout is limited to the new crate-private carrier/import,
the `GuestOwnerIo::attach_slot` result, kernel and sim implementations, the
guest owner's datagram `Paired` Err arm and exhaustive matches. TCP callers and
their stream error type do not change. No owner event, wire type, persistence,
lane, fixture, recovery path, payload mechanism or model state changes.

This is necessary interface work under the accepted resident-slot and
ended-half invariants. It is not optional hardening or an invented API.

## Test-coverage decision

Iteration 1's coverage conclusion stands. The proposed thirteenth
`udp_slots.rs` cell is the minimum non-duplicative proof of the new
composition:

- the 12 existing UDP-slot cells never drive `attach_slot` Err;
- stream refusal coverage cannot prove borrowed resident S is retained;
- the forwarder-level ended-half test cannot prove owner cleanup or later
  re-association; and
- the new source-local cell alone composes the real `SimFlowForwarder`, exact
  datagram carrier, guest owner counter/close, discard/disconnect, retained
  owner and later re-association.

The test changes no lane and needs no fixture, public test-support API,
production branch or Quint variable. It is genuinely new coverage, not neutral
compiler fallout. The user has not approved that additional cell. This review
therefore approves the technical contract for the following exact user
decision, but does not answer it:

> Approve one additional source-local UDP-slot attach-refusal cell in the
> existing `overdrive-init` guest-owner suite, increasing `udp_slots` from 12
> to 13, to verify V_g `HalfEnded` racing `Paired`, V_g close and one counter,
> resident S discard/disconnect with slot ownership retained, no guest
> pair/end-record/control message, the host pair's normal end-record path, and
> later datagram re-association; with no new lane, model or public test support?

DISTILL may author it only after an affirmative user decision. A rejection
would require the orchestrator to return the validation gap rather than let the
crafter add or silently omit the test.

## Quint boundary

No Quint change is part of this correction. The accepted `fwd_pair` model
already covers ended-half refusal and flow abort; this gap is the Rust
ownership/error carrier and resident-slot cleanup composition. The separate
pending user question about additional model coverage is neither approved nor
answered by this review.

## Roadmap revalidation

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The proposed source-local cell invokes the guest owner through `Paired` and the real `SimFlowForwarder`; the production kernel adapter has the same ownership result. |
| Acceptance-criterion coupling | Pass | The exact carrier/signature are DESIGN-owned interface contracts; lifecycle assertions are observable owner/slot effects. |
| Step decomposition | Pass | The carrier and one cell belong to existing STEP 11-06; no step or lane is added. |
| Implementation code | Pass | The roadmap pins inputs, ownership, errors and outcomes while leaving private helper structure to the crafter. |
| Concision and precision | Pass | Guest versus host records, every-error cleanup and `HalfEnded`-only classification are explicit. |
| Test boundary | Pass technically; authority pending | DISTILL owns the one source-local cell. The crafter remains forbidden to author/edit it. |
| No scope growth | Pass | No model, public API, wire, persistence, lane, fixture or unrelated mechanism is changed. |

`roadmap.json` parses and the reviewed files pass `git diff --check`. Its
validation must remain `pending`: the prior TCP-CORRELATION validation was
reopened correctly, and the newly justified cell lacks explicit user authority
and DISTILL review. Technical approval of this correction does not make STEP
11-06 executable.

## Iteration 2 verdict

**APPROVED FOR USER DECISION.** All iteration-1 findings are closed. The
crate-private V_g-only carrier is the smallest correct representation, its
typed error set and ownership transitions are complete, resident S and the
host/guest end-record paths remain exact, and the bounded fallout adds no
unapproved mechanism. The thirteenth UDP-slot cell is genuinely necessary but
still requires the explicit user decision quoted above. Roadmap validation
remains pending, DISTILL and DELIVER remain paused, and the separate Quint
question remains untouched. No production source, test body, DES state, model,
roadmap approval or commit is authorized by this review.

---

# Review iteration 3 — user-approved test fold and roadmap revalidation

## Iteration 3 metadata

| Field | Value |
|---|---|
| Review ID | `SLOT-ATTACH-REFUSAL-SHAPE-STEP11-06-ARCH-REVIEW-03` |
| Date | 2026-10-23 |
| Reviewed at | `2026-10-23T13:31:45Z` |
| Reviewer | `nw-solution-architect-reviewer` |
| Scope | The iteration-2-approved carrier and ownership contract, explicit UDP-slot test authority, canonical fold and reconciled STEP 11-06 roadmap |
| Verdict | **APPROVED** |
| Blocking findings | 0 |
| Roadmap disposition | **APPROVE after recording this review; execution remains DISTILL-gated** |

The authority is exact. After iteration 2 established that the thirteenth
UDP-slot cell was genuinely new and necessary, the user replied “Approve the
UDP-slot test” to the exact source-local coverage request. The later reply
“approve quint nodel and UDP-slot test” separately approved model work; that
model is a different future task and is not folded into this correction.

This iteration reviewed only the canonical interface/test fold and STEP 11-06
reconciliation. It did not modify or execute production code, tests, Quint,
DES or kernel state.

## Authority and fold equality

| Authorized item | Folded evidence | Assessment |
|---|---|---|
| V_g-only crate-private carrier | Handoff, feature `GuestOwnerIo` contract, ADR-0210 and roadmap all name `GuestSlotAttachRefused { error: ForwarderError, vsock: HostConn }` | Exact. |
| Stream carrier unchanged | Feature and handoff retain `GuestAttachRefused { error, vsock, outer }`; ADR-0210 keeps stream ownership separate | Exact. |
| Every attach Err returns V_g and ends local association | Handoff mapping, feature attach-refusal mapping and ADR-0210 consequences use the same every-error rule | Exact. |
| `HalfEnded` alone is the ended-half event/counter | All canonical artifacts retain every other `ForwarderError` variant's owner-defect meaning and payload | Exact typed-error projection. |
| Resident S never becomes a generic owned handle | Handoff/feature/ADR reject `GuestOwnedSocket::Slot`, optional stream outer and fake handles | Exact. |
| Guest versus host records | All artifacts say the refusing guest forwarder creates no guest pair/id/end record/control message; V_g close follows the already-attached host pair's normal end-record path | Exact. |
| Existing association-end order and retained owner | Close V_g, discard S, disconnect S, clear association/attach state with `release=false`, retain `(application socket, destination)` owner | Exact. |
| One additional source-local cell | Approval record, ADR-0150, ADR-0210, roadmap count and criterion change `udp_slots` from 12 to 13 in the existing suite | Exact user authority. |
| No model in this fold | Handoff scope, ADR-0150 and roadmap add no Quint state/count/scenario | Exact separation from the later model task. |

The feature approval record quotes the user's test approval and the exact
behavior it covers. ADR-0150 and ADR-0210 mark the bounded carrier/regression
accepted on 2026-10-23. No proposed/pending slot-refusal wording remains in the
canonical design; only roadmap/DISTILL execution status remains pending during
this review.

## Contract and invariant revalidation

The exact interface remains complete:

```rust
pub(crate) struct GuestSlotAttachRefused {
    pub(crate) error: ForwarderError,
    pub(crate) vsock: HostConn,
}

fn attach_slot(
    &mut self,
    slot: SocketAddrV4,
    vsock: HostConn,
) -> Result<ForwardId, GuestSlotAttachRefused>;
```

The error carrier returns precisely the capability the adapter consumed.
Borrowed S stays resident and is cleaned through its existing slot identity.
Every existing `ForwarderError` variant and payload crosses unchanged; no
string error, fake id, optional outer, public variant or broadened socket
capability appears. An unknown/recycled slot remains an unreachable violation
of the already-pinned per-kind order.

EVMOD-ATTACH-END remains fail-closed: the ended V_g is refused before either
half is hooked, counted once and closed. EVMOD-REC remains exact in both
contexts: no guest pair means no guest record, while the host pair created
before `Paired` ends through its ordinary one-record path. EVMOD-DGRAM/U-2
remain exact: queued S datagrams are discarded before disconnect; ownership is
retained; only a later application datagram starts the next association.

No accepted `must`, `never`, exact count, typed cause, ownership boundary or
observable outcome is relaxed. The change makes the accepted outcome
representable instead of preserving an adapter panic.

## Test authorization and boundary

The approved source-local cell is still the minimum distinct test:

- it drives V_g `HalfEnded` in the `Paired` race through the real
  `SimFlowForwarder` and `GuestFlowOwner`;
- it observes the exact returned carrier, one counter increment and V_g close;
- it proves resident S is discarded/disconnected but stays owned;
- it proves no guest pair/id/end record/control message while retaining the
  host pair's normal end-record path; and
- it sends a later application datagram and proves re-association.

The roadmap correctly changes `udp_slots` from 12 to 13 and adds no lane,
fixture, public test support or production-only test branch. DISTILL owns the
body; the crafter remains forbidden to author or edit it.

The separately approved Quint-model task is absent from this fold. Existing
STEP 11-06 Quint counts and scenario assignments stay unchanged. This review
does not validate, authorize implementation of, or absorb that future task.

## Bounded compiler and caller fallout

The fold names all required fallout: the crate-private carrier/import,
`GuestOwnerIo::attach_slot` result, kernel and sim adapters, the datagram
`Paired` error arm, exhaustive matches and the approved source-local cell. TCP
attach methods, `GuestAttachRefused`, `GuestOwnedSocket`, public
`FlowForwarder`/`ForwarderError`, owner events, wire, persistence and lifecycle
models remain unchanged. No unrelated cleanup or generalization entered the
roadmap.

## Roadmap revalidation

| Mandatory check | Result | Evidence |
|---|---|---|
| External validity | Pass | The approved cell composes `GuestFlowOwner` and the real `SimFlowForwarder` through `Paired`, while the kernel adapter implements the same V_g/S ownership contract. |
| Acceptance-criterion coupling | Pass | The carrier/signature are necessary DESIGN-owned interface shape; the criterion asserts observable counter, socket, slot and re-association results. |
| Step decomposition | Pass | One carrier and one cell stay inside existing STEP 11-06; no step or lane is added. |
| Implementation code | Pass | The roadmap fixes ownership/error/lifecycle semantics and leaves private helper structure to the crafter. |
| Concision and precision | Pass | Every-error cleanup, `HalfEnded`-only classification and guest/host record paths each have one meaning. |
| Test boundary | Pass | The new cell is explicitly user-approved and DISTILL-owned before DELIVER; the crafter cannot author/edit it. |
| Scope audit | Pass | No Quint change, public API, wire, persistence, lane, fixture, recovery mechanism or unrelated hardening is folded. |
| Mechanical validity | Pass | `roadmap.json` parses, reviewed files pass `git diff --check`, STEP 11-06 carries `SLOT-ATTACH-REFUSAL-SHAPE`, and validation remained pending through this review. |

The roadmap is approved as the implementation plan for this folded contract.
Execution remains ordered: DISTILL must author and independently review the
user-approved thirteenth UDP-slot cell and the previously authorized
TCP-CORRELATION corrections before a crafter resumes STEP 11-06. The separately
approved Quint-model work remains outside this roadmap fold and follows its own
task/review path.

## Exact roadmap validation record

Record exactly:

```json
{
  "status": "approved",
  "reviewer": "Independent SLOT-ATTACH-REFUSAL-SHAPE folded-contract and STEP 11-06 roadmap review iteration 3 approved on 2026-10-23; DELIVER remains gated until DISTILL authors and independently reviews the user-approved thirteenth source-local UDP-slot cell and the previously authorized TCP-CORRELATION corrections. The separately approved Quint-model work remains a separate task outside this fold.",
  "approved_at": "2026-10-23T13:31:45Z"
}
```

The DESIGN owner owns that scoped roadmap update and commit; this reviewer does
not self-record or commit it.

## Iteration 3 verdict

**APPROVED.** The final fold equals the technically reviewed contract and the
user's exact UDP-slot test authority. V_g-only ownership, every-error cleanup,
`HalfEnded`-only classification, resident S, distinct guest/host record paths,
the 12-to-13 test count and the DISTILL boundary are consistent across the
handoff, feature delta, ADRs and roadmap. No invariant is weakened and no
unrelated scope is included. Record the exact roadmap validation block above;
then keep DELIVER paused until the required DISTILL work and independent review
complete. No production source, test body, Quint model, DES state or commit was
changed by this review.
