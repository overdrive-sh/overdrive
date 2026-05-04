# Infrastructure Integration — Phase 2.1 aya-rs eBPF scaffolding

**Feature ID:** `phase-2-aya-rs-scaffolding`
**Driving issue:** GH #23 — `[2.1] aya-rs eBPF scaffolding + build pipeline`
**Wave:** DEVOPS
**Architect:** Apex
**Date:** 2026-05-04

---

## 1. Scope

#23 ships workspace crates compiled by cargo. There is no deployable
artifact, no container, no cloud resource. The infrastructure surface
is exactly **one Lima VM image change** (and an inherent compatibility
audit of the existing system-mode toolchain).

CI runner provisioning is documented separately in
`ci-cd-pipeline.md`; this document covers the developer-side Lima
image only.

---

## 2. Lima image change

Single edit to `infra/lima/overdrive-dev.yaml`, line 205. The current
line is:

```yaml
cargo install --locked cargo-deny cargo-nextest cargo-mutants || true
```

becomes:

```yaml
cargo install --locked cargo-deny cargo-nextest cargo-mutants bpf-linker || true
```

This is the verbatim one-line change mandated by ADR-0038 §4 and
DESIGN wave-decisions D4. No other Lima YAML edits are required.

### 2.1 Provisioning semantics

- **First-boot users:** receive `bpf-linker` automatically alongside
  the existing three cargo-installed binaries. No additional
  documentation surface.
- **Existing users:** must `limactl stop overdrive && limactl start
  overdrive --reload` (or the project-equivalent) to re-run the
  user-mode provisioning script. The `cargo install --locked` line
  is idempotent — re-running it on an already-provisioned VM
  installs only `bpf-linker` (the other three are already present
  and `cargo install` skips them).
- **The trailing `|| true`** matches the existing line's posture —
  if a single binary's install fails (e.g. transient crates.io
  outage), the provisioning script proceeds rather than blocking
  every other downstream provisioning step. The `which_or_hint`
  guard at the top of `cargo xtask bpf-build` (per ADR-0038 §4)
  catches the `bpf-linker` case at use time with a single-line
  actionable error.

### 2.2 Wall-clock impact

`cargo install --locked bpf-linker` from source is ~1–2 min on the
Lima VM's allocated CPU/memory. First-boot provisioning currently
takes ~10–15 min (the cloud-hypervisor download, kraft .deb install,
Go-based LVH install, virtme-ng pipx install, plus three cargo-installed
binaries dominate); adding one more cargo-install is in the noise.

---

## 3. Toolchain compatibility audit

ADR-0038 §4 claims the existing Lima toolchain "covers what aya
needs" with `bpf-linker` as the only missing piece. Verifying:

| Tool | Currently installed (lines 100–110 of `overdrive-dev.yaml`) | aya/bpf-linker requirement | Coverage |
|---|---|---|---|
| `clang` | yes (apt) | bpf-linker uses LLVM-15+ wrapper around clang for cross-compile | covered |
| `lld` | yes (apt) | bpf-linker uses LLVM linker | covered |
| `mold` | yes (apt) | host-side `.cargo/config.toml` linker; not used for BPF target | covered |
| `llvm` | yes (apt) | bpf-linker requires LLVM-15+ | covered (Ubuntu 24.04 ships LLVM 18) |
| `libclang-dev` | yes (apt) | aya's `bindgen`-based codegen needs libclang | covered |
| `libelf-dev` | yes (apt) | bpf-linker writes ELF objects | covered |
| `libbpf-dev` | yes (apt) | bpf-linker links against libbpf for relocation logic | covered |
| `linux-libc-dev` | yes (apt) | kernel UAPI headers required for aya-ebpf compile | covered |
| `linux-tools-common` / `-generic` | yes (apt) | provides `bpftool` (used by Tier 3 assertions) | covered |
| `bpftool` | yes (symlinked at `/usr/local/bin/bpftool`, lines 113–116) | Tier 3 `bpftool map dump` assertions | covered |
| `xdp-tools` | yes (apt) | provides `xdp-trafficgen` and `xdp-bench` (used by Tier 4, scope of #29) | covered (over-provisioned for #23) |
| `bpfcc-tools` | yes (apt) | BCC tooling — useful for ad-hoc debugging; not required by #23 | covered (over-provisioned) |
| `qemu-system-x86`/`qemu-system-arm`/`qemu-utils`/`qemu-kvm` | yes (apt) | LVH boots a kernel inside QEMU for Tier 3 | covered |
| `virtiofsd` | yes (apt) | required for the broader Lima VM workflow; not directly used by Tier 3 BPF tests | covered |
| `tcpdump`/`iproute2`/`bridge-utils` | yes (apt) | Tier 3 wire-capture assertions | covered (over-provisioned for #23) |
| `lvh` (Go-installed, line 209) | yes | the Tier 3 harness binary | covered |
| `virtme-ng` (pipx, line 215) | yes | dev-laptop alternative to LVH for fast kernel boot | covered |
| `cargo-nextest` (cargo install, line 205) | yes | the project test runner | covered |
| **`bpf-linker`** | **NOT installed currently** | **resolves BPF relocations during `bpfel-unknown-none` link** | **GAP — fixed by §2 above** |

**Conclusion.** One missing piece. The existing toolchain is
otherwise complete for #23's needs (and over-provisioned for the
Tier 3 / Tier 4 work of #29 that is already partially anticipated
in the system-mode apt list).

### 3.1 Rust toolchain note

The Lima `user`-mode script (line 198) installs rustup with `--default-toolchain none --profile minimal`; the in-repo `rust-toolchain.toml` then governs the active channel. ADR-0038's
build pipeline requires `-Z build-std=core` which depends on the
`rust-src` component. Line 204 already runs:

```yaml
rustup component add rustfmt clippy rust-src || true
```

so `rust-src` is provisioned for the active toolchain. No edit needed
to that line — it is already correct.

---

## 4. What does NOT change

For clarity, this DEVOPS slice does not modify:

- `infra/lima/overdrive-dev.yaml` system-mode scripts (lines 80–187).
- The Lima `param` block, `mounts`, `probes`, `portForwards`, or
  `env` sections.
- `.cargo/config.toml` (the host linker config — mold remains the
  host linker; `bpfel-unknown-none` builds use bpf-linker via
  cargo's per-target linker selection in the BPF crate's local
  config, not the workspace root).
- Any other `infra/` files (none exist for #23).

---

## 5. CI/Lima parity statement

After the change in §2, every developer surface that builds eBPF
has `bpf-linker` available:

| Surface | Provisioning path |
|---|---|
| Lima dev VM (macOS hosts) | `cargo install --locked bpf-linker` line in `infra/lima/overdrive-dev.yaml` |
| Linux native dev (no Lima) | `cargo xtask dev-setup` (xtask wrapper around `cargo install --locked bpf-linker`) |
| GitHub Actions `ubuntu-latest` (CI) | `cargo install --locked bpf-linker` step in the `bpf-build` job (see `ci-cd-pipeline.md` §2.1) |
| `cargo xtask bpf-build` itself | `which_or_hint("bpf-linker", "<install hint>")` runs first; missing tool produces a single-line actionable error |

Three surfaces, one install primitive (`cargo install --locked`),
one diagnostic (`which_or_hint`). Per ADR-0038 §4 and the
`feedback_no_user_install_instructions` user memory: the user is
never told to install the tool manually.

---

## 6. References

- `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md`
  §4 — toolchain provisioning specification.
- `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md`
  §4 — bpf-linker provisioning across Lima + xtask + which-or-hint.
- `infra/lima/overdrive-dev.yaml` lines 100–110 (system-mode apt
  package list), line 205 (user-mode cargo-install line).
- `feedback_no_user_install_instructions` user memory.
