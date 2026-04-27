# DESIGN Wave Decisions — phase-1-first-workload

**Wave**: DESIGN (solution-architect)
**Owner**: Morgan
**Date**: 2026-04-27
**Status**: COMPLETE — handoff-ready for DISTILL (acceptance-designer),
pending peer review by Atlas (`nw-solution-architect-reviewer`).

---

## Key decisions

Nine ratified decisions from the propose-mode pass plus one
post-ratification amendment (D10). Eight of D1–D9 are as Morgan
recommended; **D4 was overridden by the user** in favour of the
dedicated-crate option. D10 is a user-proposed extraction ratified
2026-04-27 the same day, lifting `ProcessDriver` + workload-cgroup
management + the `node_health` writer out of `overdrive-host` into a
new dedicated `overdrive-worker` crate.

| # | Decision | Rationale (one line) | ADR |
|---|---|---|---|
| D1 | `AnyState` enum mirroring `AnyReconcilerView`, with `desired`+`actual` collapsed into one `JobLifecycleState` struct | Symmetric with the existing View story; per-tick I/O proportional to the running reconciler; compile-time exhaustiveness | ADR-0021 |
| D2 | `AppState::driver: Arc<dyn Driver>` field; mechanical migration of every `run_server_with_obs` test caller | Single seam for action-shim driver access; trait-object swap shape preserved | ADR-0022 (amended 2026-04-27 by ADR-0029) |
| D3 | `reconciler_runtime::action_shim` submodule; 100 ms tick cadence in production; DST drives ticks explicitly | Action shim is the single async I/O boundary in the convergence loop; level-triggered drain via the existing broker | ADR-0023 (amended 2026-04-27 by ADR-0029) |
| **D4 OVERRIDE** | **Dedicated `overdrive-scheduler` crate (class `core`)** — option (b), not the originally-proposed module inside `overdrive-control-plane` | dst-lint mechanically enforces the BTreeMap-only iteration discipline + banned-API contract; convention erodes, mechanical enforcement does not | **ADR-0024** |
| D5 | Hostname fallback with optional `[node].id` config override; one-shot `node_health` write at boot; `Region("local")` default | Single-node honest, not special-cased; Phase 2+ multi-node is additive; operator escape hatches are explicit | ADR-0025 (amended 2026-04-27 by ADR-0029) |
| D6 | Direct cgroupfs writes; cgroup v2 ONLY; no `cgroups-rs` dep | Five fs operations don't earn a dep; operator-debuggable (every op maps to a shell command) | ADR-0026 (amended 2026-04-27 by ADR-0029) |
| D7 | `POST /v1/jobs/{id}:stop`; sub-decision s1: separate `IntentKey::for_job_stop` intent key | AIP-136 verb-suffix composes with future `:start`/`:restart`/`:cancel`; spec stays readable; honest audit trail | ADR-0027 |
| D8 | Hard refusal with explicit `--allow-no-cgroups` dev flag | §4 isolation claim is honest; production safe by default; dev escape hatch absorbs ergonomics objection | ADR-0028 |
| D9 | Write `cpu.weight` + `memory.max` in Slice 2 from `AllocationSpec::resources`; warn-and-continue on limit-write failure | Spec field does what it says; partial isolation is recoverable, full isolation is fatal | ADR-0026 (amended 2026-04-27 by ADR-0029) |
| **D10 AMENDMENT** | **Dedicated `overdrive-worker` crate (class `adapter-host`)** — `ProcessDriver` + workload-cgroup management + `node_health` writer extracted from `overdrive-host`. Composition pattern: binary-composition (`overdrive-cli` hard-depends on both control-plane and worker; runtime `[node] role` config selects which subsystems boot). `overdrive-control-plane` does NOT depend on `overdrive-worker`. `overdrive-host` shrinks back to ADR-0016's host-OS-primitives intent. | Phase 1 paper-only is the cheapest moment for the extraction; Phase 2+ multi-node split forces it anyway; matches whitepaper §3 control-plane vs node-agent boundary; mirrors ADR-0024 strategic precedent one level up | **ADR-0029** |

---

## Architecture summary

**Pattern**: Hexagonal (ports and adapters), single-process, Rust
workspace. The Phase 1 first-workload feature extends — does not
revise — the established pattern. The reconciler / workflow split
from whitepaper §18 is the central organising principle of the
control-plane crate; the dedicated `overdrive-scheduler` crate
extracts pure-function placement logic into a `core`-class peer of
`overdrive-core`.

**Paradigm**: OOP (Rust trait-based). No paradigm change.
Architecture pattern, error story, and concurrency discipline all
inherit from `phase-1-control-plane-core` verbatim.

**Key components introduced by this feature**:

- **`overdrive-scheduler` crate** (NEW, class `core`) —
  pure-function placement.
- **`overdrive-worker` crate** (NEW, class `adapter-host`,
  per ADR-0029) — hosts `ProcessDriver`, workload-cgroup management,
  and the boot-time `node_health` row writer.
- **`AnyState` enum** in `overdrive-core::reconciler` —
  per-reconciler typed `desired`/`actual` projection.
- **`JobLifecycle` reconciler** in `overdrive-control-plane` — the
  first real (non-`NoopHeartbeat`) reconciler.
- **`reconciler_runtime::action_shim`** submodule in
  `overdrive-control-plane` — the single async I/O boundary in
  the convergence loop.
- **`ProcessDriver`** in `overdrive-worker` — Linux-only, cgroup v2
  direct writes (relocated from `overdrive-host` per ADR-0029).
- **Worker-startup `node_health` row writer** in `overdrive-worker`
  — the implicit single-node identity (relocated from control-plane
  bootstrap per ADR-0025 amendment).
- **Control-plane cgroup management + pre-flight** in
  `overdrive-control-plane` — boot-time hard-refusal on
  delegation gap; control-plane slice enrolment; workload half of
  the cgroup hierarchy lives in `overdrive-worker` per ADR-0029.
- **`POST /v1/jobs/{id}:stop` handler** + CLI subcommand — the
  inverse of `submit`, end-to-end.
- **Binary-composition pattern** in `overdrive-cli`'s `serve`
  subcommand — hard-depends on both `overdrive-control-plane` and
  `overdrive-worker`; runtime `[node] role` config selects which
  subsystems boot (ADR-0029).

### Crate inventory delta

```
Before (phase-1-control-plane-core):
  overdrive-core         (core)
  overdrive-store-local  (adapter-host)
  overdrive-host         (adapter-host)
  overdrive-control-plane (adapter-host)
  overdrive-sim          (adapter-sim)
  overdrive-cli          (binary)
  xtask                  (binary)

After (phase-1-first-workload):
  overdrive-core         (core)
  overdrive-scheduler    (core)              ← NEW (D4 override; ADR-0024)
  overdrive-store-local  (adapter-host)
  overdrive-host         (adapter-host)      ← unchanged at app-arch level;
                                              ADR-0016 host-OS-primitives intent
                                              preserved per ADR-0029
  overdrive-worker       (adapter-host)      ← NEW (D10 amendment; ADR-0029).
                                              ProcessDriver + workload-cgroup
                                              management + node_health writer
  overdrive-control-plane (adapter-host)     ← extended: action shim, JobLifecycle,
                                              control-plane cgroup mgmt + preflight,
                                              :stop handler, AppState::driver
                                              (workload cgroup mgmt + node_health
                                              writer relocated to overdrive-worker
                                              per ADR-0029)
  overdrive-sim          (adapter-sim)
  overdrive-cli          (binary)            ← extended: job stop subcommand;
                                              `serve` becomes binary-composition
                                              root (ADR-0029)
  xtask                  (binary)
```

`dst-lint` core-class set grows from one (`overdrive-core`) to two
(`overdrive-core`, `overdrive-scheduler`); `overdrive-worker` is
class `adapter-host` and not scanned. Workspace Rust crate count
(excluding `xtask`) grows from seven to eight.

---

## Reuse Analysis

| Concern | Existing artifact | Disposition | Justification |
|---|---|---|---|
| Reconciler trait + runtime | `overdrive-core::reconciler::Reconciler`, `overdrive-control-plane::reconciler_runtime::ReconcilerRuntime` | EXTEND | The trait gains `type State`; the runtime gains hydrate_desired/hydrate_actual + action shim invocation. No replacement. |
| Action enum | `overdrive-core::reconciler::Action` (Phase 1 ships Noop + HttpCall + StartWorkflow) | EXTEND | Three new variants: `StartAllocation`, `StopAllocation`, `RestartAllocation`. Additive. |
| AnyReconciler enum-dispatch | `overdrive-core::reconciler::{AnyReconciler, AnyReconcilerView}` | EXTEND | New `JobLifecycle` variant on both; new sister `AnyState` enum (ADR-0021). |
| Driver trait | `overdrive-core::traits::driver::Driver` (`async fn start/stop/status/resize`) | REUSE AS-IS | Trait surface is correct for Phase 1; ProcessDriver implements it directly. |
| `SimDriver` | `overdrive-sim::driver::SimDriver` | REUSE AS-IS | DST + default-lane test fixture; first-workload exercises it via the new reconciler. |
| AllocationSpec / Handle / State | `overdrive-core::traits::driver::{AllocationSpec, AllocationHandle, AllocationState}` | REUSE AS-IS | ProcessDriver's signature consumes these directly. |
| Resources newtype | `overdrive-core::traits::driver::Resources` | REUSE AS-IS | Scheduler arithmetic; cgroup limit derivations. |
| `Job` / `Node` aggregates | `overdrive-core::aggregate::{Job, Node}` | REUSE AS-IS | Phase 1 first-workload ships ZERO schema changes. The 2026-04-27 scope correction pulled all field additions. |
| `AllocStatusRow` | `overdrive-core::traits::observation_store::AllocStatusRow` | REUSE AS-IS | Action shim writes; reconciler's hydrate_actual reads. |
| `TickContext` | `overdrive-core::reconciler::TickContext` | REUSE AS-IS | The action shim receives the same `&TickContext` the reconciler did. |
| IntentStore + ObservationStore traits | `overdrive-core::traits::*` | REUSE AS-IS | No trait surface changes; new key + row use the existing put/get/write/read APIs. |
| `LocalIntentStore` (redb) | `overdrive-store-local::LocalIntentStore` | REUSE AS-IS | New `IntentKey::for_job_stop` is just another key; `put_if_absent` semantics handle the idempotent re-stop case. |
| `LocalObservationStore` (redb) | `overdrive-store-local::LocalObservationStore` | REUSE AS-IS | New `node_health` row at boot; new `alloc_status_rows` writes from the action shim. Same trait surface. |
| EvaluationBroker | `overdrive-control-plane::reconciler_runtime::eval_broker` | REUSE AS-IS | Cancelable-eval-set semantics already correct for the JobLifecycle key shape `(job-lifecycle, jobs/<id>)`. |
| LibsqlProvisioner | `overdrive-control-plane::libsql_provisioner` | REUSE AS-IS | New `JobLifecycle` reconciler gets its DB at `<data_dir>/reconcilers/job-lifecycle/memory.db`. |
| AppState struct | `overdrive-control-plane::AppState` | EXTEND | New `driver: Arc<dyn Driver>` field (ADR-0022). Existing fields preserved. |
| `run_server_with_obs` entry point | `overdrive-control-plane::run_server_with_obs` | EXTEND (rename) | Becomes `run_server_with_obs_and_driver`; every test caller migrated mechanically. |
| TLS bootstrap | `overdrive-control-plane::tls_bootstrap` | REUSE AS-IS | The boot sequence's TLS step is unchanged; cgroup pre-flight prepends. |
| OpenAPI schema generation | `cargo xtask openapi-gen/openapi-check` | EXTEND | New `:stop` endpoint surfaces in regenerated schema; CI gate catches drift. |
| CLI HTTP client | `overdrive-cli::client` | EXTEND | New `JobStopRequest`/`Response` types imported from `overdrive-control-plane::api`; new client method. |
| ControlPlaneError | `overdrive-control-plane::error::ControlPlaneError` | EXTEND | New `Driver(#[from] DriverError)` and `Cgroup(#[from] CgroupPreflightError)` variants (additive). |
| **Scheduler (placement function)** | NONE | **CREATE NEW (D4 OVERRIDE)** | **Dedicated `overdrive-scheduler` crate, class `core`. Originally proposed as a module inside `overdrive-control-plane`; user override chose option (b) for `dst-lint` mechanical enforcement of BTreeMap-only iteration + banned-API discipline. ADR-0024.** |
| **Worker subsystem crate** | NONE | **CREATE NEW (D10 AMENDMENT)** | **Dedicated `overdrive-worker` crate, class `adapter-host`, per ADR-0029. Hosts ProcessDriver + workload-cgroup management + node_health writer. Mirrors whitepaper §3 control-plane vs node-agent boundary; matches ADR-0024 strategic precedent one level up.** |
| Action shim | NONE | CREATE NEW | New `reconciler_runtime::action_shim` submodule. ADR-0023. |
| JobLifecycle reconciler | NONE | CREATE NEW | First real reconciler; lives in `overdrive-control-plane::reconciler::job_lifecycle`. |
| ProcessDriver | NONE | CREATE NEW | Lives in `overdrive-worker::driver::process` (Linux-only) per ADR-0029, mirroring whitepaper §3 control-plane vs worker split (formerly slated for `overdrive-host` by DISCUSS pre-decision). ADR-0026 (amended). |
| Workload-cgroup management | NONE | CREATE NEW | Lives in `overdrive-worker` per ADR-0029 (the workload half of ADR-0026's cgroup work; control-plane cgroup management stays in `overdrive-control-plane`). |
| Control-plane cgroup management + pre-flight module | NONE | CREATE NEW | `overdrive-control-plane::cgroup_manager` (control-plane half) + `overdrive-control-plane::cgroup_preflight`. ADR-0028 + ADR-0026 amendment. |
| Worker-startup node_health writer | NONE | CREATE NEW | Lives in `overdrive-worker` per ADR-0029, runs at worker subsystem startup before listener bind (ADR-0025 amendment relocates from control-plane bootstrap). |
| `IntentKey::for_job_stop` constructor | NONE | CREATE NEW | One associated function on the existing `IntentKey` newtype. ADR-0027. |
| `NodeId::from_hostname` constructor | NONE | CREATE NEW | One associated function on the existing `NodeId` newtype. ADR-0025. |
| `[node]` config block | NONE | CREATE NEW | TOML parser extension; additive. ADR-0025. |
| `--allow-no-cgroups` CLI flag | NONE | CREATE NEW | `clap` flag on the existing `serve` subcommand. ADR-0028. |
| `JobLifecycleView` libSQL schema | NONE | CREATE NEW | `restart_counts` + `next_attempt_at` tables; managed inline by `JobLifecycle::hydrate`. |
| Three new DST invariants | NONE | CREATE NEW | `JobScheduledAfterSubmission`, `DesiredReplicaCountConverges`, `NoDoubleScheduling` in `overdrive-sim::invariants`. |

---

## Technology stack

No new external dependencies are added by this feature. The complete
stack:

| Component | Origin | License | Notes |
|---|---|---|---|
| Rust 2024 + tokio + serde + thiserror | workspace | MIT/Apache-2 | Inherited from prior phases. |
| `axum` + `axum-server` + `rustls` + `hyper` | workspace | MIT/Apache-2 | Inherited (ADR-0008). |
| `utoipa` + `utoipa-axum` | workspace | MIT/Apache-2 | Inherited (ADR-0009); regenerated for new `:stop` endpoint. |
| `libsql` | workspace | MIT | Inherited (ADR-0013); new reconciler gets its own per-primitive DB. |
| `redb` | workspace | MIT/Apache-2 | Inherited; new IntentStore key + observation row. |
| `rkyv` + `bytecheck` | workspace | MIT | Inherited (ADR-0011). |
| `tokio::process` | tokio (workspace) | MIT | ProcessDriver fork/exec/wait — already in the dep graph. |
| `std::fs` + `std::os::unix::*` | std | — | Cgroup direct writes (ADR-0026). No new dep. |
| `hostname` | workspace | MIT/Apache-2 | NodeId::from_hostname (ADR-0025). Already in workspace deps for the existing TLS bootstrap. |
| `clap` | workspace | MIT/Apache-2 | New `--allow-no-cgroups` flag on `overdrive serve`. |

**Notably absent**: no `cgroups-rs`, no `nix`, no FFI, no Go, no
C++. ADR-0026 deliberately rejects `cgroups-rs` in favour of direct
`std::fs` writes; the five filesystem operations do not earn the
dep cost.

**Rust-throughout discipline preserved**. Whitepaper principle 7
(no FFI to Go or C++ in the critical path) holds.

---

## Constraints established

The eight hard constraints from DISCUSS carry forward verbatim. They
are not debatable within this wave and gate every later wave's
work:

1. **Phase 1 is single-node** — no node registration, no
   taint/toleration, no multi-region. There is exactly one node (the
   local host), implicit. No operator-facing node-registration verb.
2. **Reconciler purity is non-negotiable**. The lifecycle reconciler
   MUST satisfy the existing `ReconcilerIsPure` DST invariant — no
   `.await`, no wall-clock reads, no direct store writes inside
   `reconcile`. `tick.now` only.
3. **Scheduler determinism is load-bearing**. All internal
   collections driving iteration are `BTreeMap`. `dst-lint`
   mechanically enforces this in `overdrive-scheduler` per ADR-0024.
4. **STRICT newtypes for new identifiers**. Phase 1 first-workload
   ships ZERO new identifier newtypes (the originally-proposed
   `CgroupPath` is internal to `ProcessDriver` and can be deferred
   to crafter judgement; the discipline still applies if it lands).
5. **No new fields on existing aggregates**. `Node` and `Job` ship
   unchanged. The aggregate-roundtrip proptest continues to pass
   byte-identical.
6. **Real-infrastructure tests gated `integration-tests`**.
   Default lane uses `SimDriver`; real processes / cgroups / sockets
   live behind the feature flag.
7. **Action shim is the single I/O boundary in the convergence
   loop**. Lifecycle reconciler emits Actions (data); shim
   dispatches to `Driver::start` / `Driver::stop` (I/O).
8. **Linux-only for cgroups**. macOS / Windows hosts run
   default-lane `SimDriver`; integration tests require a Linux VM.

---

## Slice gating

Per the DISCUSS-wave story map and dor-validation:

- **Slice 1** (US-01: First-fit scheduler) — UNBLOCKED. Lands in
  `overdrive-scheduler` crate per ADR-0024. Parallelisable with
  Slice 2.
- **Slice 2** (US-02: ProcessDriver) — UNBLOCKED. Lands in
  `overdrive-host::driver::process` per ADR-0026. Parallelisable
  with Slice 1. Linux-only `integration-tests`-gated tests.
- **Slice 3** (US-03: JobLifecycle reconciler + action shim +
  `job stop`) — **GATED on Slices 1 + 2 + ADR-0021**. ADR-0021 is
  the State-shape blocker DoR flagged. With ADR-0021 ratified, the
  blocker is resolved; Slice 3 is unblocked once Slices 1 and 2
  land.
- **Slice 4** (US-04: Control-plane cgroup isolation) — GATED on
  Slices 2 + 3. Slice 2 supplies the cgroup write primitives; Slice
  3 supplies a real workload to assert against in the burst test.

The DoR's pre-described 3A / 3B split (StartAllocation only +
JobScheduledAfterSubmission, then StopAllocation + RestartAllocation
+ backoff + the rest) remains available as a crafter-time escape
hatch if material complexity surfaces during DELIVER. ADR-0021
does NOT prescribe the split; it just unblocks the merged slice.

---

## Upstream changes

**None**. No SSOT artifact (whitepaper, brief.md prior sections,
prior ADRs, DISCUSS user stories) is invalidated by this DESIGN's
decisions. All eight ADRs are additive — they add new sections, new
crates, new modules, new endpoint, new config block, new variants.
No prior assumption is changed.

The `### Changed Assumptions` block in brief.md is therefore
intentionally absent from this wave's extension; the existing
assumptions hold.

`docs/feature/phase-1-first-workload/design/upstream-changes.md` is
NOT created (the file is required only when assumptions change).

---

## ADR list

| ADR | Title | Decision |
|---|---|---|
| ADR-0021 | Reconciler `State` shape | Per-reconciler typed `AnyState` enum mirroring `AnyReconcilerView`; `desired`+`actual` collapsed into one `JobLifecycleState` struct; runtime owns hydrate_desired/hydrate_actual. |
| ADR-0022 | `AppState::driver` extension | New `driver: Arc<dyn Driver>` field; production wires `ProcessDriver`, tests wire `SimDriver`; `run_server_with_obs` becomes `run_server_with_obs_and_driver`. |
| ADR-0023 | Action shim placement | `overdrive-control-plane::reconciler_runtime::action_shim` submodule; signature `dispatch(actions, &dyn Driver, &dyn ObservationStore, &TickContext)`; 100 ms tick cadence in production via injected `Clock`; DST drives ticks explicitly. |
| ADR-0024 | `overdrive-scheduler` crate (D4 OVERRIDE) | Dedicated crate, class `core`; depends only on `overdrive-core`; `dst-lint`-scanned. Originally proposed as a module inside `overdrive-control-plane`; user override chose option (b) for mechanical BTreeMap-only enforcement. Dep direction: `overdrive-core ← overdrive-scheduler ← overdrive-control-plane`. |
| ADR-0025 | Single-node startup wiring | Hostname-derived `NodeId` with optional `[node].id` override; `Region("local")` default; one-shot `node_health` row write at boot before listener binds. |
| ADR-0026 | cgroup v2 direct writes | Direct cgroupfs writes via `std::fs`; no `cgroups-rs` dep; cgroup v2 ONLY (operator confirmed); resource enforcement via `cpu.weight` + `memory.max` from `AllocationSpec::resources` in Slice 2; warn-and-continue on limit-write failure. |
| ADR-0027 | Job-stop HTTP shape | `POST /v1/jobs/{id}:stop` (AIP-136 verb-suffix); separate `IntentKey::for_job_stop` intent key (canonical form `jobs/<id>/stop`); reconciler reads both keys; spec stays readable via `GET /v1/jobs/{id}` for audit. |
| ADR-0028 | cgroup pre-flight refusal | Hard refusal at boot on missing cgroup v2 delegation; explicit `--allow-no-cgroups` dev escape hatch with loud startup banner; cgroup v1 hosts get an actionable error. |
| ADR-0029 | Dedicated `overdrive-worker` crate (post-ratification amendment) | Class `adapter-host`. Hosts ProcessDriver + workload-cgroup management + node_health writer. Composition pattern: binary-composition — `overdrive-cli`'s `serve` subcommand hard-depends on both control-plane and worker; runtime `[node] role` config selects which subsystems boot. `overdrive-control-plane` does NOT depend on `overdrive-worker`. `overdrive-host` shrinks back to ADR-0016's host-OS-primitives intent. ADRs 0022, 0023, 0025, 0026 amended in-place. |

---

## Open questions for DISTILL

**None blocking handoff.** The State-shape blocker that DoR flagged
as a HARD DESIGN dependency is resolved by ADR-0021. Every Priority
Zero / One / Two question from DISCUSS is now answered:

- **Priority Zero** — State shape: ADR-0021.
- **Priority One** — AppState::driver: ADR-0022; action shim
  placement: ADR-0023; scheduler crate boundary: ADR-0024
  (overridden); single-node startup: ADR-0025.
- **Priority Two** — cgroup mechanism: ADR-0026; `job stop` HTTP
  shape: ADR-0027; pre-flight level: ADR-0028; resource
  enforcement: ADR-0026.

DISTILL inherits the AC from US-01 / US-02 / US-03 / US-04 verbatim.
The acceptance scenarios reference well-known surfaces (`schedule`,
`Driver::start/stop`, `Action::*`, `JobLifecycleView`, `AllocStatusRow`,
HTTP path shapes) — every name is now backed by a ratified
architectural decision.

---

## Handoff package for DISTILL (acceptance-designer)

- `docs/product/architecture/brief.md` — extended with §24–§33 +
  new C4 Container diagram + new C4 Component diagram (convergence-
  loop closure).
- `docs/product/architecture/adr-0021..0028.md` — eight new ADRs.
- `docs/feature/phase-1-first-workload/design/wave-decisions.md`
  (this file).
- `docs/feature/phase-1-first-workload/discuss/user-stories.md` —
  inherited verbatim from DISCUSS; DISTILL writes test scenarios
  against US-01..04 AC.
- Reference: every architectural surface DISTILL's scenarios will
  cite (`schedule`, `JobLifecycle::reconcile`, action shim,
  ProcessDriver, cgroup pre-flight) is now named in an ADR with
  stable signatures.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-27 | Initial DESIGN wave for `phase-1-first-workload`. Proposed eight ADRs; ratified with one user override on D4 (dedicated `overdrive-scheduler` crate, not module inside `overdrive-control-plane`). All eight ADRs Accepted on ratification per project ADR convention. — Morgan. |
| 2026-04-27 | Post-ratification amendment: D10 (dedicated `overdrive-worker` crate, class `adapter-host`). User-proposed extraction lifting `ProcessDriver` + workload-cgroup management + `node_health` writer out of `overdrive-host` into a new dedicated crate. Composition pattern: binary-composition (`overdrive-cli` hard-depends on both `overdrive-control-plane` and `overdrive-worker`; runtime `[node] role` config selects which subsystems boot). `overdrive-control-plane` does NOT depend on `overdrive-worker` — the action shim calls `Driver::*` against an injected `&dyn Driver`, impl plugged in by the binary at AppState construction. `overdrive-host` shrinks back to ADR-0016's host-OS-primitives intent (`SystemClock`, `OsEntropy`, `TcpTransport`). ADRs 0022 / 0023 / 0025 / 0026 amended in-place (Amendment subsections at the end), preserving original bodies; structural shape unchanged in each. New ADR-0029 documents the extraction. brief.md updated: §3 Crate topology + §3 Phase 1 first-workload extension + §25 + §28 + §29 + §30 (cgroup hierarchy ownership) + ADR index + C4 Container diagram (new `overdrive-worker` container + binary-composition arrows from `overdrive-cli`) + C4 Component (convergence-loop) diagram (ProcessDriver moves to worker boundary; node_health writer added) + Changelog. Slice 2 brief updated: ProcessDriver path → `crates/overdrive-worker/src/driver/process.rs`; integration-test path → `crates/overdrive-worker/tests/integration/process_driver.rs`. Slice gating order unchanged — only crates move. — Morgan. |
