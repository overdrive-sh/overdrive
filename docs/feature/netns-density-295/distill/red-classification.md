# Pre-DELIVER RED classification — `netns-density-295` (correctness-recovery rewrite)

This file classifies why each acceptance body is expected to fail before
DELIVER, so the RED phase of every re-roadmapped step can confirm that a failure
is genuine missing functionality and not a setup, import, or fixture defect. It
targets the accepted replacement DESIGN (D-295-R1 to R22) and the scenario set
in `test-scenarios.md`.

Classification vocabulary:

- **RED — MISSING_FUNCTIONALITY**: the body reaches the production owner and
  fails at its RED scaffold or on wrong behaviour.
- **RED — REPRODUCED DEFECT**: the body reproduces a recorded correctness gap on
  today's code.
- **GREEN — ACCEPTED BEHAVIOUR PRESENT**: the body passes because the accepted
  behaviour already exists; it authorizes no production change.
- **GREEN — KERNEL CONTRACT PIN**: the body pins kernel behaviour the design
  relies on; it passes without #295 code.
- **BROKEN** (import, fixture, setup, or observable-not-at-port failure):
  blocks handoff until the body is fixed.
- **PENDING_ENVIRONMENT**: the body compiles and is selected, but its lane
  (native metal, x86_64 build) was not available for the run.

## Native evidence retained from the withdrawn D-295-DELIVER-04-01 revisions

Both runs stay as evidence. Neither is GREEN credit for any replacement body.

| Evidence | Observed result | Classification now |
|---|---|---|
| Native predecessor + restart + D7 run `c4d36190` | Before the exact `mtls.intercept.install.success` event on caller TAP ifindex 58748: three guest ARP replies (`0x0806`, packet_type 3, opcode 2, guest SHA/SPA `02:01:00:00:00:03` / `100.95.0.3`, target the gateway `100.95.0.1`) and one guest-source IPv4 TCP reset. | **EVIDENCE — an up TAP admits guest-kernel frames before intercept-live.** It falsifies any design that keeps the TAP up before the event. It motivates D-295-R5 (TAP down until activation). It does **not** justify admitting those frames. |
| Native down-TAP run `f1a15668` | Cloud Hypervisor v53's named attachment failed before READY: `Cannot create virtio-net device` / `Failed to open taps` / `SIOCSIFFLAGS (35092)` / `EPERM`. | **EVIDENCE — the named `tap=` path cannot keep a TAP down under the confined VMM.** It motivates D-295-R1 and R2 (`--net fd=[3]` handoff). It does not select the TAP-up oracle. |

## Superseded classifications

| Earlier classification | Status |
|---|---|
| "CONTROL-FRAME ORACLE SELECTED" — the closed ARP-reply / zero-payload TCP-reset population before the event (v2) | **SUPERSEDED.** It weakened the zero-frame invariant, which the charter forbids (Changed Assumption 2, FD § "[REF] Changed Assumptions" (item 2)). The replacement oracle is zero frames with the TAP down (S-ND295-01). |
| "fd handoff out of accepted scope / unnecessary" (v2) | **SUPERSEDED.** The native fd spikes (`.context/netns-density-295-fd-tap-spike-findings.md`, `spike/findings-persistent-fd-tap.md`) proved the handoff; D-295-R1 to R3 adopt it. |
| "DEFERRED TAP ACTIVATION DESIGN FALSIFIED … no `activate`, phase, error, fd handoff, capability grant, or confinement change is permitted" | **SUPERSEDED.** Only its named-TAP clause was falsified; the ordering and owner-state contract survives as D-295-R5 over the fd handoff. |
| The 2026-09-23 "deferred-TAP-activation amendment" v1 subsection | **SURVIVES IN INTENT** as D-295-R5; its "Cloud Hypervisor attaches the down TAP by name" and "argv stays `tap=`" clauses are superseded by D-295-R1/R2. |
| Every classification that credited the eight user-waived `panic!` placeholders as "not scored" | **SUPERSEDED.** `testing.md` forbids handing a placeholder body to DELIVER; phase B authors or deletes each (`test-scenarios.md` § *Existing-body disposition register*). |
| Phase D's REQ-295-LINKMAC framing of the fresh-host RCA (root cause A → a host `.link` policy on the substrates and the image, "one writer per managed-link address") | **SUPERSEDED 2026-09-28** by the user rulings of that date (FD § "[REF] Managed-link identity independent of host link configuration (fresh-host RCA) — pinned 2026-09-26; user rulings of 2026-09-28"). REQ-295-LINKMAC is not a requirement; root cause A's fix is the bridge creation contract alone (05-00). The phase-D rows below that rest on it are marked in place; Phase E holds the current classifications. |
| Phase D S-ND295-72 (p1)/(p2) — the startup probe's scratch-TAP address condition (three `scratch_probe_acceptance` bodies) | **SUPERSEDED 2026-09-28 — bodies DELETED** (user ruling 4: under the host-side MAC invariant a rewrite is harmless, so the condition would refuse a correct host). |
| Phase D S-ND295-72 (e) `a_provisioned_taps_recorded_address_survives_udev_initialisation` — a recorded host-side MAC compared with the live one | **SUPERSEDED 2026-09-28 — body re-authored** as `a_tap_host_address_is_judged_by_the_invariant_whatever_the_host_link_manager_wrote` (two allocations; no recorded address; Phase E). |

## Recovery proof REDs — current classification

The five proofs ran against HEAD `db3af700` plus the then-staged 02-03/04-01/04-02
work (§3.5 against HEAD plus the serve lifetime port only), per
`recovery/proof-findings.md`. They are the reproduced-defect oracles for the
replacement; after re-targeting (`test-scenarios.md` § *Recovery proof tests —
landing decisions*) phase C re-runs them for fail-for-the-right-reason.

| Proof | Body | Seeds / run | Result recorded | Classification | Re-targeted scenario / step |
|---|---|---|---|---|---|
| §3.2 node-wide admission | `node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads` | `186055177052160001`, `295032` (identical verdicts) | NA-1, NA-2, NA-4a, NA-4b, NA-G RED (admitted 16,385); NA-5, NA-4a-L, NA-4b-L GREEN; OBS-OVERLAP peak 16,386 | **RED — REPRODUCED DEFECT** (gap 1: the cap is evaluated over workload-local rows; restart bypasses placement; no linearization point) | S-ND295-05D — held-population oracle, NA-OVERLAP asserted — 07-03 |
| §3.3 supervisor | ten bodies, `shared_network_supervisor_recovery_proof` | `0x2953300000000001`, `0x295330005eed0002` (1 passed, 9 failed) | C0 GREEN; C1 RED (0 shared-owner audits in 10 s); C2 RED (admission stays open); C3-C8 UNREACHED | **RED — REPRODUCED DEFECT** (gap 2: the supervisor audits only the worker; C3-C8 unreachable until detection exists) | S-ND295-29B — required ports, C6 split — 09-01 |
| §3.4 element cleanup | `shared_element_cleanup_failure_{deletion_rejected,readback_failed}_retains_retirement_and_address` | deterministic (2 run, 2 failed) | witness GREEN; nine assertions RED (stop `Ok(())`, converged, successor admitted, `TapDelete`, address reassigned, row `Terminated`, no retry removal, 4 members left) | **RED — REPRODUCED DEFECT** (gap 5: removal hidden in infallible `Drop`) | S-ND295-07B — 07-01 |
| §3.5 killed-mode boot clear | `a_killed_serve_reboot_reclaims_clears_stale_intercept_members_then_admits` | example, reproduced twice | V1 GREEN; V0, V2-V6 RED (`health.startup.refused` reason `mtls.shared_owner`; 0 boot-two clear batches; 3 stale members survive) | **RED — REPRODUCED DEFECT** (gap 6: no production caller of boot clear). It could not run on the staged tree because mesh VMs never reach Running there (gap 7). | S-ND295-13C — 08-02 |
| §3.6 CLI fail-stop | `serve_lifetime_fail_stop` (3 bodies) | metal, recording `SimClock` | pre-port body RED (exit 0, SIGINT consumed, no bound); on the built port all GREEN | **GREEN — ACCEPTED BEHAVIOUR PRESENT** (D-295-R17 built and user-approved). Its fault is re-targeted to an unrepairable wrong-target rule because whole-table deletion becomes repairable under D-295-R15. | S-ND295-68 — active; re-verified at 09-02 |

Supporting proof §3.1 (fd handoff, native): **WORKS** with two conditions
(`IFF_VNET_HDR` requested by the launcher attach; TAP set down after VMM exit
and before any reattach), both carried by D-295-R2 and the single-attach
invariant (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the persistent-TAP lifecycle and the single-attach invariant)). It is spike evidence, not a test.

## Expected classifications for the rewritten set (before phase C runs)

| Bodies | Expected | Why |
|---|---|---|
| Every body whose driving port is a phase-B `todo!("RED scaffold …")` (new pool ops, `attach_tap_queue`, `debug_msg_mask(s)`, `attach_first_egress`, `register_launch_child_hook`, `for_target`, `check_launch_seccomp`, `remove_allocation_elements`, `converge_allocation_elements`, `observe_shared_state`, `run_shared_network_supervisor`, `GuestAttachmentLease::cleanup_pending`, `CgroupPath::workloads_slice`, the reclaim shim arm, `xtask::cloexec_lint::{scan_source, scan_workspace, render_violation}`) | RED — MISSING_FUNCTIONALITY | the body reaches the production owner and stops at its scaffold |
| S-ND295-46 real-workspace body (`the_serve_closure_creates_no_inheritable_descriptor`) once the gate exists | RED — MISSING_FUNCTIONALITY until 05-04 fixes the eight sites | the gate reports each unfixed site of the obligation's table (FD § "[REF] Driven port — VMM TAP queue attachment (D-295-R1, R2, R3, R4) — ACCEPTED 2026-09-24" (the OBL-295-CLOEXEC eight-site table)) |
| Sim-double self-tests (`adapters::guest_network::tests`, `adapters::guest_dns::tests`) | GREEN — ACCEPTED BEHAVIOUR PRESENT | phase B implements the pinned doubles faithfully (scaffold class F); they are fixture contracts, not production behaviour |
| RETARGETED bodies on existing owners (S-ND295-04, 06, 07, 10, 11, 12, 51, 52, 05B, 05C, 55, 13A, 19, 33) | RED — MISSING_FUNCTIONALITY | wrong behaviour at the existing owner (e.g. owner uid 4200, no egress step, Running-row placement, S19 journal `[TapSetDown]` only) |
| S-ND295-70 sim-listener self-tests and worker listener bodies; S-ND295-29A's listener-loss cells; the `shared_…` twins that script a connection (`test-scenarios.md` § *Intercept listener and stop-error test support*) | RED — MISSING_FUNCTIONALITY until the step that lands B-7 (no later than 05-01) | before that step `bind_transparent` returns a real socket, so `live_listeners()` is empty, every scripting call returns `false`, and no `accept` is parked |
| S-ND295-70 held-address equivalence clause, original-destination and descriptor-state bodies; `shared_…` twins that need no connection | GREEN — ACCEPTED BEHAVIOUR PRESENT possible | today's socket already refuses a held address, reports the dialled destination, and accepts with `FD_CLOEXEC` set and `O_NONBLOCK` clear, and the shared allocation path exists |
| S-ND295-54 B-6 caller-rule bodies | RED — MISSING_FUNCTIONALITY until 07-01 | retirement does not call `remove_allocation_elements` before R10, so no `ElementRemoval` is produced |
| S-ND295-39 (`EPERM` on a root-owned TAP) | GREEN — KERNEL CONTRACT PIN | kernel `tun_not_capable` |
| S-ND295-42 deny-list equality row | may be GREEN at scaffold | the constant is data transcribed from the pinned table; the verdict-partition rows stay RED until the builder exists |
| S-ND295-27 cause table | GREEN — ACCEPTED BEHAVIOUR PRESENT | the gate is cause-agnostic once the two variants exist |
| S-ND295-13B telemetry | GREEN possible | the four boot-phase events already exist (D-295-DISTILL-13) |
| Native bodies (S-ND295-01, 13C, 30B, 35, 37, 45, 62-64, 66, 67, 69) | PENDING_ENVIRONMENT until run on metal; then RED — MISSING_FUNCTIONALITY | the fd-handoff path does not exist yet |
| x86_64-only bodies (S-ND295-41, 42, 43) on the aarch64 Lima build | not compiled on that target | `#[cfg(target_arch = "x86_64")]`; run on an x86_64 build |
| S-ND295-44 aarch64 refusal arm on an x86_64 build | not compiled on that target | `#[cfg(not(target_arch = "x86_64"))]` |

## Prior-cycle classifications (history only)

Earlier cycles classified the phase-02 D12/D12A/D13/D14A bodies, the D15
shared-IP bodies, the step 02-03 D1-D7 remediation, the step 03-02 S-ND295-28
oracle correction, and the step 03-03 S19-B synchronization correction. Those
steps are committed and their bodies are GREEN or re-targeted above; the exact
runs remain in git history (`git log -- docs/feature/netns-density-295/distill/red-classification.md`)
and in `deliver/execution-log.json`. They are not current classifications.

## Phase C — fail-for-the-right-reason run results

Executed 2026-09-25/26, serially and one command at a time in the foreground,
against tree `7ecd69bb04c5f473e752a6abb6f7319f797be5a4` plus the uncommitted
test-side fixes listed under *Test-side changes in this run* (no production
code changed). Every `#295` DISTILL body was run: all 182 bodies carrying
`pending DELIVER step NN-NN (S-ND295-xx)` (enumerated from the source and
cross-checked against the scenario-to-test matrix's Rust homes), plus the
active (unmarked) `#295` bodies the package filters select or the matrix names.
Raw run logs were captured locally under `target/phase-c-295/` (not committed).

### Substrates

| Substrate | Host | `uname -r` | Runner |
|---|---|---|---|
| Lima | `overdrive` VM, aarch64, 8 vCPU, cgroup v2, root via the wrapper | `7.0.0-31-generic` | `cargo xtask lima run -- …` |
| Native metal | non-virtualized x86_64 bare metal, 16 CPUs, `systemd-detect-virt` = `none`, Cloud Hypervisor `v53.0`; the xtask fail-closed KVM preflight and canonical metal lease ran on every command | `7.0.0-29-generic` | `cargo xtask metal run -- …` (syncs this worktree unless `--no-sync`) |

The BPF object was rebuilt clean on both substrates. On Lima the warm-cache
build emitted an object without `gh295c_egress` (the documented incremental DCE
hazard), so `bpfel-unknown-none` and `target/bpf/overdrive_bpf.o` were removed
and `cargo xtask bpf-build` re-run; on metal the object was built with
`OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build` after the same clean. Both
objects carry `gh295c_egress` and `gh295c_endpoint`.

### Environment and cleanup

- **Lima was quiet throughout.** No other workspace's `cargo`/`nextest`
  process was present before, during (spot checks), or after the pass.
- **Lima, before the kernel runs:** removed `ovd-veth-bk`/`ovd-veth-cli`,
  `ovd-gbr0`, `table bridge overdrive-mtls`, the `fwmark 0x1 lookup 100` rule and
  table-100 route, lo addresses `10.200.0.2/32`, `10.200.0.3/32`,
  `10.201.0.1/32`, and the pinned `mtls-endpoints` maps. After the control-plane
  integration run: removed the leaked `alloc-nd295-teardown-keep-0.scope`,
  `ovd-tp-0002` and its pinned ingress link (left by failing RED bodies). Clean
  again after the whole pass.
- **Metal, before the native runs:** removed the veth pair, `ovd-gbr0`,
  `table bridge overdrive-mtls`, the stale **`table ip overdrive-mtls`** (the
  pre-R19 rule-order table), the fwmark rule and table-100 route, lo
  `10.200.0.2/32` and `10.200.0.3/32`, and the pinned maps. After the runs:
  removed the netns `nd295-64-124084` that the S-ND295-64 fixture leaked when its
  veth creation failed before `TimeWaitTopology` existed. No leaked Cloud
  Hypervisor, cgroup scope, TAP or netns remained after the timeout-terminated
  native bodies.

### Runs

| Run | Command (all with `--run-ignored all --no-fail-fast --status-level pass` unless noted) |
|---|---|
| C-01 | `cargo xtask bpf-build`; `cargo xtask bpf-unit` (fail-fast; `--status-level` unset); `cargo xtask lima run -- cargo nextest run -p overdrive-bpf --features integration-tests --test integration -E 'test(/guest_tcx_classifier_test_run/)'` (C-01b…d after fixes) |
| C-02 | `… -p overdrive-core --test acceptance -E 'test(/netns_density_placement_cap\|netns_density_cleanup_pending\|netns_density_exec_gate/)'` |
| C-03 | `… -p overdrive-reconcilers -E 'test(/restart_gating_acceptance\|reclaim_emission_acceptance\|service_projection_keeps_first_tcp/)'` |
| C-04 | `… -p xtask --lib -E 'test(/cloexec_lint/)'` |
| C-05 | `… -p xtask --test integration --features integration-tests -E 'test(/cloexec_lint_workspace/)'` |
| C-06 | `… -p overdrive-netlink --test integration --features integration-tests -E 'test(/tap_queue_attach\|tap_debug_msg_mask/)'`; `… -p overdrive-netlink --lib -E 'test(/debug_message_constants/)'` |
| C-07 | `… -p overdrive-dataplane --lib --features integration-tests -E 'test(/semantic_counter_vocabulary\|absent_real_objects/)'` |
| C-08 | `… -p overdrive-dataplane --test integration --features integration-tests -E 'test(/guest_tcx_inventory\|guest_tcx_egress_lifecycle/)'` |
| C-09 | `… -p overdrive-host --lib --features integration-tests -E 'test(/launch_seccomp\|vmm::tests::/)'` (aarch64: 5 bodies compile) |
| C-10 | `… -p overdrive-control-plane --lib -E 'test(/allocation_owner_acceptance\|pool_acceptance\|admission_refusal_acceptance\|shared_network_task_owner_acceptance\|validate::tests\|shared_network_test_ports::tests/)'`; `… -p overdrive-control-plane --test compile_fail` |
| C-11 | `… -p overdrive-control-plane --test acceptance -E 'test(/netns_density_guest_network\|required_serve_ports_source_scan/)'` |
| C-12 | `… -p overdrive-control-plane --test integration --features integration-tests -E 'test(/server_lifecycle\|shared_guest_network_startup\|mtls_install_fail_closed\|shared_element_cleanup_failure\|shared_network_supervisor_recovery\|guest_attachment_pool_per_server\|network_cleanup_pending_status\|dns_responder_bind\|reconcile_output_validator/)'`; C-12b/c: the S-ND295-00 body alone on a cleaned VM |
| C-13 | `… -p overdrive-worker --lib -E 'test(/shared_program_rollback_acceptance\|mtls_intercept_worker::tests\|mtls_intercept_port::/)'` |
| C-14 | `… -p overdrive-worker --test integration --features integration-tests -E 'test(/netns_density_shared_owner\|shared_intercept_members\|mtls_intercept_equivalence\|mtls_intercept_install\|egress_tproxy_capture\|name_resolve_enforce_consistency/)'` |
| C-15 | `… -p overdrive-sim --lib --features integration-tests -E 'test(/netns_density_boot_order\|adapters::/)'` |
| C-16 | `… -p overdrive-sim --test acceptance -E 'test(/netns_density_/)'` |
| C-17 | `… -p overdrive-sim --test integration --features integration-tests -E 'test(/netns_density_node_admission/)'` (251 s) |
| C-18 | `… -p overdrive-cli --test acceptance -E 'test(/render_workload_describe/)'` |
| C-19 | `… -p overdrive-system-conformance --features integration-tests` |
| C-20 | active bodies the listed filters missed: `-p overdrive-control-plane --lib` (six `scratch_probe_*` bodies), `-p overdrive-dataplane --lib --features integration-tests` (six `guest_tcx::tests` S-ND295-00 bodies), `-p overdrive-netlink --lib --features integration-tests` (`persistent_tap_and_bridge_projection_preserves_every_observable_identity_field`) |
| N-00/N-01 | `cargo xtask metal run -- cargo nextest run -p overdrive-host --lib --features integration-tests -E 'test(/launch_seccomp\|vmm::tests::/)'` (N-00 before, N-01 after the harness fix) |
| N-02 | `… -p overdrive-host --test integration --features integration-tests,kvm-tests -E 'test(/vmm_tap_queue_errors/)'` |
| N-03 | `… -p overdrive-worker --test integration --features integration-tests -E 'test(/outbound_enforce_substrate_splice/)' --no-fail-fast` |
| N-04…N-09 | the listed `overdrive-cli --test integration --features integration-tests,kvm-tests` filter, run as its six constituent groups one at a time to stay inside the tool's 30-minute cap: N-04 `serve_lifetime_fail_stop` (N-04b rerun), N-05 `serve_killed_restart_boot_clear`, N-06 `shared_network_native_faults` (722 s), N-07 `intercept_mark_fail_closed` (600 s; N-07b…e reruns of S-ND295-64), N-08 the three `guest_stack_mtls_egress` bodies (360 s), N-09 the four `vm_walking_skeleton` bodies (480 s) |

### Classification vocabulary used here

In addition to the vocabulary at the top of this file:

- **RED — preceding-step gap (NN-NN):** the body fails on the missing
  scaffold or behaviour of an *earlier* DELIVER step than the one its marker
  names (a sibling scaffold, the 06-02 owner-uid change, the 06-03 lease
  lifecycle, the 05-01 required ports, or gap 7 — no guest reaches Running
  until the 05-03 fd handoff). The marker order is sound: by the time the
  marker's step activates the body, the blocking step has landed. The body's
  own-step oracle is therefore not observable today and is verified at
  activation, in that step's RED phase. No test-side fix exists; the fix is the
  preceding step's production work.
- A body blocked by a *later* step than its marker is **BROKEN — remaining**.

### Summary

| Class | Bodies |
|---|---|
| RED — at its own step (own scaffold or own oracle) | 125 |
| RED — preceding-step gap | 49: 05-01 ×13, 05-03 ×19, 06-01 ×3, 06-02 ×5, 06-03 ×4, 07-01 ×1, 08-02 ×2, 09-01 ×2 |
| BROKEN → fixed → RED | 3 (S-ND295-47 ×1, S-ND295-43 ×2) |
| PASS (genuine) → marker removed | 3 (S-ND295-48, S-ND295-70, S-ND295-66 — one body each) |
| BROKEN — remaining (blocker) | 2 (S-ND295-64, S-ND295-45) |
| **Pending-marked total** | **182** |
| Active `#295` bodies: PASS | 75 |
| Active `#295` bodies: FAIL (production, blocker) | 3 (S-ND295-00 ×1, S-ND295-70 ×2) |

### Blockers

1. **S-ND295-64 `a_newer_sequence_reconnect_into_time_wait_is_recorded_after_both_controls` — vacuous guest case.**
   After the three fixture fixes below, both controls hold on metal three runs
   in a row. The negative control gets a bare ACK and the positive control a
   SYN-ACK. The body then passes only because its "guest case" re-probes the
   host-only tuple the positive control just consumed and asserts nothing
   (`S-ND295-64 guest TIME_WAIT reconnect outcome (recorded, not absorbed):
   SynAck`). The TS guest case needs an intercepted guest connection whose host
   side is left in `TIME_WAIT` after leg-F closes, with a wildcard listener
   waiting. It is not authored, and it cannot be exercised until a mesh guest
   reaches Running (gap 7, 05-03). The marker is kept.
2. **S-ND295-45 `every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters` — marked 05-02, blocked by 05-03.**
   Its precondition is a guest that reaches Running. On this tree every guest
   fails with `Cannot create virtio-net device` / `Failed to open taps` /
   `Ioctl failed (35092)` / `EPERM`, the named-TAP path the 05-03 fd handoff
   replaces. When 05-02 activates the body, it will stop at that precondition,
   not at the launch filter. Either the body moves to 05-03 or later, or 05-02
   carries the dependency. That is a roadmap/TS decision, so the marker is left
   unchanged.
3. **S-ND295-00 `production_host_owner_boots_only_after_real_shared_identity_is_exact` (active) fails on production.**
   C-12: `DNS client task joins: JoinError::Panic(Id(49), "shared DNS reply: Os { code: 11, kind: WouldBlock, message: \"Resource temporarily unavailable\" }", ...)` (:351:6).
   C-12b, on a fresh host without `ovd-gbr0`, fails at boot:
   `production host owner passes isolated probe, sweep, converge and audit: GuestNetworkBoot(PostconditionMismatch { operation: BridgeObserve, expected: BridgeLinkIdentity { name: "ovd-gbr0", ifindex: Some(2518), link_kind: Bridge }, observed: Some(BridgeLinkIdentity { name: "ovd-gbr0", ifindex: Some(2518), link_kind: Bridge }) })`
   (:311:10). The freshly created bridge was left with MAC `b6:f2:51:ad:47:ae`,
   failing the MAC/up check at `guest_network.rs` ~4014. C-12c, with the
   bridge present, repeats the DNS no-reply. The same fresh-bridge refusal hit
   the first native boot after cleanup (N-04, S-ND295-68 contrast body). Not
   test-fixable.
4. **S-ND295-70 `mtls_intercept_equivalence::{both_installs_hand_back_a_guard_that_releases_cleanly, re_installing_the_same_capture_converges_and_both_guards_release_cleanly}` (active, RETARGETED) fail.**
   C-14: `install_outbound against a live veth and a live leg-F must hand back a guard: NftRuleInstallFailed { op: "shared-owner-required", source: Nft { op: "shared-owner-required", source: Custom { kind: NotConnected, error: "allocation source admission requires the shared owner" } } }`
   (:436:14 and :502:14). `HostMtlsIntercept::install_outbound` refuses unless
   `converge_shared` has run (staged `9cf2b372`). The `MtlsIntercept`
   trait's documented preconditions for `install_outbound` include no such
   step, and `SimMtlsIntercept` installs without it. The equivalence harness is
   reporting a real host/sim/contract divergence. Resolving it needs a contract
   decision or a production change, not a test edit.

### Predictions contradicted

- S-ND295-39 was expected GREEN at authoring (kernel contract pin). It cannot
  run before 05-03 because its driving port `attach_tap_queue` is the 05-03
  scaffold. It is classed as a preceding-step gap.
- S-ND295-42 `the_deny_list_equals_the_measured_ioctl_numbers_and_the_audit_constants`
  was listed as "may be GREEN at scaffold". It is RED, because it builds its
  program through the 05-02 `for_target` scaffold.
- Unpredicted genuine passes, now active: S-ND295-48
  `absent_real_objects_preserve_the_operation_specific_source_family`,
  S-ND295-70 `a_destroyed_host_listener_ends_its_pending_accept_with_an_accept_failure`,
  and S-ND295-66 `a_launch_that_fails_leaves_no_tap_and_no_queue_holder`. The
  native S-ND295-66 row was expected RED.

### Observations for DELIVER

- Every native body that needs a running guest ends as nextest `TIMEOUT` at
  the default 120 s. Its precondition panic fires at 30 to 90 s, and teardown
  then overruns. The panic text above is the classification evidence. Bodies
  that wait up to 90 s for Running may need a slow-timeout override once
  guests boot. The nextest configuration was not changed.
- The S-ND295-52 bodies (06-04) reach their oracle, but today's difference is
  only the missing 06-03 lease events (`LeaseRetired`/`LeaseReleased`). Their
  06-04-specific RED shows only after 06-03 lands.
- The S-ND295-64 fixture adds its netns before `TimeWaitTopology` exists, so a
  provisioning failure leaks the namespace.

### Test-side changes in this run

| File | Change | Why |
|---|---|---|
| `crates/overdrive-bpf/tests/integration/guest_tcx_classifier_test_run.rs` | reset closure treats `SyscallError{bpf_map_delete_elem, ENOENT}` as already-clear; the two "bare Ethernet header" rows use a 14-byte header with EtherType `0x88b5` (`bare_ethernet_header`) | aya 0.13.1 `HashMap::remove` reports an absent key as the raw syscall error, never `KeyNotFound`. The pinned kernel refuses an IPv4-EtherType skb shorter than 34 bytes (`EINVAL`, probed: 14/15/20/33 refused, 34 accepted; any other EtherType at 14 accepted), so those rows never reached the classifier |
| `crates/overdrive-host/src/vmm.rs` | `ChildRun::records` matches the report tag anywhere on a line | under `--nocapture` libtest prints `test <name> ... ` without a newline, so the child's first record shared that line and was dropped; the unfiltered controls failed on a missing record |
| `crates/overdrive-host/src/vmm/launch_seccomp.rs` (test module) | `u16::try_from(program.len()).is_ok()` for `program.len() <= usize::from(u16::MAX)` | x86_64 clippy `checked_conversions` in the S-ND295-42 body (aarch64 clippy never compiles it). Clippy through the metal runner is blocked by the repo hook and Lima cannot cross-clippy x86_64 (no musl C toolchain), so the fix is clippy's own suggestion, verified to compile and run on metal |
| `crates/overdrive-cli/tests/integration/intercept_mark_fail_closed.rs` | the raw-SYN crafter declares its libc symbols; peer namespace `nd64-<pid>`; new fixed-port static peer client; the host `TIME_WAIT` entry's `rcv_nxt`/`ts_recent` read from the peer's FIN on the host veth; stale/newer probes are `rcv_nxt ∓ 100000` with `TSval = ts_recent + 1000`; well-formed TS option; the crafter's `IP_HDRINCL` result is checked; file formatted with `rustfmt` | bare `rustc` links no `libc` crate. `nd295-64-<pid>-h` exceeded 15-byte `IFNAMSIZ`. `bash /dev/tcp` never bound source port 51000, and the constant sequence numbers/TSval against a random live entry made each control a coin flip |
| `crates/overdrive-dataplane/src/guest_tcx.rs` | marker removed from `absent_real_objects_preserve_the_operation_specific_source_family` | genuine pass |
| `crates/overdrive-worker/tests/integration/mtls_intercept_install.rs` | marker removed from `a_destroyed_host_listener_ends_its_pending_accept_with_an_accept_failure` | genuine pass |
| `crates/overdrive-cli/tests/integration/shared_network_native_faults.rs` | marker removed from `a_launch_that_fails_leaves_no_tap_and_no_queue_holder` | genuine pass |

`rustfmt --check` is clean on every changed file. `cargo clippy -D warnings`
is clean for `overdrive-bpf`, `overdrive-dataplane`, `overdrive-worker` and
`overdrive-cli` tests (Lima, `integration-tests`, `kvm-tests` for the CLI).

### Baseline failures (not `#295` DISTILL bodies)

- C-01 `overdrive-bpf` `xdp_reverse_nat_redirect_neigh::reverse_path_redirects_via_neigh_on_fib_hit`: `expected XDP_REDIRECT (=4) on FIB hit, got 2` (:563:5).
- C-01 `overdrive-bpf` `xdp_reverse_nat_udp::udp_response_source_rewritten_to_vip_on_reverse_nat_hit`: `expected reverse-NAT egress verdict XDP_REDIRECT (=4) on a proto=17 FIB hit, got 2` (:470:5).
- C-12 `mtls_install_fail_closed::start_allocation_install_failure_supersedes_running_with_failed` and `restart_allocation_install_failure_supersedes_running_with_failed`: `S-MIF-04/05 A-1': the Failed row must carry MtlsInterceptInstallFailed(stage=leg_f_bind) … got Some(MtlsInterceptInstallFailed { stage: "shared_owner", detail: "shared mTLS owner unavailable" })` (:1383:5).
- C-12 `mtls_install_fail_closed::restart_driver_stop_failure_retains_mtls_and_network_protection`: `assertion left == right failed` left `[AllocationId("restart-abort-driverstop-1"), AllocationId("restart-abort-driverstop-0")]` right `[AllocationId("restart-abort-driverstop-0")]` (:2142:5).
- N-03 `outbound_enforce_substrate_splice::outbound_enforce_substrate_bidirectional_splice_zero_copy`: `start_alloc must install the iifname egress rule in the shared chain, got:` (empty; :1297:5).

### Per-body results — the 182 pending-marked bodies

Seeds appear inline where the body is seeded. Rows that share a failure
message repeat it verbatim.

| Scenario | Marker step | Body (`file::fn`) | Run | Classification | First failing line / message (verbatim) |
|---|---|---|---|---|---|
| S-ND295-01 | 10-01 | `overdrive-cli/…/guest_stack_mtls_egress.rs::microvm_dials_a_mesh_peer_by_name_and_receives_the_reply` | N-08 | RED — preceding-step gap (05-03) | precondition `workload server did not reach Running within 30s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-01 | 10-01 | `overdrive-cli/…/guest_stack_mtls_egress.rs::the_guests_first_mesh_dial_is_born_intercepted_no_cleartext_escapes` | N-08 | RED — preceding-step gap (05-03) | precondition `workload server did not reach Running within 30s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-01 | 10-01 | `overdrive-cli/…/guest_stack_mtls_egress.rs::the_guests_mesh_traffic_travels_the_peer_wire_as_mtls_never_in_the_clear` | N-08 | RED — preceding-step gap (05-03) | precondition `workload server did not reach Running within 30s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-04 | 06-03 | `overdrive-control-plane/src/guest_network.rs::assignment_replay_release_and_reuse_match_the_smallest_free_model` | C-10 | RED | `not yet implemented: RED scaffold: D-295-R8 GuestAddressPool::observe — DELIVER step 06-03` (guest_network.rs:614:9) |
| S-ND295-04 | 06-03 | `overdrive-control-plane/src/guest_network.rs::retirement_is_monotonic_and_a_retiring_lease_still_counts` | C-10 | RED | `not yet implemented: RED scaffold: D-295-R7 GuestAddressPool::retire — DELIVER step 06-03` (guest_network.rs:606:9) |
| S-ND295-05A | 06-03 | `overdrive-control-plane/…/guest_attachment_pool_per_server.rs::a_killed_restart_starts_with_an_empty_pool` | C-12 | RED | `a server restarted after being killed starts with an empty pool; specs [AllocationSpec { alloc: AllocationId("alloc-pool-after-kill-0") … network: Some(GuestNetworkAssignment { address: 100.95.0.3, tap: "ovd-tp-0003" …` (:228:5) |
| S-ND295-05A | 06-03 | `overdrive-control-plane/src/guest_network.rs::admission_refuses_at_the_cap_over_held_leases_for_every_retiring_mix` | C-10 | RED | `not yet implemented: RED scaffold: D-295-R7 GuestAddressPool::retire — DELIVER step 06-03` (:606:9) and `… observe — DELIVER step 06-03` (:614:9) |
| S-ND295-05B | 07-03 | `overdrive-control-plane/src/guest_network.rs::observe_reads_occupancy_and_requested_leases_in_one_snapshot_and_changes_nothing` | C-10 | RED — preceding-step gap (06-03) | `not yet implemented: RED scaffold: D-295-R7 GuestAddressPool::retire — DELIVER step 06-03` (guest_network.rs:606:9) |
| S-ND295-05B | 07-03 | `overdrive-core/…/netns_density_placement_cap.rs::placement_refuses_exactly_when_held_attachments_reach_the_cap` | C-02 | RED | `Test failed: held 16384 (retiring 0) at or above the cap 16384 must refuse with NoCapacity whatever the 1 own Running rows and resources; got Ok(NodeId("nd295-node"))` (:113:1; minimal input held 16384, retiring 0, own_row_count 1) |
| S-ND295-05C | 07-03 | `overdrive-reconcilers/src/workload_lifecycle.rs::a_due_restart_counts_its_predecessor_and_reclaims_it_first_at_the_cap` | C-03 | RED | `Test failed: at the cap (held 16386) with no predecessor lease nothing is emitted: [RestartAllocation { alloc_id: AllocationId("alloc-nd295-svc-0") …` (:3020:5) |
| S-ND295-05C | 07-03 | `overdrive-reconcilers/src/workload_lifecycle.rs::a_raced_restart_refusal_consumes_no_restart_budget` | C-03 | RED | `Test failed: assertion failed: (left == right)` left `[RestartAllocation { alloc_id: AllocationId("alloc-nd295-svc-0") … alloc-nd295-svc-2 …}]` right `[ReclaimAllocationN…` (:3020:5) |
| S-ND295-05D | 07-03 | `overdrive-sim/…/netns_density_node_admission.rs::node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads` | C-17 | RED — preceding-step gap (06-03) | `seed=186055177052160001: harness precondition failed (not a contract verdict): f15661's lease was never released after stop: no guest_network.lease_released { alloc } event for its allocation within 6 evaluations` (:818:9; 251 s) |
| S-ND295-05E | 06-03 | `overdrive-control-plane/src/action_shim/mod.rs::a_refused_restart_successor_still_cleans_up_its_predecessor_once` | C-10 | RED | `not yet implemented: RED scaffold: D-295-R7 GuestAddressPool::retire — DELIVER step 06-03` (guest_network.rs:606:9) |
| S-ND295-05E | 06-03 | `overdrive-control-plane/src/action_shim/mod.rs::admission_refusal_writes_nothing_and_reports_held_and_retiring` | C-10 | RED | `not yet implemented: RED scaffold: D-295-R7 GuestAddressPool::retire — DELIVER step 06-03` (guest_network.rs:606:9) |
| S-ND295-06 | 06-03 | `overdrive-control-plane/…/netns_density_guest_network.rs::provision_refusal_stops_before_driver_start_and_preserves_the_typed_owner_cause` | C-11 | RED | `the lease is retired before the owner's teardown and released only after it succeeds` left `[Owner(TapCreate)]` right `[Owner(TapCreate), LeaseRetired { alloc: "nd295-provision-refused" }, Owner(TapDelete), LeaseReleased { … }]` (:925:5) |
| S-ND295-07 | 07-01 | `overdrive-control-plane/…/netns_density_guest_network.rs::teardown_failure_holds_the_lease_until_retry_completes_then_allows_exact_address_reuse` | C-11 | RED | `driver.stop → lease_retired → element removal → teardown, and no lease_released` left `[DriverStop { alloc: "nd295-predecessor", outcome: Stopped }, Owner(TapDelete)]` right `[DriverStop …, LeaseRetired …, ElementRemoval { source: 100.95.0.2, destinations: {100.95.0.2:53, 100.95.0.2:8080}, outcome: Removed }, Owner(TapDelete)]` (:976:5) |
| S-ND295-07B | 07-01 | `overdrive-control-plane/…/shared_element_cleanup_failure.rs::shared_element_cleanup_failure_deletion_rejected_retains_retirement_and_address` | C-12 | RED | `fallible element cleanup contract violated for DeletionRejected: ["the allocation stop fails with MtlsStop(ElementRemoval) for the stopping allocation", "&*source is exactly the InterceptError the element model injected", "the allocation stop is not reported converged while its elements remain", "a successor registration on the same guest address is refused until cleanup completes", …]` (:1202:5) |
| S-ND295-07B | 07-01 | `overdrive-control-plane/…/shared_element_cleanup_failure.rs::shared_element_cleanup_failure_readback_failed_retains_retirement_and_address` | C-12 | RED | `fallible element cleanup contract violated for ReadBackFailed: [same clause list]` (:1202:5) |
| S-ND295-10 | 09-01 | `overdrive-control-plane/…/shared_guest_network_startup.rs::deliberate_link_loss_reaches_default_drop_and_the_exact_production_audit_cause` | C-12 | RED — preceding-step gap (06-01) | `not yet implemented: RED scaffold: D-295-R22 debug_msg_mask — DELIVER step 06-01` (overdrive-netlink ethtool.rs:257:5), reached through the production owner |
| S-ND295-11 | 06-02 | `overdrive-control-plane/src/guest_network.rs::early_provision_failure_without_tap_skips_attachment_query` | C-10 | RED | `expected TapObserve mismatch Tap { name: "ovd-tp-0002", ifindex: None, … owner_uid: Some(0) } vs None, got PostconditionMismatch { … owner_uid: Some(4200) }, observed: None }` (guest_network.rs:6900:9) |
| S-ND295-11 | 06-02 | `overdrive-control-plane/src/guest_network.rs::every_egress_and_debug_mask_provision_failure_refuses_publication` | C-10 | RED | `AttachFails: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-em0", … owner_uid: Some(4200) }, observed: Some(… owner_uid: Some(0) …) }` (:7650:51) |
| S-ND295-11 | 06-02 | `overdrive-control-plane/src/guest_network.rs::every_incompatible_tap_or_bridge_identity_refuses_owner_publication` | C-10 | RED | `expected TapObserve mismatch Tap { name: "ovd-tp-0002", ifindex: None, … owner_uid: Some(0) } vs None, got PostconditionMismatch { … owner_uid: Some(4200) }, observed: None }` (guest_network.rs:6900:9) |
| S-ND295-11 | 06-02 | `overdrive-control-plane/…/shared_guest_network_startup.rs::ordinary_provision_reads_back_the_complete_attachment_down_before_injected_vmm_start` | C-12 | RED — preceding-step gap (06-01) | `not yet implemented: RED scaffold: D-295-R22 debug_msg_mask — DELIVER step 06-01` (overdrive-netlink ethtool.rs:257:5), reached through the production owner |
| S-ND295-11 | 06-02 | `overdrive-control-plane/src/guest_network.rs::provision_reads_every_attachment_fact_before_reporting_success` | C-10 | RED | `every leaf effect and read-back is exact: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-s11", … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:7009:38) |
| S-ND295-11 | 06-02 | `overdrive-control-plane/src/guest_network.rs::rollback_retry_skips_attachment_query_after_tap_removal` | C-10 | RED | `the first typed cleanup error is returned, got PostconditionMismatch { operation: TapObserve, expected: … owner_uid: Some(4200) }, observed: Some(… owner_uid: Some(0) …) }` (:7490:9) |
| S-ND295-12 | 06-02 | `overdrive-control-plane/src/guest_network.rs::every_teardown_leaf_failure_continues_cleanup_and_retry_reaches_the_exact_complement` | C-10 | RED | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-12 | 06-02 | `overdrive-control-plane/src/guest_network.rs::teardown_converges_on_every_absent_part_singly_and_together` | C-10 | RED | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-12 | 06-02 | `overdrive-control-plane/…/shared_guest_network_startup.rs::two_attachment_teardown_releases_last_and_preserves_the_unrelated_attachment_byte_equal` | C-12 | RED — preceding-step gap (06-01) | `not yet implemented: RED scaffold: D-295-R22 debug_msg_mask — DELIVER step 06-01` (overdrive-netlink ethtool.rs:257:5), reached through the production owner |
| S-ND295-13A | 08-02 | `overdrive-sim/src/invariants/netns_density_boot_order.rs::reclamation_completes_before_stale_shared_network_sweep_for_every_seeded_prior_vm` | C-15 | RED | `seed=9114885741557484504: boot never cleared the stale intercept members InterceptMembers { managed_guest_ips: {100.95.148.74}, outbound_sources: {100.95.148.74}, inbound_destinations: {10.98.9.206:1163} }: no converge_allocation_elements(∅) call` (netns_density_boot_order.rs:320:9) |
| S-ND295-13C | 08-02 | `overdrive-cli/…/serve_killed_restart_boot_clear.rs::a_killed_serve_reboot_reclaims_clears_stale_intercept_members_then_admits` | N-05 | RED — preceding-step gap (05-03) | precondition `workload boot-clear-residue did not reach Running within 60s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1) }) … error: Some("  3: Cannot create virtio-net device\n  4: Failed to open taps\n  5: Enabling tap interface failed\n  6: Ioctl failed (35092)\n  7: Operation not permitted (os error 1)")` (vm_walking_skeleton.rs:573:9; 71.3 s) |
| S-ND295-13D | 08-02 | `overdrive-worker/…/netns_density_shared_owner.rs::a_failed_member_clear_refuses_startup_without_publication` | C-14 | RED | `Refuse { errno: 16 }: expected BootMemberClear carrying the clear's cause, got Intercept { source: NftSharedReplaceFailed { …` (:1819:17) |
| S-ND295-13D | 08-02 | `overdrive-worker/…/netns_density_shared_owner.rs::a_fresh_owner_clears_stale_members_before_reading_the_program` | C-14 | RED | `the fresh owner converges the members to empty before any other port call, so before it reads the program; journal: [ObserveShared, BindTransparent(127.0.0.1:0), BindTransparent(127.0.0.1:0), ConvergeShared { …` (:1749:5) |
| S-ND295-19 | 09-01 | `overdrive-control-plane/src/lib.rs::published_wrong_shared_target_retries_on_production_cadence_and_emits_one_typed_fail_stop` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_allocation_stop_ends_a_relay_classified_during_its_claim_wait` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_allocation_stop_joins_a_passthrough_child` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_allocation_stop_joins_an_inflight_enforce_child` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_completed_enforce_handle_is_torn_down_not_orphaned` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_owner_shutdown_ends_a_live_relay` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_replacement_shutdown_waits_for_the_same_authoritative_teardown` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_same_owner_reinstall_failure_keeps_readiness_closed_until_retry` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-20 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::shared_same_owner_reinstall_waits_for_prior_teardown_before_readiness` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-21 | 07-01 | `overdrive-reconcilers/src/workload_lifecycle.rs::service_projection_keeps_first_tcp_order_deduplicates_tcp_and_excludes_udp` | C-03 | RED | `assertion left == right failed` left `[8080, 53, 8080, 8443, 8080]` right `[8080, 8443]` (workload_lifecycle.rs:2055:9) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::a_double_failure_raises_no_tap_before_every_owner_is_repaired` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::a_policy_route_loss_is_repaired_with_live_members_and_the_guard_is_relinquished` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::a_worker_only_repair_still_restores_quiesced_taps_and_reopens` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::an_activation_in_flight_waits_for_reopen_and_raises_once` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::dns_task_loss_closes_new_commands_and_recovers_through_a_fresh_responder` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::every_component_loss_is_detected_and_recovered_through_its_owner` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::kernel_path_components_quiesce_and_listener_or_dns_loss_never_does` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29A | 09-01 | `overdrive-control-plane/src/lib.rs::the_gate_is_never_open_while_quiescence_is_latched` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::audit_damage_while_open_kills_only_that_vm_and_keeps_admission_open` | C-12 | RED — preceding-step gap (05-01) | `clause C6c-damage-while-open-kills-only-that-vm … 2 of 2 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::every_component_loss_is_detected_within_one_audit_and_closes_admission` | C-12 | RED — preceding-step gap (05-01) | `clause C2-detection-and-admission-closure … 24 of 24 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::healed_owner_after_fail_stop_cannot_reopen_admission` | C-12 | RED — preceding-step gap (05-01) | `clause C7a-post-fail-stop-heal-cannot-reopen … 22 of 22 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::healthy_node_audits_the_shared_owner_every_second` | C-12 | RED — preceding-step gap (05-01) | `clause C1-one-second-audit-cadence … 2 of 2 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::healthy_owner_keeps_admission_open_without_recovery_effects` | C-12 | RED — preceding-step gap (05-01) | `clause C0-healthy-preservation … 2 of 2 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::in_flight_success_after_the_deadline_cannot_reopen_admission` | C-12 | RED — preceding-step gap (05-01) | `clause C7b-in-flight-late-success-cannot-reopen … 22 of 22 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::kernel_path_loss_quiesces_once_before_repair_and_listener_or_dns_loss_never_does` | C-12 | RED — preceding-step gap (05-01) | `clause C3-component-quiescence-rules … 22 of 22 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::repair_runs_through_the_owning_component_on_the_attempt_cadence_and_reopens_once` | C-12 | RED — preceding-step gap (05-01) | `clause C4-exact-owner-repair-cadence-reopen … 22 of 22 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::supervisor_task_loss_is_observed_immediately_and_fail_stops_with_the_latest_snapshot` | C-12 | RED — preceding-step gap (05-01) | `clause C8-supervisor-task-loss-fail-stop … 24 of 24 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::unconfirmed_quiescence_kills_only_the_affected_vm_and_recovery_reopens` | C-12 | RED — preceding-step gap (05-01) | `clause C6a-unconfirmed-quiescence-kills-only-that-vm … 16 of 16 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::undetermined_quiescence_fails_the_node_with_one_typed_request` | C-12 | RED — preceding-step gap (05-01) | `clause C6b-undetermined-quiescence-slice-kill-and-fail-stop … 16 of 16 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-29B | 09-01 | `overdrive-control-plane/…/shared_network_supervisor_recovery.rs::unrepaired_loss_fail_stops_with_one_typed_request_at_the_deadline` | C-12 | RED — preceding-step gap (05-01) | `clause C5-deadline-typed-fail-stop … 22 of 22 cells (red=0, unreached=all); first: seed=0x2953300000000001 … Unreached("precondition R16: required ports not composed at boot (intercept legs None, DNS responders built 0)")` (shared_network_supervisor_recovery.rs:1030:13) |
| S-ND295-30A | 09-01 | `overdrive-control-plane/src/lib.rs::a_failed_per_vm_stop_stops_every_workload_vm_then_fails_the_node` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-30A | 09-01 | `overdrive-control-plane/src/lib.rs::a_killed_vm_leaves_every_later_audit_and_restore_universe` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-30A | 09-01 | `overdrive-control-plane/src/lib.rs::an_already_removed_scope_counts_as_stopped` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-30A | 09-01 | `overdrive-control-plane/src/lib.rs::an_unconfirmed_tap_stops_only_its_vm_and_recovery_reopens` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-30A | 09-01 | `overdrive-control-plane/src/lib.rs::an_undetermined_quiescence_stops_every_workload_vm_then_fails_the_node` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-30A | 09-01 | `overdrive-control-plane/src/lib.rs::every_damaged_per_vm_part_stops_only_that_vm_while_admission_stays_open` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-30B | 10-02 | `overdrive-cli/…/shared_network_native_faults.rs::a_booting_vms_deleted_tap_stops_only_that_vm` | N-06 | RED — preceding-step gap (05-03) | precondition `workload nd295-30bf-survivor did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-30B | 10-02 | `overdrive-cli/…/shared_network_native_faults.rs::a_removed_ingress_link_egress_link_or_guard_member_stops_only_that_vm` | N-06 | RED — preceding-step gap (05-03) | precondition `workload nd295-30bg-bystander did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-30B | 10-02 | `overdrive-cli/…/shared_network_native_faults.rs::a_tap_lost_during_quiescence_stops_only_its_vm_and_the_node_recovers` | N-06 | RED — preceding-step gap (05-03) | precondition `workload nd295-30b-survivor did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-32 | 09-01 | `overdrive-control-plane/src/lib.rs::a_call_pending_at_the_deadline_is_abandoned_uncounted` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-32 | 09-01 | `overdrive-control-plane/src/lib.rs::a_hung_audit_is_a_timeout_failure_of_its_owner` | C-10 | RED | spawned task `not yet implemented: RED scaffold: D-295-R13 run_shared_network_supervisor — DELIVER step 09-01` (lib.rs:1478:9); harness `[OVERDRIVE_SUPERVISOR_SEEDS=0x2953300000000001 cell …]: the supervisor wakes and registers its next wait` (lib.rs:2939:13) |
| S-ND295-33 | 10-03 | `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs::shared_owner_fail_stop_shuts_down_before_a_fresh_handler_reopens_admission` | C-19 | RED — preceding-step gap (09-01) | `admission closes once the loss is detected: Trajectory { observations: [StepObservation { at: 50ms, request: None, admission: Open, recovery: None, boot_closed: false }, … at: 300ms …` (:168:28) — no detection (reproduced gap 2) |
| S-ND295-33 | 10-03 | `tests/conformance/tests/shared_guest_network_fail_stop_recovery.rs::undetermined_tap_quiescence_requests_one_typed_fail_stop_before_a_fresh_handler_reopens` | C-19 | RED — preceding-step gap (09-01) | `admission closes once the loss is detected: Trajectory { observations: [StepObservation { at: 50ms, request: None, admission: Open, recovery: None, boot_closed: false }, … at: 300ms …` (:168:28) — no detection (reproduced gap 2) |
| S-ND295-34 | 05-01 | `overdrive-control-plane/…/dns_responder_bind.rs::the_responder_audit_reads_back_its_socket_and_fails_after_loss` | C-12 | RED | `not yet implemented: RED scaffold: D-295-R16 DnsResponder::audit — DELIVER step 05-01` (dns_responder/mod.rs:118:9) |
| S-ND295-35 | 10-01 | `overdrive-cli/…/vm_walking_skeleton.rs::each_vmm_holds_only_its_own_tap_queue_at_descriptor_three` | N-09 | RED — preceding-step gap (05-03) | precondition `workload nd295-fd-a did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-35 | 10-01 | `overdrive-cli/…/vm_walking_skeleton.rs::two_vm_allocations_share_the_node_bridge_without_per_workload_namespaces` | N-09 | RED — preceding-step gap (05-03) | precondition `workload nd295-a did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-37 | 10-02 | `overdrive-cli/…/vm_walking_skeleton.rs::simultaneous_external_tcx_and_guard_loss_quiesces_the_managed_tap_within_one_second` | N-09 | RED — preceding-step gap (05-03) | precondition `workload shared-guest-network-sink did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-38 | 05-03 | `overdrive-netlink/…/tap_queue_attach.rs::a_down_persistent_tap_hands_over_exactly_one_vnet_header_queue` | C-06 | RED | `not yet implemented: RED scaffold: D-295-R2 attach_tap_queue — DELIVER step 05-03` (client.rs:378:5) |
| S-ND295-38 | 05-03 | `overdrive-netlink/…/tap_queue_attach.rs::every_attach_precondition_violation_is_refused_with_its_own_typed_cause` | C-06 | RED | `not yet implemented: RED scaffold: D-295-R2 attach_tap_queue — DELIVER step 05-03` (client.rs:378:5) |
| S-ND295-39 | 06-02 | `overdrive-netlink/…/tap_queue_attach.rs::a_root_owned_tap_refuses_an_unprivileged_attach_with_eperm` | C-06 | RED — preceding-step gap (05-03) | child: `not yet implemented: RED scaffold: D-295-R2 attach_tap_queue — DELIVER step 05-03` (client.rs:378:5); parent: `an unprivileged uid-4200 process must be refused Attach(EPERM) on a root-owned TAP` left `Panicked` right `RefusedNotPermitted` (:250:5). TS expected GREEN at authoring; not reachable before 05-03 |
| S-ND295-40 | 05-03 | `overdrive-host/src/vmm.rs::a_failed_spawn_releases_the_queue_before_any_cleanup_await` | N-01 | RED | `create must attach the queue before its spawn: CleanupAwait { queue_descriptors: [], clone_present: true, carrier: (0, 1) }` (vmm.rs:2394:9) |
| S-ND295-40 | 05-03 | `overdrive-host/…/vmm_tap_queue_errors.rs::every_tap_queue_error_maps_to_its_vmm_queue_error` | N-02 | RED | `an up TAP refuses the queue attach: VmProcess { control: VmControl { pid: 111940, api_socket: "…/run/nd295-40-notdown/api" } … }` (vmm_tap_queue_errors.rs:232:45) |
| S-ND295-40 | 05-03 | `overdrive-host/src/vmm.rs::mesh_and_non_mesh_launches_preserve_shape_and_attribute_the_actual_launcher` | C-09 (aarch64) / N-01 (x86_64) | RED | `the network argument must name only descriptor 3 and the guest MAC` left `"tap=ovd-tap-002a,mac=02:00:00:00:00:2a,offload_tso=off,offload_ufo=off,offload_csum=off"` right `"fd=[3],mac=02:00:00:00:00:2a,…"` (vmm.rs:1220:9) |
| S-ND295-41 | 05-02 | `overdrive-host/src/vmm.rs::the_launched_child_inherits_exactly_descriptors_zero_to_three` | N-01 | RED | `not yet implemented: RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02` (launch_seccomp.rs:34:9) |
| S-ND295-42 | 05-02 | `overdrive-host/src/vmm/launch_seccomp.rs::the_deny_list_equals_the_measured_ioctl_numbers_and_the_audit_constants` | N-01 | RED | `not yet implemented: RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02` (launch_seccomp.rs:34:9) |
| S-ND295-42 | 05-02 | `overdrive-host/src/vmm/launch_seccomp.rs::the_launch_filter_verdict_partition_is_total` | N-01 | RED | `not yet implemented: RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02` (launch_seccomp.rs:34:9) |
| S-ND295-43 | 05-02 | `overdrive-host/src/vmm.rs::every_denied_request_returns_eperm_on_every_thread_under_the_production_hook` | N-00 → fixed → N-01 | BROKEN → fixed → RED | first run: `13 requests on 4 threads: status ExitStatus(unix_wait_status(0)) … test vmm::launch_seccomp_kernel::launch_seccomp_child_role ... OVERDRIVE_LAUNCH_SECCOMP_REPORT denied thread=main request=SIOCSIFHWADDR errno=22` (vmm.rs:2057:9; 51 of 52 records parsed). After the fix: `not yet implemented: RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02` |
| S-ND295-43 | 05-02 | `overdrive-host/src/vmm.rs::every_filtered_thread_reports_no_new_privs_and_filter_mode` | N-01 | RED | `not yet implemented: RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02` (launch_seccomp.rs:34:9) |
| S-ND295-43 | 05-02 | `overdrive-host/src/vmm.rs::foreign_syscall_abis_end_the_filtered_process` | N-00 → fixed → N-01 | BROKEN → fixed → RED | first run: `unfiltered x32 returns: status ExitStatus(unix_wait_status(0)) … test vmm::launch_seccomp_kernel::launch_seccomp_child_role ... OVERDRIVE_LAUNCH_SECCOMP_REPORT x32 returned=-1 errno=38` left `0` right `1` (vmm.rs:2178:9). After the fix: `not yet implemented: RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02` (launch_seccomp.rs:34:9) |
| S-ND295-43 | 05-02 | `overdrive-host/src/vmm.rs::the_startup_probe_installs_the_exact_launch_program` | N-01 | RED | `not yet implemented: RED scaffold: D-295-R22 check_launch_seccomp — DELIVER step 05-02` (vmm.rs:120:9) |
| S-ND295-44 | 05-02 | `overdrive-host/src/vmm/launch_seccomp.rs::a_target_without_a_program_is_unsupported` | C-09 (aarch64) | RED | `not yet implemented: RED scaffold: D-295-R22 VmmLaunchSeccompFilter::for_target — DELIVER step 05-02` (launch_seccomp.rs:34:9) |
| S-ND295-44 | 05-02 | `overdrive-host/src/vmm.rs::each_launch_filter_probe_cause_maps_to_its_typed_error` | C-09 (aarch64: scaffold vmm.rs:120:9) / N-01 (x86_64) | RED | x86_64: `a failing launch-seccomp stage must reject the probe: ()` (vmm.rs:1168:18); aarch64: `not yet implemented: RED scaffold: D-295-R22 check_launch_seccomp — DELIVER step 05-02` |
| S-ND295-44 | 05-02 | `overdrive-host/src/vmm.rs::launch_on_a_target_without_a_program_is_refused_before_any_effect` | C-09 (aarch64) | RED | `not yet implemented: RED scaffold: D-295-R22 check_launch_seccomp — DELIVER step 05-02` (vmm.rs:120:9) |
| S-ND295-44 | 05-02 | `overdrive-host/src/vmm.rs::vmm_probe_preserves_stage_order_and_rejects_each_injected_ip_execution_failure` | C-09 / N-01 | RED | `an armed NotFound failure of the removed ip tool must never be reached: Err(LaunchToolUnavailable { tool: "ip", source: Custom { kind: NotFound, error: "injected NotFound" } })` (vmm.rs:1050:13) |
| S-ND295-45 | 05-02 | `overdrive-cli/…/vm_walking_skeleton.rs::every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters` | N-09 | BROKEN — remaining (blocker) | precondition `workload nd295-s45 did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-46 | 05-04 | `xtask/src/cloexec_lint.rs::a_rendered_violation_names_its_site_call_and_rule` | C-04 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::render_violation — DELIVER step 05-04` (cloexec_lint.rs:83:5) |
| S-ND295-46 | 05-04 | `xtask/…/cloexec_lint_workspace.rs::a_workspace_without_the_cli_package_is_an_error` | C-05 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_workspace — DELIVER step 05-04` (cloexec_lint.rs:76:5) |
| S-ND295-46 | 05-04 | `xtask/…/cloexec_lint_workspace.rs::an_unparseable_serve_source_fails_the_scan_instead_of_being_skipped` | C-05 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_workspace — DELIVER step 05-04` (cloexec_lint.rs:76:5) |
| S-ND295-46 | 05-04 | `xtask/src/cloexec_lint.rs::an_unparseable_source_is_an_error_not_a_clean_file` | C-04 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_source — DELIVER step 05-04` (cloexec_lint.rs:62:5) |
| S-ND295-46 | 05-04 | `xtask/src/cloexec_lint.rs::an_unresolved_flag_argument_is_rejected` | C-04 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_source — DELIVER step 05-04` (cloexec_lint.rs:62:5) |
| S-ND295-46 | 05-04 | `xtask/…/cloexec_lint_workspace.rs::auxiliary_binaries_outside_the_serve_closure_are_not_scanned` | C-05 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_workspace — DELIVER step 05-04` (cloexec_lint.rs:76:5) |
| S-ND295-46 | 05-04 | `xtask/src/cloexec_lint.rs::cfg_test_items_are_not_scanned` | C-04 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_source — DELIVER step 05-04` (cloexec_lint.rs:62:5) |
| S-ND295-46 | 05-04 | `xtask/src/cloexec_lint.rs::every_rejected_call_family_is_reported_with_its_rule` | C-04 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_source — DELIVER step 05-04` (cloexec_lint.rs:62:5) |
| S-ND295-46 | 05-04 | `xtask/src/cloexec_lint.rs::renamed_imports_and_nix_or_rustix_wrappers_are_resolved_to_their_call` | C-04 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_source — DELIVER step 05-04` (cloexec_lint.rs:62:5) |
| S-ND295-46 | 05-04 | `xtask/src/cloexec_lint.rs::the_exemption_marker_suppresses_only_its_own_line` | C-04 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_source — DELIVER step 05-04` (cloexec_lint.rs:62:5) |
| S-ND295-46 | 05-04 | `xtask/…/cloexec_lint_workspace.rs::the_serve_closure_creates_no_inheritable_descriptor` | C-05 | RED | `not yet implemented: RED scaffold: OBL-295-CLOEXEC cloexec_lint::scan_workspace — DELIVER step 05-04` (cloexec_lint.rs:76:5) |
| S-ND295-47 | 06-01 | `overdrive-bpf/…/guest_tcx_classifier_test_run.rs::egress_classifier_delivers_only_registered_unicast_and_every_group_frame` | C-01 → fixed → C-01d | BROKEN → fixed → RED | first run: `clear endpoint 1: bpf_map_delete_elem failed` (:630:35); second: `Registered / registered guest unicast, bare Ethernet header BPF_PROG_TEST_RUN: Invalid argument (os error 22)` (:673:45). After both fixes: `Registered / another TAP's guest unicast verdict` left `0` right `2` (:689:17) |
| S-ND295-48 | 06-01 | `overdrive-dataplane/src/guest_tcx.rs::absent_real_objects_preserve_the_operation_specific_source_family` | C-07 | PASS (genuine; marker removed) | PASS — eight source-family assertions against absent interface/pins ran and hold on today's code; not predicted by TS |
| S-ND295-48 | 06-01 | `overdrive-dataplane/…/guest_tcx_inventory.rs::clean_and_receipted_inventory_observes_all_eight_exact_families` | C-08 | RED | `receipted lifecycle: the eight families never observed [1, 1, 1, 2, 1, 1, 1, 1] (order [EndpointMap, CounterMap, EndpointEntry, TcxProgram, TcxLink, EndpointMapPin, CounterMapPin, TcxLinkPin]) within 5s` (:307:9) |
| S-ND295-48 | 06-01 | `overdrive-dataplane/…/guest_tcx_inventory.rs::retained_unpinned_maps_programs_and_links_survive_handle_release_and_remain_observable` | C-08 | RED | `receipted lifecycle: the eight families never observed [1, 1, 1, 2, 1, 1, 1, 1] (order [EndpointMap, CounterMap, EndpointEntry, TcxProgram, TcxLink, EndpointMapPin, CounterMapPin, TcxLinkPin]) within 5s` (:307:9) |
| S-ND295-48 | 06-01 | `overdrive-dataplane/src/guest_tcx.rs::semantic_counter_vocabulary_maps_to_the_exact_private_array_slots` | C-07 | RED | `CounterSlots denotes exactly nine entries` left `8` right `9` (guest_tcx.rs:1917:9) |
| S-ND295-48 | 06-01 | `overdrive-dataplane/…/guest_tcx_egress_lifecycle.rs::the_egress_classifier_attaches_pins_queries_and_detaches_at_the_egress_point` | C-08 | RED | `exactly one newly loaded SCHED_CLS program named gh295c_egress is expected; found []` (:70:18) |
| S-ND295-49 | 06-01 | `overdrive-netlink/…/tap_debug_msg_mask.rs::a_fresh_tap_reads_zero_and_a_changed_level_reads_back_singly_and_in_the_dump` | C-06 | RED | worker thread `not yet implemented: RED scaffold: D-295-R22 debug_msg_mask — DELIVER step 06-01` (ethtool.rs:257:5); test thread `a fresh TAP's mask reads: Connect { source: Custom { kind: Other, error: "host netlink worker thread panicked" } }` (:49:38) |
| S-ND295-49 | 06-01 | `overdrive-netlink/…/tap_debug_msg_mask.rs::an_absent_device_is_reported_with_its_original_cause` | C-06 | RED | same scaffold (ethtool.rs:257:5); `an absent device must fail debug-get with ENODEV, got Err(Connect { … "host netlink worker thread panicked" })` (:90:18) |
| S-ND295-50 | 06-02 | `overdrive-control-plane/src/guest_network.rs::every_node_level_audit_failure_names_its_matrix_component_first` | C-10 | RED | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-50 | 06-02 | `overdrive-control-plane/src/guest_network.rs::every_per_allocation_damage_is_named_only_when_the_node_is_healthy` | C-10 | RED | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-50 | 06-04 | `overdrive-control-plane/src/guest_network.rs::a_condemned_allocation_leaves_every_later_audit_and_restore_universe` | C-10 | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-51 | 06-04 | `overdrive-control-plane/src/guest_network.rs::activation_reads_every_protection_fact_before_reporting_success` | C-10 | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-51 | 06-04 | `overdrive-control-plane/src/guest_network.rs::activation_under_a_latch_or_condemnation_changes_nothing` | C-10 | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-51 | 06-04 | `overdrive-control-plane/src/guest_network.rs::quiescence_reports_every_unconfirmed_tap_and_condemns_it` | C-10 | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-51 | 06-04 | `overdrive-control-plane/src/guest_network.rs::restore_raises_only_quiesced_active_taps_in_order_and_clears_the_latch_last` | C-10 | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { … owner_uid: Some(4200) }, observed: Some(Tap { … owner_uid: Some(0) }) }` (guest_network.rs:6825:37) — the 06-02 owner-uid change |
| S-ND295-52 | 06-04 | `overdrive-control-plane/…/mtls_install_fail_closed.rs::activation_of_a_condemned_allocation_takes_the_failure_projection` | C-12 | RED | left `[Provision, DriverStart, InterceptInstalled, InstallSuccessEvent, Activate, DriverStop, ElementRelease, Teardown]` right `[… DriverStop, LeaseRetired, ElementRelease, Teardown, LeaseReleased]` (:1466:5; the diff is the 06-03 lease events) |
| S-ND295-52 | 06-04 | `overdrive-control-plane/…/mtls_install_fail_closed.rs::restart_running_write_rejection_tears_down_network_and_releases_slot` | C-12 | RED | `restart: exactly one LeaseRetired for running-write-reject-restart-successor; journal [Provision, DriverStart, DriverStop, Teardown]` left `0` right `1` (:1246:9) |
| S-ND295-52 | 06-04 | `overdrive-control-plane/…/mtls_install_fail_closed.rs::start_running_write_rejection_tears_down_network_and_releases_slot` | C-12 | RED | `fresh start: exactly one LeaseRetired for running-write-reject-start; journal [Provision, DriverStart, DriverStop, Teardown]` left `0` right `1` (:1246:9) |
| S-ND295-52 | 06-04 | `overdrive-control-plane/…/mtls_install_fail_closed.rs::tap_activation_failure_stops_vmm_cleans_mtls_and_network_and_dominates_running` | C-12 | RED | left `[Provision, DriverStart, InterceptInstalled, InstallSuccessEvent, Activate, DriverStop, ElementRelease, Teardown]` right `[… DriverStop, LeaseRetired, ElementRelease, Teardown, LeaseReleased]` (:1466:5; the diff is the 06-03 lease events) |
| S-ND295-53 | 06-04 | `overdrive-sim/…/netns_density_activation_order.rs::a_latched_activation_retries_after_reopen` | C-16 | RED | `not yet implemented: RED scaffold: D-295-R5 activation wait on a latched quiescence — DELIVER step 06-04` (action_shim/mod.rs:1693:13) |
| S-ND295-53 | 06-04 | `overdrive-sim/…/netns_density_activation_order.rs::activation_during_recovery_runs_once_after_reopen_without_a_failed_row` | C-16 | RED | `seed=186055316088029185: activation ran while the gate is Recovering (poll 0): TapSetUp at owner call indices [1] outside the test's quiesce/restore brackets []; calls [TapCreate, TapSetUp]` (:754:13) |
| S-ND295-53 | 06-04 | `overdrive-sim/…/netns_density_activation_order.rs::fail_stop_withholds_activation_and_the_command` | C-16 | RED | `seed=186055316088029185: activation ran while the gate is Recovering (poll 0): TapSetUp at owner call indices [1] outside the test's quiesce/restore brackets []; calls [TapCreate, TapSetUp]` (:754:13) |
| S-ND295-54 | 07-01 | `overdrive-worker/src/mtls_intercept_worker.rs::a_stop_after_owner_shutdown_began_starts_nothing_and_returns_its_shutdown_entry` | C-13 | RED | `a's attempt fails: ()` (:6477:56) |
| S-ND295-54 | 07-01 | `overdrive-worker/src/mtls_intercept_worker.rs::allocation_stop_surfaces_teardown_failure_and_retry_converges` | C-13 | RED — preceding-step gap (05-01) | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-54 | 07-01 | `overdrive-worker/src/mtls_intercept_worker.rs::callers_joined_on_one_failed_stop_receive_equal_failures_with_shared_sources` | C-13 | RED | `not observed within 2 s: the first caller's attempt enters element removal` (:6126:13) |
| S-ND295-54 | 07-01 | `overdrive-worker/…/shared_intercept_members.rs::convergent_removal_with_a_pre_absent_member_and_batch_rejection_preserves_state` | C-14 | RED | `not yet implemented: RED scaffold: D-295-R10 remove_allocation_elements — DELIVER step 07-01` (mtls_intercept_port.rs:1121:9) |
| S-ND295-54 | 07-01 | `overdrive-worker/src/mtls_intercept_worker.rs::element_removal_failure_keeps_the_retiring_record_until_a_retry_converges` | C-13 | RED | `the removal failure surfaces: ()` (:6282:50) |
| S-ND295-54 | 07-01 | `overdrive-control-plane/…/server_lifecycle.rs::graceful_shutdown_propagates_worker_failure_without_a_retry_capability` | C-12 | RED | `typed worker teardown failure reaches the server caller: ()` (server_lifecycle.rs:375:26) |
| S-ND295-54 | 07-01 | `overdrive-worker/src/mtls_intercept_port.rs::remove_allocation_elements_deletes_only_present_requested_members` | C-13 | RED | `not yet implemented: RED scaffold: D-295-R10 remove_allocation_elements — DELIVER step 07-01` (mtls_intercept_port.rs:1121:9) |
| S-ND295-54 | 07-01 | `overdrive-worker/src/mtls_intercept_worker.rs::the_first_stop_after_a_failure_starts_one_retry_for_simultaneous_callers` | C-13 | RED | `the first attempt fails: ()` (:6405:63) |
| S-ND295-55 | 07-02 | `overdrive-control-plane/src/action_shim/validate.rs::a_reclaim_beside_another_action_for_the_same_allocation_is_rejected` | C-10 | RED | `a reclaim beside StartAllocation for the same allocation must be ConflictingAllocationReclaim, got Ok(())` (validate.rs:732:30) |
| S-ND295-55 | 07-03 | `overdrive-reconcilers/src/workload_lifecycle.rs::a_failing_reclaim_backs_off_one_second_and_never_stops` | C-03 | RED | left `[]` right `[AllocationId("alloc-nd295-svc-0")]`: `attempt 1 at UnixInstant(500s): the reclaim is emitted with no attempt ceiling` (:3406:5) |
| S-ND295-55 | 07-03 | `overdrive-reconcilers/src/workload_lifecycle.rs::a_pending_restart_owns_its_predecessor_until_due_at_the_cap` | C-03 | RED | left `[]` right `[AllocationId("alloc-nd295-svc-0")]`: `held 2, due true: the leftover is always reclaimed; the predecessor only when due at the cap` (:3406:5) |
| S-ND295-55 | 07-03 | `overdrive-reconcilers/src/workload_lifecycle.rs::every_return_path_reclaims_leased_unowned_finished_allocations` | C-03 | RED | left `[]` right `[AllocationId("alloc-nd295-svc-0")]`: `StopBranchStopping: reclaim set` (:3406:5) |
| S-ND295-56 | 07-02 | `overdrive-control-plane/…/netns_density_guest_network.rs::a_failed_reclaim_step_keeps_the_lease_for_the_next_attempt` | C-11 | RED — preceding-step gap (06-03) | `precondition: the lease is retired and still held (steps [Owner(TapCreate), ActivateRefused { alloc: "nd295-reclaim-…" }, DriverStop { … outcome: Stopped }, Owner(TapDelete)])` (:1076:9) |
| S-ND295-56 | 07-02 | `overdrive-control-plane/…/netns_density_guest_network.rs::reclaim_cleans_a_leased_finished_allocation_without_a_row` | C-11 | RED — preceding-step gap (06-03) | `precondition: the lease is retired and still held (steps [Owner(TapCreate), ActivateRefused { alloc: "nd295-reclaim-…" }, DriverStop { … outcome: Stopped }, Owner(TapDelete)])` (:1076:9) |
| S-ND295-56 | 07-02 | `overdrive-control-plane/…/netns_density_guest_network.rs::reclaim_without_a_lease_does_nothing` | C-11 | RED | `not yet implemented: RED scaffold: D-295-R11 ReclaimAllocationNetwork dispatch — DELIVER step 07-02` (action_shim/mod.rs:3463:13) |
| S-ND295-57 | 07-03 | `overdrive-sim/…/netns_density_reclaim.rs::a_not_yet_due_restart_keeps_its_predecessor_at_the_cap` | C-16 | RED | `seed=186055333267898369: at the cap, a successor of v57-0295005700000001 was admitted (journal index 16387) while its predecessor alloc-v57-0295005700000001-1 still held its lease (released at []); the held population would exceed 16384` (:1098:13) |
| S-ND295-57 | 07-03 | `overdrive-sim/…/netns_density_reclaim.rs::leftover_networks_are_reclaimed_until_released_on_every_path` | C-16 | RED | `seed=186055333267898369: leftover leases were not reclaimed until released within 30s of simulated time … restart predecessor alloc-r57-…: reclaims [], released [] \| stopped workload …: reclaims [], released [] \| deleted workload …: reclaims [], released []` (:862:13) |
| S-ND295-58 | 07-04 | `overdrive-core/…/netns_density_cleanup_pending.rs::cleanup_pending_matches_the_lease_and_row_state_table` | C-02 | RED | `RED scaffold: D-295-R20 cleanup_pending — DELIVER step 07-04` (guest_attachment_view.rs:43:9) |
| S-ND295-59 | 07-04 | `overdrive-control-plane/…/network_cleanup_pending_status.rs::a_stuck_stop_is_reported_cleanup_pending_and_is_not_a_running_replica` | C-12 | RED — preceding-step gap (07-01) | `the stop attempts member removal: not reached within 300 ticks; last AllocStatusResponse { … replicas_running: 0, rows: [… state: Terminated, reason: Some(Stopped { by: Reconciler }) …` (:309:9) |
| S-ND295-59 | 07-04 | `overdrive-control-plane/…/network_cleanup_pending_status.rs::crashed_and_reclaiming_allocations_are_reported_cleanup_pending` | C-12 | RED | `a crashed allocation awaiting cleanup (Failed, admitted lease) is pending: AllocStatusRowBody { alloc_id: "alloc-cleanup-job-0" … state: Failed …` (:489:5) |
| S-ND295-60 | 07-04 | `overdrive-cli/…/render_workload_describe.rs::a_cleanup_pending_allocation_renders_cleanup_pending_with_its_lifecycle_state` | C-18 | RED | `[Service per-allocation table, row 0, lifecycle state Pending]: a cleanup-pending row's State cell must read CleanupPending, never its lifecycle label "Pending"; got row cells "Pending      0          —"` (:1365:13) |
| S-ND295-61 | 08-03 | `overdrive-worker/…/netns_density_shared_owner.rs::a_differently_targeted_program_is_never_rewritten` | C-14 | RED | `a differently targeted program must fail the audit` (:2100:9) |
| S-ND295-61 | 08-03 | `overdrive-worker/…/shared_intercept_members.rs::each_deleted_intercept_object_is_restored_exactly_with_live_allocations` | C-14 | RED — preceding-step gap (08-02) | `not yet implemented: RED scaffold: D-295-R15 observe_shared_state — DELIVER step 08-02` (mtls_intercept_port.rs:1102:9) |
| S-ND295-61 | 08-03 | `overdrive-worker/src/mtls_intercept_worker.rs::every_shared_owner_error_reports_its_one_component` | C-13 | RED | `not yet implemented: RED scaffold: D-295-R15 MtlsSharedOwnerError::component — DELIVER step 08-03` (mtls_intercept_worker.rs:370:9) |
| S-ND295-61 | 08-03 | `overdrive-worker/…/netns_density_shared_owner.rs::member_loss_is_an_ipsets_failure_and_repair_restores_exactly_the_member` | C-14 | RED | `a member deleted from the kernel must fail the audit` (:1924:9) |
| S-ND295-61 | 08-03 | `overdrive-worker/…/netns_density_shared_owner.rs::policy_route_loss_is_repaired_with_live_members_and_the_prior_guard_is_relinquished` | C-14 | RED | `PolicyRoute: the loss must fail the audit` (:2001:13) |
| S-ND295-61 | 08-03 | `overdrive-worker/…/shared_intercept_members.rs::the_intercept_mark_guard_table_is_restored_exactly_with_live_allocations` | C-14 | RED — preceding-step gap (08-02) | `not yet implemented: RED scaffold: D-295-R15 observe_shared_state — DELIVER step 08-02` (mtls_intercept_port.rs:1102:9) |
| S-ND295-62 | 08-01 | `overdrive-cli/…/intercept_mark_fail_closed.rs::marked_guest_tcp_is_neither_forwarded_nor_delivered_without_the_intercept_program` | N-07 | RED — preceding-step gap (05-03) | precondition `workload nd295-probe did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-62 | 08-01 | `overdrive-cli/…/intercept_mark_fail_closed.rs::the_intercept_program_still_catches_marked_tcp_without_the_guard_table` | N-07 | RED — preceding-step gap (05-03) | precondition `workload nd295-probe did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-63 | 08-01 | `overdrive-cli/…/intercept_mark_fail_closed.rs::inbound_tcp_to_a_closed_listener_is_dropped` | N-07 | RED — preceding-step gap (05-03) | precondition `workload server did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-63 | 08-01 | `overdrive-cli/…/intercept_mark_fail_closed.rs::outbound_tcp_after_a_killed_server_is_dropped_while_the_vm_lives` | N-07 | RED — preceding-step gap (05-03) | precondition `workload nd295-probe did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-63 | 08-01 | `overdrive-cli/…/intercept_mark_fail_closed.rs::outbound_tcp_to_a_closed_listener_is_dropped_not_delivered_locally` | N-07 | RED — preceding-step gap (05-03) | precondition `workload nd295-probe did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-64 | 08-01 | `overdrive-cli/…/intercept_mark_fail_closed.rs::a_newer_sequence_reconnect_into_time_wait_is_recorded_after_both_controls` | N-07 → N-07b…e | BROKEN — remaining (blocker) | `rustc must build nd295-raw-syn` (guest_stack_mtls_egress.rs:393:5; stderr `error[E0433]: cannot find module or crate libc in this scope`) → `command ["ip", "link", "add", "nd295-64-124084-h", "type", "veth", "peer", "name", "nd295-64-124084-p"] succeeded` (:1121:5; stderr `"nd295-64-124084-p" is wrong: "name" not a valid ifname`) → `the stale-ISN probe gets a bare ACK and no SYN-ACK; the substate and sequence gates hold` left `SynAck` right `BareAck` (:788:5) → after the fixes PASS ×3 (2.5 s), both controls holding; the guest case only re-probes the host-only tuple and asserts nothing (`S-ND295-64 guest TIME_WAIT reconnect outcome (recorded, not absorbed): SynAck`) |
| S-ND295-65 | 05-01 | `overdrive-control-plane/…/required_serve_ports_source_scan.rs::no_optional_switch_gates_protection_dns_or_supervisor_composition` | C-11 | RED | `an optional switch or after-boot replacement gates serve composition (727 declarations, 11 lifecycle parameters, 3 run_server functions scanned): - src/lib.rs:239: AppState.shared_guest_network is Option < Arc < dyn guest_network :: SharedGuestNetworkOwner > > …` (:998:5) |
| S-ND295-66 | 10-02 | `overdrive-cli/…/shared_network_native_faults.rs::a_launch_that_fails_leaves_no_tap_and_no_queue_holder` | N-06 | PASS (genuine; marker removed) | PASS (1.8 s) — the missing-kernel launch reaches Failed with no new `ovd-tp-*` TAP and no Cloud Hypervisor holder; not predicted (TS expected native RED) |
| S-ND295-66 | 10-02 | `overdrive-cli/…/shared_network_native_faults.rs::a_stop_converges_when_attachment_parts_are_already_gone` | N-06 | RED — preceding-step gap (05-03) | precondition `workload nd295-66 did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-67 | 10-02 | `overdrive-cli/…/shared_network_native_faults.rs::a_mac_hijack_from_outside_the_vm_steals_nothing_and_the_victim_recovers` | N-06 | RED — preceding-step gap (05-03) | precondition `workload nd295-67-attacker did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-68 | 08-03 | `overdrive-cli/…/serve_lifetime_fail_stop.rs::a_repaired_shared_network_loss_never_ends_serve` | N-04 → rerun N-04b | RED | first boot after cleanup removed ovd-gbr0 hit the fresh-bridge defect (below); rerun: `repaired shared-network loss contract violated: [RED] a repaired loss leaves the serve lifetime pending through the recovery window — lifetime finished within 10s of the loss: true; final outcome: SharedGuestNetworkFailStop { … component: IpRules, cause: RecoveryDeadlineExceeded, attempts: 20, elapsed: 5.028317948s } … [RED] the supervisor requests no fail-stop for a loss it repairs — guest_network.shared_owner_fail_stop events: 2` (:940:5) |
| S-ND295-69 | 10-02 | `overdrive-cli/…/shared_network_native_faults.rs::a_deleted_program_table_is_repaired_with_live_mesh_vms` | N-06 | RED — preceding-step gap (05-03) | precondition `workload server did not reach Running within 90s; last observed row: … state: Failed, reason: Some(VmGuestExitUnreported { vmm_exit_code: Some(1), vmm_signal: None })` (vm_walking_skeleton.rs:573:9), then nextest TIMEOUT at 120 s |
| S-ND295-70 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::a_connection_whose_destination_cannot_be_read_is_dropped_and_the_task_keeps_waiting` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-70 | 05-01 | `overdrive-worker/…/mtls_intercept_install.rs::a_destroyed_host_listener_ends_its_pending_accept_with_an_accept_failure` | C-14 | PASS (genuine; marker removed) | PASS — the accept is first shown pending, `ss -K` destroys the listening tuple, and the accept ends with `InterceptAcceptError::Accept` within 2 s; not predicted by TS (TS: a body that passes on today's listener stays active) |
| S-ND295-70 | 05-01 | `overdrive-sim/src/adapters/mtls_intercept.rs::a_held_address_is_refused_with_eaddrinuse_until_its_last_holder_drops` | C-15 | RED | `bind_transparent's Ok arm returns the socket-free sim listener, not a real socket` (sim mtls_intercept.rs:1190:30) |
| S-ND295-70 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::a_lost_listener_ends_only_its_own_task_and_the_owner_rebinds_the_recorded_address` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-70 | 05-01 | `overdrive-sim/src/adapters/mtls_intercept.rs::an_accept_polled_without_a_runtime_fails_instead_of_panicking` | C-15 | RED | `bind_transparent's Ok arm returns the socket-free sim listener, not a real socket` (sim mtls_intercept.rs:1190:30) |
| S-ND295-70 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::an_idle_accept_task_ends_when_the_last_worker_reference_drops` | C-13 | RED | `the port owns both shared listeners: []` left `0` right `2` (mtls_intercept_worker.rs:6640:9 / 6669:9) |
| S-ND295-70 | 05-01 | `overdrive-sim/src/adapters/mtls_intercept.rs::an_install_fault_leaves_bind_on_its_success_arm` | C-15 | RED | `InstallOutbound fault: bind takes its socket-free Ok arm` left `127.0.0.1:43937` right `127.0.0.1:49152` (:1607:13) |
| S-ND295-70 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::an_unreadable_listener_address_is_reported_by_the_audit` | C-13 | RED | `the port owns exactly the two shared listeners (none before B-7): []` left `0` right `2` (mtls_intercept_worker.rs:6154:9) |
| S-ND295-70 | 05-01 | `overdrive-sim/src/adapters/mtls_intercept.rs::an_unscripted_accept_stays_parked_and_a_cancelled_accept_takes_nothing` | C-15 | RED | `bind_transparent's Ok arm returns the socket-free sim listener, not a real socket` (sim mtls_intercept.rs:1190:30) |
| S-ND295-70 | 05-01 | `overdrive-sim/src/adapters/mtls_intercept.rs::clear_faults_also_disarms_the_bind_slot` | C-15 | RED | `the cleared bind slot takes the socket-free Ok arm` left `127.0.0.1:38801` right `127.0.0.1:49152` (:1572:9) |
| S-ND295-70 | 05-01 | `overdrive-sim/src/adapters/mtls_intercept.rs::each_scripted_outcome_completes_exactly_the_accept_it_names` | C-15 | RED | `bind_transparent's Ok arm returns the socket-free sim listener, not a real socket` (sim mtls_intercept.rs:1190:30) |
| S-ND295-70 | 05-01 | `overdrive-sim/src/adapters/mtls_intercept.rs::fabricated_ports_are_deterministic_non_zero_and_distinct_among_live_listeners` | C-15 | RED | `port 0 takes 49152, then 49153 while the first is held` left `(127.0.0.1:39591, 127.0.0.1:46133)` right `(127.0.0.1:49152, 127.0.0.1:49153)` (:1253:9) |
| S-ND295-70 | 05-01 | `overdrive-worker/src/mtls_intercept_worker.rs::owner_shutdown_ends_both_accept_tasks_releases_both_listeners_and_relinquishes_the_node_guard` | C-13 | RED | `the port owns both shared listeners: []` left `0` right `2` (mtls_intercept_worker.rs:6640:9 / 6669:9) |

### Active `#295` bodies (no pending marker) that PASS — 75

Runs C-01 through C-20, N-03 and N-04; each passed on its first execution.

- `overdrive-bpf` `integration::guest_tcx_classifier_test_run` (1): `classifier_partitions_return_one_verdict_and_advance_one_exact_counter`
- `overdrive-cli` `acceptance::render_workload_describe` (1): `non_pending_allocations_render_byte_identically`
- `overdrive-cli` `integration::serve_lifetime_fail_stop` (3): `a_shared_network_fail_stop_shutdown_is_abandoned_when_the_ten_second_bound_elapses`, `a_shared_network_fail_stop_wins_over_a_ready_interrupt_and_exits_status_one`, `an_operator_interrupt_stops_a_healthy_serve_with_status_zero`
- `overdrive-control-plane` `acceptance::netns_density_guest_network` (1): `scratch_complement_never_fabricates_zero`
- `overdrive-control-plane` `compile_fail` (1): `compile_fail_cases`
- `overdrive-control-plane` `guest_network::scratch_probe_acceptance` (4): `every_cleanup_or_inventory_failure_is_aggregated_after_the_remaining_cleanup`, `every_observed_residue_family_returns_incomplete_with_the_owner_built_complement`, `exercise_transport_and_semantic_failures_are_distinct_for_every_probe_stage`, `primary_and_first_cleanup_failure_are_both_preserved_without_nested_aggregate`
- `overdrive-control-plane` `guest_network::scratch_probe_packet_acceptance` (2): `classifier_runs_precede_close_and_each_stage_is_fresh`, `every_d14_semantic_mismatch_and_lower_source_reaches_the_production_validator`
- `overdrive-control-plane` `integration::mtls_install_fail_closed` (2): `cancelling_dispatch_while_the_exec_release_is_held_drops_that_release`, `tap_activation_occurs_after_intercept_success_and_before_exec_release`
- `overdrive-control-plane` `integration::shared_guest_network_startup` (4): `cleanup_failure_refuses_and_preserves_primary_cleanup_and_observed_residue`, `ordinary_probe_faults_refuse_before_convergence_or_publication`, `production_boot_trace_completes_vm_reclamation_before_stale_sweep_starts`, `production_startup_exercises_classifier_and_detached_guard_before_admission`
- `overdrive-control-plane` `shared_network_task_owner_acceptance` (6): `actual_tokio_exit_matrix_fail_stops_before_returning_the_exact_snapshot`, `dns_replacement_is_refused_from_every_invalid_state_without_spawning`, `dns_replacement_joins_the_old_task_before_spawning_from_live_and_exited_states`, `dns_shutdown_prefers_cooperative_stop_and_awaits_the_bounded_abort_backstop`, `dns_task_owner_classifies_real_exits_and_never_overwrites_a_live_handle`, `explicit_request_is_returned_unchanged_and_intentional_shutdown_is_not_failure`
- `overdrive-control-plane` `shared_network_test_ports::tests` (2): `activate_reports_raised_latched_or_condemned`, `restore_failure_slot_keeps_the_latch`
- `overdrive-core` `acceptance::netns_density_exec_gate` (6): `every_fail_stop_cause_is_closed_and_first_request_wins`, `fail_stop_refuses_waiting_and_future_exec_without_revoking_a_prior_claim`, `generated_operation_sequences_match_the_gate_model`, `illegal_event_from_every_gate_state_is_rejected`, `recovery_closes_new_exec_claims_and_full_read_back_reopens_them`, `shared_guest_network_admission_starts_closed_until_boot_read_back_completes`
- `overdrive-dataplane` `guest_tcx::tests` (6): `a_unique_unreceipted_candidate_is_ambiguous_and_never_an_owned_count`, `capture_failure_keeps_an_observation_identity_and_only_the_first_genuine_source`, `every_locked_aya_map_kind_projects_to_exact_or_opaque_semantics`, `every_receipted_family_returns_one_and_clean_families_return_exact_zero`, `startup_tcp_probe_projects_semantics_and_all_eight_counter_pairs_without_raw_abi`, `wrong_valid_map_properties_remain_opaque_and_schema_mismatch_is_source_less`
- `overdrive-dataplane` `integration::guest_tcx_inventory` (1): `wrong_exact_path_owner_or_valid_map_schema_is_typed_and_never_fabricates_zero`
- `overdrive-netlink` `client::tests` (1): `persistent_tap_and_bridge_projection_preserves_every_observable_identity_field`
- `overdrive-netlink` `ethtool::tests` (1): `debug_message_constants_equal_the_uapi_values`
- `overdrive-reconcilers` `workload_lifecycle::restart_gating_acceptance` (1): `a_restart_that_is_not_yet_due_emits_nothing_at_any_occupancy`
- `overdrive-sim` `adapters::guest_dns::tests` (3): `serve_ends_on_the_first_end_serve_or_stop_even_before_its_first_poll`, `the_audit_slot_is_per_responder_so_a_replacement_audits_clean`, `the_probe_slot_is_shared_by_every_responder_and_is_standing`
- `overdrive-sim` `adapters::guest_network::tests` (2): `scripted_quiescence_outcomes_condemn_each_named_allocation_once`, `standing_owner_controls_preserve_exact_operation_semantics`
- `overdrive-sim` `adapters::mtls_intercept::tests` (2): `arming_one_slot_leaves_the_others_on_their_success_arms`, `clear_faults_disarms_every_slot_and_is_idempotent`
- `overdrive-worker` `integration::mtls_intercept_equivalence` (2): `bound_leg_reports_a_non_zero_kernel_assigned_port`, `two_bound_legs_never_share_a_port`
- `overdrive-worker` `integration::mtls_intercept_install` (6): `an_accepted_connection_is_blocking_and_close_on_exec`, `shared_program_absence_create_readback_idempotence_and_guard_drop`, `shared_program_replaces_only_listener_targets_and_preserves_foreign_complement`, `shared_program_valid_wrong_target_observation_is_non_mutating`, `the_host_listener_reports_a_redirected_inbound_virtual_address_as_local`, `the_host_listener_reports_a_redirected_outbound_original_destination_as_local`
- `overdrive-worker` `integration::netns_density_shared_owner` (10): `initial_leg_f_bind_refusal_returns_to_absent_without_partial_publication`, `leg_c_bind_refusal_closes_the_already_bound_leg_f_and_publishes_no_owner`, `lost_leg_f_rebinds_the_recorded_nonzero_address_before_audit_succeeds`, `occupied_original_leg_f_address_refuses_recovery_without_selecting_another_port`, `owner_shutdown_waits_the_active_claim_then_drains_every_shared_capability_and_listener`, `published_wrong_shared_target_is_observe_only_until_bounded_fail_stop`, `shared_owner_starts_once_audits_and_shutdown_drains_the_owner_tree`, `shared_rule_convergence_refusal_closes_both_sockets_and_publishes_no_tasks_or_guard`, `stop_and_owner_shutdown_during_pending_registration_return_registration_retired_and_drain_once`, `stopping_one_shared_allocation_preserves_the_unrelated_handle_and_complete_listener_owner`
- `overdrive-worker` `integration::outbound_enforce_substrate_splice` (2): `real_owner_shutdown_closes_admission_waits_one_claim_and_drains_every_shared_handle`, `two_real_shared_capabilities_keep_the_unrelated_tls_handle_live_after_one_stops`
- `overdrive-worker` `mtls_intercept_port::shared_program_rollback_acceptance` (4): `runtime_present_wrong_target_and_observe_error_are_non_mutating`, `shared_program_post_commit_failure_rolls_back_source_honestly_for_every_prior`, `shared_program_prior_snapshot_mismatch_preserves_complete_state_and_complement`, `shared_program_replace_refusal_idempotence_and_guard_cleanup_preserve_complete_state_delta`
- `overdrive-worker` `mtls_intercept_worker::tests` (1): `shared_allocation_start_after_owner_shutdown_is_rejected_before_install`

## Phase D — DESIGN-pin follow-up run (B-8 element precondition; fresh-host RCA / N-4)

Executed 2026-09-26 after the architect pinned B-8 (element install/removal
precondition) and the fresh-host RCA (root cause A → REQ-295-LINKMAC, a framing
superseded on 2026-09-28; see § *Superseded classifications* and § *Phase E*), and
after the DISTILL bodies were brought into line with those pins. Run serially,
one command at a time in the foreground, against the working tree at the time
of this run, on the two substrates below. The four phase-C blockers are
addressed: blocker 4 (host/sim/contract divergence on the install call
sequence) is resolved by the B-8 pin, and blocker 3's bridge half is owned by
05-00. Raw logs are under `target/phase-d-295/` (not committed).

### Substrates

| Substrate | `uname -r` | Runner |
|---|---|---|
| Lima | `overdrive` VM, aarch64, cgroup v2, root via the wrapper (quiet, clean before the run; `table ip nat` pre-existing) | `cargo xtask lima run -- …` |
| Native metal | non-virtualized x86_64, KVM preflight + canonical lease on every command | `cargo xtask metal run --no-sync -- …` |

The BPF object was rebuilt clean on Lima (`cargo xtask bpf-build`; the warm
Lima target dir needed a `chown` back to the user first) and carries
`gh295c_egress` and `gh295c_endpoint`.

### Environment and cleanup

- **Lima quiet throughout**; no other workspace's cargo/nextest was present.
- **Lima, after the boot-driving bodies:** the DNS-leg body and the
  bridge-identity body each boot `run_server`, which by design leaves the node
  bridge `ovd-gbr0`, `table bridge overdrive-mtls`, and the
  `mtls-endpoints` pins; they were removed after the run. The bridge-identity
  body's RAII cleanup was then confirmed to leave the node clean.
- **Metal, after the S-ND295-45 timeout:** the timed-out production boot left
  the `ovd-veth-cli`/`ovd-veth-bk` pair, `ovd-gbr0`, `table bridge
  overdrive-mtls`, the `mtls-endpoints` pins, and one `fwmark 0x1 lookup 100`
  rule + table-100 route; all removed. No leaked Cloud Hypervisor, cgroup
  scope, TAP, or netns remained.

### Gate / lint

`cargo check` and `cargo clippy --all-targets --features integration-tests
-- -D warnings` are clean for every touched crate on Lima; `overdrive-cli`
also clean with `kvm-tests`. `cargo check` on metal is clean for
`overdrive-host` (default features) and `overdrive-cli`
(`integration-tests,kvm-tests`). `rustfmt --check` is clean on every changed
file.

### Test-side changes in this run

| File | Change | Why |
|---|---|---|
| `crates/overdrive-worker/tests/integration/mtls_intercept_equivalence.rs` | the two B-8 refusal bodies destructure the `Err` and name the adapter in the panic (`[S-ND295-71][{label}]`) instead of `expect_err(())` | the message told which contract failed but not which adapter; the sim is the diverging side, and the message must say so |
| `crates/overdrive-control-plane/src/guest_network.rs` (`shared_owner_link_address_kernel`, `#[cfg(test)]`) | (d) starts each row from a fresh node + fresh owner; the doc says "a second fresh node", not "re-converges" | the first pins failed on a leaked bpffs pin (`BPF_OBJ_PIN` EEXIST) from the prior row; per-row fresh state isolates the bridge-mismatch contract |
| `crates/overdrive-netlink/tests/integration/managed_link_address.rs` | (b) waits `udevadm wait` before snapshotting each present link; (a) records the create-then-set control before the assertions | the host link manager rewrote a just-created scratch link's MAC before the snapshot (the very race the fix removes); the settled link is the (b) precondition, and (a) records its control even on a RED |
| `crates/overdrive-control-plane/tests/integration/shared_guest_network_startup.rs` | the bridge-identity body drops a `FreshHostCleanup` guard so a failed iteration leaves no foreign-MAC bridge | the repeated-boot body must not leak node state on a mid-loop refusal |

### Per-body results

| Scenario | Marker step | Body (`file::fn`) | Classification | First failing line / message (verbatim) |
|---|---|---|---|---|
| S-ND295-71 | 05-01 | `overdrive-sim/…/mtls_intercept.rs::tests::shared_convergence_records_both_exact_targets_for_non_repairing_observation` | RED | `assertion left == right failed: the dropped guard's identity equals the modeled program and no member exists, so its drop removes the program` left `Some(ConstantRules { … })` right `None` (:878) |
| S-ND295-71 | 05-01 | `…::tests::a_node_guard_dropped_while_a_member_exists_keeps_the_program_and_withdraws_the_record` | RED | `the drop withdrew the record, so an install is refused` (:932) |
| S-ND295-71 | active | `…::tests::converging_to_the_recorded_program_adopts_it_and_keeps_every_member` | PASS (genuine) | — (the sim already adopts an equal identity; kept active) |
| S-ND295-71 | 05-01 | `…::tests::a_program_replacement_is_refused_while_a_member_exists` | RED | `a replacement over a live member is refused: ()` (:1004) |
| S-ND295-71 | 05-01 | `…::tests::a_convergence_from_a_stale_prior_is_refused_and_changes_nothing` | RED | `a stale prior is refused: ()` (:1061) |
| S-ND295-71 | 05-01 | `…::tests::shared_convergence_refuses_in_the_hosts_order` | RED | `a zero port refuses before the armed fault, got NftRuleInstallFailed { op: "observe-shared", … }` (:1106) |
| S-ND295-71 | active | `…::tests::a_failed_convergence_keeps_the_recorded_program` | PASS (genuine) | — (the sim already keeps its `shared_observation` across a scripted `converge_shared` fault) |
| S-ND295-71 | 05-01 | `…::tests::an_armed_install_fault_fires_only_after_the_record_and_port_checks` | RED | `InstallOutbound: before any converge the precondition refuses first, got NftElementUpdateFailed { … }` (:1283) |
| S-ND295-71 | active | `…::tests::{armed_fault_surfaces_as_the_real_substrate_error, armed_fault_is_standing_and_fires_on_every_call, arming_one_slot_leaves_the_others_on_their_success_arms, clear_faults_disarms_every_slot_and_is_idempotent, shared_converge_and_observe_faults_are_independent_standing_slots}` | PASS (genuine) | — (retargeted onto the element-update fault + converge-first sequence; pass on the sim today) |
| S-ND295-71 | 05-01 | `overdrive-worker/…/mtls_intercept_equivalence.rs::an_install_before_convergence_is_refused_and_changes_nothing` | RED | `[S-ND295-71][host] install_outbound before any converge is refused with SharedProgramNotConverged, got Err(NftRuleInstallFailed { op: "shared-owner-required", … })` (:590) |
| S-ND295-71 | 05-01 | `…::an_install_at_a_port_other_than_the_recorded_target_is_refused` | RED | `[S-ND295-71][sim] a F install at port 33833 must be refused with SharedListenerPortMismatch, got Ok(())` (:660) |
| S-ND295-71 | 05-01 | `…::a_node_guard_dropped_with_no_members_leaves_no_program` | RED | `[S-ND295-71][sim] a node guard dropped with no member removes its program` left `Some(ConstantRules { … })` right `None` (:701) |
| S-ND295-71 | 05-01 | `…::a_convergence_from_a_stale_prior_is_refused_and_changes_nothing` | RED | `[S-ND295-71][sim] an absent prior over a present program is refused` (:742) |
| S-ND295-71 | 05-01 | `…::a_zero_listener_port_is_refused_before_any_program_change` | RED | `[S-ND295-71][sim] a zero leg F port is refused` (:798) |
| S-ND295-71 | 08-02 | `…::a_node_guard_dropped_with_no_members_leaves_no_member_state` | RED — preceding-step gap (08-02) | `not yet implemented: RED scaffold: D-295-R15 observe_shared_state — DELIVER step 08-02` (`mtls_intercept_port.rs:1102`) |
| S-ND295-71 | 08-02 | `…::a_node_guard_dropped_while_members_exist_keeps_the_program` | RED — preceding-step gap (08-02) | `not yet implemented: RED scaffold: D-295-R15 observe_shared_state — DELIVER step 08-02` (`mtls_intercept_port.rs:1102`) |
| S-ND295-70 (retargeted install pair) | active | `…::mtls_intercept_equivalence::{both_installs_hand_back_a_guard_that_releases_cleanly, re_installing_the_same_capture_converges_and_both_guards_release_cleanly}` | PASS (genuine) → phase-C blocker 4 CLOSED | both installs converge first; `[S-MIF-11/12][host]` and `[sim]` EXECUTED, every guard released cleanly |
| S-ND295-54 | 07-01 | `overdrive-worker/src/mtls_intercept_port.rs::shared_program_rollback_acceptance::remove_allocation_elements_deletes_only_present_requested_members` | RED | `not yet implemented: RED scaffold: D-295-R10 remove_allocation_elements — DELIVER step 07-01` (:1121) |
| S-ND295-54 | 07-01 | `overdrive-worker/…/shared_intercept_members.rs::convergent_removal_with_a_pre_absent_member_and_batch_rejection_preserves_state` | RED | `not yet implemented: RED scaffold: D-295-R10 remove_allocation_elements — DELIVER step 07-01` (`mtls_intercept_port.rs:1121`), reached after the fixture converged and installed |
| S-ND295-54 | 07-01 | `overdrive-worker/…/shared_intercept_members.rs::removal_is_refused_when_the_recorded_program_was_replaced_out_of_band` (NEW — R10 recorded-versus-observed row) | RED | `not yet implemented: RED scaffold: D-295-R10 remove_allocation_elements — DELIVER step 07-01` (`mtls_intercept_port.rs:1121`), reached after the out-of-band replacement |
| S-ND295-72 | 05-00 | `overdrive-netlink/…/managed_link_address.rs::a_created_bridge_carries_its_address_from_creation_and_starts_down` | RED | `assertion left == right failed: the address is set by userspace from creation (NET_ADDR_SET) …` left `1` right `3` (:178); control recorded `create-then-set … final mac=02:01:00:00:00:01 addr_assign_type=3` |
| S-ND295-72 | active | `…::managed_link_address.rs::ensure_bridge_adopts_a_present_link_of_any_kind_without_writing` | PASS (genuine) | bridge/dummy/persistent-TAP before==after; today's `ensure_bridge` already adopts a present link without writing |
| S-ND295-72 | 05-00 | `overdrive-control-plane/…/guest_network.rs::scratch_probe_acceptance::an_unchanged_scratch_tap_address_passes_the_probe_between_two_reads` | SUPERSEDED 2026-09-28 — DELETED (was RED) | `assertion left == right failed: the probe reads the scratch TAP exactly twice: []` (:4806) — the probe reads no scratch TAP yet |
| S-ND295-72 | 05-00 | `…::scratch_probe_acceptance::a_probe_that_fails_before_the_last_exercise_reads_the_scratch_tap_once` | SUPERSEDED 2026-09-28 — DELETED (was RED) | `assertion left == right failed: no re-read off the success path: []` (:4863) |
| S-ND295-72 | 05-00 | `…::scratch_probe_acceptance::a_changed_scratch_tap_address_refuses_startup_and_still_cleans_up` | SUPERSEDED 2026-09-28 — DELETED (was RED) | `the scratch-TAP condition refuses startup: ()` (:4931) |
| S-ND295-72 | 05-00 | `overdrive-control-plane/…/guest_network.rs::shared_owner_link_address_kernel::a_bridge_identity_mismatch_names_the_observed_address_and_up_state` | RED | `[changed address] the observed fact carries the read-back address and up state` left `None` right `Some(Bridge { … mac: [2, 149, 114, 0, 0, 13], up: true … })` (:10326) — today's audit reports `observed: None` |
| S-ND295-72 | 06-02 | `…::shared_owner_link_address_kernel::a_provisioned_taps_recorded_address_survives_udev_initialisation` | SUPERSEDED 2026-09-28 — RE-AUTHORED (was RED — preceding-step gap (06-02); current row in Phase E) | `the node is healthy after provision: SharedGuestNetworkAuditError { component: Bridge, source: PostconditionMismatch { operation: TapObserve, … owner_uid: Some(4200) …` — no `host_mac` record until 06-02 |
| S-ND295-00 (bridge leg) | 05-00 | `overdrive-control-plane/…/shared_guest_network_startup.rs::production_host_owner_boots_only_after_real_shared_identity_is_exact` | RED — REPRODUCED DEFECT (RCA root cause A) | `[S-ND295-00] fresh-host boot 4/5 refused: GuestNetworkBoot(PostconditionMismatch { operation: BridgeObserve, expected: BridgeLinkIdentity { … }, observed: Some(BridgeLinkIdentity { … }) })` — repeated boots: 2 read-back-exact, 2 BridgeObserve refusals, 1 unexplained probe timeout (RCA § 8, counted for neither side) |
| S-ND295-00 (DNS leg) | 05-01 | `…::shared_guest_network_startup.rs::the_shared_gateway_answers_an_absent_mesh_name_with_nxdomain` (re-authored) | RED — preceding-step gap (05-01) | `DNS client task joins: JoinError::Panic(…, "shared DNS reply: Os { code: 11, kind: WouldBlock … }")` — no responder bound until 05-01 composes the host DNS factory through `guest_dns` |
| S-ND295-64 (controls) | 08-01 | `overdrive-cli/…/intercept_mark_fail_closed.rs::both_time_wait_controls_prove_the_substate_and_sequence_gates` | RETARGETED — the vacuous guest re-probe removed; the body now asserts only the two door-independent controls (host-veth), which is a real oracle. NEW native RED at its own step (metal) | the two controls are the phase-C `SynAck`/`BareAck` pair, unchanged; not metal-run in this cut (host-veth only, no production change), classified by the phase-C control evidence |
| S-ND295-64 (guest door) | 05-03 | `overdrive-cli/…/intercept_mark_fail_closed.rs::a_guest_reconnect_into_its_leg_f_time_wait_entry_is_recorded_and_a_reopen_goes_to_the_user` (NEW; replaces the vacuous guest case) | RED — preceding-step gap (05-03) | precondition `workload server did not reach Running within 90s; … error: Some("  3: Cannot create virtio-net device\n  4: Failed to open taps\n … Operation not permitted")` (vm_walking_skeleton.rs:573), then nextest TIMEOUT at 120 s — the named-TAP path the 05-03 fd handoff replaces; the door it records additionally depends on R19 (08-01, conditional) |
| S-ND295-45 | 05-03 | `overdrive-cli/…/vm_walking_skeleton.rs::every_cloud_hypervisor_thread_carries_the_launch_filter_under_its_own_filters` (marker moved 05-02 → 05-03) | RED — preceding-step gap (05-03) | precondition `workload nd295-s45 did not reach Running within 90s; … error: Some("  3: Cannot create virtio-net device\n  4: Failed to open taps\n … Operation not permitted")` (vm_walking_skeleton.rs:573), then nextest TIMEOUT at 120 s — the named-TAP path the 05-03 fd handoff replaces |

Retained-active bodies re-run in this cut and still green: the seven active
`adapters::mtls_intercept::tests` bodies, the 10
`netns_density_shared_owner` integration bodies, the 240 control-plane library
bodies, and the 76 worker library bodies. The §3.3 supervisor proof
(`shared_network_supervisor_recovery`) and §3.4 element-cleanup proof stay
**RED — REPRODUCED DEFECT** at their own steps (09-01, 07-01), unchanged; the
`ProofIntercept` `program_lost` overlay now re-establishes against the inner
sim's own observation so it does not trip the sim's new stale-`prior` refusal,
and the boot-order invariant (§3.2 sibling, 08-02) still fails with
`no converge_allocation_elements(∅) call`, seeds printed.

### C-12b reclassified

C-12b (fresh-host `BridgeObserve` refusal) and metal N-04 (by inference) are
reclassified under the fresh-host RCA root cause A, owned by DELIVER step
05-00. They were **BROKEN — remaining (blocker)** in phase C as an active-body
production failure; they are now **RED — REPRODUCED DEFECT** on the split
S-ND295-00 bridge-identity leg (pending 05-00), with the DNS half a separate
test-body defect re-authored and pending 05-01. Blocker 3 of phase C is
resolved.

### C-12b classification note (superseded)

The phase-C blocker-3 entry (S-ND295-00 active, fails on production) is
superseded by the split above: the bridge half is the reproduced RCA defect
(05-00), and the DNS half is the re-authored preceding-step gap (05-01).

## Phase E — managed-link host-independence follow-up (2026-09-28)

Executed 2026-09-28 after the user rulings of that date (DESIGN commits
`fbb6acd4`, `c51ab167`): REQ-295-LINKMAC withdrawn as a requirement, the
startup probe's scratch-TAP condition removed, and a TAP's host-side MAC judged
by the D-295-R21 invariant, reported as `GuestNetworkFact::TapHostMac { ifindex,
address: TapHostAddress }` (`Unreserved | Reserved([u8; 6]) | Missing`). Run
serially, one command at a time in the foreground, against HEAD `c51ab167` plus
this cut's uncommitted changes.

### Substrates

| Substrate | Kernel and host | Runner |
|---|---|---|
| Lima | `overdrive` VM, aarch64, `7.0.0-34-generic`, systemd-udevd active; cold-started, quiet, and clean before the run (only the pre-existing `table ip nat`) | `cargo xtask lima run -- …` |
| Native metal | non-virtualized x86_64 (`systemd-detect-virt`: `none`), `7.0.0-29-generic`, `systemd 259 (259.5-0ubuntu3.4)`, systemd-udevd active; KVM preflight and canonical lease on every command; quiet | `cargo xtask metal run [--no-sync] -- …` |

The BPF object was built on Lima (`cargo xtask bpf-build`) and natively on
metal (`OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`: without the variable,
`bpf-build` re-dispatches into Lima, which the metal host does not have). Both
objects carry `gh295c_egress` and `gh295c_endpoint`.

### Environment and cleanup

- **Lima:** the (e) body's `NodeSharedStateSweep` removed `ovd-tp-f2e5`,
  `ovd-tp-f2e6`, `ovd-gbr0`, `table bridge overdrive-mtls`, and the
  `mtls-endpoints` pins; the empty `/sys/fs/bpf/overdrive` directory it left
  was removed after the run.
- **Metal:** before the run, an empty `/sys/fs/bpf/overdrive/probe` directory
  dated 2026-09-26 was removed (it held no pins; the only BPF objects present
  were systemd's sysctl monitor). After the run the sweep removed the same
  objects as on Lima, and the empty `/sys/fs/bpf/overdrive` was removed. No
  TAP, bridge, nft table, cgroup scope, or policy rule remained on either
  substrate.

### Gate / lint

`cargo clippy -p overdrive-control-plane -p overdrive-netlink --all-targets
--features integration-tests -- -D warnings` is clean on Lima. `rustfmt
--edition 2024 --check` is clean on the three changed Rust files. The six
remaining active `scratch_probe_acceptance` / `scratch_probe_packet_acceptance`
bodies still pass after the deletion (Lima: `6 tests run: 6 passed`).

### Test-side changes in this cut

| File | Change | Why |
|---|---|---|
| `crates/overdrive-control-plane/src/guest_network.rs` (`scratch_probe_acceptance`, `scratch_probe_packet_acceptance`) | DELETED the three E22 (p1)/(p2) bodies and their support: `Script::{tap_reads, tap_read_positions}`, `ScratchTapRead`, `ScratchTapReadScript`, `UNCHANGED_SCRATCH_TAP`, `with_tap_reads`, `with_tap_reads_and_semantic_failure`, `tap_read_positions`, `read_scratch_tap`, `tap_host_mac`, and the D14 double's `PacketProbeIo::read_scratch_tap` / `ScratchTapIdentity` | user ruling 4: the startup probe reads no scratch-TAP address; the bodies defend no contract |
| same file (production enum, class P/V) | `GuestNetworkFact::TapHostMac { ifindex: u32, address: TapHostAddress }` and the public `TapHostAddress { Unreserved, Reserved([u8; 6]), Missing }` exactly as FD § "[REF] Driven port — TAP egress guest-MAC delivery (D-295-R21) — ACCEPTED 2026-09-24" pins them; no behaviour change (the variant is constructed only in tests) | the user-approved fact shape (ruling 6) |
| same file (`allocation_owner_acceptance`) | `AllocationDamage::HostMacChanged` and `ProtectionFault::HostMacChanged` (an unreserved `fe:95:de:ad:00:01` write, expected as a recorded value) replaced by `HostMacReserved` (the TAP's own guest MAC) and `HostMacMissing` (no address) in the four bodies that used them; after the second review, the same unreserved bytes written inline as the recovery-audit damage in `every_node_level_audit_failure_names_its_matrix_component_first` became `HostMacReserved`, and its recovered audit now asserts the named fact | under the invariant an unreserved write is not damage; each table needs a reserved-address stimulus and the `TapHostAddress` fact |
| same file (`allocation_owner_acceptance`) | six NEW E22 (i1)/(i2) bodies over one shared `InvariantBreak` vocabulary (each reserved class plus the missing address) on `AuditFixture`. After review: the provision table also writes each break after the egress step; the unreserved audit passes change both TAPs' addresses between audits; the `Condemned` leg is its own 06-04 body | E22 (i1) at provision, `activate`, and the audit, the ordering case, and the (i2) contrasts |
| same file (`shared_owner_link_address_kernel`) | the (e) body re-authored with two allocations and renamed; the module doc and constants lose REQ-295-LINKMAC and the host link policy; the sweep removes the second TAP | E22 (e) as the DESIGN states it |
| `crates/overdrive-netlink/tests/integration/managed_link_address.rs`, `tests/integration.rs` | REQ-295-LINKMAC labels and host-link-policy wording removed; scenario title updated. No oracle changed, so the two bodies were not re-run | FD § *Required downstream changes* (the netlink bodies' oracles hold) |

### Per-body results

Every source-local body below stops at the same fixture precondition, the
provision owner-uid read-back (D-295-R4, the 06-02 owner-uid change), exactly as
phase C recorded for the sibling bodies on this fixture (rows S-ND295-50/51
above). A marker of 06-02 is its own step (**RED**); a marker of 06-04 is a
**preceding-step gap (06-02)**. These are fixture-precondition REDs: no
invariant oracle executed today. Each body's own oracle is unexercised until its
step's RED phase, where the crafter records the invariant oracle's semantic
RED before any production change.

| Scenario | Marker step | Body (`file::fn`) | Classification | First failing line / message (verbatim) |
|---|---|---|---|---|
| S-ND295-50 | 06-02 | `overdrive-control-plane/src/guest_network.rs::allocation_owner_acceptance::every_node_level_audit_failure_names_its_matrix_component_first` (RETARGETED stimulus after the second review: its recovery-audit damage was an unreserved write, which the invariant does not count) | RED (fixture precondition; oracle unexercised) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-aa", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-aa", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-50 | 06-02 | `overdrive-control-plane/src/guest_network.rs::allocation_owner_acceptance::every_per_allocation_damage_is_named_only_when_the_node_is_healthy` (RETARGETED stimulus) | RED | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-aa", ifindex: Some(295), link_kind: Tap, persistent: true, up: false, owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-aa", ifindex: Some(295), link_kind: Tap, persistent: true, up: false, owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-50 | 06-04 | `…::allocation_owner_acceptance::a_condemned_allocation_leaves_every_later_audit_and_restore_universe` (RETARGETED stimulus) | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-ca", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-ca", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-51 | 06-04 | `…::allocation_owner_acceptance::activation_reads_every_protection_fact_before_reporting_success` (RETARGETED stimulus) | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-v1", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-v1", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-51 | 06-04 | `…::allocation_owner_acceptance::activation_under_a_latch_or_condemnation_changes_nothing` (RETARGETED stimulus) | RED — preceding-step gap (06-02) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-l1", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-l1", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-72 | 06-02 | `…::allocation_owner_acceptance::a_reserved_or_missing_host_side_address_refuses_publication` (NEW; revised after review: each break is also written after the egress step, so only the step-7 read-back sees it) | RED (fixture precondition; oracle unexercised) | both runs: `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-aa", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-aa", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-72 | 06-04 | `…::allocation_owner_acceptance::a_reserved_or_missing_host_side_address_refuses_activation_before_any_change` (NEW) | RED — preceding-step gap (06-02) | same message, `t295-aa` (guest_network.rs:6842:37) |
| S-ND295-72 | 06-02 | `…::allocation_owner_acceptance::a_reserved_or_missing_host_side_address_is_that_allocations_audit_damage` (NEW) | RED | same message, `t295-aa` (guest_network.rs:6842:37) |
| S-ND295-72 | 06-02 | `…::allocation_owner_acceptance::an_unreserved_host_side_address_is_not_audit_damage_whenever_it_changes` (NEW; split out after review, and its passes now change both TAPs' addresses between audits) | RED (fixture precondition; oracle unexercised) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-aa", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-aa", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-72 | 06-04 | `…::allocation_owner_acceptance::a_tap_holding_a_condemned_guests_address_is_not_audit_damage` (NEW; the `Condemned` leg split out after review, because audit condemnation lands with R5 at 06-04) | RED — preceding-step gap (06-02) (fixture precondition; oracle unexercised) | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-ih", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-ih", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-72 | 06-02 | `…::allocation_owner_acceptance::a_guest_address_a_tap_took_early_is_damage_from_the_first_audit_after_that_guest_is_held` (NEW) | RED | `fixture provisions the attachment down: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "t295-ie", ifindex: Some(295), … owner_uid: Some(4200) }, observed: Some(Tap { name: "t295-ie", … owner_uid: Some(0) }) }` (guest_network.rs:6842:37) |
| S-ND295-72 | 06-02 | `…::shared_owner_link_address_kernel::a_tap_host_address_is_judged_by_the_invariant_whatever_the_host_link_manager_wrote` (RE-AUTHORED), Lima root | RED | `[S-ND295-72 (e)] systemd-udevd running on this substrate: true (Ok(Some(0)))` … `ovd-tp-f2e5 ifindex 4 after udev: mac=Some([46, 169, 182, 12, 2, 201])`, then `the node is healthy after provision: SharedGuestNetworkAuditError { component: Bridge, source: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "ovd-tp-f2e5", ifindex: Some(4), link_kind: Tap, persistent: true, up: false, owner_uid: Some(4200) }, observed: None } }` (guest_network.rs:10580:50) |
| S-ND295-72 | 06-02 | same body, native metal (first run) | RED | `ovd-tp-f2e5 ifindex 59079 after udev: mac=Some([238, 81, 190, 34, 254, 55])`, then `the node is healthy after provision: SharedGuestNetworkAuditError { component: Bridge, source: PostconditionMismatch { operation: TapObserve, expected: Tap { name: "ovd-tp-f2e5", ifindex: Some(59079), link_kind: Tap, persistent: true, up: false, owner_uid: Some(4200) }, observed: None } }` (guest_network.rs:10580:50) |
| S-ND295-72 | 06-02 | same body after review (adds the systemd version record), Lima root rerun | RED | `[S-ND295-72 (e)] host systemd: Ok("systemd 259 (259.5-0ubuntu3.4)")`, `ovd-tp-f2e5 ifindex 7 after udev: mac=Some([46, 169, 182, 12, 2, 201])`, then the same `the node is healthy after provision: … observed: None } }` (guest_network.rs:10628:50) |
| S-ND295-72 | 06-02 | same body after review, native metal rerun | RED | `[S-ND295-72 (e)] host systemd: Ok("systemd 259 (259.5-0ubuntu3.4)")`, `ovd-tp-f2e5 ifindex 59084 after udev: mac=Some([238, 81, 190, 34, 254, 55])`, then the same `the node is healthy after provision: … ifindex: Some(59084) … observed: None } }` (guest_network.rs:10628:50) |

Why the (e) body fails where it does: today's `audit_shared` resolves each held
allocation through the static pool (`action_plan`), not through the plan the
owner holds, so it reports a hand-built plan's TAP as absent (`observed: None`)
and still expects owner uid 4200. The 06-02 per-allocation audit over held
plans, with owner uid 0, replaces both, so the failure is that step's missing
behaviour, not a fixture defect. On both substrates systemd-udevd rewrote the
first TAP's address after creation (`2e:a9:b6:0c:02:c9` on Lima,
`ee:51:be:22:fe:37` on metal): a locally administered unicast outside the
reserved set, which is the case the invariant accepts. Rerun after review, udev
wrote the same address again for the same link name on each substrate. That fits
the DESIGN's residual analysis (udev's persistent address is a fixed function of
the machine and the link name).

After the review, the three revised or split source-local bodies were rerun once
serially (`3 tests run: 0 passed, 3 failed`, each at guest_network.rs:6842:37 as
above). Both substrates were checked clean afterwards. After the second review,
the node-level audit body and the unreserved-pass body (which now uses a third,
udev-shaped unreserved address) were rerun once serially (`2 tests run: 0
passed, 2 failed`, each at guest_network.rs:6842:37).
