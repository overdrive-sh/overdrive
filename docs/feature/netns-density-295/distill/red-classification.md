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
| "CONTROL-FRAME ORACLE SELECTED" — the closed ARP-reply / zero-payload TCP-reset population before the event (v2) | **SUPERSEDED.** It weakened the zero-frame invariant, which the charter forbids (Changed Assumption 2, FD 4795-4800). The replacement oracle is zero frames with the TAP down (S-ND295-01). |
| "fd handoff out of accepted scope / unnecessary" (v2) | **SUPERSEDED.** The native fd spikes (`.context/netns-density-295-fd-tap-spike-findings.md`, `spike/findings-persistent-fd-tap.md`) proved the handoff; D-295-R1 to R3 adopt it. |
| "DEFERRED TAP ACTIVATION DESIGN FALSIFIED … no `activate`, phase, error, fd handoff, capability grant, or confinement change is permitted" | **SUPERSEDED.** Only its named-TAP clause was falsified; the ordering and owner-state contract survives as D-295-R5 over the fd handoff. |
| The 2026-09-23 "deferred-TAP-activation amendment" v1 subsection | **SURVIVES IN INTENT** as D-295-R5; its "Cloud Hypervisor attaches the down TAP by name" and "argv stays `tap=`" clauses are superseded by D-295-R1/R2. |
| Every classification that credited the eight user-waived `panic!` placeholders as "not scored" | **SUPERSEDED.** `testing.md` forbids handing a placeholder body to DELIVER; phase B authors or deletes each (`test-scenarios.md` § *Existing-body disposition register*). |

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
invariant (FD 897-917). It is spike evidence, not a test.

## Expected classifications for the rewritten set (before phase C runs)

| Bodies | Expected | Why |
|---|---|---|
| Every body whose driving port is a phase-B `todo!("RED scaffold …")` (new pool ops, `attach_tap_queue`, `debug_msg_mask(s)`, `attach_first_egress`, `register_launch_child_hook`, `for_target`, `check_launch_seccomp`, `remove_allocation_elements`, `converge_allocation_elements`, `observe_shared_state`, `run_shared_network_supervisor`, `GuestAttachmentLease::cleanup_pending`, `CgroupPath::workloads_slice`, the reclaim shim arm, `xtask::cloexec_lint::{scan_source, scan_workspace, render_violation}`) | RED — MISSING_FUNCTIONALITY | the body reaches the production owner and stops at its scaffold |
| S-ND295-46 real-workspace body (`the_serve_closure_creates_no_inheritable_descriptor`) once the gate exists | RED — MISSING_FUNCTIONALITY until 05-04 fixes the eight sites | the gate reports each unfixed site of the obligation's table (FD 724-732) |
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

*Pending.* Phase C appends one row per selected body after phase B lands:
exact command, run id, observed failure, and classification. Nothing below this
heading has been executed yet.

| Scenario / body | Exact command | Observed result | Classification |
|---|---|---|---|
