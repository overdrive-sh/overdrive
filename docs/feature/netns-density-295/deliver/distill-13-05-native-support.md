# STEP 13-05 — existing GuestKit native support handoff

## Boundary and authority

This is the bounded DISTILL source handoff for the five already-authored
STEP 13-05 native cells, the 36 existing dataplane-integration real-kernel
bodies from the approved inventory, the 23 additional genuine native bodies
exposed by the first actual normal run, the one missing kernel-expression
module exposed by the first actual 46-body replay, and the missing declared
offload tool exposed by the second replay, plus the capture account required
by the already-staged tcpdump binary exposed by the third replay, and the
control-plane namespace process/filesystem closure exposed by the first
98-body attempt, plus the intrinsic positive DNS fallback fixture and missing
qualified gateway route exposed by the next 98-body attempt, and the negative
DNS control/uppercase skip-gate defect found in iteration-9 review, plus the
created-versus-borrowed gateway ownership contradiction exposed after
iteration-10 approval. It extends only the existing
`cgroup-default` and `mtls-relay` GuestKit selections plus their existing
staging declaration. Two ordinary fixtures whose failures were not privilege
effects are corrected through existing driven-port/filesystem support. The
existing private three-interface topology now fails closed on its already
documented four-end TX-offload precondition. It does not add a scenario,
assertion, pending-marker change, selectable runner group, CI lane, guest
executor, model, production API, DES event or commit.

The original STEP 13-05 crafter owns the concurrent production changes, the
five marker removals, `execution-log.json` and the step commit. This handoff
does not claim or alter that work. The pre-existing dirty `AGENTS.md` is also
preserved.

The authoritative contracts are STEP 13-05 in `deliver/roadmap.json`, the
feature delta's D8-PREFIX-CFG, D8-PREFIX-VIP, GW-LOCAL and LEGB-DIAL sections,
ADR-0152, ADR-0172, ADR-0185 and ADR-0190. The working native substrate is
`spike/findings-v33-legb-mark.md` and its v33 source/evidence directory. The
support here follows those contracts without copying the spike implementation.

## Why the existing runner is the required boundary

The two active `shared_guest_network_startup` cells enter the production
`run_server` composition. An ordinary package invocation reaches the real
cgroup bootstrap first and fails with typed `CgroupBootstrap` / ENOENT before
either prefix refusal can be asserted. That is an environment precondition,
not a production result for D8-PREFIX-CFG or D8-PREFIX-VIP. The existing
`cgroup-default` control-plane guest already mounts a private cgroup-v2 tree,
enables `cpu`, `memory`, `io` and `pids`, establishes the required nofile
limit, and runs the same staged integration binary.

The authoritative ordinary-union reproduction is durable Metal run
`fa224a04f75a914094235928`, nextest
`8e39b322-126e-4187-937f-6553983cd5a2`, with raw output retained at
`/var/lib/overdrive-metal/runs/fa224a04f75a914094235928/output.log`. Its exact
command used the approved 13-04 union filter over netlink, worker,
control-plane and sim. It ran 1,730 bodies: 1,728 passed, two failed and 133
were skipped. The only failures were the two STEP 13-05 startup bodies:

```text
cargo xtask metal run -- bash -lc \
  'task_filter=$(cat .context/netns-density-295/13-04-package-union.filter-expr); \
   env QUINT_SEED=0x94db98ed cargo nextest run \
   -p overdrive-netlink -p overdrive-worker -p overdrive-control-plane \
   -p overdrive-sim --features integration-tests --no-fail-fast \
   -E "$task_filter" --no-capture'
```

```text
a_node_address_inside_the_guest_prefix_refuses_boot_naming_the_interface
the interface and its address are named:
CgroupBootstrap(SubtreeControlWriteFailed {
    source: Os { code: 2, kind: NotFound,
                 message: "No such file or directory" }
})
```

```text
a_vip_range_overlapping_the_guest_prefix_refuses_boot_before_any_effect
the VIP range is named:
CgroupBootstrap(SubtreeControlWriteFailed {
    source: Os { code: 2, kind: NotFound,
                 message: "No such file or directory" }
})
```

The earlier unfiltered diagnostic `d78de0848f3ec6c60e4ee15a` is not used: it
ran outside the approved filter and produced 58 expected physical-host cgroup
failures. The authoritative 1,728/1,730 run also omitted the roadmap's
dataplane package, so it proves the two fixture precondition failures but is
not claimed as the final STEP 13-05 package gate. The dataplane disposition is
recorded separately below.

The three active `vsock_legb_dial` cells touch nftables, policy routing,
network namespaces and live socket state. Running them in a workspace host
namespace would make the host's route and `route_localnet` state part of the
result and would mutate physical-host nft, route and netns state. The existing
worker GuestKit guest is the qualified boundary: all module loads, sysctls,
nftables, policy routes, cgroups and test namespaces occur inside a fresh
Cloud Hypervisor guest. The physical Metal host only stages files and launches
the confined VMM.

## Exact manifest extension

The source and Metal list audit resolve these five exact active bodies once
each, with their existing Contract Shape declarations and no remaining
STEP 13-05 pending marker:

| Existing staged binary | Exact existing body | Count |
| --- | --- | ---: |
| `overdrive-control-plane` integration | `shared_guest_network_startup::a_vip_range_overlapping_the_guest_prefix_refuses_boot_before_any_effect` | 1 |
| `overdrive-control-plane` integration | `shared_guest_network_startup::a_node_address_inside_the_guest_prefix_refuses_boot_naming_the_interface` | 1 |
| `overdrive-worker` integration | `vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c` | 1 |
| `overdrive-worker` integration | `vsock_legb_dial::a_dial_to_a_peer_outside_the_guest_prefix_is_neither_marked_nor_bound` | 1 |
| `overdrive-worker` integration | `vsock_legb_dial::a_marked_dial_to_an_unregistered_tuple_is_refused_and_reaches_no_host_socket` | 1 |

The existing three guest boots, staged binaries and CID slots remain:

| Guest | CID slot | Prior bodies | STEP 13-05 bodies | Resulting bodies |
| --- | ---: | ---: | ---: | ---: |
| control-plane acceptance | 14 | 3 | 0 | 3 |
| control-plane integration | 15 | 51 | 2 | 53 |
| worker integration | 6 | 19 | 3 | 22 |
| **Total** | — | **73** | **5** | **78** |

The runner retains separate fail-closed totals for the 55-body baseline, the
18-body STEP 13-04 extension, the five-body STEP 13-05 extension and their
78-body sum. Every row is invoked by exact libtest identity and requires an
exit-zero `1 passed; 0 failed` summary. No ignored or zero-selected body can
count as a pass.

## Dataplane package and native-boundary accounting

The STEP 13-05 roadmap's ordinary package set is
`overdrive-control-plane`, `overdrive-dataplane` and `overdrive-netlink`.
`overdrive-dataplane` cannot be omitted merely because the newly activated
native cells live in the worker integration binary: the implementation changes
`HostMtlsEnforcement` construction and the conditional outbound leg-B dial in
the dataplane crate.

The source inventory finds no STEP 13-05 marker removal or new native body in
the dataplane crate. Its source-local mTLS tests remain ordinary unit evidence,
and every integration constructor site compiles with the required
`guest_prefix` and `guest_dial`. The new outbound kernel behavior is exercised
by the three exact `vsock_legb_dial` cells above through the real
`HostMtlsEnforcement`; duplicating those cells in a dataplane binary would add
a scenario and is not authorized.

The approved list receipt is
`.context/netns-density-295/13-05-normal-three-package-list.json`, SHA-256
`4bd78f192e3d017588616b041fbe25d678198adb407b02714f44876666d192b9`,
from durable Metal run `19999a70f5afa15060a6d75c` (exit zero). It inventories
1,126 control-plane, 179 dataplane and 50 netlink test identities. The
dataplane integration binary has 61 active bodies partitioned exactly as:

- seven existing `integration::mtls_inbound_guest_relay::` bodies;
- 18 ordinary/build/parser bodies that remain in the normal package run; and
- 36 existing kernel/BPF/netns/XDP/kTLS bodies that must leave the normal
  selector and execute inside a disposable guest.

The 36 identities in `DATAPLANE_NATIVE` are byte-for-byte equal to that final
partition, with no duplicate and no ignored/future body. Their family map is:

| Existing family | Bodies |
| --- | ---: |
| `atomic_swap` | 7 |
| `deregister_retry_safety` | 1 |
| `local_backend_proto_connect` | 1 |
| `maglev_real` | 1 |
| `mtls_agent_handshake` | 1 |
| `mtls_guardrails` | 1 |
| `mtls_inbound_enforce` | 1 |
| `multi_listener_tcp_udp_e2e` | 4 |
| `redirect_neigh_attach` kernel body | 1 |
| `reverse_nat_e2e` | 5 |
| `reverse_nat_udp_e2e` | 3 |
| `sanity_mixed_batch` | 1 |
| `service_map_forward` | 1 |
| `service_map_vip_port` | 1 |
| `unconnected_udp_reply_hardening` kernel bodies | 2 |
| `unconnected_udp_roundtrip` | 4 |
| `veth_attach` kernel body | 1 |
| **Total** | **36** |

The existing selectable `mtls-relay` group retains its original prefix
selection and exact seven-body count first. In that same disposable guest it
then invokes the 36 exact identities one by one. Runtime accounting fails
closed separately at seven baseline, 36 routed-native and 43 combined bodies;
every required invocation rejects a libtest-zero result carrying a runtime
`[skip]`, `skip:` or `(skip)` diagnostic. This changes no body or oracle and
cannot silently turn a missing capability into a pass.

The complete native-covered exclusion ledger is now exact:

| Package family outside the normal selector | Disposable route | Exact accounting |
| --- | --- | ---: |
| Control-plane cgroup-qualified baseline families | existing `cgroup-default` control-plane guests | 3 acceptance + 45 integration |
| Control-plane STEP 13-04 native families | existing `cgroup-default` control-plane integration guest | 6 |
| Control-plane STEP 13-05 startup refusals | existing `cgroup-default` control-plane integration guest | 2 |
| Control-plane normal-run privilege failures/bailouts | same control-plane integration guest, after the retained 78 boundary | 20 |
| Dataplane retained inbound guest-relay family | existing `mtls-relay` guest, baseline phase | 7 |
| Dataplane kernel/BPF/netns/XDP/kTLS families in the table above | same existing `mtls-relay` guest, routed-native phase | 36 |
| Dataplane real-BPF library bodies | same existing `mtls-relay` guest, library phase after retained 43 boundary | 3 |
| **Native-covered exclusions** | — | **122** |

The first three rows are the retained 56 control-plane package bodies within
the 78-body `cgroup-default` boundary; its other 22 bodies belong to worker.
The additional 20 extend only the control-plane integration guest, producing
76 control-plane exclusions and a complete 98-body guest group. The dataplane
rows retain 43 integration bodies, add three library bodies and produce 46
dataplane exclusions. The other 18 active dataplane integration bodies remain
in the normal package gate. No selected kernel-mutating family remains for the
physical Metal namespace. Compilation, focused tests and either native group
cannot substitute for the clean three-package `nextest` rerun after both
native receipts exist.

## First actual normal-run classification

The first unprivileged normal run is durable Metal
`3fb5a2f1e2bc7a8f2fccd58e`, nextest
`d8df41da-9320-4b67-b7cf-354cb4eef2d3`, seed `0x94db98ed`. Raw output is
`.context/netns-density-295/13-05-normal-three-package-run-seed94db98ed.log`,
SHA-256 `d11a288b8e76bb9cecc68f58a0b9a638b04dbeef3c812440f1ab2405996a3b23`.
It ran 1,247 identities: 1,240 passed, seven failed and 108 were skipped.

The seven failures and 18 successful-libtest privilege bailouts collapse to
23 genuine native identities plus two ordinary fixture defects; they are not
25 independent native scenarios.

### Ordinary fixture corrections — two

`action_shim_restart_passes_spec_from_action_to_driver_start_unchanged` says
explicitly that it observes only the action's unchanged spec at
`Driver::start`, with no workload-netns behavior. It called the generic test
`dispatch`, which composes the real `HostNetworkProvisioner`, and failed at
`NetnsDelFailed` before its spec oracle. The correction uses the already
existing `dispatch_with_network_provisioner` port and a test-local no-op
adapter. It adds no production fallback or public seam and keeps the exact
driver-spec assertions.

`redb_view_store_probe_fails_on_readonly_fs` modelled a read-only storage
surface by changing only its parent directory. The existing `memory.redb`
file remained writable, so redb reopened it `O_RDWR` and the probe correctly
succeeded. The correction closes the store, makes both the canonical file and
its directory read-only, reopens, then restores their original permissions.
It changes no production store behavior or expected `ProbeError`.

Both corrections passed as the unprivileged Metal login identity through the
current compiled libtest binaries in durable run
`c72471e83359a3f4b7d38b01`: two selected, two passed, zero failed.

### Genuine additional native identities — 23

The 20 control-plane integration identities are:

- two `adopt_on_restart` netns/nft bodies;
- seven `dns_responder_bind` real-`:53`/socket/composition bodies;
- two `serve_boot_provisions_veth` XDP/veth bodies;
- seven `veth_provision_idempotent` veth/offload bodies; and
- two `workload_netns_provision` netns/veth/resolver bodies.

They are the exact `CONTROL_PLANE_NORMAL_NATIVE` array and execute after the
retained 55+18+5=78 boundary in the existing control-plane
`cgroup-default` guest. Runtime accounting remains fail-closed at 55, 18, 5,
78, then adds exactly 20 and requires 98 complete. Uppercase/lowercase runtime
skip diagnostics are rejected for all 20.

```text
integration::adopt_on_restart::serve_restart_readopts_surviving_slot_and_gcs_orphan_netns
integration::adopt_on_restart::serve_restart_sweeps_surviving_per_workload_tproxy_rule
integration::dns_responder_bind::empty_fallback_binds_zero_sockets_and_warns_it_is_deaf
integration::dns_responder_bind::per_gateway_addr_fallback_binds_when_wildcard_is_held
integration::dns_responder_bind::probe_refuses_when_no_bindable_port
integration::dns_responder_bind::probe_refuses_when_store_unreadable_at_listseed
integration::dns_responder_bind::run_server_refuses_boot_when_the_guest_dns_probe_fails_through_the_required_port
integration::dns_responder_bind::the_responder_audit_reads_back_its_socket_and_fails_after_loss
integration::dns_responder_bind::wildcard_bind_answers_frontend_and_source_pins_reply
integration::serve_boot_provisions_veth::serve_boot_attaches_two_distinct_xdp_programs_to_two_distinct_veths
integration::serve_boot_provisions_veth::single_iface_collision_surfaces_typed_iface_xdp_slot_busy
integration::veth_provision_idempotent::provision_complete_pair_converges_to_silent_success
integration::veth_provision_idempotent::provision_completes_half_provisioned_pair
integration::veth_provision_idempotent::provision_creates_pair_when_absent
integration::veth_provision_idempotent::provision_disables_tx_offload_on_both_ends_and_is_idempotent
integration::veth_provision_idempotent::provision_recreates_pair_when_client_absent_but_peer_present
integration::veth_provision_idempotent::provision_recreates_pair_when_peer_absent
integration::veth_provision_idempotent::provision_repairs_tx_offload_drifted_back_on
integration::workload_netns_provision::provision_creates_and_idempotently_converges_per_workload_netns
integration::workload_netns_provision::provision_injects_node_local_responder_into_netns_resolv_conf
```

The three dataplane library identities are the two real
`hash_of_maps` BPF-map bodies and
`reverse_local_map_handle::real_map_tests::remove_deletes_the_reverse_entry`.
`guest-kit-prepare` stages the existing dataplane library libtest beside the
integration libtest. The same `mtls-relay` guest runs it only after the
retained 7+36=43 boundary; accounting then requires exactly three and 46
complete. No integration identity is duplicated.

```text
maps::hash_of_maps::tests::alloc_failure_with_zero_inner_entries_yields_typed_variant
maps::hash_of_maps::tests::create_outer_and_inner_then_atomic_swap
maps::reverse_local_map_handle::real_map_tests::remove_deletes_the_reverse_entry
```

The audit covered every binary kind in the three-package list: control-plane
library, acceptance, integration, compile-pass/fail and registry binaries;
dataplane library, integration, acceptance, properties, schema and tooling
binaries; and the netlink library. Source guard search was reconciled against
the actual runtime output. The remaining root checks in
`alloc_netns_lifecycle` protect optional bonus observations after the
unprivileged in-memory slot oracle has already executed; they do not turn a
native behavior claim into a pass. Previously routed cgroup, bridge and
dataplane-integration guards are already in the retained manifests. Netlink
has no active root/CAP bailout. Thus no unprivileged runtime bailout remains
outside the exact additional 20 + 3 ledger.

## Worker-guest ordering and qualified-node read-back

The prior 73-body sequence remains intact. STEP 13-04's disposable dummy
uplink and main-table default route still exist only around its two host-output
constant-rule cells. The runner always removes both and verifies the
non-gateway guest-prefix address is unroutable before running the old remote
and leg-C cells. Only after that prior sequence completes does the STEP 13-05
leg-B environment begin.

The STEP 13-05 environment then:

1. pins `net.ipv4.conf.{all,default,lo}.route_localnet=0` and reads all three
   values back as exactly zero;
2. installs the accepted GW-LOCAL fixture precondition, the exact
   `local 100.95.0.1/32 dev lo proto 247` route;
3. before and after every leg-B body, verifies that exact route in the local
   table, an empty main table, failed lookup of `100.95.255.200`, only `lo` in
   the root namespace, no stale `fwmark 0x1 lookup 100` rule and no stale
   table-100 route.

The body itself remains the production oracle for the transient policy route:
`HostMtlsIntercept::start_shared_owner` installs the real mark-1 rule and
table-100 local route, and each `Node::finish` plus its existing cleanup
returns the guest to the audited clean boundary. The positive cell requires
leg C at the exact registered `VM_ADDR:SERVICE_PORT`, a loopback source at leg
B and leg C, and `fwmark:0x1` on the established leg-B socket. Because the
disposable guest has no uplink or default route at that cell, and because leg
C accepted the exact local tuple, the packet cannot have escaped through an
uplink. The outside-prefix control creates only its own route-disjoint veth
peer, proves the source is the peer-link host address rather than loopback,
and removes the veth before the runner's post-cell audit. The unregistered
tuple control still requires typed `Io(ConnectionRefused)` within the bound
and a silent wildcard listener.

The gateway fixture route is never removed by a production owner. The guest's
destruction is the fixture cleanup, so no host route is installed or removed.

## Tools, runtime files and modules

The five STEP 13-05 bodies retain their prior tool treatment: the two
control-plane cells use `ip`; the worker cells use `ip`, `nft`, `ss`, `uname`
and the newly staged `sysctl`. The 36 dataplane bodies add only the tools their
existing source actually invokes: `bpftool`, `cat`, `ethtool`, `id`, `ip`,
`kill`, `ln`, `mkdir`, `mount`, `nc`, `nft`, `python3`, `sh`, `ss`, `strace`,
`sysctl`, `tar`, `tcpdump`, `true` and `uname`. The original direct-executable
audit `58bdbec68c4ea7b3bc69fc54` confirmed the other tools and the qualified
runner's `/usr/lib/python3.14` but missed the indirect
`ThreeIfaceTopology::create -> NetNs::ethtool_tx_off` call. `tool()` now stages
the existing `ethtool` binary at `/usr/bin` with its ELF interpreter and `ldd`
library closure. Bounded Metal run `6c6a56320ecafbfd509a3609` proves that
staging through the exact S-05-C replay.

The dataplane payload already staged Ubuntu `/usr/bin/tcpdump` and its `ldd`
closure, but the minimal GuestKit `/etc` contained neither `passwd` nor
`group`. The qualified tcpdump 4.99.6 package is configured to drop from root
to the `tcpdump` account before reporting capture readiness. The runner now
stages only minimal guest-local root and `tcpdump:x:985:985` passwd/group
entries and fails closed on their exact contents plus `id tcpdump` readback.
It does not copy a host account database, stage another executable, add a
chroot, or force `tcpdump -Z root`.

The 20 additional control-plane bodies reuse the existing cgroup guest's
`ip`, `nft`, `uname`, cgroup-v2, bpffs and `/var/run` support and add only the
already-invoked `ethtool`, `setsid` and `sysctl` tools. Their payload also
contains an empty ordinary `/etc/resolv.conf`, the standard bind target that
`ip netns exec` requires for production's per-netns resolver source. The
control-plane integration
guest now loads the same previously approved reject/tproxy/defragmentation
module closure as the worker guest; this is required by its real composed mTLS
probe and adopt-on-restart nft cases. No new module kind or physical-host load
is introduced. Read-only Metal `382b46beb8c157f6986e6ba0` confirms both
tools and ethtool's complete `ldd` closure.

The existing Python subprocess fixtures import `socket`, `sys` and `time`.
The host runner creates an uncompressed archive of the qualified runner's
actual `sysconfig` standard-library directory, stages it at
`/opt/overdrive-testing/python-stdlib.tar`, and extracts it before any of the
36 bodies. This includes the matching `lib-dynload` extension modules without
inventing a replacement echo server or changing a body. The standard library
was 30 MiB in the read-only audit; GuestKit's existing image sizing accounts
for both the staged archive and its extracted tree.

`cargo xtask guest-kit-prepare` remains the sole BPF producer. It runs
`bpf-build` before compiling/staging the dataplane integration and library
libtest binaries. The library binary is copied beside the integration binary
at `/opt/overdrive-testing/dataplane-lib-tests`; the existing guest peer and
exact libtest protocol execute it after the retained 43-body boundary. The
runner stages that same workspace artifact at its exact compiled path,
`/home/ubuntu/overdrive/target/bpf/overdrive_bpf.o`; a missing artifact makes
image staging fail before guest boot. Tests that use `include_bytes!` retain
their embedded object, while the existing `atomic_swap`, `maglev_real`,
`service_map_forward` and `sanity_mixed_batch` file-loading paths see the exact
file they were compiled to open.

Before the routed-native phase, the guest mounts private cgroup-v2 and bpffs
filesystems, installs the existing `/var/run -> /run` netns alias, extracts
the Python runtime, proves the BPF artifact and external tools, and prints the
artifact's SHA-256. The
original seven relay bodies run first with only their original TLS module
loaded; the extra substrate therefore cannot make the baseline pass.

The additional module closure is staged from the running 7.0 Metal kernel and
loaded dependency-first inside the guest only: `nfnetlink`, `nf_tables`,
`veth`, `nf_defrag_ipv4`, `nf_defrag_ipv6`, `nf_conntrack`, `nf_nat`,
`nf_reject_ipv4`, `nf_reject_ipv6`, `nf_tproxy_ipv4`, `nf_tproxy_ipv6`,
`nft_reject`, `nft_reject_ipv4`, `nft_tproxy`, `nft_nat`, `nft_chain_nat` and
`nft_masq`; the existing TLS module is already live. Read-only module-path
audit `afb0538bee7e6b0e28a2f6df` confirms the prior closure. Read-only Metal
audit `9f6fc1d6358b26463f1062d0` confirms the added pinned-kernel file
`/lib/modules/7.0.0-38-generic/kernel/net/netfilter/nft_nat.ko.zst`, alias
`nft-expr-nat`, and exact dependencies `nf_tables,nf_conntrack,nf_nat`. No
nested KVM/vhost module, production hook or physical-host load is added to
`mtls-relay`.

The existing seven pinned nested-device modules remain unchanged. The worker
guest's full `cgroup-default` module order is unchanged; the control-plane
integration guest reuses the identical existing nft closure for its 20 exact
new rows. All new BPF, netns, nft, cgroup, XDP and kTLS effects occur after
Cloud Hypervisor boot, through the existing guest peer, and die with that
disposable guest.

## First native replay causal receipt and nft-expression closure

The first actual 46-body `mtls-relay` replay is durable Metal run
`832e3a551f09ac4d4fd08724`, exit 101. Raw output is retained at
`.context/netns-density-295/13-05-mtls-relay-final-raw.log`, SHA-256
`2c00a7110c162a2efd37d0aabffcc2f148351058fe93935177744e7faeb0d49c`.
The original seven-body relay prefix completed first under its TLS-only module
state. The first exact routed-native invocation then reached
`integration::mtls_agent_handshake::agent_handshake_presents_held_svid_server_role`
and failed at the unchanged topology assertion because nft could not create
this existing output rule:

```text
meta mark 2 ip daddr 127.0.0.2 tcp dport 18443 dnat to 10.66.0.2:18443
                                               ^^^^
Error: Could not process rule: No such file or directory
```

The selected-native source audit finds one nft program, in
`tests/integration/helpers/mtls_netns_topology.rs`. Its expressions are exactly
the prerouting `tproxy`, output `dnat`, and postrouting `masquerade` rules used
by the selected mTLS-agent, inbound-enforcement, and guardrail families.
`nft_tproxy`, `nft_masq`, `nft_chain_nat`, `nf_nat`, and their dependencies
were already staged. No selected native body installs an nft `redirect`,
`redir`, or explicit `snat` expression; reverse-NAT and redirect wording in
the other selected families names BPF/XDP behavior rather than an nft
expression. Therefore `nft_redir` and other unrelated netfilter modules remain
outside the closure.

Linux v7.0 source registers the `nat` expression in
`vendors/linux/net/netfilter/nft_nat.c` with `MODULE_ALIAS_NFT_EXPR("nat")`;
the same implementation maps `NFT_NAT_DNAT` to `NF_NAT_MANIP_DST` and
`NFT_NAT_SNAT` to `NF_NAT_MANIP_SRC`. The pinned Metal kernel independently
confirms `CONFIG_NFT_NAT=m`, the `nft-expr-nat` alias, and the exact module
dependencies in read-only run `9f6fc1d6358b26463f1062d0`. The bounded repair
stages and loads only `net/netfilter/nft_nat` after `nf_nat` and before the NAT
chain/masquerade expression modules. It changes no test body, assertion,
group, guest, count, production behavior, physical-host module state, or
execution order.

## Second native replay classification and TX-offload confirmation

Iteration 5 approved the `nft_nat` closure. The next actual `mtls-relay`
attempt, durable Metal `fe0c4da54d92be8295bd350f`, reached the existing
S-05-C body
`integration::multi_listener_tcp_udp_e2e::adding_listener_on_resubmit_converges_without_breaking_existing`
and exited 101. Raw output is retained at
`.context/netns-density-295/13-05-mtls-relay-final2-raw.log`, SHA-256
`76cf5eb858ad7b60db932ba2169f8fb551d8777f8540b9e8ed166c0fb1f65743`.
All three reverse-NAT keys were present, but the unchanged UDP/8082,
UDP/8081 and TCP/8080 clients all produced empty stdout. The first assertion
reported UDP/8082 only because it is first; the failure affected both new and
retained listener paths.

The existing `ThreeIfaceTopology::create` source declares TX checksum offload
disabled on all four veth ends as a required precondition for incremental
XDP L4 checksum updates. It invoked `ip netns exec <ns> ethtool -K <iface> tx
off` four times but discarded every result. The dataplane GuestKit payload did
not contain `ethtool`, so none of those commands could execute in the failing
guest. Linux v7.0 veth defaults include `NETIF_F_HW_CSUM`; prior project
evidence in `docs/analysis/post-pivot-s-2-2-17-probe-2.md` records the same
CHECKSUM_PARTIAL mechanism producing backend checksum drops. Reverse-map
presence alone does not prove that a forward packet reached its backend.

The bounded fixture correction does exactly two things:

1. stages the existing `ethtool` executable in the existing dataplane guest
   payload; and
2. replaces the four discarded results with a private fail-closed helper that
   applies TX offload disable, reads back `ethtool -k`, requires the aggregate
   `tx-checksumming: off` line and prints the namespace/interface/line.

It adds no public method, error variant, product behavior or scenario. The
existing `NetNsError::IpCommand` reports command or readback failure. Existing
sysctl, route, neighbour, XDP attachment and listener behavior is unchanged;
the receipt did not justify hardening unrelated setup.

The S-05-C body retains its three reverse-key assertions and all three traffic
oracles unchanged. Diagnostic-only output now records its topology, routes,
neighbours, backend listeners and loaded BPF attachment/program identities
before any assertion can unwind the namespaces. The prior `pcaps:` path was
not evidence: the body only created an empty directory and started no capture.
No pcap existed to retain after the failed guest was destroyed.

The canonical bounded harness is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-a`. It uses the
existing public GuestKit fixture and scripted peer, the same prepared
dataplane integration binary, exact module/tool payload and exact S-05-C
identity. It creates no selectable group, environment selector, CI lane,
executor or crate-local probe. Its canonical raw evidence is retained at
`increment-a/evidence/metal-6c6a56320ecafbfd509a3609.log`.

Durable Metal run `6c6a56320ecafbfd509a3609`, exit zero, compiled the current
support source, prepared the existing GuestKit artifacts and ran only exact
S-05-C. Its retained raw receipt is
`.context/netns-density-295/13-05-s05c-offload-bounded-raw.log`, SHA-256
`99c07ea3b978d90025f0fc4fb379c6dc3b8ba8570c579a86aa2b4e2791545c82`.
The receipt proves:

- all four ends read `tx-checksumming: off`;
- client and backend default routes plus both LB connected routes were
  present;
- both permanent LB neighbours were present;
- TCP/8080 and UDP/8081/8082 listeners were bound;
- `xdp_service_map` and `xdp_reverse_nat` were attached to the two LB veths,
  with `xdp_pass` on both peers;
- the unchanged UDP/8082, UDP/8081 and TCP/8080 clients received respectively
  `ovd-multi-c-udp2`, `ovd-multi-c-udp1` and `ovd-multi-c-tcp`; and
- libtest reported exactly one pass, zero failures/ignored and
  `S05C_BOUNDED_REPLAY_PASS`.

The exact prepared BPF artifact was 10,744 bytes with SHA-256
`5aec54ee315fbf451b313b1e1d1faf0de9ff6fdb89b262357b36ae4f76d19e3a`
in read-only Metal `db410cb6ebd04a1ac7b54879`; the bounded receipt also
records the loaded program tags. The pass with the required four readbacks
confirms the disposable-guest fixture cause. It establishes no production BPF
defect and does not substitute for a full 46-body receipt.

## Third native replay classification and tcpdump account closure

Iteration 6 approved the TX-offload fixture correction. The next actual
`mtls-relay` attempt, durable Metal `b823070869ab5ea5ec879e01`, exited 101 at
the existing body
`integration::reverse_nat_udp_e2e::every_udp_reply_independently_source_rewritten`.
Raw output is retained at
`.context/netns-density-295/13-05-mtls-relay-final3-raw.log`, SHA-256
`00c8b001feee2e26b79e08f14b1eb3465df35a1d30885d85425464dd046df84a`.
All four TX-offload readbacks passed. The client veth entered and immediately
left promiscuous mode, then the unchanged readiness oracle failed because its
stderr channel disconnected before `tcpdump` emitted `listening on`.

Source and package audit established that `tcpdump` was already staged with
its ELF/`ldd` closure and executed through the exact nested
`ip netns exec` path. Read-only Metal `fa7b7ae4ef9eab0be777ce21` records the
qualified `/usr/bin/tcpdump` 4.99.6 binary and the existing unprivileged host
identity `tcpdump:x:985:985:tcpdump:/nonexistent:/usr/sbin/nologin` with group
`tcpdump:x:985:`. Ubuntu's 4.99.6-2build1 package configures tcpdump with
`--with-user=tcpdump` and installs that sysuser/group; the program resolves and
drops to it after opening the capture interface and before its readiness
banner. GuestKit creates an empty `/etc` and copies only declared files, so the
runtime identity dependency was absent despite the successful `tcpdump
--version` preflight.

The retained diagnostic at
`spike-scratch/netns-density-295-dataplane-offload/increment-b` proves the
immediate cause rather than inferring it from the disconnected channel.
Durable Metal `9d95728658b949ca36438ff0`, exit 101, executed the fixture's exact
guest-private `ip netns exec … tcpdump -U -n -i … -w … udp` shape. It retained
absent `/etc/passwd` and `/etc/group`, promiscuous-mode enter/leave, exit 1 and
the exact stderr:

```text
tcpdump: Couldn't find user 'tcpdump'
```

Its canonical raw SHA is
`10cde4d5090066d37d99f011b9a7cf58d36d10c13b21e1ea5335660dd3f972b6`.
The prior full-run readiness assertion remains unchanged and correctly failed
closed; it was not weakened to accept missing capture.

The bounded repair stages two guest-only data files in the existing payload:

```text
/etc/passwd:
root:x:0:0:root:/root:/bin/sh
tcpdump:x:985:985:tcpdump:/nonexistent:/usr/sbin/nologin

/etc/group:
root:x:0:
tcpdump:x:985:
```

The setup script requires those exact bytes and requires
`id tcpdump == uid=985(tcpdump) gid=985(tcpdump) groups=985(tcpdump)` before
any selected native body. This preserves tcpdump's package-default privilege
drop. No `-Z root`, capability, setuid bit, chroot, extra user, extra tool or
physical-host identity change is introduced. The Cloud Hypervisor process
still uses uid/gid 4200, its existing KVM/vhost supplementary groups,
Landlock, seccomp and `no_new_privs`; the tcpdump identity exists only inside
the disposable root filesystem.

Durable Metal `ed145fdfdd75dca1dbfe8c7b`, exit zero, reran only the exact
unchanged native body with those two files. The same inner diagnostic remained
live across the readiness window and emitted `listening on`. The authored
case then:

- captured all three outbound query witnesses;
- received `dns-query-0`, `dns-query-1` and `dns-query-2`;
- observed all three captured reply sources as VIP `10.0.0.1`; and
- reported one passed, zero failed/ignored and
  `UDP_CAPTURE_BOUNDED_REPLAY_PASS`.

The passing canonical raw SHA is
`9bb8dc6b24eba7f3d61c2315e810e4c3e1fe8765515e514c290bab390402aabf`.
This proves the guest account closure without changing capture arguments,
readiness, pcap parsing, positive-control counts or traffic assertions.

The complete transitive selected-native call-graph audit covered all 46
dataplane and 98 cgroup bodies, including nested `ip netns exec` and helper
commands. It found no other source-proven executable omission. The dataplane
set remains exactly `bpftool`, `cat`, `ethtool`, `id`, `ip`, `kill`, `ln`,
`mkdir`, `mount`, `nc`, `nft`, `python3`, `sh`, `ss`, `strace`, `sysctl`,
`tar`, `tcpdump`, `true` and `uname`; the module and sysctl closures remain
unchanged. This source audit does not claim the still-unrun complete 46/98
runtime gates.

## First cgroup replay classification and process/filesystem closure

After iteration 7 approval, the first actual 98-body `cgroup-default` attempt
was durable Metal `3e3b3463786963e459aadc2a`, exit 101. Raw output is retained
at `.context/netns-density-295/13-05-cgroup-default-final-raw.log`, SHA-256
`35f737fd9a985ebd8968ca0bc21f1533f421d52c9b5be809e329780543f7ef23`.
The retained three acceptance bodies passed. The first exact control-plane
native suffix body,
`integration::adopt_on_restart::serve_restart_readopts_surviving_slot_and_gcs_orphan_netns`,
then reported:

```text
Bind /etc/netns/ovd-ns-0030/resolv.conf -> /etc/resolv.conf failed: No such file or directory
exec of "setsid" failed: No such file or directory
SKIP ... survivor PID 2084 did not land in ovd-ns-0030 (fixture precondition)
```

Libtest returned zero, but the existing runner correctly rejected the runtime
skip. No adoption, slot-collision or orphan-GC oracle was accepted as a pass.

The audited source path is exact. `provision_workload_netns` creates
`/etc/netns/<ns>/` and writes its resolver source itself. `ip netns exec`
expects the ordinary bind target `/etc/resolv.conf` to exist in the guest root
filesystem. The test then directly executes
`ip netns exec <ns> setsid sleep 3600`; the existing guest already stages its
`/bin/sleep` alias. There is no `sh` in this subprocess chain and no need for a
new `/bin/sh` alias, resolver daemon, host resolver content or DNS route.

The bounded correction therefore:

1. stages qualified `setsid` with its normal ELF/`ldd` closure only for the
   control-plane integration guest's 20-body native suffix;
2. stages a generated zero-byte ordinary file at `/etc/resolv.conf` in that
   same guest; and
3. immediately before the 20-body suffix, requires that target to be an empty
   non-symlink regular file and proves `/usr/bin/setsid /bin/sleep 0` exits
   zero.

The source file and resolver content authored by production remain unchanged.
The fixture supplies only the standard destination inode on which `ip netns
exec` performs the namespace-specific bind. No host `/etc/resolv.conf` is
copied and no resolver forwarder or account is added.

The follow-up transitive vector/script audit covered all selected 98+46 bodies,
including `ip netns exec` children and shell scripts. It found no further
source-proven requirement: remaining control-plane suffixes use the already
staged nft/uname/Rust socket, ip/ethtool, and ip/sysctl/ethtool/cat paths;
retained worker `/bin/sleep` remains covered; `/bin/true` appears only as
unexecuted allocation metadata in the selected sim/intercept paths. The runner
therefore does not bulk-stage another tool or alias.

The canonical bounded proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-c`. It reproduces
the existing control-plane GuestKit boundary and runs only the exact unchanged
adoption case. Durable Metal `73e45a4bc5b719df49b62f80`, exit zero, reported
one pass, zero failed/ignored, no runtime skip and
`CONTROL_PLANE_NATIVE_BOUNDED_REPLAY_PASS`. The original case assertions prove
that the survivor PID entered the provisioned namespace and cgroup, recovery
kept that namespace, adopted its slot, prevented a fresh-slot collision and
garbage-collected the orphan namespace. Canonical raw SHA-256 is
`202242ae3ddbb958cc9a612662adbe9b552c892c7850bea2df4c7ebd3a475d53`.

The bounded pass confirms the GuestKit process/filesystem closure. It does not
substitute for the complete 98-body receipt and establishes no production
networking, resolver, adoption or cgroup defect.

## Second cgroup replay classification and DNS fallback fixture

After iteration 8 approval, the next actual 98-body attempt was durable Metal
`3421ff034fb99c41ea416500`, exit 101. Raw output is retained at
`.context/netns-density-295/13-05-cgroup-default-final3-raw.log`, SHA-256
`7a9c0bd872352433677c82e5f83959bfe300ec21fa84d99060457f09d7460781`.
The first DNS suffix case,
`integration::dns_responder_bind::empty_fallback_binds_zero_sockets_and_warns_it_is_deaf`,
reached its existing positive oracle and failed because the exact
`100.95.0.1:53` fallback bind returned `EADDRINUSE`.

The failure is not a leaked production or adoption-owned DNS descriptor. Each
selected identity runs in a fresh libtest process and GuestPeer waits for that
process to exit. The two preceding adoption bodies create only a CLOEXEC
`setsid sleep` survivor in its workload namespace and nft state respectively;
their guards reap process/netns/cgroup state. Neither path creates a DNS
socket, and CLOEXEC prevents an exec survivor from inheriting one.

The existing positive DNS fixture was intrinsically contradictory. It first
successfully created its own standard-library UDP holder at wildcard
`0.0.0.0:53`; that socket has `SO_REUSEADDR=0` and `SO_REUSEPORT=0`. Production
then attempted wildcard bind with `SO_REUSEADDR=1`, received `EADDRINUSE`, and
fell back to the exact gateway. Linux v7.0's UDP overlap predicate rejects the
exact bind whenever either overlapping socket lacks reuse, so the fixture's
own wildcard holder also blocked the fallback it expected to succeed.

Prior package summaries did not validate this positive path: on physical-host
runs, an unrelated `:53` owner made the stand-in holder fail first and the body
returned a runtime skip. The disposable guest was the first boundary where the
private holder bound and its contradictory positive assertion actually ran.

The canonical causal proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-d`. Durable Metal
`a1a5715691ae238e0a7032a7`, exit zero as a diagnostic, compared a fresh DNS
case with the exact two-adoption-plus-DNS prefix. In both guests, before the DNS
body:

- `/proc/net/udp` and `ss` showed no UDP/53 socket or owner;
- the process was in root netns inode `4026531833`;
- only loopback `127.0.0.1/8` existed; and
- no local route covered `100.95.0.1/32`.

After the body created its holder, the sole UDP/53 owner was that same libtest
PID, fd 9, with the recorded socket inode, command, fdinfo, root netns,
`SO_REUSEADDR=0`, `SO_REUSEPORT=0`, and `FD_CLOEXEC=1`. The two adoption cases
passed and left the subsequent DNS pre-state equally empty. Canonical raw
SHA-256 is
`d82c5a2ad66c928e9b8edc59ec3082073c18d9a975fe49f3db720a5e0394e209`.

The proof also exposed the separate qualified-node premise: the control-plane
GuestKit had no local `100.95.0.1/32`. Its selected source establishes no such
route; adoption veths use a distinct 10.99.x plan and are torn down. Without a
local gateway address, a corrected positive holder would reveal
`EADDRNOTAVAIL` instead of exercising fallback success.

The bounded correction preserves the existing positive outcome and production
bind behavior:

1. Both existing positive fallback bodies now use one private helper that
   binds the appliance-shaped specific loopback resolver stand-in at
   `127.0.0.53:53`. Its no-reuse socket still makes production's wildcard bind
   return `EADDRINUSE`, but it does not occupy the distinct shared-gateway
   address too.
2. The control-plane native suffix setup owns exactly
   `local 100.95.0.1/32 dev lo proto 247`, reads that route back, and proves a
   non-gateway prefix address remains unroutable. It installs no interface
   address, prefix-wide route, default route or namespace.
3. After the 20-body suffix, the runner removes that exact proto-247 route and
   reads back both its absence and continued non-gateway unroutability. On a
   body failure it attempts the same cleanup before the disposable guest is
   destroyed.

The negative no-bindable-port control and its expected typed `Bind` error are
unchanged. No production `SO_REUSEADDR`, wildcard-first/fallback algorithm,
socket ownership, public API or error mapping changes.

Canonical fixed proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-e`. Durable Metal
`61d8273016ed5daa3b3cf1cb`, exit zero, ran both positive fallback bodies in a
fresh guest and again after the exact two-adoption prefix. Every run had zero
preexisting UDP/53 owners, the exact local proto-247 route, a private
`127.0.0.53:53` holder, and a distinct production-owned
`100.95.0.1:53` socket after probe. All six exact bodies passed without skip,
and the proof ended `DNS53_QUALIFIED_FIXTURE_PASS`. Canonical raw SHA-256 is
`c9e09ba097cadab006ba7627a4869220d1022f11427be38c39a80d686ff58edc`.

This is a private test-input and qualified-node correction. It establishes no
production DNS lifecycle defect and does not substitute for the complete
98-body receipt.

## Iteration-9 review remediation — DNS negative control and skip gate

Iteration-9 source review approved the positive DNS stimulus and exact gateway
route but found one remaining genuine fixture/checker defect in the active
negative body `probe_refuses_when_no_bindable_port`.

Before remediation, its first standard-library wildcard holder successfully
bound `0.0.0.0:53` with reuse disabled. That socket alone overlaps every IPv4
local address, so it already made both production's wildcard attempt and its
exact gateway fallback return `EADDRINUSE`. The body nevertheless attempted a
second exact gateway holder. Linux correctly refused that redundant holder;
the body printed uppercase `SKIP:` and returned before its original typed
`DnsResponderError::Bind` assertion.

The existing cgroup required-case detector recognized `[skip]`, `SKIP `,
lowercase `skip:` and `(skip)`, but not uppercase `SKIP:`. Therefore libtest's
one-pass summary was accepted even though the negative oracle never ran.

Canonical old-case proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-f`. Durable Metal
`15c30cfa1bb75e34e45f9637`, exit zero, retained:

```text
SKIP: could not bind stand-in gateway :53 at 100.95.0.1
test result: ok. 1 passed; 0 failed; 0 ignored
DNS53_NEGATIVE_OLD_SKIP_HOLE_CONFIRMED
```

Raw SHA-256 is
`f15a1b4460260a0cdaac04d75d0361804a57a0857d0e93e08b6757aaa9f54ab6`.

The bounded fixture correction removes only the impossible second holder. The
first non-reuse wildcard holder remains unchanged and remains the real
all-address block. Production bind order, reuse flags, fallback, typed error
and the body's exact `matches!(err, DnsResponderError::Bind { .. })` assertion
remain unchanged. Diagnostic output retains the actual typed error.

The runner now has one private line-oriented skip detector shared by dataplane
and cgroup required invocations. It recognizes exactly the documented prefixes
`[skip]`, `SKIP `, `SKIP:`, `skip:` and `(skip)`. For libtest's inline output,
it examines text after the ` ... ` test-name separator; otherwise it examines
the trimmed line start. It therefore rejects the retained uppercase bailout
without treating a lowercase test identifier or arbitrary prose containing
“skip” as infrastructure failure.

Canonical fixed proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-g`. Durable Metal
`a9a3ea8eb54fc1ee4697ed0d`, exit zero, first applied the new detector semantics
to the retained old receipt and printed
`old-uppercase-SKIP-colon=new-detector-reject`. The exact unchanged negative
case then retained the original typed result:

```text
DNS53 negative typed error: Bind {
  addr: 100.95.0.1:53,
  source: ... AddrInUse ...
}
test result: ok. 1 passed; 0 failed; 0 ignored
DNS53_NEGATIVE_TYPED_ASSERTION_PASS
```

No skip marker occurred. Raw SHA-256 is
`de3b767616128c2285e2906260e203a805f52c965c1cfe2476f8d6e229368cf5`.
This remediates only private fixture setup and bailout detection; it adds no
scenario, body count, public API, selector, lane, executor or product behavior.

## Post-iteration-10 cgroup replay — created versus borrowed gateway route

After iteration 10 approval, durable Metal `46a9510e9f4754ac06a947dc`
attempted the complete 98-body gate and exited 101 before the 20-body suffix.
Raw output is retained at
`.context/netns-density-295/13-05-cgroup-default-final4-raw.log`, SHA-256
`aa273d913d510edce182f27e8d37c2b23bad7e1dab981dfc98d624fcdd2888a5`.
The retained and shared-startup bodies had already passed. The suffix setup
then required the gateway route to be absent before adding its fixture route;
that absence check failed.

The prior state is legitimate product-created GW-LOCAL state, not residue to
delete. The accepted contract makes the gateway route boot-sticky:
`HostGuestFlowIo::converge_gateway_route` calls
`Client::converge_ipv4_host_local_route(GUEST_NODE_GATEWAY,
GUEST_PREFIX_ROUTE_PROTOCOL)`, whose fixed shape is table local, destination
`100.95.0.1/32`, output `lo`, type Local, scope Host, no gateway, protocol 247.
It keeps an identical route, replaces a differing route and adds a missing
route. Shutdown and stale-object sweep never remove it.

The fixture precondition therefore contradicted both the qualified sequence
and GW-LOCAL: it demanded absence, then later deleted the route without knowing
whether the fixture created it or a prior product boot owned it. A nonzero
absence command alone was not accepted as route readback.

The private runner now classifies the complete kernel rendering before the
20-body suffix:

- **missing:** add exactly
  `local 100.95.0.1/32 dev lo proto 247`, write private ephemeral ownership
  marker `created`, and read the route back;
- **identical existing:** write marker `borrowed`, make no route mutation, and
  read the route back;
- **any differing route at that key:** fail closed with the actual rendering;
  no replace or broad repair.

Both accepted branches require the normalized exact rendering
`local 100.95.0.1 dev lo proto 247 scope host` (with an optional explicit
`table local` token from iproute output) and still prove
`100.95.255.200` unroutable. They install no prefix-wide route, default route,
interface address or namespace.

Cleanup reads the exact route and private marker again. It removes the route
only for `created`, then proves absence. For `borrowed`, it preserves and
re-reads the identical route. A differing/missing borrowed route or an unknown
marker fails closed. On a body failure the runner attempts the same branch-
specific cleanup before disposable-guest destruction. The marker is private
GuestKit bookkeeping under `/run`; it is not a production ownership protocol.

Canonical bounded proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-h`. Durable Metal
`678b2e2aba65af9927e27fe0`, exit zero, exercised both branches in separate
disposable guests:

- fresh branch recorded `ownership=created`, ran both positive DNS bodies,
  and cleanup read `gateway_after=''`;
- preexisting branch installed the exact qualified route, recorded
  `ownership=borrowed`, ran both adoption and both positive DNS bodies, and
  cleanup retained exact
  `local 100.95.0.1 dev lo proto 247 scope host`.

Both branches had distinct private resolver-holder and production-gateway DNS
sockets, zero skips, and non-gateway prefix unroutability. The proof ended
`DNS53_GATEWAY_OWNERSHIP_PASS`. Canonical raw SHA-256 is
`274e4541d3e9bab3a99106709995f6ff17f8d47237ab9c1a59538fca904f2033`.

This corrects only fixture ownership bookkeeping. It leaves production route
convergence/lifetime and every DNS/adoption oracle unchanged, and it does not
substitute for the complete 98-body receipt.

## Post-iteration-11 cgroup replay — strict LEGB route rendering

After iteration 11 approval, durable Metal `4575d307849bb67617734275`
attempted the complete 98-body `cgroup-default` gate and exited 101. Raw output
is retained at
`.context/netns-density-295/13-05-cgroup-default-final5-raw.log`, SHA-256
`4daf2d06b59b4be50222c364d93e8f56f16fd575bd4107f1259b7b91f691af30`.
The run passed the 20 control-plane native suffix and the worker retained and
STEP 13-04 bodies through temporary-route cleanup. The first STEP 13-05 leg-B
audit then rejected this actual local-table readback:

```text
local 100.95.0.1 dev lo proto 247 scope host 
```

The route-localnet and preceding route checks had passed. The route is the
exact qualified GW-LOCAL state approved in iteration 11; the only difference
from the audit's accepted literal is the single trailing ASCII separator
rendered by this `iproute2` build. This is a private readback-parser defect,
not a route-contract or production defect.

The audit now reuses the already reviewed control-plane gateway readback
normalization: shell suffix removal `${gateway% }` once before its existing
strict `case`. It therefore accepts the observed one-separator rendering while
preserving every token and rejects a second trailing space, unknown attribute,
changed type/table/destination/prefix/interface/protocol/scope, gateway
attribute or other route shape. The alternate rendering with an explicit
`table local` token remains the only other accepted literal.

No other audit changed. Each existing leg-B body still requires all/default/
loopback `route_localnet=0`, an empty main table, no route to
`100.95.255.200`, only loopback as node interface, no fwmark-1 rule and an
empty table 100 both before and after execution. The normalization adds no
tool, guest file, selector, body, group, lane, executor, API or production
behavior.

Canonical bounded GuestKit proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-i`. Durable Metal
`6eeb8cde64128fd1c0daa173`, exit zero, captured the raw route with the single
trailing space and the normalized exact route before and after the unchanged
existing body
`vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c`.
Both audits retained the complete qualified-node complement. The body reported
one pass, zero failed/ignored, with the client socket sourced from
`127.0.0.1`, marked `1`, and connected to the guest-prefix leg-B destination.
The receipt ended `LEGB_AUDIT_BOUNDED_REPLAY_PASS`; canonical raw SHA-256 is
`cb0c1b578b3d869d3965d474bbd8fc36070356808a8d7a5e7bbceee090dce63e`.

This bounded receipt proves the parser correction and unchanged body oracle.
It does not substitute for the complete 98-body gate.

## Post-iteration-12 cgroup replay — dummy module automatic device

After iteration 12 approval, durable Metal `ff82cf33b0ddeb9b2b00fc50`
attempted the complete 98-body `cgroup-default` gate and exited 101. Raw output
is retained at
`.context/netns-density-295/13-05-cgroup-default-final7-raw.log`, SHA-256
`8102440e235c80d6a48ea3045b767dde60287626b80597763815f4481cbdb287`.
The run passed the 20 control-plane native suffix and the worker retained and
STEP 13-04 bodies through named-sink route cleanup. The first STEP 13-05
leg-B audit then correctly rejected this interface inventory:

```text
/sys/class/net/dummy0 /sys/class/net/lo
```

The route-localnet and exact gateway readbacks had passed. The only-loopback
oracle remains the accepted no-uplink contract and is not relaxed.

The interface is fixture-owned, not product/test-body residue. GuestKit stages
`dummy.ko` in every image. The scripted guest's PID 1 iterates the staged
optional modules and calls `load_module(DUMMY_MODULE_GUEST_PATH)` before it
accepts the first host command. `load_module` calls `finit_module` with an
empty parameter string. Linux 7.0 `drivers/net/dummy.c` defines
`static int numdummies = 1`, declares
`module_param(numdummies, int, 0)`, and creates that many `dummy%d` devices
during module initialization. Permission `0` intentionally exposes no
`/sys/module/dummy/parameters/numdummies` runtime file, so the pinned source is
the authority for the empty-load default while the first guest observation
proves its one-device effect.

Both cgroup guests that stage STEP 13-04 now perform one private, fail-closed
cleanup immediately after their existing module-load command and before any
selected body. Cleanup requires all of these together:

- the dummy module exists and its permission-zero parameter file is absent;
- the complete interface set is exactly `dummy0` plus `lo`;
- `ip -d link` identifies `dummy0` as dummy-kind;
- its operstate is `down`, it has no master, and both IPv4/IPv6 address sets
  and route sets are empty.

Only after that provenance is established does the fixture delete `dummy0`.
It then requires exactly `lo` and requires the dummy module to remain loaded.
Any preexisting extra interface, configured `dummy0`, different name/kind,
address, route, master, state or module identity fails before deletion. This
does not delete an unknown or live interface and does not change the authored
leg-B audit.

The existing STEP 13-04 support remains unchanged: later it creates the
explicitly named `ovd-gk-route` dummy, assigns only `192.0.2.1/32`, installs
the bounded metric-42760 default for the same two host-output bodies, and
removes that named link and route before the remote/no-default and STEP 13-05
bodies. Keeping the module loaded preserves this path.

Canonical bounded GuestKit proof is retained at
`spike-scratch/netns-density-295-dataplane-offload/increment-j`. Durable Metal
`bec78c8640bd312348e34623`, exit zero, recorded at the first command:

```text
module=loaded runtime_numdummies_file=absent
interfaces=/sys/class/net/dummy0 /sys/class/net/lo
```

It then retained the full dummy link identity, down/no-master state and empty
IPv4/IPv6 addresses/routes before deletion; afterwards the module remained
loaded and only `lo` remained. The proof created and cleaned the exact
STEP 13-04 named sink, including its destination/source route readback, and
again observed only `lo`. Both complete STEP 13-05 audits passed around the
unchanged existing body
`vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c`.
The body reported one pass, zero failed/ignored, with client source
`127.0.0.1`, fwmark 1 and the guest-prefix leg-B destination. The receipt
ended `DUMMY_AUTO_DEVICE_CLEANUP_LEGB_PASS`; canonical raw SHA-256 is
`8ebdb62d5b18396bda26dd67f66b5715b62c5ef9e3625009c8ba122dbd16da7c`.

This corrects only fixture-owned automatic-device preparation. It adds no
tool, module, route, test body, scenario, selector, group, lane, executor, API
or production behavior, and it does not substitute for the complete 98-body
gate.

## Existing nft oracle correction and authority

The crafter's private output-rule correction is required by ADR-0172's exact
Decision: output refusal is “TCP reset, ICMP port-unreachable otherwise.” A
single plain reject preserved UDP `EPERM` but did not pin TCP reset; a single
TCP-reset rule would remove the required UDP outcome. The current private
program therefore has this exact ordered output identity:

1. leg-S exemption;
2. registered destination mark/accept;
3. guest-prefix, non-gateway, TCP match followed by nft reject type
   `NFT_REJECT_TCP_RST`;
4. guest-prefix, non-gateway plain IPv4 reject with ICMP port-unreachable.

The TCP rule terminates evaluation with a reset/drop verdict; the following
plain rule is therefore the non-TCP fallback. Both rules use the same constant
`GUEST_NODE_PREFIX` match and exact gateway inequality. They remain after the
registered-destination rule, so a diverted tuple reaches leg C first. The
prerouting drop and gateway exclusion are unchanged.

The netlink owner's normalized identity now expects four output rules, and the
existing private rollback fixture names the same four rules in the same order.
This is fixture fallout of the accepted ADR semantic set, not a new public
surface or scenario. The four existing
`vsock_constant_guest_prefix_rules` bodies are unchanged and retain TCP
`ECONNREFUSED`, UDP `EPERM`, remote drop and registered leg-C positive
oracles. The three `vsock_legb_dial` bodies are also unchanged apart from the
crafter-owned pending-marker removals. This DISTILL audit adopts the private
four-rule fixture expectation as the exact representation of ADR-0172 and
does not authorize any further rule or weaker outcome.

## Confinement and physical-host boundary

The support delta does not change GuestKit's established confinement. Cloud
Hypervisor still runs as uid/gid 4200, receives only the actual KVM and
vhost-vsock device-owner supplementary groups, traverses the root-owned
fixture parents through gid 4200 mode `0710`, and retains the existing
per-allocation ownership, Landlock grant, seccomp filter, `no_new_privs`,
device modes, runtime lifetime and RAII cleanup. No physical-host `sysctl`,
`nft`, BPF load/attach/pin, XDP attach, `ip route`, `ip rule`, cgroup mount or
network namespace operation is issued by this runner extension. Physical Metal
reads tools, module files, the prepared BPF artifact and Python runtime bytes;
only the confined guest executes them.

## Source verification

| Check | Result |
| --- | --- |
| Runner/support compile | PASS — durable Metal source package `8a6a73dc0b9bdc2ff03eea1c`, current `overdrive-fwd-guest-suite` check and no-deps Clippy plus bounded dummy-automatic-device harness formatting/Clippy, exit zero; control-plane prepared binary remains the approved source; unchanged xtask remains `dade15f1bf48082997381fec` |
| Exact roadmap package compile | PASS — durable Metal run `64bdd16ffeb8608976471ce3`, all targets with `integration-tests` for control-plane, dataplane and netlink; compile-only preflight, not the package test gate |
| Ordinary-union fixture reproduction | PASS as classification evidence — `fa224a04f75a914094235928` / nextest `8e39b322-126e-4187-937f-6553983cd5a2`: only the two selected startup bodies failed, both at typed cgroup-bootstrap ENOENT before the designed assertion; dataplane omission prevents treating it as the final package gate |
| Exact active-name resolution | PASS — durable Metal run `bd6cbed044ce86181d5ab970`; control-plane list exposes both exact cells and worker list exposes all three, with zero ignored markers reported by source audit |
| Approved three-package list inventory | PASS — SHA `4bd78f192e3d017588616b041fbe25d678198adb407b02714f44876666d192b9`, Metal `19999a70f5afa15060a6d75c` exit zero: control-plane 1,126, dataplane 179, netlink 50 |
| Dataplane integration partition | PASS — 61 active = 7 retained relay + 18 ordinary/build/parser + exact 36 routed-native; runner array matches all 36 identities byte-for-byte with zero duplicate |
| First actual normal run | CLASSIFIED — Metal `3fb5a2f1e2bc7a8f2fccd58e`, nextest `d8df41da-9320-4b67-b7cf-354cb4eef2d3`: 1,247 run, 1,240 passed, 7 failed, 108 skipped; exact 23 native + 2 ordinary fixture defects |
| Ordinary fixture corrections | PASS — unprivileged focused Metal `c72471e83359a3f4b7d38b01`, exact two selected/two passed; existing network driven port plus corrected file+directory read-only model, no production change |
| All-binary-kind root/CAP guard scan | PASS in source — runtime 18 bailouts plus 5 genuine native failures map to exact additional 20 control-plane + 3 dataplane-lib identities; remaining guards are already routed or protect non-native optional observations |
| `cgroup-default` accounting | PASS in source — retained 55 + 18 + 5 = 78 first, exact 20 control-plane native second, 98 complete |
| Selectable groups and guest boots | PASS — unchanged eight group names, three `cgroup-default` guests and slots 14/15/6 |
| `mtls-relay` accounting | PASS in source — original 7 then integration-native 36 retain the 43 boundary; exact library-native 3 follow; 46 complete; runtime skip diagnostics fail required invocations |
| Tool/runtime audit | PASS — complete transitive source audit covers selected 46+98 including nested helpers with no further executable omission; follow-up confirms exact `setsid -> sleep` chain, existing `/bin/sleep`, ordinary resolver bind target and no `/bin/sh`/`/bin/true` addition |
| tcpdump runtime identity | PASS — no-account diagnostic `9d95728658b949ca36438ff0` retained exact `Couldn't find user 'tcpdump'`; guest-only account diagnostic and unchanged exact native body pass in `ed145fdfdd75dca1dbfe8c7b`, with default privilege drop and no `-Z root` |
| Kernel-module audit | PASS — prior read-only Metal `afb0538bee7e6b0e28a2f6df` confirms the retained closure; read-only Metal `9f6fc1d6358b26463f1062d0` confirms pinned-kernel `nft_nat.ko.zst`, alias `nft-expr-nat`, dependencies `nf_tables,nf_conntrack,nf_nat`, and `CONFIG_NFT_NAT=m` |
| Selected nft-expression audit | PASS — read-only Metal `f188bef22d6c300cdecd73eb` proves selected native source uses exactly TPROXY/DNAT/masquerade, no nft redirect/SNAT, dependency-first NAT module order and unchanged 7/36/43/3/46 gates; runner SHA `871873d92dd79750819b25235671c20a6e9a189f9c0166c2d2df66de19ef2cf5` |
| BPF artifact boundary | PASS — exact workspace `target/bpf/overdrive_bpf.o`, produced by mandatory `guest-kit-prepare`; read-only Metal `db410cb6ebd04a1ac7b54879` records 10,744 bytes and SHA `5aec54ee315fbf451b313b1e1d1faf0de9ff6fdb89b262357b36ae4f76d19e3a`; bounded receipt records loaded program tags |
| STEP 13-04 temporary route ordering | PASS — setup/cleanup remains around only its two host-output cells; STEP 13-05 executes later |
| V-LEGB-MARK environment audit | PASS in source — fail-closed route-localnet, GW-LOCAL, route, interface and policy-state commands surround every existing leg-B cell |
| ADR-0172 private fixture audit | PASS — exact TCP-reset then plain non-TCP fallback, unchanged gateway exclusion/prerouting drop and unchanged native oracles |
| Formatting and Clippy | PASS — current root and increment-j source in durable Metal `8a6a73dc0b9bdc2ff03eea1c`, raw SHA `594319921ab964e170384c145065b66edbdcb0ec9772a3ebc15ef3ae2d0d9c41`; unchanged xtask retained from `dade15f1bf48082997381fec` |
| Guest-kit staging | PASS — current `guest-kit-prepare` completed inside `6c6a56320ecafbfd509a3609`; retained staging contract units remain 2/2 from `3251ba18a4ece881cf4200bb` |
| Bounded exact S-05-C confirmation | PASS — Metal `6c6a56320ecafbfd509a3609`, raw SHA `99c07ea3b978d90025f0fc4fb379c6dc3b8ba8570c579a86aa2b4e2791545c82`: four TX-off readbacks, expected topology/listeners/BPF identities, all three unchanged traffic markers, one passed/zero failed/ignored, exit zero |
| Bounded exact UDP-capture confirmation | PASS — Metal `ed145fdfdd75dca1dbfe8c7b`, raw SHA `9bb8dc6b24eba7f3d61c2315e810e4c3e1fe8765515e514c290bab390402aabf`: readiness banner, three query witnesses/replies/VIP sources, one passed/zero failed/ignored, exit zero |
| Bounded exact adoption confirmation | PASS — Metal `73e45a4bc5b719df49b62f80`, raw SHA `202242ae3ddbb958cc9a612662adbe9b552c892c7850bea2df4c7ebd3a475d53`: unchanged survivor namespace/cgroup, slot adoption/collision and orphan-GC assertions; one passed/zero failed/ignored, no skip, exit zero |
| DNS owner/sequence diagnosis | PASS — Metal `a1a5715691ae238e0a7032a7`, raw SHA `d82c5a2ad66c928e9b8edc59ec3082073c18d9a975fe49f3db720a5e0394e209`: fresh and two-adoption prefix both had zero preexisting UDP/53 owners; body-owned wildcard holder flags/inode/PID/fd/netns/command and absent gateway route retained |
| Bounded qualified DNS confirmation | PASS — Metal `61d8273016ed5daa3b3cf1cb`, raw SHA `c9e09ba097cadab006ba7627a4869220d1022f11427be38c39a80d686ff58edc`: fresh and prefix positive fallback bodies bind distinct `127.0.0.53:53` holder and `100.95.0.1:53` production socket; six exact passes, zero skips, exit zero |
| Old negative skip-hole reproduction | PASS as RED evidence — Metal `15c30cfa1bb75e34e45f9637`, raw SHA `f15a1b4460260a0cdaac04d75d0361804a57a0857d0e93e08b6757aaa9f54ab6`: uppercase `SKIP:`, libtest one-pass summary, typed assertion not reached |
| Fixed negative typed assertion | PASS — Metal `a9a3ea8eb54fc1ee4697ed0d`, raw SHA `de3b767616128c2285e2906260e203a805f52c965c1cfe2476f8d6e229368cf5`: retained old raw rejected by new detector semantics; exact case reaches typed `Bind`/`AddrInUse`, one pass, zero skip |
| Gateway ownership branches | PASS — Metal `678b2e2aba65af9927e27fe0`, raw SHA `274e4541d3e9bab3a99106709995f6ff17f8d47237ab9c1a59538fca904f2033`: fresh created/removed and identical borrowed/preserved branches, exact route readbacks, six unchanged DNS/adoption passes, zero skips |
| Post-iteration-11 cgroup replay | CLASSIFIED — Metal `4575d307849bb67617734275`, raw SHA `4daf2d06b59b4be50222c364d93e8f56f16fd575bd4107f1259b7b91f691af30`: 20 control-plane suffix bodies and worker retained/STEP 13-04 bodies passed; valid GW-LOCAL readback with one trailing `iproute2` separator was rejected before the leg-B body |
| Bounded strict LEGB audit confirmation | PASS — Metal `6eeb8cde64128fd1c0daa173`, raw SHA `cb0c1b578b3d869d3965d474bbd8fc36070356808a8d7a5e7bbceee090dce63e`: raw and one-space-normalized route retained before/after, full audit complement clean, unchanged exact body one pass/zero failed/ignored, exit zero |
| Post-iteration-12 cgroup replay | CLASSIFIED — Metal `ff82cf33b0ddeb9b2b00fc50`, raw SHA `8102440e235c80d6a48ea3045b767dde60287626b80597763815f4481cbdb287`: prior families and STEP 13-04 cleanup passed; strict no-uplink audit exposed the fixture-owned automatic `dummy0` before the leg-B body |
| Bounded dummy automatic-device confirmation | PASS — Metal `bec78c8640bd312348e34623`, raw SHA `8ebdb62d5b18396bda26dd67f66b5715b62c5ef9e3625009c8ba122dbd16da7c`: boot provenance, exact unconfigured link state, bounded deletion, named-sink create/cleanup, full LEGB audits and unchanged exact body all passed, exit zero |
| Actual normal three-package clean gate | NOT RUN — first run classified new native/fixture gaps; clean rerun follows iteration-13 source approval and both passing native receipts |
| Native 46-body `mtls-relay` replay | PASS — Metal `8066a14e0a5afc0f0c4919fd`, raw SHA `abd817092d3554facc960601d8fcefb4a052bd49eaf03f1a77204108220bd494`: one exact 7-pass baseline summary first, 39 exact one-pass suffix summaries (36 integration + 3 library), zero failed/ignored/runtime-skipped, exit zero |
| Native 98-body `cgroup-default` replay | INCOMPLETE / CLASSIFIED — iteration 12 approved strict route normalization; actual retry `ff82cf33b0ddeb9b2b00fc50` then exposed the staged dummy module's automatic unconfigured `dummy0` at the unchanged only-loopback audit after prior families passed; bounded provenance/cleanup/body proof passes, but complete 98 retry is NOT RUN pending iteration-13 source approval |
| Mutation testing | NOT RUN — reserved for the final DELIVER-wave gate |
| Commit | NOT CREATED |

## Required next evidence

A same-reviewer iteration-13 source review must approve the sixth cgroup
causal receipt, exact automatic-device provenance/cleanup and bounded
named-sink plus unchanged leg-B proof before the complete 98-body replay or
any broader gate runs. Iteration 12 already approved exact one-separator
normalization; iteration 11 exact route-source reconciliation, private
created/borrowed bookkeeping and branch-specific cleanup; iteration 10 the
negative control and shared skip detector; iteration 9 the positive DNS
stimulus and gateway shape; iteration 8 `setsid`/resolver; iteration 7 tcpdump
account closure; iteration 6 TX offload; iteration 5 `nft_nat`; and iterations
1-4 the prior fixture corrections, exact manifests, staging and complete
ledger. Those identities and counts are unchanged. The list phase is already
complete: use only the
retained SHA
`4bd78f192e3d017588616b041fbe25d678198adb407b02714f44876666d192b9`
receipt from `19999a70f5afa15060a6d75c`; recollect it only if source identities
change, in which case every native manifest returns to DISTILL.

The complete dataplane native gate is already retained and must not be rerun:

```text
Metal 8066a14e0a5afc0f0c4919fd
raw SHA abd817092d3554facc960601d8fcefb4a052bd49eaf03f1a77204108220bd494
```

After iteration-13 source approval, prepare once and collect only the pending
`cgroup-default` native gate:

```text
cargo xtask metal run -- cargo xtask guest-kit-prepare
cargo xtask metal run -- env OVERDRIVE_FWD_GUEST_GROUP=cgroup-default \
  cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite \
    --features integration-tests
```

The retained `mtls-relay` receipt already shows exactly seven baseline and 36
integration-native passes, 43 retained, then three library-native passes and
46 complete with zero failed/ignored/runtime-skipped and exit zero. The
pending `cgroup-default` receipt must first retain exactly 55 baseline + 18
STEP 13-04 + 5 STEP 13-05 = 78, then show 20
additional control-plane native passes and 98 complete; zero
failed/ignored/runtime-skipped, exit zero, with route-localnet and
qualified-node read-back.

Only those passing native receipts authorize the normal selector to exclude
the exact 76 control-plane and 46 dataplane identities in the complete
native-covered ledger: prior 99 plus the deduplicated additional 23, 122
total. The selector must include every remaining ordinary userspace/Sim test
from all three packages; its retained artifact must state the list SHA, all
122 exact native-covered exclusions, any separately approved non-native
exclusion with its independent authority, and the resulting selected count.
An unaccounted exclusion, a still-selected runtime privilege bailout or an
identity-count mismatch fails before execution.

Then run the actual normal gate as the unprivileged Metal login user so a
mistaken selector cannot mutate the physical kernel:

```text
cargo xtask metal run --no-sudo -- bash -lc '
  normal_filter=$(cat .context/netns-density-295/13-05-normal-three-package.filter-expr)
  cargo nextest run \
    -p overdrive-control-plane \
    -p overdrive-dataplane \
    -p overdrive-netlink \
    --features integration-tests \
    --no-fail-fast -E "$normal_filter" --no-capture
'
```

The retained normal receipt must name all three packages, the exact selector
and selection-inventory digest. Against the unchanged 1,355-identity list and
the complete 122-native ledger, the expected nextest accounting is 1,224 run,
1,224 passed, zero failed and 131 skipped/excluded, with process exit zero.
A package absent from the selected inventory, a total that does not reconcile,
an unaccounted exclusion, any runtime root/CAP bailout counted as a pass, or
nonzero exit fails the gate. Compilation run `64bdd16ffeb8608976471ce3`,
causal run `fa224a04f75a914094235928`, the failed normal-classification run
`3fb5a2f1e2bc7a8f2fccd58e`, failed native-classification runs
`832e3a551f09ac4d4fd08724`, `fe0c4da54d92be8295bd350f` and
`b823070869ab5ea5ec879e01`, plus bounded exact passes
`6c6a56320ecafbfd509a3609` and `ed145fdfdd75dca1dbfe8c7b`, and cgroup failure
`3e3b3463786963e459aadc2a` plus bounded pass
`73e45a4bc5b719df49b62f80`, DNS failure `3421ff034fb99c41ea416500`,
diagnosis `a1a5715691ae238e0a7032a7` and bounded pass
`61d8273016ed5daa3b3cf1cb`, old negative skip
`15c30cfa1bb75e34e45f9637` and fixed typed assertion
`a9a3ea8eb54fc1ee4697ed0d`, route-bookkeeping failure
`46a9510e9f4754ac06a947dc` and dual-branch pass
`678b2e2aba65af9927e27fe0`, strict-route-rendering failure
`4575d307849bb67617734275` and bounded leg-B audit pass
`6eeb8cde64128fd1c0daa173`, automatic-dummy failure
`ff82cf33b0ddeb9b2b00fc50` and bounded provenance/cleanup pass
`bec78c8640bd312348e34623`, do not satisfy the pending complete 98-body or
clean normal receipt. This source handoff claims no passing 98-body receipt
and none of the pending clean normal-run results in advance; the complete
46-body receipt is retained separately above.
