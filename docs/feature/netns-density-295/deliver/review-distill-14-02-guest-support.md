# DISTILL review — STEP 14-02 GuestKit execution support

| Field | Value |
| --- | --- |
| Feature | `netns-density-295` |
| Step | `14-02` — native fault evidence |
| Review role | fresh independent `nw-acceptance-designer-reviewer` |
| Review iteration | 10 |
| Date | 2026-11-01 |
| Reviewed support files | `crates/overdrive-testing/bin/fwd_guest_suite.rs`; `xtask/src/guest_kit_prepare.rs` |
| Reviewed UDP alignment files | `crates/overdrive-init/src/guest_flow/sim.rs`; `crates/overdrive-init/src/guest_flow/quint_udp_slots.rs`; `specs/quint/guest-flow-owner/udp_slots.qnt`; canonical `specs/quint/guest-flow-owner/evidence/` |
| Support-package verdict | **APPROVED** |
| Acceptance-coverage verdict | Worker leg-F: **APPROVED and evidence-complete**. Bounded UDP released-burst body: **APPROVED and GREEN at the owner/Sim boundary** after closing F-05. Its Sim/Quint alignment is approved with the explicit kernel-adapter evidence limits in iteration 10. |
| Native-execution disposition | Standalone worker leg-F remains GREEN and approved. Exactly one GuestKit validation of the existing staged CLI identity `integration::vsock_fault_matrix::closed_udp_sockets_release_their_slots_before_and_after_pairing` is ready. The source-local released-burst owner/Sim body is GREEN support evidence and is not a GuestKit selector. The native 1,100-socket churn body remains RED until an actual exact pass. Direct bare-metal/nextest evidence remains invalid. |

## Scope and evidence policy

This review is a source-only audit. It performs no native, kernel, Metal,
Lima, GuestKit or microVM execution. It changes no production source, test
body, marker, DES log or commit. The only authored file is this review.

The retained scoped source gate `8e22a77cf8ed1ca23c257d22` exited zero for
format, check and Clippy with `-D warnings` across xtask, the GuestKit runner,
the CLI integration target and the worker leg-F target. The reviewed source
still has the handed-off SHA-256 identities:

| Source | SHA-256 |
| --- | --- |
| `xtask/src/guest_kit_prepare.rs` | `fe47ec596d734877ac052d3f40ba6071816d1fa16d33e174709d4382af68d360` |
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | `598676f23f21616b870d7c0115746997552d2c4c40cd44e8cf4fb50659b4f6e7` |

The local source audit also passed `git diff --check`. At iteration 1, both
acceptance files were byte-identical to reviewed HEAD
`499b0345eba67b472a4a8f878fc269b7db5725e7`:

| Acceptance file | Git blob |
| --- | --- |
| `crates/overdrive-cli/tests/integration/vsock_fault_matrix.rs` | `a5340211fc4691fa539d20b43e9e83e02bded153` |
| `crates/overdrive-worker/tests/leg_f_repair.rs` | `6e3feedce0e4d6835d7512205c14b7ee8eb0f060` |

The direct bare-metal receipts under `.context/netns-density-295/14-02/raw`
receive no acceptance credit. As recorded in `incident-evidence.md`, they did
not use GuestKit. The read-only audit proves the named netns, veth, cgroup
children and recorded processes are absent, while an `ip overdrive-mtls`
table with a TPROXY rule to port 33823 remains with unknown causality. This
review neither attributes nor mutates that residual state.

## Iteration 1

### What is correct

The support delta extends the existing direct exact selector without changing
the default canonical group:

- `guest_kit_prepare::IN_GUEST_LANES` stages the existing CLI integration ELF
  and now also stages the standalone worker `leg_f_repair` ELF from the current
  Cargo test target. `build_in_guest_tests` consumes Cargo's reported
  executable and copies that ELF to the existing
  `target/guest-kit/in-guest/<package>--<target>` contract path. The fixture
  reads the same path through `InGuestTests::prepared`; no source-generated
  runner copy or alternate build wrapper exists.
- `STEP14_02_CLI` names exactly the five existing `vsock_fault_matrix`
  identities and `STEP14_02_WORKER` names exactly the one existing standalone
  worker identity. Each entry expects one body. The source-specific manifests
  therefore reconcile to exactly five CLI bodies and one worker body.
- `focused_selection` maps each identity to its existing package and Cargo
  test target. A zero match fails closed. Before execution the source manifest
  is checked against its fixed count. The execution requires an exact libtest
  selector, exit zero, the exact `1 passed; 0 failed` summary and no recognized
  runtime-skip diagnostic.
- A successful STEP 14-02 exact run emits its source, identity,
  source-manifest count and `canonical_117_credit=0`. The ordinary
  `cgroup-default` path does not add STEP 14-02 bodies, so the previously
  approved default canonical 117-body result remains unchanged.
- The support introduces no scenario, test, pending-marker change, production
  API, production fault hook, group name, execution lane, generated source or
  mutation/Lima path. Its new Rust items are private to the test-support
  executable; the xtask change only adds the existing Cargo test target to the
  established staging manifest.

### Disposable-guest boundary

The selected body cannot execute on the physical Metal kernel through this
route:

1. `run_cgroup_default` returns into the focused exact route before the
   canonical group when `OVERDRIVE_FWD_GUEST_EXACT` is present
   (`fwd_guest_suite.rs:2493-2496`).
2. `run_focused_exact_nested_vm_body` resolves only a declared exact identity
   and calls `run_nested_vm_test_binary` with the existing package/target and
   one-cell manifest (`fwd_guest_suite.rs:2249-2282`).
3. `run_nested_vm_test_binary` reads the staged ELF, builds a fresh GuestKit
   image and boots a fresh outer guest before it invokes any test body
   (`fwd_guest_suite.rs:2107-2127`; `GuestRun::boot_with_vcpus` at
   `:1262-1301`). All module loads, device permission changes, cgroup/bpffs/
   securityfs mounts, sysctls, nftables, network namespaces, veths and test
   processes are issued through the scripted guest peer (`:2125-2166`).
4. `GuestKit::boot` uses the production `CloudHypervisorVmm::create` over a
   `VmConfig` with `VmConfinement::confined` (`guest_kit.rs:542-624`). The
   production renderer applies the configured uid/gid-4200 identity,
   prlimit/setpriv chain, launch seccomp, Cloud Hypervisor seccomp and Landlock
   rules. The test runner does not render a separate VMM command line.
5. The five CLI bodies launch their production workload VMs from inside that
   outer disposable guest. The worker body launches no nested VMM; its
   `HostMtlsIntercept`, nft, policy-route, RLIMIT and socket effects occur in
   the disposable outer guest's kernel. In both cases `GuestRun::finish`
   prints the full console, powers off the outer guest and waits for its VMM
   exit (`fwd_guest_suite.rs:1492-1503`).

The Metal prefix supplies hardware KVM only. The GuestKit launch and outer
guest are the actual isolation boundary. Host-side preparation creates files,
builds/stages ELFs and filesystem images and reads module sources; it does not
load a module or change physical-host sysctls, nftables, cgroups, routes,
network namespaces or interfaces.

## Blocking coverage finding

### F-01 — the existing leg-F scenario bypasses the required TPROXY capture

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`  
**Severity:** blocker  
**Disposition:** OPEN; return to the STEP 14-02 acceptance designer before
worker-cell native execution.

The accepted contract is exact. Feature-delta Roadmap impact 10-02 states
that the EMFILE stimulus queues one **TPROXY-captured** connection and that,
after repair, a new **TPROXY-captured** connection accepts on the replacement
listener (`feature-delta.md:1724`). Validation V-8 repeats the same post-repair
oracle (`feature-delta.md:13181`), and the executable roadmap criterion repeats
it at `deliver/roadmap.json:1540`.

The existing scenario explicitly substitutes a different input path:

- its module contract says the connection is a direct connection rather than
  TPROXY-captured and claims another suite covers capture
  (`leg_f_repair.rs:50-53`);
- the EMFILE stimulus calls `connect(&client, leg_f)`, where `leg_f` is the
  listener's bound loopback address (`:266`, `:274-283`);
- the post-repair oracle calls `connects(leg_f)`, again directly targeting the
  replacement listener (`:297-312`).

This body can remain green if the production TPROXY rule, fwmark policy route
or delivery to leg F is absent or broken. A direct TCP handshake is not a
TPROXY-captured connection, so the scenario does not prove the accepted V-8
boundary. The earlier direct host pass cannot fill this gap and, independently,
has no GuestKit credit.

**Smallest compliant DISTILL correction:** amend this one existing scenario;
do not add a scenario, production seam, production API, fault hook or new
architecture. Keep its EMFILE, `TaskFailed { leg: F }`, still-`LISTEN`,
release-before-rebind, descriptor, inode and audit assertions. Replace only
the two contract-relevant direct leg-F dials—the queued EMFILE stimulus and
the post-repair connection—with real connections that traverse the existing
production TPROXY program and policy route into the current leg-F listener.
Use bounded test-local support in `leg_f_repair.rs` (or an already-existing
test-support fixture) to supply the workload-side network origin and retain
the required descriptor-order rule by creating the stimulus socket before
lowering RLIMIT or by opening it from a separate test process. Update the
scenario prose and Contract Shape universe to name the TPROXY-captured input.
If that body needs one additional existing tool or ELF inside the outer guest,
stage only that artifact through the current GuestKit support path.

The stale DISTILL catalogue row should be corrected with the same edit: it
currently names `shutdown(SHUT_RD)` at `distill/test-scenarios.md:197`, while
the accepted contract explicitly rejects that stimulus and requires EMFILE.
This is documentation alignment for the same existing scenario, not a new
test obligation.

No design ambiguity exists: the semantic input, failure, repair and observable
outcome are already pinned. Routing this finding to DESIGN or replacing
TPROXY with a direct connect would weaken an accepted contract.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The bounded set is a native failure matrix; the leg-F case is itself a resource-starvation/recovery path. |
| GWT / one behavior | 9/10 | Each exact Rust body expresses one fault and its outcome; support changes no body. |
| Domain language | 9/10 | Owner death, guest reset, VMM death, stalled reader, slot release and leg-F repair remain precise domain terms. |
| Coverage completeness | 3/10 | Five fault-matrix criteria are represented, but the exact V-8 TPROXY-captured leg-F input is absent. |
| User/operator value | 9/10 | The five CLI cells retain production `serve`/`deploy` journeys and observable far-end/lifecycle outcomes. |
| Priority validation | 10/10 | The support closes the proven unsafe-execution boundary without adding new behavior. |
| Observable behavior | 8/10 | Typed failure, socket state, descriptor, inode and audit outcomes are observable; the missing TPROXY input invalidates the complete V-8 claim. |
| Traceability | 10/10 | The six identities trace to P6-S14, V-8/D-9 and the exact 14-02 roadmap criteria. |
| Walking-skeleton boundary | 3/10 | The support boundary is sound, but the leg-F body bypasses the production capture boundary required by its criterion. |

The blocker and scores below 5 require `rejected_pending_revisions` for the
acceptance package even though the execution-support package itself is
approved.

## Test-design mandates and Contract Shape

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | **FAIL for leg F** | The worker and real `HostMtlsIntercept` are used, but both contract-relevant connections bypass the required production TPROXY capture boundary. The five CLI cells pass. |
| CM-B — domain abstraction | PASS | Existing scenario names and outcomes retain the accepted domain vocabulary; runner mechanics stay private support. |
| CM-C — complete focused journey | **FAIL for leg F** | The fault and repair are present, but the accepted captured-connection input and post-repair captured acceptance are missing. |
| CM-D — fixture isolation | PASS | The runner contains effects in a disposable outer GuestKit guest and uses existing production composition. |
| CM-E — universe-bound assertion | PASS / layer N/A | These are layer-4+ real-kernel integration examples; direct observable assertions are permitted. |
| CM-F — layer-dependent PBT | PASS | The native cells are explicit examples; no PBT machinery was added. |
| CM-G — two-tier rich journey | N/A | No new rich chained journey is introduced. |
| CM-H — explicit integration sad paths | PASS | The accepted faults remain individually named examples. |

All six live selected bodies retain an exact
`/// CONTRACT_SHAPE: bounded-change.` declaration and all six retain their
`pending DELIVER step 14-02` ignore marker. No marker or body changed in this
support package. F-01 requires correcting the existing leg-F declaration's
universe to include the accepted TPROXY-captured input; it does not authorize
a new scenario.

## Mechanical disposition

| Check | Result |
| --- | --- |
| Changed support scope | PASS — two support files only; no production/test-body/marker change |
| Exact CLI identity set | PASS — five existing `vsock_fault_matrix` bodies, one expected body each |
| Exact worker identity set | PASS — one existing standalone `leg_f_repair` body |
| Source-specific count | PASS — CLI 5, worker 1, checked before execution |
| Zero-match behavior | PASS — fails closed |
| Exit and libtest summary | PASS — exact exit zero and exact expected-pass summary required |
| Runtime skip | PASS — recognized skip diagnostics reject the run |
| Default canonical accounting | PASS — 117 unchanged; every focused result reports zero canonical credit |
| ELF staging | PASS — current Cargo-reported executable copied to the existing prepared path |
| Outer GuestKit confinement | PASS — production `CloudHypervisorVmm`, uid/gid 4200, seccomp and Landlock |
| Physical-host kernel effects | PASS — none in the selected-body route |
| New group/lane/scenario/API/generated runner | PASS — none |
| Retained scoped source gate | PASS — `8e22a77cf8ed1ca23c257d22`, exit 0 |
| Local `git diff --check` | PASS |
| Leg-F accepted input path | **FAIL** — direct connections replace required TPROXY-captured connections |

## Native-execution authorization boundary

The support is ready for the five exact CLI identities, one at a time, through
the existing `cgroup-default` GuestKit selector. Their future results must be
reported as five-source-manifest evidence with zero canonical-117 credit.

The exact worker identity is **not ready to execute for acceptance credit**.
Its staging and disposable-guest route are approved, but execution must wait
for F-01 remediation and independent re-review. A run of the unchanged direct
body, even inside GuestKit and even if green, would prove the wrong input path.

No physical-host cleanup, rerun or exploratory kernel action is authorized by
this review. The retained bare-metal receipts remain incident evidence only.

## Iteration 1 verdict

Iteration 1 approves the bounded STEP 14-02 GuestKit execution-support source.
The complete acceptance package remains **REJECTED PENDING REVISION** on one
precise, existing-scenario coverage blocker: V-8 requires TPROXY-captured
connections before the EMFILE failure and after repair, while the body uses
direct connections to leg F. Remediate that body and its stale catalogue row,
then return the same support and scenario for iteration 2 before worker-cell
native execution.

## Iteration 2

### Remediation scope and retained gates

Iteration 2 reviews only the authorized F-01 remediation:

- `crates/overdrive-worker/tests/leg_f_repair.rs` — the existing scenario body
  and its test-local fixture support;
- `docs/feature/netns-density-295/distill/test-scenarios.md` — the existing
  scenario catalogue row.

No production file, public API, Cargo manifest, test identity, pending marker,
runner, staging lane, GuestKit group, DES event or commit changed. The retained
remediation source gate `6f0345f1c1dfc4532dc147600` exited zero for format,
check and Clippy with `-D warnings`. The reviewed remediation identities are:

| Source | SHA-256 |
| --- | --- |
| `crates/overdrive-worker/tests/leg_f_repair.rs` | `d2e15d38119855321bfaadc9fd2b8753d2ef6ae479cf3f451fbb5bd2620098a2` |
| `docs/feature/netns-density-295/distill/test-scenarios.md` | `edff6be20d3ccf54ba43f4b8d3f3e7e6a8c3f091053ce4c7bac5e0034a6db7a3` |

This re-review remains source-only. It ran no native, kernel, Metal, GuestKit,
microVM, Lima or mutation command. Scoped `git diff --check` passes. The five
CLI bodies, the two GuestKit support files and all pending markers remain
unchanged from iteration 1.

### F-01 remediation audit

**Disposition: CLOSED.**

The existing scenario now proves both TPROXY-captured connections named by
Roadmap impact 10-02 and V-8:

1. `CapturedPath::provision` acquires two existing `TestCidrLease` leases and
   creates one test-owned workload netns/veth. The workload socket therefore
   enters the outer guest's host namespace through the named host-side veth
   (`leg_f_repair.rs:239-303`). The dialled address comes from the disjoint
   `10.250.0.0/16` test pool while leg F is bound to `127.0.0.1`, so it is not
   a direct leg-F dial. A `/32` makes the address locally routable, but no
   listener is created there.
2. The scenario asserts that the dialled destination has no listening socket,
   then installs the existing production `install_outbound_tproxy(host_if,
   leg_f.port())` rule (`:441-448`). That production rule matches TCP ingress
   on the workload's host-side veth and redirects it to the current
   `IP_TRANSPARENT` leg-F listener through the existing fwmark policy route.
   A successful connection to the unserved, route-disjoint destination can
   therefore occur only through the TPROXY delivery.
3. The stimulus socket is created after entering the workload netns on a
   dedicated thread and before `RLIMIT_NOFILE` is lowered (`:314-332`,
   `:450-456`). Only its `connect(2)` runs under the lowered limit. The
   connection targets `capture.destination`, not `leg_f`, and the subsequent
   typed `TaskFailed { leg: F }` proves that leg F attempted `accept(2)` and
   hit the required descriptor-exhaustion path (`:457-466`).
4. The existing failure oracles remain intact: after restoring the limit, the
   original socket is still `LISTEN` with the same inode; repair returns `Ok`
   with no `EADDRINUSE`; the bound address is unchanged; the new listener has
   a new inode; and the old socket descriptor is closed (`:468-488`).
5. The pre-repair per-workload TPROXY guard is dropped before convergence, so
   the worker observes its exact owned shared program. After repair, the same
   production outbound rule is installed against the repaired leg-F port and
   a second workload-netns socket dials the unserved destination
   (`:489-493`). `accepted_and_closed` requires EOF, reset or abort within two
   seconds and rejects timeout/would-block (`:341-368`, `:494-497`). A TCP
   handshake left only in the listener's accept queue would time out; the
   bounded terminal result proves the replacement task accepted and dequeued
   the TPROXY-captured connection. The existing post-repair audit then proves
   both shared-owner tasks are live (`:498-501`).

This is the smallest correction to the existing scenario. It adds no new
scenario, fault, acceptance outcome, production seam or architecture. The
bounded accepted-and-closed complement is necessary to distinguish an actual
accept from a queued handshake; it does not broaden V-8.

### Fixture lifecycle and cleanup

The test-local fixture is bounded and fail-safe within the disposable outer
guest:

- `CapturedPath` owns the workload namespace, both veth names, the loopback
  `/32` and both `TestCidrLease` tokens. Its `Drop` deletes the veth, namespace
  and `/32` before the lease fields release their durable records
  (`leg_f_repair.rs:243-250`, `:305-338`; `cidr_lease.rs:119-131`). A partial
  `provision` panic also drops the already-constructed fixture.
- Both production `TproxyInterceptGuard`s are explicitly dropped. Their
  existing RAII owner deletes only the exact per-workload prerouting rule by
  kernel handle; the shared program and sibling rules are untouched
  (`mtls_intercept.rs:1037-1095`).
- Cleanup commands are best-effort in destructors, as existing real-kernel
  fixture convention requires. Any abnormal-process residue still dies with
  the disposable outer guest. No object, route, nft rule, lease registry or
  process reaches the physical Metal host through the approved exact GuestKit
  route.
- The fixture uses only `ip`, `ss`, `uname`, Rust netlink and the existing
  test ELF. Those dependencies are already staged by the approved
  `run_nested_vm_test_binary` support; no runner or lane change is required.

The standalone scenario retains its one exact
`/// CONTRACT_SHAPE: bounded-change.` declaration and
`#[ignore = "pending DELIVER step 14-02"]` marker
(`leg_f_repair.rs:394-402`). Its universe now names the TPROXY-captured
accepted-and-closed connection. The catalogue row now names EMFILE, the
production outbound TPROXY path and the second captured-connection oracle;
the stale `shutdown(SHUT_RD)` and stale module name are removed
(`distill/test-scenarios.md:197`).

### Iteration 2 acceptance dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The existing resource-starvation and repair path remains an explicit native failure case. |
| GWT / one behavior | 9/10 | One existing scenario retains one fault, repair and acceptance outcome. |
| Domain language | 9/10 | Leg F, captured workload connection, descriptor exhaustion and repair remain exact domain terms. |
| Coverage completeness | 10/10 | The EMFILE stimulus and post-repair connection now both traverse the required production TPROXY path. |
| User/operator value | 9/10 | The replacement listener demonstrably resumes captured workload acceptance after the real kernel failure. |
| Priority validation | 10/10 | The remediation changes only the proven F-01 gap. |
| Observable behavior | 10/10 | Typed failure, LISTEN state, address, inode, descriptor, bounded captured acceptance and audit are all observed. |
| Traceability | 10/10 | Source and catalogue align exactly with Roadmap impact 10-02, V-8 and roadmap criterion 14-02. |
| Walking-skeleton boundary | 10/10 N/A | This is a focused Tier-3 scenario; its required real-adapter boundary now passes. |

### Iteration 2 mandate disposition

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Both contract-relevant connections cross the existing production outbound TPROXY rule and real `HostMtlsIntercept`; direct leg-F access remains only a pre-fault listener control. |
| CM-B — domain abstraction | PASS | Scenario and catalogue preserve the accepted domain vocabulary; netns/veth mechanics remain private fixture detail. |
| CM-C — complete focused journey | PASS | TPROXY capture, EMFILE failure, still-`LISTEN`, repair, replacement acceptance and audit form the complete accepted V-8 journey. |
| CM-D — fixture isolation | PASS | Test-local RAII owns every added fixture resource, and the approved runner contains all kernel mutation in the disposable outer guest. |
| CM-E — universe-bound assertion | PASS / layer N/A | Layer-4+ real-kernel integration permits direct observable assertions; the Contract Shape universe is exact. |
| CM-F — layer-dependent PBT | PASS | The native fault remains one explicit example. |
| CM-G — two-tier rich journey | N/A | No new rich chained journey exists. |
| CM-H — explicit integration sad paths | PASS | The EMFILE sad path remains explicitly named and bounded. |

### Iteration 2 mechanical disposition

| Check | Result |
| --- | --- |
| Changed remediation scope | PASS — one existing test body plus its existing catalogue row |
| New scenario/test identity | PASS — none |
| Pending marker | PASS — unchanged |
| Contract Shape | PASS — exact declaration retained and corrected universe recorded |
| Production API or fault seam | PASS — none added; existing production `install_outbound_tproxy` is used |
| TPROXY stimulus | PASS — workload-netns socket → host veth → production rule → leg F |
| Direct-target exclusion | PASS — dialled leased address is disjoint from loopback leg F and has no listener |
| RLIMIT order | PASS — stimulus socket exists before the soft limit is lowered |
| Failure oracle | PASS — typed leg-F `TaskFailed`, same still-`LISTEN` inode |
| Repair oracle | PASS — `Ok`, same address, new inode, old descriptor absent |
| Acceptance oracle | PASS — second captured connection terminates within 2 s; queued-only handshake fails |
| Post-repair audit | PASS |
| Fixture cleanup | PASS — veth/netns/loopback `/32`, leases and per-workload TPROXY guards owned by RAII |
| GuestKit boundary | PASS — all added kernel mutation runs only inside the existing disposable outer guest |
| Catalogue alignment | PASS — EMFILE and both captured connections recorded |
| Retained remediation source gate | PASS — `6f0345f1c1dfc4532dc147600`, exit 0 |
| Scoped `git diff --check` | PASS |

## Iteration 2 native-execution authorization boundary

All six STEP 14-02 identities are now ready for bounded native execution, one
exact identity per run, through the existing GuestKit route only:

- five `step14-02-cli` identities, each reported against the exact five-body
  CLI source manifest;
- one `step14-02-worker` identity, reported against the exact one-body worker
  source manifest.

Every result must retain `canonical_117_credit=0`, exact libtest accounting,
zero runtime skips, full outer-guest console and disposable-guest cleanup.
This approval does not authorize the roadmap's original bare-metal `cargo
nextest` commands, direct SSH test execution, or any physical-host cleanup.
The earlier bare-metal receipts remain invalid as acceptance evidence.

## Iteration 2 verdict

**APPROVED.** F-01 is closed. The existing leg-F scenario now drives the
accepted TPROXY-captured EMFILE stimulus and proves the repaired listener
actually accepts and dequeues a second captured connection. The correction
preserves every prior failure and repair oracle, adds only bounded test-local
fixture support, weakens no contract and changes no production surface.
Together with the iteration-1-approved GuestKit support, all six STEP 14-02
identities are ready for exact disposable-guest execution.

## Iteration 3

### Contract-reconciliation scope and evidence

Iteration 3 reviews the bounded correction of the existing guest-reset
scenario after its first valid GuestKit RED:

- `crates/overdrive-cli/tests/integration/vsock_fault_matrix.rs` — the existing
  scenario selects a restartable Service fixture; the other existing Job
  scenarios receive an explicit unchanged fixture selector;
- `crates/overdrive-cli/tests/integration/vsock_support.rs` — one private
  helper renders the same guest command and hold arguments as a one-replica
  Service with one TCP admission listener and no probes;
- `docs/feature/netns-density-295/distill/test-scenarios.md` — the existing
  native-lane catalogue records why the successor-bearing cell is a Service.

No production source, production API, public test identity, scenario, runner,
GuestKit lane, fault mechanism, pending marker, DES event or commit changed.
The retained source gate `59f5fa9edcd51cd5edc3ff050` exited zero for
format, check and Clippy with `-D warnings`. Scoped `git diff --check` also
passes. Current source identities are:

| Source | SHA-256 |
| --- | --- |
| `crates/overdrive-cli/tests/integration/vsock_fault_matrix.rs` | `f4a88491c713de6fb99d0c2ee2738a22515de8723e8fa1671171278bd55770c8` |
| `crates/overdrive-cli/tests/integration/vsock_support.rs` | `b479dcb7083985158ea0c613508ce6762469f31da739e44a2d11fb138967e4d4` |
| `docs/feature/netns-density-295/distill/test-scenarios.md` | `de2d96b7f7bf3f4f1ab39b43084b902897686ecb0924b9ae06deb4856d9db14e` |

This reviewer ran no native, kernel, Metal, GuestKit, microVM, Lima or
mutation command and changed only this review artifact.

### F-02 — successor-bearing Job fixture contradicted RESTART-SSOT

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`  
**Severity at discovery:** blocker  
**Disposition:** CLOSED by iteration 3 fixture correction.

This finding is reproduced, not hypothetical. Exact GuestKit run
`5a92987da14f08aef2b68d2d` drove the original guest-reset body through the
disposable outer guest and nested production workload VMM. The guest PID-1
crash ended the VMM; the host flow reset; and the original allocation reached
`Failed` with exact reason `WorkloadCrashedImmediately`. The body then waited
60 seconds for a successor that never appeared and exited 101. The retained
row records `restart_count: 0`, terminal `Failed`, and no second allocation.

That RED matches the accepted RESTART-SSOT contract:

- the reconciler finalizes a Job's first natural exit, including a crash,
  before its restart branch, so no Job crash is placed again
  (`feature-delta.md:8948-8953`);
- `restart_after_failure` is false for a Job natural exit
  (`feature-delta.md:9042-9047`);
- the Job stream reports `AttemptFailed { will_restart: false }` followed by
  terminal `Failed` (`feature-delta.md:9191-9193`).

Changing production to restart the Job would violate these accepted clauses.
Removing the successor assertion would weaken the existing R5-56 outcome that
a crash-class guest reset is charged against the restart budget. The correct
DISTILL repair is therefore a restartable workload fixture for this one
successor-bearing scenario.

### Fixture correction

The correction is exact and does not invent restart behavior:

1. `HoldWorkload` is a private two-case fixture selector. Owner death and VMM
   SIGKILL explicitly remain `Job`; only the guest-reset scenario selects
   `RestartingService` (`vsock_fault_matrix.rs:111-132`, `:174-177`,
   `:234-239`, `:274-277`). The NO-SURVIVE Job/CID-reclamation scenario and
   every other workload-kind choice are unchanged.
2. `Lane::deploy_hold_service` is `pub(super)` test support. It submits through
   the same public deploy handler as every lane workload and renders the same
   `GUEST_PEER_GUEST_PATH`, caller-supplied `hold` arguments, guest-kit kernel,
   rootfs, CPU and memory. The only semantic fixture differences are
   `[service]`, `replicas = 1`, and one ordinary TCP listener on port 8080
   (`vsock_support.rs:324-342`). The listener satisfies Service admission; it
   is not used as a new fault or oracle. No startup, readiness or liveness
   probe is declared.
3. The accepted RESET-EXIT stimulus and all original oracles remain in the
   existing body (`vsock_fault_matrix.rs:231-263`):
   - the unchanged guest `hold` command still receives
     `crash-after=1048576`, producing the PID-1 crash after the healthy
     bidirectional-flow baseline;
   - the original VMM process must disappear within 60 seconds;
   - the remote peer's first terminal event must be `Reset`;
   - the original allocation must retain the exact
     `WorkloadCrashedImmediately` reason and state `Failed`;
   - a row with a distinct allocation id must appear, proving the restartable
     Service took the successor-placement branch.

The distinct successor is the native single-crash evidence that the failure
was charged. R5-56's complete “until the budget is spent” behavior remains
owned by its seeded-sim and Quint evidence layers; this one Tier-3 cell does
not claim to exhaust the budget. No restart count, terminal outcome or error
assertion was removed or relaxed.

The catalogue now states that the successor-bearing guest-reset cell uses a
Service because RESTART-SSOT finalizes a naturally crashed Job
(`distill/test-scenarios.md:35`). Scenario prose likewise distinguishes the
Service cell from the Job cells and preserves RESET-EXIT, the far-end reset,
`WorkloadCrashedImmediately`, and charged restart language
(`vsock_fault_matrix.rs:6-21`, `:222-227`).

### Contract precedence and API audit

The fixture switch reconciles two accepted contracts without weakening
either:

- RESET-EXIT/R5-56 pins the guest PID-1 crash, VMM exit, host-flow reset,
  post-READY `WorkloadCrashedImmediately` classification and restart-budget
  charge (`feature-delta.md:13063`). It does not require that the workload be a
  Job.
- RESTART-SSOT pins that a naturally crashed Job is terminal and never gets a
  successor. A Service below its restart ceiling is the existing restartable
  workload kind used by the accepted restart model.

The test still enters through production `serve` plus the public deploy
handler and uses the existing VM driver, guest image, command and crash
stimulus. `deploy_hold_service` is private integration-test support, not a new
public or cross-crate API. The change adds no production restart branch,
configuration field, error variant, fault seam, scenario or lane.

### Markers, neighboring cells and retained native evidence

All five CLI identities retain one exact
`/// CONTRACT_SHAPE: bounded-change.` declaration and the exact
`#[ignore = "pending DELIVER step 14-02"]` marker. The body count remains
five. The owner-death and VMM-kill calls now spell `HoldWorkload::Job`, whose
match arm calls the same existing `Lane::deploy_job` with the same id and
arguments; their behavior and assertions are unchanged. Far-reader and UDP
do not use the changed hold-workload selector.

The retained exact GuestKit ledger is:

| Identity | Receipt | Disposition |
| --- | --- | --- |
| `the_serve_owners_death_resets_every_far_end_and_the_next_boot_reclaims_the_vm` | `bdc5ea0b1cda6b9e5c2717a7` | PASS, one exact body, zero canonical-117 credit; Job/NO-SURVIVE behavior unchanged |
| `a_killed_vmm_resets_the_far_end_of_its_flow` | `d109b9e974f4571bf7b29a12` | PASS, one exact body, zero canonical-117 credit; explicit Job selector is behavior-preserving |
| `a_guest_reset_ends_the_vmm_and_resets_every_far_end` | `5a92987da14f08aef2b68d2d` | semantic RED of the old Job fixture; proves the contract mismatch and gives no GREEN credit to the corrected Service source |
| far-reader cell | none | not run |
| UDP-slot cell | none | not run |
| standalone worker leg-F cell | none after F-01 correction | not run |

The two passes remain valid evidence for their unchanged exact outcomes. The
current corrected guest-reset source has no native GREEN receipt yet. The
overall STEP 14-02 execution state is therefore not GREEN and this review does
not claim otherwise.

### Iteration 3 acceptance dimensions

| Dimension | Score | Assessment |
| --- | ---: | --- |
| Happy-path bias | 10/10 | The guest reset remains a real crash/restart failure path with exact terminal evidence. |
| GWT / one behavior | 9/10 | One existing scenario retains one healthy baseline, one PID-1 crash and its bounded outcomes. |
| Domain language | 10/10 | Job, Service, successor, guest reset, VMM exit and restart-budget charge match accepted vocabulary. |
| Coverage completeness | 10/10 | VMM exit, far-end reset, exact crash reason, Failed state and successor branch are all retained. |
| User/operator value | 9/10 | The production composition exposes both the failed allocation and its restartable successor. |
| Priority validation | 10/10 | A reproduced fixture/contract mismatch is corrected without touching production. |
| Observable behavior | 10/10 | Process exit, peer reset and operator allocation rows provide independent oracles. |
| Traceability | 10/10 | The scenario reconciles RESET-EXIT/R5-56, V-8 and RESTART-SSOT explicitly. |
| Walking-skeleton boundary | 10/10 N/A | This focused Tier-3 scenario continues through production `serve` and public deploy. |

### Iteration 3 mandate and mechanical disposition

| Gate | Result | Evidence |
| --- | --- | --- |
| CM-A — production / hexagonal boundary | PASS | Service submission uses the public deploy handler; the real production composition and VM driver remain the SUT. |
| CM-B — domain abstraction | PASS | The fixture names the accepted workload kind and leaves TOML mechanics in shared test support. |
| CM-C — complete focused journey | PASS | Healthy flow, guest crash, VMM exit, reset, exact failed allocation and successor are all observed. |
| CM-D — fixture isolation | PASS | The existing lane owns the workload and teardown; no new kernel fixture or external resource is added. |
| CM-E — universe-bound assertion | PASS / layer N/A | Layer-4+ native integration uses direct process, peer and operator assertions. |
| CM-F — layer-dependent PBT | PASS | The native fault remains one explicit example. |
| CM-G — two-tier rich journey | N/A | No new journey or Tier-B surface is introduced. |
| CM-H — explicit integration sad paths | PASS | The PID-1 crash remains an explicitly named example. |
| Exact contract | PASS | Service restart and Job finality both match their accepted contracts. |
| No contract weakening | PASS | Every RESET-EXIT/error/outcome/restart assertion is retained. |
| No invented production behavior/API | PASS | Only private test support and one fixture selector are added. |
| Other CLI bodies | PASS | Owner-death/VMM-kill remain Jobs; far-reader/UDP bodies are unchanged. |
| Marker count | PASS — 5 of 5 restored |
| Source gate | PASS — `59f5fa9edcd51cd5edc3ff050`, exit 0 |
| Scoped `git diff --check` | PASS |

## Iteration 3 native-execution authorization boundary

**DISTILL verdict: APPROVED.** F-02 is closed. All six identities remain safe
for one-at-a-time exact execution through the existing disposable-outer-guest
GuestKit selector, with source-manifest accounting and
`canonical_117_credit=0`.

Execution evidence is still incomplete:

- the corrected Service guest-reset body requires its first current-source
  exact GuestKit run;
- the far-reader and UDP CLI bodies have not run;
- the corrected standalone worker leg-F body has not run.

The retained owner-death and VMM-kill passes need no fixture remediation; they
prove their unchanged Job outcomes. No direct bare-metal/nextest run, direct
SSH test execution, physical-host cleanup, marker activation, DES GREEN or
commit is authorized by this review itself.

## Iteration 3 verdict

**APPROVED FOR NATIVE EXECUTION; NOT YET GREEN.** The guest-reset fixture now
uses the existing restartable Service semantics required by its successor
oracle while preserving the exact RESET-EXIT failure, terminal and far-end
outcomes. This neither restarts a Job contrary to RESTART-SSOT nor removes the
restart-budget evidence required by R5-56. No blocking DISTILL finding
remains; current-source native execution is the next gate.

## Iteration 4

### Scope and reproduced failure

Iteration 4 reviews only the existing leg-F scenario's EMFILE fixture in
`crates/overdrive-worker/tests/leg_f_repair.rs`. It does not review or opine
on the concurrently edited guest-capture, guest-flow dispatch or UDP production
files. No production source, public API, scenario, runner, lane, contract,
pending marker, DES event or commit is changed by this remediation or review.

Exact GuestKit run `8474db7b60c69be2a2e89875` is a valid semantic
reproduction. The existing TPROXY-captured connection completed inside the
disposable outer guest on Linux `7.0.0-38-generic`, but leg F's accept task did
not end within five seconds. The exact worker cell exited 101 with zero skips
and zero canonical-117 credit. This disproves the former fixture premise that
setting `RLIMIT_NOFILE.rlim_cur` to the number of open descriptors necessarily
makes the next descriptor allocation fail.

The retained iteration-4 source gate `9a8a0f3aa78d685d43bc22000` exited zero
for format, check and Clippy with `-D warnings`; scoped `git diff --check`
passes. The reviewed source SHA-256 is
`8d33f79545199cd50c30ae6f987618b98d4729f2252ecbe79bec299ce536e854`.
This reviewer ran no native, kernel, Metal, GuestKit, microVM, Lima or mutation
command and changed only this Markdown review.

### F-03 — open-descriptor count did not close lower fd-number holes

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`  
**Severity at discovery:** blocker  
**Disposition:** CLOSED for source review; one exact GuestKit validation is
required before GREEN.

Linux applies `RLIMIT_NOFILE` to descriptor numbers, not to the cardinality of
open files. In the vendored Linux 7.0 source:

- `alloc_fd` starts from `files->next_fd`, finds the next zero bit in the open
  descriptor bitmap and returns `EMFILE` only when that candidate is greater
  than or equal to the supplied end (`vendors/linux/fs/file.c:570-607`);
- `get_unused_fd_flags` passes `rlimit(RLIMIT_NOFILE)` as that exclusive end
  (`file.c:615-623`);
- `accept4`'s `FD_ADD` reserves this descriptor before evaluating
  `do_accept`, so EMFILE occurs before the pending connection is dequeued
  (`vendors/linux/include/linux/file.h:195-203`,
  `vendors/linux/net/socket.c:2011-2023`).

The former `rlim_cur = open_descriptors()` left any unused descriptor number
below that count allocatable. Receipt `8474db7b60c69be2a2e89875` proves this was
reachable in the real standalone test process: the captured handshake
succeeded and the accept task stayed live instead of returning the expected
typed failure.

### Dense descriptor-table fixture

`reserve_dense_fd_table` now establishes the exact precondition the kernel
allocator consumes (`leg_f_repair.rs:200-224`):

1. `open_descriptors` counts the process's open descriptors and subtracts the
   `/proc/self/fd` observer descriptor. That observer is closed before the
   reservation loop, so its newly freed number is an ordinary hole for the
   first reservation to fill (`:168-173`).
2. Each reservation opens `/dev/null` with `O_CLOEXEC`. Linux assigns the
   lowest available descriptor. The helper retains every returned `OwnedFd`
   and asserts that the returned number is no greater than the current open
   count, the pigeonhole bound for the lowest free number (`:205-219`). An
   unexpected allocator advance therefore fails closed instead of silently
   accepting a sparse table.
3. After incrementing the count, the helper stops only when
   `raw + 1 == open_count`. At that point the just-allocated `raw` was the
   previous count and, because it was the lowest free descriptor, every number
   in `[0, raw)` was already occupied. Retaining `raw` makes the complete range
   `[0, open_count)` dense. The returned `soft_limit = open_count` therefore
   makes the next candidate equal to the exclusive limit, which Linux rejects
   with `EMFILE`.

The loop fills only holes up to the process's current descriptor frontier; it
does not deliberately reserve every number up to the original rlimit. Its
absolute bound is the process's existing soft limit, and this dedicated
one-test binary has a small, controlled descriptor set inside a disposable
guest.

### Observer and lifecycle race audit

The remediation removes the concrete sources of descriptor-table churn before
measurement:

- the initial direct positive control now waits for leg F to accept and close
  the connection, rather than returning after the TCP handshake while a
  transient accepted descriptor may still exist (`leg_f_repair.rs:399-427`,
  `:477-483`);
- all `ip` subprocesses, CIDR registry operations, production netlink setup
  and the short-lived `setns` thread finish before the dense-table count;
- the captured client socket is created in its workload network namespace and
  returned as an owned descriptor before the count, reservations and limit
  change (`:356-374`, `:485-499`);
- after the limit is lowered, the only initiating operation is `connect(2)` on
  that already-open socket. The Tokio timeout uses the runtime's existing
  driver; the expected leg-F `accept(2)` is the only operation in this bounded
  path that needs a new process descriptor.

A hole present when a nonterminal reservation iteration runs is selected by
the next lowest-free `/dev/null` open. A concurrent allocation that would
advance the lowest free number beyond the maintained count trips the explicit
assertion. More critically, the audited source path has no remaining owner of
a lower descriptor scheduled to close during or after densification and before
the accept attempt: the direct control and every setup process/thread have
already completed. The former setup-time cleanup calls against fresh
PID-derived names are removed; they created only expected `ip` failures and no
resource. The existing `CapturedPath::Drop` teardown still owns the real veth,
namespace, loopback `/32` and CIDR leases.

### Limit restoration and bounded failure behavior

The stimulus sequence is ordered safely (`leg_f_repair.rs:494-511`):

1. create and retain the captured client;
2. save the original soft and hard limits;
3. create and retain the dense reservations;
4. lower only the soft limit to the dense frontier;
5. connect the existing client through the unchanged production TPROXY path;
6. await the typed owner failure with the existing five-second bound;
7. restore the original limit and then drop every reservation;
8. only after restoration, unwrap the captured connect and failure results and
   continue to `ss`, repair and the remaining assertions.

Both a successful failure observation and a timeout return pass through the
explicit restoration and reservation drop before an assertion can panic. A
failure of `setrlimit` itself fails immediately. Any abnormal process-level
termination still ends this standalone test process and its disposable outer
guest, so a lowered process limit cannot contaminate another test.

### Accepted fault path and unchanged oracles

With `[0, soft_limit)` occupied, leg F's `accept4` reaches `FD_ADD`,
`get_unused_fd_flags` finds the next candidate at `soft_limit`, and returns
`EMFILE` before `do_accept`. The TPROXY-captured connection therefore remains
queued and the original listener remains `LISTEN`, exactly the accepted
D-9/V-8 mechanism. `wait_shared_owner_failure` must then return
`TaskFailed { leg: F }`; the component projection must be `LegF`.

No acceptance contract is weakened. The existing source still requires:

- the workload-netns connection to traverse the production outbound TPROXY
  rule to leg F;
- typed `TaskFailed { leg: F }` within five seconds;
- the original listener to remain `LISTEN` with the same inode;
- repair `Ok` without `EADDRINUSE` at the same address;
- a new listener inode and the old descriptor absent;
- a second TPROXY-captured connection to be accepted and closed within two
  seconds; and
- the post-repair shared-owner audit to pass
  (`leg_f_repair.rs:485-546`).

The scenario identity and exact
`/// CONTRACT_SHAPE: bounded-change.` declaration remain unchanged, and
`#[ignore = "pending DELIVER step 14-02"]` is restored
(`leg_f_repair.rs:431-446`). No new scenario, public method, production fault
hook, error variant, lane or architecture is introduced.

### Iteration 4 acceptance dimensions and mechanical disposition

| Gate | Result | Evidence |
| --- | --- | --- |
| Linux RLIMIT model | PASS | Dense fd-number range matches `alloc_fd`'s candidate-versus-exclusive-end rule. |
| Necessity | PASS | Exact GuestKit receipt `8474db7b60c69be2a2e89875` reproduces the sparse-table failure. |
| Client allocation order | PASS | Captured socket and all reservations exist before lowering the limit. |
| Descriptor observer | PASS | `/proc/self/fd` observer is subtracted, closed and its hole filled by the loop. |
| Transient control accept | PASS | Positive control waits for accepted descriptor closure before measurement. |
| Race posture | PASS | Setup operations finish before measurement; unexpected allocator advance fails closed. |
| Reservation bound | PASS | Stops at the first dense frontier; bounded by the existing process rlimit. |
| Limit restoration | PASS | Original limit restored and reservations dropped before result assertions. |
| Fault boundary | PASS | Existing production TPROXY connection drives the real leg-F accept task. |
| Typed failure | PASS | Exact `TaskFailed { leg: F }` and component `LegF` retained. |
| Listener/repair oracles | PASS | Still-LISTEN/same inode, no EADDRINUSE, new inode, old fd closed retained. |
| Post-repair capture/audit | PASS | Bounded second capture and full owner audit retained. |
| Fixture cleanup | PASS | Fresh-name preclean removed; real-resource RAII teardown remains. |
| Contract/API scope | PASS | Existing test-local HOW only; no new scenario or production surface. |
| Source gate | PASS — `9a8a0f3aa78d685d43bc22000`, exit 0 |
| Source identity | PASS — `8d33f79545199cd50c30ae6f987618b98d4729f2252ecbe79bec299ce536e854` |
| Scoped `git diff --check` | PASS |

## Iteration 4 native-execution authorization boundary

**DISTILL verdict: APPROVED FOR ONE EXACT GUESTKIT VALIDATION.** F-03 is closed
for source review. The current `leg_f_repair_rebinds_transparent_listener`
fixture is ready to run once through the existing
`OVERDRIVE_FWD_GUEST_GROUP=cgroup-default` plus exact selector inside a fresh
disposable outer GuestKit guest. The result must retain the one-body worker
source manifest, zero canonical-117 credit, zero runtime skips, the full guest
console and cleanup evidence.

The prior receipt `8474db7b60c69be2a2e89875` remains the semantic RED that
proved the old fixture invalid; it gives no GREEN credit to this source. This
review does not authorize direct host nextest, direct SSH execution, physical
host cleanup, DES GREEN, marker activation or commit, and it does not review
the separate UDP production remediation.

## Iteration 4 verdict

**APPROVED FOR NATIVE VALIDATION; NOT YET GREEN.** The dense-fd reservation
implements the Linux 7.0 descriptor-number contract that the EMFILE stimulus
actually needs, removes the reproduced sparse-table false premise, restores
the process limit before every normal assertion path, and preserves every
accepted TPROXY, typed-failure, listener, repair and post-repair oracle. No
blocking DISTILL finding remains for this exact worker cell; one current-source
GuestKit run is the next gate.

## Iteration 5

### Scope and native evidence

Iteration 5 reviews only the existing leg-F fixture's cleanup after exact
GuestKit receipt `801ca67124bed540046b82fa`. It does not review the separate
guest-capture, guest-flow dispatch or UDP production changes. This reviewer
ran no native, kernel, Metal, GuestKit, microVM, Lima or mutation command and
changed only this Markdown artifact.

Receipt `801ca67124bed540046b82fa` closes F-03's native-validation question:

- the workload-side TPROXY-captured connection completed;
- the dense fd table produced `TaskFailed { leg: F, source: code 24 / Too many
  open files }` within 114 ms, with soft NOFILE 15;
- `ss -ltn` showed the original leg-F listener still `LISTEN` with one queued
  connection;
- the subsequent repair correctly failed closed while observing the strict
  shared nft inventory: `shared IP rule inventory is partial or foreign`.

The run used the disposable outer GuestKit guest, Linux `7.0.0-38-generic`,
the exact one-body worker selector, zero skips and zero canonical-117 credit.
Its exit 101 is a semantic fixture failure, not runner infrastructure.

The retained iteration-5 source gate `1e29af77ed0bfd8a5809e1300` exited zero
for format, check and Clippy with `-D warnings` under the reported
incremental-disabled configuration. Scoped `git diff --check` passes. The
reviewed source SHA-256 is
`4cd488120753affa9ee74d5a0c38e869b600e705914abab11bf06a4cbcc2e05b`,
and the pending STEP 14-02 marker is restored.

### F-03 disposition

**CLOSED.** The native receipt proves the dense descriptor-table correction
causes the exact accepted EMFILE path. No further change to the RLIMIT fixture,
TPROXY stimulus, typed failure or still-LISTEN oracle is required.

### F-04 — the proposed sweep removes one egress rule but leaves two legacy exemptions

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`  
**Severity:** blocker  
**Disposition:** OPEN; the current worker cell is not ready for another native
run.

The native repair refusal is correct production behavior. The node-shared
owner's current program is an exact inventory of three prerouting rules and
nine output rules (`overdrive-netlink/src/nft.rs:2902-2908`, `:2956-2998`).
Its rules use `ovd295-ip-<chain>-<index>` userdata, including its current
leg-S exemptions (`nft.rs:783-820`). The strict observer rejects any additional
rule rather than ignoring it.

The fixture reaches leg F through the legacy
`install_outbound_tproxy(host_veth, port)` helper. There is no current
production caller of that helper; its callers are integration fixtures. When
invoked after the current shared owner is published, the helper changes more
than one rule:

1. `ensure_shared_routing_infra` calls `ensure_exemption` for both
   `prerouting` and `output` (`mtls_intercept.rs:843-925`).
2. `ensure_exemption` recognizes only the legacy `ovdmtls` exemption userdata
   through `nft::has_exemption`; it does not recognize the current shared
   program's differently tagged exemption (`mtls_intercept.rs:929-956`;
   `nft.rs:400-405`, `:438-445`). It therefore inserts one legacy exemption in
   each chain.
3. `install_outbound_tproxy` then appends the one legacy per-veth egress rule.
   The fixture has introduced **three** extra rules: one egress rule and two
   legacy exemptions.

The proposed `sweep_fixture_capture` calls
`sweep_per_workload_tproxy_rules` and asserts a count of one
(`leg_f_repair.rs:404-408`, `:532-536`, `:560-561`). That assertion is true but
insufficient. The sweep deliberately classifies only userdata kinds inbound,
egress and output-divert; it explicitly excludes the legacy exemption kind and
foreign rules (`overdrive-netlink/src/nft.rs:420-435`). It therefore deletes
the one egress rule and leaves both fixture-added exemptions in the strict
shared inventory.

Dropping the guard afterward cannot close the gap. Its destructor performs a
best-effort delete of the already-swept egress handle and discards the result
(`mtls_intercept.rs:1037-1095`). It neither owns nor removes the two exemption
handles. Consequently, `converge_shared_owner` must still reject the inventory
before repair, and the post-repair audit would do the same after the second
capture. A repeat native run of this source would only reproduce the known
failure.

### Why this is not production remediation

The extra egress and exemption rules are created only by the fixture's legacy
capture helper in this current composition. Receipt `801ca67124bed540046b82fa`
proved the production owner did the right thing by rejecting the resulting
inventory. The repair and audit must stay strict. Adding an exception for
legacy tags, reducing the expected 3/9 inventory, ignoring foreign rules, or
changing production convergence would weaken the accepted fail-closed
boundary and is rejected.

The proposed sweep also does not hide arbitrary foreign state: the structural
classifier leaves foreign userdata untouched, so the strict observer still
rejects it. The problem is narrower—the fixture fails to remove two additional
rules that it deterministically creates itself.

### Required bounded correction

Keep the existing scenario and every accepted oracle. Correct only its
test-local capture lifecycle so **all** state added by the legacy helper is
removed before repair and before the final audit:

- retain the fail-closed per-workload sweep and exact one-egress-rule count;
- account for the two exact legacy exemption rules inserted by this fixture,
  one in `prerouting` and one in `output`, and remove them through fallible
  by-handle operations with exact cardinality and postcondition checks; or
- avoid creating those exemptions by installing the test capture rule against
  the already-converged shared chains using existing test-support/netlink
  primitives.

Either test-support implementation must prove the post-cleanup program is
exactly the owner's current inventory. It must fail on zero, duplicate or
unexpected fixture-owned rules, propagate list/delete failures, and leave
unknown foreign rules present so the strict owner audit—not the fixture—rejects
them. Guard `Drop` alone is not evidence because its delete is intentionally
best-effort. No new production API, production branch, audit exception,
scenario or lane is authorized.

The EMFILE, captured-connection, original `LISTEN`/inode, release-before-bind,
same-address, new-inode, old-fd-closed, post-repair captured acceptance and
strict final audit assertions remain mandatory and unchanged.

### Iteration 5 mechanical disposition

| Gate | Result | Evidence |
| --- | --- | --- |
| Actual EMFILE | PASS — native code 24, typed leg-F failure |
| Original listener state | PASS — native `LISTEN` with queued connection |
| Dense-fd necessity | PASS — F-03 validated by `801ca67124bed540046b82fa` |
| Strict shared inventory | PASS — production correctly rejects partial/foreign inventory |
| Fixture egress cleanup | PASS in isolation — exact one per-workload rule swept fail-closed |
| Fixture exemption cleanup | **FAIL — two deterministic legacy exemptions remain** |
| Foreign-state posture | PASS — foreign rules remain visible to strict audit |
| Guard deletion | INSUFFICIENT — best-effort and owns only the egress handle |
| Repair readiness | **FAIL — strict observe still sees extra rules** |
| Final audit readiness | **FAIL — second capture recreates the same extras** |
| Contract weakening | PASS — none in source; proposed cleanup must preserve strictness |
| Scenario/API scope | PASS — existing test-local fixture only |
| Source gate | PASS — `1e29af77ed0bfd8a5809e1300`, exit 0 |
| Source identity | PASS — `4cd488120753affa9ee74d5a0c38e869b600e705914abab11bf06a4cbcc2e05b` |
| Marker | PASS — restored |
| Scoped `git diff --check` | PASS |

## Iteration 5 native-execution authorization boundary

**DISTILL verdict: REJECTED PENDING REVISION for the standalone worker
identity.** Do not spend another GuestKit run on this source: the audit proves
it necessarily leaves two fixture-added legacy exemptions and will fail before
repair. The five CLI identities remain independently safe for their exact
GuestKit runs.

After the fixture accounts fail-closed for all three of its rule additions,
return this same existing scenario for iteration 6. No direct host nextest,
direct SSH execution, physical-host cleanup, DES GREEN, marker activation or
commit is authorized by this review, and the separate UDP production attempt
remains outside this DISTILL scope.

## Iteration 5 verdict

**BLOCKED ON ONE PRECISE FIXTURE GAP.** F-03 is natively closed: the accepted
TPROXY/EMFILE/queued-LISTEN path works. F-04 remains open because the current
cleanup removes only the egress rule while the legacy helper also inserts one
exemption in each shared chain. Production's strict refusal is correct and
must not be weakened. Remove every fixture-added rule fail-closed, then
re-review before one exact GuestKit validation.

## Iteration 6

### Scope and source gate

Iteration 6 reviews only the simplified private capture-rule lifecycle in the
existing `leg_f_repair_rebinds_transparent_listener` scenario. The separate
guest-capture, guest-flow dispatch and UDP production work remains outside this
DISTILL review. This reviewer ran no native, kernel, Metal, GuestKit, microVM,
Lima or mutation command and changed only this review artifact.

The retained source gate `861a4e972b13b1c2d2abf1420` exited zero for format,
check and Clippy with `-D warnings`. Scoped `git diff --check` passes. The
reviewed scenario SHA-256 is
`81d18f63b980f3c7cdbb48528c956b14006ccd6c883b665dac341078d1ffe8b3`,
and its STEP 14-02 pending marker remains restored.

### F-04 remediation

**Disposition: CLOSED.**

The author reopened the fixture mechanism instead of extending the legacy
cleanup machinery. The scenario no longer calls `install_outbound_tproxy` or
`sweep_per_workload_tproxy_rules`. It therefore creates neither legacy
exemption, no table or chain, and no policy route. `worker.start_shared_owner`
has already converged the current table, exact 3/9 rules and fwmark/local-route
objects before the fixture installs its capture (`leg_f_repair.rs:579-613`).

`FixtureCaptureRule::install` adds only the one rule the packet path requires
(`leg_f_repair.rs:406-464`):

1. It reads the already-converged `overdrive-mtls` prerouting chain and refuses
   any pre-existing legacy per-workload rule or legacy exemption. The current
   shared rules are not misclassified because their userdata uses the current
   `ovd295-ip-*` identity, while the legacy classifiers use `ovdmtls` kinds.
2. It derives a unique egress userdata tag from the fixture host veth and live
   leg-F port.
3. It builds the rule with the existing production
   `nft::egress_tproxy_rule_exprs`, `MTLS_TPROXY_MARK`, loopback target and live
   port. That encoder matches ingress on the workload's host-side veth,
   requires TCP, carries the production counter, sets the shared fwmark,
   TPROXY-redirects to leg F and accepts (`overdrive-netlink/src/nft.rs:699-736`).
4. It appends the rule to the existing prerouting chain, reads it back by its
   exact tag and requires exactly one match, a counter, the expected normalized
   program and a stable kernel handle. The complete legacy workload-rule
   classifier must contain exactly that handle, and no legacy exemption may
   appear.

This is the minimum necessary test support for the accepted captured-connection
criterion. It reuses the production rule encoder and shared constants while
avoiding the obsolete helper's unrelated infrastructure convergence. It adds
no generalized cleanup or production mechanism.

### Fail-closed removal and foreign-state posture

Normal removal consumes the private ownership record and is fully checked
(`leg_f_repair.rs:466-508`):

- immediately before deletion, the tag must have exactly one rule;
- its kernel handle and normalized program must still equal the installed
  values;
- the chain's complete legacy workload-rule set must be exactly that one
  handle, with no legacy exemption;
- deletion by exact handle is fallible and any error fails the scenario;
- read-back must show the exact tag absent, zero legacy workload rules and zero
  legacy exemptions.

The scenario performs this same removal before repair and before the final
strict audit (`leg_f_repair.rs:640-664`). It therefore restores the exact
current shared program at both boundaries.

Unknown foreign rules are intentionally absent from the fixture classifier
and are never deleted. The pre/post checks constrain only the fixture's exact
tag and known legacy kinds. Any foreign rule remains in the chain, where
`converge_shared_owner` or `audit_shared_owner` rejects it through the strict
3/9 inventory. The remediation closes F-04 without weakening fail-closed
observation.

`Drop` is an abnormal-unwind fallback only (`leg_f_repair.rs:511-516`). It
attempts deletion when normal checked removal did not complete, but its
best-effort result is not used as acceptance evidence. Every normal repair and
audit path uses the fallible, exact, read-back-verified `remove` method.

### Real packet path and retained acceptance oracles

The workload-side socket still originates in the test-owned namespace and
enters through the named host veth. The dialled leased address has no listener
and differs from loopback leg F. Because the fixture appends the exact egress
TPROXY expression to the live shared prerouting chain, and the shared owner has
already converged the fwmark policy route, a successful handshake still proves
the real kernel TPROXY packet path to leg F. The prior native receipts already
proved this encoder and topology deliver the captured connection.

All accepted scenario outcomes remain unchanged:

- the captured client and dense reservations exist before the soft limit is
  lowered;
- leg F must return typed code-24 `TaskFailed` within five seconds;
- the original listener remains `LISTEN` with the same inode;
- the exact capture rule is removed before repair;
- repair releases the old listener before re-binding the same address, returns
  `Ok` without `EADDRINUSE`, creates a new inode and closes the old descriptor;
- the same exact one-rule fixture is reinstalled for the second captured
  connection, which must be accepted and closed within two seconds;
- the rule is removed with its full postcondition before the final strict audit;
  and
- the shared-owner audit must pass (`leg_f_repair.rs:607-666`).

The scenario identity, single
`/// CONTRACT_SHAPE: bounded-change.` declaration and pending marker are
unchanged. No public API, production branch, rule kind, error, fault hook,
scenario or lane is added.

### Iteration 6 mechanical disposition

| Gate | Result | Evidence |
| --- | --- |
| Legacy setup helper | PASS — no call; no table/chain/route/exemption convergence |
| Capture rule count | PASS — exactly one tagged egress rule |
| Packet program | PASS — existing production encoder plus shared table/mark constants |
| Read-back identity | PASS — exact tag, normalized program, counter and handle |
| Pre-existing owned state | PASS — zero legacy workload rules/exemptions required |
| Normal deletion | PASS — fallible exact-handle delete |
| Deletion postcondition | PASS — tag absent; zero legacy workload rules/exemptions |
| Unknown foreign state | PASS — untouched and left for strict owner rejection |
| Abnormal unwind | PASS — best-effort Drop only; not credited on normal paths |
| Repair boundary | PASS — exact shared inventory restored before convergence |
| Final audit boundary | PASS — exact shared inventory restored after second capture |
| EMFILE/TPROXY/LISTEN oracles | PASS — unchanged |
| Rebind/inode/fd oracles | PASS — unchanged |
| Post-repair captured accept | PASS — unchanged and bounded |
| Production/API scope | PASS — private test support only |
| Source gate | PASS — `861a4e972b13b1c2d2abf1420`, exit 0 |
| Source identity | PASS — `81d18f63b980f3c7cdbb48528c956b14006ccd6c883b665dac341078d1ffe8b3` |
| Marker | PASS — restored |
| Scoped `git diff --check` | PASS |

## Iteration 6 native-execution authorization boundary

**DISTILL verdict: APPROVED FOR ONE EXACT GUESTKIT VALIDATION.** F-04 is
closed. The current standalone worker identity is ready to run once through
the existing `cgroup-default` exact selector inside a fresh disposable outer
GuestKit guest. The receipt must retain the one-body worker source manifest,
zero canonical-117 credit, zero runtime skips, the full outer-guest console
and fixture cleanup evidence.

Receipt `801ca67124bed540046b82fa` remains valuable semantic evidence that
EMFILE and strict-inventory rejection work, but it gives no GREEN credit to
this simplified source. This review does not authorize direct host nextest,
direct SSH execution, physical-host cleanup, DES GREEN, marker activation or
commit, and it does not review the separate UDP production attempt.

## Iteration 6 verdict

**APPROVED FOR NATIVE VALIDATION; NOT YET GREEN.** The simplified fixture adds
one exact TPROXY capture rule to already-converged production infrastructure,
owns and validates that rule precisely, removes it fail-closed before repair
and audit, and leaves unknown foreign state visible to the strict owner. No
blocking DISTILL finding remains for this worker cell; one current-source
exact GuestKit run is the next gate.

## Iteration 7 — final bounded worker evidence review

### Evidence set and source identity

Iteration 7 reviews retained evidence only. No native, kernel, Metal,
GuestKit, microVM, Lima or mutation command was launched by this reviewer, and
no source, DES event or commit was changed.

The accepted source remains the iteration-6 scenario with SHA-256
`81d18f63b980f3c7cdbb48528c956b14006ccd6c883b665dac341078d1ffe8b3`.
The exact evidence chain is:

| Evidence | Identity | Result |
| --- | --- | --- |
| GuestKit preparation | `65aa91b9f6fa0dbdbb5bff25` | exit 0, source digest `292250d745c7060158ab539563260d323959448feca06b738c6e88f0520c963e` |
| Staged worker ELF | `/home/ubuntu/overdrive/target/guest-kit/in-guest/overdrive-worker--leg_f_repair` | SHA-256 `2e85269e24e51945a5df29fdf2d94027e00c8da5e6217aa18252372de9f60ac3` in the preparation artifact manifest |
| Exact worker run | `b74d312a08ef94d10c47b2c2` | terminal exit 0, `command_returned` |
| Exact runner result | `step14-02-worker` / `leg_f_repair_rebinds_transparent_listener` | 1 passed, 0 failed, 0 ignored, 0 runtime SKIP, source-manifest bodies 1, canonical-117 credit 0 |

The preparation metadata and its remote artifact manifest are retained under
`.context/netns-density-295/14-02/raw/65aa91b9f6fa0dbdbb5bff25/`. It explicitly
staged the standalone worker target for Linux `7.0.0-38-generic`. No later
`guest-kit-prepare` run occurred before exact worker run
`b74d312a08ef94d10c47b2c2`; the intervening UDP exact run consumed staged
artifacts but did not rebuild or replace them. The worker run therefore used
the manifest-pinned ELF.

The exact-run workspace digest is separately recorded as
`019b2f234b52ab0d06707b79ee04304bec6e1f0957eb9a08626d58a0ee7e9ab8`.
That command compiles the GuestKit runner and executes the already-prepared
worker ELF; the worker artifact identity comes from the preparation manifest.
The run's boundary record independently pins the reviewed iteration-6 fixture
SHA `81d18f63b980f3c7cdbb48528c956b14006ccd6c883b665dac341078d1ffe8b3`.
The current local scenario still has that exact SHA.

The retained raw receipt is internally consistent: local and remote manifests
agree on metadata, phase, terminal and output hashes; `output.log` is 30,742
bytes with SHA-256
`ec18d2e7685dce83f35d9f4915cd2be3ffe75814219d1322ee513979e19d9124`;
the terminal record is exit 0.

### Disposable-guest boundary

The exact command was the approved focused route:

```text
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=cgroup-default OVERDRIVE_FWD_GUEST_EXACT='leg_f_repair_rebinds_transparent_listener' cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests
```

The retained console begins with `disposable guest
step14-02-focused-leg-f-repair`, boots Linux `7.0.0-38-generic` under Cloud
Hypervisor/KVM, shows Landlock in the guest LSM set, loads the fixture modules
inside that guest, and runs exactly one libtest body there. The boundary record
classifies it as `semantic-green-verified-current-guestkit-worker` and states
that no direct host test or host cleanup was performed. All nft, policy-route,
netns/veth, RLIMIT and socket effects were therefore confined to the
disposable outer guest kernel.

### Native oracle disposition

The full receipt plus the exact passing source establish every required
observable:

| Oracle | Native evidence and executable assertion |
| --- | --- |
| TPROXY-captured stimulus | The dialled leased destination has no listener; the exact encoded egress rule is the only successful path to leg F. The body passed both rule identity/read-back checks and the captured connect. |
| Actual EMFILE | Console: `TaskFailed { leg: F, source: Os { code: 24, ... "Too many open files" } }`, soft NOFILE 15, at +87 ms. |
| Original listener remains queued and `LISTEN` | Console `ss-ltn`: `LISTEN 1 16 127.0.0.1:37905`; the body also requires the original inode to remain exact. |
| Exact capture cleanup before repair | The body cannot reach repair unless exact-tag/handle/program checks pass, the fallible delete succeeds, and read-back shows zero fixture-owned workload rules and legacy exemptions. |
| Release-before-rebind and no `EADDRINUSE` | Console: repair `Ok(())` at +127 ms. The body invokes the production repair after the original task ended and requires the same bound address. |
| New listener / old descriptor closed | The passing body requires a new listening inode and `descriptor_of(old_inode) == None`; failure of either assertion would make the one exact body fail. |
| Post-repair captured acceptance | The second exact capture rule and workload-netns client must connect; EOF/reset/abort must arrive within two seconds, distinguishing accepted-and-closed from a queued-only handshake. |
| Exact cleanup before strict final audit | The second rule undergoes the same fallible handle deletion and zero-owned-rule read-back before audit. Unknown foreign rules are untouched. |
| Strict owner audit | `audit_shared_owner().await.expect(...)` is the final behavioral assertion before shutdown. The exact body passed, proving both tasks live and the strict shared inventory accepted the restored 3/9 program. |
| Fixture teardown | Normal scope ran `CapturedPath::Drop`; no `cleanup: ip ...` failure appears in the full console. The test then shut down the shared owner and returned `ok`; the outer guest completed with terminal exit 0. |

The libtest summary is exact:

```text
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
STEP14-02-FOCUSED-RESULT source=step14-02-worker identity=leg_f_repair_rebinds_transparent_listener passed=1 source_manifest_bodies=1 canonical_117_credit=0
```

No skip diagnostic is present, and the runner's required-cell path would have
rejected one. The evidence is non-vacuous.

### Finding dispositions

| Finding | Final disposition |
| --- | --- |
| F-01 — direct connections bypassed required TPROXY capture | CLOSED in iteration 2; final native receipt proves both captured paths execute. |
| F-02 — successor-bearing Job contradicted RESTART-SSOT | CLOSED in iteration 3; outside this worker receipt. |
| F-03 — raw fd count left lower descriptor holes | CLOSED natively; code-24 EMFILE observed in `801ca67124bed540046b82fa` and again in the final GREEN receipt. |
| F-04 — legacy helper polluted strict shared inventory | CLOSED in iteration 6 and natively verified here; repair and final strict audit both pass after exact one-rule cleanup. |

No bounded worker-support finding remains open.

### Final bounded mechanical disposition

| Gate | Result |
| --- | --- |
| Reviewed scenario SHA | PASS — `81d18f63b980f3c7cdbb48528c956b14006ccd6c883b665dac341078d1ffe8b3` |
| Preparation source digest | PASS — `292250d745c7060158ab539563260d323959448feca06b738c6e88f0520c963e` |
| Staged worker ELF manifest | PASS — `2e85269e24e51945a5df29fdf2d94027e00c8da5e6217aa18252372de9f60ac3` |
| Exact run terminal | PASS — exit 0, `command_returned` |
| Guest boundary | PASS — disposable Cloud Hypervisor outer guest, Linux 7.0.0-38 |
| Exact selection | PASS — one worker identity, one source-manifest body |
| Libtest accounting | PASS — 1 passed / 0 failed / 0 ignored |
| Runtime skips | PASS — zero |
| Canonical accounting | PASS — zero credit to canonical 117 |
| EMFILE / LISTEN | PASS — native code 24 and queued listener |
| Repair / rebind | PASS — `Ok`, same address, new inode, old fd absent |
| Post-repair capture | PASS — accepted-and-closed bounded oracle |
| Strict final audit | PASS |
| Fixture and outer-guest cleanup | PASS |
| Raw local/remote identity | PASS — matching retained receipt hashes |

## Iteration 7 final bounded verdict

**APPROVED — WORKER SUPPORT EVIDENCE COMPLETE.** Exact GuestKit run
`b74d312a08ef94d10c47b2c2` is current, non-vacuous GREEN evidence for the
standalone `leg_f_repair_rebinds_transparent_listener` source reviewed in
iteration 6. It proves the accepted TPROXY/EMFILE/still-LISTEN repair contract,
exact listener replacement, post-repair captured acceptance, strict inventory
audit and cleanup inside the disposable outer guest.

This verdict is deliberately bounded. It does **not** approve the separate UDP
production remediation and does **not** declare all of STEP 14-02 GREEN. UDP
remains semantic RED under `93f96bd412c3a4e454c67a73`, owned by the original
crafter's investigation. No direct-host receipt receives acceptance credit.

## Iteration 8 — bounded UDP released-burst body review

### Scope

This iteration reviews only the existing source-local body
`a_released_burst_is_discarded_and_the_next_owner_gets_none_of_it` in
`crates/overdrive-init/src/guest_flow/tests/udp_slots.rs`. Worker findings
F-01 through F-04 remain closed and the final worker evidence verdict above is
unchanged. The separate production edits in `kernel_io.rs`, `owner.rs`,
`mod.rs`, `main.rs` and guest capture are not reviewed here.

The body is an existing R5-14/P-72 scenario, not a new test identity. It still
uses the production `GuestFlowOwner` over the existing `SimGuestOwnerIo` seam,
keeps its exact `/// CONTRACT_SHAPE: bounded-change.` declaration, and adds no
trait method, event variant, public API, model action or test seam. Its source
SHA-256 is
`5f732598a7b3738bae14e4a48f8bd44c120d2f23704c60289dc67639e5c41c8f`.

The retained checks reported by the author are green:

- check `b1c9fbc9af65d67767720df10`;
- Clippy `e27a275683c6be3fb31c52330` with `-D warnings`;
- scoped `git diff --check`.

Exact Metal `--no-sudo` run `8b5a7cc1b550c9a0cc573a3f` is an honest RED at
the real owner/Sim-I/O boundary: one test ran, zero passed, one failed, zero
ignored; the first cleanup oracle observed `discarded = 0` instead of 200.
This reviewer ran no command and changed only this review artifact.

### What the body proves correctly

The setup is bounded and concrete (`udp_slots.rs:242-325`):

- one application cookie claims one slot for destination 1 and queues exactly
  200 datagrams;
- the first `SlotReady` remains held outside the owner, so the production
  owner's slot row has no owner, association or pair;
- the application socket closes and its real `SocketReleased { cookie }`
  event is dispatched before that held readiness;
- the intended cleanup order is exact: discard the queue without reading,
  disconnect, then release;
- the old cookie gets no request, association channel or forwarder pair; and
- the same slot is recycled for a different cookie and destination, produces
  exactly one new request, and forwards one new datagram with none of the old
  200.

Those final no-leak and recycle assertions are valuable and should remain.
The other release-order, late-`Paired`, refusal and association-end bodies are
unchanged.

### Boundary and evidence limits

`Guest::new` constructs the real `GuestFlowOwner` and delegates its driving
port to `SimGuestOwnerIo` (`tests/mod.rs:130-181`). `Guest::settle` repeatedly
calls the production `dispatch_next` while released events are ready. The body
therefore exercises real owner decisions and ordering rather than a copied
test implementation.

The Sim seam is the named arbiter for this layer, not the kernel adapter. The
test does **not** execute `KernelGuestOwnerIo`, its BPF maps, socket diagnostics
or real UDP slot sockets. In particular, the kernel adapter already has a
late orphan fallback in `slot_owner`: when readiness is dispatched after the
application socket disappeared, it discards/disconnects the slot and releases
the kernel claim (`kernel_io.rs:1020-1053`). The current body must not count
that fallback as proof that `SocketReleased` reclaimed an unpublished claim.

Likewise, retained native run `7375ebb87a3432b92f4d5ea3` reported active
sends and unmatched/fallback counters through non-atomic diagnostics. Those
samples do not causally prove that every native `EPERM` came from this release
ordering, and this review assigns them no such meaning. The current Quint
projection does not model the adapter-private unpublished-claim interval; it
cannot be cited as proof of this correction and no model change is authorized
by this review.

The handed-off read-only DESIGN checker found no contract or API gap. The
existing `SocketReleased { cookie }` event remains exact. The private kernel
adapter already owns the slot metadata/map and may perform exact-cookie cleanup
before publishing the event; pending and paired slots continue through the
normal owner close/abort/end paths. This is private implementation HOW.

### F-05 — cleanup is asserted only after releasing held readiness

**Kind:** `AT_GAP_IN_DELIVERY_SCOPE`  
**Severity:** blocker  
**Disposition:** OPEN.

After dispatching `SocketReleased`, the amended body currently asserts that
the old readiness is still present (`udp_slots.rs:271-278`). It then releases
that `SlotReady`, settles the owner, and only afterward requires discarded
200, disconnect, release, zero ownership/association/request/channel/pair
(`:280-310`).

That order does not isolate the required release-before-owner-registration
behavior. A candidate implementation can leave `SocketReleased` unmatched,
then reclaim only when the later `SlotReady` calls `slot_owner`. That is
exactly the class of late kernel-adapter orphan fallback already present at
`kernel_io.rs:1044-1053`. Mirroring that fallback in the Sim seam would make
this body GREEN even though the release report itself still failed to reclaim
the unpublished slot claim.

R5-14 pins a stronger boundary: when the application socket is released,
every slot found through the slot-ownership table keyed by that cookie is
released in discard → disconnect → pool-release order
(`feature-delta.md:3104-3119`, `:13037`). Readiness is not the trigger for that
release; it is the event whose later delivery must become irrelevant because
the claim is already gone.

### Required bounded correction

Keep this one existing scenario and its final recycle half. Move the cleanup
oracle to the `SocketReleased` boundary:

1. Clear the effect log before releasing the socket report.
2. Dispatch `SocketReleased { cookie(1) }` and settle while the first
   `SlotReady` is still held.
3. **Before releasing any readiness**, require:
   - `discarded == 200`;
   - no kernel slot owner/flow/attachment and `connected == false`;
   - exact effects `discard+disconnect`, then `release`;
   - the parked queue and held readiness are gone;
   - the release report is consumed;
   - the production owner still has no owner, association or pair;
   - no request, flow-stream connect or forwarder pair exists.
4. Do not call `release_slot_ready(s)` after those assertions: successful
   release cleanup must have removed the held readiness. A later application
   send by cookie 2 remains the existing proof that the same slot recycles
   cleanly and forwards only the new datagram.

This is an assertion-order correction inside the current scenario. It adds no
test, state, seam, model action or API. It prevents both the existing
`slot_owner` fallback and any newly mirrored late fallback from satisfying the
release-report contract.

The RED run remains useful: it proves the current implementation does not
perform the required cleanup even after the late readiness path in this Sim
composition. After the assertion-order correction, a fresh semantic RED must
still fail at the immediate post-`SocketReleased` cleanup oracle, independent
of any proposed production remedy.

### Iteration 8 mechanical disposition

| Gate | Result |
| --- | --- |
| Existing scenario identity | PASS — unchanged |
| Contract Shape | PASS — exact bounded-change declaration retained |
| Production owner boundary | PASS — real `GuestFlowOwner::dispatch_next` |
| Sim seam use | PASS — accepted owner-layer arbiter; no kernel claim made |
| Honest RED | PASS — 1 run / 0 pass / 1 fail / 0 ignore, discarded 0 vs 200 |
| Release-before-readiness setup | PASS |
| Immediate release-report oracle | **FAIL — absent** |
| Late-fallback exclusion | **FAIL — cleanup is checked only after `SlotReady`** |
| Discard/disconnect/release order | PASS as an eventual oracle; must move earlier |
| No owner/association/request/channel/pair | PASS as eventual oracles; must move earlier |
| Recycled-slot isolation | PASS — retained |
| Neighboring UDP bodies | PASS — unchanged |
| New API/seam/model/scenario | PASS — none |
| Native causal attribution | PASS — none claimed |
| Check | PASS — `b1c9fbc9af65d67767720df10` |
| Clippy | PASS — `e27a275683c6be3fb31c52330` |
| Scoped `git diff --check` | PASS |

## Iteration 8 bounded verdict

**REJECTED PENDING ONE ASSERTION-ORDER CORRECTION.** The existing body has the
right production owner boundary, concrete release-before-readiness schedule,
exact cleanup/recycle outcomes and honest RED. It currently permits the held
`SlotReady` and existing orphan fallback to perform the cleanup, so it does not
prove that `SocketReleased` reclaimed the unpublished exact-cookie slot claim.
Assert complete cleanup while readiness is still held, then retain the same
slot-recycle half. Worker F-01 through F-04 remain closed and evidence-complete.

## Iteration 9 — UDP assertion-order remediation

### Scope and retained gates

Iteration 9 re-reviews only the existing UDP released-burst body. The author
changed no production source, Sim adapter, model, API, test identity, DES event
or commit for this remediation. Worker findings F-01 through F-04 remain
closed and evidence-complete.

The corrected test source has SHA-256
`76304889e9073c1d4f0598045b0e0bdacd1f2e198a2e3dcfd7630d0cfcc7c919`.
Retained Metal `--no-sudo` gates report:

- exact RED `5c779e807480c44f229568b1`: one test, zero passed, one failed,
  zero ignored; the first immediate cleanup assertion observed discarded 0
  instead of 200 before any readiness dispatch;
- check `dbdc944cb47a75c1b48cfe5f0`: pass;
- Clippy `894f905025074a4502e7b3040` with `-D warnings`: pass;
- scoped `git diff --check`: pass.

This reviewer ran no command and changed only this Markdown artifact.

### F-05 disposition

**CLOSED.**

The corrected body now places the complete cleanup oracle at the exact
`SocketReleased` boundary (`udp_slots.rs:242-317`):

1. Cookie 1 / destination 1 claims one slot and queues exactly 200 datagrams.
2. The first readiness remains held, and the real production owner has no
   published slot owner, association or pair.
3. The effect log is cleared before the socket closes and the existing
   `SocketReleased { cookie(1) }` event is released.
4. `g.settle()` dispatches that release event through the production
   `GuestFlowOwner` while readiness is still held.
5. Immediately afterward—without any `release_slot_ready(s)` call—the body
   requires all 200 datagrams discarded, kernel owner/flow/attachment absent,
   the slot disconnected, exact `discard+disconnect` then `release` effects,
   parked queue absent, readiness absent, release report consumed, no owner
   state, no request, no flow-stream connection and zero forwarder pairs
   (`:271-302`).
6. Only then does cookie 2 / destination 2 send. The same slot must recycle,
   produce the sole `DatagramStream` request and forward exactly one new
   datagram (`:304-316`).

The old held readiness is never dispatched after reclamation. The existing
late `KernelGuestOwnerIo::slot_owner` orphan fallback therefore cannot satisfy
the scenario. Any implementation that leaves `SocketReleased` unmatched still
fails on the first post-settle assertion, before owner readiness can run.

### Honest boundary and RED

The test continues to exercise the real `GuestFlowOwner::dispatch_next` over
the existing `SimGuestOwnerIo` arbiter. It does not claim to execute the kernel
adapter, BPF maps or real slot sockets. Its purpose is to pin the owner/event
contract and the Sim-visible consequences of private exact-cookie cleanup.

Exact RED `5c779e807480c44f229568b1` is now fail-for-the-right-reason evidence:
the scenario reaches the release-before-readiness state, dispatches
`SocketReleased`, and immediately finds the queue undiscarded. It does not
depend on the proposed production implementation, a newly invented state, or
the later readiness fallback.

The native `7375ebb87a3432b92f4d5ea3` non-atomic counters and the current Quint
projection remain outside this proof. No causal claim about every native
`EPERM` or model coverage is added. The handed-off DESIGN checker still finds
no contract/API gap: exact-cookie cleanup stays private adapter HOW behind the
existing event and trait shapes.

### Contract and neighboring coverage

The corrected body matches R5-14/D25 exactly: release of the application
socket frees every slot found by cookie, discards without reading, disconnects,
returns the slot, never starts an association for the released owner and never
leaks its queued datagrams to the next owner (`feature-delta.md:3104-3119`,
`:13037`).

The original final discard/recycle assertions are preserved. Other UDP bodies
cover pending/paired release, late `Paired`, refusal, timeout, pair-end and
audit fallback independently and are unchanged. This remediation introduces
no second scenario or duplicate obligation.

### Iteration 9 mechanical disposition

| Gate | Result |
| --- | --- |
| Existing scenario identity | PASS — unchanged |
| Contract Shape | PASS — exact bounded-change declaration retained |
| Release-before-readiness setup | PASS |
| Effect-log boundary | PASS — cleared before `SocketReleased` |
| Immediate discarded count | PASS — asserted before readiness |
| Immediate kernel claim cleanup | PASS — owner/flow/attachment absent, disconnected |
| Exact cleanup order | PASS — discard/disconnect then release |
| Held readiness removal | PASS — readiness must be empty before recycle |
| Owner side effects | PASS — no owner/association/request/channel/pair |
| Late-fallback exclusion | PASS — old `SlotReady` is never dispatched |
| Recycled-slot isolation | PASS — same slot, new cookie/destination, one request/datagram |
| Production owner boundary | PASS — real `GuestFlowOwner::dispatch_next` |
| Sim/kernel claim discipline | PASS — Sim arbiter only; no kernel proof claimed |
| Model claim discipline | PASS — no Quint coverage claim or model change |
| Neighboring bodies | PASS — unchanged |
| New API/seam/state/scenario | PASS — none |
| Honest RED | PASS — `5c779e807480c44f229568b1`, discarded 0 vs 200 before readiness |
| Check | PASS — `dbdc944cb47a75c1b48cfe5f0` |
| Clippy | PASS — `894f905025074a4502e7b3040` |
| Scoped `git diff --check` | PASS |

## Iteration 9 bounded verdict

**APPROVED — F-05 CLOSED; HONEST RED READY FOR IMPLEMENTATION.** The existing
UDP released-burst scenario now proves that `SocketReleased` reclaims an
unpublished exact-cookie slot claim immediately, before any held readiness can
invoke the existing orphan fallback. It retains the complete discard,
disconnect, release, no-side-effect and recycled-slot isolation oracles without
adding surface or scope.

This is acceptance-test approval, not STEP 14-02 GREEN. The production/Sim
implementation must make this exact body GREEN and then pass its own fresh
review and execution gates. Worker support remains separately approved and
evidence-complete.

## Iteration 10 — UDP Sim/Quint alignment and formal-evidence review

### Scope and source identity

Iteration 10 reviews only the bounded owner/Sim/Quint alignment required by
the iteration 9 body. It does not review the original crafter's UDP production
implementation for DELIVER approval, does not revisit worker F-01 through
F-04, and does not authorize a new scenario, API, seam, model state or native
lane. The iteration 9 released-burst body is byte-unchanged at SHA-256
`76304889e9073c1d4f0598045b0e0bdacd1f2e198a2e3dcfd7630d0cfcc7c919`.

The reviewed alignment identities are:

| Source | SHA-256 |
| --- | --- |
| `crates/overdrive-init/src/guest_flow/sim.rs` | `c928ebb3d1286a0f24cfb2d36b5371abfb2e12f1f15006b6eab1a0310aeddbe9` |
| `crates/overdrive-init/src/guest_flow/quint_udp_slots.rs` | `b48cb3cbd66e518aefedfc4dca38759728a6480b4b82cadb186cb7476ef715ce` |
| `specs/quint/guest-flow-owner/udp_slots.qnt` | `2f773c0219cc3a6319f4a8121c30ac59a7db1bdbed2cf12fea319373132ce775` |
| Current production comparison, `kernel_io.rs` | `23262cda7e67ef1649b852723bf925641fc4762c98373fa53436ec7397733dda` |

This reviewer performed a read-only source, diff, hash and generated-evidence
audit. No Rust test, native body, model checker, Metal/Lima command, production
edit, test edit, model edit, DES event or commit was executed or authored by
this review. This Markdown file remains the only review output.

### Exact resident-claim boundary

The Sim change is the minimum support needed for the existing scenario and
the existing `udp_slots` model to observe the interval that iteration 9 now
pins:

- `prepublication_slots_for_cookie` selects an exact-cookie resident claim
  only while its readiness remains held and no current flow, V_g or pair has
  crossed the owner boundary (`sim.rs:734-750`). It deliberately reads the
  current `rec.vsock`, not retained `last_vsock` history. A recycled slot is
  therefore not blocked by an association that already ended.
- Reclamation uses the existing `GuestOwnerIo::end_association` followed by
  `GuestOwnerIo::release_slot` (`:769-778`). This retains discard,
  disconnect, readiness removal and pool return through the established Sim
  effects instead of adding a second cleanup mechanism.
- `release_socket_report` removes the held report, reclaims only those
  prepublication claims, then makes the existing `SocketReleased` event
  deliverable (`:1490-1498`). Published pending and paired slots remain for
  the production owner event path.
- The analogous Sim audit hook reclaims orphaned prepublication claims before
  returning the existing live-cookie result (`:1966-1975`). It adds no trait
  method or event field.

That classification corresponds to the current kernel adapter's successful
report path. `KernelGuestOwnerIo::drain_user_events` calls the private
exact-cookie scan before publishing an owner event
(`kernel_io.rs:448-469`). The scan leaves `connected_slots` and `slot_pairs`
on the owner path and reclaims every other exact-cookie resident claim through
`end_association` then `release_slot` (`:472-505`). The Sim's held-readiness
plus no-current-flow/V_g/pair predicate is its existing representation of the
same prepublication boundary. No public or cross-crate API changed.

The iteration 9 acceptance oracle remains honest after this correction. It
still dispatches the real `GuestFlowOwner` over the established Sim port, and
it still observes complete cleanup before any held `SlotReady` can be
released. The support implementation makes that accepted body GREEN; it does
not change the body or move an assertion behind the fallback.

### Quint projection and behavior

`kernelSlots` was already a model variable and `KernelSlot` was already a
model type. The correction adds that existing variable to the compared state;
it does not add model state. `KernelSlotView` decodes the existing Quint value
(`quint_udp_slots.rs:619-668`), and the driver reads the Sim resident claim
from the existing `slot_state(slot).owner` arbiter (`:694-768`). QPROJ-5
continues to supply the separately published owner, association and pair.
The comparison can now distinguish a resident kernel claim from an owner row
without deriving either fact from the test stimulus.

The formal edits preserve the existing action, invariant and named-run
inventory:

- 12 `#[quint_test]` identities and one `#[quint_run]` remain in the driver;
- all 15 named Quint runs remain the same identities;
- `kernelSlots`, `KernelSlot`, `GuestReleaseSock`, `GuestAuditIdle`,
  `GuestPairingTimeout`, `Safety` and `ReleasedSlotBecomesFree` remain the
  existing vocabulary;
- no generation, event field, protocol message, port or map was introduced.

The semantic change is required rather than a weakening. Exact-cookie release
now ranges over resident `kernelSlots`, including the unpublished claim
(`udp_slots.qnt:402-419`). The audit orphan set, pairing-deadline precedence
and release liveness use the same resident owner (`:433-448`, `:502-508`).
The existing `LostReleaseReportIsFoundByAudit` and
`RecycledSlotNeverDeliversPreviousOwnersFrames` runs now exercise the
prepublication state directly (`:561-568`, `:600-608`). The invariants still
require every published owner to match its kernel claim, every released
kernel slot to have no owner/data/pair/flow, owner-only data while held, exact
routing, late-`Paired` abort and no datagram-free reassociation
(`:465-500`). Safety is unchanged in name and strength; liveness is
strengthened to include the resident interval.

The pairing-deadline guard is consistent with the approved clocks: the
fixed-rate release audit is due at most one second after a cookie disappears,
before a five-second pending-association deadline. The conformance driver
settles all due owner deadlines in order, so a resident orphan must be removed
before a later `GuestPairingTimeout` step. This change corrects an observation
boundary; it does not grant the application or model a new transition.

### Reproductions and source gates

The retained alignment receipts are mutually consistent:

| Evidence | Result |
| --- | --- |
| Pre-correction exact seed `0xde1cfaf4`, difference `dde03c03ab5ff2b816e86bb3` | Honest mismatch: the spec had recycled to `(2,2)` while Sim retained the former resident owner/datagram because historical V_g state blocked cleanup. |
| Existing released-burst body `195eaff772c5473615757561` | PASS; body source remains the iteration 9 hash. |
| All 26 bounded UDP owner/Sim cells `98d5584ab3599d8db3f07622` | PASS. |
| Seed `0xdc1410e7`, `40c8767c8bd0a945d147dd55` | PASS. |
| Replayed seed `0xde1cfaf4`, `2bbd154605a18019d1980c7f` | PASS after using current active association state rather than retained history. |
| Check `1750e249315bea9be59eb47a0` | PASS. |
| Clippy `47db32edeeec037629e07d730` | PASS with `-D warnings`. |
| Conformance lint `2e57ada141813571e663ffed0` | PASS. |
| Format and scoped diff checks | PASS as retained. |

These results close the support mismatch exposed by the prior seed. They are
owner/Sim/model evidence; none is represented as execution of BPF, socket
diagnostics, kernel slot maps or the production `KernelGuestOwnerIo`.

### Canonical formal record

Changing `udp_slots.qnt` changed an atomic subsystem input, so a full
three-job `--record` was mandatory. The canonical generated summary records
run `2228824-1793511799.752`, from
`2026-11-01T05:43:19.751865088Z` through
`2026-11-01T06:55:31.02853888Z`, with Quint 0.32.0, Apalache 0.56.1 and
kernel 7.0.0-38-generic. Retained Metal receipt
`fc8443bcede4d27dd0ff0a60` reports all 279 checks matched `expect`; all 21
UDP checks passed, including exhaustive safety/liveness, expected hazard
violations and all existing named runs. The summary's 28 input hashes contain
the reviewed `udp_slots.qnt` hash
`2f773c0219cc3a6319f4a8121c30ac59a7db1bdbed2cf12fea319373132ce775`;
the diff changes no other formal input hash. `verify-evidence` receipt
`9ea4762a840e20a5b48517540` passed.

The broad generated trace churn is therefore expected: repository policy
replaces the canonical evidence directory as one full subsystem record, and
git history retains its predecessor. It is not an attempt to substitute a
historical UDP-only run for current evidence.

One inherited evidence-policy discrepancy remains explicit. The current full
record times `udp-slots-hazard-late-paired-by-slot-route` at 191.905 seconds
while `checks.toml` still has `ci = true`. That flag and its 900-second check
timeout are unchanged from the preceding record, which also took about 190
seconds; no new timeout waiver or semantic credit follows from the flag. This
bounded review treats the runner warning as an inherited formal-maintenance
issue and does not use it to claim that the check fits the CI budget. It does
not invalidate the completed full-record outcome or readiness to rerun the
already-staged exact native churn identity below, but it must not be presented
as compliance with DR6-CLOSE's current under-60-second CI eligibility rule in
a final DELIVER approval. This is an explicit limit, not a waiver.

### Kernel-adapter parity and coverage limit

The report path is aligned with current production; the new audit branch is
not production evidence. The current production owner passes only cookies
already present in its published slot table to `live_cookies`
(`owner.rs:919-936`). `KernelGuestOwnerIo::live_cookies` only intersects that
input with socket diagnostics (`kernel_io.rs:1182-1188`); it does not scan and
reclaim a kernel-only `SLOT_META` claim. By contrast, the new Sim
`live_cookies` also reclaims an orphaned prepublication claim, and the
strengthened `LostReleaseReportIsFoundByAudit` model run now requires that
outcome.

This is a **coverage-only limit and an unproven production-parity hypothesis**,
not a demonstrated production defect and not evidence that the approved
outcome is unimplemented. No retained native reproduction in this review
isolates the required ordering: a kernel claim exists, its release report is
lost, the claim is still unpublished when the audit fires, and the audit—not
the later `slot_owner` missing-cookie fallback—must reclaim it. Static source
shows that the existing `live_cookies` call itself does not scan kernel-only
claims, but static source does not establish the production event/readiness
ordering or exclude cleanup through the existing fallback. Under the
repository's reachability and falsification gates, that is insufficient to
open a production finding.

The approved D25/K-B2 MUST remains unchanged: a lost report is recovered by
the level-triggered audit. The all-26 Sim gate and the 279-check formal record
cannot be cited as production or Tier-3 proof of that prepublication case.
No weakening, waiver, new native scenario or production remediation is
authorized by this review. The next exact churn run also does not isolate this
lost-report ordering, so even a pass grants it no additional credit.

### Iteration 10 disposition

| Gate | Result |
| --- | --- |
| Iteration 9 body identity and Contract Shape | PASS — unchanged existing body and exact declaration |
| Exact report-path resident claim | PASS — exact cookie, held readiness, no current flow/V_g/pair |
| Cleanup order and readiness removal | PASS — existing end-association then release effects |
| Published pending/paired path | PASS — remains on owner close/abort/end handling |
| Recycled-slot history | PASS — `last_vsock` does not classify current ownership |
| Existing QPROJ-7/QPROJ-5 split | PASS — resident claim and published owner are compared separately |
| Model inventory | PASS — no new state, action, invariant, run, generation or protocol surface |
| Safety and liveness | PASS — safety retained; release liveness covers the resident interval |
| Public/API/typed-error surface | PASS — unchanged |
| Exact body / 26-cell / exact-seed gates | PASS — retained receipts above |
| Check / Clippy / lint / format | PASS — retained receipts above |
| Canonical formal record | PASS — current hash, 279/279, 21/21 UDP, jobs=3, verify-evidence pass |
| CI timing claim | LIMIT — 191.905-second check remains `ci = true`; no CI-budget credit granted |
| Production report-path parity | PASS at source boundary for the private HOW; the separate native churn result remains required |
| Production lost-report prepublication-audit parity | COVERAGE LIMIT — mandatory and modeled, but neither proven nor disproven through the production ordering; no defect finding without a bounded native reproduction |
| Worker F-01 through F-04 | CLOSED and evidence-complete; unchanged |
| Native execution by this reviewer | NOT RUN |

## Iteration 10 bounded verdict

**APPROVED ALIGNMENT; ONE EXISTING EXACT GUESTKIT CHURN VALIDATION IS READY.**
The support implementation now gives the source-local
`a_released_burst_is_discarded_and_the_next_owner_gets_none_of_it` owner/Sim
body and existing Quint model an honest resident-kernel-claim boundary. That
body is GREEN at its userspace owner/Sim boundary. It is not staged by the
STEP 14-02 selector and supplies no native GuestKit credit.

The next safe execution scope is exactly the already-staged CLI integration
identity
`integration::vsock_fault_matrix::closed_udp_sockets_release_their_slots_before_and_after_pairing`
through the existing STEP 14-02 GuestKit selector. Its unchanged production
journey deploys `UDP_CHURN` with 1,100 sockets, requires the allocation to end
`Terminated`, observes the later round trip exactly once, and forbids
duplication (`vsock_fault_matrix.rs:355-389`). No selector, test identity,
staged body or lane is added.

The retained exact GuestKit run `93f96bd412c3a4e454c67a73` remains the native
semantic RED: send 1,042 of 1,100 returned `EPERM`, the allocation ended
`Failed`, and the body failed its `Terminated` oracle. A future exact pass may
close only this existing native churn outcome—1,100 successful churn sends
without slot exhaustion plus the later round trip and no duplication. It
does not prove the separate lost-report prepublication-audit ordering, waive
the inherited CI-flag limit, or by itself approve whole STEP 14-02. This
reviewer launches no execution. Worker support remains independently approved
and evidence-complete; every earlier UDP RED receipt remains preserved as the
iteration history.
