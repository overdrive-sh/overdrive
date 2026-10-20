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
| Native metal lanes | `shared-test-infrastructure.md` § 3, P2, P3a, P4, P5 | `overdrive-testing` (guest kit), `overdrive-fwd`, `overdrive-control-plane`, `overdrive-host`, `overdrive-init`, `overdrive-cli` | `guest_boundary` (T-1), R5-59 (a) flow and beacon seams, the in-guest lane (T-2), walking skeleton P6-S13 (10-01), native fault matrix P6-S14 (10-02), density and fail-closed P6-S15 (08-04) |

Cross-cutting: `red-classification.md` (gate runs, driver activation,
semantic RED per marker), `shared-test-infrastructure.md` (sims, rigs,
lanes), `upstream-issues.md` (blockers and routing).

## Marker convention

`#[ignore = "pending DELIVER <step>"]` naming the Roadmap impact step that
activates the body (`step N` for the new steps 1–15; `attachment-cut step
(N)` for the DG-2 grouping; the rework names of the earlier steps, e.g.
`(09-01 rework)`). The body is complete; the crafter only removes the marker.

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
| CP `guest_network::quint_owner_flows` (`flows_conf`) | `guest_network.rs:75` | `:305` | `:1041` | `:236` | `:1137` | `:1727` | 45 |
| CP `guest_network::quint_fwd_pair` (`pair_conf`) | `guest_network.rs:69` | `:361` | `:1326` | `:275` | `:1448` | `:1921` | 39 |
| CP `guest_network::quint_flow_quota` (`quota_conf`) | `guest_network.rs:67` | `:164` | `:415` | `:141` | `:431` | `:498` | 5 |
| CP `guest_network::quint_udp_slots` (`slots_conf`) | `guest_network.rs:77` | `:134` | `:339` | `:117` | `:367` | `:512` | 12 |
| CP `guest_network::quint_cid_lease` (`lease_conf_ok`, `lease_conf_325`, `lease_conf_three`) | `guest_network.rs:65` | `:654` | `:1588` | `:1750` | `:1808` | `:2112` | 21 + 1 + 1 |
| CP `quint_quiescence` (`quiesce_conf`) | `lib.rs:170` | `:174` | `:416` | `:505` | `:562` | `:685` | 10 |
| CP `reconciler_runtime::quint_probe_count` (`probe_streak_conf`) | `reconciler_runtime.rs:1831` | `:117` | `:215` | `:256` | `:365` | `:484` | 9 |
| worker `vm_driver::quint_flows_beacon` (`flows_beacon_conf`) | `vm_driver.rs:2212` | `:222` | `:538` | `:593` | `:629` | `:726` | 7 |
| init `guest_flow::quint_owner_flows` (`flows_conf`) | `guest_flow/mod.rs:43` | `:229` | `:643` | `:762` | `:797` | `:1395` | 45 |
| init `guest_flow::quint_fwd_pair` (`pair_conf`) | `guest_flow/mod.rs:39` | `:276` | `:759` | `:1306` | `:1331` | `:1819` | 37 |
| init `guest_flow::quint_udp_slots` (`slots_conf`) | `guest_flow/mod.rs:45` | `:205` | `:575` | `:669` | `:736` | `:895` | 12 |

Line numbers re-read in the round-13 integration pass (2026-10-20), after
the drivers compiled.
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
Round 13 (NO-OVERRIDE-SEAMS, STORES-IN, PORTS-CONFIG): the `quiescence`
driver's `RestartWith` / `Damage` / `RepFail` go through the composed rig
(`boot_server` over `SimGuestFlowIo` and a fresh `BootAdapters` per boot, the
stores reopened from disk, never a retained clone); every composition reads
its vsock ports back from the owner (`flow_ports()`, `BeaconIntake::bound_port`),
never a number; the init `fwd_pair` driver no longer reads `driver.gone`.
Round 16: the init `fwd_pair` driver gates `ctx(g)`, the owner view
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

## Round-13 integration pass

Every serve-booting fixture now builds its driven adapters from
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

## Round-16 cells pass

| Cell(s) | Subject | Lane · step |
|---|---|---|
| `xtask::kmod_load` tests (14) | `kmod-load` over a scripted kernel: absent → load; current → done; stale idle → unload then load; stale busy → refused naming the holder class (open context with pid / `comm`; unseen references with `refcnt`); `--busy-ok` tolerates busy; `EBUSY` + `coming`/`going` busy, + absent `initstate` gone, + `live` confirmed by a second unload, a second `EBUSY` on `live` → no `module_exit` (not softened by `--busy-ok`); second-pass race; unhandled errno | default (xtask) · GREEN |
| `overdrive_testing::forwarder_module` tests | D-20 `version` read at a NUL boundary (not the tail of `srcversion=`); stale verdict names `cargo xtask kmod-load`, never a reboot | default · GREEN |
| `required_serve_ports_source_scan` (+1 planted cell) | NO-OVERRIDE-SEAMS / STORES-IN file-wide: no host constructor in `src/lib.rs` outside `test_default_workflow_engine` (helpers, unit structs, `::default()` included); bundle constructors only in `src/node_adapters.rs`; `#[cfg(all(test, …))]` items test-only | default · GREEN once the production `src/lib.rs` holds no such construction |
| `overdrive-fwd` `unload.rs` (2) | V-EV-UNLOAD context half and socket half | metal `kvm-tests` · K, 4 |
| `overdrive-control-plane` `guest_flow_through_serve.rs` (2) | D-17: R5-57 guest-opened share and R5-41 kill-and-reuse through `run_server` + the HTTPS deploy API, read via `ServerHandle::guest_flow_observation()` | metal `kvm-tests` · 10-01 |
| `vsock_host_internal_enforce.rs` F-3, F-5, link-local | absence oracles with an unmarked positive control in the same window | Tier-3 · 14 |

## Lane setup (native metal)

The guest-kit lanes run `cargo xtask guest-kit-prepare` first
(`infra/metal/README.md`; `upstream-issues.md` S-12, T-6): it builds
`ovd_fwd.ko` for the running kernel, the static scripted peer and the
in-guest test binaries, and stages each at the contract path the fixture
reads; the fixture refuses with `LaneNotPrepared` when one is missing. The
FWD-PREREQ lane setup, on Lima and on metal, is `cargo xtask kmod-build`
then `cargo xtask kmod-load` (the hooks: `kmod-load --if-absent`); the
composed `serve` fixtures check that the loaded module is this workspace's
build (`overdrive_testing::forwarder_module`; `upstream-issues.md` D-20 on
what that comparison covers).

## Round-9 cells

| Cell | Contract / owner path | Oracle | Tier · pending |
|---|---|---|---|
| `owner_acceptance::legs_dial` (4) | R5-54 seeded half, LEGS-DIAL: `SimMtlsEnforcement` built with the composition's `OwnerDial`, driven through `MtlsEnforcement::enforce` (`Routed::Inbound`) then `teardown` | one `DialPurpose::Relay` dial after authentication and `held_flows` restored with `0x3` at teardown; no dial for a client the peer verification refuses (`AbsentBundle`); a dial error returns `GuestDial { NotServing }` and closes leg C; no owner dial outside the guest prefix | seeded sim · attachment-cut (9) |
| `prefix_boot_acceptance::phase_one_probes_the_forwarder_on_a_context_it_releases_and_refuses_a_bad_one` | D-8 phase 1, through `probe_guest_flow_startup` and through the owner's `probe_startup` | each scripted `ProbeForwarder` failure (`DeviceAbsent`, `AbiMismatch`, `NotPermanent`, a Backpressure-stage mismatch, a Cleanup-stage residue) gives `Forwarder { cause }` with no context left open and no other effect; a pass issues one `ProbeForwarder`, no serving open, and releases its context | default · (09-01 rework) |
| `overdrive-fwd` `end_record_abi::take_end_returns_records_in_end_order` | D-12, EVMOD-REC / EVMOD-WQ: `take_end` returns records oldest first | two pairs ended in a known order (abort path and clean-close path) are read in that order | Tier 3 · step 4 |
| `overdrive-worker` `vsock_leg_f_repair::leg_f_repair_rebinds_transparent_listener` | D-9 / V-8: the supervisor's repair port on the real `HostMtlsIntercept` after leg-F's accept task ends (`shutdown(SHUT_RD)`, S-11) | repair `Ok` (no `EADDRINUSE`), same address, a new LISTEN inode, the old descriptor closed, a connect succeeds, the audit is `Ok` | Tier 3 (`host-kernel-shared`) · 10-02 |
| `vsock_density` R5-25 fail-closed | a live host listener on the dialled port (S-10) | every VM address refused `ECONNREFUSED` / `ENETUNREACH` with `serve` up and down; the listener accepts nothing | native metal · 08-04 |
| `vsock_fault_matrix` R5-62 | stall window | polls until two samples 2 s apart are equal and non-zero (60 s bound), then the count stays flat for 10 s | native metal · 10-02 |

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
control-session-loss fault (D-16), R5-57 through `serve` (D-17) and the not-authored shapes of T-9 (the Tier-3
half of R5-54 among them); V-9's measurements are the benchmark GH #326.

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
