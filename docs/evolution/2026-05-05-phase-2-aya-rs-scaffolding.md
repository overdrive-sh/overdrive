# phase-2-aya-rs-scaffolding — Feature Evolution

**Feature ID**: phase-2-aya-rs-scaffolding
**Driving issue**: GH #23 — `[2.1] aya-rs eBPF scaffolding + build pipeline`
**Branch**: `marcus-sa/phase2-ebpf-start` (planning + early DELIVER); merged to `main` via PR #153 (2026-05-04)
**Duration**: 2026-05-04 (DESIGN + DEVOPS + DELIVER all completed in a single day after the planning commit `48981ef`)
**Status**: Delivered (10/10 steps PASS in `execution-log.json`; merged via PR #153 plus follow-up commits f1283dc → 349ef1e for CI hardening)
**Walking-skeleton extension**: First eBPF dataplane primitive — the kernel-side
crate, the userspace loader, the build pipeline, and the Tier 2 BPF unit harness
that every later Phase-2 slice (#24–#37: SERVICE_MAP, sockops+kTLS, BPF LSM,
telemetry ringbuf) compiles against.

---

## What shipped

The scaffolding that whitepaper §7 (eBPF Dataplane) depends on. Two new crates
plus a hybrid build pipeline that produces verifier-loadable BPF object files
on Linux/Lima and degrades cleanly on macOS:

- **`overdrive-bpf`** (class `binary`, target `bpfel-unknown-none`,
  `#![no_std]`) — kernel-side eBPF programs. Phase 2.1 ships one no-op XDP
  program `xdp_pass` plus an `LruHashMap<u32, u64>` packet counter. Compiles
  to a single ELF object at the stable path
  `target/xtask/bpf-objects/overdrive_bpf.o`. Excluded from `default-members`
  so `cargo check --workspace` skips it on macOS automatically.
- **`overdrive-dataplane`** (class `adapter-host`) — userspace BPF loader.
  Hosts the `EbpfDataplane` impl of the `Dataplane` port trait from
  `overdrive-core`, embeds the BPF object via `include_bytes!`, and ships a
  `build.rs` artifact-check shim that converts the otherwise-opaque "file
  not found" rustc error into a one-line actionable diagnostic. Method bodies
  for `update_policy` / `update_service` / `drain_flow_events` are stubs
  pointing at #24 / #25 / #27. Compiles on macOS via
  `#[cfg(target_os = "linux")]` stub bodies that return
  `DataplaneError::LoadFailed` for any aya-touching method.
- **Hybrid build pipeline** — `cargo xtask bpf-build` is the primary entry
  point; the `build.rs` shim is the defensive fallback. **No recursive
  cargo invocations from `build.rs`** — that pattern (the aya-template
  default) is the failure mode this design explicitly rejects per ADR-0038
  §3.4.
- **`bpf-linker` provisioning across three surfaces** — Lima image (`infra/
  lima/overdrive-dev.yaml` line 205 extended to add `bpf-linker` to the
  existing `cargo install --locked` invocation, plus a nightly toolchain +
  `rust-src` install line for `-Z build-std=core`); `cargo xtask
  dev-setup` for non-Lima Linux developers; and `which_or_hint` at the top
  of `cargo xtask bpf-build` itself. Per
  `feedback_no_user_install_instructions` user memory: the platform never
  tells the user to install the tool manually.
- **Tier 2 BPF unit harness wired** — `cargo xtask bpf-unit` invokes
  `cargo nextest run -p overdrive-bpf --features integration-tests --test '*'`
  against the no-op program's `PKTGEN`/`SETUP`/`CHECK` triptych. The
  triptych drives `aya::programs::Xdp::test_run()` (kernel
  `BPF_PROG_TEST_RUN`), asserting verdict `XDP_PASS` and that the counter
  map increments 0 → 1.
- **CI workflow extension** — two new GitHub Actions jobs in
  `.github/workflows/ci.yml`: `bpf-build` (compiles the ELF, uploads as
  `actions/upload-artifact@v4`, retention 14 days) and `bpf-unit`
  (consumes the artifact via `actions/download-artifact@v4` with
  `if-no-files-found: error`, runs the Tier 2 triptych). Two new
  required-status-checks added to `main` branch protection (operator
  action sequenced after the first observed pass).

The scaffold proves the seams hold end-to-end: build → load → attach →
observe → detach exercises the entire pipeline against a real kernel via
`cargo xtask lima run -- cargo xtask bpf-unit`. Subsequent Phase-2 slices
(#24+) compile against the same trait surface and the same loader; the
binary-composition edge that wires `Arc<dyn Dataplane>` into `AppState`
waits for the first slice with something concrete to call.

## Business context

`phase-1-first-workload` closed the convergence loop for process workloads —
the control plane converges declared replica count, ProcessDriver places
allocations in cgroup-isolated scopes, `alloc status` renders real `Running`
rows. Up to that point Overdrive had no kernel-side dataplane: every claim
in whitepaper §7 (XDP load balancing, sockops + kTLS, BPF LSM mandatory
access control, kernel-native telemetry) compiled against a `SimDataplane`
in-memory HashMap and nothing else.

Phase 2 closes that gap. Phase 2.1 (this feature) is the foundational
scaffolding — the *empty* dataplane, proven loadable. It maps to one
pinned roadmap item:

- **GH #23 [2.1]** — aya-rs eBPF scaffolding + build pipeline (whitepaper
  §7 / §22).

Scope explicitly held back to downstream Phase-2 issues:

- **#24 [2.2]** — SERVICE_MAP and the first non-trivial XDP program;
  closes the binary-composition edge by wiring `Arc<dyn Dataplane>` into
  `AppState`.
- **#25 [2.3]** — POLICY_MAP and the BPF map hydration path from
  `policy_verdicts` Corrosion rows.
- **#26+** — sockops + kTLS (whitepaper §8).
- **#27** — telemetry ringbuf consumer (whitepaper §12).
- **#28+** — BPF LSM mandatory access control (whitepaper §19).
- **#29** — full Tier 3 kernel matrix `[5.10, 5.15, 6.1, 6.6, latest LTS,
  bpf-next]` + Tier 4 (veristat verifier-regress + xdp-bench perf gates +
  PREVAIL second-opinion analysis). For #23 those xtask subcommands stay
  `tracing_placeholder` stubs with `// TODO(#29)` markers — there is no
  point baselining a no-op program.
- **#152** — single-kernel `cargo xtask integration-test vm latest` LVH
  smoke harness, **deferred from #23 mid-DELIVER** (see *Deferral
  decision* below).

## Wave journey

This feature has **no DISCUSS or DISTILL waves** — it is foundational
infrastructure with no LeanUX user stories or acceptance scenarios beyond
the AC1–AC3 documented in `roadmap.json`. The driving SSOT was
whitepaper §7, §19, §22; no DISCUSS-wave outcome KPIs were produced.

- **DESIGN** (2026-05-04, planning commit `48981ef`) — Morgan. Nine
  ratified decisions (Q1–Q9 locked by the orchestrator brief) producing
  one new ADR (**ADR-0038**) and the architecture spec at
  `architecture.md`. Key pivots: (D1) two crates rather than one — the
  `#![no_std]` + `bpfel-unknown-none` compile contract is incompatible
  with the `std` + `tokio` + `aya` userspace contract, so Cargo-level
  boundaries are stronger than `cfg` boundaries (ADR-0016 strategic
  precedent); (D3) hybrid `xtask bpf-build` + `build.rs` artifact-check
  shim — recursive cargo from `build.rs` is the failure mode every
  industry build pipeline (aya, libbpf-rs, redbpf) documents; (D4)
  `bpf-linker` provisioned by the platform across three surfaces, never
  by the user (per `feedback_no_user_install_instructions`); (D5) macOS
  dev story preserves the `--no-run` quality gate via
  `default-members` exclusion + `#[cfg(target_os = "linux")]` stub
  bodies; (D9) `EbpfDataplane` constructor does real work in #23 (load +
  attach the no-op program) but is **not** wired into `AppState` —
  binary-composition edge defers to the first slice with a concrete
  caller (#24, mirroring ADR-0029's pattern for `Arc<dyn Driver>`). See
  [`design/wave-decisions.md`](../architecture/phase-2-aya-rs-scaffolding/architecture.md).

- **DEVOPS** (2026-05-04, same planning commit) — Apex. Seven decisions
  (D1–D7), no new ADR (ADR-0038 from DESIGN already covered the build-
  pipeline contract). Key constraints: (D1) **single-kernel CI for #23**
  — the full kernel matrix in `.claude/rules/testing.md` §"Kernel
  matrix" is the explicit scope of #29; per-PR critical-path budget is
  ~15 min and the no-op program produces ≤1× the signal at 5× the
  wall-clock against the matrix; (D2) one install primitive (`cargo
  install --locked bpf-linker`) across Lima + CI + dev-setup, with the
  xtask `which_or_hint` as the single diagnostic surface; (D3) two new
  required-status-checks on `main` branch protection (sequenced after
  the first observed pass per `branching-strategy.md` §2); (D6) BPF
  object artifact uploaded by `bpf-build` and consumed by `bpf-unit`
  via `actions/{upload,download}-artifact@v4` — the `if-no-files-found:
  error` setting is load-bearing because the alternative silently
  no-ops a missing artifact into a confusing build.rs diagnostic. See
  the migrated `ci-cd-pipeline.md`, `branching-strategy.md`,
  `infrastructure-integration.md`, and `environments.yaml` under
  `docs/architecture/phase-2-aya-rs-scaffolding/`.

- **DISTILL** — not run. No acceptance test scenarios were produced;
  the AC1–AC3 in `roadmap.json` are unit-shaped or environmental
  rather than narrative.

- **DELIVER** (2026-05-04, commits da141f6 → f1283dc) — Software-crafter
  agents executing 10 steps across 3 phases per the roadmap. All 10
  steps PASS in `execution-log.json`; six of ten skipped `RED_UNIT`
  with structured rationale (see *RED_UNIT skip rationales* below).
  See the migrated artifacts under
  `docs/architecture/phase-2-aya-rs-scaffolding/`.

## Steps completed

| Step | Title | RED_ACCEPT | RED_UNIT | GREEN | COMMIT |
|---|---|---|---|---|---|
| 01-01 | Create `overdrive-bpf` crate (binary class, `bpfel-unknown-none`, `#![no_std]`) with `xdp_pass` no-op program + `LruHashMap<u32,u64>` counter; declare in workspace members + exclude from `default-members`; add `aya-ebpf` to `[workspace.dependencies]` | PASS | SKIPPED | PASS | PASS |
| 01-02 | Create `overdrive-dataplane` crate (adapter-host) with `EbpfDataplane` struct, three-method `Dataplane` impl (real `new()`, stub bodies for #24/#25/#27), `#[cfg(target_os="linux")]`/`not` split mirroring `SimDataplane` constructor shape | PASS | SKIPPED | PASS | PASS |
| 01-03 | Add `overdrive-dataplane` `build.rs` artifact-check shim; structured diagnostic on missing object; emit `cargo:rerun-if-changed` for the artifact path; **NO** recursive cargo invocation per ADR-0038 §3.2 | PASS | SKIPPED | PASS | PASS |
| 01-04 | Verify dst-lint impact (both new crates non-`core`, scanned out automatically) and confirm AC2 macOS compile gate passes for the workspace post-Phase-01 | PASS | PASS | PASS | PASS |
| 02-01 | Add `cargo xtask bpf-build` subcommand: `which_or_hint("bpf-linker")` at top, invoke BPF-target cargo build per ADR-0038 §3.1, copy ELF to stable path; non-zero exit with eyre report on any failure | PASS | PASS | PASS | PASS |
| 02-02 | Extend Lima image provisioning (`infra/lima/overdrive-dev.yaml` line 205) to install `bpf-linker` via `cargo install --locked` alongside existing tools; provision nightly toolchain + `rust-src` for `-Z build-std=core` (upstream issue A1) | PASS | SKIPPED | PASS | PASS |
| 02-03 | Wire `cargo xtask dev-setup` to install `bpf-linker` via `cargo install --locked` after a `which` probe; covers non-Lima Linux developers; **do NOT** instruct the user to install manually | PASS | SKIPPED | PASS | PASS |
| 03-01 | Wire `cargo xtask bpf-unit` to invoke `cargo nextest run -p overdrive-bpf --features integration-tests --test '*'`; ship one PKTGEN/SETUP/CHECK triptych for `xdp_pass` asserting verdict `XDP_PASS` and counter increments 0→1 via `aya::Program::test_run` | PASS | SKIPPED | PASS | PASS |
| 03-02 | Add two new GitHub Actions jobs to `.github/workflows/ci.yml`: `bpf-build` (compiles ELF + uploads artifact) and `bpf-unit` (downloads artifact + Tier 2); `bpf-unit` sets `if-no-files-found: error` per `ci-cd-pipeline.md` §2.2 step 7; integration-test-vm-latest **deferred to #152** (see *Deferral decision*) | PASS | SKIPPED | PASS | PASS |
| 03-03 | Update `.github/workflows/ci.yml` leading comment block to reflect Tier 2 in scope (Tier 3 single-kernel smoke deferred to #152, Tier 4 + full kernel matrix deferred to #29) and update Required status checks comment block to list the two new jobs alongside existing six | PASS | SKIPPED | PASS | PASS |

## Architectural decisions captured

The DESIGN-wave decisions are the load-bearing record. The migrated
[`architecture.md`](../architecture/phase-2-aya-rs-scaffolding/architecture.md)
is the authoritative spec; this section names the decisions that have
ongoing implications beyond #23.

- **D1: Two crates, not one.** `overdrive-bpf` (kernel) +
  `overdrive-dataplane` (loader). Cargo-level boundaries enforce the
  target-triple/std-vs-no_std split mechanically. A shared types crate
  is deferred — anchor at #24 when POLICY_MAP introduces the first
  wire-format struct that crosses the kernel/user boundary.
- **D2: Crate classes per ADR-0003.** `overdrive-bpf` is `binary`
  (artifact-producing); `overdrive-dataplane` is `adapter-host`
  (production binding of the `Dataplane` port). dst-lint scope is
  unchanged — both classes are non-`core`.
- **D3: Hybrid build pipeline.** `cargo xtask bpf-build` is primary;
  `build.rs` is a defensive shim that converts a cryptic
  `include_bytes!` failure into a one-line diagnostic. Stable artifact
  path `target/xtask/bpf-objects/overdrive_bpf.o` is load-bearing —
  every later xtask refactor must preserve it or the loader breaks.
- **D5: macOS dev story.** Workspace gains a NEW `default-members`
  declaration that omits `overdrive-bpf`; the loader compiles via
  `#[cfg(target_os = "linux")]` stub bodies. The existing `--no-run`
  pre-merge gate per `.claude/rules/testing.md` § "Running tests on
  macOS — Lima VM" remains authoritative.
- **D8: xtask harness — partial fill-in.** `cargo xtask bpf-build` is
  NEW. `cargo xtask bpf-unit` and `cargo xtask integration-test vm`
  fill in pre-existing stubs. `cargo xtask verifier-regress` and
  `cargo xtask xdp-perf` remain stubbed with `// TODO(#29)` — no point
  baselining a no-op.
- **D9: `EbpfDataplane` is NOT wired into `AppState` in #23.** The
  constructor does real work, but the binary-composition edge waits
  for #24 (SERVICE_MAP). Same pattern ADR-0029 used for `Arc<dyn
  Driver>`.

## Lessons learned

### RED_UNIT skip rationales (six of ten steps)

The scaffolding nature of the work meant that several steps had no
unit-level invariant beyond what the acceptance test (or environmental
gate) already covered. Each skip carries a structured rationale in
`execution-log.json`. The pattern across them is informative for future
similar work:

- **01-01** — workspace metadata gate is exercised by an xtask convention
  test in RED_ACCEPTANCE; no separate unit-level invariant exists for
  Cargo.toml content.
- **01-02** — cfg-gated stub branch on macOS is a one-line `Err(...)`
  return; macOS compile gate (AC7) is the structural assertion. A test
  would itself be `#[cfg(not(target_os = "linux"))]` and the project's
  macOS nextest hook routes through Lima/Linux where the test is not
  compiled.
- **01-03** — `build.rs` is 20 lines of file-presence + `cargo:rustc-env`
  emission with no branching beyond the cfg gate; Linux integration test
  in RED_ACCEPTANCE is the structural assertion.
- **02-02** — Lima provisioning is YAML configuration, not Rust code; no
  unit-level invariant exists. End-to-end gate is the live VM
  verification.
- **02-03** — RED_ACCEPTANCE tests are unit-shaped argv-construction
  assertions; no separate unit-level invariant exists beyond the planner.
- **03-01** — Tier 2 BPF unit tests are themselves the structural
  assertion per testing.md; the PKTGEN/SETUP/CHECK triptych in
  RED_ACCEPTANCE *is* the unit-level gate.
- **03-02** — GHA workflow YAML configuration; the PR-time CI run is the
  acceptance gate per AC6, no separate unit-level invariant exists.
- **03-03** — comment-block prose; the workflow YAML parse + presence of
  new jobs is the gate covered by step 03-02.

The principle: **RED_UNIT exists for unit-level invariants the
acceptance test does not already cover**. When the acceptance test IS
the structural assertion (Tier 2 PROG_TEST_RUN; cfg-gated compile
checks; YAML provisioning), there is no second invariant to scaffold,
and forcing one would be testing-theater. Recording the rationale
preserves the audit trail.

### Deferral decision: drop step 03-02 LVH harness mid-DELIVER

The original DESIGN/DEVOPS plan called for **three** new GHA jobs:
`bpf-build`, `bpf-unit`, and `integration-test-vm-latest` (a Tier 3 LVH
smoke test). During DELIVER the crafter discovered that `lvh images
pull` requires Docker and the Lima dev VM does not provision it. Three
resolutions surfaced (provision Docker in Lima; pre-bake a rootfs;
migrate to direct QEMU + initramfs as aya itself does).

A step-back analysis revealed the deeper issue: **for a no-op `xdp_pass`
program, real `xdp::attach(iface)` adds zero coverage over Tier 2's
`BPF_PROG_TEST_RUN`**. The attach path is aya's code; the verifier path
is exercised at load-time by Tier 2; the counter increment is identical
between PROG_TEST_RUN and real-attach. The nested VM (LVH or
QEMU+initramfs) only earns its keep when the kernel must differ from the
host environment — which is the kernel matrix `[5.10, 5.15, 6.1, 6.6,
latest LTS, bpf-next]` already deferred to #29.

So step 03-02 was carrying nested-VM machinery whose only justification
was matrix expansion, against a program that doesn't need real-attach
coverage at all. **Resolution**: the partial implementation was deleted
(`xtask/src/integration_vm.rs`, `xtask/test-bin/integration_vm_latest/
main.rs`, `xtask/tests/integration/integration_vm_latest_smoke.rs`,
plus the LVH-related Cargo.toml deps), the `xtask::integration_vm` task
handler reverted to a `tracing_placeholder` pointing at #152, the
roadmap was contracted to two GHA jobs, and the comment block in
ci.yml was updated to reflect the two-jobs scope. The LVH harness ships
in **#152** alongside the kernel-matrix expansion in #29.

The principle: **scope is a moving target during DELIVER**. When the
crafter surfaces an unforeseen friction, the right move is often to
ask whether the friction's cost actually buys signal — not to power
through. Three deferred items (Tier 4 + full kernel matrix + nested-VM
single-kernel smoke) are now traceable to the slices where they
actually pay off (#29 + #152), not bundled into a foundational
scaffolding PR.

### Upstream issues filed during DELIVER

Three structured findings were captured in `deliver/upstream-issues.md`
for architect-led amendment of the upstream wave artifacts:

- **A1** — `architecture.md` §4 / ADR-0038 §4 toolchain provisioning
  was incomplete: `cargo +nightly build ... -Z build-std=core` requires
  the `nightly` rustup toolchain plus `rust-src` component on nightly,
  not just `bpf-linker`. Resolution: Lima YAML extended (commit
  `d8cb059`), dev-setup xtask covers non-Lima Linux (commit `dd534f8`).
  Recommended upstream amendment: enumerate all three toolchain deps
  in `architecture.md` §4 and ADR-0038 §4.
- **A2** — `devops/ci-cd-pipeline.md` §2.2 capability claim was
  empirically false on Ubuntu 22.04+: `kernel.unprivileged_bpf_disabled
  = 2` is the default, and `BPF_PROG_LOAD` requires `CAP_BPF` before
  `BPF_PROG_TEST_RUN` is ever reached. Resolution: `cargo xtask
  bpf-unit` runs under `sudo` via the canonical `cargo xtask lima
  run --` wrapper locally; CI's GHA job runs the step under `sudo`.
  Recommended amendment: rewrite §2.2 "Capability requirements" and
  the corresponding roadmap AC bullet.
- **A3** — `lvh images pull` requires Docker; Lima dev VM lacks
  Docker. Resolution: drop the LVH harness from #23 entirely; defer to
  #152 (see *Deferral decision* above). Recommended amendment: drop the
  LVH framing from `architecture.md` §6.2 and ADR-0038 §6; reference
  #152 for the future nested-VM harness.

These findings are preserved in
[`upstream-issues.md`](../architecture/phase-2-aya-rs-scaffolding/upstream-issues.md)
for follow-up architect work.

## Links to migrated permanent artifacts

The lasting design + DEVOPS artifacts have been migrated under
`docs/architecture/phase-2-aya-rs-scaffolding/`:

- [`architecture.md`](../architecture/phase-2-aya-rs-scaffolding/architecture.md) — DESIGN-wave authoritative spec (crate topology §2, build pipeline §3, toolchain provisioning §4, macOS dev story §5, test strategy §6, `Dataplane` port integration §7, dst-lint impact §8, downstream risks §9, C4 diagrams §10).
- [`ci-cd-pipeline.md`](../architecture/phase-2-aya-rs-scaffolding/ci-cd-pipeline.md) — DEVOPS-wave CI specification (two new GHA jobs in detail; required-status-checks delta).
- [`branching-strategy.md`](../architecture/phase-2-aya-rs-scaffolding/branching-strategy.md) — branch-protection delta + sequencing constraint.
- [`infrastructure-integration.md`](../architecture/phase-2-aya-rs-scaffolding/infrastructure-integration.md) — Lima image change + toolchain compatibility audit.
- [`environments.yaml`](../architecture/phase-2-aya-rs-scaffolding/environments.yaml) — environment inventory across macos-dev / linux-lima-dev / linux-native-dev / github-actions-ci.
- [`upstream-issues.md`](../architecture/phase-2-aya-rs-scaffolding/upstream-issues.md) — DELIVER-wave findings tracking live upstream tickets.

Cross-cutting artifacts that already lived outside the feature workspace:

- **ADR-0038** at `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md` — the load-bearing architectural decision for this slice. Stays at its existing path; the architecture spec above cites it throughout.
- **ADR-0039** (added during DELIVER, commit `8e1a5ea`) at `docs/product/architecture/` — networking / security follow-up captured during the deliver wave.

## What's next

- **#24 [2.2]** — SERVICE_MAP: the first non-trivial XDP program. Wires
  `Arc<dyn Dataplane>` into `AppState`, closing the binary-composition
  edge that #23 deliberately deferred. First slice where `update_service`
  has a concrete caller.
- **#25 [2.3]** — POLICY_MAP: the first wire-format struct crossing the
  kernel/user boundary. Anchor for the deferred decision on a third
  shared-types crate (`overdrive-bpf-types`) versus a `cfg`-gated module
  inside `overdrive-bpf`.
- **#29** — kernel-matrix expansion + Tier 4 (verifier-regress, xdp-perf,
  PREVAIL). Wraps the existing `bpf-unit` job in a `strategy: matrix:`
  block over the kernel set; baselines the verifier complexity and
  xdp-bench throughput against real programs (the no-op produces
  meaningless numbers).
- **#152** — single-kernel `cargo xtask integration-test vm latest`
  harness. Lands when there is XDP branching logic worth exercising on
  real attach-to-interface (likely concurrent with #24's SERVICE_MAP).
