# ADR-0038 — eBPF crate layout (`overdrive-bpf` + `overdrive-dataplane`) and `xtask bpf-build` + `build.rs` shim build pipeline

## Status

Accepted. 2026-05-04. Decision-makers: User-ratified locked decisions
Q1–Q9 from `phase-2-aya-rs-scaffolding` DESIGN dialogue. Tags:
phase-2, dataplane, crate-boundary, build-pipeline.

## Context

Phase 2 of the Overdrive roadmap delivers the eBPF dataplane described
in whitepaper §7. Issue #23 (`[2.1] aya-rs eBPF scaffolding + build
pipeline`) is the foundation slice: every later Phase-2 issue (#24
SERVICE_MAP, #25 sockops+kTLS, #26 BPF LSM, #27 telemetry ringbuf,
#28+ policy compilation, #29 Tier 4 verifier+perf gates) lands code
into one of the crates this ADR mints.

Two architectural questions need to be settled together because the
answer to one constrains the other:

1. **Where does kernel-side eBPF source live, and where does the
   userspace loader live?** eBPF programs compile against
   `bpfel-unknown-none` with `#![no_std]` and `aya-ebpf`; the
   userspace loader compiles against the host triple with `std`,
   `tokio`, and `aya`. The two compile contracts are mutually
   incompatible.

2. **How does cargo coordinate the kernel-side build with the host-side
   build?** The aya-template default invokes cargo recursively from
   `build.rs` — a pattern with well-documented downsides (broken
   workspace caching, opaque error reporting, hostile to incremental
   rebuilds).

These questions extend three established Phase-1 ADRs:

- **ADR-0003** sets the four-value crate-class taxonomy (`core`,
  `adapter-host`, `adapter-sim`, `binary`). New crates must declare
  one of these; the assignment governs whether dst-lint scans the
  crate's source for banned APIs.
- **ADR-0016** restored `overdrive-host` to "host-OS primitive
  bindings" intent and established the per-architectural-class
  extraction pattern. Host-OS adapters get their own crate; squatting
  unrelated subsystems in `overdrive-host` is the failure mode the ADR
  reverses.
- **ADR-0029** extended the same pattern: workload supervision
  (`ExecDriver`, cgroup management, node_health writer) extracts to
  `overdrive-worker` rather than living in `overdrive-host`. Strategic
  rationale: extract per architectural class, eagerly, when the seam
  is clear and no implementation has yet calcified the boundary.

The eBPF dataplane is a third instance of the same shape — Linux-only,
kernel-bound, BPF-target compile contract — and the same strategic
logic applies.

## Decision

### 1. Two new crates: `overdrive-bpf` (kernel) + `overdrive-dataplane` (loader)

**`crates/overdrive-bpf/`** — class `binary`, target
`bpfel-unknown-none`, `#![no_std]`, deps `aya-ebpf` only.

```toml
# crates/overdrive-bpf/Cargo.toml
[package]
name        = "overdrive-bpf"
description = "Kernel-side eBPF programs (XDP/TC/sockops/BPF LSM). Compiles against bpfel-unknown-none with #![no_std]; produces an ELF object consumed by overdrive-dataplane via include_bytes!. Phase 2.1 ships one no-op XDP `xdp_pass` + LruHashMap<u32,u64> packet counter."
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
authors.workspace      = true
repository.workspace   = true
publish                = false

[package.metadata.overdrive]
crate_class = "binary"

[features]
# Workspace-wide convention. Deliberate no-op for this crate — the
# kernel target has no integration-test surface; Tier 2 BPF unit
# tests run via `BPF_PROG_TEST_RUN` from the host side under
# `cargo xtask bpf-unit`. See `.claude/rules/testing.md`
# § Workspace convention.
integration-tests = []

[dependencies]
aya-ebpf.workspace = true

[lints]
workspace = true
```

**`crates/overdrive-dataplane/`** — class `adapter-host`, host triple,
deps `aya` (workspace 0.13) + `overdrive-core` + the standard
`adapter-host` dep set.

```toml
# crates/overdrive-dataplane/Cargo.toml
[package]
name        = "overdrive-dataplane"
description = "Userspace eBPF loader. EbpfDataplane impl of overdrive-core's Dataplane port trait. Embeds the BPF ELF object via include_bytes!. Stub method bodies in Phase 2.1; SERVICE_MAP / sockops / telemetry land in #24/#25/#27."
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
authors.workspace      = true
repository.workspace   = true
publish                = false

[package.metadata.overdrive]
crate_class = "adapter-host"

[features]
integration-tests = []

[dependencies]
overdrive-core.workspace = true
async-trait.workspace    = true
thiserror.workspace      = true
tokio.workspace          = true
tracing.workspace        = true

[target.'cfg(target_os = "linux")'.dependencies]
aya = { workspace = true }

[lints]
workspace = true
```

The `[target.'cfg(target_os = "linux")'.dependencies]` block ensures
aya is pulled in only on Linux. macOS builds compile the loader
without aya at all — the `#[cfg(target_os = "linux")]` arms in the
source provide stub bodies that return `DataplaneError::LoadFailed`.

### 2. `overdrive-bpf` is excluded from `default-members`

Workspace `Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = [
    "crates/overdrive-bpf",            # NEW
    "crates/overdrive-cli",
    "crates/overdrive-control-plane",
    "crates/overdrive-core",
    "crates/overdrive-dataplane",      # NEW
    "crates/overdrive-host",
    "crates/overdrive-scheduler",
    "crates/overdrive-sim",
    "crates/overdrive-store-local",
    "crates/overdrive-worker",
    "xtask",
]
default-members = [
    "crates/overdrive-cli",
    "crates/overdrive-control-plane",
    "crates/overdrive-core",
    "crates/overdrive-dataplane",
    "crates/overdrive-host",
    "crates/overdrive-scheduler",
    "crates/overdrive-sim",
    "crates/overdrive-store-local",
    "crates/overdrive-worker",
    "xtask",
]
# `overdrive-bpf` deliberately omitted from default-members — its
# `bpfel-unknown-none` target requires bpf-linker and rejects host
# tooling. Built explicitly via `cargo xtask bpf-build`.
```

`cargo check --workspace` and `cargo nextest run --workspace` skip the
BPF crate automatically. macOS developers running either command get
a clean compile against the loader (with `#[cfg(target_os = "linux")]`
stub bodies); the kernel crate is never reached.

### 3. Build pipeline — `cargo xtask bpf-build` + `build.rs` artifact-check shim

**Primary mechanism — `cargo xtask bpf-build`:**

A new xtask subcommand that compiles `overdrive-bpf` against the BPF
target and copies the produced ELF to a stable path:

```text
1. which_or_hint("bpf-linker", "<install hint>")
2. cargo +<rust-toolchain> build
       --release
       --target bpfel-unknown-none
       -Z build-std=core
       --manifest-path crates/overdrive-bpf/Cargo.toml
3. cp target/bpfel-unknown-none/release/overdrive-bpf
      target/xtask/bpf-objects/overdrive_bpf.o
```

Stable output path decouples the loader's `include_bytes!` from
cargo's nested target layout.

**Secondary mechanism — `build.rs` shim in `overdrive-dataplane`:**

```rust
// crates/overdrive-dataplane/build.rs
fn main() {
    let path = format!(
        "{}/target/xtask/bpf-objects/overdrive_bpf.o",
        env!("CARGO_WORKSPACE_DIR")
    );
    if !std::path::Path::new(&path).exists() {
        eprintln!(
            "error: BPF object not found at {path}; \
             run `cargo xtask bpf-build` first"
        );
        std::process::exit(1);
    }
    println!("cargo:rerun-if-changed={path}");
}
```

The shim does **not** invoke cargo recursively. It does **not**
attempt to compile the BPF crate. Its only job is to convert a
"file not found in `include_bytes!`" linker error into a single-line
diagnostic that names the fix.

**The `include_bytes!` boundary:**

```rust
// crates/overdrive-dataplane/src/embed.rs
pub(crate) const OVERDRIVE_BPF_OBJ: &[u8] = include_bytes!(concat!(
    env!("CARGO_WORKSPACE_DIR"),
    "target/xtask/bpf-objects/overdrive_bpf.o",
));
```

Cargo never sees `overdrive-bpf` as a transitive dep of
`overdrive-dataplane`. The kernel crate is excluded from
`default-members`, and the `include_bytes!` argument is a path string,
not a Cargo edge.

### 4. Toolchain provisioning — `bpf-linker`

Three provisioning surfaces, all using `cargo install --locked`:

1. **Lima dev VM** (`infra/lima/overdrive-dev.yaml` line 205):
   extend the existing `cargo install --locked cargo-deny
   cargo-nextest cargo-mutants` line to add `bpf-linker`. Existing
   Lima users re-provision; new users get it on first boot.
2. **`cargo xtask dev-setup`** for non-Lima Linux developers — invokes
   `cargo install --locked bpf-linker` after a `which` probe.
3. **`cargo xtask bpf-build`** fails fast at the top via
   `which_or_hint("bpf-linker", "<install hint>")` so a missing tool
   produces a single-line actionable error rather than an opaque
   linker failure.

CI inherits the Lima image's tooling (Linux runners install via the
same line). No CI-only install path.

### 5. `EbpfDataplane` impl shape

`overdrive-dataplane` exposes `EbpfDataplane` as the production
binding of the `Dataplane` port trait from `overdrive-core`. The
trait surface (`update_policy`, `update_service`, `drain_flow_events`)
is unchanged — `EbpfDataplane` mirrors `SimDataplane`'s constructor
pattern at the seam:

```rust
pub struct EbpfDataplane {
    #[cfg(target_os = "linux")]
    bpf: aya::Ebpf,
    #[cfg(target_os = "linux")]
    _link: aya::programs::xdp::XdpLinkId,
}

impl EbpfDataplane {
    #[cfg(target_os = "linux")]
    pub fn new(iface: &str) -> Result<Self, DataplaneError> {
        // Real work in #23: load no-op XDP, attach to `iface`,
        // populate the LruHashMap counter map handle.
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(_iface: &str) -> Result<Self, DataplaneError> {
        Err(DataplaneError::LoadFailed(
            "overdrive-dataplane: non-Linux build target".into()
        ))
    }
}
```

Method bodies in #23:

| Method | #23 body | Filled by |
|---|---|---|
| `EbpfDataplane::new(iface)` | Loads no-op XDP via aya, attaches to `iface`. **Real work.** | n/a — done in #23 |
| `update_policy` | `Ok(())` (no-op stub) | #24 (POLICY_MAP) |
| `update_service` | `Ok(())` (no-op stub) | #25 (SERVICE_MAP) |
| `drain_flow_events` | `Ok(vec![])` (no-op stub) | #27 (telemetry ringbuf) |

Each stub method carries a doc comment naming the issue. `EbpfDataplane`
is **not** wired into `AppState` in #23 — that wiring waits for #24,
which is the first slice with something concrete to call.
`DataplaneError::LoadFailed(String)` already exists in
`crates/overdrive-core/src/traits/dataplane.rs:23` — no new variant.

### 6. xtask harness wiring (partial fill-in)

| Subcommand | Phase 2.1 status |
|---|---|
| `cargo xtask bpf-build` | NEW — implemented as §3 above |
| `cargo xtask bpf-unit` | FILL IN existing stub at `xtask/src/main.rs:565-572` — invokes `cargo nextest run -p overdrive-bpf --features integration-tests --test '*'` against the no-op program's PKTGEN/SETUP/CHECK triptych |
| `cargo xtask integration-test vm latest` | FILL IN existing stub at `xtask/src/main.rs:574-586` — runs the LVH smoke test asserting the no-op XDP loads, attaches to `lo`, increments its counter, detaches cleanly |
| `cargo xtask verifier-regress` | LEAVE STUBBED — add `// TODO(#29): wire when first real program lands` |
| `cargo xtask xdp-perf` | LEAVE STUBBED — add `// TODO(#29): wire when first real program lands` |

Tier 4 gates (verifier complexity, xdp-bench throughput) are
deferred to #29: there is no point baselining a no-op program; the
gates would catch nothing and the baselines would be meaningless.

### 7. Dependency graph after #23

```
overdrive-core    ←  overdrive-scheduler        ←  overdrive-control-plane  ←  overdrive-cli
                  ←  overdrive-host             ←  overdrive-cli
                  ←  overdrive-store-local      ←  overdrive-control-plane
                  ←  overdrive-worker           ←  overdrive-cli
                  ←  overdrive-dataplane (NEW)  ←  overdrive-cli (Phase 2.x: when AppState gains a dataplane field)
                  ←  overdrive-sim (dev/test)

overdrive-bpf  (no Rust dependents — consumed only as a built artifact via include_bytes!)
```

Critical edges:

- `overdrive-dataplane` depends ONLY on `overdrive-core` for the trait
  surface. Same shape as every other `adapter-host` crate.
- `overdrive-control-plane` does NOT depend on `overdrive-dataplane`.
  The dataplane is plugged into `AppState` by the binary at
  composition time — same shape ADR-0029 established for
  `Arc<dyn Driver>`. The `AppState` extension lands when a downstream
  slice gives the control plane something concrete to call (probably
  #24's SERVICE_MAP for `update_service`).
- `overdrive-bpf` has no Rust dependents — it is an artifact-producing
  crate. The `include_bytes!` boundary is a path string, not a Cargo
  edge.

The graph remains acyclic. ADR-0003, ADR-0016, ADR-0024, ADR-0029
all remain consistent with the extended graph.

## Alternatives considered

### Alternative A — Single crate `overdrive-bpf` for kernel + loader

Hold both kernel-side `#![no_std]` modules and the userspace loader in
one crate, gated by `#[cfg(target = "bpfel-unknown-none")]` /
`#[cfg(not(target = "bpfel-unknown-none"))]`, with a `build.rs` that
recursively invokes cargo to compile the kernel modules.

**Rejected.** Three reasons:

- **Recursive cargo from build.rs is the failure mode.** Documented
  pain across the eBPF-Rust ecosystem (aya, libbpf-rs, redbpf): broken
  workspace caching, opaque error messages, hostile to incremental
  rebuilds. The hybrid xtask + build.rs shim in this ADR avoids the
  recursion entirely.
- **Compile contracts are genuinely incompatible.** `#![no_std]` +
  `aya-ebpf` vs `std` + `tokio` + `aya` userspace cannot coexist in
  one Cargo unit without per-cfg dep gating that creates a maintenance
  tax matching feature-flagged crates (which ADR-0029 explicitly
  rejected).
- **Cargo-level boundaries are stronger than `cfg` boundaries.** The
  same strategic logic ADR-0016 used to reject feature-flag class
  boundaries in favor of crate boundaries: a crate boundary is the
  one boundary cargo actually draws.

### Alternative B — Three crates: kernel + shared-types + loader

Add a third `overdrive-bpf-types` crate (`#![no_std]`, no `aya` dep)
that holds wire-format structs crossing the kernel/user boundary;
both kernel and loader depend on it.

**Rejected for #23, deferred to #24.** The no-op XDP program has no
shared types beyond `u32`/`u64` (the `LruHashMap<u32, u64>` packet
counter). Aya's userspace API exposes the map by name without needing
a typed Rust mirror. The shared-types crate becomes warranted in #24
(POLICY_MAP) when typed wire-format structs cross the boundary; it
can land then without disturbing #23's two-crate boundary. Adding it
now would be speculative — Overdrive's pattern (per
`feedback_single_cut_greenfield_migrations` and ADR-0029's strategic
posture) is to extract per architectural class when the seam is
clear, not in anticipation.

### Alternative C — `build.rs` alone, no xtask subcommand

Have `overdrive-dataplane`'s `build.rs` do everything: detect the BPF
crate, invoke cargo recursively, copy the output, embed via
`include_bytes!`.

**Rejected.** Identical to Alternative A's first failure mode —
recursive cargo from build.rs. Plus the workflow this produces
("`cargo check` triggers a 30-second BPF rebuild on every host-side
change") is hostile to inner-loop development. The xtask-primary
design makes the BPF rebuild explicit and on-demand; the build.rs
shim is purely diagnostic.

### Alternative D — `xtask bpf-build` alone, no `build.rs` shim

Drop the `build.rs` and rely on developers to run `cargo xtask
bpf-build` before `cargo check`.

**Rejected.** The first time someone forgets, they get a cryptic
"file not found in include_bytes!" linker error from rustc that
takes minutes to diagnose. The `build.rs` shim costs ~10 lines and
removes the footgun. The hybrid is the published consensus shape
(see Cilium tetragon, post-2024 Aya workspace examples).

### Alternative E — Place the userspace loader in `overdrive-host`

`overdrive-host` already hosts host-OS adapters; add `EbpfDataplane`
as a sibling.

**Rejected.** Re-violates ADR-0016's intent (host-OS primitives
only) — exactly the move ADR-0029 reversed for `ProcessDriver`. The
eBPF dataplane is its own architectural class (Linux-only, BPF
subsystem, kernel-bound) with its own future Phase-2 dependencies
(`bpf-linker`, eventually large per-program test fixtures, eventually
verifier and perf gating). Squatting it in `overdrive-host` would
muddy what that crate is for and would force a rename-and-extract
under pressure once the dataplane grows. Doing the extraction now,
in #23 paper-only, is the cheapest moment — same logic ADR-0029 used
for `overdrive-worker`.

### Alternative F — Defer `bpf-linker` install to operator instructions

Document that operators must `cargo install bpf-linker` themselves;
no Lima provisioning, no xtask dev-setup, no `which_or_hint`.

**Rejected per user feedback memory** (`feedback_no_user_install_
instructions`): hook deny messages and rule docs must not carry
install hints for the user's machine — that is the platform's job.
The Lima image already provisions every other tool the platform
needs (`cloud-hypervisor`, `wasmtime`, `kraft`, `lvh`, `virtme-ng`,
`cargo-deny`, `cargo-nextest`, `cargo-mutants`); `bpf-linker` joins
the same list. Non-Lima Linux developers get `cargo xtask
dev-setup`. The single missing-tool diagnostic at the top of `cargo
xtask bpf-build` is the safety net.

## Consequences

### Positive

- **Compile-graph honesty.** A crate that depends on
  `overdrive-dataplane` is taking on the kernel/BPF surface, and the
  dep graph shows it. No hidden Cargo feature making `EbpfDataplane`
  appear; no recursive build invisibly compiling kernel code.
- **macOS dev experience preserved.** `cargo check --workspace` on
  macOS skips the kernel-side crate via `default-members` exclusion;
  the loader compiles via `#[cfg(target_os = "linux")]` stub bodies.
  The `--no-run` pre-merge gate at the macOS quality boundary
  remains green.
- **Future Phase-2 slices have a clear landing zone.** SERVICE_MAP
  (#24), sockops+kTLS (#25), BPF LSM (#26), telemetry ringbuf (#27),
  policy compilation (#28+), Tier 4 gates (#29) all extend
  `overdrive-bpf` and `overdrive-dataplane` along the seams this
  ADR establishes. No re-extraction under pressure.
- **dst-lint scope unchanged.** Both new crates are non-`core`
  (`binary` and `adapter-host`); the gate's allow-list is unaffected.
  The existing self-test (the `core`-class set is non-empty) still
  passes.
- **Tier 2/3 harness extends rather than reinvents.** The xtask
  subcommand stubs at `xtask/src/main.rs:565-594` were written
  exactly for this moment; #23 fills two of them in and leaves the
  Tier 4 stubs flagged for #29.
- **Toolchain provisioning is platform-managed.** Lima image + xtask
  dev-setup + `which_or_hint` covers every developer surface;
  `bpf-linker` is never an "install this yourself" instruction.

### Negative

- **Workspace grows from 8 crates + xtask to 10 crates + xtask.** Each
  new member adds CI overhead (a few seconds per `cargo check` /
  `cargo clippy` / `cargo nextest run`). The cost is amortised across
  every PR and is small relative to BPF-target compile time.
- **Two crates to maintain instead of one.** Mitigated by the strict
  separation of concerns (kernel vs userspace) being the same
  separation cargo enforces at the target boundary anyway. The
  alternative — `cfg`-gated single crate — pays the same conceptual
  cost without the cargo-boundary support.
- **`bpf-linker` first-install cost on Lima.** `cargo install` from
  source adds ~1–2 minutes to first-boot provisioning. Mitigated:
  one-time cost, the line already exists with three other
  `cargo install` entries. `cargo binstall` is a follow-up if
  friction warrants it.
- **macOS developers cannot run Tier 2/3.** Same constraint that
  governs every `#[cfg(target_os = "linux")]` test surface — they
  must reach for Lima. The `--no-run` compile gate is necessary but
  not sufficient; this ADR does not change that.
- **No `EbpfDataplane` in `AppState` after #23.** The
  `EbpfDataplane::new` constructor does real work (load + attach the
  no-op program), but the type is not wired into the control plane
  until #24 has something to call. Acknowledged as a deliberate
  scope decision per Q7 — #23 ships the seam, the wiring follows
  the data.

### Quality-attribute impact

- **Maintainability — modularity**: positive. Crate boundaries are a
  stronger modularity mechanism than cfg gates. Future Phase-2 slices
  extend along clear seams.
- **Maintainability — testability**: positive. `cargo xtask bpf-unit`
  and `cargo xtask integration-test vm` exercise the no-op program
  end-to-end; the harnesses are real (not stubbed) from #23 onward.
- **Maintainability — analyzability**: positive. `cargo tree -p
  overdrive-dataplane` shows aya as a direct dep; the eBPF surface
  is no longer hidden behind a Cargo feature.
- **Reliability — fault tolerance**: neutral. No runtime semantics
  introduced beyond the no-op program; the load + attach + detach
  lifecycle is the only behaviour exercised.
- **Performance efficiency — time behaviour**: neutral. No-op XDP
  program adds one BPF map lookup per packet on `lo` during the
  smoke test only; not in any production path.
- **Compatibility — interoperability**: positive. The two-crate split
  matches the published shape of the eBPF-Rust ecosystem
  (aya-template's post-2024 layout, Cilium tetragon's structure);
  contributors arriving from those projects find a familiar layout.
- **Portability — installability**: neutral. macOS still cannot run
  the dataplane; that is a Linux-kernel constraint, not an Overdrive
  decision.

### Migration

Phase 2.1 is paper-only at the time of this ADR. Both crates land in
the same PR as the no-op XDP program, the xtask `bpf-build`
subcommand, the filled-in `bpf-unit` + `integration-test vm` stubs,
the Lima `cargo install` line extension, and the brief.md additive
update. No pre-existing source moves; the seam established here is
the one that downstream Phase-2 slices extend.

## Compliance

- **ADR-0003 (crate-class labelling)**: both new crates declare
  `crate_class` (`binary` for `overdrive-bpf`, `adapter-host` for
  `overdrive-dataplane`). Existing class-string allow-list unchanged.
  dst-lint mechanism unaffected — neither new crate is `core`.
- **ADR-0016 (`overdrive-host` extraction)**: original intent
  (host-OS primitives only) preserved. `EbpfDataplane` does NOT land
  in `overdrive-host`; the per-architectural-class extraction
  precedent is followed.
- **ADR-0024 (`overdrive-scheduler` extraction)**: strategic precedent
  — extract per architectural class, eagerly, when the seam is clear.
  Same logic applies one level over.
- **ADR-0029 (`overdrive-worker` extraction)**: closest-precedent ADR.
  Mirrored Cargo.toml shape, mirrored binary-composition pattern
  (composition root in `overdrive-cli`, control-plane crate does NOT
  depend on the new adapter crate), mirrored `[target.'cfg(target_os
  = "linux")'].dependencies` block for Linux-only host deps.
- **Workspace convention** (`.claude/rules/testing.md` § Workspace
  convention): both new crates declare `integration-tests = []`
  (`overdrive-bpf` as a deliberate no-op; `overdrive-dataplane` as
  the gate for any future host-side integration tests it ships). The
  xtask self-test (`every_workspace_member_declares_integration_tests
  _feature`) catches a missing declaration at PR time.
- **Whitepaper §7 (eBPF Dataplane)**: this ADR scaffolds the userspace
  + kernel-side surface the section describes. No deviation from the
  documented architecture; #23 ships the foundation, downstream
  slices ship the substance.
- **Whitepaper §22 (Real-Kernel Integration Testing)**: Tier 2 + Tier
  3 harnesses are wired against the no-op program in #23; Tier 4
  (verifier-regress + xdp-perf) is explicitly deferred to #29 with
  `// TODO(#29)` markers.
- **`feedback_no_user_install_instructions`**: `bpf-linker` is
  provisioned via the platform (Lima image + xtask dev-setup), not
  documented as an "install this yourself" step.
- **`feedback_single_cut_greenfield_migrations`**: #23 lands every
  scaffolding piece in one PR — no deprecation period, no
  feature-flagged old paths, no two-step migration.

## References

- Whitepaper §7 — eBPF Dataplane.
- Whitepaper §19 — Security Model (the dataplane's enforcement
  surface).
- Whitepaper §22 — Real-Kernel Integration Testing (Tier 2/3/4 the
  xtask harness implements).
- ADR-0003 — Core-crate labelling via
  `package.metadata.overdrive.crate_class`.
- ADR-0004 — `overdrive-sim` single crate (sim-side of the dataplane
  port, unmodified by #23).
- ADR-0016 — `overdrive-host` extraction; original intent preserved.
- ADR-0024 — `overdrive-scheduler` extraction; strategic precedent
  one level over.
- ADR-0029 — `overdrive-worker` extraction; closest-precedent ADR
  whose Cargo.toml shape and binary-composition pattern this ADR
  mirrors.
- `crates/overdrive-core/src/traits/dataplane.rs` — the `Dataplane`
  trait surface this ADR's `EbpfDataplane` implements.
- `crates/overdrive-sim/src/adapters/dataplane.rs` — `SimDataplane`
  whose constructor shape `EbpfDataplane::new` mirrors.
- `xtask/src/main.rs:565-594` — existing `bpf-unit` /
  `integration-test vm` / `verifier-regress` / `xdp-perf` stubs;
  #23 fills two in, leaves two stubbed for #29.
- `infra/lima/overdrive-dev.yaml:205` — `cargo install --locked`
  line extended with `bpf-linker`.
- `.claude/rules/testing.md` § Tier 2 / Tier 3 / Workspace
  convention.
- `.claude/rules/development.md` § Port-trait dependencies
  (`overdrive-host` is production, `overdrive-sim` is tests — the
  same pattern `overdrive-dataplane` follows).
- `feedback_no_user_install_instructions`,
  `feedback_single_cut_greenfield_migrations`,
  `feedback_delegate_to_architect` — user feedback informing this
  ADR's posture.
- `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md` —
  feature-scoped DESIGN doc this ADR codifies.
- User Q1–Q9 ratification 2026-05-04 (locked decisions per the
  DESIGN dialogue).
