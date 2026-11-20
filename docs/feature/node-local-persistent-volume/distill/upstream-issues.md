# Upstream issues and DISTILL obligations (node-local persistent volume, GH #348)

The DESIGN wave pins contracts, boundary scenarios and named seeded invariants in `../feature-delta.md`; everything executable, every
placement and every re-sync list lives here. DISTILL applies it after the modeler has re-recorded both specs. User rulings of 2026-11-19: J-1
(a Job's failure before the driver is final), DE-1 (non-start driver errors are not a decision of this feature), one hazard each for ADR-0328
and ADR-0329, P-3a (option A: S-VOL-T3-11 tests the production write-back flush, not fork fix F1), and the subtractive ruling that dropped the
Tier-3 stop-during-boot test (S-VOL-T3-13, restored by B-2). No deferral language is written and no GitHub issue is created by this file. Where DISTILL
deviated from an obligation or took a decision, the obligation is followed by a **Deviation** or **Decision** note; an obligation met as written has none.
The scenario tables are `test-scenarios.md`; the failure each pending test shows today and the activation order are `red-classification.md`.

## DISTILL input from the DESIGN of 2026-11-19 (executable obligations, mutation targets and re-sync lists)

### Restart generation (ADR-0326, ADR-0328, ADR-0329)

1. *Seeded invariant* `restart_generation_is_placed_exactly_once_across_supersession_and_crash`, through the real
   `WorkloadLifecycle`, `ReconcilerRuntime` (tick, lane table, View persistence), the real action shim and `VmDriver` over `SimVmm` /
   `SimVolumeBacking` / `SimDriver` parked starts, and the real `stop_workload` / `restart_workload` handlers. Matrix: origin in {restart of a
   `Running` workload, resume of an operator-stopped workload} x interruption in {`stop_workload`, a second `restart_workload`,
   `supersede_all_lanes` (shutdown), a control-plane crash during the launch (`kill_for_test`, stores reopened, boot reclamation run)}; then the
   next boot. Per-observation-point coverage stays in the `VmDriver::start` tests (§ Launch path below). Assertions, seed printed: (a) unless the
   interruption is a stop the workload eventually has a `Running` allocation whose `issued_generation` equals the committed generation; after a
   stop it stays down until the next restart; (b) per generation at most one replacement launch is in flight and each abandoned launch is
   followed by exactly one relaunch (counted at the driver's `start` calls); (c) no `Failed` row and no `restart_counts` change attributable to an
   abandoned replacement launch (a crash-restart relaunch re-assigns the same count); (d) a second evaluation with the row present issues
   nothing. Fault dimension *pre-launch failure* in {none, persistent, transient}, injected at the CA port (`SimCa` failing `issue_svid`): (e)
   persistent: a Service ends with a `Failed` allocation whose terminal is `BackoffExhausted`, the launches after one operator commit are at most
   the restart ceiling plus the one uncharged generation replacement, and with `SimClock` held within one backoff window they are the replacement plus at most one ungated crash restart (a generation replacement inherits its predecessor's `last_failure_seen_at`, `workload_lifecycle.rs` ~1311-1333, so its first crash restart is immediate and later ones are gated), then at most one launch per backoff window;
   (f) transient: the Service recovers to `Running`, the `Failed` rows of the failures present with the pinned reason; (g) `issued_generation`
   and `restart_counts` gain no more entries than there were launches. Workload-kind dimension, kind in {Service, Job}, for a Job, origin in
   {restart of a Job that completed, restart of an operator-stopped Job, restart of a running Job}: (h) a Job whose current allocation carries a
   terminal claim is never given a new allocation by a restart: across many ticks no `RestartAllocation` or `StartAllocation` is emitted, no id
   is issued, and `issued_generation` and `restart_counts` gain no entry; a restart of a running Job ends it and issues nothing after. **Job
   pre-driver case (J-1):** a Job whose SVID issuance fails (persistent and transient) writes one `Failed` row, the next tick finalises it
   `Failed { exit_code: Some(0) }`, and no further `StartAllocation` is emitted however many ticks run (the pre-driver failure of a Service
   retries to `BackoffExhausted`; the Job's does not). This pins the misleading exit code as today's behaviour.
   **Deviation:** the invariant is in `crates/overdrive-control-plane/tests/integration/restart_generation_invariant.rs`, not `overdrive-sim`: it is driven through the real axum handlers and
   `overdrive-sim` has no axum edge; it opens a real redb, so it is under the crate's integration entrypoint with a module-level `placement!(host_pure)` (the binary also holds guest-placed tests). Assertion
   (e) is exact (revised): the work order's "never exceed one within one window" was wrong against production's carry rule; the test asserts at most two launches with the clock held (replacement plus one
   ungated crash restart), then at most one launch per backoff window over three further windows. The subtractive pass removed the finished-Job and operator-stopped-Job restart tests and the transient Job
   fault (the fence is covered by S-RG-02 and the quint Job runs, the identity row by the shim tests) and folded the per-origin tests of the pre-launch dimension into loops inside four tests.
2. *Pure reconcile tests* (proptest over `(rows, view, desired.generation)`): `generation_replacement_pending` is true iff the current row's
   entry is below the generation and the current row is not a Job row with a terminal (asserted against `allocation_attempt_transition`'s
   `NoChange` over generated rows); with the issued id's row absent the reconcile issues exactly one new id with entry `desired.generation` and,
   for a generation replacement, unchanged `restart_counts` / `last_failure_seen_at`; with the row present it issues nothing; N pre-placement
   bumps still coalesce into one placement (S-BIR-COALESCE-PLACE) and an evaluation after the successor's row exists does not re-place
   (S-BIR-COALESCE-NO-REPLAY, which supplies that row).
   **Deviation:** the reconciler keeps its decision through the private compat rule `restart_pending_until_successor_observed` until DELIVER adopts the helper, so the helper tests re-raise the scaffold's
   panic as their contract assertion (any other panic is resumed) and only 3 of the reconcile tests are pending; the rest guard today's behaviour.
3. *Shim test* `restart_arm_superseded_successor_runs_exact_old_predecessor_cleanup`: a `RestartAllocation` whose successor returns
   `StartSuperseded` writes no row, releases the successor's lease, SVID and supervision claim, and runs the exact-old predecessor cleanup
   (`finish_restart`), asserted at the driven ports.
3a. *Shim tests per pre-driver failure path* (sim CA, `SimDriver`, sim guest-network ports, sim observation store): (i) S5 and R7,
   `ensure_intercept_identity` fails: a `Failed` row for the issued id with `MtlsInterceptInstallFailed { stage: "identity_issue" }` whose
   `detail` carries the issuance cause, the guest lease released, the lifecycle event emitted, the dispatch `Ok(())`; in the restart arm the
   predecessor's exact-old cleanup also ran; a teardown failure does not suppress the row and is returned after it. For a Job the same row is
   written. (ii) R3, an empty driver registry in a restart: a `Failed` successor row, not `HandleMissing`. (iii) the charge, for a Service: after
   each of the above a following tick with the backoff window elapsed emits one `RestartAllocation` whose id's `restart_counts` entry is the
   failed row's plus one (crash path), and after the restart ceiling emits `FinalizeFailed { BackoffExhausted }`. (iv) N-ADM: an admission
   refusal in each arm writes no row and leaves `restart_counts` as at the predecessor; the restart arm releases the predecessor's lease; after a
   start-arm refusal with the pool still at the cap the next hydration emits nothing; after a restart-arm refusal the next hydration reads one
   slot fewer and emits again. (v) S0 and R0, a failing prior-row read: the dispatch returns the observation error with no effect and no row.
   **Deviation:** there is no separate R3 charge assertion (R7 is the chain after S5; R3 writes a row and the same charge decision applies); the N-ADM tests (iv) and the S0/R0 tests (v) are guards, because
   the refusal handling exists today. The subtractive review added guards for the existing outcomes the new arms must not disturb (S1, R2, R6, R9, S10, R14 and the boot-race stop).
4. *Mutation targets* (`workload_lifecycle.rs`): the helper's `<`, the `desired.generation` written into `issued_generation` at both sites, the
   carry of `restart_counts` for a generation replacement.
5. *Existing tests that read or seed `observed_generation`* change: `overdrive-core/tests/acceptance/workload_lifecycle_{restart,reconcile_branches,
   terminal_decision,natural_exit,recompute_deadline,release_service_vip}.rs`, `overdrive-reconcilers/tests/acceptance/vm_recreation_allocation_identity.rs`,
   `overdrive-control-plane/tests/acceptance/{generation_read_coupling,runtime_convergence_loop,single_restart_authority_liveness_trajectory}.rs`,
   `overdrive-sim/tests/acceptance/reconciler_is_pure_with_workload_lifecycle.rs`, the `hydration_trajectory/workload_lifecycle_trajectory.txt`
   golden, `overdrive-sim/tests/vm_lifecycle_latency_283_spike.rs`, and in `overdrive-control-plane/src/reconciler_runtime.rs` the production
   site `restart_status_for_alloc` (`:514-534`, which reads `view.observed_generation < desired_generation` and moves to
   `generation_replacement_pending`) and the test helper that sets `view.observed_generation` (`:1857-1935`); doc comments at `handlers.rs:908` and
   `overdrive-core/src/aggregate/mod.rs:1296`, and `api/openapi.yaml`'s description.
   **Deviation:** the field is renamed `issued_generation` mechanically; prose in `generation_read_coupling.rs` and a test-helper parameter with its doc comment in `reconciler_runtime.rs` still say
   `observed_generation`. The inline restatement of the compat rule in `reconciler_runtime.rs` is marked for deletion with the helper.

### Launch path, lane table and shutdown (ADR-0325)

- *`VmDriver::start` observation points* 1 to 6 (§ Superseding a launch of the feature delta): a signal raised at each point, with the volume
  steps first; superseded wins over a typed failure found at the same point; a teardown failure is `StartRejected`.
- *Lane table* (through the real tick; the table's use, not the token library's semantics): `supersede_lane` with and without an open lane;
  `supersede_all_lanes` before, between and after concurrent lane opens (a lane opened after is born raised); a lane signal raised before the
  lane opens, after it opens and before hydration, and during dispatch (A8); `supersede_lane` never raises another lane's signal; the lane guard
  removes its entry by key. No test overlaps two ticks of one target (ADR-0102 admits one lane per target).
  **Deviation:** the per-position `supersede_all_lanes` tests were removed as duplicates of the seeded schedule (`supersede_all_lanes_at_any_point_of_a_seeded_schedule_abandons_every_launch`, seeds 0..4 place the call at positions 3, 2, 0, 3 of the lane opens), and the handler-raise tests that duplicated the restart and stop-while-waiting tests were removed.
- *Loop shutdown arm*: `spawn_convergence_loop` with a parked `SimDriver` start and a shutdown cancel: the launch returns `StartSuperseded`, no
  row is written, the loop exits without `SimClock` advancing past the launch's bound.
- *Boundary 4 and 4a, a stop evaluated after a held launch*: a single-tick test through
  `run_convergence_tick_with_guest_network_provisioner_for_test` and `ReconcilerRuntime::supersede_lane` with a parked `SimDriver` start and a
  `SimLaunchSupersession`, and the Tier-3 stop-during-held-claim and stop-during-boot tests through `serve` + `deploy` (feature delta § Gate
  G-3 evidence lane). No new loop seam. A Tier-3 shutdown-during-claim-wait test is not required: its kernel behaviour is the stop-during-held-claim
  test's. S-VOL-66 is replaced by these tests.
  **Decision (subtractive ruling, then B-2):** the Tier-3 stop-during-boot test (S-VOL-T3-13) was dropped and is restored by the user's B-2 ruling of 2026-11-19 as the twin of the `alive[a]` arbiter read. The stop-during-held-claim test (S-VOL-T3-12) is kept and
  carries its `#[should_panic]` marker.

### Waiting state (ADR-0327, ADR-0329)

1. *Unit and proptest, `start_blocked_only_by_capacity`:* a table over the class of the current row, each with and without a pending generation
   and with the pool at `cap - 1`, `cap` and `cap + 1`: no row (true); a crashed row under budget (true); a row at the ceiling (false, true once
   a generation is pending); `BackoffExhausted` (false, true once pending); operator-stopped (false, true once pending); `Stopped{SystemGc}`
   (true); `Stopped{LivenessProbe}` (true under budget); a Job that completed or failed, and an operator-stopped Job (false, and still false with
   the generation bumped); a leased current row (false); a stopped workload, a `Running` row, a `Draining` current row, a Schedule (false); a
   `Draining` historical row beside a newer crashed current row (not blocked: the Draining test reads the current row only); retiring leases
   count toward `held`. Mutation
   targets: each arm of step 6, the lease test, the Job exception, the `at_cap` comparison.
   **Deviation:** the table gained Job rows (no row, current `Reclaimed`, current `SystemGc`, a historical `Completed` beside a current `Reclaimed`) and `Stopped{StartupProbe}` rows.
2. *Render golden tests* in `tests/acceptance/render_workload_describe.rs`: the `Waiting` line for a Service and a Job; the empty-state message
   suppressed when `capacity_wait` is `Some`; output byte-identical when `None`; the JSON field present and absent.
   **Deviation:** the goldens are in the new file `crates/overdrive-cli/tests/acceptance/workload_describe_capacity_wait.rs` and the empty-state suppression, which the pure render cannot host, is the module
   `capacity_wait_empty_state` of `crates/overdrive-cli/tests/integration/workload_describe.rs` (a describe against a stub HTTPS control plane). The JSON present/absent and wire round-trip guards were removed in the subtractive pass.
3. *Handler test* through the real router and `AppState`: the response carries `capacity_wait` when the predicate holds and omits it otherwise;
   `held` and `cap` are the pool's; a workload restarted after an operator stop with the pool at its cap shows the wait; a finished Job at the
   cap shows none.
   **Deviation:** the real handlers over `AppState`, not an HTTP router; the pool is filled to its 16,384 cap through production `StartAllocation` dispatches (its lease operations are crate-private). The
   fill is one test in the integration lane (`tests/integration/alloc_status_capacity_wait.rs`) covering the withheld first placement and the no-wait cases of a running workload, a finished Job and an operator-stopped workload (the restart-at-cap case is the composition test, item 4); about 56 s on a debug build,
   inside the default 120 s budget, so no `.config/nextest.toml` override exists.
4. *Composed test (W-3 = A, ruled 2026-11-18; the test composition is DISTILL's):* the smallest in-process production composition: the real
   router, reconciler runtime and action shim, with a test-owned real `GuestAddressPool` filled to the cap through the pool's own admission and
   `SimDriver` starts. `deploy` a VM workload and, separately, `workload restart` a running one: `describe` shows the `Waiting` line, the
   workload has no allocation row, no `Failed` row exists, `restart_counts` is unchanged and (restart case) the predecessor's lease was released;
   free one slot, advance to the resync pass, and assert the workload is placed, reaches `Running`, and `describe` no longer shows the line. No
   production surface is added.
   **Deviation:** the resync pass is realised as the per-target `workload-lifecycle` evaluation it submits, because the pass is crate-private; the two scenarios are one integration-lane test
   (`tests/integration/capacity_wait_composition.rs`) and `restart_counts` is compared as map equality. The test asserts the response's `capacity_wait { resource, held, cap }`, not the CLI-rendered `Waiting` line: see blocker B-3.
5. *Agreement properties (both directions, over generated `(kind, stop, rows of every class, view, observation, generation)` with ample node
   CPU and memory):* (a) whenever the predicate is true, `WorkloadLifecycle::reconcile` emits no `StartAllocation` or `RestartAllocation`; (b)
   with the observation lowered to `cap - 1` and the backoff window elapsed, whenever the predicate over the at-cap observation is true,
   `reconcile` emits one. The window of a launch in flight is excluded by construction (the reconciler cannot see a launch that holds the lane).
   **Deviation:** a third property, (c) a start a freed slot releases is a blocked workload, and a non-vacuity pair (a guard that the generator reaches withheld and released starts, a pending test that it reaches both verdicts) were added.
6. *Shim test* (extends 3a (iv)): after an admission refusal in each arm, the predicate over the resulting rows, the View and the pool is true.
   **Note:** the restart-arm case is a refusal whose predecessor held no lease; the lease-holding case is the N-ADM guard of 3a (iv).

### Model and boundary tests

- M7: `boot_reclamation_ends_a_mid_boot_vmm_before_any_launch_for_its_workload`, an in-process `serve` test in the shape of the
  `boot_reclamation_before_assign` cells (`crates/overdrive-control-plane/src/vm_reclamation_boot.rs`): one `SimVmm` and one `SimVolumeBacking`
  outlive two compositions; in the first a workload with a declared volume deploys and its launch is parked in the guest-boot wait, then the
  composition is dropped; the second boots through `run_server`. Asserted at the driven ports: (a) the orphan's pid is not live once boot returns
  and before the first `StartAllocation` reaches the driver; (b) the orphan's claim drains and the first successor's launch attaches without a
  `VolumeUnavailable { Held }` row and with nothing charged; (c) the same holds when the crash leaves a `Running` row (`ReclaimAllocation`)
  and when it leaves none (`DiscardStrandedArtifacts`). The conformance driver's `Reclaim` arm drives `converge` through the production owner
  and projects the orphan's liveness and the claim from `SimVmmHost` / `SimVmm::is_live` and `SimVolumeBacking::claim_state` (the
  `sim-arbiter-ok` read paired with S-VOL-T3-04).
  **Decision:** the tests are two (the no-row and the `Running`-row residues) in the module `mid_boot_volume_reclamation`, over two boots of `run_server_with_obs_and_driver` on one data directory, the workload deployed through the
  public HTTPS API and `kill_for_test` as the crash; boot reclamation is the second boot's own step and nothing calls `converge`. A `StartRecordingDriver` records every `Driver::start` with the orphan's liveness. The claim drain is
  test support, `SimVolumeBacking::hold_claim_while_live(workload, live)` (a scaffold no-op until DELIVER, with a pending contract test in `sim_volume_backing_contract.rs`), linked by the recording `Vmm` to
  `SimVmm::is_live(pid)`. The only `sim-arbiter-ok` read left in the volume driver is `claim()`, citing S-VOL-T3-04; the other arbiter markers and the `vmm_live` projection were removed (no Tier-3 twin).
- Conformance pins. The volume driver `quint_node_volume.rs` pins the twelve runs of the feature delta's table as `quint_vol_<run>` with
  `#[quint_test(max_samples = 1)]`, plus `quint_vol_randomized` (`#[quint_run]`); its `switch!` has no `LaunchPreFail`, `LaunchRefused`,
  `CapFill`, `CapDrain`, `IdFaultOn` or `IdFaultOff` arm, and `Boot` replaces `CpCrash` for the shutdown run. `GuestWrite` and `DeviceSlow`
  have no owner port (guest I/O and in-flight device latency are guest and kernel facts): their handlers check the action's guard against
  production and stimulate nothing. The restart-generation driver `quint_restart_generation.rs` pins the ten runs of the delta's table as
  `quint_rg_<run>` (mains `rg_conf` and `rg_conf_job`), plus `quint_rg_randomized`. Its `LaunchLost` is realised as
  `ReconcilerRuntime::supersede_lane(target)` on the held dispatch with neither `capFull` nor `idFault` armed (the guard of the model's
  `LaunchLost`, the shim's order); the `SimDriver` start observes the lane signal and returns `StartSuperseded`: no row, dispatch `Ok(())`.
  `CpCrash` and `NodeReboot` are a rebuild of the runtime and stores; both leave an issued id with no row and a completed or dropped tick,
  which is the model's lost launch. Until `issued_generation_for_alloc` exists the restart-generation tests are pending by
  `#[should_panic(expected = ...)]` with their semantic RED, not gated out. The seam that holds the dispatch between the tick and the outcome
  is DISTILL's to shape; if it needs production surface, return a testability request and invent no API.
  **Deviation:** the dispatch is held at a one-shot `ObservationStore::alloc_status_row` read, not at the `SimDriver` park (`test-scenarios.md` § L, c); `VmmSpawnFail` is a driven-port fault through `SimVmm::inject_create_failure` with the
  launch's `create` parked at a test-support gate (§ L, e). `issued_generation_for_alloc` exists, so the drivers are pending by `#[should_panic]`, not gated out. The Job runs now fail on any issued allocation id that no
  model allocation names and on any row with no model label, with one further unheld tick after the Job restart or resume; `quint_rg_resume_of_stopped_job_does_nothing_and_issues_no_id` is therefore pending (it was a vacuous guard) and
  moves with the generation rule. The volume driver's launch-failure runs assert the `Failed` row's reason through the owner's rows, matched by name (the `VmVolumeUnavailable` variant does not exist yet); `charged` stays model-only (pinned that way).

### Placement

Core, sim, worker-acceptance and cli-acceptance tests are `host_pure`. The control-plane seeded restart-generation invariant is `host_pure` by a module-level `placement!(host_pure)` inside the integration entrypoint
(`tests/integration/restart_generation_invariant.rs`), because that binary also holds guest-placed tests and a binary default would conflict with them. The boot-gate tests (`volume_boot_probe`) are `guest = "cgroup-default"`
(base: a real in-process `serve` with the probe port replaced); the H2 test is in the `nested-launch` guest, `kvm-tests`-gated (N-16). Every test that needs the pool or boots a VMM against it, including all of
`volume_pool_probe_pool`, `volume_pool_backing` and `volume_survives`, is `guest = "cgroup-default", profile = "volume-pool"`; the soft-case probe
guard is on the base profile. `volume_survives.rs` is gated `kvm-tests` like `guest_stack_mtls_egress.rs`. No test boots `serve` or touches LVM, dm or Cloud Hypervisor on the host, and none is `native_launcher`.
`volume-pool` is in `placement/groups.rs` (23 environments), `placement/tests.rs`, `guest_suite.rs` (mapped to the `nested-launch` guest) and
`.claude/rules/testing.md`.

The roadmap is DELIVER's; the dependency order the tests require, with what each row turns GREEN and which markers move, is `red-classification.md` § "DELIVER activation order". The guest-kit volume-pool row is first.
Its prerequisites are N-5.

S-VOL-21 (the V2 fixtures) is not authored: the V2 bytes exist only in DELIVER's one atomic envelope-bump commit.

### P-3 fixture (S-VOL-T3-11)

The claim and the kernel citations are in the feature delta § Test infrastructure decisions (P-3, P-3a).

- **Relabel (P-3a, user ruling 2026-11-19, option A).** In the production write-back attachment a guest `fsync` is an explicit flush, and stock Cloud Hypervisor already propagates a failed explicit flush
  (spike run-022 and run-030, `a3_fault_wb_explicit_flush_*`). S-VOL-T3-11 therefore tests that behaviour; it does not test fork fix F1 (ADR-0316). F1's regression test lands in the fork
  (`overdrive-sh/cloud-hypervisor`) with the F1 patch, as DELIVER's fork-side work, not in this repository (feature delta V-3).
- **Placement:** `guest = "cgroup-default", profile = "volume-pool"`, through `serve` + `deploy` like S-VOL-T3-07.
- **Scenario:** a VM workload with `[vm.volume]` is Running and the guest has written to its volume block device with `O_DIRECT`, so nothing is
  dirty in the guest page cache. A control `fsync` on that block device returns success. While the VMM runs, the test replaces the volume's
  device-mapper table (the node of the delta's § Port) with `dm-flakey` erroring writes for the whole test (suspend, load, resume): each
  segment of the current table (read with `dmsetup table`) is rewritten from `linear` to `flakey <same dev> <same offset> 0 <down> 1
  error_writes`, with up interval `0` and a non-zero down interval longer than the test, so the device is down from the first bio. The guest
  then calls `fsync` on the block device. With nothing dirty its only work is the flush (`REQ_PREFLUSH`), which `dm-flakey` errors; the test
  asserts the guest's `fsync` returns an error.
- **Guest image:** `dm-flakey` is a module of the host kernel the guest boots (`CONFIG_DM_FLAKEY=m`; `spike/findings.md` line 15,
  `spike/evidence/run-026-recon-host2.log`), so there is no kernel-config change. The guest rootfs carries the module file and its loader
  alongside `dmsetup`, staged by the guest-kit step (activation row 1). The guest uses a busybox `dd` with `oflag=direct` and `conv=fsync` (unverified
  until the guest exists).
- **Precondition:** the flush reaches `dm-flakey` only when the device under the volume's table reports a write cache. The `volume-pool` backing
  disk's cache mode is provisioning's to set so that it does; the test asserts the volume's dm device reports `write back`. Without a cache the
  guest's `fsync` succeeds for a reason that has nothing to do with the flush path.
- **Cost accepted by the user (2026-11-18):** none in production, one module file in the guest image, and seconds of extra test time.

### Re-sync list (DISTILL applies it against the re-recorded specs)

- `distill/test-scenarios.md` and `distill/red-classification.md`: `DiskSerial` and `waited_ms` become the constant `VOLUME_DISK_SERIAL` and `Held` with no field (there is no `DiskSerial` type; the two S-VOL-26c
  tests that waited on a `DiskSerial` constructor have nothing to wait for; S-VOL-08..10, which tested a `DiskSerial` grammar, are retired: the serial is pinned by the argv golden).
- The restart-generation driver loses its `Shutdown`, `Boot` and `LaunchSuperseded` arms (its model has no lane signal) and pins ten runs; the
  volume driver keeps `Shutdown`, `Boot`, `LaunchSuperseded` and `GuestReady`.
- The `SimDriver` start observes the lane signal only when scripted for `LaunchLost`, since `restart_workload` really raises it (a restart commit
  enables the loss and does not force it).
- `distill/test-scenarios.md` section E named nine runs (S-VOL-Q1..Q9) and listed the twelve runs of the volume spec instead. S-VOL-66 is replaced
  by the boundary 4 and 4a tests above.
- Every `Driver::start` call and every `impl Driver` takes `&dyn LaunchSupersession` and every exhaustive match on `DriverError` gains
  `StartSuperseded` (mechanical; the shapes are the pinned ones in the feature delta § Superseding a launch).
- The shim's `Err(other)` start arm is not a decision of this feature (DE-1): no S8 or R10 test.
- A scenario text that says "the next boot places a fresh allocation" or "the handler's pending evaluation re-launches" without the
  observed-successor rule is corrected to the feature delta § Restart generation, "Relaunch".

## Device path (R-1)

The volume device is the device-mapper node `/dev/mapper/overdrive-ovd--<hash>` (dm name = `<vg>-<lv>`, every `-` in the VG and in the LV
doubled; `<hash>` is 32 lowercase hex and contains no `-`), not `/dev/mapper/ovd-<hash>`, which does not exist. It is pinned in the feature delta
§ Port ("The device path") with the V-1 capture as grounding. Every test and fixture spells it: the argv goldens, the four name-derivation golden vectors
of the sim contract (the earlier proptest over the 47-character bound is dropped), `volume_survives.rs`'s `volume_device()`, `volume_pool_support.rs` with its `dm_node_name` doubling helper, and
`volume_pool_backing.rs` asserting the exact ADR-0312 vector for `dev-box`, the vector the sim golden shares. There is no shared name-derivation function in core: host and sim derive the name separately, and that shared vector is the cross-adapter equivalence assertion.

## Open decision

- **D-2 (user decision): `--only` cannot name a pending invariant.** `Invariant::from_str`, which `cargo run -p overdrive-sim --bin dst --
  --only` uses, resolves names against `Invariant::ALL` only, and the catalogue's pending convention keeps a pending invariant outside `ALL` (as
  the gateway invariants were), so `--only volume-single-writer` answers "unknown invariant". They run through `Harness::new().only(..).run(seed)`
  (the acceptance test prints each seed). Options: (a) leave it: the three join `ALL` at activation (N-18) and `--only` and the reproduce line
  work then; (b) make `from_str` resolve every variant, so a pending invariant is reproducible from the CLI now (a change to the catalogue
  mechanism, not to this feature). Recommendation: (a).

## Blockers and decisions for the design owner or the user

Each needs a ruling before the activation row that depends on it. No GitHub issue is proposed or created by this list.

- **B-1 (RESOLVED, pinned 2026-11-19): who enforces the 30 s `ensure` bound.** The `VolumeBacking` adapter does; the driver awaits `ensure` to its return with no timer (feature delta § Port). `SimVolumeBacking` holds no timer and models no deadline: a test parks an `ensure` with `pause_ensure_after_create`, reads `ensure_parked`, and releases it with `resume_ensure`. `the_driver_holds_no_ensure_timer_a_parked_ensure_outlives_the_deadline` parks `ensure`, advances `SimClock` past 30 s, and shows the launch still waiting; the deadline itself is proven only at Tier 3, where `sad_an_ensure_blocked_on_the_group_lock_times_out_as_a_command_failure` asserts the adapter returns the typed `TimedOut` within the bound and leaves the stalled `lvm` unkilled.
- **B-2 (RESOLVED, user ruling 2026-11-19): the `alive[a]` projection.** The recording `Vmm` double stays the arbiter; S-VOL-T3-13 is restored as the Tier-3 twin and the driver's `alive` read carries the `sim-arbiter-ok` marker citing it.
- **B-3 (RESOLVED): the composition test cannot assert the CLI-rendered `Waiting` line.** Accepted as a split: the rendered line is pinned by the CLI render tests (`overdrive-cli/tests/{acceptance,integration}/workload_describe*.rs`); the composition asserts the response's `capacity_wait`. No dev-dependency edge is added.

## Notes

- **N-5: guest-rootfs prerequisites of the `volume-pool` profile beyond the design's list (activation row 1).** The Tier-3 tests call `sfdisk` (ambiguous label), `flock` (the `ensure` deadline test),
  `mount`, `umount` and `dd`, and the journey tests a static busybox with `dd`/`head`/`printf` applets. T3-12 (stop during a held claim) and T3-09 (a replacement waits for the claim) build the suspended
  device on a loop device, so the guest also needs `losetup` and loop-device support, and the `loop` kernel module and its loader if it is not built into the guest kernel. They are guest content, not repository tooling; DELIVER's guest-kit step
  (row 1) stages them or the named tests are adjusted by DISTILL, not the crafter.
- **N-8: a fixture in `overdrive-core`.** `overdrive_core::testing::volume` (`volume_size_of_bytes`, `volume_size_gib`, `volume_size_tib`) is test
  support behind the existing `test-utils` feature, built on the pinned rkyv form of `VolumeSize`; it adds no constructor to the type and no
  production surface. If DELIVER pins a different archived form, the fixture is the one place to change (it fails with a message naming itself).
- **N-10: Tier-3 `volume-pool` tests are `#[ignore]`d.** The guest of that profile has no pool until the guest-kit volume-pool step (activation row 1,
  DELIVER's first step), so these tests cannot reach any assertion; `testing.md` allows `#[ignore]` for exactly that, and the reason names the
  resource. Every one of them already carries its `#[should_panic]` marker; that step removes each `#[ignore]` and proves the test RED on its marker.
- **N-12: `LvmVolumeBacking`'s `pub(crate)` constructor is not scaffolded.** The pinned shape is "the constructor is `pub(crate)`" but its
  parameters (the discovered node) are not pinned, and an unused constructor would fail `dead_code` under `-D warnings`. The struct has a private
  field, so no crate outside `overdrive-host` can build one; DELIVER adds the constructor with the probe body.
- **N-13: new test-support surface in the sim.** `SimVolumeBacking::fail_next_claim_state()` (one-shot: the next `claim_state` fails with
  `PoolCommand { operation: Claim }`) drives the F-3 tests; it is test support (DISTILL's to shape), not pinned API. `SimVolumePoolProbe::failing`
  replays its scripted error on every call (`io::Error` is not `Clone`; an `Io` detail keeps its kind and text).
- **N-14: the seeded invariants do not constrain a `claim_state` error.** `single_writer` is about a held claim, `data_survives_restart` about the
  volume's identity and `no_drain_failure_charges_budget` about a draining claim; none observes the outcome of an erroring `claim_state`, so a
  fourth fault in their schedules would be new scope. F-3 is covered by the two example tests.
- **N-16: the H2 test sits in the `nested-launch` guest, `kvm-tests`-gated**, because the base `cgroup-default` guest lacks the VM fixture's
  capability (`SystemdDetectVirtUnavailable`); the pool is the probe's simulated one. The test's recording `Vmm` is a decorator in the test over
  `SimVmm` (no production surface).
- **N-17: `serde_json` is a dev-dependency of `overdrive-reconcilers`** (and `Cargo.lock`) for the H3a test, which builds the workload from its
  wire spec. Test support only.
- **N-18: activation of the three DST invariants.** DELIVER's activation step adds `volume-single-writer`, `volume-data-survives-restart` and
  `volume-no-drain-failure-charges-budget` to `Invariant::ALL` and updates the catalogue count (the dst binary printed 43 at this revision; an earlier pass printed 44); `--only` and the reproduce line work
  then.
- **N-19: the invariant rig's determinism.** Each evaluator runs its rig on its own thread (current-thread runtime, one blocking thread) and
  settles by waiting for observable quiescence (64 consecutive unchanged steps, cap 4096, a clear panic at the cap; no fixed yield count), because the launch path's `tokio::fs` calls run on a blocking pool in real time. A node reboot
  restarts the control plane in the schedule. No production code changed.

## Proposals

**P-1 (approved 2026-11-17):** the `volume-pool` guest profile in `cgroup-default`; registry entry, count (23), guest-suite mapping onto the
`nested-launch` guest, `.claude/rules/testing.md` count. DELIVER adds the provisioning.

**P-2 (approved 2026-11-17):** `verification/expectations/E16-node-local-volume-survives-restart-and-reboot/` (claim only; a node reboot needs the
host owner's authorisation; surface E, run with `run-expectation.sh E16`; anchors VD-2, VD-8, VD-9; no evidence and no `satisfied` status) and its INDEX row. The directory was renamed from
`node-local-volume-survives-restart-and-reboot` to the `E16-` form.
