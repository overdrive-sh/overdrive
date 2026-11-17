# Node-local persistent volume protocol

Feature `node-local-persistent-volume` (GH #348). Decisions are numbered in `docs/feature/node-local-persistent-volume/feature-delta.md`: VD-2 (ADR-0311), VD-3 (ADR-0312), VD-8 (ADR-0317), VD-9 (ADR-0318). The ADRs are **proposed**; the model states their current decisions and nothing else.

The slice is the loop `overdrive deploy` can drive: declare a volume, create it at launch, attach it, survive crash, `workload restart`, stop, node reboot. Reclamation, erasure, a volume status row, a start gate and withdrawal are **out of scope for this slice** (ADR-0318, GH #211). A counterexample found against the larger design that contained them (model finding F-1) is recorded in the feature delta § Entry contract E3, not here.

## Abstraction

`node_volume.qnt`, module `node_volume`, is the state machine (`init`, `step`). One node, one workload (`replicas = 1`) with one declared volume, `NA` allocation ids (each minted once). The volume size is abstracted: "free >= size" is the boolean `spaceFree`.

| Owner | Modelled as |
|---|---|
| volume store | `storeUp` (composed or absent), `spaceFree`; one logical volume: `lv` (exists), `active` (false after creation until `ensure` activates it, and after a node reboot), identity `vid`, `nCreated` (volumes ever created); the guest's data as `dv` (model-only `data_version`), `maxDv` (ghost: the latest version ever written) |
| kernel claim (VD-8) | `claim`: 0 free, `a > 0` held by the VMM of allocation `a`. A killed VMM's claim lingers until `Drain`: the claim outlives the process (spike B2b). `claim_state` is the authentic arbiter; it reports `NotFound` for a missing or an inactive volume (no device node), see § Projection |
| launch path (VD-8, VD-9) | per allocation, control-plane memory: `ph` (supervision claimed, create-if-absent done, activated, claim reported Free), `waited` (poll steps, bound `BOUND`). Actions `LaunchEnsureCreate`, `LaunchEnsureActivate` (two effects; a control-plane crash can interrupt after each), `LaunchUnavailable` / `LaunchCmdFail` (typed `VolumeUnavailable`), `LaunchCheck`, `LaunchPoll`, `LaunchHeldFail` (typed `VolumeHeld`: the contract's `VolumeUnavailable { Held }`), `VmmSpawn` (takes the claim) / `VmmSpawnFail` (the VMM's own `O_EXCL` open is the atomic verdict), `LaunchDiscard` (a superseded launch's result) |
| workload-lifecycle | `WlStart`: mints a successor only when no allocation row is non-terminal (ADR-0109); it does not wait for cleanup or claim release (ADR-0106). `StopAlloc` after `OpStop`, `WorkloadRestart` (generation bump), `OpResume` |
| restart budget | `charged` (failed attempts, every typed start failure and every crash), `LIMIT` (design: 5; abstracted), `exhausted` = BackoffExhausted terminal |
| environment (faults, each draws on `FB`) | `VmmSigkill`, `DeviceSlow` (the dead VMM's in-flight I/O outlasts the launch bound), `LaunchCmdFail`, `CpCrash` (launch state lost, a spawned VMM keeps running until `Reclaim`, the existing reclamation), `NodeReboot` (volume inactive, VMMs gone, claims released). `StoreAbsentBoot` and `PoolFull` are boot-time substrate facts (before anything exists) |

Not modelled: volume size and `SizeMismatch`, `NameOccupied` and foreign logical volumes (the hashed-name collision is accepted by ADR-0312), the network, identity, probes, removal of any volume (nothing deletes a volume in this slice), a store that disappears after a volume exists (an exposure the feature delta bounds). Time is abstracted: a healthy drain finishes within the bound (`LaunchPoll` is enabled only when the holder is not a draining dead VMM, or after `DeviceSlow`).

## Instances

| Instance | Use |
|---|---|
| `vol_ok` | exhaustive safety (TLC): 4 allocation ids, fault budget 2, `LIMIT = 3`, every operator and boot-time action |
| `vol_conf` | the named runs (REC = true: `conf_step` records the action for the conformance driver) |
| `vol_live_start` | liveness: never stopped or restarted, fault budget 2 < `LIMIT` |
| `hazard/volume_hazards.qnt` | one weakened rule per decision |

## Decision, invariant, hazard

| Decision | Rule modelled | Invariants (`Safety` is the conjunction of all of them) | Hazard (`OFF`) and check |
|---|---|---|---|
| VD-8 (ADR-0317) | the launch waits for the claim, not the process; the kernel claim is the arbiter | `I_SINGLE_WRITER`; `I_NO_DRAIN_FAILURE_CHARGES_BUDGET` (no charged launch failure before the bound while only a draining claim was in the way) | `noWait`: the launch treats the VMM process exit as the claim free; `vol-hazard-wait` |
| VD-9 (ADR-0318) | `ensure` is create-if-absent then activate, idempotent under a crash between effects; failures are typed and charge the budget | `I_ENSURE_IDEMPOTENT_UNDER_CRASH`; `I_DATA_SURVIVES_RESTART` (shared with VD-2); `I_TYPED_FAILURE_NEVER_ATTACHES`; `I_BUDGET_BOUND` | `recreate`: `ensure` creates unconditionally; target `I_VD9` = data survives and one volume; `vol-hazard-ensure` |
| VD-2 (ADR-0311) | the volume belongs to the workload and outlives stop, restart, crash and node reboot | `I_STOP_KEEPS_VOLUME`; `I_DATA_SURVIVES_RESTART` (the volume present is volume 1 with the latest `data_version`, and an attached volume is active) | `stopDeletes`: stop deletes the volume; `vol-hazard-stop` |
| VD-3 (ADR-0312) | the backing choice (raw linear logical volume, extents reserved) | none: not a protocol rule | none |

`I_TYPED_FAILURE_NEVER_ATTACHES` and `I_BUDGET_BOUND` are structural: no action violates them, so they have no hazard of their own (one hazard per decision).

Liveness (weak fairness of every system action family, finite faults, `FB < LIMIT`): `EveryStartLiveness` = a declared volume with a composed store and free space eventually has a Running allocation (`StartLiveness`), and once an allocation has ended a Running one eventually exists again, i.e. a replacement attaches after the predecessor's claim drains (`ReplacementLiveness`); check `vol-live-start`. Teeth: with `FB = 3 = LIMIT` the same property is violated (three charged faults reach BackoffExhausted), verified during this run and reverted.

## Scenario to named run

All on `vol_conf` (the `#[quint_test]` pins).

The single scenario-to-run table is in `docs/feature/node-local-persistent-volume/feature-delta.md` § "Formal protocol model". Nine runs, all in `checks.toml`.

## Projection for the conformance driver

From the feature delta § "Observable projections the conformance driver reads", which is authoritative: `lv` and `active` are compared as `lv ∧ active ⇔ claim_state ≠ NotFound` (`VolumeBacking::claim_state`; an absent and an inactive volume both have no device node and report `NotFound`, so the state `lv ∧ ¬active`, which run `node_reboot_reactivates_and_data_survives` reaches, projects as `NotFound` and is otherwise observed by Tier-3 read-back; `SimVolumeBacking` reports `NotFound` while it models a volume as inactive); "claimed" from `claim_state` with `SimVolumeBacking` the named arbiter (paired Tier-3 test: `Held`, `Free`, inactive); `as` and `exhausted` from `ObservationStore::alloc_status_rows()` (`state`; a `BackoffExhausted { attempts }` terminal). Model-only (the driver does not project them; `charged` is observed through the rows as stated in the feature delta): `charged`, `holder` (which allocation holds the claim), `dv` / `maxDv` / `vid` / `nCreated` (Tier-3 read-back), `storeUp` / `spaceFree` (fault-injection inputs), `ph`, `waited`, `alive`, `orphan`, `why`, `ran`, `faults`, `slow`, `drainFail`, `conf_step`. This projection rule is the conformance driver's comparison rule; it changes no spec file.

## Assumptions

Feature-delta assumptions:

| Assumption | How it appears in the model | Discharged by |
|---|---|---|
| A1 a held `O_EXCL` claim refuses a second claimant on a dm-linear device, through any node | `VmmSpawn` takes the claim atomically; `claim` is one value | spike run-031 (F2); Tier-3 `Held`/`Free` pairing test |
| A2 dm refuses to remove an open device | not relied on: nothing removes a volume in this slice | entry contract for GH #211 |
| A3 the claim and lock outlive the VMM process while in-flight I/O drains | `VmmSigkill`, `StopAlloc`, `WorkloadRestart` leave `claim = a` until `Drain`; `GuestWrite` is allowed while the dead holder drains | spike B2b (2.4 s); the model has one `claim` value because `claim_state` takes only the kernel claim, and a free claim implies a free lock for one open file description (`fs/file_table.c:461,468 @ v7.0`) |
| A4 LVM metadata operations are crash-atomic | each store effect (create, activate) is one atomic action; a failed command leaves nothing partial | V-1 (passed: SIGKILLed `lvcreate`/`lvremove` and VMM kills leave a volume absent or present) |
| A5 a control-plane crash after `ensure` and before VMM spawn leaves the volume intact for the next launch | `CpCrash` loses launch state only; `lv`, `dv` persist | A4 |
| A6 the device persists what it acknowledges after a flush | out of model (power loss not tested) | none (stated in the feature delta) |

Modelling assumptions (this spec; each is a choice that can be revisited):

| Id | Assumption | Discharged by |
|---|---|---|
| M1 | the volume is created inactive and activated by a second effect; a node reboot leaves the volume present and inactive. A conservative refinement: real `lvcreate` returns an active volume; an inactive one comes from a reboot or a killed `lvremove` | V-1 steps 2, 4 and 5 (passed) |
| M2 | a control-plane crash marks the allocation whose launch was lost `Failed` (charged); a spawned VMM stays running with its row `Running` until `VmReclamation` kills it, then the row is `Failed` (charged) and the claim drains | a conservative abstraction of the existing path: in production the lost launch leaves no row and the next `WorkloadLifecycle` tick mints a fresh allocation (a failure-driven restart launch's budget was charged at issuance), and `VmReclamation` reaps a surviving VMM's artifacts (feature delta § Required boundary scenarios, 5); DISTILL confirms it through the production owner path. Charging more than reality exhausts sooner; it bounds when `BackoffExhausted` is reached, not an invariant |
| M3 | a healthy drain finishes within the bound; `DeviceSlow` models the in-flight I/O that outlasts it | spike B2b; the 30 s bound is a policy constant (feature delta § Validation) |
| M4 | a launch that waited the whole bound ends the stuck-drain condition (`LaunchHeldFail` clears `slow`): the bound is the longest stuck request (ADR-0317). Without it, weak fairness lets every successor wait its bound and exhaust the allocation ids | spike B2b; the exposure "a claim that never drains" in the feature delta (ADR-0317 consequences); the 30 s bound is a policy value |
| M5 | a superseded launch (operator stop or `workload restart` while Pending) runs to its bound and its result is discarded (`reconcilers.md`); it may still run `ensure` | the Tier-3 and seeded-sim scenarios of feature delta § Required boundary scenarios 4 |
| M6 | the restart budget is not reset by a successful start; every Failed allocation (typed failure or crash) charges one attempt; operator stop and restart do not | an abstraction of the existing per-chain `restart_counts` (operator restart and platform reclaim do not charge; a fresh placement resets), not changed by this design; conservative, it exhausts sooner; it bounds when `BackoffExhausted` is reached, not an invariant |

## Results

Native metal host, ordinary `cargo xtask metal run --` grammar, 14 checks, all as expected: `vol-safety-tlc` holds (exhaustive, 280 s as recorded in `evidence/summary.md`, 1.7 GB peak), `vol-live-start` holds, the three hazards (`vol-hazard-wait`, `-ensure`, `-stop`) are violations, the nine `vol-run-*` named runs hold.

Evidence is recorded in `evidence/` (summary and the hazard counterexample traces): 14 of 14 checks met `expect`, and `cargo xtask quint verify-evidence` passes for all eight subsystems.
