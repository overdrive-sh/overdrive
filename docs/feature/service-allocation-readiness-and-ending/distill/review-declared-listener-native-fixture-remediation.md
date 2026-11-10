# Independent DISTILL review — native declared-listener fixture tools

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | Exact-test `cgroup-default` guest tool staging and setup/cleanup observations for S-55's native TCP-only inbound capture test |
| Author record | `declared-listener-native-fixture-remediation.md`; retained `.context/step-05-01-*` Metal logs |
| Protected work | Original step 05-01 crafter's dirty production/API and mechanical fixture changes, activated test, DES phases and commit |
| Verdict | **APPROVED** — this supplies the existing native test's missing executable prerequisites and strengthens cleanup evidence without supplying the production capture |

## Reproduction and necessity

The assigned `integration::inbound_rules_only_for_tcp_declared_listeners::a_udp_declared_listener_gets_no_inbound_capture_and_a_tcp_one_does` body runs in the existing disposable `cgroup-default` guest. Its pre-remediation failure occurred at `MtlsInterceptWorker::start_shared_owner`, before `start_alloc` could exercise the declared-listener TCP filter. The visible `NftSharedReplaceFailed`/`ENOENT` alone did not identify a cause, so the author rebuilt the guest-suite binary with a selected-test preflight and reran **before** staging tools. The retained native log shows `/usr/bin/nft: not found` and exit 127 (`.context/step-05-01-declared-listener-preflight-before-rebuilt.log`); the old `record_uname` output was empty. The existing `clean_shared_infra` fixture invokes external `nft` to delete the node-shared tables (`inbound_tproxy_harness.rs:110–129`), while `record_uname` requires the guest `uname` executable. The old per-test `cgroup_attempt_files` planner did not select the older nft tool list for this new exact test identity. The module closure was already staged and loaded; this correction does not change kernel module or production nft behavior.

The old native failure is therefore a genuine fixture-prerequisite failure. The explicit preflight proves missing tools independently of the netlink error, and the post-staging test pass with a checked clean start shows the corrected fixture can run its intended product oracle. This review does not infer that every `ENOENT` from nft netlink means a missing CLI executable.

## Exact boundary and no-weakening audit

`fwd_guest_suite.rs` adds one exact test-name constant and, only when `cgroup_attempt_files` selects that identity, stages `nft` and `uname` through the existing `tool()` helper (`:241–246,3492–3524`). `tool()` marks each executable for its shared-library closure (`:1767–1774`). After the already planned module load, the same exact identity gets a fail-closed `/usr/bin/nft --version` and `/usr/bin/uname -r` preflight (`:3622–3636`). The existing `cgroup-default` lane, placement, module set and other tests' tool selection are unchanged. An incidental rustfmt-only hunk in the guest-suite runner changes no call or argument.

The native acceptance body still locks kernel state, scrubs shared residue, invokes the real `MtlsInterceptWorker::start_shared_owner` and `start_alloc`, and reads the production shared nft state through `observe_shared_ip_intercept_state` (`inbound_rules_only_for_tcp_declared_listeners.rs:38–88`). Its central assertion remains exact: one TCP destination for the declared TCP port and no inbound capture for the declared UDP port. The added pre-start and post-cleanup checks require the **shared intercept program to be absent** after the fixture scrub and after `stop_alloc`/`shutdown_owner`/scrub. They expose whether the external cleanup actually worked; they do not install a rule or bypass the production worker. The test retains `/// CONTRACT_SHAPE: bounded-change.` and its native placement. No public API, new lane, guest policy, production call site, or test-only capture mechanism is introduced.

## Native evidence and disposition

| Evidence | Result |
|---|---|
| Preflight before staging | Rebuilt guest-suite selected the test and failed at `/usr/bin/nft: not found`, exit 127. The earlier unrebuilt diagnostic attempt is not used as this proof. |
| Final selected test | The exact S-55 native body passed **1/1**, including unchanged TCP-only destination and the new absence complements (`.context/step-05-01-declared-listener-native-final.log`, exit 0). |
| Existing nft fixture control | `mtls_intercept_install::shared_program_absence_create_readback_idempotence_and_guard_drop` passed **1/1** in its existing placement (`.context/step-05-01-existing-nft-fixture-control.log`, exit 0). |
| Static gates | Author reports strict Metal guest-suite Clippy, changed-file rustfmt and `git diff --check` passed. My read-only diff inspection and `git diff --check` passed; no broad suite or mutation run was needed for this bounded review. |

**Verdict: APPROVED for bounded DISTILL native test support.** The added executable closure and preflight are necessary for the already assigned real-nft test, and the scrub/cleanup assertions make its starting and ending state explicit. The production TCP filter and shared nft installation remain exercised through the worker, with their original exact oracle. This is not a review of the original step 05-01 production diff or a replacement for its GREEN, DES, commit and independent DELIVER review.
