# Test scenarios: node-local persistent volume for VM workloads (GH #348)

Specification only. Nothing here is parsed or executed (`.claude/rules/testing.md` § "Testing"). The executable tests are Rust,
named in each row. There is no DISCUSS and no DEVOPS wave for this feature: acceptance criteria derive from the approved design
(`feature-delta.md`, ADR-0310 to ADR-0320, ADR-0334 to ADR-0338, VD-1 to VD-10, SD-1 to SD-6, P-1 to P-3) and the issue's "Required
behavior" as reduced by VD-10 (deletion and reclaim belong to #211). Rewritten 2026-11-19 to describe exactly what is in the tree.
Contracts, boundaries and invariants are in `feature-delta.md` and the ADRs and are not restated here; a row names the contract it
checks, the owner path, the stimulus, the oracle, the test and its status.

**Reconciliation (pre-scenario gate).** Wave decisions read: `design/wave-decisions.md`, `spike/wave-decisions.md` (DISCUSS and DEVOPS
absent: warned, not blocking). The issue requires deletion and reclaim; the approved design moves it to #211 (VD-10), a user-approved
scope decision. Result: 0 contradictions.

**How a row reads.** Tier names: **Tier 1** is the in-process lanes (layer 1 pure functions, proptest and render goldens; layer 2
composed production owners over `Sim*` driven ports and the seeded harnesses), all `host_pure`; **conformance** is a `quint-connect`
trace replay (`host_pure`, `integration-tests` lane); **Tier 3** is a guest placement (`guest = "cgroup-default"`, base, `volume-pool`
or `nested-launch` profile). Faults enter only at driven ports (`Sim*` adapters, the `Ca` port, `SimDriver` scripts, `SimClock`); no
test seeds a row, a View entry or a lease that production does not author. Status values: **PENDING** (a complete body under
`#[should_panic(expected = ...)]`; `red-classification.md` gives the failure today and where it stops), **PENDING-ignored** (the same,
`#[ignore]`d for the one missing lane resource, the `volume-pool` guest's pool), **GUARD** (holds before and after), **NOT AUTHORED**
(with its reason). Contract shapes: B bounded-change, P pure-function, U unbounded-preservation (proptest). Counts live in § K only.

**Retired ids** (kept as gaps so cross-references stay stable): S-VOL-08 to S-VOL-10 (the serial is a constant pinned by the argv
golden), S-VOL-26d (replaced by the golden test in § B), S-VOL-66 (replaced), S-SUP-11, 15, 16, 17, 21, 23, 30, 35, S-VOL-T3-06,
S-VOL-T3-08. (S-VOL-T3-13 was retired and is restored, B-2.) Each is explained at its row or in § L.

## A. Value types, spec surface and wire shapes (layer 1)

| Id | Contract / oracle | Owner path | Test (file under `crates/overdrive-core/tests/acceptance/` unless stated) | Status |
|---|---|---|---|---|
| S-VOL-01..07 | `VolumeSize` grammar: round trip in any unit case, TiB/GiB equality, canonical display, near-miss and malformed inputs rejected, overflow rejected, serde matches `FromStr` | `VolumeSize::from_str`, `Display`, serde | `vm_volume_size.rs` | PENDING (P) |
| S-VOL-08 | The guest-visible serial is the constant `VOLUME_DISK_SERIAL = "ovd-volume"`; there is no `DiskSerial` type. Retired: `vm_volume_disk_serial.rs` is deleted and the serial is pinned only by the argv golden of S-VOL-26c | the constant | retired (see S-VOL-26c) | n/a |
| S-VOL-11..20 | `[vm.volume]` parses for Job and Service; unknown key, array and missing size refused; echo canonical; `vm.volume.size` named on error; `replicas != 1` refused; single replica admitted; no volume echoes none (boundary 6); intent round trip; `spec_digest`; envelope discriminants `[0, 1]` | submit-spec parse, `WorkloadIntent` store codec | `vm_volume_spec.rs` | PENDING (P); GUARD `guard_a_job_without_a_volume_echoes_no_volume` |
| S-VOL-21 | Frozen V1 fixture, `FIXTURE_V2`, alloc-row discriminant offset | `schema_evolution/` | n/a | NOT AUTHORED (U-6: the V2 bytes do not exist before DELIVER; one atomic commit, `development.md` procedure step 6) |
| S-VOL-22..25 | `VolumeUnavailable` maps to `VmVolumeUnavailable` (pool absent; `Held`, a unit cause with no field); operator wording names volume, cause and an appliance-performable remedy (no operator shell, so no node-local command): pool absent -> re-provision the node with a volume pool; `Held` -> stop the holder with `overdrive job stop`; a node-local command (`lvm`, "on the node", "check the pool", "free pool space") is refused | `TransitionReason::from(&DriverStartFailure)`, `human_readable()` | `volume_unavailable_reason.rs` (`the_*_wording_names_*`, `no_volume_wording_sends_the_operator_to_a_node_local_command`) | PENDING (P) (`#[should_panic]`, RED on the remedy); GUARD `guard_a_volume_failure_is_a_failure` |
| S-VOL-26 | Insufficient space keeps both figures; command failure keeps the operation (PBT over `VolumeOperation`); wording names figures (`40GiB`, `4194304B`) and operation; remedies as above (insufficient space -> declare a smaller size or re-provision with a larger pool; command failure -> retried under the restart budget, re-provision if it persists) | same | `volume_unavailable_reason.rs` | PENDING (P) |
| S-VOL-26a | `ByteCount`: `<n>B` round trip over all `u64` including the unit's case (PBT), grammar refusals (PBT), overflow refused; `VolumeSize::bytes`; the insufficient-space cause carries `free` as a `"<n>B"` string beside the declared size (this is the only test of the cause's serde; the store round trip below covers its rkyv archive) | `ByteCount`, `VolumeSize`, cause serde | `volume_value_types.rs` | PENDING (P) |
| S-VOL-26b | `VolumeOperation` labels equal their serde labels | enum `as_str` and serde | `volume_value_types.rs::guard_volume_operation_labels_match_serde` | GUARD (P) |
| S-VOL-26c | Argv golden of the single `DiskAttachment` render site: `path=/dev/mapper/overdrive-ovd--<hash>,image_type=raw,direct=on,serial=ovd-volume`; the rootfs shape is unchanged | `DiskAttachment::volume(device).to_disk_arg()` | `volume_value_types.rs::the_volume_disk_renders_the_golden_argument` (pending), `guard_the_rootfs_disk_argument_is_unchanged` (guard) | PENDING (P); GUARD |
| S-VOL-26e | Launcher disk order: with a volume attached the launch passes exactly two disk values after the one `--disk` flag, the rootfs disk first and the volume disk (as `DiskAttachment::volume(device).to_disk_arg()` renders it) second; with no volume exactly one, the rootfs. Added by user ruling on 2026-11-20, GREEN at once, to guard the launcher argument order before the real-kernel `volume_survives.rs` is active | `CloudHypervisorVmm::build_confined_command` (crate-private, existing unit-test seam) | `crates/overdrive-host/src/vmm.rs::tests::a_launch_attaches_the_rootfs_disk_first_and_the_volume_disk_second_only_when_a_volume_is_attached` (host_pure, module `placement!`) | GREEN (P) |
| S-VOL-27, 28 | Describe shows size and the static serial; no volume, no section | `render::workload_describe` | `crates/overdrive-cli/tests/acceptance/workload_describe_volume.rs` | PENDING (P); GUARD |

## B. The `VolumeBacking` port on its simulation twin (layer 2) — `crates/overdrive-sim/tests/acceptance/sim_volume_backing_contract.rs`

Oracle: the port's own accessors; fault: the sim's scripted hooks. The simulation twin and the host adapter are checked against the
same scenarios, so a divergence is one failing pair.

| Id | Scenario | Tier | Status |
|---|---|---|---|
| S-VOL-30..32 | ensure creates, activates, leaves the claim free (boundary 1); repeat ensure idempotent, size not compared; two workloads, two volumes | Tier 1 | PENDING (B) |
| S-VOL-33, 34, 35 sad | no free extent typed, nothing created; a `claim_state` `open(2)` failure is `PoolCommand { operation: Claim }` and one-shot (`fail_next_claim_state`); `claim_state` of an absent volume `NotFound`, paired with a created volume's `Free` | Tier 1 | PENDING |
| S-VOL-36, 37 fault | claim held until the old writer drains, reading never releases it; node reboot leaves the volume inactive (`NotFound`), `ensure` re-activates the same volume | Tier 1 | PENDING (B) |
| S-VOL-38 | PBT over ensure/hold/drain/reboot: at most one create per workload, claims only on active volumes. Kept: no seeded invariant asserts that a claim reads `NotFound` on an inactive volume or the multi-workload property | Tier 1 | PENDING (B) |
| S-VOL-39 sad | `PoolCommand { operation }` on a failing command and the retry recovers; an ensure that failed after its create is reused by the retry (one volume) | Tier 1 | PENDING |
| S-VOL-39b fault | a claim linked to a VMM process (`hold_claim_while_live`, test support for a crashed control plane's orphan) reads `Held` while the process is live and `Free` once it is gone, with no `drain` | Tier 1 | PENDING (B) |
| golden (was S-VOL-26d) | `/dev/mapper/overdrive-ovd--` + first 32 hex of SHA-256 of the id: four golden vectors (`dev-box`, `alpha`, `a`, `payments-db`). The earlier proptest over the name's bound and injectivity is dropped (the vectors cover length and charset). There is no shared name-derivation function in core: host and sim derive the name separately, and the `dev-box` vector, shared with the host Tier-3 test, is the cross-adapter equivalence assertion | Tier 1 | PENDING (P) `the_volume_name_is_ovd_and_the_sha256_prefix_of_the_workload_id` |

## C. Composed launch path (G-1, G-2), boundaries 2 and 5, and the seeded volume invariants

Driving port `VmDriver::start` over `SimVmm` (through a recording `Vmm`), a stepped `SimBeaconIo`, `SimClock`, and the pool reaching
the driver through `SimVolumePoolProbe::probe` -> `VmDriver::new(.., volumes)`. File `crates/overdrive-worker/tests/acceptance/node_volume_launch.rs`.

| Id | Boundary | Scenario | Status |
|---|---|---|---|
| S-VOL-40 | 1 | ensure ok and claim free: one VMM create whose volume attachment is the ensured device with serial `ovd-volume` | PENDING |
| S-VOL-41 | 2 | pool absent (`volumes: None`), no free extent (both figures), failing `lvm` command (operation named), a failing `ensure` command, which is also how the adapter's 30 s deadline reaches the driver (the driver holds no timer, B-1; `the_driver_holds_no_ensure_timer_a_parked_ensure_outlives_the_deadline`): typed `VolumeUnavailable { PoolCommandFailed { .. } }`, no VMM created, nothing created | PENDING (three named sad tests plus the no-timer test) |
| S-VOL-41b | 2 | any `claim_state` `Err` (F-3): the launch ends at once, well inside the 30 s bound, as `PoolCommandFailed { operation: Claim }`; no VMM, the ensured volume kept | PENDING |
| S-VOL-42 | 2 | claim held past 30 s: `Held`, no VMM, the holder keeps the claim | PENDING |
| S-VOL-43 | 3 | pool faults after the launch do not reach the running allocation or its stop (the stop goes through `Driver::stop` over the trait with a hand-built handle); the volume stays | PENDING |
| S-VOL-44 | 4 | a replacement waits for the claim, not the process exit: no second VMM while held, none refused, attaches the moment it drains | PENDING |
| S-VOL-45 | 5 | node reboot: next launch re-activates the same volume; control-plane restart between create and attach reuses the volume | PENDING (two tests) |
| S-VOL-46 | 6 | no `[vm.volume]`: no second disk on a node with no pool; a composed pool is untouched | GUARD (two tests) |
| S-VOL-47 | | a `VmReclamation` pass leaves the volume untouched | NOT AUTHORED: the reclamation plan has no volume surface; S-M7 and S-VOL-T3-07 cover it |

**Seeded volume invariants** (`crates/overdrive-sim/src/invariants/volume_launch.rs`, driven by `crates/overdrive-sim/tests/acceptance/volume_launch_invariants.rs`,
Tier 1, seeds 1..=8, each printed, `Harness::new().only(..).run(seed)`). The three variants are outside `Invariant::ALL` (open decision D-2)
and each test is `#[should_panic]` on the oracle the invariant fails on today. The rig's `settle()` waits for observable quiescence (64
consecutive unchanged steps, cap 4096, a clear panic at the cap); it uses no fixed yield count.

| Id | Invariant | Oracle | Fault stimulus (seeded) | Status |
|---|---|---|---|---|
| S-VOL-48 | `volume-single-writer` | every launch that declares a volume holds it through its VMM, at most one VMM is alive, no launch reaches the VMM while the claim is held | launches, kills, delayed claim releases, reboots, control-plane restarts, a crash between create and attach, a failing command | PENDING |
| S-VOL-49 | `volume-data-survives-restart` | every launch attaches the one device; at most one volume is ever created | same schedule family | PENDING |
| S-VOL-50 | `volume-no-drain-failure-charges-budget` | the successor of a killed VMM does not end, creates no second VMM while the claim is held, eventually attaches once it drains (`assert_eventually!`), and its start returns no failure | a seed-chosen drain delay inside the 30 s bound | PENDING |

**Boot gate** (`crates/overdrive-cli/tests/integration/volume_boot_probe.rs`, Tier 3 placement, a real in-process `serve` with the probe port replaced):
S-VOL-BOOT-1..4 are one table-driven test, `sad_a_broken_volume_pool_refuses_startup`: each ADR-0314 scenario refuses startup naming the probe
(`health.startup.refused`, reason `volume_backing.probe`), independent of VM capability (injected `VolumeProbeDetail`: `CommandFailed` exit 4, 5 and none,
`NodeMissing`, and `Io` for the `sysfs_unreadable` row, P-5: a sysfs read failure refuses startup); S-VOL-BOOT-5 no labelled partition boots and logs `volume_backing.not_composed` (boundary 6); S-VOL-BOOT-6 a healthy pool boots clean (GUARD);
S-VOL-BOOT-7 (H2, `a_deployed_volume_workload_launches_through_the_composed_volume_pool`, `nested-launch` profile, `kvm-tests`): `serve` over a composed
`SimVolumePoolProbe` deploys a `[vm.volume]` workload and the recording `Vmm` receives the volume attachment through `run_server` -> `compose_volume_backing` -> `VmDriver::new(volumes)`.
All PENDING except BOOT-6.

**Cross-component links and boundaries 2 and 4**

| Id | Scenario | Test | Status |
|---|---|---|---|
| S-VOL-60..62 (H3a) | a `[vm.volume]` workload driven through `WorkloadLifecycle::reconcile`: every Start and Restart action and `allocation_spec_for_live_intent` carry the `VolumePayload` | `crates/overdrive-reconcilers/tests/acceptance/workload_volume_projection.rs` | PENDING |
| S-VOL-63 (H3b) | the `alloc_status` handler fills `AllocStatusResponse.volume` | `crates/overdrive-control-plane/tests/acceptance/alloc_status_volume.rs` | PENDING |
| S-VOL-64 (H3c) | every `VmVolumeUnavailable` cause survives the observation store (write, reopen, read) | `crates/overdrive-store-local/tests/acceptance/volume_unavailable_row_roundtrip.rs` | GUARD |
| S-VOL-65 (boundary 2) | a launch held past its bound writes a `Failed` row with `VmVolumeUnavailable { Held }` and the restart budget is charged (read from `view_for_workload_lifecycle`) | `crates/overdrive-control-plane/src/node_volume_boundaries.rs::a_launch_held_past_its_bound_writes_the_typed_failed_row_and_charges_the_restart_budget` | PENDING |
| S-VOL-66 | replaced by S-SUP-37 (boundary 4), S-SUP-26..28 (a stop or restart while a launch waits) and S-VOL-T3-12 | | replaced |

## D. Launch supersession (ADR-0334; boundaries 4 and 4a; Gate G-3)

**D1. `VmDriver::start` observation points** — `crates/overdrive-worker/tests/acceptance/launch_supersession.rs`, Tier 1 layer 2. Owner path:
`Driver::start(&spec, &dyn LaunchSupersession)` over `VmDriver` composed as the node composes it. Fault stimulus: a `SimLaunchSupersession` the test raises at the
point; `SimVolumeBacking` (parked or stalled `ensure`, a held claim, a parked `claim_state`), a failing `SimBeaconIo` accept, a failing `SimCgroupFs` scope removal, `SimClock`.
Oracle: the start's result, the VMM double's live processes and `create` calls, the cgroup double, the run, clone and index directories, `beacon_observation`, the volume
double's call counts. "Superseded wins at the point" means a typed failure found at the same instant is reported as `StartSuperseded`. Boundary 4a (b) is covered by the
"wins" tests only (S-SUP-02, 03, 05, 06).

| Id | Point | Tests | Status |
|---|---|---|---|
| S-SUP-01 | 1: entry, before `ensure` | `a_signal_raised_before_start_returns_superseded_having_created_nothing`, `a_volume_launch_superseded_before_ensure_creates_nothing_and_never_calls_the_pool` | PENDING |
| S-SUP-02 | 2: after `ensure` (never interrupted; the volume stays; wins over a deadline failure) | `a_signal_during_ensure_waits_for_ensure_then_abandons_whatever_ensure_returns` (the sim parks `ensure`, the signal is raised, the call is released; whatever `ensure` returns, Ok or Err, the launch ends `StartSuperseded` with no row and no charge) | PENDING |
| S-SUP-03 | 3: each claim poll and its sleep (wins over `Held` at the bound) | `a_signal_during_the_claim_poll_sleep_returns_at_once_without_polling_again`, `the_signal_wins_over_a_claim_held_at_the_bound` | PENDING |
| S-SUP-04 | 4: after the claim wait, before the first host artifact | `a_signal_during_the_last_claim_poll_creates_nothing_after_the_claim_wait` | PENDING |
| S-SUP-05 | 5: the guest-boot race (VMM torn down, volume kept; wins over the boot deadline; a launch that won its race is an ordinary success) | `a_signal_during_the_guest_boot_wait_abandons_the_wait_and_tears_the_vmm_down`, `the_signal_wins_over_the_boot_deadline_at_the_same_instant`, `a_volume_launch_superseded_in_its_boot_wait_tears_the_vmm_down_and_keeps_the_volume`; guard `guard_a_launch_that_won_its_boot_race_before_the_signal_is_an_ordinary_success` | PENDING; GUARD |
| S-SUP-06 | 6: the second boot wait on the beacon-error arm | `a_signal_in_the_second_boot_wait_abandons_it_and_tears_down`, `the_signal_wins_over_the_second_boot_deadline_at_the_same_instant` | PENDING |
| S-SUP-07 | boundary 4a (c): a failing teardown is `StartRejected`, never `StartSuperseded` | `a_failing_teardown_is_reported_as_a_start_rejection_never_as_superseded` | PENDING |
| S-SUP-08 | boundary 4a (d), (e): only its own launch is abandoned; an unraised signal leaves the launch ordinary (with and without a volume) | `a_signal_abandons_only_its_own_launch` (PENDING), `guard_a_signal_raised_for_another_launch_does_not_affect_this_one`, `guard_a_launch_whose_signal_is_never_raised_is_the_existing_launch` (GUARDs), `a_volume_launch_whose_signal_is_never_raised_is_the_ordinary_volume_launch` (PENDING: asserts the ensured device, prefix `/dev/mapper/overdrive-ovd--`, is the VMM's volume attachment; goes GREEN with the launch path, before the supersession points exist) | PENDING; GUARD |

**D2. Lane table and handlers** — `crates/overdrive-control-plane/tests/acceptance/launch_lane_supersession.rs`, Tier 1 layer 2. Owner path: the real
`stop_workload` / `restart_workload` / `submit_workload` handlers -> `ReconcilerRuntime` lane table -> the real tick
(`run_convergence_tick_with_guest_network_provisioner_for_test`: lane open, hydration, `WorkloadLifecycle::reconcile`, the real action shim) -> a `SimDriver` start the
test holds with `park_next_start`. Fault stimulus: a held launch, the guest-network owner's teardown failure. Oracle: the parked start's `observed_supersession()`, the
tick's result, the rows, `restart_counts`, the identity manager's held SVIDs, the owner's teardown calls, the driver's `start` / `stop` / `release_supervision` calls. No test
overlaps two ticks of one target (ADR-0102 admits one lane per target). The tests assert that the commit is visible when the signal is seen (the handler commits, then
signals); the handlers' ordering of the signal before the evaluation is enqueued is not observable without new surface (§ L, g). The raised-signal property of `supersede_all_lanes`
(a lane opened before, between or after the call; born raised) is covered by the seeded schedule S-SUP-18 (seeds 0..4 place the call at positions 3, 2, 0, 3 of the lane opens); the
per-position tests were removed as duplicates of it.

| Id | Scenario | Test | Status |
|---|---|---|---|
| S-SUP-10 | `supersede_lane` with no open lane does nothing (A8, boundary 4a (a)) | `supersede_lane_with_no_open_lane_does_nothing` | GUARD |
| S-SUP-12 | a signal after the lane opens and before hydration is visible on that lane (A8) | `a_signal_after_the_lane_opens_and_before_hydration_is_visible_on_the_lane` | PENDING |
| S-SUP-13 | one workload's signal never raises another's lane (4a (e)); the superseded workload ends one abandoned row (**changed by ADR-0355**; was GUARD) | `supersede_lane_for_one_workload_never_raises_another_workloads_lane` | PENDING |
| S-SUP-14 | `supersede_lane` never cancels the shutdown token (4a (h)) | `supersede_lane_never_cancels_the_shutdown_token` | PENDING |
| S-SUP-18 | at any point of a seeded schedule every launch abandons, each ending one abandoned row (A9; seeds 0..4, printed; **changed by ADR-0355**) | `supersede_all_lanes_at_any_point_of_a_seeded_schedule_abandons_every_launch` | PENDING |
| S-SUP-19, 20 | the lane guard removes its entry by key: a closing lane removes only its own entry and leaves no raised signal behind (one test) | `a_closing_lane_removes_only_its_own_entry_and_leaves_no_raised_signal_behind` | PENDING |
| S-SUP-22 | a repeated stop (`KeyExists`) raises the open lane after its commit; the `Inserted` stop and the restart are covered by S-SUP-26 and 27 | `a_repeated_stop_that_finds_the_sentinel_raises_the_open_lane_too` | PENDING |
| S-SUP-24, 25 | `submit_workload` (new, unchanged, conflicting) and resync-style evaluations of a busy target never raise the lane (4a (f)) | `submit_workload_new_unchanged_and_conflicting_never_raises_the_lane_signal`, `evaluations_submitted_for_a_busy_target_never_raise_its_lane` | GUARD |
| S-SUP-26 | a restart while a launch waits ends it at once, its `Pending` row ends as one abandoned row (`LaunchSuperseded`), no charge; lease, SVID and supervision claim released (**changed by ADR-0355**) | `a_restart_while_a_launch_waits_ends_it_at_once_and_charges_nothing` | PENDING |
| S-SUP-27 | a stop while a launch waits ends it (one abandoned row, no relaunch); the workload stays down until the next restart (**changed by ADR-0355**: the row exists) | `a_stop_while_a_launch_waits_ends_it_and_the_workload_stays_down_until_the_next_restart` | PENDING |
| S-SUP-28 | a second restart while the replacement waits ends the successor's `Pending` row as abandoned and runs the exact old predecessor cleanup (**changed by ADR-0355**) | `a_second_restart_while_the_replacement_launch_waits_runs_the_exact_old_cleanup` | PENDING |
| S-SUP-29 | a signal with no lane open does nothing and the evaluation's hydration sees the commit (A8, before the lane opens) | `a_signal_with_no_lane_open_does_nothing_and_hydration_sees_the_commit` | GUARD |
| S-SUP-31, 32 | a teardown failure while abandoning reaches the shim as the unclassified rejection (4a (c)); a driver that ignores the signal is unchanged (4a (d)) | `a_teardown_failure_while_abandoning_reaches_the_shim_as_an_unclassified_rejection`, `a_driver_that_ignores_the_signal_is_unchanged` | GUARD |
| S-SUP-33 | a restart whose commit the evaluation already hydrated sees its own `Start` superseded once; the next evaluation relaunches (4a (g)). Needs both the lane table and the generation rule (`red-classification.md` § "Tests that need two activation rows") | `a_restart_the_evaluation_already_hydrated_sees_its_own_start_superseded_once` | PENDING |

**D3. Node shutdown and the boundary 4 composition** — `crates/overdrive-control-plane/src/node_volume_boundaries.rs`, Tier 1 layer 2. `a_stop_interrupts_the_start_and_its_result_is_never_used` and the S-VOL-65 test of § C
drive the production `serve` composition of the volume conformance driver; the `loop_shutdown` module drives `spawn_convergence_loop` itself over sim ports with a `SimDriver` start held by `park_next_start`.

| Id | Scenario | Test | Status |
|---|---|---|---|
| S-SUP-34 | boundary 4a (h): node shutdown ends a waiting launch (`StartSuperseded`, its `Pending` row ends abandoned), the loop drains and exits, no simulated time passes (**changed by ADR-0355**; renamed) | `loop_shutdown::node_shutdown_ends_a_waiting_launch_with_an_abandoned_row_and_the_loop_exits` | PENDING |
| S-SUP-36 | shutdown leaves a `Running` allocation and a non-launch evaluation alone | `loop_shutdown::shutdown_leaves_a_running_allocation_and_a_non_launch_evaluation_alone` | GUARD |
| S-SUP-37 | boundary 4: a stop interrupts the start and its result is never used (no VMM attached or left alive, the volume kept, one abandoned row and no other, no charge; **changed by ADR-0355**). Kept beside S-SUP-27: it runs a volume-launching `VmDriver` with a held claim, a composition distinct from S-SUP-27's parked `SimDriver`, and its unique oracles are `attached` empty and the claim staying `Held` | `a_stop_interrupts_the_start_and_its_result_is_never_used` | PENDING |

## E. Restart generation and the launch outcomes before the driver (ADR-0335, ADR-0337, ADR-0338; boundaries 5 and 7)

**E1. The one definition of `restart_pending` and the reconcile behaviour** (Tier 1 layer 1, `crates/overdrive-reconcilers/tests/acceptance/`).

| Id | Contract / oracle | Test | Status |
|---|---|---|---|
| S-RG-01 | `generation_replacement_pending` is true iff the current row (highest numeric `-<N>` suffix among observed rows) exists, is not a finished Job row (`kind == Job && terminal.is_some()`), and its `issued_generation` entry (absent reads 0) is below the generation: 14 examples and two properties (iff against an independent restatement, with equality generated to kill the `<` mutant; agreement with `allocation_attempt_transition`'s `NoChange`, RA10) | `generation_replacement_pending.rs` | PENDING (the scaffold's unwind is caught and re-raised as the contract assertion) |
| S-RG-02 | Through the real `WorkloadLifecycle::reconcile`, evaluations chained on the returned View (the only injected fault is the lost launch: the issued id's row is absent): a lost launch is relaunched once for the pending generation with an entry `desired.generation`; a generation replacement carries `restart_counts` and `last_failure_seen_at` unchanged; with the row present nothing is issued; N bumps before placement coalesce (S-BIR-COALESCE-PLACE); an evaluation after the successor's row exists does not re-place (S-BIR-COALESCE-NO-REPLAY); a crash-restart relaunch is charged once; a first placement records the committed generation; a restart never gives a completed, failed or operator-stopped Job a new allocation (H1), while a Job row with only a reason is replaced as on `main` | `restart_generation_reconcile.rs` | PENDING: `a_lost_launch_is_relaunched_once_for_the_pending_generation`, `an_abandoned_replacement_relaunch_charges_nothing`, `a_restart_never_gives_an_operator_stopped_job_a_new_allocation`; the rest GUARD |

**E2. The action shim's launch outcomes** — `crates/overdrive-control-plane/src/action_shim/prelaunch_outcome_acceptance.rs`, Tier 1 layer 2. Owner path: the shim's pinned
driving port (`dispatch_with_guest_network_provisioner_for_test`) or the real `WorkloadLifecycle` tick, over the production `GuestAddressPool`, `IdentityMgr` and `MtlsInterceptWorker` on sim
ports and `SimObservationStore`. Fault stimulus: a `Ca` double whose `issue_svid` fails, a `Driver` double returning `StartSuperseded`, `SimDriver` scripts, the owner's provision and
teardown refusals, an observation wrapper whose point read fails or whose point-read hook takes the pool's last slot between hydration and admission (the lost race of ADR-0338). The
`Failed` predecessor rows of the refusal and teardown tests are written in the exit observer's shape; every other row is authored by production.

| Id | Contract (work-order item) | Test | Status |
|---|---|---|---|
| S-SHIM-01 | N-SUP, **changed by ADR-0355 W1/W2 (RB-8)**: a superseded launch ends its `Pending` row with one `Terminated` row `LaunchSuperseded` (terminal `None`, `started_at` `None`, the lease's address, the `Pending` row's `restart_count` 0 and `last_terminated` `None`), releases the lease, SVID and supervision claim, returns `Ok`; in the restart arm the exact-old predecessor cleanup (`finish_restart`) still runs; a failing teardown does not suppress the row; a failing row write is the typed `ShimError::Observation` (see § N) | `start_arm_superseded_launch_ends_its_row_as_abandoned_and_releases_lease_svid_and_claim`, `restart_arm_superseded_successor_ends_its_row_as_abandoned_and_runs_exact_old_predecessor_cleanup`, `start_arm_teardown_failure_does_not_suppress_the_abandoned_row`, `start_arm_failing_abandoned_row_write_is_returned_as_the_typed_observation_error` (were the two no-row guards of 05-01) | PENDING |
| S-SHIM-02 | S5, R7, Job (3a i): an identity issuance failure writes a `Failed` row `MtlsInterceptInstallFailed { stage: "identity_issue" }` carrying the cause, the lease released, the dispatch `Ok`; in the restart arm the predecessor cleanup runs; a Job's row is the same; a teardown failure does not suppress the row | `start_arm_identity_issue_failure_writes_a_failed_row_naming_the_cause`, `restart_arm_identity_issue_failure_writes_a_failed_successor_row_then_cleans_the_predecessor`, `a_job_identity_issue_failure_writes_the_same_failed_row`, `start_arm_teardown_failure_does_not_suppress_the_identity_issue_row` | PENDING |
| S-SHIM-03 | R3 (3a ii): an empty driver registry in a restart writes a `Failed` successor row, not `HandleMissing` | `restart_with_no_driver_composed_writes_a_failed_successor_row_not_handle_missing` | PENDING |
| S-SHIM-04 | S3, S6 unchanged: a guest-network provision failure and a start with no driver keep their existing row | `start_arm_guest_network_provision_failure_row_is_unchanged`, `start_arm_with_no_driver_composed_row_is_unchanged` | GUARD |
| S-SHIM-05 | S0, R0 (3a v): a failing prior-row read returns the observation error with no effect and no row | `start_arm_failing_prior_row_read_returns_the_observation_error_with_no_effect`, `restart_arm_failing_prior_row_read_returns_the_observation_error_with_no_effect` | GUARD |
| S-SHIM-06 | N-ADM (3a iv): an admission refusal writes no row and charges nothing; the start arm leaves `restart_counts` as at the predecessor and the next hydration emits nothing at the cap; the restart arm releases the predecessor's lease and the next hydration emits again with the same count | `start_arm_admission_refusal_writes_no_row_charges_nothing_and_the_next_tick_emits_nothing`, `restart_arm_admission_refusal_releases_the_predecessor_and_the_next_tick_emits_again` | GUARD |
| S-SHIM-07 | Waiting item 6: after an admission refusal in each arm the predicate `start_blocked_only_by_capacity` over the resulting rows, View and pool is true (the restart-arm case is a refusal whose predecessor held no lease; the lease-holding case is S-SHIM-06) | `after_a_start_arm_admission_refusal_the_wait_predicate_is_true`, `after_a_restart_arm_admission_refusal_the_wait_predicate_is_true` | PENDING |
| S-SHIM-08 | The charge (3a iii): a persistent identity failure is charged until the restart ceiling, then `BackoffExhausted` (the S5 then R7 chain); a Job's identity failure is finalised `Failed { exit_code: Some(0) }` and never relaunched (J-1) | `a_persistent_identity_failure_is_charged_until_the_restart_ceiling_then_exhausted`, `a_job_identity_failure_is_final_and_never_relaunched` | PENDING |
| S-SHIM-09 | Existing outcomes the new arms must not disturb: the start arm for a finalised Job row is a no-op (S1); the restart arm fences a finalised Job predecessor and a running predecessor (R2); a guest-network provision failure writes a `Failed` successor row (R6); a duplicate VM owner is a no-row success that still cleans the predecessor (R9); activation withheld at fail-stop leaves the `Running` row in the start arm (S10) and skips the predecessor cleanup in the restart arm (R14); a launch that won its boot race before the stop signal is stopped as an ordinary `Running` allocation (passes today because `supersede_lane` is a no-op; it must keep passing when the lane table lands) | `start_arm_for_a_finalised_job_row_is_a_no_op`, `restart_arm_with_a_finalised_job_predecessor_is_fenced`, `restart_arm_with_a_running_predecessor_is_fenced`, `restart_arm_guest_network_provision_failure_writes_a_failed_successor_row`, `restart_arm_duplicate_vm_owner_is_a_no_row_success_that_still_cleans_the_predecessor`, `start_arm_activation_withheld_at_fail_stop_leaves_the_running_row_and_returns_ok`, `restart_arm_activation_withheld_at_fail_stop_skips_the_predecessor_cleanup`, `a_launch_that_won_its_boot_race_before_the_stop_signal_is_stopped_as_an_ordinary_running_allocation` | GUARD |

There is no S8 or R10 test: a non-start driver error is not a decision of this feature (DE-1). Outcomes covered elsewhere and not repeated here: S7
(`tests/acceptance/lifecycle_broadcast.rs::barriered_starting_and_live_duplicate_actions_preserve_the_real_shim_row_and_event`), S9
(`tests/acceptance/action_shim_running_write_failure_stops_alloc.rs`: `running_write_failure_stops_the_started_alloc`, `vm_running_write_failure_releases_the_supervision_claim`; and
`tests/integration/mtls_install_fail_closed.rs::start_running_write_rejection_...`), R11 (`mtls_install_fail_closed.rs::restart_running_write_rejection_retires_the_lease_before_teardown_and_releases_it_last`), and the
Failed-row half of S10 (`mtls_install_fail_closed.rs::tap_activation_failure_stops_vmm_cleans_mtls_and_network_and_dominates_running`, `tap_activation_failure_retires_the_lease_and_releases_it_last`).

**E3. The seeded invariant `restart_generation_is_placed_exactly_once_across_supersession_and_crash`** — S-INV-01,
`crates/overdrive-control-plane/tests/integration/restart_generation_invariant.rs` (the crate's `integration-tests` entrypoint; module-level `placement!(host_pure)` because the
binary also holds guest-placed tests; seeds `348_001..=348_006` from `SimEntropy`, printed with the cell and the schedule on failure; `the_same_seed_gives_the_same_trace` pins
determinism). Tier 1 layer 2, no `quint`. Owner path (production end to end): the real `submit_workload`, `stop_workload` and `restart_workload` handlers, the real convergence tick
(lane table, hydration, `WorkloadLifecycle::reconcile`, View persistence over `SimViewStore`), the real action shim over the production `GuestAddressPool`, `IdentityMgr` and
`MtlsInterceptWorker`. Fault stimulus, only at driven ports: a `Ca` wrapper over `SimCa` whose `issue_svid` fails (none, persistent, transient), `SimDriver::park_next_start(true)`,
`ReconcilerRuntime::supersede_all_lanes`, and a control-plane crash (the tick future is dropped mid-dispatch, the runtime rebuilt over the persisted View, the intent store reopened
from its file, `vm_reclamation_boot::converge` run as `run_server` does). Oracle: the rows, `issued_generation_for_alloc`, `restart_counts` through `view_for_workload_lifecycle`, the
generation key, every `Driver::start` call (outcome and concurrency), the CA's `issue_svid` attempts. The rig's `settle()` waits on observable quiescence (cap 20,000 yields; the panic names the seed).

| Dimension | Assertions (work-order item 1) | Tests | Status |
|---|---|---|---|
| origin {restart of a `Running` workload, resume of a stopped one} x interruption {stop, second restart, shutdown, control-plane crash}, then the next boot | (a) eventually a `Running` allocation for the committed generation, or down after a stop; (b) at most one replacement launch in flight and one relaunch per abandoned launch; (c) no `Failed` row, no `restart_counts` change; (d) a second evaluation issues nothing | `restart_of_a_running_workload_{interrupted_by_stop,overridden_by_a_second_restart,abandoned_by_shutdown,lost_with_the_control_plane}_...` and the `resume_of_a_stopped_workload_*` four (full names in `red-classification.md`) | PENDING |
| crash-restart successor | an abandoned crash-restart successor is relaunched with the same count (charged once) | `a_crash_restart_successor_lost_with_the_control_plane_is_relaunched_with_the_same_count` (GUARD), `a_crash_restart_successor_abandoned_by_shutdown_is_relaunched_with_the_same_count` (PENDING) | GUARD; PENDING |
| pre-launch failure {persistent, transient} | (e) persistent: a Service ends `Failed` with `BackoffExhausted`, launches after one operator commit at most the restart ceiling plus the one uncharged replacement, and within one backoff window the replacement plus at most one ungated crash restart, then at most one launch per window (§ L, d); (f) transient: the Service recovers to `Running` with the failures presenting as `MtlsInterceptInstallFailed` rows; (g) `issued_generation` and `restart_counts` gain no more entries than launches. The origin (first placement, restart, resume) loops inside each test; a transient fault followed by an interrupted launch is one test | `a_persistent_identity_failure_ends_backoff_exhausted`, `a_persistent_identity_failure_launches_a_bounded_number_of_times_within_one_backoff_window`, `a_transient_identity_failure_recovers_to_running`, `a_transient_identity_fault_then_an_interrupted_launch_places_once` | PENDING |
| workload kind (H1, J-1) | (h) a restart of a running Job ends it and issues nothing after (no id, no `issued_generation` or `restart_counts` entry, no launch); a Job whose SVID issuance persistently fails writes one `Failed` row, is finalised `Failed { exit_code: Some(0) }` and is never relaunched. The finished-Job and operator-stopped-Job restarts and the transient Job fault are not re-tested here: S-RG-02 and S-Q-22, 23 cover the fence, and the shim tests cover the identity row | `a_restart_of_a_running_job_ends_it_and_issues_nothing_after`, `a_job_whose_identity_issuance_persistently_fails_is_finalised_failed_and_never_relaunched` | PENDING |
| no interruption, stop | a restart with no interruption places once and a second evaluation issues nothing; an operator stop keeps the workload down until the next restart; the same seed gives the same trace | `a_restart_with_no_interruption_places_once_and_a_second_evaluation_issues_nothing`, `an_operator_stop_keeps_the_workload_down_until_the_next_restart`, `the_same_seed_gives_the_same_trace` | GUARD |

**E4. Boot reclamation (M7)** — `crates/overdrive-control-plane/src/vm_reclamation_boot.rs`, module `mid_boot_volume_reclamation` (integration-tests lane, `host_pure`), Tier 1 layer 2. Two boots of the production
composition `run_server_with_obs_and_driver` over one data directory, one `SimVmm`, one `SimVolumeBacking`, one `SimVmmHost`, one observation store and one `SimClock`. The workload is deployed through the public HTTPS API; the
first process parks a volume launch in the guest-boot wait and is killed (`ServerHandle::kill_for_test`); the second process's own boot runs the boot reclamation, nothing in the test calls `converge`. A `StartRecordingDriver` records every
`Driver::start` with the orphan's liveness at that moment. Oracle at the driven ports: exactly one start, a new allocation, the orphan not live at that start and at the one `Vmm::create` the successor makes, the pool's claim `Free` at that
create (the orphan's claim drained), the successor attaches the volume, no `Failed` row and no `VmVolumeUnavailable` reason.

| Id | Scenario | Test | Status |
|---|---|---|---|
| S-M7-1 | the crash leaves the launch's `Pending` row only (ADR-0353); boot's closing step ends it `LaunchLostWithControlPlane`, address carried, and the executors discard its artifacts (**changed by RB-11**: was "no row") | `boot_reclamation_ends_a_mid_boot_vmm_before_any_launch_for_its_workload` | PENDING |
| S-M7-2 | the crash leaves a `Running` row (`ReclaimAllocation`), platform-reclaimed, address carried (the precondition now also asserts the `Pending` row the parked launch recorded) | `boot_reclamation_ends_a_mid_boot_vmm_when_the_crash_left_a_running_row` | PENDING |

## F. A workload waiting for guest-network capacity (ADR-0336, ADR-0338; W-3 = A)

| Id | Contract / oracle | Owner path (fault) | Tier | Test and file | Status |
|---|---|---|---|---|---|
| S-CAP-01 | `start_blocked_only_by_capacity(kind, run_desired, rows, view, generation, observation)` over the class of the current row, each with the pool at `cap - 1`, `cap`, `cap + 1`: no row, crashed under budget, `Stopped{SystemGc}`, liveness-killed and startup-probe-killed under budget, platform-reclaimed even at the ceiling, `Draining` historical beside a newer crashed row (true); at the ceiling, `BackoffExhausted`, `ServiceFailed`, operator-stopped (true only once a generation is pending); a finished Job, a natural-exit Job, a leased current row, a stopped workload, a `Running` row, a `Running` historical row, a `Draining` current row, a pending-state row, a Schedule (false); Job rows: no row, current `Reclaimed`, current `SystemGc` (true), a historical `Completed` beside a current `Reclaimed` (false); a lease on a historical row does not make the current row leased; retiring leases count toward `held`. Three agreement properties against `WorkloadLifecycle::reconcile`: (a) blocked means no start; (b) with the observation lowered to `cap - 1` and the backoff elapsed, blocked means exactly one start; (c) a start a freed slot releases is a blocked workload. RB-13: a current row that is an abandoned launch (`LaunchSuperseded`, `LaunchLostWithControlPlane`) is classified through the count basis (the newest row that ran), not as itself, so a workload whose newest row is abandoned is blocked or not exactly as its basis row says (not authored: needs the two `TransitionReason` variants, § N). Non-vacuity: a guard that the generator reaches withheld and released starts, and a pending test that it reaches both predicate verdicts | the pure predicate; `Reconciler::reconcile` | Tier 1 | `crates/overdrive-reconcilers/tests/acceptance/start_blocked_only_by_capacity.rs` | PENDING (the scaffold's unwind is caught and the test fails on its named assertion); GUARD `the_agreement_generator_reaches_starts_withheld_starts_and_released_starts` |
| S-CAP-02 | the `alloc_status` response carries `capacity_wait { resource, held, cap }` exactly while the start is blocked only by capacity (a first placement at the cap here; the restart-at-cap case is S-CAP-03); `held` and `cap` are the pool's; a running workload with room, a running one on a full node, a finished Job on a full node and an operator-stopped workload on a full node show none. The one test that fills the real pool to 16,384 attachments through production `StartAllocation` dispatches covers the full-node cases; two guards cover the no-fill cases | real `submit/stop/restart_workload` and `alloc_status` handlers over `AppState`, production reconciler and shim, the real `GuestAddressPool` (fault: a full pool; no row, lease or View entry seeded); no HTTP router | Tier 1, integration lane | `crates/overdrive-control-plane/tests/integration/alloc_status_capacity_wait.rs::a_full_node_reports_the_wait_for_a_withheld_first_placement_and_for_no_other_workload` (pending); `crates/overdrive-control-plane/tests/acceptance/alloc_status_capacity_wait.rs` (two no-fill guards) | PENDING; GUARD |
| S-CAP-03 | composed (W-3 = A): `deploy` onto a full node and `workload restart` that loses its slot: no allocation row, no `Failed` row, `restart_counts` unchanged (map equality) and (restart) the predecessor's lease released; the response carries `capacity_wait { resource, held, cap }`; free one slot, run the target's `workload-lifecycle` evaluation (the resync pass realised as the per-target evaluation it submits, because the pass is crate-private), and the workload is placed, reaches `Running` and the response drops the wait. It does not assert the CLI-rendered `Waiting` line (§ L, blocker t); that line is pinned by the cli `workload_describe*.rs` tests | handlers, runtime, shim, pool; driven ports `SimDriver`, `SimObservationStore`, `SimCa`, `SimClock`, `SimDataplane`, a ledger over the sim guest-network owner | Tier 1, integration lane | `crates/overdrive-control-plane/tests/integration/capacity_wait_composition.rs::a_deploy_onto_a_full_node_and_a_restart_that_loses_its_slot_wait_without_charge_and_resume_when_a_slot_frees` (pending); `tests/acceptance/capacity_wait_composition.rs` holds the shared rig only | PENDING |
| S-CAP-04 | render goldens: the `Waiting` line for a Service and a Job (one test), right after the kind-aware body and before the shared sections, exactly one added line, the response's counts; output byte-identical when `capacity_wait` is `None`. The earlier wire-shape guards (field present and absent, round trip, malformed refusal, label equals serde label) were removed in the subtractive pass; the wire type is exercised through the handler test and the render goldens | pure `render::workload_describe` over `AllocStatusResponse` | Tier 1 | `crates/overdrive-cli/tests/acceptance/workload_describe_capacity_wait.rs` (`describe_shows_the_waiting_line_for_a_waiting_service_and_job` and three more pending) | PENDING; the one remaining guard is `guard_a_workload_that_is_not_waiting_renders_without_the_waiting_line` |
| S-CAP-05 | the generic "check that a node is eligible" empty state is suppressed when `capacity_wait` is `Some`, and unchanged when `None` | the real `describe` command against a stub HTTPS control plane (`commands::workload::describe`) | Tier 3 placement (`guest = "cgroup-default"`, the integration binary) | `crates/overdrive-cli/tests/integration/workload_describe.rs`, module `capacity_wait_empty_state` | PENDING; GUARD |

## G. Quint conformance — S-Q-01..24

`quint-connect` trace replay against the production owner; the spec is referenced in place and never copied. Each test is `#[quint_test(.., max_samples = 1)]` for a named run, or
`#[quint_run]` for randomized traces (seed printed on failure). Lane: `integration-tests`, `host_pure`. The Safety invariants of the spec are model-checked in DESIGN; the replay compares
the driver's `State` (projected only from the owner's pinned accessors) with the spec's state after every step, so each run replays the invariant its scenario defends (column "Defends").
A spec action's handler always calls a production driving port; the exceptions with no owner port are named below.

**Volume spec**: `specs/quint/node-local-volume/node_volume.qnt`, main `vol_conf`, driver `crates/overdrive-control-plane/src/quint_node_volume.rs`. Each trace passes `Init` and
diverges at step 2, `Declare` (the production parser refuses `[vm.volume]`); every handler after `Declare` is compile-checked only. `GuestWrite` and `DeviceSlow` have no owner port
(guest I/O and in-flight device latency are guest and kernel facts) and check the action's guard against production; `VmmSpawnFail` is a driven-port fault driven through
`SimVmm::inject_create_failure` (§ L, e). The driver's one `sim-arbiter-ok` read is `claim()`, citing S-VOL-T3-04
(`volume_pool_backing.rs::claim_state_reports_held_free_and_not_found_as_the_kernel_does`) and the `alive[a]` read of the recording `Vmm` double (`vmm_live`, restored under B-2), citing S-VOL-T3-13; the other arbiter markers were removed. `charged` stays
model-only. `LaunchUnavailable`, `LaunchHeldFail` and `LaunchCmdFail` assert the `Failed` row's reason through the owner's rows, matched by name (the `VmVolumeUnavailable` variant does not exist yet).
The `alive[a]` row of the feature delta is projected from the recording `Vmm` double as its named arbiter, paired with S-VOL-T3-13 (§ L, B-2 resolved).

| Id | Run (boundary or decision) | Test | Defends | Status |
|---|---|---|---|---|
| S-Q-01 | `happy_path_deploy_to_running` (1) | `quint_vol_happy_path_deploy_to_running` | `I_SINGLE_WRITER`, `I_DATA_SURVIVES_RESTART` | PENDING |
| S-Q-02 | `restart_waits_for_drain_then_attaches` (VD-8, 4) | `quint_vol_restart_waits_for_drain_then_attaches` | `I_NO_DRAIN_FAILURE_CHARGES_BUDGET` | PENDING |
| S-Q-03 | `stop_keeps_volume_and_restart_reattaches` (VD-2) | `quint_vol_stop_keeps_volume_and_restart_reattaches` | `I_STOP_KEEPS_VOLUME` | PENDING |
| S-Q-04 | `crash_after_create_before_attach_reuses_volume` (VD-9, 5) | `quint_vol_crash_after_create_before_attach_reuses_volume` | `I_ENSURE_IDEMPOTENT_UNDER_CRASH` | PENDING |
| S-Q-05 | `node_reboot_reactivates_and_data_survives` (5) | `quint_vol_node_reboot_reactivates_and_data_survives` | `I_DATA_SURVIVES_RESTART` | PENDING |
| S-Q-06 | `launch_bound_exceeded_fails_typed_and_charges_budget` (2) | `quint_vol_launch_bound_exceeded_fails_typed_and_charges_budget` | `I_SINGLE_WRITER`; the typed charged failure is asserted by the run | PENDING |
| S-Q-07 | `no_space_fails_typed_and_never_attaches` (2) | `quint_vol_no_space_fails_typed_and_never_attaches` | asserted by the run ("a typed failure never attaches") | PENDING |
| S-Q-08 | `store_absent_fails_typed_and_never_attaches` (2; the spec's "store" is the volume pool) | `quint_vol_store_absent_fails_typed_and_never_attaches` | asserted by the run | PENDING |
| S-Q-09 | `superseded_launch_result_discarded` (4, RA4) | `quint_vol_superseded_launch_result_discarded` | `I_SUPERSEDE_ABANDONS_WAITS`, `I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING` | PENDING |
| S-Q-10 | `stop_during_boot_wait_terminates_vmm_and_keeps_volume` (4, A7) | `quint_vol_stop_during_boot_wait_terminates_vmm_and_keeps_volume` | `I_SUPERSEDE_ABANDONS_WAITS`, `I_STOP_KEEPS_VOLUME` | PENDING |
| S-Q-11 | `stop_during_ensure_lets_ensure_finish_then_abandons` (4a, point 2) | `quint_vol_stop_during_ensure_lets_ensure_finish_then_abandons` | `I_SUPERSEDE_ABANDONS_WAITS`, `I_SIGNAL_NEVER_LEAKS_ACROSS_ALLOCATIONS` | PENDING |
| S-Q-12 | `shutdown_abandons_every_launch_including_one_admitted_late` (4a (h), `Boot`) | `quint_vol_shutdown_abandons_every_launch_including_one_admitted_late` | asserted by the run ("shutdown signals every launch") | PENDING |
| S-Q-13 | randomized `vol_conf` traces | `quint_vol_randomized` | the whole `Safety` conjunction | PENDING |

**Restart-generation spec**: `specs/quint/restart-generation/restart_generation.qnt`, mains `rg_conf` (eight Service runs) and `rg_conf_job` (two Job runs), driver
`crates/overdrive-control-plane/src/quint_restart_generation.rs`. The driver's owner is the `ReconcilerRuntime` through the real handlers, convergence tick and shim. It
holds the dispatch at a one-shot `ObservationStore::alloc_status_row` read (§ L, c); `LaunchLost` is `supersede_lane` on the held dispatch with neither `capFull` nor `idFault`
armed. It has no `Shutdown`, `Boot` or `LaunchSuperseded` arm: the model has no lane signal. The Job runs fail on any issued allocation id that no model allocation names and on any
row with no model label, with one further unheld tick after the Job restart or resume.

| Id | Run (decision) | Test | Defends | Status |
|---|---|---|---|---|
| S-Q-14 | `control_plane_crash_during_restart_launch_relaunches_once` (SD-6, A10) | `quint_rg_control_plane_crash_during_restart_launch_relaunches_once` | `RunIntentProgress` | PENDING (diverges at step 8, `WlStart`) |
| S-Q-15 | `second_restart_loses_launch_then_one_placement` (SD-6) | `quint_rg_second_restart_loses_launch_then_one_placement` | `RunIntentProgress` | PENDING (step 10, `LaunchLost`) |
| S-Q-16 | `resume_while_running_stops_and_replaces` | `quint_rg_resume_while_running_stops_and_replaces` | `RunIntentProgress` | GUARD |
| S-Q-17 | `restart_of_exhausted_workload_relaunches_uncharged` (R-1) | `quint_rg_restart_of_exhausted_workload_relaunches_uncharged` | `I_CAPACITY_WAIT_NEVER_EXHAUSTS` | PENDING (step 5 projection) |
| S-Q-18 | `generation_replacement_forgives_predecessor_failure` (R-1, RA9) | `quint_rg_generation_replacement_forgives_predecessor_failure` | the carried count (`cnt` compared through `attempt_index_for_alloc`) | PENDING (step 5) |
| S-Q-19 | `persistent_prelaunch_failure_ends_in_backoff_exhausted` (ADR-0337) | `quint_rg_persistent_prelaunch_failure_ends_in_backoff_exhausted` | `PreFailExhausts` | PENDING (step 5) |
| S-Q-20 | `transient_prelaunch_failure_recovers` (ADR-0337) | `quint_rg_transient_prelaunch_failure_recovers` | `PreFailExhausts`, `RunIntentProgress` | PENDING (step 5) |
| S-Q-21 | `admission_refusal_leaves_no_row_and_waits_for_capacity` (D-1, ADR-0338) | `quint_rg_admission_refusal_leaves_no_row_and_waits_for_capacity` | `I_CAPACITY_WAIT_NEVER_EXHAUSTS` | GUARD |
| S-Q-22 | `restart_of_completed_job_does_nothing_and_issues_no_id` (H1, `rg_conf_job`) | `quint_rg_restart_of_completed_job_does_nothing_and_issues_no_id` | `I_FINISHED_JOB_ISSUES_NO_ID` | GUARD |
| S-Q-23 | `resume_of_stopped_job_does_nothing_and_issues_no_id` (H1, `rg_conf_job`). Was a guard in the earlier cut; the stricter Job projection showed that production gives a resumed operator-stopped Job a new allocation id, so the guard was vacuous | `quint_rg_resume_of_stopped_job_does_nothing_and_issues_no_id` | `I_FINISHED_JOB_ISSUES_NO_ID` | PENDING (`the Job issued allocation id`) |
| S-Q-24 | randomized `rg_conf` traces | `quint_rg_randomized` | the whole `Safety` conjunction | PENDING (the first divergence a seed hits) |

All 22 named runs of the design's table are pinned (12 volume, 10 restart-generation). Boundary scenarios 3 (unrelated state) and 6 (feature disabled) have no model run: S-VOL-43, S-VOL-46,
S-SUP-36 and the boot guards cover them.

## H. Real adapter and kernel (Tier 3, guest, example-only, sad paths named)

| Id | Scenario | Test | Placement | Status |
|---|---|---|---|---|
| S-VOL-T3-01 | A guest with no labelled partition composes no pool | `crates/overdrive-host/tests/integration/volume_pool_probe.rs::guard_a_node_without_a_labelled_partition_composes_no_volume_pool` | `cgroup-default` base | GUARD |
| S-VOL-T3-02 | Healthy pool probes clean and leaves nothing; leftover `ovd-probe-` volume removed first; workload volumes untouched; full pool still probes | `volume_pool_probe_pool.rs` | `volume-pool` | PENDING-ignored |
| S-VOL-T3-03 sad | Unusable `lvm` (`Io`); missing group (`CommandFailed`); ambiguous label (`AmbiguousLabel`); scratch node cannot appear; scratch held open; each asserts the typed `detail` variant | `volume_pool_probe_pool.rs` | `volume-pool` | PENDING-ignored |
| S-VOL-T3-04 | `claim_state` pairing: `Held` by an `O_EXCL` opener, `Free` after release, `NotFound` deactivated, `Free` re-activated (the conformance arbiter's pairing) | `volume_pool_backing.rs::claim_state_reports_held_free_and_not_found_as_the_kernel_does` | `volume-pool` | PENDING-ignored |
| S-VOL-T3-05 | `ensure`: exact size reserved, activated, hashed name; idempotent; inactive activated; `claim_state` of an absent volume; `InsufficientSpace` figures (the `B`-suffixed sizes of the `lvm` report are exercised here); `PoolCommand`; deadline as `TimedOut`; two workloads at once | `volume_pool_backing.rs` | `volume-pool` | PENDING-ignored |
| S-VOL-T3-06 | Retired: the `lvm` report and exit-status contract test. The `B`-suffixed sizes are exercised by `ensure`'s `InsufficientSpace` figures and the exit-5 and exit-4 mapping by the probe-pool scenarios | | | retired |
| S-VOL-T3-07 | `serve` + `deploy`: data survives `workload restart` (node mode `root:<gid>` `0660`, argv golden and disk order from the live process, describe shows the volume); a flushed record survives a SIGKILLed VMM; stop keeps the volume and resume reattaches | `crates/overdrive-cli/tests/integration/volume_survives.rs` | `volume-pool` (`kvm-tests`) | PENDING-ignored |
| S-VOL-T3-08 | Retired (dropped in the subtractive pass): node reboot simulated by what it leaves. The reboot is the EDD expectation E16 (`verification/expectations/E16-node-local-volume-survives-restart-and-reboot/`); the in-process S-VOL-45 and S-Q-05 cover reactivation | | | retired |
| S-VOL-T3-09 | single writer across a replacement while the old writer's I/O drains (device suspended, VMM killed): second exclusive open refused (`EBUSY`, old pid gone), successor neither running nor failed, attaches after resume with the data | `volume_survives.rs::a_replacement_waits_for_the_claim_while_the_old_writers_io_drains` | `volume-pool` | PENDING-ignored |
| S-VOL-T3-11 | a failed host flush reaches the guest's `fsync` as an error in the production write-back attachment (P-3a, V-2): `O_DIRECT` writes and a control `fsync` succeed on the linear table; the volume's dm device reports `write back`; the table is swapped to `dm-flakey ... 0 <down> 1 error_writes`; the guest's block-device `fsync` must return an error. Relabelled by the user's ruling of 2026-11-19 (P-3 option A): in the production write-back attachment a guest `fsync` is an explicit flush, and stock Cloud Hypervisor already propagates a failed explicit flush (spike run-022 and run-030, `a3_fault_wb_explicit_flush_*`); the test does not test fork fix F1 (ADR-0316), whose regression test lands in the fork with the F1 patch | `volume_survives.rs::a_failed_host_flush_reaches_the_guests_fsync_as_an_error` | `volume-pool` | PENDING-ignored |
| S-VOL-T3-12 | **(changed 2026-11-24)** also asserts the stuck VMM's scope is kept while its flush is held (`kill_scope` never removes a populated scope, ADR-0349/0351) and is removed by the reclamation pass after the drain; the added assertions are reached only after today's pending point. Boundary 4 and 4a: with the old writer's write held by a suspended device (its claim outlives its process) and the replacement waiting on the claim, `workload stop` returns promptly (well under the 30 s bound); no `Running` row and no VMM pid after the stop, the replacement never appears after the resume, no `Failed` row, nothing charged, the volume kept; the second `O_EXCL` open is `EBUSY` and the old pid is gone; a later `workload restart` attaches it with its data | `volume_survives.rs::a_workload_stop_during_a_held_claim_returns_promptly_with_no_failed_row` | `volume-pool` | GREEN at 09-03 (marker lifted) |
| S-VOL-T3-13 | boundary 4, a stop during the guest boot (restored, B-2, user ruling 2026-11-19): the VMM exists and no allocation is `Running` when `workload stop` arrives; the VMM process is terminated, no `Failed` row, nothing charged, the volume kept, a later `workload restart` attaches it. The Tier-3 twin of the volume driver's `alive[a]` arbiter read (S-SUP-05 and S-Q-10 cover the sim side). A stop that loses the race to the guest's readiness is an ordinary success that the stop then stops; the assertions hold either way | `volume_survives.rs::a_workload_stop_during_guest_boot_terminates_the_vmm_and_keeps_the_volume` | `volume-pool` | PENDING-ignored |
| S-VOL-T3-14 sad | P-4 (ADR-0314 Bound): a probe whose first `lvm` command stalls on the volume group's lock (`flock --no-fork /run/lock/lvm/V_overdrive sleep 45`, so a probe with no deadline of its own is seen to return late) returns within its one 30 s deadline, `VolumeProbeError` with `Io` of kind `TimedOut` and the scenario of the command in flight (`ToolsOrDeviceMapperUnusable` for `pvs`, `VolumeGroupMissing` for `vgs`, whichever first waits on the group lock); the stalled `lvm` is still running when `probe` returns and ends on its own after the lock is released; a following probe passes and leaves no scratch volume. The `health.startup.refused` reason for a deadline return is not asserted here: `volume_boot_probe.rs` injects a scripted `VolumeProbeError` and a `TimedOut` one reaches the same refusal path as any probe `Err` (VD-5), and that file is not part of this change | `volume_pool_probe_pool.rs::sad_a_probe_blocked_on_the_group_lock_times_out_as_an_io_failure` | `volume-pool` (`cgroup-default`) | PENDING (running) |
| S-VOL-T3-15 sad | P-5: `/sys/class/block` cannot be read (the process has no descriptor left, so opening the directory fails `EMFILE`; the probe is driven on a runtime built beforehand): the probe fails with scenario `SysfsUnreadable` and an `Io` detail, not `ToolsOrDeviceMapperUnusable`. The scenario is compared by its `Debug` name because the variant does not exist yet | `volume_pool_probe_pool.rs::sad_an_unreadable_sysfs_block_directory_fails_the_probe_sysfs_unreadable` | `volume-pool` (`cgroup-default`) | PENDING (running) |

S-VOL-T3-10 (volume node owner and mode, the `--disk` argv golden, forks F2 and F3) is asserted inside S-VOL-T3-07, T3-09 and T3-11; no separate test. A Tier-3 shutdown-during-claim-wait test is not
required (its kernel behaviour is T3-12's). Every `volume-pool` test is ignored until the guest-kit volume-pool step (prerequisite 1) provisions the pool; that step removes each `#[ignore]`.
T3-09 and T3-12 build the suspended device on a loop device, so the guest needs `losetup` and loop-device support (`upstream-issues.md` N-5).

## I. Required boundary scenarios (feature delta § Lifecycle Gate Ownership) mapped to tests

| # | Boundary | Tests |
|---|---|---|
| 1 | Gate available: `ensure` ok and claim free, the VMM launches with the second disk | S-VOL-40; S-VOL-30..32; S-Q-01; S-VOL-T3-07 |
| 2 | Gate unavailable: no pool, no space, command failure, deadline, claim held past 30 s: typed `Failed` row, budget charged, no VMM | S-VOL-41 (incl. the ensure deadline), 41b, 42; S-VOL-65 and the ensure-deadline boundary test; S-Q-06..08; S-VOL-BOOT-1..4 (boot gate); S-VOL-22..26 (the typed row) |
| 3 | Unrelated state: a running allocation is unaffected by the pool; stop and GC proceed | S-VOL-43; S-SUP-36; S-VOL-46 (no model run) |
| 4 | Late success: a stop or restart while a launch waits ends it at once (`StartSuperseded`): its `Pending` row ends as one abandoned row (ADR-0355), no charge, no VMM, volume kept, lease, SVID and claim released; a launch that won the boot race is an ordinary success; an `ensure` in flight finishes first | S-SUP-37; S-SUP-26..28; S-SUP-01..06 (the points); S-SHIM-01; S-Q-09..11; S-VOL-T3-12, S-VOL-T3-13; S-INV-01 (interruption cells) |
| 4a | (a) no lane open: S-SUP-10, S-SUP-29. (b) same-point failure: the "wins" tests S-SUP-02, 03, 05, 06 only. (c) teardown failure: S-SUP-07, S-SUP-31. (d) a driver that ignores the signal: S-SUP-32, S-SUP-08. (e) another workload's lane: S-SUP-13, S-SUP-08. (f) `submit_workload`, resync and observation wakes never raise it: S-SUP-24, 25. (g) a restart already hydrated: S-SUP-33. (h) node shutdown: S-SUP-14, S-SUP-18, S-SUP-34, S-SUP-36, S-Q-12 | as listed |
| 5 | Disconnect/reconnect: control-plane crash between `ensure` and spawn; a surviving VMM is reclaimed, its claim drains; a launch lost with the control plane ends its `Pending` row `LaunchLostWithControlPlane` at boot and the next evaluation relaunches | S-VOL-45; S-M7-1, S-M7-2; S-INV-01 crash cells; S-Q-04, S-Q-05, S-Q-14; S-RG-02 |
| 6 | Feature disabled: no `[vm.volume]` makes no `ensure`, no wait, no extra `--disk`; a node with no pool composes unchanged | S-VOL-46; S-VOL-11..20 (no volume echoes none); S-VOL-28; S-VOL-BOOT-5, 6 |
| 7 | A Job's failure before the driver is final (J-1): `Failed { exit_code: Some(0) }`, no further launch, where a Service's is charged and relaunched | S-INV-01 (J-1 and persistent/transient Service cells); S-SHIM-02 (the Job row), S-SHIM-08; S-Q-22, 23 (H1); S-Q-19, 20 |

## J. Assumptions mapped to tests

| Assumption | Discharged by |
|---|---|
| A1, A3 (a held `O_EXCL` claim refuses a second claimant; the claim outlives the VMM while I/O drains) | S-VOL-T3-04 (pairing), S-VOL-T3-09 |
| A2 (dm refuses to remove an open device) | not relied on in this slice (entry contract for #211) |
| A4, A5 (LVM metadata operations are crash-atomic; a crash after `ensure` leaves the volume) | spike evidence V-1 (evidence, not coverage); S-VOL-45 (control-plane restart between create and attach); S-Q-04 |
| A6 (the device persists what it acknowledges after a flush) | reasoned no-obligation: a media guarantee outside the appliance's control |
| A7 (a booting VMM terminated by the boot-deadline teardown; its claim lingers until I/O drains) | S-SUP-05, S-Q-10 |
| A8 (a signal after the intent commit reaches the lane, or the hydration sees the commit) | S-SUP-29 (before the lane opens), S-SUP-12 (after it opens, before hydration), S-SUP-26 (during dispatch) |
| A9 (a lane opened at any instant around `supersede_all_lanes` observes the signal) | S-SUP-18 (the seeded schedule: born raised, between opens and after) |
| A10 (after an abandoned launch the reconciler places a fresh allocation, once) | S-INV-01; S-RG-01, S-RG-02 (the spec does not discharge it: RA2 is its premise) |
| M5 (a superseded launch abandons its waits and delivers nothing; `ensure` and the VMM create are never interrupted) | boundaries 4 and 4a (section I), S-SUP-02 |
| M7 (no new launch while a surviving VMM is alive: the boot sweep ends it first) | S-M7-1, S-M7-2 (the conformance driver's `Reclaim` arm is the paired projection, S-VOL-T3-04 the arbiter's Tier-3 twin) |
| RA1 (the outcome set is exhaustive) | S-SHIM-01..08, S-INV-01 Job dimension |
| RA2 (a launch in flight is invisible to a later evaluation) | S-INV-01 assertion (b) (one launch in flight, counted at the driver's `start` calls); S-SUP-01..33 for the lane |
| RA4 (a lost or superseded launch delivers nothing) | S-SHIM-01, S-Q-09 |
| RA5 (no VMM survives to block a placement) | S-M7-1, S-M7-2 |
| RA6 (a volume-caused launch failure is one typed charged start failure) | S-VOL-41, 42, S-VOL-65, S-Q-06..08 |
| RA7 (`LaunchPreFail` and `LaunchRefused` have the production order and effects) | S-SHIM-02, 03, 06 |
| RA8 (after `CpCrash` or `NodeReboot` the View, intent and rows survive and the runtime is rebuilt) | S-INV-01 crash cells (the runtime and stores are rebuilt, boot reclamation run) |
| RA9 (the budget abstraction) | S-Q-17, S-Q-18 (`cnt` through `attempt_index_for_alloc`); S-RG-02 |
| RA10 (a Job row with a terminal claim is never replaced) | S-RG-01 (`pending_agrees_with_the_shims_finished_job_fence`, the Job examples); S-INV-01 Job dimension; S-Q-22, 23 |
| RA11 (a Job's failure before the driver is final) | S-INV-01 J-1 cells; S-SHIM-08 |

## K. Counts

Re-counted from the Rust sources by parsing every `#[test]`, `#[tokio::test]`, `#[quint_test]` and `#[quint_run]` function with its attributes (a `proptest!` function is one test;
the two shared files, `vm_reclamation_boot.rs` and the integration `workload_describe.rs`, are counted over the feature's own modules only: `mid_boot_volume_reclamation` and
`capacity_wait_empty_state`). The parser's per-file totals agree with a plain grep of the same attributes. "Pending" is `#[should_panic(expected = ...)]` and not `#[ignore]`;
"pending-ignored" is both; "guard" is neither. No test is `#[ignore]` without a `should_panic` marker. The earlier cut also carried an indicative happy versus error / fault / edge split,
my judgment per file and never audited; it is not carried forward, because the removed and merged tests would have needed re-judging by the same unaudited rule. Previous cut in
brackets: [before] -> after.

| Tier | Files | Tests | Pending (running) | Pending-ignored | Guards |
|---|---|---|---|---|---|
| Tier 1 layer 1 (pure functions, proptest, render goldens, the store round trip) | [12] -> 11 | [107] -> 104 | [84] -> 89 | [0] -> 0 | [23] -> 15 |
| Tier 1 layer 2 (composed owners over `Sim*`, shim, lane table, seeded invariants) | [12] -> 13 | [132] -> 119 | [105] -> 87 | [0] -> 0 | [27] -> 32 |
| Conformance (two drivers) | [2] -> 2 | [24] -> 24 | [20] -> 21 | [0] -> 0 | [4] -> 3 |
| Tier 3 placement (guest) | [6] -> 6 | [37] -> 34 | [7] -> 6 | [24, plus 2 unmarked ignored] -> 25 | [3, plus 1 ignored guard] -> 3 |
| **Total** | [32] -> 32 | **[300] -> 281** | **[216] -> 203** | **[24 (+2 unmarked)] -> 25** | **[58 (57 running, 1 ignored)] -> 53 (all running)** |

The layer 2 file count rises by one: the composition fill moved from `tests/acceptance/` to `tests/integration/` (the acceptance `capacity_wait_composition.rs` is now the shared rig with no test
of its own) and the handler fill gained an integration file beside its acceptance guards; the seeded restart-generation invariant also moved from `tests/` to `tests/integration/` (no count change). The layer 1 count falls by one with the deleted
`vm_volume_disk_serial.rs`. Changes by tier, from the removals, merges and additions recorded in § L and the retired ids above: layer 1 loses five guards in `overdrive-core` (the serial
guard, the `VolumeUnavailable` serde guard and three `volume_value_types.rs` guards), two pending tests in `volume_value_types.rs` (the writable-direct proptest and the case-insensitive
unit, now inside the round-trip proptest) and one merged pending and four guards in `workload_describe_capacity_wait.rs`, and gains eight pending tests and one guard in
`start_blocked_only_by_capacity.rs`; layer 2 loses eight pending lane tests, one pending shutdown test, one sim contract test, handler and composition tests (four
pending tests become two, and the two full-node guards merge into the fill test), seven pending and one guard seeded-invariant tests, and gains eight guards in the shim file; conformance gains the vacuous-guard-turned-pending
`quint_rg_resume_of_stopped_job_does_nothing_and_issues_no_id`; Tier 3 loses three BOOT-table tests (merged into one), the two dropped journeys (T3-08, T3-13) and the dropped `lvm` guard.

By section of this file (the tier table above is the authority for the tier totals):

| Section | Tests (before -> after) | Pending (running) | Pending-ignored | Guards |
|---|---|---|---|---|
| A value types, spec and wire | [44] -> 37 | [34] -> 32 | 0 | [10] -> 5 |
| B sim contract | [14] -> 13 | [14] -> 13 | 0 | 0 |
| C launch path, boundary 2, seeded volume invariants, boot gate, links | [29] -> 27 | [25] -> 23 | 0 | [4] -> 4 |
| D supersession (observation points, lane table, handlers, shutdown, boundary 4) | [46] -> 37 | [36] -> 27 | 0 | [10] -> 10 |
| E restart generation, shim launch outcomes, seeded invariant, M7 | [72] -> 72 | [54] -> 47 | 0 | [18] -> 25 |
| F capacity wait | [43] -> 43 | [33] -> 38 | 0 | [10] -> 5 |
| G conformance | [24] -> 24 | [20] -> 21 | 0 | [4] -> 3 |
| H Tier 3 pool and journeys | [28] -> 28 | 2 | [24 (+2 unmarked)] -> 25 | [2 (1 running, 1 ignored)] -> 1 |
| **Total** | **[300] -> 281** | **[216] -> 203** | **[24 (+2)] -> 25** | **[58] -> 53** |

Wall-clock. Of the six tests that fill the real pool to 16,384 attachments (about 56 s each on a debug build) four are gone, so about four times 56 s less per run; the two
remaining fills are in the integration lane and fit the default 120 s budget, so no `.config/nextest.toml` override is needed. The seeded rigs now wait on observable quiescence instead of fixed
yield counts. The whole nine-crate nextest run of this revision took 3278.7 s against 3441 s for the previous cut, but the two runs did not use the same package list
(`overdrive-bench-families` was not in this run), so the figures are not comparable beyond showing that the run did not grow; no per-test timing was measured for the seeded restart-generation
invariant, whose whole binary runs in seconds.

The seeded invariants and their seeds: S-VOL-48..50, seeds 1..=8; S-INV-01, seeds 348001..=348006; S-SUP-18, seeds 0..4 (printed). Not authored: S-VOL-21 (U-6), S-VOL-47.

## L. Judgment calls and open points

Each is verified against the code. Calls s, t and u were blockers B-2, B-3 and B-1 and are resolved below.

- **(a) A compat rule stands where a panicking helper would break the reconciler.** `generation_replacement_pending` panics (RED scaffold), so the reconciler cannot call it. The reconciler
  keeps its pre-ADR-0335 decision through the private `restart_pending_until_successor_observed` (pending iff the largest `issued_generation` entry is below `desired.generation`;
  `workload_lifecycle.rs`), restated inline in `ReconcilerRuntime::restart_status_for_alloc`; the inline copy is marked "compat copy ... delete with the helper in DELIVER step 9". DELIVER adopts the
  helper at the four sites and deletes both rules. The field rename `observed_generation` -> `issued_generation` is mechanical in the existing tests and the trajectory golden; the only
  remaining uses of the old name are prose in `generation_read_coupling.rs` and a test-helper parameter with its doc comment in `reconciler_runtime.rs`.
- **(b) The seeded restart-generation invariant lives in `crates/overdrive-control-plane/tests/integration/`**, not `overdrive-sim`: it is driven through the real `stop_workload` / `restart_workload` /
  `submit_workload` handlers (axum extractors), and `overdrive-sim` has no axum dependency. It opens a real redb, so it is in the integration entrypoint, with a module-level `placement!(host_pure)` because the binary also
  holds guest-placed tests. Seeds come from `SimEntropy`.
- **(c) The restart-generation driver holds the dispatch at a one-shot `ObservationStore::alloc_status_row` read** (a delegating `HoldObs` double of the driven port), not at the `SimDriver` park,
  because the shim's order is admission, then identity issuance, then the driver: holding at the driver would have passed admission and identity before `CapFill` or `IdFaultOn` could be armed, so
  `LaunchRefused` and `LaunchPreFail` could never be produced. Only `LaunchLost` parks `SimDriver::start`. The driver keeps `ends` and `alloc_of` records, and the dispatch result is a pinned projection (`W_REFUSED`, `W_LOST` read from the held tick's own end). A critic may revisit this line.
- **(d) Assertion (e) of the seeded invariant is exact.** Production's carry rule (`workload_lifecycle.rs` ~1311-1333) lets a generation replacement inherit its predecessor's `last_failure_seen_at`, so its first crash restart is immediate and later ones are gated. `check_persistent` therefore asserts, with `SimClock` held, at most two launches (the replacement plus one ungated crash restart), and `a_persistent_identity_failure_launches_a_bounded_number_of_times_within_one_backoff_window` then advances three single backoff windows and asserts at most one launch in each. The work order's "never exceed one" sentence is corrected in `upstream-issues.md`; the reconciler is unchanged. The three window assertions are compile-checked only until the pre-launch outcome lands (the test stops earlier today, at its "recorded as a row" assertion).
- **(e) `VmmSpawnFail` is a driven-port fault.** The `Vmm` port's `create` refuses: the driver arms `SimVmm::inject_create_failure` (the existing one-shot create refusal, no new `SimVmm` surface) and releases the launch's parked `create`. The
  model's `LP_FREE` (claim observed `Free`, VMM not yet created) was not a stopping point, because `LaunchCheck` ran the launch through `Vmm::create`; the volume driver's recording `Vmm` now parks each `create` at a test-support
  `CreateGate` (off unless the driver enables it at `Init`): `LaunchCheck` waits until the create is parked, `VmmSpawn` releases it, `VmmSpawnFail` arms the refusal and releases it
  (production classifies it, the shim writes the charged `Failed` row, no VMM exists), `LaunchSuperseded` releases a parked create first (it is past the launch's last signal observation) and a crash clears the gate. No production
  surface is needed. Like every handler after `Declare`, it is compile-checked only until the spec surface lands. Id discovery is for labelling only: the driver reads fresh allocation ids from the reconciler View's issued ids, the rows
  and `live_allocations`; the state comparison does not read them as facts.
- **(f) M7 goes through `run_server_with_obs_and_driver`.** The earlier cut composed two `AppState`s and called `converge`; this one boots the production composition twice over the same roots and deploys through the public API, so boot
  reclamation is the boot's own step and the first `StartAllocation` is the convergence loop's. The orphan's claim drain is test support:
  `SimVolumeBacking::hold_claim_while_live(workload, live)` (a scaffold no-op until DELIVER, with its pending contract test `a_claim_linked_to_a_vmm_is_released_when_the_vmm_dies`) makes the claim `Held` while a closure is true; the
  recording `Vmm` passes `move || vmm.is_live(pid)` for each volume `create`, so killing the VMM releases its claim. The tests need `ComposedNode::boot_adapters` and `config` made `pub(crate)` (test support).
- **(g) "Stop before enqueue" is not observable.** The handlers' ordering (commit, then `supersede_lane`, then `enqueue_workload_lifecycle_eval`) is observable only as far as the commit being visible when the
  signal is seen; the tests assert that. The enqueue ordering would need new surface.
- **(h) The lease-holding restart-arm refusal of work-order item 6 is covered by S-SHIM-06.** S-SHIM-07's restart-arm case is a refusal whose predecessor held no lease; the case where the predecessor held a lease
  is the N-ADM guard `restart_arm_admission_refusal_releases_the_predecessor_and_the_next_tick_emits_again`.
- **(i) The empty-state suppression has a describe-driven test** in the CLI integration lane (S-CAP-05), since the message is computed inside `commands::workload::describe`, which reads
  an HTTPS control plane; the acceptance lane's pure render cannot host it.
- **(j) No `.config/nextest.toml` override.** The capacity handler and composition tests fill the real pool to its cap of 16,384 attachments through its own admission, about 56 s each on a debug build, within the 120 s default budget; they
  are two tests under `tests/integration/`. The earlier override of 60 s with `terminate-after = 4` was removed (the file has no feature-related change).
- **(k) `vm_lifecycle_latency_283_spike::admission_close_drains_owned_starts_without_admitting_ninth` fails identically on the unmodified HEAD (`07a0d8a5b`)**: a pre-existing failure, unrelated to this feature.
- **(l) The catalogue count.** `cargo run -p overdrive-sim --bin dst -- --seed 7` prints 43 invariants; `upstream-issues.md` (N-18) says "44 today", and the earlier pass's run printed 44. Whether the
  count changed between that run and HEAD is not established here; the number DELIVER updates at activation is the printed count.
- **(m) The three pending volume DST invariants stay outside `Invariant::ALL`** (open decision D-2 stays the user's); they run through `Harness::only(..)`.
- **(n) Every pending test is now marked.** The two Tier-3 tests that were `#[ignore]`d without a marker are resolved: T3-12 carries its `#[should_panic]`, and T3-13 was dropped by the user's subtractive ruling and restored (B-2), and T3-08 and T3-06 were dropped in the same pass. No `#[ignore]` without a marker remains.
- **(o) Shape differences from the work order.** The capacity-wait handler and composed tests drive the real handlers over `AppState` rather than an HTTP router (the pool's lease operations
  are crate-private, so the node is filled through production `StartAllocation` dispatches); the resync pass is realised as the per-target `workload-lifecycle` evaluation it submits (the pass is crate-private); the render
  goldens are in a new file `workload_describe_capacity_wait.rs`; and the shim charge test covers R7 as the chain after S5 (no separate R3 charge assertion).
- **(p) EDD claim: unchanged.** `verification/expectations/E16-node-local-volume-survives-restart-and-reboot/README.md` (surface E, run with `run-expectation.sh E16`) claims what no test can show: a real node reboot leaves the volume inactive,
  the next launch activates it with no operator step, the data reads back, and `describe` shows the volume line and a typed `InsufficientSpace` reason. Its anchors (VD-2, VD-8, VD-9) are the volume ownership, the claim wait and the launch-path
  ensure; the supersession, restart-generation and capacity-wait decisions change none of them. No evidence or status was added or changed. The directory was renamed from `node-local-volume-survives-restart-and-reboot` to `E16-...` and the INDEX row updated.
- **(q) S-VOL-T3-11 is relabelled (P-3a, user ruling 2026-11-19, option A).** In the production write-back attachment a guest `fsync` is an explicit flush, which stock Cloud Hypervisor already propagates when it fails; the test checks that behaviour and does not test F1. F1 is upstream's own behaviour since 2026-11-20 (ADR-0316 amended), so no fork-side F1 regression test exists.
- **(r) Removals and merges that are judgment calls.** The per-position `supersede_all_lanes` tests and the `supersede_lane` / stop / restart raise tests that duplicated S-SUP-18 and S-SUP-26..27 were removed; S-SUP-30 asserted the sim's own `select` and was removed (its 4a (b) mapping now rests on the "wins" tests alone); S-SUP-37 and S-VOL-38 were kept after review (their unique oracles are stated at their rows).
- **(s) RESOLVED (B-2, user ruling 2026-11-19): the `alive[a]` projection.** The recording `Vmm` double stays the named arbiter of the VMM process. S-VOL-T3-13 is restored as its Tier-3 twin (`volume_survives.rs::a_workload_stop_during_guest_boot_terminates_the_vmm_and_keeps_the_volume`, `cgroup-default` / `volume-pool`, `kvm-tests`-gated, `#[should_panic]` naming its own first assertion, `#[ignore]` naming the guest pool), and the volume driver's `booting` projection reads `vmm_live` with a `// conformance-lint: sim-arbiter-ok` marker citing it.
- **(t) RESOLVED (B-3): the composition test asserts the response, not the rendered line.** `overdrive-control-plane` has no `overdrive-cli` dev-dependency and none is added. The rendered `Waiting` line is pinned by the CLI render tests (`overdrive-cli/tests/{acceptance,integration}/workload_describe*.rs`); the composition asserts the response's `capacity_wait { resource, held, cap }`.
- **(u) RESOLVED (B-1, pinned 2026-11-19): the adapter enforces the 30 s `ensure` deadline.** The driver awaits `ensure` to its return with no timer. `SimVolumeBacking` holds no timer; a test parks an `ensure` (`pause_ensure_after_create`, `ensure_parked`, `resume_ensure`). `the_driver_holds_no_ensure_timer_a_parked_ensure_outlives_the_deadline` advances `SimClock` past 30 s with `ensure` parked and asserts the launch has not ended, then resumes it and asserts the launch proceeds. `a_signal_during_ensure_waits_for_ensure_then_abandons_whatever_ensure_returns` covers a signal raised while `ensure` is parked. The Tier-3 test `sad_an_ensure_blocked_on_the_group_lock_times_out_as_a_command_failure` asserts the adapter returns the typed `TimedOut` within the bound and that the stalled `lvm` is not killed.

## M. Verified on metal

Final tree of the B-1..B-3 pass (2026-11-19). Ordinary grammar only, no `--exclusive`.

| Command | Run id | Result |
|---|---|---|
| `cargo check --workspace --all-targets --features integration-tests` | `a462c5059f90fbc6f5161877` | exit 0 |
| `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | `6ce388d050a51c9c4bec3541` | exit 0 |
| clippy of `overdrive-cli`, `-host`, `-control-plane`, `-worker` with `integration-tests,kvm-tests` | `95eb9cbbac2a587b35ccaaa3` | exit 0 |
| `cargo nextest run` over `overdrive-sim`, `-worker`, `-control-plane`, `-cli`, `-host` with `--features integration-tests --no-fail-fast` | `ada059eabdda5283f6cb35a7` | 2375 run, 2373 passed, 2 failed: the pre-existing `overdrive-sim::vm_lifecycle_latency_283_spike admission_close_drains_owned_starts_without_admitting_ninth` and the new `sad_an_ensure_that_ends_at_its_deadline_fails_as_a_command_failure`, whose `should_panic` expectation named the wrong first assertion; fixed |
| `cargo nextest run -p overdrive-worker -E 'test(/ensure/)'` after the fix | `51025b9b52ca3ebb9fe9ab59` | 5 passed |
| `cargo xtask placement-lint` | `8792ecc472b718ad382018ef` | exit 0 |
| `cargo xtask conformance-lint` | `8dbeee8c60dca0e812738790` | exit 0 |

The only failing test of the package list is main's pre-existing `admission_close_drains_owned_starts_without_admitting_ninth` (call k).

The held-clock windows in `a_persistent_identity_failure_launches_a_bounded_number_of_times_within_one_backoff_window` are compile-checked only (call d).

## N. The stuck-VM design (RB-1 to RB-13, N-1 to N-4, P-6, P-7; ADR-0348 to ADR-0355)

Added 2026-11-24. Same conventions as above: a complete executable body under `#[should_panic(expected = ...)]` naming the assertion that
fails today, a guard where the behaviour holds before and after. A scenario here records the contract, the owner path, the fault
stimulus, the oracle and the tier. **No production code was changed** (the instruction of this pass), so a test that needs an owner
API the design pins but the tree lacks is not authored: § N3 lists it with the API and the design item that pins it. The previous
cut's test counts (§ K) are not recounted; § N4 gives this pass's delta.

### N1. Authored tests (all verified on metal, § N5)

| Id | Contract | Owner path | Fault stimulus | Oracle | Tier | Test and file | Status |
|---|---|---|---|---|---|---|---|
| S-RB-01 | RB-6 / ADR-0353: when `Driver::start` is reached, so before `Vmm::create`, the allocation's `Pending` row is on record with the lease's address (`started_at`, `reason`, `terminal` all `None`) | production `StartAllocation` dispatch (`dispatch_with_guest_network_provisioner_for_test`) over the production `GuestAddressPool`, `IdentityMgr`, `SimObservationStore` | none (the driver double reads the store at `start`) | the row the store holds at the instant of `start`, and the spec's transport address | 1 (layer 2) | `action_shim/prelaunch_outcome_acceptance.rs::start_arm_records_the_pending_row_with_the_leases_address_before_the_driver_starts` | PENDING |
| S-RB-02 | the same for the restart arm's successor | production `RestartAllocation` dispatch | none | as above, for the successor id | 1 | `restart_arm_records_the_successors_pending_row_with_the_leases_address_before_the_driver_starts` | PENDING |
| S-RB-03 | RB-6 / N-REC: a failed `Pending` write creates no VMM: the driver is never reached, the lease is retired and torn down, the typed observation error returns | production start arm | the observation store refuses the next `Pending` lifecycle write (`FaultObs`, a delegating `ObservationStore`, a driven port) | `driver.starts`, the pool's lease, the owner's teardown journal, the result | 1 | `start_arm_failing_pending_write_never_reaches_the_driver_and_releases_the_lease` | PENDING |
| S-RB-04 | RB-6 terminal-row carry: the `Failed` row of a start the driver rejects carries the `Pending` row's address | production start arm | `SimDriver::script_next_start_rejection` | the `Failed` row's `workload_addr` | 1 | `start_arm_failed_start_row_carries_the_pending_rows_address` | PENDING |
| S-RB-05 | RB-6 terminal-row carry: the stop's terminal row carries the `Running` row's address | production `StopAllocation` dispatch after a production start | none | the `Terminated` row's `workload_addr` | 1 | `a_stops_terminal_row_carries_the_prior_rows_workload_addr` | PENDING |
| S-RB-06 | RB-6 terminal-row carry: `FinalizeFailed` forward-carries the prior row's address for every non-`Stable` terminal (was: drops it) | production `FinalizeFailed` dispatch | seeded `Running` row with an address, any IPv4, any genuine terminal (proptest, layer 2 PBT per Mandate 9) | the successor row's `workload_addr` and `state` | 1 | `crates/overdrive-control-plane/tests/acceptance/finalize_failed_forward_carries_workload_addr.rs::finalize_failed_genuine_terminal_forward_carries_workload_addr` (was `..._drops_workload_addr`) | PENDING |
| S-RB-07 | RB-8 / ADR-0355 W1, W2: a launch the driver returns `StartSuperseded` for ends its `Pending` row as one `Terminated` / `LaunchSuperseded` row (terminal `None`, never started, address carried, count facts of the `Pending` row), with the lease, SVID and supervision claim released and `launch.superseded` emitted; in the restart arm the exact-old predecessor cleanup still runs | production start and restart arms | `StartSuperseded` from the driver double | the allocation's rows, lease, held SVID, supervision releases, the event | 1 | `start_arm_superseded_launch_ends_its_row_as_abandoned_and_releases_lease_svid_and_claim`, `restart_arm_superseded_successor_ends_its_row_as_abandoned_and_runs_exact_old_predecessor_cleanup` (were no-row guards) | PENDING |
| S-RB-08 | ADR-0355 section 4: a failing teardown does not suppress the abandoned row; a failing row write is the typed error and leaves the `Pending` row for the next boot | production start arm | `TestSharedOwner::script_teardown_failure`; `FaultObs` refusing the `Terminated` write | the row, the result variant | 1 | `start_arm_teardown_failure_does_not_suppress_the_abandoned_row`, `start_arm_failing_abandoned_row_write_is_returned_as_the_typed_observation_error` | PENDING |
| S-RB-09 | RB-8 W3: a non-start `DriverError` from `start` ends the `Pending` row as a `Failed` `DriverInternalError` row (address carried, lease released) | production start arm | the driver double returns `GuestTransportMissing` | the row | 1 | `start_arm_a_non_start_driver_error_ends_in_a_failed_driver_internal_error_row` | PENDING |
| S-RB-10 | RB-2 / ADR-0156 amended: a stop whose VM still has its scope completes (`Terminated`) but keeps the lease Retiring, counted toward the cap; once the scope is absent a `ReclaimAllocationNetwork` attempt releases it; with no scope the lease is released at the stop (guard) | production stop and reclaim dispatch | a scope present in `SimVmHostState` for the allocation (the unreaped VMM's leftover) | the row, the pool's `observe` (lease state, occupancy) | 1 | `a_stop_keeps_the_lease_held_while_the_vms_scope_is_present_and_a_reclaim_releases_it_after` (PENDING), `guard_a_stop_releases_the_lease_at_once_when_no_scope_remains` | PENDING; GUARD |
| S-RB-11 | RB-6 duplicate guard: a duplicate start or restart dispatch never writes `Pending` over a `Running` row. ADR-0353 states this for the start arm; the restart arm's successor write (`dominating(.., None)`) is the same risk and is pinned here (§ N6, question 1) | production start and restart arms | a second dispatch of the same action | the row's state | 1 | `guard_a_duplicate_start_dispatch_never_regresses_a_running_row_to_pending`, `guard_a_duplicate_restart_dispatch_never_regresses_the_successors_running_row` | GUARD |
| S-RB-12 | RB-11 / ADR-0355 W4: boot's first step ends every `Pending` row of this node `Terminated` / `LaunchLostWithControlPlane` (address carried, terminal `None`, never `Stopped { PlatformReclaimed }`) whether the launch left no artifact or a settled scope, run dir and clone (the executors then discard the artifacts and leave the reason); a second boot writes nothing; another node's `Pending` row is left alone | production `vm_reclamation_boot::converge` | the host double's scope, run dir, clone | the row, the host's artifacts | 1 | `vm_reclamation_boot.rs::launches_lost_with_the_control_plane::{a_pending_row_with_no_host_artifact_ends_as_lost_with_the_control_plane, a_pending_row_whose_vm_left_a_settled_scope_ends_as_lost_with_the_control_plane, a_second_boot_over_the_closed_rows_writes_nothing}` (PENDING), `guard_another_nodes_pending_row_is_left_alone` | PENDING; GUARD |
| S-RB-13 | RB-4, RB-7 / ADR-0350, ADR-0354: a busy scope never refuses boot. A `Running` row is ended platform-reclaimed at once carrying its address, the scope stays for `VmReclamation`, and the row's offset is reserved as a Retiring lease counted in the occupancy; a busy `Pending` row ends as lost with the control plane and reserves from the closed row's carried address; a non-busy substrate refusal still refuses boot | production `converge` | `SimVmHostState::script_kill_scope_failure(ResourceBusy)` (the existing one-call busy) | `converge`'s result, the row, `has_scope`, the pool's `observe` | 1 | `a_busy_scope_boots_ends_the_running_row_and_reserves_its_address`, `a_busy_scope_of_a_pending_row_ends_as_lost_and_reserves_the_carried_address` (PENDING), `guard_a_non_busy_executor_error_still_refuses_the_boot` | PENDING; GUARD |
| S-RB-14 | RB-6 carry: the reclamation executor's platform-reclaimed row carries the prior row's address | production `converge` | a settled scope under a `Running` row | the row | 1 | `a_platform_reclaimed_row_carries_the_prior_rows_address` | PENDING |
| S-RB-15 | RB-11 composed, two boots of the production composition: the crash leaves the launch's `Pending` row (with its address) only; the second boot's own boot step ends it `LaunchLostWithControlPlane`, or, over a `Running` row, platform-reclaimed; no `Failed` row, nothing charged; one successor launch (**changed**: S-M7-1 and S-M7-2 asserted "no row") | `run_server_with_obs_and_driver` twice over one data directory | `ServerHandle::kill_for_test` | the rows, the VMM port's creates, the claim | 1 | `vm_reclamation_boot.rs::mid_boot_volume_reclamation::{boot_reclamation_ends_a_mid_boot_vmm_before_any_launch_for_its_workload, ..._when_the_crash_left_a_running_row}` | PENDING (stops at the `Pending`-row precondition) |
| S-RB-16 | RB-8 across the lane table and the shutdown arm: every superseded launch (a restart, a stop, a second restart, `supersede_all_lanes` at any point of a seeded schedule, node shutdown, a stop on a held claim) ends one abandoned row and never a `Running` or `Failed` row (**changed**: were "no row") | real handlers, lane table, tick, shim | a held `SimDriver` start; `supersede_lane` / `supersede_all_lanes`; the shutdown token | the rows | 1 | `tests/acceptance/launch_lane_supersession.rs` (five tests), `src/node_volume_boundaries.rs::{a_stop_interrupts_the_start_and_its_result_is_never_used, loop_shutdown::node_shutdown_ends_a_waiting_launch_with_an_abandoned_row_and_the_loop_exits}` | PENDING |
| S-RB-17 | RB-5 / ADR-0351: a VM scope has one remover. No production caller outside `RealVmHostState::kill_scope` and the `CgroupFs` adapters' own probes removes a cgroup directory (an AST scan with `syn` over every crate's production `src/`, the allow-list named); non-vacuity (the scan sees `kill_scope`'s own removal) and the scanner's own cases are guards | the source tree | none (pure) | the set of removers | 1 (layer 1) | `crates/overdrive-core/tests/acceptance/vm_scope_has_one_remover.rs::{a_vm_scope_is_removed_only_by_the_populated_first_scope_kill (PENDING), guard_the_scan_sees_the_one_allowed_remover, guard_the_scan_flags_a_scope_removal_and_ignores_a_run_dir_removal_and_test_code}` | PENDING; GUARD |
| S-RB-18 | RB-3 / ADR-0349: `kill_scope` never removes a populated scope. With the scope's last task held in exit (a buffered write parked on a suspended device, task killed), it returns `Err(ResourceBusy)` after the 3 s bound, the directory is present, the task alive and `populated 1`; after the drain it returns `Ok` and the scope is absent. Control: a scope that empties during the wait returns `Ok` in under 2 s | production `RealVmHostState::kill_scope` over real cgroup v2 | a `dm-linear` device over a loop file suspended before the write, the writer killed (the spike's fixture, no production seam) | the return kind and duration, the directory, `/proc/<pid>`, `cgroup.events` | 3 (`cgroup-default` / `volume-pool` guest) | `crates/overdrive-host/tests/integration/vm_host_state_scope_kill.rs::{kill_scope_never_removes_a_scope_whose_last_task_is_held_in_exit (PENDING), guard_a_scope_that_empties_during_the_wait_returns_ok_before_the_bound}` | PENDING; GUARD |
| S-RB-19 | RB-9 / ADR-0352: when `create` returns, the VMM process is a member of `config.cgroup_scope` and the scope's `cgroup.procs` lists it | production `CloudHypervisorVmm::create`, real Cloud Hypervisor, default cgroup root | none | `/proc/<pid>/cgroup`, the scope's `cgroup.procs` | 3 (`nested-launch`, `kvm-tests`) | `crates/overdrive-host/tests/integration/vmm_launch_in_scope.rs::a_created_vmm_is_a_member_of_its_scope_when_create_returns` | PENDING |
| S-RB-20 | RB-9 / P-7 / ADR-0352 item 8: a refused placement (the scope removed before the call; a scope with `subtree_control` set) fails `create` with a `detail` containing `joining the cgroup failed`, no hypervisor process, and the rootfs clone removed | production `create` | the scope directory (absent; `+pids` in its `subtree_control`) | the error text, the process table, the clone path | 3 | `vmm_launch_in_scope.rs::{create_against_a_removed_scope_fails_naming_the_step_and_leaves_nothing, create_against_a_scope_with_subtree_control_set_fails_naming_the_step_and_leaves_nothing}` | PENDING |
| S-VOL-T3-12 | **changed**: adds that the stuck VMM's scope is kept while its flush is held and removed by the reclamation pass after the drain (see the row in § H) | `serve` + `deploy` | a suspended device with a flush held, the VMM SIGKILLed | the scope directory | 3 | `volume_survives.rs::a_workload_stop_during_a_held_claim_returns_promptly_with_no_failed_row` | GREEN at 09-03 (bounded reap; marker lifted) |

### N2. Existing tests changed by this pass

| Test | Change | Why |
|---|---|---|
| `start_arm_superseded_launch_writes_no_row_and_releases_lease_svid_and_claim` | renamed `..._ends_its_row_as_abandoned_...`, now pending | ADR-0355 W1: the shim writes the abandoned row |
| `restart_arm_superseded_successor_runs_exact_old_predecessor_cleanup` | renamed `..._ends_its_row_as_abandoned_and_runs_...`, now pending | ADR-0355 W2 |
| `launch_lane_supersession.rs`: `supersede_lane_for_one_workload_never_raises_another_workloads_lane`, `supersede_all_lanes_at_any_point_of_a_seeded_schedule_abandons_every_launch`, `a_restart_while_a_launch_waits_ends_it_at_once_and_charges_nothing`, `a_stop_while_a_launch_waits_ends_it_and_the_workload_stays_down_until_the_next_restart`, `a_second_restart_while_the_replacement_launch_waits_runs_the_exact_old_cleanup` | the "no row" assertions become "one abandoned row" through a shared helper; each now pending | ADR-0355 |
| `node_volume_boundaries.rs::a_stop_interrupts_the_start_and_its_result_is_never_used`; `loop_shutdown::node_shutdown_ends_a_waiting_launch_with_no_row_and_the_loop_exits` | the first asserts one abandoned row; the second is renamed `..._with_an_abandoned_row_...`; both pending | ADR-0355 |
| `mid_boot_volume_reclamation` (two cells) | precondition: the parked launch's `Pending` row with its address, not "no row"; outcome: the orphan's row is `LaunchLostWithControlPlane` (Pending residue) or platform-reclaimed (Running residue), address carried; `running_row` takes the address and counter 2; both pending | ADR-0353, RB-11 |
| `finalize_failed_genuine_terminal_drops_workload_addr` | renamed `..._forward_carries_workload_addr`, asserts the address is carried; pending | RB-6 |
| `volume_survives.rs` S-VOL-T3-12 | scope assertions added (§ N1) | ADR-0349, ADR-0351 |

### N3. Not authored: the API each test waits for

Nothing below is invented. Each row names the owner API the design pins and which design item pins it; the test is written against that API by DELIVER's
first step that lands it (or by a DISTILL pass after it), per `.claude/rules/testing.md` "RED acceptance scaffolds and activation".

| Required test (design item) | Waits for | Pinned by |
|---|---|---|
| ADR-0348 Tier 1 (a) to (e): `VmDriver::stop` over an unreapable `SimVmm` returns after exactly `VMM_REAP_BOUND` on the injected clock, `Terminated`/`Stopped`, claim and CID held until the scripted reap, late reap changes no row, replacement waits and is not charged, no `kill_scope` call after `KilledNotReaped` | `VmTermination::KilledNotReaped`, `VMM_REAP_BOUND`, `SimVmm::new(Arc<dyn Clock>)` and an unreapable-process scripting method (test support, DISTILL's per the delta), `VmDriver::new(.., Arc<dyn VmHostState>)` | feature delta "A VMM kill's reap wait is bounded" items 1 to 6; ADR-0348, ADR-0351 |
| `Vmm` equivalence for `terminate` outcomes; stop-totality `KilledNotReaped` case; `VmReclamation` pass over an unremovable scope | the same, and `SimVmHostState` hold (below) | ADR-0348 Tests |
| ADR-0349: `VmHostState` equivalence (`Ok` implies absent, `ResourceBusy` leaves the scope, idempotence, non-busy error returns at once); `VmReclamation` tick over a held scope (no discard, no row, other actions still run; the next tick removes it after the hold clears); the boot double's busy case | `KILL_SCOPE_SETTLE_BOUND` and a `SimVmHostState` hold (scope held until a reap; `kill_scope` returns `ResourceBusy` at once without consuming the scripted-failure queue) | feature delta "A scope kill's settle wait is bounded" item 5 |
| ADR-0349 host-adapter units over a fake `cgroup.events` (removal issued only after the `0` read; `PermissionDenied` returned on the first attempt; a missing `cgroup.events` is `Ok`) | an injectable cgroup filesystem in `RealVmHostState`. The adapter reads the real path, so a unit over a fake file cannot show the ordering; the Tier-3 test S-RB-18 shows it on the kernel. The `PermissionDenied` case cannot be arranged as root in a guest. **Surfaced, not invented** | § N6 question 2 |
| ADR-0351 Tier 1: a reaped stop and a start-failure cleanup remove the scope through `kill_scope` and no other path; a busy scope leaves both outcomes unchanged | `VmDriver::new(.., Arc<dyn VmHostState>)`, `SimVmHostState` call recording | ADR-0351 item 1 |
| ADR-0350: the pool's `reserve_leftover` (counts toward the cap; `assign` returns `LeaseRetiring`; survives a wrap; `snapshot` omits it); `CorruptLeftoverRow` (`AddressOutsidePrefix`, `OffsetAlreadyHeld`) refuse the boot naming the allocation without a panic; a launch sweep across the offsets never receives the held CID; one bound of boot delay for several stuck VMs; the one release site F3 over a reservation (no plan, the provisioner not called) | `GuestAddressPool::reserve_leftover`, `CorruptLeftoverRow`, `ReclamationError::CorruptLeftoverRow`, a `SimVmHostState` hold | feature delta "Boot proceeds past a stuck VM", Pool API |
| ADR-0352 Tier 3 with a stand-in hypervisor (`/proc/self/cgroup` as its first act for a plain and a mesh launch; `Seccomp`, `NoNewPrivs`, stdio, `SIGPIPE`; serve crash with the stand-in held after `exec`; reap observed through the pidfd; cancellation of a dropped `create`); host unit tests (unresolvable wrapper, inherited descriptors, closed stdio, `LaunchStep` codec round trip); `Vmm` equivalence for membership; the driver makes no `place_pid` call | `launch_clone.rs` and `CloudHypervisorVmm::with_cgroup_root`; `SimVmm` scope membership. The stand-in needs a compiled executable the launch chain can run in a guest; no seam builds one today (§ N6 question 3) | ADR-0352 Tests |
| ADR-0352 step naming for `executing the launch program` (a wrapper that resolves to an absolute path and is not executable) | a seam that chooses the wrapper. The wrapper is derived from the confinement and `PATH` inside `create`; `with_binary` replaces the hypervisor, which runs after the wrapper's own `exec`, so its failure is the wrapper's exit, not the `execve` step (§ N6 question 3) | ADR-0352 item 8, P-7 |
| RB-13 count tests 1 to 3 (a crash restart abandoned by an operator restart forgives the crash; a lost launch charges once, also across two consecutive abandoned rows; a launch abandoned before anything ran); the deletion-equivalence proptest; `is_launch_abandoned` and `last_ran_alloc` units | `TransitionReason::{LaunchSuperseded, LaunchLostWithControlPlane}`, `is_launch_abandoned`, `last_ran_alloc` (a row of either reason cannot be built before the variants exist) | ADR-0355 section 1 and 2, RB-13 |
| ADR-0355: two golden fixtures in `schema_evolution/alloc_status_row.rs`; `human_readable` and `render::alloc_status` goldens for both reasons; `api/openapi.yaml` regeneration check; `describe` shows an abandoned launch as the current allocation | the two variants | ADR-0355 section 3, 5 |
| ADR-0355 Tier 1: after a restart commit, a shutdown and a boot the workload is relaunched once (Service and Job; the Job is not finalised), after a stop nothing is relaunched; a `Service` allocation whose row is `Pending` or terminal and carries an address has no backend | `is_natural_exit`'s exclusion of abandoned rows (RB-13); the backend-hydration case needs a row the reconciler's count basis skips. The existing seeded `restart_generation_invariant.rs` (S-INV-01) already drives the relaunch cells and will exercise them once the shim writes the rows | ADR-0355 section 5, 6 |
| RB-12 / ADR-0353: `Pending` and `Running` rows of one launch carry the same `restart_count` and `last_terminated` and the `Running` write is accepted | observable only after the `Pending` write exists; assertions are in S-RB-01 only for the address. **Not authored separately**: it needs a driver that reaches `Running`, so it rides on the first DELIVER activation of S-RB-01 | ADR-0353 Tests |
| ADR-0348 / RB-2 Tier 1 (c): the scope observation fails at the stop (typed error, no teardown, no row, the replay succeeds) | a scripted `observe` failure on `SimVmHostState` (it has `inject_probe_fault` only) | feature delta "The guest-address lease outlives an unreaped VMM", Tests (c) |
| ADR-0354: the steady-state `VmReclamation` tick that meets a busy scope leaves the row unchanged; a busy boot executor step does not discard artifacts and the next pass discards them after the hold clears | the `SimVmHostState` hold | ADR-0354 Tests |
| Tier 3: the node boots with a held-flush VM left over after a `serve` crash and does not reuse its CID (S-VOL-T3-12's boot variant); ADR-0352's serve-crash test | a crash facility on the CLI's `ServeHandle` (`ServerHandle::kill_for_test` exists in the control plane, not on the CLI handle) | ADR-0350 Tests, ADR-0352 Tests |

#### Sim test-support surface for the stuck-VM tests

Pinned by DISTILL (test support is DISTILL's to shape, per the feature delta items "A VMM kill's reap wait is bounded" 5 and "A scope kill's settle wait is
bounded" 5). All four are `TEST-HOOK-ONLY` methods on the existing sim adapters, in the style of `inject_exit`, `script_kill_scope_failure` and
`SimVolumeBacking::hold_claim_while_live`. No production surface and no new type.

**1. `SimVmm`: script a process as unreapable** (`crates/overdrive-sim/src/adapters/vmm.rs`)

```rust
/// One-shot, consumed by the NEXT `create` (as `inject_exit` is).
pub fn inject_unreapable_process(&self)
/// Whether `pid` was killed by `terminate` and has not been reaped.
#[must_use]
pub fn is_unreaped(&self, pid: u32) -> bool
```

`terminate` on the armed process marks it killed, does not fire its exit sender, waits `VMM_REAP_BOUND` on the injected clock and returns
`Ok(VmTermination::KilledNotReaped)`. While unreaped the process keeps everything it holds: `vsock_device_holders()` still reports its CID, and its
scope membership stays. `is_live(pid)` is `false` and `live_pids()` omits it from the kill on (it is killed); `is_unreaped(pid)` is `true` from the kill
until the reap. A second `terminate` on it waits the bound again and returns `KilledNotReaped` again. A process not armed behaves as today.

**2. `SimVmm`: the scripted reap** (no new method)

```rust
pub fn exit_process(&self, pid: u32, exit_code: Option<i32>, signal: Option<u8>) -> bool   // existing signature
```

On an unreaped process `exit_process` is the reap, as the feature delta says ("a scripted `exit_process`"): it clears the unreaped state, releases the
device entry, fires the exit sender once with the given ending, and returns `true`. On a live process it behaves as today. It returns `false` for an
unknown pid or one already reaped or exited. `end_process` accepts an unreaped entry; today it refuses any `terminated` one.

**3. `SimVmHostState`: hold a scope, release the hold** (`crates/overdrive-sim/src/adapters/vm_host_state.rs`)

```rust
/// The scope of `alloc` is held while `held()` is true.
/// # Panics
/// If `alloc` has no scope (`set_scope` first), or already has a hold.
pub fn hold_scope_while(&self, alloc: &AllocationId, held: impl Fn() -> bool + Send + Sync + 'static)
/// Clears the hold of `alloc`, whatever its closure says. A no-op when none.
pub fn release_scope_hold(&self, alloc: &AllocationId)
```

While held, `observe()` keeps reporting the scope (pids as `set_scope` seeded them; the hold never edits them), and `kill_scope` on it returns
`Err(io::Error::from(ErrorKind::ResourceBusy))` at once, changes nothing, takes no time and does not pop the `script_kill_scope_failure` queue. When
`held()` is false or the hold was released, `kill_scope` takes the ordinary path (queue first, then idempotent removal). The hold never removes the scope
by itself; the scope leaves through `kill_scope`, as a real empty scope does after the reap. `held()` is read at each `kill_scope` call, never cached.

**How the reap couples the two.** The one shared fact is `SimVmm`'s unreaped state of the process (`is_unreaped(pid)`). The hold reads it through its
closure, so `exit_process` clears both with no second call:

```rust
let vmm = SimVmm::new(clock.clone());
let host = SimVmHostState::new();
vmm.inject_unreapable_process();
let ctl = /* create the VMM through the driver or vmm.create */;
host.set_scope(alloc.clone(), BTreeSet::from([ctl.pid]));
host.hold_scope_while(&alloc, { let vmm = vmm.clone(); let pid = ctl.pid; move || vmm.is_unreaped(pid) });
// stop: terminate -> KilledNotReaped; vsock_device_holders() still has the CID; kill_scope -> ResourceBusy
vmm.exit_process(ctl.pid, None, Some(9));   // the reap: CID released, exit sender fires, the hold is over
// the next kill_scope returns Ok and removes the scope
```

`SimVmm` is `Clone` over shared state, so the closure sees the same process table. A hold with no process behind it (boot, pool and reclamation tests)
is `hold_scope_while(&alloc, || true)` ended by `release_scope_hold`.

**4. `SimVmHostState`: a scripted `observe` failure**

```rust
/// Queue a failure for the next `VmHostState::observe` call. FIFO, as `script_kill_scope_failure`.
pub fn script_observe_failure(&self, kind: io::ErrorKind)
```

Each `observe()` call pops the front entry and returns `Err(io::Error::from(kind))`, reading and changing nothing; with the queue empty it proceeds as
today. An armed `arm_observe_barrier` is honoured first (the call signals `entered` and waits for release, then fails). It does not touch the
`inject_probe_fault` slot.

### N4. Delta to § K (this pass only)

| Tier | Tests added | Tests changed (now pending) | Guards added |
|---|---|---|---|
| Tier 1 layer 1 | 1 pending, 2 guards (`vm_scope_has_one_remover.rs`) | `finalize_failed_genuine_terminal_forward_carries_workload_addr` (a proptest) | 2 |
| Tier 1 layer 2 | 17 pending, 5 guards: `prelaunch_outcome_acceptance.rs` 11 pending (S-RB-01 to 05, 07 x2, 08 x2, 09, 10) and 3 guards (S-RB-10, 11 x2); `vm_reclamation_boot.rs` 6 pending (S-RB-12 x3, 13 x2, 14) and 2 guards | 2 shim (S-RB-07), 5 lane, 2 boundaries, 2 mid-boot | 5 |
| Tier 3 | 4 pending (`vm_host_state_scope_kill.rs` 1, `vmm_launch_in_scope.rs` 3), 1 guard | S-VOL-T3-12 (GREEN at 09-03, marker lifted) | 0 |

### N5. Verified on metal (2026-11-24, ordinary grammar, no `--exclusive`)

| Command | Run id | Result |
|---|---|---|
| `cargo check -p overdrive-core -p overdrive-host -p overdrive-control-plane -p overdrive-cli --all-targets --features integration-tests,kvm-tests` | `d3fcd1ce7cc3b46c1456ed44` | exit 0 |
| `cargo nextest run -p overdrive-core -p overdrive-control-plane -p overdrive-host -p overdrive-cli --features integration-tests,kvm-tests -E 'test(/prelaunch_outcome_acceptance\|launches_lost_with_the_control_plane\|mid_boot_volume_reclamation\|finalize_failed_genuine\|vm_scope_has_one_remover\|launch_lane_supersession\|node_volume_boundaries\|vm_host_state_scope_kill\|vmm_launch_in_scope/)'` | `aba1dcee8e40b97f71601595` | 75 passed (a pending test passes when its body fails with its `expected` text) |
| the same filter on `overdrive-control-plane` `prelaunch_outcome_acceptance` after adding S-RB-09 | `c3cf5694cf76b12f415c998f` | 37 passed |
| `cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(a_workload_stop_during_a_held_claim_returns_promptly_with_no_failed_row)'` | `0d96472b6ec22c62309ade5c` | 1 passed (still stops at its old point) |
| `cargo clippy -p overdrive-core -p overdrive-host -p overdrive-control-plane -p overdrive-cli --all-targets --features integration-tests,kvm-tests -- -D warnings` | `247679f95d7fe6c961d25986` | exit 0 |
| `cargo xtask placement-lint` | `7a5d00df060d5f68466cc61a` | exit 0 |

### N6. Questions surfaced (no API invented)

1. **ADR-0353 for the restart arm.** The design states "a Start whose prior row is already non-terminal writes no `Pending` row" and gives the restart successor `dominating(.., None)`. A duplicate dispatch of one restart (the R9 shape: the driver answers `AllocationAlreadyOwned`) would write `Pending` over the successor's `Running` row if `dominating(.., None)` wins, and `restart_arm_duplicate_vm_owner_is_a_no_row_success_that_still_cleans_the_predecessor` asserts no successor row at all, which the `Pending` write (made before `start`) contradicts when no first launch exists. S-RB-11 pins the safe half (a second dispatch after a `Running` row never regresses it). The R9 test is **not changed**: the contract for a duplicate owner with no prior row is not stated, so DESIGN must say whether the `Pending` row is ended (and with which reason) or never written.
2. **The fail-closed `Failed` row after `Running` (`assert_superseding_failed_row`, `action_shim/mod.rs`, A-4)** asserts the row does not inherit the `Running` row's address. ADR-0353 says "every other row the shim or the observers write copies it from the prior row" but its table does not list this producer. Left unchanged; DESIGN to confirm.
3. **Test seams the design names but the tree does not give.** An injectable cgroup filesystem for `RealVmHostState`; a way to choose the launch wrapper in `CloudHypervisorVmm::create`; a compiled stand-in hypervisor for guest tests. The design's wording ("through `with_binary` and the existing `create` seams") cannot arrange `executing the launch program` or a stand-in's first-act record, because `with_binary` replaces only the program the wrapper runs. DISTILL did not add a seam.

### N7. N3 authored after the stuck-VM API compiled (commit following 5d553dd6c)

Each test is a complete body. PENDING = `#[should_panic(expected = ...)]` with the first assertion the body reaches today; GUARD = passes now and keeps passing. All rows
verified on metal (`cargo xtask metal run`, ordinary grammar, `OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=3600`): 221 tests of the new and touched files passed across
core, reconcilers, cli, control-plane, host and sim; worker 16 passed; clippy `-D warnings` clean on all seven crates; `placement-lint` exit 0.

| N3 row | Tests (file) | Tier, placement | Status |
|---|---|---|---|
| ADR-0348 Tier 1 (a) to (e) | `overdrive-worker/tests/acceptance/vm_driver_stop_unreaped.rs`: seven `guard_*` over the production `VmDriver::stop` and an unreapable `SimVmm` (bound, claim and CID held until `exit_process`, late reap emits no event, replacement waits and attaches, no `kill_scope` after `KilledNotReaped` in the stop and the start-failure cleanup, reapable VMM unchanged) | layer 2, host-pure | GUARD: the driver already ignores the termination value and the sim owns the bound |
| stop-totality | `vm_driver_stop_totality.rs::stop_is_total_over_a_killed_not_reaped_termination` | layer 2 | GUARD |
| ADR-0351 Tier 1, ADR-0349 driver side, ADR-0352 no `place_pid` | `vm_driver_scope_single_remover.rs`: `a_reaped_stop_removes_the_scope_through_the_host_state_port`, `a_start_failure_cleanup_removes_the_scope_through_the_host_state_port`, `the_driver_never_removes_a_scope_directory_through_the_cgroup_port`, `a_busy_scope_leaves_the_stop_outcome_and_the_other_cleanup_steps_unchanged`, `a_busy_scope_leaves_the_reported_start_failure_unchanged`, `the_driver_does_not_place_the_vmm_pid_in_its_scope`; guards `guard_a_held_scope_leaves_the_stop_outcome_and_artifacts_unchanged`, `guard_a_failed_create_leaves_no_vmm_process` | layer 2, host-pure | 6 PENDING (09-02, 09-05), 2 GUARD |
| sim hook contract | `overdrive-sim/tests/acceptance/sim_stuck_vm_contract.rs` (15 tests: unreapable bound, device held, reap, second terminate, one-shot arming, held scope `ResourceBusy` and the queue, hold read per call, release, scripted observe failure and its barrier) | layer 2, host-pure | 15 GUARD |
| `VmHostState` equivalence (ADR-0349), host-adapter units over a tempdir cgroup root | `overdrive-host/tests/integration/vm_host_state_equivalence.rs`: `a_failed_removal_with_a_non_busy_kind_returns_at_once_without_retry` (PENDING, "a removal failure that is not busy must return at once": today it retries for the 3 s bound), three guards | layer 3 file, tests host-pure over a tempdir | 1 PENDING, 3 GUARD |
| `Vmm` equivalence (ADR-0348 already-gone pid, ADR-0352 membership) | `vmm_equivalence.rs`: two guards; `a_created_vmm_is_a_member_of_its_scope_on_the_real_adapter`, `create_into_a_missing_scope_fails_on_the_real_adapter` (PENDING) | layer 3, `cgroup-default` / `nested-launch` guest | 2 PENDING, 2 GUARD |
| ADR-0350 pool: `reserve_leftover`, `CorruptLeftoverRow`, `GuestLeaseView::Reserved` | `guest_network/pool_acceptance.rs` (7 tests plus one proptest of 128 cases) | layer 1-2, host-pure | GUARD (09-01 built the pool surface) |
| ADR-0350 and ADR-0354 boot (busy scope, corrupt rows, sweep never receives the CID, one bound for several stuck VMs, one scope's failure keeps another's reservation) | `vm_reclamation_boot.rs::stuck_vm_boot` (13 PENDING, 3 GUARD) | layer 2, host-pure | PENDING: first RED "a busy scope never refuses the boot" (`converge` returns `Host(ResourceBusy)`), or the named assertion where distinct |
| one release site over a reservation; `VmReclamation` tick over a held scope; unremovable-scope pass | `stuck_vm_boot`: `repeated_reclaim_attempts_over_a_reservation_...`, `a_reservation_is_released_by_a_reclaim_attempt_after_the_reap_...` (PENDING behind the boot reservation), the three tick and pass `guard_*` | layer 2 | 2 PENDING, 3 GUARD |
| RB-2 Tier 1 (a) to (d) | `prelaunch_outcome_acceptance.rs`: four PENDING ("the unreaped VM's lease stays held Retiring while its scope is present", "the lease is held until the VMM is reaped", "a failing scope observation at the stop is the typed reclamation error", "the lease stays held while the scope is held, attempt 1") | layer 2, host-pure | 4 PENDING (09-04) |
| R9 rearrangement | `restart_arm_duplicate_vm_owner_writes_no_row_of_its_own_keeps_the_owners_row_and_lease_and_still_cleans_the_predecessor` (renamed; the old name said "no row", which the owner's `Pending` row now contradicts): the owner has its `Pending` row, the row and lease are unchanged, the predecessor is cleaned | layer 2 | GUARD (holds before and after) |
| `assert_superseding_failed_row` | `action_shim/mod.rs` A-4 now asserts equality with the `Running` row's address; its one caller `install_failure_supersedes_running_with_failed_and_never_releases_the_gate` is PENDING, "the superseding Failed row must carry the Running row's workload_addr" | layer 2 | PENDING (09-06) |
| RB-13 count tests 1 to 3, deletion equivalence, `is_natural_exit` | `overdrive-reconcilers/tests/acceptance/launch_abandoned_count.rs`: 7 PENDING examples (the reconciler counts from the newest row, not the newest row that ran; six count cells and the Job cell), the deletion-equivalence proptest PENDING, the `generation_replacement_pending` agreement proptest PENDING (06-01), 5 `last_ran_alloc` / backend guards | layer 2 | PENDING (09-08, 06-01) |
| `is_launch_abandoned`, wordings | `overdrive-core/tests/acceptance/launch_abandoned_predicates.rs` (6, one a proptest) | layer 1 | GUARD |
| goldens (`describe`, wordings) | `overdrive-cli/tests/acceptance/workload_describe_abandoned_launch.rs` (3) on `render::workload_describe` | layer 1 | GUARD; schema-evolution fixtures were already added by 09-01 |
| ADR-0355 Tier 1 relaunch | `restart_generation_invariant.rs`: two Job cells, seeds 348001 to 348006 | Tier 1 seeded | GUARD today; they bind once the shim writes the abandoned rows (09-06) |
| ADR-0352 Tier 3 with a stand-in; serve-crash test | `overdrive-cli/tests/integration/vm_crash_mid_launch.rs::a_serve_crash_just_after_the_vmm_is_spawned_leaves_nothing_outside_a_scope_boot_reclaims` | layer 3, `cgroup-default` / `nested-launch-rustc` | PENDING: "the hypervisor's first act must run inside its allocation's cgroup scope"; it recorded `["0::/overdrive.slice/control-plane.slice"]`, so the VMM spent its first instructions in `serve`'s slice. Every assertion before it passes today (the stand-in recorded and claimed its CID, survived the crash, was reclaimed by the second boot with its scope removed, and the relaunch claimed its CID with no clash) |
| S-VOL-T3-12 boot variant | `volume_survives.rs::a_node_boots_past_a_vm_stuck_in_exit_left_over_by_a_serve_crash` | layer 3, `cgroup-default` / `volume-pool` | PENDING: "a node must boot past a VM whose scope cannot be removed; boot refused: ... reclamation executor failed: VmHostState operation failed: Device or resource busy (os error 16)" (09-07) |

#### The two test hooks

**(a) Abrupt crash of the in-process `serve`: it already exists; nothing was added.** `ServeLifetime` with `ServeSignal::Kill` is the in-process stand-in for `SIGKILL`
(`ServeHandle::kill_for_test` is `pub(crate)`; the lifetime port is the public route), and `overdrive-cli/tests/integration/serve_lifetime_support.rs::kill_serve_owner` is the
test-side call (`vsock_support.rs` already boots, kills and reboots through it). It runs `ServerHandle::kill_for_test`: the gateway stops with a zero grace, every
control-plane task is aborted and joined, the convergence, exit-observer and interest-router tasks, the shared-network supervisor, the guest flow owner and the
resolver are released, and the mTLS worker is dropped on a thread that has given up every capability, so its destructors cannot touch the kernel state the dead owner held.
Workloads, cgroups, and the kernel objects stay. **What it cannot model:** the owner's memory is dropped, not torn away, so a destructor that runs only on an orderly
path still runs; no signal or exit status reaches a parent; a descriptor leaked to a child process would be released here by the drop, not held by the child. For this
feature (a crash between spawn and the first reconciliation) none of those matter: the observable is the surviving VMM and its scope, which the kill leaves exactly as
`SIGKILL` does. The facility is used by both new Tier-3 tests.

**(b) Stand-in hypervisor.** Source: `crates/overdrive-cli/tests/fixtures/standin_hypervisor.rs` (a test fixture directory; Cargo does not discover it as a test target and no
`crates/*/src` file holds it). Delivery to the guest: the source is embedded with `include_str!` in `vm_crash_mid_launch.rs`, which writes it into a staging-root tempdir and
builds it with the `nested-launch-rustc` guest's own `rustc` for `x86_64-unknown-linux-musl` (the target that guest's sysroot carries), so the guest kit gains no artifact
and no kit change is needed. The test passes it to `CloudHypervisorVmm::with_binary`. The program answers `--version` and `--help` (naming `--landlock`), as the launch
probes require. Launched with the real argument list, its first act is to append `/proc/self/cgroup` and its pid to `standin.record` in its run directory (the parent of
`--api-socket`, which the launcher chowns to the confined identity, so no environment variable or extra path crosses the launch chain); it then opens
`/dev/vhost-vsock`, takes ownership and sets the guest CID from `--vsock cid=<n>,backend=vhost-kernel`, records the outcome, and blocks holding the device. It reaches
the real kernel CID table, so a CID clash between a survivor and a new launch is `EADDRINUSE` in its record. Limits: it never boots a guest, so `start` waits on its
beacon until the boot deadline (the tests do not wait for `Running`); the wrapper-chooser seam (§ N6 question 3, `executing the launch program`) is still not available.

#### Corrections to the earlier sections

- N3 "injectable cgroup filesystem": `RealVmHostState::new(cgroup_root, run_root, index_dir)` takes the root; the tempdir units were authored against it. What a tempdir cannot show
  is removal only after a `0` reading from `cgroup.events` (a regular file is not pollable); that stays on the kernel (S-RB-18).
- N3 "`render::alloc_status` goldens": `render::alloc_status` no longer exists; the live renderer is `render::workload_describe` (the goldens target it).
- N3 hook (a) "crash facility on the CLI `ServeHandle`": exists as above.
- N6 questions 1 and 2 are settled by the design (09-06 notes): the R9 test and `assert_superseding_failed_row` are changed as in N7.
- `SimVmm::terminate` ignores its `grace` argument, so "exactly `VMM_REAP_BOUND` beyond the grace" is asserted as the bound after the kill on the shared clock.
- `hold_scope_while`: the sim doc says it panics if the allocation already has a hold; the code asserts only that the scope was seeded and a second call overwrites. Not tested; one of them should change.

#### Not authored

| Test | Missing | Why |
|---|---|---|
| `LaunchStep` codec round trip; siginfo-to-`VmmExit` mapping; unresolvable wrapper is `ConfinementUnavailable` with no child; descriptors not inherited; closed stdio; pidfd reap; cancellation of a dropped `create` | `launch_clone.rs`, `LaunchStep` and the pidfd reaper (09-05) | the types do not exist; the design pins the behaviour, not a signature a test can compile against. `CloudHypervisorVmm::with_cgroup_root` exists and is exercised by the membership tests above |
| `SimVmm` scope membership equivalence, absent-scope create refused by the sim, "a failed create leaves no scope member" | a membership observable on `SimVmm` and its refusal of an absent scope (09-05 names the behaviour: "SimVmm records membership ... and fails create for an absent scope") | no accessor is pinned; a test-support method is not invented here (CLAUDE.md "agents propose, the user adds"). **Proposal for the orchestrator:** `SimVmm::scope_members(&self, scope: &CgroupPath) -> BTreeSet<u32>`, test-hook only, with the 09-05 behaviour |
| a call recorder on `SimVmHostState` (`kill_scope` calls) | none pinned | detected through the scripted-failure queue (a probe `kill_scope` returns `Ok` exactly when the driver consumed the queued failure first) |
| `ADR-0352` step naming `executing the launch program` | a seam that chooses the wrapper (N6 question 3) | unchanged: surfaced, not invented |
| `restart_status_for_alloc` reads attempts through the count basis; `restart_after_failure`'s `ran` parameter | the signature in the tree has no `ran` and `restart_status_for_alloc` gets only `failed`; the runtime holds no observation store | the design must say where the rows are read (feature delta says "signature unchanged"): a DESIGN question, not authored |
| backend hydration through `ServiceLifecycle` for a never-ran `Service` row | only the `ServiceAllocPhase::from_row` guard was written; the existing totality test covers non-`Running` states | no row shape beyond today's reaches the reconciler's count basis |
| S-VOL-T3-12 retarget (09-03) | the retarget moves the marker to the next RED, which exists only once 09-03 bounds the reap | done at 09-03's gate: the body passes honestly once the reap is bounded, so the marker is lifted rather than retargeted (S-VOL-T3-12 is GREEN at 09-03, not 09-04) |
| CID-reuse oracle in the boot variant | the not-yet-started launch's CID is not observable through `serve` + `deploy` while the claim is held | covered at Tier 1 by `a_launch_sweep_across_the_whole_offset_range_never_receives_the_stuck_vmms_cid` |
| real-adapter `ResourceBusy` and `KilledNotReaped` | no producer on demand without the held-device fixture | `vm_host_state_scope_kill.rs` (S-RB-18) owns the real busy case; the unreapable VMM is sim-only |

#### One production line touched (flagged for the orchestrator)

`crates/overdrive-control-plane/src/guest_network.rs`: `#[expect(dead_code, ...)]` on `GuestAddressPool::reserve_leftover` became `#[cfg_attr(not(test), expect(dead_code, ...))]`.
The 09-01 scaffold's expectation is unfulfilled in the test build as soon as any test calls the method, which fails `clippy -D warnings` for the whole `overdrive-control-plane` lib-test
target; the tests that call it are the required reservation tests. A lint attribute only, no API; 09-07 removes the attribute when it adds the caller.

#### Stale names to update when the tests are activated

The roadmap, N1 and `red-classification.md` still cite `restart_arm_duplicate_vm_owner_is_a_no_row_success_that_still_cleans_the_predecessor`; its name is now
`restart_arm_duplicate_vm_owner_writes_no_row_of_its_own_keeps_the_owners_row_and_lease_and_still_cleans_the_predecessor`.
