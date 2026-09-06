# RED classification — Service-kind VM workloads

**Feature:** `service-kind-vm-workloads` (GH #257)
**Design base:** `8c89c71eee2`

## Governing repository contract

All 24 Rust functions use the only sanctioned repository RED shape from
`.claude/rules/testing.md` § “RED scaffolds and intentionally-failing commits”:

```rust
#[test]
#[should_panic(expected = "RED scaffold")]
fn scenario() {
    panic!("Not yet implemented -- RED scaffold (<scenario-id> / <contract>)");
}
```

Nextest reports these scaffolds as PASS because it exercised and matched the
exact pending marker. That is the repository's machine-checkable,
hook-compatible RED signal; it is not a claim that product behavior exists.
During GREEN, DELIVER removes the attribute and marker together and replaces
the body with the port-observable assertions named in `test-scenarios.md`.
A compile/import/setup failure, unexpected panic, missing marker, or test that
passes after its marker is removed without real assertions is `BROKEN`.

| Scenarios | Artifact | Expected hook-compatible RED | Classification |
|---|---|---|---|
| S-SVM-02..09 | `overdrive-core/tests/acceptance/service_kind_vm_workloads.rs` | 8 exact marker matches | `RED_SCAFFOLD_PRESENT / MISSING_FUNCTIONALITY` |
| S-SVM-10..16 | `overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs` | 7 exact marker matches | `RED_SCAFFOLD_PRESENT / MISSING_FUNCTIONALITY` |
| S-SVM-17 | `overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs` | 1 exact fixed-seed marker match | `RED_SCAFFOLD_PRESENT / MISSING_FUNCTIONALITY` |
| S-SVM-18..21B | `overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs` | 5 exact marker matches | `RED_SCAFFOLD_PRESENT / MISSING_FUNCTIONALITY` |
| S-SVM-22 | `overdrive-control-plane/tests/acceptance/service_kind_vm_workloads.rs` | 1 exact marker match | `RED_SCAFFOLD_PRESENT / MISSING_FUNCTIONALITY` |
| S-SVM-23..24 | `overdrive-cli/tests/acceptance/service_kind_vm_workloads.rs` | 2 exact marker matches | `RED_SCAFFOLD_PRESENT / MISSING_FUNCTIONALITY` |
| S-SVM-01, S-SVM-25, S-SVM-26, S-SVM-27A/B/C, S-SVM-28, S-SVM-29 | E08–E13 | each runner exits 75 from its checked-in example mode | `PENDING / MISSING_FUNCTIONALITY` |

S-SVM-17 deliberately asserts only the approved seeded observable invariant:
terminal state wins and a dead backend never returns to eligibility. It does
not require ProbeRunner drain/join, tombstones, write suppression, or a new
production seam. If the existing simulation boundaries cannot express the
invariant during DELIVER, that is a DESIGN testability gap rather than licence
to add API.

## Authoritative native-metal execution

The original 23-scaffold selection ran as nextest ID
`7ae2e61d-17e9-4053-80be-1999bcdc156a`: **23 passed, 1079 skipped**. Each PASS
was the required exact `should_panic` marker match. This is authoritative
hook-compatible RED evidence for the original set.

After all review remediation, the repository's exact five-package command was
rerun unchanged as nextest ID `f166d853-38c7-4119-83f8-736e61987396`:
**23 passed, 1079 skipped**. This current-tree run covers the five-package
selection while the added seeded simulation scaffold is covered by the
expanded run below.

After review split liveness-stop from later restart ownership and moved
terminal authority to its required seeded `overdrive-sim` home, the current
24-scaffold set was executed with:

```text
OVERDRIVE_METAL_KERNEL=/srv/vm/overdrive-testing/kernel OVERDRIVE_METAL_ROOTFS=/srv/vm/overdrive-testing/rootfs.ext4 cargo xtask metal run -- cargo nextest run -p overdrive-core -p overdrive-worker -p overdrive-reconcilers -p overdrive-control-plane -p overdrive-cli -p overdrive-sim --test acceptance -E 'test(service_kind_vm_workloads) or test(service_kind_vm_terminal_invariant)'
```

Nextest run `d2e943f1-54e3-4528-96b3-75dd90088214` compiled all six test
binaries and reported **24 tests run: 24 passed, 1175 skipped**. There were no
compile, import, fixture, setup, or unexpected-panic failures. Final Rust
classification: **24/24 exact hook-compatible RED marker matches; 0 BROKEN**.

After the seven peer callers were transitioned from host Exec Jobs to VM Jobs,
the materially changed example/spec source contract was rechecked on qualified
native metal in the same lease as the unchanged scoped RED selection. The
bundle's `prepare.sh check-source` passed; `guest_server.rs` and `client.rs`
both compiled for `x86_64-unknown-linux-musl`, `file` identified both as
stripped static-PIE binaries, and `readelf` found no dynamic interpreter.
Nextest run `d26c7832-c2f1-40d5-96f9-86e8e8a9d867` then reported **24 tests
run: 24 passed, 1175 skipped** across the same six acceptance binaries. No Rust
scaffold declaration, scenario ID, or production implementation changed.

A separate token-owned native-metal lifecycle check then executed
`prepare.sh prepare`, `prepare.sh check`, and `prepare.sh cleanup`. The check
remounted the private image and verified both guest-installed static binaries;
cleanup detached the mount/loop state, removed the exact marker-owned
`/srv/vm/overdrive-testing/svm-e08` tree, and left that path absent.

## Rejected intermediate evidence

A local Lima attempt never exposed guest SSH and is retained only as
`NOT_EXECUTED / TEST_SUBSTRATE_UNAVAILABLE`; it is not the final result.

During reconciliation, the 23 attributes were temporarily removed. Runs
`e5c94fd9-ea58-4944-80b9-3c02b936aedd` (16 failures before fail-fast) and
`dc41adb0-4d7b-4cd8-9e43-23b24ad70585` (23 failures) observed the bare marker
panics. That form is explicitly deprecated by the repository and those runs
are `REJECTED_NONCOMPLIANT_INTERMEDIATE`, not authoritative RED evidence.

## Built-product pending boundary

E08 is the sole walking skeleton. E09–E13 are bounded non-walking-skeleton
modes for the accepted TCP failure/K1, HTTP/K2, readiness/K3, liveness, and
zero-declared-probe journeys. All six runners validate checked-in sources and
exit exactly 75 until DELIVER activates the built default-feature binary path.
The E08/E09/E11/E13 peer callers are seven checked-in `[job] + [vm]` specs
whose static client binary is installed in the private guest image by the one
E07-style `examples/service-kind-vm-workloads/prepare.sh` path. E10 retains its
four `[service] + [exec]` fixtures solely as cross-driver HTTP controls. No
expectation invokes a Rust test, imports an Overdrive crate, recreates a spec
inline, or treats direct host-to-guest reachability as Service traffic.
