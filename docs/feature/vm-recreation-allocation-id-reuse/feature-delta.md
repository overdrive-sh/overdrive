# Feature Delta — `vm-recreation-allocation-id-reuse`

**Feature ID:** `vm-recreation-allocation-id-reuse`

**Issue:** [GH #284](https://github.com/overdrive-sh/overdrive/issues/284)

**Wave:** corrective re-DISTILL

**Scope:** application/components

**Date:** 2026-09-13

## Status and authority

The exact corrective DESIGN in this document was **explicitly ratified by the
user in conversation on 2026-09-13**, including P-105-1 through P-105-7 and the
selected alternatives P-105-4A, P-105-5A and P-105-6A. Independent review
iteration 2 returned `CHANGES_REQUESTED`; F-03 through F-06 were remediated
without changing that ratified mechanism. The user's subsequent explicit
instruction to perform this corrective re-DISTILL is the disposition that
accepts those bounded remediations and authorizes executable acceptance
specification. It does not authorize PR #292, a roadmap, or DELIVER.

This corrective DESIGN supersedes the VM-only replacement boundary in
[ADR-0104](../../product/architecture/adr-0104-vm-recreation-fresh-allocation-identity.md).
ADR-0104 remains the historical record for the reproduced #284 host-artifact
alias and for the rejected PR #292 implementation. The active architectural
choices are:

- [ADR-0105](../../product/architecture/adr-0105-driver-neutral-allocation-replacement-identity.md):
  one physical execution per `AllocationId`, independent of driver;
- [ADR-0106](../../product/architecture/adr-0106-successor-creation-does-not-wait-for-predecessor-cleanup.md):
  successor creation does not wait for predecessor cleanup;
- [ADR-0108](../../product/architecture/adr-0108-durable-reservation-consumes-successor-allocation-identity.md):
  durable reservation consumes the successor identity; and
- [ADR-0109](../../product/architecture/adr-0109-replacement-requires-terminal-predecessor-handoff.md):
  replacement requires the ratified terminal predecessor handoff.

[ADR-0107](../../product/architecture/adr-0107-successor-identity-consumption-follows-successor-owned-effect.md)
is withdrawn before acceptance because P-105-5A selected the opposite
reservation boundary.

PR [#292](https://github.com/overdrive-sh/overdrive/pull/292) remains **not
mergeable** until this re-DISTILL receives independent acceptance review and
the corrected contract is re-DELIVERed. The local baseline has restored the
pre-delivery same-ID implementation; the reverted PR code and VM/Exec-split
tests are historical rejected input, not the model to preserve.

Detailed topology lives only in the architecture
[C4 diagrams](../../product/architecture/c4-diagrams.md#driver-neutral-allocation-replacement-gh-284-corrective-proposal).
The DESIGN sections below contain no test mechanics or DELIVER roadmap; the
corrective DISTILL sections appended to this document own executable scenario
placement, pending markers, and RED evidence.

## DESIGN [REF] — conversational ratification record

The user approved the following complete bundle on 2026-09-13. These clauses
are normative here. The later explicit re-DISTILL instruction accepts the
bounded post-review documentation remediations without requesting or inventing
a third DESIGN review cycle.

| Decision | Ratified contract |
|---|---|
| **P-105-1 — allocation attempt selection** | Use one checked max-suffix allocator across accepted rows and durably issued View keys for initial placement, generation replacement, Workload Failure and Platform Reclamation. Gaps are valid; exhaustion at `u32::MAX` emits no allocation action. |
| **P-105-2 — replacement action construction** | Keep the existing public `RestartAllocation` variant. A private helper always constructs predecessor `alloc_id` plus fresh successor `spec.alloc`; no driver policy branch or new public API. |
| **P-105-3 — View meanings and carry** | Existing `restart_counts` keys are the durable issued-ID ledger and carry the stable workload's Workload Failure budget at each candidate. `last_failure_seen_at` records only genuine Workload Failure time and follows the current candidate. Reclamation and generation carry policy without charging it. |
| **P-105-4A — lifecycle handoff** | Replacement may be emitted only for the numeric-current accepted row in `Failed` or `Terminated`; `Draining` is not a sufficient handoff. The action shim re-reads and fences the predecessor before mutation. |
| **P-105-5A — identity consumption** | The runtime's durable View fsync consumes the successor ID before dispatch. A dispatch that never reaches a successor effect may leave a gap; the ID is never reused. ADR-0107 is withdrawn. |
| **P-105-6A — effect and failure ordering** | The successor proceeds first. After every completed successor outcome, the action shim makes one exact-old predecessor cleanup attempt. A successor error remains primary if both sides fail; otherwise a predecessor cleanup failure returns its existing typed error. There is no generic retry, detached task or new cleanup owner. |
| **P-105-7 — publication and history** | Successor lifecycle is written at its fresh key; predecessor row and occurrences remain unchanged. Only an accepted successor `Running` row releases Running-confirmed hooks. Rejected or failed publication follows existing successor unwind semantics and never proposes a second identity from the shim. |

## DESIGN [REF] — revalidated premise and production owner path

The corrective model follows current production ownership, not ADR-0104 prose
or PR #292's tests.

| Boundary | Observed code fact | Design consequence |
|---|---|---|
| Stable logical owner | The production convergence target and `WorkloadLifecycleState` are keyed by `WorkloadId`; actual hydration supplies all retained `AllocStatusRow`s for that workload. | `WorkloadId` remains the stable intent/policy owner. No instance aggregate or new public identity is introduced. |
| Physical execution identity | `AllocationSpec.alloc` is consumed by both Exec and VM adapters and keys execution state. VM derives its run directory, cgroup, beacon, rootfs and supervision capability from it; Exec derives its cgroup and live-allocation state from it. | One physical execution attempt must receive one `AllocationId` regardless of adapter. |
| Current corrective implementation | The unmerged branch in PR #292 makes `WorkloadLifecycle` branch on `WorkloadDriver::Vm(_)`: VM replacement emits fresh `StartAllocation`, while Exec emits same-ID `RestartAllocation`. | That driver discriminator is rejected as an application-policy boundary and must not be preserved. Driver matching remains only payload projection inside `AllocationSpec`. |
| Existing public action | `Action::RestartAllocation { alloc_id, spec, kind }` already carries two allocation identities, while `StartAllocation { alloc_id, workload_id, node_id, spec, kind }` carries one. | The existing public action surface is sufficient for a driver-neutral predecessor-to-fresh-successor transition. No new variant, field, port or driver method is required. |
| Durable action owner | The convergence runtime fsyncs the reconciler's returned View before it dispatches actions, then awaits dispatch and requeues under existing semantics. | The existing `WorkloadLifecycleView` can durably consume a successor ID before dispatch without a new store or protocol. |
| Replacement effect owner | The `RestartAllocation` action-shim arm already owns predecessor lookup, driver routing, mTLS lifecycle, structural network allocation/teardown, driver start, observation publication, supervision and Running-confirmed hooks. | The existing shim remains the single sequential owner; it needs a private control-flow reorder, not a public cleanup API or detached executor. |
| Observation ownership | `ObservationStore::write_alloc_lifecycle` accepts lifecycle state by `AllocationId` and retains bounded occurrences at that key. | A fresh successor key preserves predecessor history instead of overwriting it. |
| Exact-old VM residue | `VmDriver`, `VmReclamation` and `VmHostState` already carry exact allocation-keyed capabilities for stop, natural-exit cleanup and later residue disposal. | The generic replacement decision does not absorb driver-specific cleanup and does not redesign reclamation. |

GH #284's native `EADDRINUSE` and late-cleanup `ENOENT` captures prove the VM
artifact alias under reused identity. Greptile's separate claim that PR #292
necessarily leaks structural network resources to exhaustion remains an
untrusted hypothesis: no repository-required failing seeded production-owner
invariant or bounded real-host regression has demonstrated that outcome. It
therefore authorizes no network, retry, persistence or cleanup subsystem
change.

The production driving path remains operator → `overdrive` CLI → `overdrive
serve` control-plane HTTP handler → `IntentStore` → convergence hydration and
runtime. The CLI does not write `IntentStore` directly.

## DESIGN [REF] — exact identity and action contract

### Logical owner and physical execution

- `WorkloadId` names the stable declared workload and owns desired generation,
  Workload Failure budget and replacement policy.
- `AllocationId` names exactly one physical execution attempt and all effects
  derived from that attempt.
- Every automatic replacement receives a distinct successor `AllocationId`.
  This applies to the common microVM execution family and, temporarily, legacy
  Exec.
- Exec's eventual removal is owned separately by
  [GH #293](https://github.com/overdrive-sh/overdrive/issues/293). This pass
  neither deletes Exec nor creates a permanent compatibility branch. Future
  unikernel-in-microVM and sandbox execution adapters inherit the common
  physical-allocation rule without a Cloud-Hypervisor or
  `WorkloadDriver::Vm` policy check in `WorkloadLifecycle`.

### Existing public actions

The public action shapes do not change:

```rust
StartAllocation {
    alloc_id: AllocationId,
    workload_id: WorkloadId,
    node_id: NodeId,
    spec: AllocationSpec,
    kind: WorkloadKind,
}

RestartAllocation {
    alloc_id: AllocationId,
    spec: AllocationSpec,
    kind: WorkloadKind,
}
```

Their exact meanings are:

- `StartAllocation`: initial placement, or a fresh placement for which no
  **eligible replacement predecessor** exists, with
  `alloc_id == spec.alloc`. A production-reachable SystemGc resubmit retains an
  accepted historical SystemGc row but that intentional-stop row is excluded
  by the existing replacement-cause gate, so resubmit continues to use
  `StartAllocation`.
- `RestartAllocation`: every replacement that has an **eligible replacement
  predecessor** after current-row, state, cause, stop, budget and backoff gates,
  including eligible desired-generation, Workload Failure and Platform
  Reclamation replacement, with `alloc_id` naming that predecessor and
  `spec.alloc` naming the fresh successor; `alloc_id != spec.alloc` is
  mandatory.
- For either action, `spec.identity ==
  SpiffeId::for_allocation(&workload_id, &spec.alloc)`. For restart, the
  `workload_id` is the stable ID recovered from the selected predecessor and
  already used by `WorkloadLifecycle` to build the successor spec.
- `kind` retains its existing meaning. `RestartAllocation` remains a
  non-terminal action and gains no cause, cleanup or policy field.

`WorkloadLifecycle` is the sole producer of the predecessor/successor pair and
the sole restart/finalize authority. A `WorkloadDriver` match is permitted only
while projecting the already-selected successor into the existing
`DriverPayload`; it may not select identity, action family, handoff, budget or
cleanup policy.

## DESIGN [REF] — exact `WorkloadLifecycle` contract

### Attempt allocation

The existing private suffix parser remains the authority for the
`alloc-<workload>-<N>` attempt component. Replace the VM-only allocator with
this exact private shape:

```rust
fn next_allocation_attempt(
    allocs: &[&AllocStatusRow],
    view: &WorkloadLifecycleView,
) -> Option<u32>
```

It returns:

1. the checked successor of the greatest parseable `u32` suffix found in the
   union of accepted `allocs` and `view.restart_counts.keys()`;
2. `Some(0)` when neither source has a parseable suffix; or
3. `None` when the greatest suffix is `u32::MAX`.

Rows or View keys with an unparseable suffix do not participate in the maximum.
`None` produces no allocation action and leaves the View unchanged. Unused
numeric gaps are valid and must not be filled. The same allocator is used for
initial placement, desired-generation placement, Workload Failure replacement
and Platform Reclamation replacement for every driver.

`current_alloc` remains an observation projection: it selects the accepted row
with the greatest parseable numeric suffix. A View-only reservation is never
current. Retry deadlines, handoff and successor policy read only the numeric-
current accepted row; historical rows and reservations cannot become a
lifecycle candidate.

### Replacement action construction

Use this exact private helper shape:

```rust
fn restart_allocation_action(
    job: &Job,
    desired: &WorkloadLifecycleState,
    predecessor: &AllocStatusRow,
    successor_alloc_id: AllocationId,
) -> Action
```

It always returns `Action::RestartAllocation` with:

- `alloc_id = predecessor.alloc_id`;
- `spec.alloc = successor_alloc_id`;
- `spec.identity = SpiffeId::for_allocation(&job.id,
  &successor_alloc_id)`;
- `spec.driver` projected from `job.driver` using the existing Exec/VM payload
  construction only; and
- all resources, probes, service ports, kind and unset runtime network fields
  preserved exactly as the existing allocation-spec builder supplies them.

The helper contains no `WorkloadDriver::Vm` action or identity branch and does
not delegate policy to `DriverRegistry` or a driver port.

### Durable reservation and existing View fields

No persisted field or schema is added. The existing fields retain these exact
types and receive driver-neutral meanings:

```rust
restart_counts: BTreeMap<AllocationId, u32>
last_failure_seen_at: BTreeMap<AllocationId, UnixInstant>
```

The returned `next_view` is fsynced by the existing convergence runtime before
the action is dispatched. At that fsync:

- inserting `successor_alloc_id` into `restart_counts` consumes that physical
  identity permanently for the workload, whether or not dispatch later reads
  the predecessor, begins a successor effect or publishes a row;
- every retained `restart_counts` key is therefore an issued-ID ledger entry;
  its value is the stable workload's Workload Failure count carried at that
  candidate;
- a later attempt scans all retained issued keys, so a failed prelaunch
  dispatch leaves a valid gap and the next successor uses a higher suffix;
- `last_failure_seen_at` is policy input, not an issued-ID ledger. A value
  records the observation time of a genuine Workload Failure only, and only
  the numeric-current candidate's entry participates in a deadline; and
- reservation does not itself change any accepted row or make the successor
  current.

The policy carried into each durable successor reservation is:

| Creation cause | Successor `restart_counts` value | Successor `last_failure_seen_at` |
|---|---|---|
| Initial placement | `0` | absent |
| Workload Failure | predecessor/current budget plus one, using the existing bounded increment and ceiling semantics | `tick.now_unix` |
| Platform Reclamation | predecessor/current budget unchanged | carry the predecessor/current timestamp if present; never stamp a reclamation time |
| Desired-generation replacement | predecessor/current budget unchanged | carry the predecessor/current timestamp if present; never stamp a generation time |

Historical `restart_counts` entries stay present because their keys prove
consumed identity. Budget and backoff decisions read the numeric-current
candidate's values only; they never sum historical entries. Existing stop,
deletion, steady-state, ceiling, wake-up and View serialization semantics are
otherwise unchanged.

### Lifecycle handoff

A replacement action may be emitted only when all of the following are true:

1. the selected predecessor is `current_alloc(accepted_rows)`;
2. its accepted state is `Failed` or `Terminated`;
3. existing intent, generation, terminal-cause, stop, budget, backoff,
   placement and capability gates permit replacement; and
4. a fresh successor attempt is available and durably reserved in
   `next_view`.

`Draining` is not a sufficient ownership handoff and emits no replacement.
This narrows only the replacement-emission predicate; it does not introduce a
new lifecycle state or alter the owners that write `Draining`, `Failed` or
`Terminated`.

## DESIGN [REF] — exact action-shim ordering and failure contract

The existing `RestartAllocation` arm remains the only effect owner. It must
use the two action identities without conflating them:

- predecessor-only: row lookup/fence, prior driver resolution, predecessor
  `AllocationHandle`, driver stop, mTLS stop and structural-network teardown;
- successor-only: network-slot assignment/provisioning, injected network
  fields, SVID, driver lookup/start, lifecycle row, unwind, driver index,
  supervision and Running-confirmed hooks.

The exact order is:

1. Re-read `alloc_id` from `ObservationStore` and fence it. Continue only when
   the row still represents the selected predecessor and its accepted state is
   `Failed` or `Terminated`; preserve existing no-op/error distinctions for a
   stale transition or missing handle.
2. Resolve/capture the predecessor's existing driver capability and immutable
   workload/node facts before successor work. Do not clean it yet.
3. Determine one `successor_outcome` by running the existing successor path
   entirely under `spec.alloc`: network provision, SVID ensure, `Driver::start`,
   existing Running-or-StartRejected publication, accepted-Running driver
   index insertion, supervision release where already required, and
   Running-confirmed hooks only after accepted Running.
4. After that successor outcome completes, make exactly one ordered exact-old
   cleanup attempt: predecessor `Driver::stop` (preserving existing
   `NotFound` absence proof and driver-resolution behavior), predecessor
   `MtlsInterceptLifecycle::stop_alloc`, then predecessor structural-network
   teardown and slot release.
5. Resolve the two outcomes using the precedence table below.

This ordering does not wait for predecessor cleanup before successor creation.
It does await the one post-successor cleanup attempt before the action returns;
there is no detached task, generic retry, persisted cleanup queue, new owner or
new public/private policy port.

| Successor outcome | Predecessor cleanup outcome | Returned result |
|---|---|---|
| success | success | existing successor result |
| success | failure | the existing typed `ShimError` conversion for the failing driver, mTLS or structural-network operation |
| error | success | successor error |
| error | failure | successor error remains primary; report cleanup failure as secondary through existing structured tracing |

The cleanup sequence retains the existing fail-closed ordering within an
attempt: a non-`NotFound` driver-stop error prevents mTLS/network teardown, and
an mTLS-stop error prevents structural-network teardown. This pass does not
weaken those protection dependencies.

An accepted successor is never rolled back because predecessor cleanup failed.
The predecessor's `AllocDriverIndex` entry is removed only after its complete
cleanup succeeds; it is retained when cleanup fails. The successor's index is
inserted only after a fresh `Running` row is accepted. Because the identities
are distinct, the two entries cannot overwrite each other. A predecessor that
still holds a network slot may cause the successor to receive a different slot
or the existing typed `NetSlotExhausted`; no slot, retry or networking contract
changes here.

### Successor unwind and predecessor cleanup are distinct

Any existing successor-owned unwind caused by failed provisioning, SVID,
driver start or publication addresses only `spec.alloc`. It completes as part
of `successor_outcome`. The one predecessor cleanup attempt then addresses only
`alloc_id`. Neither path may redirect through “current workload allocation” or
the other identity.

## DESIGN [REF] — publication and history contract

- The successor lifecycle row is written under `spec.alloc`, with stable
  `workload_id` and `node_id` copied from the predecessor.
- The predecessor row and its occurrence history are never rewritten as the
  successor outcome.
- Because the successor key is fresh, no prior row is supplied to
  `build_alloc_status_row`. An accepted Running successor therefore starts its
  per-allocation history with `restart_count = 0` and
  `last_terminated = None`.
- Existing `DriverError::StartRejected` handling publishes `Failed` under the
  fresh successor key with the existing typed reason/detail mapping.
- Any accepted successor row, including `Failed`, becomes eligible to be the
  later numeric-current row because current selection is by accepted numeric
  suffix, not lifecycle success.
- Only `write_alloc_lifecycle(Running, ...) == Ok(Some(_))` permits the
  existing Running-confirmed hooks and successor `AllocDriverIndex` entry.
- A fresh-key Running write returning `Ok(None)` performs the existing complete
  successor unwind, returns `Ok(())`, publishes no row and does not ask
  `WorkloadLifecycle` for a second action from inside the shim.
- A fresh-key Running write returning `Err(error)` performs the same successor
  unwind and retains that typed observation error as the successor outcome.
- A later convergence evaluation may select a higher identity because the
  fsynced View reservation already consumed this one. This is ordinary
  re-evaluation, not a shim retry or an ADR-0099 second proposal.

Existing occurrence bounds, logical timestamps, transition sources, terminal
cause classification, StartRejected mapping, hook semantics and runtime
requeue behavior remain unchanged unless the clauses above necessarily change
the allocation key they address.

## DESIGN [REF] — DDD, components and ports

### DDD decisions

| Concept | Owner and invariant |
|---|---|
| Logical workload | `WorkloadId` is the stable aggregate identity for declared intent, desired generation and workload-level retry policy. |
| Physical allocation | `AllocationId` is an entity identity for one attempted execution. It is never an incarnation counter shared by two executions. |
| Replacement | Application transition from one accepted predecessor allocation to one durably reserved fresh successor allocation. It is not a driver operation or new aggregate. |
| Issued identity | A successor key present in the fsynced `WorkloadLifecycleView.restart_counts`, whether or not an observation row was later accepted. |
| Accepted allocation | A row accepted by `ObservationStore`; only accepted rows participate in `current_alloc`. |
| Artifact cleanup | Driver/mTLS/network effects addressed by exact physical identity behind existing driven ports. Cleanup does not choose replacement identity. |

No new bounded context, aggregate, repository, domain event, lifecycle state or
identity type is introduced.

### Component and reuse analysis

| Component | Treatment | Corrective responsibility |
|---|---|---|
| `WorkloadLifecycle` | **Extend privately** | Select numeric-current accepted predecessor, apply generic gates, allocate/reserve fresh identity, carry policy, emit existing `RestartAllocation`. |
| Convergence runtime / ViewStore | **Reuse as-is** | Fsync returned View before awaited dispatch and preserve existing requeue behavior. |
| `Action::StartAllocation` / `RestartAllocation` | **Reuse public surface as-is** | Start retains one-identity meaning; Restart carries predecessor plus fresh successor. |
| Action shim | **Reorder private control flow** | Fence predecessor, complete successor outcome, then attempt exact-old cleanup once with pinned precedence. |
| `DriverRegistry` / `Driver` | **Reuse as-is** | Route supplied payload and execute start/stop under supplied allocation identities; no replacement-policy method. |
| `ExecDriver` | **Reuse under generic contract** | Temporary compatibility adapter uses fresh physical identity until separate GH #293 removal. |
| `VmDriver` / `Vmm` | **Reuse as-is** | Create, supervise and clean microVM effects under the supplied exact identity. |
| `MtlsInterceptLifecycle` | **Reuse as-is** | Start/stop interception under the supplied successor/predecessor identity. |
| `NetSlotAllocator` / `WorkloadNetworkProvisioner` | **Reuse as-is** | Provision successor and tear down predecessor by exact allocation ID. |
| `ObservationStore` | **Reuse as-is** | Accept separate predecessor/successor rows and occurrences. |
| `AllocDriverIndex` | **Reuse with two-key ordering** | Retain old routing capability until cleanup success; add new only after accepted Running. |
| `VmReclamation` / `VmHostState` | **Reuse as-is** | Preserve independent exact-old VM residue disposal; no generic cleanup ownership moves here. |

Created components, actions, ports, stores, schemas and external integrations:
**none**.

### Driving and driven ports

The external driving boundary is unchanged: CLI commands call the existing
control-plane HTTP API, whose handlers commit intent; the convergence runtime
then drives `WorkloadLifecycle` and dispatches its actions. Neither CLI nor
drivers drive allocation-identity policy.

The existing driven ports remain effect-only boundaries:

- `ObservationStore` reads/fences predecessor and publishes successor;
- `Driver` starts/stops the exact allocation supplied;
- `MtlsInterceptLifecycle` manages interception for the exact allocation;
- network allocator/provisioner manage slot/netns/veth/TAP state for the exact
  allocation; and
- `VmHostState` observes/disposes exact VM residue.

No port gains a replacement-policy, identity-selection, cleanup-scheduling or
retry method.

## DESIGN [REF] — Lifecycle Gate Ownership

### Existing state-ownership matrix

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| Declared workload intent / desired generation | `overdrive serve` workload HTTP handler committing through `IntentStore` | The control plane has durably accepted the requested workload intent/generation. | Existing request validation and intent transaction only. | Physical allocation `Running`, Service `Stable`, probe health and predecessor cleanup. |
| Accepted allocation lifecycle row | Action shim or existing exit observer writing through `ObservationStore` | The named physical `AllocationId` reached the accepted state recorded at its LWW key. | Existing driver/observer evidence and observation acceptance. | A different allocation key, desired intent/generation, Service `Stable` or probe health. |
| Current physical allocation projection | `WorkloadLifecycle` | The selected accepted row is the workload's numeric-current physical attempt. | Accepted allocation rows only; View reservations are excluded. | Driver start/stop outcome, observation acceptance and Service health. |
| Driver start result | Selected `Driver` interpreted by the action shim | The supplied physical allocation either started or produced an existing typed failure/rejection. | Existing driver capability/probe and supplied `AllocationSpec`. | Replacement identity selection, workload retry policy and predecessor cleanup policy. |
| Allocation `Running` hook release | Action shim | A fresh `Running` row was accepted for the successor before watcher/service hooks proceed. | `ObservationStore::write_alloc_lifecycle == Ok(Some(_))`. | Service `Stable`, readiness/liveness health and old cleanup completion. |
| Service `Stable` / health | Existing `ServiceLifecycle` / `ProbeRunner` owners | Existing Service startup/readiness/liveness promises. | Existing accepted Running allocation and probe observations. | Allocation identity reservation, predecessor cleanup and legacy Exec compatibility. |
| Exact allocation cleanup | `Driver`, `MtlsInterceptLifecycle` and network ports sequenced by the action shim; `VmReclamation`/`VmHostState` for their existing residue scope | Effects belonging to the supplied exact `AllocationId` have completed or returned their existing typed failure. | Existing exact capability/index, absence proof and port result. | Successor identity choice, an accepted successor row, Service health, or unrelated allocations. |

### Gate G-1 — eligible terminal predecessor handoff

- **Existing evidence:** the current production path is allocation/exit or
  stop observation → `ObservationStore` → convergence runtime hydration →
  `WorkloadLifecycle::reconcile` → replacement action. Current
  `is_restartable` in
  `crates/overdrive-reconcilers/src/workload_lifecycle.rs` admits `Draining`;
  accepted ADR-0073 owns numeric-current/SystemGc-resubmit behavior and
  accepted ADR-0102 owns candidate-keyed timing. P-105-4A deliberately narrows
  the replacement handoff while preserving the existing cause,
  intentional-stop, budget and backoff gates. ADR-0109 owns that one lifecycle
  decision.
- **Owner:** `WorkloadLifecycle` selects the accepted numeric-current row and
  decides whether to emit replacement; the action shim re-reads the exact row
  as the mutation-boundary fence.
- **Promise:** when the gate passes, an eligible accepted predecessor exists,
  is numeric-current, has state `Failed` or `Terminated`, and has passed the
  existing stop/cause/budget/backoff rules. `Draining` does not establish
  ownership handoff.
- **Affected state/result:** only emission of `RestartAllocation` versus no
  allocation action for that evaluation.
- **Failure projection:** absent current row, an ineligible intentional-stop
  row, `Draining`, exhausted budget, active backoff, placement/capability miss,
  or unavailable successor suffix preserves each existing no-action/finalize
  result. A missing predecessor at shim re-read retains existing
  `HandleMissing`; an already-superseded/stale transition retains the existing
  no-op fence. The pure gate adds no timeout, retry or error type.
- **Explicitly unaffected:** initial placement; SystemGc resubmit via
  `StartAllocation`; Job natural-exit finalization; operator stop; Service
  startup/readiness/liveness and `Stable`; observation occurrence retention;
  driver-specific start/stop behavior; and VM reclamation.
- **Ordering:** intent/actual hydration and current-row selection occur before
  suffix reservation and action emission; the shim fence occurs before any
  successor or predecessor effect. The gate consumes no timeout budget.
- **Counterexample:** allowing a numeric-current `Draining` row to pass could
  start a successor before the existing ending owner has published the
  ratified handoff state; allowing any historical `Failed` row to pass could
  replace a newer accepted allocation.
- **Evidence lane:** re-DISTILL must map the state/cause partitions through the
  public `WorkloadLifecycle` reconciler boundary using pure/port-level evidence
  and the production hydration composition. No concrete fixture or runner is
  specified in DESIGN.

### Gate G-2 — durable successor identity consumption

- **Existing evidence:** the production convergence path is
  `WorkloadLifecycle::reconcile` → returned `next_view` → runtime ViewStore
  fsync → awaited action dispatch in
  `crates/overdrive-control-plane/src/reconciler_runtime.rs`, preserving
  accepted ADR-0035's reconciler-View ownership. Dispatch errors are requeued
  after View persistence; restart/reopen rehydrates the persisted maps.
  ADR-0108 owns the consumption decision.
- **Owner:** `WorkloadLifecycle` selects/inserts the successor key; the existing
  convergence runtime owns durable View persistence and may dispatch only
  after that persistence succeeds.
- **Promise:** when the fsync gate passes, `successor_alloc_id` is permanently
  consumed for that workload and later allocation scans select above it,
  regardless of accepted-row or launch outcome.
- **Affected state/result:** the issued-ID ledger in
  `WorkloadLifecycleView.restart_counts` and permission to dispatch the already
  returned action.
- **Failure projection:** a View encode/store/fsync failure preserves the
  runtime's existing typed failure and withholds dispatch. A dispatch failure,
  cancellation or process disconnect after successful fsync does not undo the
  key. Reopen rehydrates it; a later evaluation may leave a gap and select a
  higher suffix. Duplicate/re-driven evaluation cannot reissue the consumed
  suffix. Unparseable historical keys retain existing defensive treatment and
  are never created by the minting path.
- **Explicitly unaffected:** accepted allocation rows/current projection;
  predecessor handoff; successor driver result; predecessor cleanup; workload
  failure ceiling/backoff function; Service health; and intent generation.
- **Ordering:** select fresh suffix → return it in action and `next_view` →
  fsync View → dispatch. No new deadline is introduced and no host effect may
  precede successful View persistence.
- **Counterexample:** selecting only above accepted rows after a successful
  launch whose Running publication was rejected could reassign one physical
  identity to another execution.
- **Evidence lane:** re-DISTILL must use persistent-View integration evidence
  for fsync/reopen and a seeded simulation safety invariant for requeue/gap
  convergence. Concrete seeds, fixtures and commands belong to DISTILL.

### Gate G-3 — successor-first completion and post-successor exact-old cleanup

- **Existing evidence:** `dispatch_single` in
  `crates/overdrive-control-plane/src/action_shim/mod.rs` already awaits all
  successor and predecessor effects inside the `RestartAllocation` arm,
  preserving accepted ADR-0023's shim placement, ADR-0037's non-terminal
  restart meaning and ADR-0076's mTLS lifecycle boundary. The current arm
  orders old driver/mTLS/network cleanup before start; P-105-6A moves that
  existing cleanup sequence after the complete successor outcome and retains
  the existing port calls and typed `ShimError` conversions. ADR-0106 owns the
  non-gating cleanup choice.
- **Owner:** the action shim is the only sequencer and result arbiter. Drivers,
  mTLS and network adapters report exact-ID effects but do not choose ordering
  or precedence.
- **Promise:** successor start/publication/unwind completes under `spec.alloc`
  before exactly one predecessor cleanup attempt begins under `alloc_id`.
  Cleanup cannot delay successor creation or revoke an accepted successor.
- **Affected state/result:** action-shim return value, removal/retention of the
  predecessor `AllocDriverIndex`, and whether accepted-Running hooks are
  released for the successor.
- **Failure projection:** successor success plus cleanup failure returns the
  existing typed cleanup `ShimError`; successor error plus cleanup failure
  returns the successor error and emits the cleanup error as secondary
  structured tracing; successor `Ok(None)` or observation `Err` first performs
  existing successor-only unwind. Within the old cleanup attempt, non-
  `NotFound` driver failure short-circuits mTLS/network and mTLS failure
  short-circuits network. Existing port timeout/cancellation behavior and
  runtime requeue remain unchanged; no second cleanup attempt, detached task
  or retry owner is added.
- **Explicitly unaffected:** accepted successor `Running`/`Failed` rows and
  predecessor observation history; successor ID consumption; Workload Failure
  budget/backoff; Job/Service terminal classification; Service
  `Stable`/health; operator stop; SystemGc resubmit; and cleanup of unrelated
  allocation IDs.
- **Ordering:** predecessor re-read/fence and capability capture → complete
  successor outcome → one ordered old `Driver::stop` → old mTLS stop → old
  structural-network teardown/release → precedence selection → action return.
  The sequence adds no timeout and does not multiply any existing deadline.
- **Counterexample:** returning a cleanup error by rolling back an already
  accepted successor would make old-resource availability gate the new
  physical execution despite distinct identities; redirecting cleanup through
  the workload's current row could delete successor artifacts.
- **Evidence lane:** re-DISTILL must map the four successor/cleanup result
  combinations through the action-shim port boundary, use seeded simulation
  for lifecycle ordering/requeue, and reserve native-kernel/metal evidence for
  actual netns/veth/TAP/cgroup/socket/file nonmutation. No runner mechanics are
  fixed here.

### Boundary-obligation handoff to DISTILL

| Boundary | DESIGN obligation | Evidence lane to be authored in DISTILL |
|---|---|---|
| Available / ordinary | Eligible `Failed|Terminated` predecessor reserves a fresh identity, completes a successor outcome, then attempts exact-old cleanup. | Pure reconciler plus action-shim integration; real-host evidence only for kernel artifacts. |
| Unavailable / timeout | `Draining` emits no replacement; View persistence failure withholds dispatch; existing driver/mTLS/network failures retain the G-3 projections. No new timeout exists. | Pure partition evidence plus integration fault outcomes; existing port timeout semantics only. |
| Unrelated state | Job natural exit/finalize, Service health/`Stable`, operator stop, SystemGc resubmit, observation retention and unrelated allocations remain byte/behavior-equivalent outside exact IDs and ordering named here. | Port-to-port integration and seeded complement checks; do not broaden to unrelated hardening. |
| Late success / late old completion | Exact-old cleanup that completes after successor Running may mutate only predecessor artifacts/index and cannot delay, revoke or rewrite the successor. | Seeded ordering plus native-kernel/metal exact-artifact identity evidence. |
| Disconnect / reopen | Any successful View fsync survives runtime/process reopen; an unrowed consumed ID is not reused. No generic predecessor-cleanup replay is introduced. | Persistent ViewStore integration and seeded convergence/reopen evidence. |
| Duplicate / re-drive | View key scan chooses above every consumed suffix; accepted-row current selection ignores View-only reservations; action-shim does not propose a second action. | Pure allocator/current projection plus seeded re-drive evidence. |
| Feature disabled | Not applicable: this correction has no feature flag and applies to every composed driver, including legacy Exec until GH #293 removal. | A no-flag declaration; no artificial disabled fixture. |

The rows above define architecture-level observable boundaries, not executable
test cases. DISTILL owns scenario names, fixtures, parameters, seeds, mutation
targets and runner commands.

## DESIGN [REF] — changed and unchanged assumptions

Changed assumptions:

1. `RestartAllocation` is no longer same-ID; it is the generic
   predecessor-to-fresh-successor transition.
2. Fresh physical identity is not VM-specific and is never selected by
   `WorkloadDriver::Vm`.
3. Durable reservation itself consumes the successor identity; reaching a
   successor effect or accepted row is not required.
4. `Draining` does not authorize replacement; accepted `Failed` or
   `Terminated` does.
5. Successor creation precedes predecessor cleanup, while the same shim awaits
   one post-successor cleanup attempt and applies successor-error precedence.
6. Restart publication uses a fresh observation key and therefore starts fresh
   per-allocation crash history.
7. Legacy Exec follows the generic invariant until its separately authorized
   removal in GH #293.

Unchanged assumptions:

- one effective placement per workload, existing scheduler behavior and node
  selection;
- workload failure ceiling, backoff function, cause taxonomy and terminal
  lifecycle semantics;
- intent, View and observation persistence technologies and schemas;
- action dispatch, cancellation and convergence-runtime ownership;
- driver public API, mTLS public API, network allocation model and VM
  reclamation ownership;
- retry, retention, occurrence bounds, service health, networking, security
  and shutdown behavior outside the exact replacement ordering above; and
- no feature flag, migration protocol, compatibility action, new cleanup
  subsystem or infrastructure/system redesign.

## DESIGN [REF] — final review disposition

The exact P-105-1 through P-105-7 bundle is fully ratified; no substantive user
decision is unresolved. Independent review iteration 2, the user-capped final
cycle, closed F-01/F-02 and returned `CHANGES_REQUESTED` on F-03 through F-06.
This final architect pass records these bounded dispositions without changing
the approved mechanism:

- **F-03:** `StartAllocation` explicitly preserves SystemGc resubmit when no
  eligible replacement predecessor exists; retained history alone does not
  force `RestartAllocation`.
- **F-04:** the changed handoff, reservation and cleanup gates now carry the
  complete Lifecycle Gate Ownership declarations and architecture-level
  boundary/evidence-lane handoff.
- **F-05:** focused ADR-0109 records the independently reversible P-105-4A
  predecessor-handoff decision; exact predicates remain only in this feature
  delta.
- **F-06:** ADR-0106 now distinguishes cleanup-only typed failure from the
  both-fail successor-error/secondary-tracing outcome.

No iteration 3 was requested or authorized, and the remediation did not
self-approve or alter the final reviewer verdict. The user's later explicit
direction to accept the corrected DESIGN as re-DISTILL input closes that
governance blocker. The DISTILL handoff below still requires independent
acceptance review; PR #292 remains non-mergeable and no DELIVER authority is
created here.

## Wave: DISTILL / [REF] Consultation and reconciliation

This is a corrective re-DISTILL of an existing bug fix, not a new feature.
The repository is Rust-native (`Cargo.toml`). Repository rules override the
generic nWave examples: no `.feature` file, Gherkin runner, Python state-delta
port, production scaffold, built-binary expectation runner, roadmap, or
execution log is created. Complete Rust bodies use the existing public
`Reconciler`, convergence-runtime, action-shim, driver, observation, ViewStore,
mTLS, network, and host-adapter boundaries. No test requires a new production
API.

Documentation density resolves to the configured `lean` mode. The installed
distribution has no `scripts/shared/density_config.py` or
`scripts/shared/telemetry.py`; no replacement resolver, telemetry event, or
JSONL record was fabricated. Only the Tier-1 `[REF]` sections required for
handoff are emitted.

Prior-wave and authority reads completed before acceptance authorship:

- ✓ `AGENTS.md`, `CLAUDE.md`, and every mandatory rule in `.claude/rules/`.
- ✓ installed `nw-distill` and its referenced density-resolution contract.
- ✓ this complete canonical feature delta, including P-105-1 through P-105-7,
  G-1 through G-3, exact private signatures, outcome precedence, and changed
  assumptions.
- ✓ ADR-0104's corrective/historical status and active ADRs 0105, 0106, 0108,
  and 0109.
- ✓ `docs/product/architecture/brief.md` corrective and historical sections,
  plus the GH #284 corrective C4 context/container topology.
- ✓ `gh issue view 284 --comments` and `gh issue view 293 --comments`; both
  comment threads are empty. #284 owns the reproduced VM host-artifact alias;
  #293 owns later Exec removal and does not authorize a compatibility branch.
- ✓ the existing Rust acceptance, simulation, and qualified-metal tests
  introduced or transitioned by the original DISTILL, including all live
  Contract Shape and pending/active annotations.
- ✓ `docs/architecture/atdd-infrastructure-policy.md`, `.nwave/des-config.json`,
  the VM/Service/Job/Sim journeys, and `docs/product/kpi-contracts.yaml`.
- ⊘ feature-local DISCUSS, SPIKE, DEVOPS, and legacy wave-decision files are
  absent. This remains a warning, not a blocker: #284 and the accepted exact
  DESIGN supply scope and ports; the project ATDD policy supplies the test
  environments.

**Reconciliation passed — 0 unresolved contradictions.** ADR-0104 and PR #292
remain historical/rejected input. The active contract is the driver-neutral
predecessor-to-fresh-successor model in this feature delta. SystemGc
resubmission is inherited behavior, not a new material decision. The
docs-platform-only KPI registry has no applicable GH #284 contract, and this
DISTILL adds no new typed product outcome requiring an outcomes-registry row.

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDR | Impact |
|---|---|---|---|
| DESIGN P-105-1/P-105-2; ADR-0105 | `RestartAllocation.alloc_id` is the accepted numeric-current predecessor and `spec.alloc` is one distinct fresh successor for every composed driver, including Exec until GH #293 removes it; successor identity derives from stable workload plus successor allocation. | ADR-0105 | Pure properties must reject both the VM `StartAllocation` branch and Exec same-ID restart while preserving only payload projection by driver. |
| DESIGN P-105-1/P-105-5A; ADR-0108 | One checked allocator scans accepted row suffixes plus durable `restart_counts` keys, leaves gaps, stops at `u32::MAX`, and View fsync consumes the successor before effects. | ADR-0108 | Property tests cover the numeric domain; redb runtime close/reopen proves an unrowed reservation is not reused and no driver effect precedes View persistence. |
| DESIGN P-105-3 | Workload Failure charges one successor candidate count/time; generation and Platform Reclamation carry current policy without charge; historical issued keys remain the identity ledger. | ADR-0105/0108 | Tests assert exact candidate values and complements instead of summing or rewriting historical entries. |
| DESIGN P-105-4A/G-1; ADR-0109 | Only accepted numeric-current `Failed` or `Terminated` hands off replacement ownership; `Draining`, a historical terminal, and a View-only key do not. | ADR-0109 | Reconciler partitions must prove both eligibility and all named no-action/current-selection complements. |
| DESIGN P-105-6A/G-3; ADR-0106 | The successor outcome completes before exactly one predecessor driver/mTLS/network cleanup attempt; cleanup failure cannot delay or revoke the successor and successor error wins when both fail. | ADR-0106 | Seeded action-shim compositions hold predecessor cleanup open, observe a published successor, and exercise all four result partitions with exact IDs. |
| DESIGN P-105-7 | Predecessor row/history is immutable; a fresh successor starts at zero/None history; accepted Failed can become current; rejected publication fully unwinds without a row or second allocation proposal. | ADR-0105/0106/0108 | Observation-store acceptance/rejection tests assert both row families, exact unwind calls, and singular successor start. |
| DESIGN F-03 preservation | SystemGc resubmit remains fresh placement through `StartAllocation` because its retained row is not an eligible replacement predecessor. | n/a | A preservation partition pins the existing action family without presenting it as a new architecture choice. |
| GH #284 native evidence | Distinct VM allocation keys must prevent predecessor bind/unlink/remove/cgroup/process effects from naming successor artifacts. | ADR-0105/0106 | Existing qualified-metal Cloud Hypervisor/strace evidence remains the host-effect complement; it does not define application action policy. |

## Wave: DISTILL / [REF] Scenario and test list

| ID | Executable Rust test | Tags | Contract Shape | Acceptance obligation |
|---|---|---|---|---|
| S-284-PURE-01 | `replacement_identity_is_driver_neutral_for_exec_and_vm` | `@property @in-memory @exec @vm @pending:future-roadmap` | `pure-function` | Both drivers emit existing `RestartAllocation` with numeric-current predecessor, distinct successor, matching successor SVID, and unchanged driver payload kind. |
| S-284-PURE-02 | `workload_failure_charges_only_the_fresh_successor_candidate` | `@property @in-memory @workload-failure @error @pending:future-roadmap` | `pure-function` | One failure increments/stamps only the successor candidate; predecessor policy/history inputs stay immutable. |
| S-284-PURE-03 | `platform_reclamation_carries_policy_without_failure_charge_for_every_driver` | `@property @in-memory @platform-reclamation @non-effect @pending:future-roadmap` | `pure-function` | Exec and VM carry count/time unchanged into a fresh successor and never stamp reclamation as failure. |
| S-284-PURE-04 | `desired_generation_replacement_carries_policy_without_failure_charge` | `@property @in-memory @generation @non-effect @pending:future-roadmap` | `pure-function` | An eligible terminal generation predecessor uses Restart, stamps observed generation, and carries policy without charge. |
| S-284-PURE-05 | `only_numeric_current_failed_or_terminated_predecessor_is_eligible` | `@decision-table @in-memory @handoff @error @pending:future-roadmap` | `pure-function` | `Draining` emits no allocation action; accepted numeric-current Failed/Terminated, not historical terminal, is predecessor. |
| S-284-PURE-06 | `initial_placement_reserves_zero_and_system_gc_resubmit_stays_fresh_placement` | `@decision-table @in-memory @initial @system-gc @preservation @pending:future-roadmap` | `pure-function` | Initial placement reserves suffix zero for both drivers; SystemGc resubmit preserves fresh Start placement with retained history excluded from replacement eligibility. |
| S-284-PURE-07 | `accepted_failed_successor_is_current_while_view_only_reservation_is_not` | `@property @in-memory @publication @history @pending:future-roadmap` | `pure-function` | An accepted failed higher row is current; a higher reservation affects only successor allocation, never predecessor/policy selection. |
| S-284-PURE-08 | `retry_deadline_is_derived_only_from_numeric_current_accepted_candidate` | `@property @in-memory @backoff @exec @vm @pending:future-roadmap` | `pure-function` | A lexical-order falsifier proves both drivers read numeric-current candidate values, ignoring history and View-only time. |
| S-284-PURE-09 | `unparseable_attempts_do_not_participate_in_allocation` | `@negative @in-memory @malformed @pending:future-roadmap` | `pure-function` | Unparseable accepted/reserved keys do not enter the maximum; canonical fresh placement begins at zero. |
| S-284-PURE-10 | `allocator_issues_final_u32_identity_once_then_stops_without_wrap` | `@boundary @in-memory @error @pending:future-roadmap` | `pure-function` | `MAX-1` issues `MAX`; a maximum row/key emits no allocation action and leaves View unchanged for Exec and VM. |
| S-284-PROP-01 | `identity_advances_above_rows_and_reservations_for_every_driver` | `@property @proptest @in-memory @exec @vm @pending:future-roadmap` | `pure-function` | Generated gaps select exactly checked-max-plus-one across accepted rows and issued keys, retaining the accepted predecessor. |
| S-VM-26 | `job_kind_reclaimed_vm_is_restarted_never_fabricated_completed_zero` | `@property @in-memory @platform-reclamation @regression @pending:future-roadmap` | `pure-function` | Existing production-reachable Job+VM reclamation stays non-final and now uses predecessor-to-fresh-successor Restart. |
| S-VM-27 | `six_consecutive_reclamations_never_trip_restart_budget_exhausted` | `@property @in-memory @platform-reclamation @boundary @pending:future-roadmap` | `pure-function` | Reclamation remains replaceable above the failure ceiling while carrying, not charging, policy into the fresh successor. |
| S-284-SIM-01 | `successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver` | `@seeded-sim @exec @vm @ordering @mtls @network @pending:future-roadmap` | `bounded-change` | Seed 284105106 observes successor start, fresh Running row/index/mTLS, then one blocked exact-old driver→mTLS→network cleanup; successor stays live. |
| S-284-SIM-02 | `successor_and_cleanup_outcomes_follow_the_ratified_precedence_table` | `@seeded-sim @error @decision-table @pending:future-roadmap` | `bounded-change` | All success/failure combinations attempt old cleanup once after successor; cleanup-only error returns typed cleanup failure and both-fail returns successor error. |
| S-284-SIM-03 | `accepted_failed_successor_publishes_at_fresh_key_with_zero_history` | `@seeded-sim @start-rejected @history @pending:future-roadmap` | `bounded-change` | StartRejected writes fresh Failed zero/None, preserves predecessor, and cleans old exactly once. |
| S-284-SIM-04 | `rejected_successor_publication_fully_unwinds_without_immediate_second_proposal` | `@seeded-sim @publication-rejected @error @pending:future-roadmap` | `bounded-change` | Two LWW rejections produce no fresh row/index, one successor start, exact successor unwind, then exact predecessor cleanup; no second allocation is proposed. |
| S-284-SIM-05 | `durable_reservation_precedes_effect_and_reopen_skips_unrowed_identity_for_every_driver` | `@seeded-sim @redb @process-restart @error @exec @vm @pending:future-roadmap` | `bounded-change` | Redb View write completes before start; successor start error leaves no row; close/reopen bulk-loads reservation and both drivers start at the higher suffix. |
| S-284-METAL-P1 | `predecessor_cleanup_cannot_bind_or_remove_replacement_vm_artifacts` | `@qualified-metal @real-io @strace @preservation @pending:future-roadmap` | `bounded-change` | Retained #284 host proof: distinct beacon/run-dir/clone/index/cgroup/process families survive delayed exact-old disposal with no EADDRINUSE/ENOENT alias. |
| S-284-METAL-P2 | S-VM-28/48/81 and S-GTI-06a/b existing functions | `@qualified-metal @real-io @history @svid @rootfs @preservation @pending:future-roadmap` | `bounded-change` | Fresh VM row/history, clean rootfs, SVID/mTLS reinstall, typed failure, and final host cleanup remain required host complements but do not define the action family. |

The new/transitioned corrective contract has 18 focused/property/simulation
tests. Twelve are adverse, boundary, interruption, or failure partitions
(67%). The qualified-metal preservation cases stay fixed examples; no
generated real-I/O case duplicates the pure allocator or Sim ordering oracle.

## Wave: DISTILL / [REF] Walking-skeleton strategy

**Bug-fix N/A.** No CLI verb, HTTP route, wire type, lifecycle state, driver
method, or user journey changes. A new operator walking skeleton would
duplicate existing `overdrive deploy`/`workload describe` coverage and could
pass without proving the internal allocation-ownership correction. The
acceptance spine is S-284-SIM-01/SIM-05 at the composed control-plane ports,
plus the retained qualified-metal #284 artifact oracle. Together they prove
the behavior simulation and pure tests cannot each prove alone.

No production scaffold is present or authorized. The accepted exact API is
already sufficient; inventing any action, port, driver policy, store/schema,
retry, lock, detached task, network mechanism, type, variant, or parameter is a
DESIGN divergence.

## Wave: DISTILL / [REF] Adapter coverage

| Port / adapter | Scenario coverage | Contract proved |
|---|---|---|
| `WorkloadLifecycle::reconcile` / `next_evaluation_at` | S-284-PURE-01..10, PROP-01, S-VM-26/27 | Driver-neutral action identity, allocator union/gaps/exhaustion, candidate policy, handoff, current selection, generation/reclamation, initial placement, and SystemGc preservation. |
| `ReconcilerRuntime` + redb `ViewStore` | S-284-SIM-05 | Durable write-before-effect, dispatch-error retention, process reopen/bulk-load, and higher unrowed successor for both drivers. |
| action shim + `DriverRegistry` / `Driver` | S-284-SIM-01..04 | Successor-first sequencing, exact old/new handles, four-result precedence, StartRejected publication, rejection unwind, and singular successor start. |
| `SimObservationStore` and rejecting observation-port wrapper | S-284-SIM-01..04 | Fresh accepted Running/Failed keys, immutable predecessor, zero/None history, and `Ok(None)` no-row behavior through the existing trait. |
| `SimMtlsInterceptLifecycle` | S-284-SIM-01 | Successor mTLS is Live before old cleanup begins; old stop removes only predecessor state. |
| `NetSlotAllocator` + logical `WorkloadNetworkProvisioner` | S-284-SIM-01 | VM successor gets its own slot/plan; old teardown/release cannot remove successor ownership. Exec follows the same action policy without inventing network work. |
| `CloudHypervisorVmm`, `RealVmHostState`, cgroupfs/filesystem/Unix sockets, strace | S-284-METAL-P1/P2 | Real VM beacon/bind, run-dir children, clone/index, cgroup, process, rootfs and SVID effects remain allocation-key-disjoint and clean exactly at the host boundary. |

Every changed driven-port claim has a production-owner or real-I/O scenario.
The real Cloud Hypervisor child is a local Tier-3 fixture, not an external
consumer/provider contract; Pact-style machinery is inapplicable.

## Wave: DISTILL / [REF] Test inventory, retirement, and activation

| Artifact | Status after corrective re-DISTILL | Pending-marker owner / rationale |
|---|---|---|
| `crates/overdrive-reconcilers/tests/acceptance/vm_recreation_allocation_identity.rs` | Re-authored in place: 12 VM-only/VM-vs-Exec bodies replaced by 10 focused and one proptest driver-neutral bodies. Existing sidecar remains the direct proptest persistence path. | Every live test uses `#[ignore = "pending corrective re-DELIVER roadmap: …"]`; the future reviewed roadmap assigns activation. |
| `crates/overdrive-core/tests/acceptance/vm_reclamation_plan_purity.rs` S-VM-26/27 | Transitioned from VM `StartAllocation` assertions to fresh-successor `RestartAllocation` and exact carry complements. | Reasoned future-roadmap markers prevent the rejected implementation from reading green. |
| `crates/overdrive-sim/tests/driver_neutral_allocation_replacement.rs` | New complete seeded bodies for ordering, failure precedence, publication/history, rejection unwind, and redb reopen. | Five reasoned future-roadmap markers; no placeholder panic or missing fixture. |
| `crates/overdrive-sim/tests/e10_vm_early_exit_spike.rs` | The rejected ADR-0104 VM-only restart scenario was removed after the rejected delivery baseline was restored. Its independent no-intervening-restart reclamation control remains active. | No obsolete pending/retired restart marker or executable scenario remains; the new driver-neutral Sim file is the sole replacement contract. |
| Same-key restart/history/publication and cleanup-first tests in `action_shim_crash_observability.rs` | Seven explicitly contradictory active tests retired: successful/rejected same-key history, rejected-restart occurrence, restart telemetry pair, second-rejection same-key publication, and real-worker cleanup-first ordering. The post-assignment provision-failure preservation case now drives only valid `StartAllocation`; its invalid same-ID Restart partition was removed. | Retired markers name the fresh-key or successor-first replacement; SIM-01..04 and qualified-metal host evidence supersede them. |
| `action_shim_restart_uses_spec_from_action.rs` | Active preservation test now supplies distinct predecessor/successor IDs and proves only unchanged action-spec forwarding. | No pending marker: it does not claim to prove identity selection/publication and is already valid under both implementations. |
| S-VM-28/48/81, S-GTI-06a/b, S-284-METAL-P1 | Qualified-metal preservation bodies remain complete but return to pending against the restored same-ID production baseline. | Corrective future-roadmap markers replace the obsolete ADR-0104 step markers; the tests activate only after driver-neutral replacement lands. |

No production source, `.config/nextest.toml`, roadmap, execution log,
verification expectation, or PR state is part of this re-DISTILL.

## Wave: DISTILL / [REF] Placement and driving-port coverage

Placement follows existing Rust crate ownership:

- pure application policy remains in the reconciler acceptance binary beside
  the implementation owner;
- the two production-reachable Job+VM reclamation properties remain in their
  established core acceptance module;
- slow ordering/reopen cases live in `overdrive-sim/tests/` and drive the real
  action shim and convergence runtime with existing Sim/host-store adapters;
- real VM effects remain in the CLI integration `kvm-tests` lane, where an
  in-process `run_server` plus direct CLI handlers drive real Cloud Hypervisor
  rather than spawning the Overdrive production binary.

| Existing driving surface | Scenarios |
|---|---|
| `WorkloadLifecycle::reconcile` / `next_evaluation_at` | S-284-PURE-01..10, PROP-01, S-VM-26/27. |
| `run_convergence_tick_with_network_provisioner_for_test` using registered production `WorkloadLifecycle` | S-284-SIM-05. |
| `action_shim::dispatch_with_network_provisioner` | S-284-SIM-01..04 and the active action-spec forwarding preservation case. |
| existing `overdrive deploy`/`workload describe` handler path with real `Vmm::create` | S-284-METAL-P1/P2. |

Rust tests remain independent from `verification/expectations`: no Rust test
spawns the built Overdrive binary or emits expectation evidence. No EDD record
is added because the deterministic Sim and qualified-metal regression lanes
already supply maintained, failing evidence for the claim.

## Wave: DISTILL / [REF] Preconditions and execution lanes

- All compile and nextest commands run through `cargo xtask lima run --` on
  this macOS workspace. `cargo test`, nextest `--no-run`, and background test
  polling remain forbidden.
- Pure tests are default-lane in-memory properties. Every live property carries
  the exact rustdoc line `/// CONTRACT_SHAPE: pure-function.`.
- Sim tests require `overdrive-sim/integration-tests` plus
  `overdrive-control-plane/integration-tests`, use seed `284105106`, run on the
  current-thread Tokio flavor, and print the seed on the ordering/reopen paths.
- The redb scenario uses a per-test temporary data directory, closes the first
  runtime/database handle before reopen, and never substitutes an in-memory
  read for the durable bulk-load assertion.
- Qualified-metal preservation requires `OVERDRIVE_METAL_TARGET`, native
  non-virtualized x86_64, usable hardware KVM, provisioned VM fixtures,
  reflink-capable staging, Cloud Hypervisor, cgroup v2, and strace. These tests
  compile in Lima but execute only through `cargo xtask metal run --`; no local
  runtime result is claimed.
- No mutation testing belongs to DISTILL. The single final DELIVER-wave gate
  remains authoritative after all future steps and reviews.

## Wave: DISTILL / [REF] RED classification

The accepted corrective bodies were executed against the restored pre-delivery
same-ID implementation. Failures reached production behavior and assertions;
there were no import, fixture, compilation, or setup failures.

| Command / scope | Result | Classification and exact semantic signal |
|---|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --test acceptance --run-ignored ignored-only -E 'test(vm_recreation_allocation_identity)' --no-fail-fast` | `11 run; 11 failed` | `MISSING_FUNCTIONALITY` against restored pre-delivery production: both drivers retain same-ID replacement behavior, Draining is admitted, initial placement omits the generic reservation, and Exec ignores reservation/max boundaries. Proptest shrank to `accepted_suffix=0, reservation_gap=0`. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance --run-ignored ignored-only -E 'test(job_kind_reclaimed_vm_is_restarted_never_fabricated_completed_zero) | test(six_consecutive_reclamations_never_trip_restart_budget_exhausted)' --no-fail-fast` | `2 run; 2 failed` | `MISSING_FUNCTIONALITY`: both reached real `WorkloadLifecycle::reconcile` and received same-ID VM `RestartAllocation` instead of predecessor→fresh-successor Restart. Persisted sidecars supplied the minimized cases. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test driver_neutral_allocation_replacement --run-ignored ignored-only --no-fail-fast` | `5 run; 5 failed` | `MISSING_FUNCTIONALITY`: exact failures were old cleanup before Exec successor start; successor publication attempted at the old key (`0` fresh rejections); old-key StartRejected publication; cleanup-before-start trace; and Exec start before any reservation View write. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests,kvm-tests` | clean | `COMPILES`: every pending body, port wrapper, fixture, qualified-metal body, and assertion resolves against the accepted existing API. |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests,kvm-tests -- -D warnings` | clean | `LINTS`: the complete pending and preservation surfaces satisfy the workspace lint gate. |
| default-lane acceptance binaries for `overdrive-core`, `overdrive-reconcilers`, and `overdrive-control-plane` | `897 passed; 22 skipped` | `PRESERVATION_GREEN`: corrected scenarios remain pending, retired contracts remain inactive, and the valid active acceptance surface stays green. |
| qualified-metal preservation tests | compile-only locally | `RESOURCE_UNAVAILABLE`, not RED evidence: this workspace has no authorized native target in the DISTILL run. Existing retained #284 strace/host evidence remains the observation anchor. |

The one initially green candidate-policy case was tightened with a lexical
falsifier (`suffix 1` versus numeric-current `suffix 10`) so the current Exec
branch cannot pass by iteration coincidence. The final result is semantic RED,
not a fixture/import failure. Full raw nextest failure text is retained in the
session/tool record; this table preserves the stable classification needed by
the future roadmap.

## Wave: DISTILL / [REF] Completeness and re-DISTILL handoff

- **Empty/min/max:** initial zero, unparseable-only input, `MAX-1`, and `MAX`
  are executable; gaps and generated unions are proptested.
- **Lifecycle partitions:** Failed, Terminated, Draining, Running/current,
  historical terminal, SystemGc, accepted Failed successor, and View-only
  reservation each have a distinct oracle.
- **Policy partitions:** Workload Failure, generation, reclamation, backoff,
  initial placement, and exhaustion assert exact candidate maps/timestamps.
- **Ordering/concurrency:** a seeded blocked-old-cleanup sequence proves the
  successor Running/index/mTLS state before old cleanup, then exact driver,
  mTLS, network, slot, row, and index complements.
- **Error closure:** all four successor/cleanup results, StartRejected,
  `Ok(None)` twice-rejected publication, typed launch error, and durable reopen
  are covered without a catch-all or new error.
- **Adapter modes:** Exec and VM appear in the generic action, allocation,
  policy, handoff, ordering, and reopen contracts. VM-only host effects remain
  at qualified metal; no pure invariant is duplicated there.
- **No-action/non-effects:** Draining, exhaustion, historical terminal,
  View-only current exclusion, immutable predecessor/history, SystemGc Start
  preservation, and unrelated successor/old artifacts are explicit.
- **Contract Shape:** every new/transitioned live property carries the exact
  pure-function rustdoc; every stateful Sim/metal case is declared
  bounded-change.

No exact design/API blocker remains. Independent acceptance review is the next
gate. This artifact does not authorize a roadmap or DELIVER; after approval, a
fresh DELIVER roadmap must own every pending-marker activation and the legacy
same-key test fallout without reviving ADR-0104's rejected action split.
