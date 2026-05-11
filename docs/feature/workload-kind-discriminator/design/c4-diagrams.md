# C4 Diagrams — workload-kind-discriminator

**Feature**: workload-kind-discriminator
**Wave**: DESIGN
**Author**: Morgan
**Date**: 2026-05-10

This file ships the C4 diagrams for the feature delta. The
**System Context (L1)** and **Container (L2)** diagrams build on
the existing diagrams in `docs/product/architecture/c4-diagrams.md`
(Phase 2.1) — only the bounded context affected by this feature is
called out. The **Component (L3)** diagram is feature-specific and
shows the spec-parser pipeline, the per-kind streaming dispatcher,
and the kind-aware render layer.

---

## C4 Level 1 — System Context (no change)

The kind discriminator is purely internal type-shape work. The
operator → CLI → control plane → driver → workload boundaries do
NOT change. **No new external systems are introduced.** The Phase
2.1 L1 diagram in `c4-diagrams.md` § "Phase 2.1 — eBPF Dataplane
Containers" → "C4 Level 1 — System Context" remains the SSOT.

For convenience, here is the unchanged shape, with the workload-
kind-discriminator scope annotated:

```mermaid
C4Context
  title System Context — Overdrive (workload-kind-discriminator scope highlighted)

  Person(engineer, "Platform Engineer (Ana)", "Writes [service] / [job] / [job]+[schedule] TOML specs; submits via overdrive CLI; inspects via overdrive alloc status")
  System(overdrive, "Overdrive node", "Single binary — control plane + worker. THIS FEATURE: extends spec parser, streaming protocol, alloc-status render with kind-aware semantics.")
  System_Ext(kernel, "Linux kernel", "BPF + cgroup v2 — exec workloads under /sys/fs/cgroup/overdrive.slice/workloads.slice")
  System_Ext(fs, "Local filesystem", "redb for IntentStore + LocalObservationStore; TOML specs read from operator's filesystem")

  Rel(engineer, overdrive, "Submits [service]/[job]/[job]+[schedule] TOML; reads kind-aware streaming + alloc-status output")
  Rel(overdrive, kernel, "Spawns workloads + reads exit codes via the cgroup boundary")
  Rel(overdrive, fs, "Persists intent (WorkloadSpec) + observation (AllocStatusRow with kind)")
```

---

## C4 Level 2 — Container (annotated delta)

```mermaid
C4Container
  title Container Diagram — workload-kind-discriminator delta

  Person(engineer, "Platform Engineer (Ana)")

  Container_Boundary(workspace, "Overdrive workspace") {
    Container(core, "overdrive-core", "Rust crate (class: core)", "EXTENDED: WorkloadSpec enum + per-kind specs (ServiceSpec/JobSpec/ScheduleSpec); WorkloadKind projection; Listener / ServiceVip / CronExpr newtypes; AllocStatusRow.kind + listeners denormalised columns")
    Container(store_local, "overdrive-store-local", "Rust crate (class: adapter-host)", "Unchanged shape; reads/writes new WorkloadSpec variants verbatim through existing IntentStore + ObservationStore traits")
    Container(worker, "overdrive-worker", "Rust crate (class: adapter-host)", "Unchanged; ExecDriver consumes the same AllocationSpec regardless of kind (Job-kind backoff_limit consumed by reconciler, not driver)")
    Container(ctrl, "overdrive-control-plane", "Rust crate (class: adapter-host)", "EXTENDED: per-kind streaming protocol (ServiceSubmitEvent / JobSubmitEvent / ScheduleSubmitEvent); JobLifecycle reconciler emits typed Completed{exit_code:0}/Failed{exit_code:N} for Job kind")
    Container(cli, "overdrive-cli", "Rust binary (class: binary)", "EXTENDED: section-as-discriminator TOML parser; kind-aware render branches (Service/Job/Schedule); CLI exit code = workload exit code for Job kind; deferral URL constants")
    Container(xtask, "xtask", "Rust binary (class: binary)", "EXTENDED: dst-lint adds 'live' grep gate")
  }

  ContainerDb(redb_intent, "redb (intent)", "On-disk ACID KV — WorkloadSpec rkyv-archived rows")
  ContainerDb(redb_obs, "redb (observation)", "On-disk LWW — AllocStatusRow with kind + listeners")
  System_Ext(kernel, "Linux kernel", "cgroup v2 + exec")
  System_Ext(fs, "Operator filesystem", "[service]/[job]/[job]+[schedule] TOML specs; examples/coinflip.toml migrated to [job] shape")

  Rel(engineer, cli, "overdrive job submit ./spec.toml; overdrive alloc status --job <id>")
  Rel(engineer, fs, "Writes spec.toml")
  Rel(cli, fs, "Reads spec.toml at parse time")
  Rel(cli, ctrl, "POST /v1/jobs (Accept: application/x-ndjson) carrying WorkloadSpecInput; receives kind-tagged SubmitEvent stream", "HTTPS")
  Rel(ctrl, redb_intent, "Persists WorkloadSpec via IntentStore::put_if_absent")
  Rel(ctrl, redb_obs, "Reads AllocStatusRow (with kind + listeners) for status snapshot")
  Rel(ctrl, worker, "Dispatches Action::StartAllocation regardless of kind; reconciler tracks Job-kind terminal-or-not")
  Rel(worker, kernel, "Spawns workload, reads exit code on terminate")
  Rel(worker, redb_obs, "Writes alloc_status row with kind copied from intent at first write")
  Rel(xtask, cli, "dst-lint scans crates/overdrive-cli/src/render*.rs for 'live' literal")
```

---

## C4 Level 3 — Spec Parser Pipeline + Per-Kind Streaming Dispatch (NEW)

This is the load-bearing component diagram for the feature. It
shows the section-as-discriminator parser, the kind branch point in
the submit handler, and the three sibling streaming protocols.

```mermaid
C4Component
  title Component Diagram — Spec Parser + Streaming Dispatch (workload-kind-discriminator)

  Person(engineer, "Platform Engineer (Ana)")
  ContainerDb_Ext(fs, "Operator FS", "spec.toml")
  ContainerDb_Ext(intent, "IntentStore (redb)", "WorkloadSpec rkyv-archived")
  ContainerDb_Ext(obs, "ObservationStore (redb)", "AllocStatusRow with kind")

  Container_Boundary(cli, "overdrive-cli (binary)") {
    Component(submit_cmd, "commands::job::submit", "Rust", "Submit subcommand entrypoint; reads file path; calls parser; opens NDJSON stream")
    Component(parser, "spec::parser (NEW)", "Rust + serde + custom Deserialize", "Walks TOML Value::Table; branches on section presence; produces WorkloadSpecInput or typed ParseError naming offending sections")
    Component(render_dispatcher, "render::dispatch (NEW)", "Rust", "Reads SubmitEvent kind tag; dispatches to per-kind render functions")
    Component(render_service, "render::service (EXTENDED)", "Rust", "format_running_summary (Service vocabulary; 'live' literal removed); Listeners section; format_stopped_summary (Service variant)")
    Component(render_job, "render::job (NEW)", "Rust", "format_job_succeeded / failed / attempt_failed; format_job_alloc_status_header + attempts_table; stderr_tail rendering")
    Component(render_schedule, "render::schedule (NEW)", "Rust", "format_schedule_registered + format_schedule_alloc_status; reads SCHEDULE_EXECUTION_TRACKING_URL constant")
    Component(deferrals, "render::deferrals (NEW)", "Rust", "SCHEDULE_EXECUTION_TRACKING_URL = 'https://github.com/overdrive-sh/overdrive/issues/166'; SERVICE_VIP_ALLOCATOR_TRACKING_URL = '#167'")
  }

  Container_Boundary(ctrl, "overdrive-control-plane") {
    Component(submit_handler, "api::submit_handler", "Rust + axum", "Receives WorkloadSpecInput; validates; persists via IntentStore; opens streaming bus; emits kind-tagged SubmitEvent envelope")
    Component(stream_dispatcher, "streaming::dispatcher (NEW)", "Rust", "Reads WorkloadSpec.kind() from intent; selects ServiceSubmitEvent / JobSubmitEvent / ScheduleSubmitEvent inner enum")
    Component(stream_service, "streaming::service (EXTENDED)", "Rust", "Subscribes to alloc_status rows; emits ConvergedRunning / ConvergedFailed / ConvergedStopped per existing semantics")
    Component(stream_job, "streaming::job (NEW)", "Rust", "Subscribes to alloc_status rows; waits for ExitObserver terminal row; emits Succeeded{exit_code:0} or Failed{exit_code:N}; emits AttemptFailed for non-final failed attempts")
    Component(stream_schedule, "streaming::schedule (NEW)", "Rust", "Emits Accepted + Registered{cron, deferral_url} immediately at submit-handler ingress; stream closes (no firing semantics this slice)")
    Component(reconciler, "reconciler::JobLifecycle (EXTENDED)", "Rust", "Branches on WorkloadSpec.kind() per ADR-0037 Amendment 2026-05-10; emits per-kind TerminalCondition variants")
  }

  Rel(engineer, submit_cmd, "overdrive job submit ./spec.toml")
  Rel(submit_cmd, fs, "Reads")
  Rel(submit_cmd, parser, "Parses TOML bytes into")
  Rel(parser, submit_handler, "Sends WorkloadSpecInput via JSON over HTTPS")
  Rel(submit_handler, intent, "Persists WorkloadSpec via IntentStore::put_if_absent")
  Rel(submit_handler, stream_dispatcher, "Hands off to per-kind streaming")

  Rel(stream_dispatcher, stream_service, "Service kind →")
  Rel(stream_dispatcher, stream_job, "Job kind →")
  Rel(stream_dispatcher, stream_schedule, "Schedule kind →")

  Rel(stream_service, obs, "Subscribes to alloc_status rows")
  Rel(stream_job, obs, "Subscribes to alloc_status rows; waits for terminal exit_code")
  Rel(reconciler, stream_job, "Emits TerminalCondition::Completed/Failed via broadcast bus")

  Rel(submit_cmd, render_dispatcher, "Receives SubmitEvent stream; routes by kind tag")
  Rel(render_dispatcher, render_service, "Service kind →")
  Rel(render_dispatcher, render_job, "Job kind →")
  Rel(render_dispatcher, render_schedule, "Schedule kind →")
  Rel(render_schedule, deferrals, "Reads SCHEDULE_EXECUTION_TRACKING_URL")
  Rel(render_service, deferrals, "Reads SERVICE_VIP_ALLOCATOR_TRACKING_URL for None-vip listeners")
```

### Reading guide

- **Three branch points, one disambiguator (`WorkloadKind`)**.
  The spec parser produces it (from section presence). The
  streaming dispatcher consumes it (from intent). The render
  dispatcher consumes it again (from the kind tag on the wire).
  All three branches use the same closed enum — adding a new
  workload kind in the future means adding one variant and
  filling three exhaustive `match` arms.

- **`ConvergedRunning` lives in `stream_service` only** — the
  arrow from `stream_dispatcher` to `stream_job` does NOT carry
  a `ConvergedRunning` event. This is the structural fix for RCA
  root causes B + C.

- **`render_job` has no `format_running_summary` call site** —
  the function is reachable only from `render_service`. The
  `"live"` literal removal is a Slice 04 concern; the grep gate
  in `xtask::dst_lint` enforces it doesn't return.

- **Deferral URLs are read, not hardcoded** at the call site.
  The arrows from `render_schedule` / `render_service` to
  `deferrals` represent the constant-read; KPIs K5 and K6 rely
  on this single SSOT shape.

---

## Diagram coverage check

- [x] L1 (System Context) — unchanged; documented as such.
- [x] L2 (Container) — annotated to mark the four containers
      affected (`overdrive-core`, `overdrive-cli`,
      `overdrive-control-plane`, `xtask`).
- [x] L3 (Component) — new; shows the parser pipeline + streaming
      dispatch + render branch.
- [x] Every arrow has a verb.
- [x] No mixing of abstraction levels.
- [x] No internal class-level (L4) diagrams — feature does not
      warrant.
