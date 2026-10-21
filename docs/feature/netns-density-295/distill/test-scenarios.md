# DISTILL scenario index — netns-density-295 (EVMOD: the in-kernel forwarder)

Code-only DISTILL against the approved EVMOD design (feature-delta
§ *[REF] Interface contracts*, the EVMOD section, Lifecycle Gate Ownership,
Required boundary scenarios, Formal protocol model with its conformance
mains, named runs and pinned accessors, Roadmap impact) and
`specs/quint/{guest-flow-owner,probe-count}/`. Scenarios are specification
only; the executable bodies are the Rust tests named in the partition files.
No `.feature` files (testing.md). `[lang-mode] rust`, `[policy-mode]
inherit`, `[port-mode] n/a-rust` (Mandate 8 maps to native before/after
assertions on port-exposed observations). Reconciliation: one SSOT (the
feature-delta, with the user decisions in
`.context/round-2026-10-13/decisions.md`: the EVMOD approval of 2026-10-17
and the DISTILL rulings of 2026-10-18 — T-1 / T-2 the guest-kit fixture and
lane, RESTART-SSOT (GH #324 fixed in #295), GH #325 out of scope — and of
2026-10-19 — U-2 (GH #326), E-3, D-14 LEGS-DIAL, and after round 10
PORTS-CONFIG, GUEST-PORTS, FWD-PREREQ, NO-OVERRIDE-SEAMS (full scope),
STORES-IN, PROBE-ROOT-ENV, which replace U-1 (b)) — 0 contradictions in the
authored scope. The earlier DISTILL rulings R-D1
(the `ProbeResultRow` V2 bump) and R-D2 (the CID-claim port deleted) stand.

## Partitions

| Partition | File | Owner crates | Scope |
|---|---|---|---|
| P1 | `test-scenarios-p1-core-vocabulary.md` | `overdrive-core`, `overdrive-sim` | the forwarder vocabulary (`ForwardId`, `ForwarderGeneration`, `AbortCause`, `ForwarderError`), wire codec (request with launch token, control open, control messages, datagram frame), READY `launch=`, `KernelErrno`, `GuestStream` / dial vocabulary, the two deploy ingresses (D25, D-UP-12); `SimFlowForwarder` arbiter cells |
| P2 | `test-scenarios-p2-ovd-fwd-module.md` | `overdrive-fwd` (new) | Tier-3 module suite through `KernelFlowForwarder` (probe, attach / watch, stream battery and ends, paused / stall, datagrams, watch, context release, end-record ABI, R5-57 equivalence); UAPI unit cells |
| P3a | `test-scenarios-p3a-host-guest-flow-owner.md` | `overdrive-control-plane::guest_network` | host owner over `SimGuestFlowIo`: admission, flow ends from end records, quota, datagrams, the dial, policy, error taxonomy; drivers `owner_flows`, `fwd_pair`, `flow_quota`, `udp_slots` (host) |
| P3b | `test-scenarios-p3b-lease-quiescence-recovery-boot.md` | `overdrive-control-plane`, `overdrive-sim`, `tests/conformance` | pool and lease, boot reclamation, QHOLD recovery and the `Forwarder` recovery, boot order (G-V1), boot refusals, activation gate; drivers `cid_lease`, `quiescence` |
| P4 | `test-scenarios-p4-guest-init.md` | `overdrive-init` | guest owner over `SimGuestOwnerIo`: session, `TcpAccept`, `TcpConnect`, flow end, UDP slots; transport setup; `main`; drivers `owner_flows`, `fwd_pair`, `udp_slots` (guest) |
| P5 (carried) | `test-scenarios-p5-vmm-device-beacon.md` | `overdrive-host`, `overdrive-sim`, `overdrive-worker` | VMM-opened device (CID-NOCLAIM), RESET-EXIT, beacon intake; driver `flows_beacon` |
| P6 (carried) | `test-scenarios-p6-real-kernel-mtls-firewall.md` | `overdrive-worker`, `overdrive-control-plane` | forwarded-outbound port, leg-B dial, constant guest-prefix rules, host-internal deny set, VIP resolve branch |
| P7 (carried) | `test-scenarios-p7-probe-run-count.md` | `overdrive-core`, `overdrive-store-local`, `overdrive-worker`, `overdrive-reconcilers`, `overdrive-control-plane` | `ProbeResultRow` V2, run-sequence merge, R5-PROBE-COUNT; driver `probe_streak` |
| RESTART-SSOT | `red-classification.md` § *RESTART-SSOT* | `overdrive-reconcilers`, `overdrive-control-plane` | regression obligations 1–5 of § *Restart decision* (step 0, GH #324) |
| Native metal lanes | `shared-test-infrastructure.md` § 3, P2, P3a, P4, P5 | `overdrive-testing` (guest kit), `overdrive-fwd`, `overdrive-control-plane`, `overdrive-host`, `overdrive-init`, `overdrive-cli` | `guest_boundary` (T-1), R5-59 (a) flow and beacon seams, the in-guest lane (T-2), walking skeleton P6-S13 (14-01), native fault matrix P6-S14 (14-02), density and fail-closed P6-S15 (14-05) |

Cross-cutting: `red-classification.md` (gate runs, driver activation,
semantic RED per marker), `shared-test-infrastructure.md` (sims, rigs,
lanes), `upstream-issues.md` (blockers and routing).

## Marker convention

`#[ignore = "pending DELIVER <step>"]` naming the Roadmap impact step that
activates the body (`step N` for the new steps 1–15; `attachment-cut step
(N)` for the DG-2 grouping; the rework names of the earlier steps, e.g.
`(12-01 rework)`). The body is complete; the crafter only removes the marker.

## Conformance driver checklist (testing.md § "Per-driver checklist")

All eleven drivers: (1) gated only by `#[cfg(all(test, feature =
"integration-tests"))]` at their `mod` line, no always-false `cfg`, each
`#[quint_test]` / `#[quint_run]` carries its reasoned pending marker; (2)
the driver struct holds the production owner or its composition; (3) every
`step` action is mapped, hazard-only and other-instance actions return an
error, no `_ => {}` arm; (4)–(5) `State::from_driver` reads the pinned
accessors, and every `Sim*` read is a named arbiter marked
`// conformance-lint: sim-arbiter-ok` with its Tier-3 pair; (6) the spec is
referenced in place, every critical named run has a `#[quint_test(max_samples
= 1)]`, one `#[quint_run]`; (7) the target module is a state machine, no
`quint` CLI. `cargo xtask conformance-lint` is clean.

| Driver | `mod` gate | Driver struct | `switch!` | `from_driver` | first `#[quint_test]` | `#[quint_run]` | Runs |
|---|---|---|---|---|---|---|---|
| CP `guest_network::quint_owner_flows` (`flows_conf`) | `guest_network.rs:75` | `:305` | `:1041` | `:236` | `:1135` | `:1725` | 45 |
| CP `guest_network::quint_fwd_pair` (`pair_conf`) | `guest_network.rs:69` | `:396` | `:1361` | `:310` | `:1482` | `:1955` | 39 |
| CP `guest_network::quint_flow_quota` (`quota_conf`) | `guest_network.rs:67` | `:168` | `:427` | `:145` | `:443` | `:544` | 7 |
| CP `guest_network::quint_udp_slots` (`slots_conf`) | `guest_network.rs:77` | `:134` | `:339` | `:117` | `:367` | `:512` | 12 |
| CP `guest_network::quint_cid_lease` (`lease_conf_ok`, `lease_conf_325`, `lease_conf_three`) | `guest_network.rs:65` | `:672` | `:1606` | `:1768` | `:1826` | `:2130` | 21 + 1 + 1 |
| CP `quint_quiescence` (`quiesce_conf`) | `lib.rs:170` | `:173` | `:415` | `:500` | `:557` | `:680` | 10 |
| CP `reconciler_runtime::quint_probe_count` (`probe_streak_conf`) | `reconciler_runtime.rs:1831` | `:117` | `:215` | `:256` | `:365` | `:484` | 9 |
| worker `vm_driver::quint_flows_beacon` (`flows_beacon_conf`) | `vm_driver.rs:2212` | `:222` | `:538` | `:593` | `:629` | `:726` | 7 |
| init `guest_flow::quint_owner_flows` (`flows_conf`) | `guest_flow/mod.rs:43` | `:229` | `:643` | `:760` | `:795` | `:1393` | 45 |
| init `guest_flow::quint_fwd_pair` (`pair_conf`) | `guest_flow/mod.rs:39` | `:291` | `:782` | `:1333` | `:1372` | `:1860` | 37 |
| init `guest_flow::quint_udp_slots` (`slots_conf`) | `guest_flow/mod.rs:45` | `:205` | `:575` | `:669` | `:736` | `:895` | 12 |

Line numbers are as read on 2026-10-21; the control-plane drivers move with
later edits and are re-read at the next integration pass. The drivers
replay 12 of 12 at the RED scaffolds of `red-classification.md`. `flow_quota` pins all 7 `quota_conf` runs,
including the two M-1 queue-order runs. `fwd_safe` (`safe_runs`, 14 runs in
the Quint record `114403-1792606203.594`, 279 checks) models pair lifetime
only (no module-unload state: the appliance's kernel has `CONFIG_MODULE_UNLOAD`
off) and has no driver by design; the record of where each run's observable
consequence is checked (a `fwd_pair` conformance run, an `overdrive-fwd`
module-suite cell, V-EV-SAFE / V-EV-UNLOAD, or the DELIVER source review of
`sk_write_space`) is the module doc of `overdrive-control-plane/src/guest_network/quint_fwd_pair.rs`
(§ "`fwd_safe` has no driver"); `WatchOverrideEndThenOwnerAbortIsEnoent` is
covered by V-EV-SAFE only. Named runs left unpinned on a conformance main,
each with its reason in the driver: `cid_lease`'s two #325-window guard
runs (model-only), the init `fwd_pair` driver's `HostOwnerDeathIsFinal` and
`RecoveryEndsHeldStreamWhoseWatchEnded` (pinned on the host driver),
`quiescence`'s `ForeignHoldRestoreChangesNothingThenFailStops` (FHOLD option
(b)), `lease_ok`'s `PoolExhaustedRestartCleansThePredecessorThenPlaces`
(pinned on `lease_conf_three`).
`conformance-lint` is clean. Every driver holds its production owner, or
the composition that contains it, in a non-`Option` field (lint ruling
"Owner field", S-7 closed): the init drivers hold `GuestFlowOwner` directly
beside the lifecycle-only `GuestProcess`, the `quiescence` driver holds
`node: ComposedNode`. Every driver decodes the spec state with the shared
ITF decoder through `from_spec`. The `fwd_pair` drivers project `pr(s).out` /
`.cause` / `.errno` / `rec = read` from `last_consumed_end` (D-11) and
`ctx(s)` from the `SimFlowForwarder` context table; the host `fwd_pair`
watch dial goes through the composition's one `OwnerDial` (D-13);
`flow_quota` projects `endq` from `queued_ends` (D-12); `cid_lease` reads
`leased` from the owner and treats `up` / `reclaimed` as stimulus (S-4).
`lease` and `cursor` are not compared from a `Crash` to the next boot's
first step (S-4: `WhileUp::NoPool`, a `PartialEq` that skips the comparison;
quint-connect compares with `!=`, `runner/mod.rs:93`). The `cid_lease` run
`PoolExhaustedRestartCleansThePredecessorThenPlaces` replays on the declared
main `lease_conf_three` (D-18 closed) through two background Service leases on
usable offsets 0 and 1 (checked Admitted at every compared step) and the
relabelling model `m` ↦ usable `m + 2`; that main carries no `#[quint_run]`.
Under NO-OVERRIDE-SEAMS, STORES-IN and PORTS-CONFIG the `quiescence`
driver's `RestartWith` / `Damage` / `RepFail` go through the composed rig
(`boot_server` over `SimGuestFlowIo` and a fresh `BootAdapters` per boot, the
stores reopened from disk, never a retained clone); every composition reads
its vsock ports back from the owner (`flow_ports()`, `BeaconIntake::bound_port`),
never a number; the init `fwd_pair` driver no longer reads `driver.gone`.
The init `fwd_pair` driver gates `ctx(g)`, the owner view
(`pr(g)`, the attach counters, the guest-held `dir(d)` facts) and the guest
socket view on the `VmmExit` stimulus it applied itself (`vmm_exit_applied`,
reset per trace); its spec side always decodes the spec's values and does
not read `vmUp`, so a spec `ctx(g) = gone` before the driver's own `VmmExit`
fails the comparison; `gen(g)` / `oldCtx(g)` are compared at every step. The
`cid_lease` driver reopens the node's stores through `StoreAdapters::open`
at every composition (each boot and the S-4 read between boots, D-21): its
view store moved from `<root>/views` to `<root>/reconcilers/memory.redb` and
the node directory gains a `workflow-journal.redb`; neither changes what the
driver projects. Each driver's first named run fails at a
production RED scaffold (`red-classification.md`); the three init drivers
do so in driver construction (a step-2 `LaunchToken`).

## Serve-booting fixtures

Every serve-booting fixture builds its driven adapters from
`NodeAdapters::host(&config, cgroup_fs, kek)` and replaces the simulated
fields by name — the crate helpers `tests/common/node_adapters.rs`
(control-plane) and `serve_fixture.rs` / `vsock_support.rs` (cli), the
composed rig, the conformance harness, the `overdrive-sim` invariants and
the `vm_lifecycle_latency_283` spike — with `ServerConfig::new()` and
`NodeVsockPortConfig::kernel_assigned()`. A boot after a shutdown reopens
the stores from disk; no fixture carries a `StoreAdapters` clone. Test
assignments carry the owner's `GuestFlowPorts`; every test beacon bind is
kernel-assigned and reads its port back. The default lane is red only on the
FWD-PREREQ precondition until step K lands the module build (user ruling
U-3, option (b): step K precedes step 0; `red-classification.md`
§ *FWD-PREREQ in the default lane*).

## Kmod, scan and module cells

| Cell(s) | Subject | Lane · step |
|---|---|---|
| `xtask::kmod_load` tests | `kmod-load` over a scripted kernel: absent → load; current → done; stale idle → unload then load; stale busy → refused naming the holder class (open context with pid / `comm`; unseen references with `refcnt`); `--busy-ok` tolerates busy; `EBUSY` + `coming`/`going` busy, + absent `initstate` gone, + `live` busy (retry, never a reboot; `--busy-ok` tolerates it); unhandled errno | default (xtask) · GREEN |
| `overdrive_testing::forwarder_module` tests | D-20 `version` read at a NUL boundary (not the tail of `srcversion=`); stale verdict names `cargo xtask kmod-load`, never a reboot | default · GREEN |
| `required_serve_ports_source_scan` (+1 planted cell) | NO-OVERRIDE-SEAMS / STORES-IN file-wide: no host constructor in `src/lib.rs` outside `test_default_workflow_engine` (helpers, unit structs, `::default()` included); bundle constructors only in `src/node_adapters.rs`; `#[cfg(all(test, …))]` items test-only | default · GREEN once the production `src/lib.rs` holds no such construction |
| `overdrive-fwd` `unload.rs` (2) | V-EV-UNLOAD context half and socket half | metal `kvm-tests` · K, 4 |
| `overdrive-control-plane` `guest_flow_through_serve.rs` (2) | D-17: R5-57 guest-opened share and R5-41 kill-and-reuse through `run_server` + the HTTPS deploy API, read via `ServerHandle::guest_flow_observation()` | metal `kvm-tests` · 14-01 |
| `vsock_host_internal_enforce.rs` F-3, F-5, link-local | absence oracles with an unmarked positive control in the same window | Tier-3 · 14 |

## Guest-kit, in-guest and module-suite cells

| Cell(s) | Subject | Lane · step |
|---|---|---|
| `overdrive-init` `in_guest_lane.rs` (6) + `guest_flow::real_kernel` (6 in-guest cells) | D-22: R5-28 / R5-31's Tier-3 half in the owner lane — the V-24 witness in both accept orders; R5-31 quiescence and session-loss shapes; R5-28 guest-accepted and not-yet-held orders; the 1,000-flow burst (p50 / p99 reported). The cell's production `GuestFlowOwner` is the guest's only owner; the host side is the scripted peer over vhost-vsock, recording the control session; each case serves a positive control flow. The aborted flow is checked only once the owner has processed its V_g: the control flow, whose V_h the host opens after the aborted flow's V_g is queued, has paired (the listener accepts in order), then after 1 s more of dispatching the owner holds nothing of the aborted flow, attached only the control flow's pair, and every earlier application connection is reset and empty; R5-28's guest-accepted cell pins one consumed end record per pair the close ended (an abort) and none for a flow never attached | metal `kvm-tests` · 5 |
| `guest_flow_through_serve.rs` (+3) | U-D17-DIAL: R5-57's `Probe` / `Relay` watch share (agreement on every snapshot; quota counts `Relay`, never `Probe`); R5-63 (e) a dropped probe stream is an abort at the guest; R5-63 (d) / R5-54 a VMM exit under a held relay resets the client. Service VM `listen` + TCP readiness probe; `hold` Job dialling the Service's `workload_addr` | metal `kvm-tests` · 14-01 |
| `overdrive-fwd` `paused_and_stall.rs` (+2), `datagram.rs` (+1) | V-EV-SAFE full-target resume under the wait-queue trigger: vsock target with its peer's credit exhausted, TCP target refilled three times, UDP target whose send buffer is below one datagram — each byte-exact / every datagram once, in order. The UDP case over loopback proves resumption through the bottom-half write-space wake (`loopback_xmit` orphans the sender's skb inside `dev_queue_xmit`'s bottom-half-disabled section, `drivers/net/loopback.c:80 @ v7.0`), not the preemptible `sock_wfree` class a virtio transmit-free path takes; that class is discharged by the `fwd_safe` model and the DELIVER source review of `sk_write_space` | Lima / metal · 4 |
| `overdrive-fwd` `attach_watch.rs` (+4) | attach / watch refuse `UnsupportedSocket { side }` for a half with `SOCK_ZEROCOPY` (TCP, UDP) or a TX-timestamping bit (TCP, vsock at watch and attach); negative controls: the option cleared, a receive-only timestamping bit | Lima / metal · 4 |
| `overdrive-fwd` `unload.rs` (+2) | V-EV-UNLOAD: a datagram association holds one reference per hooked socket; the step-4 version oracle (probe `Version` stage + `inventory()` on the held context while the unload is refused) | metal `kvm-tests` · 4 |
| `overdrive-fwd` `unload_in_guest.rs` (2 host + 2 in-guest) | U-EV-UNLOAD: ULP negative control and the unload stress in the guest-kit microVM; the ULP control reads the references over named windows (`wait_refcnt` for each transition, one named hold window, `ULP_HOLD_WINDOW`, only after the positive half); the stress's sender far ends never block; its oracles are no oops and the refcount on the stock 7.0 kernel | metal `kvm-tests` · 4 |

Every cell above compiles (`integration-tests` and `integration-tests,kvm-tests`),
passes clippy `-D warnings` and lands at its expected classification
(`red-classification.md` § *Kmod, kit-lane and in-guest cells*, § *Walking
skeleton on metal*). `GuestKit::boot` launches through the production
`CloudHypervisorVmm` (`Vmm::create` over a `VmConfig`; U-4), so the kit
carries no argv.

## Lane setup (native metal)

The guest-kit lanes run `cargo xtask guest-kit-prepare` first
(`infra/metal/README.md`; `upstream-issues.md` S-12, T-6): it builds
`ovd_fwd.ko` for the running kernel, the static scripted peer and the
in-guest test binaries, and stages each at the contract path the fixture
reads; the fixture refuses with `LaneNotPrepared` when one is missing. The
FWD-PREREQ lane setup, on Lima and on metal, is `cargo xtask kmod-build`
then `cargo xtask kmod-load` (the hooks: `kmod-load --busy-ok`); the
composed `serve` fixtures check that the loaded module is this workspace's
build (`overdrive_testing::forwarder_module`; `upstream-issues.md` D-20 on
what that comparison covers).

## Boot, relay and forwarder cells

| Cell | Contract / owner path | Oracle | Tier · pending |
|---|---|---|---|
| `owner_acceptance::legs_dial` (4) | R5-54 seeded half, LEGS-DIAL: `SimMtlsEnforcement` built with the composition's `OwnerDial`, driven through `MtlsEnforcement::enforce` (`Routed::Inbound`) then `teardown` | one `DialPurpose::Relay` dial after authentication and `held_flows` restored with `0x3` at teardown; no dial for a client the peer verification refuses (`AbsentBundle`); a dial error returns `GuestDial { NotServing }` and closes leg C; no owner dial outside the guest prefix | seeded sim · attachment-cut (9) |
| `prefix_boot_acceptance::phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one` | D-8 phase 1, through `probe_guest_flow_startup` and through the owner's `probe_startup` | each scripted `ProbeForwarder` failure (`DeviceAbsent`, `AbiMismatch`, a Backpressure-stage mismatch, a Cleanup-stage residue) gives `Forwarder { cause }` with no context left open and no other effect; a pass issues one `ProbeForwarder`, no serving open, and releases its context | default · (12-01 rework) |
| `overdrive-fwd` `end_record_abi::take_end_returns_records_in_end_order` | D-12, EVMOD-REC / EVMOD-WQ: `take_end` returns records oldest first | two pairs ended in a known order (abort path and clean-close path) are read in that order | Tier 3 · step 4 |
| `overdrive-worker` `vsock_leg_f_repair::leg_f_repair_rebinds_transparent_listener` | D-9 / V-8: the supervisor's repair port on the real `HostMtlsIntercept` after leg-F's accept task ends (`shutdown(SHUT_RD)`, S-11) | repair `Ok` (no `EADDRINUSE`), same address, a new LISTEN inode, the old descriptor closed, a connect succeeds, the audit is `Ok` | Tier 3 (`host-kernel-shared`) · 14-02 |
| `vsock_density` R5-25 fail-closed | a live host listener on the dialled port (S-10) | every VM address refused `ECONNREFUSED` / `ENETUNREACH` with `serve` up and down; the listener accepts nothing | native metal · 14-05 |
| `vsock_fault_matrix` R5-62 | stall window | polls until two samples 2 s apart are equal and non-zero (60 s bound), then the count stays flat for 10 s | native metal · 14-02 |

## Completeness (`nw-at-completeness-check`)

C1a/b (empty, boundaries: zero payload frames, `MAX` and `MAX + 1`, the
probe cap 8 / 9, the reserved range edges, the restart ceiling `CEIL − 1` /
`CEIL`), C2a/b (state machines documented in the driver headers and the
specs; illegal events refused with typed errors), C3 (0/1/N pairs,
watches, datagrams, slots; 64 VMs), C4a/b (END-IDEMPOTENT, a second abort
`AlreadyEnded`, abort of an unknown id), C5a/b (stream / datagram / watch
modes; feature-disabled activation), C6a/b/c (every refusal of every codec
and of attach, the closed `ForwarderError` and `GuestNetworkError` sets),
C7a (stalled readers, full queues, a 10 s stall, 1,100 UDP sockets churned),
C7b (crash mid-recovery, quiescence mid-flow, VMM death, owner death,
`serve` death and guest kernel crash on native metal), C7c (two contexts,
overlapping recoveries, racing sessions) pass: 15 / 15 → **COMPLETE**.
Documented gaps that do not fail an item: the guest-driven
control-session-loss fault (D-16; seeded sim plus the in-guest session
cells) and R5-28's guest-side "counted" clause (seeded sim only, D-22);
V-9's measurements are the benchmark GH #326. D-17, D-22, U-D17-DIAL and T-9 are authored.

## Deleted with the old contract

The redirect-era bodies and drivers whose subject EVMOD-ADOPT removed:
`overdrive-dataplane/tests/integration/vsock_{forwarder_verdict,
seqpacket_length, flow_end_classifier, drain, egress_unframe, fragment_map,
guest_steering}.rs` and `helpers/guest_net.rs`; the
`overdrive-dataplane::guest_steering` unit cells (list C) with
`proptest-regressions/guest_steering.txt`; `quint_flow_end.rs`,
`quint_udp_empty.rs`, `quint_flows_half.rs`, `tests/half_close.rs`; the
`FlowEvent` / `FlowCell` / `CellEpoch` / empty-notice cells; the
`overdrive-bpf` `maps/guest_flow.rs` scaffold; `tests/conformance`
`prefix_landing`; the worker TAP-era egress-capture tests; the stale
`*.proptest-regressions` files of the rewritten core codecs; the earlier
pass's partition documents (`test-scenarios-p{2,3,5}-*.md`, superseded by
P3a, P4 and P3b).
