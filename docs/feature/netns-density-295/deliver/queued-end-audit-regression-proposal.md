# Queued-end audit regression proposal

## Status and authority

This document began as a DISTILL assessment and proposal. Independent review
iteration 2 approved the exact body contract, and the user explicitly approved
the single identity with `yes`. DISTILL has now authored only that body and its
module-contract table row. No test has executed; no fixture, Sim seam,
production source, DES record or commit changed.

The proposal addresses the sole open finding in `review-14-05.md`: the
production queued-end audit correction has real native RED and final native
GREEN evidence, but lacks one exact seeded Sim parent-fail/remedy-pass pair.
The correction is restored at
`8bc5ed027d541e1d9f2a597f74072d7109b93798`; the current worktree has no
source diff from that commit. This proposal does not question, discard or
weaken the correction.

## Evidence assessed

The retained native diagnostic reached the exact production mismatch:

```text
owner snapshot: live_pairs=1, live_watches=0
forwarder inventory: live_pairs=0, live_watches=0
```

The owner then falsely started Forwarder recovery. With the queued-end count
included, both final 64-VM correctness bodies pass. A no-patch single pass in
`0e359ef28e6cd48aeae3e7f8` cannot disprove an ordering race. Sim preparation
run `18017a4bff9679d3a2bacde9` exhausted disk quota before any test ran and is
not RED evidence.

The existing seeded boundaries are each incomplete for this exact handoff:

| Existing boundary | What it proves | Missing part |
| --- | --- | --- |
| `quint_flow_quota` `queued_ends` / `release_end` traces | A real production owner over `SimGuestFlowIo` queues and consumes end records in order. | The driver never calls `HostSharedGuestNetworkOwner::audit_shared`. |
| `quint_fwd_pair` recovery scripts | Forwarder recovery ordering and end-record projection. | Its audit either scripts Inventory failure or runs after repair with a clean inventory; it never audits a successful queued-end inventory. |
| R5-58 healthy Probe overlap | The existing post-snapshot/pre-inventory checkpoint reproduces an owner/audit ordering window. | The overlap creates a watch, not a queued pair end. |

These facts agree with
`.context/netns-density-295/vm-cohort/raw/queued-audit-sim-boundary-audit.txt`.

## Boundary decision

The existing composed Sim boundary can represent the exact production state.
No new production API, model field, test hook, fixture method or seam is
needed. The required pieces already exist and are used by approved tests:

| Required operation | Existing surface |
| --- | --- |
| Boot the real production owner and runtime supervisor | `shared_network_test_ports::composed::ComposedNode::boot` |
| Establish one real owner-held guest-opened pair | `ComposedNode::open_guest_flow` |
| Pause the production audit after its owner snapshot and before inventory | `HostSharedGuestNetworkOwner::arm_shared_audit_post_snapshot_checkpoint` |
| Keep a queued end unread by the owner | `SimGuestFlowIo::set_auto_ends(false)` |
| End the actual Sim forwarder pair and queue its end record | existing `SimGuestFlowIo::destination_resets(flow)` script |
| Read the external Sim inventory | `SimGuestFlowIo::forwarder_inventory()` |
| Let the production owner consume the queued record | `SimGuestFlowIo::release_end()` followed by `ComposedNode::settle()` |
| Observe recovery, holds, admission and owner places | existing `recovery_observation`, `owner_observation` and `GuestOwnerObservation` |
| Await bounded owner/supervisor teardown | existing `ComposedNode::crash()` → `kill_process` → `ServerHandle::kill_for_test` → `SimGuestFlowIo::exit_process` |

The only missing artifact is an executable regression body that composes
these existing operations in the required order. That is a test-coverage gap,
not a missing testability boundary.

## Proposed exact identity and location

The user approved exactly one source-local seeded body:

```text
shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery
```

Location:
`crates/overdrive-control-plane/src/shared_network_recovery_acceptance.rs`,
beside the existing healthy Probe overlap regression. Declaration:

```text
/// CONTRACT_SHAPE: bounded-change.
#[tokio::test(flavor = "current_thread", start_paused = true)]
```

It reuses the existing `seeds()` function and
`OVERDRIVE_R5_58_SEED`; it adds no environment variable or runner lane. The
required evidence seed is the already-retained audit-overlap seed:

```text
11400714819323198485  (0x9e3779b97f4a7c15)
```

The body must print the seed before setup and print the completed sequence
again after awaited process cleanup, before the deliberate parent-failing
assertions:

```text
R5-58 HealthyQueuedEndOverlapsSharedAudit seed=<seed> sequence=<complete ordered actions>
```

Every setup/precondition failure must include the seed and sequence accumulated
so far. The after-audit and post-release facts are captured before cleanup and
asserted only after the full sequence and cleanup have been printed.

## Exact production path and schedule

The proposed body uses the real `HostSharedGuestNetworkOwner`, its real end
consumer and its real `audit_shared`; only the driven kernel effects are Sim.

| Order | Action | Required observable state |
| ---: | --- | --- |
| 1 | Boot `ComposedNode`, call `set_auto_ends(false)`, and open one guest flow. | Owner holds one `GuestFlowPhase::Paired`; inventory is `live_pairs=1`, `live_watches=0`, `queued_ends=0`; no Forwarder recovery or quiescence hold exists. |
| 2 | Derive the sole flow id from the existing `io.pairs()` observation. | Exactly one current-generation TCP pair names `Some(flow)`; ambiguity fails the test. |
| 3 | Advance by the seed-derived offset within `SHARED_NETWORK_AUDIT_PERIOD`; arm the existing shared-audit checkpoint; advance the remainder and release the held DNS audit. | The periodic production audit snapshots owner places as pair=1/watch=0 and pauses before `io.inventory()`. |
| 4 | While the audit is paused, call `destination_resets(flow)` and settle. | SimForwarder moves the pair from live to one queued end; owner still holds the pair because auto-end delivery is disabled: owner places=1, inventory `(live_pairs=0, live_watches=0, queued_ends=1)`. |
| 5 | Release the checkpoint and settle the production supervisor; capture, but do not yet assert, the after-audit facts. | Captured expected restored-source facts: Forwarder recovery absent, holds zero, admission Open, owner holds one pair and inventory has exactly one queued end. Parent mismatches are retained in the capture. |
| 6 | Call `release_end()` and settle; capture, but do not yet assert, the post-release facts. | Captured expected restored-source facts: the real owner end consumer removes the pair, owner places=0 and inventory `(0,0,0)`, with no recovery, hold or fail-stop. Parent recovery fallout remains observable rather than short-circuiting cleanup. |
| 7 | Append `await composed process cleanup` to the action sequence and call `node.crash().await`. | The existing fixture removes `Process`, awaits `ServerHandle::kill_for_test`, joins the server-owned owner/supervisor task tree, then exits the Sim process. No live composed owner remains. |
| 8 | Print the seed and complete action sequence, then assert the captured after-audit and post-release facts. | Parent fails the no-recovery/Open-admission invariant only after cleanup. Restored source passes both captured-state assertions after the same cleanup. |

The seed controls the same periodic-audit offset as the approved Probe-overlap
regression. The checkpoint pins the load-bearing interleaving, so the test is
deterministic once the seed is printed.

## Invariant

During a successful end-record handoff, each owner place is represented
exactly once by:

```text
inventory.live_pairs + inventory.live_watches + inventory.queued_ends
```

Before the owner consumes the queued end, that sum must equal the owner's
current pair-plus-watch places. The audit must not start recovery, acquire a
quiescence hold, close admission or request fail-stop. After `release_end`,
both owner and inventory counts must converge to zero without recovery. Those
two observable snapshots are retained by value, the composed process is then
explicitly and asynchronously torn down, and only afterward are the invariant
assertions evaluated.

This pins the necessity of the current production arithmetic without
relaxing the audit: a missing, duplicated or stranded place still fails.

## Parent FAIL and restored-source PASS method

DISTILL authored the single body without committing it. Its only source path is
`crates/overdrive-control-plane/src/shared_network_recovery_acceptance.rs`.
The original step crafter owns the reviewed parent-fail/restored-pass execution,
its actual RED/GREEN DES events and the eventual test-only commit. No
production file or fixture/seam file belongs in that commit.

For the parent failure, create an isolated detached worktree at
`a2cd9691de4da949389cfe884a044ca9cc60367d` and cherry-pick only that test-only
commit. This avoids a patch file and keeps every production byte at the
approved parent. Verify mechanically that
`crates/overdrive-control-plane/src/guest_network.rs` equals the parent and
that the only parent-worktree delta is the proposed test file.

Run this exact seeded selector from the parent-plus-test worktree through the
existing unprivileged Metal route, with a task-owned build directory and
complete output:

```text
cargo xtask metal run --no-sudo -- env \
  CARGO_TARGET_DIR=/home/ubuntu/target-queued-end-parent \
  OVERDRIVE_R5_58_SEED=11400714819323198485 \
  cargo nextest run -p overdrive-control-plane --lib \
    --features integration-tests \
    -E 'test(/shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery$/)' \
    --no-capture
```

Expected parent result: nonzero test exit only after `node.crash().await`
completes. The parent ignores `queued_ends`, so expected owner places remain 1
while the read inventory reports live pair/watch places 0. The production
audit returns its existing typed `PostconditionMismatch`, and the supervisor
begins the false Forwarder recovery. The body captures that after-audit state,
attempts the same end release and post-release capture as restored source,
awaits composed owner/supervisor cleanup, prints the seed and complete action
sequence, and only then fails the captured no-recovery/Open-admission
assertion. Compilation, setup, seed parsing, collection, cleanup or fixture
failure does not count as RED.

Run the byte-identical test against restored source
`8bc5ed027d541e1d9f2a597f74072d7109b93798` with only the target-directory
name changed:

```text
cargo xtask metal run --no-sudo -- env \
  CARGO_TARGET_DIR=/home/ubuntu/target-queued-end-restored \
  OVERDRIVE_R5_58_SEED=11400714819323198485 \
  cargo nextest run -p overdrive-control-plane --lib \
    --features integration-tests \
    -E 'test(/shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery$/)' \
    --no-capture
```

Expected result: exactly one selected pass, zero failed/ignored/runtime-SKIP
bodies. The audit accepts `0 + 0 + 1 == 1`; recovery stays absent; releasing
the end converges both owner and inventory to zero; `node.crash().await`
completes before the two captured-state assertions pass.

Retain each durable Metal run id and collect that same id after route loss;
never launch a duplicate.
Preserve full output without `head` or `tail` and record source commit, dirty
state, exact command, test source SHA-256, `guest_network.rs` SHA-256, seed,
sequence, explicit `ComposedNode::crash` completion, selected/passed/failed/
ignored counts and exit status. The Sim body boots no VM and runs no benchmark,
mutation, KASAN, Lima or physical-host kernel cleanup.

## Authored changed-file boundary

| Path | Current or reviewer-owned change |
| --- | --- |
| `crates/overdrive-control-plane/src/shared_network_recovery_acceptance.rs` | Authored: the one exact seeded body and one module-contract table row describing its queued-end audit invariant. |
| `docs/feature/netns-density-295/deliver/queued-end-audit-regression-proposal.md` | Records approval, the authored body boundary and source checks; the original crafter later appends actual parent-fail and restored-source-pass receipt identities. |
| `docs/feature/netns-density-295/deliver/review-14-05.md` | Reviewer-owned iteration and disposition only; DISTILL and the crafter do not edit it. |

Raw durable receipts may be retained under the existing feature context, but
are evidence rather than source changes. `guest_network.rs`,
`shared_network_test_ports.rs`, `flow_io/sim.rs`, the Quint drivers, public
API and production composition are explicitly outside the implementation
diff. The existing queued-end correction remains byte-identical.

## Approval and authored-source record

The user explicitly answered `yes` to the request for exactly one new
executable test identity:

```text
shared_network_recovery_acceptance::a_queued_pair_end_overlapping_shared_audit_does_not_start_recovery
```

That approval authorizes only the body and evidence protocol above. The
authored implementation:

- reuses `seeds()` and `OVERDRIVE_R5_58_SEED`;
- boots the existing production-composed owner/supervisor;
- holds existing end delivery, opens one real owner-held TCP pair, pauses the
  existing audit checkpoint and queues the real Sim pair's destination reset;
- captures after-audit and post-release facts without asserting;
- releases the queued record through the real owner consumer;
- awaits `node.crash().await` task-tree cleanup;
- prints the seed and complete action trace; and
- performs both invariant assertions only after cleanup.

It adds no fixture method, seam, model action, public API, architecture
mechanism, scenario outcome, lane, benchmark or production change. The current
queued-end correction in `guest_network.rs` remains byte-identical to
`8bc5ed027d541e1d9f2a597f74072d7109b93798`.

### Source-only verification

Durable unprivileged Metal run `1603799a4a13e83bf98fd6e1` completed with
`exit_code=0`, `cause=command_returned`, and the same run was collected once.
It executed only:

```text
cargo fmt --all -- --check
cargo check -p overdrive-control-plane --lib --features integration-tests
cargo clippy -p overdrive-control-plane --lib --features integration-tests -- -D warnings
```

The source identities at that gate were:

| Source | SHA-256 |
| --- | --- |
| `crates/overdrive-control-plane/src/shared_network_recovery_acceptance.rs` | `726a3944991ca3070592b59e82e17ce0d1a13162a722a425b1fae63f891e2ca1` |
| `crates/overdrive-control-plane/src/guest_network.rs` | `d7498bf52b685404485efcc5664c8485ffac1bc6121c91ec3f1c361232e4308f` |

### Approved parent-fail and restored-source-pass receipts

The original crafter executed the exact byte-identical selector with the
explicit seed `11400714819323198485` through unprivileged Metal. The parent
worktree was detached at `a2cd9691de4da949389cfe884a044ca9cc60367d` and had
only the test-only commit `0207e1befb29b6f2623dfc97444f37247cca1ac9` applied.
Its `guest_network.rs` SHA-256 was
`3c1b9411d8d54abb2c610b3e66ce743177c0b7042f60b7cc474a88bc9a741173`; the
authored test SHA-256 was the source-gate value above. The durable parent run
was `2c362955672f8c2357a11a60`, with task-owned target directory
`/home/ubuntu/target-queued-end-parent`, and exited `100` after one selected
test failed (`0 passed, 1 failed, 544 skipped`; zero ignored/runtime skips).
The full action trace printed the seed, reached
`composed owner and supervisor cleanup completed`, and then retained the
parent's false recovery state:

```text
left: (Some(RecoveryEpisode { phase: Held, failed_attempts: 0 }), 1, Quiesced, 0, 0, 0, 0, None)
right: (None, 0, Open, 1, 0, 0, 1, None)
```

The restored-source run used the same body, seed and selector against
`9dc7bb9ba3269460c7b13dfefdf72e64067eb3c7`, with
`guest_network.rs` SHA-256
`d7498bf52b685404485efcc5664c8485ffac1bc6121c91ec3f1c361232e4308f` and
task-owned target directory `/home/ubuntu/target-queued-end-restored`. Durable
run `fbbd698701ac86754a28e70d` exited `0` with one selected pass (`1 passed,
0 failed, 544 skipped`; zero ignored/runtime skips). Its trace reached the same
awaited cleanup action and both retained tuples passed: queued inventory
`(0,0,1)` with one owner place before consumption, then owner and inventory
`(0,0,0)` after the production end consumer read the record.

Both runs used the existing canonical BPF artifact only as an unprivileged
task-owned build input, SHA-256
`5aec54ee315fbf451b313b1e1d1faf0de9ff6fdb89b262357b36ae4f76d19e3a`, via the
existing `OVERDRIVE_BPF_OBJECT` build-script override. The raw receipts and
comparison ledger are retained at:

```text
.context/netns-density-295/vm-cohort/raw/queued-end-parent-fail.log
.context/netns-density-295/vm-cohort/raw/queued-end-restored-pass.log
.context/netns-density-295/vm-cohort/raw/queued-end-audit-ledger.json
```

The initial parent preparation attempt failed before test execution because
the remote BPF artifact was stale; the unprivileged rebuild also refused a
candidate write. Those are retained as preparation evidence and are excluded
from semantic RED. No native VM, kernel effect, mutation test, KASAN run or
Lima command was used.
