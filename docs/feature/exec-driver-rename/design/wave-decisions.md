# DESIGN Wave Decisions — exec-driver-rename

**Wave**: DESIGN (solution-architect)
**Owner**: Morgan
**Date**: 2026-04-28
**Status**: COMPLETE — handoff-ready for DELIVER (software-crafter).
DISCUSS / DISTILL waves are skipped: this is internal cleanup
discovered in `phase-1-first-workload` PR review and discussed
directly with the user, who approved Option C (rename + spec
reshape) explicitly. There is no user-story / AC story map; the
roadmap's `criteria` fields carry the load.

---

## What this feature is

A focused architectural cleanup of the exec-style workload driver
shipped by `phase-1-first-workload`. Three coordinated changes,
single PR (two commits):

1. Rename `ProcessDriver` → `ExecDriver` in `overdrive-worker`
2. Rename `DriverType::Process` → `DriverType::Exec` in
   `overdrive-core`
3. Rename `AllocationSpec.image` → `AllocationSpec.command`; add
   `AllocationSpec.args: Vec<String>`; remove the magic image-name
   dispatch in `ExecDriver::build_command`

**Not** a roadmap step on `phase-1-first-workload`. That feature
is archived. This is a separate feature with its own roadmap.

## Why

**Vocabulary alignment.** Nomad calls this driver class `exec`
(https://developer.hashicorp.com/nomad/docs/deploy/task-driver/exec);
Talos uses the same vocabulary. "Process" was an internal-implementation
noun (we use `tokio::process` underneath); the operator-facing
concept is "execute a binary directly," which the wider operator
community calls `exec`. The rename costs little and aligns with the
operator's existing mental model.

**Honest field naming.** `image` is borrowed from container land
where `docker.io/library/postgres:15` is genuinely an image
identifier. For an exec driver running binaries directly,
`/bin/sleep` is a binary path, not a content-addressed image. The
`build_command` body reads `Command::new(&spec.image)` —
self-documenting evidence that the field is misnamed.

**Removing magic dispatch.** Because `AllocationSpec` cannot carry
argv today, `ExecDriver::build_command` papers over the gap with
hardcoded image-name routing:

```text
/bin/sleep   → ["60"]
/bin/sh      → ["-c", "trap '' TERM; sleep 60"]
/bin/cpuburn → busy-loop sh script
```

Production code is reading test-fixture intent. The right shape is
the spec carries argv (`args: Vec<String>`), the driver runs
`Command::new(&spec.command).args(&spec.args)`, and test fixtures
construct argv inline (the SIGTERM-trap test's `command: "/bin/sh",
args: ["-c", "trap '' TERM; sleep 60"]`; the cgroup-isolation
burst test's CPU-busy command).

## Architecture summary

**Pattern**: Hexagonal (ports and adapters), single-process, Rust
workspace. Unchanged from `phase-1-first-workload`. **No** crate
boundaries move. **No** trait method signatures change. The
`Driver` trait's surface (`start` / `stop` / `status` / `resize`),
the `overdrive-worker` crate's class (`adapter-host`), the
binary-composition pattern (`overdrive-cli` hard-depends on both
control-plane and worker), and the dependency graph
(`overdrive-core ← overdrive-worker ← overdrive-cli`) all carry
through verbatim.

**Single-cut greenfield migration** per
`feedback_single_cut_greenfield_migrations`: every fixture in the
workspace migrates in lockstep with the spec field rename. No
`#[deprecated]` aliases, no compatibility shim, no two-phase
rollout. Two cohesive commits land the rename.

## Decisions

| # | Decision | Rationale (one line) | ADR |
|---|---|---|---|
| D1 | Rename `ProcessDriver` → `ExecDriver` (`overdrive-worker`) | Aligns with Nomad `exec` driver vocabulary and Talos's terminology; "Process" was an internal-implementation noun | ADR-0029 amendment 2026-04-28 |
| D2 | Rename `DriverType::Process` → `DriverType::Exec` (`overdrive-core`) | Operator-facing identity; matches D1; future variants (`MicroVm`, `Wasm`) are operator-canonical too | ADR-0029 amendment 2026-04-28 |
| D3 | Rename `AllocationSpec.image` → `AllocationSpec.command`; add `args: Vec<String>` | Container-image terminology is wrong for an exec driver; missing args field forced magic dispatch in `build_command` (technical debt). `command` matches Nomad's `exec` task driver field name. | ADR-0026 amendment 2026-04-28 |
| D4 | Drop magic image-name dispatch in `ExecDriver::build_command`; body becomes `Command::new(&spec.command).args(&spec.args)`; setsid pre-exec hook becomes unconditional | Production code stops reading test-fixture intent; every workload gets its own process group (was already the right shape, only conditional because magic dispatch needed a switch site) | ADR-0029 amendment 2026-04-28 |

## What is NOT changing

- Crate names. `overdrive-worker` stays. The exec driver still
  lives there.
- `Driver` trait method signatures. `start` / `stop` / `status` /
  `resize` are unchanged. The spec type carried by `start`
  reshapes; the trait signature does not.
- Cgroup mechanics. ADR-0026's body — direct cgroupfs writes,
  cgroup-v2 only, `cpu.weight` + `memory.max` derivation,
  warn-and-continue posture, `mkdir → limits → cgroup.procs →
  rmdir` ordering — is all preserved verbatim.
- ADR-0028's pre-flight check.
- The `node_health` row writer's relocation to worker startup
  (ADR-0025 amendment).
- The action shim contract (ADR-0023).
- `AppState::driver: Arc<dyn Driver>` (ADR-0022). The trait-object
  type is unchanged; only the impl's struct name changes.
- The HTTP API + OpenAPI schema. The wire shape never exposed
  `image` directly — confirmed via grep of `api/openapi.yaml`,
  `api.rs`, `handlers.rs` (no matches). The CLI's job-toml
  deserialization does not reference the field either. The rename
  is purely internal.

## Migration surface (informative — non-exhaustive)

The migration touches every site in the workspace that constructs
`AllocationSpec` or matches `DriverType`. A non-exhaustive list,
to anchor the crafter's grep:

- `crates/overdrive-core/src/traits/driver.rs` — struct + enum
  variant
- `crates/overdrive-core/src/reconciler.rs` — one
  `AllocationSpec { ..., image: "/bin/sleep", ... }` site in
  `JobLifecycle::reconcile`
- `crates/overdrive-control-plane/src/action_shim.rs` —
  `build_phase1_restart_spec` constructor + `default_restart_resources`
  neighbouring helper
- `crates/overdrive-worker/src/driver.rs` — `ProcessDriver`
  struct + every internal reference + `build_command` body
- `crates/overdrive-worker/src/lib.rs` — re-exports
- `crates/overdrive-worker/tests/integration/process_driver/*.rs`
  (every file constructs `AllocationSpec`); test directory rename
  to `tests/integration/exec_driver/` and per-file test-fn name
  rename
- `crates/overdrive-worker/tests/integration.rs` — `mod` declarations
- `crates/overdrive-worker/tests/acceptance/sim_driver_only_in_default_lane.rs`
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/cluster_status_under_burst.rs`
  — currently uses `image: "/bin/cpuburn"` (magic); migrates to
  `command: "/bin/sh", args: vec!["-c", "<busy loop script>"]`
- `crates/overdrive-sim/tests/acceptance/sim_adapters_deterministic.rs`
- `crates/overdrive-cli/src/...` — only if a `serve` instantiation
  references the type by name (most binary composition is via the
  trait object, so likely no edits)

The roadmap's two steps (`01-01` for the type-name rename, `01-02`
for the spec-shape rename) decompose this surface into mechanical
units the crafter executes single-cut.

## ADR amendments produced by this DESIGN wave

- **ADR-0026** — appended `Amendment 2026-04-28 — Exec driver
  rename + AllocationSpec.args`. Documents the field rename and
  the args-add as cgroup-side narrative; explains why magic
  dispatch was technical debt; lists the migration surface; refers
  to ADR-0029's amendment for the type-rename surface.
- **ADR-0029** — appended `Amendment 2026-04-28 — Exec driver
  rename + AllocationSpec.args`. Documents the type rename
  (`ProcessDriver` → `ExecDriver`, `DriverType::Process` →
  `DriverType::Exec`), the `build_command` body cleanup, the
  test-fixture inline-args migration, and the Phase-2+ posture
  (per-driver-type spec types when `MicroVm` / `Wasm` land).
- **ADR-0021** — investigated; **no amendment required**. The
  `AnyState` enum body does not reference `image` or
  `ProcessDriver`. Confirmed via `grep` of
  `adr-0021-state-shape-for-reconciler-runtime.md`.

The amendment-in-place pattern matches the precedent ADR-0026 +
ADR-0029 already established (each was amended in place rather than
spawning a successor ADR). A new ADR-0030 for a rename would
fragment the worker-crate narrative for no benefit.

## Quality attributes — impact assessment

**Maintainability — modifiability**: positive. The exec driver's
public surface starts using the operator's existing vocabulary;
fixtures stop fighting magic dispatch.

**Maintainability — analyzability**: positive. `git grep
AllocationSpec` produces a tractable migration surface; the rename
is mechanical. `cargo doc -p overdrive-worker` will produce a
focused doc with the operator-canonical noun.

**Maintainability — testability**: positive. Test fixtures
construct argv inline and stop relying on production code to
observe magic image names — that production-reading-test-fixture
shape is the technical debt this rename retires.

**Performance**: neutral. Function calls are unchanged; the
`Command::new(...).args(...)` shape is the standard
`tokio::process` invocation.

**Reliability**: neutral. Runtime semantics unchanged; the rename
does not affect cgroup mechanics, error handling, signal
escalation, or any other behaviour observable by the operator.

**Compatibility — interoperability**: positive. A future
operator pattern reading job specs in cross-tool form (e.g. a
Nomad-spec-to-Overdrive-spec bridge) will see `command` + `args`
matching Nomad's own field names exactly — no mapping needed;
`image` would have been actively misleading in that future.

**Security**: neutral. The rename does not change the security
posture of the driver. The spec still goes through the action
shim, the SPIFFE identity is still bound to the allocation, and
the cgroup boundary is still enforced.

## Handoff annotations

**To software-crafter (DELIVER)**:

- Authoritative spec: `docs/feature/exec-driver-rename/deliver/roadmap.json`
- ADR amendments: ADR-0026 + ADR-0029 (both appended; both unstaged
  in the working tree at handoff time — the crafter commits them
  alongside the rename PR rather than landing the docs and code in
  separate PRs).
- Single-cut migration per `feedback_single_cut_greenfield_migrations`.
  No `#[deprecated]` aliases. No compatibility shim. No
  feature-flagged old name.
- Two commits, in order: (1) type-name rename
  (`ProcessDriver` → `ExecDriver`, `DriverType::Process` →
  `DriverType::Exec`), (2) spec-shape rename + magic-dispatch
  removal (`image` → `command`, add `args`, simplify
  `build_command`).
- Quality gates per step are the standard workspace gates:
  `cargo nextest run --workspace` (default lane);
  `cargo test --doc --workspace`; `cargo xtask dst`;
  `cargo xtask dst-lint`; `cargo clippy --workspace --all-targets
  --no-deps -- -D warnings`. The Lima sudo integration suite must
  stay green for step `01-02` because the cgroup-isolation burst
  test (`cluster_status_under_burst`) is in the integration lane
  and migrates from `image: "/bin/cpuburn"` (magic) to
  `command: "/bin/sh", args: vec!["-c", "<busy loop>"]`.

**To platform-architect (DEVOPS)**:

- No external integration changes. No new contract tests. No
  deployment changes. No CI changes.
- The OpenAPI schema does not change (the `image`/`command` field
  is internal — `AllocationSpec` is not part of the wire shape).
- Mutation gate ≥80% on `overdrive-worker` and `overdrive-core`
  for the touched files; the `--features integration-tests` flag
  is required for `overdrive-worker` mutation runs because the
  acceptance signal lives behind the integration-tests feature.
  On macOS, the mutation invocation routes through `cargo xtask
  lima run --` per `.claude/rules/testing.md` § Mutation testing.

---

*This DESIGN wave was conducted by Morgan in propose mode based on
user-supplied option C from `phase-1-first-workload` PR review.
DISCUSS / DISTILL waves were skipped at user direction; the
roadmap's `criteria` fields carry the AC load.*
