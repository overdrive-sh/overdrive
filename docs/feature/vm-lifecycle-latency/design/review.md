# Independent DESIGN Review — VM lifecycle latency (#283 / shared convergence #260)

## Metadata

| Field | Value |
|---|---|
| Review role | `nw-solution-architect-reviewer` |
| Review scope | Proposed lifecycle responsiveness and shared convergence ownership only |
| Design inputs | `feature-delta.md`, ADR-0102, ADR-0103, focused architecture brief and C4 proposal |
| Evidence inputs | Research, approved RCA review, issue #283/#260, accepted lifecycle/convergence ADRs, current production owner paths |
| Iteration | 1–2 |
| Review state | Complete after iteration 2 re-review |
| Verdict | **APPROVED** |

This is a review of a proposed package awaiting user ratification. The verdict
does not treat the pending policy choices as defects and does not imply that the
proposed implementation has run.

## Review conclusion (Iteration 1)

The proposal selects the right bounded seams for the two demonstrated defects.
It preserves the workload aggregate View, the existing runtime and action-shim
ownership, the independent exit observer, the `EndingInFlight` claim, the
accepted READY/Running/EXEC order, and the narrow generic stop result. The
research premise is quantified honestly: the native Service-stop trace
attributes the observed delay to the fixed request window and VMM grace, while
the writer timing is not presented as guest receipt. The design also keeps
Sim, in-process production tests, native substrate evidence and black-box
expectations at distinct boundaries, and retains the exact E09-v2 all-trial
rerun obligation.

One blocking contract boundary remained unspecified. The proposed shutdown
report can be emitted before a still-live production broker producer has joined,
so `pending_at_exit` is not defined as either the convergence-owner snapshot or
the final server-shutdown disposition. The implementation and acceptance test
cannot choose that boundary without inventing semantics. This is a design
contract completeness finding, not a claim that the proposed code has already
exhibited a runtime failure.

## Findings

### F-01 — Blocking: pending-work reporting has no final shutdown boundary

**Severity:** High / blocking for ratification

**Locations:** ADR-0102 “Timing, completion and shutdown” and
“Observability”; feature delta G-1, executable-obligation “Shutdown ownership”.

The proposal makes these requirements simultaneously:

1. On cancellation, the convergence owner closes admission, drains every
   admitted evaluation to its real result, and “reports remaining pending work”;
   submissions and self-requeues arriving during that drain “also stay
   pending” (ADR-0102 lines 148–154).
2. The `convergence.drain.completed` event exposes only the owner-side fields
   `admitted_at_close`, `completed_during_drain`, and `pending_at_exit`
   (ADR-0102 lines 167–180).
3. The existing shutdown order is preserved after the convergence join:
   workflow emit-drain joins, then interest-router joins, then HTTP/server
   teardown (ADR-0102 lines 156–162). The feature delta repeats this as the
   required shutdown obligation and says nonadmitted work is “reported, not
   run” (feature-delta lines 292–303).

The production owner path makes the two observation boundaries materially
different. `ServerHandle::shutdown` cancels and joins the convergence task
first, then cancels and joins the workflow emit-drain and interest-router tasks
(`crates/overdrive-control-plane/src/lib.rs:1406–1432`). The production
`InterestRouterBroker` is a submit-only capability into the same broker drained
by convergence (`lib.rs:3425–3463`), and accepted observation rows can submit
new workload evaluations through `route_observation_row` (`lib.rs:3465–3525`).
Therefore the broker can receive work while the admitted evaluations are being
drained and can still receive work after the convergence owner has produced its
exit snapshot but before the producer tasks are joined. The proposal itself
explicitly allows submissions during the drain to remain pending.

As written, an implementer cannot tell which of these contracts is intended:

- `pending_at_exit` is a convergence-owner snapshot, in which case later
  producer submissions are outside the required report; or
- “remaining pending work” means the final server-shutdown backlog, in which
  case no report boundary or existing owner is specified after the producer
  joins.

The ambiguity affects the observable shutdown disposition and the acceptance
obligation. It also prevents a truthful assertion about the reported count or
keys without adding an unapproved reporting mechanism. This finding does not
require persistence, recovery, a new scheduler, or a new public API.

**Required disposition:** Amend ADR-0102 and the feature delta to pin one exact
boundary and its semantics. State whether `pending_at_exit` is deliberately
the convergence-owner exit snapshot (including how submissions after that
snapshot are treated), or specify the existing shutdown step that obtains and
reports the final pending disposition after emit-drain and interest-router
joins. Align the “reported, not run” acceptance assertion and event fields with
that choice. Keep the chosen disposition process-local as the proposal
requires; do not introduce persistence or recovery in this feature.

**Evidence classification:** This is an exact proposed-contract ambiguity
grounded in the current caller/owner ordering. No proposed implementation was
run, and no runtime defect is being asserted. No new timing invariant is
claimed; the required change is to define the report boundary before
implementation and test design proceed.

## Review conclusion (Iteration 2 re-review)

The architect's correction closes F-01. ADR-0102 now defines `pending_at_exit`
as one count of the coalesced pending entries returned by the existing broker's
`counters().queued` under the broker lock, taken only after all admitted results
are consumed. It explicitly defines submissions linearized after that snapshot
as unexecuted, process-local pending work outside the unchanged report, and
states that there is no final server-backlog report or producer barrier.

The same contract is carried into the feature delta's G-1 failure projection
and its shutdown executable obligation, and the focused architecture brief
states the owner-snapshot/excluded-later-submissions rule. The event field is
also annotated with the same meaning. These clauses resolve both sides of the
original ambiguity: the report is deliberately the convergence-owner exit
snapshot, while later submissions have a defined disposition and are not
misreported as completed. No new API, persistence, recovery, scheduler or
shutdown mechanism is introduced.

The correction is necessary and sufficient for the original finding. It does
not expand the shutdown scope or alter the preserved producer join order,
observer retry semantics, deadlines, or pending user policy choices. The
remaining design package is internally consistent on the directly affected
boundary and is ready for user ratification.

## Scope and design-quality assessment

### Problem and priority

The largest measured obstruction is correctly selected. The feature delta
records the real path from HTTPS submit/stop through the broker, convergence
owner, runtime, shim and driver, and records the native healthy Service-stop
trace as approximately 2,002 ms of fixed request waiting plus approximately
10,018 ms of VMM grace. Seed `283001` is identified as the production-owner
reproduction for held start/stop blocking an unrelated Running workload, with
healthy/released controls. No performance quantile is falsely claimed.

The simpler alternatives are addressed: an inner driver pool would leave the
outer serial evaluation await; unkeyed concurrency would permit overlapping
workload Views; per-allocation actors would create new queue/retirement
ownership; an ACK would not repair the measured host wait; and strengthening
the generic stop result would cross the existing cleanup/error ownership. The
proposal keeps the remedy at the proven complete-evaluation and guest PID 1
boundaries.

### Ownership, state and conflict semantics

The Lifecycle Gate Ownership matrix is present and uses precise owners and
states. In particular, it distinguishes broker admission, durable View
publication, READY, action-shim Running/EXEC release, Service Stable, driver
`Ok`/`NotFound`, terminal-row authorship, supervised allocation claims and the
independent healthy-stop benchmark. The target mapping in ADR-0102 preserves
`workload/<WorkloadId>` as the aggregate lane, keeps `service/<ServiceId>` as a
separate service projection lane, and retains node-scoped reclamation with its
existing per-allocation claim rather than inventing a node-wide lock.

G-1 pins the load-bearing order from hydration through View fsync, hot View,
awaited action dispatch, self-requeue, result consumption and lease release.
G-2 preserves the Live-to-EndingInFlight transition, overlapping writer/VMM
work, existing best-effort errors and cleanup call completion. G-3 preserves
the accepted post-READY Running publication and interception-before-EXEC path,
while covering SHUTDOWN-before-EXEC, process-group signaling, direct-child
status, descendant reaping, group escape and no post-EXIT wait.

### Feasibility and evidence honesty

The proposed concurrency cap, FIFO eligibility, age-preserving coalescing and
advisory `TickContext.deadline` are concrete and use the existing broker and
injected clock. The host stop composition is feasible with the existing
`Vmm::terminate` process-only port because the two bounded operations can be
owned concurrently while cleanup remains in `VmDriver::stop`. The guest plan
uses the existing static PID 1 and pinned nix facilities, without adding a
Tokio runtime or a generic supervision platform.

The READY, finite Job and cooperative Service thresholds are explicitly marked
as proposed targets. The design correctly says that normal VMM reaper exit and
artifact absence are independent native observations, and that the generic
`Driver::stop` `Ok` result is not silently strengthened. The aggregate outcome
scan honestly records that zero registered outcomes were checked; it is not
presented as semantic proof. Native performance and executable obligations
remain pending, as they should for a proposal.

### Handoff completeness

The exact proposed public signatures are pinned in ADR-0102, and the private
PID 1 signatures and error surface are pinned in ADR-0103. The design records
the required Contract Shape declarations, keeps examples/expectations separate
from in-process tests, retains existing regressions and all-trial E09-v2
requirements, and explicitly forbids implementation, test, expectation or
commit work before ratification. The focused brief and C4 additions clearly
retain their proposed status and do not silently supersede the accepted
sections.

The only handoff gap found is F-01's shutdown report boundary. The pending
choices of eight shared slots, nonadmitted shutdown disposition, five-second
guest grace/process-group boundary, retained narrow stop contract and proposed
benchmark targets are policy choices recorded for the user; they are not
review findings.

## Validation performed

- Read the complete feature delta, ADR-0102, ADR-0103, focused architecture
  brief/C4 changes, research, RCA and approved RCA review.
- Read the required production owner paths for broker/convergence, runtime,
  action shim, `VmDriver`, VMM reaper, guest init, exit observer, reclamation,
  handlers and `ServerHandle::shutdown`.
- Read the relevant accepted lifecycle, View, hydration, reclamation, session
  and broker ADRs, including the accepted READY/Running/EXEC ordering.
- Read issue #283 and #260 bodies/comments; both comment lists were empty when
  fetched.
- Confirmed the working tree still has documentation/design work only; no
  production source changes were introduced by this review.
- Ran read-only `git diff --check`; it passed with no output.
- Did not run tests, native benchmarks, mutation testing or expectations. The
  proposal is documentation-only, the existing production baseline is
  unchanged, and no execution was necessary to establish the contract-boundary
  finding.

## Iterations and remediation disposition

### Iteration 1

The proposal was reviewed independently against the stated research premise,
the exact ADR contracts, current production caller/owner paths and the required
evidence boundaries. F-01 was identified as a blocking design contract
ambiguity. No remediation was requested from or applied by a crafter during
this review.

### Iteration 2 — remediation re-review

The original architect corrected ADR-0102, the feature delta and the focused
architecture brief. The correction pins the snapshot point, the broker-lock
read, coalesced-entry counting, the treatment of submissions after the
snapshot, and the absence of a final server-backlog report. Directly affected
event, acceptance-obligation and brief wording was checked for consistency.

**F-01 disposition: CLOSED.** The exact boundary is now implementable and
testable without inventing an owner or reporting surface. No further findings
were identified in the correction or its directly affected contracts. The
architect's layout validation reports only the required reviewer artifact as
an intentional validator exception; that artifact is retained, including this
full review history.

## Verdict

**APPROVED.** F-01 is closed by the iteration 2 correction. The proposed
package is concrete enough for user ratification; the five listed policy
choices remain explicit user decisions.
