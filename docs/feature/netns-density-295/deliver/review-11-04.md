# DELIVER Review — Step 11-04

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `11-04` — RESET-EXIT fork, VMM adapter half, and V-1(c) revalidation |
| Review type | Fresh independent current-source closure review |
| Reviewer | Fresh isolated DELIVER reviewer |
| Review date | 2026-11-01 |
| Implementation boundary | `4aeb1bf447ccd45ac9e287719ae9d7990d7fa30f` |
| Local provisioning amendment | `998098ec0de07e9309cc0bb2ebf888e1477206ad` |
| Seccomp lint-only remediation | `b345f65804065a9759f1154b11aedd2cf132c9c1` |
| DES completion-log persistence | `097ba23b5eaa02c4749b2c5c3e698243128c416f`, with RED disposition correction `b3d7ee118dedee3d0d433968a71e73fd4ebcec69` |
| Current source boundary | `HEAD 00ce922c43b9743651cfda535af006f2393abbe3` |
| Iteration | 1 |
| Verdict | **NEEDS_REVISION — one bounded Contract Shape metadata finding** |

This is the first on-disk review artifact for step 11-04. It records a review
performed against the current source on the date above; it does not claim that
a reviewer executed during the implementation or remediation dates. The
earlier `design-review-11-04-local.md` approved the D21-LOCAL provisioning
amendment only. The whole-slice review correctly identified the absent
per-step artifacts as a mechanical closure gap; it is not treated as a
historical step review here.

No production source, test body, DES log, roadmap, commit, submodule, or other
review artifact was changed. The only authored path is this Markdown file.
The pre-existing dirty `AGENTS.md`, the current roadmap edit, and the other
untracked review artifacts were preserved.

## Authority and review method

The accepted step contract was read from `AGENTS.md`, the 11-04 roadmap entry,
`feature-delta.md`'s VMM backend and local-provisioning contracts, STEP3-SPLIT,
ADR-0129, ADR-0143, ADR-0161, ADR-0182, the D21-LOCAL design review, and the
11-04 DISTILL audits. The current bounded consumers were then checked in
`CloudHypervisorVmm::create`, `VmDriver::start`, `GuestKit`, `SimVmm`, and the
fork source. The review applied the repository development, testing, Rust and
verification rules and the installed `nw-sc-review-dimensions`,
`nw-tdd-review-enforcement`, `nw-tdd-methodology`, and DELIVER review guidance.

The review compared exact public signatures, typed semantic failures,
ownership and reset lifecycle, provisioning refusal behavior, activated test
bodies, Contract Shape declarations, commit scope and retained receipts. No
new native, VM, kernel, Lima, mutation, KASAN, benchmark or host-cleanup run
was started. The retained disposable-production-Cloud-Hypervisor receipts
are the evidence used below.

## Step contract and implementation assessment

| Contract obligation | Current implementation and evidence | Assessment |
|---|---|---|
| D21-LOCAL pins the exact fork source and local artifact identities, with no release publication or stock fallback | `infra/provision/versions.env:11-17` pins repo `overdrive-sh/cloud-hypervisor`, revision `f252bb20b0d141d59e4c15a9ac09c5c2dd7c350d`, Rust `1.89.0`, and the exact x86_64/aarch64 checksums. The Lima parameter block carries the same values. The parent gitlink and checked-out fork both resolve to `f252bb20...`. | PASS |
| Provisioning builds metadata-free source, checksum-gates staged bytes, installs only verified bytes, and is idempotent | `infra/provision/common-user.sh:65-190` and `infra/lima/overdrive-dev.yaml:417-516` fetch the exact revision, archive without `.git`, build into the pinned target directory, strip, verify, stage, verify again, install `/usr/local/bin/cloud-hypervisor`, verify after install, and refuse mismatches. `common-system.sh` removes the stock install path; `infra/metal/provision.sh:207-211` installs the native musl prerequisite. | PASS |
| RESET-EXIT is a fork property and ends the VMM at the first guest reset | `vendors/cloud-hypervisor/vmm/src/lib.rs` at the pinned `f252bb20...` routes `EpollDispatch::Reset` to `vmm_shutdown()` and breaks the VMM loop instead of calling `vm_reboot`. The fork patch introduces no exit-status API. | PASS |
| Exact VMM adapter API and launch shape | The core port remains `Vmm::create(&self, config: &VmConfig, cid: GuestCid) -> Result<VmProcess>` (`crates/overdrive-core/src/traits/vmm.rs:112-161`). `vhost_kernel_vsock_arg` is private and renders exactly `cid=<leased>,backend=vhost-kernel` (`crates/overdrive-host/src/vmm.rs:368-375`). `SimVmm` implements the same port (`crates/overdrive-sim/src/adapters/vmm.rs:496-517`). No public method, type, enum variant, parameter, wire field, persisted field or probe stage was added. | PASS |
| STEP3-SPLIT leaves the caller half to 12-01 | The current bounded production caller passes `transport.cid()` at `crates/overdrive-worker/src/vm_driver.rs:1407`. Step 11-04 only supplies the adapter half; it does not pull `AllocationSpec.transport`, `compose_vm_transport`, or the `launch=` token into this step. | PASS |
| The VMM alone opens `/dev/vhost-vsock` and owns the device lifecycle | `CloudHypervisorVmm::create` builds the command with `--vsock`, CH seccomp, Landlock and no descriptor handoff (`crates/overdrive-host/src/vmm.rs:262-310, 532-540`). The fork's `VsockBackend::VhostKernel` branch grants only `/dev/vhost-vsock` (`vendors/cloud-hypervisor/vmm/src/vm_config.rs:898-913`). The device/CID cells assert one VMM-held descriptor, no Overdrive copy, immediate CID reuse and held-CID refusal. | PASS |
| RESET-EXIT covers the bounded fork/device matrix | The activated `vsock_vmm_device` bodies cover pre-READY PID-1 panic, triple fault, forced reboot, and the approved post-boot `sysrq-c`/`sysrq-b` matrix with a live host flow, `ECONNRESET`, one boot and CID reuse (`crates/overdrive-host/tests/integration/vsock_vmm_device.rs:511-593, 595-680`). The lower-level body deliberately does not claim production `READY` or driver classification; the full composition classification remains assigned to the attachment-cut and later end-to-end evidence. | PASS within the step boundary |
| V-1(c) proves the full production launch identity against the fork | The approved body drives the real `CloudHypervisorVmm::create` path and checks uid/gid `4200:4200`, device groups, no passwd entry, one device descriptor, exact `backend=vhost-kernel`, CH seccomp, Landlock, `NoNewPrivs=1`, seccomp mode `2` on every observed VMM task, data round-trip and clean poweroff (`vsock_vmm_device.rs:396-509`). | PASS |
| ADR-0143 remains fail-closed and denies the TAP-mutating ioctl set | `VmmLaunchSeccompFilter::for_target` remains x86_64-only and returns the typed unsupported-architecture value on other targets (`crates/overdrive-host/src/vmm/launch_seccomp.rs:26-51`). The exact 13-request deny-list, foreign-ABI kill behavior and hook ordering remain intact; the bounded lint remediation only adds the cfg-dependent `clippy::unnecessary_wraps` allowance at `:33-36`. | PASS |
| Existing typed VMM error boundary is preserved | The design requires no vsock-specific error. Missing rootfs, absent binary, confinement setup, unsupported launch architecture and probe failures retain `VmmError`/`VmmProbeError` representations; a device-open or CID refusal remains a pre-READY VMM exit projected through the existing driver failure path. | PASS |

The approved D21-LOCAL amendment is therefore implemented as a bounded
substitution over the existing provisioning owners. The implementation does
not introduce a runtime fork-identification probe, release service, second
`/dev/vhost-vsock` holder, or new application API.

## Exact API and typed-error mapping

The cross-crate interface is unchanged except for the already-approved use of
the leased CID at the existing `Vmm::create` boundary. The exact interface is:

```rust
async fn create(&self, config: &VmConfig, cid: GuestCid) -> Result<VmProcess>;
```

The adapter renders one `--vsock` value, `cid=<cid>,backend=vhost-kernel`; the
VMM opens and configures the kernel device. `VmDriver` remains the owner that
passes the transport assignment's CID. No implementation added a second
launcher method, fd handoff, `VmmProbeError` fork variant or caller-side
device operation.

The designed failure mappings are complete at the relevant boundaries:

| Designed failure | Approved and implemented representation |
|---|---|
| No x86_64 launch-filter program | `VmmProbeError::LaunchSeccompUnsupportedArch` from probe; `VmmError::ConfinementUnavailable { control: Seccomp, .. }` from `create` before any launch effect. |
| Kernel refuses the launch filter or the pre-exec hook fails | Existing `VmmProbeError::LaunchSeccompInstall` / `LaunchSeccompProbeExit` for the probe; the existing spawn/confinement error path for `create`. No new semantic variant is added. |
| Rootfs master missing | `VmmError::RootfsNotFound { path, source }`. |
| Cloud Hypervisor binary absent | `VmmError::HypervisorAbsent { searched, source }`. |
| Confined artifact preparation fails | `VmmError::ConfinementUnavailable { control: UidDrop, detail }`; no unconfined fallback. |
| VMM cannot open `/dev/vhost-vsock` or the CID is held | VMM exits before READY; the existing driver projects the no-guest-report exit as `VmGuestExitUnreported`/the existing post-READY crash classification. No caller branches on Cloud Hypervisor diagnostic text and no vsock-specific `VmmError` is introduced. |
| Local artifact source/build/stage/install/checksum failure | The established shell provisioning interface returns nonzero with a diagnostic and refuses installation. D21-LOCAL expressly does not add an application typed error or `VmmProbeError` fork variant. |

The existing `VmmProbeError` set remains the designed capability surface:
Landlock absence, KVM reachability, run-root, launch-tool and launch-seccomp
failures retain their approved variants. The checksum is the provisioning
identity check; `Vmm::probe` adds no stage and does not identify the fork at
runtime.

## Production path, ownership and lifecycle

`CloudHypervisorVmm::create` first builds the target-specific launch filter,
checks the rootfs master and creates the per-launch FICLONE clone. It then
checks the resolved Cloud Hypervisor path, prepares the clone/kernel/run
directory for the configured identity, builds the existing
`prlimit -> setpriv -> cloud-hypervisor` wrapper, appends exactly one kernel
vhost-vsock argument, registers the child hook, and spawns the VMM. The hook
closes descriptors at or above stderr, sets `PR_SET_NO_NEW_PRIVS`, installs the
classic-BPF filter before the first exec, and leaves the reaper as the sole
`wait` owner. The reaper publishes one `VmmExit` and removes the live-process
entry after the child exits.

The fork's reset dispatch exits the VMM process. Its last device close then
releases the CID and resets host vsock flows. The lower-level reset test checks
this ordering directly through the production VMM port. The production VM
driver's existing caller and exit classification remain downstream consumers;
the step does not invent a reset supervisor or a new exit-status contract.

The local provisioning ownership is also bounded: `versions.env` owns the
metal pins, the existing Lima parameter block mirrors them, `common-system.sh`
removes the stock path, `common-user.sh` owns metal's unprivileged build and
install, and Lima's existing user block owns its aarch64 compile/install
surface. No release tag, GitHub release, uploaded binary or alternate stock
source is introduced.

## Evidence reviewed

The following are retained execution receipts. They are evidence from the
recorded runs and are not claims that this reviewer performed those runs:

| Evidence | Result and boundary |
|---|---|
| Semantic launch-argument RED | `.context/netns-density-295/11-04/red-launch-argument.log` records the old `vhost_kernel_vsock_arg` RED scaffold panicking in the focused unit test. |
| Semantic RESET-EXIT RED | `.context/netns-density-295/11-04/metal-reset-exit-red-baseline.log` records the approved post-boot reset body failing against a pre-fix Cloud Hypervisor build at the first-reset assertion. |
| Canonical native VMM/device selection | `metal-vsock-vmm-device-v1c-final.log` reports 8/8 passed, 28 skipped, on native x86_64 metal with the installed pinned artifact. This includes the approved post-boot reset and V-1(c) bodies. |
| Native VMM/host-state equivalence | `metal-vmm-host-equivalence-installed-final.log` reports 4/4 passed, 31 skipped. |
| Native launch-filter/TAP ioctl revalidation | `metal-launch-seccomp-tap-ioctl.log` reports 4/4 passed. |
| Sim VMM equivalence | `metal-vmm-sim-equivalence.log` reports 2/2 passed; the Sim device cells in `lima-sim-vsock-device.log` report 4/4 passed. |
| Pure launch argument | `lima-vsock-argument-green.log` reports the exact argument test passed 1/1. |
| Production identity receipt | `metal-v1c-production-identity-installed.log` records kernel `7.0.0-38-generic`, uid/gid `4200:4200`, groups `[991]`, one vhost descriptor, ten observed VMM tasks with `NoNewPrivs=1` and seccomp mode `2`, exact `cid=16778246,backend=vhost-kernel`, data round-trip port `46245`, and a passing 1/1 selection. |
| Local artifact identities | `metal-installed-sha256-final*.log` and `lima-installed-sha256-final*.log` report `OK` for the pinned x86_64 and aarch64 hashes. The local provisioning evidence records first-run and idempotent reuse, metadata-free source checks, and no release publication. |
| Workspace gates | The retained Lima format, check and clippy receipts pass; `des-step-validation-final.log` reports the latest RED/GREEN/COMMIT phase values as PASS and `validation=PASS`. |

The attempted Lima VMM-equivalence receipt
(`lima-vmm-sim-equivalence.log`) failed while `VmFixture::provision` rejected
the nested Apple host. This is expected non-signal for the real KVM/Cloud
Hypervisor lane and is not counted as a green acceptance result. The native
metal Sim and real-adapter receipts are the relevant execution evidence.

The execution log records the implementation chronology as RED → GREEN →
COMMIT, a later exact RED PASS disposition for the approved reset body, and a
lint-only remediation with RED SKIPPED → GREEN → COMMIT. Those DES events are
mechanical implementation evidence; they are not a historical reviewer
verdict.

## Test integrity and boundary review

The approved V-1(c) and post-boot reset bodies were authored by DISTILL and
explicitly approved before the crafter activated them. The test-support
identity helper is private and derives the actual device groups; it does not
add a production API. The native tests drive the real `CloudHypervisorVmm`
port and use a disposable production Cloud Hypervisor/GuestKit guest. The Sim
tests exercise the Sim adapter through the same `Vmm` port. No test invokes a
production binary as a Rust expectation runner, mocks the SUT, or asserts only
call counts. The provisioning checks are black-box shell evidence and do not
link an Overdrive crate.

No assertion was weakened, deleted or skipped to make the implementation pass.
The main step diff removes the intended pending markers, adds the two
explicitly approved DISTILL bodies and applies the approved test-local
identity correction. The current native receipts prove the real launch,
device, reset, CID, identity and cleanup outcomes at their stated boundaries.

### Contract Shape Compliance

The activated `vsock_vmm_device` cells and the four Sim adapter cells retain
exact `/// CONTRACT_SHAPE:` declarations. The newly approved V-1(c) and
post-boot reset cells use `/// CONTRACT_SHAPE: bounded-change.`.

One mechanical defect remains in the four additional tests transitioned by
the step's marker removals:

| Transitioned test | Current declaration | Required disposition |
|---|---|---|
| `vmm_equivalence_cloud_hypervisor_vmm` (`vmm_equivalence.rs:189-190`) | No declaration | Add exact `/// CONTRACT_SHAPE: bounded-change.`. |
| `vmm_live_console_tail_is_coherent_with_final_exit_stderr_tail_for_both_adapters` (`vmm_equivalence.rs:316-325`) | Lowercase prose `` `@contract-shape:pure-function` ``; it launches Sim and real Cloud Hypervisor processes | Replace with exact `/// CONTRACT_SHAPE: bounded-change.`; this is an integration behavior, not a pure function. |
| `vmm_early_exit_preserves_code_signal_and_stderr_tail_for_both_adapters` (`vmm_equivalence.rs:374-378`) | Lowercase prose `` `@contract-shape:bounded-change` `` | Replace with exact `/// CONTRACT_SHAPE: bounded-change.`. |
| `vm_host_state_equivalence_real` (`vm_host_state_equivalence.rs:172-174`) | No declaration | Add exact `/// CONTRACT_SHAPE: bounded-change.`. |

The repository-mandated checker path is absent from this checkout and the
installed nWave tree, so this result is a direct diff-scoped source audit. The
finding is independent of the retained green receipts: the tests are live
after 4aeb1bf44, and their metadata does not satisfy the current repository
contract. No outcome anchor finding is raised for these integration examples;
the accepted anchor rule applies to acceptance bodies, while these are
single-example adapter/infrastructure integration tests.

## Findings and dispositions

### D1 — Four transitioned integration tests lack the exact Contract Shape declaration

- **Severity:** Blocker under the repository Contract Shape gate.
- **Dimension:** Test integrity / mechanical Contract Shape compliance.
- **Reachability:** The four tests were previously guarded by
  `#[ignore = "pending DELIVER step 3"]` and the step commit removed those
  guards. They are therefore live transitioned tests in the current test
  surface; this does not depend on an imagined runtime schedule.
- **Evidence:** `git diff 4aeb1bf44^ 4aeb1bf44` shows the four marker removals;
  current source lines are listed in the table above. Three bodies have no
  exact machine-read declaration and two retain stale lowercase prose. The
  live console body is also mislabeled `pure-function` despite creating and
  observing real VMM processes.
- **Impact:** The implementation behavior and native receipts remain valid,
  but the step's transitioned test evidence is not mechanically classifiable
  under the repository contract. This blocks an unqualified review approval.
- **Required remediation:** Add or replace only the rustdoc metadata lines
  shown in the table. Preserve the test bodies, assertions, fixtures,
  selectors and production code. Re-run the scoped Contract Shape source audit
  and record the bounded remediation through the original step crafter's
  review cycle. No new API, test lane, production seam or test scenario is
  required.
- **Disposition:** OPEN; blocks approval pending the original crafter's
  metadata-only remediation and this step's re-review.

No production defect, typed-error gap, API divergence, lifecycle failure,
test-theater pattern, test weakening, provisioning identity mismatch or
unproven architectural remedy was found. The driver classification is not
claimed as a STEP 11-04 lower-level result; its full composition evidence
remains assigned to the approved later cut and end-to-end steps.

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — accepted step boundary | PASS | The one VMM adapter/provisioning step and its approved STEP3-SPLIT ownership are preserved. |
| G2 — semantic RED | PASS | Launch-argument and pre-fix RESET-EXIT receipts fail for the intended behavior, not collection or syntax. |
| G3 — assertion failure | PASS | The retained RED receipts identify the intended assertion failures. |
| G4 — no domain mocks / port boundary | PASS | Native cells drive `CloudHypervisorVmm::create`; Sim cells use `Vmm`; no SUT mock or expectation runner is used. |
| G5 — domain language | PASS | Tests name CID ownership, reset, VMM/device lifecycle, production identity and adapter equivalence. |
| G6 — green required evidence | PASS | Canonical native selection 8/8, host equivalence 4/4, launch-filter 4/4, Sim selections 2/2 and 4/4; Lima check/clippy/format pass. The nested-host Lima integration attempt is excluded as non-signal. |
| G7 — green before commit | PASS | DES records RED/GREEN/COMMIT PASS for the implementation and the bounded lint remediation. |
| G8 — scoped test budget | PASS | This is infrastructure/adapter integration and Tier-3 evidence; the roadmap explicitly exempts the native reset/provisioning cells from property-test authoring. No new unit-test explosion was introduced. |
| G9 — no test weakening | PASS | No assertion was weakened or removed; approved DISTILL additions and marker activation are preserved. |
| Contract Shape gate | **FAIL** | D1: four transitioned integration tests lack exact declarations. |

## Iteration 1 verdict

**NEEDS_REVISION — test metadata only.** The current production implementation,
exact API/error contract, RESET-EXIT fork, local provisioning, production
identity confinement, device ownership, lifecycle ordering and retained native
evidence satisfy the accepted step boundary. Approval is blocked solely by
the four transitioned integration tests that do not carry the exact current
Contract Shape declaration. This finding requires a metadata-only remediation;
it does not authorize production behavior changes, new APIs, new tests or an
architectural redesign.

The artifact records no historical reviewer approval. A subsequent iteration
must verify the four declaration lines against the current source and then
replace this verdict only after the original crafter's bounded remediation is
present.

## Iteration 2 — remediation re-review

This current-source re-review checked the remediation diff after the original
step crafter added only the required rustdoc Contract Shape lines. The diff is
bounded to:

- `vmm_equivalence.rs`: three exact `/// CONTRACT_SHAPE: bounded-change.`
  declarations, with the two stale lowercase prose tags replaced;
- `vm_host_state_equivalence.rs`: one exact
  `/// CONTRACT_SHAPE: bounded-change.` declaration above the real-host body.

`git diff --check` passes for both files. The remediation changes no executable
statement, assertion, fixture, identity derivation, scheduling, production
source, public API, DES event, roadmap entry or commit. The unrelated dirty
`vm_beacon_roundtrip.rs`, `AGENTS.md`, roadmap edit and parallel review
artifacts remain outside this review and were preserved.

The four formerly open tests now have the correct classifications:

| Test | Current classification | Re-review result |
|---|---|---|
| `vmm_equivalence_cloud_hypervisor_vmm` | `bounded-change` | PASS |
| `vmm_live_console_tail_is_coherent_with_final_exit_stderr_tail_for_both_adapters` | `bounded-change`; it observes Sim and real VMM process effects | PASS |
| `vmm_early_exit_preserves_code_signal_and_stderr_tail_for_both_adapters` | `bounded-change`; it observes terminal process diagnostics | PASS |
| `vm_host_state_equivalence_real` | `bounded-change`; it observes real cgroup/run-directory/clone lifecycle | PASS |

A direct source scan now finds the exact declaration above each transitioned
body and no stale lowercase `@contract-shape` tag in either remediation file.
The original test bodies and all previously reviewed behavioral assertions are
unchanged. No test or native run was repeated because this remediation is
metadata-only and the user explicitly bounded this re-review to classification
and diff scope.

### Remediation disposition

D1 is **CLOSED**. The Contract Shape gate now passes for all four tests
transitioned by step 11-04, with integration behavior classified as
`bounded-change` rather than `pure-function`.

## Final iteration 2 verdict

**APPROVED.** The metadata-only remediation closes the sole iteration-1
finding. The current implementation remains aligned with the exact API and
typed-error contract, RESET-EXIT fork lifecycle, local provisioning identity,
production launch confinement, device ownership, and retained native evidence
documented above. This approval is a review performed against current source;
it does not fabricate a historical reviewer execution or alter the retained
DES chronology.
