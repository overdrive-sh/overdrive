# Independent Quint review — STEP 11-06 TCP capture pending-place protocol

## Review metadata

| Field | Value |
|---|---|
| Review ID | `TCP-CAPTURE-PENDING-STEP11-06-QUINT-REVIEW-01` |
| Feature | `netns-density-295` |
| Roadmap step | `11-06` |
| Review date | 2026-10-23 |
| Reviewer | Fresh independent formal-model reviewer |
| Scope | User-approved bounded model of the `TCP-CORRELATION` shared pending-place charge, transfer and release protocol |
| Verdict | **APPROVED** |
| Blocking findings | 0 |

## Authority and review boundary

The controlling product contract is `TCP-CORRELATION` in
`docs/feature/netns-density-295/feature-delta.md` section *[REF] TCP capture
correlation*, the exact accepted artifact
`.context/netns-density-295/11-06-design/correlation.md`, and independent
DESIGN review iteration 9 in
`docs/feature/netns-density-295/deliver/design-review-11-06-correlation.md`.
The subsequent user ruling approved this small protocol model together with
the separately bounded UDP-slot test. That ruling authorizes a model of the
shared pending-place protocol only: it does not authorize a runtime API,
production accessor, Sim seam, conformance driver, BPF-mechanics model, new
lane or automatic implementation-refinement claim.

This review covers:

- `specs/quint/tcp-capture-pending/tcp_capture_pending.qnt`;
- `specs/quint/tcp-capture-pending/hazard/tcp_capture_pending_hazards.qnt`;
- `specs/quint/tcp-capture-pending/checks.toml`;
- `specs/quint/tcp-capture-pending/README.md`;
- runner-written evidence under
  `specs/quint/tcp-capture-pending/evidence/`; and
- the repository's automatic discovery of that subsystem through
  `specs/quint/<subsystem>/checks.toml`.

No production, Rust test, DES, roadmap, feature-design, execution-log or
existing Quint-subsystem file was changed by this review. The reviewer did
not run a native-metal or physical-host kernel operation and did not record
new evidence.

## Contract-to-model assessment

| Accepted protocol outcome | Model evidence | Assessment |
|---|---|---|
| One shared pending place is charged at acceptance | `Accept` moves every under-bound `pending` or `ready` child into `charged` and records one `acceptedPlace`; `OneSharedCharge` equates liveness with one unreleased charge | **Exact at the model abstraction.** Immediate missing/refused classification is represented as an atomic charge-and-release. Full-bound overflow is never charged. |
| A Fast Open `Pending` child retains that charge through `Ready` and emission | `PendingBecomesReady` changes only `row`; `EmitReady` changes only `phase`, `row` and `eventCount`; `place == acceptedPlace` remains invariant | **Exact.** No second place is acquired at either transition. |
| A ready child emits once | `EmitReady` requires `eventCount == 0`, clears the row, and sets the count to one; `ReadyEmitsOnce` caps every child at one event | **Exact for the four one-shot child identities.** |
| Successful attach, consuming abort and close release the emitted handle's place | `Attach`, `Abort` and `Close` move the child to `free`, clear `place` and add it to `released`; the dedicated run executes all three | **Exact.** Only successful `Attach` creates the model-only downstream markers. |
| Missing/refused correlation and child close release without an owner event, owner flow or control traffic | Immediate missing/refused classification and held-child `Missing`, `Refused` and `ChildClose` all end in `free`; `RejectionCreatesNoOwnerOutput` forbids all three outputs | **Exact.** The exhaustive `step` relation includes every cleanup action; named runs separately witness immediate missing/refused, held-child refusal and child-close cleanup. |
| Overflow resets only at the shared bound | `Accept` chooses overflow only when `freePlaces` is empty; `OverflowOnlyAtBound` retains the historical guard; the full-bound run fills both places before overflow | **Exact for the two-place instance.** |
| Correlation capacity never refuses below the shared bound | `NoCorrelationCapacityRefusalBelowBound` requires the capacity-refusal set and historical flag to remain empty | **Exact, with direct teeth.** The only hazard enables precisely that forbidden refusal while a place is free. |
| Releasing one child makes its place reusable | Every release clears `place`; the ready/abort run and the full-bound run admit a different child onto the released place | **Exact for bounded reuse.** |
| Rejection creates no owner flow or control message | `RejectionCreatesNoOwnerOutput` covers every member of `rejected`; overflow, missing, refused and child-close paths add no downstream markers | **Exact at the declared marker abstraction.** |

The model does not weaken the accepted protocol. It adds no second quota,
does not convert an emitted handle to an uncharged state, does not permit a
below-bound capacity outcome, and does not create an output on a rejected
child.

## State, invariant and non-vacuity review

The state machine is closed over three adapter phases (`free`,
`correlation_pending`, `emitted`), four correlation-row classifications and
the model-only accounting/history sets. Every `step` action assigns every
state variable. `OneSharedCharge` is the central ownership invariant: a live
child is equivalent to a charged, unreleased child owning one member of
`PLACES`; no two live children own the same place; and the number of live
children is bounded. The retained `acceptedPlace` ghost makes the
Pending-to-emitted transfer distinguishable from release-and-recharge.

The remaining invariants close the relevant complements:

- `AdmissionAndReleaseAccounting` partitions every attempted child into
  charged, overflowed or hazard-only capacity-refused outcomes, and requires
  every released child to be free and unplaced;
- `ReadyEmitsOnce` constrains both pre-emission Ready and post-emission state,
  and requires every downstream marker to follow the one event;
- `RejectionCreatesNoOwnerOutput` makes missing/refused/closed/overflow
  outcomes observably negative; and
- the two historical ghost flags prevent a later state from concealing an
  earlier under-bound overflow or correlation-capacity refusal.

The five design runs are non-vacuity witnesses, not substitutes for the
exhaustive invariant check. Together they reach an adapter-held Pending child,
Ready, emitted, successful attachment, abort, close, immediate rejection,
held-child refusal, child close, a full bound, overflow, release and place
reuse. The `Missing` cleanup action is additionally included in the
exhaustively explored `step` relation. No witness-only or duplicate bounded
check is present.

## Hazard review

There is one hazard for the one bounded protocol decision. Setting
`capacityRefusal` adds one forbidden `Accept` outcome for a correlated child
while a shared place remains free. The recorded three-state counterexample is
exact:

1. initialize with both places free;
2. prepare child 1 as `pending`; and
3. accept child 1 into `capacityRefused` without charging it.

That trace violates `NoCorrelationCapacityRefusalBelowBound`; the accounting
partition and no-output rules continue to hold, so the counterexample is not
caused by an unrelated weakened clause. The same hazard also makes the normal
ready-at-accept critical run fail at its first post-accept expectation. No
second hazard family or rejected-alternative model is needed.

## Liveness and proof limits

The model contains safety invariants and concrete progress witnesses. It does
not contain a temporal liveness property or fairness assumption. Therefore it
does not prove that the environment eventually changes a Pending row to Ready,
that the owner eventually scans it, or that a cleanup action eventually
runs. This is an honest boundary: wake delivery, pre/post-wait scans and the
one-second audit fallback were outside the user's bounded model approval and
are stated as not modelled in the README. The named runs demonstrate that the
intended progress transitions are enabled and preserve safety; they do not
turn those examples into an eventuality proof.

The exhaustive result is exhaustive only for this finite instance: two
abstract places and four one-shot child identities. It proves all reachable
interleavings in that instance. It does not prove the numerical 4,096-object
production bound, arbitrary-length sequential reuse, or an inductive theorem
for every bound. The README states this limitation and does not present the
two-place result as the full production state universe.

Likewise, child identity in the model means that a charge and event remain
associated with the same abstract child. Application-cookie and
original-destination values, child-cookie aliasing in BPF representation,
option bytes, callback order, map lifetime, accept-queue arithmetic and
kernel cleanup are outside the state machine. The model therefore does not
prove exact production cookie/destination identity. Those facts remain in the
Linux-source/spike evidence and the independently reviewed Tier-3 composition
cell; they are not inferred from `Safety`.

## Production-composition boundary

The README correctly identifies the separate Tier-3 composition boundary:
`crates/overdrive-init/src/guest_flow/real_kernel.rs` contains
`tcp_correlation_preserves_exact_identity_and_cleans_transients_for_every_handshake_path`,
and the existing guest-kit wrapper is in
`crates/overdrive-init/tests/integration/in_guest_lane.rs`. DISTILL review
iteration 5 approved that cell's real `Connect4` / final-ACK callback /
`KernelGuestOwnerIo` shape, including an adapter-held Pending child, the exact
4,096 boundary, continuity through emission, consuming release, one-time
capture, direct rejection and cleanup.

That test is an independent implementation-evidence layer. This model adds no
driver that connects its state to Rust/Aya production state, so neither the
recorded Quint run nor this review establishes implementation refinement. No
new native run was performed or claimed here.

## Executed verification

| Check | Result |
|---|---|
| `cargo xtask lima run -- cargo xtask quint typecheck` | **PASS** — all 27 repository Quint files typecheck, including both new files. |
| `cargo xtask lima run -- cargo xtask quint check --subsystem tcp-capture-pending --jobs 3` | **PASS** — reviewer run `169093-1792763477.337`, 8/8 expected outcomes. |
| Exhaustive design `Safety` in the reviewer run | **PASS** — 229,133 generated states, 46,315 distinct states, depth 13, zero states left on the queue, no violation. |
| Exhaustive capacity-refusal hazard in the reviewer run | **PASS** — expected violation in the three-state `PreparePending -> Accept` trace. |
| `cargo xtask lima run -- cargo xtask quint verify-evidence --subsystem tcp-capture-pending` | **PASS** — recorded run `166979-1792763128.074` matches current spec/config hashes and all 8 outcomes met `expect`. |
| `cargo xtask lima run -- cargo xtask quint verify-evidence` | **PENDING OUTSIDE THIS SUBSYSTEM** — `probe-count` and `tcp-capture-pending` verify; the pre-existing `guest-flow-owner` record is stale only because `udp_slots.qnt` changed. That separate record is being remediated independently and was not re-recorded by this reviewer. |
| CI/discovery registration | **PASS** — the subsystem is auto-discovered from `specs/quint/tcp-capture-pending/checks.toml`; all eight checks are `ci = true`; no manual runner or new lane is present. |
| Scoped whitespace check | **PASS** — no whitespace error in the reviewed model/context scope. |

The repository-wide evidence verifier remains a global gate until the
separate `guest-flow-owner` record is refreshed. Its stale `udp_slots.qnt`
hash is not a failure of this subsystem and does not invalidate the passing
subsystem-local evidence.

## Scope and no-weakening audit

- One new self-contained Quint subsystem, one hazard variant, five design
  runs and one hazard run are present.
- All validation uses the existing Lima-routed Quint runner and automatic CI
  discovery. There is no runner script, platform change or new lane.
- No production API, runtime state, Sim seam, conformance driver, BPF detail,
  model variable in an existing subsystem, mutation configuration, KASAN
  configuration or physical-host operation was added.
- The finite-instance and non-refinement limits are explicit. The model does
  not claim new native evidence.
- The only expected violation is the approved capacity-refusal hazard; all
  design checks hold.

## Final verdict

**APPROVED.** The model is a faithful finite abstraction of the user-approved
shared pending-place protocol. Its exhaustive safety check covers the charge,
Pending-to-Ready-to-emitted transfer, one-time event, release, full-bound
overflow, below-bound admission, negative rejection outputs and bounded place
reuse. Its single hazard fails for the intended decision weakening, its five
critical runs provide non-vacuity witnesses, and its README states the
two-place, four-child, non-liveness and non-refinement proof limits. There are
no blocking findings.
