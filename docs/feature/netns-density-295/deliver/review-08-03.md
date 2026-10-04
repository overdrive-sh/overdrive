# DELIVER Review — Step 08-03

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `08-03` — Member, policy-route, and guard audit and repair |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning (explicit user override) |
| Iteration | 1 |
| Reviewed range | `073bf990a101b9a23b1ee139ac719aae570b9336..5403a19e715a0cc3d20d938869e73f0ad40afd16` |
| Implementation commit / tree | `5403a19e715a0cc3d20d938869e73f0ad40afd16` / `612d5494635627a88dde0b40025b3458b3628de3` |
| Review date | 2026-10-04 |
| Iteration 1 verdict | **NEEDS_REVISION** (superseded by the iteration 2 verdict below) |

Applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

The selected step is `roadmap.json`'s 08-03 entry (criteria and verification at lines 1335–1371). Its dependency 08-02 is approved in `review-08-02.md`. I reviewed the R15 runtime audit and repair contract, member-tolerant observation, guard handover, exact error mapping, and typed observation causes in `feature-delta.md` lines 3781–3961; S-ND295-61's complete test register and oracle in `distill/test-scenarios.md` lines 1231–1253; and S-ND295-68's contrast disposition in lines 1631–1641.

The production owner path is the shared-network supervisor's `run_mtls_owner`: it calls `audit_shared_owner`, classifies the returned error through `component()`, and on a recoverable finding calls `converge_shared_owner` followed by another audit (`crates/overdrive-control-plane/src/lib.rs:1515-1620`). The worker audit takes `element_effects`, observes the program, policy route, guard, then compares observed members to the capability registry (`crates/overdrive-worker/src/mtls_intercept_worker.rs:2178-2188, 2330-2404`). Repair observes the current state, refuses a present wrong-target identity, converges constant state, relinquishes the prior guard, converges registry-expected members, and audits again (`mtls_intercept_worker.rs:2407-2467`).

## Contract Shape Compliance

The production interface matches the accepted design. `MtlsSharedOwnerError::component()` has the already-pinned public signature and is now the sole worker-error component mapping. The control-plane's duplicate `component_for` closure is removed and all corresponding supervisor decisions call `source.component()` (`crates/overdrive-worker/src/mtls_intercept_worker.rs:254-292`; `crates/overdrive-control-plane/src/lib.rs:1515-1620`). The new `has_acquired_elements` registry flag and expected-member projection are private implementation details for the exact Pending-with-acquired-effects, Active, and Retiring registry complement required by R15 (`mtls_intercept_worker.rs:862-870, 1016-1032, 1053-1070`). No unapproved public method, type, variant, parameter, persisted value, wire shape, or ownership boundary was added.

Repair follows the approved sequence. `audit_shared_owner_snapshot` checks the recorded program identity before `policy_route`, `intercept_mark_guard`, and members. `converge_shared_owner_inner` selects `None` only for an absent table, uses the recorded identity for an equal program, and returns `PostconditionMismatch` before mutation for a different identity. It then calls the approved `converge_shared`, forgets the prior guard so its conditional delete cannot remove the successor, converges allocation elements to the registry set, and runs the full audit. This implements the accepted R15 path without a new port method or a guard-disarm API.

The designed semantic error set and interface representations are pinned, and the implementation matches them:

| Designed failure | Approved representation and component | Implementation |
|---|---|---|
| Listener bind/local-address/postcondition or listener task failure | `MtlsSharedOwnerError` leg variant; `LegF` or `LegC` from `leg` | Exhaustive `component()` match maps each leg-bearing variant by leg (`mtls_intercept_worker.rs:268-281`). |
| Runtime observation error, absent program, or present program with a different identity | `Intercept { source }`, with exact `InterceptError` source; `IpRules` | The audit wraps observation errors and constructs `PostconditionMismatch` with `observed: None` or the observed identity (`mtls_intercept_worker.rs:2368-2386`). |
| Missing fwmark rule/local route or missing exact R18 guard | `Intercept { source: PolicyRouteAbsent }` or `Intercept { source: InterceptMarkGuardAbsent }`; `IpRules` | The ordered route and guard checks construct those exact variants (`mtls_intercept_worker.rs:2388-2397`). |
| Runtime members differ from the registry-derived set | `MemberMismatch { expected, observed }`; `IpSets` | Exact expected and observed member sets are preserved (`mtls_intercept_worker.rs:2398-2402`). |
| Member convergence operation fails | `MemberRepair { source: InterceptError }`; `IpSets` | The original source is retained by the `MemberRepair` wrapper (`mtls_intercept_worker.rs:2462-2465`). |
| Boot clear fails or returns members after convergence | `BootMemberClear` with the original error or `MembersRemain { observed }`; `IpSets` | The existing boot path preserves both forms (`mtls_intercept_worker.rs:2274-2287`). |
| Owner lifecycle or task-observer state is unavailable | Existing supervisor variants; `Supervisor` | Exhaustive mapping is present in `component()` (`mtls_intercept_worker.rs:282-286`). |

The mapping agrees with the design's table and check order (`feature-delta.md:3902-3957`). The implementation introduces no generic-string substitute and no new semantic error variant. The specific acceptance body intended to lock this full mapping is, however, still ignored; see D1.

## Test integrity and coverage

The committed test diff removes only pending ignore markers from the two Lima S-ND295-61 bodies in `shared_intercept_members.rs` and the S-ND295-68 contrast body in `serve_lifetime_fail_stop.rs`. Their assertions and fault body are otherwise unchanged. S-ND295-68 now uses the designed owned rule rewritten to a wrong listener target for fail-stop; its separate contrast deletes the whole table and asserts the serve lifetime remains pending through repair before a later interrupt stops it with status 0 (`serve_lifetime_fail_stop.rs:25-51, 454-520, 798-840`). The two activated Lima bodies cover the member/program/route loss rows and the R18 guard row with exact-state and complement checks (`shared_intercept_members.rs:950-1010`); none was weakened or re-authored.

**D1 — Blocker: four authored S-ND295-61 bodies remain ignored.** DISTILL's S-ND295-61 row names six required bodies across `netns_density_shared_owner.rs`, the worker's `every_shared_owner_error_reports_its_one_component` test, and `shared_intercept_members.rs` (`test-scenarios.md:1243-1253`). The implementation commit activates only the latter two Lima bodies. These four bodies still carry `#[ignore = "pending DELIVER step 08-03 (S-ND295-61)"]`:

| Required body | Current marker |
|---|---|
| `member_loss_is_an_ipsets_failure_and_repair_restores_exactly_the_member` | `crates/overdrive-worker/tests/integration/netns_density_shared_owner.rs:1893` |
| `policy_route_loss_is_repaired_with_live_members_and_the_prior_guard_is_relinquished` | `netns_density_shared_owner.rs:1960` |
| `a_differently_targeted_program_is_never_rewritten` | `netns_density_shared_owner.rs:2082` |
| `every_shared_owner_error_reports_its_one_component` | `crates/overdrive-worker/src/mtls_intercept_worker.rs:5147` |

The focused Lima reproduction was:

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(member_loss_is_an_ipsets_failure_and_repair_restores_exactly_the_member)' --no-fail-fast
```

It returned `no tests to run`: 0 tests ran and all 83 tests in the binary were skipped (Nextest run `f5ea9164-2ac8-4b50-b858-e8facbb321df`). This reproduces the acceptance-coverage omission; it is not a claim of a production control-plane failure. Activate the four named S-ND295-61 bodies without changing their assertions, then rerun the roadmap selectors. The finding is bounded to the explicit “S-ND295-61 ... activated” criterion and the authored scenario register.

All three bodies changed from ignored to active in this commit retain their `Outcome anchor` and required `CONTRACT_SHAPE: bounded-change.` declaration. The still-ignored component mapping test has the exact source-local pure-function declaration `/// CONTRACT_SHAPE: pure-function.`. The reviewer-skill declaration checker path is absent in this checkout; declarations were checked directly. The sole source-local test for the `component()` pure function is currently inactive, which is part of D1. The new/transitioned bodies were not weakened: the source diff changes only the three `#[ignore]` lines described above. No test spawns the built Overdrive binary or crosses the expectation boundary.

The authored unit-test budget is not exceeded: the step's unit-level component mapping proof is one pure-function test against the source-local `component()` interface; the remaining scenario bodies are worker integration or native integration tests. Since that one component test remains ignored, its required behavior is not currently covered by an active body.

## Verification and DES

The final-source results reported by the crafter were:

| Evidence | Result |
|---|---|
| Lima worker integration selector for `shared_intercept_members` and `netns_density_shared_owner` | 16/16 passed (the default run does not activate D1's ignored bodies) |
| Lima worker library selector for `mtls_intercept_worker` | 36/36 passed (the default run does not activate the ignored component mapping body) |
| Full native S-ND295-68 selector on metal | 4/4 passed, 171 skipped, 29.123 s; Nextest run `a356840c-58e3-40ef-b39f-aa4a43dca6f4` |
| Workspace `cargo check --workspace --all-targets --features integration-tests` | Passed in Lima |
| Workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | Passed in Lima |
| Formatting and `git diff --check` | Passed |
| Focused ignored-body reproduction | Failed as expected with `0 tests run`, `83 skipped`, and `no tests to run` |

The DES log records 08-03 RED `FAIL` at `2026-10-04T00:54:30Z`, GREEN `PASS` at `01:16:36Z`, and COMMIT `PASS` at `01:18:10Z`. The full verification gates and DES phase order do not replace the missing acceptance bodies.

## Review dimensions and remediation disposition

The R15 production implementation follows the selected interface and ownership contract. The audit/repair path is serialized under the accepted `element_effects` mutex, preserves the exact expected member set, retains typed sources, and hands over rather than drops the node guard. `component()` is exhaustive and its consumer-side duplicate mapping is gone. No proven production correctness failure, error-contract ambiguity, security-invariant weakening, API divergence, test assertion weakening, fixture theater, or binary/expectation-boundary violation was found in the reviewed diff. No ordering/concurrency hypothesis was promoted; no seeded Sim finding or production remedy is proposed.

No remediation has been applied in this review. The step remains **NEEDS_REVISION** until D1's four existing test bodies are activated and the bounded integration selectors demonstrate they execute and pass. Review iteration 2 must record the result and any new findings in this same artifact.

## Iteration 1 verdict

**NEEDS_REVISION.** Production R15 audit/repair and typed component mapping match the accepted design. Step 08-03 is not complete because four S-ND295-61 acceptance bodies specified by DISTILL remain ignored, including the body that exercises the component mapping table. The focused test selector confirms that the member-loss acceptance body is not run.

## Iteration 2 — remediation review

### Metadata

| Field | Value |
|---|---|
| Iteration | 2 |
| Remediation commit / tree | `c9f48cc956b05757f7dd9dc6e5fbc1b3d08c132c` / `a4b771a991da45ed91aaf5a261e7cdd2a157b79b` |
| Remediation author and trailers | Marcus Schack Abildskov remains author; exactly `Co-Authored-By: Codex <codex@openai.com>`; `Step-Id: 08-03` |
| Final verdict | **APPROVED** |

### D1 remediation disposition and evidence

D1 is **CLOSED**. The remediation commit removes exactly the four pending `#[ignore]` markers identified in iteration 1. The source/test diff changes no assertion, stimulus, fixture, outcome anchor, or `CONTRACT_SHAPE` declaration; the only non-test change is the crafter's DES event log. The three worker integration bodies and the source-local component mapping body are now active. The pure-function test retains the exact `/// CONTRACT_SHAPE: pure-function.` declaration, and each integration body retains its `Outcome anchor` and `CONTRACT_SHAPE: bounded-change.` declaration.

The body-level review confirms the activated tests exercise the required distinctions and outcomes:

- The member-loss body audits the exact registry and observed member sets, expects `MemberMismatch`/`IpSets`, restores the deleted member, proves the intact program was not rewritten, and verifies node-guard handover.
- The policy-route/guard body exercises both owned-object losses with live members, checks each exact typed cause and `IpRules` component, repairs without rewriting the equal-identity program, verifies guard handover, and installs a later allocation.
- The wrong-target body checks the audit's `PostconditionMismatch` names both identities, repair refuses without a program write, and the absent-program control repairs as designed.
- The component mapping body constructs and checks all 14 `MtlsSharedOwnerError` variants against the accepted component table, including both boot-clear sources and the typed runtime observation causes.

The crafter's explicit selector receipts are:

| Evidence | Result |
|---|---|
| Lima worker integration selector for `shared_intercept_members` and `netns_density_shared_owner` | 19/19 passed, 1.511 s; Nextest run `484b7102-5a30-43e3-90a2-0ca9a580495d`. This run explicitly executed all three remediated integration bodies. |
| Lima worker library selector for `mtls_intercept_worker` | 37/37 passed, 1.032 s; Nextest run `99528a25-0bdd-4d93-b5f4-709d6d9a7525`. This run explicitly executed `every_shared_owner_error_reports_its_one_component`. |
| Workspace `cargo check --workspace --all-targets --features integration-tests` | Passed in Lima, 13.42 s. |
| Workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | Passed in Lima, 18.70 s. |
| Formatting | Passed. |

No native rerun was needed for this marker-only remediation: neither the native test source nor production code changed. The full S-ND295-68 metal result from iteration 1 remains applicable: 4/4 passed, 171 skipped, Nextest run `a356840c-58e3-40ef-b39f-aa4a43dca6f4`.

DES preserves the original 08-03 RED `FAIL`, GREEN `PASS`, and COMMIT `PASS` history. The marker-only remediation records RED `SKIPPED` with `APPROVED_SKIP` at `2026-10-04T01:35:18Z`, followed by GREEN `PASS` at `01:37:17Z` and COMMIT `PASS` at `01:37:43Z`. The remediation's approved skip is consistent with rerunning already-committed GREEN behavior after changing only test activation markers.

### Iteration 2 verdict

**APPROVED.** All six S-ND295-61 bodies named by DISTILL are active, including the four remediation targets. The focused integration and worker-library selectors execute the previously omitted bodies and pass. The implementation, exact API/error contract, and acceptance assertions remain unchanged from iteration 1. S-ND295-68's repaired-loss contrast remains active and green, and the required Lima check and clippy gates pass. No further remediation is required for step 08-03.
