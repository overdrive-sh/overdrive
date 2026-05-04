# DESIGN Decisions — phase-2-aya-rs-scaffolding

**Issue:** GH #23 — `[2.1] aya-rs eBPF scaffolding + build pipeline`
**Wave:** DESIGN
**Architect:** Morgan
**Date:** 2026-05-04
**Mode:** Guide → autonomous artifact production (Phases 4–7)
**ADRs produced:** ADR-0038
**User ratification:** Q1–Q9 locked decisions (orchestrator brief, 2026-05-04)

---

## Key Decisions

- **[D1] Two crates: `overdrive-bpf` (kernel) + `overdrive-dataplane` (loader).** The `#![no_std]` + `bpfel-unknown-none` compile contract for kernel-side eBPF is incompatible with the `std`+`tokio`+`aya` userspace contract. Cargo-level boundaries are stronger than `cfg` boundaries (see ADR-0016 strategic precedent). Three-crate split (adding `overdrive-bpf-types`) is premature — the no-op program has no shared types beyond `u32`/`u64`; defer to #24 when POLICY_MAP needs typed wire-format structs. (see: `architecture.md` §2; `adr-0038-...md` §1, Alternatives A & B)
- **[D2] Crate classes per ADR-0003.** `overdrive-bpf` declares `crate_class = "binary"` (artifact-producing, not a library); `overdrive-dataplane` declares `crate_class = "adapter-host"` (production binding of the `Dataplane` port to the kernel's BPF subsystem). Both classes are non-`core`; dst-lint scope is unaffected. (see: `architecture.md` §2, §8; `adr-0038-...md` §1)
- **[D3] Hybrid build pipeline: `cargo xtask bpf-build` (primary) + `build.rs` artifact-check shim (defensive).** Recursive cargo from `build.rs` (the aya-template default) is the failure mode — broken workspace caching, opaque error messages, hostile to incremental rebuilds. The xtask subcommand makes the BPF rebuild explicit and on-demand; the `build.rs` shim's only job is to convert "file not found in `include_bytes!`" into a single-line diagnostic naming the fix. Output stable path: `target/xtask/bpf-objects/overdrive_bpf.o`. (see: `architecture.md` §3; `adr-0038-...md` §3, Alternatives C & D)
- **[D4] `bpf-linker` provisioned by the platform, not by the user.** Three surfaces: (a) Lima image `cargo install --locked` line at `infra/lima/overdrive-dev.yaml:205` extended with `bpf-linker`; (b) `cargo xtask dev-setup` for non-Lima Linux developers; (c) `which_or_hint` at the top of `cargo xtask bpf-build`. CI inherits via the same Lima image. Per `feedback_no_user_install_instructions` user memory — the platform never tells the user to install tools manually. (see: `architecture.md` §4; `adr-0038-...md` §4)
- **[D5] macOS dev story: kernel crate excluded from `default-members`; loader compiles via `#[cfg(target_os = "linux")]` stub bodies.** Workspace gains a NEW `default-members` declaration that omits `crates/overdrive-bpf`. `EbpfDataplane`'s aya-touching methods are gated `#[cfg(target_os = "linux")]`; non-Linux fallthrough returns `DataplaneError::LoadFailed("non-Linux build target")`. The `--no-run` macOS pre-merge gate per `.claude/rules/testing.md` § "Running tests on macOS — Lima VM" remains the authoritative compile check. (see: `architecture.md` §5; `adr-0038-...md` §2)
- **[D6] No-op XDP `xdp_pass` + `LruHashMap<u32,u64>` packet counter, attached to `lo`.** One observable signal: counter > 0 after traffic. Tier 3 acceptance test asserts via `bpftool map dump` per the `.claude/rules/testing.md` § "Assertion rules" discipline (assert on observable kernel side effects, never on internal program reachability). (see: `architecture.md` §6; `adr-0038-...md` §5)
- **[D7] Stub `EbpfDataplane` impl ships now.** `EbpfDataplane::new(iface)` does real work (load + attach the no-op program); `update_policy`, `update_service`, `drain_flow_events` return `Ok(())` / empty `Vec` with doc comments naming the issue that fills them in (#24, #25, #27). The constructor returns `Result<Self, DataplaneError>` — uses the existing `DataplaneError::LoadFailed(String)` variant; no new error shape. (see: `architecture.md` §7; `adr-0038-...md` §5)
- **[D8] xtask harness — partial fill-in.** `cargo xtask bpf-build` is NEW. `cargo xtask bpf-unit` and `cargo xtask integration-test vm latest` fill in the existing stubs at `xtask/src/main.rs:565-572` and `:574-586` respectively. `cargo xtask verifier-regress` and `cargo xtask xdp-perf` remain stubbed with `// TODO(#29): wire when first real program lands` — there is no point baselining a no-op program. (see: `architecture.md` §6; `adr-0038-...md` §6)
- **[D9] `EbpfDataplane` is NOT wired into `AppState` in #23.** The constructor does real work, but the binary-composition edge (the slice that gives the control plane an `Arc<dyn Dataplane>` to call) waits for the first slice with something concrete to call (probably #24's SERVICE_MAP for `update_service`). Same pattern ADR-0029 used for `Arc<dyn Driver>`. (see: `architecture.md` §7, §9; `adr-0038-...md` §5)

---

## Architecture Summary

- **Pattern:** Ports-and-adapters / hexagonal architecture. The new `Dataplane` adapter (`EbpfDataplane`) sits on the production side of the same port trait that `SimDataplane` sits on for tests.
- **Paradigm:** object-oriented (Rust trait-based; per CLAUDE.md project paradigm).
- **Key components:**
  - `crates/overdrive-bpf/` — NEW. Kernel-side eBPF programs. Class `binary`. `#![no_std]`, target `bpfel-unknown-none`, deps `aya-ebpf` only.
  - `crates/overdrive-dataplane/` — NEW. Userspace BPF loader. Class `adapter-host`. Hosts `EbpfDataplane`. Embeds the BPF object via `include_bytes!`. Compiles on macOS via `#[cfg(target_os = "linux")]` stub bodies.
  - `xtask` — EXTENDED. New `bpf-build` subcommand; existing `bpf-unit` and `integration-test vm` stubs filled in; `verifier-regress` and `xdp-perf` stubs preserved.
  - `infra/lima/overdrive-dev.yaml` — EXTENDED. `cargo install --locked` line at L205 gains `bpf-linker`.
  - Workspace root `Cargo.toml` — EXTENDED. `members` grows from 9 to 11 entries; NEW `default-members` declaration omits `overdrive-bpf`. `aya-ebpf` added to `[workspace.dependencies]`.

---

## Reuse Analysis

| Existing Component | File | Overlap | Decision | Justification |
|---|---|---|---|---|
| `Dataplane` trait | `crates/overdrive-core/src/traits/dataplane.rs` | THE port we're implementing | EXTEND (impl trait) | Trait is fixed; we satisfy it. `DataplaneError::LoadFailed(String)` already covers the non-Linux build target case — no new error variant. |
| `SimDataplane` | `crates/overdrive-sim/src/adapters/dataplane.rs` | Sibling impl on the other side of the port | MIRROR shape | `EbpfDataplane::new()` mirrors `SimDataplane::new()` at the constructor seam. Not modified by #23 — its no-op responses are already correct against the trait; new map shapes land in lockstep with their kernel-side counterparts in later slices. |
| `overdrive-host` crate | `crates/overdrive-host/Cargo.toml`, `src/lib.rs` | Existing host adapter crate | SIBLING (do NOT extend) | `overdrive-host` is host-OS primitives only per ADR-0016 (restored by ADR-0029). Adding kernel/aya dependencies would re-violate that intent. New crate is the right shape. |
| `overdrive-worker` Cargo.toml | `crates/overdrive-worker/Cargo.toml` | Closest-precedent `adapter-host` Cargo.toml | TEMPLATE | Use as template — same crate-class declaration, same `integration-tests = []` no-op block, same `[target.'cfg(target_os = "linux")']` block pattern. |
| `xtask::which_or_hint` | `xtask/src/main.rs:553` | Tool-presence probe | EXTEND (call from `bpf-build`) | Existing helper; `bpf-build` calls it for `bpf-linker`. Zero new code. |
| `xtask::bpf_unit` stub | `xtask/src/main.rs:565-572` | Existing placeholder | FILL IN | Body becomes `cargo nextest run -p overdrive-bpf --features integration-tests --test '*'`. |
| `xtask::integration_vm` stub | `xtask/src/main.rs:574-586` | Existing placeholder | FILL IN | Body becomes the LVH invocation per `.claude/rules/testing.md` § Tier 3. |
| `xtask::verifier_regress` / `xdp_perf` stubs | `xtask/src/main.rs:588-594` | Existing placeholders | LEAVE STUBBED | Per Q8: deferred to #29. Add `// TODO(#29): wire when first real program lands` comment. |
| `infra/lima/overdrive-dev.yaml` cargo-install line | line 205 | Existing tool-install enumeration | EXTEND | Add `bpf-linker` to existing `cargo install --locked cargo-deny cargo-nextest cargo-mutants` line. |
| Workspace `Cargo.toml` `[workspace.dependencies]` | `Cargo.toml:24-122` | aya already at line 85 | REUSE + ADD | aya already declared at 0.13. Add `aya-ebpf.workspace = true` declaration for the kernel-side crate. |
| Workspace `Cargo.toml` `members` | `Cargo.toml:3-13` | Existing 9-member declaration | EXTEND | Add `overdrive-bpf` and `overdrive-dataplane`. NEW `default-members` declaration excludes the kernel crate. |
| ADR-0003 (crate-class taxonomy) | `docs/product/architecture/adr-0003-...md` | Class-string allow-list | REUSE (no change) | Both new crates use existing values (`binary`, `adapter-host`). |
| ADR-0016 (overdrive-host extraction) | `docs/product/architecture/adr-0016-...md` | Adapter-extraction precedent | CITE | Same strategic shape: extract per architectural class. |
| ADR-0029 (overdrive-worker extraction) | `docs/product/architecture/adr-0029-...md` | Closest-precedent extraction ADR | TEMPLATE | Mirror its structure (Status/Context/Decision/Alternatives/Consequences/Compliance/References). |

**CREATE NEW justification.** Two CREATE NEW entries (`overdrive-bpf`, `overdrive-dataplane`) are justified by: (a) no existing crate hosts kernel-side BPF source; (b) no existing crate hosts a userspace BPF loader; (c) ADR-0029 establishes the per-architectural-class extraction pattern, and eBPF is a distinct architectural class (Linux-only, BPF target, kernel-bound); (d) adding either responsibility to `overdrive-host` would re-violate ADR-0016's intent — exactly the move ADR-0029 reversed for ProcessDriver.

---

## Technology Stack

- **`aya = "0.13"`** (workspace dep, already declared) — userspace BPF loader API. MIT/Apache-2.0. Pure Rust. Maintained, broad ecosystem use (Cilium adjacent, Falco/Tetragon-adjacent). Production-ready per `.claude/rules/testing.md` § Tier 2/3.
- **`aya-ebpf`** (NEW workspace dep, latest 0.1.x) — kernel-side eBPF API for Rust. MIT/Apache-2.0. Sibling crate of `aya`. The only dep `overdrive-bpf` carries.
- **`bpf-linker`** (cargo-installable, NOT a Rust dep) — LLVM-15+ wrapper that resolves BPF relocations. MIT/Apache-2.0. Provisioned via Lima image + xtask dev-setup + which-or-hint. Pinned via `--locked`.
- **`thiserror`, `tokio`, `tracing`, `async-trait`** — already workspace deps; standard `adapter-host` set. `overdrive-dataplane` reuses them.
- **`include_bytes!`** — built-in macro; the BPF artifact crosses the kernel/user boundary as a path string, not a Cargo edge.

All OSS, all permissive licenses (MIT/Apache-2.0). No proprietary dependencies. No new build-tool dependencies beyond `bpf-linker` (which is itself MIT/Apache-2.0).

---

## Constraints Established

- **Linux-only runtime surface for the dataplane.** macOS dev gets compile-time coverage only (`#[cfg(target_os = "linux")]` stub bodies + `--no-run` pre-merge gate). Tier 2/3 require Lima.
- **Kernel-side crate excluded from `default-members`.** New workspace-level constraint; `cargo check --workspace` from any host will skip `overdrive-bpf`. Explicit `cargo xtask bpf-build` is the only way to compile it.
- **Stable BPF artifact path.** `target/xtask/bpf-objects/overdrive_bpf.o` is load-bearing — the `include_bytes!` in the loader and the `build.rs` artifact-check shim both reference it. Future xtask refactors must preserve the path or the loader breaks.
- **No recursive cargo from `build.rs`.** The hybrid pipeline's defining property; documented in `adr-0038-...md` § Alternative C as a rejected design.
- **`bpf-linker` is platform-managed, not user-managed.** The platform installs it; the user never sees an "install this yourself" instruction. Per `feedback_no_user_install_instructions`.
- **Tier 4 deferred to #29.** `verifier-regress` and `xdp-perf` stubs preserved with `// TODO(#29)` markers.
- **`AppState` extension deferred.** `EbpfDataplane` is constructed by the binary in #23 (real load + attach), but not threaded into `AppState` until a downstream slice has something to call.

---

## Upstream Changes

- (no upstream changes — no prior-wave artifacts existed for this feature)

See `upstream-changes.md` for detail.
