# Adversarial review — step 01-02

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `01-02` — tap-in-netns provisioning and C3 VM injection
- **Reviewer:** `nw-software-crafter-reviewer` (fresh isolated adversarial reviewer)
- **Review ID:** `code_rev_20260828_113652_iteration_1`
- **Iteration:** 1
- **Commit:** `d1d24329e22f7b291f8cce9f524c3a17625f516e`
- **Parent:** `6b9ffd2e1dc429b2846b2f22f30ccf4dbcd290c1`
- **Subject:** `feat(guest-stack-transparent-mtls-intercept): provision VM tap wire`
- **Trailer:** `Step-Id: 01-02`
- **Final verdict:** **NEEDS_REVISION**

## Executive summary

The commit is mechanically clean, compiles and lints in Lima, preserves unrelated work, wires the VM branch into C3, and keeps the three focused tests green. It cannot be approved because the kernel observer does not establish TAP type/persistence or the exact `/30` address, `ENODEV` can convert a missing wire into false success, the roadmap-mandated real-netns proof is absent, and the new action-shim tests violate their declared contract shape without preservation assertions. Remediate D1–D6 and return this same step's original crafter output to this reviewer for iteration 2.

## Contract Shape Compliance

**Overall: FAIL**

| Check | Status | Evidence |
|---|---|---|
| Exact per-test declarations | PASS | All 3 new tests carry the exact `CONTRACT_SHAPE` rustdoc line. |
| Outcome anchor | NOT APPLICABLE | No new acceptance test was authored in this step. |
| Banned test-name regex | PASS | No new test name matches the banned regex. |
| Semantic contract match | FAIL | The two action-shim tests declare `pure-function`, but `inject_workload_network` mutates a caller-owned `&mut AllocationSpec`. Their contract is `bounded-change`, not `pure-function`. |
| Preservation or delta checks | FAIL | Neither action-shim test snapshots the pre-state, declares the exact changed fields, or proves equality of the `AllocationSpec` complement. |
| Layer choice | FAIL | The tests invoke a private mutating helper directly and therefore do not prove either the C3 action-seam wiring or its fail-closed behavior. |

## Mechanical evidence

### Commit scope — PASS

- Stat: **46 files changed, 900 insertions, 57 deletions**.
- The behavioral changes are confined to the five roadmap-owned source files.
- `Cargo.toml` enables the compiler-required `nix` `ioctl` feature.
- The remaining source/test edits are neutral `AllocationSpec` literal fallout required by the new public fields.
- The execution log is the expected DELIVER artifact.
- No mutation exclusions were changed.
- `Cargo.toml` and the neutral literal updates are the tightly bounded compiler fallout expressly permitted by the repository instructions.
- Pre-existing dirty `roadmap.json` and `AGENTS.md` work was excluded from the commit review and preserved.

### DES phase order — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T09:11:33Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T09:25:21Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T09:57:22Z` |

All three canonical phases are present, successful, and chronologically ordered.

### Test budget — PASS

| Behaviors | Budget (`2 × behaviors`) | Actual new tests |
|---:|---:|---:|
| 6 | 12 | 3 |

### Test-integrity diff — PASS

The parent-to-step diff adds three tests and does not weaken, delete, skip, or reduce assertions in a pre-existing test. `AllocationSpec` fixture edits only initialize the five new fields to `None`.

## Blocking findings

### D1 — The required TAP type and persistence are not observed

- **Severity:** Critical
- **Dimension:** Design compliance and convergence correctness
- **Locations:**
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:1370`
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:2550`
  - `crates/overdrive-netlink/src/client.rs:163`

**Evidence:** `ObservedVmTap` calls a name-only `RTM_GETLINK` observer and records `tap_present=true` for any named link. It checks neither that the device is a TAP nor that it is persistent. A non-persistent TAP or a veth/dummy collision therefore suppresses `CreatePersistentTapInNetns` and is accepted as converged, contrary to roadmap criteria 42/46/66 and the feature-delta Earned-Trust requirement to observe device kind/persist.

**Required remediation:** Add a real, typed actual-state observation for TAP kind and persistence inside the allocation netns. Model those facts explicitly and either repair them idempotently or fail closed on an incompatible name collision; link-name presence alone must not satisfy the TAP step.

### D2 — Gateway-address observation ignores the prefix length

- **Severity:** High
- **Dimension:** Drift-repair correctness
- **Locations:**
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:2556`
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:3044`
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:1750`

**Evidence:** TAP gateway observation compares only the IPv4 address and ignores its prefix length. For example, `tap_gateway/32` is classified as the desired guest-`/30` address, so the connected guest-network route remains broken and no repair is emitted. The add path also swallows `EEXIST`, so this malformed state is not corrected.

**Required remediation:** Observe the exact address-and-prefix tuple and define a deterministic replace/rebuild repair for a wrong prefix. Verify the required `/30` postcondition after convergence.

### D3 — `ENODEV` can convert an incomplete wire into successful provisioning

- **Severity:** High
- **Dimension:** Fail-closed error handling
- **Locations:**
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:2743`
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:2784`
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:2802`
  - `crates/overdrive-netlink/src/error.rs:237`

**Evidence:** The create/add convergence path treats every `-ENODEV` as successful idempotency. `ENODEV` from `require_index` can mean the TAP or host veth is genuinely absent, not merely that a TAP was concurrently moved. The subsequent address/link-up helpers also swallow `ENODEV`, allowing `provision_vm_tap` to return `Ok` with no TAP and/or no return route. The action shim can then start the VM despite an incomplete wire, violating the step's fail-closed requirement.

**Required remediation:** Make errno idempotency operation-specific: `EEXIST` is benign for adds; `ENODEV` is benign for deletion, or for an ambiguous move only after the destination state is re-observed. Otherwise retry/recompute or return the typed error, and verify required postconditions before success.

### D4 — The roadmap-mandated Tier-3 real-kernel proof is absent

- **Severity:** Critical
- **Dimension:** Acceptance coverage and external validity
- **Locations:**
  - Missing Tier-3 real-kernel test for roadmap 01-02 criteria 45–49
  - `crates/overdrive-control-plane/src/veth_provisioner.rs:2044`
  - `crates/overdrive-netlink/src/client.rs:52`

**Evidence:** No test exercises the newly added `/dev/net/tun` ioctls, `RTM_SETLINK` namespace move, namespace address/up state, namespace `ip_forward`, host return route, restart no-op, independent drift repair, fail-closed errors, or terminal zero-residue behavior. The only converge test is a pure `Vec`-membership property. This fails the roadmap's explicit completion proof against a real per-allocation netns in Lima and leaves every new impure adapter boundary unverified.

**Required remediation:** Add the roadmap-required Lima-routed real-netns integration coverage. Observe the exact TAP kind/persistence, address/prefix/up state, `ip_forward`, and host route; re-provision and prove no-op; corrupt each fact independently and prove repair; inject a fatal adapter failure and prove the allocation refuses to start; then tear down and prove zero TAP/route residue. No guest boot or per-step mutation run is required.

### D5 — The action-shim tests declare the wrong contract and omit preservation assertions

- **Severity:** Blocker
- **Dimension:** Contract Shape Compliance and test design
- **Locations:**
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1001`
  - `crates/overdrive-control-plane/src/action_shim/mod.rs:1029`

**Evidence:** Both `AllocationSpec` injection tests falsely declare `pure-function` for a mutating operation and assert only selected post-state fields. A defect that also changes `alloc`, `identity`, `driver`, `resources`, probe descriptors, or service ports would remain green. The Exec test additionally omits assertions for `netns` and `host_veth`, despite the helper changing them.

**Required remediation:** Declare the tests `bounded-change` and use a before/after state-delta assertion with the exact permitted field set plus full complement equality. Prefer extracting a pure plan/value if that makes the Functional-Core/Imperative-Shell boundary explicit, then separately verify that the C3 seam applies it and propagates provisioning failure.

### D6 — The converge property does not prove exactness, minimality, or ordering

- **Severity:** High
- **Dimension:** Property-oracle strength
- **Location:** `crates/overdrive-control-plane/src/veth_provisioner.rs:4823`

**Evidence:** The converge property claims an exact, minimal, ordered repair set but asserts only `contains(...)` for each variant plus emptiness for the all-present case. Duplicate steps or any permutation of the required order would pass, so the load-bearing ordering/minimality contract is unprotected.

**Required remediation:** Independently construct the expected ordered `Vec` from the observed facts and assert exact vector equality, which simultaneously proves membership, uniqueness, minimality, and ordering for all 16 states.

## External validity

**Status: FAIL**

The VM branch is reachable from the existing C3 Start/Restart seam and uses the same slot-derived workload and TAP plans. However, no real entry-path or real-kernel test demonstrates that the reachable path creates a usable, persistent, drift-repaired wire or refuses startup when the wire cannot be completed.

## Verification

| Verification | Result |
|---|---|
| `git diff --check 6b9ffd2e1dc429b2846b2f22f30ccf4dbcd290c1 d1d24329e22f7b291f8cce9f524c3a17625f516e` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check -p overdrive-netlink -p overdrive-control-plane --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-netlink -p overdrive-control-plane --all-targets --features integration-tests -- -D warnings` | PASS |
| Focused Lima `nextest` run for the three new tests | PASS — 3 passed, 0 failed, 211 skipped |
| Source scan for `provision_vm_tap`, `create_persistent_tap`, `TUNSETIFF`, `TUNSETPERSIST`, and `vm_tap_converge` across production and test trees | FAIL — only the source-local pure/member tests reference the new path; no real-kernel integration coverage exists |
| Mutation testing | NOT RUN — explicitly prohibited during individual roadmap steps |

The focused test command was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --lib -E 'test(vm_tap_converge_repairs_each_observed_drift_independently) | test(vm_injection_uses_guest_address_and_complete_guest_net_channel) | test(exec_injection_keeps_transit_address_and_no_guest_net_channel)'
```

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — Exactly one acceptance active | PASS | Walking-skeleton/metal-deferred override applies. |
| G2 — Valid RED failure | PASS | Ordered DES RED event is `EXECUTED/PASS`. |
| G3 — Assertion failure | PASS | Ordered DES RED event is `EXECUTED/PASS`. |
| G4 — No domain mocks | PASS | No mocks were introduced in the new tests. |
| G5 — Business language | PASS | New test names and assertions describe the TAP converge and VM/Exec injection outcomes. |
| G6 — All green | PASS | Relevant compile, lint, and focused-test lane independently passed. |
| G7 — 100% passing before commit | PASS | DES COMMIT event is `EXECUTED/PASS`. |
| G8 — Test budget | PASS | 3 tests ≤ budget 12. |
| G9 — No test modification | PASS | No pre-existing assertion was weakened, deleted, or skipped. |

These mechanical gates do not cure the substantive correctness and coverage defects above.

## Test integrity and RPP scan

- **Test modification detected:** No.
- **Testing theater detected:** No. D5 and D6 are contract-oracle deficiencies, but the new tests still contain meaningful assertions; no classic always-green or mock-dominated theater pattern was found.
- **Escalation verification:** Not applicable.
- **RPP levels scanned:** L1–L2.
- **Cascade stopped at:** None.
- **RPP findings:** None.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 2 |
| High | 3 |
| Medium | 0 |
| Low | 0 |
| **Total** | **6** |

## Final verdict

**NEEDS_REVISION**

The implementation is not eligible to advance to the next roadmap step. D1–D6 must be remediated by the original step 01-02 crafter, after which this same reviewer should perform iteration 2.

## Iteration 2

- **Review ID:** `code_rev_20260828_122033_iteration_2`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated reviewer as iteration 1)
- **Remediation commit:** `5523e4a7981e972fc1a62468948722c63c5a7b19`
- **Parent:** `d1d24329e22f7b291f8cce9f524c3a17625f516e`
- **Subject:** `fix(guest-stack-mtls): harden VM TAP convergence`
- **Trailer:** `Step-Id: 01-02`
- **Iteration-2 verdict:** **REJECTED**

### Iteration-2 executive summary

The remediation correctly fixes D1, D2, D3, D5, and D6. Typed `RTM_GETLINK` TAP-kind/persistence observation now works against the real Lima kernel; exact-address convergence repairs `/32` to `/30`; add/move/up operations no longer accept `ENODEV`; a fresh postcondition pass refuses incomplete wires; the `AllocationSpec` tests now declare bounded change and prove the entire complement; and the converge property asserts exact ordered-vector equality. The new C3 scenarios also executed successfully through the real action seam in Lima, and an independent post-run inspection found no TAP or route residue.

The step is nevertheless rejected because D4's new real-kernel test contains broad early-return “SKIP” branches on the system under test's own provisioning failures. Breaking `/dev/net/tun`, TAP classification, netlink movement, address convergence, or another provisioning boundary can therefore turn the required real-I/O proof green without executing an assertion. The terminal oracle also omits an in-test assertion that the TAP name is absent from the host namespace. This is an always-green/testing-theater shape and violates the mandatory real-adapter contract. A new low-severity stale rustdoc defect is also present. Iteration 2 is the maximum reviewer iteration, so the unresolved blocker requires facilitator escalation rather than approval.

### D1–D6 disposition

| Finding | Disposition | Evidence |
|---|---|---|
| D1 — TAP type/persistence not observed | **RESOLVED** | `TapLinkState` and `Client::observe_persistent_tap` classify typed `IFLA_INFO_KIND=tun`, `IFLA_TUN_TYPE=IFF_TAP`, and `IFLA_TUN_PERSIST=1`; incompatible same-name links fail closed. The real C3 test observed a persistent TAP, and the collision test refused a dummy link before driver start. |
| D2 — Exact `/30` prefix not observed/repaired | **RESOLVED** | `Client::observe_addr` includes `AddressMessage.header.prefix_len`; `converge_addr` removes mismatched same-address entries before adding the exact prefix. The real test replaced `tap_gateway/30` with `/32`, restarted, and proved `/30` restored and `/32` absent. |
| D3 — `ENODEV` accepted as successful create/add | **RESOLVED** | Operation-specific `errno_is_already_exists` and `errno_is_absent` rules separate add from delete. Move, link-up, address, and route paths surface `ENODEV`; add accepts `EEXIST` only after exact re-observation. `provision_vm_tap` performs a complete fresh postcondition observation before returning `Ok`. |
| D4 — Mandatory Tier-3 real-kernel proof absent | **NOT RESOLVED — BLOCKER** | Two real-netns scenarios now exist and executed in Lima, but lines 696–708 convert any C3 `netns_provision` failure into a passing return, and lines 948–954 convert any baseline `provision_workload_netns` error into a passing return. Thus an actual adapter regression can skip the proof. The teardown assertions at lines 895–906 check netns, veth, route, and slot absence but do not assert that the TAP name is absent from the host namespace. |
| D5 — False `pure-function` declarations and no preservation proof | **RESOLVED** | Both injection tests now declare `bounded-change`, snapshot `before`, build the exact allowed `expected` state, and assert full-`AllocationSpec` equality. The real C3 success and collision scenarios separately exercise the production action seam and prove fail-closed-before-driver-start behavior. |
| D6 — Converge oracle omitted exactness/minimality/order | **RESOLVED** | The property independently constructs the expected ordered `Vec` for all generated Boolean states and asserts exact vector equality, proving membership, uniqueness, minimality, and ordering. |

### Contract Shape Compliance — PASS

| Check | Status | Evidence |
|---|---|---|
| Per-test declarations | PASS | The seven live step tests are classified: three original tests plus four remediation tests; pure properties use the exact `/// CONTRACT_SHAPE: pure-function.` line and state-changing tests declare `bounded-change`. |
| Semantic match | PASS | The two `AllocationSpec` transformations and two real-kernel C3 scenarios are bounded change; the converge, errno, and typed TAP-classifier tests are pure. |
| Bounded-change delta and complement | PASS | The injection tests assert the exact allowed field delta and entire `AllocationSpec` complement. The real-kernel scenarios declare their slot-bounded kernel-resource universe and read outcomes through the C3 seam. |
| Driving-port/external path | PASS | The two integration scenarios dispatch `StartAllocation`/`RestartAllocation`/`StopAllocation` through `action_shim::dispatch`; the collision scenario proves the VM driver remains unstarted. |
| Banned test-name regex | PASS | No new or transitioned test name matches the banned regex. |
| Outcome anchor | NOT APPLICABLE | The remediation adds integration scenarios, not a new authored acceptance-test artifact. |

### D4 remaining blocker — the real-I/O gate can pass without exercising the adapter

- **Severity:** Blocker
- **Dimension:** Testing theater, adapter integration, and external-validity enforcement
- **Locations:**
  - `crates/overdrive-control-plane/tests/integration/alloc_netns_lifecycle.rs:695`
  - `crates/overdrive-control-plane/tests/integration/alloc_netns_lifecycle.rs:948`
  - `crates/overdrive-control-plane/tests/integration/alloc_netns_lifecycle.rs:895`

**Evidence:** In the positive C3 scenario, any `Failed` row carrying `WorkloadNetnsProvisionFailed(stage="netns_provision")` prints `SKIP` and returns. That category includes the exact implementation failures the test is required to catch: a broken TUN ioctl, a bad persistence parser, a failed netns move, address convergence failure, sysctl failure, or return-route failure. In the collision scenario, every `provision_workload_netns` error likewise prints `SKIP` and returns, without distinguishing a genuine environment capability limitation from a production regression. Consequently, deleting or breaking a real adapter boundary can still produce a green test. This contradicts the mandatory adapter definition: a real-I/O test must fail when its actual system dependency or adapter is absent or broken.

The independent iteration-2 run did execute both scenarios rather than taking these branches: each emitted its explicit `EXECUTED ...` marker and passed. That proves the current commit works in the reviewer's Lima instance, but it does not make the committed test a trustworthy regression gate. After the run, independent inspection also found `ovd-tp-0013` and `ovd-tp-0014` absent and no routes for `10.99.128.76/30` or `10.99.128.80/30`. The committed terminal assertions do not themselves check host-namespace TAP absence, so a future TAP migration/leak could remain green.

**Required remediation:** Keep the pre-SUT `!is_root()` environment gate if non-canonical runners must remain supported, but never classify the SUT's own broad provisioning failure as an environment skip. On the canonical root Lima lane, require the positive scenario to reach `Running`; allow only a narrowly proven capability preflight to skip elsewhere. In the collision fixture, surface unexpected baseline provisioning errors instead of returning. Add an explicit post-terminal `ip link show dev <tap>`-is-absent assertion in the host namespace alongside the existing route/netns/veth checks.

### D7 — `nl_set_up` rustdoc contradicts the remediated fail-closed behavior

- **Severity:** Low
- **Dimension:** RPP L1 readability/documentation correctness
- **Location:** `crates/overdrive-control-plane/src/veth_provisioner.rs:1892`

**Evidence:** The rustdoc still states that `-ENODEV` is “swallowed,” while the remediated implementation now intentionally maps every `set_link_up` error, including `ENODEV`, to `LinkUpFailed`. This contradicts D3's operation-specific safety rule and can mislead a later maintainer into restoring the removed behavior.

**Required remediation:** Update the rustdoc to state that link-up is kernel-idempotent when the link exists, while every returned error—including `ENODEV`—is surfaced fail-closed.

### Scope and test-integrity review

| Check | Status | Evidence |
|---|---|---|
| Commit scope | PASS | Seven files changed: the two owned production modules, netlink export/client, the real-netns test and slot registry, plus the execution log. The wider veth helper edits remove the shared broad errno classifier identified by D3 and are covered by the affected-package suite. |
| Diff hygiene | PASS | `git diff --check d1d24329… 5523e4a7…` passed. No mutation configuration or unrelated product behavior was changed. |
| Existing-test integrity | PASS | The three iteration-1 tests were strengthened: selected assertions remain, whole-spec complement equality was added, and membership assertions became exact-vector equality. No pre-existing test was deleted, skipped, or weakened. |
| New-test integrity | **FAIL** | The broad SUT-failure early returns in D4 are an always-green/testing-theater path. |
| Test budget | PASS | 6 roadmap behaviors permit 12 tests; the step now has 7 new/transitioned tests. |

### Remediation DES phase triplet — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T11:53:46Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T12:10:04Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T12:12:32Z` |

The remediation triplet is complete, successful, and chronologically ordered after the iteration-1 triplet. The commit timestamp follows the COMMIT event.

### Iteration-2 verification

| Verification | Result |
|---|---|
| `git diff --check d1d24329e22f7b291f8cce9f524c3a17625f516e 5523e4a7981e972fc1a62468948722c63c5a7b19` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check -p overdrive-netlink -p overdrive-control-plane --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-netlink -p overdrive-control-plane --all-targets --features integration-tests -- -D warnings` | PASS |
| Five focused unit/property tests across `overdrive-control-plane` and `overdrive-netlink` | PASS — 5 passed, 0 failed |
| Two focused real-netns C3 integration scenarios with `--no-capture` | PASS — 2 passed, 0 failed; both emitted their `EXECUTED` marker, proving no skip branch was taken |
| Full affected-package Lima suite with `integration-tests` | PASS — 817 passed, 0 failed, 3 skipped |
| Post-run host TAP inspection | PASS for current run — `ovd-tp-0013` and `ovd-tp-0014` absent |
| Post-run guest-route inspection | PASS for current run — no route remained for `10.99.128.76/30` or `10.99.128.80/30` |
| Mutation testing | NOT RUN — explicitly prohibited during individual roadmap steps |

### Iteration-2 quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — Exactly one acceptance active | PASS | Walking-skeleton/metal-deferred override remains applicable. |
| G2 — Valid RED failure | PASS | Remediation DES RED event is `EXECUTED/PASS`. |
| G3 — Assertion failure | PASS | Remediation DES RED event is `EXECUTED/PASS`. |
| G4 — No domain mocks | PASS | No domain mock was introduced; real kernel state is used at the adapter boundary. |
| G5 — Business language | PASS | Test names and assertions describe persistent TAP convergence, drift repair, collision refusal, and teardown. |
| G6 — All green | PASS for current tree | Focused and full affected-package Lima suites passed. |
| G7 — 100% passing before commit | PASS | Remediation DES COMMIT event is `EXECUTED/PASS`. |
| G8 — Test budget | PASS | 7 tests ≤ budget 12. |
| G9 — No test modification to accommodate implementation | PASS | Existing assertions were strengthened, not weakened. |
| Adapter real-I/O mandate | **FAIL / BLOCKER** | Broad SUT-error returns let the real-I/O scenarios pass without reaching their kernel assertions. |

### Iteration-2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 1 |
| **Total** | **2** |

### Iteration-2 final verdict

**REJECTED**

D1, D2, D3, D5, and D6 are resolved, and the current remediation behavior succeeded against a real Lima kernel. D4 remains blocked because the committed real-I/O tests can silently return green on the adapter failures they exist to detect, and D7 leaves the remediated errno contract inaccurately documented. This is the maximum second reviewer iteration; escalate the unresolved blocker to the human facilitator before any further remediation or advancement to roadmap step 02-01.

## Iteration 3

- **Review ID:** `code_rev_20260828_124401_iteration_3`
- **Reviewer:** isolated step 01-02 reviewer
- **Commit:** `675f9a2709907fb8deb2ebd99e20abca4bea766a`
- **Parent:** `5523e4a7981e972fc1a62468948722c63c5a7b19`
- **Subject:** `fix(guest-stack-mtls): enforce real-kernel test failures`
- **Trailer:** `Step-Id: 01-02`
- **Verdict:** **APPROVED**

### Executive summary

The iteration-3 remediation resolves both findings remaining from iteration 2. The two C3 real-kernel scenarios now skip only on the explicit pre-SUT `!is_root()` environment gate; once running as root, provisioning and adapter failures fail the test instead of returning green. Both scenarios also assert terminal absence of the network namespace, host veth, host-namespace TAP, guest route, and allocation slot. The stale `nl_set_up` rustdoc now accurately documents fail-closed handling for every returned error, including `ENODEV`.

Independent execution in the canonical Lima lane reached both tests' `EXECUTED` markers and passed. Formatting, compilation, linting, the full affected-package suite, diff hygiene, scope, test integrity, Contract Shape declarations, and the DES remediation triplet all pass. No new defect was found.

### Prior-finding disposition

| Finding | Status | Evidence |
|---|---|---|
| D4 — real-netns tests could silently skip SUT failures and omitted host TAP absence | **RESOLVED** | Both scenarios retain only the pre-SUT `!is_root()` return. The positive path requires `Running`; the collision baseline uses `expect`, so unexpected provisioning failures fail. Both terminal paths explicitly verify netns, host veth, host TAP, route, and slot absence. |
| D7 — `nl_set_up` rustdoc contradicted fail-closed behavior | **RESOLVED** | The rustdoc now says setting an existing link up is kernel-idempotent and every returned error, including `ENODEV`, is surfaced fail-closed, matching the implementation. |

D1 through D7 are now resolved.

### Scope and test-integrity review

| Check | Status | Evidence |
|---|---|---|
| Commit scope | PASS | Three intended files changed: the veth provisioner documentation, the real-netns integration test, and the execution log. |
| Diff hygiene | PASS | `git diff --check 5523e4a7… 675f9a27…` passed. |
| Existing-test integrity | PASS | The remediation removes broad always-green returns and adds terminal assertions; it does not delete, skip, or weaken test behavior. |
| New-test integrity | PASS | Root-lane SUT failures now surface as failures. The sole remaining return is a narrow, explicit environment-only root precondition before SUT execution. |
| Real-I/O residue coverage | PASS | Both C3 scenarios assert absence of netns, host veth, host-namespace TAP, guest route, and allocation slot. |
| Contract Shape | PASS | Required declarations remain present and unchanged; no new live property was introduced by this remediation. |
| Test budget | PASS | 7 tests remain within the 12-test budget for 6 roadmap behaviors. |
| Regression review | PASS | No new production behavior, test-theater path, weakened assertion, or unrelated change was introduced. |

### Remediation DES phase triplet — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T12:39:32Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T12:41:41Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T12:41:55Z` |

The iteration-3 triplet is complete, successful, and chronologically ordered. The commit timestamp, `2026-08-28T12:42:02Z`, follows the COMMIT event.

### Iteration-3 verification

| Verification | Result |
|---|---|
| `git diff --check 5523e4a7981e972fc1a62468948722c63c5a7b19 675f9a2709907fb8deb2ebd99e20abca4bea766a` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check -p overdrive-netlink -p overdrive-control-plane --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-netlink -p overdrive-control-plane --all-targets --features integration-tests -- -D warnings` | PASS |
| Two focused real-netns C3 scenarios with `--no-capture` | PASS — 2 passed, 0 failed; both emitted their `EXECUTED` marker, proving the environment gate was not taken |
| Full affected-package Lima suite with `integration-tests` | PASS — 817 passed, 0 failed, 3 skipped |
| Mutation testing | NOT RUN — explicitly prohibited during individual roadmap steps |

### Iteration-3 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **0** |

### Iteration-3 final verdict

**APPROVED**

The iteration-3 remediation is complete and introduces no new finding. Step 01-02 may advance after the orchestrator completes its mechanical evidence checks.
