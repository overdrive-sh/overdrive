# DELIVER Review — Step 01-01

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `01-01` — Exit observer fires `on_alloc_terminal` for every exit |
| Review iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Reviewed implementation commits | `3c994aa9c584aecea1a2a33d2287f9d351af3fbf`; `c51ac332216a164d6aa19955f97c02a9a4cbadcf` |
| Comparison base | `cb8afedb016ef8636cc2fde910467be734db16b2` |
| Authoritative contract | `feature-delta.md` D1/D2 and ADR-0255; approved roadmap step in `deliver/roadmap.json` |
| Review verdict | **APPROVED** |

This review is bounded to D1 and D2. D3 registration ordering remains step 01-02. No review finding or implementation change is proposed.

## Executive summary

The production exit observer now calls `Driver::on_alloc_terminal(&event.alloc)` immediately after consuming an `ExitEvent`, before entering the retry-bounded row-write path. The transient `Weak<dyn Driver>` upgrade is released before the first await, and an unavailable driver is the no-op explicitly allowed by D1. The call does not depend on whether the observer later writes a row, has no prior row, or exhausts retries.

The replacement `Driver::on_alloc_terminal` documentation matches the approved D2 contract: multiple ending authors, row independence, synchronous/infallible/non-blocking behavior, idempotence, unknown-id no-op, and preservation of other allocations. The existing `VmDriver` override delegates to `ProbeRunner::stop_alloc`; that operation removes and cooperatively cancels the per-allocation supervisor and is a no-op when it is absent.

The only acceptance-body changes activate the seven pre-authored step obligations by removing their pending `#[ignore]` attributes. No assertions, fixtures, test names, or behavior were weakened or replaced. The focused step suite passed all 12 selected tests. I found no reachable production defect, contract divergence, test-integrity violation, or typed-error ambiguity in this step.

## Contract and implementation review

### D1 — exit observer ownership and call order

The accepted contract says one observer calls the owning driver once for each consumed event, before the awaited row write, regardless of `Wrote`, `NoWrite`, or `Failed`; the weak-reference upgrade may fail as a no-op (`feature-delta.md:158-172`). The implementation follows that order at `crates/overdrive-control-plane/src/worker/exit_observer.rs:205-222`: receive the event, transiently upgrade the weak driver, call the hook, drop the strong reference, then await `run_with_retry`.

The observer does not retain a strong driver reference across row-write retries. Its existing shutdown cancellation remains biased ahead of `rx.recv()` (`exit_observer.rs:206-212`), while production `ServerHandle::shutdown` drains convergence and the server before cancelling and joining observer tasks (`crates/overdrive-control-plane/src/lib.rs:3022-3094`). This preserves the existing owner and shutdown boundaries; D1 adds no detached task, runtime lookup, or new lifecycle path.

The outcome cases are covered by production-observer acceptance scenarios: successful row handling (`terminal_hook_ownership.rs:398-438`), exhausted write retries (`:500-544`), and no prior row / `NoWrite` (`:546-572`). The seeded crash scenario in `crash_ending_authority.rs:108-143` confirms that a partial startup-probe streak does not grow after the crash. Restart and sibling scenarios verify the hook affects the ended allocation only (`terminal_hook_ownership.rs:443-498, 779-824`).

### D2 — hook contract and implementation

The trait method signature is unchanged. Its replacement documentation at `crates/overdrive-core/src/traits/driver.rs:760-817` matches the approved interface text in `feature-delta.md:1112-1160`: it covers plural authors, row-independent meaning, idempotence, unknown and already-stopped IDs, no-probe-runner default behavior, synchronous infallibility, non-blocking execution, and unchanged rows and unrelated allocations.

The existing production override at `crates/overdrive-worker/src/vm_driver.rs:2063-2069` delegates to `ProbeRunner::stop_alloc`. `crates/overdrive-worker/src/probe_runner/mod.rs:374-385` removes the supervisor and signals cooperative cancellation; removal of an unknown or already-stopped allocation changes nothing. The direct D2 acceptance scenario checks unknown IDs and repeated calls against the full observed snapshot (`terminal_hook_ownership.rs:704-741`), while the double-author scenario checks the observer call followed by a genuine terminal finalize (`:743-775`).

### Exact API and typed-error contract

No method, type, variant, parameter, wire field, persisted format, ownership boundary, or lifecycle meaning was added or changed. The public `Driver::on_alloc_terminal` signature remains as designed; only its docstring was replaced. The hook is infallible and returns no error, so this step introduces no typed-error set, payload, or mapping to review. The existing observation-store retry/error behavior is unchanged.

## Test integrity and coverage

### Acceptance activation and test modifications

Roadmap step 01-01 names six D1 tests plus `crash_with_a_partial_streak_is_not_probed_after_the_crash` (`deliver/roadmap.json:14-32`). The diff removes only six D1 pending markers in `terminal_hook_ownership.rs` and the D1 pending marker in `crash_ending_authority.rs`. Test bodies and assertions are unchanged. The two D3 acceptance tests remain ignored for step 01-02.

The composed fixture drives the production exit observer and production action shim with a real `ProbeRunner`, using `Sim*` adapters at driven ports. `HookedDriver` is a test implementation at the `Driver` port: it controls when the test exit event becomes available and records the hook call, while the production observer authors the row and the production runner owns the supervisor (`terminal_hook_ownership.rs:1-31, 138-230`). This is not mock-dominated verification; assertions cover supervisor count, role liveness, row state, lifecycle events, sibling state, and recorded hook calls.

### Test budget

The step covers six distinct behavior groups: per-event hook ownership; outcome independence across write/no-write/failure; restart and sibling isolation; stopping post-crash probe progress; hook idempotence and unknown-ID behavior; and preservation of operator-stop/finalize/default-driver paths. The step has 12 tests for a budget of 12 (`2 × 6`). The 12 focused tests all passed.

### Contract Shape compliance

| Check | Result | Evidence |
|---|---|---|
| Required declaration on each live or transitioned test | PASS | All 12 live focused tests carry `CONTRACT_SHAPE: bounded-change`; the seven activated obligations retained their declarations. The source-local partial-streak test also declares `bounded-change` (`crash_ending_authority.rs:108-113`). |
| Semantic match and bounded-change complement | PASS | The tests snapshot the port-exposed observable universe and compare it with the declared allocation-local delta; sibling rows, roles, backend rows, and unaffected supervision are asserted unchanged. |
| Driving-port and production-boundary checks | PASS | The observer and shim are production code; the fixture's driver double sits at the driven port. Tests do not construct the expected terminal row or supervisor as test-owned SUT state. |
| Banned test-name pattern | PASS | No activated Rust test name matches the banned `test_…` naming pattern. |
| DISCUSS outcome anchor | Not applicable | The feature delta states that no DISCUSS wave exists (`feature-delta.md:42-45`); the accepted D1/S-1 through S-8 obligations provide traceability. |
| Unbounded-preservation snapshot rule | Not applicable | These Rust scenarios assert bounded state changes, not an unbounded filesystem/configuration preservation contract. |

## TDD and quality gates

The execution log preserves the historical Nov 5 RED/GREEN events and records the isolated Nov 9 rerun as RED/FAIL, GREEN/PASS, COMMIT/PASS in order (`deliver/execution-log.json`). The current run therefore has the required RED → GREEN → COMMIT sequence. The log stores phase outcomes rather than raw test output; the crafter's phase report records seven semantic RED failures followed by GREEN success.

| Gate | Result | Evidence |
|---|---|---|
| G1 — step acceptance activation | PASS | The seven 01-01 obligations are active; D3 tests remain pending for 01-02. Existing controls remain active. |
| G2 — valid RED | PASS | Current DES log records `RED / EXECUTED / FAIL`; crafter report classifies the seven failures as semantic. |
| G3 — assertion failure | PASS | Crafter's RED report identifies assertion failures at the required D1 outcomes; the execution log itself does not retain raw failure text. |
| G4 — no domain mocking | PASS | The test double implements the `Driver` port; production observer, action shim, and `ProbeRunner` remain under test. |
| G5 — domain language | PASS | Test names and assertions describe crashed allocations, supervision, row writes, restarts, and sibling effects. |
| G6 — step tests green | PASS with broader-suite limit | Independent focused run: 12 passed, 0 failed. The broader acceptance run is recorded below. |
| G7 — passing before commit | PASS | Current DES COMMIT event is PASS; independent focused verification also passed after the implementation commits. |
| G8 — test budget | PASS | 12 tests for 6 behavior groups; budget 12. |
| G9 — no weakened or deleted tests | PASS | Only pending ignore markers were removed from the named pre-authored tests. |

No zero-assertion, tautological, mock-dominated, always-green, circular-oracle, or misleading-name test was found. No new production behavior lacks an acceptance obligation. RPP L1-L2 scan found no actionable smell in the bounded diff.

## Verification evidence and limits

### Independent reviewer verification

The focused step command ran on the canonical Metal route:

```text
cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(a_crashed_allocation_stops_probing_as_soon_as_the_observer_consumes_the_exit) | test(a_restart_leaves_only_the_successor_supervised_without_a_second_hook_call) | test(a_crash_ends_probe_supervision_even_when_every_row_write_attempt_fails) | test(a_crash_with_no_prior_row_still_ends_probe_supervision) | test(a_crash_then_a_finalize_fire_the_hook_twice_with_one_effect) | test(a_crash_ends_only_the_crashed_allocations_supervision) | test(crash_with_a_partial_streak_is_not_probed_after_the_crash) | test(an_operator_stop_ends_probe_supervision_exactly_once) | test(finalizing_a_live_allocation_ends_probe_supervision_exactly_once) | test(the_terminal_hook_is_idempotent_and_a_no_op_for_unknown_ids) | test(a_crash_leaves_the_siblings_supervision_and_row_untouched) | test(a_driver_with_no_probe_runner_takes_the_default_path)'
```

**Result:** 12 tests passed, 0 failed; 453 tests were filtered out. `git diff --check cb8afedb016ef8636cc2fde910467be734db16b2..HEAD` passed. The crafter separately reports changed-file rustfmt and Metal Clippy for all targets in `overdrive-core` and `overdrive-control-plane`, with `integration-tests` and warnings denied, passing; those commands were not independently rerun in this review.

### Broader acceptance-run limit

The crafter reports a broader control-plane acceptance result of 413 passed, 3 failed, and 49 skipped. The three `job_stop_*` failures report `GuestNetworkBoot(FdBudget { required: 376832, available: 65536 })`; the same resource-budget error was observed when run serially. These tests and their resource-budget path are outside the changed files and D1/D2 call path. The current error is consistent with the Metal process descriptor ceiling, but this review did not establish the same tests' result at the comparison base. I therefore record this as an unresolved environment/validation limit, not as a proven pre-existing failure and not as a reproduced step regression. The focused 01-01 tests pass.

Mutation testing was not run; the approved roadmap reserves it for the single final DELIVER-wave gate.

## Commit and scope review

The implementation commit changes the exit observer, the existing trait documentation, the two pre-authored acceptance files, and the DES log. It does not change `VmDriver`, `SimDriver`, any public signature, test harness boundary, or unrelated production path. The follow-up commit records the COMMIT phase. Both commits carry exactly `Co-Authored-By: Codex <codex@openai.com>`; the implementation commit also carries `Step-Id: 01-01` and the feature task ID. No Claude Code, Claude, or Anthropic attribution is present.

The pre-existing dirty `AGENTS.md` remains outside both commits. No remediation was needed or performed.

## Findings and remediation disposition

No findings. No remediation is required. No design gap or `SPECIFICATION_AMBIGUITY` is present in the D1/D2 interface or error contract.

## Final verdict

**APPROVED** — Step 01-01 implements the accepted D1/D2 behavior with the exact existing public API shape, preserves all approved outcomes, activates only its pre-authored scenarios, and passes the focused 12-test verification. The three broader acceptance failures remain recorded as an unproven environment/validation limit; their result at the comparison base was not established. Step 01-02 may proceed after the orchestrator completes its mechanical checks and confirms this on-disk review artifact.
