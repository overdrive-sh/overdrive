# ADR-0332 — The first guest family drives the production guest flow owner through its vsock wire seam

## Status

**Accepted (2026-11-19).** Approved by the user as PD-8, option (a) with the one metric `forwarded_round_trip_ns`. Realises user ruling O1 of 2026-11-19 (the first guest family measures the guest flow owner's forwarding path in `overdrive-init` over `ovd_fwd`). DDD-8 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md).

## Context

`overdrive-init` is a `binary`-class crate with no library target; `GuestFlowOwner`, `GuestOwnerIo` and `KernelGuestOwnerIo` are `pub(crate)` (`crates/overdrive-init/src/guest_flow/mod.rs`). A benchmark crate cannot call them. Its in-guest cells are libtest cells built in the dev profile (`IN_GUEST_LANES`, `cargo test --no-run`), so debug assertions are on, which the methodology refuses for a measured binary. The production seam of the owner is the wire: a vsock stream to `GUEST_INBOUND_PORT` carrying a `TcpAccept` request, answered by an owner-initiated connection to the application at `workload_addr:listen_port`, with the guest forwarder pairing the two sockets. An in-guest cell already drives exactly this over vsock loopback against the production `overdrive-init` as PID 1 (`guest_flow::real_kernel::the_accept_path_matches_the_sim_per_connection`).

## Decision

The family measures the production `overdrive-init` guest flow owner and the `ovd_fwd` forwarder through that wire seam, from an in-guest program that plays both the host's flow peer and the application, on the guest's `CLOCK_MONOTONIC`. No production code changes and no test-only branch is added to `overdrive-init`. The program is a subcommand of the guest kit's release-built static `overdrive-guest-peer`, which `cargo xtask guest-kit-prepare` builds (`xtask/src/guest_kit_prepare.rs:615`) and every kit image installs at `/sbin/overdrive-guest-peer` (`crates/overdrive-testing/src/guest_kit.rs:563`): the family needs no prepare step and no staged file of its own. The family `guest-flow-forwarding` has one arm (`forwarding`) and one series metric, `forwarded_round_trip_ns`: after the owner has paired one flow, a 64-byte request written on the vsock side to its echoed reply read back on the vsock side, closed loop. The clock is a declared guest source, `guest_monotonic`. The guest runs the production init as PID 1 with the forwarder module and the vsock loopback module loaded.

## Alternatives

- **Split `overdrive-init` into a library and a binary and call `GuestFlowOwner` from the benchmark**: publishes the owner's internals as cross-crate API, and runs a second owner beside PID 1's (the in-guest owner lane needs a scripted PID 1 to avoid contending for the inbound port), so the measured owner is not the one that runs in production. Rejected.
- **A cell inside `overdrive-init`'s own test binary**: dev-profile build (debug assertions on), tied to libtest, and the benchmark declaration would live in a binary crate. Rejected.
- **A new in-guest binary crate or a binary in `overdrive-bench-families`**: a binary in the families package builds the package's library and so the control-plane, worker and dataplane closure (including a `build.rs` that needs the BPF object) for a program that uses none of it; a new crate duplicates what the guest peer already is. Rejected.
- **Drive the host leg over vhost-vsock and stamp round trips on the host clock**: measures the host's vhost path and the virtio kicks with the owner and cannot attribute cost to the guest component. Rejected as this family; it is a different, composed benchmark ([#370](https://github.com/overdrive-sh/overdrive/issues/370)).
- **A second metric, `accept_ns` (request written to the application's `accept()` returning)**: measures the owner's accept work, at the price of an accept loop in the program, a second series block and a second failure path. Not added (PD-8).

## Consequences

The measured path is the guest side only, and after the pair is attached it is the `ovd_fwd` forwarder under an owner-established pair; the owner's own accept-and-pair work is setup, not measured (PD-8 states the consequence). The vsock leg is loopback inside the guest (a connection to `VMADDR_CID_LOCAL` uses the local transport, `net/vmw_vsock/af_vsock.c:513-521,580-583 @ v7.0`, which delivers to the listener through `virtio_transport_recv_pkt`, `net/vmw_vsock/vsock_loopback.c:136-144 @ v7.0`, the function the guest's virtio receive path and the vhost host path also call, `net/vmw_vsock/virtio_transport.c:680` and `drivers/vhost/vsock.c:555 @ v7.0`), so the vhost-vsock transport, the host flow owner and the guest-originated (`TcpConnect`) and datagram flows are not measured, and nor is the owner's accept-and-pair work. These scope limitations are tracked by [#370](https://github.com/overdrive-sh/overdrive/issues/370) ("Benchmark the host-driven vsock leg and the other flow kinds of the guest flow owner"). The program's two timestamps of a round trip use one clock in one process, so no guest duration meets a host instant.

## Links

- [feature delta, DDD-8, Interface contracts](../../feature/guest-benchmark-substrate/feature-delta.md)
