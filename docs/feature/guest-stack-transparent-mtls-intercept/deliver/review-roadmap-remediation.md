# DELIVER roadmap remediation review

- **Review ID:** `roadmap_rev_20260829_022032_iteration_1`
- **Reviewer:** `nw-solution-architect-reviewer` (fresh isolated reviewer)
- **Reviewed commit:** `b4b94caa448be276200af34daf2d9a66b4e3aaa9`
- **Parent:** `558589a7a3ee14ebea0b8cdb6496b65a5830f777`
- **Artifact:** `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json`
- **Iteration:** 1
- **Final verdict:** **NEEDS_REVISION**

## Executive summary

The remediation fixes the large structural problems in the prior roadmap. Five
completed steps are now historical-only and trace to their full on-disk review
artifacts; fresh work starts at 02-03; the dependency chain puts the universal
metal lease/native preflight before E07-E09; Q7, D7, the wrong-hook failures,
same-id platform reclamation, exact Job stop, cleanup, and one final mutation
gate are all assigned to coherent later steps. The roadmap validates against the
DES schema, owns each of the fifteen stakeholder scenarios exactly once, and
does not promote the dirty 02-03 RED event or any historical run to corrected
GREEN evidence.

It is not executable yet. The contract-shape map misclassifies S-GTI-03, the
resolver-failure acceptance criterion drops the required no-Running outcome,
four `test_file`/`scenario_name` pairs do not resolve to each other, and one
supporting contract has two owners. Global exclusion patterns can also forbid
necessary compiler/test-harness fallout. Finally, the roadmap contains both a
forbidden issue-less future-work pointer and step-by-step implementation
algorithms; three implementation-note blocks exceed the mandatory size limit.

## Review basis

- Approved DESIGN commit `85550e4a267cbd53ac266fa54f4d8cda164910af`
  and `design/review-q7-remediation.md` iteration 6, **APPROVED**.
- DISTILL commit `558589a7a3ee14ebea0b8cdb6496b65a5830f777`,
  including all fifteen stakeholder examples, twenty-nine supporting rows,
  immutable `NOT_EXECUTED` classification, the honest 14/15 C4a gap, and
  E07-E09 stubs.
- DISTILL iteration-4 reviews: product, architecture, and acceptance
  **APPROVED**; platform **CONDITIONALLY_APPROVED** on P7 landing before any
  runtime evidence.
- Current `execution-log.json` and the complete on-disk reviews for 01-01
  through 02-02. Dirty/uncommitted 02-03 work was inspected only to confirm its
  exclusion and was not used as implementation evidence.

## Mandatory roadmap checks

### 1. External validity — PASS

Steps 02-04 through 02-06 drive built `serve`, `deploy`, `workload describe`,
and `job stop` paths on qualified native metal. E07-E09 require command, state,
wire/kernel, and cleanup evidence plus independent review. No test-owned success
path substitutes for production invocation.

### 2. Acceptance-criterion implementation coupling — FAIL

The five acceptance criteria per fresh step stay within the thirty-word limit,
but the roadmap immediately follows them with internal type/field, parser,
ordering, syscall, buffer, and error-chain instructions. D6 records the required
rewrite.

### 3. Step decomposition ratio — PASS

There are four fresh implementation steps and sixteen unique production/tooling
files in their declared scopes, for a ratio of `4 / 16 = 0.25`. The steps have
distinct P7/Q7, D7/E07, failure/E08, and reclamation-stop/E09 outcomes; no
three-step substitution pattern exists.

### 4. Implementation code in roadmap — FAIL

The implementation notes prescribe algorithms and operation order rather than
linking to the already-approved DESIGN/DISTILL contracts. This is a blocking
roadmap-format defect; see D6.

### 5. Concision and precision — FAIL

The total string-word count is 2,112, below the 3,000-word ceiling for nine
steps. Every fresh description is at most 31 words, every fresh step has five
criteria, and the longest criterion is 29 words. The 02-03, 02-04, and 02-05
implementation notes are approximately 150, 124, and 109 words, exceeding the
100-word limit and carrying procedural detail. See D6.

### 6. Unit/acceptance boundary — FAIL

The planned metal scenarios use the correct CLI driving ports and the supporting
properties are placed at source/component boundaries. The executable locator
metadata is nevertheless inaccurate and incomplete, so a crafter cannot use it
as the exact RED activation map required by the roadmap contract. See D3.

## Blocking findings

### D5 — Issue-less future-work deferral for the C4a gap

**Severity:** BLOCKER

`known_delivery_gap.disposition` says that “a future roadmap may add” the
duplicate-create coverage (`roadmap.json:459-463`) without citing an approved
issue. `CLAUDE.md:385-400` requires user approval and a verified GitHub issue at
every deferral reference, or removal of the forward pointer. The approved
DISTILL result permits the gap to remain visibly uncovered at 14/15; it does not
permit an untracked promise of later work.

**Required remediation:** retain `AT_GAP_IN_DELIVERY_SCOPE`, `C4a = FAIL`, and
14/15, but state only that the obligation is outside this roadmap and is not
counted. Remove the future-roadmap promise unless the user approves a concrete
issue and that verified issue is cited.

### D6 — Procedural implementation algorithms are embedded in the roadmap

**Severity:** BLOCKER

The 02-04 notes prescribe expression normalization, parser framing, notification
join order, socket mode, buffer size, packet filtering, and arithmetic sequence
(`roadmap.json:331`). The 02-05 notes similarly prescribe the exact fixture
mutation/error chain and restoration procedure (`roadmap.json:389`), while
02-03 repeats internal field and variant sets (`roadmap.json:272`). This violates
the mandatory “no implementation code/algorithms in roadmaps” check. It also
pushes the three note blocks beyond 100 words.

**Required remediation:** reduce every implementation note to at most 100 words,
state only architectural constraints and observable prohibitions, and link the
approved DESIGN/DISTILL sections for the exact D7, Q7, and wrong-hook mechanics.
Keep load-bearing observable outcomes in the criteria; remove algorithms,
step-by-step sequencing, internal field inventories, and tutorials.

## High-severity findings

### D1 — S-GTI-03 has the wrong Contract Shape

**Severity:** HIGH

The roadmap puts S-GTI-03 under `bounded-change`
(`roadmap.json:65-75`). The authoritative DISTILL table and Gherkin both declare
S-GTI-03 `unbounded-preservation`
(`distill/test-scenarios.md:207-223,250-256`). This changes the required
complement: the peer-wire test must prove absence of either plaintext marker
across the unbounded observed wire universe, not only selected deltas.

**Required remediation:** move S-GTI-03 to `unbounded-preservation` and ensure
the per-scenario executable map and 02-04 verification require the exact
unbounded-preservation declaration and complement oracle.

### D2 — S-GTI-08a no longer forbids a transient Running observation

**Severity:** HIGH

The 02-05 criterion requires Failed, unchanged count/View, and no READY, guest
EXIT, EXEC, frames, or sibling change (`roadmap.json:345-350`), but omits the
authoritative outcome that the resolver-failure allocation never reports
Running (`distill/test-scenarios.md:300-311`). This distinction is load-bearing:
S-GTI-05 may transiently report Running before the fresh guard installer fails,
whereas the pre-READY resolver failure may not.

**Required remediation:** add “never reports Running” to S-GTI-08a without
weakening the exact exit-code, no-restart/View, no-guest-EXIT, no-EXEC/frame,
cleanup, or sibling-preservation obligations.

### D3 — Executable scenario locators are inaccurate and incomplete

**Severity:** HIGH

The `test_file`/`scenario_name` contract is a file/function locator for RED, but
four pairs do not resolve in the reviewed tree:

| Step | Declared file | Declared scenario | Actual location/status |
|---|---|---|---|
| 01-02 | `veth_provision_idempotent.rs` | `microvm_dials_a_mesh_peer_by_name_and_receives_the_reply` | Function exists in `guest_stack_mtls_egress.rs`, not the declared file |
| 01-03 | `overdrive-init/src/main.rs` | `the_operator_sees_the_microvm_workloads_own_mesh_address_not_its_transit_hop` | Function exists in `guest_stack_mtls_egress.rs`, not the declared file |
| 02-02 | `vm_driver_stop_totality.rs` | `the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes` | Function exists in `guest_stack_mtls_egress.rs`, not the declared file |
| 02-03 | `overdrive-init/src/main.rs` | `each_microvm_slot_owns_a_mesh_address_disjoint_from_its_transit_hop` | Function exists in `veth_provisioner.rs`, not the declared file |

The grouped fresh steps also give only one singular locator for three to five
stakeholder examples. `scenario_ownership` proves ID uniqueness, but it does not
tell the crafter which inherited body to activate or which new body to create
for every scenario.

**Required remediation:** add one exact mapping per S-GTI id with owner step,
Contract Shape, test file, and executable function identity. Make every existing
pair resolve in the immutable tree; mark genuinely new executable identities as
new rather than pointing to an unrelated body. Remove misleading executable
locators from historical-only steps or replace them with locators that match
their historical objectives.

### D4 — Global exclusions turn guidance into a restrictive allowlist

**Severity:** HIGH

`implementation_scope.excluded_patterns` excludes every `**/bin/**` path and the
entire `crates/overdrive-testing/**` tree (`roadmap.json:485-512`). Repository
DELIVER rules explicitly make file lists guidance and permit tightly bounded
production, test, harness, configuration, and compiler-required fallout. These
exclusions can prohibit the correct tooling-binary location or a required shared
test harness update, and the roadmap gives no compiler-fallout escape.

**Required remediation:** remove the restrictive source/test exclusions (keeping
generated `target` and scratch paths as hygiene if desired), state that scope
lists are guidance, and explicitly permit tightly related compiler, manifest,
configuration, harness, and test fallout required by the acceptance criteria.
Continue to forbid unrelated behavior and unsanctioned public/API expansion.

## Medium-severity findings

### D7 — `C-GTI-FINALIZE-TWICE` has two effective owners

**Severity:** MEDIUM

Step 02-03 explicitly owns `C-GTI-FINALIZE-TWICE`
(`roadmap.json:240-254`). Step 02-06 again requires repeated finalization not to
duplicate work and lists finalization among its focused tests
(`roadmap.json:403-408,448-453`). This defeats the requested one-owner map and
makes later DES/review evidence ambiguous.

**Required remediation:** keep `C-GTI-FINALIZE-TWICE` owned by exactly one step
(02-03 is the natural Q7 owner). If 02-06 changes the same production area, call
the earlier property a regression gate, not a second DELIVER obligation, and do
not claim its RED/GREEN ownership again.

## Correctness checks that passed

- DES schema validation: `VALID: 2 phases, 9 steps`.
- Dependency chain: all references resolve and fresh execution is strictly
  `02-03 -> 02-04 -> 02-05 -> 02-06`.
- P7 ordering: universal Run/Sync/supported-bootstrap lease and native
  non-virtualized preflight land in 02-03 before E07-E09.
- Stakeholder ownership: all fifteen S-GTI ids occur exactly once across fresh
  steps; the failure is shape/locator fidelity, not duplicate ID ownership.
- Historical evidence: 01-01 through 02-02 each reference an existing full
  Markdown review ending in APPROVED and an ancestry-valid commit span. Their
  criteria explicitly refuse corrected Q7/D7 credit.
- Q7/D7/lifecycle content: exact `Option<i32>` preservation, no restart/count/
  View/public-shape expansion, post-READY exit 78, strict D7 counter/program/
  dump/generation/notification/capture equality, real wrong-hook fresh/restart
  failures, same-id platform reclamation, exact Job stop, cleanup, and sibling
  preservation are all present, subject to D1-D3.
- Mutation discipline: every step forbids mutation testing and the final gate
  contains exactly one qualified native-metal whole-workspace run after all
  fresh reviews and E07-E09 approvals.
- DES honesty: the current dirty 02-03 RED event is explicitly non-reusable; no
  fresh GREEN, runtime evidence, or 15/15 claim is made.
- Diff hygiene: `git diff --check b4b94caa^ b4b94caa` passes; the target commit
  changes only `roadmap.json`. Dirty source/config/test work was excluded from
  correctness evidence and preserved.

## Defect counts

| Severity | Count |
|---|---:|
| BLOCKER | 2 |
| CRITICAL | 0 |
| HIGH | 4 |
| MEDIUM | 1 |
| LOW | 0 |

## Remediation disposition

Return D1-D7 to the original roadmap author. Do not execute 02-03 and do not
approve or commit validation metadata. After remediation, a fresh isolated
roadmap reviewer must re-run the full effective review; there is no iteration
cap.

## Final verdict

**NEEDS_REVISION**

`validation.status` correctly remains `pending`. No roadmap metadata was edited
and no commit was created by this review.

## Iteration 2 — 2026-08-29

- **Review ID:** `roadmap_rev_20260829_023722_iteration_2`
- **Reviewer:** `nw-solution-architect-reviewer` (same isolated reviewer as iteration 1)
- **Reviewed commit:** `19500f405c109c378aa08f16909d273212d9b2be`
- **Parent:** `b4b94caa448be276200af34daf2d9a66b4e3aaa9`
- **Verdict:** **APPROVED**

### Executive summary

Iteration 2 resolves D1-D7 without introducing a blocker, critical, high, or
medium regression. S-GTI-03 is unbounded-preservation with a complete peer-wire
complement; S-GTI-08a explicitly forbids both READY and Running; all fifteen
stakeholder scenarios and all fifty-five atomic expansions of the twenty-nine
supporting rows have one exact owner, file, executable identity, and Contract
Shape. Existing/scaffold transition locators resolve in the reviewed commit.

The global exclusions are gone and scope is explicitly advisory with bounded
compiler, manifest, configuration, tooling, harness, and test fallout allowed.
The former C4a gap is now an executable 02-05 obligation, so the roadmap keeps
the immutable baseline honest at 14/15 while making 15/15 only a planned result
after the new test passes. Implementation notes are outcome-focused and within
the word limit. Finalization replay is owned only by 02-06. P7 still lands in
02-03 before E07-E09, and mutation testing remains one final wave gate.

### Iteration-1 finding disposition

| Finding | Disposition | Iteration-2 evidence |
|---|---|---|
| D1 — S-GTI-03 shape | **RESOLVED** | The canonical shape map and executable mapping both use `unbounded-preservation`; 02-04 verifies the complete lossless peer-wire universe. |
| D2 — resolver failure allowed Running | **RESOLVED** | The 02-05 criterion and S-GTI-08a outcome oracle both forbid READY and Running while retaining exact exit/no-restart/View and forbidden-effect checks. |
| D3 — inaccurate/incomplete locators | **RESOLVED** | Historical executable fields were removed. A single nonduplicated map now covers 15 stakeholder scenarios plus 55 atomic supporting obligations; every inherited/scaffold locator resolves in commit `19500f40`. |
| D4 — restrictive exclusions | **RESOLVED** | `excluded_patterns` is removed. `scope_policy` makes lists advisory and permits documented, tightly related compiler, manifest, configuration, tooling, harness, and test fallout. |
| D5 — issue-less C4a deferral | **RESOLVED** | No future-work pointer remains. `C4a-ATTEMPT-RESOURCE-DUPLICATE-CREATE` is owned by 02-05 with exact rejection, no-replacement/cross-ownership, and no-leak outcomes before E08 approval. |
| D6 — algorithms/oversized notes | **RESOLVED** | Notes are 64, 50, 69, and 59 words, link the approved DESIGN/DISTILL contracts, and retain constraints without parser, syscall, mutation, or fixture algorithms. |
| D7 — duplicate finalization owner | **RESOLVED** | `C-GTI-FINALIZE-TWICE` appears once in the executable map and is owned only by 02-06; 02-03 owns the distinct first-finalization/reconcile contract. |

### Mandatory roadmap checks

| Check | Result | Evidence |
|---|---|---|
| External validity | **PASS** | Built `serve`, `deploy`, `workload describe`, and `job stop` journeys close through E07-E09 on qualified native metal. |
| AC implementation coupling | **PASS** | Criteria retain only approved load-bearing Q7/D7/kernel and lifecycle constraints; private decomposition, method signatures, and implementation algorithms are absent. |
| Step decomposition | **PASS** | Four fresh implementation steps span sixteen unique production/tooling files: `4 / 16 = 0.25`; each step owns a distinct P7/Q7, D7/E07, failure/E08, or reclamation-stop/E09 outcome. |
| Implementation code | **PASS** | No code block, pseudocode, loop, conditional, or method implementation appears in the roadmap. Exact protocol/kernel terms are approved architectural constraints. |
| Concision/precision | **PASS** | Total string-word count is 2,294 of 3,000. Fresh descriptions are at most 31 words; every step has five criteria; the longest criterion is 29 words; every note is below 100 words. |
| Test boundary | **PASS** | Stakeholder and EDD cases use CLI/script driving ports; pure and component contracts use the exact DISTILL source/component seams and declarations. |

### Mapping, budget, and sequencing audit

- Stakeholder mapping count: **15**, each id exactly once. Owner groups match
  `scenario_ownership`, and every mapped shape matches
  `contract_shape_mapping` and DISTILL.
- Supporting mapping count: **55 atomic identities**, the exact expansion of
  the 29 immutable supporting rows, including the 25 stable guest-init cases,
  two metal-qualification obligations, two multi-actor cases, and three EDD
  expectations. No id or file/function pair is duplicated.
- Per-step mapped identities are 39, 14, 9, and 8. At one test per distinct
  contract these remain below the respective conservative `2 × behavior`
  ceilings of 78, 28, 18, and 16. Crafters still re-derive and record the exact
  unit-only budget during RED/COMMIT; EDD and metal journeys are not mislabeled
  as unit tests.
- DES validation returns `VALID: 2 phases, 9 steps`; all dependency references
  resolve, and fresh order is strictly `02-03 -> 02-04 -> 02-05 -> 02-06`.
- P7's Run/Sync/supported-bootstrap pre-mutation lease, retained Run descriptor,
  and native non-virtualized preflight are committed and reviewed before any
  E07-E09 runtime evidence can count.
- C4a remains `FAIL`/14-of-15 at the immutable baseline and becomes planned
  15-of-15 only after the mapped 02-05 duplicate-create contract passes.
- `C-GTI-FINALIZE-TWICE` has one owner. Same-id reclamation, failed reinstall,
  exact Job-stop deletion/idempotency, failure cleanup, and sibling preservation
  retain their separate exact owners and evidence journeys.

### Regression and integrity checks

- Exact `Option<i32>` VMM exit preservation, no restart/count/View change,
  no public-shape expansion, and ordinary post-READY status 78 remain explicit.
- D7 retains the anonymous counter, full normalized program identity, strict
  complete `GETRULE`/`GETGEN`, generation/notification guard, lossless all-frame
  capture, and exact checked packet/byte equality.
- Fresh and same-id reinstall failures still use the production-named INPUT-hook
  base-chain route and the real typed `-EOPNOTSUPP` production error path.
- Same-id evidence still requires unclean serve restart with standing durable
  intent; natural Job exit and fresh-id workload restart remain invalid.
- No stale nested-metal, output-chain, invalid-fixture, rule-count-only,
  historical-GREEN, false current 15/15, or public Beacon/describe expansion
  claim was found.
- Mutation testing is forbidden in all four steps. Exactly one qualified
  native-metal whole-workspace mutation command remains after all fresh reviews
  and E07-E09 approvals.
- The roadmap-only remediation commit passes `git diff --check`. Dirty source,
  test, configuration, and execution-log work remains excluded and preserved.

### Iteration-2 defect counts

| Severity | Count |
|---|---:|
| BLOCKER | 0 |
| CRITICAL | 0 |
| HIGH | 0 |
| MEDIUM | 0 |
| LOW | 0 |

### Iteration-2 final verdict

**APPROVED**

The roadmap is executable. Validation metadata may be marked approved and the
fresh 02-03 crafter may start only after the focused approval commit lands.
