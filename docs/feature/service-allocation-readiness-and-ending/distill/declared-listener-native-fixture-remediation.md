# Step 05-01 DISTILL remediation: declared-listener native fixture

## Reproduction and cause isolation

The assigned `overdrive-worker::integration::inbound_rules_only_for_tcp_declared_listeners::a_udp_declared_listener_gets_no_inbound_capture_and_a_tcp_one_does` body runs in the existing `cgroup-default` disposable guest. Before this remediation it failed at `MtlsInterceptWorker::start_shared_owner`, before `declared_listeners` was assigned or `start_alloc` was called. The visible error was `NftSharedReplaceFailed` from `shared-ip-atomic-transaction` with `ENOENT`; the test's `record_uname` line printed an empty kernel release. The error code by itself did not identify the missing prerequisite.

The per-test guest planner (`cgroup_attempt_files`) did not select the older nft fixture list for this new exact test name. `cgroup_tools` therefore staged `ip` but neither `nft` nor `uname`. The shared test fixture's `clean_shared_infra` invokes external `nft` best-effort, so that scrub could not run. The `cgroup-default` module list already staged the nf_tables/tproxy/fib module closure, and the guest setup loaded it before the test. No module or production nft transaction was changed.

To verify the tool absence independently of the netlink error, I added a selected-test guest preflight for `/usr/bin/nft --version` and `/usr/bin/uname -r`, rebuilt the existing guest-suite binary, and reran the exact body **before staging the tools**. The guest exited 127 with `/usr/bin/nft: not found` (`.context/step-05-01-declared-listener-preflight-before-rebuilt.log`). The original empty `uname` output and the staged-file list independently identify `uname` as absent. The first attempt to run that diagnostic used an older already-built guest-suite binary; it still reached the original test failure, so the rebuilt-runner preflight is the evidence for the missing executable.

## Bounded fixture correction

The existing `cgroup-default` per-test planner now recognizes this exact integration test by a stable descriptive test-name constant and stages only the `nft` and `uname` executables with their shared libraries. Its preflight requires both executables before the test enters `start_shared_owner`; no new lane, guest policy, public API, or product behavior was introduced. The native test still drives `start_shared_owner` then `start_alloc`, observing that a mixed TCP/UDP declaration installs exactly the TCP inbound capture. Two added fixture checks observe absence of the shared intercept program after the initial scrub and final cleanup. They make the previously silent cleanup prerequisite visible without weakening the TCP/UDP oracle.

Only `crates/overdrive-testing/bin/fwd_guest_suite.rs` and `crates/overdrive-worker/tests/integration/inbound_rules_only_for_tcp_declared_listeners.rs` changed. The target remains in its existing `cgroup-default` placement. The test retains `/// CONTRACT_SHAPE: bounded-change.`. All crafter production edits, activated tests, and DES events were preserved; this remediation made no commit.

## Native evidence

| Check | Result |
| --- | --- |
| Before: exact native body on current step production | Failed before allocation admission with `NftSharedReplaceFailed` / `ENOENT`, and empty `uname -r`. |
| Rebuilt guest-suite preflight before tool staging | Failed before the test body, exit 127: `/usr/bin/nft: not found`. |
| Metal `cargo build -p overdrive-testing --bin overdrive-fwd-guest-suite` after staging | Passed; the runner used by subsequent Nextest attempts included this fixture change. |
| Exact native body after staging | Passed 1/1 with the original TCP-only destination assertion; after adding explicit scrub/cleanup observations, passed again 1/1. The final smaller per-test planner variant also passed 1/1 (`.context/step-05-01-declared-listener-native-final.log`). |
| Existing `mtls_intercept_install::shared_program_absence_create_readback_idempotence_and_guard_drop` nft fixture control | Passed 1/1 in its existing guest placement (`.context/step-05-01-existing-nft-fixture-control.log`). |
| Metal strict Clippy for the existing guest-suite binary | Passed after placing the exact-test tool selection in the per-test planner. An intermediate four-boolean `cgroup_tools` signature failed Clippy; that transient helper change was removed before the final run. |
| Changed-file rustfmt and `git diff --check` | Passed. |

All builds and tests above ran through `cargo xtask metal run --`. No broad suite, mutation pass, production binary spawn inside a Rust test, or host-kernel nft operation was used. Independent DISTILL review remains the gate before the original step 05-01 crafter resumes GREEN and COMMIT.
