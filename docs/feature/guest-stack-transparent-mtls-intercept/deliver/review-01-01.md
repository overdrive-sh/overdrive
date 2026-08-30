# Adversarial review — step 01-01

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `01-01` — pure `VmTapPlan`
- **Reviewer:** `nw-software-crafter-reviewer`
- **Initial commit:** `003df158`
- **Remediation commit:** `6b9ffd2e1dc429b2846b2f22f30ccf4dbcd290c1`
- **Final verdict:** **APPROVED**

## Executive summary

The initial implementation correctly derived collision-free TAP names, disjoint
guest `/30` addressing, and locally administered NIC identities. Its three
source-local properties passed, but all three omitted the mandatory exact
Contract Shape declaration. Review iteration 1 therefore required revision.
The remediation added only the three declarations, preserved behavior and test
assertions, and passed re-review.

## Iteration 1 — NEEDS_REVISION

### D1 — Blocking: missing Contract Shape declarations

- **Severity:** blocker
- **Dimension:** Contract Shape compliance
- **Location:** `crates/overdrive-control-plane/src/veth_provisioner.rs`
- **Finding:** The three new source-local pure-function property tests did not
  carry the exact declaration `/// CONTRACT_SHAPE: pure-function.`
- **Required remediation:** Add the exact declaration to every live property
  without changing production or test behavior.

The affected properties were:

1. `each_microvm_slot_names_its_own_tap_device_collision_free`
2. `each_microvm_slot_owns_a_mesh_address_disjoint_from_its_transit_hop`
3. `each_microvm_slot_carries_its_own_locally_administered_nic_identity`

### Iteration-1 verification

| Check | Result |
|---|---|
| Focused properties | 3 passed, 0 failed |
| Mutation signal available at review time | 10/10 mutants caught (100%) |
| Blocking defects | 1 |

Functional behavior was sound, but the mandatory per-test declarations blocked
approval independently of the green suite.

## Iteration 2 — APPROVED

- **Review ID:** `code_rev_20260828_105248_iteration_2`
- **Artifact:** remediation commit `6b9ffd2e`
- **Verdict:** **APPROVED**

### D1 disposition — RESOLVED

| Location | Declaration |
|---|---|
| `crates/overdrive-control-plane/src/veth_provisioner.rs:4580` | `CONTRACT_SHAPE: pure-function` |
| `crates/overdrive-control-plane/src/veth_provisioner.rs:4615` | `CONTRACT_SHAPE: pure-function` |
| `crates/overdrive-control-plane/src/veth_provisioner.rs:4668` | `CONTRACT_SHAPE: pure-function` |

### Contract Shape compliance

| Check | Result |
|---|---|
| Exact declaration on all three live properties | PASS |
| Banned test-name regex | PASS |
| Outcome anchor | Not applicable — source-local pure-function properties |
| Layer choice | PASS — design-sanctioned pure-function driving port |
| Preservation or delta assertions | Not applicable |

### Regression review

The remediation changed exactly one owned file and added three rustdoc lines.
It changed no executable production or test code, weakened no assertion, and
deleted or skipped no test.

### Iteration-2 verification

| Check | Result |
|---|---|
| Remediation diff | PASS |
| Focused Lima tests | 3 passed, 0 failed |
| Production behavior drift | None |
| New findings | None |

## Final defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

## Final approval

**APPROVED.** The sole iteration-1 blocker was fully resolved. The remediation
is scope-clean and comment-only, preserves all non-vacuous property assertions,
and leaves the focused Lima suite green.
