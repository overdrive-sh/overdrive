# CI/CD Pipeline — Phase 2.1 aya-rs eBPF scaffolding

**Feature ID:** `phase-2-aya-rs-scaffolding`
**Driving issue:** GH #23 — `[2.1] aya-rs eBPF scaffolding + build pipeline`
**Wave:** DEVOPS
**Architect:** Apex
**Date:** 2026-05-04

---

## 1. Scope

#23 ships workspace crates (kernel-side `overdrive-bpf` + userspace
loader `overdrive-dataplane`) and the build/test pipeline that compiles
and exercises a no-op XDP program end-to-end. There is **no
deployable artifact** — Overdrive itself is the platform-as-a-product
(per project SSOT, whitepaper). The DEVOPS surface for #23 is therefore
strictly a CI extension plus one Lima image edit.

This document specifies the three new GitHub Actions jobs that #23
must add to `.github/workflows/ci.yml`, scoped to single-kernel CI per
the locked decision (full kernel matrix `[5.10, 5.15, 6.1, 6.6, latest
LTS] + bpf-next` is the scope of the dedicated harness-bootstrap issue
#29).

Tier 4 jobs (`verifier-regress`, `xdp-perf`, PREVAIL) remain stubbed
per ADR-0038 §6 — wiring them against a no-op program would produce
meaningless baselines. Phase 2.1 does not add Tier 4 CI jobs.

---

## 2. CI jobs added by #23

Three new jobs, ordered by their position in the build graph.
All run on `ubuntu-latest`. None require self-hosted KVM-capable
runners (the Tier 3 LVH harness uses `--qemu-disable-kvm` per
`.claude/rules/testing.md` §"Harness").

### 2.1 `bpf-build` — compile the kernel-side ELF

**Purpose.** Run `cargo xtask bpf-build` to produce
`target/xtask/bpf-objects/overdrive_bpf.o`. This is the artifact every
later test job consumes.

**Triggers.** `pull_request` to `main`; `push` to `main`;
`workflow_dispatch`. Same triggers as the existing five jobs.

**Runner.** `ubuntu-latest`.

**Wall-clock budget.** ≤ 8 min. The dominant cost is `cargo install
--locked bpf-linker` from source on cache miss (~1–2 min once warm).
The actual BPF-target build is fast (small `#![no_std]` crate, no
deps beyond `aya-ebpf`).

**Step graph.**

| # | Step | Notes |
|---|---|---|
| 1 | `actions/checkout@v4` | Standard. No `fetch-depth: 0` needed (only `mutants-diff` requires history). |
| 2 | `dtolnay/rust-toolchain@stable` | Match existing jobs. `rust-toolchain.toml` pins the channel. Add `with: components: rust-src` because `cargo +<toolchain> build -Z build-std=core` requires `rust-src`. |
| 3 | `Install mold linker` | Same shell snippet as existing jobs (`sudo apt-get install -y --no-install-recommends mold`). The bpf-target build itself does not use mold, but this matches `.cargo/config.toml`. |
| 4 | `Swatinem/rust-cache@v2` | `shared-key: "phase-2-bpf-build"`. Independent cache from the existing five `phase-1-*` keys so phase-1 cache invalidation does not cascade. |
| 5 | sccache setup (soft-fail prelude) | Verbatim copy of the four-step prelude from existing jobs (lines 96–109 of `ci.yml`). Same rationale: GHA cache-backend outage degrades to direct rustc, never fails the job. |
| 6 | `Install bpf-linker` | `cargo install --locked bpf-linker`. Cached by `Swatinem/rust-cache`'s `~/.cargo/bin` cache after the first run. The `--locked` flag is mandatory per ADR-0038 §4. |
| 7 | `cargo xtask bpf-build` | Primary command. The xtask wrapper (a) calls `which_or_hint("bpf-linker", ...)`, (b) invokes `cargo build --release --target bpfel-unknown-none -Z build-std=core --manifest-path crates/overdrive-bpf/Cargo.toml`, (c) copies the ELF to `target/xtask/bpf-objects/overdrive_bpf.o`. |
| 8 | `Upload BPF object artifact` | `actions/upload-artifact@v4` with `name: bpf-object`, `path: target/xtask/bpf-objects/overdrive_bpf.o`, `retention-days: 14`, `if-no-files-found: error`. Downstream jobs (`bpf-unit`, `integration-test-vm-latest`) consume this artifact instead of re-running `bpf-build`. |

**Caching strategy.** Two complementary caches:

- `Swatinem/rust-cache@v2` (`phase-2-bpf-build` key) caches `~/.cargo`
  (registry index, downloaded crates, installed binaries incl.
  `bpf-linker`) and the `target/` directory across PR runs on the
  same SHA family.
- sccache (soft-fail) caches individual `rustc` compilation units via
  the GHA cache backend. Same prelude as existing jobs — RUSTC_WRAPPER
  is set only when sccache-action's setup step succeeds; on outage,
  the build proceeds without sccache.

**bpf-linker provisioning trade-off (chosen path).** The job uses
`cargo install --locked bpf-linker` directly rather than calling
`cargo xtask dev-setup`. Rationale: keeping CI's install path
explicit (one shell line) is easier to debug than a nested xtask
invocation, and the xtask `dev-setup` subcommand is itself a thin
wrapper around `cargo install --locked bpf-linker`. The Lima image
keeps the same primitive (also `cargo install --locked` per ADR-0038
§4) — both surfaces use the same install command, so a regression in
either surface is easy to triage. ADR-0038 §4 explicitly permits
this trade-off ("CI inherits the Lima image's tooling automatically
[...] No additional CI-only install path"); we exercise the same
primitive without the indirection.

**Artifact contract (downstream consumers).** The
`actions/upload-artifact` produces a single ELF at the workflow's
artifact store. Both downstream jobs (`bpf-unit`,
`integration-test-vm-latest`) declare `needs: bpf-build` and use
`actions/download-artifact@v4` with `name: bpf-object` to materialise
the same ELF at `target/xtask/bpf-objects/overdrive_bpf.o` — the
stable path mandated by ADR-0038 §3 that the loader's `include_bytes!`
references.

---

### 2.2 `bpf-unit` — Tier 2 BPF unit tests

**Purpose.** Run `cargo xtask bpf-unit`, which invokes `cargo nextest
run -p overdrive-bpf --features integration-tests --test '*'` against
the no-op program's PKTGEN/SETUP/CHECK triptych (per ADR-0038 §6 +
`.claude/rules/testing.md` §"Tier 2"). Asserts via
`BPF_PROG_TEST_RUN` that the `xdp_pass` program returns `XDP_PASS`
and the `LruHashMap<u32,u64>` packet counter increments from 0 to 1.

**Triggers.** Same as `bpf-build`.

**Runner.** `ubuntu-latest`.

**Dependencies.** `needs: bpf-build` — consumes the BPF object
artifact.

**Wall-clock budget.** ≤ 5 min. `BPF_PROG_TEST_RUN` is a single
syscall per test case; the fixture set is small (one program, one
map). Most of the wall-clock is cargo's incremental rebuild of the
test binary plus aya's dependency graph.

**Step graph.**

| # | Step | Notes |
|---|---|---|
| 1 | `actions/checkout@v4` | Standard. |
| 2 | `dtolnay/rust-toolchain@stable` | Match existing jobs. |
| 3 | `Install mold linker` | Match existing jobs. |
| 4 | `Swatinem/rust-cache@v2` | `shared-key: "phase-2-bpf-unit"`. |
| 5 | sccache prelude | Verbatim copy of the four-step prelude. |
| 6 | `Install cargo-nextest` | `taiki-e/install-action@v2` with `tool: cargo-nextest`. Match the existing `test` and `integration` jobs. |
| 7 | `Download BPF object artifact` | `actions/download-artifact@v4` with `name: bpf-object`, `path: target/xtask/bpf-objects/`, `if-no-files-found: error`. Materialises `overdrive_bpf.o` at the path the loader's `include_bytes!` and the `build.rs` shim both reference. The `if-no-files-found: error` setting is load-bearing — without it a missing-artifact case (e.g. `bpf-build` panicked but emitted nothing) silently no-ops and the downstream failure surfaces as a confusing `build.rs` shim error. Fail fast at the boundary. |
| 8 | `cargo xtask bpf-unit` | Primary command. The xtask wrapper invokes nextest scoped to `-p overdrive-bpf --features integration-tests --test '*'` per ADR-0038 §6. |
| 9 | `Upload nextest JUnit report` | `actions/upload-artifact@v4` with `name: bpf-unit-junit`, `path: target/nextest/ci/junit.xml`, `retention-days: 14`, `if-no-files-found: warn`. Match the existing `test` job's artifact upload pattern. |

**Capability requirements.** None at the runner level —
`BPF_PROG_TEST_RUN` does not require CAP_BPF; aya's `Program::test_run`
wraps the kernel syscall and runs as the runner user. This is the
explicit Tier-2 boundary per `.claude/rules/testing.md`: Tier 2
proves program-level correctness against curated input; Tier 3 is
where load+attach+enforce on a real kernel is exercised.

---

### 2.3 `integration-test-vm-latest` — Tier 3 LVH smoke test

**Purpose.** Run `cargo xtask integration-test vm latest`, which
boots an LVH VM with the latest LTS kernel, loads the no-op XDP
program, attaches it to `lo`, generates traffic, asserts the
`LruHashMap<u32,u64>` counter incremented (via `bpftool map dump`),
detaches cleanly. Asserts on observable kernel side effects per
`.claude/rules/testing.md` §"Assertion rules" — never on internal
program reachability.

**Triggers.** Same as `bpf-build`.

**Runner.** `ubuntu-latest` with `--qemu-disable-kvm` (no nested
virtualisation on stock GHA runners; the LVH harness handles this
flag — same approach Tetragon and Cilium take per testing.md §Harness).

**Dependencies.** `needs: bpf-build`.

**Wall-clock budget.** ≤ 10 min. The dominant cost is LVH boot
(~30 s) + kernel image pull on cache miss (~1 min) + the BPF
load+attach+detach lifecycle (~5 s). The aya/cargo build overlaps
with the kernel pull. testing.md's per-PR critical path budget is
~15 min total; this job alone is the largest single contributor.

**Step graph.**

| # | Step | Notes |
|---|---|---|
| 1 | `actions/checkout@v4` | Standard. |
| 2 | `dtolnay/rust-toolchain@stable` | Match existing jobs. |
| 3 | `Install mold linker` | Match existing jobs. |
| 4 | `Swatinem/rust-cache@v2` | `shared-key: "phase-2-integration-vm-latest"`. |
| 5 | sccache prelude | Verbatim copy. |
| 6 | `Install LVH + qemu` | `sudo apt-get install -y --no-install-recommends qemu-system-x86 qemu-utils`. LVH itself is provisioned via `go install github.com/cilium/little-vm-helper/cmd/lvh@latest` — match Lima's pattern. Cached by `Swatinem/rust-cache`'s GOPATH-adjacent caching. |
| 7 | `Cache LVH kernel images` | Separate `actions/cache@v4` keyed on the LVH image tag for `latest`. Avoids re-downloading the ~80 MB OCI kernel image on every run. |
| 8 | `Install cargo-nextest` | Match existing jobs. |
| 9 | `Download BPF object artifact` | Per `bpf-unit` step 7 — `actions/download-artifact@v4` with `if-no-files-found: error`. Same fail-fast rationale: a missing object surfaces as a confusing LVH-side error otherwise. |
| 10 | `cargo xtask integration-test vm latest` | Primary command. The xtask wrapper invokes LVH per ADR-0038 §6 and `.claude/rules/testing.md` §Tier 3. The wrapper uses `--qemu-disable-kvm` automatically when the runner lacks `/dev/kvm` (matches Tetragon's flow). |
| 11 | `Upload bpftool map dump on failure` | `if: failure()`; uploads `target/xtask/integration-vm-latest.log` (the LVH/bpftool transcript). Matches the existing `dst` job's failure-artifact pattern. |
| 12 | `Annotate failure with reproduction command` | `if: failure()`; emits `cargo xtask lima run -- cargo xtask integration-test vm latest` to `$GITHUB_STEP_SUMMARY`. Same pattern as the existing `dst` and `mutants-diff` jobs. |

**bpf-linker on this job.** Not needed — `bpf-build` already produced
the ELF. The download-artifact step delivers the compiled object to
the path the loader expects.

**Why single-kernel only for #23.** `.claude/rules/testing.md` §Tier 3
documents the full matrix as `[5.10, 5.15, 6.1, 6.6, latest LTS,
bpf-next]`. Issue #29 is the dedicated "real-kernel integration test
harness bootstrap" that expands the matrix; #23 stays narrow to keep
the PR critical path under 15 min. When #29 lands, this job becomes
a `strategy: matrix:` over the kernel set and the job name pluralises
(`integration-test-vm-{kernel}`). The single-kernel job in #23 IS
the harness foundation #29 expands.

**Capability requirements at runtime.** The aya `Program::load` +
`xdp::attach` codepath inside the LVH VM requires CAP_BPF +
CAP_NET_ADMIN. The LVH harness boots the VM with the test process
running as root (per `.claude/rules/testing.md` §"Cgroup writes need
root or delegation" — same pattern), so the capabilities are
present transparently. Production deployment (Phase 7+ via
meta-overdrive) is a separate concern flagged in ADR-0038 §9.

---

## 3. CI-level observability

#23 has no application-level observability surface — the eBPF
telemetry pipeline ships in #31 (whitepaper §12). For #23, the
CI-level signal is:

1. **Job duration.** GitHub Actions records job wall-clock
   automatically; the `bpf-build` / `bpf-unit` /
   `integration-test-vm-latest` jobs surface against the budgets in
   §2 above.
2. **Job pass/fail.** Standard GitHub Actions surfacing.
3. **Artifact uploads on failure.** `integration-test-vm-latest`
   uploads the bpftool map dump transcript on failure for debugging
   (standard pattern matching the existing `dst` job).
4. **JUnit reports.** `bpf-unit` uploads a nextest JUnit report (same
   pattern as the existing `test` and `integration` jobs).

No new dashboard, no new alerting, no new metrics pipeline.
testing.md's K1 wall-clock guardrail (DST < 60 s) does not apply to
the new jobs — those are per-job budgets documented above.

---

## 4. Branch protection (required-status-checks)

Three new required-status-checks must be added to the `main` branch
protection rules on GitHub (Settings > Branches > Branch protection
rules), in addition to the existing six:

| Existing | New (added by #23) |
|---|---|
| `fmt-clippy` | `bpf-build` |
| `test` | `bpf-unit` |
| `dst` | `integration-test-vm-latest` |
| `dst-lint` | |
| `yaml-free-cli` | |
| `mutants-diff` | |

The `integration` job is intentionally NOT in the existing required
list (it runs `--features integration-tests` for snapshot-roundtrip
proptests; it is required-status-check but listed under different
naming convention). Match its precedent for the new jobs:
required-status-check, run on every PR + push to main.

ADR-0038's claim "dst-lint scope unchanged" requires no new check —
both new crates are non-`core` (`overdrive-bpf` is `binary`,
`overdrive-dataplane` is `adapter-host`); the existing `dst-lint`
job's `cargo xtask dst-lint` invocation continues to scan only
`crate_class = "core"` crates and the new crates are skipped
automatically.

---

## 5. Skip rationale (jobs NOT added)

Per the locked decisions and ADR-0038 §6:

- **`verifier-regress` (Tier 4 — veristat):** stays stubbed. Baselining
  verifier complexity against a no-op program produces meaningless
  numbers and a gate that catches nothing. Wiring deferred to #29
  alongside the kernel matrix expansion.
- **`xdp-perf` (Tier 4 — xdp-bench):** stays stubbed. Same reason —
  no-op XDP at line rate measures the runner's veth throughput, not
  Overdrive's dataplane.
- **PREVAIL second-opinion analysis:** nightly soft-fail per
  testing.md §Tier 4 — adds no per-PR signal, deferred with the
  rest of Tier 4 to #29.
- **Kernel matrix expansion:** scope of #29 (locked).
- **macOS CI runners:** macOS dev coverage is `cargo check
  --workspace --no-run` per ADR-0038 §5.3 — handled by the existing
  `fmt-clippy` and `test` jobs as soon as they continue to pass on
  the new code paths (which they will, because `default-members`
  excludes `overdrive-bpf` and `overdrive-dataplane` compiles via
  `#[cfg(target_os = "linux")]` stubs). No new macOS CI job needed.

---

## 6. Concrete patch shape

The PR that lands #23 makes one workflow file change
(`.github/workflows/ci.yml`):

1. Append three new top-level `jobs:` entries — `bpf-build`,
   `bpf-unit`, `integration-test-vm-latest` — after the existing
   `mutants-diff` entry.
2. Update the existing leading comment block (`.github/workflows/ci.yml`
   lines 3–6, the "Tiers 2 (BPF unit), 3 (integration-test vm kernel
   matrix), and 4 (veristat / xdp-perf / PREVAIL) are deliberately
   out of scope" prose) to reflect that Tier 2 and the single-kernel
   slice of Tier 3 are now in scope as of #23, with a pointer to
   ADR-0038. The full Tier 3 kernel matrix and Tier 4 (veristat,
   xdp-perf, PREVAIL) remain deferred to #29.
3. Update the "Required status checks" comment block (lines 8–17) to
   add the three new job names alongside the existing six. Operators
   must mirror the change in the GitHub branch-protection UI after
   the first observed pass — see `branching-strategy.md` §2 for the
   sequencing constraint.

The crafter implementing #23 mechanically writes these three jobs by
copying the structure of existing jobs (sccache prelude, mold install,
Swatinem cache, etc.) and substituting the per-job specifics from §2.
No bespoke action required.

---

## 7. References

- `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md`
  §3, §6 — build pipeline + Tier 2/3 harness specs.
- `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md`
  §3, §4, §6 — build pipeline contract, bpf-linker provisioning,
  xtask harness wiring.
- `.claude/rules/testing.md` §Tier 2, §Tier 3, §Harness, §"Running
  tests on macOS — Lima VM".
- `.github/workflows/ci.yml` — existing six-job baseline this
  document extends.
- Issue #29 — kernel matrix expansion + Tier 4 wiring (out of scope
  for #23).
