# ADR-0104 — VM recreation uses a fresh `AllocationId` and retains the logical workload owner

## Status

**Accepted — independent DESIGN review iteration 2 APPROVED; user-authorized**,
2026-09-12. Approved design commit:
`a0f9bda8cd4f2377c1a709e77e8c05850e7adaa2`. This ADR is
application/component scope for `vm-recreation-allocation-id-reuse` (GH #284)
and uses the repository's OOP, modular-monolith, ports-and-adapters
architecture.

**Corrective status, 2026-09-13:** the user rejected this ADR's VM-only
boundary. The user-ratified
[ADR-0105](adr-0105-driver-neutral-allocation-replacement-identity.md),
[ADR-0106](adr-0106-successor-creation-does-not-wait-for-predecessor-cleanup.md),
and [ADR-0108](adr-0108-durable-reservation-consumes-successor-allocation-identity.md)
plus [ADR-0109](adr-0109-replacement-requires-terminal-predecessor-handoff.md)
record the replacement choices. The exact contract ratified as P-105-1 through
P-105-7 lives in the canonical feature delta. Final independent review
iteration 2 returned `CHANGES_REQUESTED`; its bounded F-03…F-06 documentation
findings were remediated without a third review under the user's two-cycle cap,
so explicit user disposition remains required.
[ADR-0107](adr-0107-successor-identity-consumption-follows-successor-owned-effect.md)
was withdrawn before acceptance. This record is historical #284 evidence, not
PR #292 merge authority.

This ADR amends the VM-specific replacement identity described by ADR-0073,
ADR-0081, ADR-0083 and ADR-0089. It does not change their accepted ending
taxonomy, claim lifecycle, public port signatures, network ownership, or
cleanup ordering except where this record explicitly says that a VM
replacement is represented by the already-existing `StartAllocation` action.

## Context

Issue #284 records two native failure directions:

1. A failed same-allocation Service recovery attempt binds the predecessor's
   allocation-derived beacon pathname and receives `EADDRINUSE`.
2. A replacement creates the allocation-derived path and the predecessor's late
   reaper removes it; replacement staging then receives `ENOENT`.

The current production path still has the defect. `WorkloadLifecycle` selects a
restartable row and `restart_allocation_action` emits
`Action::RestartAllocation` with the same `alloc_id` and `spec.alloc`
(`crates/overdrive-reconcilers/src/workload_lifecycle.rs:929–1044, 1407–1441`).
The action-shim restart arm stops, re-provisions and starts with that same key
(`crates/overdrive-control-plane/src/action_shim/mod.rs:2253–2751`).
`VmDriver::provision_vmm` and `CloudHypervisorVmm::create` then derive the VM
run directory, beacon/API/vsock/console/kernel-copy paths, cgroup scope, clone
and clone-index identity from that key.

The existing `VmReclamation::hydrate_vm_reclamation_desired` join currently
admits `WorkloadIntent::Job` entries whose driver is VM. The native Service
bind reproduction reaches the `WorkloadLifecycle` restart branch directly; this
ADR does not widen that reclaimer join without a separately reproduced #284
reclaimer defect.

The current HEAD revalidation used the existing production-owner seed 257205
under the required Lima runner. Its no-restart control passed; the same-ID case
recorded one VMM, the second beacon bind failure and a cgroup kill of the
original VMM, with no second VMM. The issue's qualified-metal trace supplies
the complementary late-reaper deletion evidence. These observations are
distinct from the earlier source SHA named in #284; the current source still
contains the same owner path and same-ID action.

## Decision

### D1 — WorkloadId is the stable logical owner

The existing workload-scoped `WorkloadId` target remains the stable logical
owner of desired state, restart policy, failure memory and current-allocation
selection. The current application model has one effective placement per
workload target; no replica identity, `VmInstance`, `VmIncarnationId`, revision
row, or new aggregate is introduced by this ADR.

The stable owner is represented by the existing `WorkloadLifecycle` target
`workload/<workload_id>` and its persisted `WorkloadLifecycleView`. The
allocation rows remain physical execution records, not the logical owner.

### D2 — VM replacement uses existing StartAllocation with a fresh ID

For `WorkloadDriver::Vm(_)`, the automatic replacement branch emits the
existing action with this exact shape:

```text
Action::StartAllocation {
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    node_id: NodeId,
    spec: AllocationSpec,
    kind: WorkloadKind,
}
```

The `alloc_id`, `spec.alloc` and `SpiffeId::for_allocation(workload_id,
alloc_id)` values are the same **fresh** ID. It is minted in the pure
`WorkloadLifecycle` decision before the action-shim network/VM effects, using
the existing `mint_alloc_id(&WorkloadId, attempt)` producer. The VM attempt is
the checked numeric successor of the greatest attempt suffix present in either
the workload's retained allocation rows or the existing View's VM issued-ID
keys. The existing `Action::RestartAllocation { alloc_id, spec, kind }` and its
action-shim implementation remain unchanged for `Exec` only.

The explicit `overdrive workload restart` generation path already follows the
fresh `StartAllocation` path and is preserved. A running-origin generation
replacement still ends the old allocation first through the existing
`StopAllocation`, then places the fresh one. A failed/reclaimed VM row is
already terminal and is re-driven directly through fresh `StartAllocation`.

No `ReplaceAllocation` action or second public method is added. The action-shim
does not receive a predecessor ID because its existing fresh-start action is
the complete creation boundary; the predecessor remains in ObservationStore
and is handled by its existing cleanup/reclamation owner.

The private DELIVER seam is the existing helper with one implementation-only
argument:

```text
fn restart_allocation_action(
    job: &Job,
    desired: &WorkloadLifecycleState,
    row: &AllocStatusRow,
    attempt: u32,
) -> Action
```

For `WorkloadDriver::Vm(_)`, `attempt` is
the value returned by the private `next_vm_attempt` decision below and the
helper returns a fresh `StartAllocation` using
`mint_alloc_id(&job.id, attempt)`. For
`WorkloadDriver::Exec(_)`, it returns the existing same-ID
`RestartAllocation`. This helper remains private; the action enum, public
traits and all port signatures retain their current shape.

The private VM ID decision is pinned as:

```text
fn next_vm_attempt(
    allocs: &[&AllocStatusRow],
    view: &WorkloadLifecycleView,
) -> Option<u32>
```

It takes the maximum parseable numeric suffix across `allocs[*].alloc_id` and
`view.restart_counts.keys()`. With no parseable suffix it returns `Some(0)`;
otherwise it returns `max.checked_add(1)`. `None` means the existing `u32`
attempt domain is exhausted: emit no VM start and do not clamp to or reuse
`u32::MAX`. This safety-stopped edge adds no error/state/API surface. Exec
initial placement retains its current row-count derivation; this helper is the
VM path only.

Before returning any VM `StartAllocation` -- initial placement, explicit
generation replacement, Workload Failure replacement, or Platform Reclamation
replacement -- `reconcile` inserts the fresh ID into the existing
`next_view.restart_counts` map. That key presence is the durable issuance
record consumed by `next_vm_attempt`. The reconciler runtime fsyncs that View
before dispatch, so the ID is consumed before any action-shim network or VM
effect. A View write failure dispatches nothing and therefore consumes no
execution identity; a crash after the View fsync but before dispatch may leave
an unused gap, which is safe and deterministic.

### D3 — Current allocation and lineage are derived from retained rows

The existing private projection remains the current-selection contract:

```text
current_alloc<'a>(&[&'a AllocStatusRow]) -> Option<&'a AllocStatusRow>
```

It selects the row with the numerically highest attempt suffix in the existing
`alloc-{workload}-{attempt}` grammar. There is no persisted
`workloads/<id>/current` pointer in this slice. The current reference changes
only after the fresh `StartAllocation` row is accepted by the existing
`ObservationStore::write_alloc_lifecycle` boundary, which atomically accepts
that current row and its occurrence without deleting or overwriting the
predecessor row. A View-only issued-ID key is deliberately **not** current: it
records that an identity was consumed, not that publication succeeded.

The existing publication signature remains:

```text
async fn write_alloc_lifecycle(
    &self,
    current: AllocStatusRow,
    source: TransitionSource,
) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError>
```

When that call returns `Ok(Some(_))`, the fresh key is accepted as a new
current row plus occurrence in one existing store operation. `Ok(None)` or
`Err(_)` does not change current selection; the pre-dispatch View reservation
still consumes the ID. No second pointer write or cross-key transaction is
added.

The predecessor/history representation is therefore:

- the old terminal `AllocStatusRow` under its old `AllocationId`;
- the existing bounded `AllocLifecycleOccurrenceRow` history for that ID; and
- the existing workload describe `rows` collection, which presents old and
  fresh physical attempts together.

`LastTerminated` and `AllocStatusRow.restart_count` remain per-physical-key
ADR-0078 facts. They are not copied into a fresh row, because the fresh row
does not supersede the old row at its LWW key.

The existing public View field shapes stay unchanged, but their VM semantics
are explicitly amended rather than hidden under the old rustdoc:

```text
restart_counts: BTreeMap<AllocationId, u32>
last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>
```

The public field rustdoc changes in the same implementation cut; its required
meaning is exact: `restart_counts` documents “Exec: same-ID restart decisions
for this allocation. VM: key presence reserves an issued physical execution
ID; the value is the Workload Failure budget carried at that candidate.”
`last_failure_seen_at` documents “Exec: latest failure for this allocation.
VM: latest genuine Workload Failure carried at this candidate; Platform
Reclamation may carry it but never stamps a new value.” No caller may continue
to describe either VM field as a maximum over unrelated historical entries.

- **Exec:** both maps retain their accepted per-allocation meaning. The count
  is the number of same-ID restart decisions for that allocation; the timestamp
  is that candidate's latest failure input.
- **VM `restart_counts`:** key presence means `WorkloadLifecycle` has issued
  that physical VM `AllocationId`, whether or not an `AllocStatusRow` was later
  accepted. The value is the stable owner's Workload Failure budget carried at
  that candidate. This is both a candidate-keyed policy input and the durable
  issued-ID ledger; it is not the operator-visible
  `AllocStatusRow.restart_count`.
- **VM `last_failure_seen_at`:** the value remains candidate-keyed input for
  the latest genuine Workload Failure. A replacement carries that optional
  input to its fresh issued-ID key. Platform Reclamation never stamps a new
  failure time.

For an ordinary VM Workload Failure, read only
`restart_counts[candidate]`/`last_failure_seen_at[candidate]`, preserving
ADR-0102's candidate rule. If the ceiling/backoff permits replacement,
increment the Workload Failure budget once, write that value and
`tick.now_unix` under both the still-current candidate and the fresh issued-ID
key, and return the fresh action. Updating the candidate is load-bearing when
the fresh Running publication is rejected: the old row remains current and its
candidate-keyed backoff/budget must reflect the consumed launch attempt.

For VM Platform Reclamation, carry the candidate's budget and optional prior
failure time to the fresh issued-ID key without incrementing or stamping them.
This is the minimal amendment to ADR-0083's “writes no View field” rule: the
reclamation still consumes no Workload Failure budget and records no failure,
but its new physical ID must be reserved before dispatch. Initial placement
uses budget zero/no failure time. Explicit generation replacement carries the
current candidate's inputs without incrementing them. A successful Running
row retains the existing next-tick clearing behavior for that current
candidate's failure timestamp.

`reconcile` and `next_evaluation_at` therefore use the exact candidate selected
by the Run branch for budget/backoff; they do not scan historical View values.
Only `next_vm_attempt` scans VM issued-ID **keys**, because their stated
persistence contract is identity consumption. This explicitly amends the
ADR-0102 prohibition only for ID selection, not for retry policy. No View field,
owner pointer, row, codec or wire projection is added.

With retained predecessor rows, the VM restart candidate is the current row
selected by `current_alloc(&allocs_vec)` before `is_restartable` is applied. A
historical predecessor is never re-driven merely because it is restartable;
the existing single-key Exec candidate behavior is unchanged. The same
VM-current-candidate rule is used by `next_evaluation_at`.

### D4 — Existing execution capabilities own cleanup

The existing `LiveVm` remains the move-only capability for one physical
execution. It contains the `VmControl`, accepted beacon writer, private pending
`BeaconMessage::Exec`, cgroup `CgroupPath`, `VmRunDir`, `RootfsPlan` and
Running-confirmed gate. The existing
`ClaimGuard`/`ReclamationLease` map claims and accepted-session `Weak` identity
remain the only claim arbitration.

The VM artifact identity inventory is deliberately narrow. The adjacent
netns/TAP row is shown only as an explicit no-change check required by #284's
anti-generalization constraint:

| Artifact/effect | Existing identity owner | Decision |
|---|---|---|
| Run directory, beacon, vsock, API, console, kernel copy | `VmRunDir::for_alloc(root, &alloc)` and its child accessors | Fresh ID separates every path. |
| Rootfs clone and clone-index link | `RootfsPlan::for_alloc(..., &alloc, ...)`, existing recorded-link cleanup | Fresh ID separates clone/link names; the old capability or old reclaimer key cleans only old artifacts. |
| VM cgroup scope | `CgroupPath::for_alloc(&alloc)` | Fresh ID separates VM scopes. The inclusion is required by #284's captured failed-start `cgroup.kill`; generic Exec cgroup behavior is unchanged. |
| Hypervisor process | `VmControl`/PID in `LiveVm` and the existing VMM PID map | Fresh start receives a new control; no process lookup by logical owner or new ID fallback. |
| Netns/veth/TAP slot | Existing `NetSlotAllocator` + teardown-before-release | No change. #284 does not reproduce a TAP ownership mutation; no proximity-based generalization is authorized. |

The existing private move/claim signatures remain the ownership boundary:

```text
ClaimGuard::new(
    alloc: AllocationId,
    live: Arc<LiveMap>,
    beacon: Weak<BeaconWriter>,
) -> ClaimGuard
ClaimGuard::try_begin_ending(&mut self) -> Option<LiveVm>
ReclamationLease::try_acquire(
    drivers: &DriverRegistry,
    alloc_id: &AllocationId,
) -> Option<ReclamationLease<'_>>
```

`ClaimGuard` drops only its originating `Live` session; successful
`try_begin_ending` transfers the one `LiveVm` to the ending owner. A
`ReclamationLease` drops the corresponding `Driver::release_supervision` claim.
The `Driver` port signatures remain:

```text
async fn start(&self, spec: &AllocationSpec) -> Result<AllocationHandle, DriverError>
async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError>
fn live_allocations(&self) -> Option<Vec<AllocationId>>
fn try_begin_reclamation(&self, alloc: &AllocationId) -> bool
fn release_supervision(&self, alloc: &AllocationId)
```

The path and host-cleanup contracts remain exact:

```text
VmRunDir::for_alloc(root: &Path, alloc: &AllocationId) -> VmRunDir
RootfsPlan::for_alloc(
    master: PathBuf,
    master_bytes: u64,
    alloc: &AllocationId,
    staging_dir: &Path,
    index_dir: &Path,
) -> RootfsPlan
CgroupPath::for_alloc(alloc: &AllocationId) -> CgroupPath
async fn VmHostState::kill_scope(&self, scope: &CgroupPath) -> io::Result<()>
async fn VmHostState::discard_artifacts(&self, alloc: &AllocationId) -> io::Result<()>
async fn Vmm::create(&self, config: &VmConfig) -> Result<VmProcess>
async fn Vmm::terminate(
    &self,
    control: &VmControl,
    grace: Duration,
) -> Result<VmTermination>
```

`RootfsPlan`'s clone location remains the recorded clone-index target; only
the platform-owned link pathname is derived from the exact allocation key.

`VmHostState::kill_scope(&CgroupPath)` and
`VmHostState::discard_artifacts(&AllocationId)` keep their exact signatures.
The latter's accepted allocation-keyed host contract reads the recorded
clone-index target; this ADR adds no fallback that uses the logical owner's
current ID to guess or re-derive an arbitrary physical path. Cleanup of a
fresh start that fails remains the existing fresh-start unwind, and cleanup of
a terminal old allocation remains the existing disposal/reclamation path.

### D5 — Lifecycle meanings and ordering stay separate

The feature adds no lifecycle state, terminal condition, transition-reason
variant, wire field, route, configuration or timeout.

- `Platform Reclamation` still writes `Terminated / Stopped { by:
  PlatformReclaimed }` on the old allocation and wakes the same logical
  workload owner; its VM successor uses a fresh ID.
- Service startup failure and other existing VM Workload Failure restart
  candidates use a fresh ID, while the existing WorkloadLifecycle budget and
  backoff policy remain candidate-keyed with carried stable-owner inputs.
- Job natural exit remains final; Operator and SystemGc stops remain intentional
  stops; Service Stable/readiness/liveness retain their current owners.
- The action-shim continues to provision network state, then the driver claims
  and creates VM artifacts, then the fresh Running row is published, then
  existing Running-confirmed hooks run. No retry, sleep, global lock, barrier,
  or detached cleanup is introduced.
- The runtime persists the VM issued-ID reservation before that dispatch. If
  initial Running publication is rejected after driver start, the existing
  action-shim path awaits VM stop, releases supervision, tears down interception
  and structural networking, and returns the typed write error. The runtime
  then requeues; the next evaluation keeps the old current row but mints above
  every persisted issued-ID key, so it cannot reuse the rejected execution's
  identity.
- On process restart, View bulk-load restores every issued-ID reservation before
  convergence resumes. Any surviving host residue is observed and disposed by
  the existing `VmReclamation`/`VmHostState` path under that exact old ID; a
  later VM start receives a higher ID and may run before or after that cleanup.

## Lifecycle Gate Ownership

| Signal/state | Owner | Promise | May gate | Must not gate |
|---|---|---|---|---|
| Stable workload intent/generation | IntentStore + existing workload handler | Desired WorkloadId remains declared; generation advances through the existing atomic transaction | WorkloadLifecycle desired projection | Driver readiness or artifact cleanup |
| Current allocation | WorkloadLifecycle | Numeric-max retained row is the current physical attempt; accepted fresh row changes the projection while View-only reservations do not | Retained rows and owner target | Service Stable, backend health, old cleanup |
| VM replacement identity | WorkloadLifecycle decision, runtime View persistence, action-shim execution | Fresh `AllocationId` is durably reserved before fresh VM effects | Allocation/path identity for one replacement | Exec restart, lifecycle classification, TAP semantics |
| VM execution claim | VmDriver | Exactly one old or fresh execution owns its `LiveVm` ending/cleanup capability | Existing `LiveMap` and session witness | Logical current selection or unrelated allocations |
| Platform Reclamation / Artifact Disposal | VmReclamation and existing executors | Old allocation host state is reclaimed/disposed only under its own key | Existing host observations, terminality and claim | Fresh replacement artifacts, supervised nonterminal VM |
| Running / Stable / readiness / liveness | Existing action-shim and ServiceLifecycle owners | Existing state meanings and health boundaries | Existing driver/probe observations | Replacement identity and old artifact ownership |

### Gate G-104 — fresh VM replacement identity

- **Existing evidence:** current `WorkloadLifecycle` → same-ID
  `RestartAllocation` → action-shim → `VmDriver` path and the seed-257205/
  #284 native traces establish the reachable alias.
- **Owner:** `WorkloadLifecycle` owns the stable WorkloadId partition and
  replacement decision. The action-shim owns awaited effect sequencing,
  `VmDriver` owns the physical capability, and ObservationStore owns row
  acceptance.
- **Promise:** a VM replacement's ID is written to the existing View issuance
  ledger before it crosses the `StartAllocation` dispatch boundary and before
  its network, run-directory, beacon, cgroup, clone or VMM effect. The accepted
  fresh row becomes current; a rejected publication leaves only the issuance
  reservation, and the old row remains current/history.
- **Failure projection:** a fresh driver/provision rejection writes the existing
  typed `Failed` row under the fresh ID and leaves the predecessor row alone.
  A rejected Running publication uses the existing awaited fresh-start unwind;
  its View reservation survives, so the requeued/next-boot evaluation skips
  that ID. A stale old VM session fails the existing VM-only claim/witness check
  and cannot author the fresh row. Existing typed cleanup errors retain their
  current ownership and propagation.
- **Explicitly unaffected:** Exec same-ID restart; Job natural exit; intentional
  stops; Service Stable/readiness/liveness; Platform Reclamation versus
  Artifact Disposal; `Driver`/`Vmm`/`VmHostState` signatures; ObservationStore
  schema; NetSlotAllocator/TAP ownership.
- **Ordering:** choose the old current row and candidate-keyed policy; complete
  the existing running-origin stop when applicable; choose a suffix above all
  retained rows and View reservations; put the fresh VM reservation and carried
  policy inputs in `next_view`; fsync that View; only then dispatch
  `StartAllocation`, provision the existing network channel, take the driver
  claim before VM-exclusive artifacts, publish the fresh Running row, and run
  existing hooks. Old cleanup may finish before or after fresh creation,
  because its capability and all VM path names are old-ID scoped.
- **Counterexample:** if old cleanup removes an old run directory after the
  fresh VMM is created, the new directory, clone/index and cgroup remain. If
  fresh creation rejects, its cleanup cannot target the predecessor.
- **Evidence lane:** seeded `overdrive-sim` through registered production
  owners plus pure ID/path/owner-memory properties; qualified x86_64 metal
  Tier-3 Cloud Hypervisor execution with strace for bind/unlink/remove and
  final artifact complements. No black-box expectation is a substitute for
  the internal invariant.

### Boundary obligations

1. Available: accepted VM replacement has a fresh ID and current projection
   selects its row.
2. Unavailable/rejected: a View-persistence failure dispatches nothing. After
   dispatch, fresh failure is typed under the fresh ID; if Running publication
   rejects, awaited cleanup completes and the durable reservation prevents ID
   reuse while the predecessor row remains current. Candidate-keyed budget and
   backoff handle the next decision.
3. Unrelated state: Exec same-ID, Job natural exit and Service health gates
   retain their existing behavior. Initial VM keeps its existing
   `StartAllocation` action/effects while joining the pre-dispatch reservation
   rule.
4. Late success: an old watcher/reaper can only mutate old capability-owned
   artifacts and old row identity; it cannot remove or signal fresh artifacts.
5. Disconnect/reconnect: the VM-only accepted-session `Weak<BeaconWriter>` and
   claim guard remain the boundary; no reconnect protocol is added.
6. Disabled: there is no feature flag. Only automatic VM replacement changes
   action selection; initial VM keeps its existing StartAllocation/effect path
   plus the common VM reservation, and all Exec paths remain unchanged.

## Alternatives considered

### A — Selected: existing StartAllocation with fresh VM ID

It reuses the already-running fresh-placement path, preserves the accepted
public/internal action shape, makes every VM path distinct before creation and
does not add an owner or persistence system. Existing Exec same-ID restart is
left untouched.

### B — Keep stable AllocationId and add VmIncarnationId — rejected

This preserves the conflated execution identity under the allocation key and
adds another identity that every VM artifact, watcher, cleanup owner and
observation consumer would have to carry. #284 explicitly prohibits it; a
fresh `AllocationId` already is the execution boundary.

### C — Persist `workloads/<id>/current` plus revision/retention lineage — rejected

That is the full #180 revision model, including a new system-of-record pointer,
retention and migration obligations. #284 needs same-spec physical execution
separation, which retained allocation rows plus `current_alloc` already provide.
Pulling the revision model forward would be an unreviewed persistence change.

### D — Barrier, sleep, retry, blind unlink or global lock — rejected

The two failures are opposite legal interleavings. Timing, retries and a global
lock do not give an artifact an owner; blind unlink can remove a live owner's
pathname. The issue explicitly rejects all five and the existing capability
contract makes them unnecessary.

## Consequences

Positive:

- VM replacement paths no longer alias predecessor run directories, sockets,
  cgroups, clones or VMM cleanup identities.
- Old terminal rows and bounded history remain available; current selection is
  deterministic and no new persistence model is needed.
- Existing port/action shapes, Exec semantics, claim lifecycle and network
  ownership remain intact.
- The already-durable View supplies the pre-dispatch issued-ID ledger, so a
  rejected first publication or process restart cannot cause identity reuse.

Negative:

- A workload describe response contains one retained row per physical VM
  attempt; the per-allocation `restart_count` is not a workload aggregate.
- `WorkloadLifecycleView.restart_counts` now has explicit driver-specific
  semantics: VM key presence is also an issued-ID reservation, while its value
  carries the candidate's Workload Failure budget. This is a documented
  persistence-semantic amendment despite no field/codec change.
- Native artifact ownership evidence remains a DELIVER/Tier-3 obligation; the
  current Lima simulation cannot prove Unix socket or filesystem effects.

## Quality attribute impact

| Attribute | Impact |
|---|---|
| Reliability / recoverability | Disjoint execution IDs make delayed cleanup safe in both native failure directions; old rows remain available for diagnosis. |
| Integrity / least privilege | Existing move-only capabilities and exact allocation-key cleanup remain the only mutation authority; no broad unlink or global lock is added. |
| Maintainability / testability | The design reuses `StartAllocation`, `current_alloc`, existing ports and Sim bindings, keeping the change in one existing owner seam. |
| Performance | Fresh-ID derivation is pure and adds no wait, retry or serialization. Existing driver/action deadlines are unchanged. |
| Observability | Each physical attempt retains its own row and bounded occurrence history; no derived cross-allocation counter is persisted. |

## Changed Assumptions

| Superseded wording | Accepted amendment |
|---|---|
| `brief.md` Service-kind VM section: “The automatic WorkloadLifecycle restart starts a replacement allocation attempt under the same allocation id, not a fresh allocation identity.” | For `WorkloadDriver::Vm(_)`, automatic replacement emits existing `StartAllocation` with a fresh `AllocationId`; Exec remains same-ID. |
| ADR-0073 context: “crash-restart (`RestartAllocation`) reuses the alloc-id/slot” | The explicit generation restart remains fresh-ID; this ADR extends the fresh identity to automatic VM replacement. |
| ADR-0083/0089 VM reclamation recovery examples using same-ID `RestartAllocation` | Reclamation still owns the old terminal row and wake; VM re-drive uses fresh `StartAllocation`, while the existing reclamation claim/disposal and network semantics remain. |
| ADR-0083 D6: “the restart branch, on a reclaimed row, **writes no View field at all**” | VM reclamation still increments no failure budget and stamps no failure time, but it inserts/carries the fresh VM issued-ID reservation in the existing View before dispatch. Exec and ending-class semantics are unchanged. |
| ADR-0102 retry eligibility: “Do not scan unrelated historical View entries” and `last_failure_seen_at[candidate] + backoff_for_attempt(restart_counts[candidate])` | Retry policy remains candidate-keyed exactly as written. VM identity selection alone scans the existing View's issued-ID keys so an unpublished-but-executed ID cannot be reused; it never folds historical values into the candidate's deadline. |

The original statements remain historical evidence for the prior behavior and
remain applicable to Exec where explicitly stated. They are not current VM
replacement behavior after this ADR is accepted and implemented.

## References

- [GH #284](https://github.com/overdrive-sh/overdrive/issues/284), including
  comments (empty at DESIGN time)
- ADR-0073, ADR-0077, ADR-0078, ADR-0081, ADR-0082, ADR-0083, ADR-0089,
  ADR-0098, ADR-0099, ADR-0100, ADR-0101, ADR-0102, ADR-0103
- `crates/overdrive-reconcilers/src/workload_lifecycle.rs`
- `crates/overdrive-control-plane/src/action_shim/mod.rs`
- `crates/overdrive-control-plane/tests/acceptance/action_shim_running_write_failure_stops_alloc.rs`
- `crates/overdrive-worker/src/vm_driver.rs`
- `crates/overdrive-host/src/vmm.rs`
- `crates/overdrive-host/src/vm_host_state.rs`
- `crates/overdrive-core/src/vm/config.rs`
- `crates/overdrive-sim/tests/e10_vm_early_exit_spike.rs`
