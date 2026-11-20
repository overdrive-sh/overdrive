# DESIGN wave decisions: node-local persistent volume (GH #348)

**Status (2026-11-19):** every decision below is approved by the user on the date in its row; ADR-0310 to ADR-0320 and ADR-0334 to ADR-0338 are
Accepted and not yet implemented, apart from the implemented ADR-0073 and ADR-0102, which carry amendment sections. The contracts are in
`../feature-delta.md`; each ADR holds one decision. Scope: all priorities. This file owns the decision table and the limitation table.

**Terms.** The **volume pool** is the node-level LVM setup every VM volume is carved from: the `overdrive-volumes` partition, the physical
volume and the `overdrive` volume group on it, plus the `lvm` tools and device-mapper. One **VM volume** is a single logical volume in the pool.
The pool is not one of Overdrive's internal stores (intent store, observation store).

## Decisions

| Id | Decision | Alternatives rejected | ADR | Approved |
|---|---|---|---|---|
| **VD-1** | A VM workload declares at most one volume: `[vm.volume] size = "40GiB"` (`GiB` or `TiB`, at least 1); `replicas = 1` required; part of the immutable spec (envelope V2). | `[[vm.volume]]` array; flat key; byte integer; `MiB` unit and granule rule; `replicas > 1`. | 0310 | 2026-11-17 |
| **VD-2** | The volume belongs to the workload (keyed by `WorkloadId`), is derived from the workload intent (no volume record, no volume verb), survives stop, restart and crash, and only withdrawing the workload intent deletes it. | Separate volume record and verbs; key on the allocation; delete on stop. | 0311 | 2026-11-17 |
| **VD-3** | Backing is a raw `dm-linear` logical volume in VG `overdrive` with extents reserved at creation, named `ovd-<32 hex of sha256(workload id)>`, outside the reflink filesystem. | File on XFS; thin LV; loop device; partition per volume; id in the name; a name registry. | 0312 | 2026-11-17 |
| **VD-4** | Node provisioning provides the volume pool: built-in `CONFIG_BLK_DEV_DM`, `CONFIG_EFI_PARTITION` and `CONFIG_DEVTMPFS`, the `lvm` tools with their library closure (one-shot, no daemon, no udev), a wiped GPT partition labelled `overdrive-volumes`, and on it a PV and the VG `overdrive` (4 MiB extents). Overdrive sets `LVM_SYSTEM_DIR=/run/lvm` on every `lvm` spawn and only verifies. Provisioning owners: `infra/metal/{partitions.json,provision.sh}`, `xtask/src/guest_kit_prepare.rs`, `crates/overdrive-testing/src/guest_kit.rs`, `crates/overdrive-testing/src/placement/groups.rs`; the future image carries the same requirement (GH #75). | Overdrive creates the PV/VG on a blank partition; XFS on an LV in the same group; hand-written dm tables; an empty `/etc/lvm` instead of the variable. | 0313 | 2026-11-17 |
| **VD-5** | A boot probe verifies the volume pool; an absent partition is soft, a present but broken pool refuses startup (`health.startup.refused`, `volume_backing.probe`). | Lazy probe; always refuse when absent; trust command exit status; probe kernel claim semantics every boot. | 0314 | 2026-11-17 |
| **VD-6** | The volume is the second `--disk`: `image_type=raw`, `direct=on`, write-back, `sparse=true`, `serial=ovd-volume`; Overdrive never formats it; write-through not offered; the platform chowns the device node to the confined uid. | Write-through; buffered I/O; default or per-volume serial; platform formats. | 0315 | 2026-11-17 |
| **VD-7a** | Fork change F1: a failed write-through fsync completes the guest's request with an I/O error on every engine. | Upstream-only; workaround in Overdrive; not exposing write-through. | 0316 | 2026-11-17 |
| **VD-7b** | Fork change F2: a writable block device is opened `O_EXCL` (the kernel claim). | Upstream-only; OFD lock alone (the alias hole, `run-031`); a second Overdrive claim path. | 0319 | 2026-11-17 |
| **VD-7c** | Fork change F3: an explicit image type skips format sniffing. | Upstream-only; workaround in Overdrive. | 0320 | 2026-11-17 |
| **VD-8** | A replacement's launch waits for the volume's kernel claim to clear, for at most 30 s, not for VMM process exit; past the bound, a typed volume start failure (cause `Held`), charged to the restart budget. | Process exit as "free"; budget-only retry; reconciler-side deferral; fd passing; a fork exit status; also taking the OFD lock. | 0317 | 2026-11-17 |
| **VD-9** | The launch path runs an idempotent `ensure` (create if absent, activate if inactive) before each launch; failures are typed start failures. No volume reconciler, status row or lifecycle gate in this slice. | Host-backed `volume-lifecycle` reconciler + `VolumeStatus` row + start gate; create at admission; create in the shim. | 0318 | 2026-11-17 |
| **VD-10** | Slice scope: create, attach, restart, stop, reboot and describe, all driven by `serve` + `deploy`. Reclamation, erasure, a status row, a start gate, multi-node pinning and growth are not in it; each distinct workload keeps its volume's space permanently; deletion's entry contract E1 to E3 is part of this decision. | Keep reclaim, zeroing, status row and gate here, Sim-only until #211. | none | 2026-11-17 |
| **SD-1** | A stop or restart commit raises a lane signal, opened by the convergence tick before hydration and raised only by `stop_workload` / `restart_workload` after their intent commit; ADR-0102's "never cancels an active effect" is amended. | Raise on any deferred evaluation (broker); drop the start future; poll the intent store. | 0334 | 2026-11-18 |
| **SD-2** | `LaunchSupersession` is a required parameter of `Driver::start` for every driver. | A default-provided second method; a task-local signal. | 0334 | 2026-11-18 |
| **SD-3** | A superseded launch returns `DriverError::StartSuperseded`; no `TransitionReason`, no row, no budget charge; the shim releases the supervision claim, network lease and SVID. | A `DriverStartFailure` class `Superseded`. | 0334 | 2026-11-18 |
| **SD-4** | The observation-point table for `VmDriver::start`; the volume steps run before the first host artifact; `ensure` and `Vmm::create` run to their end. | Interrupting `lvm` or the VMM spawn. | 0334 | 2026-11-18 |
| **SD-5** | Node shutdown also raises the signal: the lane table owns one shutdown token, every lane token is its child, the convergence loop's shutdown arm cancels it once; a launch superseded by shutdown is torn down through the same path and writes nothing. | Leave shutdown draining launches to their bound; list the lanes and cancel each; signal from `ServerHandle::shutdown` only; abort the tasks. | 0334 | 2026-11-18 |
| **SD-6** | The reconciler records a restart's generation as handled only once the allocation issued for it is **observed** (has a row). `observed_generation` is removed; the View records `issued_generation` (an input); `restart_pending` is derived from the observed current allocation and is false for a Job's row carrying a terminal claim (H1). A relaunch of an abandoned generation replacement charges nothing. Amends the implemented ADR-0073. | Keep the stamp on emit and clear it from the shim; carry the generation on `AllocStatusRow`; leave the defect; charge every unobserved id; leave a finished Job's generation pending. | 0335 (amends 0073) | 2026-11-18 (option A) |
| **SD-6b** | A failure before the driver runs (SVID issuance; an empty driver registry in a restart) is a `Failed` row with a typed reason (`MtlsInterceptInstallFailed { stage: "identity_issue" }`, `DriverInternalError`), charged and backed off like any start failure; this also changes first placements and crash restarts on `main`. | Leave pre-driver failures as dispatch errors; pace the pending path with a View counter; charge every unobserved id. | 0337 | 2026-11-18 (option B) |
| **J-1** | A Job's failure before the driver is final: the reconciler finalises any ended Job row (`workload_lifecycle.rs:1091-1113`, `is_natural_exit` `:1757-1775`, `classify_natural_exit_terminal` `:1781-1794`) and places a Job only while it has no row, so the Job ends `Failed { exit_code: Some(0) }` and is never relaunched, as a Job whose start is rejected is today. The `exit_code: Some(0)` is already misleading for `StartRejected`. | Relaunch a Job whose first launch failed before the driver. | 0337 | 2026-11-19 (option A) |
| **DE-1** | A non-start `DriverError` from `driver.start` is not a decision of this feature: `VmDriver::start` returns only `StartRejected` and `GuestTransportMissing` (a composition error placement never issues); the shim's `Err(other)` arm stays an internal typed error with no row. | A `Failed` row `DriverInternalError` for any non-start error. | 0337 | 2026-11-19 |
| **D-1** | An admission refusal by the guest-network pool writes no row and charges nothing, paced by the reconciler's occupancy check; the restart arm's predecessor cleanup runs. The wait is visible in `describe` (ADR-0336). | A charged `Failed` row; a View counter that paces refusals. | 0338 | 2026-11-18 (option A) |
| **H1** | `workload restart` of a finished Job (a Job row with a terminal claim) is a no-op, as today; the generation is not pending against it and no id is issued. The scope limitation is L9 (#301). | Leave the generation pending and let the shim fence return; re-run a finished Job. | 0335 | 2026-11-19 (option A) |
| **H2** | The waiting state is derived by one pure predicate, `start_blocked_only_by_capacity`, in `overdrive-reconcilers`, built from the reconciler's own helpers; "generation pending" is read in the handler from the committed generation and `ReconcilerRuntime::view_for_workload_lifecycle`. | A handler-local conjunct list; excluding issued ids that hold a lease from the over-report window. | 0336 | 2026-11-19 (option A) |
| **W-1** | `describe` shows the live held count and the cap, with no start time and no history (L6, #265). | "Since" only where an input exists; a durable condition row; a shim- or reconciler-authored marker; the log event alone. | 0336 | 2026-11-18 |
| **W-2** | Guest-network capacity only; `CapacityResource` stays `#[non_exhaustive]` with one variant (L7, #261). | Include CPU and memory now. | 0336 | 2026-11-18 |
| **W-3** | The fill-the-pool test uses the smallest in-process production composition: a test-owned real `GuestAddressPool` filled through its own admission, `SimDriver` starts; no production surface. A test-composition decision, owned by DISTILL (`distill/upstream-issues.md`). | A configurable cap on `serve`; 16,384 real attachments on the metal host. | none | 2026-11-18 |
| **W-4** | The wait is shown only in `describe`; `workload list` does not exist and is its own feature (L8, #367). | Build `workload list` in this feature. | 0336 | 2026-11-18 (option 2) |
| **R-1** | The restart-generation model carries production's per-allocation restart count `cnt` and derives `charged` and `exhausted`; `LIMIT` = the production ceiling + 1. | A running count with a saturation, with `exhausted` model-only. | none (model) | 2026-11-18 (option A) |
| **VB-1** | Validation-budget growth: `vol-safety-tlc` (~271 s) is the only check outside CI (`ci = false`, `timeout_secs`); `rg-safety-tlc` (~65 s) and `rg-live-replacement` (~29 s) are `ci = true` under the two-minute rule. Figures are in each spec's `evidence/summary.md`. Any further growth returns to the user. | A bound change (`DGMAX = 1`, no `capFull`, `NA = 3`). | none (model) | 2026-11-18 (option A) |
| **P-1** | `volume-pool` guest profile in `cgroup-default`. | none | none (test infrastructure) | 2026-11-17 |
| **P-2** | EDD expectation `E16-node-local-volume-survives-restart-and-reboot`. | none | none (test infrastructure) | 2026-11-17 |
| **P-3** | One real-kernel failed-flush Tier-3 test, S-VOL-T3-11. | none | none (test infrastructure) | 2026-11-18 |
| **P-3a** | S-VOL-T3-11 proves what it can: in the production write-back attachment (ADR-0315) a failed host flush reaches the guest's `fsync` as an error, the durability guarantee of the default mode; it holds on stock Cloud Hypervisor on every engine (spike `run-022`, `run-030`, `a3_fault_wb_explicit_flush_*`, `errs_after_injection=20`) and does not guard F1. F1 changes only the write-through path, which the spike reached only with host page caching on (no stock dm target yields a succeeding write then a failing flush under direct I/O). F1's regression test lands in the fork (`overdrive-sh/cloud-hypervisor`) with the F1 patch: a write-through request whose post-write fsync fails completes with `VIRTIO_BLK_S_IOERR` on every engine, mirroring `run-030`'s cells. It is an obligation of the DELIVER step that lands the fork patches and the pin and checksum bump (ADR-0316, ADR-0319, ADR-0320, ADR-0161), and is the discharging item for F1 (validation V-3). | Keep S-VOL-T3-11 as F1's guard (it passes with F1 reverted); a host-page-cache fixture in Overdrive's tests for F1 (a configuration production never attaches). | 0316 (Consequences only) | 2026-11-19 (option A) |
| **B-1** | The 30 s `ensure` deadline is enforced by the `VolumeBacking` adapter, not the driver: `ensure` returns within 30 s, at the deadline as `PoolCommand { operation in progress, TimedOut }`, an in-flight `lvm` command running to its own end and never aborted; the driver awaits `ensure` and holds no timer. A contract detail inside VD-9 and SD-4 that changes no approved behaviour; pinned in the feature delta § Port. | Driver bounds `ensure` with its `Clock`: it would have to drop the future mid-effect or detach it, and cannot name the operation in progress. | none (inside 0318, 0334) | 2026-11-19 (architect pin) |
| **B-2** | The volume conformance driver's `alive[a]` projection keeps the recording `Vmm` double as its arbiter, paired with the restored Tier-3 test S-VOL-T3-13 (a stop during guest boot terminates the VMM and keeps the volume). | A projection with no named arbiter. | none | 2026-11-19 (user ruling) |
| **B-3** | The capacity-wait composition test asserts the response's `capacity_wait`; the rendered `Waiting` line is pinned by the `overdrive-cli` render tests. No `overdrive-cli` dev-dependency in the control plane. | A control-plane test that renders through the CLI. | none | 2026-11-19 (user ruling) |

Placement pinning, growth and the relation to #97 bind no code in this slice: answers 6 to 8 of the feature delta, with the directions carried by
L2 and L3.

## Scope limitations and their tracking issues

| # | Limitation | Issue |
|---|---|---|
| L1 | one volume per workload | #364 |
| L2 | size fixed at creation | #365 |
| L3 | pinned only implicitly (single node); no multi-node pin or volume-space-aware placement | #366 (#134, #261, #282 do not cover it) |
| L4 | nothing deletes or erases a volume until workload deletion exists; a volume's state is visible only through allocation rows | #211, with the comment carrying the entry contract E1 to E3 (https://github.com/overdrive-sh/overdrive/issues/211#issuecomment-6955577132) |
| L5 | no appliance image exists yet; the future image must define the wiped partition, tools, built-in dm and VG (the metal host and guest-kit guests are provisioned instead) | #75, with the comment listing the image requirements (https://github.com/overdrive-sh/overdrive/issues/75#issuecomment-6955578007); not a precondition |
| L6 | the waiting state has no start time and no history | #265 (https://github.com/overdrive-sh/overdrive/issues/265#issuecomment-6988095924) |
| L7 | the waiting state covers guest-network capacity only; CPU and memory waits are not shown | #261 (https://github.com/overdrive-sh/overdrive/issues/261#issuecomment-6988096862) |
| L8 | the waiting state is visible only in `describe`; `workload list` does not exist | #367 (https://github.com/overdrive-sh/overdrive/issues/367) |
| L9 | `workload restart` of a finished or stopped Job is a no-op (H1, ADR-0335); this feature keeps today's behaviour and #301 changes it | #301 (https://github.com/overdrive-sh/overdrive/issues/301) |

Not limitations (properties of the mechanism or of #348's stated scope; no issue): single writer, so `replicas = 1`; the data is lost with the
node; the guest owns the filesystem on a raw device; no snapshots, backups, replication or sharing (#97, #43, #96); `[vm.volume]` is a property
of VM workloads by definition; the volume group name, 4 MiB extent size, partition label and serial are the provisioning and attachment contract,
a closed set chosen for correctness; the 30 s claim bound and 30 s `ensure` deadline are policy values derived from the measured drain (2.4 s)
and the existing boot wait.

## Validation status

- **V-1 PASSED** (contract-shaping; native metal host, Linux 7.0.0-38): evidence `spike/findings.md` § "V-1", `spike/evidence/v1-001` to
  `v1-011`; absorbed into the feature delta § Port and ADR-0313.
- **Quint:** two specs, `specs/quint/node-local-volume/` and `specs/quint/restart-generation/`. Verdicts are in each spec's
  `evidence/summary.md` and `cargo xtask quint verify-evidence`; this file states none. Both specs are re-recorded against the design of
  2026-11-19 (the slimmer restart-generation model, one hazard each for ADR-0337 and ADR-0338, assumption RA11). `vol-safety-tlc` is the only
  check outside CI (VB-1); `rg-safety-tlc` and `rg-live-replacement` run in CI under the two-minute rule.
- **Removed invariants (user ruling 2026-11-19, "remove checks that cannot fail"):** `I_TYPED_FAILURE_NEVER_ATTACHES` and
  `I_SHUTDOWN_SIGNALS_EVERY_LAUNCH` are structurally unviolable and asserted by named runs. `I_SINGLE_WRITER` and every invariant a hazard
  violates stay. That "a typed failure never attaches" is asserted by the runs `no_space_fails_typed_and_never_attaches`,
  `launch_bound_exceeded_fails_typed_and_charges_budget` and `store_absent_fails_typed_and_never_attaches`; assumption RA6 and gate G-2 cite
  those runs.
- **Open:** none that is contract-shaping.
