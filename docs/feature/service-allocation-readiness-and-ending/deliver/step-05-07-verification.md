# Step 05-07 verification record

Date: 2026-11-13
Crafter: Codex replacement
Feature: service-allocation-readiness-and-ending
Step: 05-07 — Default startup gate is guest-reported listeners
Status: GREEN; this record is included in the step COMMIT.

## Source and contract

The source checkout was rebased onto origin/main at 21a0be69f89fd2c412bace7476580dba340a0b12; the step was executed at HEAD ff7fb882d2369a8a68d4c876c20246e456edb88d. The implementation and verification runs used isolated worktrees at that same HEAD. The GREEN worktree contained only the 05-07 production, acceptance, placement, and E08 preparation changes.

ADR-0270 and feature-delta D15 require an omitted Service startup section to materialize the fieldless GuestListeners descriptor. That gate observes every declared TCP and UDP listener, keeps the 60-second startup window, and does not consume a dial-bearing probe slot. An explicitly empty startup list remains the opt-out. D14-E2 typed ingress errors and their mappings remain unchanged. No public or cross-crate API shape, wire format, or persisted format changed.

The accepted private E08 support was revalidated through the default-feature product path. The control-plane setup applies the approved root:4200/0710 traverse posture to both the platform-owned vm parent and clone-staging directory. E08 preparation stages and checks the same-release dummy, socket-diagnostic, and ovd_fwd modules, the static modprobe, and alias metadata. overdrive-init mounts sysfs for guest kernel BTF access. These changes are private support required for the checked-in native Service journey; no operator API was added.

## DES

The replacement independently reran RED from the rebased HEAD. In isolated worktree /private/tmp/step-05-07-red-rebased-20261112, the active D15 parser property and its test-placement entry were overlaid while the production parser remained at HEAD. Metal run 0f9e3f9e7f0253e49ba986ed failed for the expected semantic assertion: the parser produced Tcp { host: "0.0.0.0", port: 1 } where the property required GuestListeners. This replacement recorded RED FAIL at 2026-11-12T22:15:12Z.

GREEN passed on the rebased implementation. This replacement recorded GREEN PASS at 2026-11-12T23:33:45Z. Implementation commit 229edec40ad544fce501823888b49cfe2c1b92db contains the step-owned changes and this verification record. After it passed, this replacement recorded DES COMMIT PASS at 2026-11-12T23:43:17Z; the follow-up record commit stores that event. No mutation test was run; the single wave gate remains after all step reviews.

## Verification

| Evidence | Result |
|---|---|
| overdrive-core acceptance, Metal run c19f11d6d4d27539ddd3b748 | 646 passed, 0 skipped |
| Composed G-10 (8), Metal run 6b68469e86f5f1439ccba785 | 1 passed |
| S-22 default startup window and D10 90-second deadline, Metal run 701d52609d48cb75bc64938e | 2 passed |
| CLI streaming test before implementation, Metal run 12e97e7b645ed3815090209b | 1 passed |
| Same CLI streaming test after implementation, Metal run eb06604e8250c5d82f0782da | 1 passed; no fix is attributed to this step |
| Both guest_listener_real_boot Tier-3 cases, Metal run 38a45c6310e0b454c0b5ee14 | 2 passed |
| BPF build, Metal run 044a7d8fc143552429a7dfe3 | Passed; guest objects produced |
| Forwarder module build, Metal run 66c107404e99e5808f4fd920 | Passed; MODULE_VERSION e2b939e45cb106448fa4d8f8ca11e52f990bf1127c8af80ed09b2e3c6299f3bc |
| Forwarder module identity check, Metal run 0e33ec4c44f86ec61bbc0cbe | Already-current version matched; no reload was needed |
| GuestKit preparation, Metal run b1e1632b8feaaf1b74fbe136 | Passed; staged 4,437 runtime inputs for kernel 7.0.0-38-generic |
| E08 default-feature positive Service, Metal run d3b256d3996fb41f35319a65 | Stable in 1,363 ms; witness guest listeners tcp:18081 |
| E08 default-feature never-listening Service, Metal run 5385378c391bf17f9e36f29b | Full restart budget exhausted; see the detailed account below |
| Scoped formatting, Metal run d151ecb676e059090512d648 | Passed for overdrive-core, overdrive-control-plane, overdrive-cli, and overdrive-init |
| Scoped Clippy, Metal run f74ab365830db3b76e2809be | Passed for those crates with integration-tests; existing dead-code warning for action_shim::EvaluationSubmission::Standalone remains |
| bash syntax and ShellCheck for examples/service-kind-vm-workloads/prepare.sh | Passed |
| ShellCheck across both prepare.sh and the unchanged run-example.sh | Reports existing SC2034 for CASE_CLEANUP_POLICY at run-example.sh:997; prepare.sh passes |
| git diff --check for the step implementation and mechanical fallout | Passed |

The positive operator capture is in [the E08 positive evidence](../../../../.context/step-05-07-native/rebased-20261112/positive). It records the built default-feature overdrive binary deploying the checked-in zero-probes Service, Stable with a guest-listener Pass, then stopping it and removing the E08-owned materialization. Cleanup deltas are zero for VM processes, probe ownership, networks, cgroups, run directories, mounts, loops, and preparation.

For the negative operator proof, the first deploy of the checked-in zero-probes-failure.toml returned D10's expected 90-second progress timeout. The product continued its restart lifecycle while the proof polled workload describe. The final status contained six allocation attempts: attempts 0–4 were Terminated by the startup probe, attempt 5 was Failed, and each recorded the missing listener tcp:18998 after 60 seconds. Re-submitting the byte-identical checked-in spec after give-up returned the terminal stream result: “startup probe[0] failed after 5 attempts: guest listeners not open after 60s: tcp:18998”. The five restart attempts correspond to six observed allocation rows. Cleanup deltas were zero for VM processes, networks, cgroups, run directories, mounts, loops, and the E08 materialization. Full stdout, poll history, describe output, and cleanup observations are in [the corrected negative capture](../../../../.context/step-05-07-native/rebased-20261112/negative-retry).

An earlier negative-capture predicate incorrectly required a non-zero Restarts column on an allocation row. The actual Service represents each restarted attempt with a distinct allocation row, so the counter is zero on each. That capture nevertheless recorded the final six-row state and zero cleanup; it did not issue the terminal re-submit. It is preserved separately in [the initial negative capture](../../../../.context/step-05-07-native/rebased-20261112/negative). The corrected capture checks the six allocation rows and issues the same spec again to collect the final Service failure.

The one-off negative capture driver is retained at .context/step-05-07-native/rebased-20261112/negative-proof.sh. It uses only the built default-feature CLI, the checked-in example spec, and prepare.sh. It does not import an Overdrive crate, launch a Rust test binary, alter the 90-second deadline, or modify run-example.sh.

## Implementation scope

The core parser now synthesizes GuestListeners for an omitted Service startup section and the probe cap counts only TCP/HTTP dial-bearing descriptors. The empty-list opt-out and explicit probe behavior are unchanged. Existing core probe-count acceptance cases were updated to the new dial-bearing meaning, including the bounded D15 fallout in vsock_deploy_validation.rs. Metal placement follows the renamed pre-authored D15 properties.

The native proof required the following private support: prepare_clone_staging_root secures both platform-owned clone path ancestors; the E08 preparation validates the actual in-guest module closure; overdrive-init mounts sysfs before loading guest BTF programs and establishes the guest transport file-descriptor limit. The native positive boot and the two guest-listener real-boot tests passed through the production composition. No API or typed public error contract was invented or weakened.
