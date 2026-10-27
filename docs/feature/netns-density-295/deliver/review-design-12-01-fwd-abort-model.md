# Independent DESIGN/formal review — STEP 12-01 `fwd_pair` abort/handoff reconciliation

## Metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_fwd_abort_handoff` |
| Date | 2026-10-26 |
| Role | Independent solution architecture / formal-model reviewer |
| Iteration | 1 |
| Reviewed proposal | `docs/feature/netns-density-295/deliver/design-12-01-fwd-abort-model.md` |
| Verdict | **CHANGES_REQUESTED — one blocking specification ambiguity** |
| Blocking findings | 1 |
| Roadmap disposition | **PENDING / NON-EXECUTABLE**; `approved_at = null` |
| Fold eligibility | **Not yet eligible**; F1 must be resolved and independently re-reviewed before the proposal is folded |

This review applies the repository DESIGN, production-reachability, native
falsification, exact-contract, and no-weakening rules together with the
`nw-design`, `nw-formal-verification-tlaplus`,
`nw-sar-critique-dimensions`, and `nw-roadmap-review-checks` instructions. The
repository's required native-Markdown review artifact and unlimited
remediation cycle govern over the review agent definition's legacy YAML and
two-iteration defaults.

The review is bounded to the proposed formal-abstraction correction, the dirty
pending annotations in the feature delta and roadmap, the actual `fwd_pair`
model and host/guest conformance drivers, the accepted R5-53/R5-63 and
EVMOD-STREAM-ABORT/WATCH-ABORT contracts, and the complete current production
caller path. It does not edit the proposal, canonical design, roadmap, model,
driver, tests, DES state, or evidence. It runs no model, Metal, test, build, or
mutation command and creates no commit.

## Reproduced premise and production-path assessment

The proposal correctly identifies a production-reachable abstraction error.
The host conformance driver's `InitWith("watch")` starts the composition's
actual `OwnerDial::dial`, retains the join handle in `dial`, and settles until
the request and watch exist while `stream` remains `None`
(`quint_fwd_pair.rs:560-618`). Production `run_dial` owns `V_h`, installs the
watch, writes the request, and waits for the reply; it moves `V_h` into a
`GuestStream` only after a matching `Paired` and the cancellation re-check
(`dial.rs:189-311`). `DialPhase::AwaitingReply` and `DialPhase::Paired` name
that exact ownership cut, and only the latter says the flow is held by its
`GuestStream` (`flow_owner.rs:495-513`).

`Abort("h")` drives the real `HostSharedGuestNetworkOwner::quiesce`. Its first
hold records `Quiesced` for each `AwaitingReply` dial, cancels every dial,
clears the owner tables, and calls the real `abort_all`
(`guest_network.rs:1249-1327`). The biased cancellation branch in `run_dial`
wins before a later reply, writes `Abort { flow }` after the request, aborts
the watch, closes the still-owner-held `V_h`, and removes the dial
(`dial.rs:247-277`, `:328-348`). This proves the seed `0xea0ef1a4`, Trace 11,
is not a test-only state and that production takes R5-53's pending-dial
outcome, not R5-63's held-stream outcome.

The current model instead creates a live host watch at `InitWith("watch")`
while `strmAbort = false`, enables `Abort("h")` from watch liveness alone, and
unconditionally applies the held-stream effect (`fwd_pair.qnt:475-494`,
`:571-585`). The recorded model premise is therefore falsified exactly as the
proposal says. The accepted production contracts are not falsified.

## Correct parts of the proposed reconciliation

| Concern | Assessment |
|---|---|
| Ownership discriminator | **Pass.** `strmHeld` represents the actual move-only `GuestStream` handoff, not watch liveness, guest attachment, owner-table absence, or a driver shadow flag. |
| Host projection | **Pass.** The proposed source is the real driver-owned object, `self.stream.is_some()`. The guest driver does not own that host object and correctly treats the variable as model-only. |
| Pre-handoff `Abort("h")` | **Pass.** Keeping the action enabled and closing owner-held `V_h` while leaving `strmAbort = false` matches R5-53 and the reproduced production path. |
| Post-handoff `Abort("h")` | **Pass.** Queueing the watch record, leaving holder-owned `V_h` and its queues unchanged, setting `strmAbort`, waking stream calls, and sending `Abort { flow }` preserves R5-63(a), WATCH-ABORT, and EVMOD-STREAM-ABORT. |
| Late `Paired` | **Pass as a required outcome.** Action order decides ownership: `Attach("g")` before abort may hand out the stream; abort before `Attach("g")` cancels the dial, and the late `Paired` cannot create a stream. This follows the biased production cancellation check and does not infer ownership from the watch. |
| Scope | **Pass.** One model fact and bounded private driver projection/action corrections are sufficient. No public API, production mechanism, new action, named run, scenario, hazard, lane, or filter is justified. |
| Evidence invalidation | **Pass.** The existing guest-flow-owner record cannot validate the falsified handoff premise. The exact seed replay and complete three-job re-record plus `verify-evidence` are mandatory after any approved model-input change. |

## F1 — blocking `SPECIFICATION_AMBIGUITY`: holder-free successor states are not closed

**Severity:** High / blocking.
**Location:** proposal §§ *Represent the existing ownership cut directly*,
*Preserve real late-message outcomes with existing actions*, and *Tighten
existing properties without adding a scenario* (`design-12-01-fwd-abort-model.md:73-163`).
**Status:** Open.

The proposal pins the new ownership fact and the two `Abort("h")` post-states,
but it does not pin every existing action whose enabledness or effect currently
assumes that a live or once-attached watch implies a host stream. The omission
leaves a production-impossible holder action reachable in the corrected model
and leaves two reachable ordering branches without an exact model outcome.

### Concrete reachable counterexample to the incomplete action contract

The following sequence is enabled by the proposal and by the existing action
set:

```text
InitWith("watch") -> Abort("h") -> Attach("g")
```

After the proposed pre-handoff abort, `strmHeld = false`, the host watch is
ended with one queued `Detached` record, and `V_h` has sent `{R,S}`. The late
guest attach can still succeed locally: existing `canAttach(g)` permits a host
watch whose phase is `ended`, so `attRes(g) = ok` and the guest pair becomes
live while the host discards `Paired`. This is the accepted R5-28 ordering: the
guest can hold its pair while the host has no stream.

In that state the current `holderReadable` does not ask whether a stream
exists. It sees watch mode, `attRes(g) = ok`, an alive dialer endpoint, and
`vs(h).sent` containing `R`, so it enables `HolderEnd("eof" | "err")`
(`fwd_pair.qnt:917-929`). The host driver then reads the retained Sim socket
directly without checking `self.stream` (`quint_fwd_pair.rs:753-780`). The
following `ReadEof("gh")` is enabled from that fabricated holder observation,
but the production driver fails it because `self.stream` is `None`
(`quint_fwd_pair.rs:908-921`). Adding `strmHeld` without closing these guards
therefore preserves a model trace that invents a holder from socket/watch
facts, precisely what the proposal forbids.

The same ownership dependency exists in the watch-mode paths for `Write`,
`Fin`, `Read`, `HolderEnd`, and `ReadEof`; their current guards use guest attach,
`strmAbort`, and socket state rather than the actual stream
(`fwd_pair.qnt:865-959`). The design must state which are holder-only and make
their enabledness depend on `strmHeld`. A conformance driver's eventual
`NotPlayable` error is not a model contract and cannot supply the missing
guard.

### Recovery transition left underspecified

`recoveryClose` currently sets `strmAbort = true` whenever the host watch is
already ended, the dialer endpoint is alive, and `strmAbort` is false
(`fwd_pair.qnt:601-609`). A watch can end before handoff; production then
cancels and removes the pending dial and has no stream to mark aborted
(`owner_runtime.rs:939-993`). Merely changing the
`RecoveryEndsHeldStream` invariant's premise does not change this transition.
The proposal also requires `strmAbort => strmHeld`, so the existing transition
would contradict its own new invariant in a reachable holder-free state.

The exact `CtxRelease("h")` / `recoveryClose` effect must be pinned for both
`strmHeld` values: only an actually held stream may acquire `strmAbort`; an
ended pre-handoff dial must remain holder-free and unmarked. This is an action
contract, not a property-only edit.

### Missing exact outcomes for the late-attach orderings

`Abort("h")` before `Attach("g")` creates two distinct guest-side orders that
the accepted R5-28 contract preserves:

1. `GuestAbort("g")` arrives while the guest flow is still unattached: the
   message is discarded and changes no guest pair; a later guest attach can
   still occur and ends through the already-closed `V_h`.
2. The guest attaches first, so its pair is live while `strmHeld = false`:
   `GuestAbort("g")` must abort that guest pair even though the host never
   received a stream.

The proposal states the first outcome and the ordinary `strmHeld = true`
outcome, but it does not expressly pin the second holder-free/live-guest-pair
branch. `strmHeld` must not become a proxy for guest pair ownership. The guest
action remains selected by the actual `pr(g)` phase.

`HolderDrop` has the analogous third state. The proposal defines an actual
stream drop (`strmHeld = true`) and cancellation while the watch is pending and
live (`strmHeld = false`), but not `strmHeld = false` after the pending dial has
already ended. That state is reachable immediately after the new pre-handoff
abort while the driver still retains the completed dial join handle. The
design must say whether `HolderDrop` is disabled there or consumes the
completed dial future with an inert model effect; it must not set the
held-stream `gh.dropped` ghost.

### Required bounded remediation

The DESIGN owner must complete the existing-action transition contract without
adding an action, scenario, model filter, driver shadow state, or production
surface:

- pin `strmHeld` as the enabledness premise for every action that represents a
  call or observation through an actual host `GuestStream`;
- pin `recoveryClose` for held and non-held ended watches so `strmAbort` is
  produced only for an actual held stream;
- pin `GuestAbort("g")` by the guest pair's actual phase for all three relevant
  orders, including a live guest pair with `strmHeld = false`;
- pin `HolderDrop` for `strmHeld = false` after the dial has already ended; and
- state that the host compares `strmHeld` from `self.stream.is_some()` as a
  top-level holder fact even when no owner projection remains, never from
  watch liveness, `attRes`, a dial-table absence, or candidate/action order.

This remediation is formal-model and private-driver contract text only. It
does not authorize an implementation, test edit, new named run, public API, or
production mechanism. The existing seed supplies the pre-handoff replay, and
the existing held-stream runs supply the post-handoff regression evidence.

## Contract and no-weakening audit

The proposal's intended two-way split preserves the accepted outcomes:

- R5-53 still returns `Quiesced` for a pending dial, aborts its watch, closes
  owner-held `V_h`, sends `Abort { flow }` while the session is live, and
  releases the node/quota places;
- R5-63(a), WATCH-ABORT, and EVMOD-STREAM-ABORT still leave holder-owned `V_h`
  unchanged, mark the actual stream owner-aborted, wake pending calls with
  `ConnectionAborted`, send `Abort { flow }`, and make the later stream drop
  close `V_h` without a second watch effect; and
- R5-28's two message orders remain distinct: an early unknown `Abort` is a
  discard, while an `Abort` after guest pairing ends that pair.

No typed error representation, public/cross-crate signature, wire or persisted
format, lifecycle owner, security boundary, or production behavior needs to
change. The pending canonical annotations correctly avoid treating the old
279-check record as current authority for this premise. No `zero`, `never`,
`must`, `exactly`, fail-closed, forbidden, or required outcome is removed or
relaxed by the intended correction. F1 blocks because an incomplete action
contract could still weaken model honesty, not because the two proposed abort
outcomes are wrong.

## Architecture dimensions and bounded roadmap checks

| Review dimension | Result |
|---|---|
| Bias and scope | Pass. The proposal corrects one falsified abstraction and adds no mechanism or technology. |
| Decision quality | **Blocked by F1.** Context, production proof, alternatives, scope, and consequences are sound, but existing-action semantics are incomplete. |
| Completeness | **Blocked by F1.** The ownership state is defined; its guard/effect closure is not. |
| Feasibility and testability | Pass in principle. The actual `GuestStream` field is available to the private host driver, and the exact seed plus existing named runs exercise both cuts after implementation. |
| Priority and simplicity | Pass. One ownership fact is the smallest correct abstraction; deriving it from watch or attach state would repeat the falsified premise. |

| Roadmap check | Bounded result |
|---|---|
| External validity | Unchanged. This correction repairs existing conformance against the real owner; it adds no product invocation path. |
| Acceptance-criterion coupling | No roadmap criterion is added or rewritten. Exact model action semantics belong to DESIGN and do not dictate private Rust structure. |
| Decomposition | No new roadmap step is justified. The bounded correction remains within STEP 12-01's formal/conformance prerequisite. |
| Implementation code | Pass. The proposal records state/effects rather than a Rust implementation. |
| Concision and precision | **Blocked only by F1's omitted branches.** The proposed core split is otherwise precise. |
| Test boundary | Pass. Existing randomized host/guest conformance and named runs remain the evidence; no production test seam or new test body is proposed. |
| Mechanical status | Pass. `roadmap.json` parses; validation is `pending` and `approved_at` is null. Scoped canonical/proposal `git diff --check` passes. |

## Iteration 1 verdict

**CHANGES_REQUESTED — `SPECIFICATION_AMBIGUITY`.** The reproduced seed proves
that `fwd_pair` must distinguish a pending dial's owner-held `V_h` from an
actual holder-owned `GuestStream`, and the proposed `strmHeld` fact projected
from `self.stream.is_some()` is the correct bounded discriminator. The two
`Abort("h")` outcomes preserve R5-53 and R5-63 without new product surface or a
contract weakening.

The proposal is not yet fold-eligible because it does not close the enabledness
and effects of existing holder actions, recovery, late `GuestAbort`, and
post-end `HolderDrop` across all reachable `strmHeld`/guest-pair combinations.
No model, conformance driver, README/feature-delta formal mapping, recorded
evidence, roadmap approval, DES event, or production/test source change may
proceed from iteration 1. After the bounded design text pins those branches, a
fresh re-review may approve the reconciliation; only then may the model input
change and the mandatory exact-seed replay, three-job record, and evidence
verification run.

---

# Review iteration 2 — complete existing-action closure

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_fwd_abort_handoff` |
| Date | 2026-10-26 |
| Role | Same independent solution architecture / formal-model reviewer |
| Reviewed scope | The proposal's bounded F1 remediation and its updated pending canonical annotation |
| Verdict | **APPROVED** |
| F1 disposition | **CLOSED** |
| Open findings | 0 |
| Fold eligibility | **ELIGIBLE** — the exact reviewed formal contract may be folded into its canonical mappings |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** until implementation, replacement evidence, and independent evidence review complete |

This iteration re-reviewed every branch named by F1 against the same actual
model, host and guest drivers, accepted contracts, and production path. It did
not reopen unrelated architecture or implementation work. It appends only this
review iteration and does not edit the proposal, canonical design, roadmap,
model, driver, production source, tests, DES state, or evidence. It runs no
model, Metal, test, build, mutation, or evidence-record command and creates no
commit.

## F1 remediation disposition

| Iteration-1 requirement | Remediated contract | Assessment |
|---|---|---|
| Project the actual holder fact at the correct boundary | `strmHeld` is top-level beside `hres`, compares `self.stream.is_some()` even after the owner projection is absent, and forbids a shadow flag or inference from dial-table/watch state (`design-12-01-fwd-abort-model.md:83-90`). | **Closed.** The driver compares the move-only object it actually owns. Owner death cannot erase or manufacture the holder fact. |
| Pin the exact handoff | Only watch-mode `Attach("g")` with a pre-state live host watch and successful existing `attachEff` may set the model fact; the host driver cross-checks that result against an actual `Ok(GuestStream)` (`:92-125`). | **Closed.** A refused attach and a late `Paired` cannot hand out a stream. Action/candidate order is not inferred from watch existence. |
| Gate every holder-only action | Watch-mode `Write("hg")`, `Fin("hg")`, `Read("gh")`, `HolderEnd`, `holderReadable`, and `ReadEof("gh")` all require `strmHeld`; the driver additionally requires `self.stream.is_some()` before `HolderEnd` touches the Sim socket (`:142-162`). | **Closed.** The iteration-1 `Abort -> late Attach -> HolderEnd -> ReadEof` trace is disabled at the first holder-only action while guest/vsock cleanup actions remain available. |
| Pin late attach and guest-message orders without conflating owners | A late guest attach may create a local guest pair while the fixed dial result remains `Quiesced` and `strmHeld` remains false. `GuestAbort("g")` branches on `pr(g).ph`: live always aborts the guest pair; unattached, ended, and refused discard in watch mode; every branch clears the message (`:164-202`). | **Closed.** A live guest pair with no host stream is represented directly. Both R5-28 orders remain possible and retain their distinct outcomes. |
| Constrain recovery itself | `recoveryClose` sets `strmAbort` only for an actually held stream whose ended watch was not already owner-aborted; holder-free recovery adds no stream, message, or socket effect (`:204-216`). | **Closed.** The transition now establishes `strmAbort => strmHeld`; the property no longer attempts to repair an incorrect transition after the fact. |
| Pin every valid `HolderDrop` state | Actual stream drop keeps T-6 and clears `strmHeld` with `strmAbort`; holder-free/live-watch drop is pending-dial cancellation with no `gh.dropped`; holder-free/ended-watch `HolderDrop` is disabled by the exact `strmHeld || live(cur, "h")` guard (`:218-235`). | **Closed.** A completed private join handle produces no protocol transition and cannot masquerade as a stream drop. |

The remediation also records the reachability and necessity of each clause at
proposal lines 258-285. These are the same concrete paths established in
iteration 1: none depends on a fabricated test-only state, theoretical
cancellation, or a model seam. The explicit rejection of another phase,
action, named run, scenario, driver flag, or production correction is correct:
the existing object, actions, randomized seed, held-stream runs, and
`owner_flows` R5-28 runs already own the relevant evidence.

## Complete transition and invariant review

The revised action contract now preserves the distinctions that matter:

| Reachable state/order | Exact outcome | Review |
|---|---|---|
| Pending dial; host abort first | Watch ends once, owner-held `V_h` closes with `{R,S}`, dial returns `Quiesced`, `guestAbortSent = vmUp`, `strmHeld = false`, `strmAbort = false`. | Matches R5-53 and reproduced Trace 11. |
| Guest attaches after pending-dial abort | Guest pair may become live; late `Paired` is discarded; fixed dial error is consumed; no host stream appears. | Matches R5-28 and the biased production cancellation order. |
| Guest receives `Abort` before local attach | Watch-mode unattached guest phase is unchanged and the message clears; the already-closed `V_h` remains the guest cleanup path if attach follows. | Matches U-5/R5-28 unknown-flow discard. |
| Guest receives `Abort` after local attach but host never got a stream | The live guest pair aborts despite `strmHeld = false`. | Correctly selects guest ownership from `pr(g).ph`, not the host fact. |
| Guest attaches before host abort | Actual future returns `Ok(GuestStream)` and `strmHeld = true`; owner abort queues the watch record, leaves `V_h` untouched, sets `strmAbort`, wakes calls, and sends `Abort`. | Preserves R5-63(a), WATCH-ABORT, and EVMOD-STREAM-ABORT. |
| Ended watch with a held stream enters recovery | Recovery marks that stream owner-aborted, preserves an already reported `hres`, and writes the accepted `Abort` while live. | Preserves U-R1 and `RecoveryEndsHeldStreamWhoseWatchEnded`. |
| Ended watch with no held stream enters recovery | No stream fact, holder outcome, control message, or socket effect is fabricated. | Matches the completed pending-dial cleanup path. |
| Actual holder drops | Still-live watch aborts, `V_h` closes, `gh.dropped` records T-6, and both current-holder facts clear; an already-ended watch adds only the close. | Preserves T-6 and the later-drop half of R5-63(a). |
| No holder and live watch drops | The pending dial cancels, its watch aborts, owner-held `V_h` closes, and the real `Abort` outcome is retained without setting `gh.dropped`. | Matches `DialCancelGuard`/`run_dial` cleanup and R5-53 cancellation semantics. |
| No holder and ended watch | `HolderDrop` is disabled; disposal of an already-completed private join handle has no modeled protocol effect. | Removes driver-bookkeeping stutter without filtering a product action. |

The revised properties are coherent with those transitions.
`WatchAbortLeavesHolderSocket` still checks the held-stream branch and gains
the ownership implication; every holder call separately requires `strmHeld`;
the recovery property constrains the actual transition; and `gh.dropped`
remains evidence of an actual `GuestStream` drop. Clearing `strmAbort` together
with `strmHeld` at object drop does not weaken
`GuestPairEndsAfterOwnerAbort`: the temporal obligation is created in the
state where `strmAbort` is true, while the accepted guest `Abort` or later
`V_h` close still drives the pair to its terminal state.

No model action or environment fault is filtered from the corrected
pre-handoff state. `Abort("h")` remains enabled before handoff, late
`Attach("g")` remains enabled, both guest-message orders remain enabled, and
ordinary guest/vsock end actions remain available. Only operations whose
production subject is an actual `GuestStream` require the actual holder fact.

## Contract, scope, and no-weakening re-review

| Contract | Result |
|---|---|
| R5-53 pending dial | Preserved exactly: typed `Quiesced`, watch abort, owner-held `V_h` close, place release, and live-session `Abort`. |
| R5-63 / WATCH-ABORT | Preserved exactly for an actual stream: holder-owned `V_h` is untouched by owner abort; calls return `ConnectionAborted`; later drop closes it once. |
| R5-28 / U-5 / K-A4 | Preserved: late `Paired` and unknown/already-ended `Abort` are discarded at their owners, while a locally live guest pair is still ended and host close remains the teardown complement. |
| U-R1 recovery | Preserved only where its subject exists: an independently held stream whose watch ended. No pre-handoff stream is invented. |
| Public and production surface | Unchanged. No public/cross-crate type, method, enum variant, error, parameter, config/CLI field, wire/persisted format, owner, lifecycle gate, seam, or production mechanism is added. |
| Formal/test surface | Same actions, named runs, scenarios, hazards, lanes, and check inventory. One model ownership fact and bounded private projection/action mappings change. |

The revision does not remove or relax an accepted `zero`, `never`, `must`,
`exactly`, fail-closed, forbidden, security, cleanup, or ownership outcome.
The split makes both existing outcomes representable instead of weakening one
to fit the other. There is no typed-error contract gap: `GuestDialError::Quiesced`
is already pinned and unchanged, and no new semantic failure is introduced.

No further user decision is required for this bounded correction. The review
approves only an exact formal representation of already-approved R5-28,
R5-53, R5-63, U-5, K-A4, WATCH-ABORT, and U-R1 behavior. Any different product
outcome, new action/scenario, or added production/test surface remains outside
this approval and requires its own authority.

## Fold, roadmap, and evidence disposition

The proposal is now eligible to be folded into the authoritative formal-model
mapping in the feature delta and the `guest-flow-owner` README. That fold may
record the approved `strmHeld` meaning, action guards/effects, invariant
premises, projection source, and unchanged check inventory. It must not mark
the old 279-check record current or claim that the model and drivers already
implement the correction.

The roadmap correctly remains `validation.status = pending` with
`approved_at = null`. Approval of the design does not approve the roadmap or
make STEP 12-01 executable. Resumption remains ordered exactly as the proposal
pins:

1. fold the approved formal contract and dispatch the bounded
   model/conformance work;
2. implement only the spec, README/feature-delta mapping, and bounded private
   conformance-driver changes required by the approved existing-action
   contract; `strmHeld` remains model-only in the guest driver;
3. replay exact seed `0xea0ef1a4` through the real owner with no filtered
   action, fabricated state, or shadow phase;
4. re-record the complete guest-flow-owner formal inputs with exactly three
   jobs and run `verify-evidence`;
5. pass the unchanged named runs and randomized host/guest conformance; and
6. obtain independent review of the replacement record, both ownership cuts,
   all remediated branches, and the absence of API, scenario, filter, or shadow
   expansion before roadmap validation may return to approved.

Any actual model, driver, check configuration, or other formal input change
after this approval invalidates the historical record and requires that full
three-job record and verification evidence. No partial check or unchanged
summary can substitute for it.

## Iteration 2 verdict

**APPROVED.** F1 is closed with exact enabledness and effects for every
reachable existing action across the pre-/post-`GuestStream` handoff. The
reconciliation preserves the distinct R5-53 pending-dial and R5-63 held-stream
outcomes, both R5-28 message orders, WATCH-ABORT, U-R1, and all security and
cleanup invariants. It adds no public or production surface, model action,
named run, scenario, hazard, lane, trace filter, or shadow driver state.

The exact proposal is eligible for canonical fold and bounded formal/
conformance implementation. The roadmap remains pending and non-executable
until the corrected inputs are implemented, exact seed replayed, the complete
three-job evidence replaced and verified, and that record independently
approved. No model, driver, production/test source, DES state, evidence,
roadmap approval, or commit was changed by this review.

---

# Review iteration 3 — Trace-14 holder-free recovery replacement

## Iteration 3 metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_fwd_abort_handoff` |
| Date | 2026-10-26 |
| Role | Same independent solution architecture / formal-model reviewer |
| Reviewed scope | Trace-14 falsification, exact holder-free `recoveryClose` replacement, and pending canonical T-5/action-map clauses |
| Verdict | **APPROVED** |
| Superseded premise | Iteration 2's claim that every holder-free ended watch had already completed pending-dial socket cleanup |
| Open findings | 0 |
| Fold eligibility | **ELIGIBLE** — the exact Trace-14 replacement may supersede only the falsified holder-free recovery clause |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** until replacement replay, complete three-job evidence, and independent evidence review |

This iteration preserves iterations 1 and 2 as the historical review record.
It reopens only the later-falsified holder-free recovery effect. Every other
approved ownership, enabledness, message-order, holder-drop, invariant, and
surface clause was rechecked for fallout and remains approved.

The reviewer read the retained raw replay, the actual host conformance driver,
`HostSharedGuestNetworkOwner::quiesce`, `run_dial` cancellation and cleanup,
the end-record consumer, the current `fwd_pair` transition, and the pending
canonical projection/action/T-5 text. This iteration appends only this review.
It does not edit the proposal, canonical design, roadmap, model, driver,
production source, tests, DES state, or evidence; run a model, Metal, test,
build, mutation, or record command; or create a commit.

## Independently verified Trace-14 evidence

The retained log
`.context/netns-density-295/12-01/quint-fwd-pair-seed-ee26ec37-verbose1.log`
records the exact Metal invocation, nextest run
`07069ac6-5be7-47a5-9f05-26b0cc412ff8`, and `QUINT_SEED=0xee26ec37`. Trace 14
reaches:

```text
InitWith("watch")
  -> TcpRst("g", false)
  -> VmmExit
  -> FwdErr("h")
  -> CtxRelease("h")
```

The pre-`CtxRelease` state is explicit at raw-log lines 2066-2091:

- the host watch is `ended` with `out = aborted`, `cause = SocketError`,
  `rec = queued`, and `nrec = 1`;
- `strmHeld = false`, `strmAbort = false`, and `hres = none`;
- the host context remains open and the dialer endpoint remains alive;
- `V_h` has `err = true` but `sent = {}` and `sshut = false`; and
- `vmUp = false` and `guestAbortSent = false`.

After the real `CtxRelease("h")` mapping, the implementation comparison at
raw-log lines 2120-2159 retains every listed record, ownership, result, and
message fact. The only differing compared socket fields are
`vs(h).sent: {} -> {R,S}` and `vs(h).sshut: false -> true`. The run fails with
the exact reproduction seed at lines 2161-2171. This is direct evidence
against iteration 2's holder-free no-socket-effect premise; it is not evidence
against `strmHeld`, the held-stream branch, or production behavior.

## Production caller and owner proof

The raw transition follows the actual current caller path:

1. Watch-mode `InitWith` starts the production `GuestStreamDial::dial`, which
   installs the watch and writes the request while `run_dial` continues to own
   `V_h`; no `GuestStream` has been returned.
2. `VmmExit` drives `vm_stop_before_callbacks`, which makes the dialed record
   socket fail and sets host-vsock `sk_err`; `FwdErr("h")` drives the sweep's
   real `sk_error_report`, ending the watch and queueing its one
   `SocketError` record.
3. The driver's deliberate end-record scheduling leaves that record queued,
   so the owner has not yet consumed it. The absence of a live forwarder watch
   makes `CtxRelease("h")` choose the existing `Forwarder` recovery path rather
   than host-owner death.
4. Recovery calls the real owner `quiesce`. The dial remains
   `DialPhase::AwaitingReply`; quiesce records `Quiesced`, cancels it, clears
   the dial tables, and awaits the bounded cleanup path.
5. The cancelled `run_dial` leaves the existing terminal watch record alone,
   performs its existing idempotent stale-watch abort, and closes the
   still-owner-held `V_h`. With the VM/control session gone, no
   `Abort { flow }` is written.

The interleaving is production-reachable: record readiness and recovery are
concurrent owner inputs, and no production ordering says the end-record task
must consume the queued record before recovery quiescence cancels the pending
dial. The trace drives production owner calls and authentic Sim socket/
forwarder arbiters; it does not create another owner, test-only dial phase, or
synthetic stream.

## Replacement transition review

The proposed replacement is the smallest state delta matching that path. In
the existing watch-mode, ended-watch, alive-dialer recovery branch:

```quint
val cleanupPending = t.vs.get("h").sent != both
val closed = shutAll(t, "h")
{ ...closed,
  strmAbort: false,
  hres: "none",
  guestAbortSent: t.guestAbortSent or (cleanupPending and t.vmUp) }
```

`shutAll` is the already-modelled close of `V_h`. It unions the local sent
flags, sets `sshut`, and records the existing `{R,S}` control-packet complement;
it does not abort the already-ended watch, create an end record, consume or
reclassify the queued record, or manufacture a holder. Its existing
`sent == both` check makes the operation idempotent.

| Retained branch/state | Exact effect after the replacement | Assessment |
|---|---|---|
| Held stream, ended watch | Existing U-R1 branch sets `strmAbort`, retains `strmHeld`, preserves prior `hres`, and records the live-session `Abort`. It does not close holder-owned `V_h`. | **Unchanged and correct.** R5-63, WATCH-ABORT, and `RecoveryEndsHeldStreamWhoseWatchEnded` retain their exact outcome. |
| No holder, ended watch, owner-held `V_h` still open | Preserve `pr(h)` including record/cause/count; close `V_h`; keep `strmHeld = false`, `strmAbort = false`, `hres = none`; record the existing pending-dial guest `Abort` only while `vmUp`. | **Correct.** This is the Trace-14/R5-53 cleanup window. |
| No holder, ended watch, `V_h` already closed | `shutAll` changes nothing; preserve the existing `guestAbortSent` state and all record/result facts. | **Correct.** A completed pre-handoff abort or typed-error cleanup is not repeated. |
| No holder, ended watch, VM gone | Close local `V_h`; keep `guestAbortSent = false`. The close packet has no live guest action because side `g` is gone. | **Correct.** Matches the raw replay and adds no post-VM message. |
| No holder, ended watch, VM/session live | Close local `V_h` and set the already-approved pending-dial `Abort { flow }` fact exactly once when cleanup was pending. | **Correct.** Matches `run_dial` after a written request and preserves R5-28/R5-53. |
| No holder, live watch | `CtxRelease("h")` still selects host-owner death; `HolderDrop` remains the pending-dial cancellation action. | **Unchanged.** The replacement does not widen recovery to a live watch. |
| Holder dropped, no holder, ended watch | Existing `ep(h) = dead` excludes the alive-dialer recovery cleanup; `HolderDrop` remains disabled after its completed protocol effect. | **Unchanged.** The branch cannot close a former holder's socket twice. |
| Non-watch pair/record recovery | Existing queued-record close behavior remains separate. | **Unchanged.** The replacement is watch-specific. |

The close is not generalized from “ended watch” alone. It remains inside the
existing state-selected `CtxRelease("h")` recovery path, the existing
watch-mode ended-watch branch, and the existing alive dialer endpoint. Within
that branch, `strmHeld = false` proves no caller owns a `GuestStream`; the
unchanged handoff rule proves the only remaining open `V_h` is the pending
dial's owner-held socket. `vs(h).sent != both` selects incomplete close cleanup,
not ownership. No watch-liveness, attach-result, record class, or candidate
order is used as a substitute for `strmHeld`.

Preserving `SocketError/queued/nrec=1` is required. The watch ended before
recovery, and `shutAll` acts on the socket owner rather than the removed
forwarder id. Replacing the record with `Detached`, incrementing `nrec`,
marking it consumed, or setting `strmAbort` would contradict the raw owner and
forwarder facts. The existing D-4 projection of `queued_ends` in released
contexts remains the arbiter; no driver's copy supplies the record.

## Retained contract and invariant audit

| Contract or invariant | Result |
|---|---|
| R5-53 pending-dial cancellation | Preserved. The still-owner-held socket closes and the fixed `Quiesced` cleanup remains owned by `owner_flows`; the model adds no new error. |
| R5-63 / WATCH-ABORT | Preserved. Only `strmHeld = true` identifies holder-owned `V_h`, and recovery still leaves that socket untouched. |
| R5-28 / U-5 / K-A4 | Preserved. Live-session pending cleanup records the existing guest `Abort`; VM-gone cleanup records none; late message and host-close orders are unchanged. |
| U-R1 held-stream recovery | Preserved. The true branch is byte-for-byte the approved semantic outcome and retains an already returned `hres`. |
| One end record and exact class | Preserved. `SocketError`, `queued`, and `nrec = 1` survive holder-free cleanup; no second record or `Detached` reclassification is introduced. |
| Holder ownership | Preserved. `strmHeld` stays false throughout the new branch; `strmAbort => strmHeld` remains true; `hres` remains `none`. |
| `WatchAbortLeavesHolderSocket` | Preserved. The changed socket is owner-held. `gh.vhTouched` remains false and still detects mutation only in the held-stream branch. |
| Holder-only actions and `HolderDrop` | Preserved exactly from iteration 2. No holder operation becomes enabled, and the terminal no-holder/no-live-watch drop remains disabled. |
| Security and cleanup | Preserved. Recovery converges on no open pending-dial socket; no guest message is emitted after VM loss, and no live guest is left without the accepted `Abort` or close complement. |

No public/cross-crate type, trait method, enum variant, typed error, parameter,
owner, lifecycle gate, wire or persisted format, config/CLI field, production
mechanism, test seam, model state, action, named run, scenario, hazard, lane,
or check is added. The exact existing socket and VM facts are sufficient.

The normative audit finds no relaxation of `zero`, `never`, `must`, `exactly`,
fail-closed, forbidden, required, security, ownership, or cleanup language.
The replacement removes a false no-effect premise and restores the required
pending-dial close; it does not permit a previously forbidden state. No new
user decision is required because the replacement is the existing R5-53
cleanup under a newly reproduced ordering.

## Fold, evidence, and roadmap disposition

The Trace-14 replacement is eligible to supersede only iteration 2's
holder-free `recoveryClose` no-effect clause in the formal contract. The
pending canonical status paragraph, projection/action map, formal handoff
register, and T-5 clause accurately state the proposed replacement and may now
record it as approved through review iteration 3. Every other iteration-2
clause remains current.

This approval is design authority for the bounded replacement; it is not
replacement evidence or roadmap approval. Roadmap validation must remain
`pending` and the historical formal record must remain non-current until:

1. the bounded model/conformance implementation adopts exactly this existing-
   state, idempotent-close branch;
2. exact seeds `0xea0ef1a4` and `0xee26ec37` replay through the real owner with
   no filtered action, fabricated record, or shadow ownership state;
3. the complete guest-flow-owner formal inputs are re-recorded with exactly
   three jobs and `verify-evidence` passes;
4. every existing named run and randomized host/guest conformance driver
   passes; and
5. an independent evidence review verifies both seeds, the full record,
   record preservation, every ownership branch, and the unchanged action/
   scenario/API inventory before roadmap validation returns to approved.

Any actual model, driver, check configuration, or other formal input change
still invalidates the prior record and requires the complete three-job record
and verification evidence. The retained failing Trace-14 log is premise
evidence only; it is not the replacement proof.

## Iteration 3 verdict

**APPROVED.** Trace 14 falsifies only iteration 2's assumption that a
holder-free ended watch implied completed pending-dial socket cleanup. The
replacement uses the existing ownership cut, socket sent flags, VM state, and
idempotent `shutAll` to represent the real recovery path: it closes only the
pending dial's owner-held `V_h`, preserves the existing terminal record and
all holder facts, and emits no guest `Abort` after VM loss.

All retained branches remain exact. The proposal adds no state, action,
scenario, hazard, API, mechanism, or contract weakening and is eligible for
the bounded canonical replacement fold. The roadmap remains pending and
non-executable until both exact seeds, the full three-job record, verification,
conformance gates, and independent evidence review pass. No model, driver,
production/test source, DES state, evidence, roadmap approval, or commit was
changed by this review.

---

# Review iteration 4 — Trace-47 watch `TakeEnd` amendment

## Iteration 4 metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_fwd_abort_handoff` |
| Date | 2026-10-26 |
| Role | Same independent solution architecture / formal-model reviewer |
| Reviewed scope | Trace-47 falsification, host watch `TakeEnd` cleanup amendment, complete pending-dial cleanup-entry audit, and pending canonical clauses |
| Verdict | **APPROVED** |
| Open findings | 0 |
| Fold eligibility | **ELIGIBLE** — the exact `TakeEnd` amendment may join the iteration-3 formal contract |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** until exact-seed replay, complete three-job evidence, and independent evidence review |

Iterations 1-3 remain the historical review record. Trace 47 does not reopen
the ownership cut or the iteration-3 recovery replacement; it proves another
production entry into the same already-approved pending-dial error cleanup.
This iteration reviews only that existing-action amendment and its bounded
fallout.

The reviewer read the retained attempt-2 raw replay, the current model's
`TakeEnd`, the host driver's `take_end_h`, the Sim end-record delivery,
production `process_end`, `run_dial` cancellation/cleanup, and the pending
canonical status/action/T-5 clauses. This iteration appends only this review.
It does not edit the proposal, canonical design, roadmap, model, driver,
production source, tests, DES state, or evidence; run a model, Metal, test,
build, mutation, or record command; or create a commit.

## Independently verified Trace-47 evidence

The retained log
`.context/netns-density-295/12-01/quint-fwd-pair-seed-ea0ef1a4-attempt2-verbose1.log`
records the exact Metal invocation, nextest run
`76bf4c31-9881-4fd7-aea7-c7d60e5686ea`, and `QUINT_SEED=0xea0ef1a4`.
Trace 47 reaches:

```text
InitWith("watch")
  -> CloseUnattached("g")
  -> CtxRelease("g")
  -> VmmExit
  -> FwdErr("h")
  -> VCtl("gh")
  -> TakeEnd("h")
```

Immediately before `TakeEnd`, raw-log lines 9497-9549 establish:

- host watch `pr(h)` is `ended` with `out = aborted`,
  `cause = SocketError`, `rec = queued`, and `nrec = 1`;
- `strmHeld = false`, `strmAbort = false`, `hres = none`, and
  `guestAbortSent = false`;
- `ep(h) = alive`, the host context is open, and the pending dial still owns
  `V_h`;
- `V_h` has `err = true`, peer shutdown `{R,S}`, and state `closing`, but its
  local `sent = {}` and `sshut = false`; and
- `vmUp = false`.

The spec and driver agree before the action. After the real `TakeEnd`, raw-log
lines 9551-9626 show `rec = read`, the same `SocketError`, `nrec = 1`, and every
same holder/result/message fact. The only divergent compared fields are the
production socket cleanup: `vs(h).sent: {} -> {R,S}` and
`vs(h).sshut: false -> true`. The run fails with the exact seed at lines
9628-9639. This is direct evidence that record consumption, like recovery,
can complete a holder-free pending dial and close its owner-held socket.

## Production caller and owner proof

The trace drives the real owner path rather than synthesizing a close:

1. `take_end_h` releases exactly one queued Sim forwarder record and settles
   the production owner.
2. `consume_ends` receives it and calls production `process_end`.
3. `process_end` records the consumed `ForwardEnd`, removes its watch-id
   mapping, finds the dial still `DialPhase::AwaitingReply`, maps the
   `SocketError` to `GuestDialError::Unreachable { ECONNRESET }`, stores that
   exact outcome, and cancels the dial token
   (`owner_runtime.rs:939-993`).
4. The biased cancellation branch in `run_dial` retrieves that fixed outcome.
   Because the request was written, it selects the existing abort-guest
   cleanup; with the VM/control session gone there is no sender, so no guest
   message is emitted. It stale-aborts the already-ended watch, closes the
   still-owned `V_h`, and removes the dial (`dial.rs:247-350`).

The terminal record and socket cleanup have different owners and remain in the
same owner dispatch: `process_end` consumes/classifies the record, while the
cancelled pending dial closes the socket it still owns. The model's existing
watch-mode `TakeEnd` no-close behavior was correct only for a `Paired` dial
whose `GuestStream` owns `V_h`; `strmHeld = false` proves Trace 47 is not that
branch.

## Exact selector and transition review

The amendment keeps the existing `TakeEnd` guard and first performs its
existing queued-to-read update. It then applies the iteration-3 cleanup
selector only when the pre-state also satisfies:

```quint
s == "h"
  and watchSide(t, "h")
  and not(t.strmHeld)
  and t.ep.get("h") == "alive"
```

Because `canTakeEnd` already requires a queued record of the current open
context, this selector denotes exactly a host watch record being consumed.
`watchSide` fixes the dial kind, `strmHeld = false` excludes caller ownership,
and the alive dialer endpoint excludes a completed holder/cancellation close.
The selector runs on the post-consumption state:

```quint
val cleanupPending = t.vs.get("h").sent != both
val closed = shutAll(t, "h")
{ ...closed,
  strmAbort: false,
  hres: "none",
  guestAbortSent: t.guestAbortSent or (cleanupPending and t.vmUp) }
```

Consequently `rec = read`, `out`, `cause`, `errno`, and `nrec` pass through
unchanged. `shutAll` supplies the existing close complement and is idempotent
when another accepted error path already closed `V_h`. `cleanupPending`
decides only whether this dispatch newly performs close/message cleanup; it
does not infer ownership.

| `TakeEnd` branch | Required effect | Assessment |
|---|---|---|
| Host watch, no holder, alive dialer, close pending | Consume the existing record; close owner-held `V_h`; keep `strmHeld = false`, `strmAbort = false`, `hres = none`; set the existing pending-dial guest `Abort` only while `vmUp`. | **Correct.** Exact production `AwaitingReply` path. |
| Host watch, no holder, alive dialer, already closed | Consume the record; idempotent `shutAll`; preserve existing `guestAbortSent`. | **Correct.** Direct abort, refused attach, prior recovery, and other completed error cleanup are not duplicated. |
| Host watch, held stream | Consume the record and preserve the current no-close behavior; `process_end` updates `DialPhase::Paired.guest_end` and does not cancel the dial. | **Unchanged and correct.** Holder-owned `V_h` remains untouched. |
| Host watch, dead dialer endpoint | Consume the record with no new cleanup. | **Unchanged and correct.** Holder/pending-dial disposal already performed the socket effect. |
| Host non-watch pair | Existing same-dispatch `closeEff` remains. | **Unchanged.** The amendment does not alter ordinary pair ownership. |
| Guest-side `TakeEnd` | Existing record-read behavior remains. | **Unchanged.** No host pending dial is owned there. |
| Released/old context | `canTakeEnd` remains disabled; D-4 projection may still observe a queued record in its released row. | **Unchanged.** No record is silently consumed across a context generation. |

The selector is not over-broad. It does not treat every watch record as a
pending dial: the actual `strmHeld` fact separates `Paired`, and `ep(h)`
separates completed disposal. It does not branch on `SocketError`; production
`process_end` cancels an `AwaitingReply` dial for every terminal watch outcome,
mapping the typed cause as already designed. Restricting the close to Trace
47's cause would incorrectly omit the same mandatory cleanup for another
terminal record.

## Complete pending-dial cleanup-entry audit

The proposal's bounded audit covers every existing `fwd_pair` action that can
initiate or observe this cleanup class:

| Entry | Existing production trigger | Formal disposition |
|---|---|---|
| `Abort(h)`, no holder | Quiescence/teardown/session-loss cancellation | Existing pre-handoff branch aborts the live watch and closes `V_h`. |
| `HolderDrop`, no holder and live watch | Caller drops pending dial future | Existing pending-cancellation branch aborts the watch and closes `V_h`. |
| Refused watch-mode `Attach(g)` | Real `Refused` result ends dial | Existing reciprocal error branch closes `V_h` with no guest `Abort`. |
| `CtxRelease(h)` recovery, ended watch | Recovery quiescence cancels remaining `AwaitingReply` dial | Iteration-3 branch preserves queued record and closes `V_h`. |
| `TakeEnd(h)`, ended watch | `process_end` consumes record and cancels remaining `AwaitingReply` dial | This amendment preserves consumed record and closes `V_h`. |

`FwdErr(h)` and `VCtl` only produce the terminal watch record; `VmmExit` only
produces the fault input. Held-stream `TakeEnd`, non-watch `TakeEnd`, guest
`TakeEnd`, `OwnerClose`, and terminal holder-free `HolderDrop` do not enter the
pending-dial cleanup branch. No additional state, action, or fallback is
needed to distinguish them.

## Retained contract and no-weakening audit

| Contract or invariant | Result |
|---|---|
| Record classification and order | Preserved. `TakeEnd` still performs one queued-to-read transition; exact outcome/cause/errno/count remain those of `last_consumed_end`. |
| R5-53 and typed dial outcomes | Preserved. `owner_flows` owns `Unreachable`, `Quiesced`, `NotServing`, and `NoSession`; this model represents only their common mandatory socket/message cleanup. |
| R5-63 / WATCH-ABORT | Preserved. `strmHeld = true` skips the amendment, so a holder's `V_h` is never closed by record consumption. |
| R5-28 / U-5 / K-A4 | Preserved. A live VM gets the already-approved pending-dial `Abort` only when cleanup is newly required; a gone VM gets none; host-close cleanup remains. |
| One record | Preserved. No new record, reclassification, count increment, or second `TakeEnd` is introduced. |
| Ownership facts | Preserved. `strmHeld` stays false, `strmAbort` stays false, and holder-free `hres` stays `none`; `strmAbort => strmHeld` remains exact. |
| `WatchAbortLeavesHolderSocket` | Preserved. The changed socket is pending-dial owner-held, so `gh.vhTouched` remains a held-stream-only ghost. |
| Recovery and `HolderDrop` | Preserved byte-for-byte from iterations 2-3. The amendment neither widens those guards nor changes their effects. |
| Security and cleanup | Strengthened to the already-required outcome: consuming a pending dial's terminal record cannot leave owner-held `V_h` open. No message is sent to a gone guest. |

No public/cross-crate type, trait method, enum variant, typed error, parameter,
owner, lifecycle gate, wire/persisted format, config/CLI field, production
mechanism, test seam, model state, action, named run, scenario, hazard, lane,
or check is added. The amendment reuses the exact existing cleanup selector
and facts.

The normative diff removes no `zero`, `never`, `must`, `exactly`, fail-closed,
forbidden, required, security, ownership, or cleanup guarantee. It corrects a
formal assumption that otherwise leaves a real owner-held socket open in the
model. No new user decision is required because the branch is the mandatory
cleanup of an already-approved typed pending-dial error.

## Fold, evidence, and roadmap disposition

The exact Trace-47 amendment is eligible to join the approved formal contract.
The pending canonical status paragraph, `TakeEnd` action map, handoff register,
and T-5 clause accurately state it and may now record approval through review
iteration 4. Iterations 2-3 remain current for every other branch.

This is design/fold authority only. Roadmap validation must remain `pending`,
and the historical evidence must remain non-current until:

1. the bounded model/conformance implementation adds exactly this post-record-
   consumption reuse of the approved selector;
2. exact seeds `0xea0ef1a4` and `0xee26ec37` replay through the real owner with
   no filtered action, fabricated result, or shadow ownership state;
3. the complete guest-flow-owner formal inputs are re-recorded with exactly
   three jobs and `verify-evidence` passes;
4. every existing named run and randomized host/guest conformance driver
   passes; and
5. an independent evidence review verifies both seed replays, every cleanup
   entry, record preservation, all held/dead/non-watch exclusions, the full
   record, and the unchanged action/scenario/API inventory before roadmap
   validation returns to approved.

Any actual model, driver, check configuration, or other formal input change
continues to require that complete replacement record. The retained Trace-47
failure proves the premise and required post-state; it is not replacement
evidence.

## Iteration 4 verdict

**APPROVED.** Trace 47 proves that consuming a terminal host-watch record can
cancel an `AwaitingReply` dial whose owner still holds open `V_h`. The amendment
uses the actual ownership and endpoint facts, applies the existing idempotent
cleanup after the exact record-read transition, and preserves the record's
outcome, cause, errno, and count. Held streams, dead endpoints, non-watch
pairs, guest records, recovery, and `HolderDrop` remain unchanged.

The amendment is necessary, exact, and fold-eligible. It adds no state,
action, API, scenario, hazard, mechanism, or contract weakening. The roadmap
remains pending and non-executable until both exact seeds, complete three-job
evidence, verification, conformance gates, and independent evidence review
pass. No model, driver, production/test source, DES state, evidence, roadmap
approval, or commit was changed by this review.
