# Adversarial review — step 03-01

- **Feature:** `netns-density-295`
- **Step:** `03-01` — Core EXEC-gate acceptance: BootClosed/recovery/fail-stop model and placement cap
- **Reviewer:** isolated DELIVER `nw-software-crafter-reviewer`
- **Review ID:** `code_rev_20260922_161500_iteration_1`
- **Iteration:** 1
- **Reviewed cumulative range:** `526e961842d7..b4400801d1f53ff11fe31b3f7b17bfc95f2e4b37`
- **Implementation commits:** `a6c2b0ac6dbd22c5b8e3e0851f38af4a656acd6`, `b4400801d1f53ff11fe31b3f7b17bfc95f2e4b37`
- **Final verdict:** **CHANGES_REQUIRED**

## Authority and scope

This review used the approved step in `docs/feature/netns-density-295/deliver/roadmap.json`, the accepted contracts in `feature-delta.md`, `distill/test-scenarios.md`, `distill/red-classification.md`, the listed ADRs, and the architecture brief. The review inspected the cumulative diff from `526e961842d7` through `b4400801d1f53ff11fe31b3f7b17bfc95f2e4b37` and the production callers that make the scheduler and gate reachable.

The cumulative implementation diff is limited to the two core production files, the two named acceptance files, and the DES execution log. No public declaration was added by the step; the production changes are a private scheduler cap and a private claim-drop debug invariant.

## Strengths

- The exact accepted EXEC public surface remains intact. `GuestNetworkExecWiring`, `GuestNetworkExecGate`, `GuestNetworkExecSupervisor`, `GuestNetworkExecClaim`, the twelve `SharedGuestNetworkComponent` variants, the six `SharedGuestNetworkFailStopCause` variants, and the recovery/fail-stop value fields match the feature-delta contract.
- The gate acceptance body enters the real public core capability API rather than a fake state machine. The model checks the legal state transitions, waiter blocking/waking, recovery refusal, typed receipt fields, and terminal first-request behavior.
- `scheduler::schedule` is the accepted pure placement port. The production reconciler calls it at `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1075-1079`, and the action-shim guest pool assignment follows only after a successful placement action at `crates/overdrive-control-plane/src/action_shim/mod.rs:1397-1401`.
- The cap implementation is private, preserves the existing `PlacementError::NoCapacity` shape, adds no operator/wire/persisted resource, and correctly checks the 16,383/16,384/16,385 boundary in the active cap body.
- The acceptance diff only removes the authored pending `#[ignore]` markers. No assertion was weakened, deleted, or replaced. `git diff --check` and `cargo fmt --all -- --check` are clean.
- Both step commits retain Marcus's existing Git author and contain exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer; no Claude/Anthropic/generated-by attribution is present.

## Contract Shape Compliance

**Overall: FAIL.** The exact `CONTRACT_SHAPE` declarations are present, but the required outcome anchors and bounded-change evidence are not.

| Check | Result | Evidence |
|---|---|---|
| `CONTRACT_SHAPE` declaration on every live test | PASS mechanically | Six declarations in `netns_density_exec_gate.rs` and one in `netns_density_placement_cap.rs`; direct count and source inspection agree. |
| Required outcome anchor | **FAIL** | `rg` finds zero `/// Outcome anchor: DISCUSS Elevator Pitch` lines in either changed acceptance file. The repository's accepted DISTILL handoff requires this line for every transitioned body (`distill/test-scenarios.md:695`), and `feature-delta.md:7327-7328` makes the same requirement explicit. |
| Banned technical test names | PASS | No live test name matches `^test_.*(returns_[0-9]+|exit_code|calls_.*_once|status_code|http_[0-9]+)`. |
| Pure-function classification | PASS | The placement test is a pure call to the public `scheduler::schedule` port and has the required `pure-function` declaration. |
| Bounded-change delta and complement | **FAIL** | The gate tests assert selected return values and public projections but do not capture/assert the accepted gate universe before and after each operation. The feature delta requires both intended delta and complement equality (`feature-delta.md:4719-4720, 4731-4732`). |
| Claim-drop evidence | **FAIL** | The property model's `active_claims` vector is only a test-side expected list (`netns_density_exec_gate.rs:151, 208-229, 266-269`). No assertion observes production `GuestNetworkExecState.active_claims` or proves that `GuestNetworkExecClaim::Drop` at `guest_network.rs:320-327` applies the required `-1` delta. |

## Blocking findings

### D1 — Every transitioned acceptance body is missing the required Outcome anchor

- **Severity:** Blocker
- **Dimension:** Contract Shape compliance / mechanical acceptance metadata
- **Locations:** `crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs:67,75,97,137,275,291`; `crates/overdrive-core/tests/acceptance/netns_density_placement_cap.rs:37`
- **Evidence:** Direct source scan reports `CONTRACT_SHAPE` counts of 6 and 1, but outcome-anchor counts of 0 and 0. No `#[ignore]` marker remains, so these are the live transitioned bodies covered by the step. This is the same mechanical declaration required by the accepted DISTILL handoff and the repository's prior DELIVER review convention.
- **Reachability:** This is a directly observable acceptance-contract defect; no runtime hypothesis is needed. The bodies are the exact selectors executed by the step and are the evidence artifacts consumed by the DELIVER gate.
- **Required disposition:** Add the exact rustdoc line `/// Outcome anchor: DISCUSS Elevator Pitch` to all seven live bodies, retaining the existing `CONTRACT_SHAPE` lines and assertions. Rerun the repository Contract Shape checker if/when its installed path is available; the requested `src/des/cli/check_contract_shape_declarations.py` path is absent in this checkout, so this review did not claim that checker passed.

### D2 — The bounded-change gate evidence does not prove the approved delta/complement or claim-drop behavior

- **Severity:** Blocker
- **Dimension:** Test integrity / Contract Shape completeness / observable behavioral coverage
- **Locations:** `netns_density_exec_gate.rs:77-95,99-134,139-271,277-315`; production state `guest_network.rs:46-49,173-200,320-327`
- **Authority:** The accepted Contract Shape table defines the gate universe as state plus active-claim count, waiter registration/wake, recovery snapshot, and terminal request, with all other named effects complement-equal (`feature-delta.md:4719-4720`). It explicitly says bounded owners must prove both intended delta and complement (`feature-delta.md:4731-4732`).
- **Evidence:** The tests assert selected public outcomes, but never snapshot the complete allowed universe before/after an operation. `DropClaim` only pops the model vector; the production private count is not observed. The recovery test drops claims after its last assertion, and the fail-stop test drops its prior claim after terminal refusal, so neither proves the `+1/-1` claim lifecycle. The current `active_claims` field is written in `claim_release` and `Drop`, but no active step assertion reads it or reaches an observable owner consequence.
- **Reachability proof:** The real production entry path is `GuestNetworkExecGate::claim_release` (`guest_network.rs:173-200`) followed by the real RAII `GuestNetworkExecClaim::Drop` (`guest_network.rs:320-327`); the active property invokes that path directly. A bounded mutation that removes the production decrement leaves the current selector's public assertions unchanged because `active_claims` is never read by any gate method. This is an evidence failure in the current production path, not a claim that the existing decrement is currently wrong.
- **Required disposition:** Strengthen the existing accepted model evidence with a sanctioned private-state delta/complement assertion for the gate universe and an explicit claim-drop assertion. Do not add a public state accessor, a second gate API, or a test-only production owner. If the accepted architecture has no sanctioned way to observe the private count from this body, surface that as a DESIGN/testability gap instead of inventing public API.

### D3 — Typed component coverage is not exhaustive or deterministic

- **Severity:** Blocker
- **Dimension:** Acceptance completeness / typed error coverage
- **Location:** `netns_density_exec_gate.rs:14-36,56-64,164-205,275-315`
- **Authority:** Roadmap criterion 03-01 requires every one of the twelve component variants and six cause variants to have an asserting typed case, with no Display/Debug string matching (`roadmap.json:361-362`). S-ND295-27 also requires the finite state table to exercise the closed vocabulary (`distill/test-scenarios.md:722-741`).
- **Evidence:** `CAUSES` is looped deterministically in `every_fail_stop_cause_is_closed_and_first_request_wins`, but all six calls begin in `BootClosed`, so they assert only `Supervisor` as the component. Deterministic component cases cover `TcxLink` and `BridgeGuard` in the two focused bodies and `Bridge`/`Dns` in the illegal-event body. The other component variants are reachable only through randomized proptest indices (`0_u8..12`), and the generated operation sequence may terminate in `FailStop` before exercising a requested index; 1,024 cases are not an exhaustive typed table. There is no deterministic component/cause pair table.
- **Reachability proof:** Every component is accepted through the real public `begin_recovery`/`fail_stop` path, and the current source has no hidden variant conversion. The missing cases are therefore reachable production inputs that the committed acceptance evidence does not deterministically exercise; this is a coverage failure, not a hypothetical private state.
- **Required disposition:** Add a deterministic table over the accepted typed arrays (including the component/cause combinations required by the roadmap), assert enum fields directly, and retain the existing randomized model as supplemental evidence. Do not match Display/Debug text and do not add enum variants or helper API.

### D4 — The required below-cap `PoolExhausted` behavior has no evidence

- **Severity:** Blocker
- **Dimension:** Acceptance completeness / error-path coverage
- **Locations:** `netns_density_placement_cap.rs:39-63`; existing pool evidence `crates/overdrive-control-plane/src/guest_network.rs:6297-6324`
- **Authority:** Roadmap criterion 03-01 explicitly requires the cap-1/cap/cap+1 boundary **and** the below-cap `GuestAddressPool` `PoolExhausted` path (`roadmap.json:364`). ADR-0121 requires below-cap pool exhaustion to remain typed non-terminal infrastructure drift rather than allocation failure.
- **Evidence:** The active placement test constructs only `AllocStatusRow` values and calls pure `scheduler::schedule`; it never constructs/calls `GuestAddressPool` or asserts `GuestNetworkError::PoolExhausted`. The only existing pool-exhaustion body fills the `/16` to `held=65,533` (`guest_network.rs:6299-6324`), which is above the fixed 16,384 admission cap and therefore cannot evidence exhaustion below the admitted boundary. The proptest pool body assigns at most 512 operations and never asserts exhaustion.
- **Reachability proof:** In the real production path, successful placement returns a `StartAllocation` action (`workload_lifecycle.rs:1075-1120`), and only then does the action shim call `assign_action_plan` (`action_shim/mod.rs:1397-1401`). The typed error path is a real private pool entry point (`guest_network.rs:452-472`) and is specifically named by the accepted contract; current changed selectors never drive it.
- **Required disposition:** Add or activate the accepted below-cap pool-error body at the existing private `GuestAddressPool` evidence boundary, using a deterministic small-prefix/drift fixture whose held count is less than 16,384. Assert the typed `PoolExhausted { held, capacity }` and no address reuse/state mutation. Keep the fixed cap and pool API unchanged; do not add a public pool surface.

## External validity and architecture review

The EXEC tests drive the accepted public capability methods directly, which is the declared Tier-1 in-memory boundary for S-ND295-27. The placement cap is the declared pure-function evidence boundary, and the production caller chain is present. No invented public method, type, enum variant, trait, or parameter appears in the cumulative step diff. No architectural expansion is required by these findings.

No implementation-bias or RPP blocker was found in the step diff. The private cap is directly required by CAP-295-A/ADR-0121; the claim-drop assertion is adjacent to the accepted ownership contract rather than a new public mechanism.

## Quantitative validation

### Test budget

The step maps to five observable behaviors: gate transition/admission, typed component/cause outcomes, accepted grouped placement boundary, fixed cap boundary, and below-cap pool exhaustion. The resulting budget is 10 test methods (`2 × 5`). The changed files contain 7 test methods (the proptest property counts as one), so the numerical budget passes. This does not compensate for the missing typed/error evidence in D3/D4.

### Test integrity and TDD gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected activation set | PASS | All six gate bodies and the one placement body are active; neither changed file contains `#[ignore]`. |
| G2 — valid RED | PASS mechanically | DES RED for 03-01 records the cap body failing semantically at 16,384 before the implementation. |
| G3 — assertion failure, not collection failure | PASS | The reported RED was a semantic assertion failure; the alternate Lima build reached and ran the test binary. |
| G4 — no unsanctioned mocks | PASS | Tests use real core capabilities and the pure scheduler port; no mocked SUT or domain double is present. |
| G5 — business/Contract Shape language | **FAIL** | D1 misses all outcome anchors; D2 misses the approved bounded-change delta/complement evidence. |
| G6 — all selected tests green | PASS with environment-qualified command | Writable-target Lima selectors passed; the configured default target is read-only. |
| G7 — green before commit | PASS | DES GREEN precedes COMMIT in `execution-log.json`. |
| G8 — test budget | PASS numerically | 7 actual methods ≤ 10-method budget. |
| G9 — no prohibited test modification | PASS | The cumulative test diff only removes pending `#[ignore]` attributes. No assertion was weakened or deleted. |

### DES phase evidence

The last 03-01 events in `docs/feature/netns-density-295/deliver/execution-log.json` are ordered `RED` at `2026-09-22T13:35:16Z`, `GREEN` at `2026-09-22T13:56:42Z`, and `COMMIT` at `2026-09-22T13:57:47Z`; each is `EXECUTED/PASS`. The COMMIT event is after `a6c2b0ac` and records the scoped `b4400801` log commit. Earlier step history is preserved and is not counted as a replacement 03-01 cycle.

## Independent verification

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance -E 'test(netns_density_exec_gate)' --no-fail-fast` | **Environment blocked**: configured `/home/marcus.guest/.cargo-target-lima` is read-only. |
| Same EXEC selector with `CARGO_TARGET_DIR=/tmp/codex-netns-density-target` inside Lima | **PASS**: 6/6 selected tests; 523 unrelated tests skipped. |
| Same placement selector with writable Lima target | **PASS**: 1/1 selected test; 528 unrelated tests skipped. |
| `PROPTEST_CASES=1024` EXEC selector with writable Lima target | **PASS**: 6/6 selected tests; 523 unrelated tests skipped. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo check --workspace --all-targets --features integration-tests` | **PASS**. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | **FAIL, pre-existing/out of scope** at `crates/overdrive-netlink/src/nft.rs:5121` (`clippy::print_stderr`); no changed step file is reported. |
| `cargo fmt --all -- --check` | **PASS**. |
| `git diff --check 526e961842d7..b4400801d1f53ff11fe31b3f7b17bfc95f2e4b37` | **PASS**. |
| Direct Contract Shape scan | **FAIL** on zero outcome anchors; declarations and no-ignore counts otherwise pass. The requested checker path was not present in this checkout. |
| Host `cargo nextest` selector | **Environment blocked** by macOS compilation of Linux-only `linux-keyutils`/libc symbols. |
| Mutation testing | **NOT RUN**, as required for this DELIVER run. |

## Commit, scope, and worktree audit

- `a6c2b0ac6dbd22c5b8e3e0851f38af4a656acd6` and `b4400801d1f53ff11fe31b3f7b17bfc95f2e4b37` both retain Marcus Schack Abildskov as author.
- Each commit has exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer, one `Step-Id: 03-01` trailer, and no Claude/Anthropic/generated-by attribution.
- The cumulative step scope is the five expected files: `guest_network.rs`, `scheduler.rs`, the two acceptance files, and `execution-log.json`.
- Pre-existing dirty `AGENTS.md` and `.serena/project.yml` were preserved and are not part of the step commits or this review artifact's intended commit.
- No mutation testing ran.

## Remediation disposition

All four findings are open blockers. The original step-03-01 crafter must remediate within the approved architecture and the same step reviewer must re-review this step. No later roadmap step may start until the outcome anchors, complete bounded-change evidence, deterministic typed table, and below-cap pool-error evidence are present and verified. No finding requires a new public API or architecture; any missing sanctioned private observation boundary must be surfaced as a DESIGN gap rather than invented.

# CHANGES_REQUIRED

---

## Iteration 2 — remediation review

- **Review ID:** `code_rev_20260922_170500_iteration_2`
- **Iteration:** 2
- **Reviewed cumulative range:** `526e961842d7..8862112537fb66b3f8f6b7a89f75a5a7b8e3c737`
- **Remediation commits:** `d288fa813e4336efa48ef66e00c34e482f3f5c23`, `8862112537fb66b3f8f6b7a89f75a5a7b8e3c737`
- **Prior iteration:** `CHANGES_REQUIRED`, four blockers (D1–D4)
- **Iteration-2 verdict:** **CHANGES_REQUIRED** — D1, D3, and D4 are closed; D2 remains a blocking DESIGN/testability gap.

### Remediation dispositions

| Finding | Disposition | Independent evidence |
|---|---|---|
| D1 — missing Outcome anchors | **RESOLVED** | All six gate bodies and the placement body now carry the exact `/// Outcome anchor: DISCUSS Elevator Pitch` line. Direct scan: 6 anchors in `netns_density_exec_gate.rs`, 1 in `netns_density_placement_cap.rs`; no `#[ignore]` remains in either file. The new pool body also carries the anchor. |
| D3 — non-exhaustive typed component/cause coverage | **RESOLVED** | `every_fail_stop_cause_is_closed_and_first_request_wins` now iterates all 12 `COMPONENTS ×` all 6 `CAUSES`, enters `Open → Recovering(component)`, and asserts typed `first.component` and `first.cause` fields (`netns_density_exec_gate.rs:282-303`). No Display/Debug matching was added. |
| D4 — missing below-cap `PoolExhausted` evidence | **RESOLVED** | The new source-local `GuestAddressPool` body uses a `/30`, holds one lease, asserts `held=1 < 16,384`, expects typed `PoolExhausted { held: 1, capacity: 1 }`, and checks the snapshot is unchanged (`crates/overdrive-control-plane/src/guest_network.rs:6327-6349`). The exact selector passed independently. |
| D2 — private active-claim decrement unobservable | **OPEN — DESIGN BLOCKER** | The remediation intentionally retained no production/API/test hook. The recorded RED spike removed only `GuestNetworkExecClaim::Drop`'s decrement and all six EXEC-gate selectors still passed. Source audit confirms there is no existing owner read or sanctioned private snapshot boundary. |

### D1 — Contract Shape metadata is closed

The live transitioned core bodies now have the required metadata:

```text
netns_density_exec_gate.rs       Outcome anchors: 6  CONTRACT_SHAPE: 6  ignores: 0
netns_density_placement_cap.rs  Outcome anchors: 1  CONTRACT_SHAPE: 1  ignores: 0
```

The added `below_cap_pool_exhaustion_is_typed_drift_and_preserves_state` body also has both required declarations. No live test name matches the banned technical-result regex. The repository checker path named by the reviewer definition is not present in this checkout, so the result above is from direct source scanning rather than a claimed checker run.

### D3 — Deterministic typed table is closed

The prior randomized-only component coverage is corrected by the nested typed table. Each of the twelve closed `SharedGuestNetworkComponent` variants is recovered and each of the six closed `SharedGuestNetworkFailStopCause` variants is passed to the real public `GuestNetworkExecSupervisor::fail_stop`; the receipt fields are compared as enum values. The loop does not add a variant, string oracle, or public helper. The focused core selector passed 7 tests, including this 72-pair table, and the `PROPTEST_CASES=1024` selector passed all 6 gate tests.

### D4 — Below-cap pool drift is closed

The new body drives the real private `GuestAddressPool::assign` implementation through its existing owner boundary. Its `/30` has exactly one non-reserved address, so exhaustion is reproduced with one held lease—strictly below the 16,384 admission boundary—without changing `GuestAddressPool`'s API or production ownership. The typed error and unchanged snapshot are asserted. This is the requested below-cap error path; the older `/16` exhaustion body remains a separate full-pool boundary.

### D2 — Blocking DESIGN/testability gap, not an invented runtime defect

The D2 remediation report is reproducible and the current source confirms its reachability boundary:

- The accepted Contract Shape table names `active_claims` as an allowed `+1/-1` delta and says the whole private gate state must preserve its complement (`feature-delta.md:4719-4720`).
- The accepted EXEC code fence makes the capabilities opaque and all fields private (`feature-delta.md:3795-3810`); the design separately rejects a public gate-state accessor or test-only admission hook (`feature-delta.md:1671-1673`).
- The DISTILL S-ND295-27 executable mapping requires the model to compare every public return and `recovery_progress` projection, plus a finite typed-cause table; it does not define a private active-count snapshot (`distill/test-scenarios.md:722-741`). S-ND295-28 assigns claim-drop evidence to the deterministic real-`VmDriver` release/cancellation schedules in the later worker step (`distill/test-scenarios.md:747-760`).
- In production, `GuestNetworkExecState.active_claims` is incremented only by `GuestNetworkExecGate::claim_release` (`crates/overdrive-core/src/guest_network.rs:173-200`) and decremented only by `GuestNetworkExecClaim::Drop` (`:320-327`). The production owner holds the opaque claim in `_exec_claim` across `VmDriver::release_for_exit_emission` (`crates/overdrive-worker/src/vm_driver.rs:1836-1839`), but no production path reads `active_claims`; a repository-wide source scan finds only those writes and the test model's vector.
- The current `Notify` wake on claim drop is not an existing observable boundary: callers cannot inspect the waiter registration, and the gate does not use `active_claims` to admit/refuse or drain a claim. Removing only the decrement therefore leaves the current public selectors green. The remediation DES RED event at `2026-09-22T14:41:57Z` records this exact bounded spike; the production decrement was restored before the remediation commit.

This is not evidence that the restored decrement currently causes a reachable stakeholder-visible failure. It is a contradiction between the accepted private-universe wording and the sanctioned executable evidence boundary: the design names an internal delta, while the accepted opaque API and S-ND295-27 mapping expose no way to observe it in this step. Closing D2 would require a DESIGN decision on one precise question:

> Is the `active_claims +1/-1` field-level delta itself a required independently observed acceptance outcome for 03-01, or is public return/projection evidence in S-ND295-27 plus the production `VmDriver` claim-lifetime evidence in S-ND295-28 the complete accepted contract?

If the former is required, the design must approve a sanctioned private evidence boundary. If the latter is intended, the accepted Contract Shape wording must be reconciled so it does not require an unobservable private counter. This review does not prescribe either mechanism, add a public API, add a hook, or mandate production behavior beyond the approved architecture. Until that DESIGN/testability decision is independently approved, D2 remains blocking and this step cannot be approved.

## Iteration-2 verification

| Command / evidence | Result |
|---|---|
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo nextest run -p overdrive-core --test acceptance -E 'test(netns_density_exec_gate) or test(netns_density_placement_cap)' --no-fail-fast` | **PASS**: 7/7 selected tests; 522 unrelated tests skipped. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target PROPTEST_CASES=1024 cargo nextest run -p overdrive-core --test acceptance -E 'test(netns_density_exec_gate)' --no-fail-fast` | **PASS**: 6/6 selected tests; 523 unrelated tests skipped. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo nextest run -p overdrive-control-plane --lib -E 'test(=guest_network::pool_acceptance::below_cap_pool_exhaustion_is_typed_drift_and_preserves_state)' --no-fail-fast` | **PASS**: 1/1 selected test; 247 unrelated tests skipped. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo check --workspace --all-targets --features integration-tests` | **PASS**. |
| `cargo fmt --all -- --check` | **PASS**. |
| `git diff --check 526e961842d7..8862112537fb66b3f8f6b7a89f75a5a7b8e3c737` | **PASS**. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | **FAIL, unchanged environment/baseline** at `crates/overdrive-netlink/src/nft.rs:5121` (`clippy::print_stderr`); no remediation file is reported. |
| Configured Lima target without override | **Environment limitation**: `/home/marcus.guest/.cargo-target-lima` is read-only; writable `/tmp` target was used for independent selectors. |
| Contract Shape direct scan | **PASS for D1**: all transitioned core bodies have anchors/declarations and no ignores; checker executable path remains absent. |
| Mutation testing | **NOT RUN**, as required. |

### TDD, integrity, and scope re-check

- The remediation DES cycle is ordered `RED 2026-09-22T14:41:57Z → GREEN 2026-09-22T14:47:53Z → COMMIT 2026-09-22T14:48:19Z`, each `EXECUTED/PASS`. The GREEN and COMMIT descriptions retain the explicit D2 DESIGN blocker rather than claiming unsupported closure.
- The remediation strengthens evidence only: anchors were added, the typed table was expanded, and one new pool error body was added. No prior assertion was weakened, deleted, skipped, or replaced; G9 passes.
- The cumulative remediation adds one tightly necessary private source-local pool test outside the original core file list to satisfy the roadmap's explicit `GuestAddressPool` criterion. It adds no public declaration or production ownership mechanism.
- No mutation testing ran. Existing dirty `AGENTS.md` and `.serena/project.yml` remain untouched and uncommitted.

## Iteration-2 verdict

D1, D3, and D4 are **RESOLVED** with independent source and selector evidence. D2 is a reproduced, non-hypothetical DESIGN/testability contradiction: the accepted feature-delta private-universe wording names an active-claim counter delta, but the accepted opaque capabilities and existing production owner provide no sanctioned observation boundary, and the real gate selectors remain green when only that decrement is removed. This review neither invents a test hook nor treats the unobservable counter as a runtime defect. The exact DESIGN decision described above is still required.

# CHANGES_REQUIRED

---

## Iteration 3 — approved DESIGN re-review

- **Review ID:** `code_rev_20260922_155200_iteration_3`
- **Iteration:** 3
- **Reviewed implementation range:** `526e961842d7..8862112537fb66b3f8f6b7a89f75a5a7b8e3c737`
- **Approved DESIGN input:** `078e00db5cf6fe6df0e1662bcf041878aa9f89b1`
- **Durable DESIGN review:** `5941c10d1959fd0d5efc67bdf0139e62f3b175e9` — **APPROVED**, zero findings
- **Prior iteration:** `CHANGES_REQUIRED`; D1, D3, D4 resolved and D2 retained as a DESIGN blocker
- **Iteration-3 verdict:** **APPROVED**

### Authority and review boundary

This re-review consumes the independently approved `D-295-DELIVER-03-01`
decision and its revised `feature-delta.md`, `distill/test-scenarios.md`,
`roadmap.json`, and architecture brief. The approved decision removes
private `active_claims +1/-1` storage from the step-03-01 acceptance oracle,
retains the opaque public-capability S-ND295-27 boundary, and assigns real
`VmDriver` claim-lifetime/acknowledgement/cancellation evidence to S-ND295-28
in step 03-02. No 03-02 evidence is credited here.

The DESIGN remediation was independently reviewed in
`docs/feature/netns-density-295/deliver/review-design-remediation-03-01.md`.
That durable review records zero findings and `# APPROVED`; its conclusion
was checked against the revised contract and current implementation/tests.

### D1–D4 dispositions

| Finding | Final disposition | Evidence |
|---|---|---|
| D1 — Outcome anchors/declarations | **RESOLVED** | All six gate bodies and the placement body carry the exact Outcome anchor and Contract Shape lines; neither core acceptance file has an ignore marker. The new below-cap pool body also has both declarations. |
| D2 — private active-claim bookkeeping oracle | **RESOLVED by approved DESIGN** | Revised roadmap notes and S-ND295-27 define public returns, projections, blocking/wake/refusal, and terminal behavior as the complete 03-01 boundary; private counter storage is explicitly not an independent outcome. S-ND295-28 remains mandatory for real `VmDriver` lifetime, writer acknowledgement, and cancellation. No accessor, snapshot, hook, or API was added. |
| D3 — deterministic typed coverage | **RESOLVED** | The nested typed table drives all 12 components × 6 causes through the real public supervisor and asserts enum fields directly. |
| D4 — below-cap PoolExhausted | **RESOLVED** | The /30 private pool body holds one lease, proves `1 < 16,384`, asserts typed `PoolExhausted { held: 1, capacity: 1 }`, and preserves the snapshot. |

### D2 reassessment against the approved decision

The revised authority resolves the prior contradiction without changing the
implementation or adding an observation seam:

- Revised `feature-delta.md` describes `GuestNetworkExecClaim` as an opaque
  lifetime. Its private `active_claims` counter is bookkeeping only; it is not
  projected or consumed by either capability.
- Revised S-ND295-27 maps step-03-01 evidence to public returns,
  `recovery_progress`, blocking, wake, refusal, terminal behavior, and typed
  component/cause coverage; it explicitly excludes a private counter oracle.
- Revised S-ND295-28 and step 03-02 retain the real production obligation:
  `release_for_exit_emission` acquires the claim before pending EXEC
  ownership and holds the opaque lifetime through writer acknowledgement and
  cancellation-owned termination.
- Revised roadmap notes remove the whole-private-state/counter requirement and
  prohibit inventing an accessor, snapshot, or test seam. Step 03-02 remains
  dependent on 03-01 with separate real-owner selectors.
- The current source has exactly the approved opaque surface. No production
  read, public projection, test-only hook, or alternate owner was introduced.

The prior bounded mutation—removing only the private decrement while keeping
public gate behavior unchanged—is therefore outside the revised S-ND295-27
oracle, not a step-03-01 defect. It remains appropriate that step 03-02 proves
the real opaque lifetime through `VmDriver` schedules; that evidence is not
claimed here.

### Contract Shape and API re-check

Direct source scan reports:

```text
netns_density_exec_gate.rs       Outcome anchors: 6  CONTRACT_SHAPE: 6  ignores: 0
netns_density_placement_cap.rs  Outcome anchors: 1  CONTRACT_SHAPE: 1  ignores: 0
```

The typed table uses direct enum equality and no Display/Debug oracle. The
placement body remains a pure scheduler-port call and introduces no partial
assignment or pre-cut `AllocationSpec` shape. The pool body is source-local
at the existing private owner boundary and adds no public API. The cumulative
implementation diff contains no new public method, type, trait, enum variant,
parameter, persisted field, accessor, hook, or owner.

No test assertion was weakened, deleted, skipped, or re-authored. The
remediation added required metadata, strengthened the typed table, and added
the exact missing below-cap error body.

## Iteration-3 verification

| Command / evidence | Result |
|---|---|
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo nextest run -p overdrive-core --test acceptance -E 'test(netns_density_exec_gate) or test(netns_density_placement_cap)' --no-fail-fast` | **PASS**: 7/7 selected tests; 522 unrelated tests skipped. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target PROPTEST_CASES=1024 cargo nextest run -p overdrive-core --test acceptance -E 'test(netns_density_exec_gate)' --no-fail-fast` | **PASS**: 6/6 selected tests; 523 unrelated tests skipped. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo nextest run -p overdrive-control-plane --lib -E 'test(=guest_network::pool_acceptance::below_cap_pool_exhaustion_is_typed_drift_and_preserves_state)' --no-fail-fast` | **PASS**: 1/1 selected test; 247 unrelated tests skipped. |
| `cargo xtask lima run -- env CARGO_TARGET_DIR=/tmp/codex-netns-density-target cargo check --workspace --all-targets --features integration-tests` | **PASS**. |
| `cargo fmt --all -- --check` | **PASS**. |
| `git diff --check 526e961842d7..5941c10d1959fd0d5efc67bdf0139e62f3b175e9` | **PASS**. |
| Contract Shape direct scan | **PASS** for transitioned bodies; all required anchors/declarations present and no ignores. The reviewer-definition checker executable is absent, so no checker pass is claimed. |
| Workspace clippy | **Environment/baseline limitation**: the existing `clippy::print_stderr` at `crates/overdrive-netlink/src/nft.rs:5121` remains outside step changes. |
| Configured Lima target without override | **Environment limitation**: `/home/marcus.guest/.cargo-target-lima` is read-only; writable `/tmp` target supplied selector evidence. |
| Mutation testing | **NOT RUN**, as required; reserved for the final DELIVER gate. |

### DES, design, and commit evidence

- The remediation DES cycle is ordered `RED 2026-09-22T14:41:57Z → GREEN 2026-09-22T14:47:53Z → COMMIT 2026-09-22T14:48:19Z`, each `EXECUTED/PASS`.
- DESIGN commit `078e00db5cf6fe6df0e1662bcf041878aa9f89b1` changes only the five authorized documentation artifacts; durable review commit `5941c10d1959fd0d5efc67bdf0139e62f3b175e9` records `APPROVED` with zero findings. Revised roadmap validation is `approved`.
- Step implementation/remediation commits through `8862112537fb66b3f8f6b7a89f75a5a7b8e3c737` retain Marcus as author and exactly one Codex trailer, with no Claude/Anthropic/generated-by attribution.
- This iteration owns only the append to this review artifact. Existing dirty `AGENTS.md` and `.serena/project.yml` remain untouched and uncommitted.

## Iteration-3 verdict

D1, D2, D3, and D4 are **RESOLVED** against the revised independently
approved contract. The current step proves the complete public opaque-capability
S-ND295-27 boundary, deterministic typed coverage, fixed-cap boundaries, and
below-cap typed pool drift without inventing API or test seams. Real
`VmDriver` claim-lifetime/writer/cancellation evidence remains assigned to
step 03-02 and is not credited here.

# APPROVED
