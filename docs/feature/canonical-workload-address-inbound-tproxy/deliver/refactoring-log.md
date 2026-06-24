# Refactoring Log — canonical-workload-address-inbound-tproxy (DELIVER Phase 3, RPP L1–L6)

Behavior-preserving quality pass over the production code this feature
added/changed (`git diff origin/main..HEAD`). Scope = the feature's hunks only,
never pre-existing code in the same files. Base for the diff: `origin/main`.

## Verdict summary

**No production-code transformation was warranted.** Every primary and
secondary target was inspected against the full RPP cascade (L1 Readability →
L2 Complexity → L3 Responsibilities → L4 Abstractions → L5 Patterns → L6
SOLID++) and found already clean at the level each file reaches. The feature
code was written to the project's strict Rust conventions and passed per-step
adversarial review (03-02 keystone APPROVED); the L1–L6 obligations were
discharged *in-flight* (see "Refactors already present in the feature code"
below), not deferred to this pass.

Manufacturing diffs to show "work done" would be churn against code that is
already clean — explicitly a SUCCESS shape under the dispatch ("3 files
improved, 17 already clean"). Here it is **0 files transformed, all inspected
targets clean**, plus **one smell surfaced-but-not-fixed** because fixing it
cleanly would refactor pre-existing code outside the feature's hunks (finding
F-1 below).

Objective L1 signal: `cargo clippy -p overdrive-core -p overdrive-control-plane
-p overdrive-worker -p overdrive-sim --all-targets --features integration-tests
-- -D warnings` is **clean** (no warnings) at HEAD.

## Refactors already present in the feature code (L1–L3 discharged in-flight)

These are not transformations performed in this pass — they are evidence the
RPP obligations were already met when the code landed, so this pass has nothing
to add:

- **L3 (Large Function / extraction)** — `reconciler_runtime.rs`: the
  `WorkloadLifecycle` actual-side projection was extracted out of the
  `hydrate_actual` match arm into `hydrate_workload_lifecycle_actual`, keeping
  the dispatcher within the `clippy::too_many_lines` budget — the same
  extraction precedent as the existing `hydrate_svid_actual_held` /
  `hydrate_service_lifecycle_actual`.
- **L1/L2 (single source of truth for a decision)** — `action_shim/mod.rs`: the
  `Stable`-vs-genuine-terminal discriminator is hoisted once as `let is_stable
  = …` and reused at the row-state computation and both destructive-teardown
  sites, so the row-state and teardown decisions provably cannot drift. (This
  is the convergence-fix gate the dispatch forbids altering — left exactly as
  authored.)
- **L3 (collapse duplication via one source)** — `aggregate/mod.rs`:
  `ServiceV1::listen_ports()` was introduced as the single declared-listener-port
  source that both readers (the `project_service_listen_ports` Service arm and
  the Slice-05 liveness-restart spec path) read through, so the projected set
  is structurally identical across them (D-BLOCKER1 one-source/two-readers).
- **L1 (naming / intent-revealing helpers)** — `service_map_hydrator.rs`: the
  mesh-gate predicate is a named `is_mesh_backend` closure applied before the
  unchanged local/remote partition; the V6-VIP arm carries an explicit
  Phase-1-unreachable rationale comment.

## Per-file inspection ledger

### Primary targets

| File:symbol | RPP reach | Verdict |
|---|---|---|
| `overdrive-core/src/reconcilers/workload_lifecycle.rs` : `project_service_listen_ports`, `WorkloadLifecycleState.service_ports`, restart/liveness spec sites | L1–L3 | **Clean.** New pure projection mirrors `project_probe_descriptors` one-for-one; PBT producer-side test pins S-PORTSET; clone-from-desired at the identical site/shape as `probe_descriptors`. No smell. |
| `overdrive-control-plane/src/reconciler_runtime.rs` : `hydrate_desired`, `read_job`, `hydrate_actual` → `hydrate_workload_lifecycle_actual` | L1–L3 | **Clean.** L3 extraction already done (above). `read_job` 4-tuple return threads the new port set at the same seam as `probe_descriptors`; mutation-gate test added for the per-alloc `workload_addr` population. No smell. |
| `overdrive-core/src/traits/observation_store.rs` : `AllocStatusRowV2`, `From<V1> for V2`, envelope `V2` + discriminant repin | L1–L4 | **Clean.** Textbook rkyv version-bump per `development.md` § "rkyv schema evolution" → "Version-bump procedure" (append-only variant, additive field, `From` up-conversion, repinned discriminant offset, golden fixtures untouched). Verbose docstring is the mandated structural guard, not dead prose. No smell. |
| `overdrive-worker/src/mtls_intercept_worker.rs` : `start_alloc`, `spawn_legs_and_record`, `record_intercept_full`, `AllocIntercept._inbound_tproxy_guards` | L1–L3 | **Clean.** `Option<TproxyInterceptGuard>` → `Vec<TproxyInterceptGuard>` threaded uniformly; per-port install loop is minimal and idiomatic (`None` addr / empty ports → zero rules, fail-closed via `?`). No smell. |
| `overdrive-control-plane/src/action_shim/mod.rs` : `dispatch_single` FinalizeFailed arm, `build_alloc_status_row`, `provision_and_inject_netns`, Start/Restart Running-row writes | L1–L2 | **Clean.** Convergence-fix `is_stable` gate left intact (forbidden to alter); `is_stable` already hoisted/reused. `workload_addr` population at the Running-row write is an observed-input copy, no derivation. No smell. |
| `overdrive-core/src/reconcilers/service_map_hydrator.rs` : `canonical`, `workload_subnet`, three-way mesh gate, conditional dispatch bump | L1–L3 | **Clean within feature hunks.** Mesh gate is a named predicate before the unchanged partition; the conditional retry bump is guarded and commented. One pre-existing duplication touched at the edge — see **F-1** (NOT fixed: it lives in pre-existing code). |
| `overdrive-core/src/traits/driver.rs` : `AllocationSpec.workload_addr`, `AllocationSpec.service_ports` | L1 | **Clean.** Two additive in-memory fields with full "persist inputs, not derived state" rationale (no serde/rkyv, recomputed each tick). No smell. |
| `overdrive-core/src/reconcilers/backend_discovery_bridge.rs` : `RunningAllocSet.running` (`BTreeSet` → `BTreeMap`), advertise addr selection | L1–L2 | **Clean.** `BTreeMap<AllocationId, Option<Ipv4Addr>>` per the ordered-collection rule (iteration feeds a DST-deterministic fingerprint); D-B2 `workload_addr.unwrap_or(host_ipv4)` fallback is a single clear expression. No smell. |

### Secondary targets — inspected, clean, untouched

All secondary-target diffs are mechanical fixture/wiring updates with no logic
smell; each was inspected and left untouched:

- `overdrive-core/src/aggregate/mod.rs` — new single-source `ServiceV1::listen_ports()` (already minimal). Clean.
- `overdrive-core/src/reconcilers/mod.rs` — one re-export line. Clean.
- `overdrive-core/src/testing/observation_store.rs` — `workload_addr: None` on a harness row. Clean.
- `overdrive-control-plane/src/lib.rs` — threads `WORKLOAD_SUBNET_BASE` into the hydrator ctor (one source, D-GATE-PRED). Clean.
- `overdrive-control-plane/src/streaming.rs` — `workload_addr: None` on a host-netns test fixture. Clean.
- `overdrive-control-plane/src/worker/exit_observer.rs` — `workload_addr: prior.workload_addr` forward-carry, same pattern as `started_at`/`kind`. Clean.
- `overdrive-sim/src/invariants/{backend_discovery_bridge,service_map_hydrator,evaluators,svid_running_set}.rs` — `.insert(id, None)` / `workload_addr: None` / `WORKLOAD_SUBNET_BASE` ctor threading. Clean.
- `overdrive-sim/src/adapters/driver.rs`, `overdrive-worker/src/driver.rs` — `service_ports: Vec::new()` / `workload_addr: None` on test spec builders. Clean.

## Findings surfaced but NOT fixed (would exceed behavior-preserving / in-scope mandate)

### F-1 — Duplicated "record a dispatch in the View" mutation in `service_map_hydrator::reconcile`

- **Where**: `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs`, the
  V6-VIP arm (the 4-line `entry.attempts / last_failure_seen_at /
  last_attempted_fingerprint` bump) and the V4-path tail (the same 4 lines,
  now guarded by `if !(local_is_empty && remote_is_empty)`).
- **Smell (L2/L3)**: the identical 4-line View-bump appears twice; a private
  helper (e.g. `record_dispatch_attempt(entry, fingerprint, now)`) would
  collapse both call sites.
- **Why NOT fixed here**: the duplication is **pre-existing in `origin/main`** —
  both copies existed before this feature; the feature only wrapped the V4-path
  copy in a guard. The *primary* value of the collapse is cleaning up the
  pre-existing V6 arm, which is outside this feature's hunks. Per the dispatch's
  scope discipline ("Do NOT refactor pre-existing code unrelated to this
  feature's hunks — that is scope creep that contaminates the feature's refactor
  commit"), this belongs in a separate, dedicated refactor of the hydrator's
  dispatch-bookkeeping, not in this feature's Phase-3 pass. Extracting it does
  not require new *public* surface (a private helper suffices) and is
  behavior-preserving, so it is a valid future refactor — just not one to fold
  into this feature's commit.

No finding required new public API surface or a behavior change; F-1 is the only
smell found, and it is correctly deferred on scope grounds, not capability
grounds.

## Other-feature / spike isolation confirmation

- `overdrive-worker/tests/integration/bidirectional_walking_skeleton.rs`
  (transparent-mtls #236) — **not touched** by this pass.
- `spike-scratch/` — **not touched**.
- No file belonging to another feature was renamed, moved, or edited.

## Gate evidence

- Clippy (touched crates, `-D warnings`, integration-tests, Lima-routed): **PASS, clean.**
- No production source files were modified in this pass, so no per-crate
  test-lane re-run was required to prove behavior preservation — the existing
  suite that the feature shipped GREEN remains byte-identically GREEN (zero
  source delta). This log is the only artifact added.
