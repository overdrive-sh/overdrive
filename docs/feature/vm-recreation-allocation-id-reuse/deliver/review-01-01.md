# DELIVER Review — Driver-neutral allocation replacement, step 01-01

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Step | `01-01` — Reserve driver-neutral successor identities |
| Commit | `1c01a1d3e1e897d154274ffae5edd959cb11fd1d` |
| Reviewer | Fresh isolated DELIVER reviewer, GPT-5.6 Luna/max |
| Review date | 2026-09-13 |
| Latest iteration | 2 |
| Current verdict | **APPROVED** |

## Review scope and authority

This review covers the committed WorkloadLifecycle identity/ledger cut and its
fourteen activated pure acceptance bodies against the user-ratified contract in
the canonical [feature delta](../feature-delta.md), ADR-0105, ADR-0106,
ADR-0108, ADR-0109, and the approved two-step roadmap. The review preserves the
pre-existing dirty `AGENTS.md` and DES execution-log changes. No production or
test source was edited by this reviewer; this Markdown artifact is the only
review output added.

The successor-first action-shim ordering, runtime View fsync/reopen behavior,
and effect cleanup are explicitly owned by step `01-02` and are not treated as
missing from this step's production implementation.

## Design-contract checklist

| Contract | Result | Evidence |
|---|---|---|
| Stable logical owner | PASS | `WorkloadId` remains the policy owner; no aggregate, lifecycle state, or identity type was added. |
| Driver-neutral replacement action | PASS | `restart_allocation_action` at `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1406-1446` always constructs the existing `Action::RestartAllocation`; the `WorkloadDriver` match only projects `ExecPayload` or `VmPayload`. |
| Predecessor/successor identity relation | PASS | The helper sets `alloc_id` to `predecessor.alloc_id` (`:1427-1429`), `spec.alloc` to the supplied successor (`:1429-1431`), and derives `spec.identity` from the successor (`:1415`). |
| Checked numeric allocator | PASS | `next_allocation_attempt` (`:1211-1223`) takes the maximum parseable suffix across accepted rows and `view.restart_counts` keys, returns zero for no parseable input, and uses `checked_add(1)` for `u32::MAX` exhaustion. |
| Numeric-current/handoff boundary | PASS | `current_alloc` (`:1225-1248`) selects only the greatest parseable accepted-row suffix. The Run branch fences current `Draining` (`:775-785`) and chooses only the numeric-current row (`:920-923`), so historical rows and View-only keys cannot become predecessors. |
| Policy carry | PASS | Workload Failure increments/stamps the fresh successor (`:1038-1061`); generation and Platform Reclamation carry the predecessor policy without charging/restamping; historical ledger keys remain present. |
| Initial placement and SystemGc | PASS | Initial placement uses the same checked allocator and inserts a zero-count issued key (`:1095-1179`). Intentional SystemGc rows are excluded from replacement and continue through `StartAllocation`, as exercised by S-284-PURE-06. |
| Public/API and persistence shape | PASS | The diff adds no action, field, port, schema, store, driver-policy method, cleanup owner, retry, lock, or network mechanism. The private helper signatures match the feature delta. |
| Step boundary | PASS | No action-shim or runtime persistence edit was made; those are step `01-02` obligations. |

The implementation therefore matches the accepted 01-01 mechanism. The
blocking issue below is an acceptance-specification gap exposed by the changed
production behavior, not authorization to restore same-ID behavior or add a
compatibility branch.

## Contract Shape and test-quality review

The fourteen step-owned bodies are active and carry the exact required
`/// CONTRACT_SHAPE: pure-function.` declaration: eleven in
`vm_recreation_allocation_identity.rs`, two in
`vm_reclamation_plan_purity.rs`, and one in `exec_reconciler_purity.rs`.
They enter through the public `Reconciler::reconcile` or
`Reconciler::next_evaluation_at` driving ports and assert returned actions,
Views, identities, and immutable input rows. They do not call private helpers,
mock the hexagon, spawn the Overdrive binary, or emit verification evidence.
The proptest remains the authored replay-persistence shape.

The commit changes test files only by removing the fourteen step-owned
`#[ignore]` markers. No assertion, fixture, expected value, test body,
Contract Shape declaration, or replay sidecar was weakened, deleted, or
rewritten. No testing-theater, port-boundary, shared-state, or test-budget
defect was found in the step-owned changes.

Quantitatively, the roadmap maps fourteen distinct pure behaviors to fourteen
activated test methods (the property macro counts as one method). The review
budget is `14 × 2 = 28`; actual activated methods are `14`, so the budget
passes. The generic one-active-acceptance rule is not applicable because this
roadmap explicitly activates the fourteen pre-authored pure bodies together.

The repository-specific outcome-anchor line mentioned by an older reviewer
role is not required here: `AGENTS.md:265-267`, the approved roadmap, and the
feature delta require the exact pure-function Contract Shape declaration,
which is present. This feature has no DISCUSS artifact from which such an
anchor could be resolved.

## DES, commit, and mechanical evidence

| Check | Result | Evidence |
|---|---|---|
| Roadmap validation | PASS | `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.roadmap validate docs/feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json` → `VALID: 1 phases, 2 steps`. Roadmap validation is approved. |
| DES phase order | PASS | `deliver/execution-log.json:6-24` records `01-01` RED `EXECUTED/FAIL` (semantic fail-for-right-reason), then GREEN `EXECUTED/PASS`, then COMMIT `EXECUTED/PASS`. |
| Commit scope | PASS | `git show --stat` reports only the WorkloadLifecycle source and the three expected step-owned acceptance files; no unrelated path is in commit `1c01a1d3`. |
| Commit attribution | PASS | Marcus Schack Abildskov remains author/committer; the message has exactly one `Co-Authored-By: Codex <codex@openai.com>` and `Step-Id: 01-01`, with no Claude/Anthropic attribution. |
| Diff hygiene | PASS | `git diff --check 1c01a1d3^ 1c01a1d3` is clean. |
| Mutation testing | NOT RUN | Explicitly prohibited during individual DELIVER steps. |

The semantic RED evidence is anchored in the corrective re-DISTILL record:
`feature-delta.md:856-888` records the fourteen bodies failing through the
real reconciler boundary against the restored same-ID implementation, and
`feature-delta.md:889-913` records the completeness and Contract Shape
handoff. The step commit's focused GREEN run confirms those bodies now pass.

## Verification evidence

All commands below were run in the required Lima environment unless noted.

| Command / evidence | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(vm_recreation_allocation_identity)' --no-fail-fast` | PASS — 11 selected tests passed; seven unrelated tests in the binary were skipped by the expression. |
| `cargo xtask lima run -- env PROPTEST_CASES=1024 cargo nextest run -p overdrive-reconcilers -p overdrive-core --test acceptance -E 'test(identity_advances_above_rows_and_reservations_for_every_driver) or test(job_kind_reclaimed_vm_is_restarted_never_fabricated_completed_zero) or test(six_consecutive_reclamations_never_trip_restart_budget_exhausted) or test(exec_reconciler_purity::restart_action_carries_full_alloc_spec_from_live_job)' --no-fail-fast` | PASS — all four selected tests passed. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance -E 'test(workload_lifecycle_enqueues_bridge_on_alloc_transitions)' --no-fail-fast` | PASS — all seven selected tests passed. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests,kvm-tests` | PASS. |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests,kvm-tests -- -D warnings` | PASS. |
| `cargo fmt --all -- --check` | PASS. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python cargo xtask dst-lint` | PASS. |
| Full `overdrive-reconcilers` acceptance binary | **FAIL — 17 passed, 1 failed**; the one failure is the stale liveness budget assertion described below. |
| Full `overdrive-core` acceptance binary | **FAIL — 549 passed, 8 failed**; all eight failures are active legacy assertions described below. |
| Workspace default acceptance binaries | **FAIL — 1240 passed, 12 failed**; the eight core failures plus the three direct liveness/golden stale cases and one runtime case still requiring step `01-02`. |

## Findings

### F-01 — BLOCKER: active acceptance specifications still assert the superseded predecessor-keyed contract

**Reachability proof.** The changed production entry point is the public
`WorkloadLifecycle::reconcile` implementation at
`crates/overdrive-reconcilers/src/workload_lifecycle.rs:686-1183`. The eight
unchanged core acceptance bodies invoke that entry point through their local
`run` helper and reach the new identity/ledger branch with an accepted
`Terminated` or liveness `Terminated` row. The full core acceptance command
actually executed those paths and failed eight tests (`549 passed, 8 failed`);
these are not compile, collection, fixture, or hypothetical failures.

The failing assertions and exact stale assumptions are:

| Test | Location | Stale assertion exposed by the current production path |
|---|---|---|
| `fresh_failure_writes_seen_at_into_next_view` | `crates/overdrive-core/tests/acceptance/workload_lifecycle_reconcile_branches.rs:807-820` | Expects Workload Failure count/time under predecessor `alloc-payments-0`; the ratified ledger now inserts count/time under fresh successor `alloc-payments-1`. |
| `subsequent_tick_within_backoff_window_emits_nothing` | `.../workload_lifecycle_reconcile_branches.rs:881-901` | Re-drives the unchanged predecessor row while expecting predecessor-keyed backoff. Under the accepted candidate rule, the fixture must present the accepted successor/current candidate or assert the deliberate consumed-gap re-drive. |
| `tick_after_backoff_elapsed_emits_restart_and_advances_seen_at` | `.../workload_lifecycle_reconcile_branches.rs:979-991` | Reads and increments the predecessor ledger key; the new successor key carries the increment and timestamp. |
| `s_bir_restart_stopped_places_fresh_instance_and_stamps` | `crates/overdrive-core/tests/acceptance/workload_lifecycle_restart.rs:333-349` | Expects a desired-generation replacement to be `StartAllocation`; an eligible terminal predecessor now requires `RestartAllocation` with predecessor `alloc_id` and fresh `spec.alloc`. |
| `s_bir_restart_running_place_places_fresh_and_stamps` | `.../workload_lifecycle_restart.rs:444-454` | Same superseded Start-vs-Restart action-family expectation after the running-origin stop reaches terminal handoff. |
| `s_bir_coalesce_place_one_instance_stamps_to_latest_generation` | `.../workload_lifecycle_restart.rs:506-514` | Same superseded Start-vs-Restart expectation for coalesced desired generation. |
| `s_roh_a_02_liveness_terminated_restarts_under_single_budget` | `.../workload_lifecycle_restart.rs:785-808` | Expects the unified Workload Failure budget to increment in the predecessor key; accepted policy carries the increment to the successor key. |
| `s_roh_a_08_crash_and_liveness_share_one_budget` | `.../workload_lifecycle_restart.rs:1011-1020` | Expects the shared budget to remain keyed by the old allocation across the replacement; the accepted ledger is keyed by each fresh candidate. |

These bodies are not step-owned pending bodies, and commit `1c01a1d3` did not
weaken them. They are nevertheless in scope as acceptance specifications:
the feature delta states that `RestartAllocation` is used for eligible
desired-generation, Workload Failure, and Platform Reclamation replacements
(`feature-delta.md:140-157`) and that the successor owns the new policy entry
while historical issued keys remain (`:253-266`). The three generation tests
and five predecessor-keyed budget tests therefore contradict the accepted
contract rather than preserve an unrelated behavior.

**Disposition and required owner.** `CHANGES_REQUESTED`. Route this finding to
the acceptance author/designer for bounded specification reconciliation, not to
a production compatibility branch. Update or retire only these stale
assertions/fixtures so they assert the accepted fresh-successor action and
candidate-keyed policy; preserve their existing behavioral coverage where it
still applies. The original 01-01 crafter must not weaken or rewrite
pre-authored acceptance bodies to force GREEN. After the acceptance artifact is
approved, rerun the original step's RED → GREEN → COMMIT discipline and this
step's reviewer re-review. No new API, action, store, schema, retry, cleanup,
or driver-policy mechanism is authorized.

### Related acceptance drift observed outside the reported core eight

The full workspace run also found three direct stale acceptance artifacts that
share the same changed WorkloadLifecycle contract:

- `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs:490`
  still expects the liveness budget under the old allocation key;
- `crates/overdrive-control-plane/tests/acceptance/single_restart_authority_liveness_trajectory.rs:252-292`
  threads one unchanged row through a fresh-successor trajectory while
  asserting the old key; and
- `crates/overdrive-sim/tests/acceptance/hydration_trajectory_golden.rs:370-378`
  pins a pre-feature `WorkloadLifecycleView` with no initial issued-ID entry,
  while 01-01 now deliberately reserves suffix zero.

These are additional acceptance-author reconciliation items, not production
findings. The separate runtime failure
(`crates/overdrive-control-plane/tests/acceptance/workload_lifecycle_backoff.rs:194`)
exercises the intentionally incomplete old RestartAllocation shim and is owned
by step `01-02`; it must not be fixed by changing 01-01's policy or adding a
compatibility path.

## Review iteration

### Iteration 1 — 2026-09-13

The committed WorkloadLifecycle implementation matches the exact
driver-neutral identity, checked allocation, handoff, policy-carry, and
SystemGc-preservation contract. Focused step tests, compiler/lint gates, DES
ordering, attribution, and test-integrity checks pass. The independent full
core run proves eight active acceptance specifications still encode the
pre-feature predecessor-keyed/action-family contract; the additional direct
acceptance drift is recorded above. Because the active acceptance suite is not
honest about the accepted production behavior, the step cannot be approved.

## Iteration-1 verdict

**CHANGES_REQUESTED.** Resolve F-01 through the acceptance-author path, then
return the original 01-01 crafter/reviewer pair for re-review. Do not advance
to step `01-02` until this review artifact records `APPROVED`; do not run
mutation testing during remediation.

## Review iteration 2 — acceptance reconciliation re-review

### Metadata

| Field | Value |
|---|---|
| Reviewed implementation | `1c01a1d3e1e897d154274ffae5edd959cb11fd1d` |
| Acceptance reconciliation | `4467df4059ebec3b9b37496aae5c38463d64ce37` |
| DES remediation evidence | `8b25d5cb09ced6be1c035f7113c574846c825f1f` |
| Reviewer | Same fresh step-specific DELIVER reviewer |
| Review date | 2026-09-13 |
| Prior verdict | **CHANGES_REQUESTED** — F-01, stale active acceptance specifications |
| Current verdict | **APPROVED** |

### F-01 remediation verification

The acceptance-author reconciliation in `4467df40` is necessary, bounded, and
does not alter production code or introduce a compatibility path. It updates
all eight reported core acceptance contracts and the three direct stale
contracts found in iteration 1:

| Acceptance contract | Remediation evidence |
|---|---|
| `workload_lifecycle_reconcile_branches.rs` — three backoff/write cases | Each now presents the accepted fresh successor as the numeric-current row, asserts `RestartAllocation { alloc_id: predecessor, spec.alloc: successor }`, and reads count/time from the successor ledger key (`:746-926`). |
| `workload_lifecycle_restart.rs` — three generation-placement cases | Each now requires `RestartAllocation`, the exact predecessor/successor pair, and a zero-count/no-failure-time successor reservation (`:334-520`). |
| `workload_lifecycle_restart.rs` — two liveness budget cases | Each now carries the shared budget across distinct physical successor keys, preserving predecessor ledger entries and asserting successor timestamps (`:772-1054`). |
| `service_kind_vm_workloads.rs` — liveness restart | It asserts the fresh successor in `spec.alloc`, absence of a predecessor-keyed new entry, successor count `1`, and successor failure timestamp (`:464-503`). |
| `single_restart_authority_liveness_trajectory.rs` — composed liveness trajectory | Each cycle uses a distinct current row and asserts predecessor/successor action identity and successor-keyed carried budget through exhaustion (`:183-320`). |
| `hydration_trajectory_golden.rs` plus its fixture | The trajectory now asserts initial issued-ID reservation and the golden records the intentional `restart_counts[alloc-…-0] = 0` View delta (`:296-306` and fixture entries). |

The reconciled assertions are stronger than the superseded ones: they prove
both identities are distinct, tie the action predecessor to the accepted
current row, and preserve the old policy entries while carrying policy to the
successor. A repository search of the affected acceptance bodies finds no
same-ID `RestartAllocation` assertion or compatibility expectation. The
action-shim's pre-01-02 same-ID effect implementation remains intentionally
untouched and is the separately owned step `01-02` production cut; it is not a
WorkloadLifecycle compatibility branch.

### Test integrity and Contract Shape

The reconciliation preserves the original behaviors and strengthens their
oracles; it does not remove a test, skip an active body, weaken an assertion,
or fabricate a production result in a fixture. The newly transitioned direct
tests carry per-test Contract Shape declarations. The existing hydration
trajectory test retains its full snapshot comparison and now adds the explicit
initial-reservation assertion. All affected tests still enter through their
existing public reconciler/runtime driving surfaces and do not spawn the
Overdrive binary or emit verification evidence.

The eight core tests and three direct stale tests are now active with no
pending marker. The one intentional fixture-regeneration helper remains
ignored with its pre-existing on-demand reason and is not part of the affected
acceptance set.

### DES and commit evidence

| Check | Result | Evidence |
|---|---|---|
| Roadmap validation | PASS | `PYTHONPATH=/Users/marcus/.claude/lib/python python3 -m des.cli.roadmap validate docs/feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json` → `VALID: 1 phases, 2 steps`. |
| Remediation DES phases | PASS | `execution-log.json:26-39` records the original 01-01 crafter remediation `GREEN / EXECUTED / PASS` at `2026-09-13T18:13:19Z` followed by `COMMIT / EXECUTED / PASS` at `18:13:31Z`; the initial semantic RED/GREEN/COMMIT remains at `:6-24`. |
| Acceptance reconciliation commit | PASS | `4467df40` changes only the six stale acceptance/fixture files; Marcus Schack Abildskov remains author and committer, exactly one `Co-Authored-By: Codex <codex@openai.com>`, and no Claude/Anthropic attribution. It is a `test(distill)` reconciliation commit and carries no step trailer. |
| Crafter remediation evidence commit | PASS | `8b25d5cb` changes only `deliver/execution-log.json`; Marcus Schack Abildskov remains author and committer, exactly one `Co-Authored-By: Codex <codex@openai.com>`, exactly one `Step-Id: 01-01`, and no Claude/Anthropic attribution. |
| Scope and whitespace | PASS | Combined diff `1c01a1d3..8b25d5cb` contains only the six acceptance/fixture files plus the required execution log; `git diff --check` is clean. Existing dirty `AGENTS.md` and this review artifact are not part of either remediation commit. |
| Mutation testing | NOT RUN | Still explicitly prohibited for this individual roadmap step. |

### Iteration-2 verification

The complete affected acceptance command was rerun through Lima:

`cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-reconcilers --test acceptance --no-fail-fast`

Result: **575 tests run, 575 passed, 0 skipped** across the two affected
acceptance binaries. This includes all eight core stale contracts and all
three direct stale contracts from F-01. The following supporting gates also
pass after remediation:

- `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests,kvm-tests`;
- `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests,kvm-tests -- -D warnings`;
- `cargo fmt --all -- --check`; and
- `PYTHONPATH=/Users/marcus/.claude/lib/python cargo xtask dst-lint`.

The focused 01-01 identity tests and the retained WorkloadLifecycle bridge
tests remain green from iteration 1. No runtime fsync/reopen or successor-first
action-shim behavior is claimed by this step; those remain step `01-02`'s
evidence obligations.

### Finding and iteration history

| Iteration | Reviewed target | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | `1c01a1d3` | **CHANGES_REQUESTED** | F-01 — eight core plus three direct stale acceptance contracts | Returned to acceptance-author path; no production change authorized. |
| 2 | `1c01a1d3` + `4467df40` + `8b25d5cb` | **APPROVED** | 0 open | F-01 closed by bounded acceptance reconciliation; DES remediation evidence and full 575/0 affected suite verified. |

## Final verdict after iteration 2

**APPROVED.** The step-01-01 WorkloadLifecycle cut implements the exact
driver-neutral predecessor-to-fresh-successor identity and successor-keyed
policy contract for Exec and VM, with checked numeric allocation selection,
terminal handoff, and SystemGc `StartAllocation` preservation. All eight core
and three direct stale acceptance specifications now assert that contract with
no same-ID compatibility expectation, and the complete affected acceptance
evidence is 575 passed / 0 skipped. DES remediation GREEN/COMMIT entries,
commit attribution, scope, formatting, compilation, clippy, and dst-lint are
verified. Step `01-01` may advance to `01-02`; mutation testing remains a
final DELIVER-wave gate only.
