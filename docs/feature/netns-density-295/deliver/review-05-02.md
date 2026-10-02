# DELIVER Review — Step 05-02: Audited launch hook and launch seccomp filter

## Review metadata

- **Feature:** `netns-density-295`
- **Step:** `05-02`
- **Iteration:** 1
- **Reviewer:** nw-software-crafter-reviewer, GPT-6 Luna (maximum reasoning)
- **Reviewed commits:** `13fb746fcedb47a49c15fd134ace2972b666fec4`, `ad4c6d8971a88037843f3d1f76551b10d40c4d25`
- **Authority:** approved `deliver/roadmap.json` step 05-02; accepted D-295-R3/R22 and ADR-0129/ADR-0143 contracts in `feature-delta.md`; DISTILL bodies S-ND295-41 through S-ND295-44.

The only dirty worktree path is the user-owned `AGENTS.md`; it was preserved. The implementation commit retains Marcus as author and has exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 05-02` trailer. The completion commit changes only `execution-log.json` and carries the same required trailers.

## Review scope and strengths

The review checked the exact launch-hook and filter contracts, the `VmmProbeError` variants, probe and create ordering, activated Rust bodies, CI selection, and the native evidence boundary.

The implementation matches the accepted private surface: the pure `VmmLaunchSeccompFilter::for_target` builder and deny-list remain private to `overdrive-host`; the single private `register_launch_child_hook` takes the built filter by value; no second hook, public method, port, or error variant was added. The hook performs close-on-exec, `PR_SET_NO_NEW_PRIVS`, then seccomp filter installation, returning the first `io::Error` immediately. `create` builds the filter before its first filesystem or queue effect and maps unsupported targets to the existing `VmmError::ConfinementUnavailable { control: Seccomp, .. }` shape ([`vmm.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/vmm.rs:320), [`launch_seccomp.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/vmm/launch_seccomp.rs:26), [`vmm.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/vmm.rs:436)).

The x86_64 builder has the pinned 24-instruction verdict partition and 13-request deny-list. The real probe uses the same builder and hook, reaps `prlimit --version`, and keeps unsupported architecture, installation/spawn failure, and non-success exit as the three distinct `VmmProbeError` variants named in the design. No extra probe or `VmmError` variant was introduced ([`vmm.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/vmm.rs:117), [`vmm.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/vmm.rs:412), [`vmm.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/vmm.rs:527)).

The production crate now uses `deny(unsafe_code)` with the one locally allowed production hook, and its narration was updated. The CI nextest selector includes the source-local `launch_seccomp_kernel` module that the prior `binary(integration)` selector omitted ([`lib.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/lib.rs:21), [CI workflow](/Users/marcus/conductor/workspaces/helios/wellington-v2/.github/workflows/ci.yml:458)).

## Contract Shape Compliance

| Check | Result | Evidence |
|---|---|---|
| Exact public and cross-crate API shape | **PASS** | Exactly the three designed `VmmProbeError` variants and constructors were added. The builder and hook use the private shapes specified by the feature delta. |
| Filter behavior and deny-list | **PASS** | The 24-instruction program, low-32-bit ioctl matching, thirteen libc-derived request values, allow partition, foreign-architecture kill, x32 syscall kill, and `nr == -1` exception are covered by S-ND295-42. |
| Hook order and error propagation | **PASS** | S-ND295-41 drives the production hook and covers descriptor inheritance plus failure of each of its three child steps. The hook returns the failing step's errno and prevents the target from running. |
| Probe and create ordering | **PASS for this step** | `create` builds the filter as its first action. The typed probe test preserves launch-seccomp causes and places the stage after `setpriv`; the separate stage-order body that removes `ip` remains assigned to 05-03. |
| Contract Shape declarations | **PASS** | Activated pure source-local tests carry the exact `/// CONTRACT_SHAPE: pure-function.` declaration. The bounded-change bodies carry `/// CONTRACT_SHAPE: bounded-change.` and the required outcome anchor. |
| Test integrity | **PASS** | The implementation commit removes the pending `#[ignore]` markers for the named DISTILL bodies. It does not edit their assertions, reduce expectations, delete tests, or add skips. |
| CI coverage | **PASS** | The workflow expression explicitly selects `overdrive-host` library tests matching `launch_seccomp_kernel` and also selects the source-local control-plane `offload_read_failure_propagation` test. |

## Roadmap criteria

| Step 05-02 criterion | Result | Evidence |
|---|---|---|
| Pure builder and deny-list with property/table coverage; hook installs close-on-exec, no-new-privs, and filter | **PASS** | `launch_seccomp.rs` builds the program without I/O; `register_launch_child_hook` installs all three steps in order. S-ND295-42 and S-ND295-41/S-ND295-43 exercise those boundaries. |
| Create-first architecture refusal; probe stage and exactly three typed causes | **PASS** | Filter construction precedes all `create` effects. The probe returns `LaunchSeccompUnsupportedArch`, `LaunchSeccompInstall`, or `LaunchSeccompProbeExit` for the three pinned causes. |
| `deny(unsafe_code)`, narration, and CI selector | **PASS** | Crate lint and adjacent documentation agree with the single audited allow; CI selects the new native module. |
| Activate S-ND295-41, 42, 43, and 44 except its stage-order body | **PASS** | The named bodies were enabled without modifying their assertions. S-ND295-44's stage-order body remains pending 05-03 as the roadmap requires. |
| Lima workspace check and Clippy | **PASS** | Both required commands are reported clean. |

## TDD and verification evidence

`execution-log.json` records `RED`, `GREEN`, and `COMMIT` as `EXECUTED` / `PASS` in that order. The step transitions pre-authored DISTILL bodies from ignored to active; the reviewed source diff contains no test expectation edits. No per-step mutation testing was run.

The required verification results reported for this review were:

1. Native x86_64 metal command for `launch_seccomp_kernel`: **10 passed**. This is the native-kernel evidence lane; it ran on metal, not Lima.
2. Lima `overdrive-host --lib` `test(launch_seccomp)` selector: **passed**.
3. Lima `overdrive-host --lib` `test(vmm)` selector: **passed**.
4. Lima `cargo check --workspace --all-targets --features integration-tests`: **passed**.
5. Lima `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings`: **passed**.
6. Formatting of the changed Rust files: **passed**. A workspace-wide `cargo fmt --check` reports an earlier formatting difference in untouched `crates/overdrive-worker/src/mtls_intercept_worker.rs`; that file is outside this step's diff and does not alter the step result.

The acceptance tests re-exec the host crate's own test binary as the DISTILL contract requires; they do not spawn the built Overdrive production binary or serve as expectation runners.

## Unproven hypothesis, not a finding

The accepted feature delta lists `x86_64-unknown-linux-gnux32` as an unsupported target and pins support to 64-bit x86_64. `rustc --print cfg --target x86_64-unknown-linux-gnux32` reports `target_arch="x86_64"` and `target_pointer_width="32"`, while `for_target` selects its program using only `#[cfg(target_arch = "x86_64")]` ([`launch_seccomp.rs`](/Users/marcus/conductor/workspaces/helios/wellington-v2/crates/overdrive-host/src/vmm/launch_seccomp.rs:33)). That is a potential mismatch in the x32 return/error path. The x32 Rust target and runtime were not available for this review, and no failing production-entry-point regression was reproduced. Under the repository's reachability and reproduction rule, this remains an unproven hypothesis: it is not accepted as a defect, does not request remediation, and does not affect this verdict.

## Findings and dispositions

No proven blocking, high, or medium findings. No remediation is requested. The x32 note above remains only an unproven hypothesis pending a real production-path reproducer.

## Iteration 1 verdict

**APPROVED.** The step satisfies its approved launch-hook, seccomp-filter, create-first refusal, typed probe, test-activation, lint, and CI criteria. Native kernel behavior was verified on x86_64 metal, and the required Lima checks passed.
