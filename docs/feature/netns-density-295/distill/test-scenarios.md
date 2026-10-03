# DISTILL scenarios — `netns-density-295` (correctness-recovery rewrite)

This document is specification prose. Per `.claude/rules/testing.md` it is not
a Cucumber input, and no `.feature` file exists. Every scenario maps to a Rust
`#[test]` / `#[tokio::test]`, a proptest, a seeded `overdrive-sim` schedule, a
Tier-2 `BPF_PROG_TEST_RUN` partition, a native-metal body, or a
benchmark/measurement receipt, exactly as its field table states. No scenario
is a verification expectation: every #295 contract, including the serve
process's exit status, is driven through a production port by a test.

It targets the accepted replacement DESIGN
(`feature-delta.md` § *Correctness-Recovery Replacement DESIGN — ACCEPTED
2026-09-24*, decisions D-295-R1 to D-295-R22, ADR-0127 to ADR-0143), not the
current implementation. A citation `FD § "<heading>"` names the section of
`docs/feature/netns-density-295/feature-delta.md` under that exact heading,
optionally followed by a locator within the section; it cites the DESIGN as it
stands after the 2026-09-25 DESIGN pins of DISTILL gaps B-1 to B-7 and N-1
(§ *DESIGN gaps — pinned*). Where an
earlier DESIGN contract (C-295-*, D-295-DISTILL-*, RUN-295-B, …) sits beside a
*PROPOSED/PENDING D-295-Rn* marker, the marker's accepted contract governs
(FD § "Wave: DESIGN / [REF] Correctness-Recovery Replacement DESIGN — ACCEPTED 2026-09-24" (the in-line marker rule)).

## Binding rules applied

- Tests never spawn the `overdrive` binary. In-process tests start the server
  through `run_server*`, `serve::run_with_kek`, or the exported handler. The
  serve process's exit status is driven through the serve lifetime port
  (`ServeLifetime::run` → `ServeExit::exit_code()`, D-295-R17), so it is a
  test (S-ND295-68), never a verification expectation. #295 authors no
  verification expectation.
- Seeded proofs live in `overdrive-sim` (tests/integration or its invariant
  modules) or, for the module-private supervisor, source-locally in
  `overdrive-control-plane`. The source-local lanes use `overdrive-sim`
  adapters only for ports declared in `overdrive-core` or `overdrive-worker`,
  and the crate-private test-local ports of § *Test-local control-plane ports*
  for the three ports `overdrive-control-plane` declares (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the seeded-sim lane and its test-local ports)). Every
  seeded body prints its seed on every verdict.
- Real guests run only on native x86_64 metal (`cargo xtask metal run --`).
  The Lima VM on this host is aarch64 and runs no Cloud Hypervisor. The
  non-x86_64 launch-refusal cases therefore run only on the aarch64 Lima build
  (user ruling 10, GH #302); the x86_64 launch-filter cases run only on an
  x86_64 build (native metal, or an x86_64 CI Lima runner).
- No kernel-version gating or kernel matrix scenario exists.
- A future-step body is complete and executable, marked
  `#[ignore = "pending DELIVER step NN-NN (S-ND295-xx)"]`. No placeholder
  `panic!` body and no `#[should_panic(expected = "RED scaffold")]` is handed
  to DELIVER (`testing.md` § "RED acceptance scaffolds and activation").
- Faults enter only through an existing or accepted driven port, a real kernel
  mutation, or a real resource the production path created. No test installs a
  production effect the production path omits.
- Tests and benchmark/measurement receipts stay separate (§ *Receipts — not
  tests*).

## Scope and out-of-scope

- In scope: the seven correctness gaps (FD § "[REF] Invalidation register (charter §2)" (the seven-gap table)) and every accepted
  decision R1-R22 with its evidence row E1-E21 (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (rows E1 to E21)) and gate cell
  G-295-0..5 (FD § "Required boundary scenarios per gate").
- Out of scope (FD § "[REF] Out-of-scope items and their issues (user rulings of 2026-09-24)"): node-wide CPU/memory accounting (GH #261);
  derived or configurable per-node guest-network capacity (GH #299); operator
  restart of a stopped Job (GH #301); aarch64 microVM launch (GH #302). No
  scenario below manufactures those outcomes. GH #234 is superseded when #295
  lands; GH #197 stays open (FD § "[REF] Charter, rulings, and evidence" (the GitHub context)).
- CAP-295-A is attachment-only; 16,384 is a fixed placeholder cap, not a
  density promise (FD § "[REF] Charter, rulings, and evidence" (user ruling 1 of 2026-09-24, D-295-R7)).

## Journey and outcome trace

| Product journey / outcome | User-valued promise | Scenarios | Registered outcome |
|---|---|---|---|
| J-OPS-003 — run a VM workload | Ana deploys, stops, and restarts VM work with the familiar verbs; lifecycle, cleanup, and capacity stay truthful | S-ND295-01, 05A-05E, 06, 07, 07B, 11, 12, 13A-13D, 35, 52, 53, 55-57, 66 | OUT-ND295-SHARED-SWITCH |
| J-OPS-004 — submit a Service | A VM Service keeps its health/selection truth and its replicas count never includes an allocation still being cleaned up | S-ND295-01, 21, 34, 36, 58-60 | OUT-ND295-SHARED-SWITCH; related OUT-SVM-SERVICE-TARGET-PROJECTION |
| J-MESH-001 — dial a mesh peer by name | A credential-free guest resolves and reaches a healthy peer by name, and no guest frame precedes interception | S-ND295-01, 08, 09, 10, 24, 34, 37, 47 | OUT-ND295-BORN-CAPTURED |
| J-SEC-003 — enforce transparent mTLS | Guest plaintext stays guest-local, the peer leg is TLS 1.3/kTLS, a compromised VMM cannot steal another guest's traffic, and every shared-network loss fails closed or stops the node | S-ND295-01, 10, 14-26, 29A-33, 37-50, 54, 61-64, 67-70 | OUT-ND295-BORN-CAPTURED; related OUT-MTLS-COMPOSED-PROXY-SKELETON, OUT-MTLS-WIRE-TLS13 |
| Operator status — cleanup pending | `overdrive workload describe` never shows an allocation whose network cleanup is unfinished as Running | S-ND295-58, 59, 60 | OUT-ND295-SHARED-SWITCH |
| Operator process lifetime | `overdrive serve` exits with status 1 on shared-network fail-stop | S-ND295-68 | OUT-ND295-SHARED-SWITCH |
| Attachment-capacity receipts | Operators receive honest attachment receipts with no density or flow claim | B-ND295-T1-BASE, B-ND295-T1-PORT4, M-ND295-E18 | OUT-ND295-DENSITY |

## Evidence lanes

| Lane | Meaning | Runner |
|---|---|---|
| **pure** | default-lane, no I/O; table or proptest | `cargo xtask lima run -- cargo nextest run -p <crate> …` (Lima is only the Linux toolchain) |
| **seeded-sim** | in-process logic over simulated adapters with printed seeds, no `run_server` (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the seeded-sim lane and its test-local ports)). The supervisor lanes are source-local in `overdrive-control-plane/src/lib.rs`: `overdrive-sim` adapters for ports declared in `overdrive-core` or `overdrive-worker`, and the crate's test-local ports (§ *Test-local control-plane ports*) for the three it declares. A worker over `SimMtlsIntercept`, or over a test-local intercept that delegates binding to it, binds no socket and runs no accept thread when its shared owner starts (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `SimMtlsIntercept` contract; the effect on D-295-R16, E16, and lane classification)), so starting it gates no body; a seeded body leaves the default lane only for another `testing.md` reason (S-ND295-05D: its wall-clock budget) | Lima runner (the Linux toolchain); seeds via the scenario's env var or proptest |
| **seeded-in-process** | seeded schedule that drives `run_server*`; because R16 always composes the real `HostMtlsEnforcement` kTLS probe (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (`compose_mtls` is deleted)), it needs Lima root and `integration-tests` | `cargo xtask lima run -- … --features integration-tests` |
| **in-process** | Lima root, real `run_server`/handler composition with sim or real injected ports (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the in-process lane)) | Lima runner, `integration-tests` |
| **lima-kernel** | Lima root real kernel objects (TAP, nft, netlink, seccomp) without a guest VM | Lima runner, `integration-tests`; x86_64-only cases on an x86_64 build |
| **tier2** | `BPF_PROG_TEST_RUN` program partition | `cargo xtask bpf-unit` / `overdrive-bpf` integration binary |
| **xtask-integration** | the `xtask` integration binary: a gate run over a real or temporary Cargo workspace (`cargo metadata`, a subprocess) | `cargo xtask lima run -- cargo nextest run -p xtask --features integration-tests` |
| **native** | non-virtualized x86_64 metal, real Cloud Hypervisor guests, `integration-tests,kvm-tests` | `cargo xtask metal run -- cargo nextest run …` |

## Stakeholder acceptance language

The four blocks below are the stakeholder-readable promises. The numbered
S-ND295 sections are the technical verification contracts that prove them.

### An operator deploys two VM workloads that reach each other by name

`@walking_skeleton @driving_port @real-io @contract-shape:bounded-change`

```gherkin
GIVEN a node ready to accept VM work
WHEN Ana deploys the checked-in VM Service and its VM client Job
THEN the callee reports Running with replicas 1/1
AND the caller reports Succeeded with the exact guest reply
AND no guest network frame leaves either VM before its protection is live
AND stopping both workloads leaves nothing the journey owned
```

### A compromised VM cannot reach or steal another guest's traffic

`@security @real-io @contract-shape:bounded-change`

```gherkin
GIVEN two guests running side by side on one node
WHEN one guest's VM process tries to rewrite its own network device or impersonate the other guest
THEN the platform refuses the device change at its source
AND even a change made from outside the VM never delivers the other guest's traffic to it
AND the platform stops only the damaged VM and restores delivery to the victim
```

### A node refuses new work when shared-network trust cannot be proved

`@operator @error @recovery @contract-shape:bounded-change`

```gherkin
GIVEN a running node loses a shared-network component
WHEN the platform cannot restore and verify it within five seconds
THEN new guest commands stay closed while it tries
AND only the VMs whose isolation cannot be confirmed are stopped
AND the node process exits with status 1 and an operator-visible reason instead of running partially trusted
```

### Capacity and cleanup stay truthful

`@operator @capacity @contract-shape:bounded-change`

```gherkin
GIVEN a node holding guest network attachments, some still being cleaned up
WHEN the operator starts, restarts, or describes workloads
THEN an attachment counts against the node until its cleanup finishes
AND a replacement starts only when there is room
AND an allocation whose cleanup has not finished is shown as cleanup-pending, never Running
```

## Technical verification contracts

Each contract carries a field table. **Discharges** names the evidence row(s),
gate cell(s) (`G<n>/r<row>` = G-295-n, boundary row n of FD § "Required boundary scenarios per gate"), and
decision(s). **Disposition** is RETAINED (contract unchanged, body kept),
RETARGETED (body rewritten to the accepted contract), NEW, or DELETED. **Step**
is the DELIVER step that activates the body (see § *DELIVER re-roadmap input*
in the feature delta); `active` means the body runs today and must stay green.

### Group A — Journey and VMM launch boundary (gap 7)

#### S-ND295-01 — Two VM workloads reach each other by name with no frame before protection

`@walking_skeleton @driving_port @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN the node is started through serve on native metal with no allocation owned by the example
WHEN the operator deploys the checked-in VM Service and VM client Job
THEN the callee reports Running 1/1 and the caller reports Succeeded after the byte-distinct reply by service name
AND each guest's network device stays down from VM creation through readiness and Running
AND the device comes up only after the allocation's protection-live event and before its command is released
AND no guest frame is captured at or before that event, and every guest frame after it is well-formed
AND the peer leg is TLS 1.3 kTLS with plaintext confined to the guest-local legs
AND stopping both workloads leaves nothing the example owned
```

| Field | Value |
|---|---|
| Discharges | E1 native, E3, E4 native (READY through fd handoff on a uid-0 TAP), E21 (e) traffic half; G2/r1, G3/r1; D-295-R1, R2, R4, R5, R21, R22 |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | in-process `serve::run_with_kek` + public `deploy` handler (no binary spawn) |
| Fault stimulus | none (healthy journey) |
| Oracle | (1) a netlink `RTM_NEWLINK` monitor on the caller TAP's ifindex, started before deploy, shows `IFF_UP` clear from CH process creation through READY and the Running row, and the first `IFF_UP` notification strictly after the realtime `mtls.intercept.install.success` event (tracing Layer, `CLOCK_REALTIME` in `on_event`, FD § "Gate G-295-2 — existing guest command release, narrowed to the new switch" (the S-ND295-01 intercept-live timing receipt)) for the exact allocation; (2) one loss-accounted exact-ifindex AF_PACKET capture, bound while the TAP is down: its first link-down report is the bind-time pending `ENETDOWN`, consumed at zero frames and never the end of the capture; it is blind until the TAP comes up, so its zero is evaluated only after a positive witness (well-formed guest frames strictly after the event) and full loss accounting (`tp_drops == 0` and `tp_packets` equal to the frames read, over a sealed final drain): zero caller-TAP frames with timestamp `<=` the event, and every later guest ARP/ICMP/TCP frame decodes with no 12-byte zero prefix (vnet header correct, E3); the down interval itself is witnessed by the counters (3) and the link history (1); (3) all six TAP counters read 0 at the event bracket; (4) the queue holder set of the caller TAP equals the Cloud Hypervisor pid (`/proc/<pid>/fdinfo` `iff:`); (5) retained kTLS/splice leg-B evidence (FD § "Feature Delta — `netns-density-295`" (the D-295-DELIVER-04-01 kTLS/splice evidence-boundary paragraph)): one `ss` TLS 1.3 tuple/inode/sole fd with bidirectional splice, loopback-only leg-B, plaintext only on leg-F/leg-S tuples, no direct bypass; (6) empty VMM/TAP/TCX/nft-member/cgroup/run-dir complement after stop. Any missing/duplicate event, lossy capture, or unprovable frame fails closed. |
| Seed / isolation | example-based; whole `overdrive-cli` integration binary is `host-kernel-shared` (`.config/nextest.toml`) |
| Rust home | `crates/overdrive-cli/tests/integration/guest_stack_mtls_egress.rs::{microvm_dials_a_mesh_peer_by_name_and_receives_the_reply, the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear, the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes}`; harness evidence for oracles (2) and (3): NEW `…::a_capture_bound_to_a_down_tap_reads_every_frame_after_the_up_transition_and_none_before` — both capture shapes (link-layer and datagram) bound to a down scratch TAP, which is then raised, attached through a raw `TUNSETIFF` queue, and written 16 distinct frames: each capture reports exactly the bind-time `ENETDOWN` at zero frames, no removal, no frame at or before the up transition, the 16 guest-direction frames byte-exact and in order, and `tp_packets` equal to the frames read with zero drops (DISTILL review B1) |
| Disposition / step | RETARGETED (pre-event oracle is now zero frames with the TAP down; the down-through-READY check runs over fd handoff) — 10-01. The capture self-test is NEW and **active** (native, no guest) |

#### S-ND295-35 — Each VMM holds exactly its own TAP queue and nothing of the server

`@real-io @adapter-integration @error @contract-shape:bounded-change`

```gherkin
GIVEN two VM allocations started through serve and deploy on one node
WHEN both guests are Running
THEN each VMM's only network descriptor is its own TAP's queue at descriptor 3
AND no VMM holds any socket, pipe, or queue the server holds, apart from its stderr pipe
AND neither VMM holds the other's queue, and the server holds no queue after launch
AND the host network namespace contains one bridge and no per-workload namespace, veth, or /30
```

| Field | Value |
|---|---|
| Discharges | E2 native, E4 native; G4/r1, G4/r5; D-295-R1, R2, R3, R4 |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve::run_with_kek` + `deploy` |
| Fault stimulus | none (concurrent second launch is the contrast) |
| Oracle | complete `/proc/<ch>/fd` of every CH pid; descriptor 3 `fdinfo` shows `iff:<own tap>` and `O_RDWR\|O_NONBLOCK`; no other TUN queue; inode set shares nothing with `/proc/self/fd` of the test process (the in-process server) except the stderr pipe; server fd table holds no TUN queue after spawn; argv contains `--net fd=[3],mac=…,offload_tso=off,offload_ufo=off,offload_csum=off` and no `tap=`; topology facts retained from the existing body; each TAP's activation is polled with a 10 s bound after its Running row (activation may follow the row) and a TAP that vanishes fails the body (DISTILL review M12) |
| Seed / isolation | example-based; `host-kernel-shared` |
| Rust home | `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs::two_vm_allocations_share_the_node_bridge_without_per_workload_namespaces` (topology, RETARGETED argv) and NEW `…::each_vmm_holds_only_its_own_tap_queue_at_descriptor_three` |
| Disposition / step | RETARGETED + NEW — 10-01 |

#### S-ND295-38 — A queue is handed over only from an exact, down, persistent TAP

`@real-io @adapter-integration @error @contract-shape:bounded-change`

```gherkin
GIVEN a persistent single-queue TAP created by the network owner and held down
WHEN the launcher attaches one queue for a VM
THEN the queue carries exactly the device settings a VM needs and the TAP is still down
AND an attach to a raised TAP, to a TAP that already has a queue, or to a name that would create a fresh device is refused with its own typed cause
AND the attach never raises, lowers, renames, persists, or deletes the TAP
```

| Field | Value |
|---|---|
| Discharges | E2 Lima; G4/r1, G4/r2, G4/r3; D-295-R2 (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the `overdrive-netlink` TUN helper and the `attach_tap_queue` contract)) |
| Contract shape | bounded-change |
| Lane | lima-kernel (no KVM needed) |
| Driving port | `overdrive_netlink::attach_tap_queue(name)` (the owner-side port the VMM adapter consumes) |
| Fault stimulus | real kernel states: TAP raised with `set_link_up`; a second attach while the first `TapQueue` is held; an absent name (creates a non-persistent device) |
| Oracle | success: `TUNGETIFF == 0x5802`, `SIOCGIFFLAGS` `IFF_UP` clear, `TapQueue::name()` equals the TAP; each refusal is exactly `TapQueueError::{NotDown, Attach(EBUSY), Flags}` with its source; after `Flags` the transient device is gone (closing destroyed it); after every case the TAP's admin state, persistence, and owner read back unchanged; dropping `TapQueue` releases the queue (a new attach succeeds) |
| Seed / isolation | example table; scratch TAP names outside `ovd-tp-` with RAII delete; `overdrive-netlink` integration binary is `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-netlink/tests/integration/tap_queue_attach.rs::{a_down_persistent_tap_hands_over_exactly_one_vnet_header_queue, every_attach_precondition_violation_is_refused_with_its_own_typed_cause}` |
| Disposition / step | NEW — 05-03 |

#### S-ND295-39 — A root-owned TAP refuses any process that holds no queue

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a persistent TAP owned by the root launcher and held down
WHEN an unprivileged process with the VM uid and no network capability tries to attach it
THEN the attach is refused as not permitted
AND the owner's own provision records the TAP owner as root
```

| Field | Value |
|---|---|
| Discharges | E4 Lima + in-process; G4/r5; D-295-R4 (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the creator owner, D-295-R4)) |
| Contract shape | bounded-change |
| Lane | lima-kernel (attach) + pure (owner expectation, carried by S-ND295-11) |
| Driving port | `create_persistent_tap(name, 0)` then `attach_tap_queue` from a forked child that sets uid/gid 4200 and clears every capability |
| Fault stimulus | real credential drop in the child |
| Oracle | child reports `TapQueueError::Attach` with `EPERM`; parent then attaches successfully (queue free); TAP owner reads back uid 0 |
| Seed / isolation | example; scratch TAP; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-netlink/tests/integration/tap_queue_attach.rs::a_root_owned_tap_refuses_an_unprivileged_attach_with_eperm` |
| Disposition / step | NEW — 05-03: its only production dependency is `attach_tap_queue`, which 05-03 lands (DISTILL review M3, DR-01). Expected GREEN once `attach_tap_queue` exists: it pins kernel behaviour the design relies on; S-ND295-11 carries the RED owner expectation at 06-02 |

#### S-ND295-40 — The VMM adapter launches from the queue and reports queue failures in their own terms

`@error @contract-shape:bounded-change`

```gherkin
GIVEN a VM configuration with a guest network attachment
WHEN the VMM adapter builds and starts the launch
THEN the network argument names only descriptor 3 and the guest MAC
AND every queue attach failure is reported as the matching VMM queue error
AND a launch that fails to start leaves no queue held, before any cleanup waits
AND the launcher no longer requires the ip tool
```

No Cloud Hypervisor version is parsed, compared, or gated (user ruling of
2026-09-25, feature delta § "User ruling of 2026-09-25"); the capability stage
reads `--version` only as diagnostic text. This scenario therefore carries no
version-refusal oracle and no `cloud_hypervisor_older_than_v53_is_refused_by_the_probe`
body (confirmed absent by `grep -rn cloud_hypervisor_older_than_v53 crates` →
zero; the name was never committed).

| Field | Value |
|---|---|
| Discharges | E2 in-process, E5 in-process (spawn-error/no-pid branch closes the queue first, F19); G4/r2; D-295-R1, R2 (feature delta § "Core `VmmError` additions", § "`CloudHypervisorVmm` in `overdrive-host`") |
| Contract shape | pure-function (render) / bounded-change (queue-error mapping through `create`; Lima queue-release) |
| Lane | pure (source-local `overdrive-host` `vmm::tests`, launch shape) + native x86_64 metal for the spawn-failure body (`launch_seccomp_kernel` is `#[cfg(target_arch = "x86_64")]` and its fixture asserts a reflink-capable staging root, so it never runs in the aarch64 Lima VM; DISTILL review M5) + native (queue-error mapping through `create`, `kvm-tests`) |
| Driving port | `CloudHypervisorVmm::create` / `cloud_hypervisor_network_arg` |
| Fault stimulus | each queue-attach failure reachable from a real kernel state (an administratively-up TAP; an absent name that creates a fresh non-persistent device; a single-queue TAP whose one queue is already held); a configured launcher path that fails to spawn |
| Oracle | exact argv `fd=[3],mac=…,offload_tso=off,offload_ufo=off,offload_csum=off`; through `create`, each reachable queue-attach failure surfaces as its pinned `VmmError` — an up TAP → `TapQueuePostcondition { violation: NotDown }`, an absent name → `TapQueuePostcondition { violation: Flags { observed } }` with `observed & IFF_PERSIST == 0`, an already-held queue → `TapQueue { stage: Attach }`; `Open`, `FlagsReadBack`, and `AdminStateReadBack` are NOT asserted — no real kernel state produces them (opening `/dev/net/tun`, `TUNGETIFF` on the open queue, and `SIOCGIFFLAGS` on the named TAP all succeed), so their one-for-one mapping is the adapter's own concern, not this public-contract body; after the spawn error returns, a fresh `attach_tap_queue` on the scratch TAP succeeds; `REQUIRED_LAUNCH_TOOLS` lacks `ip` |
| Seed / isolation | table; the queue-error body runs as root on scratch TAPs (deleted by RAII) in the `host-kernel-shared` `overdrive-host` integration binary; the launch-shape body needs no kernel object |
| Rust home | `crates/overdrive-host/src/vmm.rs::tests::mesh_and_non_mesh_launches_preserve_shape_and_attribute_the_actual_launcher` (RETARGETED argv, launch shape) + NEW `crates/overdrive-host/tests/integration/vmm_tap_queue_errors.rs::every_tap_queue_error_maps_to_its_vmm_queue_error` (native, `kvm-tests`, through `create` — retargeted from a private-mapping unit test to the public `Vmm::create` contract, item 8) + NEW `…/vmm.rs::launch_seccomp_kernel::a_failed_spawn_releases_the_queue_before_any_cleanup_await` |
| Disposition / step | RETARGETED + NEW — 05-03 |

#### S-ND295-41 — The launched child inherits exactly descriptors 0 to 3

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN the server holds inheritable sockets and pipes when it launches a VM process
WHEN the launch hook runs in the child before its first exec
THEN the exec'd process holds exactly standard input, output, error, and its queue
AND a hook step that fails turns into a launch error, never a silent launch
```

| Field | Value |
|---|---|
| Discharges | E2 Lima; G4/r1, G4/r2; D-295-R3 (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the in-child close, D-295-R3); FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (`register_launch_child_hook` and its three child steps)) |
| Contract shape | bounded-change |
| Lane | x86_64 build only (the hook takes a `VmmLaunchSeccompFilter`, which exists only on x86_64): native x86_64 metal, or an x86_64 CI Lima runner; never the aarch64 Lima VM |
| Driving port | private `register_launch_child_hook(&mut cmd, first_closed, filter)` on a re-exec of the crate's own test binary; `classify_launch_spawn_error` for the step failures |
| Fault stimulus | an inheritable `socket(2)` and `pipe(2)` deliberately opened without close-on-exec in the parent before spawn; for the step failures (DISTILL review H7), a re-exec'd intermediate that sets no-new-privileges and loads a classic-BPF filter refusing exactly one hook step with `EPERM` — the close-on-exec `close_range`, `prctl(PR_SET_NO_NEW_PRIVS)`, or the filter load `seccomp(SECCOMP_SET_MODE_FILTER)` — and self-probes that exactly that step is refused, then launches a marker target through the production hook; a no-stimulus control intermediate |
| Oracle | the child's `/proc/self/fd` is exactly `{0,1,2,3}` (`first_closed = 4`) and exactly `{0,1,2}` (`first_closed = 3`); descriptor 3 is the scratch TAP queue. Step failures (G-295-4 row 2): the control spawns, exits 0, and writes the marker; each refused step makes the spawn return `Err` with errno `EPERM`, `classify_launch_spawn_error` maps it to `VmmError::Create` whose detail names the executable and the `EPERM` text, and the target never runs (no marker) |
| Seed / isolation | example; `#[cfg(target_arch = "x86_64")]`; scratch TAP outside `ovd-tp-` |
| Rust home | NEW `crates/overdrive-host/src/vmm.rs::launch_seccomp_kernel::{the_launched_child_inherits_exactly_descriptors_zero_to_three, a_failed_close_on_exec_step_is_a_launch_error_and_the_target_never_runs, a_failed_no_new_privs_step_is_a_launch_error_and_the_target_never_runs, a_failed_filter_load_step_is_a_launch_error_and_the_target_never_runs}` |
| Disposition / step | NEW — 05-02 |

#### S-ND295-42 — The launch filter's verdict is total and correct for every syscall shape

`@property @error @contract-shape:pure-function`

```gherkin
GIVEN the launch filter built for an x86_64 build
WHEN any syscall record is evaluated against it
THEN each of the thirteen TAP-mutating requests is refused as not permitted, whatever its upper bits
AND the device-attach requests Cloud Hypervisor needs, read-only requests, and other syscalls are allowed
AND any foreign architecture or 32-bit-pointer syscall ends the process
```

| Field | Value |
|---|---|
| Discharges | E21 pure; G4/r1; D-295-R22 (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the deny-list; the exact program)) |
| Contract shape | pure-function |
| Lane | pure (x86_64 build; on other targets the module pins the unsupported return, see S-ND295-44) |
| Driving port | `VmmLaunchSeccompFilter::for_target()` + a pure classic-BPF evaluator over `program()` in the test module |
| Fault stimulus | synthetic `seccomp_data` |
| Oracle | (a) 13 requests → `ERRNO\|EPERM`, including with `args[1]` upper 32 bits set; (b) six `fd=` requests, a read-only request, and a non-`ioctl` syscall carrying a denied value → `ALLOW`; (c) foreign audit arch → `KILL_PROCESS`; (d) `nr = 0x4000_0000 + 514`, `+ 16`, any `nr >= 0x4000_0000` except `-1` → `KILL_PROCESS`, `nr = -1` → `ALLOW`; the 13 derived values equal the increment-aa numbers; composed audit value `0xC000_003E` and x32 bit pinned; `VMM_LAUNCH_DENIED_IOCTLS` equals the table; program length 24 |
| Seed / isolation | proptest over `nr` and `args[1]` (unbounded input domain) plus pinned `#[test]` rows for the table |
| Rust home | NEW `crates/overdrive-host/src/vmm/launch_seccomp.rs::tests::{the_launch_filter_verdict_partition_is_total, the_deny_list_equals_the_measured_ioctl_numbers_and_the_audit_constants}` |
| Disposition / step | NEW — 05-02 (the deny-list constant is data transcribed from the pinned table; its equality row may be GREEN at scaffold, recorded as such) |

#### S-ND295-43 — The production launch hook denies TAP-mutating requests on every thread

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a process launched through the production hook holding a TAP queue
WHEN any of its threads, including ones created after launch, issues a TAP-mutating request
THEN each request is refused as not permitted while the device-attach requests still work
AND every thread reports that it runs under the filter and can gain no privileges
AND a call through a foreign system-call interface ends the process
```

| Field | Value |
|---|---|
| Discharges | E21 Lima-root and native (f); G4/r1, G4/r5 (TUNSETOWNER refused); D-295-R22 (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the testability boundary)) |
| Contract shape | bounded-change |
| Lane | lima-kernel on an x86_64 build (CI x86_64 Lima runner) and native (f) on metal (`cargo xtask metal run -- cargo nextest run -p overdrive-host --features integration-tests -E 'test(/launch_seccomp_kernel/)'`) |
| Driving port | private `register_launch_child_hook` with the production program; the probe's `check_launch_seccomp` |
| Fault stimulus | the re-exec'd child issues the 13 requests on its queue from its main thread and three post-exec threads; x32 `syscall(0x4000_0000 + 514, …)`; i386 `int 0x80` `ioctl` where the kernel provides the i386 entry |
| Oracle | 13 × 4 `EPERM`; none of the six `fd=` requests returns `EPERM`; every `/proc/self/task/*/status` shows `NoNewPrivs: 1`, `Seccomp: 2`; descriptor table exactly 0-3; `check_launch_seccomp` returns `Ok(())`; x32 and i386 children end with `SIGSYS` |
| Seed / isolation | example; `#[cfg(all(test, feature = "integration-tests"))] #[allow(unsafe_code)] mod launch_seccomp_kernel`, every case `#[cfg(target_arch = "x86_64")]`; scratch TAP outside `ovd-tp-`, not bridged; no listed shared resource, so no `host-kernel-shared` assignment is required; CI lane selector per § *Prerequisites* in the feature delta |
| Rust home | NEW `crates/overdrive-host/src/vmm.rs::launch_seccomp_kernel::{every_denied_request_returns_eperm_on_every_thread_under_the_production_hook, every_filtered_thread_reports_no_new_privs_and_filter_mode, the_startup_probe_installs_the_exact_launch_program, foreign_syscall_abis_end_the_filtered_process}` |
| Disposition / step | NEW — 05-02 |

#### S-ND295-44 — A target with no launch filter starts no microVM

`@error @contract-shape:bounded-change`

```gherkin
GIVEN a node built for a target that has no launch filter program
WHEN the VM driver is probed or a microVM launch is attempted
THEN the probe names the unsupported architecture and the launch is refused before any effect
AND each probe failure cause maps to its own typed launch-filter probe error
AND the probe runs its stages in the order reflink, hypervisor, prlimit, setpriv, launch filter, KVM, run root
```

| Field | Value |
|---|---|
| Discharges | E21 pure/aarch64 (ruling 10, GH #302); G4/r2; D-295-R22 (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (refusal on every other target; the `create` additions)) |
| Contract shape | pure-function (mapping, order) / bounded-change (create refusal) |
| Lane | pure + aarch64 Lima build for the refusal arm (the only authority, FD § "[REF] Required downstream changes (not edited by DESIGN)" (the `distill/test-scenarios.md` item: the aarch64 lane is the refusal cases' authority)) |
| Driving port | `VmmLaunchSeccompFilter::for_target`; `CloudHypervisorVmm::create`; `Vmm::probe` over the private `VmmProbeSubstrate` test substrate |
| Fault stimulus | the compiled target; injected substrate stage failures |
| Oracle | aarch64: `for_target()` → `LaunchSeccompUnsupportedArch { target_arch: "aarch64" }`; `create` → `VmmError::ConfinementUnavailable { control: Seccomp, detail }` naming aarch64 with no rootfs clone and no queue attached (scratch TAP attach still succeeds afterwards); `check_launch_seccomp` returns `VmmProbeError::LaunchSeccompUnsupportedArch`. All targets: the stage order and each `LaunchSeccomp*` mapping (install spawn error → `LaunchSeccompInstall`, non-success incl. `SIGSYS` → `LaunchSeccompProbeExit { exit_code, signal }`) |
| Seed / isolation | table |
| Rust home | `crates/overdrive-host/src/vmm.rs::tests::vmm_probe_preserves_stage_order_and_rejects_each_injected_ip_execution_failure` (RETARGETED: `ip` stage removed, launch-seccomp stage inserted) + NEW `…::tests::each_launch_filter_probe_cause_maps_to_its_typed_error`, NEW `…::launch_seccomp::tests::a_target_without_a_program_is_unsupported` (`#[cfg(not(target_arch = "x86_64"))]`), NEW `…::launch_seccomp_kernel::launch_on_a_target_without_a_program_is_refused_before_any_effect` (`#[cfg(not(target_arch = "x86_64"))]`) |
| Disposition / step | RETARGETED + NEW — 05-02, except the retargeted stage-order body `vmm_probe_preserves_stage_order_and_rejects_each_injected_ip_execution_failure`, which is 05-03: it asserts the `ip` stage is gone, and 05-03 removes `ip` from the launch tools (DISTILL review DR-09) |

#### S-ND295-45 — Every Cloud Hypervisor thread carries the launch filter under its own filters

`@real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN a VM launched through serve and deploy on native metal
WHEN the guest reaches READY
THEN every hypervisor thread, the leader included, reports no-new-privileges and filter mode
AND each thread carries exactly one more filter than the hypervisor installs itself
```

| Field | Value |
|---|---|
| Discharges | E21 native (e) per-thread half (traffic half is S-ND295-01); G4/r1; D-295-R22 (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the testability boundary's architecture gating); FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E21 row)) |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve::run_with_kek` + `deploy` |
| Fault stimulus | none |
| Oracle | every `/proc/<ch>/task/*/status`: `NoNewPrivs: 1`, `Seccomp: 2`; `Seccomp_filters`: leader 1, `vmm` and `http-server` 2, every other thread 3 — the audited build's counts (the increment-aa control table, measured on the audited v53.0 build), re-measured by OBL-295-SECCOMP-REVERIFY when the shipped Cloud Hypervisor build changes; no version is gated at runtime (user ruling of 2026-09-25) |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs::vm_seccomp_is_verified_per_thread_not_on_the_thread_group_leader` RETARGETED and renamed `every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters` — the old body asserts the leader reports `SECCOMP_MODE_DISABLED`, which D-295-R22 makes false |
| Disposition / step | RETARGETED — 05-03: the per-thread counts need a guest at READY, which needs the 05-03 fd handoff (DISTILL review M1; native run required in that step) |

#### S-ND295-46 — No first-party descriptor is created inheritable

`@error @contract-shape:pure-function`

```gherkin
GIVEN the source of every first-party crate linked into serve
WHEN the close-on-exec gate scans it
THEN no descriptor-creating call lacks its close-on-exec flag
AND each planted violation of every rejected call family is reported with its rule
AND a site marked with the explicit exemption comment and a reason is accepted
AND a source file the gate cannot read or parse fails the scan instead of being skipped
```

| Field | Value |
|---|---|
| Discharges | E19 (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E19 row)); obligation OBL-295-CLOEXEC (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (obligation OBL-295-CLOEXEC)) |
| Contract shape | pure-function |
| Lane | pure (`scan_source` and `render_violation` over planted sources, source-local) + xtask integration (`scan_workspace` over the real workspace and over a temporary fixture workspace; it runs `cargo metadata`, a subprocess, so it is gated behind `xtask/integration-tests` like the existing `xtask/tests/integration/` bodies) |
| Driving port | `xtask::cloexec_lint::{scan_source, scan_workspace, render_violation}` with `CloexecViolation { file, line, column, call, rule }` and `CloexecRule::{MissingFlag, AlwaysInheritable, UnresolvedFlag}`, exactly as pinned (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the gate entry point)); `run` and `Task::CloexecLint` (`cargo xtask cloexec-lint`) are the operator entry and are not called by the bodies |
| Fault stimulus | planted sources: one per row of the call-family table (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the source gate's call-family table)), plus an `fcntl` with `F_DUPFD`, an `epoll_create`, a `recvmsg` without `MSG_CMSG_CLOEXEC`, a `use libc::socket as s; s(..)` rename, a `nix` and a `rustix` wrapper with and without its flag, a wrapper with no flags argument, a flag held in a variable, a `// cloexec-lint: ok <reason>` marker on the call line, one on the line above, and one two lines above, and the same violation inside a `#[cfg(test)]` item; the compound-`cfg` planted source (DISTILL review DR-10; FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the test-only items rule, pinned 2026-09-29)): the same violation under `#[cfg(all(test, feature = …))]` followed by another attribute, `#[cfg(all(feature = …, test))]`, a nested `all`, `#[cfg(test)]` after another `cfg`, and under `#[cfg(any(test, …))]`, `#[cfg(not(test))]`, and `#[cfg_attr(test, …)]`; a source that does not parse; a fixture workspace (temporary directory) whose `overdrive-cli` package has one unparseable `src/**/*.rs` file, one with a violation under `src/bin/`, and one under the crate-root `bin/`; a fixture workspace with no `overdrive-cli` package |
| Oracle | `scan_source`: the exact `(line, column, call, rule)` set per planted source in source order, `call` spelled `<root crate>::<final segment>` after the rename (`libc::socket`); `MissingFlag` for a present-but-unflagged argument, `AlwaysInheritable` for `accept`/`pipe`/`dup`/`dup2`/`inotify_init`/`epoll_create`/`F_DUPFD` and flagless wrappers, `UnresolvedFlag` for the variable; the marker suppresses only its own line (same line or the line immediately above; two lines above does not); the `#[cfg(test)]` violation is not reported, and of the compound-`cfg` rows only those whose predicate requires `test` (`all(…)` containing `test`, directly or nested, and `#[cfg(test)]` beside another `cfg`) are exempt, while the plain, `any`, `not`, and `cfg_attr` rows are reported at their exact lines; an unparseable source is `Err`, never `Ok(vec![])`. `render_violation` names the file, line, column, call, and rule. `scan_workspace`: over the real workspace `Ok(vec![])` once the eight site fixes land (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the OBL-295-CLOEXEC eight-site table)); over the fixture workspace `Err` naming the unparseable file (fail-closed, unlike `dst_lint`), the `src/bin/` and crate-root `bin/` violations not reported; with no `overdrive-cli` package `Err` |
| Seed / isolation | finite tables; fixture workspaces in `tempfile::TempDir` |
| Rust home | NEW `xtask/src/cloexec_lint.rs::tests::{every_rejected_call_family_is_reported_with_its_rule, renamed_imports_and_nix_or_rustix_wrappers_are_resolved_to_their_call, an_unresolved_flag_argument_is_rejected, the_exemption_marker_suppresses_only_its_own_line, cfg_test_items_are_not_scanned, only_a_cfg_predicate_that_requires_test_exempts_an_item, an_unparseable_source_is_an_error_not_a_clean_file, a_rendered_violation_names_its_site_call_and_rule}`; NEW `xtask/tests/integration/cloexec_lint_workspace.rs::{the_serve_closure_creates_no_inheritable_descriptor, an_unparseable_serve_source_fails_the_scan_instead_of_being_skipped, auxiliary_binaries_outside_the_serve_closure_are_not_scanned, a_workspace_without_the_cli_package_is_an_error}` (registered in `xtask/tests/integration.rs`) |
| Disposition / step | NEW — 05-04. The pure bodies are RED at the phase-B `todo!` scaffold of the pinned module; the real-workspace body is RED until the eight site fixes land |

### Group B — Guest attachment lifecycle (R4, R21, R22 mask, M2, H1)

#### S-ND295-00 — The node refuses work when its shared-network proof is incomplete

`@driving_port @error @real-io @contract-shape:bounded-change`

```gherkin
GIVEN the operator starts a node whose isolated shared-network proof meets one accepted fault
WHEN the node validates its network substrate before admitting workloads
THEN startup refuses with an actionable shared-network cause and admits nothing
AND every scratch resource is removed before the refusal is reported
```

| Field | Value |
|---|---|
| Discharges | G1/r2, G1/r3; D-295-DISTILL-5/12/14/14A (unchanged by the replacement except the R21 inventory note) |
| Contract shape | bounded-change |
| Lane | pure (dataplane projection/validator/owner-order tables) + lima-kernel (ordinary boot) |
| Driving port | `run_server` boot; private D5/D14A owner algorithm |
| Oracle | unchanged (FD § "D-295-DISTILL-14 — source-honest startup packet-probe boundary"; FD § "D-295-DISTILL-14A — private semantic validation and deterministic boot observation"); the scratch probe still attaches only the ingress classifier and its eight-counter probe array (FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the counter slot)) |
| Rust home | dataplane `guest_tcx::tests::{startup_tcp_probe_projects_semantics_and_all_eight_counter_pairs_without_raw_abi, every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics, wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less, capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source, a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count, every_receipted_family_returns_one_and_clean_families_return_exact_zero, bpf_test_run_attribute_ends_in_an_explicit_zeroed_tail}` (the last is NEW and active: it pins the layout of the `BPF_PROG_TEST_RUN` attribute the startup probe passes — `batch_size` at offset 72, the explicit zeroed `_pad` at 76, size 80 — the production defect `e496722c` fixed in DISTILL phase B, whose uninitialized 4-byte tail made the probe's `BPF_PROG_TEST_RUN` return `EINVAL` and refuse boot at random; DISTILL review DR-05); control-plane `guest_network::scratch_probe_acceptance::{exercise_transport_and_semantic_failures_are_distinct_for_every_probe_stage, every_cleanup_or_inventory_failure_is_aggregated_after_the_remaining_cleanup, every_observed_residue_family_returns_incomplete_with_the_owner_built_complement, primary_and_first_cleanup_failure_are_both_preserved_without_nested_aggregate}`, `guest_network::scratch_probe_packet_acceptance::{every_d14_semantic_mismatch_and_lower_source_reaches_the_production_validator, classifier_runs_precede_close_and_each_stage_is_fresh}`; integration `shared_guest_network_startup::{ordinary_probe_faults_refuse_before_convergence_or_publication, cleanup_failure_refuses_and_preserves_primary_cleanup_and_observed_residue, production_host_owner_boots_only_after_real_shared_identity_is_exact, production_startup_exercises_classifier_and_detached_guard_before_admission}` |
| Disposition / step | RETAINED, but the body is split per the fresh-host RCA (`docs/analysis/root-cause-analysis-netns295-fresh-bridge-boot-refusal.md`): the **bridge-identity leg** (`production_host_owner_boots_only_after_real_shared_identity_is_exact`, repeated fresh-host boots) is 05-00's activation evidence (E22 (c), S-ND295-72), and the **DNS leg** is re-authored as `the_shared_gateway_answers_an_absent_mesh_name_with_nxdomain` — the host DNS factory through the required `guest_dns` port and an in-zone (`nd295-absent.svc.overdrive.local`) NXDOMAIN+SOA oracle — pending 05-01, because today's `dataplane_override` composes no responder and `missing.` is out of the responder's scope (RCA root cause B). The two ignored superseded bodies `scratch_probe_acceptance::{healthy_probe_uses_the_exact_setup_probe_cleanup_and_inventory_order, every_setup_or_probe_failure_preserves_primary_and_still_runs_complete_cleanup}` are DELETED (superseded by D14A). The real-kernel inventory layer is S-ND295-48. The retained active body `production_startup_exercises_classifier_and_detached_guard_before_admission` boots the same production bridge, so on a host with no `ovd-gbr0` it meets the same fresh-host race (RCA root cause A) until 05-00 lands; that failure is a baseline, not a #295 RED (`red-classification.md` Phase G). |

#### S-ND295-06 — Start publishes no partial attachment

`@property @in-memory @error @contract-shape:bounded-change`

```gherkin
GIVEN the production action owner has admitted one guest lease
WHEN provisioning the attachment fails through its driven port
THEN the typed owner cause reaches the dispatch error
AND the VMM is not started and no guest command is released
AND the lease stays held until cleanup completes
WHEN instead the VMM starts and the allocation's first Running record is rejected, on a fresh start or a restart
THEN the VMM is stopped, the lease is retiring before the attachment's teardown, and the lease is released last
```

| Field | Value |
|---|---|
| Discharges | E5 seeded (start-failure ordering releases last); G2/r2; D-295-R7 |
| Contract shape | bounded-change |
| Lane | pure (control-plane acceptance over `SimSharedGuestNetworkOwner`; the refusal precedes every intercept install, so the fixture leaves the worker's shared owner unstarted and binds no socket, § *Seam fixture*) + in-process (the rejected Running write, control-plane integration binary over the same seam and one shared owner) |
| Driving port | `action_shim::dispatch_with_guest_network_provisioner_for_test(actions, state, tick, provisioner)` — the accepted C-295-B signature; the EXEC gate and the pool are read from `state` (FD § "C-295-B — network provisioner boundary" (the helpers read the EXEC gate and the pool from `state`); FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (how the gate reaches the shim)) |
| Fault stimulus | `SimSharedGuestNetworkOwner::script_provision_failure(true)` on the one owner instance the fixture passes to `AppState` and to the seam; separately, the observation store rejects the allocation's initial Running write after provision and driver start, on the fresh-start arm and on the restart arm |
| Oracle | unchanged; plus, from the pinned lease events (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the lease events)) captured by a test-local tracing Layer that stamps each event with the owner's `calls().len()`: `guest_network.lease_retired { alloc }` precedes the `TapDelete` teardown call and `guest_network.lease_released { alloc }` follows its success (the pool's operations are crate-private, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the control-plane-private pool operations), so an external body observes lease state only through these events). Rejected Running write: the journal is `Provision`, `DriverStart`, then exactly one each of `DriverStop`, `LeaseRetired`, `Teardown`, `LeaseReleased`, with the driver stop and the retirement before the teardown, the teardown before the release, and the release last |
| Rust home | `crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs::provision_refusal_stops_before_driver_start_and_preserves_the_typed_owner_cause`; NEW `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs::{start_running_write_rejection_retires_the_lease_before_teardown_and_releases_it_last, restart_running_write_rejection_retires_the_lease_before_teardown_and_releases_it_last}` (the lease contract of a rejected Running write is 06-03's; DISTILL review M7). Their structural siblings `…::{start,restart}_running_write_rejection_tears_down_network_and_releases_slot` stay active pre-#295 bodies (one start, one provision, one teardown, the slot released; DISTILL review DR-13) |
| Disposition / step | RETARGETED (seam fixture; retire-then-release event assertion) + NEW — 06-03 |

#### S-ND295-08 / S-ND295-09 — Ingress classification: valid traffic enters the protected path once; malformed or impersonated traffic cannot escape

`@property @tier2 @real-io @error @contract-shape:bounded-change`

Contracts unchanged (C-295-0, ADR-0115). The shared `COUNTERS` array grows to
nine slots (FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the counter slot)); the ingress program never touches slot 8.

| Field | Value |
|---|---|
| Discharges | G2/r1 (classification feeds the protected path) |
| Lane | tier2 |
| Rust home | `crates/overdrive-bpf/tests/integration/guest_tcx_classifier_test_run.rs::classifier_partitions_return_one_verdict_and_advance_one_exact_counter` (the eight-slot partition, today's array); NEW `…::ingress_partitions_leave_the_ninth_egress_slot_untouched`: the array has nine slots, and for every ingress row exactly the row's own slot reads 1 and slot 8 (`EgressDestinationDrop`) reads 0 — authored now, so DELIVER 06-01 edits no test body (DISTILL review M14) |
| Disposition / step | RETAINED — active (eight-slot body); NEW — 06-01 (nine-slot body) |

#### S-ND295-47 — A TAP delivers unicast only to its registered guest

`@property @tier2 @error @contract-shape:bounded-change`

```gherkin
GIVEN a managed TAP whose registered guest MAC is known to the node
WHEN frames leave the host towards that guest
THEN broadcast and multicast are always delivered
AND unicast is delivered only when addressed to the registered guest MAC
AND unicast to any other MAC and unicast when the TAP has no registration are dropped and counted once
```

The verdict table exercises no sub-Ethernet-header frame (item 9): the kernel
rejects a `BPF_PROG_TEST_RUN` skb input below `ETH_HLEN` with `EINVAL` before
the classifier runs, so asserting that refusal would prove the harness's own
bounds check, not the program's. The program's own bounds check is the
verifier-enforced guarantee (the loader rejects an out-of-bounds access at
load time), so it needs no runtime row here.

| Field | Value |
|---|---|
| Discharges | E12 (h) Tier-2 partition (FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the evidence lane's Tier-2 verdict partition)); D-295-R21 verdict table (FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the total egress verdict table)) |
| Contract shape | bounded-change |
| Lane | tier2 |
| Driving port | the embedded `gh295c_egress` classifier via `BPF_PROG_TEST_RUN`, `__sk_buff.ifindex` set to the keyed TAP |
| Fault stimulus | `ENDPOINTS` entry present/absent; destination = registered, foreign unicast, broadcast, multicast |
| Oracle | per row: verdict (`TC_ACT_OK`/`TC_ACT_SHOT`) and `EgressDestinationDrop` (slot 8) delta exactly 0 or 1; slots 0-7 unchanged |
| Seed / isolation | finite table; map state cleared per row (Tier-2 default) |
| Rust home | NEW `crates/overdrive-bpf/tests/integration/guest_tcx_classifier_test_run.rs::egress_classifier_delivers_only_registered_unicast_and_every_group_frame` |
| Disposition / step | NEW — 06-01 |

#### S-ND295-48 — The egress classifier is loaded, attached, pinned, counted, and inventoried like its ingress sibling

`@real-io @adapter-integration @error @contract-shape:bounded-change`

```gherkin
GIVEN the node's TCX program set is loaded
WHEN the owner attaches the egress guest-MAC classifier to a TAP and later detaches it
THEN the egress link reports the egress program at the TAP's egress attach point and pins beside the ingress link
AND the ninth counter is readable as the egress drop counter
AND the ownership inventory counts both receipted programs and still reports zero after cleanup
```

| Field | Value |
|---|---|
| Discharges | E12 (g) prerequisite; D-295-R21 dataplane contract (FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the exact implementation-facing contract)) |
| Contract shape | bounded-change |
| Lane | pure (counter vocabulary, object/error projection) + lima-kernel (attach/pin/query/detach, inventory) |
| Driving port | `GuestTcxProgram::attach_first_egress(interface)`, `GuestTcxLink::pin/detach`, `query_attachment(interface, TcxAttachPoint::Egress)`, `read_counter(pin, GuestTcxCounter::EgressDestinationDrop)` |
| Fault stimulus | real unpin/detach of the egress link, then an absence read-back |
| Oracle | counter vocabulary maps `EgressDestinationDrop` → private index 8, `CounterSlots` = nine, 8 → `Unsupported`; egress link `program_id()` equals the loaded egress id; pin path `links/<tap>-egress`; query returns exactly that program at egress; detach empties it; inventory reports up to two receipted programs and exact zero after cleanup |
| Seed / isolation | table + example; scratch TAP outside `ovd-tp-`; `overdrive-dataplane` integration binary is `host-kernel-shared` |
| Rust home | `crates/overdrive-dataplane/src/guest_tcx.rs::tests::semantic_counter_vocabulary_maps_to_the_exact_private_array_slots` (RETARGETED to nine); NEW `crates/overdrive-dataplane/tests/integration/guest_tcx_egress_lifecycle.rs::the_egress_classifier_attaches_pins_queries_and_detaches_at_the_egress_point`; `crates/overdrive-dataplane/tests/integration/guest_tcx_inventory.rs::{clean_and_receipted_inventory_observes_all_eight_exact_families, retained_unpinned_maps_programs_and_links_survive_handle_release_and_remain_observable, wrong_exact_path_owner_or_valid_map_schema_is_typed_and_never_fabricates_zero}` (placeholder `panic!` bodies AUTHORED as complete bodies; the program family now counts two receipted programs) |
| Disposition / step | RETARGETED + NEW + AUTHORED — 06-01. The complete body `guest_tcx::tests::absent_real_objects_preserve_the_operation_specific_source_family` performs real bpffs/TCX syscalls behind `integration-tests` in the same source-local module and is **active**: the absent-object source families are behaviour that exists today (it passes, `red-classification.md` Phase G). Its absent interface is a name under `IFNAMSIZ` confirmed absent first, and its egress row, which fails at map open whatever the slot, is not counted as R21 evidence (DISTILL review L4) |

#### S-ND295-49 — The platform can read every TAP's debug message level

`@real-io @adapter-integration @error @contract-shape:bounded-change`

```gherkin
GIVEN a freshly created TAP
WHEN the platform reads its debug message level, singly or in one dump of every device
THEN a fresh TAP reads zero and a changed level reads back exactly
AND a vanished device and a failed exchange are reported with their original cause, never as zero
```

| Field | Value |
|---|---|
| Discharges | E12 (g) debug-mask prerequisite; D-295-R22 read-back (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the audit read-back of the TAP debug message mask)) |
| Contract shape | bounded-change |
| Lane | pure (pinned constants only; the encoder and decoder are private and unpinned) + lima-kernel (the exchanges, the dump keying, and the error operation labels) |
| Driving port | `overdrive_netlink::ethtool::{debug_msg_mask(iface), debug_msg_masks()}` (plain `pub`) |
| Fault stimulus | a real `TUNSETDEBUG` on the scratch TAP's queue (test-only raw ioctl with a `SAFETY` comment); the queue is opened by a test-local raw `TUNSETIFF` (`IFF_TAP \| IFF_NO_PI \| IFF_VNET_HDR`), not the 05-03 `attach_tap_queue`, so the body needs nothing from 05-03 (DISTILL review DR-01); an absent interface (`ENODEV`) |
| Oracle | pure: each pinned constant equals its UAPI value (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the debug-mask read-back's pinned constants)); Lima: fresh TAP → `Ok(0)`; after `TUNSETDEBUG(n)` → `Ok(n)` and the dump maps the TAP's ifindex to `n`; absent → `Err` carrying `ENODEV` under operation `"debug-get"` (FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (the debug-mask read-back's errors)), never `Ok(0)` |
| Seed / isolation | table + example; scratch TAP outside `ovd-tp-`; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-netlink/src/ethtool.rs::tests::debug_message_constants_equal_the_uapi_values`; NEW `crates/overdrive-netlink/tests/integration/tap_debug_msg_mask.rs::{a_fresh_tap_reads_zero_and_a_changed_level_reads_back_singly_and_in_the_dump, an_absent_device_is_reported_with_its_original_cause}` |
| Disposition / step | NEW — 06-01 |

#### S-ND295-11 — A workload is admitted only after its complete attachment is read back down

`@real-io @adapter-integration @error @contract-shape:bounded-change`

```gherkin
GIVEN the production shared-switch owner and one admitted lease
WHEN it provisions the attachment
THEN the TAP exists, is persistent, is owned by root, carries debug level zero, and stays down
AND its bridge master, guard membership, endpoint entry, ingress program and pin, and egress program and pin all read back exactly
AND the TAP's host-side MAC is present and is neither the bridge's address nor the guest MAC of a VM the node still holds, its own included, and nothing about it is recorded
AND no step raises the TAP, and any incompatible identity refuses without publishing the allocation
```

| Field | Value |
|---|---|
| Discharges | E1 in-process (provision-down); E4 in-process (owner `Some(0)`); G3/r1 precondition; D-295-R4, R5, R21, R22 (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `provision` read-backs); FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the allocation step order)) |
| Contract shape | bounded-change |
| Lane | pure (source-local D12A tables) + lima-kernel (ordinary production composition) |
| Driving port | `GuestNetworkProvisioner::provision` on the private host owner through D12A leaves (source-local); `serve` → action shim → `provision` with an injected recording VMM (Lima) |
| Fault stimulus | scripted leaf observations (every TAP/bridge identity partition; egress attach/pin/query failures; mask `Some(n≠0)`, `None`, sourced error) |
| Oracle | exact D12A call order with egress as step 6 and, as step 7, the down read-back that checks the host-side MAC invariant (D-295-R21; nothing is recorded — a reserved or missing address refuses publication with `TapObserve` over `TapHostMac`, S-ND295-72 (i1)); expected TAP fact `owner_uid: Some(0)`, `up: false`; every failure returns its operation-tagged error (`TcxEgress*`, `TapObserve` with `TapDebugMsgMask`) and publishes nothing; Lima: the recording VMM sees no start until the real kernel shows the complete attachment down, and the same ifindex remains down through the injected VMM's READY |
| Seed / isolation | table + example; control-plane integration binary is `host-kernel-shared` |
| Rust home | `crates/overdrive-control-plane/src/guest_network.rs::allocation_owner_acceptance::{provision_reads_every_attachment_fact_before_reporting_success, every_incompatible_tap_or_bridge_identity_refuses_owner_publication, rollback_retry_skips_attachment_query_after_tap_removal}` (RETARGETED: owner 0, egress step, mask, host-side MAC invariant, egress rollback state; the reserved-address refusal table is S-ND295-72's `a_reserved_or_missing_host_side_address_refuses_publication`), `…::early_provision_failure_without_tap_skips_attachment_query` (RETAINED); NEW `…::every_egress_and_debug_mask_provision_failure_refuses_publication`; `crates/overdrive-netlink/src/client.rs::tests::persistent_tap_and_bridge_projection_preserves_every_observable_identity_field` (RETAINED); `crates/overdrive-control-plane/tests/integration/shared_guest_network_startup.rs::ordinary_provision_reads_back_the_complete_attachment_down_before_injected_vmm_start` (placeholder AUTHORED) |
| Disposition / step | RETARGETED + NEW + AUTHORED — 06-02 |

#### S-ND295-12 — Teardown leaves nothing behind and converges on parts already gone

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN one active attachment beside an unrelated one
WHEN the named attachment is torn down, with some of its parts already removed out of band
THEN the endpoint, both links and pins, the TAP (set down before deletion), and the guard member end absent
AND a part that is already absent counts as removed without a write
AND a genuine deletion failure keeps its typed cause and the lease stays held for retry
AND the unrelated attachment is unchanged
```

| Field | Value |
|---|---|
| Discharges | E5 in-process + native (S-ND295-66); G2/r2 (retry-retaining); D-295-R5 teardown (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (teardown converges on absence)), R21 (egress detach after ingress) |
| Contract shape | bounded-change |
| Lane | pure (source-local D12A tables) + lima-kernel |
| Driving port | `GuestNetworkProvisioner::teardown` |
| Fault stimulus | scripted absent parts (each of TAP, ingress/egress attachment, ingress/egress pin, endpoint entry, guard member; singly and all together); a non-absence leaf failure; Lima: out-of-band removal of one part before the production stop |
| Oracle | order endpoint delete → ingress unpin/detach → egress unpin/detach → TAP down + read-back → `RTM_DELLINK` → guard member delete → complement read-back; absent TAP skips set-down, read-back, and delete; the final complement alone decides success; unrelated attachment byte-equal (both read-backs must succeed: a failed read never compares equal) and the successor's TAP present; the lease release is the shim's, not the owner's: with both leases held as the precondition, the owner's teardown leaves the pool unchanged and the body performs no release of its own (DISTILL review M9) |
| Seed / isolation | table + example; `host-kernel-shared` |
| Rust home | `crates/overdrive-control-plane/src/guest_network.rs::allocation_owner_acceptance::every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement` (RETARGETED); NEW `…::teardown_converges_on_every_absent_part_singly_and_together`; `crates/overdrive-control-plane/tests/integration/shared_guest_network_startup.rs::two_attachment_teardown_releases_last_and_preserves_the_unrelated_attachment_byte_equal` (placeholder AUTHORED) |
| Disposition / step | RETARGETED + NEW + AUTHORED — 06-02 |

#### S-ND295-50 — The owner's audit tells node failures apart from one VM's damaged parts

`@property @error @contract-shape:bounded-change`

```gherkin
GIVEN a shared guest network with several active attachments
WHEN the owner audits the node
THEN any failing node-level part is reported as its exact component before any per-VM finding
AND when every node-level part is healthy, each attachment whose own parts are damaged is named with its first failing check
AND a VM already condemned is never checked or named again
AND the audit changes nothing
```

| Field | Value |
|---|---|
| Discharges | E11 prerequisite, E12 (g) owner half; G5/r2, G5/r3; D-295-R14/H1 (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (node-level versus per-allocation parts; the `audit_shared` contract and when `Condemned` takes effect)), R21 (the host-side MAC invariant, egress parts), R22 (mask 0; failed dump is `Bridge`, FD § "[REF] Driven port — VMM launch seccomp filter (D-295-R22) — ACCEPTED 2026-09-24" (a failed audit dump is a node-level failure)) |
| Contract shape | bounded-change |
| Lane | pure (source-local over scripted D12A leaves and scratch I/O) |
| Driving port | `SharedGuestNetworkOwner::audit_shared` |
| Fault stimulus | per node-level part: bridge identity, guard table/chains/rules, a guard member naming an unmanaged TAP, TCX program, map identity, map pins, an unmanaged endpoint entry, a failed debug-mask dump; per allocation (the node-level table's recovery audit uses the reserved host-side MAC): TAP deleted, non-persistent, owner changed, host-side MAC reserved (the TAP's own guest MAC) or missing, mask non-zero, ifindex changed, master lost, admin state versus phase, ingress/egress attachment detached, ingress/egress pin removed, endpoint value changed, guard member removed |
| Oracle | node-level → `Err(SharedGuestNetworkAuditError { component, source })` with the matrix component (guard → `BridgeGuard`, pins → `BpffsPin`, dump → `Bridge`); per-allocation → `Ok(SharedGuestNetworkAudit { damaged: {alloc: first failing check} })` with `TapHostMac { address: Reserved(<mac>) | Missing }` / `TapDebugMsgMask` / `TcxEgressQuery` / `TcxEgressLinkPin` facts (an unreserved host-side MAC is not damage, S-ND295-72 (i2)); a reported allocation is excluded from every later audit and restore; no mutating leaf is called |
| Seed / isolation | finite table |
| Rust home | NEW `crates/overdrive-control-plane/src/guest_network.rs::allocation_owner_acceptance::{every_node_level_audit_failure_names_its_matrix_component_first, every_per_allocation_damage_is_named_only_when_the_node_is_healthy, a_condemned_allocation_leaves_every_later_audit_and_restore_universe}` |
| Disposition / step | NEW — 06-02 (`a_condemned_allocation_leaves_every_later_audit_and_restore_universe` at 06-04, where condemnation lands with R5) |

#### S-ND295-10 — A removed ingress link is still blocked by the guard and condemns only that VM

`@real-io @error @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN a managed TAP that remains in the bridge guard with a healthy endpoint entry
WHEN an external actor detaches its ingress link and a valid guest frame is sent
THEN the unmarked frame is counted by the guard and dropped, reaching neither another guest nor the host
AND the node stops only that VM, naming its missing ingress attachment, not a node failure, and keeps admitting work
```

| Field | Value |
|---|---|
| Discharges | E12 (g) Lima half; G5/r3; D-295-R14/H1 |
| Contract shape | bounded-change |
| Lane | lima-kernel |
| Driving port | ordinary `run_server` composition provisions the attachment; the production supervisor runs the owner's audit and kills the damaged VM (D-295-R14) |
| Fault stimulus | D6 `query_attachment` then `detach_pinned_link` on the production pin (external mutation only) |
| Oracle | positive witnesses before the verdict: a host broadcast is seen on the peer TAP and a peer-guest frame on the host capture (both captures fail loudly on a read error); D9 guard unchanged with `DefaultDrop` +1; peer-TAP and host captures see no escaped frame; exactly one `guest_network.shared_owner_vm_killed { alloc: <victim>, cause: "attachment_damaged" }` whose error names a `TcxQuery`/`TcxLinkPin` fact; no serve shutdown request within 2 s and no `unhealthy` event; a later deploy is released its command and the peer stays Running |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | `crates/overdrive-control-plane/tests/integration/shared_guest_network_startup.rs::deliberate_link_loss_reaches_default_drop_and_the_exact_production_audit_cause` (placeholder AUTHORED; the oracle changes from node-level `TcxLink` to one per-allocation kill) |
| Disposition / step | RETARGETED + AUTHORED — 09-01: the per-allocation kill is the supervisor's, so the body observes the supervisor's event rather than calling `audit_shared` itself (DISTILL review H1, DR-09) |

### Group C — Node-wide admission (gap 1; R6, R7, R8)

#### S-ND295-02 / S-ND295-03 — A guest network assignment is complete or absent; every admitted guest gets one collision-free lease

`@property @contract-shape:pure-function`

Contracts unchanged. Rust homes: the grouped-handoff projection bodies and
`crates/overdrive-control-plane/src/guest_network.rs::pool_acceptance::assignment_uses_the_exact_prefix_bridge_gateway_dns_and_boundary_addresses`.
Disposition: RETAINED — active (pool construction becomes per instance;
mechanical, 06-03).

#### S-ND295-04 — Lease replay, retirement, and release change only the named allocation

`@property @error @contract-shape:bounded-change`

```gherkin
GIVEN any held lease map with Admitted and Retiring leases
WHEN an allocation is assigned again, retired, retired again, released, or released while absent
THEN an Admitted replay returns the byte-equal plan and a Retiring replay is refused as retiring
AND retirement happens once, never returns to Admitted, and the lease still counts
AND release removes the lease in either state and only then frees its address
AND every other lease is unchanged
```

| Field | Value |
|---|---|
| Discharges | E6 in-process pure properties; G0/r1, G0/r4; D-295-R6, R7 (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the pool operations)) |
| Contract shape | bounded-change |
| Lane | pure (proptest over operation sequences against a model) |
| Driving port | `GuestAddressPool::{assign, retire, release, snapshot, observe}` (crate-private, source-local) |
| Fault stimulus | generated sequences |
| Oracle | model equality after every step: smallest-free selection, `LeaseRetiring { alloc }`, `retire` true only for Admitted→Retiring, `observe` reports `held = Admitted + Retiring` and `retiring` exactly, `snapshot` includes both states |
| Seed / isolation | proptest (seed printed on failure, `PROPTEST_REPLAY`) |
| Rust home | `crates/overdrive-control-plane/src/guest_network.rs::pool_acceptance::assignment_replay_release_and_reuse_match_the_smallest_free_model` (RETARGETED to the lease-state model); NEW `…::pool_acceptance::retirement_is_monotonic_and_a_retiring_lease_still_counts` |
| Disposition / step | RETARGETED + NEW — 06-03. `pool_acceptance::slash_16_exhaustion_is_pool_drift_and_does_not_reuse_an_address` is DELETED: under R6 the cap refuses at 16,384 held, so `/16` exhaustion through `assign` is unreachable (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (`assign`: `PoolExhausted` is unreachable below the cap)); `below_cap_pool_exhaustion_is_typed_drift_and_preserves_state` (small constructed prefix) is RETAINED as the drift witness |

#### S-ND295-05A — Admission refuses at the cap over held leases, one pool per server

`@property @boundary @error @contract-shape:bounded-change`

```gherkin
GIVEN a node whose held leases are 16,383, 16,384, or 16,385 across every mix of Admitted and Retiring
WHEN one more allocation asks for a lease
THEN below the cap it is assigned, and at or above the cap it is refused naming held, retiring, and the cap, with no change
AND a server restarted after being killed starts with no leases
```

| Field | Value |
|---|---|
| Discharges | E6 in-process, E7 pool (`PoolExhausted` unreachable below the cap); G0/r1, G0/r2, G0/r5; D-295-R6, R7 |
| Contract shape | bounded-change |
| Lane | pure (proptest over held/retiring mixes around the boundary) + in-process (killed-mode restart) |
| Driving port | `GuestAddressPool::assign` (source-local); in-process: `run_server_with_obs_and_driver(ServerConfig::new(kek, mtls_intercept, guest_dns), obs, driver, vm_host_state, shared_guest_network, guest_network_exec, vm_cgroups)` (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)) with `SimGuestDnsFactory`, a `SimDriver`, one `SimSharedGuestNetworkOwner`, and `vm_cgroups = CgroupManager::new(<root>, Arc::new(SimCgroupFs::new()))`; `ServerHandle::kill_for_test`, then a second `run_server_with_obs_and_driver` on the same roots, then a deploy through the public handler |
| Fault stimulus | generated mixes; killed-mode restart |
| Oracle | `AdmissionCapReached { held, retiring, cap: 16_384 }` with state unchanged; the first assignment after the killed restart is the smallest free address (`100.95.0.2`), read from the guest-network assignment in the spec the `SimDriver` received (`SimDriver::started_specs()`, the C-295-A handoff) |
| Seed / isolation | proptest; in-process body in the `host-kernel-shared` control-plane integration binary |
| Rust home | NEW `crates/overdrive-control-plane/src/guest_network.rs::pool_acceptance::admission_refuses_at_the_cap_over_held_leases_for_every_retiring_mix`; NEW `crates/overdrive-control-plane/tests/integration/guest_attachment_pool_per_server.rs::a_killed_restart_starts_with_an_empty_pool` |
| Disposition / step | NEW — 06-03 |

#### S-ND295-05E — A refused admission writes nothing and says how full the node is

`@error @contract-shape:bounded-change`

```gherkin
GIVEN a node at the cap
WHEN a start or a restart successor is dispatched
THEN it is refused before any network, VMM, or protection effect
AND no allocation row, lifecycle event, teardown, or restart budget is consumed
AND an operator-visible refusal event names the allocation, held, retiring, and the cap
AND a refused restart successor still runs its predecessor's one cleanup attempt
```

| Field | Value |
|---|---|
| Discharges | E7; G0/r2; D-295-R6, R7 refusal projection (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the admission refusal projection)) |
| Contract shape | bounded-change |
| Lane | pure, source-local. The pool's operations are crate-private (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the control-plane-private pool operations)), so the cap can be reached by the pool's own `assign` only inside `overdrive-control-plane`; a source-local body therefore uses the test-local owner (§ *Test-local control-plane ports*). The refusal precedes every intercept install, so the fixture's worker (over `SimMtlsEnforcement`, `SimMtlsResolve`, `SimMtlsIntercept`) stays unstarted and binds no socket |
| Driving port | `dispatch_with_guest_network_provisioner_for_test(actions, state, tick, provisioner)` with `StartAllocation` / `RestartAllocation`, over a source-local seam fixture (§ *Seam fixture*): `SimDriver`, `SimObservationStore`, the test-local owner passed to `AppState` and to the seam |
| Fault stimulus | the fixture's pool reaches 16,384 held through its own `assign`, one of them moved to Retiring through `retire`; for the restart case the predecessor holds one of the Admitted leases and its Failed row is seeded as the restart arm's prior-row precondition (precedent `mtls_install_fail_closed.rs:676-713`) |
| Oracle | `ShimError::GuestNetwork(AdmissionCapReached { held: 16_384, retiring: 1, cap: 16_384 })`; the test-local owner's journal has no entry for the refused allocation; `SimDriver::started_specs()` unchanged; no row and no lifecycle event for it; `guest_network.admission_refused { alloc, held: 16_384, retiring: 1, cap: 16_384 }` captured once; restart: the predecessor's `lease_retired`, its `Teardown` journal entry, then its `lease_released`, exactly once each |
| Seed / isolation | example |
| Rust home | NEW `crates/overdrive-control-plane/src/action_shim/mod.rs::admission_refusal_acceptance::{admission_refusal_writes_nothing_and_reports_held_and_retiring, a_refused_restart_successor_still_cleans_up_its_predecessor_once}` |
| Disposition / step | NEW — 06-03 |

#### S-ND295-05B — Placement reads the node's held attachments, not the workload's rows

`@property @boundary @contract-shape:pure-function`

```gherkin
GIVEN a node whose held attachments are just below, at, or above the cap
WHEN placement evaluates a workload with any count of its own Running rows
THEN it refuses with no capacity exactly when held attachments reach the cap
AND CPU and memory checks are unchanged
AND the read-port returns occupancy and the requested leases from one snapshot of the server's pool
```

| Field | Value |
|---|---|
| Discharges | E6, E7; G0/r3; D-295-R8 (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the core read-port contract; placement: `schedule`)) |
| Contract shape | pure-function (placement) / bounded-change (read-port equivalence) |
| Lane | pure |
| Driving port | `overdrive_core::scheduler::schedule(nodes, needed, current_allocs, guest_attachments)`; `GuestAddressPool::observe`, source-local (the pool's operations are crate-private, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the control-plane-private pool operations); the production `GuestAttachmentView` is a private delegate over `state.guest_pool`, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the `HydrationContext` field; one pool per server: the production `GuestAttachmentView`), and is exercised end to end through runtime hydration in S-ND295-05D) |
| Fault stimulus | none |
| Oracle | `NoCapacity` iff `held >= MAX_GUEST_NETWORK_ATTACHMENTS`, independent of `current_allocs`; CPU/memory outcomes identical to the pre-change table; `observe(allocs)` returns exactly the held and retiring counts of the whole map and the lease of each requested allocation that holds one, omits requested allocations without a lease and every unrequested allocation, and leaves the pool unchanged (`snapshot()` equal before and after) — the Contract Shape the DESIGN assigns to the read-port (FD § "Effect isolation and Contract Shape classification" (the `GuestAttachmentView` row)) |
| Seed / isolation | proptest over `held` and row counts; proptest over lease maps and request sets |
| Rust home | `crates/overdrive-core/tests/acceptance/netns_density_placement_cap.rs::fixed_attachment_cap_returns_no_capacity_before_pool_assignment` RETARGETED and renamed `placement_refuses_exactly_when_held_attachments_reach_the_cap`; NEW `crates/overdrive-control-plane/src/guest_network.rs::pool_acceptance::observe_reads_occupancy_and_requested_leases_in_one_snapshot_and_changes_nothing` |
| Disposition / step | RETARGETED + NEW — 07-03 |

#### S-ND295-05C — A due restart counts its predecessor, and at the cap the predecessor is cleaned up first

`@property @boundary @contract-shape:pure-function`

```gherkin
GIVEN a Service whose allocation crashed and whose restart is due
WHEN the lifecycle reconciler evaluates it below the cap, at the cap with the predecessor still leased, at the cap without a lease, or before the restart is due
THEN below the cap it restarts and the successor is admitted before the predecessor's cleanup
AND at the cap with a leased predecessor it asks for the predecessor's network to be reclaimed and does not restart
AND at the cap without a lease, or before the restart is due, it emits nothing
AND a restart refused by a race consumes no restart budget and never reuses the reserved successor id
```

| Field | Value |
|---|---|
| Discharges | E7; G0/r1, G0/r2, G0/r4; D-295-R7, R8, R11 (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (restart gating and at-cap recreate ordering); FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the emission: a pending restart owns its predecessor)) |
| Contract shape | pure-function |
| Lane | pure (`WorkloadLifecycle::reconcile` table + proptest over occupancy and backoff) |
| Driving port | `WorkloadLifecycle::reconcile(desired, actual, view, tick)` with `State.guest_attachments` |
| Fault stimulus | none |
| Oracle | exact action vectors per table row (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the restart-gating table)); View after a simulated raced refusal: `restart_counts[predecessor]` and `last_failure_seen_at[predecessor]` unchanged, successor id reserved |
| Seed / isolation | table + proptest |
| Rust home | NEW `crates/overdrive-reconcilers/src/workload_lifecycle.rs::restart_gating_acceptance::{a_due_restart_counts_its_predecessor_and_reclaims_it_first_at_the_cap, a_restart_that_is_not_yet_due_emits_nothing_at_any_occupancy, a_raced_restart_refusal_consumes_no_restart_budget}` |
| Disposition / step | NEW — 07-03 |

#### S-ND295-05D — Across a whole node, held attachments never exceed the cap

`@property @tier1 @in-memory @error @contract-shape:bounded-change`

```gherkin
GIVEN one node filled to the cap with workloads through the real reconciler, scheduler, and action shim
WHEN new workloads are placed, one admission is in flight, a Service crashes and is replaced, and a stopped Service resumes
THEN at every lease acquisition the held population, retiring leases included, is at most the cap
AND the peak including retiring predecessors and their replacements never exceeds the cap
AND once room exists the crashed Service is replaced and the stopped one resumes
AND at the cap a crash replacement reclaims its predecessor before the successor is admitted
AND at the cap placement refuses before any dispatch, so the lease pool's own refusal never stops an overflow
AND when two workloads are placed on the one free slot before either is dispatched, the loser's dispatch is refused exactly once and its next evaluation is refused by placement without a second refused dispatch
```

| Field | Value |
|---|---|
| Discharges | E6 seeded, E7 seeded; G0/r1, G0/r2, G0/r4; D-295-R6, R7, R8, R11; proof §3.2 |
| Contract shape | bounded-change |
| Lane | seeded-sim (`overdrive-sim` integration binary: filling one node to 16,384 is quadratic in the cap and runs past the 60 s default-lane budget, which is why it carries the nextest timeout override; the started worker binds nothing, FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification)) |
| Driving port | `run_convergence_tick_with_guest_network_provisioner_for_test` + `dispatch_with_guest_network_provisioner_for_test` (accepted C-295-B signatures) over a seam fixture (§ *Seam fixture*) with `SimDriver`, `SimObservationStore`, `SimViewStore` (behind the test-local ordering decorator `OrderingViewStore`, below), `SimClock`, `SimCa`, `SimDataplane`, a started worker over `SimMtlsEnforcement`, `SimMtlsResolve`, and `SimMtlsIntercept`, and `SimSharedGuestNetworkOwner` behind the test-local `LeaseLedger` decorator. `LeaseLedger` implements both `GuestNetworkProvisioner` and `SharedGuestNetworkOwner` by delegation; its one instance is the `AppState` owner and the seams' `provisioner`. The fixture's EXEC wiring is opened with `open_after_boot()` before the first dispatch (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the gate state outside `run_server*`)). Placement and restart gating hydrate through the production `GuestAttachmentView` over `state.guest_pool`, so this body is also the end-to-end check of that view |
| Fault stimulus | `SimDriver::inject_exit_after(.., Crashed)`; seeded parking of one `provision` call; for the contended slot (block `Contended`, DISTILL review H13), `OrderingViewStore` parks one evaluation's view write-through — the tick's step after `reconcile` placed the workload and before the action shim dispatches (ADR-0035 §5 step 7 before step 9; a fresh placement always changes the view, because it reserves the allocation id) — while a second workload's evaluation runs whole on the same free slot. Production reaches this interleaving: `spawn_convergence_loop` runs up to eight evaluations on distinct targets at once and each write-through is a real fsync await. The decorator controls ordering only and delegates every write unchanged |
| Oracle | NA-1, NA-2, NA-4a, NA-4b, NA-G over the **held** population, counted at every `provision` the ledger sees as provisions minus `guest_network.lease_released` events (the pool's own counts are crate-private, FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the control-plane-private pool operations)); NA-OVERLAP (was OBS-OVERLAP) asserts the retiring-plus-replacement peak `<=` 16,384; NA-5, NA-4a-L, NA-4b-L liveness; NA-RECREATE: at the cap the predecessor's `lease_released` precedes the successor's `provision`; NA-VIEW: each at-cap placement window (NA-1 twice, NA-2) holds no `guest_network.admission_refused`, so placement over the production `GuestAttachmentView` refused before any dispatch and the pool's own `assign` never stopped the overflow (DISTILL review DR-12); NA-E7: over the whole run every allocation and every workload sees at most one `guest_network.admission_refused` (E7: at most one refused dispatch per contended slot), and a refusal naming no allocation is a harness failure; NA-E7-C (the witness that NA-E7 is not vacuous): in the contended window exactly one refusal, naming the parked workload's allocation, while the other workload is admitted, and the loser's immediate re-evaluation adds no refusal and takes no lease |
| Seed / isolation | `OVERDRIVE_ND295_ADMISSION_SEED`, a comma-separated `u64` list (defaults `186055177052160001` and `295032`); the body runs each seed on a fresh node in order and prints the seed with every verdict (DISTILL review L2); no wall time is read — the only clock is the fixture's `SimClock`, and every wait yields to the current-thread runtime (DISTILL review DR-14); four seeded victims and five shuffled blocks; no kernel object is created. One seed's fill took ~280 s on the Lima VM (`red-classification.md` Phase G), so the nextest override allows 25 × 60 s for this test only |
| Rust home | `crates/overdrive-sim/tests/integration/netns_density_node_admission.rs::node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads` (MOVED from `crates/overdrive-sim/tests/netns_density_node_admission.rs`) |
| Disposition / step | RETARGETED + MOVED — 07-03 |

### Group D — TAP activation (gap 7; R5)

#### S-ND295-51 — Only the protected TAP is raised, and quiescence accounts for every TAP

`@property @error @contract-shape:bounded-change`

```gherkin
GIVEN an attachment provisioned down with its protection recorded
WHEN activation, quiescence, and restore run in any accepted order
THEN activation raises the TAP only after re-reading every protection fact, and reports it raised
AND activation while quiescence is latched changes nothing and says so
AND activation of a condemned attachment is refused without change
AND quiescence reports every TAP it could not confirm down and condemns it, and confirms the rest
AND a netlink session lost on one TAP is only that TAP's unconfirmed entry, and the pass goes on
AND a repeat quiescence while latched sets down again any TAP a part-way restore raised
AND a set-down that never completes makes quiescence miss its bound, never blocks its caller
AND restore raises only activation-complete TAPs in order and clears the latch last
```

| Field | Value |
|---|---|
| Discharges | E1 in-process; G3/r1, G3/r2, G3/r4, G3/r5; G5/r1; D-295-R5 (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `quiesce_managed_taps` and `restore_quiesced_taps` contracts; `activate`; the private owner state)) |
| Contract shape | bounded-change |
| Lane | pure (source-local over D12A leaves) |
| Driving port | `GuestNetworkProvisioner::activate`, `SharedGuestNetworkOwner::{quiesce_managed_taps, restore_quiesced_taps}` on the private host owner |
| Fault stimulus | each protection re-read mismatch (bridge, owner, down state, host-side MAC reserved or missing `TapHostMac`, mask `TapDebugMsgMask`, guard, endpoint, ingress program/pin, egress program/pin); `set_link_up` failure; post-set-up read-back failure; per-TAP set-down failure and missing TAP during quiescence; a netlink session failure (`NetlinkError::Connect`) on every set-down (`fail_always(SetTapDown)`), on one TAP's read-back after its set-down landed, and mixed with a missing TAP and a confirmed TAP in one pass (DR-08 (b)-A, user-approved 2026-09-30); a restore that fails on its second set-up after raising one TAP, then a repeat quiescence (re-quiescence after a part-way restore, user-approved 2026-09-30); a set-down leaf that never completes (`hang_always(SetTapDown)`), raced against `SimClock::sleep` of the quiesce bound; restore failure mid-list |
| Oracle | `Ok(TapActivation::Raised)` after exact read-back; `Ok(QuiescenceLatched)` with zero mutating leaf calls; condemned → source-less `PostconditionMismatch`; post-set-up read-back failure attempts `TapSetDown`; `TapQuiescence.unconfirmed` names exactly the failing allocations with `Netlink { TapSetDown }` or `PostconditionMismatch`; the host returns no `Err`: `Connect` on every set-down gives `Ok` with each `Active` TAP unconfirmed as `Netlink { operation: TapSetDown, source: Connect }`, every set-down attempted, the `ProvisionedDown` TAP untouched, the latch set, and a later restore raising nothing; `Connect` on one read-back after its set-down makes only that TAP unconfirmed; a mixed pass keeps each TAP's own cause and restore raises only the confirmed TAP; a repeat quiesce while latched sets down and reads back exactly the TAPs a part-way restore raised, does no I/O on TAPs still quiesced or provisioned down, keeps the latch, and the next restore raises both in `AllocationId` order; a repeat with none raised does no I/O; a set-down that never completes: the bound wins the race, only the pending `SetTapDown` is journaled, and a later activation of a `ProvisionedDown` TAP returns `QuiescenceLatched` with no write (the host makes no synchronous blocking wait on the awaiting task, FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the bound applies to every adapter, pinned 2026-09-30)); restore order by `AllocationId`, stops at the first failure keeping the latch, retry resumes the remainder; `ProvisionedDown`/`Condemned` never raised |
| Seed / isolation | finite tables |
| Rust home | `crates/overdrive-control-plane/src/guest_network.rs::allocation_owner_acceptance::activation_reads_every_protection_fact_before_reporting_success` (RETARGETED: `TapActivation`, host-side MAC invariant, mask, egress; every reserved class at `activate` is S-ND295-72's `a_reserved_or_missing_host_side_address_refuses_activation_before_any_change`); NEW `…::{activation_under_a_latch_or_condemnation_changes_nothing, quiescence_reports_every_unconfirmed_tap_and_condemns_it, restore_raises_only_quiesced_active_taps_in_order_and_clears_the_latch_last, a_netlink_session_failure_is_one_taps_unconfirmed_entry_and_the_pass_continues, a_repeat_quiescence_while_latched_sets_down_what_a_partial_restore_raised, a_quiescence_whose_set_down_never_completes_is_a_bound_miss_not_a_blocked_caller}`. The whole-call case of `quiescence_reports_every_unconfirmed_tap_and_condemns_it`, which asserted the `Err` DR-08 (b)-A removes, is re-authored to the per-TAP partition (DISTILL review DR-08 (b)); the bound-miss body runs on its own thread and current-thread runtime under a 30 s watchdog |
| Disposition / step | RETARGETED + NEW — 06-04 |

#### S-ND295-52 — The action shim raises the TAP after the protection-live event and before the command

`@error @contract-shape:bounded-change`

```gherkin
GIVEN an allocation whose guest has reached READY and whose row is Running
WHEN the action shim completes the start
THEN the protection-live event precedes activation, which precedes command release
AND a start that meets a recovering command gate waits after the protection-live event and activates exactly once after the gate reopens
AND a genuine activation failure stops the VMM, retires the lease, stops protection, tears down, releases the lease last, and writes a dominating Failed row
AND no command is ever released after an activation failure
```

| Field | Value |
|---|---|
| Discharges | E1 in-process; G2/r1, G2/r2, G2/r3, G3/r2; D-295-R5, R7 (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the action-shim order; the activation failure projection)) |
| Contract shape | bounded-change |
| Lane | in-process (control-plane integration: real worker over sim enforcement with its shared owner started, real VM-driver release path, the owner at the driven port) |
| Driving port | `dispatch_with_guest_network_provisioner_for_test(actions, state, tick, provisioner)` with `StartAllocation` and `RestartAllocation`, over a seam fixture whose EXEC wiring is opened with `open_after_boot()` before dispatch; the gate and pool come from `state` (FD § "C-295-B — network provisioner boundary" (the helpers read the EXEC gate and the pool from `state`); FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the gate state outside `run_server*`)) |
| Fault stimulus | a test-local owner implementing `GuestNetworkProvisioner` and `SharedGuestNetworkOwner` whose `activate` returns a typed error, passed as the one owner instance to `AppState` and to the seam; or the `SimSharedGuestNetworkOwner`'s source-less `PostconditionMismatch` for an allocation condemned after its provision while its activation is held at the owner port (`hold_next_activation`): the test drives `script_audit_damage({A})` and one `audit_shared` call, which condemns A without setting the latch, then releases the held activation (FD § "Public deterministic shared-owner simulation API" (`script_audit_damage`, the condemned set, and `activate`)); for the gate wait, the fixture's EXEC gate is moved to Recovering (`begin_recovery(Bridge)`) before the start and reopened with `complete_attempt(None)` |
| Oracle | order journal `start_alloc → event → activate → release_for_exit_emission → on_alloc_running` (this retained ordering holds with or without the gate wait, so it stays active); gate wait: while Recovering the start parks after the protection-live event with no `activate`, no EXEC release, and no running hook, and after the reopen exactly one `activate`, one release, one running hook, and a Running row; failure (retained, active, no lease events): `Provision → Activate → Teardown`, one driver start and one stop, no release and no running hook, one `stop_alloc`, and a Failed row `WorkloadNetnsProvisionFailed { stage: "guest_network_activate" }`; failure with the lease (06-04): `driver.stop`, `lease_retired`, `stop_alloc`, teardown, `lease_released`, exactly one refusal recorded at the owner port, and the Failed row's stage (its detail is not pinned); condemned: the refusal is exactly the source-less `PostconditionMismatch { operation: TapObserve, expected: Tap { name: <A's TAP>, ifindex: None, link_kind: Tap, persistent: true, up: false, owner_uid: Some(0) }, .. }` (`observed` is not pinned) and the same failure projection follows; zero EXEC release; driver-stop failure keeps its typed error primary and withholds cleanup |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs::{tap_activation_occurs_after_intercept_success_and_before_exec_release, tap_activation_failure_stops_vmm_cleans_mtls_and_network_and_dominates_running}` (RETAINED, **active**: their oracles hold today, so they keep guarding the start order and the activation-failure unwind through 05-00…06-03; DISTILL review DR-13); NEW `…::{tap_activation_waits_on_a_recovering_exec_gate_and_runs_once_after_reopen, tap_activation_failure_retires_the_lease_and_releases_it_last, activation_of_a_condemned_allocation_takes_the_failure_projection}` (the gate wait and the lease additions, split out so the retained oracles stay active) |
| Disposition / step | RETAINED (active) + NEW — 06-04 |

#### S-ND295-53 — Activation waits out a recovery and never turns it into a failure

`@property @tier1 @in-memory @contract-shape:bounded-change`

```gherkin
GIVEN an allocation ready to activate while the node's guest-command gate is recovering, latched, or failed
WHEN the recovery completes or fails
THEN an activation that meets a recovering gate waits and then runs exactly once after it reopens
AND an activation that meets a latched quiescence raises nothing and runs again after reopen
AND at fail-stop neither activation nor the command is released and no row is written
AND none of these writes a Failed row or advances the restart budget
```

| Field | Value |
|---|---|
| Discharges | E1 seeded; G3/r2, G3/r3, G3/r4; D-295-R5 (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (waiting on the EXEC gate, the latch invariant, and no Failed row for an observed recovery)) |
| Contract shape | bounded-change |
| Lane | seeded-sim (`overdrive-sim` acceptance binary, default lane; the started worker binds nothing, FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification)) |
| Driving port | `dispatch_with_guest_network_provisioner_for_test(actions, state, tick, provisioner)` over a seam fixture whose `AppState` carries `wiring.gate()` of a real `GuestNetworkExecWiring` over `SimClock` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors)); the test keeps `wiring.supervisor()` and calls `open_after_boot` / `begin_recovery` / `complete_attempt` / `fail_stop` on it, and `quiesce_managed_taps` / `restore_quiesced_taps` on the `SimSharedGuestNetworkOwner`, at seeded instants while the dispatch future is parked; the worker's shared owner is started |
| Fault stimulus | seeded interleavings of gate transitions and the sim owner's latch |
| Oracle | activation count from the sim owner's `calls()`: the sim records `TapSetUp` both for a raised activation and for every restore call, successful or not (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the sim adapter's restore rule); FD § "Public deterministic shared-owner simulation API" (`activate`, and `restore_quiesced_taps` recording `TapSetUp`; the call journal)), and records nothing for a latched activation. No supervisor runs in this lane, so the test is the only caller of `quiesce_managed_taps` and `restore_quiesced_taps`; it brackets each of its own calls with `calls().len()`, and every `TapSetUp` outside those brackets is an activation. Exactly one such entry appears, after reopen. `guest_network.activation_withheld { alloc, reason: "fail_stop" }` at fail-stop with zero EXEC release and no row; `WorkloadLifecycleView.restart_counts` unchanged |
| Seed / isolation | `OVERDRIVE_ND295_ACTIVATION_SEEDS` (default fixed list), printed per verdict; no wall time: a pending poll yields once and ticks `SimClock`, and exhausting the step budget is a harness failure that prints the seed (DISTILL review DR-14) |
| Rust home | NEW `crates/overdrive-sim/tests/acceptance/netns_density_activation_order.rs::{activation_during_recovery_runs_once_after_reopen_without_a_failed_row, a_latched_activation_retries_after_reopen, fail_stop_withholds_activation_and_the_command}`; sim-owner surface self-tests NEW `crates/overdrive-sim/src/adapters/guest_network.rs::tests::scripted_quiescence_outcomes_condemn_each_named_allocation_once` and `…::tests::standing_owner_controls_preserve_exact_operation_semantics` (RETARGETED: `script_quiesce_outcome` replaces `script_quiesce_failure`); test-local-owner self-tests NEW `crates/overdrive-control-plane/src/shared_network_test_ports.rs::tests::{activate_reports_raised_latched_or_condemned, restore_failure_slot_keeps_the_latch}` — re-homed out of `overdrive-sim` because `activate` needs a `GuestNetworkPlan`, which is control-plane-private with no cross-crate constructor, and the `overdrive-sim`↔`overdrive-control-plane` dependency cycle makes only `TestSharedOwner` (not `SimSharedGuestNetworkOwner`) usable in a source-local lane; these are active (the double is fully implemented); the failed-restore-keeps-the-latch path (item 7) is also exercised by S-ND295-29A's re-quiescence cell and S-ND295-29B C4b. Their plans come from a test-owned `GuestAddressPool::new(..).assign`, never the static action pool 06-03 deletes (DISTILL review DR-11) |
| Disposition / step | NEW + RETARGETED — 06-04 |

### Group E — Retry-retaining cleanup and reclaim (gap 5; R10, R11)

#### S-ND295-07 — Stop removes the predecessor completely before its address is reused

`@property @in-memory @contract-shape:bounded-change`

```gherkin
GIVEN an active allocation with a lease, a registration, shared protection members, an endpoint, both links, a guarded TAP, and a VMM
WHEN the production action owner stops or replaces it
THEN it confirms the VMM is gone, retires the lease, awaits protection removal, tears down, and releases the lease last
AND no successor receives that address before the predecessor's complement is empty
AND across any seeded interleaving of failed removals, retries, and successor starts, the address is reused only after the retried removal converges
```

| Field | Value |
|---|---|
| Discharges | E8 (ordering half, and the seeded half: DISTILL review M13); G2/r2; D-295-R7, R10 (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the retirement points); FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the worker's `element_effects` mutex)) |
| Contract shape | bounded-change |
| Lane | pure (control-plane acceptance over `SimSharedGuestNetworkOwner`, as S-ND295-06). The predecessor's start reaches intercept install, so the fixture's worker shared owner is started; over the recording intercept, which delegates binding to `SimMtlsIntercept`, it binds nothing (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification)) |
| Driving port | `dispatch_with_guest_network_provisioner_for_test(actions, state, tick, provisioner)` with `StartAllocation` and `StopAllocation`, over a seam fixture with one `SimSharedGuestNetworkOwner` passed to `AppState` and to the seam; the seeded body drives the same seam from an `overdrive-sim` fixture (seeded-sim, acceptance binary, default lane) |
| Fault stimulus | `script_teardown_failure(true)` then disarm; seeded: a test-local `RemovalFaultIntercept` over `SimMtlsIntercept` refuses `remove_allocation_elements` for the predecessor's source with `NftElementUpdateFailed` (`EBUSY`) while armed, under a seeded schedule (0–1 unrelated starts before the stop, 1–3 failing stop attempts, 1–3 successor starts while held, 0–1 further stops, shuffled, 50–500 ms of simulated time apart) |
| Oracle | one ordered trace from a test-local tracing Layer that records the lease events (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the lease events)) and samples, at each, the `SimDriver` stop count, the owner's `calls().len()`, and the element removals seen by a test-local recording `MtlsIntercept` over `SimMtlsIntercept`: `driver.stop → lease_retired → element removal → teardown → lease_released`; while teardown fails there is `lease_retired` and no `lease_released`, and an unrelated start is assigned a different address (`SimDriver::started_specs()`); after the retry `lease_released`, and the next start receives the predecessor's address. Seeded verdicts over one journal: E8-REMOVAL-REACHED (each failing stop reached the removal for the predecessor's source), RETIRED-BEFORE-REMOVAL, HELD-WHILE-FAILING (retired, counted, never released while a removal fails), NO-REUSE-WHILE-HELD (no start receives the address while held, with a check that a start ran in that window so the verdict cannot pass vacuously), RETRY-CONVERGES, REUSE-AFTER-RELEASE (the lease address and the address in the `SimDriver` spec) |
| Seed / isolation | `OVERDRIVE_ND295_RETIRING_CLEANUP_SEEDS` (four defaults), printed with every verdict; no wall time (only `SimClock`) |
| Rust home | `crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs::teardown_failure_holds_the_lease_until_retry_completes_then_allows_exact_address_reuse`; NEW `crates/overdrive-sim/tests/acceptance/netns_density_retiring_cleanup.rs::a_failed_element_removal_keeps_the_address_until_a_retry_converges` |
| Disposition / step | RETARGETED + NEW — 07-01 (the seeded body's fixture passes the worker, owner, gate, and pool to `AppState` from 05-01; before that its removal fault is not wired, and it is RED at E8-REMOVAL-REACHED) |

#### S-ND295-07B — A failed protection cleanup keeps the allocation retiring until a retry succeeds

`@error @contract-shape:bounded-change`

```gherkin
GIVEN a stopping allocation whose shared protection members cannot be removed
WHEN the stop runs through the real worker and action owner
THEN the stop fails with the original removal cause
AND the lease stays retiring and counted, no structural teardown runs, and the row stays Running
AND a same-address successor is refused
AND a retried stop re-runs the removal, and only then tears down, releases, and records Terminated
```

| Field | Value |
|---|---|
| Discharges | E8 in-process; G2/r2; D-295-R10, R7, R20 (row stays Running, FD § "[REF] Operator status — network cleanup pending (D-295-R20) — ACCEPTED 2026-09-24 (operator behaviour user ruling of the same date)" (the cleanup-pending table's Retiring row)) ; proof §3.4 |
| Contract shape | bounded-change |
| Lane | in-process (control-plane integration) |
| Driving port | the real `MtlsInterceptWorker` (shared owner started) + the `StopAllocation` arm through `dispatch_with_guest_network_provisioner_for_test(actions, state, tick, provisioner)`, over a seam fixture whose one owner instance is `AppState`'s owner and the seam's `provisioner` |
| Fault stimulus | test-local `ElementFaultIntercept` implementing `MtlsIntercept`; its `remove_allocation_elements` routes to the append-only `ElementModel::remove` (batch rejected; acknowledged-then-read-back-failed) |
| Oracle | the ten assertions of the proof, retargeted: `Err(ShimError::MtlsStop(MtlsInterceptStopError::ElementRemoval { alloc_id, source }))` with `alloc_id` the stopping allocation and `&*source` the `InterceptError` the `ElementModel` injected, matched by variant and cause (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the worker's typed stop error)); `alloc_stop_converged_for_test == false`; successor `start_alloc` refused; no `TapDelete`; the lease stays Retiring — `guest_network.lease_retired { alloc }` captured and no `lease_released` (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the lease events); the pool is crate-private); row `Running`; retried stop calls `remove_allocation_elements` again; after retry no predecessor member remains, teardown then `lease_released` follow, row `Terminated` |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | `crates/overdrive-control-plane/tests/integration/shared_element_cleanup_failure.rs::{shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address, shared_element_cleanup_failure_readback_failed_retains_retirement_and_address}` |
| Disposition / step | RETARGETED (lands in place) — 07-01 |

#### S-ND295-54 — Protection removal is convergent and its failures are typed

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN an allocation's protection members, some already absent
WHEN the worker removes them
THEN it removes exactly the present members in one atomic change and reports the absent ones without error
AND a rejected change leaves every member as it was, and a failed read-back restores the prior state and keeps both causes
AND the worker keeps the retiring record and its guards until a retried stop succeeds
AND each enforced-connection teardown failure is reported per connection with its typed cause
AND every caller waiting on one stop receives the same failure
AND the first stop after a failure starts exactly one retry, however many callers ask at once
AND a stop that arrives after the owner began shutting down starts nothing and returns that allocation's shutdown outcome
```

| Field | Value |
|---|---|
| Discharges | E8 Lima + seeded (worker half); D-295-R10 (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (`remove_allocation_elements`; the netlink and worker contracts)); B-6 stop-error shape and caller rules 1, 3, and 4 (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the worker's typed stop error and what each caller receives)) |
| Contract shape | bounded-change |
| Lane | pure (worker source-local over the private seam and registry) + lima-kernel (real nft through `HostMtlsIntercept`) + in-process (the DR-06 Failed-row detail bodies) |
| Driving port | `MtlsIntercept::remove_allocation_elements(source, destinations)`; `MtlsInterceptLifecycle::stop_alloc` and `MtlsInterceptWorker::shutdown_owner` on the worker |
| Fault stimulus | duplicate or zero-port destinations (argument validation, refused before any netlink I/O — not a kernel batch rejection); a recorded identity unequal to the observed program; real pre-absent member; in the worker bodies, `TestSharedIntercept::{script_removal_failures, hold_removals}` and a failing test `MtlsEnforcement::teardown` (§ *Intercept listener and stop-error test support*) |
| Oracle | pre-I/O refusal for invalid destinations; typed refusal without mutation on identity mismatch; `Ok(InterceptState)` without the requested members and with every other member, the program, and the foreign complement unchanged. R10's two netlink-failure arms — a delete batch the kernel itself rejects leaving every member unchanged, and a post-commit read-back failure performing one inverse transition and keeping both causes — are asserted source-local, default lane, over the module-private `SharedIpElementIo` seam in `overdrive-netlink::nft` (user decision 3 of 2026-09-30; H14 resolved; the seam is a behaviour-preserving extraction of `mutate_and_readback`, adds no public surface, and its scripted `ScriptedElementIo` double is DISTILL's test support). The four cells — `a_rejected_batch_returns_its_error_after_one_send_with_no_observation_or_inverse`, `a_failed_restoration_retains_both_the_primary_and_the_restoration_cause`, `a_failed_or_mismatched_read_back_restores_once_and_returns_the_primary`, and `a_matching_read_back_returns_the_new_state_with_no_inverse` — rest on the kernel's guarantee that a rejected `nft` batch commits nothing (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (Evidence for the two element-batch failure rules)); activated at 07-01 when the owner wires the seam into the production removal path. The worker's handling of both failures is additionally proven in-process by S-ND295-07B's `ElementModel`. Typed stop errors: `MtlsInterceptStopError::HandleTeardown { alloc_id, failures }` holds one `HandleTeardownFailure { connection, source }` per failed connection in teardown order, with `&*source` the injected `MtlsEnforcementError`; `ElementRemoval { alloc_id, source }` has `&*source` the scripted `InterceptError`. B-6 caller rules, each asserted here and nowhere else: **rule 1** — two `stop_alloc(a)` calls joined on one attempt whose removal fails (the second issued while the first attempt's removal is held) receive equal errors (same variant, same `alloc_id`) whose source `Arc`s are `Arc::ptr_eq`, and that attempt called `remove_allocation_elements` once; **rule 3** — after that `Err`, two `stop_alloc(a)` callers spawned behind a `Barrier(2)` while removals are held begin exactly one new attempt (`removal_calls()` rises by one, not two, checked before and after the release) and both receive its result with `Arc::ptr_eq` sources, never the superseded error, over 16 rounds, the last converging (DISTILL review H14 rule 3, DR-12); **rule 4** — with `a`'s failed attempt retained, `b` active, and removals held so the owner shutdown's teardown of `b` is in flight, `stop_alloc(a)` runs no removal or teardown of its own, returns after the owner shutdown with an error equal to the shutdown result's entry for `a` (pointer-equal source), and `stop_alloc(b)` returns `Ok(())` because `b`'s teardown by the owner shutdown succeeded. `HandleTeardown`'s `Display` is the pinned text with every per-connection cause (two connections, in order). Failed-row detail (DR-06, user decision; the restart-abort path through a test-local `StopFaultLifecycle`): two scripted handle-teardown failures (`EBADF`, `ENOTCONN`) leave the persisted `DriverStartFailure.detail` holding the primary rejection, "for 2 handle(s)", each `<connection>: <cause>`, and the full pinned text; an `ElementRemoval` (`EBUSY`) leaves the pinned "shared intercept element removal failed: …" cause in the detail |
| Seed / isolation | table + example; the worker bodies are source-local default-lane (the rule-3 body uses real 50 ms and 2 s waits: a late caller gives a false RED, never a false GREEN); the two detail bodies are in-process in the control-plane integration binary; `overdrive-worker` integration binary is `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-worker/src/mtls_intercept_port.rs::shared_program_rollback_acceptance::remove_allocation_elements_deletes_only_present_requested_members`; NEW `crates/overdrive-worker/src/mtls_intercept_worker.rs::tests::element_removal_failure_keeps_the_retiring_record_until_a_retry_converges`; `…::tests::allocation_stop_surfaces_teardown_failure_and_retry_converges` (RETARGETED to typed per-connection failures, and onto a shared allocation because the per-allocation record it registers today is deleted with B-7's step); NEW `…::tests::{callers_joined_on_one_failed_stop_receive_equal_failures_with_shared_sources, the_first_stop_after_a_failure_starts_one_retry_for_simultaneous_callers, a_stop_after_owner_shutdown_began_starts_nothing_and_returns_its_shutdown_entry}` (B-6 rules 1, 3, 4); NEW `crates/overdrive-worker/tests/integration/shared_intercept_members.rs::{convergent_removal_with_a_pre_absent_member_and_batch_rejection_preserves_state, removal_is_refused_when_the_recorded_program_was_replaced_out_of_band}` (the second is R10's recorded-versus-observed refusal, whose only row moved off the pre-absent body under B-8: a converged host whose kernel program is replaced out of band refuses without mutating, and not with `SharedProgramNotConverged`); `crates/overdrive-control-plane/tests/integration/server_lifecycle.rs::graceful_shutdown_propagates_worker_failure_without_a_retry_capability` (RETARGETED: `ServerHandle::replace_mtls_worker_for_test` and `MtlsInterceptWorker::inject_owner_shutdown_failure_for_test` are deleted with the worker's `owner_shutdown_failures` field, FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `ServerHandle` owner fields: the deleted test overrides). The body boots `run_server_with_obs_and_driver` with `ServerConfig::new(kek, mtls_intercept, guest_dns)` whose `mtls_intercept` is a test-local `MtlsIntercept` over `SimMtlsIntercept` with an armable `remove_allocation_elements` fault, a `SimDriver`, the sim owner, `SimGuestDnsFactory`, and `vm_cgroups` over `SimCgroupFs`; deploys one workload through the API; arms the fault; calls `ServerHandle::shutdown`. Oracle: `Err`, and `teardown_failure().failures` is exactly one `MtlsInterceptStopError::ElementRemoval { alloc_id, source }` with `alloc_id` the deployed allocation and `&*source` the injected `InterceptError`) |
| Disposition / step | NEW + RETARGETED — 07-01 (the `server_lifecycle` body needs the required `ServerConfig.mtls_intercept` of 05-01 and the typed `ElementRemoval` of 07-01). Also NEW: `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs::{restart_abort_detail_names_every_failed_handle_teardown_cause, restart_abort_detail_names_the_element_removal_cause}` (DR-06). The Lima body's name keeps "batch_rejection", but its refused requests are argument refusals only (see the Oracle) |

#### S-ND295-55 — Every leased, unowned, finished allocation is reclaimed from every reconcile path

`@property @error @contract-shape:pure-function`

```gherkin
GIVEN a workload with leased allocations that are Failed or Terminated
WHEN the lifecycle reconciler evaluates it on the stop path, the delete path, behind the job terminal fence, the Running guard, the Draining guard, the operator-stop veto, the job natural-exit path, or a restart that is due at the cap while its predecessor holds no lease
THEN it asks to reclaim each such allocation that no other action in the evaluation owns
AND it never reclaims an allocation whose restart is pending and not yet due, or due with room
AND after a failed reclaim it asks again no sooner than one second later, forever, until the lease is gone
AND a reclaim may never name an allocation another action names in the same evaluation
```

| Field | Value |
|---|---|
| Discharges | E9 pure; G0/r3; D-295-R11 (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the emission; the reclaim retry memory; the validator)) |
| Contract shape | pure-function |
| Lane | pure |
| Driving port | `WorkloadLifecycle::reconcile`; `validate_reconcile_output` |
| Fault stimulus | none |
| Oracle | `ReclaimAllocationNetwork { alloc_id }` in `AllocationId` order on each listed path (the due-restart-at-the-cap path with an unleased predecessor, `ReturnPath::DueRestartAtCapUnleasedPredecessor`, carries a leased, finished leftover row and emits exactly the unleased baseline's reclaims; DISTILL review DR-12); View `reclaim_attempts` / `reclaim_emitted_at` gate re-emission at `emitted_at + backoff_for_attempt(attempts)` (constant one second today); `next_evaluation_at` = earliest deadline; entries pruned when the lease disappears; the validator rejects a reclaim beside `StartAllocation` / `RestartAllocation` / `StopAllocation` / `FinalizeFailed` for the same id |
| Seed / isolation | table + proptest over row states, leases, and ticks |
| Rust home | NEW `crates/overdrive-reconcilers/src/workload_lifecycle.rs::reclaim_emission_acceptance::{every_return_path_reclaims_leased_unowned_finished_allocations, a_pending_restart_owns_its_predecessor_until_due_at_the_cap, a_failing_reclaim_backs_off_one_second_and_never_stops}`; NEW `crates/overdrive-control-plane/src/action_shim/validate.rs::tests::a_reclaim_beside_another_action_for_the_same_allocation_is_rejected` |
| Disposition / step | NEW — 07-03; the validator body is 07-02, where the validator rule lands with the reclaim action (DISTILL review M2) |

#### S-ND295-56 — Reclaim cleans an allocation's network without touching its row

`@error @contract-shape:bounded-change`

```gherkin
GIVEN a finished allocation that still holds a lease
WHEN the reclaim action is dispatched
THEN it retires the lease, confirms the VMM is gone, removes protection, tears down, and releases the lease last
AND it writes no row and emits no lifecycle event
AND a reclaim for an allocation without a lease does nothing
AND a failing step keeps the lease and returns a typed error, while parts already removed elsewhere still let it finish
AND a reclaim of an allocation whose lease is still admitted retires it before any teardown and releases it after
```

| Field | Value |
|---|---|
| Discharges | E9 in-process; D-295-R11 shim arm (FD § "[REF] Lifecycle action — row-neutral reclaim (D-295-R11) — ACCEPTED 2026-09-24" (the shim arm)) |
| Contract shape | bounded-change |
| Lane | pure (control-plane acceptance over the seam fixture, as S-ND295-06). The seam uses the `AppState` worker (R16), not a private lifecycle implementation, and the precondition start reaches intercept install, so the worker's shared owner is started; it binds nothing (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification)) |
| Driving port | `dispatch_with_guest_network_provisioner_for_test(actions, state, tick, provisioner)` with `Action::ReclaimAllocationNetwork`, over a seam fixture whose one test-local owner (implementing `GuestNetworkProvisioner` and `SharedGuestNetworkOwner`) is `AppState`'s owner and the seam's `provisioner` |
| Fault stimulus | precondition through the production path: an allocation whose `activate` fails (the test-local owner's scripted typed error) while its teardown is scripted to fail, so the shim writes its Failed row and keeps its lease Retiring (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the activation failure projection)); then, for the failing-step case, the teardown slot left armed or an element-removal fault on the worker's test-local `MtlsIntercept`; for the already-removed case, the allocation's intercept members removed out of band at the port and its attachment parts marked removed in the test-local owner before the reclaim (the owner still issues its teardown leaves; only the intercept half is out of band at a real port — DISTILL review H8); for the admitted-lease case, a crash through `SimDriver::inject_exit_after(.., Crashed)` consumed by the production exit observer, with no lease event before the reclaim |
| Oracle | step trace (lease events plus sampled `SimDriver` stop count, owner journal, and element removals, as in S-ND295-07): retire (a no-op on an already-Retiring lease) → VMM confirmed gone (`driver.stop` `NotFound` counts) → element removal → teardown → `lease_released`; no `AllocStatusRow` write and no lifecycle event during the reclaim; a reclaim for an allocation without a lease → `Ok(())` with no owner, driver, or intercept call; a failing step → its typed `ShimError` — `GuestNetwork(Io { TapDelete })`, or `MtlsStop(MtlsInterceptStopError::ElementRemoval { alloc_id, source })` with `&*source` the injected `InterceptError` — with `lease_retired` and no `lease_released`; already removed: `DriverStop NotFound → ElementRemoval (nothing present) → TapDelete → lease_released`, one teardown, the row still Failed; admitted lease: `lease_retired → driver stop → element removal → TapDelete → lease_released` |
| Rust home | NEW `crates/overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs::{reclaim_cleans_a_leased_finished_allocation_without_a_row, reclaim_without_a_lease_does_nothing, a_failed_reclaim_step_keeps_the_lease_for_the_next_attempt, a_reclaim_whose_parts_were_removed_out_of_band_releases_the_lease, a_reclaim_retires_an_admitted_lease_before_its_teardown_and_releases_it_after}` |
| Disposition / step | NEW — 07-02 |

#### S-ND295-57 — Leftover networks are reclaimed until released, across stopped and deleted workloads

`@property @tier1 @in-memory @error @contract-shape:bounded-change`

```gherkin
GIVEN restart predecessors, stopped workloads, and deleted workloads whose network cleanup failed
WHEN the node keeps reconciling over simulated time
THEN each leftover lease is retried at the one-second cadence and eventually released
AND no row is rewritten by a reclaim
AND at the cap a predecessor whose restart is not yet due is left to its restart
```

| Field | Value |
|---|---|
| Discharges | E9 seeded; G0/r3; D-295-R11 |
| Contract shape | bounded-change |
| Lane | seeded-sim (`overdrive-sim` integration binary behind `integration-tests`: the at-cap body fills one node to 16,384 per seed; DELIVER 07-03 measured 177.361 s across three default seeds, above the default profile's 120 s timeout, so it retains the nextest budget override; its every-path sibling shares the file's fixture; DISTILL review M16) |
| Driving port | convergence-tick + dispatch seams over sim adapters (as S-ND295-05D) |
| Fault stimulus | `SimSharedGuestNetworkOwner::script_teardown_failure` armed for N seeded attempts |
| Oracle | re-dispatch instants at least one second apart per allocation; `guest_network.lease_released { alloc }` after disarm; no row write from reclaim; not-yet-due-at-cap predecessor gets no reclaim until due |
| Seed / isolation | `OVERDRIVE_ND295_RECLAIM_SEEDS`, printed per verdict; no wall time (only `SimClock`; DISTILL review DR-14). The at-cap GREEN body took 177.361 s across three default seeds (`red-classification.md` DELIVER observations) |
| Rust home | NEW `crates/overdrive-sim/tests/integration/netns_density_reclaim.rs::{leftover_networks_are_reclaimed_until_released_on_every_path, a_not_yet_due_restart_keeps_its_predecessor_at_the_cap}` (MOVED from `tests/acceptance/`) |
| Disposition / step | NEW + MOVED — 07-03 (the at-cap GREEN duration exceeds the default lane, so the file stays in the integration binary) |

### Group F — Cleanup-pending operator status (R20, user ruling 6)

#### S-ND295-58 — Cleanup is pending exactly while the lease outlives the running allocation

`@property @contract-shape:pure-function`

```gherkin
GIVEN every combination of lease state and allocation state
WHEN the platform decides whether network cleanup is pending
THEN it is pending for every retiring lease, and for an admitted lease on a Failed or Terminated allocation
AND it is not pending for an admitted lease on a live allocation
```

| Field | Value |
|---|---|
| Discharges | E20 pure; G5/r3 (gates nothing); D-295-R20 (FD § "[REF] Operator status — network cleanup pending (D-295-R20) — ACCEPTED 2026-09-24 (operator behaviour user ruling of the same date)" (the observed fact it derives from)) |
| Contract shape | pure-function |
| Lane | pure |
| Driving port | `GuestAttachmentLease::cleanup_pending(self, row_state)` |
| Oracle | exhaustive table over `{Admitted, Retiring}` × every `AllocState`, through `lease.cleanup_pending(row)`. The six former no-lease rows computed both sides locally and are dropped: with no lease there is no `GuestAttachmentLease` to ask, so "not pending without a lease" is the server projection's contract, asserted by S-ND295-59 (a retried stop's `Terminated`, lease-released row reads `false`; DISTILL review L7) |
| Rust home | NEW `crates/overdrive-core/tests/acceptance/netns_density_cleanup_pending.rs::cleanup_pending_matches_the_lease_and_row_state_table` |
| Disposition / step | NEW — 07-04 |

#### S-ND295-59 — Describe reports a stuck stop as cleanup-pending and never counts it as a running replica

`@driving_port @error @contract-shape:bounded-change`

```gherkin
GIVEN a Service replica whose stop cannot remove its network protection
WHEN the operator reads the workload's allocations
THEN that allocation is reported with network cleanup pending and is not counted as a running replica
AND once a retry succeeds the allocation is Terminated and no longer pending
AND a crashed allocation awaiting cleanup and one being reclaimed are also reported pending
```

| Field | Value |
|---|---|
| Discharges | E20 in-process; D-295-R20 server projection (FD § "[REF] Operator status — network cleanup pending (D-295-R20) — ACCEPTED 2026-09-24 (operator behaviour user ruling of the same date)" (the server projection)) |
| Contract shape | bounded-change |
| Lane | in-process (control-plane integration, `run_server` + HTTPS API) |
| Driving port | `run_server_with_obs_and_driver` with `ServerConfig::new(kek, mtls_intercept, guest_dns)` (`guest_dns` = `SimGuestDnsFactory`), a `SimDriver`, the sim owner, and `vm_cgroups` = `CgroupManager` over `SimCgroupFs` (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)); `GET /v1/allocs` through the public API |
| Fault stimulus | `ServerConfig.mtls_intercept` = a test-local `MtlsIntercept` whose `remove_allocation_elements` fails until disarmed; `SimDriver::inject_exit_after(.., Crashed)` |
| Oracle | `network_cleanup_pending: true` with `state: Running` and excluded from `replicas_running`; after disarm and retry: `Terminated`, `false`; crashed row (`Failed`, Admitted lease) → `true`, and a process-global lease-event capture installed before boot shows the lease state behind each row: the crashed allocation has no lease event (its lease is Admitted), the reclaiming one has at least one `lease_retired` and no `lease_released` while its row reads pending, the success path ends with exactly one `lease_released`, last, and every lease event names an allocation (DISTILL review M10) |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-control-plane/tests/integration/network_cleanup_pending_status.rs::{a_stuck_stop_is_reported_cleanup_pending_and_is_not_a_running_replica, crashed_and_reclaiming_allocations_are_reported_cleanup_pending}` |
| Disposition / step | NEW — 07-04 |

#### S-ND295-60 — The describe output shows CleanupPending with the lifecycle state beside it

`@contract-shape:pure-function`

```gherkin
GIVEN allocation bodies with and without network cleanup pending, for a Service and for a Job
WHEN the CLI renders workload describe
THEN a pending allocation reads CleanupPending with a detail line naming its lifecycle state, and never Running
AND every non-pending allocation renders exactly as before
```

| Field | Value |
|---|---|
| Discharges | E20 CLI; D-295-R20 rendering (FD § "[REF] Operator status — network cleanup pending (D-295-R20) — ACCEPTED 2026-09-24 (operator behaviour user ruling of the same date)" (the CLI rendering)) |
| Contract shape | pure-function |
| Lane | pure |
| Driving port | `render::workload_describe` (the one live renderer) |
| Oracle | pending row State cell `CleanupPending`; detail line `    network cleanup: pending (lifecycle state: <state_label>)`; byte-identical output for non-pending rows (golden comparison); Job verdict unchanged |
| Rust home | NEW `crates/overdrive-cli/tests/acceptance/render_workload_describe.rs::{a_cleanup_pending_allocation_renders_cleanup_pending_with_its_lifecycle_state, non_pending_allocations_render_byte_identically}` |
| Disposition / step | NEW — 07-04 |

### Group G — Boot after process loss (gap 6; R12)

#### S-ND295-13A — Boot reclaims old VMs, sweeps, clears stale members, then converges the program before admitting work

`@property @error @contract-shape:bounded-change`

```gherkin
GIVEN a previous process left VMs, attachments, and stale protection members behind
WHEN a fresh server boots
THEN old VMs are reclaimed before the attachment sweep
AND stale protection members are cleared before the constant program and policy route are converged
AND admission opens only after all of that is read back
```

| Field | Value |
|---|---|
| Discharges | E10 seeded (extended); G1/r1; D-295-R12 (FD § "[REF] Boot ordering (D-295-R12) — ACCEPTED 2026-09-24" (the boot sequence)) |
| Contract shape | bounded-change |
| Lane | seeded-in-process (drives `run_server_with_obs_and_driver`, D-295-DISTILL-13's selected boundary, FD § "D-295-DISTILL-13 — production-composed S-ND295-13 boot-order boundary" (the selected shape); R16 composes the real enforcement probe, so the module gains `integration-tests` gating) |
| Driving port | `run_server_with_obs_and_driver` with `ServerConfig::new(kek, mtls_intercept, guest_dns)` (`guest_dns` = `SimGuestDnsFactory`), one `SimVmHostState`, `SimSharedGuestNetworkOwner::with_sweep_host_state`, a test-local recording `MtlsIntercept` whose every call snapshots the sim owner's `calls().len()` and `supervisor.is_boot_closed()`, and the final `vm_cgroups` = `CgroupManager::new(<root>, Arc::new(SimCgroupFs::new()))` (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)). The test keeps `wiring.supervisor()` before the `GuestNetworkExecWiring` moves into the call |
| Fault stimulus | seeded prior-VM residue (scopes, run dirs, clones); the recording intercept starts with seeded non-empty members |
| Oracle | existing sweep-snapshot assertions, plus the relative order sweep-call index ≺ the shared owner's `converge_shared` (R12 step 5, the owner's `BridgeConverge` call) ≺ `converge_allocation_elements(∅)` ≺ `observe_shared` ≺ `converge_shared(prior, F, C)` ≺ `observe_shared_state` (zero members) ≺ `open_after_boot`. The order is relative, not "some later call exists": no `observe_shared` or intercept `converge_shared` precedes the member clear, and no intercept `converge_shared` precedes the `observe_shared` that captured its prior (DISTILL review M16). Every recorded intercept call sees `is_boot_closed() == true`, and the gate leaves BootClosed only after the last of them |
| Seed / isolation | proptest `seed in any::<u64>()`, printed; each case boots `run_server`. At RED the first case fails and the body ends in 0.9 s (`red-classification.md` Phase G); an 08-02 review item records the GREEN run time of the default case count — the module is already behind `integration-tests`, so an over-budget run gets a nextest override, never a lowered `PROPTEST_CASES` |
| Rust home | `crates/overdrive-sim/src/invariants/netns_density_boot_order.rs::tests::reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm` (RETARGETED/extended; module gated `#[cfg(all(test, feature = "integration-tests"))]`) |
| Disposition / step | RETARGETED — 08-02 |

#### S-ND295-13B — Boot phases are visible to the operator in order

`@contract-shape:bounded-change`

| Field | Value |
|---|---|
| Discharges | E10 telemetry (GREEN operational evidence, FD § "D-295-DISTILL-13 — production-composed S-ND295-13 boot-order boundary" (the boot-phase telemetry events)) |
| Lane | in-process |
| Oracle | exactly `vm_reclamation/started`, `vm_reclamation/completed`, `stale_sweep/started`, `stale_sweep/completed` of `guest_network.shared_owner_boot_phase`, in that order, captured by a tracing Layer across ordinary `run_server` |
| Rust home | `crates/overdrive-control-plane/tests/integration/shared_guest_network_startup.rs::production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts` (placeholder AUTHORED) |
| Disposition / step | AUTHORED — 08-02 |

#### S-ND295-13C — A killed server's replacement clears stale members and admits work

`@real-io @error @driving_port @contract-shape:bounded-change`

```gherkin
GIVEN a server killed abruptly while a mesh Service VM is Running, leaving stale protection members, the TAP, its links, and the VM behind
WHEN a replacement server boots on the same data and config roots
THEN it kills the old VM, sweeps the attachment, clears every stale member in one change, and only then converges the program
AND it boots instead of refusing, adopts nothing from the dead server, and admits a new allocation at the smallest free address
```

| Field | Value |
|---|---|
| Discharges | E10 native (proof §3.5, killed mode); G1/r1, G1/r5; D-295-R12, R17 killed mode |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve::run_with_kek` under `ServeLifetime` with the killed-mode signal (`ServeSignal::Kill` → `ServerHandle::kill_for_test`), then a second `serve::run_with_kek` on the same roots; `deploy` handler |
| Fault stimulus | killed mode (no graceful cleanup; the worker is dropped on a capability-less thread) |
| Oracle | M0 the `nft monitor` transcript is complete: the monitor did not exit early, wrote nothing to stderr, and printed no error or lost-events line (DISTILL review M12); V0 boot two returns `Ok`; V1 CH pids dead and scope gone before any boot-two nft batch; V2 exactly one boot-two batch deletes exactly the three stale members with no program mutation before it; V3 `observe_shared_ip_intercept_state` shows zero members after it; V4a a program read that begins after the last program batch and before admission shows the canonical final identity in the R19 order with zero members; V4b the last policy-route read after the clear and before admission shows the route present, and the final read agrees; V4c the same for the R18 guard table, if R18 stands (the route and guard are sampled about every 50 ms, so a boot whose clear-to-admission window is shorter than one sample fails closed rather than passing — a 08-02 review item if it fires); V5 admission opens; V6 no stale member survives; plus the first new lease is `100.95.0.2` and the old TAP, links, pins, endpoint entry, and guard member are absent before admission (absorbs the deleted placeholder `native_prior_vmm_reclamation_precedes_full_attachment_sweep_and_first_lease_acceptance`). `nft monitor` ordering is the kernel oracle; `EbpfDataplane`'s destructor still runs in killed mode (FD § "[REF] Boot ordering (D-295-R12) — ACCEPTED 2026-09-24" (the native-lane note on `EbpfDataplane`'s destructor)). |
| Seed / isolation | example; the body asserts euid 0; `host-kernel-shared`; timeout override retained |
| Rust home | `crates/overdrive-cli/tests/integration/serve_killed_restart_boot_clear.rs::a_killed_serve_reboot_reclaims_clears_stale_intercept_members_then_admits` |
| Disposition / step | RETARGETED (lands in place) — 08-02 |

#### S-ND295-13D — The fresh intercept owner refuses to start when stale members cannot be cleared

`@error @contract-shape:bounded-change`

```gherkin
GIVEN a fresh process whose shared protection sets hold stale members
WHEN the intercept owner starts
THEN it converges the members to empty before reading the program
AND a failed clear refuses startup with a member-clear cause, publishes nothing, and leaves the gate closed
AND a clear that commits after a refused boot publishes nothing
```

| Field | Value |
|---|---|
| Discharges | E10; G1/r2, G1/r4; D-295-R12 (FD § "[REF] Boot ordering (D-295-R12) — ACCEPTED 2026-09-24" (the boot member clear)), `MtlsSharedOwnerError::BootMemberClear` |
| Contract shape | bounded-change |
| Lane | integration (worker integration binary over a recording `MtlsIntercept` whose `bind_transparent` binds real loopback listeners; no root) + in-process (the composed boot: control-plane integration binary, Lima root because R16 composes the real kTLS probe; DISTILL review H6) |
| Driving port | `MtlsInterceptWorker::start_shared_owner`; composed: `run_server_with_obs_and_driver` whose `ServerConfig.mtls_intercept` is a test-local recording `MtlsIntercept` over `SimMtlsIntercept` seeded with stale members |
| Fault stimulus | the recording intercept's `converge_allocation_elements` fails (the kernel's `EBUSY` batch refusal as its own error), or returns non-empty members; composed: the same two, plus a clear batch the test lands after the boot has refused (the late commit) |
| Oracle | worker: call order `converge_allocation_elements(∅)` first; `Err(BootMemberClear { source })` whose source is the clear's own error, or `InterceptError::MembersRemain { observed }` with `observed` exactly the stale members (DR-08 (a)); no listener task, guard, or publication. Composed: `run_server*` returns `Err(MtlsBoot(SharedOwner(BootMemberClear { source })))` with that exact source, `health.startup.refused` is emitted, the EXEC gate is still BootClosed, no attachment is provisioned, no bind, observe, or converge follows the clear, and the members are unchanged; after the late commit the gate stays BootClosed and no new intercept call arrives within 200 ms (G-295-1 row 4). The component of `BootMemberClear` (`IpSets`) is 08-03's `component()` SSOT and is asserted in S-ND295-61's component table, not here |
| Rust home | NEW `crates/overdrive-worker/tests/integration/netns_density_shared_owner.rs::{a_fresh_owner_clears_stale_members_before_reading_the_program, a_failed_member_clear_refuses_startup_without_publication}`; NEW `crates/overdrive-control-plane/tests/integration/boot_member_clear_refusal.rs::{a_rejected_boot_member_clear_refuses_the_composed_boot_with_its_own_cause, a_boot_member_clear_that_leaves_members_refuses_with_the_members_it_observed, a_boot_member_clear_that_commits_after_the_refusal_publishes_nothing}` |
| Disposition / step | NEW — 08-02 (the composed bodies consume `ServerConfig.mtls_intercept`, so before 05-01 they fail on the unconsumed port; they are RED for their own contract only from 05-01 on) |

#### S-ND295-14 .. S-ND295-18 — Restart preserves protection while refreshing listener targets; refuses ambiguous state; rolls back source-honestly

`@real-io @error @contract-shape:bounded-change`

Contracts unchanged (D-295-DISTILL-15). R19 changes the canonical rule tail
order inside the production codec; bodies compare identities built through the
production constructors, so no body changes shape.

| Field | Value |
|---|---|
| Discharges | G1/r2, G1/r3 |
| Rust home | `crates/overdrive-worker/src/mtls_intercept_port.rs::shared_program_rollback_acceptance::{shared_program_post_commit_failure_rolls_back_source_honestly_for_every_prior, shared_program_replace_refusal_idempotence_and_guard_cleanup_preserve_complete_state_delta, shared_program_prior_snapshot_mismatch_preserves_complete_state_and_complement, runtime_present_wrong_target_and_observe_error_are_non_mutating}`; `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs::{shared_program_absence_create_readback_idempotence_and_guard_drop, shared_program_replaces_only_listener_targets_and_preserves_foreign_complement, shared_program_valid_wrong_target_observation_is_non_mutating}`; worker acceptance owner-refusal bodies |
| Disposition / step | RETAINED — active. The two ignored bodies `mtls_intercept_port::shared_program_rollback_acceptance::{replacement_and_every_rollback_disposition_preserve_exact_identity_and_source, fresh_replace_exact_prior_rollback_and_idempotent_reapply_are_complete}` (`#[ignore = "superseded by D15 stateful shared-IP evidence"]`) are DELETED |

### Group H — Intercept program, members, and fail-closure (gap 3; R15, R18, R19)

#### S-ND295-61 — Lost members, policy route, or guard are detected within a second and repaired with live workloads

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN live allocations whose protection members are installed
WHEN one member, the whole program table, the mark routing rule, the local route, or the mark guard table is deleted
THEN the worker's audit reports the program family or the member family as unhealthy
AND repair restores exactly the deleted object without rewriting a program whose identity differs
AND the prior node guard is handed over, not dropped, so the recorded targets survive and a new allocation still installs
```

| Field | Value |
|---|---|
| Discharges | E13 (sim + Lima); E11 policy-route-only case (worker half); D-295-R15 (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the runtime member audit and repair contract, R15)), R18 guard presence (conditional) |
| Contract shape | bounded-change |
| Lane | integration (worker integration binary over a recording `MtlsIntercept` that binds real loopback listeners; no root) + lima-kernel (real nft and routing through `HostMtlsIntercept`) |
| Driving port | `MtlsInterceptWorker::{audit_shared_owner, converge_shared_owner}`; `MtlsIntercept::{observe_shared_state, converge_allocation_elements}`; `Client::local_route_present` |
| Fault stimulus | sim: the recording intercept's state loses a member / `policy_route=false` / `intercept_mark_guard=false` / program absent / program with a different target; Lima: real `nft delete element`, `nft delete table ip overdrive-mtls`, `ip rule del fwmark 0x1 lookup 100`, `ip route del local 0.0.0.0/0 dev lo table 100`, `nft delete table ip overdrive-mtls-guard` |
| Oracle | audit errors, each with its exact typed source (DR-08 (a)): member mismatch → `MemberMismatch { expected, observed }` with exact sets (`component() == IpSets`); fwmark rule or local route lost → `Intercept { source: PolicyRouteAbsent }`; guard table lost → `Intercept { source: InterceptMarkGuardAbsent }` (R18-conditional); program absent → `Intercept { source: PostconditionMismatch { expected: <recorded>, observed: None } }`; program with a different target → the same with `observed: Some(<wrong>)` (all `IpRules`); repair: absent program → `converge_shared(None, F, C)`; equal identity → no program write; different identity → `PostconditionMismatch` without write (S19-A); then `converge_allocation_elements(registry_expected)` and a clean audit; guard handover: the prior guard's `Drop` never runs, recorded targets intact, a later `install_outbound` succeeds; `MtlsSharedOwnerError::component()` mapping table equals FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the new worker error variants), including `Intercept` over `PostconditionMismatch`, `PolicyRouteAbsent`, or `InterceptMarkGuardAbsent` → `IpRules`, and `BootMemberClear` over the clear's own error or `MembersRemain` → `IpSets` (the S-ND295-13D component clause lives here, with 08-03's `component()`; DISTILL review H6) |
| Seed / isolation | table + example; `host-kernel-shared` for Lima |
| Rust home | NEW `crates/overdrive-worker/tests/integration/netns_density_shared_owner.rs::{member_loss_is_an_ipsets_failure_and_repair_restores_exactly_the_member, policy_route_loss_is_repaired_with_live_members_and_the_prior_guard_is_relinquished, a_differently_targeted_program_is_never_rewritten}`; NEW `crates/overdrive-worker/src/mtls_intercept_worker.rs::tests::every_shared_owner_error_reports_its_one_component`; NEW `crates/overdrive-worker/tests/integration/shared_intercept_members.rs::{each_deleted_intercept_object_is_restored_exactly_with_live_allocations, the_intercept_mark_guard_table_is_restored_exactly_with_live_allocations}` (the second is R18-conditional) (the route and guard cases exercise `Client::local_route_present` and the two guard effects through `HostMtlsIntercept::observe_shared_state`) |
| Disposition / step | NEW — 08-03 (the R18-conditional body and guard assertions are removed by 08-01 if R18 is withdrawn) |

#### S-ND295-62 — Intercept-marked guest TCP is dropped even without the program table

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a Running mesh guest, the bridge guard intact, and host forwarding enabled
WHEN the intercept program table is deleted and the guest sends TCP to a peer guest, to the bridge gateway, or to another host address with a wildcard listener
THEN no marked frame reaches the peer's TAP and no host listener accepts the connection
AND deleting only the guard table still leaves the intercept program catching or dropping the traffic
```

| Field | Value |
|---|---|
| Discharges | E14 (a), (b), guard-only deletion; D-295-R18 (conditional on this native RED, FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the conditional parts)) |
| Contract shape | bounded-change |
| Lane | native (RED first, then GREEN) |
| Driving port | `serve::run_with_kek` + `deploy` with a probe guest image that emits scripted SYNs |
| Fault stimulus | real `nft delete table ip overdrive-mtls`; separately `nft delete table ip overdrive-mtls-guard`; pre-test `net.ipv4.ip_forward` recorded and set to 1 as a declared precondition; the probe also dials a real host interface address (the `prefsrc` of `ip route get 1.1.1.1`), not a TEST-NET address |
| Oracle | peer-TAP capture: zero forwarded intercept-marked frames; host listener on `0.0.0.0:<port>` accepts nothing, stays healthy (its accept thread alive, no accept error), and no SYN-ACK reaches the guest. **Healthy baseline (before any fault, the guard's non-interference control):** with both tables present and leg F listening, each R18 guest SYN (to the peer's address, the bridge gateway, and the host address, at a wildcard host listener's port) receives a SYN-ACK and that listener accepts nothing — the intercept answered and the guard dropped nothing. The table-loss body reads no guard; the guard-only body checks `observe_intercept_mark_guard()` is `Ok(true)` and a gateway SYN-ACK before its fault. The guard rule carries no counter (`observe_intercept_mark_guard` is a bool presence read). **Per-run SYN-entered-host check (every R18 GREEN case and the guard-only case):** the probe SYN is captured on its sender's TAP while that TAP reads back administratively up, `GuestTcxCounter::Intercept` (read with a hard error, never defaulted) rises by at least the SYNs sent, and the D9 bridge guard's `DefaultDrop` counter, read over netlink, is unchanged; a run whose TAP was already quiesced (e.g. an `IpRules` loss quiesced the managed TAPs) is void, not GREEN (DISTILL review DR-04, M11) |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-cli/tests/integration/intercept_mark_fail_closed.rs::{marked_guest_tcp_is_neither_forwarded_nor_delivered_without_the_intercept_program, the_intercept_program_still_catches_marked_tcp_without_the_guard_table}` |
| Disposition / step | NEW — 08-01 (the RED run decides R18: if neither table-loss path reproduces, R18 is withdrawn and its conditional parts are removed, FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the conditional parts' withdrawal rule)). **H3 resolved (2026-10-01):** the E14 (a) zero-forwarded oracle now carries a positive witness (a guest-TAP capture of the guest's own SERVICE_PORT peer-dial SYNs, asserted `>= 1`) and `PACKET_STATISTICS` drop accounting (asserted 0); the `SynCapture` is created with protocol 0 and only `bind` sets `ETH_P_ALL` on the target ifindex (no pre-bind contamination); `tap_is_up` for both the guest and peer TAPs and `ip_forward == 1` are asserted at the fault point, not only at the end; and the intercept table is asserted still absent across the whole window (no supervisor repair spanned it). The guard-only control body gained the same fault-point `tap_is_up` and no-repair witnesses (guard gone, intercept present, at the fault point and across the window). Classified RED at the 05-03 CH `Tap::enable` EPERM guest-boot baseline on metal (`red-classification.md` Phase G, G6-N02) |

#### S-ND295-63 — Guest TCP to an absent listener fails closed

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a Running mesh guest with the program present
WHEN the outbound listener is closed with the TAP up, or the server is killed while the VM stays up
AND the guest sends TCP to the gateway or to an address outside every managed set, at a port where a host wildcard listener waits
THEN no SYN-ACK reaches the guest and the host listener accepts nothing
AND an inbound SYN to a registered destination with its listener closed is dropped
```

| Field | Value |
|---|---|
| Discharges | E14 (c), (d), inbound control; G5/r5; D-295-R19 (conditional on this native RED) |
| Contract shape | bounded-change |
| Lane | native (RED first, then GREEN) |
| Driving port | as S-ND295-62; killed mode through `ServeSignal::Kill` |
| Fault stimulus | the leg-F listener is destroyed from outside with a sock-diag destroy on its exact listening tuple (`ss -K -l -n "src 127.0.0.1 and sport = :<leg F>"`, the port read from the owned program's target; the destroy must print the killed tuple) and its port is immediately occupied by a plain, non-transparent listener, which holds the listener-absent state until the supervisor's deadline (the S-ND295-31B port-theft shape); separately, killed-mode `serve` with CH alive (a residue guard kills the orphan on every exit); inbound: leg C's loopback listening tuple destroyed the same way and its port taken by a thief listener, then a guest-originated SYN to the peer's registered destination |
| Oracle | no SYN-ACK at the probe; wildcard host listener accept count 0, the listener healthy; inbound: zero forwarded frames on the peer's TAP, no SYN-ACK at the probe, and the thief accepts nothing (DISTILL review DR-04, M11) |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-cli/tests/integration/intercept_mark_fail_closed.rs::{outbound_tcp_to_a_closed_listener_is_dropped_not_delivered_locally, outbound_tcp_after_a_killed_server_is_dropped_while_the_vm_lives, inbound_tcp_to_a_closed_listener_is_dropped}` |
| Disposition / step | NEW — 08-01 (withdraw R19 if neither outbound case reproduces) |

#### S-ND295-64 — The TIME_WAIT side door is measured with both controls first

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a completed intercepted connection whose guest finished its own close, leaving the host side in TIME_WAIT
WHEN the outbound listener is gone and the guest reconnects from the same port with a newer sequence number while a host wildcard listener waits
THEN the platform records whether the reconnect is answered
AND a stale-sequence probe on a host-only path is answered with a bare ACK and a newer-sequence probe on that path reopens, before the guest case runs
```

| Field | Value |
|---|---|
| Discharges | E14 (e) with negative then positive control (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the `TIME_WAIT` side door); FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E14 row)); G5/r5 |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | as S-ND295-62; controls use a test-owned veth peer namespace under `overdrive_testing::cidr_lease::TestCidrLease` |
| Fault stimulus | controls: the host-only TIME_WAIT path in the test-owned namespace; guest door: killed-mode `serve` with the guest's leg-F connection in TIME_WAIT (matched on its original-destination tuple), a door listener bound on `0.0.0.0:<service port>`, and the guest reconnecting from the same port with a newer sequence number |
| Oracle | negative control: bare ACK, no SYN-ACK, entry survives; positive control (after `tcp_invalid_ratelimit`): SYN-ACK; guest case: an in-run witness requires at least one newer-sequence guest SYN on the guest's TAP, and the outcome is recorded — the door listener accepts nothing and nothing reopens, or a reopen is reported. **A reproduced SYN-ACK is not absorbed: it routes to the user (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the `TIME_WAIT` side door's routing to the user)).** |
| Seed / isolation | example; `host-kernel-shared`; named CIDR lease |
| Rust home | NEW `crates/overdrive-cli/tests/integration/intercept_mark_fail_closed.rs::{both_time_wait_controls_prove_the_substate_and_sequence_gates, a_guest_reconnect_into_its_leg_f_time_wait_entry_is_recorded_and_a_reopen_goes_to_the_user}` (the former single name `a_newer_sequence_reconnect_into_time_wait_is_recorded_after_both_controls` was never committed under that name; DISTILL review B3 (d)) |
| Disposition / step | NEW. The controls body is **active**: it pins door-independent kernel behaviour and passes on metal (`red-classification.md` Phase G, G4-N07 and G6-N01; DISTILL review B3 (a)). The guest door is 08-01: it depends on R19, whose decision is 08-01's (DISTILL review B3 (b), DR-09). **B3 (c) resolved (2026-10-01):** the guest crafter no longer falls back to source port 0 — a failed or zero `local_addr()` read exits non-zero (29), so every crafted reconnect SYN carries the real leg-F TIME_WAIT source port and the in-run witness (`>= 1` newer-sequence guest SYN on the guest's TAP, base `TW_CRAFT_SEQ_BASE`) is no longer vacuous. The hand-built raw-SYN crafter is otherwise unchanged. Classified RED at the 05-03 CH `Tap::enable` EPERM guest-boot baseline on metal (`red-classification.md` Phase G, G6-N03) |

#### S-ND295-19 — Live repair never redirects protection to a different listener

`@error @contract-shape:bounded-change`

```gherkin
GIVEN the node owner is running and one owned rule names a different listener target
WHEN the supervisor detects it and runs its bounded recovery
THEN the program is observed without mutation and never rewritten
AND TAPs are quiesced once, and each of twenty attempts is one worker repair followed by one full audit
AND at five seconds exactly one typed program-rules deadline fail-stop is requested
```

| Field | Value |
|---|---|
| Discharges | E11 seeded (S19 journal, FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the S19 consequence)); G5/r2; D-295-R13 |
| Contract shape | bounded-change |
| Lane | deterministic in-process (source-local supervisor, default lane, one fixed `SimClock` schedule: the worker's `S19Intercept` delegates binding to an inner `SimMtlsIntercept`, so the started worker binds nothing) + pure (S19-A adapter layer, RETAINED) |
| Driving port | private `SharedNetworkSupervisorHandle::run_shared_network_supervisor(ports, exec, clock, request_tx, shutdown)` with the ports of § *Test-local control-plane ports*: the test-local owner (it replaces the file-local `S19SharedOwner`, `lib.rs:1819-1888`, which is deleted so the owner double is defined once), the test-local `GuestDns`/`GuestDnsFactory`, a worker over `SimMtlsEnforcement` and `SimMtlsResolve`, and `vm_kill` over `CgroupManager::new(<root>, Arc::new(SimCgroupFs::new()))` |
| Fault stimulus | the worker's intercept (test-local `S19Intercept`) observes a canonical wrong target |
| Oracle | the test-local owner's journal and the intercept's call log, with the DNS audit stamped against both: detection = one full audit (`AuditShared`, one `ObserveSharedState` after it, the DNS audit last); one `Quiesce`; each of the 20 attempts is exactly one worker repair then one full audit — two `ObserveSharedState` (the repair's read before the shared audit, the audit's read after), one shared-owner audit call, the DNS audit last, and no program write: the repair observes the wrong target and rewrites nothing, which is the `PostconditionMismatch` refusal's observable (DISTILL review H11); no `Restore`; request `IpRules / RecoveryDeadlineExceeded / 20 / 5 s`; no attempt 21; the pending-poll synchronization of the existing body is kept; terminal ownership stays with `ServerHandle::shutdown` |
| Seed / isolation | deterministic `SimClock` schedule; every verdict prints the schedule id (`s19-wrong-leg-f-target`), not an inert seed (DISTILL review L2) |
| Rust home | `crates/overdrive-control-plane/src/lib.rs::shared_network_task_owner_acceptance::published_wrong_shared_target_retries_on_production_cadence_and_emits_one_typed_fail_stop` (RETARGETED journal and entry point); S19-A bodies in S-ND295-14..18 RETAINED |
| Disposition / step | RETARGETED — 09-01 |

#### S-ND295-20 .. S-ND295-26, S-ND295-31A, S-ND295-31B — Node-shared listener and capability lifecycle; exact-port rebind; port theft

`@real-io @error @contract-shape:bounded-change`

Outcome: the node's shared protection listeners keep protecting every live
workload through allocation stops, owner shutdown, a lost listener, and a
stolen port, and never serve a stopped allocation.

Contracts unchanged (C-295-L, D-295-DISTILL-7/11). Normal stop now removes
members through the awaited `remove_allocation_elements` (S-ND295-54); the
observable complements these bodies assert are unchanged. B-7 changes how the
worker obtains and stops its listeners, not these contracts (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on R13 to R15 and the listener-loss audit)).

**Pass-through relay stop obligations (C-295-L pin, 2026-09-25, feature delta
§ "Why a stop ends its pass-through relays").** The live twin
`shared_allocation_stop_joins_a_passthrough_child` asserts one case — stop
closes both legs of a relay established before the stop began. Two clauses of
the pin it does not cover each gain a NEW worker body: (1) a relay whose
connection is classified WHILE the stop's claim wait is in progress — stop
returns only after both of its legs are closed and no relay of that generation
remains; (2) `shutdown_owner` with a live relay — when it returns, the relay's
legs are both closed. An implementation that ends relays before the claim
wait, or registers a relay after releasing its claim (the current
`handle_shared_outbound` detach, `mtls_intercept_worker.rs:3563-3567`), passes
the twin and fails these. The shared dispatch's detach is the review item C-295-L
names for the step that lands shared stop (05-01).

| Field | Value |
|---|---|
| Discharges | G2/r4 (late success cannot resurrect a terminal); G5/r5 (listener loss) |
| Contract shape | bounded-change |
| Rust home | `crates/overdrive-worker/tests/integration/netns_density_shared_owner.rs` (ten bodies; moved from `tests/acceptance/`, integration lane), `crates/overdrive-worker/src/mtls_intercept_worker.rs::{shared_listener_task_owner_acceptance, capability_registry_acceptance, tests}::*` including NEW `tests::{shared_allocation_stop_ends_a_relay_classified_during_its_claim_wait, shared_owner_shutdown_ends_a_live_relay}` (the two C-295-L pass-through relay-stop obligations, pending `05-01`), native `crates/overdrive-worker/tests/integration/outbound_enforce_substrate_splice.rs::{two_real_shared_capabilities_keep_the_unrelated_tls_handle_live_after_one_stops, real_owner_shutdown_closes_admission_waits_one_claim_and_drains_every_shared_handle}` (both assert root instead of skipping; they pass on metal, `red-classification.md` Phase G) |
| Disposition / step | RETAINED — active. Every test double implementing `MtlsIntercept` gains faithful implementations of the three new methods (mechanical fallout). The nine acceptance bodies keep their assertions and their real client connections: from the B-7 step `RecordingSharedIntercept` returns a `LoopbackInterceptListener`. The worker's `tests::*` bodies that drive the per-allocation listener branch are deleted by the step that deletes the branch; each surviving contract is carried as § *Intercept listener and stop-error test support* tabulates (shared-allocation twins pending `05-01`). `crates/overdrive-reconcilers/src/workload_lifecycle.rs::service_projection_keeps_first_tcp_order_deduplicates_tcp_and_excludes_udp` (S-ND295-21) is `pending DELIVER step 07-01 (S-ND295-21)`: 07-01 owns the PORT-295-C projection — `ServiceV1::listen_ports()` keeps TCP listeners only and emits each port once, in first-TCP-listener order: `8080/tcp, 53/udp, 8080/tcp, 8443/tcp, 8080/udp` projects to `[8080, 8443]` (FD § "[REF] Required downstream changes (not edited by DESIGN)" (the Service listener projection, H2)) |

#### S-ND295-71 — Protection is installed only against the node's own converged program

`@error @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN a node whose protection program is established only by a successful convergence
WHEN an allocation's protection elements are installed or removed
THEN a call before any convergence is refused as not-yet-converged and changes nothing
AND an install at a port other than the recorded listener target is refused as a port mismatch
AND a convergence from a stale prior, or with a zero listener port, is refused and changes nothing
AND dropping the node's holder with no member present removes the program, and with a member present keeps it
```

| Field | Value |
|---|---|
| Discharges | B-8 element precondition (FD § "[REF] Driven port — intercept element precondition (DISTILL gap B-8) — pinned 2026-09-26" (the pinned contract; the simulation adapter)); the host/sim divergence the phase-C run recorded (`distill/red-classification.md`, blocker 4) |
| Contract shape | bounded-change |
| Lane | pure (sim self-tests, source-local) + lima-kernel (the port equivalence harness on both adapters, root) |
| Driving port | `MtlsIntercept::{converge_shared, install_outbound, install_inbound, remove_allocation_elements, observe_shared, observe_shared_state, converge_allocation_elements}` on `SimMtlsIntercept` and `HostMtlsIntercept` |
| Fault stimulus | an install or removal before `converge_shared`; an install at the crossed leg's port; a convergence from an absent or stale `prior`; a zero listener port; the drop of a node guard with and without a live member (its element guard relinquished first); on the sim, an armed element-update fault (`SimInterceptFault::NftElementUpdate`) exercised only after the record and port checks pass |
| Oracle | refusal 1 `SharedProgramNotConverged` (source-less), no state change; refusal 2 `SharedListenerPortMismatch { leg, expected, actual }` naming the recorded and passed ports; a stale `prior` `PostconditionMismatch { expected, observed }`, a zero port `NftRuleInstallFailed { op: "shared-ip-expected" }`, both non-mutating; a no-member node-guard drop leaves `observe_shared`/`observe_shared_state`/`converge_allocation_elements` at `Ok(None)` (D15's conditional delete, which the sim models); a member-present drop (member relinquished, not dropped) keeps the program; the sim's `converge_shared` refuses in the host's order (zero port, then armed fault, then stale prior, then replace-over-members); a failed `converge_shared` keeps the record so a later install at the recorded port still succeeds and a removal is not refused for want of a record. Both adapters return the same partition on the same call sequence |
| Seed / isolation | finite tables + examples; the equivalence bodies run as root in the `host-kernel-shared` `overdrive-worker` integration binary; the sim self-tests are default-lane |
| Rust home | `crates/overdrive-sim/src/adapters/mtls_intercept.rs::tests::{shared_convergence_records_both_exact_targets_for_non_repairing_observation, a_node_guard_dropped_while_a_member_exists_keeps_the_program_and_withdraws_the_record, converging_to_the_recorded_program_adopts_it_and_keeps_every_member, a_program_replacement_is_refused_while_a_member_exists, a_convergence_from_a_stale_prior_is_refused_and_changes_nothing, shared_convergence_refuses_in_the_hosts_order, a_failed_convergence_keeps_the_recorded_program, an_armed_install_fault_fires_only_after_the_record_and_port_checks}`; `crates/overdrive-worker/tests/integration/mtls_intercept_equivalence.rs::{an_install_before_convergence_is_refused_and_changes_nothing, an_install_at_a_port_other_than_the_recorded_target_is_refused, a_node_guard_dropped_with_no_members_leaves_no_program, a_convergence_from_a_stale_prior_is_refused_and_changes_nothing, a_zero_listener_port_is_refused_before_any_program_change, a_node_guard_dropped_with_no_members_leaves_no_member_state, a_node_guard_dropped_while_members_exist_keeps_the_program}`. The two S-MIF-11/12 install bodies (`both_installs_hand_back_a_guard_that_releases_cleanly`, `re_installing_the_same_capture_converges_and_both_guards_release_cleanly`) converge before installing and pass today on both adapters (the phase-C C-14 divergence closed) |
| Disposition / step | NEW — 05-01, apart from the two member-aware equivalence bodies (08-02, when the host's `observe_shared_state` stops being a `todo!`). The sim self-tests activate at 05-01; the equivalence refusal/program bodies too. The two install-`Ok` bodies are active |

#### S-ND295-72 — A managed link is correct whatever the host's link configuration

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN the node bridge and every guest TAP are managed links, on a host whose link manager may rewrite a link's address, or may not run at all
WHEN the owner reads a managed link back, at bridge creation or boot, at provision, at activation, or in an audit
THEN the bridge carries its address from creation, and a bridge identity mismatch names the observed address and up state
AND a TAP holding no address, the bridge's address, or the guest MAC of a VM the node still holds is refused at provision, refused at activation, or is that VM's damage alone in the audit
AND a TAP moved to any other address, whoever wrote it, is not damage
```

| Field | Value |
|---|---|
| Discharges | E22 (FD § "[REF] Managed-link identity independent of host link configuration (fresh-host RCA) — pinned 2026-09-26; user rulings of 2026-09-28"; FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" (the host-side MAC invariant); FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E22 row)); the fresh-host RCA root cause A (`docs/analysis/root-cause-analysis-netns295-fresh-bridge-boot-refusal.md`) |
| Contract shape | bounded-change |
| Lane | pure (source-local D12A tables over the fake attachment kernel: (i1), (i2)) + lima-kernel (`ensure_bridge` create/adopt, (a)/(b); the owner's bridge audit, (d); the owner's TAP audit after udev, (e)) + native ((e) on the metal host as provisioned, the host's `systemd --version` recorded (printed through `systemctl --version`); (c) through S-ND295-00) |
| Driving port | `Client::ensure_bridge(name, mac)`; `SharedGuestNetworkOwner::{converge_shared, provision, audit_shared}` and `GuestNetworkProvisioner::activate` on the one host owner (over the D12A leaves source-locally; over the production allocation I/O on Lima and metal) |
| Fault stimulus | an out-of-band `ip link set … address`/`… down` on `ovd-gbr0`; a TAP's host-side MAC set, from creation or later, to each reserved class — another held allocation's guest MAC (held `Active`, held `ProvisionedDown`), the TAP's own guest MAC, `GUEST_BRIDGE_MAC` — or to no address; an unreserved address (`fe:95:de:ad:00:01`, `62:a6:95:00:00:02`, `02:95:72:00:00:0d`); a TAP holding a `Condemned` allocation's guest MAC; a guest MAC a TAP took before that guest's allocation was held; on Lima and metal, systemd-udevd's own write where it runs |
| Oracle | (a) a created bridge reads back `addr_assign_type` 3 and the requested address, down, and the create-then-set control is recorded only; (b) `ensure_bridge` on a present link of any kind writes nothing; (c) S-ND295-00's fresh-host read-back is `GUEST_BRIDGE_MAC`; (d) a bridge mismatch reports the observed MAC and up state, never `observed: None`; (i1) each reserved class and a missing address is `PostconditionMismatch { operation: TapObserve, expected: TapHostMac { ifindex, address: Unreserved }, observed: Some(TapHostMac { ifindex, address: Reserved(<mac>) \| Missing }) }` — at provision, with each break written from creation and again after the egress step (step 6) has run, so that only the step-7 read-back can see it, publication is refused, the TAP is never raised, every part is rolled back, and the held attachments stay byte-equal; at `activate` it is refused with zero mutating leaf calls and the node unchanged, and the attachment still activates once the address is restored; in the audit exactly that allocation is named, the other is not, and nothing is written; the ordering case: a TAP that took a guest MAC before that guest's allocation was held passes every audit until the allocation is provisioned, and the first audit after that names the TAP's allocation alone; (i2) an unreserved address passes provision (written at either point), `activate`, and every audit, including audits that each follow a move of both TAPs to unreserved addresses they did not carry at the previous audit (each audit still reads the TAP it judges), and a TAP holding a `Condemned` allocation's guest MAC is not damage; (e) with two allocations provisioned by the production owner: after udev (where it runs) has initialized the first TAP, the audit reports no damage, whatever address udev left; an out-of-band unreserved write is no damage; the second allocation's guest MAC on the first TAP is that TAP's `TapHostMac` damage alone (`Reserved(<mac>)`), and the second allocation and the node stay healthy. No case installs, reads, or requires a host link-configuration file, and the startup probe reads no scratch-TAP address (user ruling 4) |
| Seed / isolation | finite tables + examples; the (i1)/(i2) tables are default-lane over the fake attachment kernel; scratch names outside `ovd-gbr*`/`ovd-tp-*` for (a)/(b); node-global names for (d)/(e) → the source-local kernel module is in `host-kernel-shared`; the netlink integration binary is `host-kernel-shared` |
| Rust home | `crates/overdrive-netlink/tests/integration/managed_link_address.rs::{a_created_bridge_carries_its_address_from_creation_and_starts_down, ensure_bridge_adopts_a_present_link_of_any_kind_without_writing}`; `crates/overdrive-control-plane/src/guest_network.rs::allocation_owner_acceptance::{a_reserved_or_missing_host_side_address_refuses_publication, a_reserved_or_missing_host_side_address_refuses_activation_before_any_change, a_reserved_or_missing_host_side_address_is_that_allocations_audit_damage, an_unreserved_host_side_address_is_not_audit_damage_whenever_it_changes, a_tap_holding_a_condemned_guests_address_is_not_audit_damage, a_guest_address_a_tap_took_early_is_damage_from_the_first_audit_after_that_guest_is_held}`; `crates/overdrive-control-plane/src/guest_network.rs::shared_owner_link_address_kernel::{a_bridge_identity_mismatch_names_the_observed_address_and_up_state, a_tap_host_address_is_judged_by_the_invariant_whatever_the_host_link_manager_wrote}` (the latter re-authored from `a_provisioned_taps_recorded_address_survives_udev_initialisation`); the S-ND295-00 bridge-identity leg is the `run_server` read-back (E22 (c)). S-ND295-50's and 51's per-part tables carry a reserved-address row (the TAP's own guest MAC) and a missing-address row. DELETED with the user rulings of 2026-09-28: the three `scratch_probe_acceptance` bodies (`an_unchanged_scratch_tap_address_passes_the_probe_between_two_reads`, `a_probe_that_fails_before_the_last_exercise_reads_the_scratch_tap_once`, `a_changed_scratch_tap_address_refuses_startup_and_still_cleans_up`), their scratch-TAP read support, and `PacketProbeIo::read_scratch_tap` |
| Disposition / step | NEW — 05-00 for the bridge ((a), (d), and S-ND295-00's (c)); `ensure_bridge`'s adopt body (b) is active (today's create-or-adopt already adopts without writing); 06-02 for the TAP invariant at provision and in the audit ((i1), the unreserved leg of (i2), (e)); 06-04 for (i1) at `activate` and for (i2)'s `Condemned` exclusion, which needs the audit's condemnation (R5) |

#### S-ND295-70 — The node's protection listeners belong to the protection port: a simulated node opens no socket, and a listener stops when its wait is cancelled

`@error @real-io @adapter-integration @contract-shape:bounded-change`

```gherkin
GIVEN a node whose protection listeners come from its protection port
WHEN the node starts, waits for guest connections, loses a listener, or shuts down
THEN a simulated node opens no socket and waits until a connection or a failure is delivered to it
AND a listener address is never handed out twice while held, and is free again once released
AND losing one listener ends only its own wait, and the node takes back the address it recorded
AND stopping the node ends every wait and releases both listeners without taking a waiting connection
AND on the host every accepted connection reports the address its peer originally dialled
```

| Field | Value |
|---|---|
| Discharges | B-7 port contract (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25"): `bind_transparent` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`bind_transparent` behaviour)), `local_addr` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`InterceptListener::local_addr` behaviour)), `accept` and cancel safety (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`InterceptListener::accept` behaviour)), host obligations (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `HostMtlsIntercept` obligations)), sim contract (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `SimMtlsIntercept` contract)), the worker's guarantees (C-295-L, FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (what the worker's use of it guarantees)), and the lane consequence (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification)); G5/r5 (listener loss) |
| Contract shape | bounded-change |
| Lane | pure (sim self-tests; worker source-local bodies over `TestInterceptListener`) + lima-kernel (the port equivalence harness and the host-listener obligations, root) |
| Driving port | `MtlsIntercept::bind_transparent` → `InterceptListener::{local_addr, accept}` on `SimMtlsIntercept` and `HostMtlsIntercept`; the worker's `start_shared_owner`, `wait_shared_owner_failure`, `audit_shared_owner`, `converge_shared_owner`, `shutdown_owner`, and the drop of its last `Arc`, over `TestSharedIntercept` |
| Fault stimulus | a bind at a held address; the sim and test-listener scripts `SimAcceptScript::{Connection, OriginalDestinationFailure, ListenerLost}` and `script_local_addr_failure`; an `accept` polled with no runtime; the drop of a pending `accept`; on the host, a sock-diag destroy of the listening tuple (`ss -K`) and an outbound connection diverted by the production shared program — `converge_shared`, `install_outbound(source, port)`, and the production guest TCX classifier attached to the workload link (the test-support `SharedOutboundDivert`), never a hand-installed per-interface TPROXY rule, which no production path installs under the shared design (DISTILL review H9) |
| Oracle | Sim: port-0 binds get `49152`, then `49153` while the first is held, at the requested IP; an exact non-zero address is honoured; a held address is refused with `InterceptError::TransparentListener { addr, source }` whose `raw_os_error()` is `EADDRINUSE`; `live_listeners()` drops the address when the last `Arc` drops and the exact address binds again; with nothing scripted `parked_accepts(at) == 1` and the future stays pending; dropping it gives `0`, and a `Connection` scripted afterwards is returned by the next `accept` (nothing taken by the cancelled one); each script completes exactly the `accept` it names, `ListenerLost` stands for every later `accept`, `OriginalDestinationFailure` leaves the listener usable; no-runtime `accept` returns `Err(Accept)` without consuming a script; `clear_faults` disarms the bind slot and an install fault leaves bind on its `Ok` arm (the S-MIF-08 and S-MIF-13 gaps). Worker: after `start_shared_owner` each leg has one parked accept; dropping the last worker `Arc` brings both to `0` and empties `live_listeners()` within 2 s; when `shutdown_owner` returns `Ok`, both are `0`, both listeners are released, and `node_guard_drops()` reads `0` (the guard is relinquished, D15); `ListenerLost { errno }` on leg F makes `wait_shared_owner_failure` return `TaskFailed { leg: F, source }` with that errno while leg C stays parked, and `converge_shared_owner` releases the dead listener before rebinding exactly the recorded leg-F address (never `EADDRINUSE` from its own listener), after which `audit_shared_owner` succeeds; `OriginalDestinationFailure` ends no task (the failure future stays pending, the leg re-parks); a `local_addr` failure on leg C makes `audit_shared_owner` return `ListenerLocalAddr { leg: C, .. }`. Equivalence (both adapters): S-MIF-09..12 unchanged in meaning; a held address is refused with `EADDRINUSE` until its last holder drops, then binds. Host: for an outbound connection the shared program diverts, `InterceptAccepted.local` is the destination the guest dialled and `peer` its source, and the classifier's `Intercept` counter advances for it; node-global sysctls are snapshotted, written with read-back, and restored by a guard, and every divert release step is asserted in reverse order; for an inbound one, `local` is the virtual address; a destroyed listening socket ends a pending `accept` with `Err(Accept)` within 2 s; an accepted descriptor has `O_NONBLOCK` clear and `FD_CLOEXEC` set |
| Seed / isolation | finite tables + examples; the worker bodies bound every wait at 2 s real time and advance no clock; the equivalence and host bodies run as root (`cargo xtask lima run`) in the `host-kernel-shared` `overdrive-worker` integration binary |
| Rust home | NEW `crates/overdrive-sim/src/adapters/mtls_intercept.rs::tests::{fabricated_ports_are_deterministic_non_zero_and_distinct_among_live_listeners, a_held_address_is_refused_with_eaddrinuse_until_its_last_holder_drops, an_unscripted_accept_stays_parked_and_a_cancelled_accept_takes_nothing, each_scripted_outcome_completes_exactly_the_accept_it_names, an_accept_polled_without_a_runtime_fails_instead_of_panicking, clear_faults_also_disarms_the_bind_slot, an_install_fault_leaves_bind_on_its_success_arm}`; NEW `crates/overdrive-worker/src/mtls_intercept_worker.rs::tests::{an_idle_accept_task_ends_when_the_last_worker_reference_drops, owner_shutdown_ends_both_accept_tasks_releases_both_listeners_and_relinquishes_the_node_guard, a_lost_listener_ends_only_its_own_task_and_the_owner_rebinds_the_recorded_address, a_connection_whose_destination_cannot_be_read_is_dropped_and_the_task_keeps_waiting, an_unreadable_listener_address_is_reported_by_the_audit}`; RETARGETED `crates/overdrive-worker/tests/integration/mtls_intercept_equivalence.rs::{bound_leg_reports_a_non_zero_kernel_assigned_port, two_bound_legs_never_share_a_port, both_installs_hand_back_a_guard_that_releases_cleanly, re_installing_the_same_capture_converges_and_both_guards_release_cleanly}` (the listener is read through `LegListener`); NEW `…::a_held_address_is_refused_with_eaddrinuse_until_its_last_holder_drops`; NEW `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs::{the_host_listener_reports_a_redirected_outbound_original_destination_as_local, the_host_listener_reports_a_redirected_inbound_virtual_address_as_local, a_destroyed_host_listener_ends_its_pending_accept_with_an_accept_failure, an_accepted_connection_is_blocking_and_close_on_exec}`, each accepting through `LegListener`; the outbound original-destination steps of `egress_tproxy_capture.rs` and `name_resolve_enforce_consistency.rs` RETARGETED to accept through the port (§ *Intercept listener and stop-error test support*) and to divert through `SharedOutboundDivert`, their witness now the classifier's `Intercept` counter; every root-needing body asserts root instead of skipping (DISTILL review M6) |
| Disposition / step | NEW + RETARGETED — 05-01. A body that passes on today's listener stays active (the equivalence clauses, the original-destination bodies, and possibly the descriptor-state body); the rest carry `pending DELIVER step 05-01 (S-ND295-70)` |

### Group I — Supervisor, serve boundary, and process lifetime (gaps 2 and 4; R13, R14, R16, R17)

#### S-ND295-27 / S-ND295-28 — One guest-command gate; recovery never leaks or wrongly releases a command

`@error @recovery @contract-shape:bounded-change`

Outcome: a guest command is released only through the node's one command gate,
and no recovery, cancellation, or late completion leaks a command or releases
one the gate refused.

Contracts unchanged (EXEC-close linearization, FD § "EXEC-close linearization"). The fail-stop
cause vocabulary gains `TapQuiescenceUndetermined` and `VmKillFailed`
(FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the core addition)).

| Field | Value |
|---|---|
| Discharges | G2/r4, G5/r4 (late completion cannot reopen) |
| Contract shape | bounded-change |
| Rust home | `crates/overdrive-core/tests/acceptance/netns_density_exec_gate.rs` (six bodies); `crates/overdrive-worker/tests/acceptance/netns_density_exec_release.rs` (three bodies) and the action-shim await/cancellation body |
| Disposition / step | RETAINED — active, except `netns_density_exec_gate.rs::every_fail_stop_cause_is_closed_and_first_request_wins`, RETARGETED: its `CAUSES` table gains `TapQuiescenceUndetermined` and `VmKillFailed`; the gate is cause-agnostic, so the body stays active and GREEN once phase B adds the two variants |

#### S-ND295-29A — Every shared-network component follows one bounded recovery contract

`@property @tier1 @error @contract-shape:bounded-change`

```gherkin
GIVEN each node-level component and task class of the shared network, one at a time and in pairs
WHEN the supervisor detects its loss by the one-second audit or by an immediate task exit
THEN it closes new guest commands before announcing the fault
AND it quiesces TAPs only for kernel-path components, never for a listener or DNS loss
AND each 250 ms attempt repairs the failing owners in order, then runs one full audit, then restores quiesced TAPs only if that audit is clean
AND it reopens exactly once after complete repair, or requests one typed fail-stop at five seconds or twenty attempts
AND a kernel-path loss found after a restore that failed part-way quiesces again before any further restore
AND while quiescence is latched the gate is never open
```

| Field | Value |
|---|---|
| Discharges | E11 seeded (all 12 components and task classes, IpRules-only and IpSets-only quiesce-repair-restore-reopen, policy-route-only with guard handover, double failure with no early restore, re-quiescence after a part-way restore (user-approved 2026-09-30), latch invariant L9; the activation-in-flight clause moved to the composed S-ND295-29B C9, where the supervisor drives the latch, DISTILL review M8); E17 seeded (DNS loss closes EXEC and recovers through a fresh responder); G5/r1, G5/r2, G5/r4, G5/r5; D-295-R13, R14, R15, R16 (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)"; FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the latch invariant, L9)) |
| Contract shape | bounded-change |
| Lane | seeded-sim (source-local, default lane; the started worker over `S19Intercept` binds nothing, § *Intercept listener and stop-error test support*) |
| Driving port | private `run_shared_network_supervisor(ports, exec, clock, request_tx, shutdown)` with `SharedNetworkSupervisorPorts { shared_guest_network: the test-local owner, mtls_worker: MtlsInterceptWorker over SimMtlsEnforcement + SimMtlsResolve + the test-local stateful S19Intercept, dns: DnsServeTaskOwner over the test-local GuestDns, dns_factory: the test-local GuestDnsFactory, dns_deps: SimObservationStore / SimClock / gateway / frontend, vm_kill: VmKillCapability::new(CgroupManager::new(<root>, Arc::new(fs.clone()))) over one SimCgroupFs }`, `GuestNetworkExecWiring` over `SimClock` whose `gate()` and `supervisor()` the test keeps (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the seeded-sim lane and its test-local ports); FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the seeded and in-process lanes' kill capability); § *Test-local control-plane ports*) |
| Fault stimulus | the test-local owner's standing component slots and damage set; intercept state loss (program/member/route/guard); listener task end — `S19Intercept::sim().script_accept(leg, SimAcceptScript::ListenerLost { errno: libc::EINVAL })` at the leg address the worker recorded, so the worker's accept task ends with `InterceptAcceptError::Accept` on a socket-free listener and the classifier reports `TaskFailed { leg, source }` (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (`accept`'s `Err(Accept { source })` outcome; the `SimMtlsIntercept` scripting surface)); repair releases that listener and rebinds the recorded address; the test-local `GuestDns` serve exit (return, panic, pending); the test-local owner's hanging or panicking audit; a restore that fails part-way after raising one or two TAPs (`script_restore_failure_after(n)`), followed by a second kernel-path loss the next audit reveals |
| Oracle | per cell: `begin_recovery(first component in D8 order)` precedes `guest_network.shared_owner_unhealthy { component, cause ∈ {audit_mismatch, task_exit, audit_timeout} }`; the test-local owner's journal shows exactly one `Quiesce` per kernel-path detection and none for LegF/LegC/Dns — in the re-quiescence cell the failed part-way restore leaves the latch set and EXEC closed, the second loss quiesces again with no restore in between and sets every TAP down, and after healing one successful `Restore`, one reopen, and a clear latch follow (episode order quiesce ≺ failed restore ≺ quiesce ≺ successful restore); a worker still due for repair gets non-empty worker writes stamped between its converge and the audit, and a worker that passed the latest audit is not repaired (M8); the DNS task's stop is required only for `DnsAuditFailed` (a still-running task), and none is pinned for a serve that returned or panicked (DR-18); attempts at 250 ms counted after converge + full audit; `Restore` only after a clean audit; single reopen; deadline → one request with exact component/attempts/elapsed. L9 on every schedule, read from the owner's own latch (`latched()`, set by `quiesce_managed_taps` and cleared only by a successful `restore_quiesced_taps`), never re-derived from a journal: at every observation point, `latched()` implies `supervisor.recovery_progress().is_some()` or a received fail-stop request, and `gate.claim_release().now_or_never()` yields no claim; the L9 walk opens with a seeded kernel-path loss and requires at least one latched observation, so it cannot pass without the latch ever set (M8) |
| Seed / isolation | finite matrix × `OVERDRIVE_SUPERVISOR_SEEDS` (defaults `0x2953300000000001`, `0x295330005eed0002`), seed and cell printed on every verdict, including every fixture `expect` (DISTILL review L2); deterministic `SimClock` with pending-poll synchronization |
| Rust home | NEW `crates/overdrive-control-plane/src/lib.rs::shared_network_task_owner_acceptance::{every_component_loss_is_detected_and_recovered_through_its_owner, kernel_path_components_quiesce_and_listener_or_dns_loss_never_does, a_worker_only_repair_still_restores_quiesced_taps_and_reopens, a_policy_route_loss_is_repaired_with_live_members_and_the_guard_is_relinquished, a_double_failure_raises_no_tap_before_every_owner_is_repaired, a_kernel_path_failure_after_a_part_way_restore_quiesces_again_before_any_restore, dns_task_loss_closes_new_commands_and_recovers_through_a_fresh_responder, the_gate_is_never_open_while_quiescence_is_latched}`; the test-local owner's self-test NEW `crates/overdrive-control-plane/src/shared_network_test_ports.rs::tests::a_part_way_restore_raises_some_and_a_repeat_quiescence_sets_them_down` (active). `an_activation_in_flight_waits_for_reopen_and_raises_once` is DELETED here (its activation client was test code emulating the shim) and re-authored in S-ND295-29B |
| Disposition / step | NEW — 09-01 |

#### S-ND295-29B — The composed server recovers every component through its required ports

`@driving_port @error @contract-shape:bounded-change`

```gherkin
GIVEN a server composed through its production entry with required protection and DNS ports
WHEN each shared-network component is lost
THEN admission closes within a second, repair runs through the owning component, and admission reopens once
AND an unrepairable loss ends in one typed fail-stop at five seconds, and a late recovery cannot reopen
AND a TAP that cannot be confirmed down stops only its VM, while an undetermined quiescence stops them all and fails the node
```

| Field | Value |
|---|---|
| Discharges | E11 in-process (proof §3.3 C0-C8, C6 re-targeted to the per-TAP kill scope, plus C4b and C9), E17 in-process; G5/r1-r5; D-295-R13, R14, R16 |
| Contract shape | bounded-change |
| Lane | in-process (Lima root; the real enforcement probe runs) |
| Driving port | `run_server_with_obs_and_driver(ServerConfig::new(kek, mtls_intercept, guest_dns), obs, driver, vm_host_state, shared_guest_network, guest_network_exec, vm_cgroups)` (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)) with a `SimDriver` behind the test-local `ProofDriver` decorator (it journals EXEC releases), the `SimSharedGuestNetworkOwner` behind the test-local `ProofOwner` (a journal and an activation hold; or a test-local hanging or panicking owner for those cells), `guest_dns` = `SimGuestDnsFactory` behind `ProofDnsFactory`/`ProofDns` (they record audits), and `vm_cgroups` = `CgroupManager::new(<root>, Arc::new(fs.clone()))` over one `SimCgroupFs` (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the seeded and in-process lanes' kill capability)). Admission, progress, and requests are observed as § *In-process observation* states |
| Fault stimulus | the 13 stimuli (the 11 node-level components, a DNS serve panic, and a DNS audit refusal) through `SimSharedGuestNetworkOwner` slots (`script_component_audit_failure`, `script_quiesce_outcome`, `script_audit_damage`, `script_restore_failure`); `ServerConfig.mtls_intercept` = test-local stateful intercept; `SimGuestDnsFactory::script_probe_failure`, `SimGuestDns::end_serve(Return / Panic)`, `SimGuestDns::script_audit_failure` (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` and `SimGuestDnsFactory` doubles)); test-local hanging and panicking audit owners. Before a C6 fault the test deploys allocation A through the API and creates the workloads slice and A's scope directory, with every ancestor, through `CgroupFs::create_dir` on the same `SimCgroupFs`: a kill write under a missing parent returns `NotFound`, which counts as an absent scope (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the `SimCgroupFs::snapshot()` oracle)) |
| Oracle | The cadence values asserted are ADR-0124's accepted contract — the 1 s audit period, the 250 ms attempt period, the 5 s recovery deadline, 20 attempts — written as that contract, never as the private constants, and measured on the injected clock (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (how the in-process lane bounds time without naming them)). C0 healthy; C1 one audit per owner per 1 s audit period, the DNS audit included (responder 0, at most one per period); C2 each of the 13 stimuli (the 11 node-level components, a DNS serve panic, and a DNS audit refusal reported as `audit_mismatch`) detected within one audit period of the fault, admission closed (a hung-audit cell: detection within 5 s of injected time after the fault, because the E18-derived call bound is private); C3 quiescence per component; C4 exact-owner repair on the 250 ms cadence, single reopen; C4b a failed restore (`script_restore_failure`) keeps admission Closed with `Restored { ok: false }`, a re-injected loss quiesces again (exactly two quiescences), and after the disarm admission reopens within one attempt of `Restored { ok: true }`, still two quiescences one second later (M17); C5 one typed fail-stop at the 5 s deadline or the 20th attempt, and every recovery outcome within the 5 s window after detection; C6a unconfirmed `{A}` → `guest_network.shared_owner_vm_killed { alloc: A, cause: "quiescence_unconfirmed" }`, the `SimCgroupFs` snapshot holds `1\n` at A's scope `cgroup.kill` and no entry at the workloads-slice `cgroup.kill`, recovery continues and reopens; C6b `Fail` and `Hang`, both on every seed, after VMs are deployed through the API → detection within 1 s plus one step, one request `TapQuiescenceUndetermined` through `ServerHandle::shutdown_requested`, admission never reopens, and when the request is received the snapshot holds `1\n` at the literal `<root>/overdrive.slice/workloads.slice/cgroup.kill`, every scope is still a directory, and no per-VM kill was written; `Fail` completes within 10 ms of detection, `Hang` no sooner than the quiesce bound's 1 s floor and within 5 s (M17, H10); C6c audit damage while Open → A killed (its scope write in the snapshot), admission stays open; C7a healed owner after fail-stop cannot reopen; C7b in-flight read-back after the deadline cannot reopen; C8 supervisor task loss → immediate fail-stop with the latest snapshot; C9 an activation in flight (held at the owner port) or arriving during recovery: while recovering, at most one `QuiescenceLatched` (gate Closed), no EXEC release, no Failed row; after the reopen exactly one `Raised` (gate Open); order quiesce ≺ latched ≺ successful restore ≺ raise ≺ EXEC release; the row Running (M8) |
| Seed / isolation | `OVERDRIVE_SUPERVISOR_PROOF_SEEDS`, printed per cell; control-plane integration binary is `host-kernel-shared` |
| Rust home | `crates/overdrive-control-plane/tests/integration/shared_network_supervisor_recovery.rs` — the ten bodies MOVED from `crates/overdrive-sim/tests/shared_network_supervisor_recovery_proof.rs`: `healthy_owner_keeps_admission_open_without_recovery_effects`, `healthy_node_audits_the_shared_owner_every_second`, `every_component_loss_is_detected_within_one_audit_and_closes_admission`, `kernel_path_loss_quiesces_once_before_repair_and_listener_or_dns_loss_never_does`, `repair_runs_through_the_owning_component_on_the_attempt_cadence_and_reopens_once`, `unrepaired_loss_fail_stops_with_one_typed_request_at_the_deadline`, `unconfirmed_quiescence_kills_affected_vms_before_repair_and_fail_stops` (RETARGETED and renamed `unconfirmed_quiescence_kills_only_the_affected_vm_and_recovery_reopens`), `healed_owner_after_fail_stop_cannot_reopen_admission`, `in_flight_success_after_the_deadline_cannot_reopen_admission`, `supervisor_task_loss_is_observed_immediately_and_fail_stops_with_the_latest_snapshot`; NEW `undetermined_quiescence_fails_the_node_with_one_typed_request`, `audit_damage_while_open_kills_only_that_vm_and_keeps_admission_open`, `a_failed_restore_keeps_admission_closed_until_a_later_restore_succeeds` (C4b), `an_activation_in_flight_waits_for_reopen_and_raises_once` (C9) |
| Disposition / step | RETARGETED + MOVED + NEW — 09-01 |

#### S-ND295-30A — Only VMs whose isolation cannot be confirmed are stopped

`@property @tier1 @error @contract-shape:bounded-change`

```gherkin
GIVEN several Active VMs and a kernel-path loss or a damaged per-VM part
WHEN quiescence or the audit names specific VMs, or cannot name them at all
THEN exactly the named VMs are stopped, before the supervisor calls any owner again
AND a VM whose scope is already gone counts as stopped
AND only an undetermined set or a failed per-VM stop stops every workload VM and fails the node
AND a stopped VM is never audited or restored again, and recovery continues for the rest
```

| Field | Value |
|---|---|
| Discharges | E12 seeded (a)-(g), plus the kill-loop cases (c2) and (i) and the intentional-shutdown-mid-loop case; G5/r2, G5/r3; D-295-R14 (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the kill scope)), user rulings 2 and 8, and user decision 1 of 2026-09-30 (the per-report kill loop runs to its end before any further owner call or fail-stop; each kill write is bounded by `SHARED_NETWORK_VM_KILL_CALL_BOUND`, a miss is a failed kill, and a late-landing write changes no outcome) |
| Contract shape | bounded-change |
| Lane | seeded-sim (source-local, default lane; as S-ND295-29A) |
| Driving port | as S-ND295-29A |
| Fault stimulus | the test-local owner's quiescence outcome (`Unconfirmed({A})`, `Unconfirmed(every Active VM)` — two or three, seeded, during a `Bridge` recovery: the production outcome of a netlink failure common to every TAP under DR-08 (b)-A — `Fail`, `Hang`) and damage set `{A}` for each per-allocation part class (a TAP deleted; ingress/egress attachment or pin removed; endpoint; guard member; raised while `ProvisionedDown`; owner or persistence; a reserved or missing host-side MAC; debug mask). Every case first creates the workloads slice and each reported allocation's scope directory, with every ancestor, through `CgroupFs::create_dir` on the one `SimCgroupFs`. (c) `SimCgroupFs::inject_error(SimOp::Write, <A's scope>/cgroup.kill, ErrorKind::Other)`; (d) A's scope directory is not created, so the kill write under a missing parent returns `NotFound`, the absent scope a concurrent stop leaves (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the `SimCgroupFs::snapshot()` oracle)) |
| Oracle | `SimCgroupFs::snapshot()` holds the last payload per path, not an ordered log, so each assertion names its observation point. (a),(e),(f),(g): `1\n` at A's scope `cgroup.kill` and no entry at the workloads-slice `cgroup.kill`; (d): no entry at either, and A counts as killed; (b) and (c): `1\n` at the slice `cgroup.kill` in the snapshot taken when the `TapQuiescenceUndetermined` / `VmKillFailed` request is received. Ordering: the test-local owner stores the snapshot taken at the start of each owner call in its journal, and the first owner call after a report already holds every reported allocation's kill write. `shared_owner_vm_killed { alloc, cause }` per kill; EXEC stays Open for damage found while Open; a killed allocation is never in a later audit's damage set or a later restore and is never announced again; recovery reopens exactly once after (a), (d), (e); every-Active unconfirmed: one `shared_owner_vm_killed` per VM in `AllocationId` order, each announcement's snapshot holding the kill writes of that VM and the earlier ones only (kill, then announce, per FD's kill-scope table), no workloads-slice write, the next owner call's snapshot holding every scope kill, one reopen and no fail-stop. The `alloc` field is matched whatever its rendering (`Display` or `Debug`), and the double's own `condemned()` set is not an oracle (M8, DR-18). Kill-loop cases (user decision 1 of 2026-09-30): (c2) a per-VM kill write parked past `SHARED_NETWORK_VM_KILL_CALL_BOUND` on the injected clock is a failed kill — the workloads-slice `cgroup.kill` holds `1\n`, one `VmKillFailed` request is sent, no owner call sits between the report and the slice kill, and a write that lands after its missed bound changes no outcome; (i) a kill loop straddling the 5 s recovery deadline writes every reported scope's `cgroup.kill` in `AllocationId` order before the one `RecoveryDeadlineExceeded` request, with no workloads-slice kill (the loop ran to its end, not cut short) and no owner repair call between report and request; the intentional-shutdown case — a SIGINT/SIGTERM arriving mid-loop — writes every reported kill in `AllocationId` order before the supervisor observes cancellation, with no slice kill and no fail-stop (intentional shutdown is not a fail-stop; cancellation is observed only between loops) |
| Seed / isolation | as S-ND295-29A |
| Rust home | NEW `crates/overdrive-control-plane/src/lib.rs::shared_network_task_owner_acceptance::{an_unconfirmed_tap_stops_only_its_vm_and_recovery_reopens, an_undetermined_quiescence_stops_every_workload_vm_then_fails_the_node, a_failed_per_vm_stop_stops_every_workload_vm_then_fails_the_node, an_already_removed_scope_counts_as_stopped, every_damaged_per_vm_part_stops_only_that_vm_while_admission_stays_open, a_killed_vm_leaves_every_later_audit_and_restore_universe, a_quiescence_naming_every_active_vm_stops_each_in_order_and_recovery_reopens, a_per_vm_kill_write_pending_past_its_bound_fails_the_node, a_kill_loop_running_at_the_deadline_completes_before_the_one_request, an_intentional_shutdown_mid_kill_loop_lets_the_loop_finish_first}` (the last three NEW for user decision 1 of 2026-09-30, the kill loop; each parks a `cgroup.kill` write on the injected clock via `SimCgroupFs::park_until` and drives shutdown via the rig's `request_intentional_shutdown`) |
| Disposition / step | NEW — 09-01 |

#### S-ND295-32 — Every supervisor wait is bounded and every abnormal exit closes new commands visibly

`@error @contract-shape:bounded-change`

```gherkin
GIVEN the supervisor calls an owner that does not answer, or itself ends abnormally
WHEN the owner call outlives its bound, or the supervisor returns, errs, panics, is cancelled, or loses its channel
THEN a hung audit counts as a failure of that owner's first component with an audit-timeout cause
AND a call still pending at the recovery deadline is abandoned and the attempt is not counted
AND a quiescence result that arrives after its bound is ignored: the node is already failing, and the late result kills or reopens nothing
AND an abnormal supervisor exit writes fail-stop before the typed request is returned
```

| Field | Value |
|---|---|
| Discharges | E11 bounded calls; G5/r2, G5/r4; D-295-R13 (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the full-audit call bound; the recovery attempt)), D-295-DISTILL-8 exit table |
| Contract shape | bounded-change |
| Lane | seeded-sim (source-local, default lane; as S-ND295-29A) |
| Driving port | as S-ND295-29A; `ServerHandle` exit classification |
| Fault stimulus | the test-local owner scripted to hang in `audit_shared`; its quiescence outcome `Hang`; its quiescence outcome `Late { unconfirmed: {A}, after: 2 × SHARED_NETWORK_QUIESCE_CALL_BOUND }`, which resolves on the owner's clock only after its delay |
| Oracle | `shared_owner_unhealthy { cause: audit_timeout }` exactly `SHARED_NETWORK_AUDIT_CALL_BOUND` after the audit began, on the injected clock; a quiesce call still pending at `SHARED_NETWORK_QUIESCE_CALL_BOUND` yields `TapQuiescenceUndetermined`; a call pending at the recovery deadline is abandoned and its attempt is not counted; a late quiescence result (G-295-5 row 4, DISTILL review H12): no request before the bound; at the bound one request `Bridge / TapQuiescenceUndetermined / 0 attempts / SHARED_NETWORK_QUIESCE_CALL_BOUND` whose receipt snapshot already holds the workloads-slice kill while A's scope is untouched; after the late instant plus two audit periods, no per-VM kill write and no `shared_owner_vm_killed`, the supervisor fail-stopped, and no converge or restore. These bodies are source-local, so they reach both private constants through `super::` by name, never by literal (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the constants' home and visibility)); a value change never edits them. The existing actual-Tokio exit matrix |
| Rust home | NEW `crates/overdrive-control-plane/src/lib.rs::shared_network_task_owner_acceptance::{a_hung_audit_is_a_timeout_failure_of_its_owner, a_call_pending_at_the_deadline_is_abandoned_uncounted, a_quiescence_result_after_its_bound_is_ignored}`; the test-local owner's self-test NEW `crates/overdrive-control-plane/src/shared_network_test_ports.rs::tests::a_late_quiescence_resolves_only_after_its_delay_on_the_owner_clock` (active); `…::{actual_tokio_exit_matrix_fail_stops_before_returning_the_exact_snapshot, explicit_request_is_returned_unchanged_and_intentional_shutdown_is_not_failure}` RETAINED |
| Disposition / step | NEW + RETAINED — 09-01. Step 09-01 introduces the constants and, as the later of the capture and constants steps, sets all three call bounds (`SHARED_NETWORK_AUDIT_CALL_BOUND`, `SHARED_NETWORK_QUIESCE_CALL_BOUND`, and `SHARED_NETWORK_VM_KILL_CALL_BOUND` = `max(1 s, 4 × W)` from the kill measurement) from the M-ND295-E18 receipt captured at 08-04 (re-capturing if the receipt is gone) and records them in their rustdoc (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the values before E18 through the one record home)) |

#### S-ND295-34 — Shared-gateway DNS stays truthful, and its loss closes new commands until a fresh responder is serving

`@real-io @error @adapter-integration @contract-shape:bounded-change`

| Field | Value |
|---|---|
| Discharges | E17 (seeded in S-ND295-29A, in-process in S-ND295-29B, native real bind here); D-295-R16 `GuestDns` port (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `GuestDns` and `GuestDnsFactory` ports)) and its sim doubles (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` and `SimGuestDnsFactory` doubles)) |
| Contract shape | bounded-change |
| Lane | lima-kernel / native real `:53` bind (root asserted); source-local DNS task-owner bodies; in-process (the composed probe refusal) |
| Driving port | `HostGuestDnsFactory::responder(deps)` → `GuestDns::{probe, serve, audit, stop}`; `DnsServeTaskOwner`; composed: `run_server_with_obs_and_driver` whose `ServerConfig.guest_dns` is `SimGuestDnsFactory` with `script_probe_failure(true)` (the required port; DISTILL review B6 (b), DR-03) |
| Fault stimulus | the responder's socket lost out of band — every descriptor holding the socket's inode is overwritten with `/dev/null` by `dup3`, then the body waits up to 5 s for `/proc/net/udp` to drop the socket — never the responder's own `stop()` (DISTILL review M10); the factory's scripted probe refusal |
| Oracle | existing A/NODATA/NXDOMAIN/transaction-id/source-pin evidence; `audit` reads back the recorded socket identities without mutation and fails after the out-of-band loss, and every `/proc` read fails loudly; the probe refusal through the required port: `Err(DnsResponderBoot(Probe { .. }))` carrying the scripted reason, `health.startup.refused` with reason `dns.responder.probe`, the EXEC gate BootClosed, no `TapCreate` or `TapSetUp`, and exactly one responder built |
| Rust home | `crates/overdrive-control-plane/src/lib.rs::shared_network_task_owner_acceptance::{dns_task_owner_classifies_real_exits_and_never_overwrites_a_live_handle, dns_replacement_joins_the_old_task_before_spawning_from_live_and_exited_states, dns_shutdown_prefers_cooperative_stop_and_awaits_the_bounded_abort_backstop, dns_replacement_is_refused_from_every_invalid_state_without_spawning}` (RETARGETED: the owner holds `Arc<dyn GuestDns>`); existing `dns_responder_bind` integration bodies (RETAINED; each passes the final `vm_cgroups` over `SimCgroupFs`); NEW `crates/overdrive-control-plane/tests/integration/dns_responder_bind.rs::{the_responder_audit_reads_back_its_socket_and_fails_after_loss, run_server_refuses_boot_when_the_guest_dns_probe_fails_through_the_required_port}`; `…::run_server_refuses_boot_on_dns_probe_fault_with_probe_reason` (the `dns_probe_fault` switch's body) stays active until 05-01, which deletes it with `dns_probe_fault`; fixture self-tests of the pinned sim doubles NEW `crates/overdrive-sim/src/adapters/guest_dns.rs::tests::{the_probe_slot_is_shared_by_every_responder_and_is_standing, serve_ends_on_the_first_end_serve_or_stop_even_before_its_first_poll, the_audit_slot_is_per_responder_so_a_replacement_audits_clean}` |
| Disposition / step | RETARGETED + RETAINED + NEW + DELETED-at-05-01 — 05-01 (port, and the composed probe refusal) and 09-01 (EXEC closure). The sim self-tests are active from phase B, which implements the doubles faithfully (scaffold class F) |

#### S-ND295-65 — Protection, DNS, and the supervisor are always composed

`@error @contract-shape:pure-function`

```gherkin
GIVEN the server's configuration and composition code
WHEN it is compiled and scanned
THEN a configuration without the protection and DNS ports does not compile
AND no optional field or parameter can switch off protection, DNS, or supervisor composition
```

| Field | Value |
|---|---|
| Discharges | E16 (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E16 row)); D-295-R16 (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (`ServerConfig` and the composition changes)); the `AppState` and `ServerHandle` owner contract (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors and the `ServerHandle` owner fields)) |
| Contract shape | pure-function |
| Lane | pure (trybuild + source scan) |
| Driving port | `ServerConfig::new(kek, mtls_intercept, guest_dns)`; the source of every `.rs` file under `overdrive-control-plane/src` and `overdrive-worker/src`, declarations found by name, not by file (comments and literals blanked; items gated by exactly `#[cfg(test)]` and out-of-line `#[cfg(test)]` module files skipped) |
| Oracle | trybuild: a KEK-only configuration does not type-check — the fixture coerces `ServerConfig::new` to a `fn(Arc<dyn Kek>) -> ServerConfig`, and the checked-in `.stderr` (`E0308`, naming the three parameter types) names no source file, because file placement is not part of the contract (DISTILL review DR-18). Scan, scoped to E16's composition declarations (DISTILL review DR-02): the fields of `AppState`, `ServerHandle`, and `ServerConfig`, the parameters of `AppState::{new, new_with_workflow_engine}`, of every `run_server*`, and every parameter carrying `MtlsInterceptLifecycle` wrap none of `MtlsInterceptWorker`, `ServiceBackendsResolve`, `SharedGuestNetworkOwner`, `MtlsInterceptLifecycle`, `MtlsIntercept`, `GuestDns`, `GuestDnsFactory`, `DnsServeTaskOwner`, `SharedNetworkSupervisorHandle`, or an alias of one, in `Option<…>`; `DnsServeTaskOwner.responder` is a lifecycle slot E16 does not scope — it must be found wrapping `GuestDns` in `Option` and is not reported; the required declarations exist unwrapped; no `compose_mtls`; no `dns_probe_fault`; no after-boot replacement — no `replace_mtls_worker_for_test`, no `inject_owner_shutdown_failure_for_test` or `owner_shutdown_failures`, no `ServerHandle` method that assigns an owner field, and no post-construction assignment to `state.shared_guest_network`, `state.mtls_worker`, `state.guest_network_exec`, or `state.guest_pool` in `run_server*` |
| Rust home | NEW `crates/overdrive-control-plane/tests/compile_fail/server_config_requires_intercept_and_dns_ports.rs` (+ `.stderr`); NEW `crates/overdrive-control-plane/tests/acceptance/required_serve_ports_source_scan.rs::no_optional_switch_gates_protection_dns_or_supervisor_composition`; the scanner's self-test NEW `…::the_scan_reports_optional_ports_only_in_composition_declarations` over a planted source (five reported sites, the exempt slot seen and unreported, a non-composition parameter unreported, eight declarations scanned) |
| Disposition / step | NEW — 05-01 for the scan; the trybuild fixture and the scanner self-test are active |

#### S-ND295-68 — The operator's serve process exits with status 1 on shared-network fail-stop

`@driving_port @error @contract-shape:bounded-change`

```gherkin
GIVEN a running serve whose shared network suffers a loss it cannot repair
WHEN the supervisor requests fail-stop while an interrupt is also ready
THEN the fail-stop wins, shutdown is bounded to ten seconds, and the lifetime reports status 1
AND a loss the platform repairs never ends the serve process
AND an interrupt on a healthy serve still stops it with status 0
```

This is a test, not a verification expectation: the exit status is the
`ServeExit::exit_code()` the serve lifetime port returns to `main`, and the
port is driven in-process with injected signals and clock.

| Field | Value |
|---|---|
| Discharges | E15 in-process + metal; D-295-R17 (FD § "[REF] Serve lifetime port (D-295-R17) — AS BUILT, USER-APPROVED 2026-09-23"); proof §3.6 |
| Contract shape | bounded-change |
| Lane | native (in-process `serve`, injected signals, recording `SimClock`; the fault is real kernel state) |
| Driving port | `ServeLifetime::new(signals, clock).run(handle)` over `serve::run_with_kek` |
| Fault stimulus | **RETARGETED:** one owned constant rule of `table ip overdrive-mtls` is rewritten to a different listener target (canonical wrong target, S19-A shape) — whole-table deletion is repairable under D-295-R15 and would no longer fail-stop |
| Oracle | unchanged: internal request beats a ready SIGINT; 10 s bound on the injected clock; `exit_code() == 1`; drained and abandoned cases; SIGINT control exits 0 with no bound armed. **NEW contrast body:** deleting the whole `table ip overdrive-mtls` (repairable under D-295-R15) leaves `ServeLifetime::run` pending through the recovery window with no fail-stop request; a later SIGINT then returns `ServeExit::Stopped { signal: Interrupt }` with `exit_code() == 0` |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | `crates/overdrive-cli/tests/integration/serve_lifetime_fail_stop.rs::{an_operator_interrupt_stops_a_healthy_serve_with_status_zero, a_shared_network_fail_stop_wins_over_a_ready_interrupt_and_exits_status_one, a_shared_network_fail_stop_shutdown_is_abandoned_when_the_ten_second_bound_elapses, a_repaired_shared_network_loss_never_ends_serve}` |
| Disposition / step | RETARGETED (fault only; the three existing bodies land in place and stay active and GREEN). The contrast body is NEW, marked `#[ignore = "pending DELIVER step 08-03 (S-ND295-68)"]`, because whole-table repair arrives with R15 at 08-03; re-verified at 09-02 |

#### S-ND295-33 — A failed handler shuts down before a fresh handler admits work

`@driving_port @error @contract-shape:bounded-change`

```gherkin
GIVEN the node runs VM workloads through one production server handler with required ports
WHEN shared-network ownership cannot be restored in the bounded window, or quiescence cannot be determined
THEN the handler returns one typed fail-stop request naming the cause
AND a fresh handler over the same roots accepts work only after its startup checks complete
```

The forced-shutdown/abandoned-at-exit obligation is NOT part of this scenario
(item 6, 2026-09-25): the deleted body
`forced_shutdown_timeout_records_abandoned_at_exit_separately_from_graceful_drain`
checked records only the CLI serve lifetime emits, which S-ND295-68 already
covers through its drained and abandoned cases.

| Field | Value |
|---|---|
| Discharges | E11/E15 cross-owner conformance; G5/r2; D-295-R14, R16, R17 |
| Contract shape | bounded-change |
| Lane | in-process (`tests/conformance`, exported handler + HTTPS API only) |
| Driving port | `DirectHandlerHarness` (`tests/conformance/src/lib.rs`) over `run_server_with_obs_and_driver(ServerConfig::new(kek, mtls_intercept, guest_dns), obs, driver, vm_host_state, shared_guest_network, guest_network_exec, vm_cgroups)` (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)), with `SimMtlsIntercept`, `SimGuestDnsFactory`, the `SimSharedGuestNetworkOwner`, and `vm_cgroups` over `SimCgroupFs`; the request is observed as § *In-process observation* states |
| Fault stimulus | `SimSharedGuestNetworkOwner` standing `Bridge` component failure plus `script_converge_failure(true)`; `script_quiesce_outcome(Fail)` |
| Oracle | typed `ServeShutdownRequest::SharedGuestNetwork { cause: RecoveryDeadlineExceeded / TapQuiescenceUndetermined }` via `ServerHandle::shutdown_requested`; fresh handler admits over HTTPS only after boot. The former host-residue check is dropped: it watched real host paths that a `SimDriver`/`SimCgroupFs` composition never writes, so it could not fail (DISTILL review M10) |
| Seed / isolation | example; the conformance binary is `host-kernel-shared` |
| Rust home | `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs::shared_owner_fail_stop_shuts_down_before_a_fresh_handler_reopens_admission` (RETARGETED to required ports); NEW `…::undetermined_tap_quiescence_requests_one_typed_fail_stop_before_a_fresh_handler_reopens`; `…::unconfirmed_tap_quiescence_stops_the_affected_vm_and_requests_fail_stop` DELETED (defends the superseded kill-all-then-fail-stop contract; per-VM scope is S-ND295-30A/30B); `…::forced_shutdown_timeout_records_abandoned_at_exit_separately_from_graceful_drain` DELETED (item 6 — records only the CLI serve lifetime emits, covered by S-ND295-68's drained/abandoned cases) |
| Disposition / step | RETARGETED + NEW + DELETED — 10-03 |

### Group J — Native fault evidence and non-regression

#### S-ND295-30B — On real guests, a damaged or unconfirmable VM is stopped alone

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN several mesh VMs Running on native metal
WHEN one VM's TAP disappears during an unrelated program loss, a still-booting VM's TAP is deleted, or one VM's ingress link, egress link, or guard member is removed from outside
THEN only that VM is stopped and the node recovers or stays open for the rest
AND the stopped VM's cleanup completes and its lease is released
```

| Field | Value |
|---|---|
| Discharges | E12 native (e), (f), (g) and the per-TAP set-down failure (realized as case (e)); the real per-VM `cgroup.kill` effect the whole-call branch relies on (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E12 whole-call branch: the native lane's per-VM `cgroup.kill`)); G5/r2, G5/r3; D-295-R14 |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve::run_with_kek` + `deploy`; for (f) the delayed-READY guest fixture (`crates/overdrive-cli/tests/integration/delayed_ready_guest.rs`, DISTILL review H5): a staged copy of the guest image whose `/sbin/init` is a holder that prints a hold marker, holds READY for 20 s, prints a release marker, and execs the real init |
| Fault stimulus | (e) delete one Active TAP (`ip link del`) right after deleting `table ip overdrive-mtls` so quiescence meets the missing TAP; (f) delete the TAP of the delayed-READY allocation once it is witnessed booting — exactly one new TAP, its rows only Pending, the TAP down, a Cloud Hypervisor process holding its queue, the console showing the hold marker and not the release marker — beside an Active mesh Service survivor; (g) D6 detach of one TAP's ingress link; separately its egress link; separately D9 removal of one guard member |
| Oracle | `shared_owner_vm_killed { alloc, cause }` for exactly that allocation; its CH pid ends; teardown converges on absence (TAP ifindex; for (f) the whole attachment identity: ifindex, pin inodes, endpoint); lease released (`lease_released`). (e): recovery reopens — a fresh deploy reaches Running — and the survivor's CH process lives. (f): cause `attachment_damaged`; the witnessed CH pid ends; the row is Failed with `VmGuestExitUnreported` and was never Running (the existing VMM-exit path); admission stayed open (`exec_stayed_open`); the survivor keeps the same pid, stays Running with the same restart count, and a fresh dialer Job completes a byte-distinct exchange with it (exit 0). (g): admission stays open; the bystander's CH process lives. (e) and (g) take process liveness plus a fresh admission as the survivor evidence; the byte-distinct exchange is (f)'s. Known race in (e): an audit between the table deletion and the TAP deletion reports the missing TAP as per-allocation damage instead of `quiescence_unconfirmed`; the body then fails on the exact cause, never passes vacuously (a 10-02 review item) |
| Seed / isolation | example; `host-kernel-shared`; the native-fault module's nextest budget is 10 × 60 s ((f)'s inner bounds sum to about 590 s at worst), and `TeardownBound` ends a failed body's teardown after 30 s |
| Rust home | NEW `crates/overdrive-cli/tests/integration/shared_network_native_faults.rs::{a_tap_lost_during_quiescence_stops_only_its_vm_and_the_node_recovers, a_booting_vms_deleted_tap_stops_only_that_vm, a_removed_ingress_link_egress_link_or_guard_member_stops_only_that_vm}`; fixture `crates/overdrive-cli/tests/integration/delayed_ready_guest.rs` (`HoldingInit::{DelayedReady, PowerOffBeforeReady}`, `stage_holding_rootfs`, RAII loop mount; compile-time checks keep both holds under the 30 s boot budget) |
| Disposition / step | NEW — 10-02. There is no native whole-call case, by DESIGN: E12's native lane proves per-VM kill writes only, and the whole-call slice kill is proven by the seeded S-ND295-30A (b) and the in-process S-ND295-29B C6b (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E12 whole-call branch)) |

#### S-ND295-37 — A simultaneous external loss of a TAP's link and the guard is bounded and visible

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a managed guest producing identifiable frames
WHEN an external actor deletes that TAP's ingress link and the bridge guard right after an audit
THEN the next audit names the guard as the failing node component, closes new commands, and quiesces the TAPs within one second of the deletion
AND after repair the damaged VM is stopped alone and the rest recover
AND no claim is made that the interval before quiescence is fail-closed
```

| Field | Value |
|---|---|
| Discharges | E11 native (double loss); G5/r2; D-295-R14/H1 (the per-TAP link is per-allocation, so the guard is the first node-level component) |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve` + `deploy` |
| Fault stimulus | D6 detach of one TAP's ingress link + D9 `delete_owned_guard` |
| Oracle | tracing Layer captures `guest_network.shared_owner_unhealthy { component: BridgeGuard }` before TAP-down; every managed TAP reads down within 1 s of deletion; no guest frame reaches ordinary forwarding after quiescence — before that zero verdict, 4 canary frames sent out of the sink TAP and 4 out of the bridge must each be read exactly (the positive control), and both captures are sealed (a reject-all filter, a settle, a final drain) and reconciled (`tp_drops == 0`, `tp_packets` equal to the frames read), final-drain frames counting toward the verdict (DISTILL review M11); after repair `shared_owner_vm_killed { cause: "attachment_damaged" }` for the damaged allocation and reopen |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs::simultaneous_external_tcx_and_guard_loss_quiesces_the_managed_tap_within_one_second` (RETARGETED: first component `BridgeGuard`; per-VM kill after repair) |
| Disposition / step | RETARGETED — 10-02 |

#### S-ND295-66 — A stop succeeds even when parts of the attachment are already gone

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a Running VM allocation on native metal
WHEN its TAP, separately its link pin, and separately its guard member are removed from outside before the operator stops it
THEN the stop still reaches an empty complement and releases the lease
AND a VM that fails before it is ready, while its VMM holds the TAP's queue, leaves its TAP never raised and deleted, and no queue holder anywhere
```

| Field | Value |
|---|---|
| Discharges | E5 native; G4/r2, G4/r5; D-295-R5 teardown (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (teardown converges on absence)) |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve` + `deploy` + `workload stop` handler |
| Fault stimulus | `ip link del <tap>`; an unconditional `rm` of the ingress link pin that outlives the TAP; D9 member removal; a guest image that powers off before READY (`HoldingInit::PowerOffBeforeReady` of the delayed-READY fixture, power-off at 8 s), so the failure comes after provision and the queue attach — the former missing-kernel spec failed the preflight before any TAP existed, so its assertions held vacuously (DISTILL review B4, DR-12) |
| Oracle | stop returns success; TAP, entry, links, pins, member absent; `lease_released`. Failed launch: (1) the booting witness — Cloud Hypervisor holds the TAP's queue at descriptor 3 — then a Failed row with `VmGuestExitUnreported` that was never Running; (4) the attachment complement absent and (5) `lease_released`, both before any stop; after quiescence (3) the witnessed CH process has ended and no detached `/dev/net/tun` descriptor (empty `iff:`) remains anywhere on the host; (2) an `RTMGRP_LINK` monitor started before the deploy (an overrun fails closed) shows the TAP created, never `IFF_UP`, and exactly one `RTM_DELLINK`, as its last notice |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-cli/tests/integration/shared_network_native_faults.rs::{a_stop_converges_when_attachment_parts_are_already_gone, a_launch_that_fails_leaves_no_tap_and_no_queue_holder}` |
| Disposition / step | NEW — 10-02. The launch-failure body was active at HEAD and is now pending: under the new stimulus it needs a guest that boots, which needs the 05-03 fd handoff |

#### S-ND295-67 — A MAC hijack from outside the VM steals nothing and the victim's delivery returns

`@real-io @error @security @contract-shape:bounded-change`

```gherkin
GIVEN two Active guests A and V on native metal
WHEN a process outside the launch filter, holding a copy of A's queue as the VM uid with no capabilities, sets A's host-side MAC to V's guest MAC
THEN A's TAP carries no frame addressed to V, while A's own unicast and every broadcast still arrive
AND V receives no host unicast while the entry is poisoned
AND the next audit names A as damaged by its host-side MAC, now V's guest MAC and so a reserved address, and stops only A
AND after A's cleanup the poisoned entry is gone, V answers the host again, and V's MAC is re-learned on V's port within a second
```

| Field | Value |
|---|---|
| Discharges | E12 (h); D-295-R21 (FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24"), R14; pre-control RED oracle = increment-z (`spike/findings-mac-fdb-isolation.md` STEPs 4-6) |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve` + `deploy` |
| Fault stimulus | `pidfd_getfd` of A's descriptor 3 into a forked child that drops to uid 4200 with `CapEff=0` and issues `SIOCSIFHWADDR` on it — the child reports twice: its `CapEff`, `O_NONBLOCK`, and the `CLOCK_MONOTONIC` instant of the change, then its reads; host-originated unicast to V's guest MAC (16 frames per stimulus) |
| Oracle | ordered by the audit period (DISTILL review H4): both TAPs are awaited Active, and the captures on A and V are opened, with a pre-change control, before the change; every poisoned-window stimulus is injected and read before A's kill, and A's TAP stands throughout. (1) exact-ifindex capture on A's TAP plus a read on the held queue: zero frames to V, zero stolen; node-wide `EgressDestinationDrop` rises by at least the 16 frames sent; (2) exact positive controls: host unicast to A's MAC arrives on A; broadcast reaches every guest; (3) capture on V's TAP: no host unicast while poisoned; (4) within one audit period plus the full-audit latency ceiling (1 s + 1 s: the kill follows the first audit that starts after the change, and that audit's own duration is bounded by the full-audit rule) `shared_owner_vm_killed { alloc: A, cause: "attachment_damaged" }` for A only (the `TapHostMac { address: Reserved(<V's guest MAC>) }` fact behind that cause is proven source-locally by S-ND295-72); EXEC stays Open and V is untouched; A's teardown by its ordinary lifecycle, kill→teardown interval recorded; (5) within 1 s of A's complement read-back, `bridge fdb show` lists V's MAC on no foreign port, a raw ICMP echo from the host is answered by V, and V's MAC is a learned non-permanent entry on V's port |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-cli/tests/integration/shared_network_native_faults.rs::a_mac_hijack_from_outside_the_vm_steals_nothing_and_the_victim_recovers` |
| Disposition / step | NEW — 10-02 |

#### S-ND295-69 — Losing the whole program table with live mesh VMs is repaired in place

`@real-io @error @contract-shape:bounded-change`

```gherkin
GIVEN a mesh Service and its client Running on native metal
WHEN the protection program table is deleted from outside
THEN new commands close, TAPs are quiesced, the program and members are restored, the TAPs come back up, and admission reopens
AND the Service answers its client again without a restart
```

| Field | Value |
|---|---|
| Discharges | E11 native `IpRules` table deletion; G5/r1; D-295-R15 |
| Contract shape | bounded-change |
| Lane | native |
| Driving port | `serve` + `deploy` |
| Fault stimulus | `nft delete table ip overdrive-mtls` |
| Oracle | `shared_owner_unhealthy { component: IpRules }` within 3 s of the deletion; each TAP reads down within one audit period plus 1 s of the detection; the state `observe_shared_ip_intercept_state` read before the fault — the exact program identity and its `2 + P` members — is restored exactly (compared whole, not by membership); TAPs read up; admission reopens; no allocation is killed and restart counts are unchanged; a long-lived mesh client (one exchange every 250 ms, never exiting) shows three response needles before the fault and three again after the repair, and a fresh dialer Job exits 0 (DISTILL review M12) |
| Seed / isolation | example; `host-kernel-shared` |
| Rust home | NEW `crates/overdrive-cli/tests/integration/shared_network_native_faults.rs::a_deleted_program_table_is_repaired_with_live_mesh_vms` |
| Disposition / step | NEW — 10-02 |

#### S-ND295-36 — Existing identity, health, placement, and resource guarantees stay intact

`@real-io @regression @contract-shape:unbounded-preservation`

Outcome: the shared switch changes nothing an operator already relies on —
workload identity, health and selection, placement, and resource accounting
behave as before, over fd handoff.

Contract unchanged; mapped existing bodies (`guest_stack_mtls_egress`,
`service_kind_vm_workloads`, VM cgroup/accounting equivalence,
`mtls_resolve_rekey`, Service dataplane selection) remain mandatory. Every
`guest_stack_mtls_egress` body other than the S-ND295-01 trio is RETAINED and
must stay green over fd handoff.

| Field | Value |
|---|---|
| Discharges | G2/r3, G4/r4 (a spawn completing after stop is reaped by the existing VMM owner — existing VM lifecycle bodies) |
| Contract shape | unbounded-preservation |
| Fixture fallout | Every retained caller of `run_server_with_obs_and_driver(s)` (the fallout inventory, FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` row)) passes the final `vm_cgroups`: `CgroupManager::new(<root>, Arc::new(SimCgroupFs::new()))`, or, where the caller composes a real `VmDriver`, a manager over the same root and `CgroupFs` it gives that driver (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the `vm_cgroups` parameter)). Every retained `AppState` fixture passes the worker, owner, gate, and pool of § *Seam fixture* once DELIVER 05-01 cuts the constructors. Neither changes an assertion |
| Disposition / step | RETAINED — verified at 10-04 |

## Recovery proof tests — landing decisions

The five proofs committed in `b5ef001b` are RED on today's code and carry no
pending marker. Each lands as follows. None spawns the `overdrive` binary: the
§3.5 proof spawns only `nft`, `stdbuf nft monitor`, `udevadm`, and `rustc` as
diagnostic tools, and every other proof is purely in-process.

| Proof | Current file | Landing | Lane | Re-targeting | Marker / step | Seeds | Group | Reason |
|---|---|---|---|---|---|---|---|---|
| §3.2 node-wide admission | `crates/overdrive-sim/tests/netns_density_node_admission.rs` | MOVE to `crates/overdrive-sim/tests/integration/netns_density_node_admission.rs`, registered in `tests/integration.rs` | seeded-sim | S-ND295-05D: held (Admitted + Retiring) population; OBS-OVERLAP becomes the NA-OVERLAP assertion; NA-RECREATE added; per-server pool; one `LeaseLedger` owner instance for `AppState` and the seams; a started worker over `SimMtlsEnforcement`, `SimMtlsResolve`, and `SimMtlsIntercept` | `pending DELIVER step 07-03 (S-ND295-05D)` | `OVERDRIVE_ND295_ADMISSION_SEED`, defaults `186055177052160001`, `295032` | none (no kernel object; the sim-composed worker binds nothing); timeout override retargeted to `package(overdrive-sim) & binary(integration) & test(node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads)`, 25 × 60 s for two seeds' fills (~280 s each on the Lima VM, `red-classification.md` Phase G) | User ruling 2 keeps seeded proofs in `overdrive-sim`. Its own-binary rationale (the process-global pool) disappears under R6, and `testing.md` places integration-shaped tests under `tests/integration/`; joining `binary(integration)` also puts it in the CI integration lane |
| §3.3 supervisor | `crates/overdrive-sim/tests/shared_network_supervisor_recovery_proof.rs` | MOVE to `crates/overdrive-control-plane/tests/integration/shared_network_supervisor_recovery.rs` (registered in the control-plane `tests/integration.rs`) | in-process | S-ND295-29B: required ports inject the intercept and DNS faults (`SimGuestDnsFactory`); the final `vm_cgroups` is a `CgroupManager` over one `SimCgroupFs`; C6 split into C6a per-TAP kill with recovery, C6b undetermined → `TapQuiescenceUndetermined`, C6c damage while Open, each with its snapshot oracle; ADR-0124's cadence asserted as contract, never the private constants | `pending DELIVER step 09-01 (S-ND295-29B)` | `OVERDRIVE_SUPERVISOR_PROOF_SEEDS`, defaults `0x2953300000000001`, `0x295330005eed0002` | whole control-plane integration binary is `host-kernel-shared` | It boots `run_server_with_obs_and_driver`, so it is the in-process lane, not seeded-sim (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the seeded-sim and in-process lanes)); after R16 it needs Lima root for the real enforcement probe; the contract's owner is the control-plane supervisor, and `testing.md` keeps single-owner behaviour in the owning crate's integration suite, not in the adapter-sim crate |
| §3.4 element cleanup | `crates/overdrive-control-plane/tests/integration/shared_element_cleanup_failure.rs` | IN PLACE | in-process | S-ND295-07B: removal through `remove_allocation_elements`; typed `ElementRemoval`; lease Retiring; row Running; retry converges | `pending DELIVER step 07-01 (S-ND295-07B)` | none (deterministic) | control-plane integration, `host-kernel-shared` | Correct owner and lane already |
| §3.5 killed-mode boot clear | `crates/overdrive-cli/tests/integration/serve_killed_restart_boot_clear.rs` | IN PLACE | native | S-ND295-13C: R12 order oracles V0-V6 in the R19/R18 program shape; absorbs the first-lease and complement obligations of the deleted native placeholder; no reference to the deleted clear adapter | `pending DELIVER step 08-02 (S-ND295-13C)` | example | `overdrive-cli` integration, `host-kernel-shared`; timeout override retained | `overdrive-cli` owns `serve` + `deploy`; production-faithful residue needs killed mode and a real guest |
| §3.6 CLI fail-stop | `crates/overdrive-cli/tests/integration/serve_lifetime_fail_stop.rs` | IN PLACE | native (in-process `serve`) | S-ND295-68: the fault becomes an owned rule rewritten to a wrong target, because whole-table deletion is repaired under R15 | none — active and GREEN on the built serve lifetime port; must stay GREEN through 08-03 and is re-verified at 09-02 | example | `host-kernel-shared` | Built port (D-295-R17); only the fault needs re-targeting to stay a fail-stop after R15 |

## Existing-body disposition register

RETAINED and RETARGETED bodies are named in each scenario's field table. The
bodies below are DELETED by phase B (deletion discipline: a body defending a
superseded contract is removed, not salvaged).

| Body | Why |
|---|---|
| `overdrive-control-plane/src/guest_network.rs::scratch_probe_acceptance::healthy_probe_uses_the_exact_setup_probe_cleanup_and_inventory_order` | ignored "D14A superseded D5 setup order" |
| `…::scratch_probe_acceptance::every_setup_or_probe_failure_preserves_primary_and_still_runs_complete_cleanup` | ignored "D14A superseded D5 failure order" |
| `…::pool_acceptance::slash_16_exhaustion_is_pool_drift_and_does_not_reuse_an_address` | `/16` exhaustion through `assign` is unreachable under R6/R7 |
| `overdrive-worker/src/mtls_intercept_port.rs::shared_program_rollback_acceptance::replacement_and_every_rollback_disposition_preserve_exact_identity_and_source` | ignored "superseded by D15 stateful shared-IP evidence" |
| `…::shared_program_rollback_acceptance::fresh_replace_exact_prior_rollback_and_idempotent_reapply_are_complete` | same |
| `overdrive-control-plane/tests/integration/shared_guest_network_startup.rs::native_prior_vmm_reclamation_precedes_full_attachment_sweep_and_first_lease_acceptance` | placeholder `panic!` body; its obligations move into the production-faithful §3.5 killed-mode proof (S-ND295-13C) |
| `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs::unconfirmed_tap_quiescence_stops_the_affected_vm_and_requests_fail_stop` | defends the superseded kill-all-then-fail-stop contract (Changed Assumption 16, FD § "[REF] Changed Assumptions" (item 16)) |
| `crates/overdrive-sim/tests/netns_density_node_admission.rs`, `crates/overdrive-sim/tests/shared_network_supervisor_recovery_proof.rs` | moved (see above) |
| `crates/overdrive-sim/tests/acceptance/netns_density_reclaim.rs` | MOVED (fix pass) to `crates/overdrive-sim/tests/integration/netns_density_reclaim.rs`, bodies unchanged apart from the wall-time removal (S-ND295-57, DISTILL review M16, DR-14) |
| `crates/overdrive-control-plane/src/lib.rs::shared_network_task_owner_acceptance::an_activation_in_flight_waits_for_reopen_and_raises_once`, with its `activation_client` / `ClientStep` support | DELETED (fix pass): its activation client was test code emulating the shim, and `condemned() == {A}` was guaranteed by the double; the clause is re-authored as S-ND295-29B C9, where the composed supervisor drives the latch (DISTILL review M8) |
| `crates/overdrive-control-plane/tests/integration/dns_responder_bind.rs::run_server_refuses_boot_on_dns_probe_fault_with_probe_reason` | DELETED by DELIVER 05-01 together with `dns_probe_fault` (it drives the removed switch); active until then; its contract is re-authored against the required port (S-ND295-34; DISTILL review B6 (b), DR-03) |
| `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs`'s host-residue baseline and its `cleanup_complete` phase check | DELETED (fix pass): they watched host paths a `SimDriver`/`SimCgroupFs` composition never writes (S-ND295-33; DISTILL review M10) |
| the late-commit block of `crates/overdrive-worker/tests/integration/netns_density_shared_owner.rs::a_failed_member_clear_refuses_startup_without_publication` | DELETED (fix pass): the test called `land_member_batch` on its own fake; the late-commit clause is re-authored against the composed boot (S-ND295-13D, `boot_member_clear_refusal.rs`; DISTILL review H6) |

Other dispositions that phase B must apply:

- Every `panic!("Not yet implemented -- RED scaffold …")` placeholder named in
  S-ND295-10, 11, 12, 13B, and 48 is AUTHORED as a complete body.
- `overdrive-sim/src/adapters/guest_network.rs::tests::test_wiring_returns_one_owner_port_and_one_boot_closed_exec_pair`
  (stepless marker): run once; GREEN → remove the stale marker; RED → mark
  `pending DELIVER step 06-04 (S-ND295-53)`.
- Every test double implementing `MtlsIntercept`, `GuestNetworkProvisioner`,
  or `SharedGuestNetworkOwner` gains faithful implementations of the new
  methods so RETAINED bodies stay valid once DELIVER wires them.
- The file-local test double `lib.rs::shared_network_task_owner_acceptance::S19SharedOwner`
  is deleted when S-ND295-19 is retargeted; `TestSharedOwner`
  (§ *Test-local control-plane ports*) replaces it, so the owner double is
  defined once.
- The B-7 and B-6 test-support fallout — the listener test support, the
  test-local intercept doubles, the worker bodies that drive the per-allocation
  listener branch, the port equivalence harness, the original-destination
  bodies, and the sim lifecycle's stop error — is applied as § *Intercept
  listener and stop-error test support* states.
- `clear_shared_ip_intercept_elements_atomically` has no test; DELIVER step
  08-02 deletes it (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the netlink contract: `clear_shared_ip_intercept_elements_atomically` deleted)). Phase B does not delete production code.

## Scenario-to-test matrix

| ID | Shape | Lane(s) | Disposition | Step |
|---|---|---|---|---|
| S-ND295-00 | bounded-change | pure + lima-kernel | RETAINED (2 superseded bodies DELETED) + NEW (layout pin) | active (retained bodies, layout pin); 05-00 (bridge-identity leg); 05-01 (DNS leg) |
| S-ND295-01 | bounded-change | native | RETARGETED + NEW (capture self-test) | 10-01; capture self-test active |
| S-ND295-02, 03 | pure-function | pure | RETAINED | active |
| S-ND295-04 | bounded-change | pure (proptest) | RETARGETED + NEW | 06-03 |
| S-ND295-05A | bounded-change | pure (proptest) + in-process | NEW | 06-03 |
| S-ND295-05B | pure-function | pure | RETARGETED + NEW | 07-03 |
| S-ND295-05C | pure-function | pure | NEW | 07-03 |
| S-ND295-05D | bounded-change | seeded-sim | RETARGETED + MOVED | 07-03 |
| S-ND295-05E | bounded-change | pure (source-local) | NEW | 06-03 |
| S-ND295-06 | bounded-change | pure + in-process | RETARGETED + NEW | 06-03 |
| S-ND295-07 | bounded-change | pure + seeded-sim | RETARGETED + NEW | 07-01 |
| S-ND295-07B | bounded-change | in-process | RETARGETED (proof §3.4) | 07-01 |
| S-ND295-08, 09 | bounded-change | tier2 | RETAINED + NEW (nine-slot body) | active; 06-01 (nine-slot body) |
| S-ND295-10 | bounded-change | lima-kernel | RETARGETED + AUTHORED | 09-01 |
| S-ND295-11 | bounded-change | pure + lima-kernel | RETARGETED + NEW + AUTHORED | 06-02 |
| S-ND295-12 | bounded-change | pure + lima-kernel | RETARGETED + NEW + AUTHORED | 06-02 |
| S-ND295-13A | bounded-change | seeded-in-process | RETARGETED | 08-02 |
| S-ND295-13B | bounded-change | in-process | AUTHORED | 08-02 |
| S-ND295-13C | bounded-change | native | RETARGETED (proof §3.5) | 08-02 |
| S-ND295-13D | bounded-change | integration (real loopback sockets) + in-process (composed boot) | NEW | 08-02 |
| S-ND295-14..18 | bounded-change | pure + lima-kernel | RETAINED (2 superseded DELETED) | active |
| S-ND295-19 | bounded-change | seeded-sim | RETARGETED (S19-B) | 09-01 |
| S-ND295-20..26, 31A, 31B | bounded-change | pure + integration (real loopback sockets) + lima-kernel + native | RETAINED | active |
| S-ND295-27, 28 | bounded-change | pure (PBT) + VmDriver schedules | RETAINED (one body's cause table extended) | active |
| S-ND295-29A | bounded-change | seeded-sim | NEW | 09-01 |
| S-ND295-29B | bounded-change | in-process | RETARGETED + MOVED (proof §3.3) | 09-01 |
| S-ND295-30A | bounded-change | seeded-sim | NEW | 09-01 |
| S-ND295-30B | bounded-change | native | NEW (per-VM kill only; whole-call branch proven by 30A (b) and 29B C6b) | 10-02 |
| S-ND295-32 | bounded-change | seeded-sim | NEW + RETAINED | 09-01 |
| S-ND295-33 | bounded-change | in-process (conformance) | RETARGETED + NEW + DELETED | 10-03 |
| S-ND295-34 | bounded-change | lima-kernel + source-local + in-process | RETARGETED + NEW (+ one body DELETED at 05-01) | 05-01 / 09-01 |
| S-ND295-35 | bounded-change | native | RETARGETED + NEW | 10-01 |
| S-ND295-36 | bounded-change | native + existing | RETAINED | 10-04 |
| S-ND295-37 | bounded-change | native | RETARGETED | 10-02 |
| S-ND295-38 | bounded-change | lima-kernel | NEW | 05-03 |
| S-ND295-39 | bounded-change | lima-kernel | NEW | 05-03 |
| S-ND295-40 | pure-function / bounded-change | pure + native | RETARGETED + NEW | 05-03 |
| S-ND295-41 | bounded-change | lima-kernel (x86_64 CI runner) + native | NEW | 05-02 |
| S-ND295-42 | pure-function | pure (x86_64, proptest) | NEW | 05-02 |
| S-ND295-43 | bounded-change | lima-kernel (x86_64) + native (f) | NEW | 05-02 |
| S-ND295-44 | pure-function / bounded-change | pure + aarch64 Lima | RETARGETED + NEW | 05-02 (stage-order body 05-03) |
| S-ND295-45 | bounded-change | native | RETARGETED | 05-03 |
| S-ND295-46 | pure-function | pure + xtask-integration | NEW | 05-04 |
| S-ND295-47 | bounded-change | tier2 | NEW | 06-01 |
| S-ND295-48 | bounded-change | pure + lima-kernel | RETARGETED + NEW + AUTHORED | 06-01 |
| S-ND295-49 | bounded-change | pure + lima-kernel | NEW | 06-01 |
| S-ND295-50 | bounded-change | pure | NEW | 06-02 (condemned-universe body 06-04) |
| S-ND295-51 | bounded-change | pure | RETARGETED + NEW | 06-04 |
| S-ND295-52 | bounded-change | in-process | RETAINED (two active bodies) + NEW | active (two bodies); 06-04 |
| S-ND295-53 | bounded-change | seeded-sim | NEW + RETARGETED | 06-04 |
| S-ND295-54 | bounded-change | pure + lima-kernel + in-process | NEW + RETARGETED | 07-01 |
| S-ND295-55 | pure-function | pure (proptest) | NEW | 07-03 (validator body 07-02) |
| S-ND295-56 | bounded-change | pure | NEW | 07-02 |
| S-ND295-57 | bounded-change | seeded-sim | NEW + MOVED | 07-03 |
| S-ND295-58 | pure-function | pure | NEW | 07-04 |
| S-ND295-59 | bounded-change | in-process | NEW | 07-04 |
| S-ND295-60 | pure-function | pure | NEW | 07-04 |
| S-ND295-61 | bounded-change | pure + integration (real loopback sockets) + lima-kernel | NEW | 08-03 |
| S-ND295-62 | bounded-change | native | NEW | 08-01 |
| S-ND295-63 | bounded-change | native | NEW | 08-01 |
| S-ND295-64 | bounded-change | native | NEW | active (controls); 08-01 (guest door) |
| S-ND295-65 | pure-function | pure (trybuild + scan) | NEW | 05-01 (scan); active (trybuild fixture, scanner self-test) |
| S-ND295-66 | bounded-change | native | NEW | 10-02 |
| S-ND295-67 | bounded-change | native | NEW | 10-02 |
| S-ND295-68 | bounded-change | native (in-process serve) | RETARGETED (proof §3.6) + NEW contrast body | active (three bodies); 08-03 (contrast body); re-verified 09-02 |
| S-ND295-69 | bounded-change | native | NEW | 10-02 |
| S-ND295-70 | bounded-change | pure + lima-kernel | NEW + RETARGETED | 05-01 |
| S-ND295-71 | bounded-change | pure + lima-kernel | NEW | 05-01 (08-02 for the two member-aware equivalence bodies) |
| S-ND295-72 | bounded-change | pure + lima-kernel + native | NEW | 05-00 (bridge); 06-02 (TAP invariant at provision and audit, (e)); 06-04 ((i1) at `activate`, the `Condemned` exclusion); (b) active |

**Counts.** 84 scenario entries (the IDs above with every ranged row
expanded: S-ND295-02/03, 08/09, 14-18, 20-26, 27/28, 31A/31B). By scenario:
**RETAINED 22** (00, 02, 03, 08, 09, 14-18, 20-26, 27, 28, 31A, 31B, 36) —
S-ND295-00 stays RETAINED but is split into a bridge-identity leg (05-00) and a
re-authored DNS leg (05-01), per the fresh-host RCA; **RETARGETED 22** (01, 04,
05B, 05D, 06, 07, 07B, 10, 11, 12, 13A, 13B, 13C, 19, 29B, 32, 33, 34, 35, 37,
45, 68 — including the four landed proofs §3.2-§3.5 and the re-targeted §3.6
fault); **NEW 40** (05A, 05C, 05E, 13D, 29A, 30A, 30B, 38-44, 46-67 except 45,
69, 70, 71, 72). Individual NEW bodies are also added inside RETARGETED
scenarios (S-ND295-45's marker moved 05-02 → 05-03; its precondition is a guest
that Runs, which the 05-03 descriptor handoff provides). **DELETED:** 8 bodies plus 2 moved files (register
above; the eighth is the S-ND295-33
`forced_shutdown_timeout_records_abandoned_at_exit_separately_from_graceful_drain`
body, item 6); the step that lands B-7 also deletes, with the per-allocation
listener branch, the worker bodies that drive it (§ *Intercept listener and
stop-error test support*). Error, fault, or boundary scenarios: **68 of 84
(80.9 %)**. By lane, counted once per expanded scenario ID for every lane its
matrix row names: pure 47, seeded-sim 8, seeded-in-process 1, in-process 12,
lima-kernel 30, tier2 3, native 27, integration 11, xtask-integration 1
(recomputed 2026-09-30 for the fix pass of DISTILL review iteration 1: S-ND295-07
gains a seeded-sim body; S-ND295-06, 13D, and 34 gain in-process bodies;
S-ND295-40's spawn-failure body is native, not lima-kernel; S-ND295-41 is also
native. The fix pass adds no scenario ID, so the 84 entries and the 80.9 %
error share stand. Recomputed 2026-09-28 to add S-ND295-71 (pure, lima-kernel) and S-ND295-72
(pure, lima-kernel, native), which the 2026-09-25 tally predates; recomputed
2026-09-25: the `integration` lane — the worker/control-plane
integration binaries with real loopback sockets, S-ND295-13D, 20-26, 31A, 31B,
61 — was omitted before the WP-5 move to the worker integration binary; and
S-ND295-40's queue-error body moved from `overdrive-host` `vmm::tests` to its
`tests/integration` native `kvm-tests` lane, item 8, adding 1 to native).
Of the eight seeded-sim entries, S-ND295-05D and S-ND295-57 are behind
`integration-tests`, for their wall-clock budgets. No verification expectation.

## Evidence-row coverage (E1-E21)

| Row | Seeded / pure | In-process | Lima-kernel | Native |
|---|---|---|---|---|
| E1 zero frames before intercept-live | S-ND295-53 | S-ND295-51, 52, 11 | — | S-ND295-01 |
| E2 queue confinement, descriptors | — | S-ND295-40 | S-ND295-38, 41 | S-ND295-35 |
| E3 vnet header | — | — | — | S-ND295-01 |
| E4 TAP owned by uid 0 | S-ND295-11 (owner `Some(0)`) | — | S-ND295-39 | S-ND295-01, 35 |
| E5 launch failure, teardown absence | S-ND295-06 | S-ND295-40, 12 | S-ND295-12 | S-ND295-66 |
| E6 held ≤ cap | S-ND295-05D | S-ND295-04, 05A | — | T1 receipts report held/retiring |
| E7 no hot loop, recreate ordering | S-ND295-05C, 05D (NA-E7 over the run; NA-E7-C, one contended slot) | S-ND295-05A, 05E | — | — |
| E8 retry-retaining cleanup | S-ND295-07, 54 | S-ND295-07B, 54 (`server_lifecycle` shutdown body) | S-ND295-54 | — |
| E9 reclaim | S-ND295-55, 56, 57 | — | — | — |
| E10 boot after process loss | S-ND295-13A (seeded-in-process), 13D | S-ND295-13B, 13D (composed boot refusal) | — | S-ND295-13C |
| E11 supervisor matrix, latch | S-ND295-29A, 19, 32 | S-ND295-29B | — | S-ND295-37, 69 |
| E12 per-VM kill scope | S-ND295-30A, 50 | S-ND295-29B (C6) | S-ND295-10 | S-ND295-30B, 67 |
| E13 member/route/guard repair | S-ND295-61 | — | S-ND295-61 | — |
| E14 intercept-marked TCP fails closed | — | — | — | S-ND295-62, 63, 64 |
| E15 CLI consumes fail-stop | — | S-ND295-68 | — | S-ND295-68 (metal fault) |
| E16 required ports | S-ND295-65 | — | — | — |
| E17 DNS loss closes EXEC | S-ND295-29A | S-ND295-29B, 34 (probe refusal through the required port) | S-ND295-34 | S-ND295-34 (real bind) |
| E18 latency and mutex hold | — | — | — | M-ND295-E18 (measurement receipt) |
| E19 creation-time close-on-exec | S-ND295-46 (`scan_source` tables; `scan_workspace` in the xtask integration binary) | — | — | — |
| E20 cleanup-pending | S-ND295-58, 60 | S-ND295-59 | — | — |
| E21 launch filter | S-ND295-42, 44 | — | S-ND295-41, 43 | S-ND295-43 (f), 45, 01 |
| E22 managed-link identity independent of host link configuration | S-ND295-72 ((i1), (i2): the host-side MAC invariant at provision, `activate`, and the audit), 50 and 51 (reserved and missing rows) | — | S-ND295-72 (a, b `ensure_bridge`; d bridge mismatch; e a TAP judged by the invariant after udev), 00 (c bridge read-back) | S-ND295-72 (e on the metal host as provisioned) |
| E23 no panic on any owner / kill / supervisor path (user decision 2 of 2026-09-30; release is `panic = "abort"`) | — | — | the three `pids.max` thread-refusal cells (`overdrive-testing::pids_max`): `block_on_host_netlink_returns_connect_when_a_thread_is_refused` (→ `NetlinkError::Connect`, DELIVER 06-02), `quiesce_managed_taps_never_aborts_when_a_thread_is_refused` over one managed `Active` TAP (→ `Ok` with that TAP's `unconfirmed`/`Connect`, DR-08 (b)-A, DELIVER 06-04), `a_kill_write_under_thread_refusal_returns_an_io_error_and_never_aborts` (→ typed `io::Error`, DELIVER 09-01) | — |

The E23 lane is Lima-root only: the stimulus is a real cgroup-v2 `pids.max`
cap that refuses the OS thread an owner/kill path needs, with no seeded or
native-guest analogue. Each cell asserts the typed per-path outcome and that
the process never panics or aborts; each is RED today against the pre-fix
paths that panic on a refused thread (`std::thread::scope`'s `scope.spawn`;
`tokio::fs`/`spawn_blocking`), the no-panic-baseline violation the three
DELIVER steps resolve. See red-classification.md § *Phase G — Run 7*.

## Gate boundary coverage (G-295-0..5 × FD § "Required boundary scenarios per gate")

| Row | G-295-0 | G-295-1 | G-295-2 | G-295-3 | G-295-4 | G-295-5 |
|---|---|---|---|---|---|---|
| 1 available | 05A, 05C, 05D | 13A, 13C, 00 | 52, 01 | 51, 52, 01 | 35, 41, 43, 45 | 29A, 29B |
| 2 unavailable | 05A, 05E, 05C | 13D (worker and composed boot), 00, 15 | 52, 05E, 06, 07B | 53, 51 (incl. the bound miss), 52 (incl. the gate wait) | 38, 40, 41 (each failed hook step is a launch error), 44, 66 | 29A, 30A, 32, 33, 19 |
| 3 unrelated | 05B, 55 | 00, 14 | 52, 36 | 53, 52 | 38 (owner TAP state untouched) | 30A, 50, 10, 58 |
| 4 late success | 05C, 05D | 13D (a clear committed after the composed refusal) | 20-26 (retained registry fences) | 53 | 36 (existing VMM reaping) | 27, 29B (C7a/C7b), 32 (incl. a quiescence result after its bound) |
| 5 disconnect/reconnect | 05A (killed restart, empty pool) | 13C | 36 (VMM exit before release) | 51 (restore raises only activation-complete TAPs) | 39, 43, 50, 66 | 29A (DNS/listener loss), 70 (listener loss and exact rebind), 63, 64 |
| 6 feature disabled | not applicable — single cut, no feature flag (S-ND295-65 proves no optional composition switch) | n/a | n/a | n/a | n/a | n/a |

## Receipts — not tests

These are recorded separately from the Rust suite and are never counted as
test evidence. #295 has no verification expectation: every contract,
including the serve process's exit status, is a test through a production
port.

| Artifact | Kind | Contract | Home | Step |
|---|---|---|---|---|
| B-ND295-T1-BASE | benchmark receipt | N=16,384 Job-shaped attachments, M=0; exact inventories (now including the egress program, egress link, egress pin, and the ninth counter slot); held and retiring counts; memory/update samples; sweep and empty complement | `target/benchmarks/netns-density-295/T1-BASE/` via `crates/overdrive-control-plane/bin/netns_density_benchmark.rs` | 10-05 |
| B-ND295-T1-PORT4 | benchmark receipt | N=16,384 Service-shaped attachments with TCP 8080/8081/8443/9000; as T1-BASE plus 65,536 destinations | `target/benchmarks/netns-density-295/T1-PORT4/` | 10-05 |
| M-ND295-E18 | measurement receipt (a benchmark under `testing.md` § "Classify external execution before writing it") | At T1-BASE and T1-PORT4 with every attachment activated through the owner: full-audit latency per owner; `quiesce_managed_taps` / `restore_quiesced_taps` wall time and the time the last TAP reads back down; the worker member audit's `element_effects` hold time; and — per user decision 1 of 2026-09-30 (the kill loop) — the per-report kill loop's total wall time K and the largest single per-VM or workloads-slice `cgroup.kill` write W, measured with one scope and one Cloud Hypervisor process per allocation. It feeds the rules `SHARED_NETWORK_AUDIT_CALL_BOUND = max(1 s, 4 × L)`, `SHARED_NETWORK_QUIESCE_CALL_BOUND = max(1 s, 4 × Q)`, and `SHARED_NETWORK_VM_KILL_CALL_BOUND = max(1 s, 4 × W)` at T1-PORT4, rounded up to a whole millisecond; tests that a bound plus one attempt fits the 5 s window, and that a quiescence bound plus the whole kill loop K plus one attempt also fits it (the loop runs to its end before any further owner call or fail-stop); restates the double-loss exposure; and decides R15's mutex choice (> 100 ms reopens it). A full audit > 1 s at T1-PORT4, or a window misfit, goes to the user (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the full-audit call bound; quiescence and restore latency at density)). **Record.** The values have one record home: the rustdoc beside each value — the three bound constants (`SHARED_NETWORK_AUDIT_CALL_BOUND`, `SHARED_NETWORK_QUIESCE_CALL_BOUND`, `SHARED_NETWORK_VM_KILL_CALL_BOUND`) and the worker's `element_effects` field — naming the rule, receipt id M-ND295-E18, a host description, `uname -r`, the Cloud Hypervisor version, and the measured SHA (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the one record home)). Neither this table nor any DISTILL or DELIVER document records the values. The raw samples and the method are retained as a committed benchmark report at `docs/research/benchmarks/netns-density-295-m-nd295-e18/`, outside `docs/feature/**` (which finalize archives). The host is described by CPU model, kernel, and Cloud Hypervisor version, never by the metal target address (`OVERDRIVE_METAL_TARGET` is operator configuration) | raw output `target/benchmarks/netns-density-295/E18/` via the same benchmark binary (not committed); the committed report above | captured at 08-04; 09-01, the later of the capture step and the step that introduces the constants, sets all three bounds (audit, quiesce, and the per-VM kill-write bound `SHARED_NETWORK_VM_KILL_CALL_BOUND`) from the 08-04 receipt, re-capturing if that receipt is gone, and writes the rustdoc record and the report in the same commit (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the values before E18 and which step sets them)) |
| E20 cleanup-pending | test only | Decision: no verification expectation. No production-faithful black-box fault holds network cleanup pending for a bounded, observable interval without a test hook. A deleted program table is repaired within the recovery window and the stop is retried; a crashed VMM's predecessor cleanup runs after the one-second restart backoff; a per-allocation teardown failure cannot be induced from outside the process without a hook. A capture would be a race, not a deliberate contrasting case. The operator surface is fully exercised by S-ND295-59 (the same handler over the HTTPS API) and S-ND295-60 (the one live renderer). | — | — |

No benchmark or receipt carries a threshold DISTILL invented.

## Tier and fixture policy

- Layer 1-2 unbounded domains use proptest (pool operation sequences,
  held/retiring mixes, launch-filter `nr`/`args`, reconcile occupancy and
  ticks); finite vocabularies use one table per outcome (components, causes,
  per-allocation damage classes, verdict rows, error mappings).
- Layer 3+ (lima-kernel, in-process with real probes, native) is
  example-only; every sad path is a named body.
- Seeded bodies print the seed with every verdict and reproduce with the
  scenario's env var (or `PROPTEST_REPLAY`); they never lower
  `PROPTEST_CASES`.
- Every real-kernel fixture uses scratch names outside `ovd-tp-`, RAII
  cleanup, append-only diagnostics, and the whole-binary `host-kernel-shared`
  group of its crate. The fix pass put two more whole binaries in that group:
  `overdrive-control-plane`'s library test binary, whose
  `shared_owner_link_address_kernel` module mutates node-global names (DISTILL
  review B5), and `overdrive-host`'s integration binary (M4). The
  `overdrive-host` library test binary holds only scratch TAPs and needs no
  group. Hand-built netns fixtures (the S-ND295-64 controls) use
  `TestCidrLease`; no test draws a net slot.
- A native body that boots an in-process `serve` fails fast and never hangs:
  the VM poll panics as soon as the row it waits on is terminal and not the
  wanted state, `TeardownBound` (armed as each body's first statement) ends a
  failed body's teardown after 30 s by killing the workload scopes the body
  created, and the native-fault modules carry a 600 s nextest budget. A
  root-needing body asserts root; none skips silently (DISTILL review M6).
- A fixture establishes a scenario's preconditions and never authors the
  outcome its oracle observes. A precondition comes from the owner that
  produces it: a lease from the pool's own `assign`/`retire` (source-local only,
  FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the control-plane-private pool operations)), a gate state from the paired `GuestNetworkExecSupervisor`
  (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the gate state outside `run_server*`)), a restart predecessor's prior row from the precedent seed the
  restart arm reads (`mtls_install_fail_closed.rs:676-713`). No fixture authors
  a row, lease, gate transition, kill, or request that its scenario's oracle
  observes.
- The six `SHARED_NETWORK_*` constants are private items of the
  `overdrive-control-plane` crate root (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the private constants)). Source-local tests
  (S-ND295-19, 29A, 30A, 32) reach them through `super::` by name, never by
  literal, so a value that M-ND295-E18 sets never edits a test. Tests outside
  the crate's source cannot name them and none is widened for them: they assert
  ADR-0124's accepted cadence (1 s audit period, 250 ms attempt period, 5 s
  deadline, 20 attempts) as that contract and bound their waits as
  § *In-process observation* states (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (how the in-process lane bounds time without naming them)).

### Seam fixture (B-1, M4, L6)

Every body that drives `dispatch_with_guest_network_provisioner_for_test` or
`run_convergence_tick_with_guest_network_provisioner_for_test` builds its
`AppState` through one fixture helper per test file. The two seams keep their
accepted C-295-B signatures and read the EXEC gate and the pool from `state`
(FD § "C-295-B — network provisioner boundary" (the helpers read the EXEC gate and the pool from `state`)). The helper owns:

- **one owner instance**: in `overdrive-control-plane`'s `tests/` suites and
  in `overdrive-sim`, `Arc<SimSharedGuestNetworkOwner>` or a test-local type
  implementing both `GuestNetworkProvisioner` and `SharedGuestNetworkOwner`
  (a delegating decorator such as S-ND295-05D's `LeaseLedger`, or the owner of
  S-ND295-52 and 56 whose `activate` returns a scripted error); in source-local
  bodies, the `TestSharedOwner` of § *Test-local control-plane ports*
  (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (why each is a parameter: `shared_guest_network`));
- **one `GuestNetworkExecWiring`** over the fixture clock, keeping `gate()` and
  `supervisor()`. The gate starts BootClosed and only the supervisor moves it.
  A body whose dispatch reaches `claim_release` (activation, or
  `release_for_exit_emission`) first calls `supervisor.open_after_boot()`, or
  moves the gate to Recovering or FailStop to exercise the wait and withhold
  branches (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the gate state outside `run_server*`));
- **one `Arc<GuestAddressPool>`** from the doc-hidden `GuestAddressPool::new`
  with today's pool constants, passed only to `AppState` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the composition roots); FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (one pool per server: the pool as an `AppState` constructor parameter));
- **one `Arc<MtlsInterceptWorker>`** over `SimMtlsEnforcement`,
  `SimMtlsResolve`, and `SimMtlsIntercept` or a test-local `MtlsIntercept`
  whose `bind_transparent` delegates to an inner `SimMtlsIntercept`
  (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (why each is a parameter: `mtls_worker`); § *Intercept listener and stop-error test support*). No
  fixture substitutes a no-op worker.

DELIVER 05-01 cuts the pinned constructors (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors)). From then on the
helper passes the worker, owner, gate, and pool to `AppState::new` or
`new_with_workflow_engine`; until then it passes today's inputs and keeps the
rest for its bodies, every one of which is pending a step at or after 05-01. The
helper is the only line of each file that changes with the constructors.

- **One owner source (L6).** When `AppState` carries an owner and the seam
  receives a `provisioner`, the seam's network effects use only the
  `provisioner` (`action_shim/mod.rs:1433-1451`). A body passes the helper's one
  owner instance as that parameter, so `AppState`'s owner and the seam's
  provisioner are the same object and every owner observation has one source.
- **Worker state and lane.** A body whose dispatch reaches intercept install
  first starts the worker's shared owner (`start_shared_owner`). Over the
  fixture's worker, that binds no socket and runs no accept thread
  (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `SimMtlsIntercept` contract)), so the body keeps the lane its other properties give it
  (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification)). A body whose contract ends before any install (a refusal at
  `assign` or `provision`) may leave the shared owner unstarted: its cleanup's
  `stop_alloc` of an allocation the worker never registered returns `Ok`
  (`mtls_intercept_worker.rs:2895-2903`; B-6 caller rule 2, FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (what each caller receives, rule 2)). The
  source-local supervisor bodies (S-ND295-19, 29A, 30A, 32) start the worker's
  shared owner in the default lane, as the existing
  `shared_network_task_owner_acceptance` module already does.
- **Lease observation.** The pool's operations are crate-private
  (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the control-plane-private pool operations)). A body outside the crate's source observes lease state through
  the pinned events `guest_network.lease_retired { alloc }`,
  `guest_network.lease_released { alloc }`, and
  `guest_network.admission_refused { alloc, held, retiring, cap }`
  (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the admission refusal projection and the lease events)), captured by a test-local tracing Layer, and observes assigned
  addresses through the guest-network assignment in `SimDriver::started_specs()`
  (the C-295-A handoff). Only source-local bodies call `assign`, `retire`,
  `observe`, or `snapshot`.

### In-process observation (L4)

In the in-process lane the server runs `run_server_with_obs_and_driver(s)` and
the test advances the injected clock (`ServerConfig.clock`, a `SimClock`) in
steps (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (how the in-process lane bounds time without naming them)).

- `GuestNetworkExecWiring` is taken by value (FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)). The test keeps
  `wiring.gate()` and `wiring.supervisor()` before moving the wiring into the
  call (`overdrive-core/src/guest_network.rs:162-172`).
- After each clock step the test polls each waiting observation exactly once
  with `FutureExt::now_or_never` and drops the future:
  - `gate.claim_release()` (`guest_network.rs:176-200`): pending means
    admission is closed (BootClosed or Recovering); `Some(Some(claim))` means
    Open, and the claim is dropped at once, so the test never holds a claim
    across a step or against a recovery; `Some(None)` means FailStop.
  - `handle.shutdown_requested()` (`lib.rs:2584-2588`): pending until a request;
    dropping it is cancel-safe, because it selects over the retained task and
    the request receiver (`lib.rs:1270-1290`).
- `supervisor.recovery_progress()` (`guest_network.rs:276-288`) and
  `supervisor.is_boot_closed()` are non-blocking and read directly.
- No observation future is held across a step, and none is awaited.
- Horizons: detection within one audit period (1 s) of a non-hung fault, and
  within 5 s of injected time for a hung audit; every recovery outcome within
  the 5 s window after detection (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (how the in-process lane bounds time without naming them)).

### Test-local control-plane ports (B-4 as pinned, L8)

`overdrive-sim` depends on `overdrive-control-plane`, which dev-depends on
`overdrive-sim`, so in the control-plane crate's own source-local tests the sim
types implement a second compiled copy of the control-plane traits and do not
coerce to `Arc<dyn crate::…>` (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the seeded-sim lane: why the `overdrive-sim` owner and DNS types cannot serve source-locally)). Source-local bodies therefore use
test-local implementations of the three ports the crate declares, defined once
in the crate-private module `crates/overdrive-control-plane/src/shared_network_test_ports.rs`
(`#[cfg(test)] mod shared_network_test_ports;` in `lib.rs`). It replaces the
file-local `S19SharedOwner` (`lib.rs:1819-1888`). Its shape is test-support
surface decided here; it is not production API, and only this crate's
source-local tests call it.

- **`TestSharedOwner`** implements `GuestNetworkProvisioner` and
  `SharedGuestNetworkOwner` and models the parts of `SimSharedGuestNetworkOwner`'s
  pinned contract the cells use (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the sim adapter); FD § "Public deterministic shared-owner simulation API" (the pinned surface as the replacement DESIGN changes it)): standing refusal
  slots for provision, teardown, converge, and restore; one standing slot per
  node-level component for `audit_shared`; a standing damage set; a standing
  quiescence outcome (`Unconfirmed(set)`, `Fail`, `Hang`, or `Late {
  unconfirmed, after }`, which stays pending and resolves only after `after`
  on the owner's clock — S-ND295-32's late result); a restore that raises the
  first `n` quiesced TAPs and then fails (`script_restore_failure_after(n)`;
  `script_restore_failure` resets `n`), with the raised and quiesced sets
  modelled so a repeat quiescence sets down exactly what a part-way restore
  raised (S-ND295-29A); an audit that
  hangs or panics (S-ND295-29A, 32); a condemned set (an allocation named by a
  quiescence or audit result is never named again, and `activate` refuses it
  with the source-less `PostconditionMismatch`); and a latch that
  `quiesce_managed_taps` sets before it returns and only a
  `restore_quiesced_taps` returning `Ok` clears, under which `activate` returns
  `Ok(QuiescenceLatched)` with no effect.
  - **Observation.** `journal()` returns one entry per port call, in call
    order, naming the call, its allocation where it has one, and its outcome:
    `Provision`, `Activate` (`Raised`, `Latched`, or `Refused`), `Teardown`,
    `ProbeStartup`, `SweepStale`, `ConvergeShared`, `AuditShared` (`Healthy`,
    `NodeFailed(component)`, `Damaged(set)`, `Hung`, or `Panicked`), `Quiesce`
    (`Unconfirmed(set)`, `Failed`, `Hung`, or `Late`), and `Restore` (`Ok` or
    `Failed`). `active()` reads the allocations currently raised. When built
    over a `SimCgroupFs` clone and the owner's clock
    (`with_cgroup_snapshots(fs, clock)`), each entry also carries the
    `snapshot()` taken as the call began, which is the E12 ordering observation
    point (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the `SimCgroupFs::snapshot()` oracle)). `latched()` and `condemned()` read the latch and the
    condemned set directly.
  - **Latch invariant (L8).** The E11 latch invariant (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E11 row)) reads
    `latched()` — the bit `activate` itself consults — and never re-derives a
    latch from a call log. A log cannot derive it soundly: the pinned sim owner
    records `TapSetUp` for activations and for restores that fail as well as
    succeed (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the sim adapter's restore rule); FD § "Public deterministic shared-owner simulation API" (`activate`, and `restore_quiesced_taps` recording `TapSetUp`; the call journal)). The journal still names
    each restore's outcome, so an ordering oracle can tell a failed restore
    from a successful one.
  - **Plans.** Its self-tests and every source-local body build a
    `GuestNetworkPlan` through a test-owned `GuestAddressPool::new(..).assign`,
    never the process-global action pool that 06-03 deletes (DISTILL review
    DR-11).
- **`TestGuestDnsFactory`** and **`TestGuestDns`** implement
  `GuestDnsFactory` and `GuestDns` with the pinned `SimGuestDnsFactory` /
  `SimGuestDns` semantics (FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` and `SimGuestDnsFactory` doubles)): a standing probe-refusal slot shared
  by every responder the factory built, a build log, a per-responder standing
  audit-refusal slot, and a `serve` that stays pending until `end_serve(Return)`,
  `end_serve(Panic)`, or `stop`.

`SimSharedGuestNetworkOwner`, `SimGuestDns`, `SimGuestDnsFactory`, and
`SimGuestAttachmentView` serve `overdrive-control-plane`'s `tests/` suites and
other crates, with exactly their pinned API (FD § "Public deterministic shared-owner simulation API" (the pinned surface as the replacement DESIGN changes it); FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` and `SimGuestDnsFactory` doubles); FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (`SimGuestAttachmentView`)).

### Intercept listener and stop-error test support (B-6, B-7)

This is test-support surface decided here, not production API. B-7 leaves the
sim listener's scripting names to DISTILL (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `SimMtlsIntercept` scripting surface)), and B-6 leaves the
sim lifecycle's variant mapping to DISTILL (FD § "[REF] Required downstream changes (not edited by DESIGN)" (the consequences of pins B-6 and B-7: the sim lifecycle)).

**When the port changes.** B-7's port change — `bind_transparent`'s return
type, the host listener, the worker's accept tasks, and the deletion of
`start_alloc`'s per-allocation listener branch — lands in the DELIVER step
that carries B-7, no later than 05-01 (FD § "[REF] Required downstream changes (not edited by DESIGN)" (the consequences of pins B-6 and B-7: DELIVER)). Phase B does not change
`bind_transparent`: adopting the new return type needs the host listener and
the worker's accept tasks, which are B-7's production work (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `HostMtlsIntercept` obligations; the notes for DELIVER review)), and phase B changes no production behaviour. Phase B adds
`InterceptListener`, `InterceptAccepted`, and `InterceptAcceptError` exactly as
pinned (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the pinned contract)), unused by production, so the test support below
implements the listener trait from phase B on. Every phase-B test compiles on
both sides of the change: a test-local intercept delegates `bind_transparent`
to an inner adapter, and test code that reads a bound listener goes through
the `LegListener` bridge. The B-7 step changes exactly these test-support
lines and no test body:

1. the `Ok` arm of `SimMtlsIntercept::bind_transparent` returns the registered
   `SimInterceptListener`, and the module documentation saying that arm binds a
   real socket and is integration-lane
   (`overdrive-sim/src/adapters/mtls_intercept.rs:18-23`, `:260-266`) is
   corrected with it;
2. `TestSharedIntercept::bind_transparent` returns its `TestInterceptListener`;
3. `RecordingSharedIntercept::bind_transparent` returns a
   `LoopbackInterceptListener`; its `listener_clones` hold
   `Arc<LoopbackInterceptListener>`, and `terminate_listener_task` calls
   `lose()`;
4. the `std::net::TcpListener` implementations of the `LegListener` bridges are
   deleted, having no caller;
5. the one line per file that names the listener type a test-local double's
   `bind_transparent` returns — `type BoundListener = std::net::TcpListener;`
   (or `TcpListener` in a source-local module) — becomes
   `type BoundListener = Arc<dyn InterceptListener>;`. Every double spells its
   `bind_transparent` return through that alias, so its delegation compiles
   unchanged on both sides of the step (DISTILL review B6 (a)). The files:
   `overdrive-control-plane/src/lib.rs`,
   `overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs`,
   `overdrive-control-plane/tests/integration/{boot_member_clear_refusal, mtls_install_fail_closed, network_cleanup_pending_status, server_lifecycle, shared_element_cleanup_failure, shared_network_supervisor_recovery}.rs`,
   `overdrive-worker/src/mtls_intercept_worker.rs`,
   `overdrive-worker/tests/integration/{netns_density_shared_owner, outbound_enforce_substrate_splice}.rs`,
   `overdrive-sim/src/invariants/netns_density_boot_order.rs`, and
   `overdrive-sim/tests/acceptance/netns_density_retiring_cleanup.rs`.

**`SimMtlsIntercept` listener surface** (`overdrive-sim`, fixture class F;
the contract is FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `SimMtlsIntercept` contract)):

```rust
// overdrive_sim::adapters::mtls_intercept, re-exported beside SimMtlsIntercept
pub struct SimInterceptListener { /* private; implements InterceptListener */ }

#[derive(Debug)]
pub enum SimAcceptScript {
    /// The next `accept` returns `Ok(accepted)`.
    Connection(InterceptAccepted),
    /// The next `accept` returns `Err(OriginalDestination { source })`, with
    /// `io::Error::from_raw_os_error(errno)`; the listener stays usable.
    OriginalDestinationFailure { errno: i32 },
    /// That and every later `accept` returns `Err(Accept { source })`, with
    /// `io::Error::from_raw_os_error(errno)`: the listener accepts nothing more.
    ListenerLost { errno: i32 },
}

impl SimMtlsIntercept {
    #[must_use] pub fn script_accept(&self, at: SocketAddrV4, script: SimAcceptScript) -> bool;
    #[must_use] pub fn script_local_addr_failure(&self, at: SocketAddrV4, errno: i32) -> bool;
    #[must_use] pub fn live_listeners(&self) -> Vec<SocketAddrV4>;
    #[must_use] pub fn parked_accepts(&self, at: SocketAddrV4) -> usize;
}
```

- *Bind.* The standing bind fault fires first, unchanged. Otherwise a bind at
  port 0 takes the smallest port ≥ 49152 that no live listener of this adapter
  holds at that IP; a non-zero address is honoured exactly; an address a live
  listener holds is refused with `InterceptError::TransparentListener { addr,
  source }` whose source is `io::Error::from_raw_os_error(libc::EADDRINUSE)`.
- *Liveness.* A listener is live from its bind until its last `Arc` drops. The
  adapter keeps only a `Weak` reference, so neither the adapter nor a script
  extends a listener's life. `live_listeners()` returns the live addresses in
  ascending order.
- *Scripts.* `script_accept(at, s)` appends `s` to the FIFO of the live
  listener at `at` and wakes a parked `accept`. `script_local_addr_failure(at,
  errno)` makes that listener's `local_addr()` return
  `Err(io::Error::from_raw_os_error(errno))` from then on. Both return `false`,
  and record nothing, when no live listener holds `at`. Scripts die with their
  listener; a new listener at the same address starts with none.
- *Accept.* Polled inside a Tokio runtime with nothing scripted, `accept`
  stays pending and consumes nothing. `parked_accepts(at)` counts that
  listener's `accept` futures that have been polled, are pending, and are not
  dropped (0 when no live listener holds `at`). Dropping a pending future
  lowers the count and takes no script. Polled with no current runtime,
  `accept` returns `Err(Accept { source })` whose `io::Error` names the
  missing runtime, and consumes nothing.
- The adapter holds no socket, descriptor, thread, task, timer, clock, or
  entropy.

Before the B-7 step the `Ok` arm still returns a real socket, so
`live_listeners()` is empty and both scripting calls return `false`.

**Worker source-local support** (`overdrive-worker/src/mtls_intercept_worker.rs`
test code). `overdrive-sim` depends on `overdrive-worker`, so in the worker's
own source-local tests `SimMtlsIntercept` implements a second compiled copy of
`MtlsIntercept` and cannot be used. That file's test code defines, once and
shared by its test modules:

- `TestInterceptListener`, with exactly `SimInterceptListener`'s semantics;
- on `TestSharedIntercept`: the live-listener table and `script_accept`,
  `script_local_addr_failure`, `live_listeners`, and `parked_accepts` with the
  sim's signatures and meaning (over a local `TestAcceptScript` with
  `SimAcceptScript`'s variants); `script_removal_failures(&self, count: usize,
  cause: fn() -> InterceptError)`, after which the next `count` calls to
  `remove_allocation_elements` each return a fresh `cause()`;
  `removal_calls(&self) -> usize`, counting every call on entry; and
  `hold_removals(&self, held: bool)`, under which each call waits after
  entering until the hold is released (the condvar barrier
  `RetirementBarrierIntercept` uses), so S-ND295-54 can join a caller to an
  attempt that is provably in flight; and `node_guard_drops(&self) -> usize`,
  counting drops of the guards `converge_shared` returned.

**Worker shared-owner support**
(`overdrive-worker/tests/integration/netns_density_shared_owner.rs`).
`LoopbackInterceptListener` implements `InterceptListener` over a plain
loopback `tokio::net::TcpListener` bound at the requested address: `local_addr`
reads the socket; `accept` is tokio's cancel-safe accept, returning the accepted
stream as a blocking `OwnedFd`, the peer, and `getsockname` as `local`; and
`lose()` shuts the listening socket down, so a pending or later `accept`
returns `Accept`. It exists so the ten RETAINED S-ND295-20..26 bodies keep
their oracles — real client connections from allocation source addresses, and
real socket release — unchanged. Because those bodies bind real sockets and make
real connections, the file lives in the `overdrive-worker` integration binary
(`.claude/rules/testing.md` § "Integration vs unit gating"); phase B moved it
there from `tests/acceptance/` with no assertion change (lane decision settled
2026-09-25).

**`LegListener` bridge.** A test-local trait with `fn bound_v4(&self) ->
std::io::Result<SocketAddrV4>` and a blocking `fn accept_leg(&self) ->
std::io::Result<(OwnedFd, SocketAddrV4, SocketAddrV4)>` (stream, peer,
local), implemented for `std::net::TcpListener` and for
`Arc<dyn InterceptListener>`. The `TcpListener` implementation accepts through
the production `accept_outbound_and_recover_orig_dst` and reads the peer from
the accepted socket; the port implementation drives `accept()` on a
current-thread runtime it builds. One copy lives in
`overdrive-worker/tests/integration/leg_listener.rs` (registered in
`tests/integration.rs`) for the port equivalence harness,
`MetalSharedIntercept`, and the original-destination bodies;
`overdrive-control-plane/src/shared_network_test_ports.rs` carries a
`bound_v4`-only copy for `S19Intercept`.

**Test-local intercept doubles** (`grep -rn 'impl MtlsIntercept for' crates/`;
FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on every existing `impl MtlsIntercept`)). Each returns an `InterceptListener` once the B-7 step lands:

| Double | File | Phase B / fix pass | At the B-7 step |
|---|---|---|---|
| `TestSharedIntercept` | `overdrive-worker/src/mtls_intercept_worker.rs` (source-local) | faithful new trait methods; the support above | returns `TestInterceptListener` (item 2); its alias (item 5) |
| `S19Intercept` | `overdrive-control-plane/src/lib.rs` (source-local) | delegates `bind_transparent` to an inner `SimMtlsIntercept`, exposed as `sim()`, and records the bound address through `LegListener::bound_v4` | its alias (item 5) |
| `RecordingSharedIntercept` | `overdrive-worker/tests/integration/netns_density_shared_owner.rs` | unchanged listener | returns `LoopbackInterceptListener` (item 3); the file's alias (item 5) |
| `ActivationBarrierIntercept` | same file | delegates to `RecordingSharedIntercept`, as today | the file's alias (item 5) |
| `MetalSharedIntercept` | `overdrive-worker/tests/integration/outbound_enforce_substrate_splice.rs` (native) | delegates to `HostMtlsIntercept` and reads the address through `LegListener::bound_v4` | its alias (item 5) |
| `ElementFaultIntercept` | `overdrive-control-plane/tests/integration/shared_element_cleanup_failure.rs` | delegates to its `SimMtlsIntercept`, as today | its alias (item 5) |
| `RetirementBarrierIntercept`, `JournalIntercept` | `overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs` | delegate to a `SimMtlsIntercept` | the file's alias (item 5) |
| `RemovalFaultIntercept` | `overdrive-control-plane/tests/integration/server_lifecycle.rs` (S-ND295-54) | delegates to its `SimMtlsIntercept`; armable removal fault | its alias (item 5) |
| `RemovalFaultIntercept` | `overdrive-control-plane/tests/integration/network_cleanup_pending_status.rs` (S-ND295-59) | delegates to its `SimMtlsIntercept`; removal fails until disarmed | its alias (item 5) |
| `ProofIntercept` | `overdrive-control-plane/tests/integration/shared_network_supervisor_recovery.rs` (S-ND295-29B) | delegates to its `SimMtlsIntercept`; the `program_lost` overlay | its alias (item 5) |
| `RecordingBootIntercept` | `overdrive-control-plane/tests/integration/boot_member_clear_refusal.rs` (S-ND295-13D, fix pass) | delegates to a seeded `SimMtlsIntercept`; scripted boot-clear outcomes | its alias (item 5) |
| `RecordingIntercept` | `overdrive-control-plane/tests/acceptance/netns_density_guest_network.rs` (S-ND295-07, 56) | delegates to its `SimMtlsIntercept`; records removals, out-of-band member removal | its alias (item 5) |
| `BootOrderIntercept` | `overdrive-sim/src/invariants/netns_density_boot_order.rs` (S-ND295-13A) | records every call over a seeded `SimMtlsIntercept` | its alias (item 5) |
| `RemovalFaultIntercept` | `overdrive-sim/tests/acceptance/netns_density_retiring_cleanup.rs` (S-ND295-07, fix pass) | delegates to its `SimMtlsIntercept`; armable removal fault | its alias (item 5) |

**Worker bodies that drive the per-allocation listener branch.** The B-7 step
deletes that branch together with the bodies that drive it (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the notes for DELIVER review: the per-allocation listener branch)).
Their contracts stay covered:

| Body today (`mtls_intercept_worker.rs::tests`) | Carried after the step by |
|---|---|
| `idle_accept_wait_stops_when_the_worker_owner_is_released` | S-ND295-70 `an_idle_accept_task_ends_when_the_last_worker_reference_drops` |
| `owner_shutdown_joins_children_closes_sockets_and_retains_each_rule_guard` | S-ND295-70 `owner_shutdown_ends_both_accept_tasks_releases_both_listeners_and_relinquishes_the_node_guard` |
| `allocation_stop_surfaces_teardown_failure_and_retry_converges` | itself, RETARGETED in phase B onto a shared allocation (S-ND295-54) |
| `replacement_shutdown_waits_for_the_same_authoritative_teardown`, `same_owner_reinstall_waits_for_prior_teardown_before_readiness`, `same_owner_reinstall_failure_keeps_readiness_closed_until_retry`, `completed_enforce_handle_is_torn_down_not_orphaned`, `allocation_stop_joins_an_inflight_enforce_child`, `allocation_stop_joins_a_passthrough_child`, `allocation_start_after_owner_shutdown_is_rejected_before_install` | a NEW twin in the same module named `shared_<original name>`, with the per-allocation registration replaced by `start_shared_owner` and a `shared_spec` allocation, each connection delivered by `TestSharedIntercept::script_accept`, and the original oracle. A twin that needs a scripted connection carries `pending DELIVER step 05-01 (S-ND295-20)`; a twin that passes today stays active |

The originals stay active until the B-7 step removes them with the branch.

**Port equivalence harness**
(`overdrive-worker/tests/integration/mtls_intercept_equivalence.rs`).
`bound_ipv4_port` reads through `LegListener::bound_v4`; S-MIF-09..12 keep
their assertions; the module's lane statement names one reason, that
`HostMtlsIntercept` needs `CAP_NET_ADMIN` and real `nft`, which holds on both
sides of the change. S-ND295-70 adds the held-address clause.

**Original-destination evidence** (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the `HostMtlsIntercept` obligations: original-destination evidence)). Whether the host listener
reuses `accept_outbound_and_recover_orig_dst` and `accept_inbound_leg` is the
crafter's choice, so no test that must outlive the B-7 step calls them
directly:

- The two bodies whose whole contract is a helper's —
  `mtls_intercept_install.rs` at `:974` (outbound) and `:1309` (inbound) —
  stay as they are and are deleted with their helper if the step deletes it,
  together with the helper's source-local unit test
  (`mtls_intercept.rs:1226`). Their evidence is then carried through the host
  listener by S-ND295-70's two original-destination bodies.
- The bodies that use a helper as one step of a wider contract —
  `egress_tproxy_capture.rs` (`:647`, `:762`) and
  `name_resolve_enforce_consistency.rs` (`:629`), all outbound — bind through
  `HostMtlsIntercept::bind_transparent` and accept through
  `LegListener::accept_leg`, asserting the recovered original destination as
  today. Before the step the bridge reaches the production outbound helper;
  after it, the host listener. Their documentation that names the helper as a
  mutation target names the port instead. The fix pass also moved their
  outbound divert off the hand-installed `install_outbound_tproxy` rule, which
  no production path installs under the shared design, onto the production
  shared program (`SharedOutboundDivert`: `converge_shared`,
  `install_outbound`, and the guest TCX classifier), with the node-global
  sysctls restored by a guard (DISTILL review H9).

**Stop-error fixtures (B-6).** `SimMtlsInterceptLifecycle` keeps
`inject_stop_failure_once(alloc_id, detail)` and its events'
`failures: Vec<String>` (the scripted detail strings). A scripted stop failure
is returned as `MtlsInterceptStopError::HandleTeardown { alloc_id, failures:
vec![HandleTeardownFailure { connection, source }] }` with `connection =
EnforcedConnectionId::new(alloc_id.clone(), 0)` and `source =
Arc::new(MtlsEnforcementError::TeardownFailed { id: connection.clone(),
source: io::Error::other(detail) })` — one transiently failed connection
teardown, which is what the sim models. `StartPriorTeardownFailed` records the
detail string it popped, not a field of the error. `RecordingMtlsLifecycle::stop_alloc`
(`overdrive-sim/tests/driver_neutral_allocation_replacement.rs:553`) only
forwards the inner result and constructs no error, so it needs no change.
The DR-06 detail bodies (S-ND295-54) use a test-local `StopFaultLifecycle`
(`overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs`), an
`MtlsInterceptLifecycle` whose successor stop returns either a
`HandleTeardown` with two failures (`EBADF`, `ENOTCONN`) or an
`ElementRemoval` (`EBUSY`), built directly from the pinned shapes.

## PBT/parametrize density and vocabulary

- Unbounded: S-ND295-04, 05A, 05B, 05C, 13A, 27, 42, 55 (proptest).
- Finite, table-driven: S-ND295-11, 12, 29A, 30A, 40, 44, 47, 49, 50, 51, 58,
  61, 70 (one row per `SimAcceptScript` outcome), 72 (one row per reserved
  class of the host-side MAC invariant, plus the missing address).
- Seeded schedules: S-ND295-05D, 07 (the retiring-cleanup schedule, fix pass), 29A, 29B, 30A, 32, 53, 57; S-ND295-19 is one deterministic schedule with a printed id.
- Shared test vocabulary (helpers only, never production types):
  `GuestAttachment`, `HeldLease`, `RetiringLease`, `ManagedTap`,
  `ProtectionLive`, `ReleaseLast`, `ProofMark`, `CondemnedVm`.
- Step-reuse ratio (informational, no gate): the 65 prose Gherkin blocks
  contain 338 Given/When/Then lines and 338 distinct line texts — **1.00×**
  (recomputed 2026-09-28).
  Rust has no step-decorator layer; each prose step is written for its
  scenario, and readability is not collapsed to raise the ratio. Reuse lives
  in the shared Rust fixtures named in each field table.

## Completeness audit (canonical 15 items)

| Item | Result | Evidence |
|---|---|---|
| C1a empty/minimum | PASS | empty pool after killed restart (05A); zero leases (58); zero members at boot (13A/13C); zero frames before the event (01) |
| C1b boundaries | PASS | 16,383/16,384/16,385 held with retiring mixes (05A, 05B); `nr = -1`, `>= 0x4000_0000` (42); descriptor 3/4 boundary (41); one-second cadence edges (55, 57); 20 attempts / 5 s (29A) |
| C2a state machine documented | PASS | lease Admitted→Retiring→released (04); owner ProvisionedDown→Active→QuiescedActive/Condemned (51); gate BootClosed/Open/Recovering/FailStop (27) |
| C2b illegal event per state | PASS | assign on Retiring (04); activate while latched / condemned (51); restore with no latch (51); complete_attempt after FailStop (27, 29B); reclaim beside another action (55); stop after owner shutdown began (54); bind at a held address (70); element install or removal before any convergence (71) |
| C3 zero/one/many | PASS | 0/1/N damaged allocations (30A); 0/1/many members (54, 61); 0/1/2 receipted programs (48); two VMs (35, 67) |
| C4a apply twice | PASS | repeat activate is idempotent (51); a repeat quiesce while latched sets down only what a part-way restore raised and is otherwise no-I/O (51, 29A); convergent removal with pre-absent members (54); teardown on absent parts (12); re-converging to the recorded program adopts it (71); `ensure_bridge` adopts a present link without writing (72) |
| C4b inverse without prerequisite | PASS | release of an absent lease (04); reclaim without a lease (56); teardown of absent parts (12, 66) |
| C5a mode combinations | PASS | x86_64 vs other targets (42, 44); cold boot vs killed restart (13A, 13C); Job vs Service render (60); node-level vs per-allocation audit (50) |
| C5b orthogonality | PASS | listener/DNS loss never quiesces (29A); damage while Open leaves the gate Open (30A); non-pending rows render byte-identically (60) |
| C6a malformed input | PASS | duplicate/zero-port destinations (54); non-persistent / raised / busy TAP (38); foreign ABI syscalls (42, 43) |
| C6b each declared error | PASS | every `TapQueueError`/`VmmError`/`VmmProbeError` variant (38, 40, 44); a failed launch-hook step as `VmmError::Create` (41); `MembersRemain`, `PolicyRouteAbsent`, `InterceptMarkGuardAbsent` (13D, 61); the DR-06 Failed-row detail (54); R10's two element-batch failure arms over the `SharedIpElementIo` seam (54, H14 resolved); `AdmissionCapReached`/`LeaseRetiring` (05A, 04); `ElementRemoval`/`HandleTeardown` (54, 07B); `BootMemberClear`/`MemberMismatch`/`MemberRepair` (13D, 61); both new fail-stop causes (30A, 27); `SharedProgramNotConverged` and the ordered install/removal partition (71); the `TapHostMac { address: Reserved | Missing }` and `Bridge` refusal facts (72, 50, 51); a refused OS thread as `NetlinkError::Connect` / `io::Error` on the owner, quiesce, and kill paths (E23 no-panic lane, 06-02/06-04/09-01) |
| C6c closed error set | PASS | exhaustive mapping tables with no catch-all (40, 44, 61 `component()`); eight-cause gate table (27) |
| C7a degraded resource | PASS | a netlink session lost on one TAP or on every TAP (51, 30A); a set-down that never completes (51); failed debug-mask dump (50); kill write failure (30A); a guest that powers off before READY (66) |
| C7b interruption | PASS | killed-mode serve (13C, 63); hung audit/quiesce (32, 30A); a quiescence result after its bound (32); activation in flight during detection (29B C9); a part-way restore (29A, 29B C4b) |
| C7c concurrent actors | PASS | in-flight admission (05D); two workloads placed on one free slot (05D NA-E7-C); raced restart refusal (05C, 05D); concurrent second VM launch (35); element mutex vs audit (61); joined and simultaneous stop callers (a 16-round barrier race), and a stop racing owner shutdown (54) |

**Verdict: COMPLETE — 15/15**, no open gap. The last open item —
R10's two element-batch failure arms at the adapter (a kernel-rejected delete
batch leaves every member unchanged; a post-commit read-back failure performs
one inverse transition and keeps both causes) — is resolved by user decision 3
of 2026-09-30: both arms rest on the kernel's batch abort and are asserted
source-local, default lane, over the NEW module-private `SharedIpElementIo`
seam (S-ND295-54's four seam cells; a behaviour-preserving `mutate_and_readback`
extraction, no new public surface, its scripted double DISTILL's test support),
and the worker's handling of both stays additionally proven in-process
(S-ND295-07B) (§ *DESIGN gaps*, H14). No domain extension is opted in.

## DESIGN gaps — pinned

Every gap DISTILL returned is pinned: B-1 to B-5 and N-1 from the first pass,
approved in two DESIGN review rounds; B-6 and B-7, found while aligning the
scenarios to those pins; and B-8 and the fresh-host RCA (N-4), pinned
2026-09-26 while classifying the phase-C run. N-4's TAP half was re-decided by
the user rulings of 2026-09-28 (no host link-configuration requirement; a TAP's
host-side MAC judged by the D-295-R21 invariant, reported as
`TapHostMac { ifindex, address: TapHostAddress }`). The iteration-1 review
routed further DESIGN pins (2026-09-29) and user decisions (2026-09-30), which
the fix pass follows. The one returned testability boundary (H14, below) is
now resolved by user decision 3 of 2026-09-30 (the `SharedIpElementIo` seam);
every gap is pinned and the scenarios above follow the pins.

| ID | Gap as returned | Pin (FD) | What changed here |
|---|---|---|---|
| B-1 | How `AppState`'s two public constructors obtain the pinned `mtls_worker` (R16), `guest_network_exec` (R5), and `guest_pool` (R6), and which gate state a fixture starts in | **PINNED** — both constructors take the worker, the shared owner, the gate, and the pool as required parameters (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `AppState` constructors and the `ServerHandle` owner fields)); the two test-gated seams keep their C-295-B signatures and read the gate and pool from `state` (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (how the gate reaches the shim); FD § "C-295-B — network provisioner boundary" (the helpers read the EXEC gate and the pool from `state`)); a fixture's gate stays BootClosed until the fixture's paired supervisor moves it (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the gate state outside `run_server*`)); `ServerHandle`'s owner fields lose their `Option`, and `replace_mtls_worker_for_test` and `inject_owner_shutdown_failure_for_test` are deleted (FD § "[REF] Driven port — TAP activation gate (D-295-R5) — ACCEPTED 2026-09-24" (the `ServerHandle` owner fields)) | Seam calls drop `exec_gate`/`guest_pool` (S-ND295-06, 05E, 07, 52, 53, 56); § *Seam fixture*; S-ND295-65 scans the owner, both `ServerHandle` owner fields, and after-boot replacement; the `server_lifecycle` shutdown body is retargeted to an `ElementRemoval` injected through `ServerConfig.mtls_intercept` (S-ND295-54) |
| B-2 | No production-faithful native stimulus for E12's whole-call quiescence failure | **PINNED** — E12's native lane proves per-VM kill writes only; the whole-call slice kill is proven by the seeded and in-process cells through a `SimCgroupFs` snapshot; `run_server_with_obs_and_driver(s)` take a final `vm_cgroups: CgroupManager` (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (where the capability's `CgroupManager` comes from); FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E12 whole-call branch); FD § "EXEC-close linearization" (the `run_server_with_obs_and_driver(s)` signatures)) | S-ND295-30B carries no native whole-call case; S-ND295-29B C6a/C6b and 30A assert snapshot writes; 05A, 13A, 29B, 33, 36, 59 pass `vm_cgroups` |
| B-3 | E19 named "an `xtask` check" without an entry point | **PINNED** — `xtask::cloexec_lint::{scan_source, scan_workspace, render_violation, run}`, `CloexecRule`, `CloexecViolation`, `Task::CloexecLint` / `cargo xtask cloexec-lint`; `scan_workspace` fails closed on an unreadable or unparseable file; the obligation's site table has eight sites (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (obligation OBL-295-CLOEXEC, its source gate, and the gate entry point)) | S-ND295-46 bodies authored against the pinned entry point, with the unparseable-file case |
| B-4 | `SimGuestAttachmentView`, `SimGuestDnsFactory`, and `SimGuestDns` named without scripting signatures | **PINNED** — exact sim surfaces (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (`SimGuestAttachmentView`); FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (the `SimGuestDns` and `SimGuestDnsFactory` doubles)); source-local supervisor lanes use crate-private test-local ports because of the `overdrive-sim` ↔ `overdrive-control-plane` dependency cycle (FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the seeded-sim lane and its test-local ports)) | § *Test-local control-plane ports*; in-process and `tests/` bodies use the pinned sim types |
| B-5 | Home of the E18-derived call bounds | **PINNED** — one record home, the rustdoc beside each value; the later of the capture step and the constants step (09-01 in this plan) sets and records the values; source-local tests name the constants, in-process tests assert ADR-0124's contract (FD § "[REF] Runtime shared-network supervisor (D-295-R13, R14, R15, R16) — ACCEPTED 2026-09-24 (R14 kill scope user ruling of the same date)" (the private constants and the E18-derived bounds' record home); FD § "[REF] Required downstream changes (not edited by DESIGN)" (the E18-derived bounds, B-5)) | The DISTILL bounds table is withdrawn; the M-ND295-E18 receipt row and S-ND295-29B, 32 follow the pin |
| B-6 | How `stop_alloc` and `shutdown_owner` return R10's typed stop error, whose sources are not `Clone`, to every concurrent or later caller | **PINNED** — `MtlsInterceptStopError` and `HandleTeardownFailure` are `Clone`, holding each typed source as `Arc<InterceptError>` / `Arc<MtlsEnforcementError>`; a caller reaches the cause through the field (`&*source`); callers joined on one attempt receive clones with pointer-equal sources, a later caller after `Err` begins exactly one atomically claimed retry, and after owner shutdown `stop_alloc` begins nothing and returns that allocation's shutdown entry or `Ok`; `MtlsInterceptOwnerShutdownError`, `ShimError::MtlsStop`, and `PriorTeardown` are unchanged (FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (the worker's typed stop error, DISTILL gap B-6)) | S-ND295-07B, 54 (the `server_lifecycle` body included), and 56's `a_failed_reclaim_step_keeps_the_lease_for_the_next_attempt` assert the variant and reach the cause through `&*source`; S-ND295-54 owns the oracles for caller rules 1, 3, and 4; the sim lifecycle returns `HandleTeardown` (§ *Intercept listener and stop-error test support*) |
| B-7 | `MtlsIntercept::bind_transparent` returns a bound `std::net::TcpListener`, so a worker composed over the sim binds real sockets and runs accept threads, and under R16 every test that dispatches a start or runs a convergence tick would be integration-lane | **PINNED** — `bind_transparent` returns `Arc<dyn InterceptListener>` (`local_addr`; cancel-safe `async fn accept() -> Result<InterceptAccepted, InterceptAcceptError>`); `SimMtlsIntercept` opens no socket, descriptor, thread, or timer, and its `accept` stays pending until a test scripts an outcome; production keeps today's `IP_TRANSPARENT` socket; an accept task stops by cancellation; the worker is no reason to gate a test; the step that lands it, no later than 05-01, deletes the per-allocation listener branch (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25"; FD § "[REF] Required downstream changes (not edited by DESIGN)" (the consequences of pins B-6 and B-7)) | Lanes of S-ND295-05D, 07, 19, 29A, 30A, 32, 53, 56, and 57 re-decided; S-ND295-29A's listener loss is a scripted `ListenerLost`; § *Intercept listener and stop-error test support*; NEW S-ND295-70 |
| N-1 (note) | The read-port's module path | **PINNED** — `overdrive_core::traits::guest_attachment_view`, re-exported from `overdrive_core::traits`; `HydrationContext` stays in `overdrive_core::reconcilers::hydration` (FD § "[REF] Component — node-wide guest-attachment admission (D-295-R6, R7, R8) — ACCEPTED 2026-09-24 (R7 user ruling of the same date)" (the core read-port's module path)) | Phase-B scaffolds use the pinned path |
| N-2 (note) | R16 makes every `run_server*` boot run the real `HostMtlsEnforcement` kTLS probe, so the S-ND295-13 seeded invariant becomes Lima-root in-process | FD § "[REF] Serve-boundary ports (D-295-R16) — ACCEPTED 2026-09-24" (`compose_mtls` is deleted); FD § "D-295-DISTILL-13 — production-composed S-ND295-13 boot-order boundary" (the selected shape) | S-ND295-13A keeps its boundary and seed and gains `integration-tests` gating |
| N-3 (note) | D-295-R22 falsifies the S-VM-09 assertion that the CH thread-group leader reports `SECCOMP_MODE_DISABLED` (`vm_walking_skeleton.rs:1102-1105`) | FD § "[REF] Evidence-lane matrix (charter §4 and §5)" (the E21 row, case (e)) | The body is re-targeted as S-ND295-45, and its marker moves 05-02 → 05-03 (its precondition is a Running guest, which the 05-03 descriptor handoff provides) |
| B-8 | `HostMtlsIntercept` refuses an element install/removal without a recorded program (constructed-source `NftRuleInstallFailed`, or `install_inbound`'s fallback to the retired per-rule installer) while `SimMtlsIntercept` installs unconditionally — the same call sequence returns `Ok` on one adapter and `Err` on the other (phase-C blocker 4, `red-classification.md`) | **PINNED** — one added `InterceptError::SharedProgramNotConverged`; the ordered install partition (not-converged → `SharedListenerPortMismatch` → `NftElementUpdateFailed`) and removal partition; the sim holds the record and models the owned program with the host's transitions; a failed `converge_shared` keeps the record; `install_inbound`'s fallback deleted (FD § "[REF] Driven port — intercept element precondition (DISTILL gap B-8) — pinned 2026-09-26") | NEW S-ND295-71; the equivalence harness converges before installing (phase-C C-14 closed); the sim self-tests, `SimMtlsIntercept`'s program model, and every test-local `MtlsIntercept` double return `SharedProgramNotConverged`; the removal bodies split the recorded-versus-observed refusal into its own body (S-ND295-54); the `ProofIntercept` `program_lost` overlay re-establishes against the inner sim's observation; B-8 lands in 05-01 (removal clause 07-01, failed-converge clause with R15) |
| Review iteration 1 (2026-09-29 pins; 2026-09-30 user decisions) | DR-01, DR-06, DR-07, DR-08 (a)/(b), DR-10, H2, and the pins DISTILL already followed (DR-16, 17, 19, 20, L6) | **PINNED** — DR-06 (user-decided): `MtlsInterceptStopError`'s `Display` keeps every per-connection cause, and `ElementRemoval` names its removal cause (user-approved 2026-09-30); DR-07: `Client::local_route_present` at 07-01, `intercept_mark_guard` reads `false` (not observed) before 08-01; DR-08 (a): `PolicyRouteAbsent`, `InterceptMarkGuardAbsent`, `MembersRemain` as the observation checks' causes; DR-08 (b)-A (user-approved 2026-09-30): the host classifies every per-TAP failure, a netlink session it cannot obtain included, as that TAP's `unconfirmed` entry and returns no `Err`; a repeat quiescence while latched sets down what a part-way restore raised, and the supervisor re-quiesces after a part-way restore (user-approved 2026-09-30); every adapter's quiescence call is bounded (a hang is a bound miss); DR-10: a `cfg` predicate requiring `test` makes an item test-only; H2: 07-01 owns the PORT-295-C projection; DR-01: 06-02 (and 06-01 as its evidence stands) depend on 05-03 (FD § "[REF] Decisions Table" (the DISTILL review iteration-1 pins and the user decisions of 2026-09-30); FD § "[REF] Required downstream changes (not edited by DESIGN)") | S-ND295-13D, 21, 39, 41, 46, 49, 51, 54, 61, 29A, 30A, 32 re-authored or extended; the re-roadmap dependencies and rows of the feature delta's DISTILL section |
| H14 | S-ND295-54's R10 clauses (1) a delete batch the kernel itself rejects leaves every member unchanged, and (2) a post-commit read-back failure performs one inverse transition and keeps both causes, had no deterministic real-kernel stimulus, and the adapter's prior private seam (`SharedInterceptProgramIo`) carries the program's observe and replace, not the element batch or its read-back | **RESOLVED — user decision 3 of 2026-09-30.** Both clauses rest on the kernel's batch abort (a rejected `nft` batch commits nothing) and are asserted source-local, default lane, over a NEW module-private `SharedIpElementIo` seam in `overdrive-netlink::nft` — a behaviour-preserving extraction of `mutate_and_readback`, no new public surface, its scripted `ScriptedElementIo` double DISTILL's test support (FD § "[REF] Decisions Table" (user decisions of 2026-09-30, decision 3); FD § "[REF] Driven port — intercept element release, member convergence, boot clear (D-295-R10, R12, R15, R18, R19) — ACCEPTED 2026-09-24 (R18, R19 conditional on native RED)" (Evidence for the two element-batch failure rules)) | S-ND295-54 gains four seam cells (`a_rejected_batch_returns_its_error_after_one_send_with_no_observation_or_inverse`, `a_failed_restoration_retains_both_the_primary_and_the_restoration_cause`, `a_failed_or_mismatched_read_back_restores_once_and_returns_the_primary`, `a_matching_read_back_returns_the_new_state_with_no_inverse`), GREEN regression-locks against the extraction, `#[ignore]`d until 07-01; the worker's handling of both errors stays additionally proven by S-ND295-07B |
| N-4 (note) | The fresh-host RCA (root cause A) — `converge_shared` creates `ovd-gbr0` with no address and sets it after, racing the host's link manager, so ~half of fresh-host boots refuse at `BridgeObserve` (metal N-04 by inference) | **PINNED** — `Client::ensure_bridge(name, mac)` creates the bridge with its address; the refusal names its cause via `GuestNetworkFact::Bridge`; a TAP's host-side MAC is judged by the D-295-R21 invariant (`TapHostMac { ifindex, address: TapHostAddress }`, user-approved 2026-09-28), never against a recorded value; no host link-configuration requirement and no startup-probe scratch-TAP condition (FD § "[REF] Managed-link identity independent of host link configuration (fresh-host RCA) — pinned 2026-09-26; user rulings of 2026-09-28"; user rulings of 2026-09-28) | NEW S-ND295-72 (its scratch-TAP probe bodies deleted 2026-09-28); S-ND295-00 split (bridge-identity leg 05-00, DNS leg 05-01); C-12b reclassified under RCA root cause A with 05-00 as owner (`red-classification.md`) |

**Consequence for DELIVER 05-01.** Once `AppState` requires a worker, a
fixture whose dispatch reaches intercept install starts the worker's shared
owner. B-7 lands no later than 05-01, so over `SimMtlsIntercept` that owner
binds nothing, and every existing `AppState` fixture keeps its lane with no
assertion change (FD § "[REF] Driven port — intercept listener (DISTILL gap B-7) — pinned 2026-09-25" (the effect on D-295-R16, E16, and lane classification); FD § "[REF] Required downstream changes (not edited by DESIGN)" (the consequences of pins B-6 and B-7: the lane move withdrawn)).
