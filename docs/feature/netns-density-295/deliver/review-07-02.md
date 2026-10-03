# DELIVER Review — Step 07-02

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `07-02` — reclaim action, shim arm, validator rule, and View fields |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning |
| Iteration | 1 |
| Commits reviewed | `e0e337fee581f92e94873602087cadaaebc07dfe`, `5a4a40ae7fbabc236e2e81d4e3db107b60cb0df2` |
| Verdict | **APPROVED** |

The review applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

Reviewed the two step commits against approved roadmap step 07-02, feature-delta D-295-R11 part 1, ADR-0136, and DISTILL scenarios S-ND295-56 and the validator body of S-ND295-55. The review covered the action and validator public shapes, typed errors, persisted View inputs, row-neutral shim behavior, production wiring, the exact acceptance-test stimulus and port boundary, Contract Shape declarations, TDD evidence, and the reported verification gates. S-ND295-55 emission, placement, and retry behavior assigned to 07-03 were not treated as 07-02 requirements.

Roadmap criteria and split: `roadmap.json:1115-1149`. Accepted action, View, shim, and validator contract: `feature-delta.md:4907-5108` and ADR-0136. DISTILL acceptance stimulus and oracle: `test-scenarios.md:1014-1037`; validator scope: `:989-1012`.

## Contract Shape Compliance

The interface shape matches the accepted design. `Action::ReclaimAllocationNetwork { alloc_id: AllocationId }` remains the ID-only variant and its documentation states that reclaim writes no allocation row or lifecycle event (`overdrive-core/src/reconcilers/mod.rs:675-683`). The commits remove the stale RED-scaffold comments; they add no public type, trait member, method, parameter, action field, enum variant, or wire format.

The public validator error shape is the accepted `ReclaimConflictAction` set—`StartAllocation`, `RestartAllocation`, `StopAllocation`, and `FinalizeFailed`—and `ReconcilerOutputViolation::ConflictingAllocationReclaim { alloc_id, other }`, with the specified error text and derives (`action_shim/validate.rs:127-205`). `validate_reconcile_output(&[Action])` retains its signature. The private `reclaim_owner` helper adds no public surface. The runtime already invokes this validator before dispatch (`reconciler_runtime.rs:1578-1585`), so the rule is connected to production output validation.

`WorkloadLifecycleView` has precisely the two accepted additive persisted inputs, each `#[serde(default)]`: `reclaim_attempts: BTreeMap<AllocationId, u32>` and `reclaim_emitted_at: BTreeMap<AllocationId, UnixInstant>` (`workload_lifecycle.rs:1887-1895`). No deadline or other derived state was added. The fields are documented as inputs to recompute the live backoff, matching the “persist inputs, not derived state” contract.

The private action-shim path checks for an owned guest-network lease, retires it, stops resolved drivers while tolerating `NotFound`, awaits intercept removal, then awaits guest-network teardown. The guest pool releases the lease only after successful teardown; the allocation-driver index is removed only after the cleanup completes (`action_shim/mod.rs:1660-1724`). Typed failures return before lease release. The helper takes no observation store or lifecycle-event sender, and the acceptance helper verifies row, lifecycle-occurrence, and event-bus equality around reclaim (`netns_density_guest_network.rs:824-855`). The `ReclaimAllocationNetwork` dispatch arm is wired at `action_shim/mod.rs:3555-3565`.

The validator applies only the four accepted allocation-action conflicts, keys `RestartAllocation` by its predecessor `alloc_id`, permits distinct allocation ids, and reports the conflict in either order (`action_shim/validate.rs:194-205, 240-300`). The exact pre-authored validator body checks all four types in both orders, typed fields, and distinct-allocation cases (`validate.rs:719-780`).

## Test integrity, authorship, and coverage

The acceptance-file diff contains five removed S-ND295-56 pending markers, one removed S-ND295-55 validator marker, and test-support setup changes. No expected value, assertion, test body, or scenario was weakened, deleted, replaced, skipped, or re-authored.

The fixture now arms `SimSharedGuestNetworkOwner` teardown failure during the activation-refusal precondition, then clears it before each reclaim body (`netns_density_guest_network.rs:1092-1122`). This matches the pre-authored S-ND295-56 fault stimulus: activation fails while teardown is scripted to fail, leaving a Failed row and Retiring lease (`test-scenarios.md:1034`). The separate intercept-removal fault is needed to leave protection members present for the already-removed case, whose precondition asserts those members before removing them.

The out-of-band setup now removes intercept members by invoking `MtlsIntercept::remove_allocation_elements` on the test-local adapter, then observes the absent members. The owner-side `remove_parts_out_of_band` call changes only the test-local `attached` model; the subsequent reclaim still invokes the production action path and the owner teardown port. The assertion verifies that teardown was called once and found the owner parts already absent (`netns_density_guest_network.rs:517-532, 536-562, 1301-1362`). The local `destinations` binding preserves the same expected member set and trace assertion. These changes implement the accepted test stimulus and port boundary; they do not alter the scenario oracle.

Every activated S-ND295-56 body retains `CONTRACT_SHAPE: bounded-change`; the source-local S-ND295-55 validator body has the exact `CONTRACT_SHAPE: pure-function.` rustdoc declaration (`validate.rs:721`). The step activates five S-ND295-56 bodies for five specified outcomes and one S-ND295-55 validator body for one outcome: six tests for six behaviors, within the `2 × behaviors` budget of 12. No new production composition or expectation runner was added. The acceptance tests drive the in-process action-shim composition named by DISTILL; the validator rule is also called by the runtime before dispatch.

## Verification

The step execution log records RED, GREEN, and COMMIT in order, each `EXECUTED` with `PASS` (`execution-log.json:965-983`). The RED classification records semantic failures at the reclaim-dispatch and validator-conflict seams, rather than collection or compile failures (`distill/red-classification.md:431-437`). The required GREEN and COMMIT gates were reported as passing:

| Gate | Result | Evidence |
|---|---|---|
| Acceptance selector | PASS, 8 | `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(netns_density_guest_network)' --no-fail-fast` |
| Action-shim selector | PASS, 31 | `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --lib -E 'test(action_shim)' --no-fail-fast` |
| Workload-lifecycle selector | PASS, 23 | `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --lib -E 'test(workload_lifecycle)' --no-fail-fast` |
| Workspace check | PASS | `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` |
| Workspace clippy | PASS | `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` |
| Formatting and diff check | PASS, reported | `rustfmt` and `git diff --check` |

These are the step/orchestrator-reported command results; this reviewer did not rerun the suite. Mutation testing was not run during the individual roadmap step, as required.

## DES, commit, and workspace evidence

Both commits preserve Marcus as Git author, include exactly `Co-Authored-By: Codex <codex@openai.com>`, and carry `Step-Id: 07-02`. No Claude, Anthropic, or generated-by attribution appears in either commit. Before this review, the only pre-existing dirty path was the user's `AGENTS.md`; it was preserved. This Markdown review is the only file written by the reviewer.

## Findings and remediation dispositions

No proven in-scope defect, public API divergence, test weakening, acceptance-authoring violation, or port-boundary violation was found. No remediation was required or dispatched.

## Iteration 1 verdict

**APPROVED.** The action dispatch and validator implement the accepted R11 part-1 contract; the persisted View retains only the specified retry inputs; and S-ND295-56's fixture and out-of-band stimulus changes are necessary to match the accepted DISTILL fault model and port boundary. The activated acceptance oracles remain intact, all reported required gates pass, and no later 07-03 behavior was required for this review.
