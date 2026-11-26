# Feature delta: node-local persistent volume for VM workloads (GH #348)

**Status:** the design is approved by the user; rulings run through 2026-11-23. The decision table (VD-1 to VD-10, SD-1 to SD-6, J-1, DE-1, D-1,
H1, H2, W-1 to W-4, R-1, VB-1 to VB-3, RB-1 to RB-12, B-1 to B-3, P-1 to P-6) and the scope-limitation table (L1 to L9) are owned by
[`design/wave-decisions.md`](design/wave-decisions.md); each ADR (0310 to 0320, 0334 to 0338, 0348 to 0355) holds one decision. This file owns the interface
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
   (the SVID the shim minted before the launch; the launch has no `Running` row, so no row-woken `SvidLifecycle` would drop it), then
   `Driver::release_supervision(alloc)`. It writes the allocation's **abandoned row** (ADR-0355: `Terminated`, reason
   `LaunchSuperseded`, § "Launches that end before running"; this amends the no-row outcome ADR-0334 first stated and step 05-01 implemented)
   and returns `Ok(())` for the action: no `Running`, no `Failed`, no
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
| – | `create_run_dir` ... `Vmm::create` | **no observation point**: a bounded effect that runs to its end (the rootfs clone and the VMM spawn are not interruptible), so the VMM is always inside its scope when an abandonment finds it (ADR-0352: it is created there) |
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
   `cleanup_after_start_failure` where it created a host artifact, and returns `StartSuperseded`; the shim writes the abandoned row (`LaunchSuperseded`), charges nothing and
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

**Today.** `CloudHypervisorVmm::terminate` (`crates/overdrive-host/src/vmm.rs:650-690`) notifies the reaper to kill, then awaits the reap with no
bound (`rx.changed().await`, `:670` for zero grace, `:688` otherwise). A VMM with a thread blocked in uninterruptible I/O (a flush held on a
suspended or failing dm device) never becomes reapable, so `VmDriver::stop` (`vm_driver.rs:2047-2212`) never returns, the shim's `StopAllocation`
arm (`action_shim/mod.rs:2743`) never writes the terminal row, the row stays `Running` and the lane stays held. The `terminate` callers are
`VmDriver::stop` (`:2128`, grace `VM_STOP_GRACE` = 10 s), the start-failure and boot-race cleanup (`:938`, `:1311`, zero grace) and the failed EXEC
release (`:2315`, zero grace).

**The contract (user ruling 2026-11-23, decision 11, option a).**

1. **The outcome.** `overdrive_core::traits::vmm::VmTermination` gains one unit variant, `KilledNotReaped`: the kill was delivered and the process
   was still not reaped when `VMM_REAP_BOUND` elapsed. It is an outcome, not a `VmmError`: `VmmError` keeps meaning "the kill or spawn itself
   failed". It carries no payload (the pid is the caller's `VmControl`). The existing variants are unchanged: `ExitedWithinGrace(VmmExit)`, and
   `Killed` for a process reaped after the kill (and for an already-gone process).
2. **The bound.** `pub const VMM_REAP_BOUND: Duration = Duration::from_secs(5)` in `overdrive-core::traits::vmm`, beside the trait. 5 s is about
   twice the held-flush drain measured in the spike (2.4 s, `design/wave-decisions.md` § Scope limitations); a reap after SIGKILL takes
   milliseconds when nothing is stuck; the worst case, a stop of an unreapable VMM, costs `VM_STOP_GRACE` + `VMM_REAP_BOUND` = 15 s of lane time
   (a VMM already SIGKILLed from outside still waits out the grace first, since the grace wait ends only on the reap). It is a policy value.
3. **Start and clock.** The bound starts at the instant `terminate` requests the kill, on the grace-expired path and the zero-grace path. `SimVmm`
   takes an `Arc<dyn Clock>` as a required constructor parameter and waits `VMM_REAP_BOUND` on it (`Clock::sleep`), so seeded simulation drives
   the bound. `CloudHypervisorVmm` measures it on real time (`tokio::time::sleep`), as it does for `grace`, and takes no clock.
4. **The `Vmm::terminate` rustdoc states** (each clause is covered by the equivalence test):
   - *On `Ok(Killed | ExitedWithinGrace)`:* the process is reaped. *On `Ok(KilledNotReaped)`:* SIGKILL has been delivered, no thread can run
     guest code again, and the process is not reaped; it releases every resource it holds (descriptors, the volume's kernel claim, the vsock
     device, its cgroup membership) only when it is reaped.
   - *The adapter keeps waiting for the process after returning `KilledNotReaped`.* A late reap runs the adapter's ordinary reap path (its exit
     watch fires, its live record is dropped); it is not an error and changes nothing the caller wrote.
   - *Edge cases:* `grace == Duration::ZERO` kills at once and then waits the bound; an already-gone pid is `Ok(Killed)`.
   - *`Err(VmmError)`* only when the substrate refuses the kill; never for a slow reap.
5. **`SimVmm`.** A test marks a process unreapable (the constructor and scripting method are DISTILL's); `terminate` on it marks the process
   killed, waits `VMM_REAP_BOUND` on the injected clock and returns `KilledNotReaped`. The process keeps every resource it holds until the test
   reaps it (a scripted `exit_process`): its `device` entry (the vsock CID holder `vsock_device_holders` reports) and its scope membership are
   released at the scripted reap, and its exit sender fires only then. The reapable path is unchanged.
6. **`VmDriver::stop` on `Ok(KilledNotReaped)`.** The outcome is a completed stop. The driver writes the beacon writer's `vmm_has_exited` as for any
   `terminate` completion and runs the shared artifact cleanup, which **omits the scope step** after `KilledNotReaped` (the scope is populated by
   definition; `VmReclamation` removes it after the reap). The run directory and the clone are still removed (both are removable while the
   process is stuck). The driver does not touch the guest lease; the shim holds it (§ "The guest-address lease outlives an unreaped VMM").
   `status()` is `Err(NotFound)` (the entry is `EndingInFlight`, as for every stop) until the shim's `release_supervision`. No new `DriverError`.
   The `Driver::stop` contract says: on `Ok`, the VMM has been killed and is reaped, or killed and not yet reaped; the shim comment at
   `action_shim/mod.rs:2774` ("`Driver::stop` awaits process quiescence") is corrected to that. An unreaped stop is visible without a new event:
   the allocation is `Terminated` and its scope is still in `VmHostState::observe()`.
7. **One rule for the scope step in every cleanup.** The artifact cleanup shared by the stop, the start-failure cleanup and the boot-race cleanup
   skips the scope step when its `terminate` returned `KilledNotReaped`, and runs it otherwise (§ "A VM scope has one remover"). The start-failure
   cleanup reports the failure it was reporting, as today. The failed EXEC release ignores the outcome, as today.
8. **What the shim writes.** Nothing new. `StopAllocation` proceeds as for a reaped VMM: `retire_guest_network_lease`, `mtls_lifecycle.stop_alloc`,
   `teardown_for_dispatch` (which, while the VMM is unreaped, tears the network down but keeps the lease Retiring), then the `Terminated` row with
   `TransitionReason::Stopped { by: Reconciler }` and the action's `terminal`, no `Failed` row, no restart-budget input, then
   `release_supervision`. A `Running` row becomes `Terminated` within `VM_STOP_GRACE + VMM_REAP_BOUND` plus the cleanup time. `restart`
   (`RestartAllocation`) stops its predecessor through the same `Driver::stop` and is bounded the same way.
9. **Who owns each leftover.**

   | Leftover | Owner | Removed when |
   |---|---|---|
   | the hypervisor process | the adapter's reaper task | it reaps the process, whenever that is |
   | the cgroup scope (non-empty while a thread is stuck) | `VmReclamation` | the first pass after `release_supervision` and the reap: a terminal VM allocation with host artifacts and no driver claim emits `DiscardStrandedArtifacts` (`vm_reclamation.rs:161-171`); the pass runs at boot and every `VM_RECLAMATION_SWEEP_INTERVAL` (30 s); a pass that cannot remove it returns and the next retries |
   | the run directory and the clone | `VmDriver::stop`; `VmReclamation` as the backstop | at stop; otherwise the same pass |
   | the volume's kernel claim | the kernel | when the stuck I/O drains and the process exits; Overdrive neither releases nor fences it (ADR-0317); a replacement's claim wait covers it |
   | the vsock device and its CID | the kernel, at the process's exit | the same reap |
   | the guest-address lease | the shim | § "The guest-address lease outlives an unreaped VMM" |

10. **No path pays both bounds.** A stop pays `VMM_REAP_BOUND` and, after `KilledNotReaped`, no `KILL_SCOPE_SETTLE_BOUND` (item 7); after a reaped
    stop the scope is empty and `kill_scope` returns within milliseconds; boot reclamation and `VmReclamation` call `kill_scope` and never
    `terminate`.
11. **Not changed.** `Driver::stop`'s signature; the lane and supersession path (ADR-0334); the claim wait and its 30 s bound (ADR-0317);
    `VM_STOP_GRACE`; the exit watcher (it finds `EndingInFlight` at a late reap and returns); the reconcilers; the observation row shapes;
    `kill_for_test`; the `GuestAddressPool` methods other than § "Boot proceeds past a stuck VM"; the `DiscardStrandedArtifacts` action and its
    executor; `VmHostState`'s signatures.

**Lifecycle Gate Ownership.** Adds no gate. One existing state meaning changes: an allocation ended by a stop is `Terminated` once the VMM is killed
and can run no guest code, not once its process is reaped. The kernel fact: a fatal signal puts SIGKILL pending on every thread of the group and
wakes it (`complete_signal`, `kernel/signal.c:1003-1025 @ v7.0`); a thread in an uninterruptible wait exits when the wait ends. The user approved
the meaning with decision 11.

**Boundary scenarios.** (1) The VMM reaps within the bound: `Killed` or `ExitedWithinGrace`, unchanged. (2) It does not: `KilledNotReaped`, the row
`Terminated` `Stopped`, supervision released. (3) Other allocations' lanes are unaffected. (4) Late reap after the stop: the exit watcher finds no
`Live` entry, so no `ExitEvent` and no row write. (5) A control-plane crash while a process is unreapable: boot handles it as for any
unsupervised VMM (§ "Boot proceeds past a stuck VM"). (6) A reapable VMM never sees the new outcome. (7) Observing the scope fails at the stop: the
stop returns the typed error, the row stays `Running`, the replay retries.

**Tests.**

- Tier 3: S-VOL-T3-12 (`crates/overdrive-cli/tests/integration/volume_survives.rs:754`), already authored and pending on this defect; the DELIVER
  step removes its `#[should_panic]` marker. It proves the real adapter, a real SIGKILL and a real held flush, and additionally asserts that no
  lease is released while the held-flush VMM is unreaped and that it is released after the drain (real scope removal, real device close).
- Tier 1, seeded, production `VmDriver::stop` over `SimVmm` with an unreapable process: (a) `stop` returns `Ok`, advancing the `SimClock` by exactly
  `VMM_REAP_BOUND` after the grace, and the shim writes `Terminated`/`Stopped`, no `Failed`, no charge; (b) the `SimVolumeBacking` claim is still
  `Held` and the `SimVmm` vsock holder still reports the CID after the stop, and both clear only at the scripted reap; (c) the scripted late reap
  emits no `ExitEvent` and changes no row; (d) a replacement launch during the stuck window waits on the claim, is not charged before its bound, and
  attaches after the scripted reap and drain; (e) after `KilledNotReaped` the driver makes no `kill_scope` call, in the stop and in the
  start-failure cleanup alike, and still removes the run directory and the clone.
- `Vmm` equivalence: `CloudHypervisorVmm` and `SimVmm` agree on an already-gone pid (`Killed`) and on the reapable outcomes.
- `VmReclamation`: a pass over a terminal allocation whose scope cannot be removed returns and emits again on the next pass; after the reap the
  next pass removes it.
- Stop-totality: the doubles of `vm_driver_stop_totality.rs` gain a `KilledNotReaped` case.

### The guest-address lease outlives an unreaped VMM (RB-2; amends ADR-0156)

**The ruling.** User, 2026-11-23, decision 12, option a: a guest-address lease is released only once the allocation's cgroup scope is observed
absent; while the scope is present the lease is retired and held, and the stop still completes. The reasoning and the amendment are in ADR-0156.
No public API.

**The fact.** A guest's CID is `GUEST_CID_BASE` plus its lease offset (ADR-0156). The kernel unhashes a CID at the last close of the device file
(`vhost_vsock_dev_release`, `drivers/vhost/vsock.c:753-760 @ v7.0`), and a launch whose CID is still hashed fails `-EADDRINUSE`
(`vhost_vsock_set_cid`, `:804-813 @ v7.0`). A killed-not-reaped VMM still holds the file, so freeing its offset would let next-fit hand its CID to a
new launch. `GuestAddressPool::release` and `GuestNetworkProvisioner::teardown` state release-last as *reaped*, not *stopped*.

**Why "scope absent" proves "every descriptor of the VMM is closed".** Four facts compose; each is cited at `v7.0`.

| # | Claim | Source |
|---|---|---|
| K1 | Every task of the VMM is a member of the scope from its first instruction | ADR-0352; `clone3` with `CLONE_INTO_CGROUP` links the child to the scope's css_set before it is woken (`cgroup_post_fork`, `kernel/cgroup/cgroup.c:6992-6993`; `wake_up_new_task`, `kernel/fork.c:2684`); `execve` does not change membership (`fs/exec.c:946-957`); § "A VMM is born inside its scope", C4 and C6 |
| K2 | A task closes its descriptors before it leaves the scope. In `do_exit`: `exit_files` (`kernel/exit.c:971`) drops the last reference of the file table, `put_files_struct` (`fs/file.c:506-516`) runs `close_files` (`:474-504`) and `filp_close` (`fs/open.c:1478-1486`) per descriptor, `fput_close` (`fs/file_table.c:583-587`) queues the final `__fput` as task work on the exiting task (`__fput_deferred`, `:519-541`, `task_work_add` at `:528-531`; the add can fail only after `exit_task_work` has run). `exit_task_work` (`kernel/exit.c:976`) then runs `__fput` (`fs/file_table.c:443-483`), which calls the file's `release` (`:468-469`). The task leaves its css_set only afterwards, in `finish_task_switch` (`kernel/sched/core.c:5183-5193`, `cgroup_task_dead`; `do_cgroup_task_dead`, `kernel/cgroup/cgroup.c:7072-7102`, `css_task_move` at `:7082`) | as cited |
| K3 | The vsock device's release is the last close of the VMM's device file: `vhost_vsock_dev_release`, `drivers/vhost/vsock.c:753-760`. The VMM is the only holder of it (ADR-0146); the launcher passes the VMM standard I/O and the TAP queue only (ADR-0129) | as cited |
| K4 | The volume's claim ends in the file's release: `blkdev_release` (`block/fops.c:703-707`) calls `bdev_release` (`block/bdev.c:1124-1166`), which syncs the device when it is the last opener (`:1141-1142`; this is the held flush), then ends the claim (`bd_yield_claim`, `:1110-1122`, called at `:1147-1148`; `bd_end_claim`, `:689-`). The OFD lock is removed earlier in `__fput` (`locks_remove_file`, `fs/file_table.c:461`, before `release` at `:468`). The VMM opens the volume once (A3) and no other process holds a duplicate | as cited |
| K5 | `cgroup.events` `populated` is 1 while any task of the scope is on the css_set and 0 only after the last `do_cgroup_task_dead` (`cgroup_is_populated`, `kernel/cgroup/cgroup.c:3817-3824`; spike `findings-cgroup-events.md`: 0/20 early, 0/20 missed) | as cited |

Consequence: a stuck close (the held flush) is inside `exit_task_work`, before `cgroup_task_dead`, so the scope stays `populated 1` for as long as
the device and the claim are held, and reads `populated 0` only after both are released. The removal that follows an empty reading is ADR-0349's;
that it is the only removal is ADR-0351's.

**The one release site.** `teardown_guest_network` (`action_shim/mod.rs:1232`) is the only caller of `GuestAddressPool::release`. The stop, the
restart predecessor and successor cleanups, the start-failure cleanups and `reclaim_guest_network` all reach it through `teardown_for_dispatch`.
Its order: (1) read the allocation's lease through `GuestAddressPool::observe` and its plan through `snapshot`; no lease: return `Ok`. (2)
`VmHostState::observe()`; a failure returns the existing `ShimError::Reclamation` before any effect, so the replay retry applies and nothing is torn
down twice. (3) If a plan exists, `GuestNetworkProvisioner::teardown(plan)` (it converges on absence, `guest_network.rs:135-141`); a **boot
reservation has no plan** (nothing was provisioned for it, § "Boot proceeds past a stuck VM"), and the step is skipped for it. Today the function
returns `Ok` at step (1) when the pool holds no plan, which would leave a reservation held forever; the order above replaces that. (4) Scope absent:
`release`. Scope present: the lease is left Retiring (retired if Admitted) and the function returns `Ok`, which lets `StopAllocation` write its
terminal row.

**The late release.** A `Terminated` or `Failed` row whose allocation still holds a lease is already a network-reclaim candidate:
`WorkloadLifecycle` emits `Action::ReclaimAllocationNetwork` for it (`workload_lifecycle.rs:485-516`), paced by `reclaim_attempts` and
`reclaim_emitted_at`, and its executor `reclaim_guest_network` re-runs `teardown_for_dispatch`. The retry has no attempt ceiling and a constant
backoff (`workload_lifecycle.rs:520-549`, `:89-91`), terminal rows stay in `managed_targets`, and the restart path does not mark the predecessor
handled, so the predecessor's lease is reclaim-eligible once the successor row exists. After the reap, `VmReclamation` removes the scope
(`DiscardStrandedArtifacts`) and the next reclaim attempt finds it absent and releases. For a reservation, `reclaim_guest_network` needs no change:
no driver entry exists after a restart, so `resolve_drivers_for_alloc` returns every registered driver and each `Driver::stop` answers
`NotFound`, which the function accepts (`action_shim/mod.rs:1266-1273`), and `MtlsInterceptLifecycle::stop_alloc` is a no-op for an unknown
allocation. `reclaim_guest_network` and the helpers above gain a `&dyn VmHostState` (the dispatch already holds one), which is crate-internal.

**Cost.** One `observe()` (a bounded directory walk) per teardown, not a hot path. One attachment slot per unreaped VMM until the reap, counted as
Retiring against the node-wide cap (`at_cap`): enough unreaped VMMs refuse every new launch and every restart's successor with
`AdmissionCapReached` until they reap. Accepted with decision 12. A lease is process-local and not persisted (ADR-0118).

**Tests.** Tier 1, seeded, production shim and `WorkloadLifecycle` over `SimVmm` and `SimVmHostState`: (a) after the unreaped stop the pool reports
the lease Retiring, the row is `Terminated`, and an assign that wraps to the offset takes another; (b) after the scripted reap, a `VmReclamation`
pass and a reclaim attempt, the lease is released and no launch clashed; (c) the scope observation fails at the stop: typed error, no teardown, no
row, the replay succeeds; (d) the lease is held while the scope is held and released only after the scope's removal, in a trace where the scripted
reap is delayed past several reclaim attempts. The sim obligation: `SimVmHostState.observe()` reports an unreaped process's scope, and its
`kill_scope` removes it only once `SimVmm` has reaped that process; the two adapters share that one fact. A reservation case is in § "Boot
proceeds past a stuck VM".

### A scope kill's settle wait is bounded (RB-3; ADR-0349)

**Today.** `RealVmHostState::kill_scope` (`crates/overdrive-host/src/vm_host_state.rs:229-257`) writes `cgroup.kill` and then retries
`tokio::fs::remove_dir` up to 150 times at 20 ms (3 s), returning the last error whatever its kind, each attempt a blocking `rmdir` on the
blocking pool. The `v7.0` source predicts `-EBUSY` while a task of the scope is visible (`cgroup_drain_dying`, `kernel/cgroup/cgroup.c:6254-6272 @ v7.0`)
and an uninterruptible sleep when every remaining task is already exiting (`:6274-6297`). The running kernel does neither (spike
`findings-cgroup-events.md`): `rmdir` of a scope whose last task is held in exit returns `Ok` in about 1 ms with the task alive and holding the
device. So today's `kill_scope` can return `Ok` for a scope whose VMM is not gone. The port rustdoc
(`overdrive-core/src/traits/vm_host_state.rs:138-165`) states the settle contract with no bound.

**The contract (user ruling 2026-11-23, decision 13, option a, with decision 16: the wait is on "empty").**

1. **The outcome.** No signature change: `async fn kill_scope(&self, scope: &CgroupPath) -> std::io::Result<()>`. `Err` with
   `kind() == io::ErrorKind::ResourceBusy` means the kill was delivered and the scope is not removed. No new type. Every other `Err` kind is a
   substrate refusal, returned at once and never retried (today's "retry whatever the error" is removed).
2. **The bound.** `pub const KILL_SCOPE_SETTLE_BOUND: Duration = Duration::from_secs(3)` in `overdrive_core::traits::vm_host_state`, beside the
   trait. 3 s is today's 150 x 20 ms; an ordinary scope settles in milliseconds. It is a policy value. The 20 ms poll interval stays private to the
   host adapter.
3. **Order, coverage and clock.** `kill_scope` (a) writes `cgroup.kill`; (b) waits for the scope's `cgroup.events` to report `populated 0`; (c) only
   then removes the directory. It never issues the removal while the scope reports `populated 1`. The bound starts when `kill_scope` is entered and
   covers the whole call; the removal after an empty reading does not wait. `RealVmHostState` measures it on real time (`tokio::time`) and takes no
   clock. The wait is a read of `cgroup.events` that cannot miss a transition and is cancellable by the bound's timer, with no thread blocked in
   the kernel (the spike's discipline, arm then read then test then poll `POLLPRI`, is the adapter's, not a contract). At the bound with
   `populated 1` the call returns `ResourceBusy`, the scope in place, no removal attempted. A scope that is empty at entry, or whose
   `cgroup.events` is absent (`NotFound`), skips the wait. A removal after `populated 0` that fails is a real error returned at once. A busy
   return always takes the full bound: the convergence loop re-submits a target whose tick emitted an action immediately, and the busy wait is the
   only pacing on that retry.
4. **`VmHostState::kill_scope` rustdoc states** (each clause is covered by the equivalence test):
   - *On `Ok`:* `populated 0` was observed for the scope and the directory is absent (removed, or `NotFound`), so no task of the scope is left and
     every descriptor of its VMM is closed. A following `observe()` does not report it.
   - *On `Err(ResourceBusy)`:* `cgroup.kill` was written, the scope is present and was not removed, and the bound ended the call. A following
     `observe()` may report an empty `pids` (exiting tasks are not listed): an empty `pids` is not emptiness. The call returns within the bound
     plus scheduling latency, never earlier.
   - *Edge cases:* an absent scope is `Ok`; a second call for a still-busy scope is `ResourceBusy` again after its own bound; a scope that empties
     during the wait returns `Ok` without waiting out the bound; a non-`ResourceBusy` error is returned at once.
   - *Errors:* `ResourceBusy` is the only kind a caller retries; any other kind is a refusal (`PermissionDenied`, `EIO`).
5. **`SimVmHostState`.** It does not wait and measures no time. A test holds an allocation's scope (the holding method is DISTILL's); `observe()`
   keeps reporting it, `kill_scope` on a held scope returns `Err(ResourceBusy)` at once, changes nothing and does not consume the scripted-failure
   queue, and the scope leaves only when the hold is cleared. It never removes a held scope. The hold and `SimVmm`'s unreaped process share one
   fact, the reap, so the scripted reap clears both. `script_kill_scope_failure(ErrorKind::ResourceBusy)` remains a one-call busy.
6. **What each caller does on `ResourceBusy`.**

   | Caller | On `ResourceBusy` |
   |---|---|
   | `execute_reclaim_allocation`, `execute_discard_stranded_artifacts` (`action_shim/reclamation.rs:239`, `:338`), `VmReclamation` tick | `ReclamationError::Host(err)` as for any substrate failure: no `discard_artifacts`, no terminal row, no evaluation submitted. The tick requeues the target (`reconciler_runtime.rs:1648-1655`, `:1764-1770`) and the 30 s resync pass is the backstop; the busy wait is the pacing. `dispatch` records the first error and continues with the tick's other actions (`action_shim/mod.rs:951-957`) |
   | the boot executor step | § "A busy boot scope writes its terminal row" and § "Boot proceeds past a stuck VM" |
   | the VM driver's cleanup | § "A VM scope has one remover" |
   | the lease gate | § "The guest-address lease outlives an unreaped VMM": a busy scope keeps its lease |

7. **Not changed.** The `VmHostState` signatures, `RealVmHostState::new`'s parameters, `observe`, `probe`, `discard_artifacts`, the
   existing `ReclamationError` and `ConvergeError` variants (`ReclamationError` gains `CorruptLeftoverRow`, § "Pool API"), the planner, `VmReclamation`'s View and the tick's requeue.

**Cost.** A stuck scope costs the `VmReclamation` target one bound of waiting per tick until the process is reaped; actions for other allocations
in the same tick are delayed by up to that bound for each stuck scope. A stuck scope costs one open file and one waiter, no blocked thread.

**Tests.**

- **`kill_scope` never removes a populated scope.** Host adapter, real cgroupfs (Tier 3; the spike's `volume-pool` hold is the fixture): with the
  scope's last task held in exit, `kill_scope` returns `ResourceBusy` at the bound, the directory is present for the whole hold (the early `rmdir`
  of the running kernel would have broken this), the task is alive; after the drain it returns `Ok` with the scope absent. That test also shows the ordering (the
  directory is present for the whole hold, so the removal follows the `0` read) and the mid-wait case (a scope that empties during the wait
  returns `Ok` without waiting out the bound). No host-adapter unit over a fake `cgroup.events` is written: `RealVmHostState::new` takes the
  cgroup root, so a tempdir can be injected (no seam is added), but a regular file cannot stand in for cgroupfs's notification (a regular file
  is not pollable for `cgroup.events` changes), so the wait cannot be exercised there.
- `VmHostState` equivalence (`vm_host_state_equivalence.rs`): both adapters agree on `Ok` implying absent and no task left, on `ResourceBusy`
  leaving the scope present and observed, on idempotence, and on a non-busy error returning at once without retry.
- Host adapter, unit, over a tempdir passed as `RealVmHostState::new`'s `cgroup_root` (the existing parameter; no seam): a scope directory with no
  `cgroup.events` is `Ok` and absent afterwards; an absent scope is `Ok`; a non-`ResourceBusy` error is returned at once with no retry (arranged
  with `cgroup.kill` a directory, which fails the write with a non-`NotFound` kind). A `PermissionDenied` removal cannot be arranged as root and
  `serve` always runs as root on the appliance, so it has no obligation of its own: the any-other-kind case above is the same branch.
- Tier 1, seeded, production `VmReclamation` tick over `SimVmHostState` with a held scope: the tick returns busy without discarding, writing a row
  or advancing the lease; other allocations' actions in the same tick still run; after the hold clears the next tick removes the scope.
- The boot-reclamation test double (`vm_reclamation_boot.rs:821`) gains the busy case.

### A VM scope has one remover (N-1; ADR-0351)

**Today.** The VM driver removes a VM scope itself: `cleanup_driver_artifacts` (`vm_driver.rs:1736-1738`, the stop) and `attempt_start_cleanup`
(`:1318-1324`, the start-failure and boot-race cleanup) write `cgroup.kill` through `CgroupManager::cgroup_kill` and then call
`CgroupManager::remove_workload_scope` (`worker/cgroup_manager.rs:290`), a bare `rmdir` with no wait, which on the running kernel removes a scope
under a live task. The lease gate needs every removal of a VM scope to be the populated-first one of § "A scope kill's settle wait is bounded".

**The contract (user ruling 2026-11-23, N-1, option a).**

1. `VmDriver` takes `Arc<dyn VmHostState>` as a required constructor parameter (no default, `development.md` § "Port-trait dependencies").
2. The two cleanup sites replace their `cgroup_kill` write and their `remove_workload_scope` call with one `VmHostState::kill_scope` call, in the
   same position of the cleanup order. After the change `kill_scope` is the only remover of a VM scope.
3. **`ResourceBusy` is the best effort the cleanup already is.** The stop completes as ADR-0348 says, the start-failure cleanup returns the failure
   it was reporting, the run directory and the clone are still removed in the same order after the scope step, no error type or event is added, and
   the scope is left to `VmReclamation`. Every other error kind keeps today's best-effort handling of a failed scope step. After
   `KilledNotReaped` the scope step is omitted (§ "A VMM kill's reap wait is bounded", item 7).
4. **Deletions (the method loses its callers).** `CgroupManager::remove_workload_scope` has no caller after item 2: it is deleted, with
   `tests/acceptance/cgroup_manager/remove_workload_scope_idempotent.rs`, `tests/integration/cgroup_manager/remove_workload_scope_propagates_real_io_error.rs`
   and the `mod` lines naming them (`tests/acceptance/cgroup_manager/mod.rs`, `tests/integration.rs:69`); the two callers that use it for test
   cleanup (`overdrive-control-plane/tests/integration/cgroup_isolation/alloc_scope_has_writable_cpu_weight_and_memory_max.rs:192`, `:233`) remove
   their scope through `VmHostState::kill_scope` or `tokio::fs::remove_dir`, the crafter's choice. `CgroupManager::cgroup_kill` loses its last caller
   in the same change (its only callers were the two cleanup sites) and is deleted with `cgroup_kill_idempotent.rs`,
   `cgroup_kill_writes_one_newline.rs`, `cgroup_kill_propagates_real_io_error.rs`, `real_cgroup_fs/cgroup_kill_terminates_pids.rs` and
   `real_cgroup_fs/refused_thread_kill.rs`, unless the crafter finds a caller outside `src/` and `tests/` of `overdrive-worker`; it reports that
   caller rather than keeping the method. `StartCleanupStage::CgroupKill` and `CgroupRemove` collapse to one stage, which is internal structure.
   No stub, shim or deprecation is left (`development.md` § "Deletion discipline").
5. **Enforcement.** A structural test (an AST scan in the shape of the existing dst-lint family, run in the default lane) scans the production
   `src/` of every workspace crate for the calls that remove a cgroup directory: `CgroupFs::remove_dir` (the port method) and
   `remove_dir_all`, by method and path name. The only allowed callers are `RealVmHostState::kill_scope` (the host adapter's removal of the
   scope path) and the `CgroupFs` adapters' own startup probes, which remove the probe directory they created: the host probe
   (`overdrive-host/src/cgroup_fs.rs:169`, and its `best_effort_remove_dir` helper at `:195-196`) and the sim adapter's probe
   (`overdrive-sim/src/adapters/cgroup_fs.rs:451`, `:483`, `:496`). The port method's definition and the adapters' `remove_dir` bodies are
   not calls. **Decided under this design's authority:** the allow-list includes the `CgroupFs` adapters' own probe removals, because they remove
   a directory the probe itself created, never a VM scope. It is the lease gate's structural defence: a second remover cannot be added unseen.
   **Stale comments rewritten in the same change** (they describe a `remove_workload_scope` and a `cgroup_kill` that no longer exist):
   `overdrive-sim/src/adapters/cgroup_fs.rs:413-415` (`cleanup_after_start_failure` "always issues `cgroup_kill` immediately before
   `remove_workload_scope`"), and `overdrive-core/src/traits/cgroup_fs.rs:152` (the "cgroup manager's `remove_workload_scope` wrapper swallows
   `NotFound`"). Each is rewritten to name `VmHostState::kill_scope` as the caller that tolerates `NotFound` and the populated-first removal.

**Tests.** Tier 1, production `VmDriver` over `SimVmm` and `SimVmHostState`: a reaped stop and a start-failure cleanup remove the scope through
`kill_scope` (the sim records the call) and never by any other path; a held scope makes both return their normal outcome (the stop completes and the
shim writes `Terminated`/`Stopped`; the start failure is the one reported), with the run directory and clone removed and the scope still observed;
after the hold clears, a `VmReclamation` pass removes it; after `KilledNotReaped` the driver makes no `kill_scope` call.

### A VMM is born inside its scope (F1; ADR-0352)

**Today.** The driver creates the scope (`vm_driver.rs:1539`), `Vmm::create` spawns the VMM (`:1650`), and only then
`CgroupManager::place_pid_in_scope(&scope, control.pid)` writes the pid to `cgroup.procs` (`:1848`). The spawn
(`overdrive-host/src/vmm.rs`, `build_confined_command` `:272-322`, `register_launch_child_hook` `:330-377`) does no cgroup enrolment, and
`VmConfig.cgroup_scope` (`overdrive-core/src/vm/config.rs:898`) is not read by it. A `serve` crash between the spawn and the write leaves a
VMM that holds a vsock device outside an empty scope: boot reads `populated 0`, removes the scope and does not reserve, and a later launch is
refused `EADDRINUSE`. The stop path has the same hole. The wrapper chain (`prlimit`, `setpriv`, optionally `ip netns exec`) is one exec chain of
one process, so enrolling only the first pid also left every thread and child created before the write outside the scope.

**The contract (user ruling 2026-11-23, review finding F1; ADR-0352, accepted).**

1. **The postcondition.** On `Ok`, `Vmm::create` has returned a VMM whose process was created inside `config.cgroup_scope`: its first instruction ran
   as a member, so every process and thread of the VMM tree is a member from its first instruction. *Precondition:* the scope directory exists and
   is a valid placement target for the caller (item 8). On `Err` no process remains. `Vmm::create`'s rustdoc and `VmConfig.cgroup_scope`'s rustdoc
   state this. No signature changes.
2. **The spawn primitive.** One private module of `overdrive-host`, `crates/overdrive-host/src/vmm/launch_clone.rs` (beside `launch_seccomp`), owns
   the spawn. Its one safe entry point takes the prepared launch (program path, argv, the scope's directory descriptor, the seccomp program, the
   stdio descriptors) and returns the child's pid, its pidfd and the read end of its stderr to the adapter; the other names in the
   module are the crafter's. It performs a raw `clone3` with `CLONE_INTO_CGROUP | CLONE_PIDFD`, `cgroup` set to the scope's directory descriptor,
   `pidfd` pointing at a parent-side `c_int`, `exit_signal = SIGCHLD`, no `CLONE_VM`, `CLONE_THREAD` or `CLONE_PARENT_SETTID`, `stack = 0`, and
   `size` of at least `CLONE_ARGS_SIZE_VER2` (88). The child then `exec`s the existing launch chain. `std::process::Command` and
   `tokio::process::Command` are not used for the VMM launch. `register_launch_child_hook` is deleted.
   *Rejected alternative, the child's own `cgroup.procs` write:* it leaves a window in which the child runs outside its scope, so the guarantee
   rests on the window being harmless and not on a kernel property. Accounting is not the reason: `copy_mm` (`kernel/fork.c:2223 @ v7.0`) runs
   before `cgroup_can_fork` (`:2322`), so `clone3` charges the address-space copy to `serve`'s cgroup as well; only the charges made after
   `exec` land in the scope under either mechanism.
   *Why a pidfd (CLONE_PIDFD):* it gives (a) a handle that names this process and no other, so the kill cannot hit a recycled pid; (b) exit
   observation as a pollable descriptor (`pidfd_poll`, `fs/pidfs.c:278-299 @ v7.0`) that `tokio::io::unix::AsyncFd` waits on, with no blocking
   thread and no ownership of `SIGCHLD`; (c) the reap through `waitid(P_PIDFD)` (`kernel/exit.c:1770-1779`) with the exit status; and (d) a kill
   through `pidfd_send_signal`, whose default scope is the thread group (`kernel/signal.c:4100-4106`). The pidfd is `O_CLOEXEC`
   (`kernel/fork.c:1894`) and is installed in the parent's table only (`:2248-2251`, `:2463`).
3. **What the child does** (the audited hook of ADR-0129 and ADR-0143, now run in the clone child, in this order, and nothing else): set the
   signal mask empty and `SIGPIPE` to its default disposition, as std's spawn does (the Rust runtime ignores `SIGPIPE`, and an ignored disposition
   survives `exec`); `dup2` the stdio descriptors onto 0, 1 and 2; `close_range(3, ~0, CLOSE_RANGE_CLOEXEC)`; `prctl(PR_SET_NO_NEW_PRIVS, 1)`;
   load the seccomp filter; `execve`. Any failing step writes its step identifier and `errno` to the error pipe (item 4) and ends the child with
   `_exit`. The steps and their order are those the hook runs today; the filter is the same `VmmLaunchSeccompFilter`. (Joining the cgroup is
   `clone3`'s own step in the kernel, `CLONE_INTO_CGROUP`; its failure is reported by `clone3` in the parent and never reaches the pipe, item 8.)
4. **Standard I/O, environment, argv, failure report.** Preserved exactly: stdin and stdout are `/dev/null`, stderr is a pipe whose read end is
   the adapter's stderr capture (the bounded `VmmDiagnostics` capture of today), the environment is `serve`'s environment at the spawn with nothing
   added or removed, the working directory is inherited, and the argv is the one `build_confined_command` builds today (wrapper prefix, hypervisor
   binary, `--seccomp`, `--landlock` and the rules). Whether `build_confined_command` keeps returning a `Command` the primitive reads, or returns
   the argv directly, is the crafter's. The program is resolved to an absolute path **before** the clone, by the search `execvp` performs over `serve`'s `PATH`
   (the first executable candidate; when no candidate is executable but one exists, the first existing candidate is returned and its `execve`
   reports `EACCES`, as `execvp` ends with `EACCES`), so the child calls `execve` and never searches; a wrapper with no candidate on `PATH`
   is reported as `io::ErrorKind::NotFound`, which `classify_launch_spawn_error`
   already maps to `ConfinementUnavailable`. Every parent-side descriptor the child `dup2`s from (the `/dev/null`
   descriptor, the stderr pipe's write end) is moved with `fcntl(F_DUPFD_CLOEXEC, 3)` before the clone and the original closed, so its number is at
   least 3 whatever `serve`'s own stdio is: with stdio closed, `open` and `pipe2` return 0 to 2, and a source in 0 to 2 would be clobbered by an
   earlier `dup2` (`F_DUPFD_CLOEXEC` takes the lowest free descriptor at or above its argument, `fs/fcntl.c:460-461` into `f_dupfd`,
   `fs/file.c:1498-1504 @ v7.0`). The same holds for the error pipe's write end and the scope descriptor, which the child does not `dup2` but
   must not lose to a `dup2` target. The parent opens an error pipe (`O_CLOEXEC`
   both ends) before the clone and reads it to end after, **in `spawn_blocking`, never as a blocking read on a runtime worker thread** (item 5, hand-off).
   The end-of-file can be delayed: a concurrent launch's `clone3` copies `serve`'s descriptor table (no `CLONE_FILES`), so that launch's child
   holds a copy of this pipe's write end until its own `exec` or `_exit` closes it. The delay is bounded by that child's non-blocking steps and
   no timeout is added. End-of-file with no data means the child reached `exec`; five bytes are the child's
   step and `errno` (the payload below), in which case the parent, still inside the same `spawn_blocking` call, reaps the child
   (`waitid(P_PIDFD, WEXITED)`; the child is ending in `_exit`), closes the descriptors and returns the error. Exec failure therefore
   keeps the semantics std's spawn gives today.

   **The error-pipe payload (user ruling 2026-11-23, option a: a failed launch names its step).** The child writes exactly five bytes in one
   `write`: one step byte, then the `errno` as a native-endian `i32`. The parent reads to end-of-file: no data is "reached `exec`"; five bytes
   are a step and an `errno`; any other length is a child that died mid-write and is reported as the step `Execve` with the `errno` `EIO`
   (the parent never guesses a step from a short read). The encoding is private to `launch_clone.rs`; it is a contract between the child and
   the parent of one module and is not a wire or persisted format.
   *The closed step set.* A crate-private enum `LaunchStep` in `launch_clone.rs`, a fixed `u8` discriminant per variant, with
   `as_str(self) -> &'static str` on the enum (`.claude/rules/rust.md` § "Label enums own their string representation"); decoding an unknown
   byte is the `Execve` case above. The steps, in the child's order:

   | Step | `as_str` | Child step it names |
   |---|---|---|
   | `SignalReset` | `resetting signal state` | empty signal mask and `SIGPIPE` default |
   | `Stdio` | `wiring standard I/O` | the `dup2` calls |
   | `CloseRange` | `closing inherited descriptors` | `close_range` |
   | `NoNewPrivs` | `setting no_new_privs` | `prctl(PR_SET_NO_NEW_PRIVS)` |
   | `SeccompLoad` | `loading the seccomp filter` | the filter load |
   | `Execve` | `executing the launch program` | `execve` |

   Joining the cgroup is not a `LaunchStep`: it happens inside `clone3` in the kernel, before the child has a first instruction, so its refusal
   is a `clone3` error of the parent and is named by the parent (item 8, `joining the cgroup`).
5. **Holding, waiting for and killing the child.** `HostVmm`'s reaper task owns the child: it holds the pidfd and the stderr reader and nothing
   else holds a handle. Exit is `AsyncFd` readability of the pidfd followed by `waitid(P_PIDFD, WEXITED | WNOHANG)`, looped until it reports the
   exit; the siginfo (`CLD_EXITED`, `CLD_KILLED`, `CLD_DUMPED`, `si_status`) yields the same `VmmExit { exit_code, signal }` as `classify_exit` does
   today. The kill is `pidfd_send_signal(pidfd, SIGKILL)`; `ESRCH` is "already gone", not an error, as `let _ = child.start_kill()` is today. The
   `live` map, `VmProcessState { kill_now, outcome }`, `terminate`, `VMM_REAP_BOUND`, `KilledNotReaped` and the late reap are unchanged (ADR-0348):
   the pidfd is readable exactly when the thread group is dead and not held by a delayed leader, so a VMM with a thread stuck in uninterruptible
   I/O is not reapable here either, and `terminate` returns `KilledNotReaped` at the bound as it does for `Child::wait` today. `cgroup.kill` is
   still written only by `VmHostState::kill_scope` (ADR-0349, ADR-0351).
   **Hand-off and cancellation (no orphan on a dropped `create`).** The whole launch is one synchronous function of `launch_clone.rs` that
   `create` runs inside `tokio::task::spawn_blocking`: it resolves the program, opens the descriptors, calls `clone3`, reads the error pipe
   to end and returns. A `spawn_blocking` closure runs to completion once started, whether or not the future awaiting it is dropped, so the
   launch is never cut between `clone3` returning and its result existing. The child is owned from the line after `clone3` returns, with no
   `.await` and no fallible step between: the primitive wraps the pidfd in a `LaunchedChild` guard at once. **The guard kills and hands the
   pidfd to a reaper on drop:** `Drop` sends `pidfd_send_signal(SIGKILL)` (`ESRCH` is ignored) and moves the pidfd to a detached reap task
   spawned on the `tokio::runtime::Handle` the primitive captured before it entered the blocking pool (`Handle::spawn` is synchronous and
   takes no lock); that task waits for readability and `waitid`s, with the bounded reap and late reap of ADR-0348, so a killed child is never
   left a zombie. `HostVmm` takes the guard out of its `Option` into the reaper (item 5) only after `create`'s own success path has passed;
   a guard dropped on any earlier path (an error-pipe failure, the `create` future dropped while the blocking call runs and its output
   discarded) therefore kills and reaps the VMM and leaves no process. **Decided under this design's authority:** the kill-and-reap-on-drop
   guard is the contract of the hand-off, not an option left to the crafter; a launch that is dropped never leaves a running VMM. Test: a `create` future dropped while the launch is in `spawn_blocking`
   leaves no process of the stand-in and no zombie (Tier 3, below).
6. **The unsafe surface.** `launch_clone.rs` is the crate's one production `#[allow(unsafe_code)]`. The waits, the signal, the pipe and the
   descriptor handling around it use safe `rustix` wrappers, as the crate already does for `ioctl_ficlone`. The module states the safety argument:
   - *After `clone3` the child is a single-threaded copy of a multithreaded parent.* A lock another thread held at the clone (an allocator arena, a
     stdio lock, a tokio internal) stays held for ever in the child, so the child, from the instant `clone3` returns `0` until `execve`, allocates
     nothing, takes no lock, runs no destructor and runs no code that can panic or unwind. It makes only direct system calls and the
     async-signal-safe libc calls `dup2`, `sigprocmask`, `sigaction`, `write`, `execve` and `_exit`.
   - *Everything the child needs is built before the clone:* the NUL-terminated absolute path, the argv and envp pointer arrays, the seccomp program,
     the descriptor numbers and the error pipe. The child never returns to Rust code: it ends in `execve` or `_exit`.
   - *Descriptors are opened `O_CLOEXEC` before the clone* (the scope directory as `O_PATH | O_DIRECTORY | O_CLOEXEC`; the kernel takes an `O_PATH`
     descriptor of the directory, `kernel/cgroup/cgroup.c:6796`, `:6824-6837`), so a concurrent `exec` elsewhere in `serve` cannot inherit them;
     the child's `close_range` marks everything from 3 up close-on-exec, as the hook does today.
   - *Child kind.* `exit_signal = SIGCHLD` makes it an ordinary child that `waitid` reaps without `__WCLONE` (`eligible_child`,
     `kernel/exit.c:1161`).
7. **The kernel facts the placement rests on** (read at `v7.0`):

   | # | Fact | Source |
   |---|---|---|
   | C1 | `CLONE_INTO_CGROUP` is accepted by `clone3` only; `cgroup` must not exceed `INT_MAX` and `size` must be at least 88 | `kernel/fork.c:2840-2842`, `:2899`; `include/uapi/linux/sched.h:109` |
   | C2 | The target is a cgroup2 directory descriptor: an `O_PATH` descriptor is accepted, a `cgroup.procs` file is not; a non-cgroup or non-directory is `EBADF`, a v1 cgroup is `EBADF`, a removed cgroup is `ENODEV` | `kernel/cgroup/cgroup.c:6796-6813`, `:6824-6837`; `css_tryget_online_from_dir` `:7245-7248`; `cgroup_get_from_file` `:6745-6748` |
   | C3 | Permission and delegation are the checks the `cgroup.procs` write makes: write permission on the target's `cgroup.procs` with the caller's credentials, write permission on the common ancestor of source and target, the namespace-visibility rule (`ENOENT`), `EOPNOTSUPP` for an invalid domain and `EBUSY` for a target with `subtree_control` set | `cgroup_may_write` `:5298-5312`, called at `:6820`; `cgroup_attach_permissions` `:5345-5364`, called at `:6838`; `cgroup_migrate_vet_dst` `:2800-2822`; the write form's own call `:5394-5397` |
   | C4 | Placement is atomic with the child's creation: `copy_process` calls `cgroup_can_fork`, which takes `cgroup_mutex` and the threadgroup lock and builds the child's css_set; `cgroup_post_fork` links the child onto it; `kernel_clone` wakes the child only afterwards. A failure after `cgroup_can_fork` runs `cgroup_cancel_fork` and no process exists | `kernel/fork.c:2322`, `:2467`, `:2483`, `:2684`; `kernel/cgroup/cgroup.c:6777-6780`, `:6844`, `:6992-6993` |
   | C5 | A `cgroup.kill` that lands between the css_set being built and the child being linked kills the child as soon as it is linked, so a clone in flight cannot escape a scope kill | `kernel/cgroup/cgroup.c:6785-6788`, `:7024`, `:7047-7049` |
   | C6 | `execve` does not change cgroup membership: `fs/exec.c` names cgroup only to hold the threadgroup lock around `de_thread` | `fs/exec.c:946-957`, `:1011` |

   The source and target of the placement are the same as for the write `place_pid_in_scope` makes today (the VMM was forked from `serve`, so it
   starts in `serve`'s cgroup), so the delegation checks C3 are the ones the driver's write passes in every Tier-3 run now.
8. **Failure.** A refusal at any of these points fails `create` and creates no process, the rootfs clone is removed (`launch-spawn` stage), and
   the error is classified as every `create` error is. No `VmmError` variant or field is added: `VmmError::Create { detail: String }` already
   carries the step in its `detail`, which `start_rejected_unclassified` shows the operator as it shows every `Create` detail today:

   | Refusal | Surfaces as |
   |---|---|
   | opening the scope directory fails (absent, no search permission) | the primitive's `io::Error` (step `joining the cgroup`), before any clone; `VmmError::Create` through `classify_launch_spawn_error`, with the same `detail` as the `clone3` row below |
   | `clone3` returns `EBADF`, `ENODEV`, `EACCES`, `EPERM`, `EBUSY`, `EOPNOTSUPP`, `ENOENT`, `ENOMEM` | the primitive's `io::Error` (step `joining the cgroup`), `VmmError::Create` through `classify_launch_spawn_error`; the `detail` reads `joining the cgroup failed: <launch executable>: <errno text>` |
   | a child step fails (`LaunchStep`, item 4) | the step and `errno` from the error pipe, `VmmError::Create`; the `detail` reads `<step.as_str()> failed: <launch executable>: <errno text>`, for example `loading the seccomp filter failed: /usr/bin/prlimit: Invalid argument (os error 22)`; an unresolvable wrapper is `NotFound` at the pre-clone resolution and maps to `ConfinementUnavailable` (unchanged) |

   `classify_launch_spawn_error` keeps its classification (it reads the `errno`'s `io::ErrorKind`); the step is added to the `detail` text it
   builds and changes no variant. A failing `execve` whose `errno` is `NotFound` for the wrapper is still `ConfinementUnavailable`.

   `EINVAL` from an undersized `clone_args` is a programming error the Tier-3 launch test catches; it has no operator recovery.
9. **`CloudHypervisorVmm` needs the cgroup root** to resolve `CgroupPath` to a directory: `with_cgroup_root(PathBuf)`, a builder beside
   `with_binary`, `with_image_dir` and `with_run_dir_root`, defaulting to the cgroup2 mount (`/sys/fs/cgroup`). The composition root passes the value
   it gives `CgroupManager`. With the default and no such scope the open fails typed, so a test that forgets it fails and does not touch another
   cgroup.
10. **`place_pid_in_scope` is deleted.** The driver's call and its failure branch (`vm_driver.rs:1848-1861`, the
    `start_rejected_unclassified("place VMM pid in scope")`), `CgroupManager::place_pid_in_scope` and
    `tests/acceptance/cgroup_manager/place_pid_in_scope.rs` go in the same change; it has no other caller. The resource limits
    (`write_resource_limits`, `:1563`) are written before the spawn, so `memory.max` applies from the VMM's first instruction.
11. **`SimVmm`.** `create` records the process as a member of `config.cgroup_scope` from its creation until the scripted reap; `SimVmHostState`
    observes that one fact. A `SimVmm` with an unreapable process (ADR-0348) keeps its scope populated. A `SimVmm` whose scope is absent fails
    `create` typed, as the host adapter does.
12. **The launch seccomp boot probe is dropped (P-6, user ruling 2026-11-23).** The probe ran through the `pre_exec` hook this change deletes
    and is not ported to `launch_clone`; the real launch is the check. Overdrive ships its own appliance OS with a pinned kernel (ADR-0068), and
    the seccomp filter is part of every launch, so a filter the kernel cannot load fails the first launch (item 8). Whether `serve`'s other
    startup checks are needed is #376. Deleted in the same change:
    - `VmmProbeSubstrate::check_launch_seccomp` and its `RealVmmProbeSubstrate` implementation (`overdrive-host/src/vmm.rs`), and the test
      double's `launch-seccomp` stage;
    - the call site, the `launch-seccomp` stage of `CloudHypervisorVmm::probe`, and the stage-order expectation that names it (the probe's other
      stages and their order are unchanged);
    - its tests: the stage-order and typed-error cases for the launch-seccomp stage, `the_startup_probe_installs_the_exact_launch_program`
      (S-ND295-43), and the probe half of `launch_on_a_target_without_a_program_is_refused_before_any_effect` (S-ND295-44; the half asserting
      `create` refuses before any effect stays);
    - the boot-refusal causes only the probe returned: `VmmProbeError::LaunchSeccompUnsupportedArch`, `LaunchSeccompInstall` and
      `LaunchSeccompProbeExit`, their constructors, and the `Vmm::probe` rustdoc sentence naming them (`overdrive-core/src/traits/vmm.rs`).
    A seccomp failure at launch surfaces at the first launch, not at boot: an architecture with no filter program fails `create` before any
    effect as `VmmError::ConfinementUnavailable { control: Seccomp }` (unchanged, `create`'s own check); a filter the kernel refuses to load
    is a child-step failure of item 8, whose `errno` comes back through the error pipe, the child ends with `_exit`, and `create` fails as
    `VmmError::Create` through `classify_launch_spawn_error`, never an unconfined run. The error pipe carries the step with the `errno`
    (item 4, user ruling 2026-11-23, option a), so the `detail` names the step, `loading the seccomp filter failed: ...`; no `VmmError` variant
    is added here.
13. **The contract shape.** `Vmm::create` is a bounded-change function: it creates one process tree and changes membership of exactly one scope.
    The universe of what it may change is the allocation's run directory, one scope and one process tree.

**Is it contract-shaping validation?** No, and no spike is listed. The test is whether a falsified premise would change a contract DISTILL pins:
the port's postcondition (created inside the scope), `VmControl { pid, api_socket }`, `VmmError` and the `Vmm` trait are the same under any
placement mechanism, and the one adapter-visible choice, the cgroup root, is a builder. The premises are all kernel behaviour, and each is read in
the source (C1 to C6 above) rather than assumed. The one a spike would otherwise settle, whether `serve` running as root under the guest's cgroup
delegation may place a child in a workload scope, is answered by identity: C3 applies to `CLONE_INTO_CGROUP` the same permission checks it applies
to the `cgroup.procs` write, with the same source and target, and that write succeeds in every run of the production launch today. A falsified
premise would reject the mechanism the user ruled on, which returns to the user; it would not reshape what DISTILL pins. The first Tier-3 test
below discharges the rest (membership survives the exec chain, C6; the child runs the confinement steps).

**Boundary scenarios.** (1) A normal launch: the VMM tree is in the scope from its first instruction; the scope's `cgroup.procs` lists the wrapper
pid at `create`'s return. (2) The scope cannot be a target (removed, `subtree_control` set, not writable): `create` fails typed and no process
exists. (3) `serve` dies at any instant of the launch: the child is either not yet created or a member of the scope from its first instruction;
no hypervisor process runs outside the scope. (4) A mesh VM (`ip netns exec` first): the same, the first program is `ip`. (5) A `cgroup.kill` of
the scope races a launch: a child created before the kill is killed, one created after is a member (C5). (6) The hypervisor binary is absent or
the wrapper is not found: the error pipe reports it, the child is reaped, no process remains.

**Tests.**

- **Test seams: none is added.** The launch tests need three things, and each is existing surface. (1) *The stand-in hypervisor* is a small Rust
  program the test compiles with the guest's own `rustc` and passes to `with_binary`; the `cgroup-default` / `nested-launch-rustc` profile
  provides `rustc` and `cc` in the guest (`overdrive-testing/src/guest_suite.rs`, the profile the nested-launch tests already use). It is test
  support, DISTILL's, and sits in the test's own tempdir. (2) *The launch wrapper* is the confinement's `prlimit` (or `ip` for a mesh VM),
  resolved on `serve`'s `PATH` (item 4); a test in its own guest sets `PATH` to a tempdir first, holding a `prlimit` file with mode `0644`, so the
  wrapper resolves and `execve` returns `EACCES` (the step `executing the launch program`). No builder chooses the wrapper: it is derived from the
  confinement, and a seam beside it would let a test run a launch the product never builds. (3) *The cgroup root* of `CloudHypervisorVmm` is
  `with_cgroup_root` (item 9), a builder the test-only `with_binary` already sits beside; the composition root passes the value it gives
  `CgroupManager`. A stand-in cannot hold a thread in uninterruptible I/O, so the `KilledNotReaped` Tier-3 case below runs only as S-VOL-T3-12's
  real held flush; the stand-in test covers a normal exit, a signal and a held-after-`exec` stand-in. The `VmmExit` core-dump case is covered by
  a unit test of the siginfo-to-`VmmExit` mapping over synthetic `CLD_DUMPED` values, not by a real core dump.
- Tier 3 (real cgroupfs, a `cgroup-default` guest): the hypervisor stand-in supplied through the existing `with_binary` seam records its own
  `/proc/self/cgroup` as its first act; the record names the scope, for a plain and a mesh launch. The same test reads the `cgroup.procs` of the
  scope after `create` returns and finds every process of the tree, and reads the stand-in's `/proc/self/status` `Seccomp` and `NoNewPrivs`
  fields (the confinement steps ran before `exec`), its stdio (stdin and stdout `/dev/null`, stderr the capture) and its signal disposition of
  `SIGPIPE` (default).
- Tier 3, serve crash: with the stand-in holding the launch just after `exec`, crash the in-process `serve` (the existing crash-and-restart
  fixture) and start a second `serve`: no process of the stand-in is outside a scope the boot step reclaims; and with the real hypervisor, a launch
  sweep after the boot never receives the CID the crashed VMM held and produces no `EADDRINUSE`.
- Tier 3, refusal: `create` against a scope removed before the call, and against a scope with `subtree_control` set, each fails typed with no
  process created and the rootfs clone removed.
- Tier 3, step naming (a `cgroup-default` guest, through `with_cgroup_root`, the guest's `PATH` and the existing `create` seams): `create` fails with a `detail`
  naming the step for each of the steps a test can make fail with no new production seam: `joining the cgroup` (the scope removed or
  `subtree_control` set, the same fixtures as the refusal test), and `executing the launch program` (a wrapper that resolves to an
  absolute path and is not executable, so `execve` returns `EACCES`; the wrapper is a mode-`0644` `prlimit` first on the guest's `PATH`). Each asserts the `detail` contains the step's `as_str` and the
  `errno` text, and that no process remains. `loading the seccomp filter`, `wiring standard I/O`, `closing inherited descriptors`,
  `setting no_new_privs` and `resetting signal state` cannot be made to fail from a test without a production seam (a fault-injecting
  filter, a descriptor limit the child cannot lift, a syscall interposer): none is invented. They are covered by a unit test on the
  pipe codec in `launch_clone.rs`: every `LaunchStep` round-trips through the five-byte payload, an unknown step byte and a short read
  decode as `Execve` / `EIO`, and `as_str` is distinct for every step. That the real child writes the right step byte for each failing
  step is read in the module's audited child sequence at review, not tested.
- Tier 3, reap: the stand-in's exit is observed through the pidfd with the right `VmmExit` (a normal exit, a signal, a core dump); `terminate`
  kills through the pidfd; the `KilledNotReaped` case is S-VOL-T3-12's real held flush (a stand-in cannot hold a thread in uninterruptible I/O).
- Tier 3, cancellation: a `create` future dropped while its launch is inside `spawn_blocking` leaves no stand-in process and no zombie child
  of `serve` once the launch returns.
- Host adapter, unit: an unresolvable wrapper fails `create` with no spawned child and maps to `ConfinementUnavailable`; the scope, pidfd, error
  and stdio descriptors are not inherited by the hypervisor; with `serve`'s descriptors 0 to 2 closed, a launch still gives the stand-in its
  `/dev/null` stdin and stdout and its stderr capture, and the child's `dup2` sources are all at least 3.
- `Vmm` equivalence: `CloudHypervisorVmm` and `SimVmm` agree that a created process is a member of `config.cgroup_scope` until reaped, that a
  missing scope fails `create`, and on the `terminate` outcomes (ADR-0348).
- Tier 1, production `VmDriver` over `SimVmm`: the driver makes no `place_pid` call (the method no longer exists) and a VM whose `create` fails
  leaves no scope member.

### Cost budget of the stuck-VM design (`design.md` § "No unrequested scope")

- **Launch hot path.** `clone3` without `CLONE_VM` copies `serve`'s address space exactly as the `fork` inside `std`'s spawn does today, so the
  fork cost is unchanged; `CLONE_INTO_CGROUP` builds the child's css_set in the same call instead of a later `cgroup.procs` write, which
  removes one filesystem write per launch. The added work per launch is the program resolution (one `PATH` search, as `execvp` does in the
  child today), a handful of `fcntl` and `pipe2` calls, one `spawn_blocking` hop, and the pidfd (one descriptor). The launch is not on a
  request path and is dominated by the guest boot (seconds). Launch latency budget: no measurable regression against today's fork+exec.
  Measured in the first Tier-3 launch test; pending.
- **Extra durable write.** One additional `Pending` observation-row write per launch (RB-6), before `Vmm::create`; one row, one LWW write,
  on the launch path that already performs the network provisioning and the rootfs clone. No additional write on a running allocation.
- **Boot.** One pass over the node's `Pending` rows and one bounded wait: boot waits at most one `KILL_SCOPE_SETTLE_BOUND` for all stuck
  scopes together (parallel executors), instead of one per scope.
- **Validation.** The new tests are default-lane unit and Tier-1 seeded tests plus Tier-3 tests in `cgroup-default` guests (membership,
  refusal, step naming, reap, cancellation, serve crash); no new lane, build variant or toolchain. Model checking: `cid_lease` and the runs it
  carries, re-checked by the modeler; the checks' wall time and memory are **pending** (taken from the modeler's recorded evidence, not
  estimated here), within VB-3's bound (`lease_five` without `StopUnreaped`; any growth returns to the user).

### Boot proceeds past a stuck VM (RB-4; ADR-0350, which amends ADR-0156)

**Today.** `vm_reclamation_boot::converge` (`crates/overdrive-control-plane/src/vm_reclamation_boot.rs:75`) runs the reclamation executors for every
leftover VM, and `kill_scope(...).await?` in `execute_reclaim_allocation` (`action_shim/reclamation.rs:239`) turns any error into
`ConvergeError::Reclamation`, which `run_server` returns at `lib.rs:4552-4563`: `serve` refuses to boot. With the bounded `kill_scope` a leftover
VM whose last task is stuck in exit would refuse the boot after the bound, on every restart, until the I/O drains.

**The contract (user ruling 2026-11-23, decision 14, option c: "the node should not refuse to start just because of a dead vm").** `converge` keeps
its signature (`async fn converge(state: &AppState) -> Result<(), ConvergeError>`). In order:

1. **Close the lost launches.** Before anything else, every row of this node that is still `Pending` is ended with the one row of § "Launches that
   end before running" (W4: `Terminated`, `LaunchLostWithControlPlane`, the row's `workload_addr` carried), whether or not its VM left a scope. A
   boot process owns no launch from before the restart, so every `Pending` row belongs to a dead launch. The step reserves nothing and kills
   nothing; it only makes the row terminal, so everything after it sees a terminal row for a launch lost with the control plane.
2. **Observe.** The leftover VM scopes (`VmHostState::observe`) and the allocation rows (`alloc_status_rows`, read after step 1; the observation
   store opens in the composition root before `run_server`, and `GuestAddressPool::new` at `lib.rs:4346` runs before `converge` at `:4552`). A scope
   maps to its allocation by name, `<alloc_id>.scope` (`cgroup.rs:57-62`, `observe_scopes`, `vm_host_state.rs:123-143`).
3. **Kill, in parallel.** The executors run for all scopes concurrently, each waiting at most `KILL_SCOPE_SETTLE_BOUND`, so boot waits one bound in
   the worst case and not one per stuck scope. **Every executor runs to completion and its result is collected** (a `join_all` over the planned
   actions, not a short-circuiting `try_join` and not today's `?` inside the loop): one scope's failure never drops another scope's result, so it
   can never lose that scope's reservation. The results are kept in the planner's action order (a `BTreeMap` of scope, deterministic under
   seeds).
   **How `converge` learns a scope is busy.** An executor reports busy as `Err(ReclamationError::Host(err))` with
   `err.kind() == io::ErrorKind::ResourceBusy` (the `kill_scope` result of ADR-0351 reaching the executor's `?`). The ADR-0354 Running case, whose
   executor writes the terminal row and then still reports the busy error, arrives the same way: the row is already written, the result is still
   `Err(Host(ResourceBusy))`, and `converge` treats it as busy and not as a failure. No field or return type of the executors changes; the kind is
   the signal. Any other result is not busy: `Ok` (settled, nothing to reserve), `Host` with another kind, or `Observation`.
4. **Reserve what is still busy.** Each scope whose result is busy (item 3) is a VM still holding its device. For each, `converge`
   reserves the guest-address offset named by the allocation's row `workload_addr` (§ "The guest address is on record before the VMM exists") in
   `GuestAddressPool`, before the first `assign`. The row is read as step 2 saw it: the row step 1 closed carries the address of the `Pending`
   row it ended, so closing the row first loses nothing the reservation needs. A scope that settled within the bound is removed and reserves
   nothing. A scope whose row carries
   no `workload_addr`, or which has no row, is not reserved: the address is on record before the VMM exists, so such a VMM cannot hold a CID (a
   scope with no row is a stranded artifact the planner already discards, row 5 of `plan_reclamation`). No assign runs between the kill and the
   reservation, because boot reclamation completes before the convergence loop dispatches anything (ADR-0102, `lib.rs:4530` before the loop).
5. **A busy scope is not an error.** Any other executor error stays `ConvergeError::Reclamation` and refuses the boot: it is a substrate
   refusal, not a stuck VM. The row of a busy scope is § "A busy boot scope writes its terminal row". **Ordering that keeps every
   reservation:** all executors finish (item 3), then `converge` reserves for every busy scope (item 4), and only then returns: it refuses with the
   first non-busy executor error in action order, or, if there is none, with the first reservation failure of the corrupt-row case below. A non-busy
   error in one scope therefore never prevents the reservation of another's busy VM; the refused boot reserved what it could, which is harmless
   because the pool is rebuilt at the next boot from the same rows.
6. **Surface.** The existing surfaces carry it: the allocation's terminal row, the scope that stays in `VmHostState::observe()`, and the attachment
   occupancy (`GuestAttachmentOccupancy` `held` and `retiring`) that `describe` already shows (ADR-0336). No new event, row, field or `describe`
   line.
7. **Continue.** `converge` returns `Ok(())`; cleanup retries on every later pass.

**The order is the contract (user ruling 2026-11-23, RB-11).** Step 1 precedes the observation and the executors; the reservation (step 4) follows
the kill and precedes the first `assign`, which cannot run before `converge` returns (the convergence loop, the interest router and the HTTP
listener start after it, `lib.rs:4530` before `:4813`, `:4861`, `:5032`). The executors never rewrite the reason of a row step 1 closed: the planner
plans `ReclaimAllocation` only for a non-terminal row and `DiscardStrandedArtifacts`, which authors no row, for a terminal one
(`plan_reclamation`, `vm_reclamation.rs:145-189`), and `execute_reclaim_allocation` refuses a terminal row as a no-op. Step 1 is idempotent: a
crash between it and the executors leaves terminal rows, which the next boot's executors discard and whose busy scopes it reserves from the same
carried address.

**Pool API (crate-internal to `overdrive-control-plane`).** `GuestAddressPool::reserve_leftover(&self, alloc: &AllocationId, held: Ipv4Addr)`,
with `held` the row's `workload_addr`; it returns `Result<(), CorruptLeftoverRow>` (the corrupt-row case below, and nothing else). The pool derives the offset as `held - GUEST_NODE_PREFIX.network()`, the inverse of the
one derivation `assign` uses, so the CID is still computed in one place. It inserts a **Retiring lease** for `alloc` at that offset: the existing
lease type in the state a stuck VMM's lease already has (ADR-0156), so the held bitmap, `leases`, the occupancy counts and the attachment cap stay
one source of truth, and a reservation counts toward the cap. A reservation carries no `GuestNetworkPlan`: no network or flow ports exist for it, and
`snapshot()` does not list it (`observe` does). `assign` for the same allocation id returns the existing `LeaseRetiring`. An address outside the
prefix, or an offset already held by another allocation, is **corrupt persisted data, not a structurally impossible path**: the address is read from
a row at boot, and a row can be damaged or written by another build, so `reserve_leftover` never panics on it (`unreachable!` is wrong here, `rust.md`
§ "Logically unreachable"). `reserve_leftover` returns the failure and `converge` refuses the boot with `ConvergeError::Reclamation`, naming the allocation and the row's address.
A corrupt row is its own failure mode and has its own typed variant (`rust.md` § "Errors": distinct failure modes get distinct variants);
`ReclamationError::Host` stays the `VmHostState` substrate failure and is not reused for it.

**How the pool reports a reservation (`GuestAddressPool::lease_observation`, QPROJ-2).** A reservation has a CID and no launch token, so the
view of a lease is a sum, and a reservation with a launch token is unrepresentable. The `lease_observation()` signature and
`GuestLeaseObservation { next_fit, leases: BTreeMap<AllocationId, GuestLeaseView> }` are unchanged: `leases` keeps every Admitted and Retiring
lease, and a reservation is one of them (it is a Retiring lease), so the cap, the occupancy counts, `CidUnique` and the release gate see it
where they saw it before. `GuestLeaseView` changes from a struct to an enum (`overdrive-control-plane`, `guest_network`, re-exported
`pub` from `guest_network`):

- `GuestLeaseView::Launched { cid: GuestCid, state: GuestAttachmentLease, launch: LaunchToken }`: a lease `assign` made. `state` is Admitted or
  Retiring, as today.
- `GuestLeaseView::Reserved { cid: GuestCid }`: a lease `reserve_leftover` made. It is Retiring by construction and has no launch token field.
- Accessors, all `const fn` and infallible: `cid(&self) -> GuestCid` (both variants); `state(&self) -> GuestAttachmentLease` (`Launched` returns
  its `state`; `Reserved` returns `Retiring`); `launch(&self) -> Option<LaunchToken>` (`Some` for `Launched`, `None` for `Reserved`).
  The struct's `pub` fields `cid`, `state` and `launch` are removed; a reader that needs the launch token calls `launch()` and decides what the
  absence means for it.

*Rejected.* (1) `launch: Option<LaunchToken>` on the struct: it keeps the struct shape but makes a reservation with a launch, and an Admitted
reservation, representable, and every reader gets an `Option` it can `unwrap` away. (2) A `Reserved` variant of `GuestAttachmentLease`: that type
is the `overdrive-core` read-port vocabulary of `GuestAttachmentView` that `WorkloadLifecycle` and the operator status match on
(`cleanup_pending`), and a reservation has the Retiring meaning there; a third variant changes core public API and every exhaustive match for a
distinction no reconciler decision uses. (3) Omitting reservations from `lease_observation` and listing them in a second map: `leases` would stop
being "every lease the pool holds", `CidUnique` and the stuck-VM tests that need the reserved CID would read two maps, and the one place that
states the invariant would split.

*Readers.* No production reader consumes `GuestLeaseView`. The production hydration reads `GuestAttachmentView::observe` through
`GuestPoolAttachmentView`, whose type is `GuestAttachmentObservation { occupancy, leases: BTreeMap<AllocationId, GuestAttachmentLease> }` in
`overdrive-core`; it already reports a reservation as Retiring and carries no launch token, so `WorkloadLifecycle` (`guest_attachments`), the
capacity gate and the operator status are unchanged, and nothing an operator observes changes. Every `GuestLeaseView` reader is a test,
conformance driver or fixture: a reader of the launch token (the guest's READY, the placed-instance fixture, the owner-flow and cid_lease
projections' launch comparison) calls `launch()` and fails its step when a lease it expects to be launched is `Reserved`; a reader of the CID or
state (the cid_lease `lease` projection, the G-V0 uniqueness and range checks, the vm-reclamation boot assertions) calls `cid()` or `state()` and
sees reservations like any other lease. A conformance projection that models only launched leases rejects a `Reserved` view it did not expect
rather than omitting it.

**Public surface.** `overdrive-core` does not change. `GuestLeaseView` and `GuestLeaseObservation` are `pub` in `overdrive-control-plane`
(through `pub mod guest_network`) and are read by in-workspace test crates (`overdrive-sim`'s tests), so the struct-to-enum change is a public
API change of `overdrive-control-plane`; no external consumer exists. `GuestAddressPool::assign`, `release`, `snapshot`, `observe` and
`reserve_leftover` keep their signatures. This supersedes the struct shape pinned in the netns-density feature delta
(`docs/feature/netns-density-295/feature-delta.md`, QPROJ-2 `GuestLeaseView { cid, state, launch }`); its `lease_observation` contract text
(`leases` holds exactly the Admitted and Retiring leases; a Retiring lease keeps its CID and launch token) holds for every `Launched` lease, and a
`Reserved` lease keeps its CID with no launch token. The pool's private lease record follows the same split (a launched lease owns its plan and
state; a reservation owns neither plan nor launch), so that `assign` has no "Admitted without a plan" arm to declare unreachable; that is internal
structure and the crafter's.

*Contract* (`overdrive-control-plane`, `action_shim::reclamation`, beside `ReclamationError`):

- `ReclamationError` gains `CorruptLeftoverRow(#[from] CorruptLeftoverRow)`, a pass-through embedding (`rust.md` § "Errors", pass-through not
  duplication), with `#[error("{0}")]` so its text is the inner `Display`.
- `CorruptLeftoverRow` is a `pub` `thiserror` enum, `Debug`, with two variants, each naming the allocation:
  `AddressOutsidePrefix { allocation: AllocationId, address: Ipv4Addr }` and
  `OffsetAlreadyHeld { allocation: AllocationId, address: Ipv4Addr, held_by: AllocationId }`.
- Constructors, one per variant: `CorruptLeftoverRow::address_outside_prefix(allocation, address)` and
  `CorruptLeftoverRow::offset_already_held(allocation, address, held_by)`.
- `Display`: `AddressOutsidePrefix` is `leftover row for allocation {allocation} is corrupt: address {address} is outside the guest prefix
  {GUEST_NODE_PREFIX}`; `OffsetAlreadyHeld` is `leftover row for allocation {allocation} is corrupt: address {address} is an offset already
  held by allocation {held_by}`.
- `converge` maps it to `ConvergeError::Reclamation(ReclamationError::CorruptLeftoverRow(..))` unchanged (the existing `#[from]` on
  `ConvergeError::Reclamation`; no variant is added to `ConvergeError` or `GuestNetworkError`, no `io::Error` is involved, and the busy test of
  item 3 is unaffected because the kind is not `ResourceBusy`). Boot refuses and the error text names the row; the ordering of item 5 holds.
- **API surface.** `ReclamationError` is `pub` in a `pub mod`, and `ConvergeError` embeds it and is itself embedded in
  `ControlPlaneError::VmReclamationBoot` (`error.rs:749`), so both are public API of `overdrive-control-plane`, reachable by downstream crates.
  The new variant and the new type are public additions to that surface; neither enum is `#[non_exhaustive]`, so an exhaustive downstream
  `match` on `ReclamationError` adds an arm. Inside the workspace, the CLI renders these errors through `Display` and matches none.
  `reserve_leftover` and `GuestAddressPool` stay crate-internal.

This is the intent-store rule that corrupt persisted data refuses
boot (ADR-0048 § 3). It is corruption of a record, not a dead VM: ruling 14c ("never refuse boot for a dead VM", step 5 above) is unaffected,
because a stuck VM whose row is sound still reserves and boots. Two live VMMs cannot hold one CID (`EADDRINUSE`,
`drivers/vhost/vsock.c:804-813 @ v7.0`), so a duplicate offset is a damaged row, never two real VMs. Test: a leftover row with an address outside
the prefix, and two leftover rows carrying one offset, each refuse the boot naming the allocation; neither panics.

**The one release site.** A reservation is released only by `teardown_guest_network`, the lease gate of § "The guest-address lease outlives an
unreaped VMM", through the network-reclaim retry that the allocation's terminal row already triggers. For a reservation that function skips the
provisioner teardown (no plan) and releases once the scope is absent; while the scope is present it returns `Ok`, so the retry neither errors nor
stalls. There is no other release path and no release at boot.

**`VmHostState` gains no method.**

**Lifecycle Gate Ownership.** Adds no gate: the reservation is the existing Retiring lease and its release gate is unchanged. One state meaning
moves, recorded in ADR-0350: ADR-0156's "boot reclamation completes before the first `assign`" becomes "boot reclamation, and the reservation of
every VM it could not remove, complete before the first `assign`".

**Boundary scenarios.** (1) A leftover VM that settles within the bound: removed at boot, no reservation. (2) A stuck VM with a `Running` row: the
row becomes terminal and carries the address, the offset is reserved, boot continues, no launch receives its CID. (3) A stuck VM with a terminal
row carrying the address: reserved from that row. (4) A launch parked in guest boot at the crash: its `Pending` row carries the address; boot
closes it first (`LaunchLostWithControlPlane`, address carried), kills the VM, and reserves from the closed row. (5) The VM reaps later: `VmReclamation` removes the scope, the network-reclaim retry releases the reservation. (6) Several stuck
VMs: one bound of boot delay, all reserved. (7) A busy scope never makes boot fail; a non-busy substrate error still does. (8) Reservations at the
cap: a new launch is refused `AdmissionCapReached`. (9) A second crash in the stuck window: the next boot derives the reservation again from the
rows. (10) The `Pending` write failed: no VMM exists, nothing to reserve.

**Tests.**

- **Stuck leftover boots and does not reuse its CID.** Tier 1, seeded, production `converge` and a production launch over `SimVmm` and
  `SimVmHostState` with a held scope: `converge` returns `Ok`; the pool reports the offset held (Retiring, counted in occupancy); a sweep of
  launches across the whole offset range, including a next-fit wrap onto the held offset, never receives the stuck VMM's CID (the `SimVmm` vsock
  holder is the arbiter) and produces no `EADDRINUSE`. Tier 3: the same on a real guest with a held flush across a `serve` crash and restart.
- **Boot reserves from a launch parked in guest boot.** Tier 1, seeded, production `converge` over `SimVmm`, `SimVmHostState` and a store holding
  only the launch's `Pending` row: the scope is held and the VMM unreaped; `converge` returns `Ok`, the pool holds the offset named by the row, a
  launch sweep including a wrap never receives that CID, and the row is `Terminated` with `LaunchLostWithControlPlane` and carries the address. The
  store records no `Stopped { by: PlatformReclaimed }` write for it.
- **The one release site (F3).** Tier 1, seeded, production shim, `WorkloadLifecycle` and `converge`, the scope held: repeated
  `ReclaimAllocationNetwork` attempts for the reserved allocation each return `Ok` promptly and the reservation stays held; after the scripted
  reap, a `VmReclamation` pass and one more attempt, the offset is assignable. A reservation with no plan passes `teardown_guest_network` without
  calling the provisioner (the sim provisioner records no teardown), and the same function with an Admitted lease still tears down. The reclaim path
  over a restarted process (no driver entry) returns `Ok` for the reservation.
- Pool: `reserve_leftover` counts toward the cap; `assign` of a reserved allocation returns `LeaseRetiring`; the reservation survives a next-fit
  wrap; `snapshot()` omits it; `observe` reports it Retiring; `lease_observation()` reports it as `GuestLeaseView::Reserved` with the offset's CID, `state()`
Retiring and `launch()` `None`, and no `Launched` lease shares its CID.
- A non-busy executor error still refuses the boot with `ConvergeError::Reclamation`.
- Tier 3: S-VOL-T3-12's boot variant asserts the node boots with the held-flush VM left over.

### The guest address is on record before the VMM exists (N-2; ADR-0353)

**The contract (user ruling 2026-11-23, N-2: save the address when it is handed out).** The offset is `workload_addr - GUEST_NODE_PREFIX.network()`
(`100.95.0.0/16`, a compile-time constant; the inverse is `GuestAddressPool::release`, `guest_network.rs:651-652`) and the CID is
`GUEST_CID_BASE + offset`. Boot reads the address from `AllocStatusRow.workload_addr` (`observation_store.rs:984`) and nowhere else; the VMM's
command line is not read and `VmHostState` gains no method. That holds only if every VMM that can hold a CID has its address on a row.

*The row and the moment.* The row state is the existing `AllocState::Pending`; the field is the existing `workload_addr: Option<Ipv4Addr>`
(`AllocStatusRowV2`, GH #241), set to the `GuestTransportAssignment::workload_addr` of the lease the pool has just handed out. The write is **one
call to `write_alloc_lifecycle`** (the writer the Running row uses), made in the action shim immediately after `provision_and_inject_netns` returns
`Ok(plan)` and before `ensure_intercept_identity` and `driver.start`, so before `Vmm::create` is reachable:

- `StartAllocation`: between the `Ok(plan)` of the `match provision_and_inject_netns(..)` arm (`action_shim/mod.rs:1828`) and
  `ensure_intercept_identity` (`:1897`).
- `RestartAllocation`: in the successor's `Ok(plan)` arm (`:2342`), before `ensure_intercept_identity` (`:2343`). The row is the successor's.

`provision_and_inject_netns` is unchanged. A provision failure writes no `Pending` row (no VMM can exist) and keeps its `Failed` row. The row:
`state: Pending`, `started_at`, `reason`, `detail`, `terminal`, `stderr_tail` all `None`, `workload_addr: Some(addr)`, `kind` as the action's. A
Start whose prior row is already non-terminal (a stale duplicate dispatch: `Pending`, `Running`, `Draining`, `Suspended`) writes no `Pending` row:
that row is the record, and it carries the same address because `assign` replays the same lease for an id it holds. **The Restart arm applies the
same rule to its successor:** it reads the successor allocation's own row (the key `write_alloc_lifecycle` writes) before the `Pending` write, and
writes none when that row is non-terminal; the successor's `Pending` write is `dominating(.., None)` only when the successor has no non-terminal
row. So a duplicate dispatch of one restart (R9, the driver's `AllocationAlreadyOwned`) never writes over the owner dispatch's `Pending` or
`Running` row, and it writes no row of its own: it returns the successor outcome `Ok(())` to `finish_restart` and keeps the lease, which is the
owner's (the same id replays the same lease). No new terminal reason is needed. A duplicate-owner answer after a `Pending` write made by *this*
dispatch cannot occur: a supervision claim is taken only by a `start` whose dispatch wrote the `Pending` row first, rows are never deleted, and the
claim of a launch that ended before running is released before the dispatch returns, so a second dispatch of the id finds a non-terminal row
(the owner is still launching or running) or a claim-free id. DELIVER confirms the release-before-return premise on each launch-ending path; a
path that violates it is a defect to fix there, never a new reason. The R9 test arranges the owner with its `Pending` row present and asserts the
row, the lease and the predecessor cleanup; an owner claim with no row is not a state `serve` plus `deploy` produces. **If the `Pending` write
fails, no VMM is created:** the arm retires and tears down the lease (`retire_guest_network_lease`, `teardown_for_dispatch`) and returns the typed
`ShimError` (N-REC). Cost: one observation write per launch, on the launch path that already makes the Running write.

*Every later row carries the address.* The `Running` row sets it. Every other row the shim or the observers write for the allocation copies it
from the prior row:

| Producer | Today | Contract |
|---|---|---|
| the `Failed` row of a failed start (`action_shim/mod.rs:2027-2031`, `:2515-2519`) | `None` | carries the `Pending` row's address |
| terminal row of `FinalizeFailed` (`:1693`) | drops it (`if is_stable { prior } else { None }`) | forward-carries `prior_row.workload_addr` always |
| the stop's terminal row, and the reclamation executor's row (`action_shim/reclamation.rs:265`) | `None` | carry the prior row's address |
| exit-observer terminal row (`exit_observer.rs:562`) | carries it | unchanged |
| the `Failed` row that supersedes a `Running` row on the post-`Running` fail-closed paths (`fail_closed_on_mtls_install_with_guest`, `fail_closed_on_guest_network_activation`; S10) | `None`; `assert_superseding_failed_row` asserts it does not inherit the `Running` row's address | carries the `Running` row's address. The VMM reached `Running` and may hold its CID until the stop settles (a stop can end `KilledNotReaped`, ADR-0348), so boot must find the address on the allocation's newest row; a row that drops it would hide a stuck VM from reservation (ADR-0350). The assertion changes to equality with the `Running` row's address |
| the `Failed` row of a provision failure (`fail_closed_on_netns_provision`) | `None` | unchanged: no `Pending` row and no VMM exist, and the lease is retired; there is no address to carry |

*Persisted format.* None changes: `AllocStatusRow.workload_addr` and `AllocState::Pending` exist in the persisted `AllocStatusRowV2` payload, so no
rkyv envelope bump and no new fixture. The wire shape (`AllocStateWire::Pending`) already has the state.

*Readers.* A `Pending` row is a launch record, not a placement. Every reader that acts on a running allocation gates on `Running` or on a non-`Pending`
state already (ADR-0276's backend hydration is total over state; the exit observer, the scheduler's capacity count and the readiness and probe
paths read `Running` rows). The lifecycle reconciler's treatment of a `Pending` row and the LWW ordering of the write are checked by the tests
below; `LogicalTimestamp::dominating` over the prior row (Start) and `dominating(.., None)` (Restart successor) make the later `Running` or
`Failed` row dominate the `Pending` row, and both rows of one launch derive their crash facts from the same prior row, so `restart_count` and
`last_terminated` are identical and never counted twice. A `Pending` row never outlives its launch (§ "Launches that end before running").

**Tests.**

- **Every VM allocation has its address on record before `Vmm::create`.** Tier 1, seeded, production `StartAllocation` and `RestartAllocation`
  dispatch over `SimVmm` and the production observation store: when `Vmm::create` is entered the store holds the allocation's `Pending` row and its
  `workload_addr` equals the lease's address, for both arms and for a first placement, a crash restart and a generation replacement. When the
  `Pending` write is made to fail, `Vmm::create` is never entered, the lease is released and the typed error is returned. A stale duplicate dispatch
  against a non-terminal row writes no second `Pending` row. The `Pending` and `Running` rows of one launch carry the same `restart_count` and
  `last_terminated`, and the `Running` write is accepted by the store.
- **The terminal-row carry.** The stop's terminal row, the `FinalizeFailed` row, the `Failed` row of a failed start (Start and Restart arms), the
  `Failed` row of the post-`Running` fail-closed paths and the reclamation executor's row each carry the prior row's `workload_addr`; a Service allocation whose row is `Pending` or terminal and carries an
  address has no backend.
- **Readers.** A grep-level check in DELIVER of every reader of `workload_addr` outside the `Running` gates (the handlers at `handlers.rs:129-144`,
  the CLI render at `render.rs:606`) for a reading that assumes a non-`Running` row has none.

### A busy boot scope writes its terminal row (N-3; ADR-0354)

**The contract (user ruling 2026-11-23, N-3, option a).** When boot reclamation's `kill_scope` returns `ResourceBusy`, the kill is delivered and the
VMM can run no guest code (ADR-0348), so the boot executor step writes the allocation's terminal row as for a settled scope: the platform-reclaimed
row `execute_reclaim_allocation` writes (`Terminated`, `Stopped { by: PlatformReclaimed }`), carrying the prior row's `workload_addr`. This is the
case of a `Running` row; a `Pending` row is already `Terminated` with `LaunchLostWithControlPlane` (step 1 of § "Boot proceeds past a stuck VM"),
so the executor meets a terminal row and writes none, busy or not. It does not
discard artifacts (the scope, the run directory); `VmReclamation` emits `DiscardStrandedArtifacts` for a terminal VM allocation with host artifacts
and retries each pass. The row makes the workload eligible for restart at once, its replacement waiting on the volume claim (ADR-0317).

This is the one caller exempt from "busy means no row write". The steady-state `VmReclamation` tick that meets the same busy scope writes no row,
and its row is already terminal when the scope is stuck after a stop. How the two callers of `execute_reclaim_allocation` differ is internal
structure; the contract is the observable outcome: after `converge` returns on a busy scope the row is terminal and carries the address; after a
tick meets the same busy scope the row is unchanged.

**Tests.** A busy boot executor step writes the terminal row and does not discard artifacts; the next `VmReclamation` pass discards them after the
hold clears. The steady-state tick that meets the same busy scope leaves the row unchanged.

### Launches that end before running (N-4; ADR-0355, which amends ADR-0334)

**The ruling (user, 2026-11-23, N-4, option b).** A launch writes its `Pending` row before `Vmm::create` (§ "The guest address is on record before
the VMM exists"), so a launch that ends before it runs (superseded by a stop, a restart or node shutdown; lost with the control plane) ends that row
with a terminal row that names why. `WorkloadLifecycle`'s current-allocation logic is unchanged. Not chosen: skipping `Pending` rows in
`current_alloc` and `generation_replacement_pending`, and relaunching a `Pending` row after a timeout; both are in ADR-0355.

**The ruling (user, 2026-11-24, RB-13): a launch that never ran does not count.** The abandoned rows stay in history and `current_alloc` still
returns the newest row; the derivation of a new launch's count, of whether a generation is pending and of a natural exit skips them (§ 2).

**1. The state and the reasons.**

- State `Terminated`; `terminal: None`; `started_at: None`; `workload_addr`, `kind` and `listeners` carried from the `Pending` row; `detail` and
  `stderr_tail` `None`. Never `Failed`: an abandoned launch is not a start failure (ADR-0337). `terminal` is `None` for the reason the
  platform-reclaimed row has none: the workload's intent still stands, and the `allocation_attempt_transition` fence and the finished-Job rule of
  ADR-0335 key on `terminal.is_some()`.
- Two fieldless `TransitionReason` variants, appended after `VmVolumeUnavailable`: `LaunchSuperseded` ("launch abandoned: superseded by a stop, a
  restart or node shutdown") and `LaunchLostWithControlPlane` ("launch abandoned: lost with the control plane"). Both `is_failure() == false`;
  `human_readable` carries the wordings. `From<&DriverStartFailure>` is untouched.
- **One predicate, on the row's reason.** `pub fn is_launch_abandoned(row: &AllocStatusRow) -> bool` in `overdrive_core::transition_reason`
  (beside `is_platform_reclaimed`, `#[must_use]`): true iff `row.state == Terminated` and `row.reason` is `LaunchSuperseded` or
  `LaunchLostWithControlPlane`. It is the one place the two reasons are named as a set; no other site enumerates them (`rust.md` § "A key prefix and
  its record classifier are ONE SSOT"). `is_platform_reclaimed` does **not** read the two reasons: a platform reclamation ran, and it stays a count
  basis (§ 2). Its callers (`workload_lifecycle.rs:411`, `:468`, `:1173`, `:1789`, `:1821`; the `reclamation.rs:554` test) are unchanged in meaning.

**2. Count: a launch that never ran does not count (user ruling 2026-11-24, RB-13).** The restart count of a new launch is derived from the
**count basis**, the newest row that ran: `last_ran_alloc(rows)` (§ "Restart generation is handled when its successor is observed"), which is
`current_alloc` over the rows for which `is_launch_abandoned` is false. The new launch's count is:

- a **crash restart** (no generation pending): the basis row's `restart_counts` entry plus one;
- an **operator restart** (a generation replacement): the basis row's entry unchanged, so a `Failed` basis row is forgiven (ADR-0335);
- a **fresh placement** (no row ran, including a first launch that was abandoned): 0.

A platform-reclaimed basis row keeps its existing treatment: no ceiling check and no `+ 1` (ADR-0083). The never-ran rows stay in history; only the
derivation skips them. **Invariant:** for any chain of rows, deleting the abandoned rows changes no later launch's count, `charged` or `exhausted`.
It holds because every read of the derivation below takes the basis row and never the abandoned row, whatever its View entries hold:

- `attempts` (`view.restart_counts[basis]`, 0 with no basis), `last_failure_seen_at[basis]` and its carry to the successor, the backoff gate, the
  ceiling decision (`restart_decision(attempts, is_platform_reclaimed(basis), generation_replacement)`) and the finalisation of an exhausted
  budget (`FinalizeFailed` names the basis row) at `workload_lifecycle.rs:410`, `:467`, `:1156`, `:1173`, `:1315-1325`;
- whether a generation is pending: `generation_replacement_pending(view, desired.generation, last_ran_alloc(rows))`, at `:366`, `:451`, `:925` and
  in `restart_status_for_alloc`, which receives the basis from its caller (§ "Restart generation is handled when its successor is observed", `ran`);
- the natural-exit test: `is_natural_exit` excludes an abandoned row (`&& !is_launch_abandoned(row)`), so a Job whose launch never ran is not
  finalised `Failed { exit_code: Some(0) }`.

`current_alloc` is **unchanged**: it returns the newest row, abandoned or not. The abandoned row is the workload's current allocation for
`describe`, for the restartable filter (`is_restartable`: `Terminated`, not an intentional stop), for the id minted above it and for the
predecessor a `RestartAllocation` names, so the existing restart path relaunches it with no new branch: after a shutdown or a lost launch as a
crash restart of the basis row (count + 1, the charge its crash already carries, once), after a restart commit as a generation replacement
(the basis row's count). After `stop` the stop intent is set and nothing is relaunched.

*Why a crash is forgiven.* The operator's restart of a launch that was itself a crash restart: the crash row #1 (count 0) is restarted as #2
(count 1), the commit abandons #2, and the operator's #3 takes the count of #1, 0. With the abandoned row's own count it would start at 1 and the
crash would never be forgiven. A lost launch with no operator action relaunches as #3 with #1's count + 1 = 1, the same count #2 held.

**3. Persisted format.** `TransitionReason` is archived inside `AllocStatusRowV1`. The two variants are fieldless and appended at the tail, so every
discriminant is unchanged and the archived size is unchanged (neither exceeds `VmKernelFormatUnsupported`). No envelope version changes:
`AllocStatusRowEnvelope` keeps `V1`, `known_discriminants` stays `[0]`, `discriminant_offset_from_end` stays `416`, `FIXTURE_V1` is untouched (the
rule of § "Interface contracts", Persisted formats, 2; the precedent of the `StoppedBy` tail variants and `VmVolumeUnavailable`). Two golden
fixtures are added in `crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs`, one per variant. If the size check at DELIVER finds the
archived size changed, paragraph (2) of Persisted formats applies. `api/openapi.yaml` is regenerated.

**4. The write sites.** One writer in every case: `write_alloc_lifecycle` with `LogicalTimestamp::dominating(.., Some(prior))`,
`TransitionSource::Reconciler`, the row of § 1; each emits one lifecycle transition `Pending -> Terminated` with the reason.

| # | Site | Where | Reason |
|---|---|---|---|
| W1 | `StartAllocation`, `driver.start` returns `Err(StartSuperseded)` | `action_shim/mod.rs:1955-1960`, after `release_superseded_launch`, `retire_guest_network_lease` and `teardown_for_dispatch` | `LaunchSuperseded` |
| W2 | `RestartAllocation`, the successor's `StartSuperseded` | `action_shim/mod.rs:2470-2489`, before `finish_restart`; the row is the successor's | `LaunchSuperseded` |
| W3 | `Err(other)` from `driver.start` (S8, R10) | the existing `Err(other)` arms | a `Failed` row, `DriverInternalError { detail }` (the S6 / R8 row and charge); the `Pending` row they would leave is ended |
| W4 | boot, every `Pending` row of this node | the first step of `vm_reclamation_boot::converge`, before the observation and the executors (below) | `LaunchLostWithControlPlane` |

W1 and W2 rework the committed step 05-01 (the driver's observation points, the lane table and the producers of 05-02 are unchanged): the shim's
"no-row `StartSuperseded` arm" writes the abandoned row after the existing releases; a teardown failure does not suppress the row and is returned
after it; a failure of the row write itself is the typed `ShimError` (N-REC: the `Pending` row stays until the next boot closes it). The
`emit_launch_superseded` message ("no allocation row written") is rewritten, and the shim tests and acceptance tests that assert `alloc_status_rows()`
empty after a supersession assert the row instead. W1 and W2 land in the same DELIVER step as, or after, the `Pending` write. A launch whose
`Pending` write failed has no `Pending` row and creates no VMM. A launch whose VMM is running when its `Running` write fails (S9, R11) is not a
launch that ended before running; it keeps the existing N-REC exposure, its `Pending` row stays until boot, and boot closes it with W4 (the
VM's scope, if any, is then discarded by the executors).

*Boot (W4; user ruling 2026-11-23, RB-11: one reason for a launch lost with the control plane).* `converge` keeps its signature and the planner is
unchanged (it walks host surfaces). **The first step of `converge` is the one site that closes `Pending` rows at boot, and it runs before the
observation and the reclamation executors:** it reads `ObservationStore::alloc_status_rows()` and, for every row whose `state` is `Pending` and
whose `node_id` is this node, writes the W4 row, whether or not the row's VM left a scope, a run directory or a clone. Every `Pending` row a crash
left therefore ends `Terminated` with `LaunchLostWithControlPlane`; the executors then see a terminal row (`DiscardStrandedArtifacts` for a VM
with artifacts, which authors no row) and cannot rewrite the reason. The executor has no `Pending`-specific branch. A boot process owns no launch
from before the restart and boot reclamation completes before the convergence loop dispatches anything, so every `Pending` row at that point
belongs to a dead launch. The step is idempotent (the written row is terminal), makes no reservation and kills nothing (the reservation of a stuck
VM is step 4 of § "Boot proceeds past a stuck VM", from the address the closed row carries) and submits no evaluation: the boot resync pass and
the interest router's List reach the closed rows. A read or write failure refuses the boot as any other `ConvergeError::Reclamation`.

**5. Readers.**

| Reader | Effect of the abandoned row |
|---|---|
| ADR-0276 backend hydration, the exit observer, the scheduler (`Running` rows only) | none: `Terminated` is `NotRunning`, no `ExitEvent` exists, no capacity is counted |
| probes, readiness, `ServiceLifecycle`, `SvidLifecycle` | the allocation never reached `Running`; their `AllocStatus` interest wakes evaluate it and emit nothing; the SVID was dropped by the shim |
| `WorkloadLifecycle` | `current_alloc` returns the abandoned row; § 2 is the whole change (the count basis is the newest row that ran); `next_allocation_attempt` mints above it, so the id is no longer a gap |
| LWW counter, ADR-0077 and ADR-0078 | `Pending -> Terminated` is neither a terminal-to-`Running` transition nor a terminal row superseded by a non-terminal one: `restart_count` and `last_terminated` stay the `Pending` row's and are never counted twice; the relaunch takes a new id |
| ADR-0335 handled-generation rule | a generation is handled when its issued id has a row that ran; an abandoned row handles nothing, so a generation whose successor was abandoned is still pending and its relaunch is a generation replacement with the count basis's count |
| `describe`, the restart-budget answer of the handlers | `describe` shows the newest row, abandoned or not; `restart_status_for_alloc` reads attempts and pending through the count basis its caller passes as `ran` |
| `describe` and the API | state `Terminated` with the reason through `TransitionReason::human_readable`; the OpenAPI enum gains two reasons |

**6. Tests.** Tier 1 seeded unless stated, over the production shim and observation store.

- A superseded Start and a superseded Restart each end with one `Terminated` row carrying `LaunchSuperseded`, `terminal: None`, the lease's address, no
  `Running` and no `Failed` row, the `Pending` row's `restart_count` and `last_terminated`; a failing teardown still writes the row and returns the
  error; a failing row write returns the typed error.
- After a restart commit, a shutdown and a boot, the workload is relaunched once (Service and Job; the Job is not finalised); after a stop nothing
  is relaunched; the budget is never advanced by an abandoned launch (the carry assertion over seeds and schedules).
- **Count tests (RB-13).** Tier 1, pure `reconcile` over generated rows and a hand-built View, beside the existing
  `workload_lifecycle_restart.rs` acceptance tests:
  1. *A crash restart abandoned by an operator restart forgives the crash.* Row #1 crashed (`Failed`, count 0); #2 is its crash restart (count 1);
     #2 ends `LaunchSuperseded`; the generation committed after #2's issue is pending. The relaunch #3 carries count 0 (the basis is #1), on the
     generation-replacement path. The same chain with #2 `LaunchLostWithControlPlane` gives the same count.
  2. *A lost launch with no operator action charges the crash once.* Same chain, no generation pending: the relaunch carries #1's count + 1 = 1, and
     the same count after two consecutive abandoned rows (#2 and #3 both `LaunchLostWithControlPlane` give #4 the count 1, not 2).
  3. *A launch abandoned before anything ran* (no basis): the relaunch carries 0; an operator restart of a `Running` workload stops it (R2) and its successor is abandoned
     by shutdown after the generation was handed that successor: the relaunch is still a generation replacement carrying the stopped row's
     count, not `+ 1`.
- **Deletion-equivalence (proptest).** Over generated chains of rows (`Failed`, `Terminated` by crash, platform-reclaimed, operator-stopped,
  `Running`, and runs of consecutive abandoned rows of both reasons at arbitrary positions), generated desired generations and a View whose
  `restart_counts`, `last_failure_seen_at` and `issued_generation` entries for abandoned ids are arbitrary: the `reconcile` output over the chain
  and over the chain with its abandoned rows deleted agree on whether a restart or `FinalizeFailed` is emitted, on the successor's `restart_counts`
  entry and on `last_failure_seen_at` of the successor, and `restart_after_failure` / `generation_replacement_pending` agree likewise. The
  generator draws ids from one counter so the deleted chain keeps its order; the property fails if any read takes the abandoned row.
- **Every crash-left `Pending` row ends with `LaunchLostWithControlPlane`** (RB-11). Tier 1, seeded, production `converge` over `SimVmm`,
  `SimVmHostState` and the production observation store, one case per kind of leftover: a `Pending` row with no host artifact; one whose VM left a
  settled scope, a run directory and a clone; one whose VM left a busy scope (an unreaped `SimVmm` process). In each, the row ends `Terminated`,
  reason `LaunchLostWithControlPlane`, `terminal: None`, the `Pending` row's `workload_addr` carried, and the store records no
  `Stopped { by: PlatformReclaimed }` write for it. The executors run after the closing step: the sim records a `DiscardStrandedArtifacts` and no
  `ReclaimAllocation` for these rows, the row's reason is unchanged after `converge` returns, and a busy scope's offset is reserved from the closed
  row before the first `assign` (a launch sweep across the offset range, including a wrap, never receives that CID). A `Running` row is still
  ended by the executor as platform-reclaimed, and a busy `Running` scope writes its row (ADR-0354). A second boot writes nothing. A seeded
  crash between the closing step and the executors leaves terminal rows the next boot's executors discard, with the same reservation. The
  two-boot cell `mid_boot_volume_reclamation` asserts the row, not "no row".
- S8 and R10 write a `Failed` `DriverInternalError` row.
- `is_launch_abandoned` unit tests for the two reasons (and `false` for every other reason and for a `Failed` row carrying
  neither); `last_ran_alloc` unit tests (skips abandoned rows, `None` when none ran); the two schema-evolution fixtures; the `render::alloc_status` and
  `human_readable` goldens for both reasons; `api/openapi.yaml` regenerated.

### Quint changes for the guest-flow-owner and restart models

The modeler edits the specs; none is edited here. `specs/quint/guest-flow-owner/cid_lease.qnt` and `checks.toml`:

1. **Scope state (ADR-0349, ADR-0351; decision 16).** A per-offset `scope` flag (present or removed) and an action `RemoveScope(o)` enabled only
   when the offset's device is free (the guard is `populated 0`). `Reap(o)` is the task's exit and the model's `populated 0`; it does not remove the
   scope. `LateRelease(o)` and the release at an ordinary stop read the scope's absence, not the device. K-C7 is restated: *a workload scope is
   removed only after `cgroup.events` reports `populated 0` for it; the scope's absence therefore implies the VMM's device is released. The
   running kernel removes a scope under a live stuck task when `rmdir` is issued without that wait (spike `findings-cgroup-events.md`); the model
   never does.* K-C8 is added to the README: *a VMM holds its device only while its process is a member of its scope (ADR-0352; Tier 3 test of §
   "A VMM is born inside its scope").* Hazard `removeScopeWhilePopulated` (one for decisions 16 and N-1): `RemoveScope` loses its guard; the
   checker must find `NoLaunchClash` or `LeaseOutlivesVmm` violated. Named run `ScopeIsRemovedOnlyAfterItIsEmpty`: stop unreaped, the lease is held
   and a redeploy waits, attempt removal (disabled), reap, remove, release, a wrap-around assign succeeds.
2. **Boot reservation is an entry in `late` (ADR-0350).** `Reclaim` is no longer enabled only when no orphan is unreaped. It is enabled whenever
   boot runs, kills every orphan and, for each orphan still unreaped (its device held), puts its offset in the existing `late` map. `Assign` already
   skips `late` offsets; `LateRelease` already frees them under the scope guard; `CidUnique` and `NoUntrackedLease` already range over `late`. No
   `reserved` set, no `ReleaseReservation` action, no new invariant range.
3. **The record precedes the VMM (ADR-0353).** A durable `recorded` map (offset to allocation): `Assign` sets it, `Launch(a)` is enabled only when
   `a`'s offset is recorded, `Crash` keeps it and drops the pool, and `Reclaim` fills `late` from `recorded` for unreaped orphans. Hazard
   `launchBeforeRecord`: `Launch` no longer requires the record, so a crash during guest boot leaves an orphan `Reclaim` cannot reserve; the checker
   must find `NoLaunchClash` violated. Named run `ServeCrashDuringGuestBootReservesFromTheRecordedAddress`: assign, record, launch parked, crash,
   boot reserves from the record, a wrap-around assign skips it, the orphan exits, the reservation is released. Projection for the conformance
   driver: `recorded` through `ObservationStore::alloc_status_rows()` (the `workload_addr` of the allocation's row); the driving port for
   `Assign`/`Launch` is the production dispatch of `StartAllocation`/`RestartAllocation`; `late` and the lease from `GuestAddressPool::observe` and
   `snapshot`; `dev` from `SimVmm::vsock_device_holders` (the marked arbiter read).
4. **Checks to drop (one hazard per decision, runs only for distinct critical scenarios).**
   - Drop run `StopOfAnUnreapedVmmHoldsItsLeaseUntilTheReap` and its `checks.toml` entry
     `cid-lease-run-conf-ok-stop-of-an-unreaped-vmm-holds-its-lease-until-the-reap`: `ScopeIsRemovedOnlyAfterItIsEmpty` takes its steps (stop
     unreaped, lease held, redeploy waits, reap, release) and adds the removal between the reap and the release.
   - Keep run `NextFitSkipsAnUnreapedLease` (both `checks.toml` entries): the skip of a held offset before any release is a scenario no other run
     reaches.
   - Drop run `ServeCrashWithAnUnreapedVmmBlocksAssignUntilItExits` and its `checks.toml` entry
     `cid-lease-run-conf-ok-serve-crash-with-an-unreaped-vmm-blocks-assign-until-it-exits`: its premise (boot waits for the orphan) is removed by
     decision 14, and `ServeCrashDuringGuestBootReservesFromTheRecordedAddress` covers the crash, the reservation, the skip and the release. Do not
     add `ServeCrashWithAnUnreapedVmmReservesItsCidAndBoots` or a `bootWithoutReservation` hazard: with `Reclaim` filling `late`, that hazard's
     counterexample is the one `assignBeforeReclaim` already finds (an unreaped orphan's offset is assignable). (User ruling 2026-11-23, RB-12:
     `launchBeforeRecord` is kept and `bootWithoutReservation` is dropped.)
   - Keep hazards `releaseUnreapedAtStop` (decision 12), `removeScopeWhilePopulated` (decisions 16 and N-1), `launchBeforeRecord` (N-2) and
     `assignBeforeReclaim` (decision 14).
5. **No change for ADR-0352 or the boot ordering (RB-9, RB-11).** The models already say a VMM holds its device only while it is a member of its
   scope (K-C8), which the `clone3` placement makes true from the first instruction and which no model action distinguishes from the write form.
   The boot ordering changes no model: `cid_lease`'s `Reclaim` reads `recorded`, which the closed row still carries (the projection is still
   `alloc_status_rows()`' `workload_addr`), and `node-local-volume`'s `Boot` already ends every `PENDING` allocation as `TERMINATED`/lost with no
   distinction for a launch that left a scope. No hazard, run or check is added for either.
6. **Validation budget (decision 15, user ruling 2026-11-23).** `lease_five` runs a CFG without `StopUnreaped`; it did not finish. The unreaped stop
   stays exhaustively checked on three offsets (`lease_ok` and its siblings), which reaches the pool-wrap case, and is covered on five offsets by
   the named runs and the randomized traces (`specs/quint/guest-flow-owner/`, commit `7f32d5b83`). The new hazards are added on three offsets only.
   Any further growth returns to the user.

`restart-generation` (ADR-0355): the model's `W_LOST` (a launch lost or superseded, "no row") becomes a row: the allocation ends `ABANDONED`, a
terminal, restartable class that is not a natural exit and that no derivation reads (RB-13). The shared rule `restart_budget.qnt` takes the count
basis: its `cur` argument is the newest allocation that is not `ABANDONED` (0 when none), and `pendingGen` is the staleness `old` of that row, not
of the newest row; `budgetCharged`, `budgetExhausted`, `budgetStartCnt` and `budgetMayRestart` keep their bodies, so the importing specs
(`restart-generation` and `node-local-volume`) change only the argument they pass, through one shared definition of "the newest non-abandoned
allocation" in `restart_budget.qnt` (the module stays definitions-only). The `ABANDONED` allocation's own `cnt` is carried by no one. The rule's
header comment states the three cases of § "Launches that end before running", item 2. **Hazard `abandonedRowCounts`** (the one hazard of RB-13):
the count basis is the newest allocation, abandoned or not; the deletion-equivalence invariant over chains of several consecutive `ABANDONED`
allocations (the count, `charged` and `exhausted` equal those of the chain with the `ABANDONED` allocations removed) must be violated. The
invariant is `I_ABANDONED_ROWS_DO_NOT_COUNT`, in the `restart-generation` spec only (RA9 is its assumption; the volume spec imports the rule and
adds no invariant or hazard, so `vol-safety-tlc` grows by the changed argument only, a cost the modeler reports); `exhausted` is read from the
count basis. The
`node-local-volume` run `superseded_launch_result_discarded` regains its expectation that the relaunch charges 0 (`charged(st) == 0` after
`WlStart(3)`: the crash of allocation 1 is forgiven by the operator's restart, the abandoned allocation 2 not counted). No other run, check or
witness is added: the two count scenarios of § 6 are Tier-1 tests, and the existing runs
`control_plane_crash_during_restart_launch_relaunches_once` and `second_restart_loses_launch_then_one_placement` pass through the new rule. `PENDING`
is the same state, now projected as a
`Pending` row. `CpCrash` and `NodeReboot` take every `PENDING` allocation to `ABANDONED` at the boot step. RA2's premise ("a launch in flight is
invisible ... `canStart` requires no `PENDING` allocation") is unchanged; RA4 reads "delivers nothing: an uncharged abandoned row, no VMM". One hazard,
`abandonWritesNoRow` (an abandoned launch leaves its row `PENDING`): the checker must find `RunIntentProgress` violated. No new named run: the
existing runs `control_plane_crash_during_restart_launch_relaunches_once` and `second_restart_loses_launch_then_one_placement` pass through the new
state. `node-local-volume`: `alloc` gains no state; its TERMINATED projection carries the reason (`superseded`, `lost`, other),
`I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING` reads "no `Running`, no `Failed`, no budget input, no VMM" instead of "no row", and `Boot` ends every `PENDING`
allocation as `TERMINATED`/lost. `cid_lease` needs nothing further for ADR-0355. The rows for a superseded or lost launch (`why[a] == W_SUPERSEDED`,
`W_LOST`, `as[a] == PENDING`, `hasRow`) are restated in § "Observable projections".

### Review disposition (2026-11-23)

The two independent design reviews of the stuck-VM design are resolved as follows; the decisions themselves are unchanged.

| Finding | Resolution |
|---|---|
| F1 the VMM is born outside its scope | § "A VMM is born inside its scope"; ADR-0352 (accepted, `clone3` with `CLONE_INTO_CGROUP`) |
| F2 `remove_workload_scope` has no caller | deleted with its tests; structural single-remover test; § "A VM scope has one remover"; ADR-0351 |
| F3 one release site of a boot reservation | `teardown_guest_network`; a reservation has no plan; § "Boot proceeds past a stuck VM" and § "The guest-address lease outlives an unreaped VMM" |
| F4 uncited kernel claims | the K1 to K5 table in § "The guest-address lease outlives an unreaped VMM" |
| F6 start-failure cleanup after `KilledNotReaped` | the scope step is omitted in every cleanup; § "A VMM kill's reap wait is bounded", item 7 |
| F5, A1, A2 one decision per ADR | ADR-0348 to ADR-0355, one decision each |
| A3, B, E restated prose and history | contracts moved here; ADR text states the current decision only |
| C1 one site closes `Pending` rows at boot | the first step of boot, before the executors; the executor has no `Pending` branch |
| C2 one predicate | one structural predicate on the two reasons, `is_launch_abandoned`; no site enumerates them (RB-13) |
| C3, C4 boot reserves only busy scopes, as a `late` entry | § "Boot proceeds past a stuck VM"; § "Quint changes" item 2 |
| C5 one hazard per decision | § "Quint changes" item 4 |
| D1, D2, D3 | no `ReservationRefused` (`unreachable!`); no `vm.reclamation.scope_busy`, `vm.reclamation.boot_stuck` or `vm.lifecycle.stop_not_reaped` event (an unreaped stop is visible as a `Terminated` allocation whose scope is still observed); the `terminate` edge case is "an already-gone pid is `Killed`" |
| F no path pays both bounds | § "A VMM kill's reap wait is bounded", item 10 |

### Restart generation is handled when its successor is observed (SD-6; ADR-0335, which amends ADR-0073, with ADR-0337 and ADR-0338)

**Decision.** Whether a generation has been handled is derived from observation: a generation is handled once the allocation issued for it **has
an allocation row that ran** (an abandoned row, ADR-0355, ran nothing and handles nothing; RB-13). The View records only what was issued (an input), never that a launch happened. Today `WorkloadLifecycle` stamps
`observed_generation` on the tick that emits a replacement (`workload_lifecycle.rs:1309`, `:1437-1439`) and the runtime fsyncs the View before it
dispatches (`reconciler_runtime.rs:1557-1561`); a launch that ends with no row (superseded, or lost with the control plane) then leaves the
workload down beside the old operator-stopped or replaced row (veto `:1069`, restartable filter `:1146-1148`).

**Pinned contract (overdrive-reconcilers and overdrive-control-plane).**

- `WorkloadLifecycleView.observed_generation` is **deleted**. `WorkloadLifecycleView` gains `#[serde(default)] pub issued_generation:
  BTreeMap<AllocationId, u64>`: the desired generation each issued allocation id serves, written at the two issuing sites (the restart arm
  `:1305-1335` and the placement arm `:1370-1440`) in the same `next_view` as the id's `restart_counts` entry and with the same lifetime. The
  value is `desired.generation` at issue in every case. It is never consulted to decide whether a launch happened. CBOR additive field per
  `development.md` § "Reconciler I/O" -> "Schema evolution"; no rkyv envelope: `AllocStatusRow` carries no generation and gains none.
- `pub fn last_ran_alloc<'a>(allocs: &[&'a AllocStatusRow]) -> Option<&'a AllocStatusRow>` in `overdrive-reconcilers`, beside `current_alloc`
  (RB-13): `current_alloc` over the rows for which `overdrive_core::transition_reason::is_launch_abandoned` is false. It is the **count basis**,
  the one definition of "the newest row that ran". It names no reason itself; `current_alloc` is unchanged.
- `pub fn generation_replacement_pending(view: &WorkloadLifecycleView, desired_generation: u64, ran: Option<&AllocStatusRow>) -> bool` in
  `overdrive-reconcilers`: true iff `ran` is `Some(row)`, **`row` is not a finished Job row (`row.kind == WorkloadKind::Job &&
  row.terminal.is_some()`, H1)**, and `view.issued_generation.get(&row.alloc_id)` (absent reads `0`) is less than `desired_generation`. The
  finished-Job test is the action shim's own (`allocation_attempt_transition` returns `NoChange` for a Job row with a terminal;
  `workload_lifecycle.rs:131-139`). Every caller passes `last_ran_alloc(..)` over the **observed** rows (the row whose id has the numerically
  highest `-<N>` suffix among rows whose suffix parses and that are not abandoned, `workload_lifecycle.rs:1506`), so an issued id with no row, and
  an issued id whose row is abandoned, are invisible to it. With no such row it is false. It is the only definition of `restart_pending`: it
  replaces the three expressions in `workload_lifecycle.rs` (`:366`, `:451`, `:925`) and the one in
  `ReconcilerRuntime::restart_status_for_alloc` (`reconciler_runtime.rs:583`), which takes the count basis as a parameter (below).
- `pub fn restart_after_failure(kind: WorkloadKind, run_desired: bool, failed: &AllocStatusRow, ran: Option<&AllocStatusRow>, attempts: u32,
  generation_replacement: bool) -> bool` gains `ran`, the count basis, and nothing else: `failed` is still the current row, named as the
  `RestartAllocation` predecessor and tested for `Draining`, restartability and a Job's natural exit; `attempts` is
  `view.restart_counts[ran]` (0 with none); the ceiling decision reads `ran.is_some_and(is_platform_reclaimed)`. When the current row is not
  abandoned, callers pass `Some(failed)` and the function is as before.
- `ReconcilerRuntime::restart_status_for_alloc(&self, target: &TargetResource, kind: WorkloadKind, run_desired: bool, failed: &AllocStatusRow,
  ran: Option<&AllocStatusRow>, desired_generation: u64) -> (u32, bool)` gains `ran`, the count basis, and nothing else (RB-14). The runtime
  holds no observation store, so it cannot read the basis; the caller, which holds the workload's observed rows, passes it, and the method reads
  only the `WorkloadLifecycle` view it already holds. `failed` is the workload's current row (`current_alloc`), as before. With `attempts =
  view.restart_counts[ran.alloc_id]` (0 with `ran` `None`): `.0 = attempts.saturating_add(1)`; `.1 = restart_after_failure(kind, run_desired,
  failed, ran, attempts, generation_replacement_pending(&view, desired_generation, ran))`. **Callers pass `ran = last_ran_alloc(rows)` over the
  workload's observed rows.** A caller that holds only a just-written `Failed` row (the Job stream, `failed_row_from_event`) passes
  `Some(failed)`: a `Failed` row is never abandoned, so it is its own basis and the answer is as before. The answer differs from a
  `failed`-only decision only when `failed` is abandoned, which only a `Terminated` current row can be: for a Job, `is_natural_exit` excludes
  it, so it is a restart candidate rather than a finished run; and the abandoned row's own `restart_counts` and `issued_generation` entries
  (written at its issue, a crash restart's at the basis count + 1, a generation replacement's at the desired generation) would move the ceiling
  boundary one early and hide a pending generation that the basis still shows. The model-driven callers that pass the current row
  (`quint_restart_generation::finalize_observed`, `quint_node_volume`, `quint_cid_lease::decision`) pass `last_ran_alloc` over the rows they
  already read. No function reads a store: `reconcile` and `restart_after_failure` stay pure, and the hydration of the rows stays with the caller.
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
the target has returned, and exactly one of three things is true: it wrote a row that ran (`last_ran_alloc` is that id, its entry equals the
desired generation, nothing is pending); it wrote an abandoned row (ADR-0355: the count basis is still the older row, the generation is still
pending if it was, and **this** evaluation issues one new id and launches once); or it wrote none and its outcome is one of the named no-row
outcomes of § Launch outcomes, the current allocation is still the older row, the generation is still pending, and **this** evaluation issues one
new id and launches once.

**Relaunch.** The next evaluation places one fresh allocation for the committed generation, adding no charge of its own. This is the one statement
of the rule; every other section, scenario and gate points here. It holds after a superseded launch, a launch lost with the control plane and a
node shutdown, for a first placement, a restart of a `Running` workload and a resume of a stopped one alike (a crash-restart successor is
charged once, § Budget below).

**Budget.** A launch that never ran does not count (§ "Launches that end before running", item 2). The abandoned row is the current row of the
relaunch (`Terminated`, `LaunchSuperseded` or `LaunchLostWithControlPlane`) and is the predecessor the `RestartAllocation` names, but the
relaunch derives its count from the count basis, `last_ran_alloc`: the basis row's `restart_counts` value plus one for a crash restart; unchanged
for a generation replacement, which forgives a `Failed` basis row (`workload_lifecycle.rs:1313-1322` reads the basis row, not the current row); 0
with no basis. `last_failure_seen_at`, the backoff gate, the ceiling decision and its finalisation are the basis row's too. An abandoned
crash-restart successor is thereby charged once and **not twice**, and a crash restart abandoned by an operator restart is forgiven. The abandoned
id's `restart_counts` and `issued_generation` entries are written at its issue and stay with it, never read again.

**Growth of the View.** `restart_counts`, `last_failure_seen_at` and `issued_generation` entries are never pruned (true on `main` for the first
two). An id is issued only when the observed state changed since the last issue: a launch that writes a row (bounded by the restart ceiling per
operator commit), a supersession (one per operator commit or shutdown), an admission refusal (one per lost capacity race, N-ADM), or a
stale-dispatch fence (one per observed state change). A finished Job issues none (H1). No outcome issues an id on every tick while the observed
state is unchanged.

**Operator-stop veto and the other states.** The veto (`:1069`) is `!restart_pending && current is operator-stopped`; after stop -> restart ->
abandoned launch the current row is the abandoned one (not operator-stopped, a restartable row), the count basis is the old operator-stopped row
and the generation is pending against it, so the relaunch happens as a generation replacement. Bug 3 stays: a same-spec `deploy` bumps nothing. R1-crash stays. The Job terminal fence (`:938-951`), `next_evaluation_at` and
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
| S8 | Start `:1948` | `Err(other)` from `driver.start`: `Err`, no row | **`Failed` row** `DriverInternalError` (N-4, W3; the `Pending` row is ended). Reached only by a `SimDriver` script (DE-1): `VmDriver::start` returns only `StartRejected` and `GuestTransportMissing`, a composition error placement never issues (`driver.rs:272-274`); only a `SimDriver` script produces another. The arm stays an internal typed error. The captured `rejected_network_cleanup` is returned after the row on the `StartRejected` path (`:2189-2191`), the one reachable path |
| S9 | Start `:2043-2097` | the row write fails: `Err` | **N-REC** |
| S10 | Start `:2123-2168` | mTLS install or guest-network activation fails after `Running`: `Failed` row superseding it. Activation `Ok(false)` (`:2151-2152`): the Running row exists, hooks are skipped | a row exists; the fail-stop exit is **N-FAILSTOP** |
| S11 | Start (SD-3) | `StartSuperseded` | **abandoned row** (N-4): `Terminated`, `LaunchSuperseded`, never counted, relaunched from the count basis (the newest row that ran) |
| R0 | Restart `:2200` | `find_prior_alloc_row(..).await?` fails: `Err`, no row, no effect taken yet | **N-REC (read side)** |
| R1 | Restart `:2200-2202` | predecessor row absent: `HandleMissing`, no row | **N-UNR**: the arm is emitted only for the current allocation of the observed rows, under the lane, and no code deletes an allocation row |
| R2 | Restart `:2203-2210` | predecessor `NoChange` (a Job row with a terminal), or not `Failed`/`Terminated`: `Ok(())`, no successor | **N-STALE**: not reached through the reconciler for a finished Job (H1: a reconciler-side no-emit); the fence fires only when the row changed between hydration and dispatch |
| R3 | Restart `:2216-2218` | empty driver set: `HandleMissing`, no row (a node that booted without the VM driver while a VM workload's intent persists) | the guard is removed; the arm follows the missing-driver path (R8): `Failed` successor row; charged for a Service, final for a Job |
| R4 | Restart `:2229-2239` | admission refusal: `finish_restart(Err)`, predecessor's cleanup runs, no successor row | **N-ADM** (ADR-0338) |
| R5 | Restart `:2240-2262` | provision error that is not `GuestNetwork` | **N-UNR**, as S4 |
| R6 | Restart `:2263-2291` | any other `GuestNetwork` error: `Failed` successor row `WorkloadNetnsProvisionFailed` | unchanged |
| R7 | Restart `:2293-2330` | `ensure_intercept_identity` fails: successor unwound, `finish_restart(Err)`, no successor row | **`Failed` successor row** (ADR-0337), reason as S5, then `finish_restart`; a Job's row is final (S5-J) |
| R8 | Restart `:2332-2344` | no driver of the kind composed: synthesised `StartRejected`, `Failed` row | unchanged |
| R9 | Restart `:2349-2364` | duplicate owner: `finish_restart(Ok)` | **N-IDEM**, as S7. The successor's `Pending` write is skipped when its own row is non-terminal (§ "The guest address is on record"), so the duplicate writes no row and leaves the owner's row as it is |
| R10 | Restart `:2419-2439` | `Err(other)` from `driver.start`: `finish_restart(Err)`, no row | **`Failed` successor row**, as S8 (N-4, W3), then `finish_restart(Err)` |
| R11 | Restart `:2465-2526` | the successor row write fails | **N-REC** |
| R12 | Restart (SD-3) | `StartSuperseded` through `finish_restart` | **abandoned row** (N-4), then `finish_restart` |
| R13 | Restart `:2554-2574` | `write_alloc_lifecycle` returned `Ok(None)` (the store refused a non-dominating write): the successor is unwound, then `finish_restart` | **N-UNR**: `Ok(None)` is returned only when a row already holds the key with a dominating or equal timestamp (`observation_store.rs:1778-1781`); a freshly issued id has no row |
| R14 | Restart `:2616` (Start `:2151-2152`) | activation `Ok(false)` (the guest-network EXEC gate is at fail-stop, `guest_network.activation_withheld`): `Ok(())` after the Running row is written; the restart arm skips `finish_restart` | **N-FAILSTOP**: the successor's row exists; the skip happens only while the fail-stop ends `serve` (`overdrive-core/src/guest_network.rs:111`); the next boot's reclamation ends every unsupervised VMM |

The no-row outcomes and their handling:

- **N-SUP (supersession).** Caused only by an operator commit or by shutdown. Ends in the abandoned row (N-4, `LaunchSuperseded`), so it is no
  longer a no-row outcome. Relaunched by the next evaluation from the count basis, never counted. One id per commit or shutdown. A launch lost with the control plane is
  closed the same way at boot (`LaunchLostWithControlPlane`).
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
| allocation `a` ended by supersession (`why[a] == W_SUPERSEDED`) | the conjunction of three owner observables: `alloc_status_rows()` holds a `Terminated` row for `a` with `reason == LaunchSuperseded` and `terminal == None` (N-4); `Driver::live_allocations()` (`fn live_allocations(&self) -> Option<Vec<AllocationId>>`, `overdrive-core/src/traits/driver.rs`) does not list `a`; and the shim's `dispatch` result for the action is `Ok(())`. `from_driver` fails on a `Running` or `Failed` production row for a superseded id |
| `delivered[a]` | model-only ghost; the comparison of "a `LaunchSuperseded` row, never `Running`, for a superseded id" at every step already refutes a delivered result |
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
| `exhausted` | `alloc_status_rows()`: the count basis (the highest id with a row that is not abandoned, `last_ran_alloc`) carries `terminal == BackoffExhausted { attempts }`, read after the evaluation that observes that row has run; the model's `exhausted(s)` must equal it at every step. It is the one comparison that checks the production ceiling against the model's `LIMIT` (= ceiling + 1) |
| `why[a] == W_PREFAIL` | `alloc_status_rows()`: a `Failed` row for `a` whose `reason` is `MtlsInterceptInstallFailed { stage: "identity_issue", .. }` or `DriverInternalError { .. }`; `Driver::live_allocations()` does not list `a`; the dispatch result is `Ok(())` |
| `why[a] == W_REFUSED` (D-1) | no row for `a`, `Driver::live_allocations()` does not list `a`, the dispatch result is `Err(ShimError::GuestNetwork(AdmissionCapReached { .. }))`, and `GuestAddressPool` occupancy is unchanged by `a` |
| `why[a] == W_LOST`, now `as[a] == ABANDONED` (a launch that ended before running without a failure: superseded, or lost with the control plane; N-4) | `alloc_status_rows()` holds a `Terminated` row for `a` with `terminal == None` and `reason` `LaunchSuperseded` (the held tick completed with dispatch `Ok(())`) or `LaunchLostWithControlPlane` (after a rebuild, `CpCrash` or `NodeReboot`, and the boot `converge`, whose first step ends every `Pending` row this way whether or not its VM left a scope); `Driver::live_allocations()` does not list `a` |
| `as[a] == PENDING` | the id is issued (`issued_generation_for_alloc` is `Some`), `alloc_status_rows()` holds the `Pending` row of `a` (N-2) and the `run_convergence_tick` future the driver holds at its seam has **not** completed (the production tick is awaiting the dispatch, holding the lane). The driver reads the future's completion, the owner's own behaviour, not a map it maintains; after a rebuild and boot `converge` no row is `Pending`, so a `PENDING` expectation after a rebuild fails |
| `JOB` and `jobTerm(a)` | `JOB` is model-only (it selects the workload kind the driver submits). `jobTerm(a)` is `alloc_status_rows()` row `a` with `kind == WorkloadKind::Job` and `terminal.is_some()`, the test `generation_replacement_pending` and the shim's `allocation_attempt_transition` use |
| `faults`, `declared`, `idFault`, `capFull`, `conf_step`, `og`, `gl` | model-only: `idFault` is armed on the sim CA, `capFull` by filling the sim pool, `declared` is the effect of `submit_workload`, `conf_step` records the action taken for the driver and is never compared, `og` has no production counterpart, `gl` is checked through the carry relation of `cnt` |

Driving ports of the restart-generation actions (every `switch!` arm calls one): `Declare` -> `submit_workload`; `OpStop` -> `stop_workload`;
`OpResume` and `WorkloadRestart(a)` -> `restart_workload`; `WlStart(a)` -> one convergence tick whose emitted `StartAllocation` or
`RestartAllocation` dispatch is held at a test-owned seam (DISTILL shapes it; if it needs production surface DISTILL returns a testability
request and invents none); `LaunchRunning`, `LaunchStartFail`, `LaunchPreFail` and `LaunchRefused` -> release the held dispatch through the shim's
`dispatch` with the sim CA, `SimDriver` start result and sim guest-network pool armed for that outcome; `LaunchLost(a)` ->
`ReconcilerRuntime::supersede_lane(target)` on the held dispatch with neither `capFull` nor `idFault` armed (the guard of the model's
`LaunchLost`), whose `SimDriver` start observes the lane signal and returns `StartSuperseded` (the abandoned row, dispatch `Ok(())`); `StopAlloc(a)` -> the tick that emits `StopAllocation` and its dispatch; `VmmCrash(a)` -> the
`SimDriver` exit event that the exit observer turns into a `Failed` row; `JobExit(a)` -> the `SimDriver` exit of a Job's process; `CpCrash`,
`NodeReboot` -> rebuild the runtime and the stores (`kill_for_test`, View and stores reopened, boot reclamation run); `IdFaultOn/Off`,
`CapFill/Drain` -> sim arming. A `LaunchLost` and a `CpCrash` (after the boot `converge`) leave the same class of observable: an issued id whose row is `Terminated`, `terminal == None`, with
reason `LaunchSuperseded` or `LaunchLostWithControlPlane`, and a finished or dropped tick.

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
  same shim arms. A launch in flight has only its `Pending` row (N-2), so a stop has nothing to act on
  (`StopAllocation` is emitted only for `Running` rows); `VmDriver::stop` acts only on `Live`. The superseded launch then ends its `Pending` row
  with the abandoned row (N-4).
- **Owner:** the convergence owner's lane (ADR-0102), signalled by the two operator handlers and by node shutdown; observed by the driver.
- **Promise:** if the signal is raised, the launch it covers delivers no result and leaves nothing behind it; if it is not raised, the launch is
  exactly the launch G-1 and G-2 describe.
- **Affected state:** the result of this one launch (`StartSuperseded` instead of `Ok` or a typed failure).
- **Failure projection:** signal raised at any observation point -> `StartSuperseded`, the abandoned row (`Terminated`, `LaunchSuperseded`, N-4), no budget input; teardown failure while abandoning
  -> the existing unclassified `StartRejected` (a `Failed` row); signal raised with no lane open -> nothing (the evaluation's hydration reads the
  committed intent); signal after `accept_ready` was selected -> the launch succeeds and the stop is evaluated against its `Running` row;
  duplicate, repeated or late signals behave as one; disconnect (a control-plane crash) -> the lane table is process-local and is rebuilt empty,
  the launch is lost with the process and the next evaluation after boot relaunches it (boundary scenario 5); shutdown begun while the lane is opening, or before it opens -> the lane is born raised, the launch returns
  `StartSuperseded` at its first observation point and writes nothing.
- **Explicitly unaffected:** the restart budget (a superseded launch is not a failure), every other allocation row, the predecessor's terminal state
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
4. Late success: a stop or restart while a launch waits (claim poll, boot wait) ends the launch at once with `StartSuperseded`: the abandoned row, no budget
   charge, no VMM attached or left alive, the volume kept, the supervision claim, the guest network lease and the SVID released; a launch whose
   boot race had already selected `accept_ready` is an ordinary success that the stop then stops; an `ensure` in flight finishes first. A
   superseded launch's result is never delivered.
4a. Supersession, the remaining boundaries: (a) the signal raised with no lane open does nothing and the evaluation's hydration sees the commit;
   (b) a failure found at the same observation point as the signal is reported as `StartSuperseded`; (c) a teardown failure while abandoning
   reaches the shim as the existing unclassified rejection; (d) a driver that ignores the signal is unchanged; (e) a signal for one workload's
   lane never reaches another workload's launch; (f) `submit_workload`, resync and observation wakes never raise it; (g) a restart whose committed
   generation the evaluation already hydrated may see its own `Start` superseded once; the next evaluation relaunches; (h) node shutdown: a
   launch waiting on the claim or the boot, an `ensure` in flight and a lane opened after shutdown began all abandon (the `ensure` first
   finishing), return `StartSuperseded`, write the abandoned row and charge nothing, the loop drains and exits, and the next boot relaunches; a `Running`
   allocation and a non-launch evaluation are unaffected; `supersede_lane` never cancels the shutdown token.
5. Disconnect/reconnect: control-plane crash between `ensure` and VMM spawn: the next launch's `ensure` finds the volume and proceeds; a
   surviving VMM is reclaimed, its claim drains, the wait covers it. A launch that died with the control plane leaves its `Pending` row (N-2); boot
   ends it (ADR-0355, W4: `LaunchLostWithControlPlane`, the first step of boot, whether or not its VM left a scope) and the next evaluation relaunches it, the abandoned row not counted (§ Restart generation, "Budget"). A surviving VMM's run directory or clone is reaped by `VmReclamation` (`plan_reclamation`: `DiscardStrandedArtifacts`, or `ReclaimAllocation`
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
| Invariants (`Safety`) | `I_SINGLE_WRITER`, `I_DATA_SURVIVES_RESTART`, `I_ENSURE_IDEMPOTENT_UNDER_CRASH`, `I_NO_DRAIN_FAILURE_CHARGES_BUDGET`, `I_STOP_KEEPS_VOLUME`, `I_SUPERSEDE_ABANDONS_WAITS`, `I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING`, `I_SIGNAL_NEVER_LEAKS_ACROSS_ALLOCATIONS`; "a typed failure never attaches" and "shutdown signals every launch" are asserted by named runs, not invariants (2026-11-19) | `I_FINISHED_JOB_ISSUES_NO_ID`, `I_CAPACITY_WAIT_NEVER_EXHAUSTS` (`exhausted` implies the count basis ended by a charged cause: `W_CRASH`, `W_STARTFAIL` or `W_PREFAIL`), `I_ABANDONED_ROWS_DO_NOT_COUNT` (RB-13: deleting the `ABANDONED` allocations from a chain changes no count, `charged` or `exhausted`) |
| Temporal | `EveryStartLiveness` | `RunIntentProgress`, `PreFailExhausts` |
| Hazards, one per decision | `noWait` (VD-8), `recreate` (VD-9), `stopDeletes` (VD-2), `noInterrupt` (SD-1 to SD-5) | `stampOnEmit` (ADR-0335: the stamp on emit decides, violates `RunIntentProgress`), `prefailNoRow` (ADR-0337: a pre-driver failure writes no row, so an id is issued per tick, violates `PreFailExhausts`), `refusalCharged` (ADR-0338: an admission refusal is charged, so a node held at the cap exhausts a healthy workload's budget, violates `I_CAPACITY_WAIT_NEVER_EXHAUSTS`), `abandonWritesNoRow` (ADR-0355), `abandonedRowCounts` (RB-13: the count basis is the newest allocation, abandoned or not; violates `I_ABANDONED_ROWS_DO_NOT_COUNT`) |
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
| restart while a launch waits on the claim: the `LaunchSuperseded` row, no charge, no VMM | volume | `superseded_launch_result_discarded` |
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
for "a launch holds the lane" (the launch's row is `Pending` until the dispatch resolves, and a launch that ends before running leaves the abandoned row, N-4); one action resolves it in the order the shim runs, admission
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
| RA4 | a launch lost or superseded delivers nothing: an abandoned row (N-4), no charge, no VMM | the volume spec's `I_SUPERSEDED_LAUNCH_DELIVERS_NOTHING`; the shim test of a superseded restart successor |
| RA5 | no VMM survives to block a placement (a booting orphan is reclaimed before any launch; a `RUNNING` orphan holds a non-terminal row) | the volume spec's M7 and its DISTILL test |
| RA6 | a launch failure caused by the volume is one typed start failure that writes a charged `Failed` row | the volume spec's runs `no_space_fails_typed_and_never_attaches`, `launch_bound_exceeded_fails_typed_and_charges_budget` and `store_absent_fails_typed_and_never_attaches`, and the volume launch tests |
| RA7 | `LaunchPreFail` and `LaunchRefused` have the production order and effects of the table | shim tests of the pre-driver paths and of the refusal; `guest_network.rs:541-591` |
| RA8 | after `CpCrash` or `NodeReboot` the View, the intent and the observation rows survive and the runtime is rebuilt with a new lane table | ADR-0035 (View persistence), `vm_reclamation_boot::converge` before the loop |
| RA9 | the budget abstraction (R-1, RB-13): the model carries production's per-allocation count and derives `charged` and `exhausted` from the newest allocation that ran, with `LIMIT` = the production ceiling + 1 (`workload_lifecycle.rs:51`, `:408`, `:465`, `:1311-1318`); an abandoned allocation is never that allocation | the conformance driver compares `cnt` through `attempt_index_for_alloc` and `exhausted` through the rows; the run `generation_replacement_forgives_predecessor_failure`; the invariant `I_ABANDONED_ROWS_DO_NOT_COUNT` and its hazard `abandonedRowCounts`; the Tier-1 deletion-equivalence proptest (§ "Launches that end before running", item 6) |
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
  The same fault on the abandoned-row write (N-4, W1, W2) returns the typed error and leaves the launch's `Pending` row until the next boot closes it
  (W4). That stale `Pending` current row is not `Running`, not restartable and not a natural exit, so `WorkloadLifecycle` falls through to
  fresh placement and relaunches in the same boot under the next attempt id, one id per tick as on `main`. A launch whose VMM runs when its
  `Running` write fails is stopped in the shim's rejected-write arm (`driver.stop`, then `release_supervision`), which then retires the lease and
  tears down, so no duplicate VM runs beside the relaunch. Its `Pending` row stays until the next boot closes it (W4); boot's executors reclaim
  only a leftover scope or artifacts. When that launch's teardown also keeps its lease (the VMM was killed and not reaped, its scope
  still present: the release gate of ADR-0348), the lease stays Retiring and counts toward the cap until that boot, with the row; the pool is rebuilt
  at boot and step 4 of § "Boot proceeds past a stuck VM" reserves the address from the closed row if the scope remains. No reclaim of the lease
  exists before then, because the reclaim retry acts only on a terminal row and a `Pending` row is a live launch.
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
of `specs/quint/restart-generation/restart_budget.qnt` (a lost or superseded launch ends in an abandoned row that no derivation reads, N-4, RB-13; a generation replacement
inherits the count of the newest row that ran). The model
creates a volume inactive and activates it by a second effect (M1), a conservative refinement of LVM, where `lvcreate` returns an active volume;
inactive volumes arise from a node reboot (run `node_reboot_reactivates_and_data_survives`), which `ensure` absorbs.

## Wave: DISTILL

Specification: `distill/test-scenarios.md`. RED proof: `distill/red-classification.md`.
Executable obligations, placement, the DELIVER activation order, re-sync lists and open decisions: `distill/upstream-issues.md`.
No DISCUSS or DEVOPS wave existed; acceptance criteria derive from this document, ADR-0310 to ADR-0320 and the issue as reduced by VD-10.
