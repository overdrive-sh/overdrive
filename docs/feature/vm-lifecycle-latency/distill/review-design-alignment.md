# Independent DISTILL architecture and roadmap review — vm-lifecycle-latency

## Metadata

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Review role | `nw-solution-architect-reviewer` |
| Model | GPT-5.6 Luna, maximum thinking (inherited session model) |
| Reviewed baseline | `cc94b7c8` (approved DESIGN baseline) plus the current DISTILL working-tree diff |
| Review date | 2026-09-10 |
| Reviewed artifacts | DISTILL sections in `feature-delta.md`, `deliver/roadmap.json`, ADR-0102, ADR-0103, approved DESIGN review, changed tests, example and E09 expectation interfaces |
| Native state | Pending; no native profile distribution or amended E09 capture is claimed |
| Iteration 1 verdict | **CHANGES_REQUESTED** for roadmap readiness; architecture alignment itself is approved |
| Iteration 2 verdict | **APPROVED** after documentation-only remediation |

## Review mandate and authority

This review covers the new DISTILL sections, the pending delivery roadmap, and
the changed test, example, harness, and expectation interfaces. It does not
reopen the approved architecture, redesign the lifecycle, or assess native
performance as if it had been measured.

The authoritative contract is the user-ratified [ADR-0102](../../../product/architecture/adr-0102-bounded-convergence-evaluation-ownership.md)
and [ADR-0103](../../../product/architecture/adr-0103-responsive-vm-stop-and-guest-supervision.md),
with the approved DESIGN review at
`docs/feature/vm-lifecycle-latency/design/review.md`. ADR-0102 pins the five
timestamped broker/router/shim signatures, eight complete evaluation slots,
one active `TargetResource` lease, result-consumption ownership, and the
convergence-owner `pending_at_exit` snapshot. ADR-0103 preserves the public
`Driver::stop`, `Vmm::terminate`, and `BeaconWriter::request_stop` interfaces
and pins the private guest-init boundary, one child-led process group, the
five-second guest grace, and immediate poweroff after completion. The DISTILL
sections correctly state that these decisions are unchanged.

The user's refinement is also binding: use Sim and in-process evidence where
they establish the contract, reuse the existing E09-v2 expectation for its
existing operator outcome, add no expectation ID without a distinct outcome,
and skip DEVOPS. The roadmap remains `validation.status: pending`, as required;
this review does not self-approve it.

## Architecture alignment

| Dimension | Evidence | Result |
|---|---|---|
| Problem and priority | The DISTILL walking skeleton is S-VLL-01: an HTTPS-submitted workload reaches `Running` while a different production-owner effect is held (`feature-delta.md:421-425`). The history amendment traces the global E09 gate and restores the independently observed worker path (`:399-417`). | Pass |
| Ownership and state | The component and lifecycle tables preserve aggregate workload `View` ownership, runtime fsync-before-dispatch, action-shim and exit-observer authorities, `EndingInFlight`, and the narrow generic stop result (`feature-delta.md:121-149`). The shutdown snapshot wording matches ADR-0102's locked broker read and excludes later submissions. | Pass |
| API, wire, and persistence boundaries | The current source diff adds no production API. The roadmap explicitly requires ADR-0102's exact public signature changes and unchanged `Evaluation`, `BrokerCounters`, wire, and `run_convergence_tick` shapes (`roadmap.json:39-43`). ADR-0103's unchanged public interfaces and private init changes are represented in S-VLL-07–10 (`feature-delta.md:444-447`). | Pass |
| Feasibility and effect isolation | Sim tests drive the production server through HTTPS with injected `Driver`/`Clock` ports; the semaphore substitutes only external Driver latency. Native Rust scaffolds stay in the existing in-process integration composition, while E09 remains a built-product black-box expectation. The Contract Shape declarations cover the changed source-local tests and shell checks. | Pass |
| External validity and evidence honesty | Step 01-03 retains the real E09 example and harness plus E06/E08/E10/E11 boundaries; the expectation is reset to `pending`. The author explicitly records the focused Rust result as expected RED/pending panics and the accidental native diagnostic as no validation or measurement (`feature-delta.md:500-558`). | Pass |
| Priority and sequencing | The three steps are correctly ordered as complete convergence ownership, responsive host/guest termination, then native measurement/E09 validation. No new scheduler, persistence, recovery, guest ACK, or DEVOPS mechanism is introduced. | Pass |

The history correction is compatible with the approved design: removing the
per-cohort `failure-submit-release` join and restoring the 60-second
observation/600-second owner windows is the stated S-VLL-12 amendment. It does
not change the lifecycle ownership or turn the E09 example into a capacity
claim.

## Exact interface and test-surface audit

The current implementation is intentionally still pre-GREEN. The existing
source shows the old broker signatures at
`crates/overdrive-core/src/eval_broker.rs:85,96`, the old
`InterestRouterBroker` constructors at
`crates/overdrive-control-plane/src/lib.rs:3447,3455`, and the old action-shim
dispatch at `crates/overdrive-control-plane/src/action_shim/enqueue_evaluation.rs:50`.
The roadmap's first criterion correctly requires an atomic migration to the
exact ADR-0102 signatures rather than a compatibility method. The constructor
implementation's actual owner is `lib.rs`, which matters to roadmap locator
precision below.

The changed test surfaces are honest DISTILL specifications:

| Surface | Current state | Contract Shape / boundary result |
|---|---|---|
| Core broker | Two explicit pending-body S-VLL-05a/b tests use `#[should_panic(expected = "RED scaffold")]` and describe the independent reference model, timestamp domains, replacement age, eligible FIFO, counter deltas, and fairness. | `pure-function` declarations are present; no compatibility API is added. |
| Sim production owner | The existing two held-effect tests remain live RED; four seven/eight-slot tests and two admission-close tests add real HTTP/server-owner assertions; the healthy control remains green. Two owner-lease/report tests remain explicit pending scaffolds. | `bounded-change` declarations are present; the semaphore is test-only and does not create a production seam. |
| Guest init | S-VLL-09 is a live RED trace through the existing generic lifecycle composition. The helper still contains the old shutdown callback until GREEN changes the approved private signature. | `pure-function` declaration is consistent with the existing callback-trace tests. |
| Worker stop | S-VLL-07 is a live SimVmm/BeaconWriter ordering RED test; S-VLL-08 is an explicit pending matrix for writer outcomes and overlapping VMM grace. | `bounded-change` declarations are present; the test does not strengthen `Driver::stop Ok`. |
| Native init and profiles | S-VLL-10a/b and S-VLL-11 are named pending native in-process scaffolds. They do not spawn the Overdrive binary or emit expectation evidence. | `bounded-change` declarations are present; native kernel/process and quantile obligations remain pending. |
| E09-v2 | The checked-in example, host-safe scheduler test, expectation README/runner, index, and harness fixture are updated in place. The expectation and index are `pending`; no new ID is introduced. | The example remains the operator-runnable product journey and the harness remains the black-box evidence boundary. |

The author evidence reports ten live behavioral RED tests, eight explicit
pending-body scaffolds, and one healthy control. Its selected Rust command
reports 16 hook-compatible expected passes, but the retained output shows ten
expected behavioral failures, five selected pending-body panics, and one
healthy control; this is not acceptance completion. The three native scaffolds
were compiled but not executed. That accounting is faithful to the repository
RED rule.

## Mandatory roadmap checks

| Check | Result | Evidence and assessment |
|---|---|---|
| 1. External validity | **PASS** | Step 01-03 drives the existing default-feature E09 product through `verification/harness/run-expectation.sh` and retains the existing E06/E08/E10/E11 black-box lanes. The native profile owner is an in-process production composition, as required by the accepted design. |
| 2. Acceptance-criterion coupling | **PASS under the approved contract** | Criterion 01-01 names exact public signature changes because ADR-0102 makes those shapes contractual and the repository forbids invented compatibility APIs. `pending_at_exit`, target leases, and stage events are also named approved observable contracts. No new private decomposition or unsanctioned public surface is prescribed. |
| 3. Step decomposition | **PASS** | Three steps target seven concrete production paths in their implementation scopes (and 14 unique listed file/directory entries overall), a ratio below 2.5. The steps are distinct dependent boundaries rather than three substitution copies. |
| 4. Implementation code | **PASS** | The roadmap contains behavioral criteria, approved signatures, verification commands, and constraints, but no method body, algorithm, pseudocode, or loop implementation. |
| 5. Concision and precision | **FAIL — blocking finding R-01** | `jq -r '.. | strings' docs/feature/vm-lifecycle-latency/deliver/roadmap.json | wc -w` returns 1125. The small-roadmap ceiling is 500. All three descriptions are under 50 words and each step has five criteria, but criteria 01-02/5 and 01-03/5 are 31 words against the 30-word local limit. See R-01. |
| 6. Unit/acceptance boundaries | **PASS with locator finding R-02** | Sim and native Rust tests stay in-process; E09 remains a built-product black-box expectation; host-safe shell checks do not claim native outcomes. The grouped step locator metadata does not enumerate all named scenarios, which is a traceability gap rather than a boundary collapse. |

## Findings, evidence, and required dispositions

### R-01 — Roadmap exceeds the mandatory small-roadmap concision gate

**Severity:** Blocker for roadmap approval

**Location:** `docs/feature/vm-lifecycle-latency/deliver/roadmap.json:15-210`;
the quantitative rule is the `nw-roadmap-design` / `nw-roadmap-review-checks`
small-roadmap limit.

The roadmap has 1125 whitespace-delimited words across JSON string values,
against the 500-word/token ceiling for a one-to-three-step roadmap. The three
steps repeat the full RED/GREEN/COMMIT/reviewer workflow, exact ADR authority,
acceptance-designer boundary, mutation rule, and file-list guidance in nearly
identical 61-word implementation notes. The two 31-word criteria also exceed
the local 30-word criterion limit. This is a roadmap-gate failure, not a claim
that the approved architecture is too broad or that any production behavior
should be removed.

**Required disposition:** Compress repeated workflow and scope text into one
roadmap-level note and shorten redundant step notes/metadata until the measured
total is within the small-roadmap ceiling. Retain the exact approved API
constraints, scenario IDs, lifecycle owners, verification commands, native
pending status, and all load-bearing acceptance behavior. Do not solve the
count by deleting the S-VLL contract or by expanding the architecture.

### R-02 — Grouped steps do not provide an exact locator for every owned scenario

**Severity:** Medium, nonblocking after R-01

**Locations:** `roadmap.json:45-56`, `:102-110`, and `:157-162`.

Each step lists multiple `scenario_ids` but has only one `scenario_name`:

| Step | Scenario IDs | Current locator |
|---|---|---|
| 01-01 | S-VLL-01 through S-VLL-06 and S-VLL-13 | `slow_start_does_not_block_independent_convergence` |
| 01-02 | S-VLL-07 through S-VLL-10b and S-VLL-13 | `completed_shutdown_write_has_no_two_second_floor` |
| 01-03 | S-VLL-11 through S-VLL-13 | `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials` |

The authoritative DISTILL table does name the other executable functions and
pending bodies (`feature-delta.md:437-450`), so this is not missing acceptance
specification. It does make the roadmap's `test_file`/`scenario_name` fields
ambiguous for a fresh crafter and reviewer: the primary locator covers only
one of nine, six, or three listed scenarios. S-VLL-13 is intentionally a
cross-step preservation regression, but that shared status should be explicit.

**Required disposition:** Add exact per-scenario locators in the roadmap (for
example, a comma-separated `scenario_name` value following existing repository
convention, or a compact mapping field) and mark S-VLL-13 as a shared
regression gate. Preserve the existing feature-delta scenario contracts and do
not add production API or new test seams.

### R-03 — Roadmap file-list precision has one nonexistent path and duplicates

**Severity:** Low, nonblocking

**Locations:** `roadmap.json:60-89` and `:116-144`.

Step 01-01 names `crates/overdrive-control-plane/src/interest_router.rs`, but
that file does not exist; `InterestRouterBroker` is defined in
`crates/overdrive-control-plane/src/lib.rs:3437-3463`, which is also listed.
`eval_broker.rs` is repeated at lines 83 and 88, and `overdrive-init/src/main.rs`
is repeated at lines 139 and 142. The roadmap correctly says file lists are
guidance and includes the real `lib.rs`, so this does not block implementation
or indicate architecture drift.

**Disposition:** Correct the stale path and duplicate entries during the R-01
roadmap edit. No production change is required.

## Non-findings and explicit limitations

The current source diff does not add a public method, type, enum variant,
trait, wire frame, persistence store, recovery protocol, or lifecycle owner.
The exact signature changes are future DELIVER work named by ADR-0102/0103,
not an implementation improvisation in the DISTILL diff. The E09 global-gate
removal and 600-second/60-second window restoration are the user-approved
history amendment. The repeated S-VLL-13 regression appears in all dependent
steps as a preservation gate; it does not create a second ownership contract.

No new production defect was inferred from a cancellable future, a test-only
abort, a hypothetical scheduler order, or the absence of native measurements.
The native diagnostic archive is explicitly labeled incomplete and not a
successful expectation or latency capture, so it is retained as a limitation.
The roadmap's pending status and final wave-level mutation gate are correct;
mutation must remain after all three step reviews and native evidence.

## Verification performed

| Check | Result |
|---|---|
| Read required project rules, review brief, accepted architecture brief section, ADR-0102, ADR-0103, and approved DESIGN review | Pass |
| `jq empty docs/feature/vm-lifecycle-latency/deliver/roadmap.json` | Pass |
| `jq -r '.. \| strings' docs/feature/vm-lifecycle-latency/deliver/roadmap.json \| wc -w` | 1125; fails the 500-word small-roadmap gate |
| `git diff --check cc94b7c8` over the DISTILL paths | Pass |
| Author's focused Rust record `.context/vm-lifecycle-latency-distill/rust-r4vu5xqj/` | Exit 0 for expected RED/pending panics and one healthy control; no GREEN claim |
| Author's host-safe scheduler record `.context/vm-lifecycle-latency-distill/shell-eqtv8rcs/scheduler.txt` | Pass; surrogate orchestration only |
| Author's host-safe runner record `.context/vm-lifecycle-latency-distill/shell-eqtv8rcs/runner.txt` | Pass; external timeout/transcript mechanics only |
| Author's unchanged-baseline runner record `.context/vm-lifecycle-latency-distill/runner-baseline-jypkq8wq/0.txt` | Expected pre-existing fixture failure reproduced; not amended evidence |
| Full native expectation suites, native profile distributions, amended E09 capture, mutation testing | Not run; correctly pending for DELIVER |

The review did not run a full native expectation suite or mutation test. The
available records and static checks are sufficient to assess the contract and
roadmap gate, while the unmeasured native distributions remain an explicit
later acceptance obligation.

## Iteration and remediation disposition

### Iteration 1 — initial independent review

The DISTILL sections and current diff were reviewed against the ratified ADR
signatures, ownership/state boundaries, evidence-layer rules, and all six
roadmap checks. Architecture alignment, evidence honesty, and test boundaries
passed. R-01 was opened as a blocking roadmap concision failure. R-02 was
opened as a medium scenario-locator ambiguity, and R-03 as a low file-list
precision issue. No remediation was applied by this review, and no other agent
or reviewer was asked to change the architecture.

## Iteration 1 verdict

**CHANGES_REQUESTED.** The approved architecture and current DISTILL test /
expectation boundaries are aligned, and the pending native state is honest.
The roadmap must first close R-01's mandatory small-roadmap concision gate;
R-02 should be corrected at the same documentation pass so every grouped
scenario has a mechanical locator. R-03 is nonblocking hygiene. This verdict
does not authorize implementation, native validation, mutation testing, or a
new architecture mechanism.

## Iteration 2 — remediation re-review

**Review basis:** The author supplied a documentation-only remediation. The
current `roadmap.json` and the DISTILL sections were re-read against the
ratified ADR-0102 and ADR-0103 contracts, the approved DESIGN review, and the
original changed test/example/expectation boundaries. No production, test,
example, harness, or expectation behavior changed, and no ADR decision was
altered; the remediation changed only roadmap and feature-delta documentation.
The roadmap remains `validation.status: pending` for root's aggregation; this
review assesses its readiness and does not mutate that status.

The accepted architecture remains intact. The only pre-existing feature-delta
changes outside the new DISTILL sections are the user-directed strategy
amendment to use a native integration-test guest fixture and the user-directed
reuse of E09 v2 for the public product outcome. The current delta still carries
the exact broker and private guest-init obligations, owner/snapshot boundary,
stage/event contracts, restored E09 windows, evidence-layer boundaries, and
native pending limitation. No new API, lifecycle owner, scheduler, persistence,
recovery, protocol, or DEVOPS requirement is introduced.

### R-01 disposition — CLOSED

The remediation reduces the measured roadmap string population to **497
words**, below the 500-word ceiling for a one-to-three-step roadmap. Each step
has five criteria; the largest criterion is 13 words and the largest step
description is three words. The compact roadmap still retains the three
dependent boundaries, exact ADR references and signature constraint, all
scenario IDs, exact verification commands, the `validation.status: pending`
state, and the final wave-level mutation command.

The compression preserves load-bearing content in the authoritative
feature-delta handoff. Its `Delivery execution gates` section keeps the full
scenario oracles, exact locator rule, S-VLL-13 shared-gate requirement,
compiler-fallout boundary, RED/pending-body distinction, fresh-agent and
on-disk-review rules, final mutation timing, and DEVOPS exclusion. The
scenario table continues to carry the accepted ownership, ordering, timing,
failure, cleanup, E09, and native-measurement obligations. The roadmap is
therefore concise without deleting an approved contract or outsourcing
acceptance-specification design. **R-01 is closed.**

### R-02 disposition — CLOSED

The current `roadmap.json#/scenario_locators` mapping contains all 16 named
S-VLL keys and 52 exact locators. A static recheck found every referenced path
and every terminal function symbol present. Every scenario ID listed by a step
has a locator, and no locator key is orphaned. S-VLL-13 is explicitly declared
in `shared_regression_gate` for steps 01-01, 01-02, and 01-03.

The delivery gate now explains that each step's `test_file` and
`scenario_name` are its primary entry locator while `scenario_locators` is the
complete mapping. This keeps the schema compact while giving a fresh crafter
and reviewer an exact file/function or shell command/runner for every owned
scenario. The locators preserve the required boundary split: Sim and native
Rust tests use the production composition in process, pure broker properties
remain source-local, and S-VLL-12 uses the checked-in E09 example and external
runner. No public test seam or production API was added. **R-02 is closed.**

### R-03 disposition — CLOSED

Each step's `files_to_modify` list now contains existing paths with no
within-step duplicate. The stale `crates/overdrive-control-plane/src/interest_router.rs`
entry is gone; the actual `InterestRouterBroker` owner remains the listed
`crates/overdrive-control-plane/src/lib.rs`. The earlier repeated
`eval_broker.rs` and `overdrive-init/src/main.rs` entries are gone. The CLI
integration module appears in both 01-02 and 01-03 intentionally because it
owns distinct S-VLL-10 and S-VLL-11 scenarios; that cross-step reuse is
consistent with the exact locator map and is not a duplicate entry within a
step. **R-03 is closed.**

### Iteration 2 roadmap readiness

| Check | Result | Evidence and assessment |
|---|---|---|
| External validity | **PASS** | 01-03 retains the existing E09 v2 example, black-box runner and E06/E08/E10/E11 lanes; internal stage and guest-group obligations remain in-process native tests. The completed roadmap still yields an invocable public journey. |
| Acceptance-criterion coupling | **PASS under the ratified contract** | Criteria describe observable lifecycle behavior. The exact ADR-0102 signature requirement is an explicit user/design contract and is necessary to prevent invented public surface; it does not prescribe an unapproved implementation. |
| Step decomposition | **PASS** | There are three steps and six unique production source files in their file lists, for a 0.5 ratio. The steps are distinct convergence, termination, and native-evidence boundaries; the shared CLI test module serves different scenarios. |
| Implementation code | **PASS** | The roadmap contains no method body, pseudocode, algorithm, loop, or implementation recipe. |
| Concision and precision | **PASS** | The independently measured string population is 497 words; criteria max out at 13 words, descriptions at three, and each step has five criteria. Repeated delivery constraints live once in the feature-delta gate. |
| Unit/acceptance boundaries | **PASS** | The 52 locators distinguish in-process Rust/Sim evidence from the built-product E09 expectation. The retained commands do not make expectations invoke Cargo, a Rust test binary, or an overdrive crate. |

The current roadmap is implementation-bound without authorizing improvisation:
the feature delta supplies the complete scenario oracles and constraints, the
roadmap supplies the ordered delivery steps and exact verification commands,
and `validation.status` truthfully remains pending until root aggregates this
review with the other gates. Native distributions and the amended E09 capture
remain future evidence, as explicitly recorded; their absence is a limitation
of current validation rather than architecture drift. The 01-03 criterion that
native acceptance remains pending is a status guard at this handoff; it does not
weaken the same step's requirement for future native distributions and E09
evidence before validation can advance.

## Final verdict

**APPROVED.** R-01, R-02, and R-03 are closed by the documentation-only
remediation. The accepted architecture, exact API boundaries, lifecycle
ownership, evidence-layer separation, and user-directed E09/Sim strategy are
preserved. The roadmap is concise, traceable, concrete, and ready for the
separate DELIVER workflow while its pending validation status and native
evidence obligations remain honest.
