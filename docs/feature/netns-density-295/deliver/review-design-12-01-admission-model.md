# Independent DESIGN review — STEP 12-01 admission/refusal action semantics

## Current disposition

| Field | Value |
|---|---|
| Current iteration | 4 — post-approval authority and fold reconciliation |
| Decisions | `ADMISSIONREFUSAL1` (reviewed as `ADMISSION-REFUSAL-1`) and `ACTPRIORITY1` (reviewed as `ACT-PRIORITY-1`) |
| User authority | **APPROVED 2026-10-26** — exact quote: “i approve the rest of the decisions.” |
| Canonical authority | `feature-delta.md` authority-table row `BFIFO1, PABORTERR1, ACTPRIORITY1, ADMISSIONREFUSAL1, QUIESCENCEOPEN1, CHCGROUP55RUNNER1` |
| Open technical findings | 0 — F1/F2 closed in Iteration 2; F3/F4 closed in Iteration 3 |
| Final verdict | **APPROVED** |
| Roadmap disposition | Validation remains **pending evidence** with `approved_at = null` |

Iterations 1-3 below remain the review and remediation history. Iteration 4 at
the end of this artifact supersedes only the outstanding user-authority gate.

## Metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_admission_model` |
| Date | 2026-10-26 |
| Role | Independent solution architecture / formal-model reviewer |
| Iteration | 1 |
| Reviewed proposal | `docs/feature/netns-density-295/deliver/design-12-01-admission-model.md` |
| Verdict | **CHANGES_REQUESTED — SPECIFICATION_AMBIGUITY** |
| Blocking technical findings | 2 |
| Activation-priority disposition | Technically ready as a separate material user decision |
| Admission/refusal disposition | Not ready for user decision or implementation |
| Roadmap disposition | **PENDING / NON-EXECUTABLE**; `approved_at = null` |

This review applies the repository DESIGN, production-reachability,
correctness/invariant precedence, exact action/port, typed-error,
no-unrequested-scope, and no-weakening rules together with `nw-design`,
`nw-sar-critique-dimensions`, and `nw-roadmap-review-checks`. The repository's
required native-Markdown artifact governs over the reviewer definition's
legacy YAML format.

The review is bounded to the two proposed material decisions: atomic guest-flow
admission/refusal and activation-result precedence. It reads their retained raw
replays, actual owner entry points, task/cancellation/resource ownership,
accepted R5-26/QHOLD/U-1/G-V3 contracts, and pending canonical/roadmap clauses.
It creates only this review artifact. It does not edit the proposal, canonical
design, roadmap, production/model/driver/test source, DES state, or evidence;
run a model, Metal, test, build, mutation, or record command; or create a
commit.

## Reproduced evidence

### Admission seed `0xafd56990`

The retained log
`.context/netns-density-295/12-01/owner-flows-afd56990-verbose1.log` drives the
existing named run `QuiesceAbortsFlowsAndRefusesAdmissionUntilRestore` through
the production owner. After the first quiescence hold, it queues guest flow 2
and plays `HostAcceptFlow(2)`. The model closes flow 2; production registers it
as `GuestFlowPhase::Registered { request: Nothing }`. Raw lines 580-629 show
that this is the only compared-state difference.

The source proves the cause. `spawn_flow` currently registers any accepted
connection for a known non-Retiring CID with a session before reading its
request and without consulting `GuestAdmission` (`owner_runtime.rs:364-401`).
The later `handle_flow` reads/decodes the request, then checks launch/session,
Active, quiescence, and capacity and sends the existing typed refusal
(`:475-623`). Holding request delivery exposes the intermediate registration.

The proposed temporary early-close gate is correctly rejected. The accepted
R5-26 owner cell requires the same quiesced connection, once its buffered
current-launch request is read, to produce exactly
`Refused { flow, Quiesced }`. Immediate close preserves empty owner state but
loses that required wire outcome.

The contradiction is real:

- U-1/R5-26 requires admission check plus registration to linearize as one
  owner step relative to quiescence; and
- QHOLD requires `guest_flows`, `dials`, and `datagrams` to be empty while
  admission is quiesced.

### Activation seed `0xf23a4188`

The retained log
`.context/netns-density-295/12-01/owner-flows-f23a4188-verbose1.log` reaches a
valid Provisioned CID that is simultaneously quiesced and lacks a live control
session. `ActivateQuiesced` returns `QuiescenceLatched`; the unchanged state
then enables `ActivatePending`, whose model result is
`ControlSessionPending`. Production returns `QuiescenceLatched` again. Raw
lines 1030-1179 show that result as the only mismatch.

Production order is explicit under the owner lock (`guest_network.rs:918-958`):

1. invalid/mismatched/Retiring plan -> existing postcondition error;
2. quiesced -> `QuiescenceLatched`;
3. already Active -> `Activated`;
4. no session -> `ControlSessionPending`; and
5. otherwise transition to Active -> `Activated`.

The proposal correctly limits its formal priority statement to a valid
Provisioned CID, where steps 2, 4, and 5 apply.

## Technically correct parts

| Concern | Assessment |
|---|---|
| Admission linearization | Correct objective. The accept owner must decide under the same state lock as quiescence/lifecycle, before any admitted registration/place exists. |
| Existing refusal set | Complete at the admission layer: `NotActive`, `Quiesced`, `Capacity`; no new variant or wire value is needed. |
| Attribution | Correct. Request token must equal the snapshotted launch before any refusal is written; unknown/no-session/Retiring and stale-launch connections remain silent. |
| Race outcomes | Correct. Accept/admit before first hold registers and is aborted by the hold; first hold before accept fixes refusal without admitted state. |
| Formal action reuse | Correct in principle. `HostAcceptFlow` can represent the complete refusal branch while the admitted branch retains `HostReadReqN`; no new formal action/state is inherently required. |
| Activation priority | Correct. `ActivatePending` must additionally require `not(quiesced)`; `ActivateQuiesced`, `Activate`, and `ActivateRetiring` retain their effects. |
| No weakening intent | Correct. The proposal refuses to weaken `QuiescedHoldsNoFlow` or replace the typed refusal with silent close. |

## F1 — blocking: refusal-only sockets have no bounded owner or resource contract

**Severity:** Critical / blocking.
**Location:** proposal §§ *Linearization and ownership* and *Approval and
downstream gates* (`design-12-01-admission-model.md:85-127`, `:222-244`).
**Status:** Open.

The proposed refusal branch deliberately keeps the accepted socket and a task
alive long enough to read up to 32 request bytes under
`FLOW_REQUEST_DEADLINE`, while forbidding that connection from entering
`guest_flows`, `datagrams`, a pair, or any quota/node place. The document says
the existing accept task may own it and that no detached task may outlive
shutdown, but it does not pin where the join handle lives, how many such tasks
may exist, or what happens when that bound is exhausted.

This is an architectural resource and lifecycle gap, not private helper
structure:

- the accept loop can continue draining the kernel queue and creating one
  refusal read per arriving connection, so a per-task deadline bounds duration
  but not concurrent count (`arrival rate × deadline`);
- each task retains at least a socket, task, cancellation token, and request
  buffer, while the design expressly assigns it no existing flow place;
- the current tracked owner is `CidState::flow_tasks`. Quiescence and teardown
  cancel and await those handles, and pending-task count contributes to the
  node dial-capacity calculation. Reusing it therefore contradicts the claim
  that the refusal takes no node accounting unless that consequence is
  explicitly accepted;
- leaving the task outside `flow_tasks` supplies neither a join/drain owner nor
  a protocol-level bound and risks exactly the detached lifetime the proposal
  forbids; and
- awaiting the refusal inline in the single accept loop would bound concurrency
  at one but introduces head-of-line blocking for up to
  `FLOW_REQUEST_DEADLINE`, a material performance/ordering choice absent from
  the proposal.

The host file-descriptor ceiling is not the required flow/refusal bound. The
accepted design budgets descriptors around bounded flows, sessions, and the
serve reserve; an agent cannot silently consume the reserve with an unbounded
new class of private sockets merely because the process eventually hits
`RLIMIT_NOFILE`.

The lifecycle is also incomplete across later owner events. The proposal says
the refusal result is fixed at accept even if teardown/session state changes,
and separately says no task may outlive teardown. It does not specify whether
teardown cancels before reading and therefore closes silently, waits for the
fixed refusal attempt, or sends only if the snapshotted session remains live.
Those alternatives have different wire, shutdown latency, and resource
outcomes.

### Required DESIGN remediation

Before user presentation, the architect must pin:

1. the single owner of every refusal-only task/socket and the exact global
   drain/join path for owner shutdown, quiescence overlap, teardown, and
   session loss;
2. a finite concurrency/resource bound and what existing or new capacity it
   consumes;
3. the observable overload result when that bound is full;
4. whether accept processing remains concurrent or is serialized, including
   the resulting throughput/head-of-line cost; and
5. the exact response-attempt rule when lifecycle/session cancellation follows
   the admission snapshot but precedes the complete request read.

These are not choices for the crafter. Reusing `flow_tasks`, serializing the
accept loop, or adding a bounded refusal-task collection each changes capacity,
ordering, or ownership. Under `.claude/rules/design.md` they are material
decisions requiring explicit user approval once specified. The reviewer does
not select one.

## F2 — blocking: request-error versus snapshotted-admission priority is incomplete

**Severity:** High / blocking.
**Location:** proposal § *Linearization and ownership*, especially the “first
failing admission cause,” request consumption, token validation, and typed
response clauses (`design-12-01-admission-model.md:89-122`).
**Status:** Open.

The proposal pins priority among `NotActive`, `Quiesced`, and `Capacity`, and
pins stale/no-session silence. It does not pin what the refusal-only reader
does when the complete buffered request is malformed or uses an unsupported
version.

Current production decodes before evaluating phase/quiescence/capacity and,
when it can extract an attributable launch, sends `Malformed` or `Unsupported`.
The proposed mechanism snapshots an admission refusal before decoding. Both
of these interpretations fit the current prose:

- preserve decode priority and send `Malformed`/`Unsupported`; or
- treat the snapshotted admission cause as fixed and send
  `NotActive`/`Quiesced`/`Capacity` after only token validation.

They are different typed wire outcomes. Neither green tests nor private
implementation structure may choose between them. The typed error contract
must name the complete priority and mapping.

### Required DESIGN remediation

Pin the full refusal decision table after accept:

- incomplete request, EOF/error, and deadline;
- malformed and unsupported complete requests;
- stale launch token;
- snapshotted `NotActive`, `Quiesced`, or `Capacity`;
- sender/session loss after snapshot; and
- the no-failure admitted branch.

For each row, state whether a response is attempted, the exact existing
`FlowRefusal` value, which snapshotted/current session may receive it, socket
close timing, and whether later owner state can change the result. This can use
only existing wire variants, but their precedence is a contract, not an
implementation detail.

## Activation priority — technically ready as a separate decision

The activation amendment has no technical finding. For a valid Provisioned
CID, the exact result priority is:

```text
quiesced                     -> QuiescenceLatched
not quiesced && !sessLive    -> ControlSessionPending
not quiesced && sessLive     -> Activated
```

The formal change is exactly:

```quint
ActivatePending:
  alloc == "Provisioned" && not(quiesced) && not(sessLive)
```

`ActivateQuiesced` keeps `alloc == "Provisioned" && quiesced`. Both refusal
actions remain no-state-change results; `Activate`, `ActivateRetiring`, retry
budget, callers, variants, and production remain unchanged. The exact seed
supplies the overlap evidence, and existing named runs retain non-overlap
coverage. No new run/action/state/filter is needed.

This is independently decidable from refusal-task ownership. The combined
proposal must therefore be surfaced as separate numbered decisions; approval
of activation priority cannot be inferred from a later admission-mechanism
choice, or vice versa.

## No-weakening and scope audit

| Contract | Review disposition |
|---|---|
| QHOLD / `QuiescedHoldsNoFlow` | Must remain exact. A refusal-only task cannot appear in admitted `guest_flows`, `dials`, `datagrams`, pair inventory, or accepted flow quota. |
| R5-26 / U-1 | Both race orders remain required. No late connect may attach or send `Paired`. |
| Existing typed refusal | `Refused { Quiesced }` for the stable reproduced current-launch/live-session case remains required. F2 blocks the other priority rows. |
| ATTR-TOKEN / ATTR-LIVE-CLAIM | Stale tokens and replaced sessions must not receive or redirect a response across launches. |
| Capacity/resource bounds | F1 unresolved. No accepted contract authorizes an unbounded rejection class or silent reserve consumption. |
| G-V3 activation | Activation correction preserves both variants and pins their overlap priority without changing state. |
| Public/wire surface | Existing types suffice; no new field/variant/API is justified by the current proposal. |

No approved `zero`, `never`, `must`, `exactly`, fail-closed, forbidden,
required, security, attribution, or cleanup term may be relaxed to close F1 or
F2. In particular, redefining “empty while quiesced” to permit accepted
registrations is invalid, and silently dropping the refusal is not equivalent
to the existing accepted typed response.

## Authority gate after technical remediation

The proposal is material and will require explicit user approval after F1 and
F2 are resolved. `.claude/rules/design.md` § *Surface material design
decisions to the user before they become accepted* says:

> “A design document is a record of a proposal, not evidence that the user has
> accepted it.”

It expressly applies to changes in:

> “the owner or meaning of a domain identity, state, lifecycle transition, or
> cleanup authority”

and:

> “a public or internal action/port contract.”

Admission linearization, refusal-task ownership/capacity, response priority,
and activation-result precedence fall directly into those categories. An
independent review cannot authorize them.

The activation decision is technically reviewable now:

### ACT-PRIORITY-1 — activation overlap priority

For a valid Provisioned CID, approve quiescence before session availability:
quiesced returns `QuiescenceLatched`; otherwise no live session returns
`ControlSessionPending`; otherwise activation returns `Activated`.
`ActivatePending` gains only `not(quiesced)`. Existing variants, state effects,
retry policy, API, and production behavior remain unchanged. Runtime/API/wire/
state-space/check-count cost is zero; exact seed `0xf23a4188` and the full
formal evidence gates remain required.

The admission decision is not yet reviewable. It must not be presented as an
approve/reject request until its owner/bound/overload/cancellation and complete
typed-priority table are exact. Once remediated, it must be a separate numbered
decision with runtime/FD/task bounds and validation cost.

## Roadmap and next-gate disposition

Roadmap validation correctly remains `pending` with `approved_at = null`.
No production/model/driver implementation may be dispatched from this review.

The next valid sequence is:

1. remediate F1 and F2 in the proposal without weakening QHOLD, R5-26,
   ATTR-TOKEN, or the stable Quiesced refusal outcome;
2. independently re-review the exact admission decision and its normative
   diff;
3. surface the admission and activation decisions separately for explicit
   user approval;
4. record exact authority/date and perform the repository's post-approval
   fold review; and only then
5. implement and run exact seeds `0xafd56990` and `0xf23a4188`, all unchanged
   named/randomized owner-flow targets, the complete three-job
   guest-flow-owner record, `verify-evidence`, and independent replacement-
   evidence review.

## Iteration 1 verdict

**CHANGES_REQUESTED — SPECIFICATION_AMBIGUITY.** The reproduced admission
contradiction is real, and atomic admission plus refusal from existing surfaces
is the correct direction. The proposal does not yet specify the bounded owner,
capacity, global drain, overload behavior, lifecycle cancellation, or complete
typed refusal priority of the refusal-only socket/task. Those gaps can create
an unbounded private resource class or change wire/performance behavior and
cannot be delegated to implementation.

The activation-priority correction has zero technical findings and is ready as
the separate ACT-PRIORITY-1 user decision, but no implementation authority is
granted here. The combined proposal is not ready for acceptance. STEP 12-01
and roadmap validation remain pending. No production/model/driver/test source,
DES state, evidence, roadmap approval, or commit was changed by this review.

---

# Review iteration 2 — bounded refusal-reader remediation

## Iteration 2 metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_admission_model` |
| Date | 2026-10-26 |
| Role | Same independent solution architecture / formal-model reviewer |
| Reviewed scope | F1/F2 remediation, resource accounting, kernel saturation, lifecycle drain, typed priority, and the unchanged activation decision |
| Verdict | **CHANGES_REQUESTED — RESOURCE_CONTRACT_AMBIGUITY** |
| Iteration-1 F1 | Substantially remediated; exact accounting/cost remains open as F3/F4 |
| Iteration-1 F2 | **CLOSED** |
| New blocking findings | 2 |
| Activation-priority disposition | Still technically ready as separate ACT-PRIORITY-1 |
| Admission/refusal disposition | Not yet reviewable by the user |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

This iteration preserves iteration 1 as the historical review. It reads the
revised exact task owner, node bound, lifecycle table, typed response table,
current task/data structures, startup fd check, stream accept loop, actual
Linux 7.0 listen/backlog/reset paths, and pending canonical clauses. It appends
only this review iteration and changes no proposal, canonical artifact,
roadmap, source/model/test, DES state, evidence, or commit.

## Iteration-1 remediation dispositions

| Requirement | Revised contract | Disposition |
|---|---|---|
| One owner and drain path | Reuses `CidState::flow_tasks` and `FlowTask` cancellation/join, with quiesce, teardown, session-loss, and root-shutdown outcomes stated. | **Closed in ownership/lifecycle intent.** No second task collection or detached task is proposed. |
| Finite concurrency and overload | Proposes node-wide `GUEST_FLOW_MAX_PAIRS`, stops accepting at saturation, leaves children in the kernel queue, and relies on Linux RST after backlog fullness. | **Partially closed.** F3 and F4 identify incorrect accounting and incomplete saturation/cost details. |
| Response after later lifecycle changes | Biased cancellation closes silently; final launch and exact session identity are rechecked; failed send is not retried or redirected. | **Closed.** This preserves ATTR-TOKEN/ATTR-LIVE-CLAIM and gives teardown/session/root shutdown exact outcomes. |
| Full typed priority | Incomplete/EOF/error/deadline are silent; stale/invalid token is silent; attributable decode errors retain `Unsupported`/`Malformed`; valid request uses snapshotted `NotActive`/`Quiesced`/`Capacity`; later lifecycle mismatch suppresses the response. | **F2 CLOSED.** Every existing response and precedence is pinned. |
| Activation overlap | Same exact `not(quiesced) && not(sessLive)` guard for `ActivatePending`. | **Still approved technically.** |

The revised admission mechanism correctly keeps refusal readers out of
`guest_flows`, `datagrams`, forwarder inventory, and `held_flows`, retains
concurrent reads under `FLOW_REQUEST_DEADLINE`, and makes the cross-CID
starvation exposure explicit rather than hiding it as private structure. The
remaining findings concern whether the proposed bound and material cost are
actually the values the user would be asked to approve.

## F3 — blocking: reader-count predicate and fd-budget arithmetic are not exact

**Severity:** High / blocking.

**Location:** proposal § *ADMISSION-REFUSAL-1* lines 96-111 and decision lines
280-290.

### The proposed count includes non-refusal datagram tasks

The proposal defines refusal-reader count as:

> the number of `flow_tasks` keys absent from their CID's `guest_flows`

That is not the exact class. Existing `set_datagram_awaiting_paired` removes a
connection from `guest_flows`, keeps its `FlowTask` in `flow_tasks`, sets
`FlowTask.flow = Some(flow)`, and inserts the association in `datagrams`
(`owner_runtime.rs:1375-1397`). Every such datagram task satisfies the proposed
“key absent from `guest_flows`” predicate and would be charged to the refusal
pool.

Conservative overcounting is still a contract change: it can stop the global
stream accept loop below the promised 65,536 refusal readers based on admitted
datagram work. The decision claims a separate exact bound and a particular
cross-CID exposure; its predicate must distinguish refusal readers using the
already-existing task/owner facts without charging another lifecycle class.
The crafter cannot silently choose a different count.

### The fd formula does not match current source or the stated added pool

The proposal says the startup requirement changes from:

```text
4 * GUEST_FLOW_MAX_PAIRS + 16,384 + 1 + reserve
```

to five times that global constant. Current production source does not use
that expression. It imports `GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION = 4,096` and
checks:

```text
4 * 4,096 + 16,384 + 1 + 32,767 = 65,536
```

(`guest_network.rs:821-827`). The global `GUEST_FLOW_MAX_PAIRS` proposed for
refusal readers is 65,536 (`wire.rs:792`). Therefore:

- adding exactly one descriptor for every proposed reader to current source
  yields `65,536 + 65,536 = 131,072`; while
- the proposal's written `5 * GUEST_FLOW_MAX_PAIRS + 16,384 + 1 + 32,767`
  yields `376,832`.

The canonical feature delta separately describes an accepted
`4 × GUEST_FLOW_MAX_PAIRS` term, which would itself be `311,296` with the
other terms and does not match the current source. The proposal cannot present
“four to five” as a simple bounded increment without reconciling that existing
source/design mismatch and giving the user the exact before/after requirement.

### Required remediation

Before user presentation:

1. pin an exact existing-state predicate for refusal readers that excludes
   datagram and admitted flow tasks;
2. state the exact numeric current-source requirement, accepted target
   requirement, and proposed post-decision requirement;
3. identify whether the pre-existing `4 × global` versus `4 × per-allocation`
   mismatch is compiler/mechanical fallout, a separate defect, or a canonical
   wording error; and
4. state the exact `GuestNetworkError::FdBudget.required` value the approved
   implementation must enforce.

This is material decision cost. A sixfold difference in required `RLIMIT_NOFILE`
cannot be left to implementation or hidden in a symbolic term.

## F4 — blocking: saturation liveness and the cheaper isolation alternative are incomplete

**Severity:** High / blocking.

**Location:** proposal lines 112-130, rejected alternatives, and
ADMISSION-REFUSAL-1 decision text.

Stopping the sole stream accept loop at the refusal bound is a coherent
backpressure direction, and Linux 7.0 does reset new vsock requests when
`sk_acceptq_is_full` (`virtio_transport_common.c:1544-1546`). Two material
parts remain unspecified.

### Accept rearm must be lossless

The current loop waits only on root shutdown and `io.accept`; `flow_tasks`
completion removes a map entry but exposes no accept-loop wake source
(`owner_runtime.rs:110-140`, `:1346-1349`). A saturated loop that stops calling
`accept` needs a lossless “count below bound” wake/recheck contract. The private
primitive remains the crafter's HOW, but DESIGN must require the observable
liveness and atomicity:

- check/register against the same owner state so the count never exceeds the
  bound;
- no lost completion between the full check and waiting; and
- acceptance resumes after any completion/cancellation makes capacity
  available, unless root shutdown won.

Without that contract, an implementation can remain stopped forever even
after every refusal reader exited.

### The node-wide starvation choice lacks the nearest cheaper alternative

The proposal explicitly permits one guest to occupy all 65,536 refusal slots
and stop accepts for every CID. That is a material reduction of the existing
per-allocation bulkhead's practical isolation, even though refusal readers are
not admitted flows. Explicit user approval can authorize the exposure, but the
decision request must compare the nearest alternative: a per-CID refusal-reader
limit, optionally combined with the global limit, derived from the same
`flow_tasks` owner and requiring no second collection or public API.

The current rejected-alternatives table discusses unbounded reuse, inline
serialization, and a second task map, but not that direct bounded alternative.
The user cannot evaluate the stated cross-CID starvation cost without its
capacity, fd-budget, and fairness tradeoff.

The kernel queue cost must also be stated accurately. Production calls
`listen(i32::MAX)`; Linux clamps it to the appliance netns's `somaxconn`
(`net/socket.c:1918-1924`) and stores that as the vsock backlog
(`af_vsock.c:1920-1941`). That existing finite queue and its connected children
are the saturation buffer before new requests receive RST. The decision should
name the configured bound or state it as the pinned environment input whose
resource cost is separately bounded; “existing AF_VSOCK backlog” alone does
not quantify the new steady full-pool behavior.

### Required remediation

Add to ADMISSION-REFUSAL-1:

1. the lossless accept-rearm invariant above;
2. the actual listener-backlog bound/configuration and resource consequence;
3. a comparison of global-only versus per-CID-plus-global reader caps; and
4. the exact selected isolation/overload outcome and its fd/task budget.

This does not require a public API or another userspace queue. It is the
material behavior and cost the private implementation must realize.

## Retained technical conclusions

### Typed refusal precedence

Iteration-1 F2 is fully closed. The new table preserves current decode
classification before admission refusal, authenticates the token and exact
session before every response, keeps incomplete/read-error/deadline silent,
and suppresses response after CID/session/launch transition. It uses only
existing wire values and does not permit cross-launch delivery.

### Lifecycle ownership

Using `flow_tasks` is structurally compatible with existing teardown,
quiescence, session-loss, and root-shutdown cancellation. The response is fixed
at snapshot but is only attempted after the final identity check; later restore
cannot admit it, and later teardown/session replacement cancels or suppresses
it. Once the count and saturation contract are corrected, no second task owner
is needed.

### Activation priority

ACT-PRIORITY-1 remains technically exact and independently decidable:

```text
valid Provisioned + quiesced                  -> QuiescenceLatched
valid Provisioned + !quiesced + !sessLive    -> ControlSessionPending
valid Provisioned + !quiesced + sessLive     -> Activated
```

The formal change remains only `not(quiesced)` on `ActivatePending`; no other
variant, state, retry, caller, or check changes.

## Authority and next decision gate

Both final decisions remain material under `.claude/rules/design.md`: one
changes task ownership/capacity/admission behavior and the other changes an
internal action's result priority. Technical review cannot approve either for
implementation.

ACT-PRIORITY-1 remains ready for a separate user decision exactly as written in
iteration 1. ADMISSION-REFUSAL-1 is not yet ready: F3 and F4 must be remediated
so its bound, cost, isolation exposure, and liveness are concrete and internally
consistent.

No approval of ACT-PRIORITY-1 may be inferred as approval of the admission
mechanism. If the orchestrator presents activation now, it must be a separate
numbered approve/reject item. Admission requires another bounded re-review
before presentation.

## Roadmap and evidence disposition

Roadmap validation correctly remains `pending` with `approved_at = null`.
No production/model/driver work is authorized.

After F3/F4 remediation and user approval of separate decisions, the existing
evidence obligations remain: exact seeds `0xafd56990` and `0xf23a4188`, every
unchanged named/randomized owner-flow target, complete guest-flow-owner record
with exactly three jobs, `verify-evidence`, and independent proof/source
review. The additional resource-bound, lifecycle, priority, and backpressure
checks must remain private/source-local and use no new public seam, but their
exact count follows the approved decision.

## Iteration 2 verdict

**CHANGES_REQUESTED — RESOURCE_CONTRACT_AMBIGUITY.** The original task owner,
lifecycle cancellation, and typed response-priority gaps are substantially
resolved. The proposal's claimed exact refusal-reader count includes datagram
tasks; its fd-budget formula conflicts with current source and does not state a
single correct before/after value; and its saturation decision lacks a
lossless accept-rearm contract, quantified kernel-backlog cost, and comparison
with the nearest per-CID isolation alternative.

ACT-PRIORITY-1 still has zero technical findings and is separately reviewable.
ADMISSION-REFUSAL-1 is not ready for a user decision or implementation. STEP
12-01 and roadmap validation remain pending. No source/model/test, DES state,
evidence, roadmap approval, or commit was changed by this review.

---

# Review iteration 3 — exact resource contract and user-decision readiness

## Iteration 3 metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_admission_model` |
| Date | 2026-10-26 |
| Role | Same independent solution architecture / formal-model reviewer |
| Reviewed scope | F3/F4 remediation, exact reader identity, fd arithmetic, saturation liveness, kernel backlog, isolation alternative, and both material decisions |
| Technical verdict | **APPROVED FOR PRESENTATION** |
| Open technical findings | 0 |
| Authority verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |
| Admission disposition | Reviewable as ADMISSION-REFUSAL-1; not accepted or executable |
| Activation disposition | Reviewable separately as ACT-PRIORITY-1; not accepted or executable |
| Roadmap disposition | **PENDING / NON-EXECUTABLE** |

Iterations 1-2 remain the historical review record. This iteration rechecks
only the exact remaining resource contract and its effects on the accepted
admission/activation outcomes. It appends this review and changes no proposal,
canonical artifact, roadmap, production/model/driver/test source, DES state,
evidence, or commit.

## F3 remediation — exact count and fd budget

### Reader identity

The revised predicate is exact within `CidState::flow_tasks`:

```text
task.owner_launch == Some(snapshotted_launch) && task.flow == None
```

Existing task shapes establish the partition:

| Task class | `owner_launch` | `flow` | Counted as refusal reader? |
|---|---:|---:|---|
| Ordinary stream/pre-decode flow task | `None` | `None` initially | No |
| Datagram after request decode | `None` | `Some(flow)` | No |
| Refusal-only reader | `Some(snapshot)` | `None` for its life | Yes |

Control tasks also use `owner_launch = Some`, but live in the separate
`control_tasks` map and are outside the predicate. The snapshot field also
serves the required attribution check, so the design does not add a marker,
collection, or observation solely for counting.

The same owner-state lock covers the node-wide count and task insertion, and
the single stream accept loop is the only producer. The proposed count can
therefore never exceed `GUEST_FLOW_MAX_PAIRS = 65,536`.

### FD-budget arithmetic

The revision correctly separates three facts:

| Meaning | Exact required value |
|---|---:|
| Current source check | `4 × 4,096 + 16,384 + 1 + 32,767 = 65,536` |
| Already-accepted canonical baseline | `4 × 65,536 + 16,384 + 1 + 32,767 = 311,296` |
| ADMISSION-REFUSAL-1 target | `5 × 65,536 + 16,384 + 1 + 32,767 = 376,832` |

The pre-existing source use of `GUEST_FLOW_MAX_PAIRS_PER_ALLOCATION` is an
implementation divergence from the already-approved global formula. Correcting
it to 311,296 is existing STEP 12-01 fallout, not a new admission decision.
ADMISSION-REFUSAL-1 then adds exactly 65,536 descriptors and makes
`GuestNetworkError::FdBudget.required = 376,832`.

This distinction gives the user both relevant costs: the proposal's
incremental accepted-baseline cost is 65,536 fds, while the current deployed
source check would rise by 311,296 when the already-required baseline fix and
this decision land together. The symbolic and numeric contracts now agree.

**F3 CLOSED.**

## F4 remediation — lossless saturation and isolation tradeoff

### Accept-loop liveness

The revised contract pins the needed observable concurrency behavior without
dictating a private primitive:

1. count/check and refusal-task insertion share the owner-state lock;
2. a full loop registers completion interest before its final locked recheck;
3. every ordinary removal and bulk take publishes capacity after removal;
4. root shutdown wins over a capacity wake; and
5. every non-shutdown capacity release eventually leads to another locked
   check and `accept` call.

This is sufficient to prevent overflow and lost-wake deadlock. Whether the
crafter uses a private notify, semaphore-like permit, or equivalent wake token
is internal HOW; it may not change the count or observable liveness.

### Kernel backlog

The production listener requests `listen(i32::MAX)`. Linux 7.0 clamps that to
the network namespace's existing `net.core.somaxconn`
(`net/socket.c:1918-1924`) and stores the effective value on the vsock listener
(`af_vsock.c:1920-1941`). Naming that existing environment input `B` is exact:

- ADMISSION-REFUSAL-1 adds no backlog slots or sysctl requirement;
- while userspace refusal capacity is full, at most `B` connected children
  remain queued by that existing listener bound; and
- a request after the queue is full receives the existing vsock RST
  (`virtio_transport_common.c:1544-1546`) with no control refusal because no
  flow id was accepted/read.

No fixed value from a development host is falsely promoted into the appliance
contract, and no new benchmark or kernel configuration is needed for this
existing boundary.

### Global versus per-CID cap

The revised alternatives analysis is adequate. A per-CID 4,096 cap derived
from the same map would not reduce worst-case node task/fd cost, and the single
kernel FIFO accept head cannot skip a saturated CID. At that head the owner
must choose one of three additional behaviors:

- accept and close silently, weakening the existing typed refusal;
- stop all accepts, producing the same global blocking earlier; or
- add per-CID scheduling/queues, a new fairness subsystem no accepted outcome
  requires.

The proposal chooses none of those expansions. It instead surfaces the exact
global-only cost: one guest can occupy all refusal-reader capacity during
quiescence and temporarily stop accepts for every CID until a reader completes
or times out. This is a material isolation exposure, not an inferred harmless
implementation detail, and the user decision names it explicitly.

**F4 CLOSED.**

## Full contract re-review

| Concern | Result |
|---|---|
| QHOLD / zero admitted state | Preserved. Refusal readers exist only in private task ownership, never `guest_flows`, `datagrams`, dials, pairs/watches, quota, or `held_flows`. |
| R5-26 race | Preserved. Accept/admit before hold is registered then aborted; hold before accept snapshots `Quiesced`, never registers, and emits the existing refusal when attribution remains valid. |
| Typed priority | Complete. Decode `Unsupported`/`Malformed` precedes snapshotted admission cause; invalid/stale attribution is silent; stable valid request gets exact `NotActive`/`Quiesced`/`Capacity`. |
| Later lifecycle changes | Complete. Biased cancellation and final launch/session check suppress response after teardown/session/replacement; restore cannot admit a fixed refusal; failed send is not retried. |
| Task lifetime | Complete. Existing `flow_tasks` owns cancellation and join through quiesce, teardown, session loss, and root shutdown. |
| Resource bound | Complete. Exact predicate, global maximum, fd budget, deadline, lossless rearm, kernel backlog, and overflow result are pinned. |
| Performance/isolation | Material cost is explicit: concurrent readers avoid ordinary head-of-line blocking, but one guest may fill the global pool and force kernel-queue backpressure node-wide. |
| Formal honesty | `HostAcceptFlow` represents the complete stable Quiesced refusal branch; admitted reads remain separate; the driver must observe the real message and close without injecting either. |
| Activation priority | Unchanged from prior review and exact. |

No new public API, wire field, enum variant, persistent state, formal action,
or formal state is required. The private wake primitive and reuse of existing
`FlowTask` fields are bounded internal implementation under the approved
resource/ownership contract. No accepted `zero`, `never`, `must`, `exactly`,
fail-closed, attribution, typed response, or cleanup outcome is silently
weakened.

The global isolation reduction is the one explicit contract tradeoff. It may
proceed only if the user approves that exact exposure and cost; reviewer
approval alone supplies no authority.

## Exact reviewable user decisions

The two decisions are independently decidable and must be presented
separately.

### ADMISSION-REFUSAL-1 — bounded non-admitted refusal readers

Approve all of the following as one admission/resource contract:

1. Accept/admission linearizes under the owner lock with quiescence and CID
   lifecycle. Unknown/no-session/Retiring closes silently; otherwise first
   failure is `NotActive`, then `Quiesced`, then `Capacity`; success alone
   registers/takes admitted places.
2. A failed admission creates no admitted flow/place. The existing
   `flow_tasks` map owns a refusal reader identified exactly by
   `owner_launch = Some(snapshot)` and `flow = None`; it reads under
   `FLOW_REQUEST_DEADLINE`, authenticates, sends at most one existing refusal,
   closes, and is canceled/joined by existing lifecycle drains.
3. Decode priority is `Unsupported`/`Malformed` before the snapshotted
   admission cause for an attributable request. Incomplete/error/deadline,
   stale token, or changed CID/launch/session closes silently; send failure is
   not retried or redirected.
4. Node-wide refusal readers are capped at `GUEST_FLOW_MAX_PAIRS = 65,536`
   using the exact task predicate. Full check/insertion are atomic; completion
   rearm is lossless; root shutdown has priority.
5. At saturation userspace stops accepting. The unchanged kernel backlog has
   effective bound `B = net.core.somaxconn`; after its `B` children, Linux
   resets new vsock connects with no control refusal.
6. The already-accepted fd baseline is 311,296; current source's 65,536 check
   is corrected separately to that baseline. This decision adds 65,536 and
   requires exactly 376,832.
7. Runtime cost is at most 65,536 extra concurrent accepted sockets, Tokio
   tasks, cancellation tokens, and 32-byte buffers for one request deadline.
   One guest may occupy the global pool and temporarily block accepts for all
   CIDs. No per-CID scheduling/fairness subsystem is added.
8. The existing formal action/run inventory stays unchanged. Exact seed
   `0xafd56990`, private resource/priority/lifecycle checks, all unchanged
   named/randomized owner-flow targets, the complete three-job formal record,
   `verify-evidence`, and independent replacement review remain mandatory.

This decision changes private resource ownership, node admission capacity,
startup fd requirement, cross-CID saturation behavior, and the internal
`HostAcceptFlow` action contract. It changes no public API or wire shape.

### ACT-PRIORITY-1 — activation overlap priority

For a valid Provisioned CID, approve:

```text
quiesced                  -> QuiescenceLatched
!quiesced && !sessLive    -> ControlSessionPending
!quiesced && sessLive     -> Activated
```

Only `ActivatePending` gains `not(quiesced)`. Variants, state effects, retry
budget, API, production behavior, and check inventory remain unchanged. Exact
seed `0xf23a4188` and the normal full evidence gates remain mandatory. Runtime
and public-surface cost is zero.

## Authority, roadmap, and next gates

Both decisions are material under `.claude/rules/design.md`: one establishes a
new internal capacity/ownership/backpressure contract and explicit isolation
exposure; the other pins an internal action-result priority. This technical
approval makes them reviewable, not accepted.

The orchestrator must obtain separate explicit approve/reject responses. On
approval, canonical text must record the exact decision and date, followed by
a bounded fold re-review before implementation dispatch. Approval of either
decision cannot be inferred as approval of the other.

Roadmap validation remains `pending` with `approved_at = null`. No production,
model, conformance, test, or evidence work is authorized by this review. After
authority and fold gates, implementation still requires exact seeds
`0xafd56990` and `0xf23a4188`, unchanged named/randomized targets, the complete
guest-flow-owner record with exactly three jobs, `verify-evidence`, and
independent proof/source review.

## Iteration 3 verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** F3 and F4 are closed. The
admission proposal now pins an exact existing task owner and reader identity,
finite global bound, correct accepted-baseline/proposal fd arithmetic,
lossless saturation rearm, actual Linux backlog/RST behavior, complete typed
priority, every lifecycle drain, and the global-isolation tradeoff. It is
technically coherent, minimal within the requested outcomes, and has zero open
technical findings.

ADMISSION-REFUSAL-1 and ACT-PRIORITY-1 are ready as separate user decisions.
Neither is accepted or implementation authority. STEP 12-01 and roadmap
validation remain pending. No source/model/test, DES state, evidence, roadmap
approval, or commit was changed by this review.

---

# Review iteration 4 — user authority and canonical fold

## Iteration 4 metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_admission_model` |
| Date | 2026-10-26 |
| Reviewed scope | User authority plus exact canonical folds for `ADMISSIONREFUSAL1` and `ACTPRIORITY1` |
| Prior technical verdict | Iteration 3: zero open findings, approved for presentation |
| Authority verdict | **APPROVED** |
| Final verdict | **APPROVED** |
| Roadmap disposition | **PENDING EVIDENCE**; `approved_at = null` |

## Authority reconciliation

The decision sequence is explicit in the canonical authority table. The user
first said “Approve 240-second exception,” which the immediately preceding
`FLOWQUOTA240` row limits to that timeout decision. After the remaining
reviewed decisions were presented, the user said “i approve the rest of the
decisions.” The 2026-10-26 authority row names both `ADMISSIONREFUSAL1` and
`ACTPRIORITY1`, maps them to the exact separately reviewed decisions, and
repeats their material resource and ordering terms. Both authority gates are
therefore closed by express approval, rather than by inferring approval of one
from the other.

## `ADMISSIONREFUSAL1` canonical fold comparison

The accepted clause under guest-opened flow admission matches the Iteration 3
decision:

| Reviewed contract | Accepted fold | Result |
|---|---|---|
| Admission linearizes with quiescence and CID lifecycle under the owner lock | Same linearization point | Exact |
| Unknown/no-session/Retiring closes silently; then `NotActive`, `Quiesced`, `Capacity`; success alone admits | Same priority and success boundary | Exact |
| Failure creates no admitted flow/place | Only success enters `guest_flows` and takes admitted places | Exact |
| Existing `flow_tasks` owns a non-admitted reader identified by `owner_launch = Some(snapshot)` and `flow = None` | Same existing owner and exact identity | Exact |
| Read under `FLOW_REQUEST_DEADLINE`; decode `Unsupported`/`Malformed` precedes snapshotted cause for an attributable current request | Same deadline, attribution, and typed priority | Exact |
| Incomplete/error/deadline, invalid/stale attribution, or changed CID/launch/session closes silently; failed send is not retried | Same terminal branches | Exact |
| Existing lifecycle drains cancel and join refusal readers | Same ownership and lifecycle closure | Exact |
| Exact global cap 65,536 with atomic insertion/count and lossless rearm | Same cap and wake protocol requirements | Exact |
| At cap, userspace stops accepting; unchanged backlog input `B = net.core.somaxconn`; later connects reset without control refusal | Same existing kernel boundary | Exact |
| Accepted fd baseline 311,296; increment 65,536; exact required value 376,832 | Same arithmetic and startup requirement | Exact |
| One guest may occupy the global pool; no per-CID queue/scheduling/fairness mechanism | Same explicit isolation exposure and rejected expansion | Exact |
| No new public API, wire field, enum variant, formal action, or formal state | Canonical clause expressly excludes each | Exact |

The canonical action map also retains the reviewed `HostAcceptFlow` split: its
failed-admission branch observes the complete buffered request and exact
refusal/close with no admitted state, while the successful branch retains the
existing separate `HostReadReqN` actions. No unreviewed action or production
mechanism appears in the fold.

## `ACTPRIORITY1` canonical fold comparison

The accepted activation clause preserves the exact reviewed result order for a
valid Provisioned CID:

```text
quiesced                  -> QuiescenceLatched
!quiesced && !sessLive    -> ControlSessionPending
!quiesced && sessLive     -> Activated
```

Only existing `ActivatePending` gains `not(quiesced)`. `ActivateQuiesced`,
`Activate`, `ActivateRetiring`, variants, effects, retry budget, callers, and
check inventory remain unchanged. This is the exact zero-runtime-cost proposal;
the fold adds no API, state, action, error variant, retry rule, or production
behavior.

## Evidence disposition

Roadmap validation remains `pending` with `approved_at = null`. Approval and
this fold review do not replace exact seeds `0xafd56990` and `0xf23a4188`,
their unchanged named/randomized targets, the complete three-job
guest-flow-owner record, `verify-evidence`, or independent proof/source review.
This iteration changes only this review artifact.

## Iteration 4 verdict

**APPROVED.** User authority for `ADMISSIONREFUSAL1` and `ACTPRIORITY1` is
exact and dated 2026-10-26. Both canonical folds match the remediated,
zero-finding Iteration 3 proposals, including the global resource/isolation
cost and the activation-result priority. No broader API, wire shape, fairness
policy, formal action/state, scenario, production mechanism, or outcome
weakening was introduced. The prior `USER_DECISION_REQUIRED` dispositions are
closed. Roadmap validation remains pending replacement evidence.
