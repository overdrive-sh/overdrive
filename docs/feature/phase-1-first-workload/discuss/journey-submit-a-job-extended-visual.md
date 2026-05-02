# Journey Visual — Submit a Job and Watch It Run (extended)

**Feature**: `phase-1-first-workload`
**Base journey**: `docs/product/journeys/submit-a-job.yaml` (inherits steps 1-4 unchanged on the wire; step 4 is materially extended in observable output).
**Persona**: Ana, Overdrive platform engineer.
**Confidence pattern**: Curious → Focused → Confident → Trusting.

> **Phase 1 is single-node.** Control plane and worker run co-located on
> one machine. There is exactly one node — the local host — and it is
> implicit. There is no operator-facing node-registration verb, no
> taint, no toleration, no multi-node placement choice. The local node
> is a precondition, not a step.

---

## ASCII flow — extended journey

```
                                                     PHASE 1: phase-1-control-plane-core (inherited)
                                                                        |
                                                       =================|=================
                                                       v                v                 v
[1. Submit job (inh.)] -> [2. Commit intent (inh.)] -> [3. Reconcilers alive (extended)] -> [4. Inspect allocations (materially extended)]
        |                         |                              |                                       |
   Skeptical                  Focused                       Focused                                Focused
        v                         v                              v                                       v
"is the loop wired?"       "did the store               "is the lifecycle           "did the loop close end-to-end?
                            take it?"                    reconciler running?"        Will I see a real Running row?"
        |                         |                              |                                       |
        +--------+--------+-------+--------+--------+-------+----+--------+-------------+-------+----------------+
                                                                                                                |
                                                                          phase-1-first-workload (NEW)          |
                                                                                                                v
                                                                                                  [5. Recover from crash]
                                                                                                          |
                                                                                                       Curious
                                                                                                          v
                                                                                              "does it actually self-heal?"
                                                                                                          |
                                                                                                          v
                                                                                  [6. Control plane stays responsive]
                                                                                                          |
                                                                                                      Skeptical
                                                                                                          v
                                                                                          "does cgroup actually hold?"
                                                                                                          |
                                                                                                          v
                                                                                            [7. Stop the job and drain]
                                                                                                          |
                                                                                                       Focused -> Trusting
                                                                                                          v
                                                                                                "process gone, scope gone,
                                                                                                 allocation terminated"
```

---

## Emotional arc

| Step | Entry | Exit | Win deposited |
|---|---|---|---|
| 1. Submit job (inh.) | Skeptical | Focused | (inherited) commit_index/spec_digest behaviour from prior feature still holds. |
| 2. Commit intent (inh.) | Focused | Confident | (inherited) IntentStore::put closes deterministically. |
| 3. Reconcilers alive (extended) | Focused | Confident | "Two reconcilers registered. Broker dispatched > 0 — lifecycle ran." |
| 4. Inspect allocations (extended) | Focused | Trusting | "Real Running row. Spec digest matches local compute." |
| 5. Recover from crash | Curious | Trusting | "I killed the process; the platform converged on its own." |
| 6. CP responsive | Skeptical | Trusting | "Workload at 100% CPU on the same machine; CLI still answers in 12 ms." |
| 7. Stop and drain | Focused | Trusting | "Process gone. Scope gone. Allocation Terminated. Clean shutdown." |

The arc is **discovery joy plus problem relief**: the operator starts by being curious whether the empty states will fill in, hits the satisfaction of seeing a Running row, then gets an unexpected confidence boost from watching the platform self-heal without operator intervention.

---

## TUI mockups (per step)

### Step 1 — Submit a job (inherited)

```
$ overdrive job submit ./payments.toml
Accepted.
  Job ID:            payments
  Intent key:        jobs/payments
  Spec digest:       sha256:7f3a9b12...
  Outcome:           created
  Endpoint:          https://127.0.0.1:7001
  Next:              overdrive alloc status --job payments
```

(Identical to the base journey's step 1. The Job aggregate shape is unchanged from `phase-1-control-plane-core`; this feature does not extend it.)

---

### Step 3 — cluster status with two reconcilers (extended)

```
$ overdrive cluster status
Mode:               single
Region:             default
Reconcilers:
  - noop-heartbeat
  - job-lifecycle      <-- new in this feature
Broker:
  Queued:             0
  Cancelled:          0
  Dispatched:         2  <-- non-zero now: lifecycle ran for `payments`
```

---

### Step 4 — Inspect allocations (materially extended)

**Happy path: 1 replica reaches Running on the local node**

```
$ overdrive alloc status --job payments
Job ID:             payments
Spec digest:        sha256:7f3a9b12...
Replicas (desired): 1
Allocations:
  ALLOC ID         NODE      STATE      RESOURCES           STARTED
  a1b2c3...        local     Running    2000mCPU / 4 GiB    2026-04-27T10:15:32Z
```

**Pending path: insufficient capacity on the local host**

```
$ overdrive alloc status --job big-job
Job ID:             big-job
Spec digest:        sha256:abc1234...
Replicas (desired): 1
Allocations:
  STATE             REASON
  Pending           No node has capacity (job needs 8 GiB; local node free
                                          memory is 4 GiB)
```

> Note the empty-state-with-reason pattern: even when no allocation runs, the CLI says **why**. This is the structural answer to "honest empty states are honest about the cause, not just the symptom."

---

### Step 5 — Recover from a process crash (new)

```
# Operator kills the process directly
$ kill -9 12345

# Within seconds, alloc_status reflects the failure...
$ overdrive alloc status --job payments
Allocations:
  ALLOC ID         NODE      STATE         RESOURCES           STARTED
  a1b2c3...        local     Terminated    2000mCPU / 4 GiB    2026-04-27T10:15:32Z

# ...and the next tick converges back to Running with a fresh alloc_id
$ overdrive alloc status --job payments
Allocations:
  ALLOC ID         NODE      STATE      RESOURCES           STARTED
  d4e5f6...        local     Running    2000mCPU / 4 GiB    2026-04-27T10:15:48Z
```

**Backoff exhausted**

```
$ overdrive alloc status --job broken-job
Allocations:
  ALLOC ID         NODE      STATE                            STARTED
  z9y8x7...        local     Failed (backoff exhausted: 5)    2026-04-27T10:15:32Z
```

---

### Step 6 — Control plane stays responsive under workload pressure (new)

```
# Workload bursting CPU on the same host
$ stress --cpu 4 &     # inside the workload

$ time overdrive cluster status
Mode:               single
Region:             default
Reconcilers:        noop-heartbeat, job-lifecycle
Broker:             queued=0 cancelled=0 dispatched=14

real    0m0.012s    <-- control plane still snappy
user    0m0.008s
sys     0m0.002s
```

---

### Step 7 — Stop the job and watch it drain (new)

```
$ overdrive job stop payments
Accepted. Stop requested for job `payments`.

$ overdrive alloc status --job payments
Allocations:
  ALLOC ID         NODE      STATE         RESOURCES           STOPPED
  d4e5f6...        local     Terminated    2000mCPU / 4 GiB    2026-04-27T10:18:01Z
```

---

## Shared artifacts at a glance

| Artifact | First introduced | Single source of truth | Risk |
|---|---|---|---|
| `node_id` | (precondition — single-node) | `overdrive-core::aggregate::Node::id` (NodeId newtype, exactly one row in `node_health` at runtime) | LOW (single-node precondition; still typed) |
| `node_capacity` | (precondition — single-node) | `overdrive-core::aggregate::Node::capacity` (Resources) | MEDIUM (capacity check drives Pending reasoning) |
| `placement_decision` | Step 4 | scheduler module output (NEW) | MEDIUM |
| `alloc_id` | Step 4 | `AllocationId` newtype emitted by `Action::StartAllocation` | HIGH (multi-hop ID) |
| `alloc_state` | Step 4 | `AllocationState` enum (already in `traits/driver.rs`) | HIGH (state machine) |
| `cgroup_path` | Step 4 (driver-side) | `ProcessDriver` derives from `alloc_id` (NEW) | MEDIUM |
| `spec_digest` | Step 4 | `ContentHash::of(rkyv_archive(Job))` (inherited) | HIGH (still applies; no Job field added in Phase 1) |
| `restart_count` | Step 5 | lifecycle reconciler's libSQL `view` (NEW) | LOW (internal) |

See `shared-artifacts-registry.md` for the full registry with consumers and validation hooks.

---

## CLI UX compliance check

| Rule | Status |
|---|---|
| Help available on every command (`--help`) | Inherited from prior feature; new commands (`job stop`) follow the same pattern. |
| First output within 100ms on localhost | Required (Step 4 + Step 6). Control-plane responsiveness is itself a UAT scenario. |
| Empty states are honest and actionable | Step 4 Pending rows name the cause; Step 5 Failed states name the backoff state. |
| Errors answer "what / why / how to fix" | Step 6 cgroup-delegation error answers all three. |
| Color paired with text labels | Inherited convention; `Running` / `Pending` / `Terminated` / `Failed` are colored AND labelled. |
| `NO_COLOR` env respected | Inherited. |
| TTY detection disables animations in pipes | Inherited. |

---

## Integration checkpoints (cross-step)

| Checkpoint | Steps | What must be true |
|---|---|---|
| Lifecycle reconciler emits StartAllocation | 1+3 → 4 | A committed Job triggers a broker evaluation that the lifecycle reconciler drains, producing `Action::StartAllocation` with the right `alloc_id`, `job_id`, single-node `node_id`, `spec`. |
| Action shim dispatches to driver | 4 (start), 5 (restart), 7 (stop) | The runtime's action shim (NEW in this feature) consumes allocation-management actions and calls into `ProcessDriver` accordingly, then writes `AllocStatusRow`. |
| cgroup scope name = alloc_id | 4 → 6 | `overdrive.slice/workloads.slice/<alloc_id>.scope` exists for every Running allocation; the path on disk matches what the CLI displays. |
| Pure-function contract holds | 1, 4, 5 | The lifecycle reconciler is added to the DST `ReconcilerIsPure` invariant; twin invocation produces identical output. |

---

## What's NOT in scope here (and why)

- **Multi-node clusters.** Phase 1 is single-node co-located; multi-node + Raft is Phase 2.
- **Operator-facing node registration.** Implicit precondition — exactly one node, written by the server at startup. No CLI verb in Phase 1; lands when multi-node placement does.
- **Taints and tolerations.** Out of Phase 1 entirely. With one node, there is no placement choice for a taint to gate against. Lands alongside multi-node + Raft (the user is expected to split GH #20 into separate cgroup-isolation and taint/toleration issues).
- **microVM, WASM, unikernel drivers.** Process driver only — others land in later features.
- **Real cgroup-based right-sizing.** §14 right-sizing reads memory pressure via eBPF — Phase 2+.
- **Network policy / mTLS / sockops.** No dataplane in Phase 1.
- **Real Corrosion gossip.** `LocalObservationStore` (single-writer redb) handles observation; CRDT gossip is Phase 2.
- **Operator auth / SPIFFE operator IDs.** Phase 5.

These are intentional non-goals, not gaps. The walking skeleton stays narrow enough to be testable in DST end-to-end while still moving the operator from "platform commits things" to "platform runs things and converges."

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial visual narrative for the journey extension. Inherits steps 1-4 from `submit-a-job.yaml`; adds steps 5, 6, 7. |
| 2026-04-27 | Scope correction — Phase 1 is single-node. Removed the prior "step 0 (register a host node)" entirely. Removed every reference to taints, tolerations, default `control-plane:NoSchedule`, and multi-node placement choice. |
