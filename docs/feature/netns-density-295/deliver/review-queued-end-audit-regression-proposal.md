# DISTILL review — queued-end audit regression proposal

## Metadata

- **Feature / finding:** `netns-density-295` / `review-14-05.md` F-01
- **Review type:** independent acceptance-design proposal review
- **Review date:** 2026-11-01
- **Proposal:** `deliver/queued-end-audit-regression-proposal.md`
- **Boundary audit:**
  `.context/netns-density-295/vm-cohort/raw/queued-audit-sim-boundary-audit.txt`
- **Proposed identity:**
  `shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery`
- **Proposed evidence seed:** `11400714819323198485`
- **Verdict:** **CHANGES_REQUIRED**

## Review boundary

This review assesses the proposal only. It authorizes no test body, production
change, Sim seam, public API, model action, runner lane, DES event, native
execution or commit. The user must explicitly approve the final exact test-body
contract before DISTILL authors it.

The review read the complete proposal, the complete three-iteration F-01 review
artifact and the raw existing-boundary audit. It then checked the named
production owner, supervisor, checkpoint, composed-node fixture and Sim
forwarder/end-delivery surfaces in the current source and at parent
`a2cd9691de4da949389cfe884a044ca9cc60367d`.

No test, native command, mutation run, KASAN run, Lima command, VM, kernel
operation or host cleanup was executed. No source, DES or existing evidence
artifact was modified by this reviewer.

## Verdict

**CHANGES_REQUIRED.** The proposed test reaches the exact production queued-end
audit handoff through existing surfaces and would fail for the intended reason
against the parent while passing with the restored arithmetic. No new seam,
model or production mechanism is needed.

One bounded proposal gap blocks approval for user decision: the exact schedule
does not end the composed process. `ServerHandle` explicitly documents that
Drop does not stop the server, and the existing adjacent seeded audit-overlap
test calls `node.crash().await` after its final assertions. The new proposal
must likewise pin an awaited owner/supervisor teardown and arrange the expected
parent assertion after that teardown.

## Strengths

- The proposal isolates one missing evidence class rather than reopening the
  accepted production correction or inventing a broader design.
- The exact identity is new and therefore correctly held for explicit user
  approval. Its location beside the existing R5-58 overlap body reuses the
  accepted test vocabulary, seed parser and composed production boundary.
- `ComposedNode::boot` uses the real `boot_server` composition, real
  `HostSharedGuestNetworkOwner`, real runtime supervisor and real
  `audit_shared`; only the driven kernel/forwarder effects are Sim.
- The schedule is production-reachable. The audit takes owner pair/watch counts,
  releases the state lock and then reads forwarder inventory. A real forwarder
  pair may end in that interval, moving from `live_pairs` to `queued_ends`,
  while the owner keeps its pair until its independent end consumer reads the
  record. The retained native mismatch demonstrates that this is not an
  invented schedule.
- `set_auto_ends(false)` controls delivery at an existing driven port. It
  represents delayed scheduling of the production end consumer; it does not
  fabricate owner state, mutate the owner table or add a test-only production
  state.
- `destination_resets(flow)` drives a real Sim forwarder pair to its actual end
  transition. With auto-run enabled, the forwarder runs to idle and its own
  inventory becomes `(live_pairs=0, live_watches=0, queued_ends=1)`.
- `release_end()` wakes the real owner end-consumer path. The Sim seam removes
  its pair bookkeeping only when `next_end` actually takes the queued record,
  and the production owner then removes its held place.
- The checkpoint already exists and is placed exactly after the production
  owner snapshot and before `io.inventory()`. The adjacent accepted Probe
  overlap proves that its release drives the periodic supervisor audit rather
  than a direct test call to `audit_shared`.
- The parent/restored worktree protocol keeps the body byte-identical and
  production bytes independently pinned. Compilation, setup or fixture failure
  is explicitly excluded from semantic RED.
- The proposal retains complete seed/action output, exact selector accounting,
  source identities and separate target directories. It requests no native VM,
  privileged kernel operation, benchmark, mutation run or canonical-manifest
  credit.

## Exact production-path audit

| Proposed step | Current source path | Assessment |
|---|---|---|
| Boot composed production owner/supervisor | `ComposedNode::boot` → `boot_server` with `GuestNetworkSource::Composed` | PASS |
| Establish one owner-held TCP pair | `ComposedNode::open_guest_flow` → production listener/owner dispatch over `SimGuestFlowIo` | PASS |
| Identify the sole current-generation pair | `SimGuestFlowIo::pairs` filters the current context generation and exposes `SimPairInfo { kind, flow, generation }` | PASS |
| Pause after owner snapshot | `HostSharedGuestNetworkOwner::audit_shared` computes `state.pairs`/`dial_watches`, drops the lock, then awaits the existing checkpoint before inventory | PASS |
| Queue a real end without owner consumption | `destination_resets(flow)` runs the Sim forwarder; `set_auto_ends(false)` holds `next_end` delivery | PASS |
| Audit successful queued handoff | Restored source compares `live_pairs + live_watches + queued_ends` with owner pair/watch places | PASS |
| Parent falsification | Parent compares only live pairs/watches, retries against the still-held owner pair, returns existing typed `PostconditionMismatch` and marks Forwarder recovery | PASS |
| Consume and converge | `release_end` permits one `next_end`; the production end consumer removes the owner pair; inventory and owner reach zero | PASS |

The schedule does not rely on a forced private owner row, a test-authored
recovery episode, a direct `audit_shared` call, or a state that only a proposed
seam could create. It composes existing causes through the real owner path.

## Finding

| ID | Severity | Disposition |
|---|---|---|
| QEARP-01 | **Blocker** | Open — the proposed six-step schedule leaves the composed process live and places the expected parent assertion before an awaited teardown. |

### QEARP-01 — Pin bounded owner lifetime and cleanup in the exact body contract

**Evidence:** the proposed schedule ends after `release_end()` and convergence.
It does not call `ComposedNode::crash`. The adjacent existing body
`a_healthy_probe_watch_overlapping_shared_audit_does_not_start_recovery` ends
every seed with `node.crash().await`
(`shared_network_recovery_acceptance.rs:867-884`). `ComposedNode::crash`
removes the `Process`, awaits `kill_process`, joins the server-owned task tree
through `ServerHandle::kill_for_test`, shuts down the shared-network supervisor
and owner tasks, and only then exits the Sim process
(`shared_network_test_ports.rs:1123-1142`, `:1176-1181`).

That explicit path matters because `ServerHandle` documents that dropping it
does not stop the server; callers must shut it down or use the existing killed
mode. Relying on the per-test Tokio runtime to disappear eventually is weaker
than the established fixture contract and leaves no ordered cleanup evidence in
the restored PASS sequence.

The proposal also says the parent should fail on the no-recovery/Open-admission
assertion immediately after checkpoint release. A panic at that point prevents
an explicit asynchronous cleanup call. Parent RED can still be retained without
that leak by capturing the after-audit facts, completing the bounded cleanup,
and asserting the captured facts afterward.

**Required minimal correction:** add a final cleanup phase to the exact proposed
body:

1. Capture the after-audit recovery/hold/admission/owner/inventory facts rather
   than immediately asserting the expected no-recovery tuple.
2. Release the end record, settle and capture the post-release facts already
   required by step 6. Keep this action sequence identical in the parent and
   restored worktrees; the parent may retain mismatched facts after its false
   recovery, which are part of its expected failure evidence.
3. Append the full cleanup action to the seed sequence and call
   `node.crash().await`, using the existing fixture only.
4. Print the complete seed and action sequence, then assert the captured
   after-audit and post-release facts. Against the parent, the captured
   recovery/mismatch state must make the same no-recovery assertion fail only
   after cleanup. Against restored source, all assertions must pass.

An equivalent structure is acceptable if it guarantees the same awaited
`ComposedNode` process teardown before the deliberate parent-failing assertion.
Do not add an async Drop guard, fixture method, seam, API, model event or
production cleanup mechanism.

## Invariant and failure-reason assessment

The proposed one-pair universe gives the arithmetic real teeth:

| State | Owner places | Inventory places | Required result |
|---|---:|---:|---|
| Paired | 1 | `1 + 0 + 0 = 1` | Healthy |
| End queued, unread | 1 | `0 + 0 + 1 = 1` | Healthy; no recovery |
| End consumed | 0 | `0 + 0 + 0 = 0` | Converged; no recovery |
| Parent audit during queued handoff | 1 | parent observes live pair/watch count `0` | Typed mismatch; false recovery, expected RED |

Because the test fixes the universe at exactly one pair, a missing, duplicated
or stranded place cannot cancel against another place. The test therefore
proves both the necessity of counting the queued handoff and the complement
after consumption. It does not generalize or weaken the audit contract.

The proposed parent failure is causally specific. The checkpoint guarantees the
owner snapshot is pair=1/watch=0 before the destination reset. Auto-end delivery
keeps the owner at one place. The external Sim inventory is independently
observed as `(0,0,1)`. Only then is the audit released. The parent cannot pass
via the current-owner recheck because that recheck still sees owner pair=1 and
live pair/watch=0. A failure before those preconditions is fixture/setup
evidence and, as the proposal requires, does not count as RED.

## Scope and test-budget assessment

Exactly one new executable identity is justified. Existing queue-order tests do
not call `audit_shared`; existing recovery scripts audit a failed or repaired
inventory; the existing successful overlap creates a watch rather than a queued
end. Combining those existing bodies would not prove the missing production
handoff.

The body remains a source-local, example-based seeded Sim acceptance test. It
adds no property generator over real I/O and no native lane. Its private
checkpoint and Sim scheduling operations are existing test-support surfaces,
which DISTILL may compose without a DESIGN change. The proposed source-file
allowlist is otherwise bounded and correct.

## Review dimensions and mandates

| Gate | Result | Evidence |
|---|---|---|
| Exact behavior / non-weakening | PASS | One pair is represented exactly once before, during and after the queued handoff; no audit outcome is relaxed. |
| Production driving boundary | PASS | Real production owner, end consumer, supervisor and audit; Sim only at driven kernel effects. |
| Reachability | PASS | Same owner-snapshot/inventory ordering as native RED; delayed end consumption is a scheduler choice already exposed by the driven port. |
| Parent FAIL / remedy PASS honesty | PASS subject to QEARP-01 | Byte-identical body and exact source worktrees; typed mismatch/recovery is the only accepted parent RED. |
| Observable assertions | PASS | Owner observation, external Sim inventory, supervisor recovery, admission, holds and fail-stop request are existing port/fixture observations. |
| API / seam / model scope | PASS | No addition required. |
| Lifecycle and cleanup | **FAIL** | QEARP-01: no explicit awaited process teardown in the proposed schedule. |
| Contract Shape | PASS | Exact required Rust declaration is `/// CONTRACT_SHAPE: bounded-change.` |
| Domain naming | PASS | Identity names queued pair end, shared audit and forbidden recovery; no delivery-coordinate helper/result name is proposed. |

CM-A, CM-B, CM-D, CM-F and CM-H pass. CM-C is blocked only by the omitted
final cleanup phase: once added, the journey covers boot, pair, queued end,
successful audit, consumption, convergence and owner teardown. The proposal's
fixed one-pair example and existing eight-seed default are appropriate for this
layer; no additional PBT or model action is warranted.

## Verification record

- Read the full proposal, full `review-14-05.md` F-01 context and full raw
  existing-boundary audit.
- Inspected the parent and restored `audit_shared` implementations.
- Traced `ComposedNode::boot`, `open_guest_flow`, audit stepping, process kill,
  owner/recovery observation, `set_auto_ends`, `destination_resets`, inventory,
  `next_end` and `release_end`.
- Confirmed the parent omits `queued_ends` from both snapshot and current-owner
  comparisons, while restored source counts it in both.
- Confirmed no implementation, native execution, mutation testing, KASAN,
  Lima, VM/kernel effect, DES write or commit occurred.

## Remediation disposition

Revise only the proposal's exact schedule, evidence sequence and parent-failure
method to include the existing awaited `ComposedNode::crash` cleanup before the
deliberate failing assertion. Return the proposal to this same reviewer. Do not
author the new test or request user approval until QEARP-01 is closed.

---

# Iteration 2 — awaited composed-process cleanup

## Iteration 2 metadata

- **Review type:** focused remediation re-review by the same independent
  acceptance-design reviewer
- **Review date:** 2026-11-01
- **Finding under review:** QEARP-01 only
- **Implementation or execution:** none
- **Iteration 2 verdict:** **APPROVED_PROPOSAL_FOR_USER_DECISION**

## Iteration 2 verdict

**APPROVED_PROPOSAL_FOR_USER_DECISION.** QEARP-01 is resolved. The exact plan
now captures both observable snapshots without asserting them, executes the same
end-release and settle sequence in the parent and restored worktrees, awaits
`node.crash().await`, prints the complete seed/action trace, and only then
evaluates the assertions that deliberately fail against the parent.

No open technical findings remain. This verdict approves the proposal for the
user's explicit decision; it does not authorize authoring or executing the new
test identity.

## QEARP-01 disposition

| Required correction | Iteration 2 evidence | Result |
|---|---|---|
| Preserve after-audit facts without an early panic | Schedule step 5 captures recovery, holds, admission, owner places and queued inventory without asserting. | PASS |
| Keep the parent/restored action sequence identical | Step 6 calls `release_end`, settles and captures post-release facts in both worktrees, including parent recovery fallout. | PASS |
| Await the established cleanup path | Step 7 appends the cleanup action and awaits `node.crash`, which removes `Process` and joins the server-owned owner/supervisor task tree. | PASS |
| Make cleanup precede the expected parent failure | Step 8 prints the complete seed/sequence and only then asserts both captured snapshots. | PASS |
| Exclude cleanup failure from semantic RED | The parent protocol accepts only the captured no-recovery/Open-admission assertion after successful cleanup; setup, collection, fixture and cleanup failures are explicitly non-RED. | PASS |
| Retain cleanup evidence | Both receipts must record explicit `ComposedNode::crash` completion in addition to source, seed, sequence and test counts. | PASS |

The plan no longer relies on `ServerHandle` Drop or on runtime destruction as
cleanup. It uses the adjacent acceptance fixture's existing killed-process path
and adds no guard, seam, fixture method, API or production mechanism.

## Final proposal assessment

The proposal now defines one complete bounded journey:

1. Boot the production-composed owner and supervisor.
2. Establish one real owner-held current-generation TCP pair.
3. Pause the real periodic audit after its owner snapshot.
4. End the actual Sim forwarder pair and retain its queued record.
5. Release and observe the real audit without short-circuiting cleanup.
6. Release the end through the real owner consumer and observe convergence.
7. Await full composed-process teardown.
8. Assert the retained facts, yielding parent FAIL and restored PASS for the
   exact queued-end arithmetic.

The state is production-reachable, independently anchored by the native RED,
and constructed only through existing driving/driven boundaries. The parent
omits `queued_ends` in both comparisons and must start false Forwarder recovery;
restored source counts the one queued place and must stay Open with no hold,
recovery or fail-stop. The one-pair universe prevents compensating count errors.

No production/API/seam/model/lane change is needed. One new executable identity
is proportionate because no existing body combines successful `audit_shared`
with the queued-end handoff. The proposed exact Contract Shape declaration,
seed, selector, isolated worktrees, byte-identical test source, full output and
source-hash evidence are adequate.

## Iteration 2 verification

| Check | Result |
|---|---|
| Proposal SHA-256 | `d0a4a86776d09b3284c2680fa96f7e815a34e4506c4801a34e2d8f61bd0f5b48` |
| Exact identity unchanged | PASS |
| Exact seed unchanged | PASS — `11400714819323198485` |
| Parent/restored source pins unchanged | PASS |
| Existing-surface-only boundary | PASS |
| Awaited owner/supervisor cleanup | PASS |
| `git diff --check` | PASS |
| Implementation/native/mutation/KASAN/Lima/host effects | NOT RUN |

## Final disposition

The proposal may now be presented to the user for explicit approval of exactly:

```text
shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery
```

Approval would authorize only the one source-local seeded body, its module
contract-table row and the parent-fail/restored-pass evidence protocol already
written in the proposal. Until the user approves, do not author, execute or
commit it.

---

# Authored-body review — approved queued-end audit regression

## Authored-body metadata

- **Review type:** independent review of the user-approved executable body
- **Review date:** 2026-11-01
- **User approval:** exact single regression proposal approved with `yes`
- **Reviewed identity:**
  `shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery`
- **Authored test-source SHA-256:**
  `726a3944991ca3070592b59e82e17ce0d1a13162a722a425b1fae63f891e2ca1`
- **Source-gate run:** `1603799a4a13e83bf98fd6e1`
- **Verdict:** **APPROVED**

## Authored-body verdict

**APPROVED.** The authored body implements the independently approved proposal
through the existing production-composed owner, supervisor, audit checkpoint
and Sim driven port. It creates one owner-held TCP pair, moves that pair into
one unread queued-end handoff, captures the successful-audit and post-consumer
states, awaits the full composed-process cleanup, and only then asserts the
retained facts.

No open findings remain. Approval authorizes the original crafter to execute
the already-approved isolated parent-fail/restored-pass evidence protocol. It
does not establish RED or GREEN itself and does not authorize a production,
fixture, seam, model, API or lane change.

## Exact changed-file and identity audit

The executable diff is confined to
`crates/overdrive-control-plane/src/shared_network_recovery_acceptance.rs`:

- one module-contract table row for the queued-end R5-58 handoff;
- one import of the existing private `SimPairKind`; and
- one exact new test function.

No production source, `guest_network.rs` arithmetic, Sim implementation,
`ComposedNode` fixture, checkpoint, public API, Quint model, runner lane or
other test body is edited. The identity appears once and carries the required
exact declaration:

```rust
/// CONTRACT_SHAPE: bounded-change.
```

The body reuses `seeds()` and `OVERDRIVE_R5_58_SEED`; it adds no environment
variable and the evidence seed remains `11400714819323198485`.

## Proposal-to-body conformance

| Approved requirement | Authored evidence | Result |
|---|---|---|
| Print seed before setup | The first statement of every seed iteration emits `R5-58 HealthyQueuedEndOverlapsSharedAudit seed=...`. | PASS |
| Boot the existing production composition | `ComposedNode::fresh` plus `node.boot()` reaches the real production owner/supervisor composition. | PASS |
| Delay only end-record delivery | `node.process().io.set_auto_ends(false)` uses the existing driven-port scheduler control. | PASS |
| Establish exactly one current-generation TCP pair | `open_guest_flow` is followed by `io.pairs`; the body requires exactly one `SimPairKind::Tcp` entry with `Some(flow)`, plus owner/inventory counts `(1,1,0,0)`. | PASS |
| Start from Open/no-hold/no-recovery | The precondition tuple requires no Forwarder episode, zero holds and `GuestAdmission::Open`. | PASS |
| Pause the real audit after its owner snapshot | The body arms `arm_shared_audit_post_snapshot_checkpoint`, advances the seeded remainder, releases the held DNS audit and awaits `checkpoint.reached`. | PASS |
| Create one unread queued end | `destination_resets(flow)` runs the real Sim forwarder; before audit release the body requires owner places 1 and inventory `(0,0,1)`. | PASS |
| Release a successful production audit | `checkpoint.release` followed by `node.settle` drives the existing periodic supervisor path; no direct `audit_shared` call exists. | PASS |
| Capture after-audit facts before assertions | Recovery, holds, admission, owner places, all inventory fields and fail-stop request are stored by value. | PASS |
| Consume through the real owner end path | `release_end` plus `settle` permits `GuestFlowIo::next_end`; the second snapshot expects owner/inventory zero. | PASS |
| Preserve identical parent/restored schedule | No branch depends on recovery result before release, settle, capture or cleanup; both source worktrees execute the same action sequence. | PASS |
| Await cleanup before deliberate failure | The body appends the cleanup action, awaits `node.crash`, records completion and prints the full sequence before either retained-state assertion. | PASS |
| Assert the exact complement | After audit: `(recovery=None, holds=0, Open, owner=1, inventory=0/0/1, fail-stop=None)`; after consumption: the corresponding owner/inventory counts are all zero. | PASS |

## Production-path and failure-reason audit

The test exercises the exact intended handoff:

```text
production owner pair = 1
  -> periodic audit snapshots owner pair/watch places = 1/0
  -> destination reset ends the actual Sim forwarder pair
  -> inventory becomes live=0/watch=0/queued=1
  -> owner has not consumed next_end and still holds one pair
  -> audit reads that successful inventory
```

Against parent `a2cd9691de4da949389cfe884a044ca9cc60367d`, both the snapshot and
current-owner comparisons ignore `queued_ends`; the audit must report its
existing `PostconditionMismatch` and the supervisor must create a Forwarder
recovery episode. The authored first retained tuple then differs from the
expected no-recovery/Open tuple after cleanup. This is the approved semantic
RED, not a test-authored failure state.

Against restored source `8bc5ed027d541e1d9f2a597f74072d7109b93798`, inventory
places are `0 + 0 + 1`, equal to the one owner place. The audit succeeds,
`release_end` lets the production end consumer remove the pair, and the second
tuple reaches zero without recovery, hold, admission closure or fail-stop.

The fresh composed node, exact one-pair observations and exact inventory counts
make compensating missing/duplicate places impossible. The test neither seeds
owner state nor inspects a driver-maintained mirror as its oracle.

## Lifecycle, cleanup and diagnostic honesty

The body satisfies the iteration 1 cleanup finding. It captures all facts
needed for the parent/restored verdict, then calls `node.crash().await` before
the assertions. That established path removes `Process`, awaits
`ServerHandle::kill_for_test`, shuts down the supervisor and owner task tree,
and exits the Sim process. Only after the awaited call returns does the body
print the complete action sequence and evaluate its assertions.

Every authored assertion includes the current seed and accumulated sequence.
The boot error is also wrapped with both. Existing fixture preconditions may
still panic internally, but the seed is printed before any setup; such a
failure is excluded from semantic RED by the approved protocol. Cleanup or
fixture failure likewise cannot be credited as the parent failure.

## Test-design mandates and completeness

| Gate | Result | Evidence |
|---|---|---|
| CM-A — production boundary | PASS | Real owner, end consumer, supervisor and audit; Sim only at the driven forwarder/kernel port. |
| CM-B — domain language | PASS | Identity and diagnostics name queued pair end, shared audit, recovery and owner consumption. |
| CM-C — complete journey | PASS | Boot → pair → paused audit → queued end → audit → consume → converge → awaited cleanup. |
| CM-D — fixture isolation | PASS | No business rule is duplicated; the test composes existing surfaces and asserts their observations. |
| CM-E — bounded-change universe | PASS | The tracked universe closes recovery, holds, admission, owner places, live pairs/watches, queued ends and fail-stop before and after consumption. |
| CM-F — PBT mode | PASS | One source-local seeded Sim body reuses the existing eight-seed surface; no real-I/O generator is added. |
| CM-G — two-tier journey | PASS / N/A | Native RED/GREEN already supplies the real-kernel complement; no new state-machine model is warranted. |
| CM-H — explicit fault path | PASS | The destination reset and delayed end delivery are named, deterministic causes. |

The body closes F-01's missing C2/C7 ordering schedule and C6 necessity proof.
Its one-pair cardinality covers the exact 0/1 handoff boundary; existing queue
and recovery suites retain broader ordering/model coverage. No second identity
or scenario is justified.

## Source verification

| Check | Result |
|---|---|
| Authored source hash | PASS — `726a3944991ca3070592b59e82e17ce0d1a13162a722a425b1fae63f891e2ca1`, matching the proposal. |
| `guest_network.rs` source | Unchanged from restored correction per the authored handoff. |
| Durable unprivileged source gate | PASS — run `1603799a4a13e83bf98fd6e1`, exit 0; fmt, scoped check and scoped Clippy `-D warnings` passed. |
| `git diff --check` | PASS. |
| Parent-fail/restored-pass selector | NOT RUN — correctly reserved for the original crafter after this review. |
| Native / mutation / KASAN / Lima / host effects | NOT RUN. |

## Authored-body disposition

The authored body is approved without remediation. The original crafter may
now run the exact unprivileged isolated-worktree protocol from the approved
proposal, retaining the full parent-fail and restored-pass receipts. DISTILL
must not edit production or interpret compilation, fixture, cleanup or seed
failure as semantic RED.
