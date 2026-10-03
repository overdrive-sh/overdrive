# DELIVER Review — Step 06-04

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `06-04` — TAP activation gate: activate/quiesce/restore under one sequencer |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning |
| Iteration | 1 |
| Commits reviewed | `7b22b98a4a1fd916ac8c08f655e0d73b929e051c`, `ecd914e18d80c58f40553949e65e5017d08a9c74` |
| Verdict history | Iteration 1: **APPROVED** |

The review applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`.

## Scope and contract sources

Reviewed the changed owner, action-shim, and activated test paths against approved roadmap step 06-04, feature-delta D-295-R5 and D-295-R21, and DISTILL scenarios S-ND295-50/51/52/53/72 plus E23. The roadmap is approved at validation iteration 4. The review focused on the exact `TapActivation` and quiescence result partitions, one-owner lifecycle sequencing, EXEC-gate waiting, activation failure projection, host-side MAC re-read, Condemned exclusion, test activation and integrity, and the step's verification gates.

The E23 oracle was assessed against the approved roadmap correction in commit `a8a1eda4` and the bounded independent reconciliation at `.context/distill-e23-06-04.md`. That correction records the two outcomes already permitted by the accepted feature-delta E23 lane; it is not a new implementation or API proposal.

## Contract Shape Compliance

The production interface shape matches the accepted contract. `GuestNetworkProvisioner::activate(&GuestNetworkPlan) -> Result<TapActivation>`, `TapActivation::{Raised, QuiescenceLatched}`, `SharedGuestNetworkOwner::quiesce_managed_taps() -> Result<TapQuiescence>`, and `restore_quiesced_taps() -> Result<()>` use the accepted signatures and result types. The patch adds no public method, type, variant, trait member, parameter, error variant, persisted field, or wire change. The new guest-network EXEC-gate parameter is on private dispatch helpers; the existing public and test-gated dispatch signatures remain unchanged.

The production owner uses its existing shared owner and a single private `allocation_lifecycle` sequencer. Its one allocation map carries each exact plan, ifindex, ingress and egress program identities, and the private `ProvisionedDown`, `Active`, `QuiescedActive`, and `Condemned` phases. Activation, provision, teardown, audit, quiescence, and restore acquire that sequencer; activation cannot interleave with quiescence or restore.

The host lifecycle behavior matches D-295-R5:

- `activate` returns `QuiescenceLatched` without mutation while the latch is set; refuses a missing, unequal, or `Condemned` record with the accepted source-less `TapObserve` mismatch; and otherwise reads back the bridge, TAP identity/master, host-side MAC invariant, debug mask, guard, endpoint, ingress attachment/pin, and egress attachment/pin before `TapSetUp`.
- A completed activation is idempotent after read-back. After raising a TAP, the owner reads back the bridge and TAP, including stable ifindex, master, owner, persistence, and up-state. A failed final read-back attempts set-down plus a down read-back; a typed set-down failure takes precedence. These branches retain the existing error taxonomy.
- `quiesce_managed_taps` latches before mutation, visits every `Active` allocation in allocation order, sets it down and reads it back, moves confirmed allocations to `QuiescedActive`, and records per-TAP failures as `Condemned` plus `unconfirmed`. It continues after each per-TAP failure and returns `Ok(TapQuiescence)`; the host implementation does not return `Err` for per-TAP failures, including `Connect`.
- A repeated quiescence pass handles any `Active` allocations left by a partial restore and does no I/O when none remain. Restore raises only `QuiescedActive` allocations in order, preserves the latch on failure, and clears it only after complete success. `ProvisionedDown` and `Condemned` allocations are excluded.
- Host allocation-I/O methods used by quiescence await `Client::set_link_down` and `observe_persistent_tap_identity` directly on the ambient runtime. The quiescence path does not use `block_on_host_netlink`, spawn a blocking task, or synchronously wait for a task that is awaiting it. Its future remains droppable at awaits for the caller's bound.

The action shim reads the gate from `state.guest_network_exec` in the production owner path and in the existing test-gated seam. After the protection-live event it obtains a release claim, awaits activation, and only then reaches the existing command-release hook. `QuiescenceLatched` drops the claim and retries behind the gate; fail-stop emits exactly `guest_network.activation_withheld { alloc, reason: "fail_stop" }` and returns without releasing activation or EXEC. The existing `VmDriver::release_for_exit_emission` independently claims the same gate and retains its claim through the beacon acknowledgement. No driver API was added.

The D-295-R21 check runs during activation before any mutation. `reserved_host_macs` includes held allocations and the bridge MAC while excluding `Condemned`; a reserved or missing observed address returns the pinned `TapHostMac` mismatch. The owner audit also excludes Condemned allocations from later audit reads and from the reserved set. This meets S-ND295-72's activation and Condemned-exclusion clauses.

The roadmap's `script_quiesce_outcome` Sim surface was already present and accepted before these commits (`overdrive-sim/src/adapters/guest_network.rs:132`). Its absence from this step's file diff is not a missing implementation: it scripts the accepted `Unconfirmed`, `Fail`, and `Hang` outcomes without introducing a cross-crate `GuestNetworkPlan` constructor. No Sim adapter or test-port API expansion was added here.

**Source evidence:** the accepted port shapes are at `feature-delta.md:1636` and `guest_network.rs:108`, `:121`, `:131`, `:151`, `:177`, `:180`; the private phases and shared sequencer are at `guest_network.rs:2419` and `:2436`. Host activation, audit, quiescence, and restore are at `guest_network.rs:3823`, `:4716`, `:5219`, and `:5268`; host set-down/read-back are awaited at `:2111` and `:2117`. The reserved-MAC calculation and activation check are at `:2620` and `:3864`. The shim gets the gate from `AppState` at `action_shim/mod.rs:1327` and `:1411`, uses it in `:1676`, and reaches the activation point from the start and restart arms at `:2654` and `:3168`. The existing driver release claim is at `vm_driver.rs:1861`. Scenario authority is `test-scenarios.md:621`, `:831`, `:860`, `:885`, and `:1408`; the corrected E23 body is `guest_network.rs:11976`.

## Test integrity, coverage, and budget

The step activates the assigned bodies without changing their assertions, expected values, or stimuli, except for the separately reconciled E23 oracle described below. The 06-04 commit removes pending markers for the S-ND295-50/51/72 owner bodies, the S-ND295-52 lease and gate-wait integration bodies, and the three S-ND295-53 seeded Sim bodies. The two retained S-ND295-52 bodies remain active. No test was deleted, skipped, or weakened.

E23's original body required `Connect` even when the production owner could confirm the real TAP down without creating a new OS thread. The accepted E23 clause permits exactly two outcomes: confirmed-down with no thread refusal, or that TAP's exact `TapSetDown/Connect` entry following an actual refusal. The corrected body checks the real cgroup is at `pids.max`, reads the same TAP's ifindex and administrative state directly from the kernel, checks the refusal counter, and requires the exact per-TAP typed error and EAGAIN/WouldBlock evidence for the refusal branch. Its empty-`unconfirmed` branch requires that same TAP to be down and the refusal counter unchanged; up, absent, replaced, unverified, `Err`, panic, and abort fail. A no-cap control confirms the same TAP down. This reconciles the test to the approved contract and strengthens its production evidence; it does not accept an arbitrary `Ok` or change production code/API.

The source-local owner tests drive the production owner through its allocation-I/O port and assert exact call ordering, state, typed causes, and no-mutation complements. The S-ND295-52 tests exercise the in-process action-shim composition. The S-ND295-53 bodies exercise the real EXEC gate and Sim owner through seeded schedules, print their seeds, assert no activation while recovering, and cover reopen and fail-stop. E23 exercises the production host owner in-process against the real Lima kernel and cgroup; no Rust test launches the Overdrive binary or acts as an expectation runner.

Counting the assigned observable behaviors gives 20 behaviors and a budget of 40 under the review role's `2 × behaviors` rule: S-ND295-50's condemned-allocation universe; S-ND295-51's activation success/idempotency, pre-read refusal, latch/condemnation refusal, mixed quiescence partition, per-TAP `Connect`, repeat quiescence, bound miss, and partial-restore retry; S-ND295-52's activation ordering, recovery wait, no-lease failure projection, lease failure projection, and Condemned failure projection; S-ND295-53's recovery/reopen, latched retry, and fail-stop outcomes; S-ND295-72's reserved-address refusal/unreserved acceptance and Condemned exclusion; and E23's exact real-kernel result partition. Seventeen bodies are activated or reconciled for this step (10 source-local S-ND295-50/51/72 bodies, E23, three S-ND295-52 integration bodies, and three S-ND295-53 seeded bodies); the two retained S-ND295-52 bodies also remain active, for 19 assigned bodies considered, within budget. None is a source-local pure-function Rust property; the transitioned bodies retain their applicable `CONTRACT_SHAPE` declarations and outcome anchors.

| Gate | Result | Evidence |
|---|---|---|
| G1 — activation discipline | PASS | Only the roadmap's assigned S-ND295-50/51/52/53/72 and E23 bodies were activated for 06-04; retained S-ND295-52 bodies stay active. |
| G2 — valid RED | PASS, recorded | The 06-04 DES log records RED PASS before GREEN; the subsequent GREEN FAIL is followed by GREEN PASS. |
| G3 — meaningful assertions | PASS | Owner bodies assert typed results, kernel/fake state, order, and mutation complements; E23 independently reads the real TAP and cgroup. |
| G4 — boundary discipline | PASS | Sim and source-local tests use the accepted owner/allocation-I/O ports; the integration and E23 bodies stay in-process. |
| G5 — business/domain language | PASS | Activated bodies retain their S-ND295 identifiers, outcome anchors, and `CONTRACT_SHAPE` declarations. |
| G6 — all green | PASS, reported | All seven roadmap verification commands passed as reported for the step. |
| G7 — green before commit | PASS, recorded | GREEN PASS precedes COMMIT PASS in the DES log. |
| G8 — test budget | PASS | 20 behaviors; budget 40; 19 assigned active bodies (17 activated/reconciled and 2 retained). |
| G9 — no test weakening | PASS | Assertions and stimuli were preserved. E23's only semantic change follows the separately approved two-outcome contract correction and strengthens independent evidence for both branches. |
| External validity | PASS | Action-shim tests use the production test seam and real state gate; the seeded Sim tests verify recovery composition; E23 invokes the production host owner with real kernel resources. |

**Test integrity:** no testing theater, weakened acceptance, missing `CONTRACT_SHAPE` declaration, expectation-runner boundary violation, or test-only schedule promoted to a production defect was found. Mutation testing was not run during this roadmap step, as required.

**RPP scan:** L1 and L2 were reviewed within the changed surface. No actionable step-scoped smell was found; the owner reads and state transitions remain one ordered lifecycle operation as specified by the accepted port contract.

## DES, commit, and verification evidence

The 06-04 execution log records `RED PASS → GREEN FAIL → GREEN PASS → COMMIT PASS`; it preserves the honest failed GREEN attempt and records no commit before a passing GREEN. The feature commit retains Marcus as author, includes exactly `Co-Authored-By: Codex <codex@openai.com>`, carries `Step-Id: 06-04`, and contains no Claude, Anthropic, or generated-by attribution. The separate execution-log commit has the same required Codex trailer and step identifier. The only dirty file observed outside the reviewed commits was the pre-existing user `AGENTS.md`; it was preserved.

The following results were reported for the step and were not rerun during this review:

| Command | Reported result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --test acceptance -E 'test(netns_density_activation_order)' --no-fail-fast` | 3 passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --lib -E 'test(adapters::guest_network)' --no-fail-fast` | 6 passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --lib -E 'test(allocation_owner_acceptance) or test(shared_network_test_ports)' --no-fail-fast` | 27 passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(quiesce_managed_taps_never_aborts_when_a_thread_is_refused)' --no-fail-fast` | 1 passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(mtls_install_fail_closed)' --no-fail-fast` | 15 passed |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | Passed |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | Passed |
| Changed-file formatting and `git diff --check` | Passed |

## Findings and remediation dispositions

No proven blocker or required remediation was found. No failure hypothesis is promoted to a finding; no new regression was needed because the review found no reachable defect. The pre-existing Sim scripting surface and the unmodified retained S-ND295-52 bodies satisfy the criteria without additional file edits.

## Iteration 1 verdict

**APPROVED.** The implementation matches the approved R5/R21 API and lifecycle contract, serializes activation/quiescence/restore on the one shared owner, waits on the `AppState` EXEC gate, preserves the Condemned and host-MAC partitions, and meets the no-thread-blocking quiescence requirement. The activated tests preserve the accepted port boundaries and outcomes, including the independently corrected, production-evidenced E23 disjunction. The required workspace and test gates were reported green, and no reachable in-scope defect was proven.
