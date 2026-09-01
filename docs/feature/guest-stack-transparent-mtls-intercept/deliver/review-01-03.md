# Adversarial review — step 01-03

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `01-03` — VM launch network attachment and in-guest addressing
- **Reviewer:** `nw-software-crafter-reviewer` (fresh isolated adversarial reviewer)
- **Review ID:** `code_rev_20260828_133247_iteration_1`
- **Iteration:** 1
- **Commit:** `0844d182d020fdefad15129776ec5d64685c1a64`
- **Parent:** `675f9a2709907fb8deb2ebd99e20abca4bea766a`
- **Subject:** `feat(guest-stack-transparent-mtls-intercept): wire guest VM networking`
- **Trailer:** `Step-Id: 01-03`
- **Final verdict:** **NEEDS_REVISION**

## Executive summary

The commit is mechanically clean, compiles and lints in Lima for the affected packages, checks `overdrive-init` for both production musl targets, and passes all eight focused tests. The host launch prefix, atomic `VmConfig` attachment, one-token cmdline composition, exact ioctl sequence, fail-closed pre-EXEC branch, and unchanged Beacon published language are present in source.

The step cannot be approved because the new PID-1 path cannot run in the repository's actual minimal guest rootfs: it reads unmounted `/proc`, enumerates unmounted `/sys`, and writes beneath a missing `/etc`. A mesh guest therefore reports `EXIT 78` and powers off before executing the workload. The new `ip` launch dependency is also absent from the VMM capability probe and is misreported as a missing `prlimit` wrapper if spawn fails. Remediate D1–D3 and return this step's original crafter output to this reviewer for iteration 2.

## Contract Shape Compliance

**Overall: PASS**

| Check | Status | Evidence |
|---|---|---|
| Exact per-test declarations | PASS | All 8 new source-local tests carry the exact rustdoc line `/// CONTRACT_SHAPE: pure-function.`. |
| Semantic contract match | PASS | The tests transform owned/local values or inspect a constructed command without external I/O. |
| Outcome anchor | NOT APPLICABLE | No new acceptance-test artifact was authored in this step; the roadmap explicitly defers the guest-boot outcome to the metal lane. |
| Banned test-name regex | PASS | No new test name matches the repository's banned regex. |
| Preservation or delta checks | PASS | The complete/non-mesh composition tests assert the full attachment and exact cmdline token; the VMM test asserts the ordered argv and unchanged non-mesh shape. |
| Layer choice | PASS WITH DEFERRED EXTERNAL PROOF | The current step's roadmap permits compile plus argv/cmdline unit proof and defers the real guest boot; D1 is a production external-validity defect found by reading that deferred path, not a false Contract Shape declaration. |

## Mechanical evidence

### Commit scope — PASS

- Stat: **8 files changed, 792 insertions, 31 deletions**.
- Behavioral edits are confined to the four roadmap-owned production files.
- Three host integration fixtures contain only compiler-required neutral `VmConfig` literal fallout (`network: None`).
- The execution log is the expected DELIVER artifact.
- No mutation exclusions were changed.
- Pre-existing dirty `roadmap.json`, `AGENTS.md`, and prior review artifacts were excluded from this commit review and preserved.

### DES phase order — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T13:04:49Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T13:09:48Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T13:19:22Z` |

All three canonical phases are present, successful, and chronologically ordered. The commit timestamp, `2026-08-28T13:19:28Z`, follows the logged COMMIT phase.

### Test budget — PASS

| Behaviors | Budget (`2 × behaviors`) | Actual new tests |
|---:|---:|---:|
| 5 | 10 | 8 |

### Test-integrity diff — PASS

The parent-to-step diff adds eight tests and does not weaken, delete, skip, or reduce assertions in a pre-existing test. The three existing integration-fixture edits only initialize the folded network attachment to `None`.

## Blocking findings

### D1 — PID 1 assumes filesystems and a directory that the real guest rootfs does not provide

- **Severity:** Critical
- **Dimension:** External validity and production-path correctness
- **Locations:**
  - `crates/overdrive-init/src/main.rs:321`
  - `crates/overdrive-init/src/main.rs:363`
  - `crates/overdrive-init/src/main.rs:369`
  - `crates/overdrive-testing/src/vm_fixture.rs:756`
  - `crates/overdrive-testing/src/vm_fixture.rs:784`
  - `crates/overdrive-cli/tests/integration/vm_resources_sizing.rs:85`

**Evidence:** `configure_guest_network` first reads `/proc/cmdline`, `single_non_loopback_interface` later reads `/sys/class/net`, and `apply_guest_network` writes `/etc/resolv.conf`. The actual guest fixture creates only empty `proc` and `sys` mountpoints and never mounts either filesystem; it does not create `etc` at all. The established fixture documentation explicitly says “the minimal rootfs mounts none” for `/proc`. The spike-proven NIC discovery avoided sysfs and used `if_nameindex(3)`.

The failure is on the production entry path, not merely in a test harness. `run` sends `READY`, calls this setup before receiving `EXEC`, then sends `EXIT 78` and returns on any setup error. On every mesh boot against the repository's rootfs, the first `/proc/cmdline` read fails with `ENOENT`; even if that were bypassed, `/sys/class/net` and `/etc/resolv.conf` fail next. The workload therefore never executes.

**Required remediation:** Make the PID-1 path self-sufficient for the rootfs it actually boots. Mount procfs before reading the kernel cmdline; either mount sysfs or use the spike-proven `if_nameindex`/a netlink observer for interface discovery; and create or otherwise guarantee `/etc` before writing `resolv.conf`. Keep each failure typed and preserve the existing non-zero `EXIT`/never-EXEC behavior. Do not claim the deferred metal scenario in this step, but ensure the implementation is compatible with the known minimal-rootfs contract so the later metal gate can execute it.

### D2 — The new `ip` executable is neither probed nor reported honestly on spawn failure

- **Severity:** High
- **Dimension:** Capability probing and error honesty
- **Locations:**
  - `crates/overdrive-host/src/vmm.rs:198`
  - `crates/overdrive-host/src/vmm.rs:239`
  - `crates/overdrive-host/src/vmm.rs:370`
  - `crates/overdrive-host/src/vmm.rs:390`
  - `crates/overdrive-host/src/vmm.rs:746`

**Evidence:** A networked VM changes the actual spawned executable from `prlimit` to `ip`, but `probe_confinement_toolchain` still checks only `prlimit` and `setpriv`. If `ip` is absent, the node passes `Vmm::probe`, then the first mesh allocation fails at `cmd.spawn()`. That failure is classified as `ConfinementUnavailable { UidDrop }` with detail `confinement wrapper prlimit not found`, even though `prlimit` was never the failed executable. The adjacent lifecycle comments also still assert that `argv[0]` is always `prlimit`.

**Required remediation:** Include the new launch executable in boot-time capability validation with an honest typed diagnostic, and make spawn-error attribution use the executable actually passed to `Command::new`. Preserve the existing non-networked classification and update the stale launch-chain comments. Add an oracle for both mesh and non-mesh attribution so a missing `ip` cannot be reported as a missing UID-drop wrapper.

### D3 — The init dependency documentation falsely claims crate-wide unsafe prohibition remains intact

- **Severity:** Low
- **Dimension:** RPP L1 documentation accuracy
- **Location:** `crates/overdrive-init/Cargo.toml:56`

**Evidence:** The dependency comment says `nix` is used only through safe wrappers and that `#![forbid(unsafe_code)]` “stays intact.” This commit changes the crate to `#![deny(unsafe_code)]` and adds narrowly allowed raw ioctl calls. The source scopes and justifies those calls correctly, but the package manifest now gives reviewers and operators the opposite safety model.

**Required remediation:** Update the dependency comment to describe the enabled ioctl macros and the crate's deny-by-default, locally justified unsafe boundary.

## External validity

**Status: FAIL**

The VM branch is wired through the production composer and produces the required ordered host argv and a single platform-owned cmdline token. The source-level guest lifecycle is fail closed and leaves the Beacon enum/parser unchanged. However, the reachable guest entry path is incompatible with the exact minimal rootfs used by the real boot fixture, so the implementation cannot currently reach the required ioctl/resolver postcondition or operator exec. The roadmap's honest metal deferral does not turn that known incompatibility into a valid implementation.

## Verification

| Verification | Result |
|---|---|
| `git diff --check 675f9a2709907fb8deb2ebd99e20abca4bea766a 0844d182d020fdefad15129776ec5d64685c1a64` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests -- -D warnings` | PASS |
| Focused Lima `nextest` run for all 8 new tests | PASS — 8 passed, 0 failed, 971 skipped |
| `cargo xtask lima run -- cargo check -p overdrive-init --target x86_64-unknown-linux-musl` | PASS |
| Lima aarch64-musl check with `clang --target=aarch64-unknown-linux-musl` supplied to `cc-rs` | PASS |
| Host unsafe scan | PASS — `overdrive-host` retains `#![forbid(unsafe_code)]`; no host unsafe block was added |
| Beacon published-language diff | PASS — no `BeaconMessage` variant or parser/formatter change |
| Minimal-rootfs path audit | FAIL — procfs/sysfs are unmounted and `/etc` is absent, while the new init path requires all three |
| Metal/KVM guest boot | NOT RUN — explicitly deferred by roadmap step 01-03 |
| Mutation testing | NOT RUN — explicitly prohibited during individual roadmap steps |

The focused test command was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init -E 'test(kernel_cmdline_appends_one_space_free_platform_token_and_rejects_whitespace) | test(mesh_vm_launch_enters_its_netns_before_the_existing_wrapper_and_attaches_its_tap) | test(guest_network_parser_reads_the_platforms_single_space_free_token) | test(guest_network_parser_rejects_partial_or_duplicate_platform_tokens) | test(guest_network_parser_leaves_non_mesh_cmdlines_unchanged) | test(complete_mesh_network_inputs_become_one_attachment_and_one_guest_addressing_token) | test(incomplete_mesh_network_inputs_are_rejected_before_vm_provisioning) | test(non_mesh_vm_keeps_the_platform_default_cmdline_and_has_no_attachment)'
```

The aarch64 check used:

```text
cargo xtask lima run -- env CC_aarch64_unknown_linux_musl=clang CFLAGS_aarch64_unknown_linux_musl=--target=aarch64-unknown-linux-musl cargo check -p overdrive-init --target aarch64-unknown-linux-musl
```

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — Exactly one acceptance active | PASS | The roadmap's walking-skeleton/metal-deferred override applies. |
| G2 — Valid RED failure | PASS | Ordered DES RED event is `EXECUTED/PASS`. |
| G3 — Assertion failure | PASS | Ordered DES RED event is `EXECUTED/PASS`. |
| G4 — No domain mocks | PASS | No mocks were introduced in the eight new tests. |
| G5 — Business language | PASS | Test names and assertions state cmdline, attachment, argv, parser, and fail-closed composition outcomes. |
| G6 — All green | PASS | Relevant compile, lint, cross-target check, and focused-test lanes passed. |
| G7 — 100% passing before commit | PASS | DES COMMIT event is `EXECUTED/PASS`. |
| G8 — Test budget | PASS | 8 tests ≤ budget 10. |
| G9 — No test modification | PASS | No pre-existing assertion was weakened, deleted, or skipped. |

These mechanical gates do not cure D1's production external-validity failure or D2's dishonest runtime error path.

## Test integrity and RPP scan

- **Test modification detected:** No.
- **Testing theater detected:** No. The eight tests have non-vacuous oracles; their green result is narrower than the deferred real-guest contract but not an always-green or mock-dominated shape.
- **Escalation verification:** Not applicable.
- **RPP levels scanned:** L1.
- **Cascade stopped at:** L1, after the stale manifest safety comment in D3.
- **RPP findings:** D3.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 1 |
| High | 1 |
| Medium | 0 |
| Low | 1 |
| **Total** | **3** |

## Final verdict

**NEEDS_REVISION**

The implementation is not eligible to advance to the next roadmap step. D1–D3 must be remediated by the original step 01-03 crafter, after which this same reviewer should perform iteration 2.

## Iteration 2

- **Review ID:** `code_rev_20260828_135906_iteration_2`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated reviewer as iteration 1)
- **Remediation commit:** `32f136743902341b6e00a8d950ea2966ba303605`
- **Parent:** `0844d182d020fdefad15129776ec5d64685c1a64`
- **Subject:** `fix(guest-stack-transparent-mtls-intercept): bootstrap guest networking`
- **Trailer:** `Step-Id: 01-03`
- **Iteration-2 verdict:** **REJECTED**

### Iteration-2 executive summary

The remediation fixes the production-breaking minimal-rootfs path from D1: PID 1 creates `/proc` and `/etc`, mounts procfs before reading the kernel cmdline, discovers the NIC through the spike-proven `if_nameindex` path without sysfs, and retains the `EXIT 78`/never-exec failure ordering. It also fixes the runtime half of D2 by probing `ip`, attributing an immediate mesh spawn failure to `ip`, and preserving the established non-mesh `prlimit` classification. D3's unsafe documentation is accurate now. Both musl targets compile, all affected packages check and lint in Lima, and all twelve focused tests pass.

The step is nevertheless rejected. D2's public typed probe error still calls an `ip` failure `ConfinementToolchainAbsent` and says every launch-tool spawn error means “not found,” the step now exceeds its established test budget, the `ip` probe test does not execute or inject the probe path it claims to prove, and the new bounded-change bootstrap test does not assert the declared state complement. These are two blocker-level test-design failures plus two high-severity error/test-oracle defects. The repository's no-iteration-cap rule applies: remediate them and return this same step to this reviewer for iteration 3.

### D1–D3 disposition

| Finding | Disposition | Evidence |
|---|---|---|
| D1 — PID 1 assumes unavailable guest-root paths | **RESOLVED** | `bootstrap_guest_root_at` creates `proc` and `etc`, `mount_procfs_at` mounts procfs before `/proc/cmdline` is read, and `single_non_loopback_interface` now uses the spike-proven `if_nameindex` API with no `/sys/class/net` dependency. `configure_then_exec` is the production `run` path and sends exactly `EXIT 78` before returning an error without invoking the execution closure. |
| D2 — `ip` is neither probed nor honestly attributed | **PARTIALLY RESOLVED — HIGH REMAINS** | `REQUIRED_LAUNCH_TOOLS` includes `ip`; `Vmm::probe` calls `probe_launch_toolchain`; immediate mesh spawn failure names `ip`; non-mesh `NotFound` remains `ConfinementUnavailable { UidDrop }`. The public probe variant and constructor still say `ConfinementToolchainAbsent` for `ip`, and the display says “not found on PATH” for every spawn error, including `PermissionDenied`. See the remaining D2 finding below. |
| D3 — unsafe documentation contradicts the implementation | **RESOLVED** | `overdrive-init/Cargo.toml` now documents mount/interface enumeration, deny-by-default unsafe policy, and the locally justified ioctl boundary; the required `mount` feature is enabled. |

### D2 remaining — The public probe type still misclassifies the namespace launcher as confinement

- **Severity:** High
- **Dimension:** Typed-error honesty and public VMM-port design
- **Locations:**
  - `crates/overdrive-core/src/traits/vmm.rs:445`
  - `crates/overdrive-core/src/traits/vmm.rs:452`
  - `crates/overdrive-core/src/traits/vmm.rs:486`
  - `crates/overdrive-host/src/vmm.rs:765`

**Evidence:** The remediation correctly broadens the prose and host probe to cover `ip`, but the public structural variant remains `VmmProbeError::ConfinementToolchainAbsent` and its constructor remains `confinement_toolchain_absent`. `ip netns exec` is the namespace launcher, not a UID-drop/confinement tool. Consumers matching the typed error therefore receive a false cause even though `Display` was generalized. In addition, `probe_launch_toolchain` maps every `Command::output` spawn failure into text saying the tool was “not found on PATH”; an existing but non-executable `ip` produces `PermissionDenied`, not absence.

**Required remediation:** Complete the tightly bounded VMM-port fallout: use a launch-tool-unavailable variant/constructor whose type does not call `ip` confinement, and make its display preserve “unavailable/could not execute” unless the source is specifically `NotFound`. Keep the runtime `VmmError` mapping already added. Assert the exact typed variant, tool, and diagnostic for both `NotFound` and a non-absence spawn error.

### D4 — The remediation exceeds the established unit-test budget

- **Severity:** Blocker
- **Dimension:** Test budget enforcement
- **Locations:**
  - `crates/overdrive-host/src/vmm.rs:870`
  - `crates/overdrive-init/src/main.rs:766`

**Evidence:** Iteration 1 established five roadmap behaviors and a maximum of ten unit tests. The step had eight tests. This remediation adds four more source-local tests, bringing the step total to twelve. The acceptance behavior count did not change: procfs/bootstrap, `ip` capability, and error attribution are correctness of the same VMM/init criteria, not new stakeholder behaviors. `12 > 2 × 5` is a mandatory budget blocker.

**Required remediation:** Consolidate the related host VMM oracles without dropping behavioral coverage. In particular, the existing launch-shape test can carry the mesh/non-mesh attribution assertions, while a single probe-plan/error test can cover the exact required tools and typed failure. Preserve the two distinct init outcomes and return the total to ten or fewer.

### D5 — The capability-probe test verifies a constant, not the probe path named by the test

- **Severity:** High
- **Dimension:** Implementation-mirroring oracle and external-path coverage
- **Location:** `crates/overdrive-host/src/vmm.rs:876`

**Evidence:** `launch_capability_probe_includes_the_network_namespace_executable` asserts only that the private `REQUIRED_LAUNCH_TOOLS` constant contains `"ip"`. It does not call `probe_launch_toolchain`, inject its command runner, or invoke `Vmm::probe`. Removing `probe_launch_toolchain().await?` from `Vmm::probe`, or making the probe skip the constant, leaves this test green. Its assertion message claims the node “must refuse” a host without `ip`, but no refusal is observed. This is a high-severity implementation-mirroring/misleading-name shape, not one of the instant-rejection always-green theater classes.

**Required remediation:** Exercise a functional-core probe plan or an injected command-runner seam actually consumed by `probe_launch_toolchain`, and assert that an `ip` failure returns the exact typed probe error. Keep the real `Vmm::probe` wiring directly visible and covered through the same plan; do not mutate process-global `PATH` in parallel tests.

### D6 — The bounded-change bootstrap test does not prove its declared complement

- **Severity:** Blocker
- **Dimension:** Contract Shape Compliance
- **Location:** `crates/overdrive-init/src/main.rs:766`

**Evidence:** The declaration promises that an empty minimal root “gains only required bootstrap directories.” The test proves `proc` and `etc` exist and singles out `sys` as absent, but it never enumerates the resulting root. A regression that also creates `tmp`, `dev`, or any arbitrary file remains green, contradicting the bounded-change declaration's “only” complement. This is the same missing-complement class that the repository's Contract Shape gate treats as a blocker.

**Required remediation:** Snapshot or enumerate the root and assert exact equality with `{etc, proc}` after bootstrap, in addition to the mount-target/order assertion. Make cleanup robust against a failed assertion so the test does not leave shared temp state.

### Contract Shape Compliance — FAIL

| Check | Status | Evidence |
|---|---|---|
| Exact pure-function declarations | PASS | Both new pure tests use the exact required `/// CONTRACT_SHAPE: pure-function.` line. |
| Bounded-change declarations | PASS | Both new init tests declare their changed outcome explicitly. |
| Bounded-change delta and complement | FAIL | The minimal-root test asserts the required additions and one named absence, not exact complement equality; D6. |
| Observable failure outcome | PASS | The setup-failure test observes one parsed `BeaconMessage::Exit { status: 78 }` and proves the execution closure was not called. |
| Banned test-name regex | PASS | No new or transitioned test name matches the banned regex. |
| Outcome anchor | NOT APPLICABLE | The real guest boot remains explicitly metal-deferred for this step. |

### Mechanical evidence — PASS except test budget

- Stat: **5 files changed, 271 insertions, 78 deletions**.
- The production changes are limited to the host VMM, guest init, the manifest feature/documentation required by init, and the public `VmmProbeError` documentation required by D2. The execution log is the expected fifth file.
- No unrequested public type or method was added. The `Vmm` trait fallout changes existing error documentation only; D2 identifies the remaining dishonest existing identifier rather than requesting a new abstraction.
- Pre-existing dirty `roadmap.json`, `AGENTS.md`, and review artifacts remain outside the remediation commit.
- No mutation exclusions changed.

#### Remediation DES phase order — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T13:40:53Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T13:46:07Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T13:52:41Z` |

The remediation triplet is complete, successful, ordered, and followed by the commit at `2026-08-28T13:52:41Z`.

#### Test budget — BLOCKER

| Behaviors | Budget (`2 × behaviors`) | Step total after remediation |
|---:|---:|---:|
| 5 | 10 | 12 |

#### Test integrity — PASS

All four remediation tests are additive. No pre-existing test, declaration, assertion, or skip state was weakened, deleted, or relaxed between the iteration-1 commit and the remediation commit.

### External validity — PASS

The production entry path now works against the known minimal-root contract by construction: `run` sends `READY`, enters `configure_then_exec`, bootstraps procfs and `/etc`, reads the platform token, discovers the non-loopback NIC without sysfs, applies the ioctls and resolver, and only then receives/executes the operator command. Any bootstrap, parse, ioctl, route, or resolver failure takes the one `EXIT 78` branch and cannot call exec. The actual KVM boot remains honestly deferred to the roadmap's metal gate; no metal result is claimed here.

The host entry path also invokes the expanded probe and constructs the correct launcher-specific command. D2 is about structural/error vocabulary honesty, not missing production wiring.

### Iteration-2 verification

| Verification | Result |
|---|---|
| `git diff --check 0844d182d020fdefad15129776ec5d64685c1a64 32f136743902341b6e00a8d950ea2966ba303605` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests -- -D warnings` | PASS |
| Focused Lima `nextest` run for all 12 step tests | PASS — 12 passed, 0 failed, 971 skipped |
| `cargo xtask lima run -- cargo check -p overdrive-init --target x86_64-unknown-linux-musl` | PASS |
| Lima aarch64-musl check with `clang --target=aarch64-unknown-linux-musl` supplied to `cc-rs` | PASS |
| Minimal-root source audit | PASS — procfs is mounted before cmdline read; `/etc` is created; NIC enumeration has no sysfs dependency; configuration precedes exec |
| Host unsafe scan | PASS — `overdrive-host` retains `#![forbid(unsafe_code)]` and adds no unsafe block |
| Beacon published-language diff | PASS — no Beacon variant, parser, or formatter changed |
| Metal/KVM guest boot | NOT RUN — explicitly deferred by roadmap step 01-03 |
| Mutation testing | NOT RUN — explicitly prohibited during individual roadmap steps |

The focused test command was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init -E 'test(kernel_cmdline_appends_one_space_free_platform_token_and_rejects_whitespace) | test(mesh_vm_launch_enters_its_netns_before_the_existing_wrapper_and_attaches_its_tap) | test(launch_capability_probe_includes_the_network_namespace_executable) | test(spawn_failure_names_ip_for_mesh_and_keeps_prlimit_confinement_for_non_mesh) | test(minimal_guest_root_bootstrap_creates_proc_and_etc_preconditions) | test(guest_setup_failure_reports_nonzero_exit_and_never_executes_operator_command) | test(guest_network_parser_reads_the_platforms_single_space_free_token) | test(guest_network_parser_rejects_partial_or_duplicate_platform_tokens) | test(guest_network_parser_leaves_non_mesh_cmdlines_unchanged) | test(complete_mesh_network_inputs_become_one_attachment_and_one_guest_addressing_token) | test(incomplete_mesh_network_inputs_are_rejected_before_vm_provisioning) | test(non_mesh_vm_keeps_the_platform_default_cmdline_and_has_no_attachment)'
```

### Iteration-2 quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — Exactly one acceptance active | PASS | The roadmap's walking-skeleton/metal-deferred override remains applicable. |
| G2 — Valid RED failure | PASS | The remediation RED event is `EXECUTED/PASS`. |
| G3 — Assertion failure | PASS | The remediation RED event is `EXECUTED/PASS`. |
| G4 — No domain mocks | PASS | The new seams use closures and real local sockets/filesystem state, not domain mocks. |
| G5 — Business language | PASS | Names state minimal-root, EXIT/no-exec, launch capability, and launcher attribution outcomes. |
| G6 — All green | PASS | Relevant check, lint, cross-target, and focused-test lanes pass. |
| G7 — 100% passing before commit | PASS | The remediation COMMIT event is `EXECUTED/PASS`. |
| G8 — Test budget | **FAIL / BLOCKER** | 12 step tests exceed the established budget of 10. |
| G9 — No test modification | PASS | The remediation is additive and does not weaken prior tests. |

### Iteration-2 test integrity and RPP scan

- **Test modification detected:** No.
- **Testing theater detected:** Yes, high-severity implementation-mirroring/misleading-oracle shape in D5; no zero-assertion, tautological, mock-dominated, or always-green blocker pattern was found.
- **Escalation verification:** Not applicable; no requirement-driven test relaxation occurred.
- **RPP levels scanned:** L1.
- **Cascade stopped at:** L1, at D2's stale/misleading public error identifiers and absence wording.
- **RPP findings:** D2.

### Iteration-2 defect counts

| Severity | Count |
|---|---:|
| Blocker | 2 |
| Critical | 0 |
| High | 2 |
| Medium | 0 |
| Low | 0 |
| **Total** | **4** |

### Iteration-2 final verdict

**REJECTED**

D2 and D4–D6 must be remediated by the original step 01-03 crafter. Under this repository's DELIVER override there is no review iteration cap; after remediation, this same reviewer must perform iteration 3 before the step may advance.

## Iteration 3

- **Review ID:** `code_rev_20260828_142007_iteration_3`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated reviewer as iterations 1 and 2)
- **Remediation commit:** `60c92e0a92de0ac3b5c663e96545b240bc58f92e`
- **Parent:** `32f136743902341b6e00a8d950ea2966ba303605`
- **Subject:** `fix(guest-stack-transparent-mtls-intercept): harden launch probe oracles`
- **Trailer:** `Step-Id: 01-03`
- **Iteration-3 verdict:** **REJECTED**

### Iteration-3 executive summary

The remediation completes D2's typed diagnostic: `LaunchToolUnavailable` now names the launch prerequisite rather than confinement, retains the original `io::Error`, and distinguishes PATH absence from other execution failures. It returns the step to its ten-test budget through legitimate consolidation, replaces D6's partial complement with exact `{etc, proc}` equality and RAII cleanup, and retains the complete host launch/parser behavior. The complete focused suite passes, as do formatting, affected-package Lima check and clippy, and both init musl target checks.

The step is still rejected. The replacement for D5 gives `CloudHypervisorVmm` a `#[cfg(test)]` field and a test-only early-return branch inside `Vmm::probe`; the new test exercises that substitute branch, not the production `Command`-based launch probe. It remains green if the production helper stops probing `ip`, and the test accommodation also moved launch-tool probing ahead of the established reflink/cloud-hypervisor checks, changing multi-fault refusal precedence. This is a blocker-level test-only SUT path and violates the repository's explicit rule that production code is not shaped by simulation. Separately, the public `Vmm::probe` contract still promises only ADR-0082's five substrate probes and omits the now-mandatory `prlimit`, `setpriv`, and `ip` prerequisite, leaving implementations and callers with a contradictory trait contract.

### D2 and D4–D6 disposition

| Finding | Disposition | Evidence |
|---|---|---|
| D2 — public probe type misclassifies the namespace launcher | **RESOLVED** | `VmmProbeError::LaunchToolUnavailable { tool, source }` and `launch_tool_unavailable` use honest launch vocabulary. `NotFound` renders `not found on PATH`; `PermissionDenied` renders `could not execute`; both retain the exact source kind and are asserted structurally and textually. |
| D4 — remediation exceeds the test budget | **RESOLVED** | Related host and init tests were consolidated without dropping assertions. The step now has exactly ten tests for five roadmap behaviors, meeting `10 <= 2 x 5`. |
| D5 — capability test verifies a constant rather than the VMM probe path | **PARTIALLY RESOLVED; BLOCKER REPLACEMENT REMAINS** | The new test invokes `Vmm::probe` and observes ordered `prlimit`, `setpriv`, `ip` visits, but only through the `#[cfg(test)]` branch that bypasses the production command path. See D7. |
| D6 — bounded-change bootstrap omits the exact complement | **RESOLVED** | The test enumerates the root into a `BTreeSet`, asserts exact equality with `{etc, proc}`, confirms both entries are directories and the proc mount target was used, and removes the scratch root from `Drop` even after assertion failure. |

### D7 — The launch-probe test substitutes a test-only `Vmm::probe` path

- **Severity:** Blocker
- **Dimension:** Testing theater, port-boundary compliance, and production behavior preservation
- **Locations:**
  - `crates/overdrive-host/src/vmm.rs:77`
  - `crates/overdrive-host/src/vmm.rs:118`
  - `crates/overdrive-host/src/vmm.rs:172`
  - `crates/overdrive-host/src/vmm.rs:178`
  - `crates/overdrive-host/src/vmm.rs:318`
  - `crates/overdrive-host/src/vmm.rs:792`
  - `crates/overdrive-host/src/vmm.rs:907`

**Evidence:** `LaunchToolProbe`, the `CloudHypervisorVmm::launch_tool_probe` field, and its builder exist only under `#[cfg(test)]`. In a test build, `probe_required_launch_tools` invokes that closure and returns at line 184; it never calls production's `probe_launch_toolchain`, whose `tokio::process::Command` loop is compiled at lines 792-797. The new test injects exactly this alternate branch. It therefore remains green if the production helper skips `ip`, changes its command construction, or simply returns `Ok(())`. The assertion message's claim that it exercises “the production probe path” is false.

The accommodation also moved `probe_required_launch_tools` from after the reflink and cloud-hypervisor checks to the first line of `Vmm::probe`. That lets the injected `ip` error short-circuit before any real substrate access, but it changes which typed refusal an operator sees when both the existing substrate and a launch prerequisite are broken. No design or public-contract change authorizes that precedence regression. This is the exact “signature contortion whose only caller is a test” prohibited by `.claude/rules/development.md` § Production code is not shaped by simulation, and it is implementation-mirroring theater: the test proves its private surrogate loop, not the executable production boundary.

**Required remediation:** Remove the `#[cfg(test)]` field, builder, and alternate early-return branch. Put command execution behind a genuine production-compiled port or functional boundary that the real adapter and the test both execute, so breaking or removing production's `ip` probe makes the `Vmm::probe` test fail. Preserve the previously established overall probe ordering unless a design/trait-contract amendment explicitly changes it; the test must not require production refusal precedence to move merely to avoid real substrate probes.

### D8 — The public `Vmm::probe` contract omits its new mandatory launch prerequisites

- **Severity:** High
- **Dimension:** Public port-contract completeness and behavioral honesty
- **Location:** `crates/overdrive-core/src/traits/vmm.rs:62`

**Evidence:** The trait's `# Postconditions on Ok` still says the adapter proves reflink, cloud-hypervisor/Landlock, KVM, and the run root, and explicitly calls those “ADR-0082 §D5's five fault-injection scenarios.” The production implementation now also refuses startup unless `prlimit`, `setpriv`, and `ip` can be executed. The error enum documents the added condition, but the method contract—the repository's port SSOT—does not. A second `Vmm` implementation can satisfy the written trait contract while omitting all three prerequisites, and callers cannot learn from the method contract that `Ok(())` guarantees the launch wrapper chain is executable.

**Required remediation:** Amend `Vmm::probe`'s public postconditions/errors to state that every mandatory launch executable (`prlimit`, `setpriv`, and `ip`) was executed successfully enough to prove availability, and that a spawn failure returns `LaunchToolUnavailable` with its original source. Remove or qualify the stale “five scenarios” completeness claim and document any intentionally required probe ordering.

### Contract Shape Compliance — PASS

| Check | Status | Evidence |
|---|---|---|
| Exact pure-function declarations | PASS | Every live source-local pure property retains the exact required `/// CONTRACT_SHAPE: pure-function.` declaration. |
| Bounded-change declarations | PASS | The launch-probe, minimal-root, and setup-failure tests name their changed state explicitly. |
| Bounded-change delta and complement | PASS | The minimal-root test now asserts exact `{etc, proc}` complement; the setup failure asserts one `EXIT 78` and no exec; the injected probe test asserts exact ordered visits and the exact returned error. |
| Cleanup robustness | PASS | `ScratchRoot::Drop` removes the unique PID/counter path even when an assertion unwinds. |
| Banned test-name regex | PASS | No new or transitioned test name matches the banned regex. |
| Outcome anchor | NOT APPLICABLE | The real guest boot remains explicitly metal-deferred for step 01-03. |

D7 is a test-validity/production-shape blocker, not a missing Contract Shape declaration or complement.

### Mechanical evidence — PASS

- Stat: **4 files changed, 190 insertions, 88 deletions**.
- Scope is limited to the public VMM probe error, the host VMM remediation, init test consolidation/cleanup, and the execution log.
- Pre-existing dirty `roadmap.json`, `AGENTS.md`, and review artifacts remain outside the remediation commit.
- `git diff --check` passes, and no mutation exclusion changed.
- The public error-variant replacement has no stale `ConfinementToolchainAbsent`/constructor uses in the Rust workspace.
- `overdrive-host` retains crate-level `#![forbid(unsafe_code)]`; the remediation adds no unsafe block.
- The Beacon Published Language is unchanged.

#### Remediation DES phase order — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T14:07:53Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T14:08:48Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T14:12:25Z` |

The triplet is complete, successful, ordered, and the COMMIT timestamp exactly matches commit `60c92e0a92de0ac3b5c663e96545b240bc58f92e` at `2026-08-28T14:12:25Z`.

#### Test budget — PASS

| Behaviors | Budget (`2 x behaviors`) | Step total after consolidation |
|---:|---:|---:|
| 5 | 10 | 10 |

#### Test integrity — PASS

The commit modifies and removes tests, but only through legitimate consolidation/parameterization:

- The two host tests are folded into the probe error-kind table and the mesh/non-mesh launch test; their `ip` inclusion, mesh `ip` attribution, non-mesh `UidDrop` attribution, and full argv assertions remain.
- The non-mesh guest parser test is folded into the mesh parser test with its exact `None` assertion preserved.
- The minimal-root test strengthens its complement and cleanup; no assertion or failure condition is relaxed.

No test is skipped, ignored, weakened, or changed to accept a broader production outcome. D7 concerns the newly introduced alternate SUT path, not assertion deletion.

### External validity — PASS with test-boundary blocker

The production build of `Vmm::probe` does call the real launch-tool helper, and that helper iterates `prlimit`, `setpriv`, and `ip`, invokes `<tool> --version`, and maps spawn failures through the honest typed error. The init path retains exact minimal-root bootstrap, pre-exec network setup, `EXIT 78`, and no-exec behavior. Thus the intended paths are statically production-reachable and the walking-skeleton metal proof remains honestly deferred.

The green launch-probe unit test does not validate that production command boundary because D7 substitutes it under `cfg(test)`; external production reachability does not cure the blocker-level test oracle.

### Iteration-3 verification

| Verification | Result |
|---|---|
| `git diff --check 32f136743902341b6e00a8d950ea2966ba303605 60c92e0a92de0ac3b5c663e96545b240bc58f92e` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests -- -D warnings` | PASS |
| Focused Lima `nextest` run for all ten step tests | PASS — 10 passed, 0 failed, 971 skipped |
| `cargo xtask lima run -- cargo check -p overdrive-init --target x86_64-unknown-linux-musl` | PASS |
| Lima aarch64-musl check with `clang --target=aarch64-unknown-linux-musl` supplied to `cc-rs` | PASS |
| Launch-error source audit | PASS — no stale confinement variant/constructor; exact source kind retained |
| Minimal-root source audit | PASS — exact `{etc, proc}` complement and RAII cleanup |
| Host unsafe scan | PASS — crate-level forbid retained, zero new unsafe |
| Beacon published-language diff | PASS — no variant, parser, or formatter changed |
| Metal/KVM guest boot | NOT RUN — explicitly deferred by roadmap step 01-03 |
| Mutation testing | NOT RUN — explicitly prohibited during individual roadmap steps |

The focused test command was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init -E 'test(kernel_cmdline_appends_one_space_free_platform_token_and_rejects_whitespace) | test(vmm_probe_rejects_each_injected_ip_execution_failure_with_honest_diagnostics) | test(mesh_and_non_mesh_launches_preserve_shape_and_attribute_the_actual_launcher) | test(minimal_guest_root_bootstrap_creates_proc_and_etc_preconditions) | test(guest_setup_failure_reports_nonzero_exit_and_never_executes_operator_command) | test(guest_network_parser_maps_the_mesh_token_and_preserves_non_mesh_cmdlines) | test(guest_network_parser_rejects_partial_or_duplicate_platform_tokens) | test(complete_mesh_network_inputs_become_one_attachment_and_one_guest_addressing_token) | test(incomplete_mesh_network_inputs_are_rejected_before_vm_provisioning) | test(non_mesh_vm_keeps_the_platform_default_cmdline_and_has_no_attachment)'
```

### Iteration-3 quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — Exactly one acceptance active | PASS | The roadmap's walking-skeleton/metal-deferred override remains applicable. |
| G2 — Valid RED failure | PASS | The remediation RED event is `EXECUTED/PASS`. |
| G3 — Assertion failure | PASS | The remediation RED event is `EXECUTED/PASS`. |
| G4 — No domain mocks | PASS | No domain port or domain behavior is mocked; D7 is a separate test-only infrastructure/SUT-path violation. |
| G5 — Business language | PASS | Names describe launch refusal/diagnostics, launch shape, bootstrap, fail-closed execution, parser, and worker outcomes. |
| G6 — All green | PASS | Relevant check, lint, cross-target, and focused-test lanes pass. |
| G7 — 100% passing before commit | PASS | The remediation COMMIT event is `EXECUTED/PASS`. |
| G8 — Test budget | PASS | 10 step tests meet the budget of 10. |
| G9 — No impermissible test modification | PASS | Consolidation retains every prior outcome/assertion; no weakening, skip, or deferred fix. |

### Iteration-3 test integrity and RPP scan

- **Test modification detected:** Yes; legitimate consolidation/parameterization with all prior behavioral assertions retained.
- **Testing theater detected:** Yes; blocker-level implementation-mirroring/test-only SUT path in D7.
- **Escalation verification:** Not applicable; no requirement-driven relaxation or escalation marker is present.
- **RPP levels scanned:** L1-L4.
- **Cascade stopped at:** L4, at the test-only launch-probe abstraction crossing the real adapter boundary.
- **RPP findings:** D7. D8 is a public-contract completeness defect rather than an RPP smell.

### Iteration-3 defect counts

| Severity | Count |
|---|---:|
| Blocker | 1 |
| Critical | 0 |
| High | 1 |
| Medium | 0 |
| Low | 0 |
| **Total** | **2** |

### Iteration-3 final verdict

**REJECTED**

D7 and D8 must return to the original step 01-03 crafter. Under this repository's DELIVER override there is no review iteration cap; after remediation, this same reviewer must perform iteration 4 before the step may advance.

## Iteration 4

- **Review ID:** `code_rev_20260828_143347_iteration_4`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated reviewer as iterations 1–3)
- **Remediation commit:** `c035ac38e02417b276aa344763ca5b6b1bc2ae3b`
- **Parent:** `60c92e0a92de0ac3b5c663e96545b240bc58f92e`
- **Subject:** `fix(guest-stack-transparent-mtls-intercept): validate production probe composition`
- **Trailer:** `Step-Id: 01-03`
- **Iteration-4 verdict:** **APPROVED**

### Iteration-4 executive summary

The remediation resolves both remaining findings without introducing a new defect. D7's `#[cfg(test)]` field, builder, and alternate early-return SUT path are gone. A production-compiled `VmmProbeSubstrate` boundary now owns every external startup-probe operation; `CloudHypervisorVmm::new` installs the real host implementation, the real `Vmm::probe` composes that boundary in the established order, and the test substitutes only the driven substrate port while exercising the exact production orchestration and launch-tool loop. Deleting the launch-tool call, omitting `ip`, or changing the ordered plan now breaks the exact visited-stage oracle. The production order is restored to reflink → Cloud Hypervisor/Landlock → `prlimit` → `setpriv` → `ip` → KVM → run-dir, and per-VM `create` does not invoke the startup probe.

D8 is also complete: the public `Vmm::probe` contract names all five substrate checks plus the three launch executables, defines successful-spawn/ignored-exit-status semantics, preserves the original `io::Error` on failure, distinguishes `NotFound`, and declares first-failure ordering. The step remains at ten tests for five behaviors. All 965 affected-package tests and the focused ten-test slice pass in Lima, as do formatting, check, clippy, and both init musl target checks.

### D7–D8 disposition

| Finding | Disposition | Evidence |
|---|---|---|
| D7 — launch-probe test substitutes a test-only `Vmm::probe` path | **RESOLVED** | `VmmProbeSubstrate`, `RealVmmProbeSubstrate`, and `CloudHypervisorVmm::probe_substrate` are compiled in production. `CloudHypervisorVmm::new` installs the real implementation. `Vmm::probe` invokes the same boundary and shared `probe_launch_toolchain` in every build. The sole remaining `#[cfg(test)]` in `vmm.rs` gates only the test module; there is no test-only field, builder, helper, or early return in the SUT. The recording implementation replaces a driven infrastructure port, not `Vmm::probe` or its orchestration. |
| D8 — public `Vmm::probe` contract omits mandatory launch prerequisites | **RESOLVED** | The trait postconditions now enumerate `prlimit`, `setpriv`, and `ip`, define successful spawn as the availability proof regardless of `--version` exit status, retain the original error, distinguish PATH absence from other execution failures, and pin the intentional complete order and first-failure behavior. |

### New findings

None.

### Probe composition and regression audit — PASS

| Requirement | Status | Evidence |
|---|---|---|
| No `cfg(test)` SUT structure | PASS | The old `LaunchToolProbe`, `launch_tool_probe` field, `with_launch_tool_probe`, and `probe_required_launch_tools` branch are absent. `#[cfg(test)]` appears only on `mod tests`. |
| Production-compiled boundary | PASS | Private `VmmProbeSubstrate` is defined at `overdrive-host/src/vmm.rs:78`; the production `RealVmmProbeSubstrate` implements every operation and is installed by `CloudHypervisorVmm::new`. |
| Real `Vmm::probe` uses the boundary | PASS | `Vmm::probe` calls `check_reflink`, `check_cloud_hypervisor`, shared `probe_launch_toolchain`, `check_kvm`, and `check_run_dir` on `self.probe_substrate`. |
| `ip` omission/removal reds the oracle | PASS | The success oracle requires exact `[..., prlimit, setpriv, ip, kvm, run-dir]`; both failure cases require reaching `ip` after the two preceding tools. Removing the production helper call, skipping `ip`, or deleting it from `REQUIRED_LAUNCH_TOOLS` changes the observed vector and fails. |
| Complete refusal order | PASS | Production composition is reflink → Cloud Hypervisor/Landlock → shared tool loop (`prlimit`, `setpriv`, `ip`) → KVM → run-dir. Failure cases assert truncation exactly at `ip`; the success case asserts the full sequence. |
| Per-VM `create` avoids startup probing | PASS | The only `probe_substrate` calls are in `Vmm::probe`; `create` retains its per-launch clone/presence/confinement/spawn sequence and invokes no startup probe or launch-tool availability loop. |
| Public contract honesty | PASS | Core trait lines 72–99 describe checks, tools, success semantics, first-failure ordering, source retention, and diagnostic classification. |

### Contract Shape Compliance — PASS

| Check | Status | Evidence |
|---|---|---|
| Exact pure-function declarations | PASS | All live source-local pure properties retain the exact `/// CONTRACT_SHAPE: pure-function.` declaration. |
| Bounded-change declaration | PASS | The probe property declares the complete stage order and injected `ip` failure outcome. |
| Delta and complement | PASS | Failure cases assert the exact prefix ending at `ip`; the success case asserts the exact seven-stage sequence including KVM and run-dir. Exact typed variant, tool, source kind, and display are retained. |
| Test isolation | PASS | Each table case and the success case owns a fresh recording substrate and vector; no process-global PATH or shared mutable fixture is used. |
| Banned test-name regex | PASS | No new or transitioned test name matches the banned regex. |
| Outcome anchor | NOT APPLICABLE | The real guest boot remains explicitly metal-deferred for step 01-03. |

### Mechanical evidence — PASS

- Stat: **3 files changed, 168 insertions, 56 deletions**.
- Scope is limited to the public VMM probe contract, host VMM probe remediation/test, and execution log.
- No new public type or method was introduced; `VmmProbeSubstrate` and both implementations remain private to the host adapter.
- Pre-existing dirty `roadmap.json`, `AGENTS.md`, and review artifacts remain outside the remediation commit.
- `git diff --check` and `cargo fmt --all -- --check` pass.
- No mutation exclusion changed, and no mutation command was run.
- `overdrive-host` retains crate-level `#![forbid(unsafe_code)]`; the remediation adds no unsafe code.
- The Beacon Published Language and guest-init behavior are unchanged.

#### Remediation DES phase order — PASS

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-08-28T14:26:07Z` |
| GREEN | EXECUTED / PASS | `2026-08-28T14:26:25Z` |
| COMMIT | EXECUTED / PASS | `2026-08-28T14:29:44Z` |

The triplet is complete, successful, ordered, and the COMMIT timestamp exactly matches commit `c035ac38e02417b276aa344763ca5b6b1bc2ae3b` at `2026-08-28T14:29:44Z`.

#### Test budget — PASS

| Behaviors | Budget (`2 × behaviors`) | Step total |
|---:|---:|---:|
| 5 | 10 | 10 |

#### Test integrity — PASS

The remediation renames and strengthens the one host probe test. It preserves both prior injected error cases and their exact typed/text diagnostics, adds the restored pre-tool stage prefix to the failure oracle, and adds a successful full-order oracle covering KVM and run-dir. No assertion, failure condition, test, or Contract Shape obligation was weakened, skipped, ignored, or deleted. The other nine step tests are unchanged and pass.

### External validity — PASS

`CloudHypervisorVmm::new`, the production composition path, always wires `RealVmmProbeSubstrate`. Its tool operation invokes `tokio::process::Command::new(tool).arg("--version")`; the shared production loop supplies the exact `prlimit`, `setpriv`, and `ip` plan and maps failures through `LaunchToolUnavailable`. The composition root's existing wire → probe → use lifecycle therefore executes the real host checks once at startup. Per-VM `create` remains separate and does not repeat them. The roadmap's actual KVM guest boot stays honestly deferred to the metal gate.

### Iteration-4 verification

| Verification | Result |
|---|---|
| `git diff --check 60c92e0a92de0ac3b5c663e96545b240bc58f92e c035ac38e02417b276aa344763ca5b6b1bc2ae3b` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init --all-targets --features integration-tests -- -D warnings` | PASS |
| Full affected-package Lima `nextest` run | PASS — 965 passed, 0 failed, 16 skipped |
| Focused Lima `nextest` run for all ten step tests | PASS — 10 passed, 0 failed, 971 skipped |
| `cargo xtask lima run -- cargo check -p overdrive-init --target x86_64-unknown-linux-musl` | PASS |
| Lima aarch64-musl check with `clang --target=aarch64-unknown-linux-musl` supplied to `cc-rs` | PASS |
| `cfg(test)`/probe-boundary source audit | PASS — only test-module gate remains; production and test share the orchestration/helper |
| Per-VM create source audit | PASS — no startup probe invocation |
| Public trait-contract audit | PASS — tools, semantics, original source, and exact order documented |
| Host unsafe scan | PASS — crate-level forbid retained, zero new unsafe |
| Metal/KVM guest boot | NOT RUN — explicitly deferred by roadmap step 01-03 |
| Mutation testing | NOT RUN — explicitly prohibited during individual roadmap steps |

The focused test command was:

```text
cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-host -p overdrive-worker -p overdrive-init -E 'test(kernel_cmdline_appends_one_space_free_platform_token_and_rejects_whitespace) | test(vmm_probe_preserves_stage_order_and_rejects_each_injected_ip_execution_failure) | test(mesh_and_non_mesh_launches_preserve_shape_and_attribute_the_actual_launcher) | test(minimal_guest_root_bootstrap_creates_proc_and_etc_preconditions) | test(guest_setup_failure_reports_nonzero_exit_and_never_executes_operator_command) | test(guest_network_parser_maps_the_mesh_token_and_preserves_non_mesh_cmdlines) | test(guest_network_parser_rejects_partial_or_duplicate_platform_tokens) | test(complete_mesh_network_inputs_become_one_attachment_and_one_guest_addressing_token) | test(incomplete_mesh_network_inputs_are_rejected_before_vm_provisioning) | test(non_mesh_vm_keeps_the_platform_default_cmdline_and_has_no_attachment)'
```

### Iteration-4 quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — Exactly one acceptance active | PASS | The roadmap's walking-skeleton/metal-deferred override remains applicable. |
| G2 — Valid RED failure | PASS | The remediation RED event is `EXECUTED/PASS`. |
| G3 — Assertion failure | PASS | The remediation RED event is `EXECUTED/PASS`. |
| G4 — No domain mocks | PASS | The recording implementation is a driven infrastructure-port adapter; it does not replace domain behavior or `Vmm::probe`. |
| G5 — Business language | PASS | The strengthened name and oracles state startup refusal order and honest launcher diagnostics. |
| G6 — All green | PASS | Full affected-package, focused, check, lint, formatting, and cross-target lanes pass. |
| G7 — 100% passing before commit | PASS | The remediation COMMIT event is `EXECUTED/PASS`. |
| G8 — Test budget | PASS | Ten step tests meet the budget of ten. |
| G9 — No impermissible test modification | PASS | The renamed probe test is strictly strengthened; no prior assertion or behavior is lost. |

### Iteration-4 test integrity and RPP scan

- **Test modification detected:** Yes; legitimate rename and assertion strengthening only.
- **Testing theater detected:** No. Deleting/skipping the production launch-tool composition or `ip` plan breaks the exact driving-port outcome.
- **Escalation verification:** Not applicable; no test relaxation or escalation marker is present.
- **RPP levels scanned:** L1–L6.
- **Cascade stopped at:** None; all scanned levels are clean for this remediation.
- **RPP findings:** None.

### Iteration-4 defect counts

| Severity | Count |
|---|---:|
| Blocker | 0 |
| Critical | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |
| **Total** | **0** |

### Iteration-4 final verdict

**APPROVED**

D7 and D8 are resolved, every mandatory quality and verification gate passes, and no new defect was found. Step 01-03 may advance under the repository's DELIVER sequence.
