# C4 Level 2 — Container Diagram

**Wave**: DESIGN
**Date**: 2026-04-30

Feature-scoped extension of brief.md §C4 Level 2 (Phase 1
first-workload). Highlights the new edges and the new in-process
broadcast channel; everything else is inherited unchanged.

```mermaid
C4Container
  title Container Diagram — cli-submit-vs-deploy-and-alloc-status

  Person(operator, "Operator (Ana, TTY)")
  Person(ci, "CI / automation (non-TTY)")

  Container_Boundary(workspace, "Overdrive workspace") {
    Container(cli, "overdrive-cli", "Rust binary", "submit (streaming default + --detach + IsTerminal); alloc status (rewritten renderer)")
    Container(api_types, "overdrive-control-plane::api", "Rust module (shared types)", "SubmitEvent, TransitionReason, TerminalReason, TransitionSource, AllocStateWire, TransitionRecord, RestartBudget, ResourcesBody (all NEW); SubmitJobResponse, AllocStatusResponse (existing, AllocStatusResponse extended in place)")
    Container(ctrl, "overdrive-control-plane", "Rust crate (adapter-host)", "axum router; submit_job handler (content-negotiated); streaming_submit_loop (NEW); alloc_status handler (extended); ReconcilerRuntime; EvaluationBroker; action shim (extended); JobLifecycle reconciler")
    Container(core, "overdrive-core", "Rust crate (core)", "Reconciler trait + AnyState/View enums; AllocStatusRow (extended with reason+detail); AllocState (extended with Failed); TransitionReason (NEW, also used by api types via re-export); LifecycleEvent (NEW, internal-only)")
    Container(worker, "overdrive-worker", "Rust crate (adapter-host)", "ExecDriver; cgroup management; node_health writer")
    Container(store_local, "overdrive-store-local", "Rust crate (adapter-host)", "LocalStore (intent); LocalObservationStore (observation; rkyv schema admits new fields additively)")
    Container(host, "overdrive-host", "Rust crate (adapter-host)", "SystemClock (the production Clock impl behind the wall-clock cap)")
    Container(sim, "overdrive-sim", "Rust crate (adapter-sim)", "SimClock (DST), SimObservationStore (DST); both honour the new row schema")
  }

  ContainerDb(redb_intent, "intent.redb", "redb file", "IntentStore backing")
  ContainerDb(redb_obs, "observation.redb", "redb file", "ObservationStore backing")
  ContainerDb(libsql, "libSQL files", "On-disk SQLite", "Per-reconciler private memory (existing)")
  System_Ext(kernel, "Linux kernel", "cgroup v2 + process API")

  Rel(operator, cli, "TTY: streaming default. `submit ./job.toml` -> NDJSON")
  Rel(ci, cli, "Non-TTY: auto-detach. Or explicit `submit ./job.toml --detach` -> JSON ack")

  Rel(cli, api_types, "Imports SubmitEvent, AllocStatusResponse, ... from")
  Rel(cli, ctrl, "POST /v1/jobs (Accept: application/x-ndjson | application/json); GET /v1/allocs?job=...", "rustls HTTPS / chunked NDJSON or single JSON")

  Rel(ctrl, api_types, "Re-uses (the api module IS in this crate)")
  Rel(ctrl, core, "Implements handlers against ports; uses TransitionReason, LifecycleEvent, AllocState, etc.")
  Rel(ctrl, store_local, "IntentStore::put_if_absent; ObservationStore::write + alloc_status_rows")
  Rel(ctrl, libsql, "ReconcilerRuntime hydrates JobLifecycleView per tick")
  Rel(ctrl, host, "Arc<dyn Clock> production-binds to SystemClock for the wall-clock cap")
  Rel(ctrl, worker, "Action shim calls Driver::start/stop on Arc<dyn Driver>")

  Rel(worker, kernel, "ExecDriver writes cgroup files; spawns workload via tokio::process")
  Rel(worker, store_local, "Writes node_health row at startup")

  Rel(ctrl, ctrl, "broadcast::Sender<LifecycleEvent>: action shim emits, streaming_submit_loop subscribes (in-process channel; not a network hop)")

  Rel(sim, ctrl, "DST harness threads SimClock; sleep advances simulated time past the cap")
```

## Notes

### What's new in this diagram vs brief.md §C4 Level 2

1. **`broadcast::Sender<LifecycleEvent>` self-edge on
   `overdrive-control-plane`** — the new push channel from action shim
   to streaming handler. In-process; not a network hop. Covered by
   ADR-0032 [D4].
2. **`SimClock` edge from `overdrive-sim` to `overdrive-control-plane`**
   — explicit because the wall-clock cap [D3] is the new DST surface.
   The same `Clock` injection that ADR-0013 §2c established now drives
   the streaming-handler timer.
3. **`api_types` shown as a labelled module within
   `overdrive-control-plane`** (not a separate crate) — reflects
   ADR-0014 §Considered alternatives D ("place shared types in a
   separate crate, rejected on YAGNI for Phase 1"). The new types
   land in this same module.

### What's unchanged

- The `cli → ctrl` HTTP edge — still rustls / HTTP/2; the change is
  inside the response media type, not the transport.
- The `ctrl → store_local` edges — still typed `IntentStore` /
  `ObservationStore` traits; the new row fields are additive on
  `AllocStatusRow`'s rkyv shape.
- The `ctrl → worker` edge via `&dyn Driver` — unchanged; the
  `DriverError::StartRejected.reason` field that gets captured into
  `AllocStatusRow.detail` already exists.
- The `worker → kernel` edge — unchanged; the streaming surface does
  not touch the kernel directly.

### What's deliberately NOT in this diagram

- A separate `streaming_submit_loop` container — it's not a container,
  it's a function inside the `submit_job` handler. Promoting it would
  imply a separable deployment unit, which it is not.
- A network/RPC edge between handler and reconciler runtime — they
  share `AppState` via `axum::extract::State`. In-process; no RPC.
