# Reconciler View vs State — Understanding Three Layers of Cluster State

When you write a reconciler in Overdrive, you'll see four parameters arrive at the `reconcile` function:

```rust
fn reconcile(
    &self,
    desired: &State,
    actual: &State,
    view: &Self::View,
    tick: &TickContext,
) -> (Vec<Action>, Self::View)
```

The question "what exactly is the difference between `view` and `state`?" is reasonable. They both look like typed projections handed in by the runtime. They both must be pure inputs. The distinction, however, is load-bearing: they come from different stores with fundamentally different consistency models. Understanding this separation is what makes a reconciler author's mental model click into place.

**The short answer:** `desired` and `actual` are cluster-shared state (Intent and Observation respectively — linearizable and eventually consistent). `view` is private to your reconciler alone — it lives in a dedicated per-reconciler database. They are siblings in the reconcile signature, but siblings born from different parents.

---

## The Three-Layer State Taxonomy

Overdrive draws a hard boundary between three state layers, each with different consistency guarantees and different write rules. Every reconciler reads from all three on every evaluation tick.

| Layer | What it holds | Consistency | Stored in | Who writes | Reconcilers read | Reconcilers write |
|---|---|---|---|---|---|---|
| **Intent** | Desired state: jobs, policies, certificates, allocations | Linearizable (Raft-replicated) | `IntentStore` (redb or openraft+redb, per region) | Control plane only, via Raft | Yes — as `&desired` | Only via `Action` enum |
| **Observation** | Live state: allocation status, service endpoints, node health | Eventually consistent (seconds-fresh, CRDT-gossiped) | `ObservationStore` (Corrosion/CR-SQLite, global) | Owner-writer principle (each node writes its own rows) | Yes — as `&actual` | Never directly |
| **Memory** | Reconciler private state: restart counts, backoff windows, attempt history | Private to one reconciler (local ACID SQLite) | Per-reconciler libSQL at `<data_dir>/reconcilers/<name>/memory.db` | This reconciler only | Yes — as `&view` | Yes — via `NextView` return |

The whitepaper calls this boundary "load-bearing" because it isolates three different bug classes. A Raft partition does not corrupt observation (stored separately). A Corrosion backfill does not overwrite intent (stored separately). A reconciler logic bug does not leak its memory into the shared stores (isolated in libSQL). Nothing in the codebase can accidentally cross these boundaries — the type system prevents it.

---

## Understanding `desired` and `actual`

Both arrive at your `reconcile` function as `&State`. The runtime hydrates them together, before calling you, from the same two underlying stores:

- **`desired`** comes from the IntentStore — the cluster specification. For a job reconciler, this would be the job spec you submitted, the node list the cluster knows about, all the authoritative configuration.
- **`actual`** comes from the ObservationStore — the live reality. For a job reconciler, this would be the currently-running allocations, the health status of each, the resources actually in use.

They are *not* the same type because they come from different stores, and the runtime handles the difference:

```rust
// How the runtime hydrates them (simplified)
let desired = runtime.hydrate_desired(&reconciler, target).await;  // from IntentStore
let actual  = runtime.hydrate_actual(&reconciler, target).await;   // from ObservationStore
```

In Phase 1, for the JobLifecycle reconciler, both `desired` and `actual` are the same struct shape: `JobLifecycleState`. But they are interpreted differently:

```rust
struct JobLifecycleState {
    pub job: Option<Job>,                                  // shared field
    pub nodes: BTreeMap<NodeId, Node>,                    // shared field
    pub allocations: BTreeMap<AllocationId, AllocStatusRow>,  // shared field
}

// How the reconciler interprets them:
// desired.job = the job spec you asked for
// actual.allocations = what is actually running right now
```

The reconciler does one fundamental thing: it looks at `desired` and `actual`, spots the gap, and emits actions to close it.

---

## Understanding `view`

`view` is profoundly different. It is not cluster-shared. It is not hydrated from the authoritative stores. It is *private to your reconciler*, hydrated from a tiny database that only your reconciler touches.

Before `reconcile` is called, the runtime invokes an async method you implement:

```rust
async fn hydrate(
    &self,
    target: &TargetResource,
    db: &LibsqlHandle,  // your private database
) -> Result<Self::View, HydrateError>;
```

This is the *only* place you, as a reconciler author, touch libSQL directly. You read rows from your private database, decode them into your `View` struct, and hand them back. The runtime then passes this `&view` to your pure sync `reconcile` function.

What goes in a `view`? State that you need to remember between reconciliation ticks. For the JobLifecycle reconciler:

```rust
struct JobLifecycleView {
    pub restart_counts: BTreeMap<AllocationId, u32>,    // how many times has this alloc crashed?
    pub next_attempt_at: BTreeMap<AllocationId, Instant>, // when can we try restarting this alloc again?
}
```

Why not store this in `actual` (Observation)? Because `actual` is owner-writer — only the allocations themselves write to their own status rows. A reconciler cannot write arbitrary fields into Observation rows. Why not store it in `desired` (Intent)? Because you don't want to pollute your cluster specification with runtime memory. Why not store it in a global database? Because it is private to your reconciler — no other reconciler should read it, no other reconciler should write it.

So it lives in its own per-reconciler database, hydrated at the start of every reconciliation tick.

---

## Why This Separation Exists

Three reasons, in order of importance:

### 1. Different Consistency Models

The three layers have fundamentally different guarantees:

- **Intent** must be linearizable. When you ask "what is the job spec for job X?", you need *the* authoritative answer, not a stale cache. That job is running on nodes right now based on that spec. Raft ensures that.
- **Observation** tolerates eventual consistency. "Is allocation Y running right now?" can have a few seconds of stale data — the dataplane handles the reconciliation naturally through health checks and retries. Corrosion (CR-SQLite with CRDT semantics) is designed for this: every node gets a complete local copy within seconds, and partial stale data is OK because the protocol is built for it.
- **Memory** is single-threaded. Your reconciler is the only writer to its libSQL file. There is no distributed consensus needed. It is pure ACID — fast, local, synchronous.

Mixing these layers would break the guarantees. If you tried to store your restart count in Observation, you would be fighting Corrosion's eventual-consistency semantics — by the time your restart count propagated to all nodes, some of them would have already issued conflicting restart actions. If you tried to store it in Intent, you would be bloating your cluster specification with operational memory.

### 2. Different Write Surfaces

Each layer has a different rule about who writes and how:

- **Intent writes** go through the `Action` enum. You emit `Action::StartAllocation`, `Action::StopAllocation`, etc. These are data, not closures. The runtime commits them through Raft. Because they're serializable data, they're auditable, reproducible, and compatible with DST replay.
- **Observation writes** follow the owner-writer principle. Only the node running an allocation writes its status. A reconciler never writes Observation directly. (The action shim — the runtime layer that executes your actions — writes Observation rows on your behalf, but that's outside `reconcile`.)
- **Memory writes** are you, privately. You return a `NextView` from `reconcile`. The runtime diffs it against the old view and persists the delta to libSQL. No Raft, no gossip, no eventual consistency. It just writes.

### 3. Pure Function Reasoning — DST and ESR Proofs

This separation is what makes Overdrive's two flagship correctness properties work:

- **DST (Deterministic Simulation Testing).** The `reconcile` function is pure over its four inputs: `desired`, `actual`, `view`, and `tick`. Because it is pure, the DST harness can call it twice with identical inputs and assert byte-identical outputs. If non-determinism sneaks in (a hidden global, a wall-clock read, an RNG), the DST invariant catches it. The `hydrate` method is where async I/O happens; the pure boundary is drawn cleanly so DST can test the actual logic.

- **ESR (Eventually Stable Reconciliation).** The academic field has a proof technique (Anvil, USENIX OSDI '24, Best Paper) for verifying reconcilers reach their desired state and stay there. The proof works on pure functions over typed inputs. Overdrive adopted this shape: hydrate (async, I/O) → reconcile (pure, logic) → runtime executes actions (I/O). The purity of `reconcile` is what makes the ESR proof tractable.

If `view` were shared cluster state, DST would have to account for race conditions. If `desired` and `actual` were mutable, the pure reasoning would collapse. The three-layer separation is not a stylistic preference — it is the structural foundation for mechanically-verified correctness.

---

## A Concrete Example: JobLifecycle

The JobLifecycle reconciler is the first real reconciler shipping in Overdrive (Phase 1, first-workload). It starts, monitors, and stops job allocations based on desired vs. actual count.

**The desired-vs-actual loop:**

```rust
fn reconcile(
    &self,
    desired: &JobLifecycleState,    // job spec from IntentStore
    actual: &JobLifecycleState,     // running allocations from ObservationStore
    view: &JobLifecycleView,        // restart history from private libSQL
    tick: &TickContext,
) -> (Vec<Action>, JobLifecycleView) {
    // Question: how many allocations should exist?
    let desired_count = desired.job.as_ref().map(|j| j.replicas).unwrap_or(0);
    let actual_count = actual.allocations.len();

    if desired_count > actual_count {
        // Need to start new allocations
        return (vec![Action::StartAllocation { ... }], next_view);
    }

    if desired_count < actual_count {
        // Need to stop excess allocations
        return (vec![Action::StopAllocation { ... }], next_view);
    }

    // Count is right. Check health of running allocations.
    // If one has crashed repeatedly, apply backoff before restarting.
    for (alloc_id, alloc) in &actual.allocations {
        if alloc.state == AllocationState::Failed {
            let restart_count = view.restart_counts.get(alloc_id).unwrap_or(&0);
            let next_retry = view.next_attempt_at.get(alloc_id);

            if *restart_count > MAX_ATTEMPTS {
                // Exhausted retries. Emit no action; let it stay dead.
                continue;
            }

            if let Some(retry_at) = next_retry {
                if tick.now < *retry_at {
                    // Still in backoff window. Wait.
                    continue;
                }
            }

            // Safe to retry.
            return (vec![Action::RestartAllocation { ... }], next_view);
        }
    }

    (vec![], next_view)
}
```

Notice the layers in play:

- `desired.job.replicas` — Intent. The spec you submitted.
- `actual.allocations` — Observation. What is actually running.
- `view.restart_counts` — Memory. How many times *this reconciler* has restarted this allocation. No other reconciler can see this.
- `tick.now` — Time injected by the runtime, so DST can control it. Never call `Instant::now()` directly.

The reconciler's logic is pure: given these four typed inputs, it deterministically produces actions and a next-view. The runtime handles the I/O (start/stop via the driver, persist next-view to libSQL, write allocation status to Observation).

---

## How the Boundaries Are Enforced

The separation between the three layers is not enforced by convention alone. Overdrive has three mechanical gates that prevent accidental violations:

### Compile-Time: `dst-lint`

The `dst-lint` gate scans any `core`-class crate for banned nondeterminism APIs: `Instant::now()`, `rand::*()`, raw `tokio::net::*`. If you try to call these inside `reconcile`, the gate rejects your PR at compile time. Wall-clock reads inside `reconcile` are caught here — the only legitimate path to "now" is `tick.now`, which is a struct field access.

This enforces the "no direct I/O" boundary. `hydrate` is exempt from the ban because its whole purpose is async libSQL work.

### Runtime: `ReconcilerIsPure` DST Invariant

When the DST test harness runs, it invokes your `reconcile` twice with identical inputs and asserts bit-identical outputs. If you smuggle non-determinism through the trait boundary (interior mutability, TLS statics, FFI to C), the invariant catches it.

### Type System: Distinct Trait Objects

The `IntentStore` and `ObservationStore` are distinct trait objects. You cannot pass an `IntentStore` where an `ObservationStore` is expected. This prevents the accidental pollution of Observation rows with Intent data (or vice versa).

---

## Prior Art: This Is Well-Trodden Ground

The pattern Overdrive ships — hydrate-then-reconcile with a pure reconcile function — is not novel. Five independent projects converged on the same shape, across different languages and domains:

- **kube-rs (Rust, Kubernetes)**: `Store<K>` is an async-populated cache. Controllers read from sync `Store::get()` inside the reconcile loop. Async hydration, sync read.
- **controller-runtime (Go, Kubernetes)**: The cache-backed client exposes sync reads. The level-triggered reconciler loop drives repeated reconciles as resources change.
- **Anvil (USENIX OSDI '24, Rust, verified Kubernetes controllers)**: Pure `reconcile_core` over typed inputs, with an async shim for I/O. Verified for ZooKeeper, RabbitMQ, and FluentBit controllers — the shim pattern generalizes beyond Kubernetes.
- **The Elm Architecture (2012, functional UI)**: `update : Msg -> Model -> (Model, Cmd Msg)`. Pure function returning commands (data, not closures). Runtime interprets the commands.
- **Redux (2015, JavaScript, state management)**: Pure reducers `(state, action) -> newState`. Middleware (async layer) dispatches actions to reducers. The split isolates purity.

That five independent communities, over a decade, converged on the same shape strongly suggests it is the right structure for control logic. Overdrive is early, not fringe.

---

## What Not to Confuse

- **`view` vs cached state.** `view` is hydrated fresh at the start of each reconciliation tick. It is not a long-lived cache. If you need to carry state between ticks, it lives in `view`. But on tick N+1, the runtime re-hydrates `view` from the database, so you get a fresh snapshot.
- **`actual` vs reality.** `actual` is a snapshot of Observation at the start of the tick. By the time your `reconcile` runs, new events may have happened in the cluster. Your actions will converge over subsequent ticks.
- **`desired` vs configuration files.** `desired` is the cluster Intent — the job specs, policies, and allocations the scheduler has assigned. It is not raw YAML; it is the in-memory projection the runtime hydrated from the IntentStore.

---

## Further Reading

For the precise runtime contract, see [ADR-0013 — Reconciler Primitive Runtime](../product/architecture/adr-0013-reconciler-primitive-runtime.md) (§2 trait shape, §2b runtime contract, §2c TickContext, §Enforcement).

For the per-reconciler typed `State` enum (how `desired` and `actual` are currently being refactored), see [ADR-0021 — State Shape for Reconciler Runtime](../product/architecture/adr-0021-state-shape-for-reconciler-runtime.md).

For the DST invariants that mechanically enforce purity, see [ADR-0017 — Overdrive Invariants Crate](../product/architecture/adr-0017-overdrive-invariants-crate.md).

For the layered state taxonomy and the design rationale, see [Whitepaper §18 — Reconciler and Workflow Primitives](../whitepaper.md) (*Three-Layer State Taxonomy*, *Correctness Guarantees*).

For hands-on patterns, see [Development Rules — Reconciler I/O](../../.claude/rules/development.md) (includes the `RegisterReconciler` worked example).

---

## Research Foundation

This explanation is built on [`docs/research/reconciler-view-vs-state-for-explanation-doc.md`](../research/reconciler-view-vs-state-for-explanation-doc.md) (Phase 1 research, APPROVED 2026-04-27, 22 sources, average reputation 0.99).

---

## Review Metadata

```yaml
review:
  reviewer: nw-documentarist-reviewer
  date: 2026-04-27
  iteration: 1
  verdict: APPROVED
  classification_check: PASS
  validation_completeness: PASS
  collapse_check:
    tutorial_drift: NONE
    howto_drift: NONE
    reference_drift: MINOR  # justified — "What Not to Confuse" is clarification, not specification
  scores_agreement:
    type_purity: 88           # AGREE — explanation-dominant prose with minimal worked example
    flesch_reading: 72        # AGREE — appropriate complexity for reconciler authors; 70-80 target met
    style_consistency: 94     # AGREE — prior-art section longer than typical but justified; 1-pt shortfall not material
  reader_risk: LOW
  findings:
    critical: []
    high: []
    medium: []
    low:
      - "style_consistency_95_target: prior-art section longer than typical Overdrive subsections (5 projects, ~200 words). Justified by research recommendation to address reader concern 'is this experimental?'. The 1-point shortfall from the ≥95% target is not material. Recommend: accept as-is."
      - "polish_optional: 'What Not to Confuse' could optionally add 'why not store retry history in Observation?' clarification. Already answered in 'Why This Separation Exists' §2; brief version avoids redundancy. Recommend: accept as-is."
      - "diagrams_nice_to_have: no flow diagrams. Prose + taxonomy table sufficient for reconciler authors. Recommend: accept as-is."
  recommendation_to_orchestrator: |
    APPROVED for Phase 3 handoff. The doc correctly answers the user's question with clarity, precision, and appropriate depth. All quality gates met or acceptable. Three-layer mental model clearly explained through taxonomy, rationale, worked example, and enforcement mechanisms. Prior-art convergence (five independent projects) positions the design as canonical rather than speculative. All SSOT citations accurate and verbatim. The brief "What Not to Confuse" section is legitimate clarification, not reference-type specification. No blocking issues; minor polish opportunities are optional. Doc is publication-ready.
```
