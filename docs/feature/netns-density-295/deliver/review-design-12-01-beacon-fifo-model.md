# Independent DESIGN/formal review — STEP 12-01 beacon FIFO model identity

## Current disposition

| Field | Value |
|---|---|
| Current iteration | 2 — post-approval authority and fold reconciliation |
| Decision | `BFIFO1` (reviewed as `BFIFO-1`) |
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
| Review ID | `arch_rev_20261026_step_12_01_beacon_fifo` |
| Date | 2026-10-26 |
| Role | Independent solution architecture / formal-model reviewer |
| Iteration | 1 |
| Reviewed proposal | `docs/feature/netns-density-295/deliver/design-12-01-beacon-fifo-model.md` |
| Technical findings | 0 |
| Authority finding | 1 blocking user-decision gate |
| Verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |
| Technical disposition | **APPROVED FOR PRESENTATION** — exact proposal is coherent and reviewable |
| Roadmap disposition | **PENDING / NON-EXECUTABLE**; `approved_at = null` |

This review applies the repository DESIGN, production-reachability,
native-falsification, exact-contract, no-unrequested-scope, and no-weakening
rules together with `nw-design`, `nw-formal-verification-tlaplus`,
`nw-sar-critique-dimensions`, and `nw-roadmap-review-checks`. The required
native-Markdown artifact governs over the reviewer definition's legacy YAML
format.

The review is bounded to the reproduced beacon conformance failure, the
proposed `owner_flows` FIFO abstraction, its private conformance mapping, the
accepted D-UP-19 and beacon lifecycle contracts, and the pending canonical and
roadmap annotations. It creates only this review artifact. It does not edit the
proposal, canonical design, roadmap, model, driver, production source, tests,
DES state, or evidence; run a model, Metal, test, build, mutation, or record
command; or create a commit.

## Reproduced premise and source verification

The retained raw log
`.context/netns-density-295/12-01/quint-flows-beacon-randomized-seed-2331472b-verbose1.log`
records Metal run `50c02457-e6a3-49f4-a701-d22376d53bcc` and
`QUINT_SEED=0x2331472b`. Its trace opens beacon connection 2 before connection
1, stops the VMM, and then selects `HostAcceptBeacon(1)`:

```text
Init
  -> Provision
  -> VmStart
  -> VmStop
  -> Provision
  -> VmStart
  -> GuestOpenBeacon(2)
  -> GuestOpenBeacon(1)
  -> VmStop
  -> HostAcceptBeacon(1)
```

Raw-log lines 21707-21741 show connection 2 becoming pending first; lines
21743-21777 show connection 1 appended second. `VmStop` retains both pending
generation-2 connections at lines 21779-21812. The current model then closes
connection 1 at lines 21814-21848. The actual Sim listener refuses that
fabricated identity at lines 21850-21864 because connection 2 is the FIFO
head. The reproduction is exact and deterministic.

The current model is the cause. `HostAcceptBeacon(b)` is enabled by
`bcn(b).st == "pending"` alone and may choose either bounded connection id.
The current private driver `accept_order` records the real open order and
rejects a model-selected non-head id before releasing one accept. That
bookkeeping prevents a false comparison, but it cannot make the model action a
production transition.

The FIFO fact is independently grounded at every real boundary:

| Boundary | Verified behavior |
|---|---|
| Production owner | `BeaconIntake::bind` starts one `run_accept_task`; it awaits one `BeaconIo::accept()` at a time and passes that returned child identity to the asynchronously spawned read task. Read concurrency does not choose or reorder accepted children (`beacon_intake.rs:58-68`, `:128-173`). |
| Host adapter | `VsockBeaconIo::accept` executes one `accept_with` and returns the peer identity of that exact accepted child (`overdrive-host/src/beacon_io.rs:128-167`). |
| Linux 7.0 | A connected vsock child is linked at the accept queue tail (`af_vsock.c:499-510 @ v7.0`); dequeue removes `accept_queue.next` (`:688-707`); `vsock_accept` uses that dequeue (`:1824-1878`). The listener queue is FIFO. |
| Sim adapter | `SimBeaconIo` owns `State.queue: VecDeque<u64>`; `queue_connection` uses `push_back`, and `try_accept` uses `pop_front` (`beacon_io.rs:112-128`, `:208-242`, `:361-383`). |

The proposal therefore removes a model-superset trace the real port cannot
execute. It does not infer production policy from the Sim adapter: Linux and
the production adapter establish the contract, and the Sim adapter already
matches it.

## Exact formal-contract review

### Queue state and bounds

The proposed single model variable is sufficient:

```quint
var bq: List[int]
```

`bq` is explicitly model-only. It records the ordered history of the existing
`GuestOpenBeacon` and `HostAcceptBeacon` actions. It is not a runtime queue,
owner observation, driver copy, or new product API. The bounded beacon instance
has `BCONNS = Set(1, 2)`, and the no-duplicate invariant limits `bq` to:

```text
[], [1], [2], [1,2], [2,1]
```

For a fixed existing `bcn` pending-membership state, only the two-both-pending
case gains a second ordering. The state-space multiplier introduced by order
is therefore at most two. There is no production runtime or hot-path cost.

### Transition closure

| Existing transition | Reviewed `bq` effect | Assessment |
|---|---|---|
| `InitWith` | Empty list. | Exact. No queue survives model initialization. |
| `GuestOpenBeacon(b)` | Preserve every current guard/effect and append `b` once. | Exact. The current phase guard prevents opening an id that is still pending/reading/live; a reused closed id was previously accepted and removed. |
| `HostAcceptBeacon(b)` | Require nonempty queue with `bq.head() == b`, keep the existing live-driver `reading` versus stopped-driver `closed` result, and remove one head. | Exact. One action remains one sequential `BeaconIo::accept`; no arbitrary child selection is retained. |
| `VmStop` | Preserve queue. | Exact. It closes accepted `reading`/`live` children, which have already left `bq`; unaccepted children survive with buffered READY and original generation under K-A2-prime/P-37. |
| `Provision`, `VmStart` | Preserve queue. | Exact. Lease/generation changes do not dequeue stale listener children. |
| `HostReadReady` | Preserve queue. | Exact. Token/claim decisions occur after accept. |
| `DriverStop` | Preserve queue. | Exact. It closes the live claimed beacon and owns no pending listener child. |
| Non-beacon actions/helpers | Preserve `bq` beside the existing beacon state; non-beacon instances remain empty. | Complete. Adding `bq` to `keepB`/the explicit lifecycle assignments closes every action without enabling beacon behavior outside `CFG = "beacon"`. |

No action removes a non-head item, sorts ids, or clears stale queue entries at
a generation boundary. This preserves all current lifecycle branches:

- a queued dead-launch child remains at its position and is accepted before a
  later child;
- an accepted child has already left the queue before `VmStop`, READY rejection,
  claim rejection, or `DriverStop` closes it;
- a stale head is accepted and then follows the existing `NoLiveLaunch` or
  token/live-launch refusal;
- a new generation may enqueue behind stale children; and
- id reuse can append only after the prior occurrence was accepted, removed,
  and closed.

### Structural safety and fairness

`BeaconQueueExact` is an appropriate structural clause of the existing
`BeaconSafety`:

- every queue element is one of the two bounded connection ids;
- ids are unique; and
- queue membership is exactly `bcn(b).st == "pending"`.

This catches every missing append, pop, lifecycle preservation, and illegal
duplicate. The transition rule itself establishes order. Its production teeth
are the exact randomized conformance replay: weakening `HostAcceptBeacon` to
any pending id reproduces the driver refusal because the accepted child's real
identity changes the existing `bcn` projection.

Adding `bq` to `allVars` is required and sufficient for the existing weak-
fairness formulas. Per-connection `HostAcceptBeacon(b)` fairness remains
coherent: a non-head action is correctly disabled; after earlier heads are
removed, it becomes enabled and weak fairness applies. No queued child is
starved by the FIFO restriction, and no new liveness property or fairness
assumption is introduced.

The check inventory may remain unchanged. `BeaconSafety` and the existing
beacon safety/liveness checks cover the new structural clause; the existing
randomized driver supplies the concrete FIFO identity falsifier. A new named
scenario, hazard family, lane, or check would duplicate those evidence layers
and increase validation cost without a new outcome.

## Conformance honesty and driver simplification

Removing private `accept_order: VecDeque<i64>` is correct and necessary. The
model queue, not test bookkeeping, decides when `HostAcceptBeacon(b)` is an
allowed transition. The driver action continues to release one real accept and
does not receive a chosen-id seam.

The existing post-step projection is enough to prove identity:

- with a live driver launch, the actual returned `BeaconConnKey` appears in
  `BeaconObservation.unread` and projects that exact id as `reading`;
- without a live launch, the actual socket appears in QPROJ-7
  `closed_by_driver` and projects that exact id as `closed`; and
- `SimBeaconSocketObservation.queued` continues to project pending membership,
  never order.

If the model and real queues ever diverge, the wrong id becomes
`reading`/`closed` or remains `pending`, and the ordinary state comparison
fails. No model state is projected from itself, and no driver shadow queue,
numeric sort, trace filter, retry, or identity relabelling is required.

This is not an infeasible-trace filter. `GuestOpenBeacon(2)` followed by
`GuestOpenBeacon(1)` remains valid and creates `[2,1]`; VMM and generation
transitions preserve it. Only `HostAcceptBeacon(1)` while 2 is still the head
is removed, because neither Linux, the production port, nor the Sim port can
perform that transition.

## D-UP-19 boundary and retained outcomes

The replacement is correctly narrow. D-UP-19 may continue to abstract accept
order for owner models whose invariants and conformance actions do not select a
specific connection identity from a one-at-a-time accept port. Only
`owner_flows` beacon acceptance needs the exception because
`HostAcceptBeacon(b)` names the child while the production operation returns
the FIFO head.

Boot reclamation remains one unordered `Reclaim` action. R5-59 remains a
per-connection, order-insensitive adapter-equivalence test. Neither contract
authorizes the beacon formal driver to choose an identity the real accept did
not return.

All accepted product outcomes remain unchanged:

| Existing contract | Result |
|---|---|
| K-A2-prime / P-37 | Preserved. Pending child and buffered READY survive VMM death and CID reuse. |
| ATTR-TOKEN | Preserved. READY is checked against the exact accepted child's retained generation. |
| ATTR-LIVE-CLAIM | Preserved. A stale or no-live-launch child cannot take/release another launch's claim. |
| D-UP-9 | Preserved. VMM exit ends live launch/claim state but does not own pending listener children. |
| Beacon safety/liveness | Preserved. Current result states and fairness remain; only physically impossible accept identity choices are removed. |
| Production and API surface | Unchanged. Runtime already uses FIFO; no public/internal production port, type, method, parameter, config, wire format, or persistence changes. |

No `zero`, `never`, `must`, `exactly`, fail-closed, forbidden, required,
security, attribution, ownership, or cleanup outcome is relaxed. The proposal
prevents a model-selected child from receiving another child's token/close
outcome; it strengthens formal identity fidelity to the existing product.

## Scope, alternatives, and cost audit

The proposal identifies the reachable failure and rejects the cheaper-looking
incorrect alternatives with evidence:

- reinterpretation fabricates identity;
- numeric sorting rejects real enqueue schedules;
- trace filtering hides an enabled model transition;
- retaining `accept_order` makes the harness a shadow source of truth;
- a public ordered observation or chosen-id accept adds unnecessary surface;
- generation-boundary clearing contradicts K-A2-prime; and
- a new action/scenario/runtime reorderer duplicates the existing transition.

The cost presented with the user decision must be explicit:

| Cost class | Bound |
|---|---|
| Product runtime / hot path | Zero. No production code or runtime state changes. |
| Public/API/storage/wire | Zero. `bq` exists only in Quint and no schema or port changes. |
| Conformance support | Net simplification: remove one private `VecDeque` shadow and its pre-step refusal. No replacement bookkeeping. |
| Check/lane count | Zero growth. Same checks, named runs, randomized target, three-job record workflow, and lanes. |
| Formal state space | At most two orderings for the only existing state with both bounded ids pending; actual TLC/Apalache state and wall-time deltas must be reported by the required replacement record. |

This quantitative cost context is not stated as its own table in the proposal,
but it follows exactly from the proposed `BCONNS = {1,2}` bound and unchanged
check inventory. The architect/orchestrator must include it in the user-facing
decision so the approval satisfies `.claude/rules/design.md` § *No unrequested
scope*; no proposal redesign is required.

## Blocking authority gate

The author is correct that explicit user approval is required. The applicable
repository rule is `.claude/rules/design.md` § *Surface material design
decisions to the user before they become accepted*:

> “A design document is a record of a proposal, not evidence that the user has
> accepted it.” Before a material design decision is marked accepted, carried
> into DISTILL, or used to authorize DELIVER, it must be surfaced to the user
> and explicitly approved.

The rule expressly applies to an amendment that changes:

> “a public or internal action/port contract”

and to:

> “persistence, retry, ordering, recovery, compatibility, or migration
> behavior.”

The interpretation is narrow but decisive. This proposal changes no product
outcome and no production port: production FIFO is already fixed. It does,
however, amend the accepted D-UP-19 ordering abstraction and the internal
formal action contract of `HostAcceptBeacon(b)` from “any pending connection”
to “the FIFO head.” It also adds model state whose only purpose is enforcing
that ordering. Those exact categories are named by the rule, so technical
reviewer agreement cannot substitute for user authority.

The current request authorizes independent review, not approval of a numbered
D-UP-19 amendment. The pending proposal/canonical/roadmap prose and this review
are therefore not implementation authority. Under the rule's reviewer clause,
the verdict must remain `CHANGES_REQUESTED` until the exact decision is
explicitly approved.

## Exact reviewable user decision

The proposal is technically ready to be presented as one approve/reject
decision:

### BFIFO-1 — bounded D-UP-19 beacon FIFO exception

Approve the following exact formal-contract amendment:

1. `owner_flows` beacon acceptance records one model-only FIFO `bq` over the
   existing bounded ids `{1,2}`.
2. Existing `GuestOpenBeacon(b)` appends; existing `HostAcceptBeacon(b)` is
   enabled only for the head and removes exactly that head.
3. `VmStop`, generation changes, `DriverStop`, READY processing, and every
   unrelated action preserve queued pending children; stale children remain
   ahead of later connections.
4. `BeaconQueueExact` joins existing `BeaconSafety`, and `bq` joins `allVars`;
   no new check, run, scenario, hazard, lane, action, or fairness policy is
   added.
5. The beacon conformance driver removes `accept_order` and proves accepted
   identity through the existing unread/closed post-state projections after
   releasing one real FIFO accept. It adds no shadow state or API.
6. D-UP-19 remains unchanged for reclamation and every other order-insensitive
   accept model; R5-59 remains per-connection and order-insensitive.
7. Product behavior and public/internal production interfaces remain
   unchanged. Runtime/API/wire/storage cost is zero; check/lane count is
   unchanged; the bounded formal ordering adds at most a factor of two for the
   both-pending membership state, with actual record cost to be reported.
8. Implementation does not authorize roadmap execution. Seed `0x2331472b`,
   unchanged named/randomized beacon conformance, the complete three-job
   guest-flow-owner record, `verify-evidence`, and independent replacement-
   evidence review must pass before roadmap validation is approved.

The affected boundaries are only the D-UP-19 formal abstraction,
`owner_flows` beacon state/action enabledness, and private conformance support.
The rejected alternatives are those already recorded in the proposal; none
preserves both production identity and existing surface as cheaply.

## Roadmap and next-gate disposition

Roadmap validation correctly remains `pending` with `approved_at = null`.
Before any implementation or DISTILL/model handoff:

1. the orchestrator must present BFIFO-1 and obtain an explicit approve/reject
   response;
2. on approval, the canonical D-UP-19 exception must record that exact user
   authority and date; and
3. a bounded post-approval re-review must verify that the accepted fold equals
   BFIFO-1 before implementation is dispatched, consistent with the rule's
   required approval-then-independent-review sequence.

Only after those authority gates may bounded implementation begin. Because the
Quint input then changes, completion still requires exact-seed replay, every
unchanged named/randomized beacon conformance target, the complete
guest-flow-owner record with exactly three jobs, `verify-evidence`, and
independent proof/source review. The historical record cannot become current
before those gates.

## Iteration 1 verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** The proposal is technically
coherent, minimal, bounded, grounded in the real Linux/production/Sim FIFO, and
complete across the accepted beacon lifecycle. `bq`, `BeaconQueueExact`, the
head-only existing action, `allVars`, and removal of the private driver queue
form an honest model/conformance correction with zero open technical findings.
No product outcome, runtime behavior, production interface, state owner, or
security invariant changes.

The proposal is approved for presentation as BFIFO-1, not accepted into the
architecture and not implementation authority. The accepted D-UP-19 internal
action/ordering contract changes, so `.claude/rules/design.md` requires the
user's explicit approval despite the absence of a user-visible outcome. The
roadmap remains pending and STEP 12-01 remains non-executable. No model,
driver, production/test source, DES state, evidence, roadmap approval, or
commit was changed by this review.

---

# Review iteration 2 — user authority and canonical fold

## Authority reconciliation

The decision sequence is explicit in the canonical authority table. The user
first said “Approve 240-second exception,” which the immediately preceding
`FLOWQUOTA240` row limits to that timeout decision. After the remaining
reviewed decisions were presented, the user said “i approve the rest of the
decisions.” The 2026-10-26 authority row names `BFIFO1` among those remaining
decisions and maps it to the exact `BFIFO-1` reviewed here. Authority is no
longer inferred or pending.

## Canonical fold comparison

The accepted `BFIFO1` clause in `feature-delta.md` under the `owner_flows`
beacon action map matches the reviewed proposal:

| Reviewed contract | Accepted fold | Result |
|---|---|---|
| Model-only `bq: List[int]` over existing ids `{1,2}` | Same type and bound | Exact |
| `GuestOpenBeacon(b)` appends | Same existing action and effect | Exact |
| `HostAcceptBeacon(b)` requires and removes the head | Same existing action guard/effect | Exact |
| Lifecycle and unrelated actions preserve stale queued children | `VmStop`, generation change, `DriverStop`, READY processing, and unrelated actions preserve them | Exact |
| `BeaconQueueExact` joins `BeaconSafety`; `bq` joins `allVars` | Same invariant and variable closure | Exact |
| Driver removes private `accept_order` and observes real FIFO identity through existing post-state | Same `unread`/`closed_by_driver` projection | Exact |
| No product/API/action/scenario/lane/fairness expansion | Canonical clause expressly excludes each | Exact |

The fold retains D-UP-19 for all other order-insensitive accepts and R5-59 as
per-connection and order-insensitive. It adds no API, production behavior,
observable outcome, or unreviewed ordering policy.

## Evidence disposition

Roadmap validation remains `pending` with `approved_at = null`. Approval and
this fold review do not replace exact seed `0x2331472b`, every unchanged
named/randomized beacon target, the complete three-job guest-flow-owner record,
`verify-evidence`, and independent proof/source review. This iteration changes
only this review artifact.

## Iteration 2 verdict

**APPROVED.** User authority for `BFIFO1` is exact and dated 2026-10-26, the
canonical fold matches the zero-finding `BFIFO-1` proposal, and no broader API,
outcome, action, scenario, or mechanism was introduced. The prior
`USER_DECISION_REQUIRED` disposition is closed. Roadmap validation remains
pending replacement evidence.
