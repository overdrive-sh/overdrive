# Feature delta: node-local persistent volume for VM workloads (GH #348)

**Status:** the design is approved by the user; rulings run through 2026-11-23. The decision table (VD-1 to VD-10, SD-1 to SD-6, J-1, DE-1, D-1,
H1, H2, W-1 to W-4, R-1, VB-1, VB-2, RB-1, B-1 to B-3, P-1 to P-3) and the scope-limitation table (L1 to L9) are owned by
[`design/wave-decisions.md`](design/wave-decisions.md); each ADR (0310 to 0320, 0334 to 0338, 0348) holds one decision. This file owns the interface
contracts, the boundary scenarios and the model's boundary. Executable test obligations, mutation targets and test re-sync lists are DISTILL's
input in [`distill/upstream-issues.md`](distill/upstream-issues.md). The model is two Quint specs, `specs/quint/node-local-volume/` and
`specs/quint/restart-generation/`; their verdicts are in each spec's `evidence/summary.md`, not in this document.

**Terms.** The **volume pool** is the node-level LVM setup every VM volume is carved from: the `overdrive-volumes` partition, the physical
volume and the `overdrive` volume group on it, plus the `lvm` tools and device-mapper. One **VM volume** is a single logical volume in the pool.
The pool is not one of Overdrive's internal stores (intent store, observation store).

Evidence: `spike/findings.md` (including § "V-1. LVM on a minimal node without udev"), `spike/wave-decisions.md` (native metal host, Linux
7.0.0-38, Cloud Hypervisor fork `f252bb20b` plus spike patches F1 to F3; since 2026-11-20 the fork is upstream `main` plus our commits, and F1 and F3 are
upstream's own behaviour, so only F2 remains a fork change, ADR-0161 § Amendment). Research:
`docs/research/platform/vm-persistent-volume-block-path-and-backing-research.md`, `vm-volume-copy-on-write-clone-prior-art-research.md`,
`docs/research/orchestration/stop-during-start-comprehensive-research.md`.

## [REF] Slice and decision index

**Slice (VD-10).** This feature delivers the loop that `overdrive deploy` can drive today: declare a volume, create it, attach it, survive crash,
`workload restart`, stop, node reboot, and see it in `describe`. Workload deletion does not exist (GH #211), so reclamation, erasure and a
volume status row have no production trigger; they are an entry contract for the #211 slice (§ Entry contract). Two consequences the user ruled
on knowingly: (a) the slice is drivable through `overdrive serve` + `overdrive deploy` wherever the node's provisioning provides the volume pool
(today the metal test host and guest-kit test guests; no appliance image exists yet, GH #75, L5, not a precondition); (b) with no deletion, each
distinct workload id keeps its volume's space permanently, even after stop; the only remedy on an operator-less node is re-provisioning.

The decisions are listed in [`design/wave-decisions.md`](design/wave-decisions.md).

## [REF] Answers to the issue's DESIGN questions

1. **Spec surface:** `[vm.volume] size = "40GiB"`, one volume (VD-1).
2. **Identity and ownership:** keyed by `WorkloadId`; desired state is the workload intent; no separate record (VD-2).
3. **Lifecycle:** create-or-activate at launch, converging each time (VD-9); attach by launching the VMM with the second disk (VD-6); detach is
   VMM exit, the claim draining (VD-8); delete and garbage collection are the #211 slice's.
4. **Single writer:** kernel `O_EXCL` claim (fork F2, VD-7b) and Cloud Hypervisor's OFD lock; the replacement waits for the claim, not the
   process (VD-8).
5. **Backing and placement:** a `dm-linear` logical volume in VG `overdrive` on a dedicated labelled partition, outside `<data_dir>` and every
   clone surface (VD-3, VD-4).
6. **Placement pinning:** one node holds the volume; with one node (`local`) it is implicit (L3). The direction for the multi-node slice is an
   intent binding holding the `NodeId`, written once by an atomic put-if-absent at first placement and a hard scheduler constraint, not an
   observation row and not pinned at submission.
7. **Growth:** grow-only, only while detached (extend under the claim, relaunch); not delivered because the spec is immutable per id (L2).
8. **Relation to #97:** the node-local volume stays the local tier; an object-backed volume would be a new explicit declaration, never a silent
   change to `[vm.volume]`.

Answers 6 to 8 bind no code in this slice; L2 and L3 carry them.

## [REF] Reuse analysis

Every overlapping component, with contract shape, universe and the crafter's assertion mechanism.

| Component | Disposition | Contract shape | Universe | Assertion mechanism |
|---|---|---|---|---|
| `VmDriver::provision_vmm` / `Vmm::create` | Extend: `ensure`, claim wait, second `--disk`, node chown | bounded-change | the volume's logical volume, its device node's owner and mode, the VMM argv; nothing else | argv golden test through the single `DiskAttachment` render site; `stat` of the node before and after; `SimVolumeBacking` records its calls |
| `DiskAttachment::to_disk_arg` | Extend: `serial`, `direct` | pure-function | none | unit + mutation target (already "the one site") |
| `VmReclamation` / `VmHostState` | Unchanged; volumes are not on its clone, run-dir or scope surfaces | pure-function plan | n/a | acceptance test: a reclamation pass leaves the volume untouched |
| `WorkloadLifecycle` | Unchanged except the three `VmPayload` projection sites | pure-function `reconcile` | none | existing ESR/DST invariants |
| `action_shim` `StartAllocation` / `RestartAllocation` pre-driver failure arms (SVID issuance; empty registry in restart) | Extend: write a typed `Failed` row, release the lease, return a failed teardown after the row (§ Launch outcomes) | bounded-change | the failed id's one allocation row and its guest lease; nothing else | shim tests at the driven ports; the seeded invariant's pre-driver fault dimension |
| `WorkloadLifecycle` restart-pending rule (`generation_replacement_pending`) and the capacity predicate `start_blocked_only_by_capacity` | New helper (replaces three inline expressions) and new predicate, both pure and in `overdrive-reconcilers`, beside `current_alloc` and `restart_after_failure` | pure-function | none | proptests over `(rows, view, generation)`; the agreement properties of the waiting-state section |
| `compose_vm_driver` / `run_server` | Extend: compose and probe the volume pool first, pass it to the driver | bounded-change | boot events, driver registry | boot test with a broken substrate |
| `prepare_confined_paths` | Extend: chown the volume node | bounded-change | that node only; no operator artifact's mode widens | `stat` assertions |
| `AllocStatusResponse`, `render::workload_describe` | Extend: volume size, capacity wait | pure-function | none | render golden tests |
| `infra/metal/{partitions.json,provision.sh}` | Extend in DELIVER: the metal host's volume pool | bounded-change | the test host's data disk | provisioning self-test |
| `xtask/src/guest_kit_prepare.rs`, `crates/overdrive-testing/src/guest_kit.rs`, `crates/overdrive-testing/src/placement/groups.rs` | Extend in DELIVER: a volume pool for guest-kit test guests (§ Provisioning obligation) | bounded-change | the guest's own backing disk and staged rootfs | provisioning self-test inside a guest; a guest without the pool composes none |

No existing alternative covers volume creation or claim handling: there is no `lvm`, `dmsetup` or `device-mapper` use anywhere in `crates/`,
`infra/` or `xtask/`.

## [REF] End-to-end flow (production entry points)

All through `overdrive serve` and `overdrive deploy`; no test-only wiring.

1. **Boot.** `run_server` composes the volume pool before the VM driver: find the partition labelled `overdrive-volumes` (kernel `PARTNAME`
   attribute); absent: soft `volume_backing.not_composed`; present: `probe`; failure refuses startup with `health.startup.refused` reason
   `volume_backing.probe`. The VM driver is composed with the volume pool.
2. **Deploy.** `overdrive deploy dev-box.toml` with `[vm.volume] size = "40GiB"`. `submit_workload` validates (grammar, `replicas = 1`),
   archives the V2 envelope, writes the intent.
3. **Launch (first allocation).** `WorkloadLifecycle` places the allocation as for any VM workload. `VmDriver::start` calls `ensure` (creates the
   40 GiB logical volume, reserving extents, and activates it), waits for the claim to be free (immediately free the first time), then the VMM
   adapter chowns the node to the confined uid and launches Cloud Hypervisor with the second `--disk`. The guest formats and mounts it.
4. **Crash, `workload restart`.** The old VMM dies (terminal row); the successor is minted (ADR-0104/0109); its launch runs `ensure` (activates
   the existing volume), waits up to 30 s for the claim to clear (ADR-0317), then attaches the same volume with its data.
5. **Stop.** `workload stop` ends the allocation; the volume stays; a resumed or restarted workload attaches it again.
6. **Node reboot.** Volumes are inactive after boot; the next launch's `ensure` activates the volume.
7. **Describe.** `overdrive workload describe <id>` shows the volume size; a volume failure shows as the allocation's typed reason.

## [REF] Interface contracts

Exact public and cross-crate contracts. Internal structure (private functions, module layout, the shell-out helper and report parsing) is the
crafter's.

### Spec surface (VD-1)

`overdrive-core/src/aggregate/`: `Vm` gains `volume: Option<VmVolume>`; `VmVolume { size: VolumeSize }`. `VolumeSize` is a validated newtype of
bytes with `FromStr`/`Display`/serde matching `<uint><unit>`, unit case-insensitive `GiB|TiB`, value at least 1; `Display` emits `GiB` unless the
value is an exact `TiB` multiple. Both parser-side `VmInput`s gain `volume: Option<VmVolumeInput { size: String }>` (`deny_unknown_fields`); TOML
is `[vm.volume] size = "40GiB"`. Validation in `validate_driver_input`: `AggregateError::Validation { field: "vm.volume.size", .. }` for grammar;
`field: "vm.volume", message` "a volume requires replicas = 1" when the workload's `replicas != 1`. The rule applies to every kind that carries
`replicas` (Job and Service); a Schedule is rejected at submit today, so a Schedule with a volume is rejected before this rule.
`ServiceSpecOutput` / `JobSpecInput` echo the declaration. `DriverPayload::Vm(VmPayload)` gains `volume: Option<VolumePayload { workload:
WorkloadId, size: VolumeSize }>`. The one projection from intent to payload is `Vm::to_payload(&self, workload: &WorkloadId) -> VmPayload` (pure,
`overdrive-core`): it converts `kernel` and `rootfs` to paths and sets `volume` to `Some(VolumePayload { workload: workload.clone(), size })` when
`Vm.volume` is set. The three projection sites in `WorkloadLifecycle` (the start action, the restart action, and the third site at
`workload_lifecycle.rs:2126`) each become `DriverPayload::Vm(vm.to_payload(&job.id))`, so they cannot diverge; no site destructures the `Vm`
fields. Every other construction of `VmPayload` (test fixtures, the cid-lease fixture) states `volume: None`; nothing but `to_payload` sets `Some`.

Persisted formats: (1) a `WorkloadIntentEnvelope::V2` bump with `JobV2`/`ServiceV2`/`ScheduleV2`/`VmV2` payloads and a `ServiceSpecEnvelope` V2
per ADR-0048; the V1 fixtures are re-pinned as an explicit frozen V1 (built directly as `Envelope::V1(<frozen V1 payload>)`, never through
`latest()` or the aliases, which now produce V2), as `.claude/rules/development.md` § "rkyv schema evolution" requires when a larger V2 variant
is appended, `known_discriminants` becomes `[0, 1]`, and `FIXTURE_V2` is added in the same commit; a stored V1 workload re-archives to different
bytes so its `spec_digest` changes, and under the single-cut greenfield policy the upgrade path is deleting the data directory (§ exposures). (2)
The new `TransitionReason` variants are appended at the end of the enum, which `AllocStatusRowV1` archives; if the archived size of
`AllocStatusRowV1` changes, the V1 fixtures are re-pinned as an explicit frozen V1 (same rule) and `discriminant_offset_from_end` is re-pinned in
the same commit (no new row envelope version: struct-literal call sites stay).

### Port: `VolumeBacking` (VD-3, VD-4, VD-5, VD-8, VD-9)

`overdrive-core/src/traits/volume_backing.rs`.

```
#[async_trait]
pub trait VolumeBacking: Send + Sync {
    async fn ensure(&self, workload: &WorkloadId, size: VolumeSize) -> Result<PathBuf, VolumeBackingError>;
    async fn claim_state(&self, workload: &WorkloadId) -> Result<ClaimState, VolumeBackingError>;
}
pub enum ClaimState { Free, Held }

#[async_trait]
pub trait VolumePoolProbe: Send + Sync {
    async fn probe(&self) -> Result<Option<Arc<dyn VolumeBacking>>, VolumeProbeError>;
}
```

`VolumePoolProbe` is the driven port the composition root calls to discover and probe the pool (§ Probe); the root must receive every driven
adapter from its caller (production wires the host adapter, tests wire the sim twin). A probe returns the usable `VolumeBacking` it has just
probed, so there is no step between "probed" and "usable". `probe` is `async`: it spawns `lvm` and reads sysfs, and neither blocks the runtime.

Behavioural contract (the trait docstrings pin these):

- `ensure` (bounded-change: creates or activates only `ovd-<hash(workload)>` in VG `overdrive`; returns the activated device path). An existing
  volume: activated, returned; its size is not compared. A missing volume with enough free extents: created with exactly `size` reserved
  (nothing is written and it is never formatted; LVM's default of zeroing the first 4 KiB is not relied on, the provisioning wipe is, ADR-0313),
  activated, returned. A volume that exists but is inactive (the state after every node reboot, V-1) is activated: creation and activation are
  separate observations, never assumed from each other. An `lvcreate` that reports the volume already exists (an earlier attempt's command
  outlived its deadline and finished) is treated as an existing volume. Not enough free extents: `InsufficientSpace { needed, free }`, nothing
  created; `free` comes from the free extents observed before the create (the `lvm` report), not from mapping an exit status, since a filter
  refusal also exits 5. (The no-pool case is not an `ensure` outcome: a node with no pool has no `VolumeBacking`, § Launch path.) A command
  failure, or the whole `ensure` outliving its one deadline (30 s): `PoolCommand { operation, source }`; a spawned command that outlives the
  deadline runs to its own end and is never aborted (`rust.md`). Idempotent and safe to repeat after a crash.
- **The `ensure` deadline: the adapter enforces it, the driver does not (B-1, 2026-11-19; pinned inside VD-9 and SD-4, no behaviour changes).**
  The trait docstring pins these properties:
  - *Bound.* `ensure` returns, `Ok` or `Err`, within 30 s of the call, whatever the pool does. The 30 s is one deadline for the whole call,
    covering every `lvm` spawn it makes (report, create, activate); it is not a per-command timeout, and it is a private policy value of the
    adapter, not a parameter of the port.
  - *Owner.* The `VolumeBacking` adapter starts the deadline when `ensure` is entered, on the adapter's own timer. `LvmVolumeBacking` is
    `adapter-host`, where a real timer is allowed; the driver passes no deadline and holds no timer on `ensure`. The driver awaits `ensure` to its
    return and never races it against its `Clock` or drops it mid-call.
  - *At the deadline.* `ensure` returns at that instant `Err(PoolCommand { operation, source })`, `operation` the `lvm` command in flight (or,
    when the deadline passes between two commands, the next command it would have run) and `source.kind() == io::ErrorKind::TimedOut`. It spawns
    no further command after the deadline. A command in flight is not killed, aborted or awaited: its child process runs to its own end, reaped
    by a task the adapter owns, and its exit status and output are discarded, never acted on or retried by that call. A deadline return is not
    `NotFound` or `InsufficientSpace`; the caller learns nothing about whether the volume was created or activated.
  - *After the deadline.* The abandoned command may still create or activate the volume; nothing the call returned depends on it. The next
    `ensure` observes the volume afresh (an existing volume, an `lvcreate` that reports it exists, an inactive volume that is activated) and,
    if it finds `lvm`'s lock still held by the abandoned command, spends its own deadline waiting for it.
  - *Why the adapter.* (1) The error names the operation in progress; only the adapter knows it, so a driver-side bound could not produce the
    typed `PoolCommand`. (2) A driver cannot bound a call it awaits without dropping the future mid-effect or detaching it, and
    `reconcilers.md` § "Superseded in-flight work runs to its bound" forbids the first. (3) `ensure` returning by itself within 30 s is what
    makes the stop-latency bound of § Superseding a launch hold for every `VolumeBacking`, not only the host one.
  - *The sim.* `SimVolumeBacking` holds no timer. Its deadline is a scripted outcome: an `ensure` scripted to end at its deadline returns the
    `TimedOut` `PoolCommand` for a named operation, and the sim can park an `ensure` in flight until the test releases it, so a test raises the
    signal while `ensure` is running and then lets it return an `Ok` or the deadline `Err`. Whether the abandoned command goes on to create the
    volume is a sim script too. Exact sim methods are DISTILL's.
- **The device path** `ensure` returns, `claim_state` opens, and `DiskAttachment::volume` renders is the device-mapper node of the volume:
  `/dev/mapper/` followed by the dm name, and the dm name is `<vg>-<lv>` with every `-` inside `<vg>` and inside `<lv>` doubled. For VG
  `overdrive` and LV `ovd-<hash>` that is `/dev/mapper/overdrive-ovd--<hash>`. `<hash>` is the first 32 lowercase hexadecimal characters of the
  SHA-256 of the workload id (ADR-0312) and contains no `-`, so only the `-` of the `ovd-` prefix is doubled. (A scratch volume `ovd-probe-<x>`
  maps the same way.) The convenience symlink `/dev/overdrive/ovd-<hash>` is not used. Grounding: the V-1 capture (`spike/findings.md` § V-1.4,
  `v1-008`) shows LV `ovd-test` in VG `overdrive` as `/dev/mapper/overdrive-ovd--test`; the real-name node is asserted in Tier 3 (S-VOL-T3-04).
  The Quint specs name no device path.
- `claim_state` (no persistent effect, returns without blocking): makes one non-blocking attempt to take the volume's exclusive claim (an
  `O_EXCL` open of its device-mapper node, the path above) and releases it; `Free` iff it took it, `Held` iff the open fails `EBUSY`. An absent
  volume **or an inactive one** (no device node, so `ENOENT`, as after every node reboot): `Err(VolumeBackingError::NotFound)`; `NotFound` does
  not distinguish "no such volume" from "present but inactive" (§ Observable projections gives the conformance rule). It is the authentic arbiter
  of the kernel's claim for the conformance projection. It does not take the OFD lock: Cloud Hypervisor opens the disk once
  (`vendors/cloud-hypervisor/block/src/factory.rs:85-93 @ f252bb20b`) and takes the lock on that descriptor
  (`vendors/cloud-hypervisor/virtio-devices/src/block.rs:1026`), so the lock and the claim belong to one open file description; the kernel removes
  the lock before it releases the device, so a free claim implies a free lock (ADR-0317). Any other `open(2)` failure is `Err(PoolCommand {
  operation: Claim, source })`: `Claim` names the `open(2)` of the device node, not an `lvm` command.
- Nothing else is a method: the launch path owns the bounded wait (below).

`VolumeBackingError` is `#[non_exhaustive]`: `NotFound { workload }`, `InsufficientSpace { needed: VolumeSize, free: ByteCount }`,
`PoolCommand { operation: VolumeOperation, source: io::Error }`.

`overdrive-core/src/volume.rs` pins the value types the errors carry:

- `VolumeOperation` (`#[non_exhaustive]`, `Debug Clone Copy PartialEq Eq Hash`, serde kebab-case matching `as_str`, `ToSchema`, rkyv, because
  `VolumeUnavailableCause::PoolCommandFailed` carries it): `Report` (`"report"`, an `lvs`/`vgs` read), `Create` (`"create"`, `lvcreate`),
  `Activate` (`"activate"`, `lvchange -ay`), `Claim` (`"claim"`, the `O_EXCL` open of the device node in `claim_state`, for a failure other than
  `EBUSY` and `ENOENT`). `as_str` is enum-owned. When the `ensure` deadline passes, the operation in progress at that moment is the one
  reported, with `source` of kind `TimedOut`. The boot probe reports through `VolumeProbeScenario`.
- `ByteCount`: a validated newtype of a byte count, any `u64` including 0 (free space can be 0 and is not a GiB multiple, so `VolumeSize`
  cannot carry it). `ByteCount::new(bytes: u64) -> Self` is total; `get(self) -> u64`; `Display` emits `<n>B`; `FromStr` parses `<uint>B` (the
  `b` unit is case-insensitive; a missing unit or any other unit is a `ByteCountParseError`); serde is the `Display`/`FromStr` string; rkyv
  archives the `u64`. `VolumeUnavailableCause::InsufficientSpace.free` and `VolumeBackingError::InsufficientSpace.free` carry it, and the
  OpenAPI schema for `free` is a string.
- `VolumeSize::bytes(self) -> u64`: the one accessor the host adapter and the sim read the size through.

`overdrive-host`: `LvmVolumeBacking`, the only place that spawns `lvm`, with a `pub(crate)` constructor so that only `LvmVolumePoolProbe`
constructs one; `LvmVolumePoolProbe` (unit-like, `new()` infallible and kernel-silent, implements `VolumePoolProbe`) discovers the labelled
partition, probes, and constructs the one `LvmVolumeBacking` over the discovered node. Every spawn is `lvm <subcommand> --config '<pinned>' ...`
with `LVM_SYSTEM_DIR=/run/lvm` in the spawn's environment (V-1; ADR-0313). The pinned `--config` is `activation{udev_sync=0 udev_rules=0
monitoring=0} devices{filter=["a|^<labelled partition node>$|","r|.*|"] use_devicesfile=0} backup{backup=0 archive=0} global{use_lvmpolld=0
locking_dir="/run/lock/lvm"}`, the filter built from the node discovered by `PARTNAME`. Exit status maps to typed errors: 0 is success (stderr
warnings accompany metadata writes and are never read as failure); 5 is a filter refusal or insufficient free space (the `InsufficientSpace`
source); 4 is a missing system directory (a bug of this contract); any other non-zero is `PoolCommand`. When `lvcreate` returns 0 its device node
already exists, so no polling is needed.

`overdrive-sim`: `SimVolumeBacking`, with fault injection (claim held until a `drain` step, free-space exhaustion, command failure); it is the
named arbiter for `claim_state` in conformance. `SimVolumePoolProbe` is its probe twin: `composed(SimVolumeBacking)` returns `Some(Arc<dyn
VolumeBacking>)` over that backing, `absent()` returns `None`, and `failing(VolumeProbeError)` returns the error. The sim's inspection and
fault-injection methods beyond those named here are test-support, DISTILL's to shape. A node with no pool has one route to `VolumeUnavailable {
PoolAbsent }`: `VmDriver` holds `volumes: None`, because the probe returned `None` (the model's `StoreAbsentBoot` maps to composing
`SimVolumePoolProbe::absent()`). A backing never returns a "pool absent" error: it exists only after a probe found a pool.

### Probe (VD-5)

`VolumePoolProbe::probe(&self) -> Result<Option<Arc<dyn VolumeBacking>>, VolumeProbeError>` (`async`; the host implementation is
`LvmVolumePoolProbe`). Contract (the trait docstring pins it):

- **Preconditions:** none beyond a constructed probe; calling it does not require the node to have a pool.
- **`Ok(None)`** iff no partition has the label `overdrive-volumes`. Nothing is created or removed.
- **`Ok(Some(backing))`** iff exactly one partition has the label and every probe scenario passed. The backing is usable at once and addresses
  that partition's volume group `overdrive`.
- **`Err(VolumeProbeError)`** iff a labelled partition is present (or the label is ambiguous) and a scenario failed. No backing is returned.
- **Effects:** it may create and remove one scratch volume named with the prefix `ovd-probe-`, and removes a leftover one first; it touches no
  other volume. On any exit, including `Err`, it leaves no scratch volume it created, except `ScratchRemoveFailed` and a deadline return (below),
  where an abandoned command may still create the scratch volume or fail to remove it.
- **Repeatable:** safe to call again after a crash or an `Err`; a second call after an `Ok` is a fresh probe with the same result on an unchanged
  node. A scratch volume left by an abandoned command is removed first by the next probe.
- **Bound (P-4, 2026-11-21).** `probe` returns, `Ok` or `Err`, within 30 s of the call, whatever `lvm` does; the same 30 s as the `ensure`
  deadline (B-1), and likewise a private policy value of the adapter, not a parameter of the port.
  - *One deadline for the whole call,* not per command: it starts when `probe` is entered, on the adapter's own timer (`LvmVolumePoolProbe` is
    `adapter-host`), and covers every `lvm` spawn the probe makes (`pvs`, `vgs`, `lvs`, the scratch `lvcreate` and `lvremove`), mirroring
    `ensure`. The sysfs partition discovery is a local read that does not wait on `lvm` and is not itself bounded.
  - *At the deadline* `probe` returns at that instant `Err(VolumeProbeError { scenario, detail: Io(source) })`, `source.kind() ==
    io::ErrorKind::TimedOut` (no new `VolumeProbeDetail` variant; this mirrors `PoolCommand { TimedOut }`), and `scenario` is the scenario the
    command in flight reports on any failure (when the deadline passes between two commands, the scenario of the next command it would have run).
    It spawns no further command after the deadline. A command in flight is not killed, aborted or awaited: its child runs to its own end, reaped
    by a task the adapter owns, and its status and output are discarded (`rust.md`).
  - *Operator-visible:* a deadline return refuses startup exactly as any probe `Err` does (VD-5): `health.startup.refused`, reason
    `volume_backing.probe`, `ControlPlaneError::VolumeBackingBoot`, the error naming the scenario and a `TimedOut` source. No fallback runs:
    `overdrive serve` never waits on a hung `lvm`, and the node boots again on the appliance's next start.
  - *The sim.* `SimVolumePoolProbe::failing(VolumeProbeError)` already returns any scripted error, including a `TimedOut` one; no timer in the sim.

`VolumeProbeError { scenario: VolumeProbeScenario, detail: VolumeProbeDetail }` (`thiserror`; `detail` is the error's `source`, never a flattened
`String`). `VolumeProbeDetail` is `#[non_exhaustive]`: `Io(io::Error)`, `CommandFailed { exit_status: Option<i32>, stderr: String }` (diagnostic
text only, never parsed), `AmbiguousLabel { nodes: Vec<PathBuf> }`, `NodeMissing { path: PathBuf }`. `VolumeProbeScenario` (`as_str`, in this
order; neither probe enum is serialised, archived or in the OpenAPI schema, so adding a variant has no persisted or wire effect):
`SysfsUnreadable` (P-5, 2026-11-21: reading `/sys/class/block` to discover the partition labelled by `PARTNAME` failed, detail `Io`; it is the
first stage of the probe and is not an `lvm` or device-mapper failure), `ToolsOrDeviceMapperUnusable` (the `lvm` tools or device-mapper),
`VolumeGroupMissing` (including a group not on the labelled partition, and a label matched by more than one partition, detail `AmbiguousLabel`;
it also reports a failing `lvs` in the leftover-scratch sweep and an unparseable `vg_free`, which arise only when the volume group itself is
broken), `ScratchCreateOrNodeFailed`, `ScratchRemoveFailed`. The probe removes a scratch volume left by a crashed
probe first (`lvremove -f` removes it in either state, V-1) and skips the scratch step with an event when the pool has no free extent. It touches
no volume that is not a scratch volume; a workload volume left inactive by a reboot is the launch path's `ensure` to activate. "Probed before
use" rests on two facts, not on a witness type: `compose_volume_backing` is the only caller of `VolumePoolProbe::probe`, and `run_server` is the
only production path into `VmDriver::new`; the `LvmVolumeBacking` constructor is `pub(crate)`, so only `LvmVolumePoolProbe` builds one, after the
scenarios pass. The boot uses the probe result exactly once, as three outcomes: `Some` moves into `compose_vm_driver` and from there into
`VmDriver::new`; `None` is the soft `volume_backing.not_composed` event and the driver holds no pool (a declared volume then fails its start as
`PoolAbsent`); `Err` refuses startup (§ Composition root). The root never keeps the pool.

### Disk attachment (VD-6, VD-7)

`overdrive-core/src/vm/config.rs`: `VOLUME_DISK_SERIAL: &str = "ovd-volume"`, the one serial (multiple volumes are L1, #364). `DiskAttachment`
gains private `serial: Option<&'static str>` and `direct: bool`; `to_disk_arg()` renders
`path=<p>,image_type=raw[,readonly=on][,direct=on][,serial=<s>]`. `DiskAttachment::new(path, readonly)` keeps its signature and meaning (no
serial, no `direct`; the rootfs site is unchanged). The one new constructor is `DiskAttachment::volume(device: PathBuf) -> Self`: writable,
`direct` on, serial `VOLUME_DISK_SERIAL`. There is no public way to build a read-only or serial-less direct attachment, and no way to set
`direct` or a serial otherwise. For device `/dev/mapper/overdrive-ovd--<hash>` (§ Port, the device path) it renders exactly
`path=/dev/mapper/overdrive-ovd--<hash>,image_type=raw,direct=on,serial=ovd-volume`; that string is the argv golden. `CloudHypervisorVmm` builds
the second `--disk` from `VmConfig.volume` with `DiskAttachment::volume`. The `--disk` order (rootfs first) is checked inside the adapter crate,
which owns its command builder. `VmConfig` gains `volume: Option<VolumeAttachment { device: PathBuf }>`. The volume `--disk` follows the rootfs
`--disk` (so it is `/dev/vdb`). `prepare_confined_paths` also chowns the symlink-resolved device node to `root:<VMM gid>` mode `0660` (a block
device node is refused to the dropped uid otherwise, spike C3); a failure is the existing `ConfinementUnavailable { control: UidDrop }`. The owner
and mode are not restored at teardown; no other VMM can open another workload's node because each VMM's Landlock allow-list is derived from its
own launch arguments (ADR-0315). `rlimit_fsize` is unchanged. Fork: only F2 (ADR-0319, the `O_EXCL` open) is a fork change; F1 (ADR-0316) and F3 (ADR-0320) are upstream Cloud Hypervisor's own behaviour
since the fork moved to upstream `main` (2026-11-20, ADR-0161 § Amendment); the pinned revision and checksums follow that move.

**Wire contract: the VMM's exit status.** Exit status 111 from the forked Cloud Hypervisor means the volume disk's exclusive open found the disk held ([ADR-0319](../../product/architecture/adr-0319-a-writable-block-device-is-opened-exclusively-by-the-vmm.md)).

### Launch path and driver failures (VD-8, VD-9)

`VmDriver::provision_vmm`, for a `VolumePayload`, after claiming supervision and before `Vmm::create`: `ensure` (awaited to its return; the 30 s
deadline is the adapter's, § Port, and the driver holds no timer on it, so a `PoolCommand` with a `TimedOut` source reaches the driver as an
ordinary `Err` and becomes `VolumeUnavailable { PoolCommandFailed { operation } }`, unless the launch is superseded, which wins at point 2);
then poll `claim_state` on the
driver's `Clock` (the interval is the crafter's) until `Free` or 30 s have passed. One new `#[non_exhaustive]` variant:
`VmStartFailure::VolumeUnavailable { cause: VolumeUnavailableCause }` where `VolumeUnavailableCause` is `PoolAbsent | InsufficientSpace { needed,
free } | PoolCommandFailed { operation } | Held` (`Held` carries nothing: the wait is the constant 30 s bound; a `NotFound` for a volume `ensure`
just returned is a bug and maps to `PoolCommandFailed`). Any `Err` from `claim_state` (`NotFound`, or `PoolCommand { operation: Claim }`) ends the
launch at once, with no further polling, as `VolumeUnavailable { PoolCommandFailed { operation: Claim } }`: only `Ok(Held)` is polled again,
until the 30 s bound makes it `Held`. This is the same outcome class as the model's `LaunchUnavailable` and `LaunchCmdFail`: a typed
`VolumeUnavailable`, the budget charged, nothing attached. One new `TransitionReason::VmVolumeUnavailable { cause }`; `From<&DriverStartFailure>`
stays total; `human_readable` names the volume, the cause and the remedy. It is an ordinary start failure, charged to the restart budget. No
volume pool composed with a declared volume: `PoolAbsent`. The launch holds its workload's convergence lane (ADR-0102) for at most the `ensure`
deadline (30 s) plus the claim wait (30 s) plus the boot wait (30 s), about 90 s, **if nothing supersedes it**. A `workload stop`, a `workload
restart` or node shutdown raises the lane's supersession signal (§ Superseding a launch), the launch abandons its waits at once, and the lane is
released after the launch's teardown: the hold a stop can face is the `ensure` command still running (at most its 30 s deadline) plus
`Vmm::create` and the teardown, never the claim wait or a boot wait.

**Order of the launch's steps.** The volume steps (`ensure`, then the claim wait) run **before the first host artifact**: after the supervision
claim and before `create_run_dir`, the cgroup scope, the index link, the rootfs clone and `Vmm::create`. A launch superseded in either volume
step has therefore created nothing on the host (its only state is the supervision claim, which the shim releases), and the claim wait never
holds a run directory or a scope the reclamation sweep would have to consider.

### Superseding a launch (SD-1 to SD-5; ADR-0334)

The signal's whole path, with its owner at each hop:

1. **Owner of the lane: the convergence owner (ADR-0102), through `run_convergence_tick`.** The tick's first step, before hydration, opens a
   lane for its `TargetResource` in the runtime's lane table and closes it (RAII) when the tick returns. A lane holds one
   `tokio_util::sync::CancellationToken`, created as `child_token()` of the lane table's one **shutdown token** (SD-5). ADR-0102 admits one open
   lane per target, so the table is keyed by target: `open` registers the token under the target's key and the guard removes that key.
   `run_convergence_tick` and `spawn_convergence_loop` keep their signatures.
2. **Producers: the two operator handlers, after their intent write commits, and node shutdown (§ Node shutdown raises the signal).**
   `ReconcilerRuntime::supersede_lane(&self, target: &TargetResource)` cancels the open lane's token for `target`, and does nothing when no lane
   is open. It is idempotent. `stop_workload` (on both `Inserted` and `KeyExists`) and `restart_workload` call it with `workload/<id>` after the
   `put_if_absent` / `txn` returns and before `enqueue_workload_lifecycle_eval`. No other caller: `submit_workload`, the interest router, resync,
   exit observers and self-requeues never supersede (§ Gate G-3 counterexample). The one other producer is shutdown, through
   `supersede_all_lanes`.
3. **Consumer: the driver, through a core port.** In `overdrive-core::traits::driver`:

   ```rust
   #[async_trait]
   pub trait LaunchSupersession: Send + Sync {
       /// True from the moment the signal is raised, forever after (level, never edge). A step boundary reads this.
       fn is_superseded(&self) -> bool;
       /// Resolves when `is_superseded()` is true (immediately if it already is); never resolves otherwise. Cancel-safe.
       async fn superseded(&self);
   }
   ```

   The trait docstring states the four properties: precondition none; postcondition above; edge case: a signal raised before the call, or twice,
   behaves as once; invariant: `superseded()` resolves iff `is_superseded()` is true at some instant after the call. The production impl is a
   private type of `overdrive-control-plane` over the lane's token; `overdrive-sim` provides `SimLaunchSupersession`, a signal a test raises and a
   `superseded()` that parks on a waker (the `SimClock::sleep` shape, never auto-resolving); its `raise()` makes `is_superseded()` true and
   resolves every pending and later `superseded()` as one step. Exact sim constructors are DISTILL's.
4. **The port change.** `Driver::start` becomes `async fn start(&self, spec: &AllocationSpec, supersession: &dyn LaunchSupersession) ->
   Result<AllocationHandle, DriverError>`. The parameter is required for every driver; a driver with no interruptible wait names it
   `_supersession` and says so (SD-2). `DriverError` gains one unit variant `StartSuperseded` (Display: "start superseded by a newer desired
   state"). It is a typed outcome, not a failure: it has no `DriverStartFailure`, no `TransitionReason`, and `From<&DriverStartFailure> for
   TransitionReason` is untouched, so a superseded launch has no representation that could be written as a failure row or charged to a budget.
   The rustdoc of `Driver::start` pins the contract for every driver: `StartSuperseded` is returnable only after the driver observed the signal
   and means a clean teardown, with nothing left behind it (a teardown that fails is `StartRejected`, never `StartSuperseded`); a driver with no
   interruptible wait never returns it. The shim reaches the signal for its call as `state.runtime.lane_supersession(target)`, which returns the
   open lane's signal. Every production dispatch of `StartAllocation` / `RestartAllocation` runs inside a convergence tick; a call with no open
   lane returns an unraised signal, so the method is total.
5. **What the shim does with `Err(DriverError::StartSuperseded)`.** *Common to both arms:* the successor's network release it runs on any `Err`
   (`retire_guest_network_lease` then `teardown_for_dispatch`; in the restart arm `cleanup_restart_successor`), then `IdentityMgr::drop_svid(alloc)`
   (the SVID the shim minted before the launch; no row exists, so no row-woken `SvidLifecycle` would drop it), then
   `Driver::release_supervision(alloc)`. It writes **no allocation row** and returns `Ok(())` for the action: no `Running`, no `Failed`, no
   restart-budget input. It emits one `tracing` event `launch.superseded` with the allocation id and workload target. The durable
   `issued_certificates` audit row stays. *`RestartAllocation` arm:* the superseded outcome is the successor outcome `Ok(())` passed to
   `finish_restart` (`action_shim/mod.rs:1367`), exactly as the duplicate-owner outcome is, so the exact-old predecessor cleanup of ADR-0106
   (`cleanup_restart_predecessor`) still runs; the one exit that skips `finish_restart` is the fail-stop `Ok(false)` of activation (N-FAILSTOP).
   *`StartAllocation` arm:* there is no predecessor. The world is what it was before the launch except for the unused allocation id (an unused
   suffix gap, never a reuse; its `issued_generation` entry stays with it).
6. **What the next evaluation sees.** The lane is released and the next evaluation of the target is admitted (FIFO, ADR-0102 unchanged, no
   reserved slot). `stop`: the stop sentinel is set, so there is nothing to launch; the workload stays down until the next `workload restart`.
   `restart`, node shutdown or a signal inside the conservative window: the generation is still pending and the next evaluation relaunches it
   (§ Restart generation is handled when its successor is observed, "Relaunch").

**`VmDriver::start` observation points (SD-4).** All reads are `is_superseded()` (a level read) unless stated. A superseded launch tears down
exactly what the existing start-failure path tears down, and returns `StartSuperseded`:

| # | Point | Superseded here |
|---|---|---|
| 1 | entry, and after the supervision claim, before `ensure` | return, nothing created. The shim's `release_supervision` removes the claim by key and is a no-op for an id never claimed (`vm_driver.rs:2100-2106`), so one point covers both sides of the claim |
| 2 | after `ensure` returns, whatever its result | return. `ensure` itself is never interrupted: the driver awaits it, and it returns by itself within the adapter-owned 30 s deadline (§ Port), an in-flight `lvm` command running to its own end; the ensured volume stays (VD-2). A deadline `Err` observed here with the signal raised is `StartSuperseded`, not a `Failed` row |
| 3 | each claim-poll iteration: before `claim_state`, and while sleeping the poll interval (`select!` of `supersession.superseded()` against `Clock::sleep`, `biased`, superseded first) | return at once; the interval's remainder is not waited |
| 4 | after the claim wait, before the first host artifact | return, nothing created |
| – | `create_run_dir` ... `Vmm::create`, then `place_pid_in_scope` | **no observation point**: a bounded effect that runs to its end (the rootfs clone and the VMM spawn are not interruptible), so the VMM is always inside its scope when an abandonment finds it |
| 5 | the guest-boot race: `select!` over `biased; accept_ready, supersession.superseded(), exit.recv(), clock.sleep(VM_BOOT_DEADLINE)`; READY is polled first because a launch that won its boot race is an ordinary running allocation | `cleanup_after_start_failure` (terminate the VMM with zero grace, remove clone and link, kill and remove the scope, remove the run directory, clear the beacon record), then return |
| 6 | the second `VM_BOOT_DEADLINE` wait on the beacon-error arm | same as 5 |

Rules that follow, each closing a reachable defect:

- **Superseded wins at every point it is observed, including over a typed failure found at the same point** (an `ensure` error, `Held` at the
  bound, a boot deadline). A launch that finished its boot race (`accept_ready` selected) before the signal was observed is an ordinary success:
  it returns `Ok`, its `Running` row is written, and the stop is evaluated against it by the normal path. The post-boot steps (listener board,
  beacon writer, exit watcher) have no observation point.
- **A teardown that fails is not hidden:** if `cleanup_after_start_failure` reports a failure, `start` returns the existing unclassified
  `StartRejected` carrying it (the shim writes the `Failed` row as for any start failure and the reclamation sweep reaps what is left once the
  claim is released). `StartSuperseded` means a clean abandonment.
- **Every VM launch gets the boot-wait abandonment**, not only volume launches: the stop latency of a booting VM drops from up to 30 s to the
  teardown time.
- **Accepted cost: no shim pre-check.** An evaluation whose lane was raised before its `Start` is dispatched still provisions the guest network
  and mints the SVID before `driver.start` observes the signal at point 1, then tears both down. The waste is bounded by one provision and one
  issuance per operator commit or shutdown; a shim check would add a second observation site for no correctness gain.

**Stop-latency bound after the signal.** `ensure` in flight: at most the `ensure` deadline (30 s). `Vmm::create` in flight: its own duration.
Claim wait or boot wait: the teardown. Then the lane is released and the stop's evaluation is admitted at the next free admission. ADR-0102's
"eight stalled launches fill every slot" is unchanged for a stop of a workload that is **not** itself launching, and no longer applies to a stop
of a launching workload, whose own launch releases its slot at once.

### Node shutdown raises the signal (SD-5; ADR-0334)

**Today** (`crates/overdrive-control-plane/src/lib.rs`): `spawn_convergence_loop` (`:5126`) owns the admitted evaluations in a `FuturesUnordered`;
its `select!` arm `shutdown.cancelled()` (`:5237`) closes admission and the loop then drains `active` to completion (`:5220`). Three graceful
paths cancel the same `convergence_shutdown` token and await the loop task: `ServerHandle::shutdown` (`:3079`, after the gateway stage `:3056`),
`FailedBootTasks::shutdown` (`:3215`) and the `run_server` early-failure return (`:4844`). `kill_for_test` (`:2985`) aborts the task and models
process death; it is unchanged. So an admitted launch that is waiting runs on to its own bound (about 90 s) before the node can shut down.

**Design.**

1. **One owner, one token.** The runtime's lane table owns one shutdown `CancellationToken`. Every lane token is `shutdown.child_token()`
   (a child of an already-cancelled parent is returned already cancelled, and a parent's `cancel` cancels every child). There is no "list the open
   lanes, then signal each" step, so a lane opened while shutdown begins, or after it, is born raised: the check and the act are one primitive
   (`rust.md` § "Check-and-act must be atomic"). `supersede_lane(target)` cancels only that lane's child; it never cancels the parent.
2. **One entry point, called from one place.** `ReconcilerRuntime::supersede_all_lanes(&self)` cancels the shutdown token; it is idempotent, takes
   no lock across an `.await` and has no other effect. `spawn_convergence_loop` calls it in its existing `shutdown.cancelled()` arm, in the same
   arm that sets `admission_closed`. Because the three graceful paths all end in that arm, the one call covers all of them. An evaluation the loop
   admitted but whose tick future has not been polled yet opens its lane after the cancel and observes the signal at its first observation point;
   an evaluation not yet admitted is never admitted.
3. **What a launch does.** The same as for a stop: it abandons its waits at the observation points, tears down through
   `cleanup_after_start_failure` where it created a host artifact, and returns `StartSuperseded`; the shim writes no row, charges nothing and
   releases the supervision claim, the guest network lease and the SVID. The loop awaits that teardown as part of draining `active`, so the stores
   and drivers the shim uses are still up. An `ensure` in flight and `Vmm::create` still run to their end.
4. **Nothing is written, and the next boot converges because the reconciler observes it.** After the node boots again `WorkloadLifecycle`
   hydrates the committed generation and the rows and decides from them: a first placement has "no running allocation" and is placed; a
   `workload restart` or a resume whose launch was abandoned has its generation still pending (§ Restart generation, "Relaunch"). The View
   records what was issued, never that the launch happened, which is why this holds after the View was persisted before the dispatch that
   shutdown abandoned. A `Running` allocation, a launch whose boot race had already selected `accept_ready`, and every non-launch evaluation are
   unaffected.
5. **Cooperative only.** The token is observed at step boundaries and inside the two waits; nothing is `abort()`ed or dropped mid-effect.
6. **Latency.** Shutdown's wait for a launch drops from about 90 s to the `ensure` deadline (30 s, when `ensure` is in flight) plus `Vmm::create`
   and the teardown. The signal is raised when the convergence stage begins, after the gateway stage of `ServerHandle::shutdown` (bounded by
   `drain_deadline`); launches run on during that stage as today.

**Not changed:** `kill_for_test`; the convergence loop's drain of admitted evaluations to their result (ADR-0102 § Timing); admission closing;
the broker; the `run_convergence_tick` and `spawn_convergence_loop` signatures.

### A VMM kill's reap wait is bounded (RB-1; ADR-0348)

**Today.** `CloudHypervisorVmm::terminate` (`crates/overdrive-host/src/vmm.rs:650-690`) notifies the reaper to kill, then awaits the reap with
no bound (`rx.changed().await`, `:670` for zero grace, `:688` otherwise). A VMM with a thread blocked in uninterruptible I/O (a flush held on
a suspended or failing dm device) never becomes reapable, so `VmDriver::stop` (`vm_driver.rs:2047-2212`) never returns, the shim's
`StopAllocation` arm (`action_shim/mod.rs:2743`) never writes the terminal row, the row stays `Running` and the lane stays held. The
`terminate` callers are `VmDriver::stop` (`:2128`, grace `VM_STOP_GRACE` = 10 s), the start-failure and boot-race cleanup (`:938`, `:1311`,
zero grace) and the failed EXEC release (`:2315`, zero grace).

**The contract (user ruling 2026-11-23, decision 11, option a).**

1. **The outcome.** `overdrive_core::traits::vmm::VmTermination` gains one unit variant, `KilledNotReaped`: the kill was delivered and the
   process was still not reaped when `VMM_REAP_BOUND` elapsed. It is an outcome, not a `VmmError`, because the substrate did not refuse
   anything; `VmmError` keeps meaning "the kill or spawn itself failed". The variant carries no payload: the pid is the caller's
   `VmControl`, and no exit exists to report. The existing variants are unchanged: `ExitedWithinGrace(VmmExit)`, and `Killed` for a process
   reaped after the kill (and for an already-gone process).
2. **The bound.** `pub const VMM_REAP_BOUND: Duration = Duration::from_secs(5)` in `overdrive-core::traits::vmm`, beside the trait, so every
   adapter and test names one value. 5 s is about twice the held-flush drain measured in the spike (2.4 s, `design/wave-decisions.md` § Scope limitations), so a disk that is
   draining and healthy reaps inside it and the new outcome stays the rare case; a reap after SIGKILL takes milliseconds when nothing is
   stuck; and it is short enough that the worst case, a stop of an unreapable VMM, costs `VM_STOP_GRACE` + `VMM_REAP_BOUND` = 15 s of lane
   time (a VMM already SIGKILLed from outside still waits out the grace first, since the grace wait ends only on the reap). It is a policy
   value (ADR-0348 Consequences).
3. **When the bound starts and what clock measures it.** The bound starts at the instant `terminate` requests the kill, on both the
   grace-expired path and the zero-grace path. The wait is measured on the injected `Clock`: `SimVmm` takes an `Arc<dyn Clock>` as a required
   constructor parameter and waits `VMM_REAP_BOUND` on it (`Clock::sleep`, which parks until the harness advances time), so seeded simulation
   drives the bound. `CloudHypervisorVmm` is the host binding of the port and measures it on real time, as it already does for `grace`
   (`tokio::time::sleep`); it takes no clock.
4. **The `Vmm::terminate` rustdoc states** (each clause is part of the port contract and is covered by the equivalence test):
   - *Postcondition on `Ok(Killed | ExitedWithinGrace)`:* the process is reaped. *On `Ok(KilledNotReaped)`:* SIGKILL has been delivered to the
     process, no thread can run guest code again, and the process is not reaped; it releases every resource it holds (descriptors, the volume's
     kernel claim, the vsock device, its cgroup membership) only when it is reaped.
   - *The adapter keeps waiting for the process after returning `KilledNotReaped`.* A late reap runs the adapter's ordinary reap path
     (its exit watch fires, its live record is dropped); it is not an error and changes nothing the caller wrote.
   - *Edge cases:* a second `terminate` against a process that is killed and not yet reaped re-waits `VMM_REAP_BOUND` from its own call and may
     return `KilledNotReaped` again (idempotent; it never re-sends a different outcome for a reaped process, which is `Killed`). `grace ==
     Duration::ZERO` kills at once and then waits the bound. An already-gone or unknown pid is `Ok(Killed)` as today.
   - *`Err(VmmError)`* only when the substrate refuses the kill, as today; never for a slow reap.
5. **`SimVmm`.** It models a process that does not reap by scripted state: a test marks a process unreapable (the constructor and the scripting
   method are DISTILL's), `terminate` on it marks the process killed, waits `VMM_REAP_BOUND` on the injected clock and returns
   `KilledNotReaped`; the process keeps every resource it holds until the test reaps it (a scripted `exit_process`), exactly as the real
   process does. Concretely its `device` entry (the vsock CID holder `vsock_device_holders` reports) is NOT released at `terminate` for an
   unreapable process, but is released at the scripted reap; its exit sender fires only at the scripted reap. The reapable path is unchanged.
6. **`VmDriver::stop` on `Ok(KilledNotReaped)`.** The outcome is a completed stop. The driver emits one `tracing` event
   `vm.lifecycle.stop_not_reaped` (allocation id, pid), writes the beacon writer's `vmm_has_exited` as for any `terminate` completion, runs
   `cleanup_driver_artifacts` in the same order (`cgroup.kill`, scope removal, run directory, clone then index link), treating a scope
   removal that fails for the non-empty scope as the best effort it already is, and returns `Ok(())`. The driver does not touch the guest
   lease; the shim holds it (item 11). `status()` is `Err(NotFound)` (the
   entry is `EndingInFlight`, as for every stop); the entry stays `EndingInFlight` until the shim's `release_supervision`. No new
   `DriverError`: the return type of `Driver::stop` is unchanged. The `Driver::stop` contract says: on `Ok`, the VMM has been killed and is
   reaped, or killed and not yet reaped (ADR-0348); the shim comment at `action_shim/mod.rs:2774` ("`Driver::stop` awaits process quiescence")
   is corrected to that.
7. **The other callers.** Start-failure and boot-race cleanup and the failed EXEC release ignore the outcome as they do today; a
   `KilledNotReaped` there leaves the same leftovers, owned as in item 9, and the failure they were reporting is unchanged.
8. **What the shim writes.** Nothing new. `StopAllocation` proceeds exactly as for a reaped VMM: `retire_guest_network_lease`,
   `mtls_lifecycle.stop_alloc`, `teardown_for_dispatch` (which, while the VMM is unreaped, tears the network down but keeps the lease
   Retiring: item 11), then the `Terminated` row with `TransitionReason::Stopped { by: Reconciler }` and
   the action's `terminal`, no `Failed` row, no restart-budget input, then `release_supervision`. A `Running` row therefore becomes
   `Terminated` within `VM_STOP_GRACE + VMM_REAP_BOUND` plus the cleanup time. `restart` (`RestartAllocation`) stops its predecessor through
   the same `Driver::stop` and is bounded the same way.
9. **Who owns each leftover.**

   | Leftover | Owner | Removed when |
   |---|---|---|
   | the hypervisor process | the adapter's reaper task (`CloudHypervisorVmm`) | it reaps the process, whenever that is; the adapter's live record is dropped then |
   | the cgroup scope (non-empty while a thread is stuck) | `VmReclamation` | the first pass after `release_supervision` and after the process is reaped: terminal VM allocation with host artifacts and no driver claim emits `DiscardStrandedArtifacts` (`vm_reclamation.rs:161-171`); the pass runs at boot and every `VM_RECLAMATION_SWEEP_INTERVAL` (30 s). A pass that cannot remove it returns and the next pass retries |
   | the run directory and the clone | `VmDriver::stop` (best effort, both are removable while the process is stuck); `VmReclamation` as the backstop | at stop; otherwise the same pass |
   | the volume's kernel claim | the kernel | when the stuck I/O drains and the process exits; Overdrive neither releases nor fences it (ADR-0317). A replacement's claim wait covers it; past its bound the replacement fails typed `Held` (ADR-0317) |
   | the vsock device and its CID | the kernel, at the process's exit | the same reap |
   | the guest-address lease (its offset names the CID) | the shim: `teardown_guest_network`, at stop; the network-reclaim path, if the VMM was unreaped at stop | retired at stop; released only once the allocation's scope is absent (item 11), so a lease whose VMM is unreaped is released by the first network-reclaim attempt after `VmReclamation` has removed the scope |

10. **Not changed.** `Driver::stop`'s signature; the lane and supersession path (ADR-0334); the claim wait and its 30 s bound (ADR-0317);
    `VM_STOP_GRACE`; the exit watcher (it finds `EndingInFlight` at a late reap and returns, as it does today for every stop); the reconcilers;
    the observation row shapes; `kill_for_test`; the `GuestAddressPool` methods and their signatures; `GuestNetworkProvisioner`'s signatures;
    the `DiscardStrandedArtifacts` action and its executor; `VmHostState`'s signatures.
11. **The guest-address lease is released only once the VMM is reaped (amends ADR-0156).** No public API.
    - *The fact.* A guest's CID is `GUEST_CID_BASE` plus its lease offset (ADR-0156). The kernel unhashes a CID at the last close of the device
      file (`vhost_vsock_dev_release`, `drivers/vhost/vsock.c:753-760 @ v7.0`), and a launch whose CID is still hashed fails
      `-EADDRINUSE` (`vhost_vsock_set_cid`, `:804-813 @ v7.0`). A killed-not-reaped VMM still holds the file, so freeing its offset would let
      next-fit hand its CID to a new launch. Release-last (`GuestAddressPool::release`, `GuestNetworkProvisioner::teardown` rustdoc) means
      *reaped*, not *stopped*; both docs say so.
    - *The precondition, and why this fact.* The shim releases a lease only when the allocation's cgroup scope is absent from
      `VmHostState::observe()` (`scopes`). `rmdir` of a scope succeeds only when the cgroup has no task left (`cgroup_rmdir` ->
      `cgroup_drain_dying`, `kernel/cgroup/cgroup.c:6254-6272`, `:6309-6311 @ v7.0`; a live thread in an uninterruptible wait is a visible task,
      so `-EBUSY`), and a task leaves its cgroup only after it has dropped its files (ADR-0156 § Decision, the exit-ordering citations). The
      shim reads the resource, not the stop's outcome: it needs no signal from `Driver::stop`, and it is right if the process is reaped a
      moment after the bound.
    - *The mechanism, one function.* Every release goes through `teardown_guest_network`; the stop, the restart predecessor and successor
      cleanups, the start-failure cleanups and `reclaim_guest_network` all reach it through `teardown_for_dispatch`, so the gate holds at every
      site. Its order: (1) the plan is read from the pool; none, return `Ok` as today. (2) `VmHostState::observe()`; a failure returns the
      existing host-substrate `ShimError::Reclamation` before any effect, so the existing replay retry applies and nothing is torn down twice.
      (3) `GuestNetworkProvisioner::teardown(plan)`, unchanged. (4) Scope absent: `release`, as today. Scope present: the lease is left
      Retiring (retired if still Admitted), not released, and the function returns `Ok`. The `Ok` is what lets `StopAllocation` write its
      terminal row (item 8).
    - *The late release.* A `Terminated` or `Failed` row whose allocation still holds a lease is already a network-reclaim candidate:
      `WorkloadLifecycle` emits `Action::ReclaimAllocationNetwork` for it (`workload_lifecycle.rs:485-516`), paced by `reclaim_attempts` and
      `reclaim_emitted_at` (`backoff_for_attempt`), and its executor `reclaim_guest_network` re-runs `teardown_for_dispatch`.
      `GuestNetworkProvisioner::teardown` converges on absence, so the re-run on a torn-down network returns `Ok`. After the reap,
      `VmReclamation` removes the scope (`DiscardStrandedArtifacts`, item 9), the next reclaim attempt finds it absent and releases. No new
      action, no new View field, no new port method. `reclaim_guest_network` and the helpers above gain a `&dyn VmHostState` (the dispatch
      already holds one); that is crate-internal structure.
    - *Boot.* A lease is process-local and not persisted (ADR-0118): after a restart there is no Retiring lease to settle. The invariant
      across a restart is boot reclamation's: `vm_reclamation_boot::converge` removes every unsupervised VMM's scope, and so completes only
      after those VMMs have exited, before the first `assign`. A VMM that stays unreaped across a `serve` crash is the `kill_scope` busy case
      (Open point below), not a lease case.
    - *Cost.* One `observe()` (a bounded directory walk) per teardown, which is not a hot path. One attachment slot per unreaped VMM until
      the reap, counted as Retiring against the cap; at the cap a new launch is refused `AdmissionCapReached`, typed, as for any held lease.

**Lifecycle Gate Ownership.** This adds no gate. It changes the meaning of one existing state, recorded here and in ADR-0348: an allocation
ended by a stop is `Terminated` once the VMM is killed and can run no guest code, not once its process is reaped. The kernel fact behind it:
a fatal signal puts SIGKILL pending on every thread of the group and wakes it (`complete_signal`, `kernel/signal.c:1003-1025 @ v7.0`); a thread
in an uninterruptible wait exits when the wait ends. The user approved this meaning with decision 11.

**Boundary scenarios.** (1) The VMM reaps within the bound: `Killed` or `ExitedWithinGrace`, unchanged. (2) It does not:
`KilledNotReaped`, the row `Terminated` `Stopped`, supervision released. (3) Unrelated allocations and other workloads' lanes are unaffected:
the stuck stop releases its own lane at the end of its own wait. (4) Late reap after the stop: the exit watcher finds no `Live` entry, so no
`ExitEvent` and no row write; the row stays as written. (5) A control-plane crash or node reboot while a process is unreapable: the process dies
with the node on reboot; after a control-plane crash only, the boot sweep handles it as for any unsupervised VMM (M7). (6) A reapable VMM never
sees the new outcome. (7) An unreaped VMM's lease is held Retiring after the stop: a later assign that wraps onto its offset skips it, and the
lease is released after the reap, not before. (8) Observing the scope fails at the stop: the stop returns the typed error, the row stays
`Running`, and the replay retries.

**Tests this decision requires (for the acceptance designer; none is written here).**

- Tier 3: S-VOL-T3-12 (`crates/overdrive-cli/tests/integration/volume_survives.rs:754`), already authored and pending on this defect. The
  DELIVER step removes its `#[should_panic]` marker. It proves the real adapter, a real SIGKILL and a real held flush; it cannot be produced
  by simulation.
- Tier 1, seeded `overdrive-sim`, production `VmDriver::stop` over `SimVmm` with an unreapable process: (a) `stop` returns `Ok`, advancing the
  `SimClock` by exactly `VMM_REAP_BOUND` after the grace, and the shim writes `Terminated`/`Stopped`, no `Failed`, no charge; (b) the
  `SimVolumeBacking` claim is still `Held` and the `SimVmm` vsock holder still reports the CID after the stop, and both clear only at the
  scripted reap; (c) the scripted late reap emits no `ExitEvent` and changes no row; (d) a replacement launch during the stuck window waits on
  the claim, is not charged before its bound, and attaches after the scripted reap and drain.
- A `Vmm` equivalence clause (`development.md` § "Trait definitions specify behavior"): `CloudHypervisorVmm` and `SimVmm` agree on the
  idempotent second `terminate` and on the reapable outcomes.
- `VmReclamation`: a pass over a terminal allocation whose scope cannot be removed returns without waiting and emits again on the next pass;
  after the reap the next pass removes it.
- Stop-totality: the existing `vm_driver_stop_totality.rs` doubles gain a `KilledNotReaped` case.
- Tier 1, seeded, production shim and `WorkloadLifecycle` over `SimVmm` and `SimVmHostState` (item 11): (e) after the unreaped stop the pool
  reports the lease Retiring, the row is `Terminated`, and an assign that wraps to the offset takes another; (f) after the scripted reap, a
  `VmReclamation` pass and a reclaim attempt, the lease is released and no launch clashed. The sim obligation: `SimVmHostState.observe()`
  reports the unreaped process's scope, and its `kill_scope` removes it only once `SimVmm` has reaped that process; the two adapters share
  that one fact. (g) The scope observation fails at the stop: typed error, no teardown, no row, the replay succeeds.
- Tier 3: S-VOL-T3-12 additionally asserts no lease is released while the held-flush VMM is unreaped, and that a lease is released after the
  drain (real scope removal, real device close).
- Quint (`specs/quint/guest-flow-owner/cid_lease.qnt`; the modeler edits it, this is the required change): the stop of a running allocation
  gains a second outcome, `StopUnreaped(a)`: the workload goes `done` as for `Stop`, but the device stays held by `a` and the lease stays held,
  recorded in a model-level `late: offset -> workload` map (a workload can accumulate several, so not a per-workload field). `Reap(o)`
  clears the device of an offset in `late`. `LateRelease(o)` frees the lease and the `late` entry, enabled only when the device is not held by
  the lease's holder (the existing `canRelease` guard shape). `Crash` drops `late` with the pool and turns a held device into an orphan, as for
  a running VMM; `Reclaim` is enabled only when no orphan is unreaped, and an unreaped orphan leaves through `OrphanExit`. Invariants:
  `CidUnique` and `NoUntrackedLease` range over `late` as well; `LeaseOutlivesVmm` is unchanged and is the check with teeth. Progress on the
  liveness instance: an offset in `late` whose device is free is eventually released. Named runs for `checks.toml`:
  `StopOfAnUnreapedVmmHoldsItsLeaseUntilTheReap`, `NextFitSkipsAnUnreapedLease` (three offsets, the wrap lands on the held one),
  `ServeCrashWithAnUnreapedVmmBlocksAssignUntilItExits`. One hazard variant, `releaseUnreapedAtStop`: the unreaped stop frees the lease with
  the stop (today's behaviour for a reaped VMM); the checker must find `NoLaunchClash` / `LeaseOutlivesVmm` violated. Assumption K-C7:
  the scope is absent only after the device is released (cgroup.c:6254-6311 and ADR-0156's exit-ordering citations), discharged by the
  Tier 3 addition above. Conformance mapping for the DISTILL driver: `StopUnreaped` is the real stop handler and tick over an unreapable
  `SimVmm` process; `Reap` is the scripted `SimVmm::exit_process`; `LateRelease` is the production convergence tick that dispatches
  `ReclaimAllocationNetwork`; `late` and the lease are projected from `GuestAddressPool::observe` and `snapshot` (already pinned for
  `lease`), and `dev` from `SimVmm::vsock_device_holders` (the existing marked arbiter read).

**Open point carried to DELIVER, not a contract.** `VmHostState::kill_scope` (`vm_reclamation_boot.rs:5`) must return on a scope that rmdir
refuses as busy rather than wait. The cell above covers it; the code is read when the step lands.

**Open questions (surfaced, not decided here).**
- `kill_scope`'s port rustdoc says it retries removal until the scope settles, with no bound. For a scope held by an unreaped VMM that never
  returns, which stalls `VmReclamation`'s single `node/<id>` target for every VM and, at boot, `converge`. Item 11 depends on `Ok` meaning
  "scope removed". Bounding the settle wait and returning a busy `io::ErrorKind` (no new type) changes the `VmHostState` port contract, so it
  needs the user's ruling before DELIVER. The pre-existing open point above is the same question.
- Network-reclaim pacing (`backoff_for_attempt`) has no attempt ceiling in the code read here (`reclaim_attempts` is kept while the lease
  exists). DELIVER confirms there is none, since a ceiling would strand the lease; the late release depends on it.
- A stopped allocation whose operator restart is pending is its workload's restart owner until the successor row is observed
  (`pending_restart_owner`), so its late reclaim is deferred until then. DELIVER confirms the lease still releases after the successor row.

### Restart generation is handled when its successor is observed (SD-6; ADR-0335, which amends ADR-0073, with ADR-0337 and ADR-0338)

**Decision.** Whether a generation has been handled is derived from observation: a generation is handled once the allocation issued for it **has
an allocation row**. The View records only what was issued (an input), never that a launch happened. Today `WorkloadLifecycle` stamps
`observed_generation` on the tick that emits a replacement (`workload_lifecycle.rs:1309`, `:1437-1439`) and the runtime fsyncs the View before it
dispatches (`reconciler_runtime.rs:1557-1561`); a launch that ends with no row (superseded, or lost with the control plane) then leaves the
workload down beside the old operator-stopped or replaced row (veto `:1069`, restartable filter `:1146-1148`).

**Pinned contract (overdrive-reconcilers and overdrive-control-plane).**

- `WorkloadLifecycleView.observed_generation` is **deleted**. `WorkloadLifecycleView` gains `#[serde(default)] pub issued_generation:
  BTreeMap<AllocationId, u64>`: the desired generation each issued allocation id serves, written at the two issuing sites (the restart arm
  `:1305-1335` and the placement arm `:1370-1440`) in the same `next_view` as the id's `restart_counts` entry and with the same lifetime. The
  value is `desired.generation` at issue in every case. It is never consulted to decide whether a launch happened. CBOR additive field per
  `development.md` § "Reconciler I/O" -> "Schema evolution"; no rkyv envelope: `AllocStatusRow` carries no generation and gains none.
- `pub fn generation_replacement_pending(view: &WorkloadLifecycleView, desired_generation: u64, current: Option<&AllocStatusRow>) -> bool` in
  `overdrive-reconcilers`: true iff `current` is `Some(row)`, **`row` is not a finished Job row (`row.kind == WorkloadKind::Job &&
  row.terminal.is_some()`, H1)**, and `view.issued_generation.get(&row.alloc_id)` (absent reads `0`) is less than `desired_generation`. The
  finished-Job test is the action shim's own (`allocation_attempt_transition` returns `NoChange` for a Job row with a terminal;
  `workload_lifecycle.rs:131-139`). `current` is `current_alloc(..)` over the **observed** rows (the row whose id has the numerically highest
  `-<N>` suffix among rows whose suffix parses, `workload_lifecycle.rs:1506`), so an issued id with no row is invisible to it. With no row at all
  it is false. It is the only definition of `restart_pending`: it replaces the three expressions in `workload_lifecycle.rs` (`:366`, `:451`,
  `:925`) and the one in `ReconcilerRuntime::restart_status_for_alloc` (`reconciler_runtime.rs:514-534`), whose signature and that of
  `restart_after_failure` are unchanged.
- `ReconcilerRuntime::issued_generation_for_alloc(&self, target: &TargetResource, alloc_id: &AllocationId) -> Option<u64>`: the conformance and
  test read of `issued_generation`, in the shape of `attempt_index_for_alloc` (`reconciler_runtime.rs:546`).
- Where `restart_pending` is false, behaviour is unchanged. Where it is true, ADR-0073's R2 stop and R3/R4 placement are unchanged, except that
  no tick stamps anything: R3 and R4 issue the successor id with its `issued_generation` entry, and the generation stops being pending on the
  first evaluation that observes that id's row.

**A finished Job (H1; scope limitation L9, #301).** `workload restart` of a Job whose current allocation carries a terminal claim is a no-op, as
on `main`: the handler still bumps the generation and clears the stop sentinel (`restart_workload`, `handlers.rs:940-989`, unchanged), and the
reconciler issues nothing because the generation is not pending against that row. The rows this covers: a Job that completed or failed (the
natural-exit idempotency guard at `workload_lifecycle.rs:1091-1104` already emits nothing for them), and an operator-stopped Job, whose row
carries `Stopped { by: Operator }` as its terminal (the stop's `StopAllocation` carries it, `:850-860`, and the shim threads an action's terminal
onto the row, ADR-0037 § 4; a Job row that carries only the `reason` has no terminal and is replaced exactly as on `main`). The second is the one
that mattered: on `main` the restartable filter selects an operator-stopped row while a generation is pending, so a `RestartAllocation` is
emitted; the shim returns at its `NoChange` fence (R2) with no successor, and the stamp on emit hid that the generation is never handled. A
`workload restart` of a **running** Job is unchanged: the stop of R2 ends it and writes the terminal, and the Job stays down. The handler adds no
answer for the no-op; the misleading `restarted` answer is #301's symptom. The shim's `NoChange` fence stays as a stale-dispatch fence (R2).

**Why a successor is never launched twice for one generation.** The runtime persists the View, then awaits the whole dispatch, including
`driver.start`, inside the tick (`reconciler_runtime.rs:1557-1628`); the lane is held to the end of the tick, and ADR-0102 admits the next
evaluation of the target only after the lane is released and hydrates after admission. So when an evaluation hydrates, the previous launch for
the target has returned, and exactly one of two things is true: it wrote a row (`current_alloc` is that id, its entry equals the desired
generation, nothing is pending), or it wrote none and its outcome is one of the named no-row outcomes of § Launch outcomes, the current
allocation is still the older row, the generation is still pending, and **this** evaluation issues one new id and launches once.

**Relaunch.** The next evaluation places one fresh allocation for the committed generation, charging nothing. This is the one statement of
the rule; every other section, scenario and gate points here. It holds after a superseded launch, a launch lost with the control plane and a
node shutdown, for a first placement, a restart of a `Running` workload and a resume of a stopped one alike (a crash-restart successor is
charged once, § Budget below).

**Budget.** A relaunch of an abandoned generation replacement charges nothing: the path carries the predecessor's `restart_counts` value and
`last_failure_seen_at` unchanged (`workload_lifecycle.rs:1313-1322`), and the predecessor is the same older row. A relaunch of an abandoned
crash-restart successor charges as today and **not twice**: `successor_attempts = predecessor_attempts + 1` (`:1313-1317`) is computed from the
current row, which is still the failed predecessor, so the relaunch assigns the same count the abandoned issue assigned. The backoff gate
(`:1288-1294`) reads the predecessor's `last_failure_seen_at`, which the abandoned issue did not change; only the relaunch's own id gets
`last_failure_seen_at = now` (`:1333`). The abandoned id's `restart_counts` and `issued_generation` entries stay as the reservation.

**Growth of the View.** `restart_counts`, `last_failure_seen_at` and `issued_generation` entries are never pruned (true on `main` for the first
two). An id is issued only when the observed state changed since the last issue: a launch that writes a row (bounded by the restart ceiling per
operator commit), a supersession (one per operator commit or shutdown), an admission refusal (one per lost capacity race, N-ADM), or a
stale-dispatch fence (one per observed state change). A finished Job issues none (H1). No outcome issues an id on every tick while the observed
state is unchanged.

**Operator-stop veto and the other states.** The veto (`:1069`) is `!restart_pending && current is operator-stopped`; after stop -> restart ->
abandoned launch the current row is the old operator-stopped one and the generation is pending, so the veto is overridden and the relaunch
happens. Bug 3 stays: a same-spec `deploy` bumps nothing. R1-crash stays. The Job terminal fence (`:938-951`), `next_evaluation_at` and
`pending_restart_owner` use the same helper.

**Assumption A10** (§ Formal protocol model): after an abandoned launch the reconciler places a fresh allocation for the generation, once.
Discharged by the seeded invariant `restart_generation_is_placed_exactly_once_across_supersession_and_crash` (`overdrive-sim`) **only, pending
DISTILL**: the spec's `canStart` builds in the no-`PENDING` guard (RA2), the premise of A10, so checking A10 against the spec would be circular.

#### Launch outcomes: every one writes a row or is a named no-row outcome (ADR-0337, ADR-0338)

A pending generation skips the backoff gate (`:1288`), carries the predecessor's attempts, and the runtime requeues `Immediate` after any tick
that emitted an action (`reconciler_runtime.rs:1537-1539`, `:1653-1659`), whatever the dispatch returned. An outcome that returns an error and
writes no row would mint a fresh id every tick, grow the View, fsync and repeat its failing step, with no charge and nothing visible to the
operator. The table covers every `Err` and early return of the two dispatch arms (`crates/overdrive-control-plane/src/action_shim/mod.rs`; the
supersession arm of § Superseding a launch is added).

| # | Arm and point | Outcome today | After this design |
|---|---|---|---|
| S0 | Start `:1756` | `find_prior_alloc_row(..).await?` fails: `ShimError::Observation`, `Err`, no row, no effect taken yet | **N-REC (read side)** |
| S1 | Start `:1757-1763` | prior row `NoChange` (a finalised Job row): `Ok(())` | a row exists for the id; not a no-row outcome |
| S2 | Start `:1791-1793` | admission refusal from `provision_and_inject_netns`: `Err`, no row | **N-ADM** (ADR-0338): no row, uncharged, the wait visible in `describe` |
| S3 | Start `:1800-1844` | any other `GuestNetwork` error: lease retired, torn down, `Failed` row `WorkloadNetnsProvisionFailed { stage: "guest_network" }` | unchanged (row, charged for a Service; a Job's row is final) |
| S4 | Start `:1813` | provision error that is not `GuestNetwork`: `Err`, no row | **N-UNR**: `provision_and_inject_netns` returns only `ShimError::GuestNetwork` (`:1156-1175`) |
| S5 | Start `:1857-1871` | `ensure_intercept_identity` fails (CA signing, audit write, unrecorded failure; `issue_svid.rs:44-70`): lease retired and torn down, `Err`, no row | **`Failed` row** (ADR-0337), reason `MtlsInterceptInstallFailed { stage: "identity_issue", detail }`. A Service: charged, backed off, relaunched. **A Job: final** (S5-J) |
| S5-J | Start, a Job | as S5 (and S3, S6): the Job's row is `Failed` | the Job natural-exit branch finalises any ended Job row (`workload_lifecycle.rs:1091-1113`; `is_natural_exit` `:1757-1775`); `classify_natural_exit_terminal` reaches its default arm (`:1793`) for these reasons, so the Job ends `Failed { exit_code: Some(0) }`, and a Job is placed only while it has no row, so it is never relaunched (J-1). A Job whose start is rejected ends the same way today; the `exit_code: Some(0)` is equally misleading there |
| S6 | Start `:1872-1900` | no driver of the kind composed: synthesised `StartRejected`, `Failed` row `DriverInternalError` | unchanged (row; charged for a Service, final for a Job) |
| S7 | Start `:1904-1910`, `:1936-1940` | duplicate VM owner (`AllocationAlreadyOwned`): `Ok(())`, no new row | **N-IDEM**: unreachable for a freshly issued id; stays an idempotent no-op |
| S8 | Start `:1948` | `Err(other)` from `driver.start`: `Err`, no row | **N-UNR** (DE-1): `VmDriver::start` returns only `StartRejected` and `GuestTransportMissing`, a composition error placement never issues (`driver.rs:272-274`); only a `SimDriver` script produces another. The arm stays an internal typed error. The captured `rejected_network_cleanup` is returned after the row on the `StartRejected` path (`:2189-2191`), the one reachable path |
| S9 | Start `:2043-2097` | the row write fails: `Err` | **N-REC** |
| S10 | Start `:2123-2168` | mTLS install or guest-network activation fails after `Running`: `Failed` row superseding it. Activation `Ok(false)` (`:2151-2152`): the Running row exists, hooks are skipped | a row exists; the fail-stop exit is **N-FAILSTOP** |
| S11 | Start (SD-3) | `StartSuperseded` | **N-SUP**: no row, uncharged, relaunched uncharged |
| R0 | Restart `:2200` | `find_prior_alloc_row(..).await?` fails: `Err`, no row, no effect taken yet | **N-REC (read side)** |
| R1 | Restart `:2200-2202` | predecessor row absent: `HandleMissing`, no row | **N-UNR**: the arm is emitted only for the current allocation of the observed rows, under the lane, and no code deletes an allocation row |
| R2 | Restart `:2203-2210` | predecessor `NoChange` (a Job row with a terminal), or not `Failed`/`Terminated`: `Ok(())`, no successor | **N-STALE**: not reached through the reconciler for a finished Job (H1: a reconciler-side no-emit); the fence fires only when the row changed between hydration and dispatch |
| R3 | Restart `:2216-2218` | empty driver set: `HandleMissing`, no row (a node that booted without the VM driver while a VM workload's intent persists) | the guard is removed; the arm follows the missing-driver path (R8): `Failed` successor row; charged for a Service, final for a Job |
| R4 | Restart `:2229-2239` | admission refusal: `finish_restart(Err)`, predecessor's cleanup runs, no successor row | **N-ADM** (ADR-0338) |
| R5 | Restart `:2240-2262` | provision error that is not `GuestNetwork` | **N-UNR**, as S4 |
| R6 | Restart `:2263-2291` | any other `GuestNetwork` error: `Failed` successor row `WorkloadNetnsProvisionFailed` | unchanged |
| R7 | Restart `:2293-2330` | `ensure_intercept_identity` fails: successor unwound, `finish_restart(Err)`, no successor row | **`Failed` successor row** (ADR-0337), reason as S5, then `finish_restart`; a Job's row is final (S5-J) |
| R8 | Restart `:2332-2344` | no driver of the kind composed: synthesised `StartRejected`, `Failed` row | unchanged |
| R9 | Restart `:2349-2364` | duplicate owner: `finish_restart(Ok)` | **N-IDEM**, as S7 |
| R10 | Restart `:2419-2439` | `Err(other)` from `driver.start`: `finish_restart(Err)`, no row | **N-UNR**, as S8 |
| R11 | Restart `:2465-2526` | the successor row write fails | **N-REC** |
| R12 | Restart (SD-3) | `StartSuperseded` through `finish_restart` | **N-SUP** |
| R13 | Restart `:2554-2574` | `write_alloc_lifecycle` returned `Ok(None)` (the store refused a non-dominating write): the successor is unwound, then `finish_restart` | **N-UNR**: `Ok(None)` is returned only when a row already holds the key with a dominating or equal timestamp (`observation_store.rs:1778-1781`); a freshly issued id has no row |
| R14 | Restart `:2616` (Start `:2151-2152`) | activation `Ok(false)` (the guest-network EXEC gate is at fail-stop, `guest_network.activation_withheld`): `Ok(())` after the Running row is written; the restart arm skips `finish_restart` | **N-FAILSTOP**: the successor's row exists; the skip happens only while the fail-stop ends `serve` (`overdrive-core/src/guest_network.rs:111`); the next boot's reclamation ends every unsupervised VMM |

The no-row outcomes and their handling:

- **N-SUP (supersession).** Caused only by an operator commit or by shutdown. Relaunched by the next evaluation, uncharged. One id per commit or
  shutdown.
- **N-ADM (admission refusal; ADR-0338).** The pool refuses an admission that the reconciler's occupancy check, made at hydration, had allowed
  (`guest_network.rs:541-591`; the cap comparison at `:569`). `AdmissionCapReached` is reachable: other targets admit concurrently up to the cap
  (16,384). `LeaseRetiring` is not reachable for a freshly issued id (the pool is keyed by allocation id, `:548-566`); `PoolExhausted` is not
  reachable in production (the guest prefix is a /16, `guest_transport/mod.rs:594`, with up to 65,533 usable offsets, `guest_network.rs:487-515`).
  *Start arm:* the refusal returns before the lease is taken, so the next hydration reads occupancy at the cap and the reconciler emits nothing
  (`scheduler.rs:115`) until a slot frees. *Restart arm:* the refusal runs the predecessor's cleanup (`finish_restart`,
  `cleanup_restart_predecessor` `:1340-1360`), which retires the predecessor's lease and releases its slot once its stop and teardown succeed
  (`:1201`); the next hydration reads occupancy one lower and the reconciler **emits again**. The bound is **one extra id per lost race**, each
  race being another target's admission of the last slot; it never grows per tick while the pool stays at the cap. A failed predecessor stop or
  teardown leaves the lease `Retiring`, still counted toward the cap (`guest_network.rs:667-669`), so the reconciler emits nothing (its
  row-neutral reclaim pass, `workload_lifecycle.rs:1295-1297`, owns a leased predecessor).
- **N-IDEM.** An idempotent no-op; no rule.
- **N-STALE.** Bounded by the state change that triggers it; it cannot recur on unchanged rows.
- **N-REC.** The row's own write, or the prior-row read before any effect (S0, R0), failed. No row-based remedy exists; the behaviour is
  unchanged from `main`; the exposure is stated once in § Scope limitations and exposures.
- **N-FAILSTOP.** The fail-stop of the guest-network EXEC gate: a row exists, `serve` is ending, boot reclamation owns the rest.
- **N-UNR.** Unreachable, with the evidence in the table (S4, S8, R1, R5, R10, R13).

**Pinned reasons and rows (no new variant).**

- S5, R7: `TransitionReason::MtlsInterceptInstallFailed { stage: "identity_issue", detail }`, where `detail` is the `Display` of the
  `IssueSvidDispatchError` (which includes its source), not of the `ShimError` wrapper. The variant's `stage` is a `String` carrying a closed
  vocabulary (`transition_reason.rs:187`); the vocabulary gains `"identity_issue"`, a documentation change and no type change, so there is no
  rkyv, envelope or `api/openapi.yaml` schema change (regenerate the file only if a doc comment propagates into the description). The identity
  is the intercept's identity (`ensure_intercept_identity`), and the failure is the fail-closed refusal to run without it.
- R3 reuses R8's reason (`DriverInternalError { detail }`, the `Unclassified` `DriverStartFailure` the missing-driver path already synthesises,
  so `From<&DriverStartFailure>` is unchanged and total). Every row above is `state: Failed`, `started_at: None`, `workload_addr: None`,
  `terminal: None` (the reconciler owns `BackoffExhausted`, ADR-0037 § 4), written by the same fail-closed row writer the netns precedent uses
  (`fail_closed_on_netns_provision`, `:586-637`), which emits the lifecycle event and returns `Ok(())`. Before the row is written the arm
  releases what it acquired: the guest network lease (retire, then teardown); a teardown failure does not suppress the row and is returned after
  it, as `:1829-1843` does. The restart arm's row is the successor's (`prior: None`), followed by `finish_restart`, so the exact-old predecessor
  cleanup of ADR-0106 still runs once.
- **Operator rendering.** `overdrive workload describe` shows these through the existing allocation reason rendering
  (`TransitionReason::human_readable`): `mTLS intercept install failed (identity_issue): <detail>` and the `DriverInternalError` wording. No
  render change.
- **Charge.** The row makes the failed id the current allocation, so the next restart of a Service computes from it exactly as for any start
  failure: the id's `restart_counts` entry (the predecessor's `+ 1` for a crash restart, the predecessor's value for a generation replacement,
  `0` for a first placement) feeds the next issue, the backoff gate applies from then on, and a persistent failure ends in `BackoffExhausted`
  after the restart ceiling plus the one uncharged generation replacement. A transient failure recovers on the next successful launch.

**This changes behaviour on `main`** for ordinary first placements and crash restarts: a fresh `StartAllocation` whose SVID issuance fails
returns the error with no row today and the reconciler re-issues each tick; after this design the operator sees a `Failed` allocation with the
reason, and a persistent failure of a Service finalises `BackoffExhausted`.

### Operator surface

`AllocStatusResponse` gains `volume: Option<VolumeBody { size: String }>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`), filled
by `handlers::alloc_status` from the workload intent. `render::workload_describe` gains `render_volume_section` (presence-guarded): `Volume  40GiB
(guest disk: virtio serial ovd-volume)`, the serial being static text. A volume failure is visible through the existing allocation reason
rendering of the new `TransitionReason`. OpenAPI regenerated.

### Operator surface: a workload waiting for capacity (D-1, W-1, W-2, W-4, H2; ADR-0338 for the admission handling, ADR-0336 for the view)

**What the operator sees.** While intent says run, no allocation is running or launching, and the node's guest-network attachments are full,
`overdrive workload describe <id>` says so (the analogue of a Kubernetes pod `Pending` with `PodScheduled=False`). The same state holds for a
first placement or restart on a full node and for an admission refusal after the reconciler's check; they are indistinguishable by design.

**Where `describe` reads from.** `render::workload_describe` renders the `GET /v1/allocs?job=<id>` response (`ApiClient::alloc_status_for_workload`,
`commands::workload::describe`), whose handler is `handlers::alloc_status`. The handler already reads the workload intent, the workload's
allocation rows and, in one call, `state.guest_pool.observe(&alloc_ids)` (the pool's occupancy with the leases of those allocations,
`GuestAttachmentObservation`). The wait is projected from those inputs plus three reads it adds: a point read of `IntentKey::for_workload_stop`
(the key `restart_workload` reads), a point read of `IntentKey::for_workload_generation`, and `ReconcilerRuntime::view_for_workload_lifecycle`.

**Wire (additive, `api::AllocStatusResponse`).** `capacity_wait: Option<CapacityWait>` with `#[serde(default, skip_serializing_if =
"Option::is_none")]`; `CapacityWait { resource: CapacityResource, held: u32, cap: u32 }` (`Debug Clone PartialEq Eq Serialize Deserialize
ToSchema`); `CapacityResource` is `#[non_exhaustive]`, serde kebab-case, with one variant, `GuestNetworkAttachments` (`as_str()` returns
`"guest-network-attachments"`, enum-owned). JSON: `"capacity_wait": {"resource": "guest-network-attachments", "held": 16384, "cap": 16384}`.
`held` is `GuestAttachmentOccupancy.held` (it counts retiring leases, which still occupy the cap, ADR-0133 D-295-R7); `cap` is
`MAX_GUEST_NETWORK_ATTACHMENTS`. There is no start time and no history (W-1, L6, #265). The wait is visible only in `describe`; `workload list`
does not exist (W-4, L8, #367). The response type is not persisted. `api/openapi.yaml` gains the `capacity_wait` property and the `CapacityWait`
and `CapacityResource` schemas, regenerated with `cargo run -p overdrive-control-plane --bin openapi -- generate`. `overdrive workload describe`
has no `--json` mode, so the field is carried by the API response only.

**Human rendering (`render::workload_describe`).** A presence-guarded section `render_capacity_wait_section`, printed immediately after the
kind-aware body:

```
Waiting  guest-network attachment capacity is full on this node (16384 of 16384 attached); placement resumes when one frees.
```

When `snapshot.capacity_wait` is `Some`, `commands::workload::describe` sets `empty_state_message` to empty (the generic "check that a node is
eligible" text is wrong here). When it is `None` the output is byte-identical to today.

**Where each half is tested (B-3, user ruling 2026-11-19).** The capacity-wait composition test in `overdrive-control-plane` asserts the
response's `capacity_wait` (`CapacityWait { resource, held, cap }`) and nothing about rendering; the control plane takes no `overdrive-cli`
dev-dependency. The rendered `Waiting` line and the emptied `empty_state_message` are pinned by the `overdrive-cli` render tests over a
hand-built snapshot.

**The derivation: one pure predicate shared with the reconciler (H2).** The wait is "this workload's start is blocked **only** by capacity",
computed by one pure function in `overdrive-reconcilers`, beside `current_alloc`, `restart_after_failure` and `generation_replacement_pending`,
and built from them:

```
pub fn start_blocked_only_by_capacity(
    kind: WorkloadKind,
    run_desired: bool,                       // the stop sentinel is absent (restart_workload clears it: a resume is "run")
    rows: &[&AllocStatusRow],                // this workload's observed allocation rows
    view: &WorkloadLifecycleView,            // the reconciler's View for workload/<id>
    desired_generation: u64,                 // the committed generation, absent reads 0
    observation: &GuestAttachmentObservation // one snapshot of occupancy and leases (ADR-0134)
) -> bool
```

It is true iff all hold, in the order of the reconciler's own rules (`workload_lifecycle.rs`):

1. `kind` is not a Schedule (the arm is written out: `WorkloadKind::Schedule => false`, never a wildcard; describe rejects a Schedule) and
   `run_desired` (`:843`).
2. No row that the reconciler treats as active is `Running` (`:986-987`), and the **current** row is not `Draining` (`:1009`: the reconciler reads
   the current row only, so a historical `Draining` row beside a newer terminal row does not block).
3. `observation.occupancy.at_cap()`.
4. `current = current_alloc(rows)`. With no row (a first placement) the predicate is true. Otherwise `current` holds no lease in
   `observation.leases`: a leased predecessor at the cap is the reconciler's row-neutral reclaim pass (`:1295-1297`), already shown as
   `network_cleanup_pending`; that is progress, not a wait.
5. `pending = generation_replacement_pending(view, desired_generation, Some(current))` (ADR-0335, with the finished-Job exception).
6. By the class of `current`, mirroring the reconciler's order:
   - a Job and **any** row `is_natural_exit` (the Job natural-exit branch scans every active row, `:1091-1104`; it finalises or idles and never
     starts): false;
   - `is_operator_stopped(current)` (veto, `:1069`): `pending`;
   - `is_intentionally_stopped(current)` otherwise (a `Stopped { SystemGc }` row: not vetoed, not restartable, so it falls through to a fresh
     placement, `:1146-1148`, `:1348`): true;
   - `is_restartable(current)` (a crashed row, `Stopped { LivenessProbe | StartupProbe }`, a platform-reclaimed row, a `BackoffExhausted` or
     `ServiceFailed` row): `restart_after_failure(kind, run_desired, current, attempts, pending)` with `attempts` read from
     `view.restart_counts` (the decision `:1175-1260` takes: a row at the ceiling with no generation pending is blocked for good; a pending
     generation reopens it);
   - anything else (including a `Pending` row): false.

Blocked for good (false): a Job with a terminal claim and no generation pending (never pending, H1), `BackoffExhausted` with no generation
pending, and an operator-stopped row with no generation pending. Waiting at the cap (true): a restarted workload, `Stopped{LivenessProbe}`,
`Stopped{SystemGc}` and a crashed row under budget. A backoff window still open does not make the predicate false.

**Where "generation pending" comes from in the handler.** `handlers::alloc_status` gains two reads: (1) `state.store.get(IntentKey::
for_workload_generation(&id))` decoded with `TxnOp::decode_counter` (absent reads 0), the same read `hydrate_desired` does
(`workload_lifecycle.rs:667-678`); (2) `state.runtime.view_for_workload_lifecycle(&target)` (`reconciler_runtime.rs:486`, an existing `pub`
accessor returning an owned `WorkloadLifecycleView`, the default when absent), `target` being `workload/<id>`. Both go into the predicate
unchanged; the handler computes no pending flag itself, so the one `<` of ADR-0335 lives in `generation_replacement_pending` alone. The View is
read whole because the predicate also needs `restart_counts`. The rows, the pool, the stop key, the generation and the View are separate reads,
so one `describe` can combine adjacent instants; the next `describe` is correct.

`GuestAttachmentOccupancy::at_cap(self) -> bool` (`held >= MAX_GUEST_NETWORK_ATTACHMENTS`) is new in `overdrive-core::guest_network` and is the
one definition the **four** existing comparisons use (`scheduler.rs:115`, `workload_lifecycle.rs:475` and `:1298`, and the pool's own admission
at `guest_network.rs:569`) and the predicate uses.

**Over-report, stated exactly.** Three bounded cases, all only while the pool is at its cap. (1) A restart's backoff window is still open.
(2) Another limit also blocks the workload (no healthy node, CPU or memory `NoCapacity`, `workload_lifecycle.rs:1353-1367`): the predicate reads
the pool only, so a freed slot does not place it; showing those causes is #261's (L7). (3) A launch is in flight: a launch takes its
guest-network lease before its allocation row exists and holds it until the row is written, up to the launch's own bound (about 30 s for a VM,
about 90 s with a volume nothing supersedes), so when it takes the last slot `describe` can show `Waiting` for a workload that is starting.
Window (3) is bounded and clears when the row is written. After a slot frees, a withheld workload is placed by the 30 s resync pass (no
observation row changes for it), so the line can persist up to one period after the cap clears.

**Source: derived at read time, not persisted (chosen).** "Waiting now" is current state: it holds exactly while the inputs say so and cannot be
stale, because nothing is stored ("Persist inputs, not derived state"). It adds no action, row, envelope, wake or View field, and reads the same
fact (`at_cap`) the reconciler gates on. A durable condition row is rejected (ADR-0336). A refusal that has cleared leaves nothing: it charged
nothing, lost nothing, and the workload was then placed; the log event `guest_network.admission_refusal` stays for diagnosis.

**Read-port.** Existing: `overdrive_core::traits::GuestAttachmentView::observe(&self, allocs: &[AllocationId]) -> GuestAttachmentObservation`
(ADR-0134). Production implementation `GuestPoolAttachmentView` (`overdrive-control-plane/src/guest_network.rs`); `Sim*` counterpart
`overdrive_sim::adapters::SimGuestAttachmentView`. The handler keeps its present call `state.guest_pool.observe(&alloc_ids)` and takes
`.occupancy` from the snapshot it already reads for `network_cleanup_pending`; no new port, no change to `AppState`.

**Not a gate.** The projection writes nothing, delays no state and gates nothing: `Not applicable` beyond G-1 to G-3.

**Other capacity waits (checked).** The volume pool's `InsufficientSpace` is not a wait: it is a typed `VmVolumeUnavailable` start failure, a
charged `Failed` row an operator must resolve by provisioning; a held claim waits at most 30 s inside one launch and ends as `Held`, also a row.

### Decisions on the waiting state

W-1, W-2, W-4 and H2 are recorded in ADR-0336 and in [`design/wave-decisions.md`](design/wave-decisions.md); W-3 (the fill-the-pool test's
composition) is a test decision owned by DISTILL (`distill/upstream-issues.md`).

### Composition root

`run_server` order: cgroup preflight, probe-runner gate, `compose_volume_backing` (probe), `compose_vm_driver(volumes)`, `registry.insert`,
`boot_server`.

- `VmAdapters` gains `volume_pool: Arc<dyn VolumePoolProbe>`. The pool travels with the VM adapters, not `BootAdapters`: only the VM driver
  consumes it, and a simulated VMM travels with its simulated pool the way it does with its beacon (DG-4). The field is required: no `Default`,
  no builder. `NodeAdapters::host` sets `Arc::new(overdrive_host::LvmVolumePoolProbe::new())`; a fixture that composes no pool passes
  `SimVolumePoolProbe::absent()`, and one that does passes `SimVolumePoolProbe::composed(..)`. `BootAdapters` is unchanged.
- `async fn compose_volume_backing(probe: &dyn VolumePoolProbe) -> Result<Option<Arc<dyn VolumeBacking>>, ControlPlaneError>` calls
  `probe.probe()`. `None` logs `volume_backing.not_composed` and returns `Ok(None)`. `Err(source)` logs `health.startup.refused` with reason
  `volume_backing.probe` and returns the dedicated variant `ControlPlaneError::VolumeBackingBoot(VolumeProbeError)` (never `Internal`). The
  refusal does not depend on `VmCapability`: a present but broken pool refuses on a node whose VMM probe is `Optional` too. `run_server` calls it
  with `vm.volume_pool.as_ref()` before it hands `vm` on.
- `compose_vm_driver` gains `volumes: Option<Arc<dyn VolumeBacking>>`, placed after `vm`; it destructures `volume_pool: _` out of `VmAdapters`.
- `VmDriver::new` gains a tenth and last parameter `volumes: Option<Arc<dyn VolumeBacking>>`, after `guest_listener_board`; it is mandatory
  (`None` states that no pool is composed). A start whose payload carries a volume while the driver holds none fails `VolumeUnavailable {
  PoolAbsent }`; a start without a volume never touches the pool.

### Provisioning obligation (VD-4, VD-10)

Overdrive only verifies the volume pool (ADR-0313); the node's provisioning creates it. DELIVER extends the two provisioning owners that exist
today:

- **The metal test host:** `infra/metal/partitions.json` and `infra/metal/provision.sh` create the wiped partition labelled `overdrive-volumes`,
  the physical volume and the volume group `overdrive` (4 MiB extents).
- **Guest-kit test guests.** Tier-3 volume tests run inside guest-kit guests, so each such guest has its own volume pool: the guest's kernel
  provides `CONFIG_BLK_DEV_DM`, `CONFIG_EFI_PARTITION` and `CONFIG_DEVTMPFS` (the guest kit's kernel is the metal host's own Linux 7.0 kernel);
  its staged rootfs carries the `lvm` binary with its library closure and a writable `/run`; and it is attached a second block device, a backing
  disk holding a wiped GPT partition labelled `overdrive-volumes` with the physical volume and the `overdrive` group, created by the preparation
  step (never by Overdrive) and fresh for each guest. The obligation sits in `xtask/src/guest_kit_prepare.rs` (prepares the backing disk at a
  target-scoped path), `crates/overdrive-testing/src/guest_kit.rs` (stages the `lvm` closure and attaches the disk) and
  `crates/overdrive-testing/src/placement/groups.rs` (the closed registry; the volume Tier-3 tests run in `cgroup-default`, profile
  `volume-pool`). A guest of any other profile has no pool and composes none (the soft case of the probe).
- **The appliance image:** none exists yet (GH #75, L5); not a precondition.

## [REF] Test infrastructure decisions

P-1 and P-2 were approved by the user on 2026-11-17; P-3 on 2026-11-18.

**P-1: the `volume-pool` guest profile.** The closed placement registry (`crates/overdrive-testing/src/placement/groups.rs`) gains one profile,
`volume-pool`, in the `cgroup-default` group. A guest of this profile is a `nested-launch` guest (it can boot `serve` and launch a workload VM)
that also has the volume pool as § Provisioning obligation states, plus `dmsetup` and the `dm-flakey` module with its loader (P-3). Every
Tier-3 volume test is placed `guest = "cgroup-default", profile = "volume-pool"`, except S-VOL-T3-01 (the guest with no pool), which stays on the
base profile. DISTILL adds the registry entry (the profile boots without the pool until DELIVER's step lands, so those tests are RED on "the
probe finds no labelled partition"); DELIVER's first step adds the provisioning, so the Tier-3 tests are runnable and proven RED before the real
LVM adapters are written.

**P-2: an EDD expectation for the operator journey.** `verification/expectations/E16-node-local-volume-survives-restart-and-reboot`: `overdrive
deploy` a VM workload with `[vm.volume]`, write data in the guest, `overdrive workload restart`, reboot the node, read the data back, and run
`overdrive workload describe`. Anchors: VD-2, VD-8, VD-9. DISTILL authors the expectation; DELIVER or DEVOPS captures its evidence under
`.claude/rules/verification.md` (a native-metal capture runs `--exclusive`). A reboot of the shared metal host is physical maintenance and needs
the owner's authorisation.

**P-3 (approved 2026-11-18; relabelled 2026-11-19, P-3a): one real-kernel failed-flush test, S-VOL-T3-11.** Claim: in the production
write-back attachment (ADR-0315), with nothing dirty in the guest or host cache, a flush that the volume's host device fails reaches the
guest as an `fsync` error through the real VMM. This is the durability guarantee of the default mode. It holds on stock Cloud Hypervisor on
every engine (spike `run-022` and `run-030`: `a3_fault_wb_explicit_flush_*`, `errs_after_injection=20`), so the test does not guard F1. F1
(ADR-0316) changes only the write-through path, a failed post-write fsync, which a guest reaches by switching its virtio-blk cache mode; the
spike reached it only with host page caching on, because with direct I/O to the device no stock dm target produces a write that succeeds
followed by a flush that fails. F1 is upstream's own behaviour (ADR-0316 amended 2026-11-20), so no fork-side regression test exists for it (§ Validation, V-3). Faults enter only at
the kernel device. The fixture mechanics (placement, the `dm-flakey` table rewrite, the write-cache precondition, the guest-image addition,
cost): `distill/upstream-issues.md` § "P-3 fixture".

- **Kernel behaviour relied on** (`vendors/linux`, `v7.0`), each link read in the source:
  - `fsync` on a block device issues a flush. `blkdev_fsync` (`block/fops.c:613-633`) writes back dirty pages (`:619`; none after `O_DIRECT`
    writes), then calls `blkdev_issue_flush` (`:628`), which submits an empty `REQ_OP_WRITE | REQ_PREFLUSH` bio and waits
    (`block/blk-flush.c:470-476`). `-EOPNOTSUPP` alone is turned into success (`fops.c:629-630`); an I/O error is returned.
  - The flush reaches the virtio device only while the guest disk has a write cache: the block layer strips `REQ_PREFLUSH` and completes an empty
    bio with `BLK_STS_OK` when the device has none (`block/blk-core.c:817-827`), and virtio-blk sets `BLK_FEAT_WRITE_CACHE` from
    `VIRTIO_BLK_F_CONFIG_WCE` / `VIRTIO_BLK_F_FLUSH` (`drivers/block/virtio_blk.c:1068-1084`, `:1505-1506`). This is the write-back disk of VD-6.
  - On the host the flush reaches the table. The dm device inherits `BLK_FEAT_WRITE_CACHE` from the device under its table
    (`BLK_FEAT_INHERIT_MASK`, `include/linux/blkdev.h:353-356`; applied in `block/blk-settings.c:799`), so the block layer does not strip the
    flush at the dm device (`blk-core.c:821-826`). `dm-linear` marks `flush_bypasses_map` (`drivers/md/dm-linear.c:65`) but `dm-flakey` does not,
    so a flakey table sends the flush through the targets (`drivers/md/dm.c:1549-1561`).
  - `dm-flakey` declares a flush bio: `ti->num_flush_bios = 1` (`drivers/md/dm-flakey.c:326`), so `dm.c:1554-1555` does not skip it; `__map_bio`
    calls the target's `map` on the flush clone (`dm.c:1426`).
  - `dm-flakey` errors that flush. In `flakey_map` (`dm-flakey.c:493-569`) the down test `elapsed % (up + down) >= up` (`:505-506`) is always
    true with `up = 0` (the constructor requires `up + down` non-zero, `:304-308`). An empty flush bio has `bio_data_dir == WRITE`
    (`include/linux/bio.h:46-47`), so it takes the write branch: with `error_writes` set (`dm-flakey.c:114`, `:530`) the target calls
    `bio_io_error(bio)` and returns `DM_MAPIO_SUBMITTED` (`:530-533`).
  - Host side: the volume disk is `direct=on` (VD-6), so the guest's earlier data writes are not dirty in the host page cache and the host
    `fsync` finds no data to write back; its only error source is the explicit flush, which stock Cloud Hypervisor already reports to the
    guest (spike `run-022`, `run-030`).

## [REF] Observable projections the conformance driver reads

Two owners, two specs, two conformance drivers. The volume protocol's owner is the VM launch path (`VmDriver::start`), spec
`specs/quint/node-local-volume/`, driver `quint_node_volume.rs`; the tables up to "Restart-generation projections" are its projections. The
restart-generation protocol's owner is `WorkloadLifecycle` driven through `ReconcilerRuntime`, spec `specs/quint/restart-generation/`, driver
`quint_restart_generation.rs`; its projections are the last table.

| Model variable | Production accessor |
|---|---|
| `lv[w]` exists and `active[w]` | `VolumeBacking::claim_state(&w)` under the **projection rule `lv ∧ active ⇔ claim_state ≠ NotFound`**. A volume that exists but is inactive (the state after every node reboot) has no `/dev/mapper/overdrive-ovd--<hash>` node, so the real adapter's `O_EXCL` open fails `ENOENT` and reports `NotFound`: `claim_state` cannot tell `lv ∧ ¬active` from `¬lv`. The driver therefore compares the model's `lv ∧ active` with `claim_state ≠ NotFound`, and observes `lv ∧ ¬active` by Tier-3 read-back. `SimVolumeBacking` must report `NotFound` for as long as it models a volume as inactive, so sim and real agree. |
| `claimed[w]` (a VMM holds the device) | `VolumeBacking::claim_state(&w)` (`Held`; `Free` otherwise, for an active volume), with **`SimVolumeBacking` the named arbiter** (the kernel's claim table is a driven-port fact the owner does not hold). Paired Tier-3 test, on `LvmVolumeBacking` against a scratch volume: held by an `O_EXCL` opener reports `Held`; after release reports `Free`; deactivated (`lvchange -an`, no device node) reports `NotFound`, and `Free` again once re-activated. |
| `alloc[a].state` | `ObservationStore::alloc_status_rows()` (`state`) |
| `exhausted[w]` (BackoffExhausted reached) | `ObservationStore::alloc_status_rows()`: a row whose `terminal` is `BackoffExhausted { attempts }` (the source `handlers::restart_budget_from_rows` already reads) |
| `charged[w]` | model-only. The budget is the reconciler's private per-chain `restart_counts` (`workload_lifecycle.rs:1289-1296`), so counting `Failed` rows or reading the row's `restart_count` (ADR-0078) is not the budget. `no_drain_failure_charges_budget` is observed through rows instead: while the predecessor's drain is within the 30 s bound, no allocation row of the workload records a start failure, and `BackoffExhausted` is not reached. |
| `holder[w]` (which allocation holds the claim) | model-only (the kernel does not record it) |
| `data[w]` (the volume's contents survive) | model-only; Tier-3 read-back through `serve` + `deploy` |

Every other volume-spec variable is model-only and not compared: `storeUp` and `spaceFree` (fault-injection inputs), `vid`, `nCreated`, `dv`,
`maxDv` (Tier-3 read-back), `ph`, `waited`, `orphan`, `why`, `ran`, `faults`, `slow`, `drainFail`, `declared` and `stopped` (the effects of
`submit_workload`, `stop_workload` and `restart_workload`, which the driver calls), and `conf_step`. `alive` is projected for an allocation whose
launch is booting (`ph[a] == LP_BOOT`) by the `alive[a]` row below; for every other `ph` the driver does not compare it.

The conformance driver composes the owner through the pinned seams: `SimVolumePoolProbe::composed(sim)` → `probe()` → `VmDriver::new(..,
volumes)`; `CpCrash` rebuilds the driver by probing the same sim again. Its driving ports are `VmDriver::start` and `VmDriver::stop`, the shim
dispatch, the handlers `stop_workload` / `restart_workload`, `ReconcilerRuntime::supersede_lane` / `supersede_all_lanes`, and the sim fault
methods; the payload it submits is `Vm::to_payload` output.

| Model variable | Production accessor |
|---|---|
| `sup[a]` (the launch of allocation `a` has been signalled) | model-only; the driver raises it by calling the handler, `supersede_lane` or `supersede_all_lanes`; the effect shows in the launch's outcome (next row) |
| `down` (the node has begun shutdown) | model-only; the driver sets it by calling `ReconcilerRuntime::supersede_all_lanes`; the owner's observable effect is the open lane's `LaunchSupersession::is_superseded()` for a driver-held lane, read only to assert the action's guard |
| allocation `a` ended by supersession (`why[a] == W_SUPERSEDED`) | the conjunction of three owner observables: `alloc_status_rows()` holds **no row** for `a`; `Driver::live_allocations()` (`fn live_allocations(&self) -> Option<Vec<AllocationId>>`, `overdrive-core/src/traits/driver.rs`) does not list `a`; and the shim's `dispatch` result for the action is `Ok(())`. `from_driver` fails on a production row for a superseded id |
| `delivered[a]` | model-only ghost; the comparison of "no row for a superseded id" at every step already refutes a delivered result |
| `alive[a]` while booting | `VmDriver::live_allocations()` lists `a` and the recording `Vmm` has a created, not terminated VMM for `a` (`SimVmm`'s live-VMM accessor, named as the arbiter of the VMM process, which the owner does not hold; B-2, user ruling 2026-11-19: the recording `Vmm` double stays the arbiter, paired with the restored Tier-3 test S-VOL-T3-13, a stop during guest boot terminates the VMM and keeps the volume) |

**Restart-generation projections** (spec `specs/quint/restart-generation/`; driver `crates/overdrive-control-plane/src/quint_restart_generation.rs`).

| Model variable | Production accessor |
|---|---|
| `dg` (committed desired generation) | `WorkloadLifecycle::hydrate_desired(..).generation` (the value at `workloads/<id>/generation`); the driver sets it only through `restart_workload` |
| `stopped` | `WorkloadLifecycle::hydrate_desired(..)` field `desired_to_stop`; the driver sets it through `stop_workload` and clears it through `restart_workload` |
| `minted[a]` and `ig[a]` | `ReconcilerRuntime::issued_generation_for_alloc(&self, target, alloc_id) -> Option<u64>`: `None` for an id the reconciler never issued, `Some(g)` with `g == ig[a]` for an issued id, with or without a row |
| `hasRow(a)` and the row's state | `ObservationStore::alloc_status_rows()` lists `a` and its `state`; an issued id with no row projects as "minted, no row" |
| `cnt[a]` (the per-allocation restart count at issue; R-1) | the existing `ReconcilerRuntime::attempt_index_for_alloc(&self, target, alloc_id) -> u32` (`reconciler_runtime.rs:544`): `cnt[a] == attempt_index_for_alloc(target, a) - 1` for every id `a` for which `issued_generation_for_alloc` is `Some`. No new surface |
| `charged` | not a state variable: the spec derives it from `cnt` and the current row, both projected, so it is compared by construction (the carry relation is checked at every `WlStart`) |
| `exhausted` | `alloc_status_rows()`: the current row (the highest id with a row, `current_alloc`) carries `terminal == BackoffExhausted { attempts }`, read after the evaluation that observes that row has run; the model's `exhausted(s)` must equal it at every step. It is the one comparison that checks the production ceiling against the model's `LIMIT` (= ceiling + 1) |
| `why[a] == W_PREFAIL` | `alloc_status_rows()`: a `Failed` row for `a` whose `reason` is `MtlsInterceptInstallFailed { stage: "identity_issue", .. }` or `DriverInternalError { .. }`; `Driver::live_allocations()` does not list `a`; the dispatch result is `Ok(())` |
| `why[a] == W_REFUSED` (D-1) | no row for `a`, `Driver::live_allocations()` does not list `a`, the dispatch result is `Err(ShimError::GuestNetwork(AdmissionCapReached { .. }))`, and `GuestAddressPool` occupancy is unchanged by `a` |
| `why[a] == W_LOST` (a launch with no row and no failure: superseded, or lost with the control plane) | `alloc_status_rows()` holds no row for `a`, `Driver::live_allocations()` does not list `a`, and the held tick is over: its future completed (dispatch `Ok(())`, the supersession outcome) or was dropped by a rebuild (`CpCrash`, `NodeReboot`) |
| `as[a] == PENDING` | the id is issued (`issued_generation_for_alloc` is `Some`), `alloc_status_rows()` has no row for it and the `run_convergence_tick` future the driver holds at its seam has **not** completed (the production tick is awaiting the dispatch, holding the lane). The driver reads the future's completion, the owner's own behaviour, not a map it maintains; a rebuild leaves no held tick, so a `PENDING` expectation after a rebuild fails |
| `JOB` and `jobTerm(a)` | `JOB` is model-only (it selects the workload kind the driver submits). `jobTerm(a)` is `alloc_status_rows()` row `a` with `kind == WorkloadKind::Job` and `terminal.is_some()`, the test `generation_replacement_pending` and the shim's `allocation_attempt_transition` use |
| `faults`, `declared`, `idFault`, `capFull`, `conf_step`, `og`, `gl` | model-only: `idFault` is armed on the sim CA, `capFull` by filling the sim pool, `declared` is the effect of `submit_workload`, `conf_step` records the action taken for the driver and is never compared, `og` has no production counterpart, `gl` is checked through the carry relation of `cnt` |

Driving ports of the restart-generation actions (every `switch!` arm calls one): `Declare` -> `submit_workload`; `OpStop` -> `stop_workload`;
`OpResume` and `WorkloadRestart(a)` -> `restart_workload`; `WlStart(a)` -> one convergence tick whose emitted `StartAllocation` or
`RestartAllocation` dispatch is held at a test-owned seam (DISTILL shapes it; if it needs production surface DISTILL returns a testability
request and invents none); `LaunchRunning`, `LaunchStartFail`, `LaunchPreFail` and `LaunchRefused` -> release the held dispatch through the shim's
`dispatch` with the sim CA, `SimDriver` start result and sim guest-network pool armed for that outcome; `LaunchLost(a)` ->
`ReconcilerRuntime::supersede_lane(target)` on the held dispatch with neither `capFull` nor `idFault` armed (the guard of the model's
`LaunchLost`), whose `SimDriver` start observes the lane signal and returns `StartSuperseded` (no row, dispatch `Ok(())`); `StopAlloc(a)` -> the tick that emits `StopAllocation` and its dispatch; `VmmCrash(a)` -> the
`SimDriver` exit event that the exit observer turns into a `Failed` row; `JobExit(a)` -> the `SimDriver` exit of a Job's process; `CpCrash`,
`NodeReboot` -> rebuild the runtime and the stores (`kill_for_test`, View and stores reopened, boot reclamation run); `IdFaultOn/Off`,
`CapFill/Drain` -> sim arming. A `LaunchLost` and a `CpCrash` leave the same observable: an issued id with no row and a finished or dropped tick.

Single-writer is discharged by assumption A1 (the kernel claim) plus the Tier-3 pairing test; the model encodes the kernel verdict, it does not
prove it.

## [REF] Lifecycle Gate Ownership

### State ownership

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| allocation `Failed` with `VmVolumeUnavailable` | `VmDriver` start result, written by the shim | this launch attempt could not attach the volume | G-1, G-2 | the predecessor's terminal state, cleanup, the restart budget policy |
| allocation `Running` | driver start plus guest beacon | the guest reported ready | VMM boot, G-1, G-2 as launch preconditions | volume existence after launch |
| volume claim and lock | the kernel and the VMM | exactly one writer holds the device | CH's own open | allocation row state |
| allocation `Failed` with a pre-driver reason (`MtlsInterceptInstallFailed { identity_issue }`, `DriverInternalError`) | the action shim's `StartAllocation` / `RestartAllocation` arms | this dispatch could not bring the allocation to the driver; it is recorded; a Service's is charged like any start failure and a Job's is final | the identity issuer and the driver registry | the predecessor's terminal state and cleanup, other workloads' allocations, the restart-budget policy |
| "generation pending" (derived) | `WorkloadLifecycle`, from the observed current allocation and the View's issuance record | the committed generation has no observed successor | allocation rows, `issued_generation` | the shim's dispatch outcome, any View marker of what was emitted |
| lane supersession signal | the convergence owner's lane for `workload/<id>` (ADR-0102 as amended by ADR-0334); its token is a child of the lane table's one shutdown token. Raised by `stop_workload` / `restart_workload` (the lane's own token) and by node shutdown (`supersede_all_lanes`, the parent) | an operator stop or restart committed, or the node began shutting down, while this evaluation held the lane | those two handlers' commits and the convergence loop's shutdown arm | the restart budget, any allocation row, any other evaluation of the lane (`ServiceLifecycle`, `SvidLifecycle`), probes, a `Running` allocation, readiness, reclamation, the pre-feature path of a driver that ignores it |

### Gate G-1 — `VolumeEnsure`

- **Existing evidence:** ADR-0104, ADR-0318; production path `submit_workload` -> `WorkloadLifecycle::reconcile` -> `schedule` ->
  `StartAllocation`/`RestartAllocation` -> shim -> `VmDriver::start`.
- **Owner:** the launch path of `VmDriver`, through `VolumeBacking::ensure`.
- **Promise:** the volume exists, is activated and its device path is returned.
- **Affected state:** the result of this launch (proceeds to G-2 or returns a typed start failure).
- **Failure projection:** `PoolAbsent`, `InsufficientSpace`, command failure, and a command past its deadline (a timeout is a command failure) ->
  `VolumeUnavailable`, recorded as the allocation's `Failed` row; duplicate or repeated `ensure` is idempotent; a launch signalled by a stop or
  restart returns `StartSuperseded` at observation point 2 (G-3), whatever `ensure` returned, and `ensure` itself is never interrupted.
- **Explicitly unaffected:** running allocations, stop, GC, probes, readiness, `WorkloadLifecycle` placement.
- **Ordering:** after the supervision claim and observation point 1; before the claim wait, the first host artifact and `Vmm::create`; consumes
  the one `ensure` deadline (30 s, covering all its `lvm` spawns, enforced by the adapter, § Port), additional to the 30 s claim wait and the 30 s boot wait of an unsuperseded
  launch.
- **Counterexample:** gating placement on volume existence (a `WorkloadLifecycle` gate with a status row) adds a row family, an interest and a
  gate for a create that is already convergent at the point of use (ADR-0318).
- **Evidence lane:** unit with `SimVolumeBacking`; seeded sim; Tier 3 through `serve` + `deploy`.

### Gate G-2 — `VolumeClaim`

- **Existing evidence:** ADR-0109 (terminal predecessor precedes successor), ADR-0106 (cleanup does not gate the successor), ADR-0317;
  production path shim `RestartAllocation` -> `VmDriver::start`.
- **Owner:** the launch path of `VmDriver`, through `VolumeBacking::claim_state`.
- **Promise:** at the instant of the attempt the volume's kernel claim was free.
- **Affected state:** the result of this launch.
- **Failure projection:** bound passed -> `VolumeUnavailable { Held }`, as the allocation's `Failed` row; any `Err` from `claim_state` (including
  `NotFound` for a volume `ensure` just activated, a bug) -> `VolumeUnavailable { PoolCommandFailed { operation: Claim } }` at once.
- **Explicitly unaffected:** the predecessor's terminal state and cleanup (ADR-0106), guest network lease, identity, restart-budget policy, the
  boot deadline value.
- **Ordering:** after `ensure`, before the first host artifact and `Vmm::create`; its 30 s is additional to `VM_BOOT_DEADLINE` (30 s) and runs
  inside the one convergence lane the launch already holds (ADR-0102); one wait per attempt. Each poll iteration and each poll sleep observes the
  lane signal (points 3 and 4); a stop or restart for the workload ends the wait at once and the `Held` projection applies only while the signal
  is not raised.
- **Counterexample:** waiting for VMM process exit releases the successor 2.4 s early (spike B2b) and fails it; deferring the restart in a
  reconciler would mint an allocation per poll.
- **Evidence lane:** seeded sim with `SimVolumeBacking` delayed release; Tier 3 on metal with a delayed in-flight write; the model runs
  `no_space_fails_typed_and_never_attaches`, `launch_bound_exceeded_fails_typed_and_charges_budget` and
  `store_absent_fails_typed_and_never_attaches`, which assert that a typed failure never attaches.

### Gate G-3 — `LaunchSupersession` (SD-1 to SD-5)

- **Existing evidence:** ADR-0102 (active-target exclusivity, "admission precedes hydration", "never cancels an active effect"),
  `.claude/rules/reconcilers.md` § "Superseded in-flight work runs to its bound" (cancellation is right before an effect starts and at a step
  boundary, through a cooperative token; a superseded result is discarded); production path `stop_workload` / `restart_workload`
  (`handlers.rs:855`, `:940`) -> `IntentStore` commit -> `ReconcilerRuntime::supersede_lane` -> open lane of `workload/<id>` -> shim
  `StartAllocation` / `RestartAllocation` (`action_shim/mod.rs:1872`, `:2332`) -> `VmDriver::start`. Shutdown path: `ServerHandle::shutdown` /
  `FailedBootTasks::shutdown` / the `run_server` early return (`lib.rs:3079`, `:3215`, `:4844`) -> `convergence_shutdown.cancel()` ->
  `spawn_convergence_loop`'s `shutdown.cancelled()` arm (`lib.rs:5237`) -> `ReconcilerRuntime::supersede_all_lanes` -> every lane token -> the
  same shim arms. The shim writes no allocation row before `driver.start` returns, so a launch in flight has no row for a stop to act on
  (`StopAllocation` is emitted only for `Running` rows); `VmDriver::stop` acts only on `Live`.
- **Owner:** the convergence owner's lane (ADR-0102), signalled by the two operator handlers and by node shutdown; observed by the driver.
- **Promise:** if the signal is raised, the launch it covers delivers no result and leaves nothing behind it; if it is not raised, the launch is
  exactly the launch G-1 and G-2 describe.
- **Affected state:** the result of this one launch (`StartSuperseded` instead of `Ok` or a typed failure).
- **Failure projection:** signal raised at any observation point -> `StartSuperseded`, no row, no budget input; teardown failure while abandoning
  -> the existing unclassified `StartRejected` (a `Failed` row); signal raised with no lane open -> nothing (the evaluation's hydration reads the
  committed intent); signal after `accept_ready` was selected -> the launch succeeds and the stop is evaluated against its `Running` row;
  duplicate, repeated or late signals behave as one; disconnect (a control-plane crash) -> the lane table is process-local and is rebuilt empty,
  the launch is lost with the process and the next evaluation after boot relaunches it (boundary scenario 5); shutdown begun while the lane is opening, or before it opens -> the lane is born raised, the launch returns
  `StartSuperseded` at its first observation point and writes nothing.
- **Explicitly unaffected:** the restart budget (a superseded launch is not a failure), every allocation row, the predecessor's terminal state
  and cleanup (ADR-0106), the other evaluations of the lane, running allocations, probes, readiness, reclamation, `ensure`'s volume (kept, VD-2),
  a launch whose driver ignores the signal, `submit_workload`, the shutdown drain of admitted evaluations (every admitted evaluation still runs to
  its result, ADR-0102 § Timing; only a launch's waits are abandoned), `Running` allocations at shutdown, `kill_for_test`.
- **Ordering:** *handler:* intent commit, then `supersede_lane`, then `submit_evaluation`. *Owner:* lane opened, then hydration, then reconcile,
  then dispatch. The pairing closes the check-and-act window without a re-check: a commit whose signal finds the lane open cancels it; a commit
  whose signal finds none precedes the lane's open, hence precedes the evaluation's hydration, which reads it. The window that remains is
  conservative: a signal landing after the lane opened but before hydration also cancels a launch the hydration already knew the committed state
  for (a `restart` whose new-generation `Start` then returns superseded). The cost is one extra evaluation per operator commit. The next
  evaluation, from any wake, relaunches (SD-6). *Shutdown:*
  `supersede_all_lanes` is a single `cancel()` of the parent of every lane token; opening a lane is `child_token()` of the same parent; the token
  tree linearises them, so a lane that opens at any instant is either signalled by the cancel or born cancelled. The cancel is permanent for the
  process.
- **Why `submit_workload` never supersedes:** a workload's spec is immutable per id: a byte-identical resubmit is `Unchanged` and a different spec
  at the same id is `409 Conflict` (`handlers.rs:475`, `:524-554`, `:598`), so a submit never changes the desired state a launch in flight was
  started for. Only `stop` and `restart` (and shutdown) do.
- **Counterexample:** raising the signal on *any* evaluation submitted for a busy target is wrong: a predecessor's late exit row, a probe result,
  the 30 s resync or an `EnqueueEvaluation` handoff would abandon a healthy launch, and each re-launch the same way. Likewise shutdown must not
  list the open lanes and cancel each (a lane opened between the list and the cancel is missed), nor be raised by `ServerHandle::shutdown` alone
  (it would miss the `FailedBootTasks` and `run_server` early-return paths).
- **Evidence lane:** unit and seeded sim with `SimLaunchSupersession`, `SimClock` and `SimVolumeBacking` (boundary scenarios 4 and 4a); the lane
  table through the real tick; the loop's shutdown arm through `spawn_convergence_loop` with a parked `SimDriver` start; Tier 3 on metal in a
  `volume-pool` guest through `serve` + `deploy` (a delayed in-flight write holds the claim; `workload stop` returns promptly with no `Failed`
  row; a stop during a guest boot). The tests are listed in `distill/upstream-issues.md`.

### Required boundary scenarios

1. Gate available: `ensure` ok and claim free -> the VMM launches with the second disk and the guest sees it.
2. Gate unavailable: no volume pool, no space, command failure or deadline -> `Failed` row with `VmVolumeUnavailable`; claim held past 30 s ->
   `Failed` row with cause `Held`; both charge the budget and neither leaves a VMM attached.
3. Unrelated state: a running allocation is unaffected by anything on the volume pool; stop and GC proceed.
4. Late success: a stop or restart while a launch waits (claim poll, boot wait) ends the launch at once with `StartSuperseded`: no row, no budget
   charge, no VMM attached or left alive, the volume kept, the supervision claim, the guest network lease and the SVID released; a launch whose
   boot race had already selected `accept_ready` is an ordinary success that the stop then stops; an `ensure` in flight finishes first. A
   superseded launch's result is never delivered.
4a. Supersession, the remaining boundaries: (a) the signal raised with no lane open does nothing and the evaluation's hydration sees the commit;
   (b) a failure found at the same observation point as the signal is reported as `StartSuperseded`; (c) a teardown failure while abandoning
   reaches the shim as the existing unclassified rejection; (d) a driver that ignores the signal is unchanged; (e) a signal for one workload's
   lane never reaches another workload's launch; (f) `submit_workload`, resync and observation wakes never raise it; (g) a restart whose committed
   generation the evaluation already hydrated may see its own `Start` superseded once; the next evaluation relaunches; (h) node shutdown: a
   launch waiting on the claim or the boot, an `ensure` in flight and a lane opened after shutdown began all abandon (the `ensure` first
   finishing), return `StartSuperseded`, write no row and charge nothing, the loop drains and exits, and the next boot relaunches; a `Running`
   allocation and a non-launch evaluation are unaffected; `supersede_lane` never cancels the shutdown token.
5. Disconnect/reconnect: control-plane crash between `ensure` and VMM spawn: the next launch's `ensure` finds the volume and proceeds; a
   surviving VMM is reclaimed, its claim drains, the wait covers it. A launch that died with the control plane leaves no row (the shim writes none
   before `driver.start` returns); the next evaluation relaunches (§ Restart generation, "Relaunch"). A surviving VMM's run directory or clone is reaped by `VmReclamation` (`plan_reclamation`: `DiscardStrandedArtifacts`, or `ReclaimAllocation`
   when a non-terminal row exists), booted by `vm_reclamation_boot::converge`. That sweep runs in `run_server` before the convergence loop, the
   interest router and the HTTP listener start (`lib.rs:4530` before `:4813`, `:4861`, `:5032`), fails the boot closed on error, and ends every
   VMM the restarted process does not supervise, whether or not its allocation has a row (`vm_reclamation_boot.rs:92-148`), so no launch for the
   workload is minted while an orphan VMM is alive (assumption M7).
6. Feature disabled: a workload with no `[vm.volume]` makes no `ensure`, no wait, no extra `--disk`; a node with no volume pool composes
   unchanged.
7. A Job's failure before the driver is final (J-1): the Job's `Failed` row is finalised `Failed { exit_code: Some(0) }` and no further launch
   is issued, where a Service's is charged and relaunched.

Named seeded `overdrive-sim` invariants: `single_writer`, `data_survives_restart`, `no_drain_failure_charges_budget`,
`restart_generation_is_placed_exactly_once_across_supersession_and_crash`; `assert_eventually!` a replacement attaches once the predecessor's
claim drains. Tier 3 (metal, guest group `cgroup-default`): create, write, `workload restart`, read back; a node reboot and read back; a delayed
in-flight write replacement; the `Held`/`Free`/inactive pairing test; a broken-substrate probe catalogue.

## [REF] Formal protocol model

**The model is two specs (user ruling 2026-11-18).** Each protocol has its own spec, hazards and conformance driver: the two in one state machine
exceeded the validation budget. Each spec's `README.md` states its abstraction; its `evidence/summary.md` holds every verdict, timing and memory
figure; `cargo xtask quint verify-evidence` is the authority on whether the recorded evidence matches the spec.

| Item | `node-local-volume` (ADR-0310 to 0320, 0334) | `restart-generation` (ADR-0335, 0337, 0338) |
|---|---|---|
| Decisions modelled | VD-2, VD-8, VD-9, SD-1 to SD-5 | SD-6, SD-6b/J-1, D-1, H1, R-1 |
| State | the volume and claim, the launch phases, `alive`, `orphan`; the lane signal `sup` and `down`; ghosts `afterSup`, `delivered`, `chg`; the restart budget by the **shared rule** `specs/quint/restart-generation/restart_budget.qnt` (imported, not reimplemented): the per-allocation count `cnt` and the staleness `old` (issued for an earlier generation) of the current row and a `PENDING` launch, from which `charged` and `exhausted` are derived; `exh` materialises `exhausted` for the driver. A generation bump (`OpResume`, `WorkloadRestart`) makes every issued allocation stale | the generation `dg`, the issued generation `ig` and count `cnt` per id (the View's `restart_counts` at issue; `charged` and `exhausted` are derived), ghost `gl`, hazard marker `og`, environment flags `idFault` and `capFull`, the constants `JOB` and `LIMIT` (= the production ceiling + 1). **No lane signal**: for the generation rule a superseded launch and a lost launch are identical (no row, no charge) |
| The launch | phases `LP_*`: `ensure`, the claim poll, the VMM spawn, the boot wait; a launch that observes the lane signal abandons in the step that reaches the observation point (the step that raises the signal for a launch in a claim wait, at the create gate or in the boot wait; `LaunchEnsureActivate` for a launch whose `ensure` was in flight; `WlStart` for a lane born after shutdown), so the model has no separate abandon action; outcomes `LaunchHeldFail`, `LaunchUnavailable`, `LaunchCmdFail`, `VmmSpawnFail`, `GuestReady`; a launch that cannot provide the volume at all (no pool, no free extent) fails typed inside `WlStart`, in the same step, with no observable `PENDING` | one held dispatch, resolved by one of: `LaunchRunning`, `LaunchStartFail` (the one charged start-failure class, volume causes included), `LaunchPreFail` (a row, `W_PREFAIL`), `LaunchRefused` (no row, `W_REFUSED`), `LaunchLost` (no row, `W_LOST`, no charge: a launch superseded by a stop, a restart or shutdown, or lost with the control plane); `CpCrash` and `NodeReboot` also lose a pending launch |
| Invariants (`Safety`) | `I_SINGLE_WRITER`, `I_DATA_SURVIVES_RESTART`, `I_ENSURE_IDEMPOTENT_UNDER_CRASH`, `I_NO_DRAIN_FAILURE_CHARGES_BUDGET`, `I_STOP_KEEPS_VOLUME`, `I_SUPERSEDE_ABANDONS_WAITS`, `I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING`, `I_SIGNAL_NEVER_LEAKS_ACROSS_ALLOCATIONS`; "a typed failure never attaches" and "shutdown signals every launch" are asserted by named runs, not invariants (2026-11-19) | `I_FINISHED_JOB_ISSUES_NO_ID`, `I_CAPACITY_WAIT_NEVER_EXHAUSTS` (`exhausted` implies the current row ended by a charged cause: `W_CRASH`, `W_STARTFAIL` or `W_PREFAIL`) |
| Temporal | `EveryStartLiveness` | `RunIntentProgress`, `PreFailExhausts` |
| Hazards, one per decision | `noWait` (VD-8), `recreate` (VD-9), `stopDeletes` (VD-2), `noInterrupt` (SD-1 to SD-5) | `stampOnEmit` (ADR-0335: the stamp on emit decides, violates `RunIntentProgress`), `prefailNoRow` (ADR-0337: a pre-driver failure writes no row, so an id is issued per tick, violates `PreFailExhausts`), `refusalCharged` (ADR-0338: an admission refusal is charged, so a node held at the cap exhausts a healthy workload's budget, violates `I_CAPACITY_WAIT_NEVER_EXHAUSTS`) |
| Checks | `vol-safety-tlc` (the only check outside CI, VB-1; user ruling 2026-11-23, decision 10, VB-2: the exhaustive check is accepted at about 19 min, 16 workers and a 16 GB heap, `specs/quint/node-local-volume/checks.toml`, and stays `ci = false`; the earlier "~271 s" is superseded), `vol-live-start`, one `vol-hazard-*` per hazard, one `vol-run-*` per run | `rg-safety-tlc` (~65 s), `rg-live-replacement` (~29 s), `rg-live-prefail`, one `rg-hazard-*` per hazard, one `rg-run-*` per run; all `ci = true` under the two-minute rule |

Boundary scenarios 3 (unrelated state) and 6 (feature disabled) are not protocol behaviour and have no model run: seeded simulation and Tier 3
cover them. The runs are the ones DISTILL pins with `#[quint_test]`, each in the driver of its spec. The volume spec's identifiers (`storeUp`,
`StoreAbsentBoot`, the run `store_absent_fails_typed_and_never_attaches`) keep their earlier spelling; each "store" there is the volume pool.

| Scenario | Spec | Named run |
|---|---|---|
| deploy to a running allocation | volume | `happy_path_deploy_to_running` |
| killed VMM, successor waits for the drain, same volume and data | volume | `restart_waits_for_drain_then_attaches` |
| operator stop keeps the volume, resume reattaches | volume | `stop_keeps_volume_and_restart_reattaches` |
| crash after create, before attach | volume | `crash_after_create_before_attach_reuses_volume` |
| node reboot: volume inactive, re-activated, data survives | volume | `node_reboot_reactivates_and_data_survives` |
| claim held past the bound | volume | `launch_bound_exceeded_fails_typed_and_charges_budget` |
| no free extent | volume | `no_space_fails_typed_and_never_attaches` |
| no volume pool composed | volume | `store_absent_fails_typed_and_never_attaches` |
| restart while a launch waits on the claim: no row, no charge, no VMM | volume | `superseded_launch_result_discarded` |
| `workload restart` of a `Failed` current row: the generation becomes pending, the successor forgives the failure | volume | `restart_of_failed_row_forgives_the_failure` |
| stop while the guest boots: the VMM is terminated, its claim drains, the volume is kept | volume | `stop_during_boot_wait_terminates_vmm_and_keeps_volume` |
| stop while `ensure` is in flight: `ensure` finishes, and the launch abandons in the step that ends it | volume | `stop_during_ensure_lets_ensure_finish_then_abandons` |
| node shutdown while a launch waits, and an evaluation admitted before the close that launches after it; the next boot (`Boot`) attaches | volume | `shutdown_abandons_every_launch_including_one_admitted_late` |
| restart of a `Running` workload, control-plane crash during the replacement launch: one fresh allocation, `Running`, no charge | restart-generation | `control_plane_crash_during_restart_launch_relaunches_once` |
| stop, resume, a second restart that loses the launch: exactly one launch for the latest generation | restart-generation | `second_restart_loses_launch_then_one_placement` |
| a restart of a `Running` workload while its stop is pending: the allocation is stopped and one replacement placed | restart-generation | `resume_while_running_stops_and_replaces` |
| `workload restart` of an exhausted workload: one uncharged relaunch carrying the predecessor's count (`LIMIT - 1`) | restart-generation | `restart_of_exhausted_workload_relaunches_uncharged` |
| (R-1) a generation replacement forgives the predecessor's last failure | restart-generation | `generation_replacement_forgives_predecessor_failure` |
| a persistent pre-driver failure of a Service: charged rows, then `BackoffExhausted`, bounded ids | restart-generation | `persistent_prelaunch_failure_ends_in_backoff_exhausted` |
| a transient pre-driver failure: the failed launch is a charged row, the next launch runs | restart-generation | `transient_prelaunch_failure_recovers` |
| (D-1) an admission refusal at the cap: no row, no charge, no launch until occupancy drops | restart-generation | `admission_refusal_leaves_no_row_and_waits_for_capacity` |
| (H1) `workload restart` of a Job that completed: nothing pending, no id issued (main `rg_conf_job`) | restart-generation | `restart_of_completed_job_does_nothing_and_issues_no_id` |
| (H1) stop then `workload restart` of a Job: stays down, no id issued (main `rg_conf_job`) | restart-generation | `resume_of_stopped_job_does_nothing_and_issues_no_id` |

The volume spec's restart limit `LIMIT` is a model parameter: the exhaustive, liveness and hazard mains use 3 to bound the state space, and the conformance main `vol_conf` (every named run) uses 6, production's `RESTART_BACKOFF_CEILING` + 1 (production restarts a failed row iff its count is below the ceiling, so a chain fails ceiling + 1 times), so `exhausted` holds at the same failure count in the model and in production. Twelve runs are in the volume spec and ten in the restart-generation spec. One run takes `Boot` (the volume spec's shutdown run), so DISTILL pins
`Boot` in the volume driver only. A `workload restart` or `workload stop` commit enables a `LaunchLost` in a run but does not force it (the
launch may already be past its last observation point): the model is a superset of production.

**What the restart-generation model fixes.** A launch is one held dispatch: `WlStart(a)` issues allocation `a` and it is `PENDING`, which stands
for "a launch holds the lane" (the shim writes no row before the dispatch resolves); one action resolves it in the order the shim runs, admission
(`capFull`), then identity issuance (`idFault`), then the driver. `canStart` requires no `PENDING` or `RUNNING` allocation (RA2), the operator-stop
veto absent or a generation pending, and `charged < LIMIT` or a pending generation (`restart_decision(.., generation_replacement = true)`,
`workload_lifecycle.rs:51`). With `JOB`, `canStart` also requires `curA == 0`: a Job is placed once, and any row ends it, including a pre-driver
`Failed` row (`W_PREFAIL`); production's natural-exit branch finalises every ended Job row (RA11). `LaunchLost` is an environment event enabled
by `OPS` containing `"lost"`, with no fairness. It requires `not(capFull)` and `not(idFault)`, matching the shim order (admission, then
identity issuance, then the driver's observation point 1): a launch that fails admission or issuance resolves as `LaunchRefused` or
`LaunchPreFail` before the driver can observe the signal, so `supersede_lane` on the held dispatch realises `LaunchLost` exactly when neither is
armed. `CpCrash` and `NodeReboot` still model loss at any moment. `LIMIT` = the production ceiling + 1 because production restarts a failed row iff its count is
below the ceiling, so a chain fails ceiling + 1 times before `BackoffExhausted`. After `workload restart` of an exhausted workload the
replacement carries `LIMIT - 1`, so a later crash of it exhausts the budget again at once (ADR-0073: counts are carried across `workload
restart`). Not modelled (each stated in the README): platform reclaim, the `Stopped{SystemGc}` fresh placement, a same-spec re-deploy, a
`TERMINATED` current row of a Service or no row at all (only bumps `dg`).

Assumptions of the volume spec and what discharges each:

| Assumption | Discharged by |
|---|---|
| A1 a held `O_EXCL` claim refuses a second claimant on a dm-linear device, through any node | spike run-031 (F2); Tier-3 pairing test |
| A2 dm refuses to remove an open device (`drivers/md/dm.c:357-372 @ v7.0`) | not relied on in this slice (nothing removes a volume); entry contract for #211 |
| A3 the claim and lock outlive the VMM process while in-flight I/O drains | spike B2b; the disk is opened once and the lock taken on that descriptor (`vendors/cloud-hypervisor/block/src/factory.rs:85-93`, `virtio-devices/src/block.rs:1026` @ `f252bb20b`), so the kernel removes the lock before the bdev `release` frees the claim (`fs/file_table.c:461,468 @ v7.0`); a lost race otherwise is an ordinary typed start failure (ADR-0317) |
| A4 LVM metadata operations are crash-atomic | V-1: 20 iterations of SIGKILLed `lvcreate` and `lvremove` and 15 whole-VMM SIGKILLs left a volume either absent or present, never half-built (`v1-008`, `v1-011`) |
| A5 a control-plane crash after `ensure` and before VMM spawn leaves the volume intact for the next launch | A4 |
| A6 the device persists what it acknowledges after a flush | reasoned no-obligation: a media guarantee outside the appliance's control |
| A7 terminating a VMM that is still booting ends it like any other VMM kill: its claim lingers until the in-flight I/O drains, then the next launch's claim wait covers it | A3 and the existing boot-deadline teardown, which already terminates a booting VMM with zero grace through `cleanup_after_start_failure` |
| A8 a signal raised after the intent commit reaches the lane, or the evaluation's hydration sees the commit | the open-before-hydrate ordering (§ Gate G-3 Ordering), a unit test that raises the signal before the lane opens, after it opens and before hydration, and during dispatch |
| A9 a lane opened at any instant around `supersede_all_lanes` observes the signal | the lane table's own construction (every lane token is a child of the one shutdown token), a unit test of the table's use that runs `supersede_all_lanes` before, between and after concurrent lane opens; no test of the token library's semantics |
| A10 after an abandoned launch the reconciler places a fresh allocation for the generation, once | the seeded invariant `restart_generation_is_placed_exactly_once_across_supersession_and_crash` (pending DISTILL) and the pure reconcile tests; the restart-generation spec models what follows from A10 and does not discharge it (RA2 is its premise); code facts: `reconciler_runtime.rs:1701-1771`, ADR-0102 admission |
| A11 a VMM that was SIGKILLed but is not reaped (a thread in an uninterruptible wait) can never run guest code again, so an allocation `Terminated` by a stop while its claim is held is the model's `StopAlloc` (`alive` false, claim held until `Drain`) | `complete_signal`, `kernel/signal.c:1003-1025 @ v7.0` (SIGKILL pending on every thread); Tier-3 S-VOL-T3-12 (ADR-0348) |
| M5 a launch with the lane signal abandons its waits and delivers nothing; `ensure` and the VMM create are never interrupted | boundary scenarios 4 and 4a (A7 to A9) |
| M7 after a control-plane crash or a node reboot no new launch for the workload is minted while a VMM that survived the crash (including a mid-boot one) is alive: the boot reclamation sweep ends every such VMM before the reconcilers run | verified in code (the sweep kills every unsupervised VMM whatever its row; it completes before the convergence loop, the interest router and the HTTP listener start; `vm_reclamation_boot.rs:92-148`, `plan_reclamation` rows 1, 3, 4, `lib.rs:4530` before `:4813`, `:4861`, `:5032`); the DISTILL test `boot_reclamation_ends_a_mid_boot_vmm_before_any_launch_for_its_workload` |

Assumptions of the restart-generation spec (ids are stable; RA3 is not an assumption of this spec):

| Id | Assumption | Discharged by |
|---|---|---|
| RA1 | the outcome set is exhaustive: every `Err` and early return of the two dispatch arms is one of the launch resolutions or a named no-row outcome (§ Launch outcomes) | the table's code citations; shim tests; the Job dimension of the seeded invariant |
| RA2 | a launch in flight is invisible to a later evaluation, so `canStart` requires no `PENDING` allocation and one generation is never launched twice | lane exclusivity and the dispatch awaited inside the tick (ADR-0102, `reconciler_runtime.rs:1557-1628`); the seeded invariant |
| RA4 | a launch lost or superseded delivers nothing: no row, no charge, no VMM | the volume spec's `I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING`; the shim test of a superseded restart successor |
| RA5 | no VMM survives to block a placement (a booting orphan is reclaimed before any launch; a `RUNNING` orphan holds a non-terminal row) | the volume spec's M7 and its DISTILL test |
| RA6 | a launch failure caused by the volume is one typed start failure that writes a charged `Failed` row | the volume spec's runs `no_space_fails_typed_and_never_attaches`, `launch_bound_exceeded_fails_typed_and_charges_budget` and `store_absent_fails_typed_and_never_attaches`, and the volume launch tests |
| RA7 | `LaunchPreFail` and `LaunchRefused` have the production order and effects of the table | shim tests of the pre-driver paths and of the refusal; `guest_network.rs:541-591` |
| RA8 | after `CpCrash` or `NodeReboot` the View, the intent and the observation rows survive and the runtime is rebuilt with a new lane table | ADR-0035 (View persistence), `vm_reclamation_boot::converge` before the loop |
| RA9 | the budget abstraction (R-1): the model carries production's per-allocation count and derives `charged` and `exhausted`, with `LIMIT` = the production ceiling + 1 (`workload_lifecycle.rs:51`, `:408`, `:465`, `:1311-1318`) | the conformance driver compares `cnt` through `attempt_index_for_alloc` and `exhausted` through the rows; the run `generation_replacement_forgives_predecessor_failure` |
| RA10 | a Job row with a terminal claim is never replaced: the generation is not pending against it, matching the action shim's `allocation_attempt_transition` | the pure property asserting the two predicates equal over generated rows; the Job dimension of the seeded invariant; the two `rg_conf_job` runs |
| RA11 | a Job's failure before the driver is final: production finalises any ended Job row (`workload_lifecycle.rs:1091-1113`; natural exit through `is_natural_exit` `:1757-1775` and `classify_natural_exit_terminal` `:1781-1794`) and places a Job only while it has no row. The one production path that re-drives a Job row is a platform-reclaimed row (`is_natural_exit` excludes it, `:1775`), which the spec does not model; a pre-driver failure of that re-drive's successor is final in the same way | the `JOB` conjunct of `canStart` (`curA == 0`) and `I_FINISHED_JOB_ISSUES_NO_ID` model it; the Job pre-driver case of the seeded invariant and the shim tests |

Conformance drivers: two, both in `crates/overdrive-control-plane/src/`, each a `#[cfg(all(test, feature = "integration-tests"))]` module.
`quint_node_volume.rs` replays the volume spec against the VM launch path (`VmDriver::start` over the node's composed ports, the shim dispatch,
the handlers and `ReconcilerRuntime::supersede_lane` / `supersede_all_lanes`); its arms include `Boot`, `Shutdown` and
`GuestReady`, and none of `LaunchPreFail`, `LaunchRefused`, `CapFill`, `CapDrain`, `IdFaultOn`, `IdFaultOff`. `quint_restart_generation.rs`
replays the restart-generation spec against `WorkloadLifecycle` through `ReconcilerRuntime`; its owner field is the `ReconcilerRuntime`, it has
one arm per `step` action (no `Shutdown`, `Boot` or `LaunchSuperseded`), and its State reads only the accessors of the restart-generation table.
Neither driver reads a `Sim*` adapter in `from_driver`; the volume driver keeps its single paired `SimVolumeBacking::claim_state` arbiter read.

## [REF] Validation

| Id | Item | Class | Falsification returns |
|---|---|---|---|
| V-1 | LVM tools work on a minimal image-equivalent node, and the labelled partition is found without udev | contract-shaping; **PASSED** (2026-11-17): `spike/findings.md` § "V-1", `spike/evidence/v1-001` to `v1-011` | none: VD-3, VD-4, VD-5 stand; the contract absorbs `LVM_SYSTEM_DIR=/run/lvm` and the kernel features provisioning supplies |
| V-2 | A failed volume device is reported to the guest as an I/O error through the real VMM, in the production write-back attachment (the default mode's durability guarantee) | a Tier-3 test obligation, not a spike (P-3): S-VOL-T3-11 | a failure would be a defect in the VMM or the attachment, found by DELIVER, not a contract change |
| V-3 | F1 (ADR-0316): a write-through request whose post-write fsync fails completes with `VIRTIO_BLK_S_IOERR` on every engine (io_uring, AIO, sync) | upstream behaviour since 2026-11-20 (see below); originally a fork-side test obligation (P-3a) | a failure would be a defect in upstream's fsync propagation, found by DELIVER, not a contract change |

**V-3 (P-3a, 2026-11-19; amended 2026-11-20).** F1 was first planned as fork code with its regression test in `overdrive-sh/cloud-hypervisor`.
The fork has since moved to upstream `main` and F1 is upstream's own behaviour (upstream commits `91e4be988`, `8e9c08689`, `1f468d822`), so
this repository carries no F1 patch and no F1 regression test; the DELIVER step lands the fork pin and checksum bump (ADR-0161 § Amendment) and
F2 (ADR-0319). The property to hold is unchanged: for each engine, a write-through request whose post-write fsync fails completes with
`VIRTIO_BLK_S_IOERR`, with no acknowledgement and no wedge on the sync engine. S-VOL-T3-11 is not its discharging item.

**V-1 result** (nested guest on the metal host, kernel 7.0.0-38, read-only root with a tmpfs on `/run` only, no udev, no LVM daemon, distro
`lvm2 2.03.31` plus its `ldd` closure; deciding run `v1-008`): all five steps pass on the pinned `--config`, with one addition,
`LVM_SYSTEM_DIR=/run/lvm` (without it every `lvm` command exits 4, `v1-005`, `v1-006`). (1) Discovery by `PARTNAME=overdrive-volumes` from sysfs
finds exactly one partition; the filter hides every other device and refuses `pvcreate`/`vgcreate` on them (exit 5). (2) `vgcreate -s 4m`,
`lvcreate -L 64m`: the `/dev/mapper` node exists when `lvcreate` returns, so no polling is needed; writes go only to `/run`, devtmpfs and the
PV. (3) `lvs --reportformat json -o ... --units b` parses. (4) A fresh guest sees the volume inactive; `lvchange -ay` recreates the node with no
udev and the data written earlier reads back. (5) SIGKILLed `lvcreate`/`lvremove` and whole-VMM SIGKILLs leave no leaked extent and no orphan dm
device; a killed `lvremove` can leave an LV present but inactive, which nothing in this slice removes (the #211 entry contract's case). Not shown,
bounded and not contract-shaping: power loss or a torn media write (A6); concurrent `lvm` invocations and hundreds of volumes; the real
`ovd-<32 hex>` name; the confined uid's access to a dm node (spike C3); the future appliance image (L5).

**Provisioning requirements the result adds (ADR-0313):** `CONFIG_BLK_DEV_DM=y`, `CONFIG_EFI_PARTITION=y` (without it the kernel does not export
`PARTNAME`), `CONFIG_DEVTMPFS=y` mounted on `/dev`, a writable tmpfs on `/run`, the `lvm` binary with its library closure (no static `lvm` exists
on the host), no `lvm.conf`, udev rules, `dmeventd` or `lvm2-monitor`.

SD adds no contract-shaping validation item: its premises are a teardown that already exists (A7), the lane ordering (A8, a unit test) and the
model checks. The 30 s claim bound and the 30 s `ensure` deadline are policy values: the worst case is a charged, typed start failure, revisited
if a production `Held` or command-timeout failure is ever observed.

## [REF] Entry contract for the workload-deletion slice (GH #211)

Not this slice's mechanism; the constraints the slice that makes deletion reachable must meet (VD-10; full text in
https://github.com/overdrive-sh/overdrive/issues/211#issuecomment-6955577132).

- E1. Deletion removes the workload intent only after the node reports the volume removed, and refuses a same-id resubmit meanwhile (otherwise a
  resubmit in the window attaches the old data).
- E2. Reclaim takes the volume's exclusive claim, marks the volume `reclaiming` before erasing, zeroes the whole device, then removes it; extents
  are zero before reuse so a deleted workload's data never reaches another workload. A present-but-inactive volume, which a killed `lvremove` can
  leave (V-1), is an ordinary state for reclaim.
- E3. Reclaim is planned only when the workload has no `Pending` or `Running` allocation (a larger model found a launch that had passed the claim
  check, a reclaim taking the claim, a control-plane crash, and the surviving VMM opening a volume marked `reclaiming`).

## [REF] Scope limitations and exposures

The scope limitations L1 to L9 and their tracking issues are the table in [`design/wave-decisions.md`](design/wave-decisions.md).

Exposures with a stated bound:

- A restart commit that lands between the lane's opening and the evaluation's hydration can supersede the very `Start` that hydration produced:
  one extra evaluation per operator commit, never a loop (§ Gate G-3 Ordering). A stop latency of up to the 30 s `ensure` deadline when the
  signal arrives during `ensure` (a hung `lvm` command is not aborted, `.claude/rules/reconcilers.md`). Node shutdown raises the signal at the
  start of the convergence stage, after the gateway stage of `ServerHandle::shutdown` (bounded by `drain_deadline`).
- **N-REC.** A persistent failure of the observation write of a launch's own row, or of the prior-row read (S0, R0), with the View store healthy,
  leaves the generation pending and re-issues one id per tick, as on `main`. A disk-level fault ends the evaluation at the View write first
  (`reconciler_runtime.rs:1557-1561`); a fault confined to `observation.redb` is a node-fatal store fault and no row-based remedy applies.
- The single-cut greenfield upgrade (deleting the data directory) removes the intent store while the volume group keeps its volumes: a redeploy of
  the same workload id then attaches the old volume with its old data and size. The upgrade re-provisions the volume partition with the data
  directory (ADR-0313).
- A full volume pool fails each launch as `VolumeUnavailable { InsufficientSpace }` and ends as `BackoffExhausted` with that cause as the last
  reason (L4 explains why the pool only shrinks); admission does not check pool space.

## External integrations (handoff to platform-architect)

Contract tests recommended for the `lvm` command-line tools: the JSON report format and exit-status behaviour Overdrive parses are pinned by
golden outputs captured from the node's LVM2 version and re-captured on every LVM2 bump (consumer-driven contract against a subprocess). The
Cloud Hypervisor fork's disk argument surface (`serial`, `direct`, `image_type`, `O_EXCL` on block devices) is pinned by the fork's F2 test, upstream's own
tests for F1 and F3, and the argv golden test. Enforcement tooling: the existing dst-lint and `conformance-lint` families and whatever ADR-0082 D5 enforces for probe
adapters.

## Handoff

DISTILL: boundary scenarios above; the conformance drivers per § Observable projections; the executable obligations of
`distill/upstream-issues.md` § "DISTILL input from the DESIGN of 2026-11-19"; Tier-3 data-survival scenarios through `serve` + `deploy`
(restart, node reboot). Development paradigm: object-oriented (project `CLAUDE.md`); ports-and-adapters with the `overdrive-core` port,
`overdrive-host` adapter, `overdrive-sim` twin. The DELIVER roadmap sequences the work (DELIVER owns it).

## Quint results

Two specs, `specs/quint/node-local-volume/` and `specs/quint/restart-generation/` (checked on the metal host). Verdicts, counts, durations and
memory are in each spec's `evidence/summary.md`; none is restated here. Model assumptions that are not decisions of this feature: a VMM that
survives a control-plane crash keeps its allocation `Running` until `VmReclamation` kills it; the volume spec applies the restart budget rule
of `specs/quint/restart-generation/restart_budget.qnt` (a lost or superseded launch has no row and is not charged; a generation replacement
inherits its predecessor's count). The model
creates a volume inactive and activates it by a second effect (M1), a conservative refinement of LVM, where `lvcreate` returns an active volume;
inactive volumes arise from a node reboot (run `node_reboot_reactivates_and_data_survives`), which `ensure` absorbs.

## Wave: DISTILL

Specification: `distill/test-scenarios.md`. RED proof: `distill/red-classification.md`.
Executable obligations, placement, the DELIVER activation order, re-sync lists and open decisions: `distill/upstream-issues.md`.
No DISCUSS or DEVOPS wave existed; acceptance criteria derive from this document, ADR-0310 to ADR-0320 and the issue as reduced by VD-10.
