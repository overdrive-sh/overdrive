# Feature delta: node-local persistent volume for VM workloads (GH #348)

**Status: PROPOSED.** Every decision below is a proposal for the user to approve
(`design/wave-decisions.md` numbers them VD-1 to VD-10, with VD-7a to VD-7c). Nothing is accepted, no ADR is
accepted, and nothing here authorizes DISTILL or DELIVER until the user approves. The one
contract-shaping validation item, V-1, passed (§ Validation); no contract-shaping item is open.

Evidence: `spike/findings.md` (including § "V-1. LVM on a minimal node without udev"),
`spike/wave-decisions.md` (native metal host, Linux 7.0.0-38, Cloud Hypervisor fork `f252bb20b`
plus spike patches F1 to F3). Research:
`docs/research/platform/vm-persistent-volume-block-path-and-backing-research.md`,
`vm-volume-copy-on-write-clone-prior-art-research.md`.

## [REF] Slice and decision index

**Slice (VD-10, a scope decision for the user).** This feature delivers the loop that
`overdrive deploy` can drive today: declare a volume, create it, attach it, survive crash,
`workload restart`, stop, node reboot, and see it in `describe`. Workload deletion does not
exist (GH #211), so reclamation, erasure and a volume status row have no production trigger;
they are an entry contract for the #211 slice (§ Entry contract), not this slice's mechanism
(`CLAUDE.md` § Build vertical slices). Two consequences the user rules on knowingly: (a) on a
real appliance the slice is drivable only once the image provides the partition, LVM tools,
built-in dm and the volume group (L5; GH #75 is a placeholder), so until then only provisioned
nodes such as the metal host can run it; (b) with no deletion, each distinct workload id keeps
its volume's space permanently, even after stop, and the only remedy on an operator-less node is
re-provisioning. The entry contract E1 to E3 is part of VD-10 and is proposed, not ruled.

| Id | Decision | ADR |
|---|---|---|
| VD-1 | `[vm.volume]` with `size` (`GiB`/`TiB`); one per workload; `replicas = 1` | 0296 |
| VD-2 | Volume belongs to the workload; stop keeps it; only withdrawal deletes it | 0297 |
| VD-3 | Raw linear logical volume, extents reserved; hashed name in VG `overdrive` | 0298 |
| VD-4 | The image provides dm, `EFI_PARTITION`, devtmpfs, LVM tools and the provisioned, wiped VG on a labelled partition; Overdrive sets `LVM_SYSTEM_DIR=/run/lvm` and only verifies | 0299 |
| VD-5 | Boot probe (Earned Trust); refuse startup when the store is broken | 0300 |
| VD-6 | Second raw disk: write-back, `direct=on`, `image_type=raw`, `serial=ovd-volume` | 0301 |
| VD-7a | Fork change F1: a failed write-through fsync completes the request with an I/O error | 0302 |
| VD-7b | Fork change F2: a writable block device is opened `O_EXCL` | 0305 |
| VD-7c | Fork change F3: an explicit image type skips format sniffing | 0306 |
| VD-8 | Replacement waits for the claim to clear (30 s), not for process exit | 0303 |
| VD-9 | The launch path ensures (creates if absent, activates if inactive) the volume; no volume reconciler | 0304 |
| VD-10 | Slice scope: reclaim, erasure, status row, start gate, pinning, growth are not in this slice | none (wave decision) |

## [REF] Answers to the issue's DESIGN questions

1. **Spec surface:** `[vm.volume] size = "40GiB"`, one volume (VD-1).
2. **Identity and ownership:** keyed by `WorkloadId`; desired state is the workload intent; no
   separate record (VD-2).
3. **Lifecycle:** create-or-activate at launch, converging each time (VD-9); attach by launching
   the VMM with the second disk (VD-6); detach is VMM exit, the claim draining (VD-8); delete and
   garbage collection are the #211 slice's (§ Entry contract).
4. **Single writer:** kernel `O_EXCL` claim (fork F2, VD-7b) and Cloud Hypervisor's OFD lock; the
   replacement waits for the claim, not the process (VD-8).
5. **Backing and placement:** a `dm-linear` logical volume in VG `overdrive` on a dedicated
   labelled partition, outside `<data_dir>` and every clone surface (VD-3, VD-4).
6. **Placement pinning:** one node holds the volume; with one node (`local`) it is implicit. Not
   delivered (L3); the direction for the multi-node slice is an intent binding holding the `NodeId`,
   written once by an atomic put-if-absent at first placement and a hard scheduler constraint, not an
   observation row (eventually consistent) and not pinned at submission.
7. **Growth:** grow-only, only while detached (extend under the claim, relaunch); not delivered
   because the spec is immutable per id (L2).
8. **Relation to #97:** the node-local volume stays the local tier; an object-backed volume would
   be a new explicit declaration, never a silent change to `[vm.volume]`.

Answers 6 to 8 are directions that bind no code in this slice; the user approves the limitations
L2 and L3 that carry them, not separate decisions.

## [REF] Reuse analysis

Every overlapping component, with contract shape, universe and the crafter's assertion mechanism.

| Component | Disposition | Contract shape | Universe | Assertion mechanism |
|---|---|---|---|---|
| `VmDriver::provision_vmm` / `Vmm::create` | Extend: `ensure`, claim wait, second `--disk`, node chown | bounded-change | the volume's logical volume, its device node's owner and mode, the VMM argv; nothing else | argv golden test through the single `DiskAttachment` render site; `stat` of the node before and after; `SimVolumeBacking` records its calls |
| `DiskAttachment::to_disk_arg` | Extend: `serial`, `direct` | pure-function | none | unit + mutation target (already "the one site") |
| `VmReclamation` / `VmHostState` | Unchanged; volumes are not on its clone, run-dir or scope surfaces | pure-function plan | n/a | acceptance test: a reclamation pass leaves the volume untouched (`claim_state` result identical, device present) |
| `WorkloadLifecycle` | Unchanged except the three `VmPayload` projection sites | pure-function `reconcile` | none | existing ESR/DST invariants |
| `compose_vm_driver` / `run_server` | Extend: compose and probe the volume store first, pass it to the driver | bounded-change | boot events, driver registry | boot test with a broken substrate |
| `prepare_confined_paths` | Extend: chown the volume node | bounded-change | that node only; no operator artifact's mode widens | `stat` assertions |
| `AllocStatusResponse`, `render::workload_describe` | Extend: volume size | pure-function | none | render golden tests |
| `infra/metal/{partitions.json,provision.sh}` | Extend in DELIVER (test host only): partition, PV, VG | bounded-change | the test host's data disk | provisioning self-test |

No existing alternative covers volume creation or claim handling: there is no `lvm`, `dmsetup` or
`device-mapper` use anywhere in `crates/`, `infra/` or `xtask/`.

## [REF] End-to-end flow (production entry points)

All through `overdrive serve` and `overdrive deploy`; no test-only wiring.

1. **Boot.** `run_server` composes the volume store before the VM driver: find the partition labelled
   `overdrive-volumes` (kernel `PARTNAME` attribute); absent: soft `volume_backing.not_composed`;
   present: `probe`; failure refuses startup with `health.startup.refused` reason
   `volume_backing.probe`. The VM driver is composed with the store.
2. **Deploy.** `overdrive deploy dev-box.toml` with `[vm.volume] size = "40GiB"`. `submit_workload`
   validates (grammar, `replicas = 1`), archives the V2 envelope, writes the intent.
3. **Launch (first allocation).** `WorkloadLifecycle` places the allocation as for any VM workload.
   `VmDriver::start` calls `ensure` (creates the 40 GiB logical volume, reserving extents, and activates
   it), waits for the claim to be free (immediately free the first time), then the VMM adapter chowns the
   node to the confined uid and launches Cloud Hypervisor with the second `--disk`. The guest formats and
   mounts it.
4. **Crash, `workload restart`.** The old VMM dies (terminal row); the successor is minted
   (ADR-0104/0109); its launch runs `ensure` (activates the existing volume), waits up to 30 s for the claim
   to clear (ADR-0303), then attaches the same volume with its data.
5. **Stop.** `workload stop` ends the allocation; the volume stays; a resumed or restarted workload attaches
   it again.
6. **Node reboot.** Volumes are inactive after boot; the next launch's `ensure` activates the volume.
7. **Describe.** `overdrive workload describe <id>` shows the volume size; a volume failure shows as the
   allocation's typed reason.

## [REF] Interface contracts

Exact public and cross-crate contracts. Internal structure (private functions, module layout, the shell-out
helper and report parsing) is the crafter's.

### Spec surface (VD-1)

`overdrive-core/src/aggregate/`: `Vm` gains `volume: Option<VmVolume>`; `VmVolume { size: VolumeSize }`.
`VolumeSize` is a validated newtype of bytes with `FromStr`/`Display`/serde matching `<uint><unit>`, unit
case-insensitive `GiB|TiB`, value at least 1; `Display` emits `GiB` unless the value is an exact `TiB` multiple.
Both parser-side `VmInput`s gain `volume: Option<VmVolumeInput { size: String }>` (`deny_unknown_fields`); TOML is
`[vm.volume] size = "40GiB"`. Validation in `validate_driver_input`: `AggregateError::Validation { field:
"vm.volume.size", .. }` for grammar; `field: "vm.volume", message` "a volume requires replicas = 1" when the
workload's `replicas != 1`. The rule applies to every kind that carries `replicas` (Job and Service); a Schedule is
rejected at submit today, so a Schedule with a volume is rejected before this rule. `ServiceSpecOutput` /
`JobSpecInput` echo the declaration. `DriverPayload::Vm(VmPayload)` gains `volume: Option<VolumePayload { workload:
WorkloadId, size: VolumeSize }>`.

Persisted formats: (1) a `WorkloadIntentEnvelope::V2` bump with `JobV2`/`ServiceV2`/`ScheduleV2`/`VmV2` payloads and a
`ServiceSpecEnvelope` V2 per ADR-0048; the V1 fixtures are regenerated as an explicit frozen V1 (appending a larger V2
changes the root size rkyv archives), `known_discriminants` becomes `[0, 1]`, `FIXTURE_V2` is added in the same commit;
a stored V1 workload re-archives to different bytes so its `spec_digest` changes, and under the single-cut greenfield
policy the upgrade path is deleting the data directory (see Exposures). (2) The new `TransitionReason` variants are
appended at the end of the enum, which `AllocStatusRowV1` archives; if the archived size of `AllocStatusRowV1` changes,
the V1 fixtures are regenerated as an explicit frozen V1 and `discriminant_offset_from_end` is re-pinned in the same
commit (no new row envelope version: struct-literal call sites stay).

### Port: `VolumeBacking` (VD-3, VD-4, VD-5, VD-8, VD-9)

`overdrive-core/src/traits/volume_backing.rs`.

```
#[async_trait]
pub trait VolumeBacking: Send + Sync {
    async fn ensure(&self, workload: &WorkloadId, size: VolumeSize) -> Result<PathBuf, VolumeBackingError>;
    async fn claim_state(&self, workload: &WorkloadId) -> Result<ClaimState, VolumeBackingError>;
}
pub enum ClaimState { Free, Held }
pub struct ProbedVolumeBacking(/* constructed only by the adapter's probe */);
```

Behavioural contract (the trait docstrings pin these):

- `ensure` (bounded-change: creates or activates only `ovd-<hash(workload)>` in VG `overdrive`; returns the
  activated device path). An existing volume: activated, returned; its size is not compared. A missing volume with
  enough free extents: created with exactly `size` reserved (nothing is written and it is never formatted; LVM's
  default of zeroing the first 4 KiB is not relied on, the image wipe is, ADR-0299), activated, returned. A volume
  that exists but is inactive (the state after every node reboot, V-1) is activated: creation and activation are
  separate observations, never assumed from each other. An `lvcreate` that reports the volume already exists (an earlier
  attempt's command outlived its deadline and finished) is treated as an existing volume. Not enough free extents: `InsufficientSpace { needed, free }`, nothing created; `free` comes from
  the free extents observed before the create (the `lvm` report), not from mapping an exit status, since a filter
  refusal also exits 5. No store composed: `StoreAbsent`. A command failure, or the whole `ensure` outliving its one
  deadline (30 s, the same order as the existing boot wait): `StoreCommand { operation, source }`; a spawned command
  that outlives the deadline runs to its own end and is never aborted (`rust.md`). Idempotent and safe to repeat
  after a crash.
- `claim_state` (no persistent effect, returns without blocking): makes one non-blocking attempt to take the volume's
  exclusive claim (an `O_EXCL` open) and releases it; `Free` iff it took it. An unknown volume: `NotFound`. It is the
  authentic arbiter of the kernel's claim for the conformance projection. It does not take the OFD lock: for one open
  file description the kernel removes the lock before it releases the device, so a free claim implies a free lock
  (ADR-0303).
- Nothing else is a method: the launch path owns the bounded wait (below).

`VolumeBackingError` is `#[non_exhaustive]`: `StoreAbsent`, `NotFound { workload }`, `InsufficientSpace { needed:
VolumeSize, free: ByteCount }`, `StoreCommand { operation: VolumeOperation, source: io::Error }`. `ByteCount` is a new
validated newtype of bytes in `overdrive-core` (no existing bytes newtype exists). `VolumeOperation::as_str` is
enum-owned (kebab labels).

`overdrive-host`: `LvmVolumeBacking`, the only place that spawns `lvm`. Every spawn is `lvm <subcommand> --config '<pinned>' ...`
with `LVM_SYSTEM_DIR=/run/lvm` in the spawn's environment (V-1). The pinned `--config` is `activation{udev_sync=0 udev_rules=0
monitoring=0} devices{filter=["a|^<labelled partition node>$|","r|.*|"] use_devicesfile=0} backup{backup=0 archive=0}
global{use_lvmpolld=0 locking_dir="/run/lock/lvm"}`, the filter built from the node discovered by `PARTNAME`. LVM insists on a
system directory that no `--config` key can move. The variable is chosen over an empty `/etc/lvm` in the image (both passed in
V-1; ADR-0299): it keeps LVM's writes to `/run` and the physical volume, is quiet (an empty-string value prints backup warnings
on a read command), and is set at the one spawn site. Command surface: exit 0 is success
and stderr `WARNING: This metadata update is NOT backed up.` accompanies every metadata write, so stderr is never read as
failure; exit 5 is a filter refusal or `insufficient free space` (the `InsufficientSpace` source); exit 4 is a missing
system directory (a bug of this contract if it occurs). `lvs --reportformat json --units b` sizes carry a `B` suffix and the
report has a second top-level `log` key. When `lvcreate` returns 0 its device node already exists, so no polling is needed.
`overdrive-sim`: `SimVolumeBacking`, with fault injection (claim held until a `drain`
step, free-space exhaustion, command failure, store absent); it is the named arbiter for `claim_state` in conformance.

### Probe (VD-5)

`LvmVolumeBacking::probe() -> Result<Option<ProbedVolumeBacking>, VolumeProbeError>`: `None` when no partition is labelled
`overdrive-volumes`. `VolumeProbeError { scenario: VolumeProbeScenario, detail }`; `VolumeProbeScenario` (`as_str`):
`ToolsOrDeviceMapperUnusable`, `VolumeGroupMissing` (including a group not on the labelled partition: the device filter is
built from that partition; a label matched by more than one partition is the same scenario, the detail naming the ambiguity),
`ScratchCreateOrNodeFailed` (create fails, or the node does not appear), `ScratchRemoveFailed`.
It removes a scratch volume (name prefix `ovd-probe-`) left by a crashed probe first (such a volume is committed and inactive
after the reboot; `lvremove -f` removes it in either state, V-1), and skips the scratch step with an event when the pool has no
free extent. The node check runs once, after the create command returns. The probe touches no volume that is not a scratch
volume; a workload volume left inactive by a reboot is the launch path's `ensure` to activate. The full cycle measured about
95 ms (V-1) plus about 20 ms for the leftover-scratch scan, so the boot cost is not a design driver. The composition root
takes only `Option<ProbedVolumeBacking>`, which only the adapter's probe constructs (the subtype layer). The structural and
behavioural layers are whatever ADR-0082 D5 enforces for the existing probe adapters; this design adds no new enforcement
machinery.

### Disk attachment (VD-6, VD-7)

`overdrive-core/src/vm/config.rs`: `DiskSerial` newtype (at most 20 bytes, `[A-Za-z0-9_-]`); `VOLUME_DISK_SERIAL =
"ovd-volume"`. `DiskAttachment` gains `serial: Option<DiskSerial>` and `direct: bool`; `to_disk_arg()` renders
`path=<p>,image_type=raw[,readonly=on][,direct=on][,serial=<s>]`. `VmConfig` gains `volume: Option<VolumeAttachment {
device: PathBuf, serial: DiskSerial }>`. The volume `--disk` follows the rootfs `--disk` (so it is `/dev/vdb`).
`prepare_confined_paths` also chowns the symlink-resolved device node to `root:<VMM gid>` mode `0660` (a block device node
is refused to the dropped uid otherwise, spike C3); a failure is the existing `ConfinementUnavailable { control: UidDrop }`.
The owner and mode are not restored at teardown; no other VMM can open another workload's node because each VMM's Landlock
allow-list is derived from its own launch arguments (ADR-0301). `rlimit_fsize` is unchanged. Fork: F1 to F3 (ADR-0302, ADR-0305,
ADR-0306) and the pinned revision and checksums bump.

### Launch path and driver failures (VD-8, VD-9)

`VmDriver::provision_vmm`, for a `VolumePayload`, after claiming supervision and before `Vmm::create`: `ensure`; then poll
`claim_state` on the driver's `Clock` (the interval is the crafter's) until `Free` or 30 s have passed. One new
`#[non_exhaustive]` variant: `VmStartFailure::VolumeUnavailable { cause: VolumeUnavailableCause }` where
`VolumeUnavailableCause` is `StoreAbsent | InsufficientSpace { needed, free } | StoreCommandFailed { operation } | Held {
waited_ms: u64 }` (a `NotFound` for a volume `ensure` just returned is a bug and maps to `StoreCommandFailed`). One new
`TransitionReason::VmVolumeUnavailable { cause }`; `From<&DriverStartFailure>` stays total; `human_readable` names the volume,
the cause and the remedy. It is an ordinary start failure, charged to the restart budget. No volume store composed with a
declared volume: `StoreAbsent`. The launch holds its workload's convergence lane (ADR-0102) for at most the `ensure` deadline (30 s)
plus the claim wait (30 s) plus the boot wait (30 s), a worst case of about 90 s, 60 s more than the boot wait it already holds; a
`workload stop` or `restart` for that workload is evaluated after the launch ends, so it cannot pre-empt the wait.

### Operator surface

`AllocStatusResponse` gains `volume: Option<VolumeBody { size: String }>` (`#[serde(default, skip_serializing_if =
"Option::is_none")]`), filled by `handlers::alloc_status` from the workload intent. `render::workload_describe` gains
`render_volume_section` (presence-guarded): `Volume  40GiB (guest disk: virtio serial ovd-volume)`, the serial being static
text. A volume failure is visible through the existing allocation reason rendering of the new `TransitionReason`. OpenAPI
regenerated.

### Composition root

`run_server` order: cgroup preflight, probe-runner gate, `compose_volume_backing` (probe), `compose_vm_driver(volumes)`,
`registry.insert`, `boot_server`. `BootAdapters` gains the volume store (a `SimVolumeBacking` default for fixtures).

## [REF] Observable projections the conformance driver reads

Owner of the protocol: the VM launch path (`VmDriver::start`) with `WorkloadLifecycle`'s restart authority. Quint spec:
`specs/quint/node-local-volume/`.

| Model variable | Production accessor |
|---|---|
| `lv[w]` exists | `VolumeBacking::claim_state(&w)`: `NotFound` means absent |
| `claimed[w]` (a VMM holds the device) | `VolumeBacking::claim_state(&w)`, with **`SimVolumeBacking` the named arbiter** (the kernel's claim table is a driven-port fact the owner does not hold). Paired Tier-3 test: on `LvmVolumeBacking`, a scratch volume held by an `O_EXCL` opener reports `Held`, and `Free` after release, matching the sim for the same scenario. |
| `alloc[a].state` | `ObservationStore::alloc_status_rows()` (`state`) |
| `exhausted[w]` (BackoffExhausted reached) | `ObservationStore::alloc_status_rows()`: a row whose `terminal` is `BackoffExhausted { attempts }` (the source `handlers::restart_budget_from_rows` already reads) |
| `charged[w]` (attempts charged to the budget) | model-only. The budget is the reconciler's private per-chain `restart_counts` (`workload_lifecycle.rs:1289-1296`: carried per allocation chain, unchanged across `workload restart` and platform reclaim, reset by a fresh placement), so counting `Failed` rows or reading the row's `restart_count` (a crash-observability counter, ADR-0078) is not the budget. The invariant `no_drain_failure_charges_budget` is observed through rows instead: while the predecessor's drain is within the 30 s bound, no allocation row of the workload ends `Failed` with reason `VmVolumeUnavailable { Held }`, and `BackoffExhausted` is not reached. |
| `holder[w]` (which allocation holds the claim) | model-only (the kernel does not record it) |
| `data[w]` (the volume's contents survive) | model-only; Tier-3 read-back through `serve` + `deploy` |

Driving ports the actions map to: `submit_workload` (declare), `WorkloadLifecycle` reconcile and shim dispatch
(`StartAllocation`, `RestartAllocation`, `StopAllocation`), `VmDriver::start`, and `SimVolumeBacking` fault injection (VMM kill
leaves the claim until `drain`; store absent; no space; command failure). Single-writer is discharged by assumption A1 (the kernel
claim) plus the Tier-3 pairing test; the model encodes the kernel verdict, it does not prove it.

## [REF] Lifecycle Gate Ownership

### State ownership

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| allocation `Failed` with `VmVolumeUnavailable` | `VmDriver` start result, written by the shim | this launch attempt could not attach the volume | G-1, G-2 | the predecessor's terminal state, cleanup, the restart budget policy |
| allocation `Running` | driver start plus guest beacon | the guest reported ready | VMM boot, G-1, G-2 as launch preconditions | volume existence after launch |
| volume claim and lock | the kernel and the VMM | exactly one writer holds the device | CH's own open | allocation row state |

### Gate G-1 — `VolumeEnsure`

- **Existing evidence:** ADR-0104, ADR-0304; production path `submit_workload` -> `WorkloadLifecycle::reconcile` -> `schedule` ->
  `StartAllocation`/`RestartAllocation` -> shim -> `VmDriver::start`.
- **Owner:** the launch path of `VmDriver`, through `VolumeBacking::ensure`.
- **Promise:** the volume exists, is activated and its device path is returned.
- **Affected state:** the result of this launch (proceeds to G-2 or returns a typed start failure).
- **Failure projection:** `StoreAbsent`, `InsufficientSpace`, command failure, and a command past its deadline (a timeout is a
  command failure) -> `VolumeUnavailable`, recorded as the allocation's `Failed` row; duplicate or repeated `ensure` is
  idempotent; a launch whose result arrives after its allocation was ended is discarded (it runs to its bound, never aborted).
- **Explicitly unaffected:** running allocations, stop, GC, probes, readiness, `WorkloadLifecycle` placement.
- **Ordering:** after supervision claim; before the claim wait and `Vmm::create`; consumes the one `ensure` deadline (30 s, covering all
  its `lvm` spawns), additional to the 30 s claim wait and the 30 s boot wait.
- **Counterexample:** gating placement on volume existence (a `WorkloadLifecycle` gate with a status row) adds a row family, an
  interest and a gate for a create that is already convergent at the point of use (ADR-0304).
- **Evidence lane:** unit with `SimVolumeBacking`; seeded sim; Tier 3 through `serve` + `deploy`.

### Gate G-2 — `VolumeClaim`

- **Existing evidence:** ADR-0109 (terminal predecessor precedes successor), ADR-0106 (cleanup does not gate the successor),
  ADR-0303; production path shim `RestartAllocation` -> `VmDriver::start`.
- **Owner:** the launch path of `VmDriver`, through `VolumeBacking::claim_state`.
- **Promise:** at the instant of the attempt the volume's kernel claim was free.
- **Affected state:** the result of this launch.
- **Failure projection:** bound passed -> `VolumeUnavailable { Held }`, as the allocation's `Failed` row; unknown volume -> start
  failure (a bug, `StoreCommandFailed`).
- **Explicitly unaffected:** the predecessor's terminal state and cleanup (ADR-0106 unchanged), guest network lease, identity,
  restart-budget policy, the boot deadline value.
- **Ordering:** after `ensure`, before `Vmm::create`; its 30 s is additional to `VM_BOOT_DEADLINE` (30 s) and runs inside the one
  convergence lane the launch already holds (ADR-0102); one wait per attempt; a stop or restart for the workload waits behind it.
- **Counterexample:** waiting for VMM process exit releases the successor 2.4 s early (spike B2b) and fails it; deferring the restart
  in a reconciler would mint an allocation per poll.
- **Evidence lane:** seeded sim with `SimVolumeBacking` delayed release; Tier 3 on metal with a delayed in-flight write.

### Required boundary scenarios (executable obligations)

1. Gate available: `ensure` ok and claim free -> the VMM launches with the second disk and the guest sees it.
2. Gate unavailable: no store, no space, command failure or deadline -> `Failed` row with `VmVolumeUnavailable`; claim held past 30 s
   -> `Failed` row with cause `Held`; both charge the budget and neither leaves a VMM attached.
3. Unrelated state: a running allocation is unaffected by anything on the volume store; stop and GC proceed.
4. Late success: the lane serialises a stop or restart behind a launch, so a launch cannot be superseded while it waits; a restart
   evaluated after the launch ends never reattaches a stale result (model run `superseded_launch_result_discarded` is the conservative
   abstraction).
5. Disconnect/reconnect: control-plane crash between `ensure` and VMM spawn: the next launch's `ensure` finds the volume and proceeds; a
   surviving VMM is reclaimed by `VmReclamation`, its claim drains, the wait covers it. DISTILL confirms, through the production owner
   path, the existing recovery of a `Pending` allocation whose launch died with the control plane (this slice adds no recovery).
6. Feature disabled: a workload with no `[vm.volume]` makes no `ensure`, no wait, no extra `--disk`; a node with no volume store
   composes unchanged.

Seeded `overdrive-sim` invariants: `single_writer`, `data_survives_restart`, `no_drain_failure_charges_budget`; `assert_eventually!` a
replacement attaches once the predecessor's claim drains. Tier 3 (metal, guest group `cgroup-default`): create, write, `workload
restart`, read back; a node reboot and read back; a delayed in-flight write replacement; the `Held`/`Free` pairing test; a
broken-substrate probe catalogue.

## [REF] Formal protocol model

Specification: `specs/quint/node-local-volume/` (its `README.md` states the abstraction). Decisions modelled: VD-2, VD-8, VD-9.
Scenario-to-run mapping and verdicts are in § Quint results at the end of this document. Boundary scenarios 3 (unrelated state) and 6
(feature disabled) are not protocol behaviour and have no model run: seeded simulation and Tier 3 cover them.

Assumptions and what discharges each:

| Assumption | Discharged by |
|---|---|
| A1 a held `O_EXCL` claim refuses a second claimant on a dm-linear device, through any node | spike run-031 (F2); Tier-3 pairing test |
| A2 dm refuses to remove an open device (`drivers/md/dm.c:357-372 @ v7.0`) | not relied on in this slice (nothing removes a volume); entry contract for #211 |
| A3 the claim and lock outlive the VMM process while in-flight I/O drains | spike B2b; for one open file description the kernel removes the lock before the bdev `release` frees the claim (`fs/file_table.c:461,468 @ v7.0`), so a free claim implies a free lock; a lost race otherwise is an ordinary typed start failure (ADR-0303) |
| A4 LVM metadata operations are crash-atomic | V-1 (passed): 20 iterations of SIGKILLed `lvcreate` and `lvremove` and 15 whole-VMM SIGKILLs left a volume either absent or present, never half-built, no leaked extent, no orphan dm device, no manual repair (`v1-008`, `v1-011`) |
| A5 a control-plane crash after `ensure` and before VMM spawn leaves the volume intact for the next launch | A4 |
| A6 the device persists what it acknowledges after a flush | reasoned no-obligation: a media guarantee outside the appliance's control that only a power-loss test could probe |

## [REF] Validation

| Id | Item | Class | Falsification returns |
|---|---|---|---|
| V-1 | LVM tools work on a minimal image-equivalent node, and the labelled partition is found without udev | contract-shaping; **PASSED** (2026-11-17): § V-1 result below; `spike/findings.md` § "V-1", `spike/evidence/v1-001` to `v1-011` | none: VD-3, VD-4, VD-5 stand; the contract absorbs `LVM_SYSTEM_DIR=/run/lvm` and the image kernel features |

No validation item is open. The 30 s claim bound and the 30 s `ensure` deadline are policy values, not validation items: the worst case is a
charged, typed start failure, revisited if a production `Held` or command-timeout failure is ever observed.

**V-1 result** (nested guest on the metal host, kernel 7.0.0-38, read-only root with a tmpfs on `/run` only, no udev, no LVM daemon,
distro `lvm2 2.03.31` plus its `ldd` closure, a GPT backing file with a decoy and a look-alike partition; deciding run `v1-008`, metal run
`04b3d5f85b8c04814d243bf3`). All five steps pass on the pinned `--config` unchanged, with one addition recorded above:
`LVM_SYSTEM_DIR=/run/lvm` (without it every `lvm` command exits 4, `v1-005`, `v1-006`).

1. Discovery by `PARTNAME=overdrive-volumes` from sysfs finds exactly one partition (the look-alike `overdrive-volumes-old` does not match); the
   filter hides every other device and refuses `pvcreate`/`vgcreate` on them (exit 5); hidden devices are byte-identical through all four guests.
2. `vgcreate -s 4m`, `lvcreate -L 64m`: the `/dev/mapper` node exists about 8 ms into a 32 ms `lvcreate`, a block device of exactly 131072
   sectors, so no polling is needed. Writes go only to `/run`, devtmpfs and the PV (the root image hash is unchanged). Cost: `vgs` 20 ms, `lvcreate`
   32 ms, `lvremove` 41 ms: a probe cycle of about 95 ms.
3. `lvs --reportformat json -o ... --units b` parses (sizes `...B`, a second `log` key).
4. A fresh guest sees the volume inactive; `lvchange -ay` recreates the node in about 7 ms with no udev and the data written earlier reads back.
5. 20 SIGKILLed `lvcreate` and `lvremove` iterations (30 kills landed) and 15 whole-VMM SIGKILLs: no leaked extent, no orphan dm device, `vgs`, `lvs`,
   `vgck`, `vgchange -ay` and a clean `lvcreate` exit 0 with no manual repair. A killed `lvremove` can leave an LV present but inactive (4 of 17
   landed kills): nothing in this slice removes a workload volume, so that case is the #211 entry contract's; the reachable inactive volume here is
   the one every node reboot leaves, and `ensure` observes presence and activation separately (§ Port).

Not shown, bounded and not contract-shaping: behaviour under power loss or a torn media write (A6); concurrent `lvm` invocations and hundreds of volumes
(LVM serialises through `/run/lock/lvm`; Tier-3 in DELIVER observes `ensure` for two workloads at once); the real `ovd-<32 hex>` name and the first-4-KiB
claim; the confined uid's access to a dm node (spike C3 covers it); the appliance image itself (L5).

**Image requirements the result adds (ADR-0299):** `CONFIG_BLK_DEV_DM=y`, `CONFIG_EFI_PARTITION=y` (without it the kernel does not export `PARTNAME`),
`CONFIG_DEVTMPFS=y` mounted on `/dev`, a writable tmpfs on `/run`, the `lvm` binary with its library closure (no static `lvm` exists on the host: the
image carries the distro libraries, about 7.3 MB, or builds a static LVM2). No `lvm.conf`, udev rules, `dmeventd` or `lvm2-monitor` are needed.

## [REF] Entry contract for the workload-deletion slice (GH #211)

Not this slice's mechanism; the constraints the slice that makes deletion reachable must meet, from review and from model-checking a larger design
(a host-backed `volume-lifecycle` reconciler, a `VolumeStatus` row and a start gate, removed from this slice by ADR-0304). Proposed as part of VD-10;
the full text goes in the proposed comment on #211.

- E1. Deletion removes the workload intent only after the node reports the volume removed, and refuses a same-id resubmit meanwhile (otherwise a
  resubmit in the window attaches the old data).
- E2. Reclaim takes the volume's exclusive claim, marks the volume `reclaiming` before erasing, zeroes the whole device, then removes it; extents
  are zero before reuse so a deleted workload's data never reaches another workload. A present-but-inactive volume, which a killed `lvremove`
  can leave (V-1, 4 of 17 landed kills), is an ordinary state for reclaim.
- E3. Reclaim is planned only when the workload has no `Pending` or `Running` allocation (the larger model found a launch that had passed the claim
  check, a reclaim taking the claim, a control-plane crash, and the surviving VMM opening a volume marked `reclaiming`).
## [REF] Scope limitations and exposures (proposals; no issue created)

| # | Limitation | Pinned in | Lifting it | Issue |
|---|---|---|---|---|
| L1 | One volume per workload; no named volumes | ADR-0296 | `[vm.volume.<name>]`, serial `ovd-<name>`, per-volume identity | **propose new**: "Several named volumes per VM workload" |
| L2 | Size fixed at creation; no grow, no shrink | Answers, 7 | a size that can change after admission (the spec is immutable per id), offline `lvextend` under the claim, relaunch; the guest grows its filesystem | **propose new**: "Grow a node-local volume while detached" |
| L3 | Pinned to one node only implicitly (Phase 1 has one node `local`); no multi-node pin, no volume-space-aware placement | Answers, 6 | an intent binding `workloads/<id>/volume` holding the `NodeId`, written once by an atomic put-if-absent at first placement, a hard scheduler constraint, volume space in capacity | **propose new**: "Pin a workload with a node-local volume to its volume's node" (verified: #134 taints, #261 CPU/memory capacity and #282 replicas do not cover it) |
| L4 | Nothing deletes or erases a volume until workload deletion exists; freed space never returns; a volume's state is visible only through allocation rows | ADR-0297, ADR-0304 | the #211 slice per § Entry contract, which also decides a status row, describe surface and start gate | existing **#211** (verified: no volume, finalizer or node-local-state mention); propose a comment carrying E1 to E3 |
| L5 | The image does not yet define the labelled, wiped partition, LVM tools, built-in dm or the provisioned VG | ADR-0299 | the image build | existing **#75** (verified: it owns the node OS image build, nothing on disk layout or LVM); propose a comment listing the image requirements |

Not limitations (properties of the mechanism or of #348's stated scope; each is a ruling for the user to confirm): single writer, so `replicas = 1`; the
data is lost with the node; the guest owns the filesystem on a raw device; no snapshots, backups, replication or sharing (#97, #43, #96, #348 out of
scope); `[vm.volume]` is a property of VM workloads by definition (VD-1), so other driver kinds carry none, and a Schedule is rejected at submit
today independently of this feature; the volume group name, 4 MiB extent size, partition label and serial are the image and attachment contract
(ADR-0299, ADR-0301), a closed set chosen for correctness, not an operator knob; the 30 s claim bound and 30 s `ensure` deadline are derived from the
measured drain (2.4 s) and the existing boot wait, not an expectation to configure.

Exposures with a stated bound:

- The single-cut greenfield upgrade (deleting the data directory) removes the intent store while the volume group keeps its volumes: a redeploy of
  the same workload id then attaches the old volume with its old data and its old size, and the pool keeps the old reservation. The upgrade
  re-provisions the volume partition with the data directory (ADR-0299).
- A full volume pool fails each launch as `VolumeUnavailable { InsufficientSpace }` and ends as `BackoffExhausted` with that cause as the last reason
  (L4 explains why the pool only shrinks); admission does not check pool space.

## External integrations (handoff to platform-architect)

Contract tests recommended for the `lvm` command-line tools: the JSON report format and exit-status behaviour Overdrive parses are pinned by golden
outputs captured from the image's LVM2 version and re-captured on every LVM2 bump (consumer-driven contract against a subprocess). The Cloud Hypervisor
fork's disk argument surface (`serial`, `direct`, `image_type`, `O_EXCL` on block devices) is pinned by the F1 to F3 patch tests and the argv golden
test. Enforcement tooling: the existing dst-lint and `conformance-lint` families and whatever ADR-0082 D5 enforces for probe adapters.

## Handoff

DISTILL: scenarios per § Required boundary scenarios; conformance driver per § Observable projections; Tier-3 data-survival scenarios through `serve` +
`deploy` (restart, node reboot). Development paradigm: object-oriented (project `CLAUDE.md`); ports-and-adapters with the `overdrive-core` port,
`overdrive-host` adapter, `overdrive-sim` twin. The DELIVER roadmap sequences the work (DELIVER owns it).

## Quint results

`specs/quint/node-local-volume/` (checked on the metal host): `vol-safety-tlc` (exhaustive, 4 allocation ids, fault budget 2, restart-budget limit 3; see `evidence/summary.md` for
the recorded duration and memory, not a CI check) holds; `vol-live-start` (every start eventually attaches under fairness) holds, and a temporary fault budget equal to the restart
limit made it fail as predicted; hazards fail on their decision's invariant: `vol-hazard-wait` (VD-8, `I_NO_DRAIN_FAILURE_CHARGES_BUDGET`), `vol-hazard-ensure`
(VD-9, `I_DATA_SURVIVES_RESTART` / `I_ENSURE_IDEMPOTENT_UNDER_CRASH`), `vol-hazard-stop` (VD-2, `I_STOP_KEEPS_VOLUME`). No counterexample against the design.
The model's launch failure is one abstract typed failure; the model's `VolumeHeld` is the contract's `VolumeUnavailable { Held }`, and the `ensure`
deadline (a timeout is a command failure) changes names, not the modelled transitions.

| Scenario | Named run |
|---|---|
| deploy to a running allocation | `happy_path_deploy_to_running` |
| killed VMM, successor waits for the drain, same volume and data | `restart_waits_for_drain_then_attaches` |
| operator stop keeps the volume, resume reattaches | `stop_keeps_volume_and_restart_reattaches` |
| crash after create, before attach | `crash_after_create_before_attach_reuses_volume` |
| node reboot: volume inactive, re-activated, data survives | `node_reboot_reactivates_and_data_survives` |
| claim held past the bound | `launch_bound_exceeded_fails_typed_and_charges_budget` |
| no free extent | `no_space_fails_typed_and_never_attaches` |
| no store composed | `store_absent_fails_typed_and_never_attaches` |
| restart while a launch waits | `superseded_launch_result_discarded` |

Model assumptions that are not decisions of this feature, with no discharging item here: a `Pending` allocation whose launch dies with the control plane is
ended and charged by the existing allocation-recovery behaviour (DISTILL confirms it, boundary scenario 5); the model charges one attempt per `Failed` allocation, an abstraction of the
reconciler's per-chain `restart_counts` (which `workload restart` and platform reclaim do not charge, and a fresh placement resets): conservative for
the invariants, since it exhausts sooner; a VMM that survives a control-plane crash keeps its allocation `Running` until `VmReclamation` kills it. They
bound when `BackoffExhausted` is reached, not any invariant.

**Evidence is recorded and verified.** `specs/quint/node-local-volume/evidence/` holds the summary and the hazard counterexample traces; 14 of 14 checks met
`expect`, and `cargo xtask quint verify-evidence` passes for all eight subsystems. V-1 changes names, not modelled transitions: the model creates a volume
inactive and activates it by a second effect (M1), a conservative refinement of LVM, where `lvcreate` returns an active volume; inactive volumes arise from
a node reboot (run `node_reboot_reactivates_and_data_survives`), which `ensure` absorbs by activating a present volume. No spec change was needed: dropping
the OFD lock from `claim_state` leaves the model's single `claim` value unchanged.
