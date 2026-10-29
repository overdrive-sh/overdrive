# STEP 13-06 — existing GuestKit native support handoff

## Boundary and authority

This is the bounded DISTILL fixture handoff for the 13 already-authored and
already-activated `vsock_host_internal_enforce` D26 cells. It extends only the
existing `cgroup-default` worker manifest in
`crates/overdrive-testing/bin/fwd_guest_suite.rs`. It does not add or change a
scenario, test body, assertion, pending marker, selectable group, lane, guest
executor, production mechanism, public API, typed error, DES event or commit.

The source boundary remains the production `MtlsInterceptWorker::start_shared_owner`
path used by every unchanged cell. The kernel boundary remains a disposable
Cloud Hypervisor `GuestKit` guest on the qualified Metal launcher. No nft,
network-namespace, route, socket-mark or module effect from this run touched
the physical Metal kernel.

The existing retained counts remain independently fail-closed:

| Boundary | Count | Disposition |
|---|---:|---|
| `cgroup-default` baseline | 55 | Unchanged |
| STEP 13-04 extension | 18 | Unchanged |
| STEP 13-05 extension | 5 | Unchanged |
| Control-plane normal-native extension | 20 | Unchanged |
| Retained `cgroup-default` boundary | 98 | Unchanged and still checked before D26 accounting |
| STEP 13-06 D26 extension | 13 | Added to the existing worker manifest |
| Complete `cgroup-default` boundary | 111 | Exact `98 + 13` |
| Complete `mtls-relay` boundary | 46 | Source and execution path untouched |

The original STEP 13-06 crafter owns F-02's normal package receipt and the
subsequent implementation-review remediation commit. This DISTILL handoff
supplies the missing F-01 selection, staging and native evidence only.

## Exact selected identity

Every entry is an exact libtest identity with expected count one. The first ten
run with the no-NIC guest's ordinary route state. The final three share one
bounded dummy default-route setup and cleanup because their authored first UDP
transmit must reach the output hook.

| # | Exact `overdrive-worker` integration identity | Route support |
|---:|---|---|
| 1 | `integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_loopback_service_is_refused_econnrefused_without_hang` | Ordinary guest state |
| 2 | `integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_host_interface_address_is_refused` | Ordinary guest state |
| 3 | `integration::vsock_host_internal_enforce::a_marked_udp_first_transmit_to_a_local_service_fails_eperm` | Ordinary guest state |
| 4 | `integration::vsock_host_internal_enforce::a_marked_udp_socket_reusing_an_unmarked_flows_source_port_is_still_rejected` | Ordinary guest state |
| 5 | `integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_a_subnet_directed_broadcast_reaches_no_service` | Ordinary guest state; authored connected `/24` route |
| 6 | `integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_link_local_service_is_refused` | Ordinary guest state; authored `/32` route |
| 7 | `integration::vsock_host_internal_enforce::a_marked_tcp_connect_to_a_neighbour_stays_reachable` | Ordinary guest state; authored connected `/24` route |
| 8 | `integration::vsock_host_internal_enforce::a_packet_carrying_the_owner_bit_among_other_bits_is_refused` | Ordinary guest state |
| 9 | `integration::vsock_host_internal_enforce::a_packet_carrying_only_the_leg_s_or_tproxy_mark_reaches_a_local_service` | Ordinary guest state |
| 10 | `integration::vsock_host_internal_enforce::a_marked_connect_storm_across_repeated_table_replacements_reaches_no_local_service` | Ordinary guest state |
| 11 | `integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_the_limited_broadcast_reaches_no_service` | Bounded D26 dummy default route |
| 12 | `integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_the_all_hosts_multicast_group_is_rejected_eperm` | Bounded D26 dummy default route |
| 13 | `integration::vsock_host_internal_enforce::a_marked_udp_transmit_to_an_off_host_address_is_not_rejected` | Bounded D26 dummy default route |

The manifest contains exactly 13 `vsock_host_internal_enforce` identities and
`STEP13_06_NATIVE_BODIES` is exactly 13. `exec_required_cgroup_cells` drives
all of them, so a zero-exit test process must also contain the exact
`1 passed; 0 failed` summary and must not contain a recognized runtime-skip
diagnostic.

## Unchanged scenario and oracle surface

The support change does not edit
`crates/overdrive-worker/tests/integration/vsock_host_internal_enforce.rs`.
The 13 cells retain their existing Contract Shape declarations and all
observable assertions:

- marked loopback, host-interface, link-local, broadcast and multicast paths
  retain the exact `ECONNREFUSED` or first-transmit `EPERM` verdicts and the
  no-host-socket outcome;
- the established UDP tuple remains unable to bypass the D26 rules;
- subnet-directed broadcast, limited broadcast and multicast retain their
  positive or supplementary absence controls;
- the marked neighbour and off-host unicast controls remain reachable;
- the owner mark under additional bits remains denied while the leg-S and
  TPROXY marks remain accepted;
- repeated production table replacements retain zero successful marked
  connects; and
- every cell continues to boot and replace the shared table only through
  `MtlsInterceptWorker::start_shared_owner`.

No assertion was weakened to accommodate the disposable guest.

## Support diff and causal reasoning

### Kernel expression closure

The D26 production table now contains an nft `fib daddr type` expression.
Metal probe `02f81d15e5eedec1514da1c3` on Linux `7.0.0-38-generic` pinned the
loadable-module closure:

| Module | Qualified source path | Declared dependency |
|---|---|---|
| `nft_fib` | `net/netfilter/nft_fib` | `nf_tables` |
| `nft_fib_ipv4` | `net/ipv4/netfilter/nft_fib_ipv4` | `nf_tables,nft_fib` |

`nf_tables` and `nfnetlink` were already staged by the existing worker guest.
The runner now stages only `nft_fib.ko` and `nft_fib_ipv4.ko` when the D26
manifest is non-empty. The unrelated `mtls-relay` guest does not receive these
modules, preserving its exact 46-body closure and semantics.

The existing worker guest already supplies the other selected dependencies:
`ip`, `nft`, `uname`, the `/var/run -> /run` netns compatibility alias, cgroup
v2, bpffs, `veth`, nft reject/TProxy dependencies, the dummy module and the
prepared `overdrive-worker` integration ELF with its shared-library closure.

### Route precondition and complement

Actual run `dfe91c93f25d1e3ff5e96649` proved the limited-broadcast control was
unroutable in the no-NIC guest. Reusing the earlier STEP 13-04 route verbatim
then exposed a second mismatch in `6a47b2c20a5e00268b36ce65`: that fixture
assigned `192.0.2.1` locally, while the unchanged D26 off-host cell uses
`192.0.2.1` as its remote destination. The production D26 rule therefore
correctly classified it as host-local and returned `EPERM`; the test body was
not at fault. Run `c418e87e864e38b407b7706b` then rejected an overlength
fixture interface name before any routed body, preserving fail-closed setup.

The final private support uses interface `ovd-gk-d26`, local source
`198.51.100.1/32` and a metric-42761 default route. Before selected bodies it
requires `192.0.2.1` to be unroutable and absent from the local table. After
setup it reads back:

```text
192.0.2.1 dev ovd-gk-d26 src 198.51.100.1 uid 0
    cache  destination_type=unicast
```

Only the limited-broadcast, multicast and off-host cells run inside this
route lifetime. Cleanup removes the exact default and interface, then proves
`192.0.2.1` is again unroutable and still absent from the local table. The
final receipt contains exactly one setup and one cleanup readback.

## Fail-closed count, skip and cleanup behavior

The existing line-oriented runtime-skip detector is unchanged. For each
selected D26 invocation it trims leading whitespace, examines text after a
libtest `" ... "` prefix when present, and rejects diagnostics beginning with
`[skip]`, `SKIP `, `SKIP:`, `skip:` or `(skip)`. All 13 D26 cells use the
required execution path; there is no root or capability bailout counted as a
pass.

Durable-log audit `3d7be371ebcdaf58df45ac75` over the successful raw receipt
reported:

```text
D26 started=13 ok=0 failed=0 cleanup_scrub_lines=117 route_setup=1 route_cleanup=1 runtime_skip_diagnostics=0
```

`ok=0` is expected for this textual audit because each live test emits
append-only evidence after libtest's `...` marker and libtest prints the final
`ok` on a later line. The runner itself validates every invocation's exact
`test result: ok. 1 passed; 0 failed;` summary before adding its count. The
successful 13-count equality therefore supplies the non-vacuous pass proof.

The 117 cleanup-scrub lines are exactly nine authored scrub observations for
each of 13 D26 cells. Each cell's `EnforceSandbox` removes its veth, namespace,
link-local route, shared nft tables, mark-1 policy rules and table-100 local
route before releasing the CIDR lease. The one route cleanup readback is in
addition to those authored scrubs. `GuestRun::finish` then powers off each
disposable guest and requires its VM exit within the existing bound. The
complete command's terminal exit zero proves all three guest lifecycles and
the final exact 111-body accounting completed.

## Source identity

The successful native command ran against implementation commit
`b51d5f3dc1e35483abe6d6809fb29c3fd38b15e5` plus this uncommitted DISTILL
runner support. Exact SHA-256 identities at the handoff are:

| Source | SHA-256 |
|---|---|
| `crates/overdrive-testing/bin/fwd_guest_suite.rs` | `e5a3d72b42a504def620d26b95b7474038ac13760af1cce0d3e421328bafec80` |
| `crates/overdrive-worker/tests/integration/vsock_host_internal_enforce.rs` | `eb65379af9ff172eda3af666111cdf908ed4dabe7d56d41f4490d25852ff35b8` |
| `crates/overdrive-netlink/src/nft.rs` | `00a1b57ff41a2ebe4b5ede4acb2fb82b6bd3f98dff3a65b759dd888b1ba0f4e0` |
| `crates/overdrive-worker/src/mtls_intercept_port.rs` | `f4a5be044f3ad4fa759d74aba546db02c23d281cb3afc1dd2ab45ad41c2b2444` |

The runner Git-blob identity is
`80ed8434f171f55b4be7e6afa97516ddf6ea4e57`; the unchanged activated D26 test
file Git-blob identity is `965146f85b938d630656552c0e1ea0ac23ef89eb`.

## Commands and durable receipts

### Module closure probe

```text
cargo xtask metal run -- bash -c 'set -eu; uname -r; for module in nft_fib nft_fib_ipv4 nft_reject nft_reject_ipv4 nf_tables nfnetlink; do ...; done'
```

Durable run `02f81d15e5eedec1514da1c3`, exact exit 0.

### Complete existing cgroup group plus D26

```text
cargo xtask metal run -- bash -c 'set -eu; OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build; cargo xtask guest-kit-prepare; OVERDRIVE_FWD_GUEST_GROUP=cgroup-default cargo run -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests'
```

Durable run `1bb49a61e3a35201fdc93597`, exact exit 0, finished
`2026-10-29T12:19:31Z`. Read-only collection used:

```text
cargo xtask metal collect 1bb49a61e3a35201fdc93597
```

Collection exited 0. The retained raw `output.log` is 397,357 bytes with
SHA-256 `197e7578db123a2297ebc180f2b4c35776ebd66debef5d733861ce207556941c`.
The durable `terminal.v1` has SHA-256
`d37197a3749a4e78c73eae6997577018bae95818b75ba2c0329a058822a6df4f`
and records `exit_code=0`, `cause=command_returned`.

### Source checks

```text
cargo xtask metal run -- bash -c 'cargo fmt --all -- --check && cargo check -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests && cargo clippy -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests -- -D warnings'
```

Durable run `fa13f31bb138b5c40cc2a202`, exact exit 0.

## Scope and handoff

The bounded DISTILL source is ready for independent acceptance-design review.
No commit has been created. After that review approves the runner support, the
original STEP 13-06 crafter can persist the runner, this handoff and the
review artifact with its bounded remediation, retain the native run identity,
and produce F-02's separate normal package receipt. Mutation testing remains
unauthorized and was not run.
