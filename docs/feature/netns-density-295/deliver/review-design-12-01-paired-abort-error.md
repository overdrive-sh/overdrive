# Independent DESIGN review — STEP 12-01 pre-Paired guest `Abort` typed error

## Current disposition

| Field | Value |
|---|---|
| Current iteration | 2 — post-approval authority and fold reconciliation |
| Decision | `PABORTERR1` (reviewed as `PABORT-ERR-1`) |
| User authority | **APPROVED 2026-10-26** — exact quote: “i approve the rest of the decisions.” |
| Canonical authority | `feature-delta.md` authority-table row `BFIFO1, PABORTERR1, ACTPRIORITY1, ADMISSIONREFUSAL1, QUIESCENCEOPEN1, CHCGROUP55RUNNER1` |
| Technical findings | 0 |
| Final verdict | **APPROVED** |
| Roadmap disposition | Validation remains **pending evidence** with `approved_at = null` |

The Iteration 1 metadata and verdict below record the pre-approval review state.
Iteration 2 at the end of this artifact supersedes only its authority gate.

## Metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_paired_abort_error` |
| Date | 2026-10-26 |
| Role | Independent solution architecture / typed-contract reviewer |
| Iteration | 1 |
| Reviewed proposal | `docs/feature/netns-density-295/deliver/design-12-01-paired-abort-error.md` |
| Technical findings | 0 |
| Authority finding | 1 blocking user-decision gate |
| Verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |
| Technical disposition | **APPROVED FOR PRESENTATION** — exact proposal is coherent and reviewable |
| Roadmap disposition | **PENDING / NON-EXECUTABLE**; `approved_at = null` |

This review applies the repository DESIGN, production-reachability, exact API,
typed-error, no-unrequested-scope, and no-weakening rules together with
`nw-design`, `nw-sar-critique-dimensions`, and
`nw-roadmap-review-checks`. The repository's required native-Markdown artifact
governs over the reviewer definition's legacy YAML format.

The review is bounded to the reproduced `GuestAcceptedError -> HostRecvAbort`
typed-result mismatch, the actual guest/wire/host/dial cleanup path, the
proposed existing-variant mapping, every other `GuestDialError` producer, and
the pending canonical and roadmap annotations. It creates only this review
artifact. It does not edit the proposal, canonical design, roadmap, model,
driver, production source, tests, DES state, or evidence; run a model, Metal,
test, build, mutation, or record command; or create a commit.

## Reproduced divergence

The retained log
`.context/netns-density-295/12-01/owner-flows-seed-3c09cb16-verbose1.log`
records Metal run `5c554937-a39a-40dc-b2aa-b64121b30f76`,
`QUINT_SEED=0x3c09cb16`, and the existing named conformance run
`AcceptedVgErrorAbortsWithAbort`.

Its relevant suffix is:

```text
DialOpen(1)
  -> GuestAcceptV(1)
  -> GuestAcceptedError(1)
  -> HostRecvAbort(1)
```

Raw-log lines 355-389 establish that the guest's accepted/unattached flow is
closed, the host dial remains requested with result `pending`, and one guest
`Abort` is in flight. After `HostRecvAbort`, lines 400-479 show agreement on
the flow phases and message consumption. The only compared-state difference
is exact and typed:

```text
model:      Unreachable:io
production: Unreachable:ECONNRESET
```

The failure occurs in an existing named run through production owner code. It
is not a test-only state, imagined schedule, new scenario, or driver-created
value.

## Complete production and wire path

| Boundary | Verified behavior |
|---|---|
| Guest accepted-flow owner | A terminal `GuestHangup::Error` on accepted but unattached `V_g` or C enters `GuestFlowOwner::handle_hangup`, aborts the owned sockets, and sends the existing `ControlMessage::Abort { flow }` while the control session is live (`overdrive-init/src/guest_flow/owner.rs:662-699`). |
| Wire | `Abort` is control type 3 with only `flow`. Its canonical encoder writes detail and errno as zero; the decoder rejects nonzero values (`wire.rs:458-489`, `:497-514`, `:555-562`). The guest's private `ErrorKind` is not transported. |
| Host control owner | Receipt of `Abort { flow }` calls `abort_guest_flow(..., FlowCloseCause::AbortReceived)` (`owner_runtime.rs:258-286`). |
| Pending-dial mapping | For `DialPhase::AwaitingReply`, `abort_guest_flow` stores exactly `GuestDialError::Unreachable { errno: KernelErrno::ECONNRESET }` and cancels the dial token (`owner_runtime.rs:1102-1153`). |
| Dial result and cleanup | The biased cancellation branch returns the stored outcome through `cancellation_error`; ordinary error cleanup stale-aborts the watch, closes owner-held `V_h`, removes the dial, and releases its places before return (`dial.rs:261-271`, `:328-390`). |
| Conformance projection | `dial_result` preserves `ENODEV` and `ECONNRESET` explicitly and maps only every other `Unreachable` errno to `Unreachable:io` (`quint_owner_flows.rs:1012-1028`). |

The proposed value is therefore the real public result. The model cannot
remain generic without weakening the comparison, and the driver cannot infer
the cause from `conf_step` without becoming an action-sensitive normalizer.

## Typed error contract gate

The workspace `AGENTS.md` § *Typed error contract gate* requires an accepted
typed error to identify:

> “the semantic error set and pin its representation at the relevant
> interface: the variants, payloads, and mappings callers rely on.”

The proposal satisfies that gate rather than bypassing it:

| Semantic condition | Exact interface representation | Mapping source | Assessment |
|---|---|---|---|
| Guest sends payload-free `Abort { flow }` while matching host dial is `AwaitingReply` | `GuestDialError::Unreachable { errno: KernelErrno::ECONNRESET }` | Fixed owner mapping at `abort_guest_flow`; no errno exists on the wire | Complete and exact. |
| Host vsock connect with no VMM device | `Unreachable { ENODEV }` | Host connect errno | Preserved distinctly. |
| VMM/device release resets an in-flight dial | `Unreachable { ECONNRESET }` | Host socket reset | Preserved; action identity remains `VmStop`, not `HostRecvAbort`. |
| Other pre-pair host connect/watch/write I/O | `Unreachable { exact errno }`; formal projection `Unreachable:io` unless separately modelled | Host operation errno | Preserved; no normalization or erasure. |
| Guest application refusal | `Refused { guest errno }` | `Refused` wire payload | Preserved as a different variant. |
| Guest connect timeout | `TimedOut { ETIMEDOUT }` | `TimedOut` wire payload | Preserved as a different variant. |
| Pairing deadline | `PairingDeadline` | Owner deadline | Preserved. |
| Teardown / first quiescence / observed session loss | `NotServing` / `Quiesced` / `NoSession` | Owning lifecycle cause | Preserved. |

The fixed reset value is not represented as if it came from the guest's
private error or a reserved wire field. It is an explicit semantic mapping by
the host owner for a payload-free peer abort. The existing
`Unreachable { KernelErrno }` variant and payload are sufficient; no generic
string, arbitrary existing variant, new variant, or new payload is needed.

The proposal must update the interface documentation because the current
field comment says “the host kernel's errno.” After approval, the exact meaning
is: a host operation's errno where one exists, and the owner-fixed
`ECONNRESET` mapping for payload-free pre-Paired peer `Abort`. This is a
semantic documentation correction, not a Rust API-shape or behavior change.

## Exact formal-action review

The only model transition change is inside existing `HostRecvAbort(h)` when
`hf(h).st == "req"` and `dl.res(h) == "pending"`:

```quint
dl' = dlEndOne(h, "Unreachable:ECONNRESET")
```

`dlEndOne` already limits the write to an open requested/paired host flow whose
dial result remains pending. The action keeps its exact causal identity in
`conf_step`. Every other effect remains the existing one: consume the inbound
message, close the open host flow, preserve guest/U-5 state, and leave any
already-fixed result unchanged.

The existing named run should assert the exact result after the same action.
That is an honest strengthening of its current oracle, not a new scenario or
test body. The conformance driver already projects the returned reset value and
requires no source change.

### Complete phase audit

| `HostRecvAbort` state | Typed-result effect | Other effect | Assessment |
|---|---|---|---|
| `AwaitingReply` / model `req`, result pending | Fix `Unreachable:ECONNRESET`. | Close host dial, consume message; production cancellation performs existing socket/place cleanup. | Exact reproduced branch. |
| Dial result already fixed/host flow closed | No result change. | Discard late `Abort` under U-5. | Preserved. |
| `Paired`, dial result already `Ok` | No result change. | Existing paired watch/stream abort path; completed dial remains successful. | Preserved. |
| Unknown flow | No result exists or changes. | Existing owner discard. | Preserved. |

The amendment does not turn every `ECONNRESET` into the same formal cause.
`conf_step` remains `HostRecvAbort` here and `VmStop` for device release. The
public error type exposes variant and errno, not an additional cause field; the
model retains more causal detail through its action trace without inventing
public surface.

## No other I/O erasure or surface growth

The correction is deliberately action-specific. `HostAbortAccept`, which
represents a separate host-side V_h I/O failure, remains `Unreachable:io`
unless that action independently observes and models a concrete errno.
`DialUnreachable` remains `ENODEV`, and VMM exit remains `ECONNRESET`.
Changing only `HostRecvAbort` cannot normalize or erase those paths.

The rejected alternatives are correctly excluded:

- driver-wide normalization would erase legitimate errno distinctions;
- action-sensitive driver relabelling would compare against a harness fiction;
- adding errno to `Abort` would change the wire without an available source
  value;
- reusing guest `ErrorKind` would fabricate a carrier;
- `Refused`, `NoSession`, and `PairingDeadline` name different semantic
  conditions; and
- a new `PeerAborted` variant adds API/caller fallout when current production
  and the existing typed family already provide a total mapping.

No public type, enum variant, method, parameter, wire field, persisted format,
owner, lifecycle gate, state, action, scenario, mechanism, seam, hazard, lane,
or check is added. Production code and runtime behavior already implement the
proposed result.

## Cleanup and retained-contract audit

| Contract | Result |
|---|---|
| DG-5-E | Preserved. Guest accepted/unattached error closes the guest flow and sends one existing `Abort`. |
| U-1 / U-5 | Preserved. Closed guest flow cannot pair; late control for a closed host flow does not replace its result. |
| K-A4 | Preserved. Pending-dial cancellation cleanup closes owner-held `V_h`, which tears down V_g, and releases the slot/quota place. |
| INB-1 typed result | Strengthened to the exact production payload using the existing variant. |
| ENODEV and other I/O values | Preserved distinctly by the production return and conformance projection. |
| VMM-exit reset | Preserved as the same public variant/payload with a different action path. |
| Wire compatibility | Preserved. `Abort` remains type 3 with zero detail/errno and the same eight-byte shape. |

The raw named run proves the result and main cleanup state through the real
owner. This review does not authorize any change to control-message emission
or other message-side behavior beyond the proposed typed-result mapping. Any
separate claim about an additional or missing wire write would require its own
production-path reproduction under the workspace reachability gate; it is not
inferred from this typed-result mismatch.

No accepted `zero`, `never`, `must`, `exactly`, fail-closed, forbidden,
required, security, ownership, or cleanup outcome is weakened. The change
replaces a generic model label with the stronger observable result.

## Scope and cost

| Cost class | Bound |
|---|---|
| Production runtime / hot path | Zero. Current production already returns this value. |
| API shape / callers | Zero. Existing variant and payload only; documentation becomes exact. |
| Wire / storage / migration | Zero. No format change. |
| Formal state space | No cardinality growth. `Unreachable:ECONNRESET` already exists in `Dl.res` for `VmStop`; one existing transition selects it instead of `Unreachable:io`. |
| Check/lane count | Zero growth. Tighten one existing named-run expectation; retain all existing checks, randomized targets, jobs, and lanes. |
| Evidence | Exact seed replay plus the already-required full three-job record and independent review. |

The proposal's failure, alternatives, and costs are sufficiently bounded for a
single user decision. There is no adjacent error-taxonomy cleanup or generalized
errno work in scope.

## Blocking authority gate

Explicit user approval is required. `.claude/rules/design.md` § *Surface
material design decisions to the user before they become accepted* says:

> “A design document is a record of a proposal, not evidence that the user has
> accepted it.” Before a material decision is marked accepted or carried into
> later waves, it must be surfaced and explicitly approved.

The rule specifically covers amendments to:

> “a public or internal action/port contract, including the meaning of an
> existing field or parameter.”

This proposal changes both named categories. It pins the public semantic
meaning of the existing `GuestDialError::Unreachable.errno` payload for a
payload-free peer abort, and it changes the internal formal result of
`HostRecvAbort`. Runtime behavior and API shape do not change, but that does
not remove the approval requirement for an amended interface meaning.

The current task authorizes technical review, not approval of this exact
mapping. Pending prose and reviewer agreement cannot substitute for the user's
decision. Under the rule's reviewer clause, the verdict must remain
`CHANGES_REQUESTED` until that authority is recorded.

## Exact reviewable user decision

The proposal is technically ready to present as one approve/reject decision:

### PABORT-ERR-1 — payload-free pre-Paired guest `Abort` mapping

Approve the following exact contract:

1. When the host receives canonical `ControlMessage::Abort { flow }` for the
   matching `DialPhase::AwaitingReply` dial whose result is not already fixed,
   the dial returns exactly
   `GuestDialError::Unreachable { errno: KernelErrno::ECONNRESET }`.
2. `ECONNRESET` is the owner-fixed semantic mapping for this payload-free peer
   abort. It is not claimed to be transported from the guest or observed from
   a reserved wire field.
3. Existing `HostRecvAbort(h)` assigns only
   `dlEndOne(h, "Unreachable:ECONNRESET")`; action identity, phases, message
   consumption, socket/place cleanup, and late-discard behavior remain.
4. `ENODEV`, VMM-exit `ECONNRESET`, and other host I/O errnos remain distinct;
   `HostAbortAccept` remains generic `Unreachable:io` unless separately pinned.
5. `Abort` wire encoding remains payload-free; no new variant, payload, API,
   state, action, scenario, mechanism, or driver normalization is added.
6. The public error documentation records the fixed mapping alongside actual
   host-operation errnos. Runtime, API-shape, wire, storage, state-space, and
   check/lane costs are zero.
7. The existing `AcceptedVgErrorAbortsWithAbort` expectation is tightened to
   the exact payload. Seed `0x3c09cb16`, unchanged named/randomized
   `owner_flows` targets, the complete three-job guest-flow-owner record,
   `verify-evidence`, and independent replacement-evidence review remain
   mandatory before roadmap validation.

The affected boundaries are the public typed-error meaning, the
`HostRecvAbort` formal result, its existing named-run oracle, README mapping,
and documentation. No production implementation or caller change is required.

## Roadmap and next-gate disposition

Roadmap validation correctly remains `pending` with `approved_at = null`.
Before any model/DISTILL implementation:

1. the orchestrator must present PABORT-ERR-1 and obtain an explicit
   approve/reject response;
2. on approval, the feature delta must record the exact authority and date in
   present-tense typed-contract text; and
3. a bounded post-approval re-review must verify that the accepted fold equals
   PABORT-ERR-1 before implementation is dispatched, consistent with the
   repository's approval-then-review sequence.

After those authority gates, the bounded formal/documentation correction still
requires the exact seed replay, all unchanged named/randomized owner-flow
targets, complete guest-flow-owner re-record with exactly three jobs,
`verify-evidence`, and independent proof/source review. The historical record
cannot become current before those gates.

## Iteration 1 verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** The proposal is technically
complete, action-specific, and consistent with the actual production owner,
payload-free wire, existing public variant, conformance projection, and cleanup
path. It preserves legitimate errno distinctions and introduces no API, wire,
state, action, scenario, mechanism, or runtime change. There are zero open
technical findings.

The exact mapping is approved for presentation as PABORT-ERR-1, not accepted
into the architecture and not implementation authority. It changes a public
typed payload meaning and an internal formal action contract, so explicit user
approval is mandatory. The roadmap remains pending and STEP 12-01 remains
non-executable. No model, driver, production/test source, DES state, evidence,
roadmap approval, or commit was changed by this review.

---

# Review iteration 2 — user authority and canonical fold

## Authority reconciliation

The decision sequence is explicit in the canonical authority table. The user
first said “Approve 240-second exception,” which the immediately preceding
`FLOWQUOTA240` row limits to that timeout decision. After the remaining
reviewed decisions were presented, the user said “i approve the rest of the
decisions.” The 2026-10-26 authority row names `PABORTERR1` among those
remaining decisions and maps it to the exact `PABORT-ERR-1` reviewed here.
Authority is no longer inferred or pending.

## Canonical fold comparison

The accepted `PABORTERR1` clause in `feature-delta.md` under the host-opened
flow outcome contract matches the reviewed proposal:

| Reviewed contract | Accepted fold | Result |
|---|---|---|
| Matching payload-free guest `Abort { flow }` in `DialPhase::AwaitingReply`, before a result is fixed | Same exact selector | Exact |
| `GuestDialError::Unreachable { errno: KernelErrno::ECONNRESET }` | Same public variant and fixed payload | Exact |
| `ECONNRESET` is semantic because the wire carries no errno | Same rationale; no fabricated carrier or reserved field | Exact |
| Existing `HostRecvAbort(h)` records `dlEndOne(h, "Unreachable:ECONNRESET")` | Same existing action and effect | Exact |
| `ENODEV`, VMM-exit reset, other host I/O, and `HostAbortAccept` remain distinct | Same retained branches, including generic `Unreachable:io` for `HostAbortAccept` | Exact |
| No wire, variant, payload shape, API, state, action, scenario, runtime, caller, or cleanup mechanism change | Canonical clause expressly excludes these changes | Exact |

The fold preserves the accepted cleanup path: every dial error closes V_h and
releases the slot and applicable relay quota before returning. It does not
erase a legitimate host I/O errno or recast an owner-initiated lifecycle end as
`Unreachable`.

## Evidence disposition

Roadmap validation remains `pending` with `approved_at = null`. Approval and
this fold review do not replace exact seed `0x3c09cb16`, every unchanged
named/randomized owner-flow target, the complete three-job guest-flow-owner
record, `verify-evidence`, and independent proof/source review. This iteration
changes only this review artifact.

## Iteration 2 verdict

**APPROVED.** User authority for `PABORTERR1` is exact and dated 2026-10-26,
the canonical fold matches the zero-finding `PABORT-ERR-1` typed mapping, and
no broader API, wire, state, action, error taxonomy, caller, or mechanism was
introduced. The prior `USER_DECISION_REQUIRED` disposition is closed. Roadmap
validation remains pending replacement evidence.
