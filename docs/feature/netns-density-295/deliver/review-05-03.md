# DELIVER Review — Step 05-03

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `05-03` — Cloud Hypervisor fd handoff: `TapQueue` and `attach_tap_queue` |
| Iteration | 1 |
| Reviewer | Fresh isolated `nw-software-crafter-reviewer` |
| Commits | `2ebf70da771b134f0af0e15eb057f0de4305aa70`, `11e75e5dee865b7e3dcfbec0d2b1afd186888260` |
| Verdict | **APPROVED** |

## Scope and contract reviewed

Reviewed only Step 05-03 against the approved roadmap, accepted feature-delta decisions D-295-R1/R2/R3, ADR-0127/0128/0129, and DISTILL scenarios S-ND295-38/39/40/44/45. The reviewed implementation surface was the TAP queue helper, the VMM adapter's queue mapping and launch ordering, the FICLONE cleanup changes, the specified activated test bodies, and the necessary Cargo dependency resolution.

The review applied the exact contract: one checked queue from an existing down persistent single-queue TAP; descriptor 3 and `fd=[3]` in Cloud Hypervisor's network argument; each attach error mapped to the existing `VmmError::TapQueue*` taxonomy; the parent command dropped before any subsequent await; the launch hook remains after fd mapping; no `ip` launch prerequisite or Cloud Hypervisor version gate; and the two scoped FICLONE cleanup corrections. No public API or error variants beyond the accepted design were introduced.

## Findings

No proven defect or design divergence was found. No remediation is required.

The Cargo changes are within the accepted dependency contract. ADR-0129 explicitly requires `command-fds` 0.3.3 with its Tokio extension and records the 0.31.3 `nix` dependency. The bounded premise audit at `docs/feature/netns-density-295/deliver/dependency-premise-audit-05-03.md` establishes that the feature-delta's “no new crate” clause qualifies the separate `libc` relocation, not the expressly approved `command-fds` dependency; it also records why the resolved `libc` update is required by `nix` 0.31.3. This does not contradict or relax an accepted invariant, so no DESIGN premise or roadmap validation is invalidated.

## Contract and implementation evidence

### Queue API, flags, and error taxonomy

- The public `TapQueue` retains the designed private `OwnedFd` and TAP name, with the designed `name`, `as_fd`, and `into_owned_fd` operations in `crates/overdrive-netlink/src/client.rs:253-280`. The helper has the pinned signature, `attach_tap_queue(name: &str) -> Result<TapQueue, TapQueueError>`, at `client.rs:378-379`.
- The helper opens `/dev/net/tun` with `O_RDWR | O_NONBLOCK | O_CLOEXEC`, requests exactly `IFF_TAP | IFF_NO_PI | IFF_VNET_HDR`, and does not request multiqueue (`client.rs:380-407`). It checks the exact persistent flags `0x5802`, reads administrative state, and refuses an up TAP (`client.rs:410-457`). Descriptor ownership is RAII-managed on every refusal path.
- `map_tap_queue_error` maps all six designed `TapQueueError` variants one-for-one to the existing four `VmmError::TapQueue` stages and two `VmmError::TapQueuePostcondition` violations (`crates/overdrive-host/src/vmm.rs:406-432`; taxonomy in `crates/overdrive-core/src/traits/vmm.rs:424-476`). The real-kernel VMM integration body asserts the three producible failures through `Vmm::create`; it explicitly leaves the three kernel-unproducible read/open failures unasserted as DISTILL specifies (`crates/overdrive-host/tests/integration/vmm_tap_queue_errors.rs:1-35, 211-288`).
- The launch argument is exactly `fd=[3],mac=…,offload_tso=off,offload_ufo=off,offload_csum=off`; it no longer names the TAP (`crates/overdrive-host/src/vmm.rs:375-386`). The existing launch-hook registration follows fd mapping, with the close range beginning at descriptor 4 when a queue exists and 3 otherwise (`vmm.rs:584-595`), matching ADR-0129's order and descriptor set.

### Queue lifetime and launch prerequisites

- `CloudHypervisorVmm::create` attaches the queue from the existing `VmNetworkAttachment` and transfers its owned descriptor into the command mapping (`vmm.rs:575-593`). It calls `cmd.spawn()`, then explicitly drops `cmd` before matching the result (`vmm.rs:603-605`). The spawn-failure and no-PID cleanup awaits occur only after that drop (`vmm.rs:607-631`). The successful child therefore owns the inherited queue while the parent command releases its copy at spawn return.
- `REQUIRED_LAUNCH_TOOLS` is exactly `prlimit` and `setpriv` (`vmm.rs:78`); the launch argument builder uses the existing confinement wrapper directly (`vmm.rs:368-386`). The activated S-ND295-44 test pins the stage sequence and proves the `ip` probe is not reached (`vmm.rs:1185-1217`). The accepted user ruling forbids version gating; this change adds no version comparison or gate. Existing Cloud Hypervisor version output remains diagnostic while the existing capability probe checks the required launch capability.
- The activated S-ND295-45 body now reaches a Running guest through the fd handoff and checks the filter state on every Cloud Hypervisor thread (`crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs:1418-1449`).

### FICLONE cleanup

- `ficlone_rootfs` now calls `remove_file` directly, accepts only `NotFound`, and returns other removal failures (`crates/overdrive-host/src/vmm.rs:749-760`). The ioctl-failure path records both the original FICLONE error and any non-`NotFound` cleanup error, including the typed error kind, while preserving the original return classification (`vmm.rs:761-785`).
- The create-time cleanup helper applies the same `NotFound` rule and records other cleanup errors with the path, failure stage, error, and kind (`vmm.rs:435-445`). `ficlone_rootfs` remains within the existing single `spawn_blocking` invocation (`vmm.rs:501-505`); its synchronous filesystem operations and FICLONE ioctl were not split or moved to another execution path.

## Acceptance and test integrity

The only edits to the pre-authored acceptance bodies in the Step 05-03 diff remove their pending-step `#[ignore]` attributes. The diff shows no changed assertions, expectations, fixtures, test names, or test bodies. The activated bodies are exactly S-ND295-38, S-ND295-39, S-ND295-40, S-ND295-44's stage-order body, and S-ND295-45. Their existing Contract Shape declarations remain present; no new property body was added. The real-kernel adapter bodies use scratch TAPs and assert observable kernel state and typed errors; the VMM error body enters through `Vmm::create`; the native spawn test enters through production `create` and observes descriptor release before its cleanup await.

The three source-local behavioral tests in the Step 05-03 diff are the launch-shape, probe-stage-order, and failed-spawn queue-release tests. For the test-budget calculation, the activated bodies cover 12 distinct observable outcomes: successful exact queue attachment; refusal of an up TAP; refusal of an already-held queue; refusal and transient-device removal for an absent name; unprivileged refusal on the root-owned TAP; each of the three kernel-producible `Vmm::create` error mappings; exact fd-3 argument rendering; queue release before the failed-create cleanup await; probe-stage order with no `ip`; and per-thread filter state for a Running guest. Input variants within each outcome remain one behavior. The unit-test budget is 24; three source-local tests are within it. Adapter and native integration bodies remain at their specified boundaries and are not counted as unit tests.

No testing-theater pattern or weakened assertion was found. The DES record preserves the initial failed GREEN attempt and its successful retry in order: RED PASS, GREEN FAIL, GREEN PASS, COMMIT PASS (`docs/feature/netns-density-295/deliver/execution-log.json`, Step 05-03 events at 19:15:15Z, 19:31:04Z, 19:48:11Z, and 19:50:15Z).

## Mechanical evidence

Both commits retain Marcus as author, contain exactly one `Co-Authored-By: Codex <codex@openai.com>` trailer, and carry `Step-Id: 05-03`. The recorded execution-log event sequence is complete and ordered. The roadmap validation is approved (`docs/feature/netns-density-295/deliver/roadmap.json`, `validation.status`). The commit diff passes `git diff --check`.

The following command outcomes were supplied by the DELIVER orchestrator; this reviewer did not rerun the Lima or metal suites:

| Verification | Reported result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-netlink --test integration --features integration-tests -E 'test(tap_queue_attach)' --no-fail-fast` | 3 passed |
| `cargo xtask metal run -- cargo nextest run -p overdrive-host --test integration --features integration-tests,kvm-tests -E 'test(vmm_tap_queue_errors)' --no-fail-fast` | 1 passed |
| `cargo xtask metal run -- cargo nextest run -p overdrive-host --lib --features integration-tests -E 'test(a_failed_spawn_releases_the_queue_before_any_cleanup_await)' --no-fail-fast` | 1 passed |
| `cargo xtask metal run -- cargo nextest run -p overdrive-cli --test integration --features integration-tests,kvm-tests -E 'test(every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters)' --no-fail-fast` | 1 passed |
| `cargo xtask lima run -- cargo nextest run -p overdrive-host --lib -E 'test(vmm)' --no-fail-fast` | 4 passed |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | Passed |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | Passed |
| Changed Rust formatting | Passed, as reported by the orchestrator |

## Contract Shape Compliance

All activated tests retain their pre-authored per-test `CONTRACT_SHAPE` declarations. Source-local pure-function bodies use the exact rustdoc declaration required by the repository. No acceptance body was rewritten to obtain a green result, and no new public API, test-only production seam, descriptor carrier, persisted format, or error variant was added.

## Verdict and disposition

**APPROVED.** Step 05-03 matches its accepted API, descriptor ownership and launch order, error taxonomy, security boundary, cleanup behavior, test dispositions, and dependency contract. There are no findings to remediate; the next roadmap step may proceed under the repository's orchestration sequence.
