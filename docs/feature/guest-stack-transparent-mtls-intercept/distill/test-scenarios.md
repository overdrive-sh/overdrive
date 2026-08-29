# DISTILL test scenarios — guest-stack-transparent-mtls-intercept (GH #222)

**Specification only; not parsed or executed.** This repository bans `.feature`
files. The Gherkin below is the behavioural contract; executable acceptance
tests are Rust. Scope is the egress-first VM Job slice. Inbound guest services
remain #257.

Authoritative upstream inputs are ADR-0088, ADR-0089, the DESIGN section of
`../feature-delta.md`, and `../design/wave-decisions.md`. The reconciled
lifecycle, rule-hit, and execution-substrate amendment is commit
`85550e4a267cbd53ac266fa54f4d8cda164910af`; DESIGN review iteration 6 is
**APPROVED**. The component reuse gate remains exactly **8 REUSE-AS-IS / 10
EXTEND / 1 CREATE-NEW**. DISTILL does not reclassify those nineteen rows.

## Reconciliation record

- DISCUSS `user-stories.md`, `story-map.md`, and `wave-decisions.md` are absent.
  The feature followed SPIKE → DESIGN, so each absence is a warning rather than
  an empty input.
- SPIKE remains WORKS for the routed guest topology. It does not prove the
  later lifecycle, diagnostic, or kernel-witness contracts.
- DESIGN Q7/Q9, including D7's mutation-aware exact-rule-hit oracle, supersedes
  the earlier generic “rule increment” wording. DESIGN commit `85550e4a` also
  makes platform reclamation with standing intent the sole same-allocation VM
  Job re-drive and pins runtime evidence to native, non-virtualized x86_64 KVM.
- DEVOPS `wave-decisions.md` is absent. Native-metal qualification, the
  host-wide lease, and EDD evidence capture are therefore explicit roadmap /
  DEVOPS obligations before DELIVER may claim the metal gate.
- The product journey `run-a-vm-workload.yaml` still says an unreported guest
  death consumes retry/backoff. That conflicts with the later ratified Job
  run-once rule: pre-READY rejection and ordinary Job exit finalize without a
  restart; only platform reclamation while intent still stands re-drives the
  same allocation. The later Job rule is authoritative for this feature.
  Product/Journey ownership must remove the stale retry sentence; this
  DISTILL pass records the conflict instead of claiming zero contradictions.

**Reconciliation result:** one documented stale product-journey conflict,
resolved for implementation by the later Job run-once and platform-reclamation
decisions; no unresolved DESIGN ambiguity.

## Pinned lifecycle and observation model

The one legal success sequence is:

```text
deploy accepted
  -> C3 creates the allocation netns, host-veth and tap
  -> observer learns the real allocation/interface identities
  -> capture is armed on the exact tap and host-veth
  -> real VMM spawn
  -> guest initialization while the NIC is down
  -> READY (all guest network setup complete; guest blocked)
  -> production intercept install succeeds
  -> D7 stable `before` cut proves the exact rule is live
  -> existing asynchronous EXEC release
  -> operator command's first connection
  -> D7 exact packet/byte equality + original destination at leg-F
  -> TLS on the inter-agent wire, with no peer-path cleartext
  -> ordinary operator EXIT
```

The observation harness may be prepared before deploy, but no allocation id,
slot, netns inode, interface name/index, MAC, guest address, capture-ready edge,
or C3 result exists in a Given fixture. The harness learns those facts only
during the real deployment after C3, then arms capture before real VMM spawn.

The control plane commits the allocation's durable `Running` row before it
calls the production intercept installer. A fresh or restart install failure
may therefore be observed briefly as Running before that row is superseded by
terminal Failed. That row is not permission to run the operator command: the
VM's deferred EXEC release remains after successful intercept installation.
Failure scenarios assert terminal Failed, no EXEC/operator marker, no guest
frame or cleartext, and total cleanup; they do not assert that no transient
durable Running row existed.

Before READY, minimal-root initialization, token parsing, NIC-down admission,
IPv6 suppression and read-back, `arp_notify=0` and read-back, static address,
netmask, link-up, route, and resolver configuration must all succeed. Any
failure powers off before READY, emits no guest `EXIT`, reaches neither Running
nor EXEC, and is classified from the real VMM termination. After READY, every
`EXIT`, including status 78, is the operator command's result and never a boot
failure sentinel. Beacon Published Language, public describe fields, and
persistence schemas remain unchanged.

Platform reclamation is the sole same-allocation restart route for a VM Job in
this slice. An unclean control-plane restart with durable intent and a
non-terminal VM row but no fresh supervisor claim drives the existing
boot-epoch reclamation path; the platform-reclaimed row is then re-driven with
the same `AllocationId`. A Job's natural result/crash is run-once and finalizes.
`overdrive workload restart <id>` is generation replacement and mints a fresh
allocation, so it belongs to the fresh-start gate and is not S-GTI-06.

## D7 exact unchanged-rule-hit oracle

S-GTI-02 and its Rust decoder/oracle properties own the complete ratified D7
contract. S-GTI-01 states only the stakeholder-visible named-peer reply; E07
black-boxes that same public outcome and does not duplicate this oracle. “A
rule counter increased,” tag+handle equality, a partial dump, or leg-F arrival
alone is not enough for the Rust D7 witness.

1. The production allocation-scoped outbound rule contains one anonymous,
   non-terminal `counter` after the unchanged host-veth and TCP matches and
   before the byte-identical TPROXY/mark/accept tail. Shared, inbound,
   output-divert, and sibling rules remain counter-free unless independently
   selected by their own allocation identity.
2. The selected rule is unique only after a strict complete multipart
   `GETRULE` succeeds. Its full userdata, handle, and normalized ordered
   expression program must equal the production encoder. Normalization retains
   every expression kind, register, operand, address, port, mark, verdict, and
   order; only the live counter values become a typed placeholder. Missing,
   duplicate, extra, reordered, malformed, or unknown expressions fail.
3. `GETRULE` uses a dedicated socket and one absolute deadline. Every message
   must have the kernel sender, request sequence, expected rule reply type and
   family, valid message/attribute/nesting length and alignment, and no trailing
   bytes. Success requires exactly one zero-status `NLMSG_DONE`. Error,
   `NLM_F_DUMP_INTR`, overrun, timeout, EOF, extra/missing DONE, malformed data,
   or a partial dump fails before uniqueness is evaluated.
4. Strict `GETGEN` accepts exactly one complete kernel `NEWGEN` reply with the
   request sequence, expected family, and full nonzero generation id. After
   production install and before the first generation read, a read-only socket
   joins `NFNLGRP_NFTABLES` with loss reporting. Every snapshot is bracketed
   `GETGEN(G) -> complete GETRULE -> GETGEN(G)`. All brackets and the final
   drain must retain the initial nonzero `G`; any notification, generation
   change/decrease/wrap, replacement, delete/reinsert, unrelated transaction,
   `ENOBUFS`, overrun, or notification loss fails.
5. Two equal generation-bracketed snapshots separated by a capture-confirmed
   quiet interval define `before`; two equal guarded snapshots after the
   round-trip define `after`. The observer never installs, replaces, resets, or
   deletes a production rule.
6. The exact host-veth ingress witness is `AF_PACKET/SOCK_DGRAM`, armed before
   VMM spawn, and retains direction, ifindex, and protocol. It uses
   `recvmsg(MSG_TRUNC)` with a 65,535-byte L3 buffer, and closing
   `PACKET_STATISTICS` must report zero drops. A rule-eligible record is exactly
   a kernel-valid unfragmented IPv4 packet on that ifindex whose protocol is
   TCP. Fragmentation, malformed/truncated input, loss, or offload ambiguity
   fails. Each record contributes one packet and validated IPv4 `tot_len`, the
   `skb->len` domain at the priority -150 prerouting counter; L2 length, snap
   length, TCP payload length, or guessed bytes are forbidden substitutes.
7. Let `C` be the complete eligible-packet count and `L` the checked sum of
   those `tot_len` values. Require `C > 0`, `L > 0`,
   `after.packets.checked_sub(before.packets) == Some(C)`,
   `after.bytes.checked_sub(before.bytes) == Some(L)`,
   `before.packets.checked_add(C) == Some(after.packets)`, and
   `before.bytes.checked_add(L) == Some(after.bytes)`. The first eligible
   packet is the expected SYN and every eligible packet has the same
   directional tuple. Regression, reset, wrap, competing traffic, or
   incomplete capture cannot false-pass.
8. The same original destination must arrive at leg-F, the inter-agent path
   must carry TLS, and no cleartext request/response copy may reach the peer
   path. Same-tag adoption keeps accumulated counts but takes a fresh baseline;
   normal stop deletes the exact handle; boot recovery sweeps before reinstall;
   no comparison crosses a restart. For teardown, let `B` be the complete
   guarded chain-order sequence of the target plus all quiescent sibling
   allocation-rule snapshots before deletion, where each item is `(userdata,
   handle, normalized full program, packets, bytes)`. Let `A` be the complete
   sequence over that same allocation-rule universe after deletion. The exact
   oracle is `A == filter(B, handle != target_handle)`, and the target handle
   is absent.
   Thus every surviving sibling keeps its identity, full normalized snapshot,
   counter state, and relative order; no unchanged absolute ordinal is claimed.

## Native-metal execution and global lease

Runtime metal examples execute only on a **native, non-virtualized x86_64 KVM
host** through `cargo xtask metal run --`. Nested KVM is forbidden. Lima and
other virtualized hosts may compile the gated Rust tests with `--no-run`, but a
compile is not runtime evidence.

The preflight fails closed unless all of these agree: `uname -m` is `x86_64`;
`systemd-detect-virt` reports the literal result `none`; `/proc/cpuinfo` has no
`hypervisor` flag and exposes the required hardware virtualization extensions;
`/dev/kvm` is an openable character device; KVM API version is 12 and a probe
can create and immediately close a VM fd; cgroup v2, Cloud Hypervisor, the
kernel, and the selected rootfs exist. Missing tools, contradictory signals,
permission errors, failed CPU/API/VM-create probes, or an unknown
virtualization result are a block, never a skip or pass.

The canonical metal writer boundary must use one host-global advisory lease,
`/run/lock/overdrive-metal-shared.lock`. `MetalAction::Run`,
`MetalAction::Sync`, and every supported direct bootstrap writer acquire that
same lease before their first remote-tree mutation, including before
`rsync --delete`. A Run retains the same remote descriptor across sync, native
preflight, build and execution, evidence collection, bounded cleanup, and final
residue probes; Sync and direct bootstrap retain it through their final
write/verification boundary. Raw or legacy writers that do not participate in
this lease are prohibited while using the canonical `~/overdrive` tree. The
bootstrap must either acquire the lease itself or verify and retain an
inherited lease descriptor; a feature-local lock is not sufficient.

Acquisition uses `flock` with a finite 120-second timeout and records holder
PID, UTC start, action, scenario/expectation id when applicable, workspace, and
commit SHA. Ownership is acknowledged before mutation and released only after
the associated final probe, including on signal or error; a timeout reports the
current owner metadata and aborts without mutation. The next roadmap and
DEVOPS handoff must assign and land this canonical Run/Sync/bootstrap writer
boundary plus the native preflight before the E07 runtime claim. Until
then the pending stub must block rather than treat current unleased commands
as evidence. Once implemented, the same ownership epoch serializes both the
shared remote tree and host-global kernel fixtures across worktrees.

## Acceptance examples and budget

There are **15** acceptance examples. Splitting restart success/failure,
pre-/post-READY exit semantics, and teardown with/without a rule is intentional;
they are mutually exclusive outcomes, not one scenario with contradictory
Then clauses. Exactly one scenario carries the walking-skeleton marker:
S-GTI-01.

| ID | Tags | Contract shape | Outcome |
|---|---|---|---|
| S-GTI-01 | `@walking_skeleton @driving_port @real-io @kvm` | bounded-change | a VM Job dials a mesh peer by name and receives its reply |
| S-GTI-02 | `@driving_port @real-io @kvm @property` | unbounded-preservation | the first guest mesh dial is born captured and exactly accounted |
| S-GTI-03 | `@real-io @kvm @wire-assertion` | unbounded-preservation | inter-agent traffic is TLS and peer-path cleartext is absent |
| S-GTI-04 | `@real-io @kvm` | bounded-change | non-mesh egress remains plaintext and functional |
| S-GTI-05 | `@real-io @kvm @error` | bounded-change | fresh intercept-install failure refuses execution |
| S-GTI-06a | `@real-io @kvm @restart` | bounded-change | platform reclamation re-enrols the same allocation before EXEC |
| S-GTI-06b | `@real-io @kvm @restart @error` | bounded-change | failed same-allocation reinstall remains terminal and closed |
| S-GTI-07 | `@real-io @kvm` | bounded-change | describe shows the guest address, not the transit hop |
| S-GTI-08a | `@real-io @kvm @error @cleanup` | bounded-change | a real resolver-apply failure is terminal, truthful, and residue-free |
| S-GTI-08b | `@real-io @kvm` | bounded-change | post-READY operator exit 78 is an ordinary command result |
| S-GTI-09 | `@property @in-memory` | pure-function | slots derive collision-free tap names |
| S-GTI-10 | `@property @in-memory` | pure-function | slots derive bounded, disjoint guest networks |
| S-GTI-11 | `@property @in-memory` | pure-function | slots derive unique locally-administered unicast MACs |
| S-GTI-12a | `@real-io @kvm @teardown` | bounded-change | Job stop removes exactly the installed allocation rule |
| S-GTI-12b | `@real-io @kvm @teardown` | unbounded-preservation | Job stop without an installed rule is idempotent and preserves siblings |

### Stakeholder scenarios

```gherkin
@walking_skeleton @driving_port @real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-01 — A microVM workload dials a mesh peer by name and receives the reply
  Given an mTLS-composed "overdrive serve" and a named mesh peer are available
  When the operator deploys a "[vm]"+"[job]" whose command dials the peer by name
  Then the first named-peer connection succeeds and receives the peer's byte-distinct reply

@driving_port @real-io @kvm @property @contract-shape:unbounded-preservation
Scenario: S-GTI-02 — The guest's first mesh dial is born captured and exactly accounted
  Given an mTLS-composed "overdrive serve" is available
    And the observation harness is prepared but has no allocation identity
  When the operator deploys a "[vm]"+"[job]" whose first action is a mesh dial
  Then after real C3 provisioning the harness binds the exact allocation, netns, tap, host-veth, MAC, and guest address
    And capture is armed on both exact interfaces before real VMM spawn
    And every guest-originated L2 frame before the D7 before-cut fails the example
    And capture loss, ambiguity, malformed data, or uncertain ordering fails the example
    And EXEC is released only after the generation-stable exact rule baseline exists
    And the first eligible SYN has the expected guest-address-to-mesh-VIP tuple
    And checked packet and IPv4-total-length deltas equal the complete eligible capture
    And the original destination reaches leg-F over a TLS-protected path with no peer-path cleartext

@real-io @kvm @wire-assertion @contract-shape:unbounded-preservation
Scenario: S-GTI-03 — The guest's mesh traffic travels the peer wire as mTLS, never in the clear
  Given a deployed microVM is dialing a mesh peer through its live egress guard
  When byte-distinct request and response payloads cross the connection
  Then the inter-agent wire carries TLS application-data records in both directions
    And neither plaintext marker appears on the external peer path
    And the kernel reports TLS installed on the protected legs

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-04 — The same guest reaches a non-mesh destination in the clear
  Given a deployed microVM whose mesh egress is guarded
  When its command dials an address outside mesh membership
  Then the non-mesh peer receives the plaintext request
    And the guest receives the unchanged reply

@real-io @kvm @error @contract-shape:bounded-change
Scenario: S-GTI-05 — A fresh mesh-guard installation failure refuses the workload
  Given the qualified native host will reject installation of the fresh VM Job's egress guard
  When the operator deploys a "[vm]"+"[job]"
  Then bounded describe observation reaches terminal Failed with an actionable guard-install detail
    And EXEC is never released and the operator command never runs
    And no guest-originated frame or cleartext egress escapes
    And bounded cleanup leaves no allocation-scoped residue

@real-io @kvm @restart @contract-shape:bounded-change
Scenario: S-GTI-06a — Platform reclamation re-enrols the same allocation before it runs again
  Given a VM Job is Running and its durable intent and allocation row exist
  When the operator ends "overdrive serve" uncleanly and restarts it on the same data directory
  Then "overdrive workload describe <id>" reports the platform-reclamation ending for the unsupervised VM
    And its next attempt reuses the same allocation identity
    And the replacement guest remains blocked until its new guard satisfies D7
    And its first post-reclamation mesh flow is protected with no peer-path cleartext

@real-io @kvm @restart @error @contract-shape:bounded-change
Scenario: S-GTI-06b — Failed re-enrolment after platform reclamation stays closed
  Given a VM Job is Running and its durable intent and allocation row exist
    And the native kernel environment will reject the guard reinstall after reclamation
  When the operator ends "overdrive serve" uncleanly and restarts it on the same data directory
  Then "overdrive workload describe <id>" shows the next attempt reused the same allocation identity
    And describe reaches terminal Failed with the reinstall detail
    And EXEC is never released for the replacement guest
    And no cleartext egress escapes

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-07 — The operator sees the guest address, not its transit hop
  Given an mTLS-composed "overdrive serve" is available
  When the operator deploys a "[vm]"+"[job]"
  Then bounded "overdrive workload describe <id>" observation reports the canonical workload address as the guest /30 host address
    And it is not the transit /30 address

@real-io @kvm @error @cleanup @contract-shape:bounded-change
Scenario: S-GTI-08a — A real resolver-apply failure is terminal, truthful, and residue-free
  Given a second independent allocation is Running with a quiescent egress rule
    And the VM spec selects a custom rootfs whose resolver target makes the production resolver write fail
  When the operator deploys the failing "[vm]"+"[job]"
  Then bounded "overdrive workload describe <id>" observation shows one terminal Failed attempt with the resolver-stage detail
    And the exact observed VMM exit code is preserved when present
    And the durable restart count and budget are unchanged
    And the allocation never reports READY or Running and the operator command never runs
    And no operator marker, guest EXIT frame, or guest-originated network frame is observed
    And within the cleanup deadline no VMM, cgroup, rootfs clone, clone index, run directory, netns, tap, veth, route, nft rule, capture process, socket, or file descriptor remains for the failed allocation
    And the independent allocation and its exact rule identity and counter state are unchanged

@real-io @kvm @contract-shape:bounded-change
Scenario: S-GTI-08b — Operator exit 78 after READY is an ordinary result
  Given an mTLS-composed "overdrive serve" and the observation harness are available
  When the operator deploys a "[vm]"+"[job]" whose command exits with status 78
  Then the observed guest lifecycle orders READY before EXEC and EXIT
    And "overdrive workload describe <id>" reports the ordinary Job result with exit code 78
    And it does not report a setup rejection or an unreported guest exit
    And the result consumes no restart attempt

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-09 — Each microVM slot names its own tap device collision-free
  Given any valid slot, including zero, one, maximum minus one, and maximum
  When its guest tap plan is derived
  Then the tap name is "ovd-tp-" plus the four-hex slot and fits IFNAMSIZ
    And a distinct slot has a distinct name

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-10 — Each microVM slot owns a bounded guest network disjoint from transit
  Given any valid slot, including zero, one, maximum minus one, and maximum
  When guest and transit plans are derived
  Then the guest /30 begins at the mesh base plus 0x8000 plus slot times four
    And gateway and guest are its first and second usable addresses
    And guest and transit /30s are disjoint and remain within the mesh block

@property @in-memory @contract-shape:pure-function
Scenario: S-GTI-11 — Each microVM slot carries a unique local unicast NIC identity
  Given any valid slot, including zero, one, maximum minus one, and maximum
  When its guest MAC is derived
  Then the multicast bit is clear and the locally-administered bit is set
    And a distinct slot has a distinct MAC

@real-io @kvm @teardown @contract-shape:bounded-change
Scenario: S-GTI-12a — Job stop removes exactly the installed guest egress guard
  Given two independent VM Jobs are Running with generation-stable exact rule snapshots
  When the operator runs "overdrive job stop <target-id>"
  Then the target allocation's exact egress rule is absent after a complete guarded dump
    And the complete after-stop ordered allocation-rule sequence equals the complete before-stop sequence after filtering the exact target handle
    And bounded cleanup leaves no target allocation residue

@real-io @kvm @teardown @contract-shape:unbounded-preservation
Scenario: S-GTI-12b — Job stop without a guest egress guard is idempotent
  Given a VM Job has a terminal pre-READY attempt with no installed egress rule
    And an independent VM Job remains Running with a generation-stable rule snapshot
  When the operator runs "overdrive job stop <target-id>" twice
  Then the results are Stopped followed by AlreadyStopped
    And no target rule appears
    And the complete after-stop ordered allocation-rule sequence equals the unchanged complete before-stop sequence because no target handle existed
```

## Supporting source-local and component examples

These examples carry implementation-sensitive assertions that do not belong in
metal Gherkin. Pure properties use the exact rustdoc declaration
`/// CONTRACT_SHAPE: pure-function.` on every live property.

### Slot-boundary rejection property

`P-GTI-SLOT-BOUNDARY` is the source-local property
`net_slot_rejects_first_value_above_max_before_any_guest_plan_is_derived` in
`crates/overdrive-control-plane/src/veth_provisioner.rs`
(`pure-function`). Given the raw integer `NET_SLOT_MAX + 1`, when `NetSlot::new`
is invoked, it returns the typed out-of-range error before tap, guest-network,
or MAC derivation. S-GTI-09/10/11 separately drive every valid partition,
including `0`, `1`, `NET_SLOT_MAX - 1`, and `NET_SLOT_MAX`. The live property
must carry the exact line `/// CONTRACT_SHAPE: pure-function.`.

### Guest-init closed failure partitions

The sanctioned pre-READY error set is closed over the existing `InitError`
variants below. Each named case forces only its stage, returns exactly the
listed variant, and forbids every later init operation plus READY, EXEC, and the
operator command. `NoExecReceived`, `UnexpectedBeaconMessage`, `BeaconParse`,
`Spawn`, and `Reboot` are post-READY/operator-channel errors and are explicitly
outside this pre-READY set.

| Stable case / family | Forced stage | Exact allowed typed variant | Executable identity | Contract Shape |
|---|---|---|---|---|
| `C-GTI-ROOT-DIRECTORY` / minimal root | create required `proc` or `etc` directory | `InitError::GuestDirectory` | `minimal_root_directory_failure_is_typed_and_suppresses_all_later_init` | bounded-change |
| `C-GTI-ROOT-PROC` / minimal root | mount procfs | `InitError::ProcMount` | `minimal_root_proc_mount_failure_is_typed_and_suppresses_all_later_init` | bounded-change |
| `C-GTI-MODULE-OPEN` / platform init | open a present staged vsock module | `InitError::ModuleOpen` | `module_open_failure_is_pre_ready_and_closed` | bounded-change |
| `C-GTI-MODULE-LOAD` / platform init | load a present staged vsock module | `InitError::ModuleLoad` | `module_load_failure_is_pre_ready_and_closed` | bounded-change |
| `C-GTI-VSOCK-SOCKET` / platform init | create beacon socket | `InitError::Socket` | `beacon_socket_failure_is_pre_ready_and_closed` | bounded-change |
| `C-GTI-VSOCK-CONNECT` / platform init | exhaust bounded beacon-connect attempts | `InitError::Connect` | `beacon_connect_failure_is_pre_ready_and_closed` | bounded-change |
| `C-GTI-TOKEN-MISSING` / token | missing required token after a VM network was assigned | `InitError::GuestNetworkConfig` | `assigned_guest_network_rejects_missing_platform_token` | pure-function |
| `C-GTI-TOKEN-ADDR` / token | malformed guest address | `InitError::GuestNetworkConfig` | `guest_network_token_rejects_malformed_address` | pure-function |
| `C-GTI-TOKEN-PREFIX` / token | non-integer or out-of-range prefix | `InitError::GuestNetworkConfig` | `guest_network_token_rejects_invalid_prefix` | pure-function |
| `C-GTI-TOKEN-GATEWAY` / token | malformed gateway | `InitError::GuestNetworkConfig` | `guest_network_token_rejects_malformed_gateway` | pure-function |
| `C-GTI-TOKEN-DNS` / token | malformed DNS address | `InitError::GuestNetworkConfig` | `guest_network_token_rejects_malformed_dns` | pure-function |
| `C-GTI-CMDLINE-READ` / token | read `/proc/cmdline` | `InitError::GuestNetworkIo` | `cmdline_read_failure_is_pre_ready_and_closed` | bounded-change |
| `C-GTI-IF-ENUM` / suppression | enumerate the single non-loopback NIC | `InitError::GuestNetworkSyscall` | `interface_enumeration_failure_is_pre_ready_and_closed` | bounded-change |
| `C-GTI-NIC-DOWN` / suppression | NIC is unexpectedly up | `InitError::GuestNetworkConfig` | `network_admission_rejects_an_interface_that_is_already_up` | pure-function |
| `C-GTI-IPV6-WRITE` / suppression | write `disable_ipv6=1` | `InitError::GuestNetworkIo` | `ipv6_disable_write_failure_suppresses_later_network_setup` | bounded-change |
| `C-GTI-IPV6-READBACK` / suppression | read-back is not disabled | `InitError::GuestNetworkConfig` | `ipv6_readback_must_confirm_disabled` | pure-function |
| `C-GTI-ARP-WRITE` / suppression | write `arp_notify=0` | `InitError::GuestNetworkIo` | `arp_notify_write_failure_suppresses_later_network_setup` | bounded-change |
| `C-GTI-ARP-READBACK` / suppression | read-back is not zero | `InitError::GuestNetworkConfig` | `arp_notify_readback_must_confirm_zero` | pure-function |
| `C-GTI-IOCTL-SOCKET` / static apply | create IPv4 ioctl socket | `InitError::GuestNetworkSyscall` | `ioctl_socket_failure_suppresses_all_static_apply` | bounded-change |
| `C-GTI-ADDRESS` / static apply | set address | `InitError::GuestNetworkSyscall` | `address_failure_suppresses_netmask_link_route_and_resolver` | bounded-change |
| `C-GTI-NETMASK` / static apply | set netmask | `InitError::GuestNetworkSyscall` | `netmask_failure_suppresses_link_route_and_resolver` | bounded-change |
| `C-GTI-LINK` / static apply | bring link up | `InitError::GuestNetworkSyscall` | `link_failure_suppresses_route_and_resolver` | bounded-change |
| `C-GTI-ROUTE` / static apply | install default route | `InitError::GuestNetworkSyscall` | `route_failure_suppresses_resolver` | bounded-change |
| `C-GTI-RESOLVER` / resolver | write `/etc/resolv.conf` | `InitError::GuestNetworkIo` | `resolver_write_failure_suppresses_ready_and_exec` | bounded-change |
| `C-GTI-READY-SEND` / readiness | write READY to the already connected beacon | `InitError::Io` | `ready_send_failure_is_pre_ready_and_suppresses_exec` | bounded-change |

These cases live in `crates/overdrive-init/src/main.rs`. The token parser
properties range over arbitrary malformed bytes and field boundaries; the
suppression properties range over NIC flags and read-back values. Every live
pure property in the table carries exact
`/// CONTRACT_SHAPE: pure-function.`. The source-local total closure property
`P-GTI-PRE-READY-ERROR-CLOSURE` /
`every_sanctioned_pre_ready_failure_maps_to_the_closed_init_error_set`
(`pure-function`) generates every sanctioned stage and proves that the result
is exactly one of `ModuleOpen`, `ModuleLoad`, `Socket`, `Connect`, `Io`,
`GuestDirectory`, `ProcMount`, `GuestNetworkConfig`, `GuestNetworkIo`, or
`GuestNetworkSyscall`; no other `InitError` variant and no later effect is
accepted. It also carries the exact pure-function rustdoc line.

### Diagnostic and cleanup totality

Bounded examples cover a console tail over 8 KiB, over five fragments, an
unterminated final fragment, invalid UTF-8, and nonempty-console precedence over
VMM stderr. Absence, empty content, unreadable metadata, open/read failure, and
mid-read error are separate cases. Every case must still return the original
start rejection, select bounded stderr or the stable neither-source detail, and
run the same cleanup; diagnostic collection may never replace or mask the
rejection or cleanup error. Cleanup examples poll to a finite deadline for the
same complete residue set asserted by S-GTI-08a and preserve an independent
allocation and nft rule. The diagnostic selection matrix is
`C-GTI-DIAGNOSTIC-TOTALITY` (`bounded-change`); repeated failed-start cleanup is
`C-GTI-FAILED-START-CLEANUP-TWICE` (`bounded-change`) and must converge to the
same residue-free state on both applications.

### Reconciler/action-shim component example

`P-GTI-JOB-EXIT-CLASSIFIER` is the source-local property
`every_unreported_pre_ready_vmm_exit_maps_to_failed_without_restart` in
`crates/overdrive-reconcilers/src/workload_lifecycle.rs` (`pure-function`). It
ranges over every `Option<i32>` VMM exit code plus an arbitrary signal value and
proves the signal cannot change the exact mapping to
`TerminalCondition::Failed { exit_code }`. The live property must carry exact
`/// CONTRACT_SHAPE: pure-function.`.

`C-GTI-08-RECONCILE` (`bounded-change`) maps to the existing
workload-lifecycle reconciler and action shim. Given a Job allocation row
carrying an unreported pre-READY VMM exit, a seeded private View, and a nonzero
durable restart count, reconcile must emit exactly one finalization action with
the exact Failed exit code, emit no restart action, and return a View identical
to its input. Dispatch must persist the same terminal claim while retaining the
durable count. This component example owns the private action vector, View
equality, and no-restart checks; S-GTI-08a owns only port-visible state and
cleanup.

`C-GTI-08-EXIT78` (`pure-function`) proves the complement: a READY-observed
operator `EXIT 78` maps to the ordinary Job result and never to the pre-READY
rejection reason.

### Illegal-event state matrix

Each row is a stable source-local property, mapped to the named source and
classified independently in `red-classification.md`. Every property carries
exact `/// CONTRACT_SHAPE: pure-function.`.

| ID | State | Forbidden event and expected disposition | Executable identity / source | Contract Shape |
|---|---|---|---|---|
| `P-GTI-ILLEGAL-01` | harness prepared, no C3 identity | claiming capture-ready or spawning the VMM is rejected | `capture_ready_requires_the_real_c3_identity` / CLI guest-stack observer module | pure-function |
| `P-GTI-ILLEGAL-02` | C3 complete / capture armed | a guest-originated L2 frame before the D7 before-cut fails the witness | `pre_baseline_guest_frame_always_invalidates_born_captured` / CLI guest-stack observer module | pure-function |
| `P-GTI-ILLEGAL-03` | guest initializing | READY before every required setup/read-back is rejected | `ready_requires_every_guest_init_stage` / `overdrive-init/src/main.rs` | pure-function |
| `P-GTI-ILLEGAL-04` | READY, blocked | EXEC before a stable exact rule baseline is forbidden | `exec_release_requires_a_stable_exact_rule_baseline` / action-shim guest-stack component | pure-function |
| `P-GTI-ILLEGAL-05` | intercept live | notification, generation change, loss, reset ambiguity, or program mismatch invalidates the witness | `live_intercept_is_invalidated_by_every_guard_mutation_signal` / CLI guest-stack observer module | pure-function |
| `P-GTI-ILLEGAL-06` | operator command running | setup-failure classification is forbidden; status 78 is an operator result | `running_job_exit_can_never_be_classified_as_guest_setup_failure` / workload lifecycle | pure-function |
| `P-GTI-ILLEGAL-07` | terminal | later READY, EXEC, or duplicate finalization cannot reopen or duplicate the attempt | `terminal_vm_job_rejects_every_reopening_event` / workload lifecycle | pure-function |

### Mutating-operation replay inventory

This is the complete Q7/Q9 mutating-operation inventory. Application-level
mutations with a replay contract name their apply-twice example. The grouped
attempt-owned resource creation below remains an explicit C4a gap: teardown
replay and an ownership argument do not prove correct behavior when creation is
requested twice. Read-only observations are not mislabeled as mutations.

| Mutating operation | Apply-twice contract / justification | Executable identity | Contract Shape |
|---|---|---|---|
| C3 create/converge netns, host-veth, tap, addresses, route, and forwarding | applying the same allocation plan twice converges to the same identities and kernel state | `C-GTI-C3-CONVERGE-TWICE` / `c3_converge_twice_preserves_the_same_vm_network_plan` | bounded-change |
| shared fwmark rule, local route, nft table/chains, and exemptions | two ensures produce one structurally identical shared-infra set | `C-GTI-SHARED-INFRA-TWICE` / `shared_tproxy_infrastructure_converges_on_second_ensure` | bounded-change |
| outbound guard install/adopt | applying the same exact userdata key twice adopts one handle and preserves its accumulated counter | `C-GTI-GUARD-INSTALL-TWICE` / `same_egress_guard_install_twice_adopts_one_rule` | bounded-change |
| outbound guard deletion / Job stop | S-GTI-12b applies stop twice; no target is recreated and sibling sequence equality holds | `S-GTI-12b` | unbounded-preservation |
| boot-epoch platform-reclamation claim | repeated executor evaluation for one boot epoch emits at most one same-id re-drive | `C-GTI-RECLAMATION-ONCE` / `same_boot_epoch_claims_each_unsupervised_allocation_once` | pure-function |
| terminal allocation finalization/status persistence | replaying the same finalization cannot create a second transition, alter the exact exit code, or increment durable restart count | `C-GTI-FINALIZE-TWICE` / `same_job_finalization_is_terminal_and_count_preserving` | bounded-change |
| failed-start teardown | applying teardown twice converges to the same complete residue-free state | `C-GTI-FAILED-START-CLEANUP-TWICE` | bounded-change |
| rootfs clone, run directory, listeners, VMM, and capture processes | **UNMAPPED C4a gap:** these are effects owned by one application-level start attempt, but the package has no repeat-request AT proving typed rejection, no replacement/cross-ownership, and no leak; teardown replay is not creation replay | none — `AT_GAP_IN_DELIVERY_SCOPE` | unassigned — gap |
| guest directory/proc/network writes and ioctls | N/A: executed once per fresh guest boot with fresh kernel/rootfs state; there is no replay driving port, and duplicate READY/init progression is rejected by `P-GTI-ILLEGAL-03/-04` | state-machine proof | pure-function |
| operator EXEC release | N/A: this is a single state transition, not a replayable resource mutation; premature or duplicate release is an illegal event covered by `P-GTI-ILLEGAL-04` | state-machine proof | pure-function |
| tap/network/MAC/cmdline/config/classifier derivation | N/A: pure derivations do not mutate external state | S-GTI-09/10/11, `P-GTI-SLOT-BOUNDARY`, `P-GTI-JOB-EXIT-CLASSIFIER` | pure-function |
| D7 `GETRULE`/`GETGEN`, AF_PACKET capture, console selection, and describe | N/A: read-only observations; their kernel traffic counters are observed packet-path side effects, not an apply operation | D7 and diagnostic closure properties | pure-function |

### Netlink/capture error closure

`P-GTI-D7-ERROR-CLOSURE` (`pure-function`) is the source-local decoder/oracle
property family; every live property carries exact
`/// CONTRACT_SHAPE: pure-function.`. Its generated cases cover zero target,
one exact target, and
duplicate targets; counter absent/partial/duplicate/wrong-width; expression
missing/extra/reordered/unknown; wrong sender/sequence/type/family; malformed or
misaligned messages/attributes; missing/extra/error DONE; `NLM_F_DUMP_INTR`;
timeout/EOF/overrun/trailing/partial dump; zero/extra/malformed `GETGEN` reply;
generation change/decrease/wrap; notification and notification loss; counter
reset/regression/wrap/overflow; capture truncation/drop/fragment/offload
ambiguity; and packet/byte equality mismatch. Counter-free siblings remain
valid. These are decoder/oracle properties, not duplicated metal boots.

### Interruption and concurrent-actor supporting examples

`M-GTI-INTERRUPT-BOOT` (`bounded-change`) is a focused native-metal example.
After real C3 has
produced the allocation identity and capture is armed, it terminates the real
VMM before READY. The production start path must report the same pre-READY
rejection class, never release EXEC, and complete the full bounded residue
cleanup while an independent allocation remains unchanged. This is an actual
external interruption during the operation, distinct from a stage returning an
expected error.

`M-GTI-CONCURRENT-DEPLOY` (`unbounded-preservation`) starts two built-binary VM
Job deploy commands in parallel inside one host-lease holder. Both real C3
paths must derive distinct
allocation ids, netns/tap/veth identities, guest networks, MACs, and exact nft
rules; each capture may accept only its own tuple, and both Jobs must converge
without cross-observation. Cleanup is delta-scoped per allocation. This is the
parallel multi-actor case; the host-wide lease serializes competing test runs,
not the two allocations inside this example.

## Canonical AT-completeness audit

This is **specification coverage**, not execution status. The immutable-baseline
execution classification is in `red-classification.md` and currently records
`NOT_EXECUTED` for this remediation.

| Item | Verdict | Concrete evidence |
|---|---|---|
| C1a zero/min | PASS | S-GTI-09/10/11 explicitly include slot 0 and 1 |
| C1b partition boundary | PASS | S-GTI-09/10/11 drive max-1/max; `P-GTI-SLOT-BOUNDARY` invokes `NetSlot::new(NET_SLOT_MAX + 1)` as its action and proves typed rejection before derivation |
| C2a state machine documented | PASS | legal lifecycle sequence is explicit above |
| C2b illegal event per state | PASS | `P-GTI-ILLEGAL-01` through `-07` each name an executable property, source, shape, forbidden event, and result; each has an immutable-base row |
| C3 0/1/N cardinality | PASS | D7 target-selection properties cover zero/one/duplicate targets; S-GTI-12a/b and their Rust teardown properties cover empty, singleton, and multiple ordered allocation-rule sequences |
| C4a apply twice/idempotency | **FAIL** | application-level converge/install/delete/reclamation/finalization/teardown operations have repeat mappings, but the attempt-owned rootfs/run-dir/listener/VMM/capture creation group has no correct-non-idempotency AT; teardown replay does not substitute |
| C4b inverse without prerequisite | PASS | S-GTI-12b stops an allocation for which no guard was installed |
| C5a mode combinations | PASS (N/A) | this feature introduces no independent user mode-flag parameter; mesh and reclamation branches are scenarios, not flags |
| C5b flag orthogonality | PASS (N/A) | with no independent mode flags, orthogonality is not applicable |
| C6a malformed input | PASS | address, prefix, gateway, and DNS token cases are distinct |
| C6b each declared error | PASS | the stable guest-init table adds minimal-root directory/proc failures and individually forces every sanctioned pre-READY stage; install, diagnostics, dump, generation, and capture failures remain distinct |
| C6c closed error set | PASS | `P-GTI-PRE-READY-ERROR-CLOSURE` pins the exact ten allowed `InitError` variants and excludes every post-READY variant; D7 has its separate closed parser/oracle set |
| C7a degraded resource | PASS | console read variants, resolver failure, capture loss, netlink loss, and strict timeout paths fail conservatively |
| C7b interruption mid-operation | PASS | M-GTI-INTERRUPT-BOOT terminates the real VMM after capture-ready and before READY, then proves rejection and total cleanup |
| C7c concurrent actors | PASS | M-GTI-CONCURRENT-DEPLOY runs two deploys in parallel and proves distinct identities/captures/rules; S-GTI-12 additionally preserves a live sibling during stop |

**Specified: 14/15 → COMPLETE by the canonical ≥13 threshold.** C4a is the
one explicit gap. No infrastructure waiver is counted as coverage, and this
score does not assert that all examples are implemented or have run.

## Adapter, test, and EDD map

| Contract | Production seam / driving port | Executable location or obligation |
|---|---|---|
| real deploy + describe | built `overdrive deploy`; `overdrive workload describe` | Rust acceptance module; E07 named-peer reply |
| exact Job stop | built `overdrive job stop <id>` → `commands::deploy::stop` | S-GTI-12a/b Rust acceptance/integration tests |
| same-allocation re-drive | unclean serve restart → boot VM reclamation → standing-intent lifecycle re-drive | S-GTI-06a/b Rust acceptance/integration tests |
| fresh and restarted install | existing allocation start/restart dispatch into the production mTLS worker | S-GTI-01/05/06a/06b |
| guest token and static apply | production kernel cmdline → `overdrive-init` | source-local parser/stage properties; S-GTI-08a |
| slot boundary rejection | `NetSlot::new` before tap/network/MAC derivation | `P-GTI-SLOT-BOUNDARY` in `veth_provisioner.rs` |
| closed pre-READY init errors | `overdrive-init` bootstrap/module/vsock/token/suppression/static/resolver/READY stages | `C-GTI-*` failure table + `P-GTI-PRE-READY-ERROR-CLOSURE` in `overdrive-init/src/main.rs` |
| total exit-code classification | `WorkloadLifecycle::classify_natural_exit_terminal` | `P-GTI-JOB-EXIT-CLASSIFIER` in `workload_lifecycle.rs` |
| terminal/no-restart classification | workload-lifecycle reconciler + action shim | `C-GTI-08-RECONCILE` source/component example |
| post-READY status 78 | real READY/EXEC/EXIT path | S-GTI-08b + `C-GTI-08-EXIT78` Rust tests only |
| exact rule-hit witness | production rule encoder/installer + read-only strict netlink/capture observer | `P-GTI-D7-ERROR-CLOSURE` + S-GTI-02 Rust tests only |
| failed-start cleanup | production VMM/worker guard drops and host observation surfaces | Rust cleanup-totality examples + S-GTI-05/08a |
| interruption and concurrent allocations | real VMM termination; two parallel built-binary deploys | M-GTI-INTERRUPT-BOOT / M-GTI-CONCURRENT-DEPLOY supporting metal examples |
| illegal lifecycle events | observer/init/action-shim/workload-lifecycle pure transition boundaries | `P-GTI-ILLEGAL-01` through `-07` |
| mutation replay | C3/shared infra, guard install/delete, reclamation claim, terminal finalization, failed-start teardown | `C-GTI-C3-CONVERGE-TWICE`, `C-GTI-SHARED-INFRA-TWICE`, `C-GTI-GUARD-INSTALL-TWICE`, S-GTI-12b, `C-GTI-RECLAMATION-ONCE`, `C-GTI-FINALIZE-TWICE`, `C-GTI-FAILED-START-CLEANUP-TWICE` |
| tap/network/MAC derivation | existing veth provisioner and the one CREATE-NEW `VmTapPlan` | S-GTI-09/10/11 source-local properties |
| native host qualification | `cargo xtask metal run --` with non-virtualized preflight and global lease | roadmap/DEVOPS implementation obligation; E07 stub |

The sole EDD stub is `E07-vm-job-calls-exec-service`. Its exact
checked-in operator journey lives at
`examples/guest-stack-transparent-mtls-intercept/`: one Exec Service, one VM
Job, and a reply-dependent successful caller result. The runner may compile
the checked-in helper sources only through the bundle's shared `prepare.sh`,
which owns static linkage, the qualified private rootfs, exact guest install,
same-filesystem/traversable data path, production KEK credential, and bounded
marker-owned cleanup. The product runs in a fresh anonymous session keyring,
and no ambient key is purged. Its cleanup is armed before launch, owns a
token-bound private process group before any `keyctl`/serve descendant, tracks
the direct wrapper by PID/start time across the serve `exec` handoff, and uses
bounded TERM/KILL polling on signal, failure, or handshake timeout; it never
performs an unbounded wrapper wait or signals an unverified/reused PID. It may
not synthesize source, Cargo manifests, or specs inline. `callee.toml` uses the
supported explicit-empty startup policy so no production-unreachable inferred
host-namespace TCP probe can terminate the sole Service; convergence is the
public Service allocation state `Running` with replicas `1/1`.

E07 remains pending until its built-binary commands, bounded public
observations, public stop results, marker-owned cleanup, and native-host
preflight are executed and independently reviewed. It does not inspect or
repair private product cleanup state. D7 framing/counters/capture/TLS/kTLS;
boot failure, diagnostic, C4a,
restart/reclamation, stop/idempotency, sibling, nft/FIB, cleanup, and replay
contracts stay exclusively in Rust tests and have no EDD expectation.
