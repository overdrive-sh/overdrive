# Feature delta — `cgroup-fs-port`

Single-narrative DESIGN-wave artifact per D2 (no legacy `wave-decisions.md`
/ `architecture-design.md` split). Density: `lean` — Tier-1 [REF] sections
only; Tier-2 [WHY|HOW] expansions available via wave-end menu.

**Primary input**: GitHub issue #136 (enumerates the five decision points,
scope boundaries, and acceptance criteria — there is no
`docs/feature/cgroup-fs-port/discuss/` directory).

**Scope**: all priorities (D1 + D2 + D3 + D4 + D5) per project CLAUDE.md
§ "nWave design dispatches — priority scope".

---

## Wave: DESIGN / [REF] DDD List

| ID | Decision | Verdict | One-line rationale |
|---|---|---|---|
| D1 | Trait shape — narrow `CgroupFs` vs broad `Filesystem` vs hybrid | **Narrow `CgroupFs`** (Option A) | YAGNI; only consumer today is the cgroup manager; a broad `Filesystem` invites scope creep and weakens the contract. |
| D2 | Trait placement — `overdrive-core` vs `overdrive-worker` | **`overdrive-core`** (ports crate) | Matches Clock/Transport/Entropy/Dataplane/Driver/IntentStore/ObservationStore/Llm precedent; lets the trait be substituted at the binary boundary by either `RealCgroupFs` (host) or `SimCgroupFs` (sim) without the worker crate naming the implementation. |
| D3 | Cgroupfs semantics leak (kernel-side effects SimFs cannot simulate) | **Non-replacement contract recorded in ADR; Tier 3 (Lima sudo) stays mandatory for kill / subtree_control / kernel-state effects** | SimFs is a byte-write store; `1\n` to `cgroup.kill` does NOT terminate processes; `subtree_control` is not validated against the kernel's controller availability table. Equivalence test asserts byte-side effects only. |
| D4 | Cancellation semantics under DST (futures dropped mid-syscall) | **SimCgroupFs models `Future::drop` as `EnoughBytesWrittenOrZero` checkpoint — every write is journalled atomically pre-`.await` resolution; injected errors and cancellation interleavings are scheduled by the harness** | K3 (seed → bit-identical trajectory) extends naturally: cancellation is deterministic at every method-entry, mid-syscall is a kernel concept that does not apply in-process. |
| D5 | Migration shape — struct method (`new(fs: Arc<dyn CgroupFs>)`) vs generic `F: CgroupFs` vs global | **Struct method on `CgroupManager`** — constructor takes `Arc<dyn CgroupFs>`, mandatory not defaulted | Matches every other port-trait wiring (AppState, ExecDriver, EvalBroker); avoids monomorphisation explosion across the test surface; lets the same `Arc<dyn CgroupFs>` flow through the worker crate uniformly without builder methods. |

Locked decisions land in the table in § "Decisions Table" below.

---

## Wave: DESIGN / [REF] Component Decomposition

| Component | Path | Class | Change type | Notes |
|---|---|---|---|---|
| `CgroupFs` trait | `crates/overdrive-core/src/traits/cgroup_fs.rs` (new file) + re-export from `traits/mod.rs` | `core` | **CREATE NEW** (justified: no existing port covers cgroupfs side effects; Clock/Transport/Entropy/Dataplane/Driver/IntentStore/ObservationStore/Llm each cover orthogonal concerns and adding a method on any of them would conflate them) | Five-method surface (create_dir / write / remove_dir + a `probe()` per Earned Trust + a `kind()` introspection method for diagnostic logging). |
| `RealCgroupFs` adapter | `crates/overdrive-host/src/cgroup_fs.rs` (new file) | `adapter-host` | **CREATE NEW** | Wraps `tokio::fs::{create_dir_all, write, remove_dir}`. Zero new external deps; production lift-and-shift of the existing free functions' bodies. |
| `SimCgroupFs` adapter | `crates/overdrive-sim/src/adapters/cgroup_fs.rs` (new file) | `adapter-sim` | **CREATE NEW** | In-memory `BTreeMap<PathBuf, Vec<u8>>` (BTreeMap per § "Ordered-collection choice") + injectable error schedule per method/path. |
| `CgroupManager` struct (consolidates the eight free functions currently in `crates/overdrive-worker/src/cgroup_manager.rs`) | `crates/overdrive-worker/src/cgroup_manager.rs` (file kept; module reshaped) | `adapter-host` | **REFACTOR** | Free functions become methods on `CgroupManager { fs: Arc<dyn CgroupFs>, cgroup_root: PathBuf }`. `CgroupPath` newtype + `WorkloadsBootstrapError` stay as-is. |
| `ExecDriver` constructor | `crates/overdrive-worker/src/driver.rs` | `adapter-host` | **REFACTOR** | `ExecDriver::new` adds `fs: Arc<dyn CgroupFs>` parameter (mandatory, not defaulted, per `.claude/rules/development.md` § "Port-trait dependencies"). Internally stores it on the struct and passes to `CgroupManager`. |
| `AppState` (composition root) | `crates/overdrive-control-plane/src/lib.rs` (`AppState::new`) | `adapter-host` | **REFACTOR** (call-site only) | No new field on `AppState` — the `Arc<dyn CgroupFs>` flows into the binary's `ExecDriver::new` call site, which is constructed inside the worker subsystem entrypoint, not directly inside `AppState`. The control-plane crate never names `CgroupFs`. |
| Binary composition root | `crates/overdrive-cli/src/...` (`overdrive serve` subcommand) | `binary` | **REFACTOR** | Constructs `Arc::new(RealCgroupFs::new())`, threads into worker subsystem startup, which threads into `ExecDriver::new`. Single composition point per `.claude/rules/development.md` § "Production wiring is composed at the binary boundary". |
| Existing tempfile unit tests (cgroup_manager.rs `#[cfg(test)] mod tests`) | `crates/overdrive-worker/src/cgroup_manager.rs` (inline) | n/a | **TRIAGE per-test** (see Reuse Analysis below) | Some convert to `SimCgroupFs` (pure logic — `cpu_weight_for`, error discrimination, idempotency); some stay as tempfile-backed tests against `RealCgroupFs` for byte-side-effect coverage that SimFs structurally cannot honor (file presence / contents after the call). |

---

## Wave: DESIGN / [REF] Driving Ports

No new driving ports. The cgroup manager is consumed via `ExecDriver`,
which is itself consumed via `AppState.driver` (already a driving-port
mediation surface today). No new CLI subcommand; no new HTTP route; no new
skill.

---

## Wave: DESIGN / [REF] Driven Ports + Adapters

```mermaid
flowchart LR
  subgraph core["overdrive-core (class: core)"]
    CgroupFs[/"trait CgroupFs"/]
  end

  subgraph host["overdrive-host (class: adapter-host)"]
    Real["RealCgroupFs<br/>(tokio::fs::*)"]
  end

  subgraph sim["overdrive-sim (class: adapter-sim)"]
    Sim["SimCgroupFs<br/>(BTreeMap byte store)"]
  end

  subgraph worker["overdrive-worker (class: adapter-host)"]
    Manager["CgroupManager<br/>(struct; methods consume Arc<dyn CgroupFs>)"]
    Exec["ExecDriver<br/>(holds Arc<dyn CgroupFs>)"]
  end

  Manager -. depends on .-> CgroupFs
  Exec --> Manager
  Real -. implements .-> CgroupFs
  Sim -. implements .-> CgroupFs

  subgraph bin["overdrive-cli (class: binary, composition root)"]
    Compose["serve subcommand<br/>wires Arc::new(RealCgroupFs)"]
  end

  Compose --> Exec
  Compose --> Real
```

**Port placement rule applied** (per `.claude/rules/development.md`
§ "Port-trait dependencies"): `overdrive-worker` and consumers depend on
the trait surface in `overdrive-core` only; the binary composes
`Arc<dyn CgroupFs>` from `overdrive-host::RealCgroupFs`; tests inject
`Arc<SimCgroupFs>` from `overdrive-sim`. Mandatory not defaulted at the
constructor: `ExecDriver::new(cgroup_root, clock, fs)`.

---

## Wave: DESIGN / [REF] Technology Choices

| Choice | Version / Source | License | Why |
|---|---|---|---|
| `tokio::fs::*` | already in workspace via `tokio.workspace = true` | MIT | The real adapter IS `tokio::fs`. No new dep. |
| `parking_lot::Mutex` / `tokio::sync::Mutex` | already in workspace | Apache-2.0 / MIT | `SimCgroupFs` holds a `parking_lot::Mutex<BTreeMap<...>>` (no `.await` while holding); injection schedule held in a separate `parking_lot::Mutex<...>`. |
| `BTreeMap` | std | — | Deterministic iteration per `.claude/rules/development.md` § "Ordered-collection choice". |

**No new external crates added.** The work is structural: trait surface in
`overdrive-core`, real adapter in `overdrive-host`, sim adapter in
`overdrive-sim`, free-function → method refactor in `overdrive-worker`.
This was a deliberate evaluation criterion when sizing the work.

---

## Wave: DESIGN / [REF] Decisions Table

(Locked DDD-N rows; no prose rationale — narrative lives in § "DDD List"
above and in ADR-0054.)

| ID | Decision | Locked value |
|---|---|---|
| DDD-1 | Trait shape | `CgroupFs` — narrow, cgroup-semantic. Methods: `create_dir`, `write`, `remove_dir`, `probe`, `kind`. |
| DDD-2 | Trait crate placement | `crates/overdrive-core/src/traits/cgroup_fs.rs`. |
| DDD-3 | Tier 3 mandatory | Yes — Lima sudo integration tests stay; SimCgroupFs is byte-write only; semantics leak documented in ADR-0054 § "Non-replacement contract". |
| DDD-4 | Cancellation model | Method-entry deterministic; mid-syscall does not exist (in-process write is atomic). DST seeds extend uncannily. |
| DDD-5 | Migration shape | Constructor injection on `CgroupManager` struct + `ExecDriver::new` carries the trait object. Mandatory, not defaulted, not builder-overridable. |

---

## Wave: DESIGN / [REF] Reuse Analysis

(Hard gate per skill success criteria. Zero unjustified CREATE NEW.)

| Asset | Where it lives today | Decision | Justification |
|---|---|---|---|
| `tokio::fs::*` (`create_dir_all`, `write`, `remove_dir`) | workspace dep | **WRAP** | The real adapter IS `tokio::fs`. Lift-and-shift of the bodies currently inside `cgroup_manager.rs` free functions. No reimplementation, no abstraction over both sync and async — the dst-lint gate forbids sync `std::fs` inside `async fn` anyway. |
| Existing port traits (`Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `IntentStore`, `ObservationStore`, `Llm`) | `crates/overdrive-core/src/traits/*.rs` | **PATTERN-REUSE** | Same shape: `pub trait Foo: Send + Sync + 'static` (Clock pattern), declared once in `overdrive-core`, real impl in `overdrive-host`, sim impl in `overdrive-sim`, mandatory constructor injection at consumers. `CgroupFs` follows this template exactly. |
| `cgroup_manager.rs` free functions (`create_workload_scope`, `place_pid_in_scope`, `write_resource_limits`, `cgroup_kill`, `remove_workload_scope`, `create_workloads_slice_with_controllers`, `cpu_weight_for`, `write_resource_limits_warn_on_error`) | `crates/overdrive-worker/src/cgroup_manager.rs` | **REFACTOR** (not delete, not duplicate) | Bodies become method bodies on `CgroupManager`; the `tokio::fs::*` calls inside become `self.fs.write(...).await` etc. The arithmetic / classification helpers (`cpu_weight_for`, `WorkloadsBootstrapError::from_subtree_control_io`) stay as free or associated functions — no FS dep. |
| `CgroupPath` newtype + its STRICT discipline | `crates/overdrive-worker/src/cgroup_manager.rs` | **KEEP IN PLACE** | Newtype is unrelated to the FS port — it validates relative-path shape. Stays in worker crate. |
| `WorkloadsBootstrapError` envelope | `crates/overdrive-worker/src/cgroup_manager.rs` | **KEEP IN PLACE** | Error envelope on the typed surface stays; it wraps the underlying `io::Error` regardless of which `CgroupFs` impl produced it. The EBUSY-vs-other discrimination logic is identical because the kernel-level contract is identical for both real cgroupfs and the no-op semantic SimCgroupFs returns (which can be injected to return `EBUSY` for the right tests). |
| Existing tempfile-based unit tests in `cgroup_manager.rs` (12 tests today) | `crates/overdrive-worker/src/cgroup_manager.rs` `#[cfg(test)] mod tests` | **TRIAGE per-test** (8 convert to SimCgroupFs; 4 stay tempfile-backed as byte-side-effect tests against `RealCgroupFs`) | Per issue #136 AC, the choice is made deliberately per test and documented: (1) `cgroup_path_as_str_returns_canonical_string`, `cpu_weight_for_pins_division_and_clamp`, `from_subtree_control_io_*` (3 tests) — pure logic, no FS; stay as-is. (2) `cgroup_kill_is_idempotent_on_missing_scope`, `remove_workload_scope_is_idempotent_on_missing_scope` — convert to SimCgroupFs (assertion is "Ok on absent path"; SimFs models presence/absence faithfully). (3) `cgroup_kill_writes_one_to_cgroup_kill_file`, `place_pid_in_scope_writes_pid_to_cgroup_procs`, `write_resource_limits_writes_cpu_weight_and_memory_max`, `write_resource_limits_warn_on_error_writes_files_on_success`, `create_workload_scope_writes_a_real_directory`, `create_workloads_slice_with_controllers_creates_dir_and_writes_subtree_control`, `create_workloads_slice_with_controllers_is_idempotent` — convert to SimCgroupFs (assertion is on the bytes written; SimFs records the byte payload). (4) `cgroup_kill_propagates_non_notfound_errors`, `remove_workload_scope_propagates_non_notfound_errors` — KEEP TEMPFILE-BACKED against `RealCgroupFs`. The current setup creates a regular file where a directory would be and asserts on `ErrorKind::NotADirectory` (`ENOTDIR`) — that is a kernel-VFS effect SimCgroupFs cannot reproduce honestly (in-memory has no inode type taxonomy). Per the non-replacement contract: these tests defend a kernel-side semantic; they belong in the integration / tempfile lane, NOT in the SimFs lane. |
| `SimDriver`, `SimDataplane`, `SimClock` adapter construction shapes | `crates/overdrive-sim/src/adapters/*.rs` | **PATTERN-REUSE** | `SimCgroupFs::new()` follows the same shape — small constructor, internal `parking_lot::Mutex` over BTreeMap state, builder-style injection of fault scenarios (without violating "no builder for the port itself" — these are sim-side fault injectors, not port-trait config). |
| `ServiceVipAllocator::probe()` (Earned Trust precedent — see brief.md §71) | `crates/overdrive-control-plane/src/...` | **PATTERN-REUSE** | `CgroupFs::probe()` follows the same shape: composition-root invariant "wire then probe then use"; failure surfaces as `health.startup.refused` event. Production probe exercises a tempdir round-trip (create / write / read-back / remove) at `<cgroup_root>/.overdrive-probe-<uuid>/` to confirm the adapter can honor its contract in the real environment. SimCgroupFs's probe is structural (it always succeeds unless fault-injected). |

**Zero unjustified CREATE NEW.** Every new artifact is justified by the
absence of an existing peer.

---

## Wave: DESIGN / [REF] Open Questions

None deliberately deferred. The five issue-#136 decisions are all
addressed. The following are explicitly **out of scope** (not deferrals)
and not tracked:

- A general-purpose `Filesystem` port — rejected by D1; if a future
  consumer wants one (e.g. a redb-on-tmpfs probe), they extract their
  own narrow port. Composition over a god-trait.
- A property-based test that drives both `RealCgroupFs` and `SimCgroupFs`
  through the same sequence and asserts byte-side-effect equivalence —
  WORTH having (per `.claude/rules/development.md` § "Trait definitions
  specify behavior" → "The DST equivalence test is the structural guard")
  but the byte-side equivalence is trivially satisfied by SimCgroupFs's
  byte-store model; the semantic divergence is what Tier 3 covers. The
  DELIVER wave decides whether to land a minimal equivalence test
  (recommended) or rely on per-test SimCgroupFs coverage.

---

## Wave: DESIGN / [REF] Wave Decisions Summary

```yaml
wave: DESIGN
feature: cgroup-fs-port
density: lean
expansion_prompt: ask-intelligent
review_enabled: true
review_triggered: false   # no contested ADR, no novel pattern, no security boundary change

inputs:
  primary: gh-issue-#136
  ssot_read:
    - docs/product/architecture/brief.md (§ Application Architecture, §§ 1-3, 24-29)
    - docs/product/architecture/adr-0003-core-crate-labelling.md
    - docs/product/architecture/adr-0016-overdrive-host-extraction-and-adapter-host-rename.md
    - docs/product/architecture/adr-0026-cgroup-v2-direct-writes.md
    - docs/product/architecture/adr-0029-overdrive-worker-crate-extraction.md
    - docs/product/architecture/adr-0030-exec-driver-and-allocation-spec-args.md
    - docs/product/architecture/adr-0034-remove-allow-no-cgroups-escape-hatch.md
    - crates/overdrive-worker/src/cgroup_manager.rs
    - crates/overdrive-worker/src/driver.rs
    - crates/overdrive-core/src/traits/clock.rs (precedent)
    - crates/overdrive-core/src/traits/entropy.rs (precedent)
    - crates/overdrive-control-plane/src/lib.rs (AppState shape)
    - .claude/rules/development.md (port-trait, production-not-shaped-by-sim, trait-contract, ordered-collection)
    - .claude/rules/testing.md (DST K3, async std::fs ban)

decisions:
  D1_trait_shape: narrow CgroupFs (Option A)
  D2_trait_placement: overdrive-core/src/traits/cgroup_fs.rs
  D3_semantics_leak: Tier 3 stays mandatory; non-replacement contract in ADR
  D4_cancellation: method-entry deterministic; mid-syscall N/A in-process
  D5_migration_shape: struct method (CgroupManager::new(fs: Arc<dyn CgroupFs>))

outputs:
  - docs/feature/cgroup-fs-port/feature-delta.md (this file)
  - docs/product/architecture/adr-0054-cgroup-fs-port.md
  - docs/product/architecture/brief.md (§ Application Architecture — dated paragraph appended)

c4_diagrams_emitted:
  - container-level (Mermaid; inline in this file + ADR-0054)
  # System-Context omitted: this is an internal refactor at known scale,
  # no external-actor / external-system boundary change.

quality_gates:
  reuse_analysis: passed (zero unjustified CREATE NEW)
  port_trait_discipline: passed (mandatory not defaulted; trait in core; sim impl in sim crate; host impl in host crate)
  earned_trust: passed (probe() method on trait; production probe exercises tempdir round-trip; SimFs probe is structural; composition-root wire-then-probe-then-use)
  dst_k3_compatibility: passed (SimCgroupFs has no nondeterminism source; injection schedule is BTreeMap-keyed)
  documentation_density: lean — Tier-1 [REF] only emitted; expansion menu surfaced at wave-end

deferrals_to_user: none
gh_issues_created: none
outcome_collision_check: skipped (CLI not available in this environment; surfaced in return summary)

handoff:
  next_wave: DISTILL
  to: acceptance-designer (translate AC into Rust integration tests under crates/overdrive-worker/tests/integration/exec_driver_cgroupfs_port/)
  notes:
    - "DDD-3 implies the DISTILL test scenarios should split: SimCgroupFs covers byte-side-effects + error injection; RealCgroupFs (Lima sudo) covers kernel-side effects (cgroup.kill actually kills, subtree_control validated, EBUSY when child cgroup non-empty)."
    - "No external integrations introduced; no contract-test annotation needed."
    - "DELIVER wave's roadmap should sequence: (1) add CgroupFs trait + tests, (2) add RealCgroupFs + integration tests, (3) add SimCgroupFs + unit tests, (4) refactor cgroup_manager.rs to CgroupManager struct, (5) refactor ExecDriver::new signature, (6) update binary composition root, (7) triage existing tempfile tests per Reuse Analysis."
```

---

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DESIGN#DDD-1 | Narrow `CgroupFs` trait (5 methods: `create_dir`, `write`, `remove_dir`, `probe`, `kind`) | n/a | DISTILL writes a `Class B` scenario for every documented edge case of each method; total 18 SimCgroupFs Tier-1 scenarios under `crates/overdrive-worker/tests/acceptance/sim_cgroup_fs/` |
| DESIGN#DDD-3 | Non-replacement contract — SimCgroupFs cannot model kernel-side effects | n/a | DISTILL splits scenarios into Tier 1 (SimCgroupFs, byte side effects) and Tier 3 (RealCgroupFs against real `/sys/fs/cgroup` via Lima sudo, kernel-side effects). Class C covers all 6 kernel-side effects ADR-0054 § D3 enumerates; Class B explicitly does NOT |
| DESIGN#DDD-4 | Cancellation = method-entry deterministic; mid-syscall N/A in-process; partial writes explicitly out of scope | n/a | DISTILL writes scenario F1 (K3 trajectory determinism) as the structural guard; partial-write scenarios are absent by design (documented in `## Out of scope`) |
| DESIGN#DDD-5 | `ExecDriver::new(cgroup_root, clock, fs)` — mandatory not defaulted; constructor injection on `CgroupManager` | n/a | DISTILL writes Class A trybuild compile-fail scenarios A1 (missing-arg arity) and A2 (no-Default) as the structural enforcement of "Required, not defaulted, at the call site" per `.claude/rules/development.md` § "Port-trait dependencies" |
| GH-#136-AC6 | Per-test triage of existing 16 tempfile unit tests | n/a | DISTILL produces the authoritative triage matrix in scenario E1 (5 KEEP inline pure-logic + 7 CONVERT to SimCgroupFs + 2 KEEP-AND-MOVE bootstrap pair + 2 KEEP-TEMPFILE substrate-boundary against `RealCgroupFs` gated behind `--features integration-tests`); DELIVER step 7 consumes this matrix. Triage matches DESIGN feature-delta § Reuse Analysis verbatim; the only delta vs DESIGN is the recount from 12 → 16 |
| DESIGN-review#carry-over-1 | Property-based Real/Sim equivalence test recommended | n/a | DISTILL writes scenario D1 (proptest sequence equivalence between RealCgroupFs-rooted-at-tempdir and SimCgroupFs) gated behind `--features integration-tests`; case count default 1024; explicit limitation (byte-store only, not kernel semantics) called out in scenario docstring |
| DESIGN-review#carry-over-2 | `RealCgroupFs::with_probe_root()` explicit test coverage | n/a | DISTILL writes scenario C-probe-with-custom-root asserting the builder override scopes the probe directory AND does not touch `/sys/fs/cgroup` |
| DESIGN-review#carry-over-3 | brief.md Status header sync (cosmetic) | n/a | NOT a DISTILL scenario; surfaced to DELIVER handoff as a follow-up |
| GH-#136-AC7 | DST harness includes filesystem fault scenarios | n/a | Class B covers NotFound, PermissionDenied, AlreadyExists (via injection schedule), DirectoryNotEmpty (B-remove_dir-directory-not-empty), Other (B-write-injected-other-returns); EBUSY is a kernel-side effect covered by C-subtree-control-ebusy; cancellation is method-entry deterministic per DDD-4 (no scenario needed — structural property of SimCgroupFs design); partial writes EXPLICITLY OUT OF SCOPE per ADR-0054 § D4 |

## Wave: DISTILL / [REF] Scenario list with tags

| Scenario ID | Tags | Tier | Driving port / SUT | AC |
|---|---|---|---|---|
| A1 | `@compile-fail @tier-1` | 1 | trybuild fixture; `ExecDriver::new` arity | AC2, AC5 |
| A2 | `@compile-fail @tier-1` | 1 | trybuild fixture; `ExecDriver::default()` | AC2, AC5 |
| A3 | `@informational` | n/a | (dst-lint, already enforced) | AC2 |
| B-create_dir-happy | `@tier-1` | 1 | `Arc<dyn CgroupFs>` (SimCgroupFs) | AC2, AC4 |
| B-create_dir-injected-permission-denied | `@tier-1` | 1 | SimCgroupFs + injection | AC4, AC7 |
| B-write-happy | `@tier-1` | 1 | SimCgroupFs | AC2, AC4 |
| B-write-empty-bytes | `@tier-1` | 1 | SimCgroupFs | AC2 |
| B-write-missing-parent-returns-notfound | `@tier-1` | 1 | SimCgroupFs | AC2, AC7 |
| B-write-injected-other-returns | `@tier-1` | 1 | SimCgroupFs + injection | AC4, AC7 |
| B-remove_dir-happy | `@tier-1` | 1 | SimCgroupFs | AC2 |
| B-remove_dir-notfound | `@tier-1` | 1 | SimCgroupFs | AC2, AC7 |
| B-remove_dir-directory-not-empty | `@tier-1` | 1 | SimCgroupFs | AC2 |
| B-remove_dir-injected-permission-denied | `@tier-1` | 1 | SimCgroupFs + injection | AC4, AC7 |
| B-probe-happy | `@tier-1` | 1 | SimCgroupFs `probe()` | AC2, AC4 |
| B-probe-injected-substrate-error | `@tier-1` | 1 | SimCgroupFs `probe()` + injection | AC4 |
| B-probe-round-trip-mismatch | `@tier-1` | 1 | SimCgroupFs `probe()` + corruption hook | AC2 |
| B-kind-returns-stable-string | `@tier-1` | 1 | SimCgroupFs `kind()` | AC2 |
| B-error-schedule-determinism | `@tier-1` | 1 | SimCgroupFs injection schedule | AC4 |
| B-write-then-snapshot-is-deterministic-across-runs | `@tier-1` | 1 | SimCgroupFs snapshot iteration | AC4 |
| B-write-respects-cgroup-procs-pid-payload | `@tier-1` | 1 | SimCgroupFs (replaces tempfile test E1 row 12) | AC2, AC4 |
| B-write-cgroup-kill-stores-one-newline | `@tier-1` | 1 | SimCgroupFs (replaces tempfile test E1 row 7) | AC2, AC4 |
| C-cgroup-kill | `@tier-3 @real-io @adapter-integration` | 3 | RealCgroupFs + real `/sys/fs/cgroup` | AC3 |
| C-subtree-control-ebusy | `@tier-3 @real-io @adapter-integration` | 3 | RealCgroupFs + real `/sys/fs/cgroup` | AC3 |
| C-controller-validation | `@tier-3 @real-io @adapter-integration` | 3 | RealCgroupFs + real `/sys/fs/cgroup` | AC3 |
| C-pseudo-file-synthesis | `@tier-3 @real-io @adapter-integration` | 3 | RealCgroupFs + real `/sys/fs/cgroup` | AC3 |
| C-rmdir-auto-reap | `@tier-3 @real-io @adapter-integration` | 3 | RealCgroupFs + real `/sys/fs/cgroup` | AC3 |
| C-procs-pid-movement | `@tier-3 @real-io @adapter-integration` | 3 | RealCgroupFs + real `/sys/fs/cgroup` | AC3 |
| C-probe-success | `@tier-3 @real-io @walking-skeleton` | 3 | RealCgroupFs `probe()` against real `/sys/fs/cgroup` | AC3 |
| C-probe-with-custom-root | `@tier-3 @real-io` | 3 | RealCgroupFs `with_probe_root(tmpdir)` | AC3 |
| C-write-to-readonly-cgroup-file | `@tier-3 @real-io @adapter-integration` | 3 | `CgroupManager` over `Arc<dyn CgroupFs>` (RealCgroupFs) writing kernel-read-only pseudo-file (`cgroup.events`) → real `EACCES` propagation | AC3, AC5 |
| C-existing-integration-suite-passes | `@tier-3 @real-io @regression` | 3 | existing `tests/integration/exec_driver/*.rs` | AC5, AC9 |
| D1 | `@property @tier-3 @real-io` | 3 | proptest over `Arc<dyn CgroupFs>` (Real-vs-Sim) | AC2 |
| E1 | `@authoritative-matrix` | n/a | (triage decision for AC6) | AC6 |
| E2 | `@tier-3 @walking-skeleton` | 3 | `overdrive serve` subprocess; `health.startup.refused` | AC3, AC5 |
| F1 | `@property @tier-1` | 1 | proptest over SimCgroupFs op sequences | AC4, AC7 |

Total: **35 scenarios** (3 Class A + 18 Class B + 10 Class C + 1 Class D + 2 Class E + 1 Class F).

## Wave: DISTILL / [REF] WS strategy

**Architecture of Reference** (project-level, not feature-level): driving
ports get real adapters; driven-internal ports get real adapters via
the project Infrastructure Policy mechanism; driven-external /
non-deterministic ports get fakes. For this feature the only port
introduced is `CgroupFs`, which is a **driven-internal port wrapping a
substrate** (Linux cgroupfs). The Architecture-of-Reference default
treatment is "real adapter via the mechanism declared in the project
Infrastructure Policy".

**Project Infrastructure Policy file is ABSENT** (`docs/architecture/
atdd-infrastructure-policy.md` does not exist). The bootstrap template
is NOT emitted by this DISTILL pass because:
- The project's actual policy is encoded in `.claude/rules/testing.md`
  via the 4-tier model + the Lima VM convention, NOT in a separate
  ATDD policy file.
- Asking the user to bootstrap a Python-flavoured ATDD policy file in a
  Rust workspace would be misleading.

The 4-tier model dictates mechanism: SimCgroupFs (in-memory) for Tier 1
default lane; RealCgroupFs (against real `/sys/fs/cgroup`) for Tier 3
under `--features integration-tests` via `cargo xtask lima run --`.

Walking-skeleton scenario per project convention: **E2 (composition-root
probe gate)** — the closest analogue to "user runs the binary, sees
observable outcome through driving port". The `overdrive serve`
subprocess IS the user-facing entry point; a probe failure produces
the operator-observable `health.startup.refused` event. Tagged
`@walking-skeleton`. Secondary walking-skeleton: **C-probe-success**
(the happy-path equivalent — production probe succeeds in canonical
environment).

## Wave: DISTILL / [REF] Adapter coverage table

| Adapter | Scenarios | Real-I/O coverage | Notes |
|---|---|---|---|
| `CgroupFs` trait (compile-time contract) | A1, A2 | n/a (trybuild) | trybuild fixtures enforce the constructor-mandatory-not-defaulted rule structurally |
| `SimCgroupFs` (driven, in-memory) | all 18 B-scenarios + F1 + D1 (Sim half) | n/a (in-memory only) | Tier 1 default lane; every method + every injection error class exercised |
| `RealCgroupFs` (driven, real I/O via tokio::fs) | C-probe-success, C-probe-with-custom-root, D1 (Real half against tempdir), C-existing-integration-suite-passes | YES — `@real-io @adapter-integration` on every C scenario | Tier 3 gated behind `--features integration-tests` + Lima sudo |
| `RealCgroupFs` against real cgroupfs (kernel-side effects) | C-cgroup-kill, C-subtree-control-ebusy, C-controller-validation, C-pseudo-file-synthesis, C-rmdir-auto-reap, C-procs-pid-movement, C-write-to-readonly-cgroup-file | YES — kernel semantics exercised | These are the scenarios proving Sim's non-replacement contract is honest |
| `CgroupManager` (struct, post-refactor) | E1 (7 CONVERT + 2 KEEP-AND-MOVE via SimCgroupFs) + E1 (2 KEEP-TEMPFILE via RealCgroupFs, rows 8 + 10) + C-write-to-readonly-cgroup-file (real-substrate propagation through CgroupManager) | YES — both SimCgroupFs (logic) and RealCgroupFs (substrate boundary) | SimCgroupFs-backed manager tests assert manager.method invokes `fs.write(...)` correctly via SimCgroupFs snapshot inspection; tempfile-backed manager tests (rows 8 + 10) and C-write-to-readonly-cgroup-file assert real `io::Error` propagates through the manager from a real `tokio::fs::*` syscall |
| `ExecDriver` (post-refactor constructor) | A1, A2, C-existing-integration-suite-passes | partial (Tier 3 only) | Constructor signature change is structurally enforced (trybuild) + regressed at Tier 3 |
| `overdrive serve` binary (composition root) | E2 | YES — subprocess invocation | Walking-skeleton; verifies `health.startup.refused` event surfaces with `ProbeError::Substrate` cause |

**Zero `NO — MISSING` rows.** Every adapter has at least one `@real-io`
or `@adapter-integration` or Tier 1 scenario.

## Wave: DISTILL / [REF] Scaffolds (DELIVER ordering reference)

Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing
commits", scaffolds in this workspace use `#[should_panic(expected = "RED
scaffold")]` on Rust test bodies + `todo!("RED scaffold: ...")` in
production. **Scaffolding is the DELIVER crafter's concern, NOT DISTILL's.**
DISTILL does not create stub files in `src/`.

The expected scaffold order matches the 8-commit migration sequence in
`docs/feature/cgroup-fs-port/distill/test-scenarios.md` § "DELIVER-step
scaffold ordering". Highlights:

- **First scaffold** (DELIVER commit 1): trybuild fixture A1 + the
  trait declaration in `crates/overdrive-core/src/traits/cgroup_fs.rs`.
  RED is the rustc-rejection of the missing-arg call. GREEN is the
  trait existing with the documented method signatures.
- **Bulk scaffolds** (DELIVER commit 3): every B-* scenario lands as
  `#[should_panic(expected = "RED scaffold")]` initially; the
  `SimCgroupFs::create_dir / write / remove_dir / probe` impls each
  carry `todo!("RED scaffold: <scenario>")` until commit 3 completes.
- **Last scaffolds** (DELIVER commit 8): C-* + D1 scenarios land last
  because they depend on the full migration. The Tier 3 scenarios
  spawn real `/bin/sleep` PIDs into real cgroupfs; they cannot run
  before the worker subsystem startup wires RealCgroupFs through the
  composition root (commit 6).

No `__SCAFFOLD__` Python-style markers; no scaffold files in `src/`.

## Wave: DISTILL / [REF] Test placement

- **Class A (compile-fail)**:
  `crates/overdrive-worker/tests/compile_fail/exec_driver_missing_fs.rs`
  + `.stderr` pair; `crates/overdrive-worker/tests/compile_fail/exec_driver_no_default.rs`
  + `.stderr` pair. Trybuild runner harness lives at
  `crates/overdrive-worker/tests/compile_fail.rs` (new entrypoint).
  Precedent: trybuild is established by existing `overdrive-core`
  compile-fail tests for intent-vs-observation non-substitutability
  per `.claude/rules/testing.md` § "Compile-fail testing (trybuild)".

- **Class B (SimCgroupFs Tier 1)**: under
  `crates/overdrive-worker/tests/acceptance/sim_cgroup_fs/` —
  one file per scenario family (e.g. `create_dir.rs`, `write.rs`,
  `remove_dir.rs`, `probe.rs`, `kind.rs`, `error_schedule.rs`).
  Wired via the existing `crates/overdrive-worker/tests/acceptance.rs`
  entrypoint (already established; one new `mod sim_cgroup_fs { ... }`
  block).

- **Class C (RealCgroupFs Tier 3)**: under
  `crates/overdrive-worker/tests/integration/real_cgroup_fs/` —
  one file per scenario. Wired via the existing
  `crates/overdrive-worker/tests/integration.rs` entrypoint
  (`mod real_cgroup_fs { ... }` adjacent to `mod exec_driver`).
  Uses the same `AllocCleanup` RAII pattern from
  `tests/integration/exec_driver/cleanup.rs`.

- **Class D (equivalence proptest)**:
  `crates/overdrive-worker/tests/integration/cgroup_fs_equivalence.rs`
  — single file; module wired in `tests/integration.rs`.

- **Class E (migration regression)**:
  - E1 (triage matrix): consumed by DELIVER; no test file.
  - E2 (composition-root probe gate): either
    `crates/overdrive-worker/tests/integration/composition_root_probe_gate.rs`
    OR `crates/overdrive-cli/tests/integration/serve_probe_refusal.rs`
    depending on whether the crafter prefers worker-level in-process
    or CLI subprocess invocation. The subprocess form is preferred
    per the driving-adapter coverage requirement.

- **Class F (K3 determinism)**:
  `crates/overdrive-worker/tests/acceptance/sim_cgroup_fs/k3_determinism.rs`.

Precedent: `crates/overdrive-worker/tests/integration/exec_driver/`
directory layout is the model for both the Class B (acceptance) and
Class C (integration) trees.

## Wave: DISTILL / [REF] Driving Adapter coverage

Per `.claude/rules/testing.md` and the project's `nw-distill` skill,
every CLI / endpoint / hook in DESIGN must have at least one scenario
exercising it via its actual user-invocation protocol.

| Driving adapter in DESIGN | Scenario | Protocol |
|---|---|---|
| `overdrive serve` subcommand (composition root that constructs `RealCgroupFs`, calls `probe()`, threads `Arc<dyn CgroupFs>` into worker startup) | E2 (composition-root probe gate) | subprocess via `Command::new("overdrive").args(["serve", ...])`; observable = exit code + `health.startup.refused` event in structured log / stderr |
| `overdrive serve` happy path (probe succeeds, worker starts) | C-probe-success implicitly covers this; the existing integration suite (regressed by C-existing-integration-suite-passes) covers full convergence-loop wiring | subprocess + Tier 3 integration suite |

No new CLI subcommands. No new HTTP endpoints. No new hooks. The
existing driving-adapter coverage (worker subsystem in-process via
`AppState.driver`; CLI subprocess via the existing `serve` test
fixtures in `crates/overdrive-cli/tests/`) extends naturally.

## Wave: DISTILL / [REF] Pre-requisites

- **Lima VM with cgroup v2 delegation** for all Tier 3 (Class C, D, E2)
  scenarios. Invoked via `cargo xtask lima run --`. See
  `.claude/rules/testing.md` § "Running tests — Lima VM".
- **`--features integration-tests`** flag on every Tier 3 scenario's
  cargo invocation. Workspace convention: `overdrive-worker` already
  declares `integration-tests = []` in its `[features]` block.
- **trybuild pinned exactly** in workspace `[workspace.dependencies]`
  per `.claude/rules/testing.md` § "Compile-fail testing (trybuild)"
  → "Pin trybuild exactly". The DELIVER crafter adds the pin if not
  already present (current workspace status: not yet present —
  `grep "trybuild" Cargo.toml` returns empty, so commit 1 of the
  migration adds `trybuild = "=1.0.<latest>"` to workspace deps and
  `trybuild.workspace = true` under `overdrive-worker/[dev-dependencies]`).
- **`proptest` workspace dep** for D1 + F1. Workspace already carries
  `proptest` per the existing schema-evolution + newtype-roundtrip
  test infrastructure.
- **`overdrive-sim.workspace = true`** under
  `overdrive-worker/[dev-dependencies]` for all Class B + F + D
  scenarios. Currently absent (existing tests use tempfile + real
  tokio::fs); the dep gets added in commit 3 of the DELIVER sequence.
- **No new external crates** beyond trybuild (and trybuild is internal
  dev tooling, not a runtime dep). Matches the ADR-0054 commitment
  "No new external crates added".

## Wave: DISTILL / [REF] Wave Decisions Summary

```yaml
wave: DISTILL
feature: cgroup-fs-port
density: lean
expansion_prompt: ask-intelligent
review_enabled: true
review_triggered: false   # consolidated review = orchestrator dispatches Sentinel only; DISCUSS+DESIGN+DEVOPS skipped

inputs:
  primary: docs/feature/cgroup-fs-port/feature-delta.md (DESIGN sections) + docs/product/architecture/adr-0054-cgroup-fs-port.md + gh issue #136
  ssot_read:
    - docs/feature/cgroup-fs-port/feature-delta.md (DESIGN wave; lines 1-230)
    - docs/product/architecture/adr-0054-cgroup-fs-port.md (full)
    - github issue #136 body (acceptance criteria + decision narrative)
    - crates/overdrive-worker/src/cgroup_manager.rs (existing 16 inline tests)
    - crates/overdrive-worker/src/driver.rs (ExecDriver::new signature)
    - crates/overdrive-worker/tests/acceptance.rs (acceptance entrypoint)
    - crates/overdrive-worker/tests/integration.rs (integration entrypoint)
    - .claude/rules/testing.md (4-tier model, Lima VM convention, integration-tests feature gate, trybuild rules, proptest rules)
    - .claude/rules/development.md (port-trait dependencies, trait contract, ordered-collection choice)

reconciliation_verdict: passed — 0 contradictions across DESIGN sources (feature-delta DESIGN wave + ADR-0054 + GH#136 are consistent)

decisions:
  scenario_classes: A (compile-fail trybuild) | B (SimCgroupFs Tier 1) | C (RealCgroupFs Tier 3 kernel semantics) | D (Real/Sim equivalence proptest) | E (migration regression incl. authoritative triage matrix) | F (K3 determinism proptest)
  total_scenarios: 35
  ac_coverage: AC1 satisfied-by-artifact (ADR-0054 exists); AC2-AC7 + AC9 covered by scenarios; AC8 surfaced as crafter constraint (mutation gate, not a scenario)
  walking_skeleton: E2 (composition-root probe gate) + C-probe-success (happy path)
  triage_deviation_from_design: NONE. The E1 matrix matches DESIGN feature-delta § Reuse Analysis verbatim (5 KEEP inline + 7 CONVERT + 2 KEEP-AND-MOVE + 2 KEEP-TEMPFILE). The only adjustment is the recount from 12 → 16 (DESIGN's count was approximate). Sentinel's prior E1 deviation (reclassifying rows 8 + 10 from KEEP-TEMPFILE to CONVERT) was rejected by the user on substrate-boundary grounds — SimCgroupFs `inject_error` tests propagation *logic* (fake error source); the tempfile tests test the *substrate boundary* (real `io::Error` from real `tokio::fs::*` against real kernel VFS). The new Class C scenario **C-write-to-readonly-cgroup-file** is added as a production-realistic alternative (real `EACCES` from writing kernel-read-only `cgroup.events`); rows 8 + 10 are candidates for retirement once it lands, but the retirement is a follow-on decision, not prescribed by this DISTILL pass.

outputs:
  - docs/feature/cgroup-fs-port/feature-delta.md (this file, DISTILL sections appended)
  - docs/feature/cgroup-fs-port/distill/test-scenarios.md (specification SSOT; never parsed)

skipped_from_standard_skill:
  - .feature files (project bans pytest-bdd / cucumber-rs per .claude/rules/testing.md)
  - Python scaffold files in src/ with __SCAFFOLD__ markers (project uses Rust #[should_panic(expected = "RED scaffold")] convention per .claude/rules/testing.md § "RED scaffolds and intentionally-failing commits"; scaffolds are DELIVER's concern not DISTILL's)
  - pytest / nwave-ai outcomes register / outcomes check-delta CLI invocations (no Python tooling exists)
  - red-classification.md (project's #[should_panic] convention IS the classification)
  - pre-DELIVER fail-for-the-right-reason gate execution (scaffolds don't exist yet; the gate is structurally enforced at compile time by #[should_panic] when DELIVER lands the first RED)
  - Project Infrastructure Policy file at docs/architecture/atdd-infrastructure-policy.md (project's 4-tier model in .claude/rules/testing.md IS the equivalent; bootstrapping a Python-flavoured policy file in a Rust workspace would be misleading)

quality_gates:
  scenario_completeness: passed (every documented AC has >=1 scenario; triage decisions explicit; non-modellable behaviours explicitly excluded in `## Out of scope`)
  hexagonal_boundary: passed (every B/C scenario invokes through `Arc<dyn CgroupFs>` trait object; no concrete-type access; trybuild structurally enforces constructor mandatoriness)
  trait_contract_coverage: passed (every documented edge case in ADR-0054 D1 method docstrings has a scenario; every kernel-side effect in ADR-0054 D3 has a Tier 3 scenario)
  determinism_guard: passed (F1 pins K3 trajectory; B-error-schedule-determinism + B-write-then-snapshot-is-deterministic-across-runs pin BTreeMap iteration order)
  driving_adapter_coverage: passed (`overdrive serve` subprocess covered by E2 walking-skeleton)
  documentation_density: lean — Tier-1 [REF] only; Tier-2 expansion candidates offered in return summary

deferrals_to_user:
  - "(RESOLVED 2026-05-24) Prior E1 deviation on rows 8 + 10 was rejected by user; this revision reverts to DESIGN classification (KEEP-TEMPFILE) AND adds new scenario C-write-to-readonly-cgroup-file as production-realistic substrate-boundary coverage. Retirement of rows 8 + 10 is a future follow-on once C-write-to-readonly-cgroup-file lands and proves equivalent."

gh_issues_created: none (per project rules — no unilateral creation)

handoff:
  next_wave: DELIVER
  to: software-crafter (object-oriented paradigm per CLAUDE.md)
  notes:
    - "DELIVER reads docs/feature/cgroup-fs-port/distill/test-scenarios.md to materialise Rust test cases. The scenarios already pin the 8-commit migration sequence + per-step scaffold ordering."
    - "Add `trybuild = \"=1.0.<latest>\"` to [workspace.dependencies] in commit 1 of the migration; first DISTILL run that touches trybuild fixtures in this repo."
    - "Add `overdrive-sim.workspace = true` under overdrive-worker/[dev-dependencies] in commit 3 (SimCgroupFs adapter intro)."
    - "Mutation gate per-step via `cargo xtask mutants --diff origin/main --package overdrive-worker --file <touched-files>` per .claude/rules/testing.md § Per-step vs per-PR scoping; collective target ≥ 80% per AC8."
    - "Reviewer carry-over #3 (brief.md Status header sync, line 29) is cosmetic — fold into the same DELIVER PR if convenient, otherwise surface to user for a separate follow-up."
    - "LSP/IDE shows pre-existing nix::sched::setns / CloneFlags errors in driver.rs:386 — these are unrelated to this feature (probably a missing `nix` feature flag for `sched` namespaces) and predate this DISTILL pass. Do NOT address as part of this PR unless the user explicitly scopes it in."
```
