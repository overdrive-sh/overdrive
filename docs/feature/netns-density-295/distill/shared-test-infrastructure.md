# DISTILL — shared test infrastructure (netns-density-295, EVMOD)

Test support DISTILL owns (fixtures, test-local port implementations, sim
scripting). Production surface used here is only what the feature-delta
pins.

## 1. Lanes and placement

| Lane | Where | Gate |
|---|---|---|
| Default (pure, sim) | `overdrive-core/tests/{acceptance,schema_evolution}`, `overdrive-sim/tests/acceptance`, source-local `#[cfg(test)]` modules (control-plane `guest_network/**`, `*_acceptance.rs`; `overdrive-init` `guest_flow/**`, `transport/**`), `overdrive-worker` / `overdrive-reconcilers` / `overdrive-store-local` acceptance | none |
| Integration (real kernel, sockets, fs; Lima) | `crates/*/tests/integration/`, the `overdrive-fwd` module suite, the source-local Quint drivers and composed rigs | `integration-tests` |
| In-guest / native metal | `overdrive-host` `vsock_vmm_device` and `vsock_beacon_accept_path` (R5-59 (a) beacon seam); control-plane source-local `guest_network/flow_io/accept_path_tier3.rs` (R5-59 (a) flow seam); `overdrive-fwd` `guest_boundary.rs`; `overdrive-init` `tests/integration/in_guest_lane.rs` (runs `transport::real_kernel` / `guest_flow::real_kernel` inside a guest); `overdrive-cli` `vsock_walking_skeleton` (14-01), `vsock_fault_matrix` (14-02), `vsock_density` (14-05) and the guest stack | `integration-tests,kvm-tests` via `cargo xtask metal run --` |

Every Quint driver is source-local in its owner's crate behind
`#[cfg(all(test, feature = "integration-tests"))]`, with `max_samples = 1`
on every `#[quint_test]` and one `#[quint_run]`, `Config::nondet` the single
element `"<main>::<module>::conf_step"`, and the state projected by
qualified names. `.config/nextest.toml`: the whole `overdrive-fwd`
integration binary is in `host-kernel-shared` with ten slow periods; the
control-plane library test binary and the dataplane / worker /
control-plane / CLI / host integration binaries stay whole-binary members;
the `overdrive-init` integration binary (the in-guest lane) joins it with
five slow periods; the CLI `vsock_fault_matrix::` cells get five and the
`vsock_density::` cells twenty. The stale
redirect-era overrides (`vsock_drain::sent_is_exact_past_32_bit_wrap`,
`vsock_prefix_landing_conformance`, `name_resolve_enforce_consistency`) were
removed.

## 2. Sim adapters and seam sims

| Sim | Crate / visibility | Role |
|---|---|---|
| `SimFlowForwarder`, `SimForwarderDevice`, `SimSocket` (+ `SimShutdown`, `SimVsockState`, `SimFarEnd`, `SimLocalRead`, `SimContextState`, `SimSocketKind`, `SimSocketSetup`) | `overdrive-sim::adapters::flow_forwarder` (re-exported at `overdrive_sim::adapters::*`) | `FlowForwarder<SimSocket>`; the named arbiter of the forwarder's kernel facts (QPROJ-8): context table, queues, per-direction state, classification, end records. Stepping contract (rustdoc): one `run(id, source)` = one `fwd_pair` hop (one stream byte / one datagram frame item, or the classified end once the queue is empty, never both); `peer_resume_reading` delivers everything queued in one call; UDP `local_read` one datagram per call; `received_shutdown` reads back `peer_shutdown` (`vs(s).ps`). Bodies are scaffolds (step 4). Tier-3 pair: the `overdrive-fwd` suite |
| `SimGuestFlowIo` | `overdrive-control-plane::guest_network::flow_io::sim` (`pub(crate)`) | DG-3 seam: held effects, `settled()`, QPROJ-7 arbiter (`socket_observation`, `closed_by_owner`); forwarder half over `SimFlowForwarder`; additions `vsock_half` (D-5: the host vsock half kept after the owner closed it, until the seam drops), `peer_of_opt`, `latest_peer_socket`, `caused()` |
| `SimGuestOwnerIo`, `SimGuestClock` | `overdrive-init::guest_flow::sim` (dev-dependency `overdrive-sim`) | DG-5 seam; guest socket facts; forwarder half over `SimFlowForwarder` |
| `SimBeaconIo` | `overdrive-sim` | DG-4 beacon listener; `beacon_socket_observation` arbiter |
| `SimVmm` | `overdrive-sim` | `vsock_device_holders` arbiter; `guest_reset`, `exit_process`, `kill`, `live_pids` |
| `SimGuestStreamDial` | `overdrive-sim::adapters::guest_stream_dial` | scripted `GuestStreamDial` for the probe runner and leg-S consumers (every `SimMtlsEnforcement::new` / `HostMtlsEnforcement::new` in tests outside `overdrive-dataplane` passes one as `guest_dial`, LEGS-DIAL; the dataplane fixtures pass a local `NotServing` dial, `tests/integration/helpers/guest_dial.rs`) |
| `SimSharedGuestNetworkOwner` | `overdrive-sim` | records `provision` as `PairAttach`, `teardown` as `PairAbort`, `converge_shared` as `ForwarderOpen` |
| `SimMtlsIntercept` | `overdrive-sim` | shared-table arbiter (`shared_table_state`, `fail_next_table_transaction`, `script_crash_retains_table`) |
| `SimObservationStore` | `overdrive-sim` | probe-row arbiter for `quint_probe_count` |
| `SimVmmHost`, `RecordingVmm`, `compose()` | `overdrive-control-plane::vm_reclamation_boot` (test) | boot reclamation and the `cid_lease` driver |

## 3. Composed rigs and fixtures

- `shared_network_test_ports::composed::ComposedNode` — boots
  `run_server_with_obs_and_drivers` with the production owner over
  `SimGuestFlowIo`, `SimMtlsIntercept`, `SimClock`; the supervisor is stepped
  at the DNS port's audit (`TestGuestDnsFactory::hold_audits` /
  `release_audit`, `upstream-issues.md` T-4); repairs park at the seam's
  `BindListener` / `OpenForwarder`.
- `owner_acceptance::support::Composition` — the host owner and its mTLS
  worker built by the crate-private `compose_guest_network` (D-UP-16) over
  the seam and the sim mTLS ports, as `run_server` builds them;
  `intercept_live` uses the worker's public `start_shared_owner` +
  `start_alloc`. Its `SimMtlsEnforcement` is built with the composition's
  one `OwnerDial` as its `guest_dial` (LEGS-DIAL), so leg S reaches the
  owner only through the binding `compose_guest_network` makes
  (`legs_dial.rs`). `shared_network_test_ports::composed` boots as
  `run_server` does: phase 1 (`probe_guest_flow_startup`) over the seam,
  then `boot_server` with `GuestNetworkSource::Composed` (D-8; T-8 closed).
- The guest-kit microVM fixture (`overdrive-testing`, test support only;
  user ruling T-1, 2026-10-18): `guest_kit` (the metal host's stock 7.0
  kernel from `VmFixture`, `ovd_fwd.ko` built for that release, static
  `overdrive-init` and the static scripted peer — the module, the peer and
  the in-guest test binaries are built and staged at contract paths by the
  lane setup `cargo xtask guest-kit-prepare` (`xtask/src/guest_kit_prepare.rs`),
  and the fixture only checks they exist (`GuestKitError::LaneNotPrepared`); ext4 images with either as PID 1, the vsock modules, optionally the
  forwarder and loopback modules; the CH fork launched through the
  production `overdrive-host` `CloudHypervisorVmm` (`Vmm::create` over a
  `VmConfig`, U-4 (a): the one renderer of the launch argument,
  `--vsock cid=<cid>,backend=vhost-kernel`, under the production confinement;
  `overdrive-testing` depends on `overdrive-host`); `GuestKitVm` kill / wait /
  console); a CID-band registry of per-file bands from offset `0x2000`,
  above the production band, with a guard test; `guest_peer` +
  `bin/guest_peer.rs` (the scripted guest vsock peer: a typed line protocol
  over raw vsock and TCP sockets, deterministic stream pump and drain, reset,
  the guest's own `KernelFlowForwarder` context, spawn / join, exec, kernel
  crash, poweroff; as a non-PID-1 program it is the serve + deploy lanes'
  guest application: `round-trip`, `hold`, `flood`, `udp-churn`, `idle`,
  `run-with-modules`); `guest_app` (`RemotePeer`: TCP and UDP servers in their
  own namespace on a leased test `/24`); `guest_lane` (the T-2 lane: plays the
  node beacon, holds the guest's control session, runs one in-guest cell with
  `--exact`, passing only on "1 passed"); `vsock` (raw `AF_VSOCK` helpers).
  `overdrive-cli/tests/integration/vsock_support.rs` is the serve + deploy
  lane support (`Lane::boot` over `spawn_vm_server_mtls_composed`).
- `overdrive-fwd/tests/integration/support.rs` — `open_context`,
  `tcp_pair`, `vsock_pair`, `udp_pair`, `attach_stream`, `reset_close`,
  `wait_end_of`, `close_watched`.
- `reconciler_runtime::probe_count_fixture` — the production
  `ReconcilerRuntime`, `ProbeRunner`, shim and `SimDriver`.

## 4. Fixture migrations applied in the integration pass

- LEGS-DIAL (round 9): `MtlsInterceptWorker::new` takes its four ports
  everywhere; the enforcement takes the guest dial —
  `HostMtlsEnforcement::new(.., GUEST_NODE_PREFIX, guest_dial)` and
  `SimMtlsEnforcement::new(identity, limits, guest_dial)`. Tests pass
  `SimGuestStreamDial` (or, in `overdrive-dataplane`, a local `NotServing`
  dial); `run_server` passes the boot's `OwnerDial` on the composed path and
  the supplied owner on the supplied-owner path.
- `SharedGuestNetworkOwner::restore` returns
  `Result<ForwardingRestore, QuiescenceRestoreError>` in every test decorator.
- `GuestNetworkOperation::{PairInstall, PairRemove}` → `{PairAttach,
  PairAbort}`; `ForwarderLoad` → `ForwarderOpen`.
- Edition-2024 reserved word: `gen` bindings renamed in the drivers.
- 2026-10-19: every control-plane driver overrides `State::from_spec` with the
  shared ITF decoder (serde's default renders `#bigint` as a string and
  `#map` as a record); runtime sleeps in drivers are created inside
  `block_on`; the real-cgroup worker tests establish the `overdrive.slice`
  delegation themselves (`cgroup_cleanup::delegate_overdrive_slice`, E-2);
  `HostMtlsEnforcement::new(.., GUEST_NODE_PREFIX)` in every test (LEGB-DIAL).

## 5. Lint discipline

`conformance-lint` treats `Option` as a wrapper (an `Option<Owner>` slot holds
the owner, an `Option<u32>` stays bookkeeping; xtask unit test
`an_optional_owner_slot_is_an_owner_and_an_optional_scalar_is_bookkeeping`).
Every projection read of a named arbiter carries
`// conformance-lint: sim-arbiter-ok <QPROJ id, variable, Tier-3 pair>` on
the read's line or the line above. Seed printing in source-local seeded
cells uses a scoped `#[allow(clippy::print_stderr, reason = …)]`.
