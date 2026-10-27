# Independent DESIGN/formal review — STEP 12-01 release projection

## Current disposition

| Field | Value |
|---|---|
| Current iteration | 2 — post-approval authority and fold reconciliation |
| Decision | `RELEASEPROJ1` |
| User authority | **APPROVED 2026-10-26** — exact quote: “Approve RELEASEPROJ1” |
| Canonical authority | `feature-delta.md` authority row `RELEASEPROJ1` |
| Canonical contract | QPROJ-3 `RELEASEPROJ1` clause and `owner_flows` QPROJ-2-first `alloc`/`gen` map |
| Technical findings | 0 |
| Final verdict | **APPROVED** |
| Roadmap disposition | Validation remains **pending evidence** with `approved_at = null` |

The Iteration 1 metadata and verdict below record the pre-approval review
state. Iteration 2 at the end of this artifact supersedes only its user-
authority disposition.

## Review metadata

| Field | Value |
|---|---|
| Review ID | `arch_rev_20261026_step_12_01_release_projection` |
| Date | 2026-10-26 |
| Proposal | `docs/feature/netns-density-295/deliver/design-12-01-release-projection.md` |
| Decision | `RELEASEPROJ1` |
| Scope | QPROJ-2/QPROJ-3 meaning of `owner_flows.alloc` and its compared generation after pool release |
| Technical findings | 0 |
| Authority finding | 1 blocking user-decision gate |
| Technical disposition | **APPROVED FOR PRESENTATION** |
| Verdict | **CHANGES_REQUESTED — USER_DECISION_REQUIRED** |
| Roadmap disposition | Validation remains **pending / non-executable** with `approved_at = null` |

This review is bounded to the reproduced release-projection contradiction and
the proposal's existing-state correction. It reads the raw trace, formal
action, production dispatch, pool and owner implementations, accepted
observation contracts, and same-CID late-finish path. It adds only this native
Markdown review artifact. It changes no proposal, canonical design, roadmap,
model, driver, production/test source, evidence, DES state, branch, or commit,
and runs no model, test, Metal, build, mutation, or evidence job.

## Reproduced premise

The native failure is exact and bounded:

- Metal run: `b7880b3e-1cb9-48f6-9e25-fee60bcaa742`.
- Seed: `0xf23a4188`.
- Raw trace 44 ends with `Teardown -> Quiesce -> Release` while control
  connection 1 remains accepted but unread in `reading`.
- After `Release`, model and production agree on the quiescence latch, zero
  claim, every session/flow/dial result, and the retained `reading` connection.
- The sole difference is the combined allocation projection:

```text
model:          alloc = Absent,   lease_gen = None
implementation: alloc = Retiring, lease_gen = Some(1)
```

The production row is real; the lease it currently represents is not. The
trace therefore falsifies only the owner-row-first projection premise. It does
not falsify formal `Release`, accepted-unread teardown, release-last, pool CID
reuse, ATTR-LIVE-CLAIM, or the owner cleanup path.

## Complete production owner path

The proposal cites and preserves the real normal release path:

1. `retire_guest_network_lease` changes the pool lease from `Admitted` to
   `Retiring` before the VMM is stopped.
2. Terminal dispatch stops and releases VMM supervision, stops the allocation's
   mTLS lifecycle, and awaits `teardown_for_dispatch`.
3. `teardown_guest_network` awaits `GuestNetworkProvisioner::teardown`; only
   after `Ok` does it call `GuestAddressPool::release`.
4. Owner teardown marks the CID `Retiring`, clears the claimed session and its
   sender/cancellation authority, drains every flow/dial/datagram and pair, but
   retains a control task whose key is still in `unread`.
5. The teardown postcondition treats that task correctly: it is an accepted
   connection whose first record is not yet attributed, not a live control
   session or admitted flow. Teardown returns `Ok` only after the serving state
   is empty.
6. `GuestAddressPool::release` removes the lease and returns its offset to the
   free set. QPROJ-2 therefore reports no lease for the allocation.

This is the production path the formal `Release` action maps. No alternative
owner, retry, callback, or recovery protocol is needed to explain the state.

## Authority of QPROJ-2 and QPROJ-3

The proposed split assigns each fact to its real owner:

| Fact | Authoritative observation | Review result |
|---|---|---|
| Whether the allocation currently holds a lease | QPROJ-2 `GuestAddressPool::lease_observation().leases` | Exact: `release` removes the row and frees the offset. |
| State and launch of a held lease | QPROJ-2 lease row | Exact for both `Admitted` and `Retiring`. |
| Owner serving phase while an Admitted lease exists | Matching QPROJ-3 CID row | Exact: only `Provisioned` or `Active` is valid. |
| Accepted-unread control-reader ownership | QPROJ-3 `Retiring` row and `unread` | Exact: this is process-local cleanup, not a lease. |
| Model `sess(c).st = reading` | Existing QPROJ-3 `unread` membership | Unchanged. |
| Model `alloc` after pool release | QPROJ-2 absence | Exact: `Absent`, with no compared lease generation. |

The four-case projection is complete at conformance step boundaries:

1. no QPROJ-2 lease means `alloc = Absent` and no compared lease generation;
2. a QPROJ-2 `Retiring` lease means `alloc = Retiring` with that lease launch,
   whether the fully drained QPROJ-3 row has already disappeared or remains as
   the bounded reader cleanup row;
3. a QPROJ-2 `Admitted` lease requires a QPROJ-3 row with the same CID,
   allocation, and launch and with phase `Provisioned` or `Active`; and
4. absence, identity mismatch, or QPROJ-3 `Retiring` under an Admitted lease is
   a conformance failure rather than a fallback.

The proposal's “only when” tombstone clauses also prevent an absent lease from
masking an owner leak. If a QPROJ-3 row remains after teardown/release, it is
valid only as the exact cleanup row reviewed below. This is a state-meaning
restriction, not permission to ignore an arbitrary owner row.

## Cleanup tombstone exactness

The retained row is bounded by the actual teardown and replacement predicates:

| State/resource | Required tombstone value | Production proof |
|---|---|---|
| phase | `Retiring` | Teardown sets it before clearing/draining. |
| live session, sender, cancellation authority | absent | Teardown clears `session`/`session_sender` and takes `session_cancel`. |
| guest flows and flow tasks | empty | Teardown cancels/joins the task drain; its final postcondition rejects residue. |
| flow ids and forward pairs | empty | Teardown aborts/removes every pair and its flow identity; the postcondition rejects a remaining pair. |
| dials, replies, cancels, requests, and dial watches | empty | Teardown cancels dials and removes their watch and auxiliary ownership; it rejects a remaining dial. |
| datagrams and admitted places | empty / zero | Teardown drains them, and `held_flows` consequently remains zero. |
| control tasks | exactly the keys in `unread` | Teardown drains every other control task and retains accepted-unread readers. |
| retained task launch | tombstone launch | `spawn_control` stores `owner_launch = Some(accepted_launch)`. |

Private result caches are not lease, admission, flow, pair, watch, or socket
ownership and remain implementation-owned. They are neither a model input nor
authority to keep forwarding state alive. No new public observation field is
needed: the conformance projection continues to use the existing QPROJ-2 and
QPROJ-3 contract, while the required independent source review verifies the
private cleanup predicates. Exporting private task maps or a new tombstone API
would exceed the proposal.

## Fail-closed behavior after release

The retained row cannot serve traffic or acquire authority:

- `spawn_control` accepts into owner state only when the CID phase is not
  `Retiring`; a new control connection to the tombstone closes.
- flow admission requires a non-Retiring row and live session attribution; the
  tombstone has neither.
- activation rejects an unknown or `Retiring` CID and cannot convert the row
  back to `Active`.
- reading the retained connection compares both the task's accepted launch and
  wire token with the current live row. A Retiring old row yields no claim; a
  replacement row yields `StaleLaunch`.
- the row holds no flow, datagram, dial, pair, watch, attachment place, or pool
  offset.

Reporting `alloc = Absent` therefore does not conceal an admission-capable
allocation. It reports the pool fact while `sess(c).st = reading` continues to
report the independent cleanup task fact.

## Same-CID replacement and late finish

The one-CID fixture exercises the important reuse boundary. Production
`provision` replaces a prior row only when it is `Retiring`, has a different
launch, has no session or serving resources, and contains only accepted-unread
control tasks with their old `owner_launch`. It transfers those task handles
and unread keys into the new row without relabelling them.

The old reader remains generation-safe:

1. its `accepted_launch` remains the old launch;
2. the new row's `launch` is different;
3. the read closes as stale before taking a claim;
4. `finish_connection` removes a task only when that task's `owner_launch`
   matches its recorded accepted launch;
5. it clears a session only when the row launch also matches; and
6. it removes a Retiring row only when that row still has the same old launch.

Consequently a late old finish removes only its own task and unread key. It
cannot clear the replacement session, claim for the new launch, delete the new
row, or poison a later session. If it finishes before replacement, it removes
the empty Retiring tombstone; QPROJ-2 still independently reports the held
Retiring lease until pool release.

## Formal-model congruence

The existing model already expresses every required result:

- `Teardown` changes the lifecycle to `Retiring`, closes the claimed session
  and flows, and preserves a `reading` connection;
- `Release` requires the VMM and driver down, changes only `alloc` to `Absent`,
  and preserves session state;
- `HostReadOpen` closes a retained reader when no live matching launch exists;
  and
- `Provision` creates the next generation, after which an old reader still
  fails its launch comparison.

Changing the model to retain `alloc = Retiring` after `Release` would make the
lease and compared generation disagree with QPROJ-2 and would weaken the
release-last/CID-reuse meaning. Cancelling the reader at teardown would remove
the accepted ATTR-LIVE-CLAIM state. The projection correction is therefore
both necessary and smaller than either alternative.

No Quint state, action, effect, guard, run, scenario, hazard, temporal
property, check, or trace filter changes. The existing driver changes only the
`alloc`/compared-generation selection and its precise documentation. Product
behavior and public type shapes remain unchanged.

## Outcome and invariant preservation

The proposal preserves the accepted contracts together:

- `pool.release` means zero lease and a reusable CID offset;
- a Retiring pool lease exists only until that release;
- formal `Release` reaches `Absent` and retains the accepted-unread session
  abstraction;
- teardown returns only after every claimed session and forwarding resource is
  gone;
- a reader accepted before teardown closes fail-closed without a claim;
- stale work cannot cross a launch generation;
- no row, task, session, or lease survives process exit; and
- driver history or shadow state never substitutes for QPROJ-2/QPROJ-3 facts.

No `zero`, `never`, `must`, exact cleanup, security, release-last, or
generation-isolation outcome is weakened. The proposal separates two facts
that the current projection incorrectly conflates; it does not create a
second lifecycle system.

## Materiality and user authority

The runtime/API/wire/storage and formal-state costs are zero. Evidence cost is
the already-required exact seed replay, unchanged owner-flow targets,
three-job record, `verify-evidence`, and independent proof/source review.

The change is still material because it amends the accepted meaning of the
existing `owner_flows.alloc` projection and the QPROJ-3 `Retiring` observation.
`.claude/rules/design.md` states that a proposal is not evidence of user
acceptance and requires explicit approval before a material decision is marked
accepted or carried into DISTILL/DELIVER; the rule expressly includes the
meaning of an existing state or internal action/port contract. Technical review
cannot supply that authority.

## Exact reviewable user decision

### RELEASEPROJ1

Approve this bounded projection/state-meaning correction:

> Make QPROJ-2 pool lease presence and generation authoritative for
> `owner_flows.alloc` and its compared generation. An absent pool lease
> projects `Absent` with no compared lease generation; a Retiring pool lease
> projects `Retiring` with its launch; an Admitted pool lease requires the
> matching QPROJ-3 CID/allocation/launch in `Provisioned` or `Active`. Permit a
> QPROJ-3 `Retiring` row after teardown and pool release only as the exact
> serving-state-empty, fail-closed accepted-unread cleanup tombstone. Preserve
> its `reading` projection, generation-safe same-CID transfer and late finish,
> existing formal actions/state/runs/checks, production behavior, and public
> API/wire shapes.

Approval authorizes no new release callback or method, owner, public tombstone
type, observation map, formal state/action/scenario, cleanup policy, trace
filter, or product behavior.

## Roadmap and evidence disposition

`deliver/roadmap.json` validation remains `pending` with `approved_at = null`.
If the user approves `RELEASEPROJ1`, implementation still requires:

- exact replay of seed `0xf23a4188`, including Trace 44;
- the unchanged teardown and stale-launch named runs;
- every unchanged `owner_flows` named and randomized target;
- the complete guest-flow-owner record with exactly three jobs;
- `verify-evidence`; and
- independent source/proof review of the projection and private tombstone
  predicates.

No canonical acceptance or roadmap-validation approval follows from this
technical review.

## Final verdict

**CHANGES_REQUESTED — USER_DECISION_REQUIRED.** The proposal has zero open
technical findings. QPROJ-2 is the correct owner of lease presence and
generation; QPROJ-3 remains the correct owner of the bounded accepted-unread
cleanup reader; the four-case projection is complete; and the actual teardown,
same-CID replacement, and late-finish paths preserve release-last, fail-closed
admission, generation isolation, and zero forwarding residue. `RELEASEPROJ1`
is approved for presentation exactly as written above, but is not yet accepted
or implementation authority.

---

# Review iteration 2 — user authority and canonical fold

## Authority reconciliation

After the zero-finding independent review presented the exact four-case
QPROJ-2/QPROJ-3 correction, the user explicitly said “Approve RELEASEPROJ1” on
2026-10-26. The canonical authority table records that exact quote and decision
identifier. The authority gate is therefore closed without inference from any
other decision.

## Canonical fold comparison

The accepted `RELEASEPROJ1` clause in `feature-delta.md` and the accepted
`owner_flows` projection table match the reviewed proposal:

| Reviewed contract | Accepted fold | Result |
|---|---|---|
| QPROJ-2 owns `owner_flows.alloc` lease presence and its compared generation | Same QPROJ-2-first authority | Exact |
| No pool lease projects `Absent` and no compared generation | Same absent case, including a retained cleanup row | Exact |
| A Retiring pool lease projects `Retiring` with that pool launch | Same Retiring case | Exact |
| An Admitted pool lease requires matching QPROJ-3 CID/allocation/launch in `Provisioned` or `Active` | Same positive requirements | Exact |
| Missing/mismatched QPROJ-3 row or `Retiring` under an Admitted lease is a conformance failure | Same rejection behavior | Exact |
| QPROJ-3 may outlive release only as the exact serving-state-empty accepted-unread cleanup row | Same session, task, flow, dial, datagram, pair, watch, held-place, and launch restrictions | Exact |
| Cleanup row admits nothing, cannot activate or claim, and still projects the old connection as `reading` | Same fail-closed meaning | Exact |
| Same-CID replacement transfers old unread/task ownership without relabelling `owner_launch`; late finish cannot affect the new launch | Same generation-safe replacement and finish contract | Exact |
| Existing formal state/actions/runs/checks and production behavior remain | Same unchanged formal and runtime surface | Exact |
| No API, wire, callback, owner, formal input/action/scenario/hazard/check, or validation approval | Canonical authority and contract expressly exclude these expansions | Exact |

The fold adds no outcome or mechanism beyond the reviewed decision. It retains
release-last and offset reuse, accepted-unread fail-closed cleanup,
ATTR-LIVE-CLAIM, same-CID generation isolation, NO-SURVIVE, and zero forwarding
residue.

## Evidence disposition

`deliver/roadmap.json` validation remains `pending` with `approved_at = null`.
User approval and this fold review do not replace:

- exact replay of seed `0xf23a4188`, including Trace 44;
- the unchanged teardown and stale-launch named runs;
- every unchanged `owner_flows` named and randomized target;
- the complete guest-flow-owner record with exactly three jobs;
- `verify-evidence`; or
- independent source/proof review of the projection and private tombstone
  predicates.

This iteration changes only this review artifact.

## Iteration 2 verdict

**APPROVED.** User authority for `RELEASEPROJ1` is exact and dated
2026-10-26. The canonical fold matches the zero-finding reviewed four-case
projection and cleanup-tombstone contract without extra API, wire, model,
runtime, ownership, cleanup, or outcome changes. The prior
`USER_DECISION_REQUIRED` disposition is closed. Roadmap validation remains
pending replacement evidence.
