# P5 — VMM device and beacon intake (netns-density-295, carried from the earlier pass)

Carried from the earlier DISTILL pass (2026-10-13) and re-checked against
the EVMOD design only for compilation in the 2026-10-18 integration pass;
the pool, lease and boot-reclamation cells and the `cid_lease` driver moved
to P3b (`test-scenarios-p3b-lease-quiescence-recovery-boot.md`).

Owners: `CloudHypervisorVmm` (`overdrive-host`), `SimVmm` (`overdrive-sim`,
QPROJ-7 arbiter of `vsock_device_holders`), `GuestAddressPool`
(`overdrive-control-plane`), `VmDriver` + `BeaconIntake`
(`overdrive-worker`), `vm_reclamation_boot::converge`.

Markers: pool, lease and beacon bodies `pending DELIVER attachment-cut step`;
`SimVmm` device cells `pending DELIVER step 2` (they build `GuestCid`); the
fork launch argument `pending DELIVER step 3`; the RESET-EXIT fork cells
`pending DELIVER step 1`.

## VMM launch and device (CID-NOCLAIM, RESET-EXIT)

| Test | Contract | Owner / stimulus | Oracle | Tier | Pending |
|---|---|---|---|---|---|
| `overdrive-host` `vmm::tests::the_vsock_device_argument_names_the_leased_cid_and_the_vhost_kernel_backend_only` | exactly one `--vsock cid=<cid>,backend=vhost-kernel`, no `fd=` / `socket=` | private `vhost_kernel_vsock_arg` over leased CIDs | rendered args | unit | step 3 |
| `vsock_vmm_device::the_vmm_opens_its_own_vsock_device_and_overdrive_holds_no_copy` | sole holder | `Vmm::create` on the real fork | `/proc/<vmm>/fd` holds one `/dev/vhost-vsock`; `/proc/self/fd` none | Tier-3 metal `kvm-tests` | step 3 |
| `vsock_vmm_device::the_cid_is_free_at_once_after_its_vmm_exits` | VMM exit is the device's last close (V-30 Q2a/Q2d) | terminate, relaunch same CID | second VMM holds the device | Tier-3 | step 3 |
| `vsock_vmm_device::a_launch_on_a_held_cid_exits_before_ready_and_leaves_the_holder_running` | K-C1 | second launch on a held CID | clash VMM exits; holder alive | Tier-3 | step 3 |
| `vsock_vmm_device::a_guest_pid1_crash_under_panic_1_ends_the_vmm`, `a_guest_triple_fault_ends_the_vmm` | RESET-EXIT | `init=/nonexistent` with `panic=1`; `reboot=t` | VMM exit within 30 s; CID free | Tier-3 | step 1 |
| `overdrive-sim` `vmm::tests::{a_launch_holds_its_cid_device_until_terminate, a_launch_on_a_held_cid_exits_before_ready_and_the_holder_is_unaffected, a_guest_reset_ends_the_vmm_and_frees_its_cid, an_orphan_keeps_its_device_until_killed}` | the arbiter's own semantics (RESET-EXIT, NO-SURVIVE orphan) | `SimVmm` hooks | `vsock_device_holders` | unit | step 2 |

## Beacon intake (`overdrive-worker/tests/acceptance/vsock_beacon_intake.rs`)

Pending attachment-cut (bounded-change): `a_ready_of_the_live_launch_takes_the_claim_and_completes_the_start`,
`a_stale_queued_ready_is_closed_and_does_not_poison_the_claim` (P-37),
`a_ready_read_after_stop_takes_nothing` (ATTR-LIVE-CLAIM),
`a_second_connection_of_the_same_launch_is_closed_session_held`,
`a_connection_closed_with_nothing_buffered_is_closed_without_effect`,
`a_first_line_that_is_not_ready_closes_the_connection_without_effect`,
`a_connection_from_a_cid_with_no_live_launch_is_closed_at_accept`. Oracle:
`VmDriver::beacon_observation` + `SimBeaconIo` queued / `closed_by_driver`.
DG-4 bind cells: `binding_the_intake_binds_the_beacon_listener`,
`a_bind_failure_is_typed_and_binds_nothing`,
`the_bind_refusal_renders_its_pinned_message` (**active**),
`a_fresh_driver_reports_an_empty_beacon_observation`. The two bind cells are
GREEN on activation since `BeaconIntake::bind` binds through its port so
`serve` boots (`upstream-issues.md` S-1).

R5-59 (a), beacon seam (`overdrive-host/tests/integration/vsock_beacon_accept_path.rs`,
`kvm-tests`, pending attachment-cut (6)): `VsockBeaconIo` against
`SimBeaconIo` with a guest-kit microVM at a band CID as the peer — the
named Tier-3 pair of the sim's beacon accept arbiter that `flows_beacon`
reads (QPROJ-7). Both adapters close a peer whose CID is not a `GuestCid`
(DR1-CLOSE D-1).

## Node admission (`overdrive-sim/tests/integration/netns_density_node_admission.rs`)

`node_wide_attachment_admission_never_exceeds_the_t1_cap_across_workloads`
(S-ND295-05D re-derived on the pinned traits, plus G-V0: one CID per held
lease, CID = BASE + usable offset). Seeded Tier 1. Pending attachment-cut.

## Conformance drivers

| Driver | Spec / main | Named runs | `#[quint_run]` | Projection | Pending |
|---|---|---|---|---|---|
| `vm_driver::quint_flows_beacon` (`FlowsBeaconDriver`) | `owner_flows.qnt` — `flows_beacon_conf` (`"flows_beacon_conf::owner_flows::conf_step"`, PD-6) | 7 | 1 | `VmDriver::beacon_observation` (QPROJ-4); `SimBeaconIo::beacon_socket_observation` (marked arbiter, Tier-3 V-22) | attachment-cut (6) |

## Migrations and deletions

`Vmm::create(&VmConfig, GuestCid)` adapted in every impl and test double;
the claim adapters (`overdrive-host/src/cid_claim.rs`, its Tier-3 test,
`overdrive-sim` `guest_cid_claim`) and the `command-fds` workspace
dependency were deleted. Spike tests `vm_finalize_failed_ownership_spike`
and `e10_vm_early_exit_spike` use `BeaconIntake` over `SimBeaconIo` with
READY carrying the lease token.

## Blockers

The earlier pass's beacon and model items (UI-V-BEACON-VMSTOP,
UI-V-BEACON-FIFO, UI-V-BEACON-ACTIONS) were addressed by the DESIGN
gap closure (D-UP-9, D-UP-20, PD-6); this pass did not re-derive the beacon
cells beyond compilation. UI-V-QT1 is closed: activated `#[quint_test]`
runs replay to the semantic RED (D-UP-1, `red-classification.md`).
UI-V-VMTAPPLAN is the step-7 signature change noted in `upstream-issues.md`
O-3.
