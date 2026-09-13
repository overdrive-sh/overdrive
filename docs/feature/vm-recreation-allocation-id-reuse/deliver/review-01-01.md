# DELIVER Review — VM recreation allocation identity, step 01-01

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Step | `01-01` — Reserve fresh VM execution identities |
| Commit | `a0f19ebe8008f85136e8f619cb909de1243672f6` |
| Reviewer | Fresh isolated `nw-software-crafter-reviewer` |
| Review date | 2026-09-13 |
| Latest iteration | 3 |
| Current verdict | **APPROVED** |
| Iteration 1 verdict | **CHANGES_REQUESTED**; R01 subsequently rejected as out of scope |

## Review scope and authority

This review covers the committed implementation and the fourteen step-owned
acceptance activations against:

- `docs/product/architecture/adr-0104-vm-recreation-fresh-allocation-identity.md`;
- `docs/feature/vm-recreation-allocation-id-reuse/feature-delta.md`;
- approved roadmap step `01-01` in
  `docs/feature/vm-recreation-allocation-id-reuse/deliver/roadmap.json`;
- the repository's `AGENTS.md`, `CLAUDE.md`, and required Rust/testing rules.

The review preserves the pre-existing dirty `AGENTS.md` and the untracked DES
execution log. No production or test source was edited by this reviewer; this
artifact is the only review output added.

## Design-contract checklist

| Contract | Result | Evidence |
|---|---|---|
| Stable logical owner remains `WorkloadId` / `WorkloadLifecycle` | PASS | No owner, aggregate, replica identity, or lifecycle state was added. |
| VM initial, generation, Workload Failure, and Platform Reclamation starts use a fresh ID | PASS | `workload_lifecycle.rs:1043-1071` handles failure/reclamation and `:1121-1173` handles initial/generation placement through `next_vm_attempt`. |
| Existing `StartAllocation` shape and matching `alloc_id`, `spec.alloc`, and SVID identity | PASS | `start_allocation_action` and `allocation_spec` at `workload_lifecycle.rs:1404-1457` construct the existing action and derive all three identity values from one `AllocationId`. |
| Existing `RestartAllocation` remains same-ID for Exec | PASS | `restart_allocation_action` at `workload_lifecycle.rs:1459-1477` selects same-ID `RestartAllocation` for non-VM drivers; no action-shim code changed. |
| Exact private helper signatures | PASS | `restart_allocation_action(job, desired, row, attempt) -> Action` and `next_vm_attempt(allocs, view) -> Option<u32>` are present at `:1461-1466` and `:1481-1485`. |
| Numeric maximum across retained rows and View reservation keys | PASS | `next_vm_attempt` takes the maximum parsed suffix from both input populations and uses checked successor arithmetic. |
| Candidate-keyed retry policy and Workload Failure/reclamation carry semantics | PASS | `reconcile_inner` reads `failed.alloc_id` policy at `:947-1042`; VM writes the advanced or carried values at `:1053-1070`. `next_evaluation_at` mirrors candidate selection at `:365-378`. |
| View-only reservations are non-current; history remains row-owned | PASS | `current_alloc` receives only `actual.allocations`; reservations are consulted only by `next_vm_attempt`. No observation or persistence shape changed. |
| Exhaustion and malformed-suffix behavior | PASS | Checked successor returns `None` at `:1485`; the step's boundary and malformed-suffix tests pass. |
| No unsanctioned public surface or architecture | PASS | The commit adds no public method, type, field, variant, trait, parameter, dependency, retry, sleep, lock, compatibility path, or persistence mechanism. The only core-file change is semantic rustdoc. |

## Contract Shape Compliance — iteration 1 assessment

**Overall: FAIL pending one rustdoc-only remediation.**

The fourteen step-owned pure-function tests all retain the exact required
`/// CONTRACT_SHAPE: pure-function.` declaration, and no step-owned test name
matches the banned implementation-detail pattern. The tests are return-only
tests through the public `Reconciler::reconcile` / `next_evaluation_at` driving
ports; they do not invoke private helpers or mock the hexagon.

The reviewer role's mechanical Contract Shape gate also requires the literal
rustdoc line `/// Outcome anchor: DISCUSS Elevator Pitch` on every transitioned
acceptance test. That line is absent from all twelve transitioned tests in
`crates/overdrive-reconcilers/tests/acceptance/vm_recreation_allocation_identity.rs`
and both transitioned tests in
`crates/overdrive-core/tests/acceptance/vm_reclamation_plan_purity.rs`.
The feature's DISCUSS artifacts are intentionally absent, but that does not
remove the mechanical metadata requirement for a transitioned acceptance body.
The remediation is metadata-only and does not invent a production contract.

No `unbounded-preservation` or `bounded-change` test was newly activated in
this step, so those shape-specific judgment checks are not applicable here.

## Quantitative validation

The roadmap maps fourteen distinct step scenarios to fourteen transitioned
pure-function test bodies: twelve in the reconciler acceptance file and two
in the core reclamation acceptance file. Counting each mapped scenario as one
observable behavior gives a test budget of `14 × 2 = 28`; the step activates
14 tests, so the budget passes. The generic one-active-acceptance rule is not
applied because the approved roadmap explicitly authorizes this step's batch
of fourteen pre-authored pure scenarios.

The tests enter through the public reconciler driving port and assert returned
actions/views plus immutable input rows. No domain-layer object is mocked or
tested through a private production seam. The step is not a walking skeleton;
the approved seeded production-owner composition and native effects are
reserved for steps `01-02` and `01-03`.

## DES and commit verification

| Check | Result | Evidence |
|---|---|---|
| Roadmap validation | PASS | Roadmap `validation.status` is `approved`; its approved review records 21/21 scenario mappings. |
| DES phase order | PASS | `deliver/execution-log.json` records `RED EXECUTED FAIL`, `GREEN EXECUTED PASS`, then `COMMIT EXECUTED PASS` for `01-01`. |
| Commit scope | PASS | `git show --stat` reports exactly the four roadmap files: two production/action-contract files and two acceptance files. |
| Attribution | PASS | Commit retains Marcus Schack Abildskov as author/committer, has exactly one `Co-Authored-By: Codex <codex@openai.com>`, and has `Step-Id: 01-01`. |
| Diff hygiene | PASS | `git diff --check a0f19ebe^ a0f19ebe` is clean. |
| Test integrity | PASS | Test-file changes remove only the authorized step-01-01 `#[ignore]` markers; no assertion, fixture, expected value, or test body was weakened or deleted. |

## Verification evidence

The following commands were run through the repository's required Lima
runner during this review:

| Command / evidence | Result |
|---|---|
| Focused 01-01 selection across `overdrive-reconcilers` and `overdrive-core` | PASS — 14/14 |
| Full `overdrive-reconcilers` acceptance binary | PASS — 19/19 |
| Full `overdrive-core` acceptance binary | PASS — 557/557 |
| Retained proptest replay selection with `PROPTEST_CASES=0` | PASS — 3/3 selected replay tests; no collection or source-parallel warning |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `PYTHONPATH=/Users/marcus/.claude/lib/python cargo xtask dst-lint` | PASS |
| Mutation testing | Not run, as expressly prohibited inside an individual roadmap step |

The focused pure tests establish action identity agreement, numeric current
selection, candidate-only deadlines, carried policy inputs, reservation
consumption, malformed suffix handling, and checked exhaustion. The broader
acceptance runs preserve the existing Exec, lifecycle, and action-shim
contracts. They do not substitute for the later seeded owner-path and
qualified-metal evidence required by the roadmap.

## Findings

| ID | Severity | Reachability / evidence | Remediation disposition |
|---|---|---|---|
| R01 | Blocker — mechanical Contract Shape compliance | Every transitioned acceptance test lacks the required exact outcome-anchor line. Locations: `vm_recreation_allocation_identity.rs:209,237,261,287,315,342,370,389,405,423,440,474` and `vm_reclamation_plan_purity.rs:491,557`. The absence is observable whenever these live tests are compiled/reviewed; green execution cannot supply missing metadata. | **OPEN.** Return to the original step-01-01 crafter for one rustdoc-only insertion, `/// Outcome anchor: DISCUSS Elevator Pitch`, immediately after each existing Contract Shape declaration. Preserve all test bodies, assertions, fixtures, sidecars, production API, and behavior. Re-review this same step after remediation. |

No production correctness, API-shape, scope, test-honesty, or reachability
finding is open beyond R01. Static suspicions about malformed IDs outside the
accepted state contract, driver changes across one immutable workload intent,
and cleanup ordering were not promoted to findings because no reachable
failure was reproduced through the current production owner path.

## Review iteration 1 verdict

**CHANGES_REQUESTED.** The implementation satisfies ADR-0104's exact VM/Exec
identity and candidate-memory contract, has clean DES/commit mechanics, and
all executed verification layers pass. The fourteen transitioned acceptance
tests are missing the reviewer-mandated outcome-anchor metadata. This is a
bounded rustdoc-only remediation; no production change or API invention is
authorized. Step `01-02` must not start until this same reviewer records an
approved re-review artifact update.

## Review iteration 2 — finding disposition and re-review

### Metadata

| Field | Value |
|---|---|
| Reviewed commit | `a0f19ebe8008f85136e8f619cb909de1243672f6` |
| Reviewer | Same step-specific implementation reviewer |
| Review date | 2026-09-13 |
| Prior finding | R01 — missing outcome-anchor metadata |
| Current verdict | **APPROVED** |

### R01 disposition

R01 is **REJECTED**, with no remediation authorized. The repository's exact
policy search found no accepted requirement for an
`/// Outcome anchor: DISCUSS Elevator Pitch` line. `AGENTS.md:265-267`, the
approved roadmap step, and the feature-delta mandate the exact per-test
`/// CONTRACT_SHAPE: pure-function.` declaration, which all fourteen
step-owned tests retain. The feature explicitly has no DISCUSS artifacts, so
the proposed literal would not be a resolvable design anchor. The requested
outcome is the fresh VM allocation identity decision and its pure acceptance
evidence; no production or test behavior fails because this reviewer-only
metadata rule is absent.

The initial finding therefore cannot be promoted to a design, implementation,
or test remediation requirement. Adding the line would modify the
pre-authored acceptance metadata outside the accepted step contract and would
be out of scope. No test, production source, API, DES event, or commit was
changed during this re-review.

### Iteration 2 verification

The iteration-one implementation and verification evidence remains valid:

- the exact fourteen focused pure tests pass, including the selected retained
  proptest replay run;
- full `overdrive-reconcilers` acceptance passes 19/19 and full
  `overdrive-core` acceptance passes 557/557;
- Lima-routed workspace check, workspace clippy with `-D warnings`, formatting,
  and `dst-lint` pass;
- the committed diff remains limited to the four roadmap files, with the
  required author, single Codex co-author trailer, and `Step-Id: 01-01`;
- no mutation testing was run, as required for an individual roadmap step.

The exact design/API checklist remains fully passing: VM starts use
`StartAllocation` with one fresh identity and pre-dispatch View reservation;
Exec retains same-ID `RestartAllocation`; candidate policy remains
candidate-keyed; numeric suffix exhaustion is checked; and no unsanctioned
public or persistence surface is present.

### Finding and iteration history

| Iteration | Reviewed target | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `a0f19ebe8008f85136e8f619cb909de1243672f6` | **CHANGES_REQUESTED** | R01 | Returned for policy adjudication; no code remediation. |
| 2 | `a0f19ebe8008f85136e8f619cb909de1243672f6` | **APPROVED** | 0 open | R01 rejected as out of scope and unproven; no remediation. |

## Final verdict after iteration 2 (superseded by post-push re-review)

**APPROVED.** No proven, reachable, in-scope implementation, API-shape,
test-integrity, scope, or verification defect remains for step `01-01`.
ADR-0104's fresh VM identity contract is implemented with the exact private
helper signatures and unchanged public action/port shapes; the fourteen
roadmap scenarios pass with their required pure-function Contract Shape
declarations; DES and commit evidence are complete; and the reviewed code
passes the focused and affected acceptance gates. Step `01-01` may advance to
the next roadmap step.

## Review iteration 3 — post-push fixture remediation

### Metadata

| Field | Value |
|---|---|
| Feature | `vm-recreation-allocation-id-reuse` |
| Step | `01-01` — Reserve fresh VM execution identities |
| Original implementation | `a0f19ebe8008f85136e8f619cb909de1243672f6` |
| Remediation commits | `f049c06fbf42b7931844ef29991016f31e78c7c6`; `d7d3ab123139a39ec57618cb3748496d5fc417d2` |
| Remediation scope | `crates/overdrive-control-plane/src/vm_reclamation_boot.rs` tests and DES execution log only |
| Reviewer | Same step-specific implementation reviewer |
| Review date | 2026-09-13 |
| Prior status | Iteration 2 **APPROVED**; post-push stale-assertion failures reopened the step for this bounded re-review |
| Current verdict | **APPROVED** |

### Post-push failure and remediation disposition

The second actual `01-01` RED phase is recorded in
`deliver/execution-log.json` at `2026-09-13T09:54:52Z`, followed by GREEN PASS
at `10:02:43Z` and COMMIT PASS at `10:03:03Z`. The two reported failures were
the pre-feature same-ID assertions in the existing boot-reclamation tests:
`same_boot_epoch_claims_each_unsupervised_allocation_once` and
`boot_reclamation_executor_persists_one_platform_claim_and_one_lifecycle_redrive`.
Their expected `Action::RestartAllocation`/predecessor-ID checks were stale
for a VM workload after ADR-0104; no production failure or new architecture
gap was reported.

The original step crafter's remediation is **accepted as necessary and
bounded**:

- `f049c06f` changes only the `#[cfg(test)]` module in
  `vm_reclamation_boot.rs` plus the DES log. It adds the local
  `assert_fresh_vm_start` assertion helper at `:240-260`, which requires one
  existing `StartAllocation`, the expected fresh `AllocationId`, the stable
  `WorkloadId`, `spec.alloc == action.alloc_id`,
  `SpiffeId::for_allocation(workload_id, alloc_id)`, a VM payload, and no
  `RestartAllocation`.
- The first boot test still proves exactly one Platform Reclamation claim:
  `plan_reclamation` returns one `ReclaimAllocation` at
  `vm_reclamation_boot.rs:353-356`, and the exact reclaimed postcondition
  returns no second claim at `:378-389`. Its lifecycle decision now emits
  fresh IDs `...-1` and, when a second decision is explicitly evaluated
  against the still-unpublished predecessor row, `...-2` at `:421-431`.
  The latter is a fresh retry decision above the View reservation, not a
  second Platform Reclamation claim; it is required by ADR-0104's
  “reservation keys are consumed and never current” rule.
- The composed boot test still drives the real `converge` owner and observes
  the old row as `Terminated / Stopped { by: PlatformReclaimed }` at
  `:530-547`. It asserts one fresh identity re-drive with matching action,
  spec, VM payload, and SVID identity at `:572-586`, and the repeated
  `converge` pass still leaves the old terminal row byte-equal at `:597-607`.
  The second pure lifecycle evaluation uses the next fresh ID
  (`:587-595`) so a consumed predecessor identity cannot be reused.
- `d7d3ab12` adds only the second `01-01` COMMIT PASS event to the DES log.

The remediation does not alter `ServiceLifecycle`, `SvidLifecycle`, their
handoff producers, or any production action-shim/reclamation code. The
affected boot test file had no independent ServiceLifecycle/SvidLifecycle
handoff assertion to remove; the separate handoff suites remain unchanged.
No acceptance assertion, fixture precondition, Contract Shape classification,
or proptest sidecar was weakened. Both remediation commits retain Marcus
Schack Abildskov as author and committer, exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer, and `Step-Id: 01-01`.

### Iteration 3 design-contract re-check

| Contract | Result | Evidence |
|---|---|---|
| Platform Reclamation remains owned by the existing boot executor | PASS | The remediation leaves `converge`, `plan_reclamation`, `execute_reclaim_allocation`, and the old-row observation path unchanged. |
| VM re-drive uses existing fresh `StartAllocation` | PASS | `assert_fresh_vm_start` matches only `Action::StartAllocation` and validates action/spec/SVID identity agreement. |
| VM never emits same-ID `RestartAllocation` | PASS | The helper asserts no `RestartAllocation` in both changed VM lifecycle checks. |
| Exec semantics remain unchanged | PASS | No Exec fixture or production code changed; the existing `RestartAllocation` branch in `workload_lifecycle.rs:1073-1088` remains untouched by the remediation. |
| One claim and exact old-row cleanup remain intact | PASS | Both tests retain the one-claim planner postcondition and the repeated boot `converge` old-row equality assertion. |
| Independent ServiceLifecycle/SvidLifecycle handoffs remain intact | PASS | No producer, consumer, action-dispatch, or handoff test outside the two stale VM assertions changed. |
| Public/API/persistence boundary | PASS | The remediation adds no public method, type, field, variant, trait, parameter, persistence mechanism, retry, sleep, lock, or compatibility branch. |

The second pure lifecycle evaluation is not a hidden idempotency claim: an
unaccepted `StartAllocation` has consumed its reserved identity, so a later
decision must choose above that reservation. The actual production owner path
handles successful publication by changing the accepted-row current
projection; the already-reviewed seeded owner test handles rejected
publication and its higher-ID retry. No ordering or concurrency defect is
being inferred from the fixture.

### Iteration 3 verification

| Command / evidence | Result |
|---|---|
| Lima-routed focused boot-reclamation selection for `same_boot_epoch_claims_each_unsupervised_allocation_once` and `boot_reclamation_executor_persists_one_platform_claim_and_one_lifecycle_redrive` | PASS — 2/2 (nextest run `ba28a716-a9d3-4f46-91aa-b8c33402c1e0`) |
| Lima-routed `overdrive-reconcilers` + `overdrive-core` acceptance selection | PASS — 576/576 (nextest run `4733a11b-8144-495e-9854-effc370f6d7e`) |
| Orchestrator-reported full `overdrive-control-plane` post-push suite | PASS — 565/565 |
| Lima-routed extended `overdrive-control-plane --features integration-tests` run | PASS — 810/810 executed; nextest reported one leaky test but no failure, outside the remediation diff |
| Lima-routed workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima-routed workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `PYTHONPATH=/Users/marcus/.claude/lib/python cargo xtask dst-lint` | PASS |
| Mutation testing | Not run, as expressly skipped for individual steps and by the user-directed final disposition |

### Iteration 3 finding disposition and history

| Finding / iteration | Status | Disposition |
|---|---|---|
| R01 from iteration 1 — outcome-anchor metadata | **REJECTED** | No repository/design mandate; feature has no DISCUSS artifacts; no remediation authorized. |
| Iteration 2 | **APPROVED** | Exact ADR-0104 implementation and 14/14 pure scenarios passed. |
| Post-push remediation review | **APPROVED** | Two stale same-ID assertions were replaced by exact fresh VM action/spec/SVID checks; one Platform Reclamation claim, old-row idempotence, and independent handoffs remain intact. |

## Final verdict after iteration 3

**APPROVED.** The post-push failures were stale same-ID expectations in two
existing VM boot-reclamation tests, not a production or design defect. The
original crafter's test-only remediation now checks the exact existing fresh
`StartAllocation` contract, proves matching `AllocationId`/`AllocationSpec`/
`SpiffeId` identity, rejects VM `RestartAllocation`, preserves the one
Platform Reclamation claim and old terminal-row complement, and leaves Exec,
ServiceLifecycle, and SvidLifecycle behavior untouched. The second actual
`01-01` RED → GREEN → COMMIT trace is complete, all independent verification
commands are green, and no in-scope finding remains. Step `01-01` is approved
to remain complete.
