# Adversarial review — step 02-03

- **Feature:** `guest-stack-transparent-mtls-intercept`
- **Step:** `02-03` — universal metal qualification and pre-READY lifecycle closure
- **Reviewer:** `nw-software-crafter-reviewer` (fresh isolated reviewer)
- **Review ID:** `code_rev_20260829_02_03_iteration_1`
- **Iteration:** 1
- **Reviewed commit:** `9bf4881529f9bf808573eb50f51cfdf546f2e456`
- **Parent:** `eb53c11d213eafa90ae0506a265d945f62630f95`
- **Subject:** `feat(guest-stack-mtls): close pre-ready lifecycle and qualify metal`
- **Trailer:** `Step-Id: 02-03`
- **Verdict:** **NEEDS_REVISION**

## Executive summary

The commit is not safe to advance. The guest's production path now reads
`/sys/class/net/<iface>/flags`, but PID 1 mounts only procfs and the canonical
rootfs contains an empty `/sys` mountpoint. A qualified native-metal boot of
the reviewed commit therefore failed before READY. Independently, a guest that
connects vsock and then powers off before READY is consumed by the biased
Beacon-error arm and recorded as `DriverInternalError`, not the required
`VmGuestExitUnreported { Option<i32>, signal }`; the native walking-skeleton
test reproduced that exact incorrect terminal reason.

The green unit suite does not protect these paths. Most new guest-init tests
inject one generic `GuestNetworkConfig` error through a fake adapter regardless
of the exact variant required by DISTILL, the closed-error test constructs its
own accepted list and asks a matching predicate to accept it, and the declared
pure properties use picked examples instead of the required generated input
domains. New acceptance tests also omit the mandatory exact Outcome Anchor.
This is testing theater at the step's defining boundary.

The universal lease implementation substantially improves shared-host
serialization, and its holder mechanics work on Linux. It still writes the
remote helper with `scp` before acquiring the lease, while the regression test
does not invoke Run, Sync, or bootstrap and its no-mutation sentinel is never
connected to a writer. Native qualification also omits the ratified cgroup-v2
and artifact checks. Finally, the C3 executable does not resolve at the
roadmap's exact file locator and remains a rename of the prior partial test
rather than a complete second-converge snapshot.

Formatting, bash syntax, workspace check/clippy, the 2,265-test default suite,
and focused source/component tests pass on the immutable target in Lima. Those
green results do not supersede the two failed native guest executions or the
test-integrity defects. No mutation testing was run.

## Review scope and immutable evidence

The implementation and all verification commands were evaluated from a clean,
detached worktree at the exact reviewed commit. Later uncommitted 02-04+ source,
configuration, and test work in the Conductor workspace was excluded. The
review covered:

- the approved roadmap and all 39 step-owned executable mappings;
- the approved DESIGN and DISTILL Q7/native-metal contracts and their final
  review dispositions;
- the complete target diff, including guest init, lifecycle classification,
  action-shim gating, C3 replay, the metal bootstrap/lease/preflight, and tests;
- the canonical rootfs builder and `VmDriver` boot-race path needed to validate
  production reachability; and
- fresh Lima and native-metal executions from the immutable target.

This reviewer changed only this Markdown review artifact. Source and tests were
read only. Temporary detached review worktrees were used so later dirty work
could not influence the result.

## Defect counts

| Severity | Count |
|---|---:|
| Blocker | 4 |
| Critical | 0 |
| High | 1 |
| Medium | 2 |
| Low | 1 |

## Mechanical evidence

### Commit scope and DES discipline

| Check | Result | Evidence |
|---|---|---|
| Exact parent | PASS | `9bf48815^` is `eb53c11d`. |
| Commit scope | PASS | 12 files, 1,385 insertions, 173 deletions; no later dirty 02-04+ work is present. |
| Trailer | PASS | Exact `Step-Id: 02-03`. |
| Diff whitespace | PASS | `git diff --check eb53c11d..9bf48815`. |
| Fresh DES cycle | PASS | Fresh RED `02:58:51Z`, GREEN `03:12:19Z`, COMMIT `03:12:44Z`; all `EXECUTED/PASS`, chronological. |
| Earlier interrupted RED | EXCLUDED | The `20:31:20Z` RED predates the fresh isolated cycle and supplies no inherited GREEN/COMMIT claim. |
| Mutation discipline | PASS | No mutation command or exclusion change in this step. |

The commit timestamp is exactly the fresh COMMIT event timestamp. Necessary
scope expansion into the existing natural-exit acceptance surface is tightly
related. No Beacon, describe, observation, persistence, REST/OpenAPI, or public
Driver shape changes are in the commit.

### Test budget

The approved roadmap maps 39 distinct behaviors to 02-03: 21 bounded-change
and 18 pure-function contracts. The target has one mapped test per identity
plus a second half of `C-GTI-08-RECONCILE` at the reconciler boundary, for 40
step-owned source/component tests. Parametrized/proptest bodies count once.

| Behaviors | Budget (`2 × behaviors`) | Actual tests | Result |
|---:|---:|---:|---|
| 39 | 78 | 40 | PASS |

The budget passes. D3 and D4 concern the honesty and completeness of those
tests, not their count.

## Contract Shape Compliance

**Overall: FAIL.**

| Check | Result | Evidence |
|---|---|---|
| Per-test declaration | PASS mechanically for mapped pure properties | All 18 mapped pure-function identities have the exact `/// CONTRACT_SHAPE: pure-function.` line at their live function. |
| Exact executable locator | **FAIL** | `C-GTI-C3-CONVERGE-TWICE` is absent from the mapped `veth_provision_idempotent.rs`; it exists only in `alloc_netns_lifecycle.rs`. |
| Exact Outcome Anchor | **BLOCKER** | Both newly added acceptance tests named `unreported_pre_ready_vmm_exit_finalizes_once_without_restart_or_view_change` omit `Outcome anchor: DISCUSS Elevator Pitch`. |
| Banned test-name regex | PASS | No new/transitioned test matches the banned implementation-detail expression. |
| Pure-function property mechanism | **FAIL** | The mapped token/NIC/readback/closure tests are picked examples or hand-built values, despite DISTILL requiring arbitrary malformed bytes, field boundaries, NIC flags, readbacks, and every sanctioned stage. |
| Bounded-change delta and complement | **FAIL** | Guest-init stage tests do not assert the exact typed delta plus the complete no-later-effect universe; C3 does not compare a full before/after kernel snapshot. |
| Layer choice | **FAIL for the admission test** | `network_admission_rejects_an_interface_that_is_already_up` injects a fake error and never exercises the production flags decision or an extracted pure classifier. |

The mechanical declaration spelling is necessary but not sufficient. The
semantic Contract Shape failures are blocking because the tests remain green
under regressions in the exact error variants, the real NIC-up classifier, and
the full pre-READY suppression boundary.

## Findings

### D1 — the canonical minimal guest can never pass the new NIC-down check

- **Severity:** Blocker
- **Dimension:** External validity, production reachability, and fail-closed guest ordering
- **Locations:**
  - `crates/overdrive-init/src/main.rs:144-152,191-205,508-522`
  - `crates/overdrive-testing/src/vm_fixture.rs:756-805`

`overdrive-init` creates only `proc` and `etc`, mounts procfs, and then reaches
the guest network path. `LinuxGuestNetworkOps::require_down` reads
`/sys/class/net/<interface>/flags`. The canonical ext4 fixture creates an empty
`sys` directory but mounts nothing there, and no other pre-init component
exists: this binary is PID 1. Consequently the flags read returns `ENOENT` on
the production rootfs, every correctly tokenized VM powers off before READY,
and no successful VM Job can reach the intercept or EXEC gate.

This is externally observable, not hypothetical. On the qualified native host,
the immutable target's existing real-VM walking skeleton
`vm_workload_runs_to_completion_and_exit_code_reaches_operator` reached final
`Failed` instead of `Terminated`; the guest connected but closed before sending
READY. The real mTLS guest scenario subsequently timed out waiting for the
allocation rule that can only be installed after `Driver::start` accepts READY.

**Required remediation:** read `IFF_UP` through a production-reachable ioctl or
netlink boundary that does not require sysfs, reusing the existing typed socket
and flags support where appropriate. If mounting sysfs is instead selected, it
is a new typed minimal-root stage and must be reconciled into the approved
closed error set and ordering contract before implementation. Add a real
canonical-rootfs boot regression that reaches READY and proves the NIC remained
silent beforehand.

### D2 — post-connect pre-READY shutdown is classified by the wrong host arm

- **Severity:** Blocker
- **Dimension:** Lifecycle correctness, exact cause preservation, and protocol race handling
- **Locations:**
  - `crates/overdrive-worker/src/vm_driver.rs:1170-1180,1264-1277,1330-1344`
  - `crates/overdrive-init/src/main.rs:146-156`
  - `crates/overdrive-reconcilers/src/workload_lifecycle.rs:1450-1537`

The corrected guest order deliberately connects vsock before token,
suppression, static apply, resolver, and READY. A failure in any of those later
stages closes the accepted session and powers off. `VmDriver::start`, however,
races `accept_ready` against the VMM exit with a biased Beacon-first branch. EOF
or an empty line resolves `BootRaceOutcome::Beacon(Err(_))`, which performs
cleanup and calls `start_rejected_unclassified`; it does not consume the VMM
exit watch or produce `VmGuestExitUnreported`.

The native walking-skeleton execution reproduced the exact defect:

`Failed (reason=DriverInternalError { detail: "beacon accept failed: beacon accept: unparseable first line: empty beacon line" })`.

Therefore the new total classifier and action-shim finalization tests are not
on the production path for token/network/READY failures. Exact `Option<i32>`
preservation, `FinalizeFailed`, no restart, and durable count preservation can
all be correct in isolation while the live guest records a different reason.

**Required remediation:** keep the boot race active through receipt of a valid
READY, or make EOF/session failure before READY await and consume the resolved
VMM exit before classifying the start rejection. Add a controlled regression
where the guest connection is accepted, then closes before READY, and prove the
exact VMM code/signal reaches `VmGuestExitUnreported` and the Job finalization
path without `DriverInternalError` or restart.

### D3 — the guest-init error matrix is testing a fake error, not the ratified typed partitions

- **Severity:** Blocker
- **Dimension:** Test integrity, assertion quality, PBT enforcement, and complete error closure
- **Locations:**
  - `crates/overdrive-init/src/main.rs:1037-1179`
  - `crates/overdrive-init/src/main.rs:1271-1337,1348-1515`
  - `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md:381-427`

`FakeNetworkOps::hit` returns `InitError::GuestNetworkConfig` for every injected
stage. The mapped enumeration, IPv6/ARP write, ioctl-socket, address, netmask,
link, route, and resolver tests then assert only `is_err()` and that the last
visited enum equals the injected stage. DISTILL requires exact
`GuestNetworkSyscall` or `GuestNetworkIo` variants for those cases. The tests
would stay green if production returned the wrong error type for every one of
them.

The remaining closure is similarly non-falsifiable:

- `every_sanctioned_pre_ready_failure_maps_to_the_closed_init_error_set`
  manually constructs the ten accepted variants and passes them to a predicate
  written from the same ten-variant list; a new variant, a missing production
  stage, or an incorrect stage-to-variant mapping does not fail it;
- `ready_send_failure_is_pre_ready_and_suppresses_exec` injects
  `GuestNetworkConfig`, never drives a beacon write, and never observes EXEC;
- root/module/socket tests stop at their immediate helper and do not prove the
  complete later READY/EXEC/operator complement through
  `complete_pre_ready_init`;
- the connect case performs one synthetic attempt, not exhaustion of the
  bounded production retry loop;
- mapped pure token/admission/readback tests use a handful of literals rather
  than the arbitrary bytes, field boundaries, NIC flags, and readback domains
  required by DISTILL and the repository PBT mandate; and
- `network_admission_rejects_an_interface_that_is_already_up` never calls the
  production admission decision at all.

These are false-green tests at the exact functionality introduced by the step.

**Required remediation:** drive every forced stage through one complete
pre-READY orchestration harness, return and assert its exact sanctioned
`InitError` variant, and compare the full later-effect trace including READY,
EXEC, and operator execution. Make the closed-set classifier exhaustive and
mutation-sensitive. Use proptest for malformed token bytes/field partitions,
NIC flag values, and readback domains. Extract a pure admission decision if
needed rather than injecting the expected rejection from the fake adapter.

### D4 — newly added acceptance tests omit the mandatory Outcome Anchor

- **Severity:** Blocker
- **Dimension:** Mechanical Contract Shape compliance
- **Locations:**
  - `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:451-474`
  - `crates/overdrive-core/tests/acceptance/workload_lifecycle_natural_exit.rs:335-380`

Both newly added acceptance tests named
`unreported_pre_ready_vmm_exit_finalizes_once_without_restart_or_view_change`
have a Contract Shape line but no exact
`Outcome anchor: DISCUSS Elevator Pitch` line. The reviewer mandate makes this
a mechanical block for new acceptance tests. Their bounded-change bodies also
assert selected fields rather than one full declared durable-row/action/View
universe, which compounds D3's semantic shape failure.

**Required remediation:** add the exact Outcome Anchor to both acceptance
tests and express the complete bounded delta/complement: exact lifecycle action
set, unchanged View, no restart, one persisted successor, retained durable
count, and equality of every non-permitted row field.

### D5 — the C3 executable does not resolve at its approved locator and remains partial

- **Severity:** High
- **Dimension:** Roadmap traceability, bounded-change completeness, and immutable-baseline honesty
- **Locations:**
  - `docs/feature/guest-stack-transparent-mtls-intercept/deliver/roadmap.json:569-574`
  - `crates/overdrive-control-plane/tests/integration/alloc_netns_lifecycle.rs:636-902`
  - `docs/feature/guest-stack-transparent-mtls-intercept/distill/red-classification.md:66`

The approved executable map requires
`crates/overdrive-control-plane/tests/integration/veth_provision_idempotent.rs::c3_converge_twice_preserves_the_same_vm_network_plan`.
That pair does not exist in the target. The function exists only in
`alloc_netns_lifecycle.rs`, where the commit merely renames the prior
`vm_c3_converges_persistent_tap_repairs_drift_and_tears_down_without_residue`
body.

The immutable DISTILL baseline classified the old coverage incomplete because
no one example proved the complete C3 kernel delta twice. The renamed body
compares only the TAP ifindex across the clean second converge. It does not
snapshot and compare the netns inode, host/workload-veth identities, complete
address state, route, forwarding state, and complement universe. Hidden
recreation or adjacent kernel mutation can therefore pass.

**Required remediation:** land the executable at the approved file/function
locator, or obtain a reviewed roadmap correction before changing that locator.
Use one complete before/after C3 snapshot for the exact allocation-scoped
netns/veth/TAP/address/route/forwarding universe and prove equality on the
second converge. Do not count a rename of the immutable partial test as the
newly complete obligation.

### D6 — canonical metal tests do not execute the boundaries they claim to prove

- **Severity:** Medium
- **Dimension:** External validity, concurrency proof, and no-mutation timeout semantics
- **Locations:**
  - `infra/metal/bootstrap.sh:155-207`
  - `xtask/src/main.rs:1289-1388`

The holder test correctly proves `flock`, metadata publication, timeout, signal
cleanup, and reacquisition on Linux. It does not invoke `MetalAction::Run`,
`MetalAction::Sync`, or any bootstrap mode. Their participation is asserted by
searching source strings. The `must-not-exist` mutation path is never passed to
the holder or a writer, so its absence is tautological and cannot prove that a
timed-out canonical command performs no mutation.

There is also a real pre-acquisition write: bootstrap copies
`lease-holder.sh` to remote `/tmp` with `scp` before the remote process obtains
the lock. The approved DISTILL contract requires acquisition before the first
remote-tree mutation and says a timeout performs no mutation. The unique path
limits collision risk, but it does not satisfy that exact ownership boundary.

**Required remediation:** make the lease helper available without a
pre-acquisition remote write (for example, through a provisioned canonical
helper or a descriptor-preserving streamed launcher), and add an executable
operation-order harness for each Run/Sync/supported-bootstrap route. Drive real
timeout and signal outcomes and assert that no shared or temporary remote write
occurs before acknowledgement.

### D7 — native qualification omits ratified prerequisites and tests source text

- **Severity:** Medium
- **Dimension:** Native evidence validity and fail-closed preflight completeness
- **Locations:**
  - `infra/metal/bootstrap.sh:313-359`
  - `xtask/src/main.rs:1370-1388`
  - `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md:160-174`

The Run preflight correctly checks architecture, literal non-virtualization,
hypervisor flags, vmx/svm, character-device KVM, open permission, API version
12, and create/close VM fd. It does not check cgroup v2, Cloud Hypervisor, the
kernel, or the selected rootfs, all of which the ratified native preflight lists
as mandatory fail-closed prerequisites. A canonical `cargo xtask metal run --
true` therefore certifies less than the approved native execution surface.

The mapped test cannot catch behavioral drift: it reads `bootstrap.sh` and
looks for diagnostic strings and `0xAE01`. Dead code or comments satisfy it,
and there are no missing/virtualized/permission/API/create-VM failure
executions.

**Required remediation:** complete the cgroup/artifact gates at the canonical
boundary (with selected-artifact inputs where necessary), and test the
executable preflight over every missing, contradictory, permission, API, and VM
creation partition. Source substring assertions may remain only as a lint, not
as the mapped bounded-change contract.

### D8 — xtask still tells operators that the native runner requires nested KVM

- **Severity:** Low
- **Dimension:** Operator guidance and approved substrate coherence
- **Locations:** `xtask/src/main.rs:104-115,680-694`

The command help and target-resolution docs still say the box requires
`x86_64 + nested KVM`. The effective DESIGN/DISTILL contract and the new
preflight explicitly reject nested or otherwise virtualized hosts. The runtime
fails safely, but the user-facing guidance sends operators toward an invalid
substrate.

**Required remediation:** replace the remaining nested-KVM wording with native,
non-virtualized x86_64 hardware KVM while retaining `kvm-tests` only as the
Cargo feature name.

## External validity

**FAIL.**

The host-global lock and x86_64/KVM probes execute on the designated native
host, but the feature-defining guest path does not. The reviewed commit's real
VM walking skeleton failed in 2.11 seconds with `DriverInternalError` after an
empty pre-READY beacon line, and the existing guest-stack mTLS scenario timed
out after 120 seconds without observing the target allocation rule. These are
production entry-point executions through the real metal runner, Cloud
Hypervisor, canonical rootfs, and guest-init binary.

The default and focused green suites remain useful compilation/regression
evidence, but most Q7 failure tests terminate at private injected helpers and
cannot replace the failing driving-port result.

## Quality gates

| Gate | Result | Evidence |
|---|---|---|
| G1 — selected acceptance slice | **FAIL** | Qualified native guest executions fail before the required lifecycle/intercept outcome. |
| G2 — valid RED | PASS mechanically | Fresh isolated RED is logged before GREEN; no inherited RED is claimed. |
| G3 — assertion failure quality | **FAIL** | D3, D6, and D7 identify false-green/tautological assertions. |
| G4 — no domain mocks | PASS narrowly | Fakes sit behind private I/O seams; the problem is that several tests assert the fake's injected answer instead of the production decision. |
| G5 — business language | PASS | Test names use stable domain outcomes; D4 is the separate mechanical anchor failure. |
| G6 — all in-scope tests green | **FAIL** | Default/focused suites pass, but two required native guest executions fail. |
| G7 — GREEN before COMMIT | PASS | Fresh GREEN precedes COMMIT in the canonical log. |
| G8 — test budget | PASS | 40 ≤ 78. |
| G9 — no prohibited test weakening | PASS | Removal of the old setup `EXIT 78` assertion is an approved contract transition; no assertion was weakened to preserve obsolete behavior. |

## Test integrity and RPP scan

- **Test modification detected:** no prohibited weakening, deletion, or skip.
- **Testing theater detected:** yes. D3's generic injected error and self-built
  closed set, D6's disconnected mutation sentinel/source search, and D7's
  diagnostic-string search can pass without the claimed production behavior.
- **Escalation verification:** not applicable; the implementation did not
  request a requirement change.
- **RPP levels scanned:** L1-L4. No independent refactoring finding is raised;
  the necessary extraction opportunities are directly tied to D1, D3, D6,
  and D7 rather than standalone cleanup.

## Verification record

All commands below used the immutable reviewed commit. No mutation command was
run.

| Verification | Result |
|---|---|
| `git diff --check eb53c11d..9bf48815` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `bash -n infra/metal/{bootstrap,lease-holder,provision}.sh` | PASS |
| Lima `cargo test -p xtask metal_qualification_tests` | PASS — 2 tests |
| Host xtask test attempt | Non-signal — macOS has no `flock`; the required Linux/Lima rerun passed |
| Lima `cargo nextest run -p overdrive-init` | PASS — 31/31 |
| Focused lifecycle classifier tests | PASS — 3/3 |
| Focused reconciler component test | PASS — 1/1 |
| Focused action-shim/illegal-release tests | PASS — 2/2 |
| Focused S-GTI-09/10/11 and slot-boundary properties | PASS — 4/4 |
| Focused root C3 replay | PASS — 1/1, but D5 shows its oracle/locator remains incomplete |
| Lima workspace `cargo check --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima `cargo nextest run --workspace --all-targets` | PASS — 2,265 passed, 23 skipped |
| Native `cargo xtask metal run -- true` | PASS; lease acquired before sync and owner metadata removed afterward |
| Native real VM walking skeleton | **FAIL** — final `DriverInternalError`, expected ordinary exit-0 termination |
| Native real guest-stack mTLS scenario | **FAIL/TIMEOUT** — target rule never became observable within 60 seconds; nextest timed out at 120 seconds |
| Post-native owner/process probe | PASS — lease owner file absent and no Cloud Hypervisor process remained |

The first native guest-stack attempt encountered a stale excluded BPF build
artifact after sync. It was rebuilt under the canonical lease and the real
guest tests were rerun from the unchanged source marker. That build prerequisite
is not used as the basis for any finding; D7 concerns only the explicitly
ratified preflight inputs.

## Remediation disposition

Return D1-D8 to the original step-02-03 crafter. Do not start 02-04. Preserve
all later dirty work and remediate only this step. After the remediation commit:

1. rerun a fresh RED → GREEN → COMMIT cycle and log only phases the crafter
   executes;
2. rerun every exact roadmap mapping, including the corrected C3 locator;
3. rerun the complete default Lima suite and the executable metal writer/
   preflight failure partitions;
4. rerun at least one canonical-rootfs real guest success and one controlled
   accepted-connection/pre-READY-failure journey on qualified native metal; and
5. return to this step-specific reviewer for iteration 2.

No finding is waived or deferred. The repository's review/remediation cycle has
no iteration cap.

## Final verdict

**NEEDS_REVISION.** Four blockers, one High, two Medium, and one Low defect
remain. The green default suite and complete DES log do not replace the failed
native guest path or the missing Contract Shape/error-partition proofs. Step
02-03 is not approved and the next roadmap step must not begin.

---

## Iteration 2 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_2`
- **Reviewer:** `nw-software-crafter-reviewer` (fresh isolated replacement)
- **Reviewed initial commit:** `9bf4881529f9bf808573eb50f51cfdf546f2e456`
- **Reviewed remediation commit:** `297e4ea4d369ec865f3303b4cdb2d7b0c4746e84`
- **Remediation parent:** `9bf4881529f9bf808573eb50f51cfdf546f2e456`
- **Step trailer:** exact `Step-Id: 02-03` on both commits
- **Review date:** 2026-08-29
- **Verdict:** **NEEDS_REVISION**

### Iteration 2 summary

The remediation closes D1, D2, D4, D6, and D8. The native VM walking
skeleton now boots and completes on qualified metal, the accepted-session
close consumes and preserves the exact VMM exit, both acceptance tests carry
the required Outcome Anchor, the lease tests execute the canonical Run/Sync/
bootstrap boundary, and the stale nested-KVM operator wording is gone. D5's
semantic locator and complete C3 snapshot are also corrected and the exact
mapped test passes.

The step is still blocked at its defining guest-token boundary. Production
`configure_guest_network_with` treats absence of `overdrive.net=` as success
and reaches READY without any NIC work. Its comment invents a
“Legacy/non-mesh VM” category that the product and approved contract do not
have. The mapped missing-token property stays green only because it calls a
`#[cfg(test)]` wrapper that production never calls. This is a direct
false-green counterexample to D3's claimed complete error closure.

The remediation also fails the required integration-feature clippy gate at
the newly mapped C3 test, leaves the ratified kernel/rootfs native-preflight
signals optional, and introduces a synchronous whole-file console read in
`VmDriver::start` with the wrong diagnostic bounds and precedence. A green
default suite cannot override those production and quality-gate failures.

### Iteration 2 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 2 | D3, D5 |
| High | 1 | N1 |
| Medium | 1 | D7 |
| Low | 0 | — |

### Iteration 1 disposition audit

| Prior finding | Iteration 2 disposition | Evidence |
|---|---|---|
| D1 — guest admission reads an unmounted sysfs file | **CLOSED** | `LinuxGuestNetworkOps::require_down` now uses `SIOCGIFFLAGS` on a datagram socket (`overdrive-init/src/main.rs:576-585`). The exact native VM walking skeleton passes. |
| D2 — accepted pre-READY close becomes `DriverInternalError` | **CLOSED** | `BootRaceOutcome::Beacon(Err(_))` now awaits the bounded VMM ending and delegates to `guest_exit_unreported_failure` (`vm_driver.rs:1336-1363`). The focused exact-code/signal regression passes. N1 concerns the newly added detail collection, not the repaired typed class. |
| D3 — fake/incomplete guest-init error matrix | **REMAINS BLOCKER** | Stage fixtures now return the right broad variants and several generated domains were added, but the production missing-token path succeeds while its mapped test calls only `#[cfg(test)] required_guest_network_config`. The claimed complete closure is therefore false. |
| D4 — acceptance Outcome Anchors and incomplete complements | **CLOSED** | Both acceptance tests now carry exact `Outcome anchor: DISCUSS Elevator Pitch`; the focused finalization/action complement tests pass. |
| D5 — C3 locator and partial snapshot | **PARTIALLY CLOSED; BLOCKER REMAINS** | The exact mapped locator now delegates to a full netns/veth/TAP/address/route/forwarding snapshot and passes. However the new declaration fails the mandatory integration-feature clippy gate under `clippy::doc_markdown`. |
| D6 — metal tests search source and miss canonical writers | **CLOSED** | The test now invokes `bootstrap.sh` through Run, Sync, and direct-bootstrap contention paths, connects the no-mutation sentinel, and the helper is streamed through the lease session before any remote helper-file write. |
| D7 — native preflight omits ratified prerequisites | **REMAINS MEDIUM** | cgroup-v2 and Cloud Hypervisor checks landed, and explicitly named missing artifact paths fail. Empty kernel/rootfs selections still bypass both checks, and canonical `metal run -- true` passes with both variables unset. |
| D8 — nested-KVM operator wording | **CLOSED** | xtask now consistently requires native, nonvirtualized x86_64 hardware with usable KVM. |

### Missing-token branch disposition — unresolved Blocker

The user-mandated branch audit confirms a production contract violation:

- `parse_guest_network_cmdline` returns `Ok(None)` when no
  `overdrive.net=` token exists (`overdrive-init/src/main.rs:399-405`).
- The only conversion of that absence into
  `InitError::GuestNetworkConfig` is
  `required_guest_network_config`, which is compiled only under
  `#[cfg(test)]` (`main.rs:445-450`).
- Production `configure_guest_network_with` instead matches `None`, claims
  that “Legacy/non-mesh VMs” need no wire, and returns `Ok(())`
  (`main.rs:464-485`). `complete_pre_ready_init` then proceeds directly to
  READY (`main.rs:222-227`).
- `assigned_guest_network_rejects_missing_platform_token` invokes the
  test-only wrapper rather than `configure_guest_network_with`
  (`main.rs:1750-1760`). The nearby parser test explicitly preserves the
  same invented non-mesh absence (`main.rs:1425-1449`).

DISTILL pins `C-GTI-TOKEN-MISSING` as “missing required token after a VM
network was assigned” with exact `InitError::GuestNetworkConfig`, and the
roadmap criterion requires token completion before READY. There is no product
category that authorizes a VM boot without that platform assignment. The
current test proves an unreachable test helper while production silently
accepts the forbidden state.

**Required remediation:** make the production configuration path require the
platform token and return `InitError::GuestNetworkConfig` on absence before
any READY. Drive the mapped property through the same production-reachable
decision used by `configure_guest_network`; remove the invented legacy/
non-mesh success contract and its affirmative test. The regression must prove
the complete later complement: no NIC mutation, READY, EXEC, operator command,
or guest EXIT after absence is detected.

### D5 — the exact mapped C3 test breaks the mandatory clippy gate

- **Severity:** Blocker
- **Dimension:** Required quality gate and Contract Shape declaration mechanics
- **Location:** `crates/overdrive-control-plane/tests/integration/veth_provision_idempotent.rs:34-37`

The semantic D5 repair is sound: the roadmap identity now exists at the exact
file/function, and the delegated body compares the netns inode, host/workload
veth and TAP ifindices, complete IPv4 address outputs, namespace routes, the
host guest-return route, and forwarding state across the second converge. The
focused root test passes.

The step nevertheless fails its required command:

`cargo clippy --workspace --all-targets --features integration-tests -- -D warnings`

Clippy rejects the new exact Contract Shape rustdoc with
`clippy::doc_markdown` (“item in documentation is missing backticks”). The
declaration cannot be rewritten with backticks because the repository requires
its exact machine-readable spelling; the established solution is a scoped
`#[allow(clippy::doc_markdown)]` with the same Contract Shape rationale used by
other mapped tests.

**Required remediation:** retain the exact declaration and add the narrowly
scoped lint allowance. Rerun the exact integration-feature clippy command.

### N1 — remediation introduces blocking, unbounded, wrong-precedence console diagnostics

- **Severity:** High
- **Dimension:** Async adapter safety, bounded diagnostics, and approved design fidelity
- **Locations:**
  - `crates/overdrive-worker/src/vm_driver.rs:1336-1378`
  - `crates/overdrive-worker/src/vm_driver.rs:1640-1670`
  - `.claude/rules/rust.md:809-830`
  - `design/wave-decisions.md:178-190`

The new `guest_console_tail` is a synchronous helper called directly from
async `VmDriver::start`. It uses `std::fs::read`, blocking a Tokio worker and
reading the entire console file before slicing the final 8 KiB. It never
applies the approved last-five-fragment bound. It also makes VMM stderr the
leading detail and appends the guest console, whereas the ratified design says
a nonempty guest console is the primary detail and bounded VMM stderr is
fallback only when the console is absent, empty, or unreadable. `.ok()?`
collapses every metadata/open/read failure without the separately observable
totality the design requires.

This is production behavior added by the remediation commit, not harmless
future scaffolding. A large console can block the boot path and allocate its
whole size, while the operator receives the wrong primary diagnostic.

**Required remediation:** snapshot asynchronously before destructive cleanup,
read only the bounded tail, apply both the 8-KiB and last-five-fragment bounds
with unterminated/lossy-UTF-8 behavior, and select nonempty console as primary
with bounded stderr/neither-source fallback. Diagnostic failure must not mask
the typed rejection or cleanup. If full diagnostic totality remains owned by
02-05, do not land an incorrect partial implementation in 02-03.

### D7 — kernel and rootfs preflight signals are optional

- **Severity:** Medium
- **Dimension:** Fail-closed native evidence validity
- **Locations:**
  - `infra/metal/bootstrap.sh:313-321`
  - `infra/metal/native-preflight.sh:51-56`
  - `xtask/src/main.rs:1420-1529`

The remediation correctly extracts and executes `native-preflight.sh`, adds
cgroup-v2 and Cloud Hypervisor gates, and tests missing paths when the fixture
explicitly supplies them. The actual runner passes empty values when
`OVERDRIVE_METAL_KERNEL` or `OVERDRIVE_METAL_ROOTFS` is unset, and the script
uses `-z ... || -f ...`; absence therefore succeeds. The test fixture always
sets both variables and has no missing-selection partition.

This was reproduced through the canonical runner: both artifact variables
were unset and `cargo xtask metal run -- true` passed native qualification.
That contradicts DISTILL's requirement that the kernel and selected rootfs
exist and that every missing signal fail closed.

**Required remediation:** give the canonical Run boundary an authoritative
kernel/rootfs selection or require nonempty artifact inputs, then fail on both
missing selection and nonexistent file. Add executable partitions for unset,
empty, missing, wrong-type, and readable-file success without source-text
inspection.

### Contract Shape and mapping audit

| Gate | Iteration 2 result |
|---|---|
| 39 roadmap-owned executable identities | All resolve mechanically |
| 18 pure-function declarations | Exact rustdoc spelling present |
| Missing-token semantic identity | **BLOCKER** — mapped test uses a test-only helper, not the production decision |
| P-GTI pre-READY closure | **FAIL** — the production missing-token counterexample passes while the self-constructed accepted-variant list remains green |
| Acceptance Outcome Anchors | PASS — both exact lines present |
| C3 bounded-change locator and semantic snapshot | PASS |
| C3 integration-feature clippy gate | **BLOCKER** |
| Test budget | PASS — remains below the step maximum |

### Mechanical and DES evidence

- Combined step range `eb53c11d..297e4ea4`: **18 files changed, 2,331
  insertions, 221 deletions**.
- Remediation range `9bf48815..297e4ea4`: **14 files changed, 1,161
  insertions, 263 deletions**.
- Both commits have the exact `Step-Id: 02-03` trailer.
- The remediation DES cycle records RED `2026-08-29T08:07:17Z`, GREEN
  `08:33:49Z`, and COMMIT `08:34:10Z`, all `EXECUTED/PASS`; the later
  GREEN/COMMIT records at `08:42:31Z` correspond to the final amended commit.
- `git diff --check` passes for both the remediation range and the complete
  step range.
- Existing dirty 02-04+ and user-owned files were excluded by reviewing the
  exact commit in a detached worktree. No source or test file was changed by
  this reviewer.

### Iteration 2 verification record

All source and Rust verification used a detached worktree at exact commit
`297e4ea4d369ec865f3303b4cdb2d7b0c4746e84`. Rust compile/test commands ran
through Lima; native runtime used the canonical metal runner. No mutation
testing was run.

| Verification | Result |
|---|---|
| `git diff --check 9bf48815..297e4ea4` and `eb53c11d..297e4ea4` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `bash -n infra/metal/{bootstrap,lease-holder,native-preflight,provision}.sh` | PASS |
| Lima `cargo nextest run -p overdrive-init -p xtask` | PASS — 179/179 |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | **FAIL** — D5 `clippy::doc_markdown` at the new C3 declaration |
| Lima workspace `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets` | PASS — 2,270 passed, 23 skipped |
| Focused exact VMM/classifier/finalization/illegal-release set | PASS — 7/7 |
| Focused mapped root C3 replay with `integration-tests` | PASS — 1/1 |
| Optional full `integration-tests` execution | Non-step failure — existing OpenAPI YAML drift stopped after 845 passes; the 02-03 ranges do not touch that schema |
| Native `cargo xtask metal run -- true` | PASS — and reproduces D7 because artifact selections were empty |
| Native real VM walking skeleton after in-lease BPF rebuild | PASS — 1/1 |
| Post-native lease owner and Cloud Hypervisor residue probe | PASS |

### Iteration 2 remediation disposition

Return D3, D5, D7, and N1 to the step-02-03 crafter. Do not start 02-04.
Preserve all pre-existing dirty work. After remediation:

1. rerun a real RED → GREEN → COMMIT cycle for the production missing-token
   failure and the executable unset-artifact partitions;
2. rerun all 39 exact mappings and the integration-feature clippy gate;
3. rerun the complete default Lima suite plus focused C3 and accepted-close
   regressions;
4. rerun canonical native preflight success and the real VM walking skeleton;
5. return to the step-specific reviewer for another iteration.

No finding is waived or deferred. No mutation testing belongs in this step.

### Iteration 2 final verdict

**NEEDS_REVISION.** Two Blockers, one High, and one Medium remain. The missing
platform token still reaches READY through an invented compatibility branch,
the mapped C3 change fails the required integration-feature clippy gate, the
new diagnostic path violates async and bounded-selection contracts, and the
native preflight still accepts absent artifact selection. Step 02-03 remains
unapproved and 02-04 must not begin.

## Iteration 3 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_3`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated step reviewer)
- **Reviewed initial commit:** `9bf4881529f9bf808573eb50f51cfdf546f2e456`
- **Reviewed first remediation:** `297e4ea4d369ec865f3303b4cdb2d7b0c4746e84`
- **Reviewed second remediation:** `47c0e69362319eed9e34dbbcb847e1db519eac18`
- **Second-remediation parent:** `297e4ea4d369ec865f3303b4cdb2d7b0c4746e84`
- **Step trailer:** exact `Step-Id: 02-03` on all three commits
- **Review date:** 2026-08-29
- **Verdict:** **NEEDS_REVISION**

### Iteration 3 executive summary

The remediation closes D5 and D7 and repairs the production half of the
missing-token branch: `overdrive-init` now requires the token through the same
production function the lifecycle calls. It also repairs N1's principal
runtime semantics: console collection is asynchronous, reads at most the final
8 KiB, retains at most five fragments including an unterminated fragment,
uses lossy UTF-8, prefers nonempty guest console to separately bounded VMM
stderr, and preserves the typed rejection through cleanup.

The step still cannot advance because the host half of the supposedly
universal assignment remains optional. `provision_and_inject_netns` returns
early whenever `mtls_worker` is absent, and `compose_vm_network` still accepts
the complete all-`None` tuple as a token-free VM. Changing one walking-skeleton
scenario to the mTLS composition masks that contradiction rather than removing
it. On qualified native metal, the next existing accepted VM scenario
reproduced the regression: its intended guest exit 7 became the new
missing-token pre-READY rejection with VMM exit 0.

The four adopted `guest_stack_mtls_egress.rs` scaffolds introduce two further
high-severity test-integrity defects. The fresh and restart failure cases use
an OUTPUT-hook destructive nft fixture instead of the approved INPUT-hook
fixture, do not pin the required typed cause/errno, and delete rather than
restore the shared baseline. The newly green restart, resolver-failure, and
stop scenarios omit most of their approved D7, no-effect, cleanup, sibling,
and complement oracles. Their native runs pass, demonstrating that the weak
oracles are executable, not that the approved contracts are satisfied.

Formatting, shell syntax, the integration-feature workspace check and clippy
gate, the 2,274-test default suite, the focused C3 test, native mandatory
artifact preflight, the corrected production-composition walking skeleton,
and all four adopted guest-stack tests passed. The native counterexample above
is a direct accepted-path failure and supersedes those green subsets. No
mutation testing was run.

### Iteration 3 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 1 | D3 |
| Critical | 0 | — |
| High | 2 | A1, A2 |
| Medium | 1 | N1 |
| Low | 0 | — |

### Prior-finding disposition audit

| Finding | Iteration 3 disposition | Evidence |
|---|---|---|
| D1 — guest admission read an unmounted sysfs file | **CLOSED (remains closed)** | The production ioctl path and qualified native positive boot remain green. |
| D2 — accepted pre-READY close became `DriverInternalError` | **CLOSED (remains closed)** | Exact VMM code/signal preservation remains in the common typed rejection path. |
| D3 — missing-token production/test split | **PARTIALLY CLOSED; BLOCKER remains** | Guest init now rejects a missing token through its production lifecycle. Host admission still omits the assignment outside the mTLS-worker gate and the VM driver still admits the all-absent compatibility tuple. The native S-VM-02 counterexample fails. |
| D4 — Outcome Anchors and complements | **CLOSED for the 02-03-owned tests** | The previously remediated mapped tests remain correctly anchored. A2 concerns newly adopted later-step scenarios. |
| D5 — C3 locator/clippy | **CLOSED** | The exact mapped integration test passes and workspace clippy with `integration-tests` is clean under `-D warnings`. |
| D6 — lease tests missed canonical writers | **CLOSED (remains closed)** | Canonical Run/Sync/bootstrap coverage remains intact; native commands retained and released the shared lease. |
| D7 — kernel/rootfs selection was optional | **CLOSED** | Unset, empty, missing, and directory partitions fail; valid readable regular files pass. The qualified canonical smoke passed with the actual staged kernel and rootfs. |
| D8 — stale nested-KVM wording | **CLOSED (remains closed)** | Native non-virtualized x86_64 remains the only runtime evidence surface. |
| N1 — blocking, whole-file, wrong-precedence console diagnostic | **PARTIALLY CLOSED; MEDIUM remains** | The implementation now has the approved async/bounds/precedence behavior. Its new totality test still does not drive the required empty, metadata-failure, open-failure, read-failure, and mid-read partitions through the real typed-rejection-plus-cleanup path. |

### D3 — universal VM assignment is still conditional and breaks accepted VMs

**Severity:** BLOCKER  
**Status:** Open

**Evidence**

- `crates/overdrive-control-plane/src/action_shim/mod.rs:915-917`
- `crates/overdrive-worker/src/vm_driver.rs:114-123`
- `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs:393-442,599-602,638-668`
- approved roadmap criterion: every VM completes the token/static-network
  sequence before READY; there is no legacy/non-mesh VM product category

The guest-side fix is real: `configure_guest_network_with` now calls
`required_guest_network_config`, and the mapped property drives that same
lifecycle. The host side still contradicts it. The action shim explicitly
returns without assigning a slot, netns, TAP, guest address, gateway, prefix,
DNS, or token whenever `mtls_worker.is_none()`. The driver then treats the
complete all-absent tuple as a valid token-free VM and launches it with the
platform-default command line.

The walking-skeleton edit changes only S-VM-01 to
`spawn_vm_server_mtls_composed`. Eleven other accepted real-VM scenarios still
use `spawn_vm_server`, whose `SimDataplane` composition leaves `mtls_worker`
absent. A qualified native run of
`vm_non_zero_guest_exit_code_is_reported_not_the_hypervisors` failed in 5.728s:
the row carried
`VmGuestExitUnreported { vmm_exit_code: Some(0), vmm_signal: None }` instead of
the guest command's required exit 7. The corrected S-VM-01 passed only because
its composition was selectively changed.

This is both a real suite regression and the exact forbidden compatibility
shape. The platform assignment must be universal for admitted VMs, independent
of whether a real or substituted dataplane supplies the mTLS worker. Remove or
reject the all-absent VM path, make every supported VM composition provide the
same C3 assignment, and rerun the complete real-VM acceptance surface on
qualified metal. Switching individual tests to a stronger composition is not
closure.

### A1 — adopted guard-failure fixtures violate the approved kernel and restoration contract

**Severity:** HIGH  
**Status:** New finding in adopted tracked work

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:177-207,1708-1748,1911-1928`
- `distill/review-q7-platform.md`, P8a closure and E09 feasibility
- `distill/red-classification.md`, S-GTI-05 and S-GTI-06b rows

`MalformedMtlsPreroutingTable` deletes the entire live `overdrive-mtls` table,
recreates `prerouting` as an OUTPUT route chain, and deletes the table again on
drop. The approved fixture is the production-named **INPUT-hook base chain**,
with a disposable preflight proving the exact real `append-egress` /
`append-rule` / `-EOPNOTSUPP` cause on the appliance kernel. Product cleanup
must precede separately trapped, delta-scoped restoration to the exact recorded
baseline.

The adopted tests instead accept a human-readable substring and terminal
state. They never assert the typed install variant, failing operation, or
errno; never preflight the encoded expression; never snapshot the nft/FIB
baseline; and never restore it. If `install()` panics after deleting the table
but before returning its guard, even its deletion-only `Drop` cannot run. The
same defective helper drives both fresh failure and failed reinstallation.

Both tests happened to pass on the current metal kernel. That does not satisfy
the ratified fixture or prove exact cleanup. Implement the approved INPUT-hook
fixture with assertion/signal-safe delta restoration and pin the complete typed
cause chain before treating either scenario as green.

### A2 — adopted later-step acceptance scenarios turn green with incomplete or vacuous oracles

**Severity:** HIGH  
**Status:** New finding in adopted tracked work

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:1781-2098`
- `distill/test-scenarios.md`: S-GTI-06a/06b, S-GTI-08a, S-GTI-12a/12b
- `deliver/roadmap.json`: those executable identities are owned by later steps,
  not 02-03

The commit replaces four explicit RED scaffolds while the authoritative
classification still records them as incomplete. The bodies do not implement
their approved outcomes:

- S-GTI-06a observes `Running`, an execution marker, and `Some(rule)`, then
  counts zero physical-interface segments to an unassigned/unreachable mesh
  address. It does not establish D7, the replacement guest's first protected
  flow, TLS/kTLS, or no peer-path cleartext. Zero traffic to an unreachable
  destination is not the required protection witness.
- S-GTI-08a has no independent running sibling, no READY/Running/EXEC/EXIT/wire
  observer, no resolver-stage console-detail assertion, and no bounded check of
  the complete VMM/cgroup/clone/index/run-dir/netns/TAP/veth/route/nft/capture/
  socket/fd residue set. It passes while almost the entire approved complement
  is absent.
- S-GTI-12a checks only one outbound nft rule and a sibling rule string. It does
  not close the approved full teardown complement or the separately mapped
  no-rule/idempotent S-GTI-12b path.

The blind `wait_for_data_dir_release` two-second sleep and non-RAII
loop-device/mount mutation further weaken failure safety: the sleep neither
polls nor proves release, and any assertion between `losetup`/`mount` and the
manual cleanup can strand host state.

The two S-GTI-05/08 tests passed in 7.401s, and the S-GTI-06/12 pair passed in
169.147s. Those timings validate the exact nextest 240-second override but
also show the incomplete tests can return green. Restore the scaffolds until
their owner steps, or complete every approved oracle and assertion-safe cleanup
now and update the authoritative classification consistently.

### N1 — diagnostic behavior is repaired, but totality/cleanup proof is incomplete

**Severity:** MEDIUM  
**Status:** Open verification gap

**Evidence**

- `crates/overdrive-worker/src/vm_driver.rs:1641-1700,1925-2012`
- `distill/test-scenarios.md`, “Diagnostic and cleanup totality”

The production code now performs the right selection and cannot replace the
typed `GuestExitUnreported` class with a diagnostic error: every I/O failure is
collapsed to `None`, cleanup is awaited, and the same typed constructor runs.
The focused byte, fragment, unterminated, lossy-UTF-8, and console-precedence
checks pass.

The new test nevertheless labels itself total while covering only a readable
file, an absent path, and a directory, then constructs the fallback rejection
separately. It does not inject unreadable metadata, distinct open/read failure,
or a mid-read error; does not cover empty content or stderr fallback; and does
not drive any of those outcomes through `VmDriver::start` to prove cleanup runs
once and the primary typed rejection remains unchanged. Those are explicit
separate cases in the approved contract. This work is mapped to 02-05, so it
may be deferred there, but the partial test must not claim total closure in the
current commit.

### D5, D7, and adopted-file dispositions

#### D5 — resolved

The narrowly scoped `clippy::doc_markdown` allowance preserves the exact
machine-read Contract Shape line. Both the exact C3 replay and the mandatory
workspace integration-feature clippy command pass.

#### D7 — resolved

`native-preflight.sh` now requires nonempty kernel and rootfs paths and checks
each is a readable regular file. The xtask regression covers unset, empty,
missing, and directory partitions independently. The canonical qualified
smoke passed with `/srv/vm/overdrive-testing/kernel` and
`/srv/vm/overdrive-testing/rootfs.ext4`; unset selection no longer qualifies.

#### Other adopted files

- `.claude/rules/rust.md`: the new structured-async-effect rule is coherent
  with the existing concurrency rules; no defect found.
- `.config/nextest.toml`: the exact S-GTI-06 override is finite and justified.
  Its native pair completed in 169.147s, beyond the default 120-second ceiling
  and below the exact 240-second override.
- `crates/overdrive-worker/src/mtls_intercept_worker.rs`: changing idle accept
  loops to hold `Weak<MtlsInterceptWorker>` removes the owner-retention cycle;
  the bounded poll rechecks owner liveness and the focused worker suite passes.
  No defect found in the adopted change.
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs`: A1 and A2
  remain blocking review findings.

### Mechanical, DES, and verification evidence

- Exact reviewed remediation: `47c0e69362319eed9e34dbbcb847e1db519eac18`.
- Exact parent: `297e4ea4d369ec865f3303b4cdb2d7b0c4746e84`.
- Exact trailer: `Step-Id: 02-03`.
- Second-remediation diff: 11 files, 961 insertions, 78 deletions.
- Complete fresh-step range `eb53c11d..47c0e693`: 23 files, 3,259
  insertions, 266 deletions.
- `git diff --check` passes for the remediation and complete step ranges.
- Fresh second-remediation DES cycle: RED `09:23:21Z`, GREEN `09:40:11Z`,
  COMMIT `09:40:39Z`, all `EXECUTED/PASS` and chronological. Commit timestamp
  is `09:40:54Z`.
- The reviewed worktree had no tracked dirty files. Pre-existing untracked
  review/roadmap/design/DISTILL artifacts were preserved.

All Rust compile/test commands below used the exact tracked source at
`47c0e693`; Lima supplied the Linux compile/default lane and the canonical
metal runner supplied native KVM. The only repository file this reviewer
changed is this native Markdown review artifact. No mutation testing was run.

| Verification | Result |
|---|---|
| `git diff --check 297e4ea4..47c0e693` and `eb53c11d..47c0e693` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `bash -n infra/metal/{bootstrap,lease-holder,native-preflight,provision}.sh` | PASS |
| Lima `cargo nextest run -p overdrive-init -p xtask -p overdrive-worker` | PASS — 323/323 |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets` | PASS — 2,274 passed, 23 skipped |
| Lima exact `c3_converge_twice_preserves_the_same_vm_network_plan` | PASS — 1/1 |
| Native `cargo xtask metal run -- true` with actual selected artifacts | PASS |
| Native mTLS-composed S-VM-01 after in-lease BPF rebuild | PASS — 1/1 |
| Native non-mTLS-composed S-VM-02 | **FAIL — D3 reproduced:** intended guest exit 7 became pre-READY `VmGuestExitUnreported` with VMM exit 0 |
| Native adopted S-GTI-05 and S-GTI-08a | PASS — 2/2 in 7.401s; A1/A2 are static oracle/fixture defects |
| Native adopted S-GTI-06 and S-GTI-12a | PASS — 2/2 in 169.147s; A1/A2 are static oracle/fixture defects |
| Post-native owner, Cloud Hypervisor, and alloc-cgroup residue probes | PASS — owner absent, 0 VMMs, 0 alloc cgroups |
| Mutation testing | NOT RUN — repository rule requires one final DELIVER-wave gate |

### Iteration 3 verdict

**NEEDS_REVISION.** D5 and D7 are closed, and N1's production behavior is
substantially corrected. D3 remains a direct native blocker: universal VM
assignment is still conditional, the token-free all-absent driver path remains,
and an existing accepted real-VM scenario fails. The adopted acceptance file
also introduces two high-severity false-green surfaces and leaves the explicit
diagnostic-totality proof incomplete.

Return D3, A1, A2, and N1 to the original step-02-03 crafter. Do not start
02-04. Continue review/remediation until a fresh re-review returns
**APPROVED** with zero unresolved blocker, high, or medium findings.

## Iteration 4 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_4`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated step reviewer)
- **Reviewed initial commit:** `9bf4881529f9bf808573eb50f51cfdf546f2e456`
- **Reviewed first remediation:** `297e4ea4d369ec865f3303b4cdb2d7b0c4746e84`
- **Reviewed second remediation:** `47c0e69362319eed9e34dbbcb847e1db519eac18`
- **Reviewed third remediation:** `018e3b65d51f9c983f32023051a2e262714ed8bd`
- **Third-remediation parent:** `47c0e69362319eed9e34dbbcb847e1db519eac18`
- **Step trailer:** exact `Step-Id: 02-03` on the reviewed commit
- **Review date:** 2026-08-29
- **Verdict:** **NEEDS_REVISION**

### Iteration 4 executive summary

The remediation closes the production and native-real-VM portions of D3.
Every production VM now crosses the C3 assignment seam even when the optional
mTLS worker is absent, the all-absent VM tuple is a typed rejection, S-VM-02
passes again, and all 14 real-VM walking-skeleton tests pass on qualified
native metal. N1 is also closed: the new bounded diagnostic reader partitions
are driven through `VmDriver::start`, preserve the original typed rejection,
select console/stderr correctly, call the reader and VMM cleanup once, and
leave no driver/run-dir/rootfs/index claim.

The step still cannot advance. D3 is not universal across supported
compositions: the default, non-root action-shim VM composition now enters the
real netns provisioner before its VM-shaped `SimDriver`, records a provision
failure, and never starts the driver. The mandatory default workspace suite
therefore fails its positive VM supervision contract (2,275 pass, one fails).
Universal C3 needs a faithfully substituted provisioner boundary for the
default component composition, not a production-only hardwired side effect
that prevents the composed VM driver from being exercised.

A1 remains high severity. The new INPUT-hook fixture and typed
`append-egress` / `append-rule` / `EOPNOTSUPP` preflight are correct, and its
watchdog attempts restoration on normal, panic, parent-death, and
partial-construction paths. Its alleged exact nft baseline is not exact,
however: it snapshots `nft list table` text without handles or production
userdata, deletes the whole table, and recreates it from that lossy text.
Production's structural `NFTA_RULE_USERDATA` ownership tags disappear while
the textual comparison still passes. A post-suite native inspection showed 12
identical `meta mark 0x2 accept` exemptions in each shared chain, demonstrating
the resulting duplicate-infra contamination.

A2 also remains high severity. The six bodies are executable and all 11
guest-stack native tests pass, but their central exactness claims remain
structurally false. `RuleInfo` contains only `(handle, userdata)`, so the
purported full ordered rule snapshot contains neither the normalized expression
program nor counters. S-GTI-06a checks rule presence plus packet timestamps,
TLS records, kTLS, and no litmus cleartext, but does not run the ratified D7
generation/notification/full-program/exact-counter oracle. S-GTI-08a's
“exact rule and counter” equality uses the same counter-free value and counts
only parsed TCP frames, allowing ARP/UDP/ICMP escape; its polling trail cannot
prove absence of a transient state it did not sample. S-GTI-12a/b can likewise
pass while a sibling program or counter changes. The amended immutable
classification consequently overstates D7 and these scenarios as complete.

### Iteration 4 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 1 | D3 |
| Critical | 0 | — |
| High | 2 | A1, A2 |
| Medium | 0 | — |
| Low | 0 | — |

### Prior-finding disposition audit

| Finding | Iteration 4 disposition | Evidence |
|---|---|---|
| D1 — guest admission read an unmounted sysfs file | **CLOSED (remains closed)** | Production ioctl admission and the complete native real-VM module remain green. |
| D2 — accepted pre-READY close became `DriverInternalError` | **CLOSED (remains closed)** | Typed VMM exit-code/signal mapping remains intact. |
| D3 — missing-token production/test split | **PARTIALLY CLOSED; BLOCKER remains** | Production assignment is now independent of `mtls_worker`, all-none VM input is rejected, S-VM-02 passes, and all 14 native walking-skeleton tests pass. The non-root default VM action-shim composition no longer reaches its composed driver and breaks the mandatory workspace suite. |
| D4 — Outcome Anchors and complements | **CLOSED for 02-03-owned tests; A2 remains for adopted tests** | Previously remediated owned declarations remain intact; adopted later-step bodies still have false/incomplete complements under A2. |
| D5 — C3 locator/clippy | **CLOSED (remains closed)** | Exact C3 replay passes on native metal and workspace integration-feature clippy is green. |
| D6 — lease tests missed canonical writers | **CLOSED (remains closed)** | Every native command acquired the canonical host-global lease and no-sync source identity check. |
| D7 — kernel/rootfs selection was optional | **CLOSED (remains closed)** | Native qualification passed with the explicit staged kernel/rootfs. |
| D8 — stale nested-KVM wording | **CLOSED (remains closed)** | Runtime evidence came only from qualified native x86_64 KVM. |
| A1 — wrong hook/typed cause/restoration | **PARTIALLY CLOSED; HIGH remains** | INPUT hook and exact typed production preflight are correct. Lossy whole-table text replay drops userdata/handles, is not delta-scoped, false-passes its own equality, and leaves duplicate shared exemptions. |
| A2 — incomplete/vacuous adopted scenarios | **PARTIALLY CLOSED; HIGH remains** | Journeys, reachable flows, TLS/kTLS, cleanup polling, sibling setup, and S-GTI-12b are materially improved and execute. D7, full rule/counter identity, no-frame/state complements, and exact preservation are still absent. |
| N1 — diagnostic totality/cleanup proof | **CLOSED** | Six reader outcomes traverse real `VmDriver::start`; focused test passes with exact typed class/detail, once-only reader/terminate counts, and bounded residue assertions. |

### D3 — universal production C3 breaks the supported default VM composition

**Severity:** BLOCKER  
**Status:** Open

**Evidence**

- `crates/overdrive-control-plane/src/action_shim/mod.rs:905-938`
- `crates/overdrive-control-plane/tests/acceptance/action_shim_running_write_failure_stops_alloc.rs:355-380`
- Lima default workspace run: 2,275 passed, one failed
- qualified native real-VM module: 14/14 passed, including S-VM-02

`network_assignment_required` now correctly returns true for every VM, and
`compose_vm_network` correctly rejects the all-absent tuple. Those changes
repair the actual metal path. The action-shim seam still calls the real
`provision_workload_netns`/`provision_vm_tap` functions directly. The existing
default-lane VM action-shim composition deliberately uses a VM-shaped
`SimDriver` with no root or netns. Its positive control dispatch now returns
`Ok` only because real provisioning was mapped to a terminal failure; the
composed driver was never started, so `live_allocations()` is empty instead of
holding the allocation.

This is not a stale expected value: the test is the non-vacuity control for a
load-bearing supervision-claim contract. Changing it to expect failure or
switching it to Exec would delete the VM-specific proof. C3 must remain
mandatory while the provisioner is represented by a production/faithful-test
port or equivalent supported composition that can supply the complete same
assignment without privileged host mutation. The mandatory default suite must
return green before D3 is closed.

### A1 — textual whole-table replay destroys nft ownership metadata

**Severity:** HIGH  
**Status:** Open

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:240-258,275-402`
- `crates/overdrive-netlink/src/nft.rs:187-195`
- approved DISTILL P8a/E08/E09 exact, trapped, delta-scoped restoration contract
- native post-run `nft -a list table ip overdrive-mtls`

The positive half is now correct. `MalformedMtlsPreroutingTable` installs the
production-named `prerouting` base chain at INPUT. The disposable preflight
calls the real encoded `install_outbound_tproxy` and pins
`OutboundTproxyInstall -> NftRuleInstallFailed { op: "append-egress" } ->
NetlinkError::Nft { op: "append-rule" } -> EOPNOTSUPP`. The watchdog is
constructed before child mutation becomes observable and attempts restoration
on normal finish, Rust panic/drop, watchdog signal, or parent death.

The restoration itself violates the approved contract. `PacketPathBaseline`
stores ordinary `nft list table` output. That text does not contain the
kernel-assigned handles or Overdrive's raw `NFTA_RULE_USERDATA` tags. The
watchdog then deletes the entire table and feeds that text back through
`nft -f`. Its final text diff can therefore pass even though the recreated
rules are foreign to the production structural decoder. On the next normal
ensure, `has_exemption` sees no ownership tag and prepends another exemption.

After the 11-test native guest-stack module passed, a read-only native
inspection showed each shared chain containing 12 identical exemptions:

```text
chain prerouting: handles 25, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13
chain output:     handles 26, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24
```

The test thus certifies “exact” restoration while the shared security state is
structurally changed and contaminated. Restore the fixture delta without
deleting/replaying unrelated state, retain the typed userdata/handle/program
facts required by the normalized baseline, and assert structural final
equality on normal and trapped paths.

### A2 — later-step bodies still lack their ratified exact observables

**Severity:** HIGH  
**Status:** Open

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs:965-992,2290-2362,2588-2650,2715-2785`
- `crates/overdrive-netlink/src/nft.rs:187-195`
- `distill/test-scenarios.md`, complete D7 contract and S-GTI-06a/06b/08a/12a/12b
- `distill/red-classification.md`, amended D7 and live-body claims

The remediation eliminates the most obvious vacuity. S-GTI-06a now drives a
genuine same-id platform-reclamation route and a reachable protected first
flow; S-GTI-06b is distinct and sibling-free; S-GTI-08a has an independent
running sibling and real resolver failure; S-GTI-12a checks target filtering;
and S-GTI-12b drives `Stopped` then `AlreadyStopped`. All 11 native module
tests pass.

They still cannot support their strongest assertions:

- `outbound_rule_snapshot` and `outbound_allocation_rule_snapshots` stringify
  `nft::RuleInfo`. That type contains only `handle` and `userdata`. It has no
  expression program, packet counter, byte counter, ruleset generation, or
  notification-loss state. S-GTI-08a's “exact rule and counter” equality and
  S-GTI-12a/b's “full ordered snapshot” therefore false-pass if a sibling's
  program or counters change.
- S-GTI-06a proves nonempty guest segments after one presence-query barrier,
  bidirectional TLS records, exact-socket TLS 1.3 kTLS, and no litmus bytes. It
  does not establish the ratified D7 full-program identity, two stable
  generation-bracketed snapshots, notification/loss closure, or exact checked
  packet/IPv4-byte deltas. Calling the timestamp assertion “D7 readiness” does
  not supply those missing observables.
- S-GTI-08a samples describe every 10 ms and later asserts that the sampled
  vector contains no `Running`; this cannot exclude an unobserved transient.
  Its “no network frame” count parses only IPv4 TCP with the guest source IP,
  so ARP, IPv4 UDP/ICMP, and other guest-originated L2 frames are outside the
  asserted universe. It also has no direct READY/guest-EXIT observation.

The classification's new claim that D7 implementation/executable coverage has
landed is contradicted by the reviewed source. Either restore honest RED/
incomplete classifications until the owner steps, or implement the complete
ratified observers and complements before retaining these live-body claims.

### N1 closure

`GuestConsoleTailReader` is a production asynchronous driven port with a real
filesystem adapter. The test-only scripted adapter partitions content, empty,
open, metadata, read, and mid-read outcomes while invoking the real
`VmDriver::start` path. All cases preserve
`GuestExitUnreported { vmm_exit_code: Some(0), vmm_signal: None }`; nonempty
console wins, all absent/error outcomes select bounded VMM stderr, the reader
is called once, VMM termination is called once, and run directory, clone,
index, and live claims are absent. The focused two worker tests pass. No N1
finding remains.

### Mechanical, DES, and verification evidence

- Exact reviewed remediation: `018e3b65d51f9c983f32023051a2e262714ed8bd`.
- Exact parent: `47c0e69362319eed9e34dbbcb847e1db519eac18`.
- Exact trailer: `Step-Id: 02-03`.
- Third-remediation diff: 12 files, 1,351 insertions, 250 deletions.
- Complete fresh-step range `eb53c11d..018e3b65`: 29 files, 4,486
  insertions, 392 deletions.
- `git diff --check` passes for the remediation and complete step ranges.
- Fresh third-remediation DES cycle: RED `11:32:24Z`, GREEN `11:33:12Z`,
  COMMIT `11:34:33Z`, all `EXECUTED/PASS` and chronological. Commit timestamp
  is `11:34:39Z`.
- The reviewed worktree had no tracked dirty files. Pre-existing untracked
  review/roadmap/design/DISTILL artifacts were preserved.

All Rust compile/test commands below used the exact tracked source at
`018e3b65`. Lima supplied the Linux compile/default lane and the canonical
metal runner supplied native KVM. The only repository file this reviewer
changed is this native Markdown review artifact. No mutation testing was run.

| Verification | Result |
|---|---|
| `git diff --check 47c0e693..018e3b65` and `eb53c11d..018e3b65` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `bash -n infra/metal/{bootstrap,lease-holder,native-preflight,provision}.sh` | PASS |
| Lima focused worker diagnostic/all-none tests | PASS — 2/2 |
| Lima `cargo nextest run -p overdrive-worker -p overdrive-control-plane --no-fail-fast` | **FAIL — 689 passed, one failed:** D3 positive VM composition |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets --no-fail-fast` | **FAIL — 2,275 passed, one failed, 23 skipped:** D3 positive VM composition |
| Native exact S-VM-02 | PASS — 1/1 in 2.169s |
| Native complete `vm_walking_skeleton` module | PASS — 14/14 in 132.387s |
| Native complete `guest_stack_mtls_egress` module | PASS — 11/11 in 169.928s |
| Native exact `c3_converge_twice_preserves_the_same_vm_network_plan` | PASS — 1/1 in 0.434s |
| Native read-only shared nft inspection | **FAIL — A1 contamination:** 12 duplicate exemptions in each shared chain |
| Mutation testing | NOT RUN — repository rule requires one final DELIVER-wave gate |

### Iteration 4 verdict

**NEEDS_REVISION.** Production/native D3 and N1 are repaired, and the newly
implemented acceptance journeys execute successfully. Approval is still
blocked by one mandatory default-lane VM-composition regression and two
high-severity false-green evidence surfaces. Return D3, A1, and A2 to the
original step-02-03 crafter. Do not start 02-04. Continue the same
review/remediation cycle until the reviewer records **APPROVED** with zero
unresolved blocker, high, or medium findings.

## Iteration 5 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_5`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated step reviewer)
- **Reviewed replacement commit:** `1bb5e86d1c7d90f6d92d541eafa2df09379313e3`
- **Replacement parent:** `47c0e69362319eed9e34dbbcb847e1db519eac18`
- **Replaced, unapproved commit:** `018e3b65d51f9c983f32023051a2e262714ed8bd`
- **Step trailer:** exact `Step-Id: 02-03` on the reviewed commit
- **Review date:** 2026-08-29
- **Verdict:** **NEEDS_REVISION**

### Iteration 5 executive summary

The replacement closes D3. `WorkloadNetworkProvisioner` is now an explicit
driven port; ordinary production dispatch supplies the real host-mutating
adapter, while the default component composition supplies a faithful
substitute without bypassing assignment. VM assignment is unconditional with
respect to mTLS composition, both Start and Restart cross the same seam, and
the complete eight-field assignment reaches the VM-shaped `SimDriver`. The
positive composition proves a real driver start, a live supervision claim, a
committed Running row, one derived workload/TAP plan, and the exact delivered
spec. The mandatory default suite is green, the qualified native real-VM
module is 14/14 green, and C3's native replay is green. There is no token-free
or conditional-mTLS VM bypass.

A2 is also closed. S-GTI-02 is explicitly ignored for owner step 02-04, and
S-GTI-05/06/08/12 are honest ignored panic scaffolds for their later owner
steps. The immutable classification leaves D7 and the split S-GTI-05/06/08/12
obligations incomplete and describes the live INPUT-hook work only as fixture
prerequisite evidence. The native module ran seven current 02-03/live fixture
tests while the five incomplete later-step tests remained skipped; no stale
live-green oracle or documentation claim was found. N1 remains closed and its
focused real-start totality test is green.

A1 is materially improved but remains high severity. The replacement no
longer deletes/replays the shared table. Its exact baseline retains table,
chain, and rule handles; complete ordered JSON expression programs and
counters; raw `GETRULE` userdata; and FIB state. The production INPUT-hook
failure remains pinned through `append-egress` -> `append-rule` ->
`EOPNOTSUPP`, and normal, post-ready panic, watchdog signal, post-ready parent
death, and the specifically injected post-marker construction failure all
pass. Native state is clean before and after the suite: table handle 74,
prerouting/output chain handles 1/2, one exact canonical exemption in each
chain at rule handles 35/36, no duplicate or foreign rule, and unchanged
fwmark/FIB state.

The claimed interruption safety is nevertheless false at the actual mutation
boundaries. The watchdog renames or creates a kernel object first and writes
the restoration marker second. A signal, parent death, or shell failure in
that interval enters `restore` without the marker that tells it to undo the
successful mutation. The injected partial-construction test occurs only after
`original-renamed` was written and therefore cannot exercise this gap. The
one-time contamination repair has the same problem at larger scale: after its
fail-closed audit, it performs 26 separate delete/insert operations without an
atomic nft transaction, watchdog, or rollback. An interruption or operation
failure can leave a partially repaired chain or temporarily leave a shared
security chain with no exemption. The current clean host proves the happy
repair completed once; subsequent runs take the clean-state early return and
cannot prove interruption safety of the destructive branch.

### Iteration 5 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 0 | — |
| Critical | 0 | — |
| High | 1 | A1 |
| Medium | 0 | — |
| Low | 0 | — |

### Prior-finding disposition audit

| Finding | Iteration 5 disposition | Evidence |
|---|---|---|
| D1 — guest admission read an unmounted sysfs file | **CLOSED (remains closed)** | Production ioctl admission and the complete native real-VM module remain green. |
| D2 — accepted pre-READY close became `DriverInternalError` | **CLOSED (remains closed)** | Typed VMM exit-code/signal mapping remains intact. |
| D3 — missing-token production/test split | **CLOSED** | Explicit production/test provisioner adapters preserve universal VM assignment; complete eight-field injection, actual driver start/supervision, default 2,276-test suite, 14-test native VM module, and native C3 replay all pass. |
| D4 — Outcome Anchors and complements | **CLOSED for 02-03-owned tests** | Later-step adopted claims are no longer represented as completed tests; A2 is closed by honest ignored/incomplete handoff. |
| D5 — C3 locator/clippy | **CLOSED (remains closed)** | Exact C3 replay and mandatory workspace integration-feature clippy pass. |
| D6 — lease tests missed canonical writers | **CLOSED (remains closed)** | Every native command acquired the canonical host-global lease; follow-up commands passed the no-sync source-identity check. |
| D7 — kernel/rootfs selection was optional | **CLOSED (remains closed)** | Native qualification required and selected `/srv/vm/overdrive-testing/kernel` and `/srv/vm/overdrive-testing/rootfs.ext4`. |
| D8 — stale nested-KVM wording | **CLOSED (remains closed)** | Runtime evidence came only from qualified native x86_64 KVM. |
| A1 — wrong hook/typed cause/restoration | **PARTIALLY CLOSED; HIGH remains** | Hook, typed cause, structural baseline, delta scope, and all exercised restoration paths are correct; mutation-to-marker gaps and non-transactional one-time repair still permit interrupted shared-state corruption. |
| A2 — incomplete/vacuous adopted scenarios | **CLOSED** | D7 and every incomplete later-step obligation are ignored/RED or explicitly incomplete; the current fixture is not claimed as S-GTI-05 completion. |
| N1 — diagnostic totality/cleanup proof | **CLOSED (remains closed)** | Six reader outcomes still traverse real `VmDriver::start`; typed class/detail, once-only reader/terminate counts, and bounded residue assertions pass. |

### A1 — restoration journal and contamination repair are not interruption-safe

**Severity:** HIGH  
**Status:** Open

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:271-335`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:366-418`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:445-513`
- approved DISTILL P8a/E08/E09 partial-construction, trapped restoration, and exact shared-state requirements

The delta-scoped design and exact comparison surface are now correct. An
existing `prerouting` chain is renamed, preserving the chain object, handle,
rules, programs, counters, and userdata; only a temporary production-named
INPUT-hook chain is created. The exact typed production call fails as required,
and the watchdog restores the renamed object rather than replaying the table.
FIB state and an existing output chain are likewise retained.

The recovery journal is recorded after each destructive kernel mutation:

- `nft add table` precedes `table-created`;
- `nft rename ... prerouting ... saved` precedes `original-renamed`;
- `nft add chain ... prerouting` precedes `malformed-created`.

If HUP/INT/TERM arrives, the parent dies, or the shell exits in any interval
between a successful nft command and its marker write, `finish -> restore`
cannot know that mutation happened. In the rename case it leaves the original
production chain stranded under the saved name. The partial-construction test
does not close this complement: `inject-after-rename` exits only after line 320
has already written `original-renamed`.

The audit-pinned repair first validates the exact contaminated table, ordered
handles, userdata, and expression programs, which properly prevents deletion
of unknown state. It then deletes 24 rules and inserts two replacements as 26
independent netlink operations. A failure or interruption after the first
deletion has no rollback, and there is an interval in each chain after the last
delete and before insertion where no exemption exists. The canonical metal
lease prevents another supported writer; it does not make process death or
kernel-operation failure atomic.

Make the kernel mutation and its recovery knowledge interruption-safe. For the
fixture, journal intent before mutation and make restoration idempotently
inspect both possible object names/states, then inject failures at every
mutation/journal boundary. For the one-time repair, use one atomic nft batch or
an equally exact rollback/watchdog protocol, with a fault test that proves each
intermediate failure restores the audited starting state or reaches the exact
canonical state. Do not rely on the now-clean host's early-return path as proof
of the destructive branch.

### D3, A2, and N1 closure evidence

#### D3 — closed

- `WorkloadNetworkProvisioner` receives the exact derived
  `WorkloadNetnsPlan` and optional same-slot `VmTapPlan`.
- Ordinary `dispatch` always supplies `HostNetworkProvisioner`, whose adapter
  calls the production netns/veth and TAP convergers and production teardown.
- `network_assignment_required` is `mtls_composed || DriverType::Vm`; all four
  VM/Exec x mTLS combinations are pinned, and a VM can never use the
  Exec/non-mTLS no-assignment branch.
- Both Start and Restart call `provision_and_inject_netns` before
  `Driver::start`. VM injection fills `netns`, `host_veth`, `workload_addr`,
  `guest_tap`, `guest_mac`, `guest_gateway`, `guest_prefix_len`, and
  `guest_dns` from the acknowledged plans.
- The non-root component composition supplies `SimNetworkProvisioner`, then
  proves one provision, one complete started spec, a Running observation, and
  the VM `SimDriver`'s live supervision claim. `SimDriver::started_specs`
  records the actual `Driver::start` input, so the proof is non-vacuous.
- The production host path is exercised by the qualified native 14-test
  walking-skeleton module, including non-mTLS S-VM-02; all pass.

#### A2 — closed

The executable later-step bodies from the obsolete remediation are gone.
S-GTI-02 is ignored for D7/02-04, and S-GTI-05, the combined 06 scaffold, the
combined 08 scaffold, and the combined 12 scaffold are ignored panic REDs for
their named later owner steps. `red-classification.md` calls D7
`NEWLY_INCOMPLETE`, retains 06b/08b/12b as distinct incomplete obligations,
and calls the live fault fixture only an S-GTI-05 prerequisite. A repository
search found no remaining live-green names or documentation claim that closes
D7 or complete S-GTI-05/06/08/12.

#### N1 — remains closed

`real_start_rejection_is_total_over_console_diagnostic_failures` remains a
real `VmDriver::start` matrix over content, empty, open, metadata, read, and
mid-read outcomes. It preserves `GuestExitUnreported`, console/stderr
precedence, one reader call, one VMM termination, and absence of run-dir,
rootfs-clone, index, or supervision residue. The exact focused test passed.

### Mechanical, DES, and verification evidence

- Exact reviewed replacement: `1bb5e86d1c7d90f6d92d541eafa2df09379313e3`.
- Exact parent: `47c0e69362319eed9e34dbbcb847e1db519eac18`.
- Exact trailer: `Step-Id: 02-03`.
- Replacement diff: 15 files, 1,196 insertions, 786 deletions.
- Complete fresh-step range `eb53c11d..1bb5e86d`: 32 files, 3,784
  insertions, 381 deletions.
- `git diff --check` passes for the replacement and complete step ranges.
- Historical obsolete third-remediation cycle remains separately recorded as
  RED `11:32:24Z`, GREEN `11:33:12Z`, COMMIT `11:34:33Z`.
- Current replacement cycle is RED `12:04:27Z`, GREEN `12:26:39Z`, COMMIT
  `12:27:10Z`; all six events are `EXECUTED/PASS`, the cycles are distinct,
  and the current cycle is chronological before commit time `12:28:30Z`.
- The reviewed worktree had no tracked dirty files. Pre-existing untracked
  review/roadmap/design/DISTILL artifacts were preserved.

All Rust compile/test commands below used the exact tracked source at
`1bb5e86d`. Lima supplied the Linux compile/default lane and the canonical
metal runner supplied native KVM. The only repository file this reviewer
changed is this native Markdown review artifact. No mutation testing was run.

| Verification | Result |
|---|---|
| `git diff --check 47c0e693..1bb5e86d` and `eb53c11d..1bb5e86d` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `bash -n infra/metal/{bootstrap,lease-holder,native-preflight,provision}.sh` | PASS |
| Lima focused D3 composition/assignment tests | PASS — 2/2 |
| Lima focused N1/all-absent rejection tests | PASS — 2/2 |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets --no-fail-fast` | PASS — 2,276 passed, 23 skipped |
| Native complete current `guest_stack_mtls_egress` selection | PASS — 7/7, 239 filter-skipped; includes all three fault-fixture tests, while static audit confirms the five later-step tests remain `#[ignore]` REDs |
| Native complete `vm_walking_skeleton` module | PASS — 14/14 in 131.759s |
| Native exact `c3_converge_twice_preserves_the_same_vm_network_plan` | PASS — 1/1 in 0.431s |
| Native nft/FIB inspection before and after | PASS for current state — table 74; chains 1/2; one canonical rule per chain at handles 35/36; no duplicate/foreign rule; fwmark rule and local route unchanged |
| Post-native Cloud Hypervisor and alloc-cgroup residue probes | PASS — no executable VMM or allocation cgroup residue |
| Mutation testing | NOT RUN — repository rule requires one final DELIVER-wave gate |

### Iteration 5 verdict

**NEEDS_REVISION.** D3, A2, and N1 are closed, the mandatory default and full
native VM suites pass, and the shared nft state is currently canonical. A1
still has one high-severity interruption-safety defect: successful kernel
mutations can occur before their recovery markers, and the one-time repair is
multi-operation destructive work without atomicity or rollback. Return A1 to
the original step-02-03 crafter. Do not start 02-04; continue the same
review/remediation cycle until a re-review records **APPROVED** with zero
unresolved blocker, high, or medium findings.

## Iteration 6 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_6`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated step reviewer)
- **Reviewed remediation:** `8bc1abff645833c0f02501e8e7e402f3307dc41c`
- **Remediation parent:** `1bb5e86d1c7d90f6d92d541eafa2df09379313e3`
- **Step trailer:** exact `Step-Id: 02-03` on the reviewed commit
- **Review date:** 2026-08-29
- **Verdict:** **NEEDS_REVISION**

### Iteration 6 executive summary

The remediation closes the two specific interruption gaps reported in
Iteration 5. Every watchdog-owned nft/FIB setup and cleanup mutation now has a
durably synced intent record before the command, and recovery reconciles live
table/chain names, the saved production-chain handle, and fixture ownership
without trusting the post-mutation `applied-*` marker. The expanded native
matrix executes intent, mutation, completion-marker, READY, signal,
parent-death, and retry boundaries for the states it constructs. Those tests
pass while preserving the exact existing production object graph.

The contamination repair is also genuinely atomic. All 24 audited deletes and
two tagged inserts are framed between one `NFNL_MSG_BATCH_BEGIN`/`END`; every
operation requests an ACK, and success is returned only after all 26 operation
sequences acknowledge after kernel commit. The qualified native test injects
one rejected delete at each of 27 operation boundaries, receives the typed
`ENOENT` NACK, and proves exact rollback of table/chain/rule handles, programs,
counters, userdata, and FIB. The valid transaction converges to one exact owned
exemption per chain, with no externally visible exemption gap.

A1 is nevertheless not closed because the new matrix does not exercise the
required clean production path and its stand-ins are not faithful. The E08
contract begins with the feature table, fwmark rule, and local route absent,
then allows the unchanged production installer to create shared exemptions,
the output chain, and FIB state before the real INPUT-hook append fails. The
fixture cannot even snapshot a truly absent table-100 route state: `ip -j route
show table 100` exits nonzero and `PacketPathBaseline::capture_table` panics.
When an unrelated route is added only to make table 100 readable, the real
clean-baseline production call reaches the correct typed failure but the
watchdog cannot restore it. It exits 97 and leaves the test-owned table and
INPUT chain, production-created unowned output chain, both exemption rules,
fwmark rule, and local route in the disposable network namespace.

The false pass is structural. `owned_table_is_disposable` permits deletion
only when the table contains zero rules and every table/chain object carries
the fixture comment. Production creates two exemption rules and creates its
output chain without that comment. Likewise, the no-output fault test manually
creates an empty output chain with the fixture comment instead of invoking the
production installer, and the absent-table matrix never invokes production at
all. No fault matrix exercises the fwmark/local-route cleanup branches. The
current native host is already converged, so its 11-test green result bypasses
all of these required clean-baseline deltas.

D3, A2, and N1 remain closed. The default workspace is now 2,278/2,278 green;
the qualified native real-VM module remains 14/14 green; the complete current
guest-stack selection is 11/11 green on the converged host; and C3 native
replay remains green. The remediation did not reintroduce later-step live
claims or change the VM assignment/diagnostic surfaces.

### Iteration 6 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 0 | — |
| Critical | 0 | — |
| High | 1 | A1 |
| Medium | 0 | — |
| Low | 0 | — |

### Prior-finding disposition audit

| Finding | Iteration 6 disposition | Evidence |
|---|---|---|
| D1 — guest admission read an unmounted sysfs file | **CLOSED (remains closed)** | Production ioctl admission and the complete native real-VM module remain green. |
| D2 — accepted pre-READY close became `DriverInternalError` | **CLOSED (remains closed)** | Typed VMM exit-code/signal mapping remains intact. |
| D3 — missing-token production/test split | **CLOSED (remains closed)** | Focused assignment/component tests, default workspace, full native VM module, and native C3 replay all pass. |
| D4 — Outcome Anchors and complements | **CLOSED for 02-03-owned tests** | No changed file revives a later-step body or overstates its complement. |
| D5 — C3 locator/clippy | **CLOSED (remains closed)** | Native C3 replay and workspace integration-feature clippy pass. |
| D6 — lease tests missed canonical writers | **CLOSED (remains closed)** | Every native command acquired the canonical host-global lease; all follow-ups passed no-sync source identity. |
| D7 — kernel/rootfs selection was optional | **CLOSED (remains closed)** | Native qualification used the explicit staged kernel and rootfs. |
| D8 — stale nested-KVM wording | **CLOSED (remains closed)** | Runtime evidence came from qualified native x86_64 KVM; disposable clean-state probes used a native network namespace, not nested virtualization. |
| A1 — wrong hook/typed cause/restoration | **PARTIALLY CLOSED; HIGH remains** | Journaling/reconciliation gaps and atomic repair are closed. The required clean production-created nft/FIB delta is unrepresentable or unrestorable, and the green matrices substitute non-production state. |
| A2 — incomplete/vacuous adopted scenarios | **CLOSED (remains closed)** | S-GTI-02/05/06/08/12 remain honest ignored later-step obligations and D7 remains incomplete. |
| N1 — diagnostic totality/cleanup proof | **CLOSED (remains closed)** | Focused real-start diagnostic totality and all-absent VM rejection tests pass. |

### A1 — clean-baseline production delta is neither captured nor restored

**Severity:** HIGH  
**Status:** Open

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:48-82`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:317-340`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:414-529`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:746-879`
- `crates/overdrive-worker/src/mtls_intercept.rs:677-760`
- E08 clean-baseline and exact fixture-delta contract
- qualified native isolated-network-namespace reproductions

#### The strict baseline rejects the required absent route state

`PacketPathBaseline::capture_table` unconditionally requires
`ip -j route show table 100` to succeed. In a fresh native network namespace,
where the feature local route and table are genuinely absent, iproute2 returns
`ipv4: FIB table does not exist`; the exact production INPUT-hook test panics
at baseline capture before installing the fixture. The absent-table fault
matrix misses this because it runs on the already-converged host and therefore
inherits the host's existing table-100 local route in every supposedly
disposable case.

Absence is a valid typed baseline state, not infrastructure failure. Capture
must normalize an absent table 100 to an empty route collection while still
rejecting genuine query/decoder errors.

#### The recovery ownership predicate rejects production-created delta

With one unrelated blackhole route in table 100 so the baseline query is
readable, the same exact test proceeds in a fresh native network namespace.
The fixture creates its owned table and wrong-hook `prerouting` chain. The real
`install_outbound_tproxy` then creates the fwmark rule and local route, inserts
the prerouting exemption, creates the production output base chain, inserts
its exemption, and receives the required typed `append-egress` -> `append-rule`
-> `EOPNOTSUPP` rejection.

Restoration fails five retries later with watchdog status 97. The isolated
post-failure snapshot contains:

- test-owned table handle 1 and INPUT-hook `prerouting` handle 1;
- production output chain handle 3 with no fixture comment;
- canonical exemption rules at handles 2 and 4;
- the production fwmark rule and local default route;
- the unrelated blackhole baseline route.

That result follows directly from `owned_table_is_disposable`: it requires
zero rules and an owner-comment count equal to table plus chain count. The
actual production delta necessarily violates both predicates. The pre-existing
table/output cleanup branch has the analogous defect: `chain_owned output`
requires the fixture comment, but production `ensure_base_chain` does not add
one.

The new green tests do not refute the reproduction:

- `table_creation_gap_restores_an_absent_disposable_table` drives only the
  watchdog's table/chain operations; it never calls production.
- The no-output section of
  `cleanup_fault_matrix_restores_exact_production_and_disposable_baselines`
  manually creates an empty output chain carrying the fixture owner comment;
  production creates an un-commented chain and inserts an exemption.
- No matrix makes the feature fwmark/local route absent, invokes production,
  and faults/retries their cleanup boundaries.
- The ordinary host run begins with table 74, chains 1/2, exemptions 35/36,
  the fwmark rule, and local route already present, so production creates none
  of the missing shared delta.

Exercise the real installer from the required clean nft/FIB baseline in an
isolated native network namespace. Journal its possible shared mutations
before the call, structurally identify only the exact baseline-absent objects
it creates, and reconcile them on normal, assertion, setup/cleanup signal,
parent-death, journal-write, and mutation boundaries. Preserve unrelated FIB
objects and fail closed on foreign table/chain/rule/route/rule state. The exact
test must finish with the original absent table/feature-FIB state, not merely
pass on a host where all shared infrastructure pre-exists.

### Closed A1 portions

#### Mutation journaling and reconciliation

For watchdog-owned mutations, `journal_intent` writes and syncs intent before
the kernel command. Recovery ignores `applied-*` as authority and inspects the
live table, saved/current chain names, saved baseline chain handle, and fixture
comments. The expanded setup/cleanup matrices prove idempotent retry across
the listed mutation and marker gaps for the object shapes they actually
construct. This closes the Iteration 5 mutation-before-marker finding, subject
to the clean-production-delta defect above.

#### Atomic contamination repair

`apply_rule_transaction_atomically` emits one nfnetlink batch containing the
complete 26-operation repair. The kernel delivers requested operation ACKs
after processing the batch; the implementation tracks every operation
sequence and rejects any NACK. The native disposable-table test drives an
invalid delete before, between, and after every valid operation, and exact
baseline equality proves rollback at all 27 boundaries. A final valid batch
produces one exact owned exemption in each chain and unchanged FIB. No atomic
repair finding remains.

### D3, A2, and N1 regression audit

- **D3:** unchanged production `dispatch` still supplies the host provisioner;
  VM assignment remains independent of optional mTLS composition; the
  substitute component port still acknowledges the same workload/TAP plans;
  all eight fields reach an actually started and supervised VM-shaped driver.
  Focused Lima tests, default workspace, full native VM, and C3 replay pass.
- **A2:** the commit changes only the fault fixture, netlink atomic API, one
  byte-equivalent TAP-name copy, and DES log. S-GTI-02 and the four later-step
  scaffolds remain ignored, with D7/S-GTI-05/06/08/12 incomplete in the
  immutable classification.
- **N1:** the real-start six-outcome diagnostic totality matrix and the
  all-absent VM network rejection remain unchanged and green.
- **Adopted `client.rs` edit:** the direct byte copy preserves the validated
  `ifreq.ifr_name` bytes and trailing zero while avoiding target-dependent
  `c_char` signedness. No behavioral or scope defect found.

### Mechanical, DES, and verification evidence

- Exact reviewed remediation: `8bc1abff645833c0f02501e8e7e402f3307dc41c`.
- Exact parent: `1bb5e86d1c7d90f6d92d541eafa2df09379313e3`.
- Exact trailer: `Step-Id: 02-03`.
- Remediation diff: 4 files, 833 insertions, 63 deletions.
- Complete fresh-step range `eb53c11d..8bc1abff`: 34 files, 4,556
  insertions, 383 deletions.
- `git diff --check` passes for the remediation and complete step ranges.
- Fresh Iteration 6 DES cycle: RED `12:58:24Z`, GREEN `13:40:02Z`, COMMIT
  `13:40:19Z`, all `EXECUTED/PASS` and chronological before committer time
  `13:40:44Z`.
- The reviewed worktree had no tracked dirty files. Pre-existing untracked
  review/roadmap/design/DISTILL artifacts were preserved.

All Rust compile/test commands below used the exact tracked source at
`8bc1abff`. Lima supplied the Linux compile/default lane and the canonical
metal runner supplied native KVM and disposable native network namespaces.
The only repository file this reviewer changed is this native Markdown review
artifact. No mutation testing was run.

| Verification | Result |
|---|---|
| `git diff --check 1bb5e86d..8bc1abff` and `eb53c11d..8bc1abff` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `bash -n infra/metal/{bootstrap,lease-holder,native-preflight,provision}.sh` | PASS |
| Lima focused atomic-batch encoder/ACK tests | PASS — 2/2 |
| Lima focused D3 assignment/component tests | PASS — 2/2 |
| Lima focused N1/all-absent rejection tests | PASS — 2/2 |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets --no-fail-fast` | PASS — 2,278 passed, 23 skipped |
| Native complete current `guest_stack_mtls_egress` selection on converged host | PASS — 11/11 in 68.310s, 239 filter-skipped |
| Native complete `vm_walking_skeleton` module | PASS — 14/14 in 132.262s |
| Native exact `c3_converge_twice_preserves_the_same_vm_network_plan` | PASS — 1/1 in 0.439s |
| Native exact INPUT-hook fixture in truly clean disposable netns | **FAIL — A1 reproduced before setup:** absent table 100 is treated as a fatal snapshot error |
| Native exact INPUT-hook fixture in clean disposable netns with unrelated table-100 route | **FAIL — A1 reproduced after real typed rejection:** watchdog exits 97 and leaves table/chains/exemptions/fwmark/local-route fixture delta inside the disposable namespace |
| Native host nft/FIB inspection before and after all runs | PASS for host state — table 74; chains 1/2; one canonical rule per chain at handles 35/36; fwmark/local route unchanged; no duplicate, foreign, or `ovd-gti-*` residue |
| Post-native VMM and alloc-cgroup probes | PASS — no executable Cloud Hypervisor or allocation cgroup residue |
| Mutation testing | NOT RUN — repository rule requires one final DELIVER-wave gate |

### Iteration 6 verdict

**NEEDS_REVISION.** The Iteration 5 journal gap is closed and the one-time
repair is now an atomic, rollback-proved nfnetlink transaction. D3, A2, and N1
remain closed. A1 still has one high-severity false-green fixture defect: its
required clean FIB baseline cannot be captured, and after the real production
installer creates its allowed shared nft/FIB delta, the restoration ownership
predicate refuses that exact delta and leaves it behind. Return A1 to the
original step-02-03 crafter. Do not start 02-04; continue the same
review/remediation cycle until a re-review records **APPROVED** with zero
unresolved blocker, high, or medium findings.

## Iteration 7 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_7`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated step reviewer)
- **Reviewed remediation:** `4dcd1261bcb620efdbcdd08f9f562856e7fd2921`
- **Remediation parent:** `8bc1abff645833c0f02501e8e7e402f3307dc41c`
- **Step trailer:** exact `Step-Id: 02-03` on the reviewed commit
- **Review date:** 2026-08-29
- **Verdict:** **NEEDS_REVISION**

### Iteration 7 executive summary

The remediation closes the exact clean-state failure reproduced in Iteration
6. Table-100 route capture now obtains a strict complete typed dump and selects
table 100, so genuine absence is represented by an empty array while command
and decoder failures still abort. Before the unchanged real installer runs,
the fixture durably syncs intents for every shared production mutation it can
make. A child-process audit accepts only the exact fixture-owned table and
wrong-hook chain, the production-created un-commented output base chain,
canonical tagged exemptions, and the one canonical production FIB rule and
route. Foreign nft and FIB additions refuse cleanup. On a genuinely empty
native network namespace, the complete new normal/fault/panic/signal/
parent-death matrix passes, preserves an unrelated blackhole route, exercises
the real typed `append-egress` -> `append-rule` -> `EOPNOTSUPP` failure, and
returns to exact absence.

A1 is still not closed because recovery records pre-existing nft state only as
coarse presence markers rather than as the exact baseline delta it must
restore. In particular, `output-present` suppresses all output cleanup. If the
production-named output base chain already exists but is empty, the unchanged
installer inserts its tagged exemption into that pre-existing chain; recovery
then retains the production-created rule. This is not hypothetical: a
qualified native isolated-network-namespace run against an empty pre-existing
prerouting plus output table reached the required typed production failure,
then failed exact restoration and left the output exemption at handle 5.

The same baseline/delta mismatch also affects FIB reconciliation. A qualified
native isolated namespace containing a pre-existing priority-123
`fwmark 0x1/0xff lookup 100` rule caused the real production path to adopt that
rule, create only its local route, and fail at the intended nft append.
Recovery exited 97: it preserved the baseline rule and removed the nft table,
but the text cleanup predicate repeatedly selected the baseline rule and never
reached removal of the production-created local route. The committed green
matrix covers an absent output chain and canonical/absent FIB state, so neither
pre-existing partition is represented. Exact fixture restoration therefore
remains false-green for supported pre-existing baselines.

D3, A2, and N1 remain closed. The default workspace is 2,278/2,278 green; the
qualified native guest-stack selection is 13/13 green; the complete real-VM
module is 14/14 green; and the exact C3 replay is green. The remediation changes
only the fault fixture and DES log and does not reintroduce later-step live
claims or alter assignment, VM diagnostics, or production install behavior.

### Iteration 7 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 0 | — |
| Critical | 0 | — |
| High | 1 | A1 |
| Medium | 0 | — |
| Low | 0 | — |

### Prior-finding disposition audit

| Finding | Iteration 7 disposition | Evidence |
|---|---|---|
| D1 — guest admission read an unmounted sysfs file | **CLOSED (remains closed)** | Production ioctl admission and the complete native real-VM module remain green. |
| D2 — accepted pre-READY close became `DriverInternalError` | **CLOSED (remains closed)** | Typed VMM exit-code/signal mapping remains intact. |
| D3 — missing-token production/test split | **CLOSED (remains closed)** | Default workspace, full native VM module, and native C3 replay pass. |
| D4 — Outcome Anchors and complements | **CLOSED for 02-03-owned tests** | No changed file revives a later-step body or overstates its complement. |
| D5 — C3 locator/clippy | **CLOSED (remains closed)** | Native C3 replay and workspace integration-feature clippy pass. |
| D6 — lease tests missed canonical writers | **CLOSED (remains closed)** | Every native command acquired the canonical host-global lease and passed source identity/preflight. |
| D7 — kernel/rootfs selection was optional | **CLOSED (remains closed)** | Native qualification required the explicit staged kernel and rootfs; an intentionally selection-free smoke failed closed before execution. |
| D8 — stale nested-KVM wording | **CLOSED (remains closed)** | Runtime evidence came from qualified native x86_64 KVM; clean-state probes used disposable native network namespaces. |
| A1 — wrong hook/typed cause/restoration | **PARTIALLY CLOSED; HIGH remains** | Truly absent capture and tested clean production delta now restore exactly. Pre-existing output/FIB partitions still fail exact real-installer restoration. |
| A2 — incomplete/vacuous adopted scenarios | **CLOSED (remains closed)** | S-GTI-02/05/06/08/12 remain honest ignored later-step obligations and D7 remains incomplete. |
| N1 — diagnostic totality/cleanup proof | **CLOSED (remains closed)** | The complete native VM module and default regressions remain green. |

### A1 — pre-existing output and FIB state are not reconciled as exact baseline deltas

**Severity:** HIGH  
**Status:** Open

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:241-289`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:552-734`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:863-945`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:1596-1772`
- `crates/overdrive-worker/src/mtls_intercept.rs:677-760`
- feature-delta E08 exact fixture/production-delta restoration contract
- qualified native isolated-network-namespace counterexamples

#### A pre-existing output chain retains the production-created exemption

Setup records only an `output-present` marker. The real installer adopts a
structurally present output chain and calls `ensure_exemption(output)`, which
inserts the production-tagged exemption when that chain was empty. During
recovery, however, output flush/delete is guarded by `! output-present`; no
branch removes only the exemption that was absent from the baseline. The exact
baseline's output ownership and programs are not stored for restoration.

The qualified native counterexample began with table `overdrive-mtls` and
empty, correctly typed prerouting/output base chains. The unchanged installer
created its FIB objects and both exemptions, then returned the required typed
INPUT-hook append rejection. Recovery restored the original prerouting chain
and removed the production FIB state, but `fixture.finish()` compared the live
state with the captured baseline and failed. The only nft difference was the
production-owned canonical exemption left in the pre-existing output chain at
handle 5. The committed pre-existing-table scenario creates prerouting only;
because output starts absent there, recovery deletes the entire created output
chain and never exercises this partition.

There is a second static unsupported partition in the same audit: every
`existing-table` audit unconditionally reads `baseline-prerouting-handle`, even
though setup explicitly supports a pre-existing table with no prerouting
chain. Such a baseline has no handle file and cannot enter cleanup.

#### FIB cleanup uses a text match that is not the recorded typed delta

The fixture marks `had-fwmark-rule` only when a baseline JSON object is byte-
equal to one canonical rendering. Production's actual presence predicate
adopts any dumped rule carrying the same fwmark and table, independent of
priority or mask. Cleanup then uses `ip rule show | grep` and an untyped delete
instead of subtracting the captured baseline from current typed state.

In a fresh qualified native namespace, the baseline rule was:

`priority 123 fwmark 0x1/0xff lookup 100`

The real installer correctly exercised its presence path, added the canonical
local route, and reached typed `EOPNOTSUPP`. The watchdog removed the nft table
but exited 97 after five retries. Its baseline rule remained intact, while the
production-created local table-100 route remained leaked because cleanup never
advanced past the mismatched rule deletion. This run is a direct
real-installer counterexample, not a stand-in or injected production failure.

Persist and compare the exact pre-existing nft/FIB baseline needed to compute
the production delta. Remove only a tagged exemption proven absent from a
pre-existing chain, and only the exact FIB object proven absent from the
baseline; preserve every baseline object. Do not use `output-present` or a
display-text grep as mutation authority. Add qualified native real-installer
partitions for an empty pre-existing output chain, a table without prerouting,
and a pre-existing adopted/colliding FIB rule, then drive their normal and
cleanup-interruption boundaries to exact baseline equality.

### Closed A1 portions

#### Genuine absence and strict typed capture

`table_100_routes` now runs a strict complete `ip -j route show table all`
query, rejects command/JSON failures, and filters typed table-100 records. The
new native absent-table proof sees `[]` in a fresh namespace, while the real
production scenario begins with feature nft/FIB state absent and finishes at
the exact same baseline.

#### Real clean production delta and fail-closed foreign additions

The new clean scenario calls the unchanged installer from absent state and
observes the exact typed append cause chain. It covers production intent-write
failures, all declared setup/cleanup journal and mutation boundaries, panic,
watchdog signal, parent death, a preserved unrelated table-100 blackhole route,
and both absent/pre-existing-table-without-output object graphs. Foreign nft
chain/rule and FIB route/rule additions cause cleanup refusal and remain
preserved. These partitions are genuine and no longer use the empty commented
output-chain stand-in identified in Iteration 6.

#### Atomic repair and earlier journaling

The atomic 24-delete/two-insert contamination repair, per-operation rollback
proof, and watchdog-owned pre-mutation journaling remain unchanged and green.
No atomicity or watchdog-owned journal finding remains.

### D3, A2, and N1 regression audit

- **D3:** production dispatch and VM assignment behavior are unchanged; the
  complete 14-test native VM module and exact C3 replay pass.
- **A2:** the commit changes only the fault fixture and DES log. Later-step
  S-GTI bodies remain ignored and no runtime claim is advanced.
- **N1:** the complete native VM module remains green, including diagnostic
  totality and all-absent rejection coverage; post-run process/cgroup probes
  are empty.

### Mechanical, DES, and verification evidence

- Exact reviewed remediation: `4dcd1261bcb620efdbcdd08f9f562856e7fd2921`.
- Exact parent: `8bc1abff645833c0f02501e8e7e402f3307dc41c`.
- Exact trailer: `Step-Id: 02-03`.
- Remediation diff: 2 files, 782 insertions, 27 deletions.
- Complete fresh-step range `eb53c11d..4dcd1261`: 34 files, 5,311
  insertions, 383 deletions.
- `git diff --check` passes for the remediation and complete step ranges.
- Fresh Iteration 7 DES cycle: RED `14:13:37Z`, GREEN `14:41:42Z`, COMMIT
  `14:45:10Z`, all `EXECUTED/PASS` and chronological before committer time
  `14:46:12Z`.
- The reviewed worktree had no tracked dirty files. Pre-existing untracked
  review/roadmap/design/DISTILL artifacts were preserved.

All Rust compile/test commands below used the exact tracked source at
`4dcd1261`. Lima supplied the Linux compile/default lane and the canonical
metal runner supplied native x86_64 KVM and disposable native network
namespaces. Every metal command acquired the host-global lease, synced the
reviewed source, and passed fail-closed preflight with the selected
`/srv/vm/overdrive-testing/kernel` and
`/srv/vm/overdrive-testing/rootfs.ext4`. The only repository file this reviewer
changed is this native Markdown review artifact. No mutation testing was run.

| Verification | Result |
|---|---|
| `git diff --check 8bc1abff..4dcd1261` and `eb53c11d..4dcd1261` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Shell syntax for repository `*.sh` files | PASS |
| Lima focused atomic-batch encoder/ACK tests | PASS — 2/2 |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets --no-fail-fast` | PASS — 2,278 passed, 23 skipped |
| Native complete current `guest_stack_mtls_egress` selection | PASS — 13/13 in 86.777s, 239 skipped |
| Native complete `vm_walking_skeleton` module | PASS — 14/14 in 131.749s, 238 skipped |
| Native exact `c3_converge_twice_preserves_the_same_vm_network_plan` | PASS — 1/1 in 0.407s, 781 skipped |
| Native real installer with empty pre-existing output base chain | **FAIL — A1 reproduced:** exact comparison found the production exemption retained at output handle 5 |
| Native real installer with pre-existing masked fwmark/table-100 rule | **FAIL — A1 reproduced:** watchdog exited 97 and retained the production local route |
| Native host nft/FIB inspection after all runs | PASS for host state — table 74; chains 1/2; one canonical rule per chain at handles 35/36; canonical fwmark/local route unchanged; no foreign or `ovd-gti-*` residue |
| Post-native VMM and alloc-cgroup probes | PASS — no Cloud Hypervisor process or allocation cgroup residue |
| Mutation testing | NOT RUN — repository rule requires one final DELIVER-wave gate |

### Iteration 7 verdict

**NEEDS_REVISION.** The Iteration 6 absent-table-100 and clean production-delta
failure is closed for the committed absent/output-absent partitions, and all
regressions remain green. A1 still has one high-severity exact-restoration
defect: recovery cannot subtract production additions from pre-existing output
and noncanonical-but-adopted FIB baselines. Two qualified native real-installer
counterexamples fail and leave production delta behind inside their disposable
namespaces. Return A1 to the original step-02-03 crafter. Do not start 02-04;
continue the same review/remediation cycle until a re-review records
**APPROVED** with zero unresolved blocker, high, or medium findings.

## Iteration 8 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_8`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated step reviewer)
- **Reviewed remediation:** `c1532a59ab7a5c9b6e70ad24a30112a6d893cea5`
- **Remediation parent:** `4dcd1261bcb620efdbcdd08f9f562856e7fd2921`
- **Step trailer:** exact `Step-Id: 02-03` on the reviewed commit
- **Review date:** 2026-08-29
- **Verdict:** **NEEDS_REVISION**

### Iteration 8 executive summary

The new clean-audit implementation closes the specific baseline-delta defects
reported in Iteration 7. It durably persists the complete normalized nft JSON,
raw GETRULE handles/userdata, and typed FIB JSON. Before every cleanup action,
an isolated audit reconstructs that baseline, proves all original table/chain/
rule handles, programs, counters, userdata, and FIB objects unchanged, and
classifies only the exact residual production delta. Exemptions are deleted by
their audited handles, created chains/tables only after exact structural proof,
and FIB deletion uses typed rtnetlink dumps plus the returned unique object.
The previous display-text mutation authority is gone from this path.

The expanded qualified-native matrix is genuine. It calls the unchanged real
installer for empty pre-existing output, output absence, table-without-
prerouting, empty output-without-prerouting, masked fwmark adoption, colliding
local-route adoption, and the combined adopted FIB pair while an unrelated
blackhole route remains present. Normal restoration and 210 applicable cleanup
journal/mutation/signal boundary runs return to byte-/handle-exact baselines.
The absent baseline, production-intent failures, panic, watchdog signal,
parent death, and foreign nft/FIB refusal partitions also remain green. No
stand-in replaces the typed `append-egress` -> `append-rule` -> `EOPNOTSUPP`
production failure.

A1 nevertheless remains open because the same file still has a second,
unaudited real-installer entry point. `assert_typed_input_hook_failure` calls
`DeltaScopedMalformedPrerouting::install()`, which selects
`try_install_in_mode(..., None)`: it persists no exact audit baseline and does
not set `allow-production-delta`. The non-audit recovery branch cannot remove
a production exemption inserted into a pre-existing output chain, and this
commit removed its old FIB cleanup entirely. The exact Iteration-7 native
counterexample therefore still fails. From empty, correctly typed pre-existing
prerouting/output chains, the real installer reaches the required typed
failure; restoration then leaves the output exemption at handle 5 plus the
canonical fwmark rule and local route. The test's exact baseline comparison
fails. The ordinary 13-test host selection hides this because that host is
already converged with all three shared objects.

The safe clean-audit path is now sound for the reviewed partitions, but adding
a second green path does not repair the existing production-calling path that
asserts the same exact-restoration contract and can contaminate a qualified
host. D1-D8, A2, and N1 remain closed. The default workspace is 2,279/2,279
green; the qualified native guest-stack selection is 13/13 green; the complete
real-VM module is 14/14 green; and exact C3 is green.

### Iteration 8 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 0 | — |
| Critical | 0 | — |
| High | 1 | A1 |
| Medium | 0 | — |
| Low | 0 | — |

### Prior-finding disposition audit

| Finding | Iteration 8 disposition | Evidence |
|---|---|---|
| D1 — guest admission read an unmounted sysfs file | **CLOSED (remains closed)** | Production ioctl admission and the complete native real-VM module remain green. |
| D2 — accepted pre-READY close became `DriverInternalError` | **CLOSED (remains closed)** | Typed VMM exit-code/signal mapping remains intact. |
| D3 — missing-token production/test split | **CLOSED (remains closed)** | Default workspace, complete native VM module, and exact native C3 replay pass. |
| D4 — Outcome Anchors and complements | **CLOSED for 02-03-owned tests** | No changed file revives a later-step body or overstates its complement. |
| D5 — C3 locator/clippy | **CLOSED (remains closed)** | Exact native C3 and workspace integration-feature clippy pass. |
| D6 — lease tests missed canonical writers | **CLOSED (remains closed)** | Every native command acquired the canonical host-global lease and passed source identity/preflight. |
| D7 — kernel/rootfs selection was optional | **CLOSED (remains closed)** | Native qualification required the explicit staged kernel and rootfs. |
| D8 — stale nested-KVM wording | **CLOSED (remains closed)** | Runtime evidence came from qualified native x86_64 KVM; fault probes used disposable native network namespaces. |
| A1 — wrong hook/typed cause/restoration | **PARTIALLY CLOSED; HIGH remains** | The new exact clean-audit mode closes all requested adopted-baseline partitions, but the existing unaudited real-installer test still leaks production nft/FIB delta on the same valid baseline. |
| A2 — incomplete/vacuous adopted scenarios | **CLOSED (remains closed)** | S-GTI-02/05/06/08/12 remain honest ignored later-step obligations and D7 remains incomplete. |
| N1 — diagnostic totality/cleanup proof | **CLOSED (remains closed)** | The complete native VM module and default regressions remain green. |

### A1 — one real-installer fixture path still bypasses exact delta recovery

**Severity:** HIGH  
**Status:** Open

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:211-234`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:1020-1131`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:1212-1224`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:1792-2116`
- feature-delta E08 exact fixture/production-delta restoration contract
- qualified native isolated-network-namespace reproduction of the exact
  `input_hook_fault_is_typed_and_normal_restoration_is_structurally_exact`
  identity

`try_install_clean` routes through the new persisted-baseline audit, but
`install` still routes through `try_install_in_mode` with `clean_audit_test =
None`. `assert_typed_input_hook_failure` uses `install` and then calls the
unchanged real installer. Thus the test whose name and body assert normal
structural exactness never receives the remediation's authority or cleanup
logic.

The consequences are visible in `restore_once`:

- the non-audit output branch runs only when output was baseline-absent and
  requires fixture ownership; it cannot delete a production-tagged exemption
  added to a baseline-present output chain;
- FIB cleanup now exists only under `allow-production-delta`, so the non-audit
  real-installer path never removes a fwmark rule or local route it caused
  production to create.

The exact native counterexample began with table `overdrive-mtls` and empty,
correctly typed prerouting/output base chains. Production added its FIB rule,
local route, and exemptions, then returned the required typed
`OutboundTproxyInstall` / `append-egress` / `append-rule` / `EOPNOTSUPP`
failure. The watchdog restored the original prerouting chain and exited
successfully, but `fixture.finish()` failed exact equality. The residual state
was:

- the original table and chains plus the canonical tagged output exemption at
  handle 5;
- canonical priority-32765 `fwmark 0x1 lookup 100`;
- canonical `local default dev lo table 100`.

This is the same valid pre-existing-output counterexample reported in
Iteration 7, now with both FIB objects additionally leaked. It is not exercised
by the new green partitions because those call `install_clean`; the ordinary
qualified-host selection passes only because its starting output exemption and
FIB objects already exist, so production creates no delta.

Route every fixture call that invokes the real installer through the persisted
exact-delta recovery path, including
`input_hook_fault_is_typed_and_normal_restoration_is_structurally_exact`, or
remove the unsafe duplicate path and retain one authoritative production
fixture. The exact prior counterexample must pass under its existing test
identity and finish with no output exemption or FIB residue. Non-production
watchdog construction tests may retain a narrower mode, but it must not be
usable around the real installer without exact recovery.

### Closed A1 portions

#### Exact persisted nft/FIB delta and typed deletion

The clean-audit path persists normalized nft objects plus raw ownership
metadata and complete FIB arrays. `exact_nft_delta` proves the baseline table,
chains, handles, programs, counters, and userdata, accepts only canonical
tagged exemption additions, and distinguishes created chains/tables from
pre-existing objects. Rule deletion uses audited nft handles. Typed FIB
deletion dumps live rtnetlink objects, requires uniqueness, and deletes the
returned object; the audit prevents either deletion when the baseline contains
an adopted object. The netlink unit suite is 45/45 green, and the native matrix
executes the real deletion paths.

#### Requested adopted-baseline and interruption partitions

The committed clean native scenario genuinely covers all Iteration-7 requested
partitions: empty pre-existing output, output absent, table without prerouting,
output without prerouting, masked fwmark, colliding route, combined adopted FIB
plus unrelated route, and foreign nft/FIB refusal. Across seven pre-existing
partition normals and 210 applicable cleanup fault runs, every exact baseline
comparison passes. The original absent, journal-failure, panic, signal,
parent-death, atomic-repair, and strict table-100 capture portions also remain
green.

### D3, A2, and N1 regression audit

- **D3:** production dispatch and VM assignment behavior are unchanged; the
  complete 14-test native VM module and exact C3 replay pass.
- **A2:** later-step S-GTI bodies remain ignored and no runtime claim is
  advanced by the fixture/netlink-only remediation.
- **N1:** the complete native VM module remains green, including the existing
  diagnostic-totality and all-absent rejection coverage; post-run VMM/cgroup
  probes are empty.

### Mechanical, DES, and verification evidence

- Exact reviewed remediation: `c1532a59ab7a5c9b6e70ad24a30112a6d893cea5`.
- Exact parent: `4dcd1261bcb620efdbcdd08f9f562856e7fd2921`.
- Exact trailer: `Step-Id: 02-03`.
- Remediation diff: 4 files, 859 insertions, 360 deletions.
- Complete fresh-step range `eb53c11d..c1532a59`: 34 files, 5,813
  insertions, 386 deletions.
- `git diff --check` passes for the remediation and complete step ranges.
- Fresh Iteration 8 DES cycle: RED `15:14:05Z`, GREEN `15:33:13Z`, COMMIT
  `15:41:15Z`, all `EXECUTED/PASS` and chronological before committer time
  `15:44:07Z`.
- The reviewed worktree had no tracked dirty files. Pre-existing untracked
  review/roadmap/design/DISTILL artifacts were preserved.

All Rust verification used exact tracked source at `c1532a59`. Lima used an
isolated target directory because the shared cache contained pre-existing
root-owned entries. The canonical metal runner supplied native x86_64 KVM and
disposable native network namespaces; every command acquired the host-global
lease, synced the reviewed source, and passed fail-closed preflight with
`/srv/vm/overdrive-testing/kernel` and
`/srv/vm/overdrive-testing/rootfs.ext4`. The only repository file this reviewer
changed is this native Markdown review artifact. No mutation testing was run.

| Verification | Result |
|---|---|
| `git diff --check 4dcd1261..c1532a59` and `eb53c11d..c1532a59` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Shell syntax for repository `*.sh` files | PASS |
| Lima complete `overdrive-netlink` unit suite | PASS — 45/45 |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets --no-fail-fast` | PASS — 2,279 passed, 23 skipped |
| Native complete current `guest_stack_mtls_egress` selection | PASS — 13/13 in 143.397s, 239 skipped |
| Native clean-audit adopted-baseline/fault matrix | PASS as part of the complete 13-test selection — seven partition normals plus 210 cleanup boundaries |
| Native complete `vm_walking_skeleton` module | PASS — 14/14 in 131.833s, 238 skipped |
| Native exact `c3_converge_twice_preserves_the_same_vm_network_plan` | PASS — 1/1 in 0.421s, 781 skipped |
| Native exact ordinary fixture with empty pre-existing output | **FAIL — A1 reproduced:** output exemption handle 5 plus canonical fwmark/local-route state remained |
| Native host nft/FIB inspection after all runs | PASS for host state — table 74; chains 1/2; one canonical rule per chain at handles 35/36; canonical fwmark/local route unchanged; no foreign or `ovd-gti-*` residue |
| Post-native VMM and alloc-cgroup probes | PASS — no Cloud Hypervisor process or allocation cgroup residue |
| Mutation testing | NOT RUN — repository rule requires one final DELIVER-wave gate |

### Iteration 8 verdict

**NEEDS_REVISION.** The Iteration-7 exact-baseline model, typed deletion, and
requested clean-audit partition/fault matrix are now implemented soundly and
pass qualified native execution. A1 still has one high-severity split-path
defect: the existing exact-restoration test invokes the real installer through
the unaudited fixture mode, and the exact prior native counterexample still
leaks a production output exemption and both FIB objects. Return A1 to the
original step-02-03 crafter. Do not start 02-04; continue the same
review/remediation cycle until a re-review records **APPROVED** with zero
unresolved blocker, high, or medium findings.

## Iteration 9 re-review

- **Review ID:** `code_rev_20260829_02_03_iteration_9`
- **Reviewer:** `nw-software-crafter-reviewer` (same isolated step reviewer)
- **Reviewed remediation:** `c33f0396edf86c1db888a4c36b751911258c48fb`
- **Remediation parent:** `c1532a59ab7a5c9b6e70ad24a30112a6d893cea5`
- **Step trailer:** exact `Step-Id: 02-03` on the reviewed commit
- **Review date:** 2026-08-29
- **Verdict:** **APPROVED**

### Iteration 9 executive summary

The final A1 split path is closed. The generic fixture now carries an explicit
recovery type state. `FixtureOnlyRecovery` can construct, stop, signal, and
exercise fixture-owned setup/cleanup faults, but exposes no production
invocation or production-intent journaling method. `ExactProductionRecovery`
is the only type with `invoke_real_installer_expect_typed_input_hook_failure`;
construction of that type first durably writes the exact normalized nft
baseline, raw GETRULE ownership metadata, complete FIB-rule JSON, and complete
table-100 route JSON, then enables the audited recovery subprocess. A source
scan finds one `install_outbound_tproxy` call, inside that exact-recovery-only
implementation. The old untyped `install`, `try_install_in_mode`, optional
`clean_audit_test`, and free production-invocation helper are absent.

The ordinary exact-restoration test no longer depends on the converged native
host. Its outer process enters a disposable native network namespace; its inner
process creates the exact Iteration-7 counterexample—an existing table with
empty, correctly typed prerouting and output base chains—then invokes the real
installer only through `ExactProductionRecovery`. It compares the full
nft/FIB/ownership snapshot with the pre-install baseline and separately proves
that output has no exemption, no adopted fwmark rule exists, and no adopted
local route exists. The exact test passes on qualified native metal. This is
the same test identity and starting state that failed in Iteration 8, not a new
parallel happy path.

The authoritative clean production scenario is otherwise unchanged. Its seven
adopted-baseline partitions, 210 applicable cleanup interruption boundaries,
production-intent journal boundaries, atomic duplicate repair, panic/signal/
parent-death recovery, and foreign nft/FIB fail-closed probes all pass again as
part of the complete 13-test native guest-stack selection. The exact audit and
typed nft/FIB deletion logic accepted in Iteration 8 remains intact. D1-D8,
A2, and N1 remain closed, and no new blocker, high, or medium defect was found.

### Iteration 9 defect counts

| Severity | Count | Findings |
|---|---:|---|
| Blocker | 0 | — |
| Critical | 0 | — |
| High | 0 | — |
| Medium | 0 | — |
| Low | 0 | — |

### Prior-finding disposition audit

| Finding | Iteration 9 disposition | Evidence |
|---|---|---|
| D1 — guest admission read an unmounted sysfs file | **CLOSED (remains closed)** | Production ioctl admission is unchanged and the complete qualified native VM module is 14/14 green. |
| D2 — accepted pre-READY close became `DriverInternalError` | **CLOSED (remains closed)** | Typed VMM exit-code/signal classification remains intact; the native module remains green. |
| D3 — missing-token production/test split | **CLOSED (remains closed)** | Production dispatch and assignment behavior are outside the narrow fixture refactor; the full native VM module and exact C3 replay pass. |
| D4 — Outcome Anchors and complements | **CLOSED for 02-03-owned tests** | No changed file activates a later-step body or weakens the accepted complement assertions. |
| D5 — C3 locator/clippy | **CLOSED (remains closed)** | Exact native C3 and workspace integration-feature clippy pass. |
| D6 — lease tests missed canonical writers | **CLOSED (remains closed)** | Every native command acquired the canonical host-global lease before preflight and execution. |
| D7 — kernel/rootfs selection was optional | **CLOSED (remains closed)** | Every native run failed closed through preflight with the explicit staged kernel and rootfs. |
| D8 — stale nested-KVM wording | **CLOSED (remains closed)** | Evidence came from qualified native x86_64 bare-metal KVM and native disposable network namespaces. |
| A1 — wrong hook/typed cause/restoration | **CLOSED** | The sole real-installer fixture call is available only on `ExactProductionRecovery`; the ordinary exact test now reproduces and restores the prior empty-chain counterexample, while the seven-partition/210-boundary matrix remains green. |
| A2 — incomplete/vacuous adopted scenarios | **CLOSED (remains closed)** | S-GTI-02/05/06/08/12 remain honestly ignored later-step obligations; this remediation advances no runtime claim. |
| N1 — diagnostic totality/cleanup proof | **CLOSED (remains closed)** | Default regressions and the complete native VM module pass; terminal host VMM/cgroup probes are empty. |

### A1 final split-path closure

**Status:** Closed

**Evidence**

- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:206-233`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:235-334`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:365-426`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:1242-1300`
- `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress_fault_fixture.rs:1867-1872`
- feature-delta E08 exact fixture/production-delta restoration contract

`DeltaScopedMalformedPrerouting<R>` now makes recovery authority part of its
type. The common implementation captures the baseline, but only
`ExactProductionRecovery::audit_test` supplies the audit executable identity
and causes all four baseline files plus `allow-production-delta` to be durably
written. The production constructor is fixed to the real table. Production
intent journaling and the real installer call are methods solely on
`DeltaScopedMalformedPrerouting<ExactProductionRecovery>`. Conversely, the
fixture-only constructors return `DeltaScopedMalformedPrerouting<FixtureOnlyRecovery>`,
which has neither method. There is no optional-audit constructor remaining.

`assert_typed_input_hook_failure` now obtains its fixture from
`install_clean_fixture`, whose return type is explicitly
`DeltaScopedMalformedPrerouting<ExactProductionRecovery>`. The ordinary test
creates the formerly failing baseline inside `unshare --net`, captures it,
runs the unchanged typed `OutboundTproxyInstall` -> `append-egress` ->
`append-rule` -> `EOPNOTSUPP` assertion, and demands exact equality after
watchdog recovery. Its explicit terminal assertions prove all three former
residues absent: the output exemption, priority-32765 fwmark/table-100 rule,
and canonical local default route.

The exact qualified-native invocation passes 1/1 in 0.352 seconds with 251
tests filtered. The complete guest-stack selection passes 13/13 in 143.616
seconds and therefore also re-executes the clean production test containing
all seven baseline partitions and 210 cleanup boundaries. Static review of the
unchanged audit path confirms that every cleanup action still re-audits the
live object graph, deletes exemptions by audited handles, deletes only proven
created chains/tables, and delegates FIB cleanup to the typed unique-object
netlink APIs accepted in Iteration 8. Foreign state still makes the watchdog
fail closed before deletion and is then removed only by explicit test-owned
manual teardown.

### D3, A2, and N1 regression audit

- **D3:** no production dispatch, VM assignment, or action-shim code changed.
  The complete 14-test native `vm_walking_skeleton` module and exact native C3
  replay pass.
- **A2:** the five later-step S-GTI bodies remain ignored with their owner-step
  explanations. This fixture-only remediation makes no premature behavior
  claim.
- **N1:** default workspace regressions are 2,279/2,279 green. The complete
  native VM module remains 14/14 green, and final host inspection found no
  Cloud Hypervisor process or allocation cgroup.

### Mechanical, DES, and verification evidence

- Exact reviewed remediation: `c33f0396edf86c1db888a4c36b751911258c48fb`.
- Exact parent: `c1532a59ab7a5c9b6e70ad24a30112a6d893cea5`.
- Exact trailer: `Step-Id: 02-03`.
- Remediation diff: 2 files, 236 insertions, 145 deletions.
- Complete fresh-step range `eb53c11d..c33f0396`: 34 files, 5,904
  insertions, 386 deletions.
- `git diff --check` passes for the remediation and complete step ranges.
- Fresh Iteration 9 DES cycle: RED `16:04:32Z`, GREEN `16:11:42Z`, COMMIT
  `16:20:34Z`, all `EXECUTED/PASS`, chronological and before committer time
  `16:20:41Z`.
- The reviewed worktree had no tracked dirty files. Pre-existing untracked
  review/roadmap/design/DISTILL artifacts were preserved.

All Rust verification used exact tracked source at `c33f0396`. Lima used an
isolated target directory to avoid the pre-existing ownership problem in the
shared cache. The canonical metal runner acquired its global lease, verified
the synchronized source identity, and passed fail-closed native x86_64/KVM
preflight with `/srv/vm/overdrive-testing/kernel` and
`/srv/vm/overdrive-testing/rootfs.ext4` for every run. The only repository file
this reviewer changed is this native Markdown review artifact. No mutation
testing was run.

| Verification | Result |
|---|---|
| `git diff --check c1532a59..c33f0396` and `eb53c11d..c33f0396` | PASS |
| `cargo fmt --all -- --check` | PASS |
| Shell syntax for repository `*.sh` files | PASS |
| Lima workspace `cargo check --workspace --all-targets --features integration-tests` | PASS |
| Lima workspace `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| Lima default `cargo nextest run --workspace --all-targets --no-fail-fast` | PASS — 2,279 passed, 23 skipped |
| Native complete current `guest_stack_mtls_egress` selection | PASS — 13/13 in 143.616s, 239 skipped |
| Native clean exact-baseline/fault matrix | PASS as part of the complete 13-test selection — seven partition normals plus 210 cleanup boundaries, journal gaps, atomic repair, and foreign-state refusals |
| Native exact ordinary prior counterexample | PASS — 1/1 in 0.352s, 251 skipped; empty pre-existing prerouting/output restored with no output exemption, fwmark rule, or local route |
| Native complete `vm_walking_skeleton` module | PASS — 14/14 in 131.671s, 238 skipped |
| Native exact `c3_converge_twice_preserves_the_same_vm_network_plan` | PASS — 1/1 in 0.442s, 781 skipped |
| Native terminal nft/FIB inspection | PASS — feature table handle 74; chains 1/2; one exact canonical mark-2 exemption per chain at handles 35/36; no `ovd-gti-*` table; canonical priority-32765 fwmark rule and canonical table-100 local route unchanged |
| Post-native VMM and allocation-cgroup probes | PASS — no Cloud Hypervisor process and no `alloc-*` cgroup |
| Mutation testing | NOT RUN — repository rule requires one final DELIVER-wave gate |

### Iteration 9 verdict

**APPROVED.** A1's remaining unsafe production entry point is removed, the
fixture's recovery authority is enforced by type state, and the existing
ordinary exact-restoration identity now passes the exact prior native
counterexample with no production delta left behind. All prior findings are
closed, the full baseline/fault matrix and regressions remain green, and there
are zero unresolved blocker, critical, high, medium, or low findings. Step
02-03 may advance to the next roadmap step subject to the orchestrator's normal
mechanical gate.
