# RED classification — Service-kind VM workloads

## ADR-0101 revision-3 behavioral regression handoff — 2026-09-08

This subsection is the current scoped amendment; the original-wave scaffold
history below is not its execution status. No production code or diagnostic
invariant was changed. Full regressions remain intentionally failing and
uncommitted until implementation; no broad `should_panic` or arbitrary failure
is accepted as successful execution.

### Consolidated-review oracle remediation — CD-0101-01/02

Only `service_backend_projection.rs` and its scoped DISTILL documentation were
changed for these two findings. The acceptance suite remains 12 tests; the
paired source-local property, original diagnostic and production APIs/behavior
are unchanged. Independent re-review is pending, not self-approved.

The exact composed-suite command below was rerun as Nextest
`d800e94f-1417-4113-a041-1b71a4c539de`, then after test-only initializer/lint
cleanup as final run `4bc1f645-5589-4531-96c9-45c68a2726c6`: **12 run,
2 passed, 10 behavioral failures, 24 skipped**. Final duration 0.477 seconds;
inner nextest exit **100**, outer xtask exit **1**. No compilation/setup error.
All first failure categories remain those in the scenario table below.

BE-07 now executes its exact stamp/VIP assertions **before** the existing
pair-count RED. Its production-authored baseline is unchanged across normal
runtime/View registration. The three executed stamp cases are:

| Existing input boundary | Tick supplied to reconciliation | Observed same-key prior counter | Exact planned counter / writer |
|---|---:|---:|---|
| First backend publication | 12 | absent | 13 / AppState node `local` |
| Deciding probe failure before reload | 15 | 13 | 16 / AppState node `local` |
| Same hydrated failure after existing process-local tick reset | 1 | 13 | 14 / AppState node `local` |

Each planned write is compared with
`LogicalTimestamp::dominating(tick, state.node_id.clone(), observed_same_key_stamp)`.
The high-prior row is the real earlier dispatch output, never injected or
rewritten. Explicit prior/tick inequalities and a nonzero planned-write count
prove the cases are reached. The third case then fails at **1 legacy write
versus 4 required write/handoff actions** before FinalizeFailed. Complete-pair,
correlation and subsequent serial-dispatch suffix assertions remain authored,
not exercised; these exact stamp controls do not turn BE-07 green.

The fixture now checks every backend-row event and observed row against the
VIP retained directly from allocator setup, independent of published fields.
The two green controls and every reached withdrawal therefore execute this
VIP oracle. BE-03's explicit empty row is still missing; its empty-row VIP
assertion remains pending publication, not falsely counted as green evidence.
BE-07's correlation and BE-10's local-map key expectations now derive from
the retained allocator VIP. BE-10 still stops before its missing hydrator
handoff, so the local-map suffix remains unexecuted.

The unchanged original diagnostic command below was also repeated as Nextest
`0b7ed041-1979-45ff-9b0a-0be4c50dc7a3`: **1 run, 1 behavioral failure**,
0.029 seconds; inner 100 / outer 1. Seed **257209** still reaches the same
retained terminal-veto failure after actual same-ID restart.

Scoped rustfmt and `git diff --check` pass. The diagnostic-only clippy command
with `--cap-lints warn` was rerun after fixing the new test initializer's
`useless_let_if_seq` warning; final exit 0, **no warning in the changed test
file**. The baseline production/exit-event warnings described below remain;
this is not a strict-clippy success. No production, DESIGN, review artifact,
native, mutation, DES, or commit work was performed for this remediation.

### Unchanged original reachability witness

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test e09_v2_failed_service_reachability_spike --no-capture
```

Nextest `8bfffae9-b2a3-4d65-b75c-b1596f3e59c7`: **1 run, 0 passed, 1 failed**;
nextest exit 100, outer xtask exit 1. Seed **257209** reached the healthy
baseline, actual startup Failed (three attempts), actual same-ID restart, and
the retained terminal veto. Six subsequent ServiceLifecycle ticks left the
backend healthy. The original final assertion failed:
`stored backend regained eligibility despite unchanged ServiceLifecycle terminal veto`.
Classification: **BEHAVIORAL_RED / MISSING_FUNCTIONALITY**, not setup or compile
failure. The original file was preserved byte-for-byte during this task.

### New composed regression set

```text
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(service_backend_projection)' --no-capture --no-fail-fast
```

Nextest `c971a51a-48bd-485c-8aed-61d1c482beec`: **12 run, 2 passed, 10 failed,
24 skipped**; nextest exit 100, outer xtask exit 1; 0.395 seconds executing
tests after compilation. All ten failures are behavioral assertions after
legitimate setup, not compile/import/setup failure.

After test-only formatting/lint cleanup, repeat run
`5fa4578e-dcd3-4f4f-bf86-3710e8a06b36` reproduced the identical **2 green / 10
behavioral RED** split (0.429 seconds; nextest 100 / outer 1), with identical
domain failure points and printed seeds.

Final authored-tree rerun `b7177476-0b90-4fe2-b807-4c37c81cf11c` again reports
**12 run: 2 passed, 10 behavioral failures, 24 skipped**, 0.350 seconds,
nextest 100 / outer 1. No compile warning or setup failure occurred in this
final nextest run.

| Scenario | Current observed result | Classification |
|---|---|---|
| BE-01 | Single-listener preterminal health and exact repeat equality | GREEN control |
| BE-02 | Single-listener control passes; three-listener case stores only one ServiceId | Behavioral RED |
| BE-03 | Missing-allocator control passes; assigned listener with no alloc has no explicit empty row | Behavioral RED |
| BE-04 | Exact write refusal observed; three later ticks still leave row absent | Behavioral RED |
| BE-05 | Real Failed achieved; non-Running membership remains in stored backend vector | Behavioral RED |
| BE-06 | Exact rejected withdrawal, independently drained Failed, unchanged normal redb View reload, and same-ID Running all verified; repair still leaves healthy=true | Behavioral RED |
| BE-07 | Exact initial/tick-ahead/prior-ahead stamps and allocator VIP pass; real deciding action vector has one legacy write instead of two complete write/handoff pairs before FinalizeFailed | Behavioral RED |
| BE-08 | Single-listener threshold/reset/recovery control passes; three-listener case stores one row | Behavioral RED |
| BE-09 | Complete healthy -> actual readiness Fail -> Pass List/Watch trajectory passes through real resolver and DNS projection | GREEN control |
| BE-10 | Healthy resolver/DNS baseline passes; no real hydrator handoff is queued | Behavioral RED |
| BE-11 | Start/SVID control passes; operator Stop has SVID but no ServiceLifecycle wake | Behavioral RED |
| BE-12 | Five normal liveness restarts and typed final failure reached; finalization has SVID but no ServiceLifecycle wake | Behavioral RED |

Assertions after these first failing points remain authored, not yet exercised.
In particular, do not claim BE-10's downstream local-map suffix passed.

### Paired pure property

```text
cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --lib -E 'test(backend_policy_preserves)' --no-capture
```

Nextest `c1ae9c52-6357-41cd-8ba3-675a3d9d78b8`: **1 passed, 38 skipped**,
exit 0. Fixed proptest seed 257222, 128 cases, every finite veto/readiness/status
combination per case. This is preservation evidence for the unchanged pure
policy function, not proof that complete-row projection is implemented.

After the bounded integer conversion cleanup, final property run
`e3a638a8-7bbd-406b-a8d6-d8853ca1495c` again reports **1 passed, 38 skipped**,
exit 0.

### Non-behavioral checks and rejected intermediate evidence

Initial authoring attempts exposed wrong test API spelling, a zero-listener
declaration rejected by existing admission, a probe schedule that had not yet
advanced its first interval, and a fault mistakenly consumed by an earlier
Stable write. These were test-fixture errors, **BROKEN_INTERMEDIATE**, never
accepted RED. They were corrected using current APIs, actual probe clock
advancement, and a reachable pre-Stable backend-write site. No production seam
or acceptance criterion was changed to accommodate them.

Strict scoped clippy was attempted with
`cargo xtask lima run -- cargo clippy -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -- -D warnings`.
It stops at pre-existing production `service_lifecycle.rs:498`
`clippy::too_many_lines` (107/100). A diagnostic-only command allowance for
that lint then reaches pre-existing `action_shim/mod.rs:1403`
`clippy::single_match_else`. Both runs exit outer 1 / inner 101. No production
lint suppression or unrelated refactor was added, and no lint-clean claim is
made. No native-metal, full-100, mutation, or DES run was performed.

A further diagnostic allowance for those two existing lints reached existing
`large_futures` warnings in the unrelated Sim exit-event invariant/harness.
Diagnostic `--cap-lints warn` was used solely to inspect the new test's own
warnings; its successful compiler exit is not a strict-clippy success. Test
literal formatting, a redundant borrow/closure, bounded conversion and large
consumer-helper future allocation were corrected. Test-local lint expectations
explain the required literal Contract Shape token and the intentionally
current-thread, Send-but-not-Sync observation subscription. No production lint
or repository lint configuration was changed.

The final diagnostic clippy pass reports no warning in the new
`service_backend_projection.rs` file; its integration-binary warning is the
pre-existing `exit_event_observable_outcome.rs:36` large future. Strict clippy
remains blocked by the baseline warnings above. Scoped rustfmt check and
`git diff --check` pass.

Full scenario, scope, safety/convergence and completeness details:
[ADR-0101 acceptance design](adr-0101-acceptance.md).

---

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
