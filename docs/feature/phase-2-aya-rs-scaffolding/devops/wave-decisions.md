# DEVOPS Decisions — phase-2-aya-rs-scaffolding

**Issue:** GH #23 — `[2.1] aya-rs eBPF scaffolding + build pipeline`
**Wave:** DEVOPS
**Architect:** Apex (Platform & Delivery Architect)
**Date:** 2026-05-04
**Mode:** Subagent — orchestrator-locked decisions, autonomous artifact production
**ADRs produced:** none (no infrastructure decision is large enough to warrant a new ADR; ADR-0038 from DESIGN already covers the build-pipeline contract)
**User ratification:** all 9 standard DEVOPS decisions locked by the orchestrator brief (2026-05-04); no user dialogue invoked

---

## Key Decisions

- **[D1] Kernel matrix scope-down for #23.** The PR-time CI job for Tier 3 runs SINGLE kernel only — `cargo xtask integration-test vm latest` against `ubuntu-latest` with `--qemu-disable-kvm`. The full matrix `[5.10, 5.15, 6.1, 6.6, latest LTS] + bpf-next` documented in `.claude/rules/testing.md` §"Kernel matrix" is the explicit scope of issue #29 (the dedicated real-kernel integration-test harness bootstrap). Rationale: testing.md's per-PR critical-path budget is ~15 min; running the full matrix against a no-op program produces 5× the wall-clock for ≤1× the signal. The single-kernel job in #23 is the harness foundation #29 expands by wrapping it in a `strategy: matrix:` block with no other refactor. (see `ci-cd-pipeline.md` §2.3, §5)
- **[D2] bpf-linker provisioning path: Lima `cargo install --locked` line + CI direct `cargo install --locked` step + `cargo xtask dev-setup` for non-Lima Linux + `which_or_hint` at the top of `cargo xtask bpf-build`.** Three surfaces, one install primitive (`cargo install --locked bpf-linker`), one diagnostic. The CI job uses the install command directly rather than calling `cargo xtask dev-setup` because keeping CI's install path explicit (one shell line) is easier to debug than a nested xtask invocation, and the xtask `dev-setup` subcommand is itself a thin wrapper around `cargo install --locked bpf-linker`. ADR-0038 §4 explicitly permits this trade-off ("CI inherits the Lima image's tooling automatically [...] No additional CI-only install path"). Per `feedback_no_user_install_instructions` user memory: the user is never told to install the tool manually. (see `infrastructure-integration.md` §5; `ci-cd-pipeline.md` §2.1)
- **[D3] Three new required-status-checks added to `main` branch protection.** `bpf-build`, `bpf-unit`, `integration-test-vm-latest` join the existing six (`fmt-clippy`, `test`, `dst`, `dst-lint`, `yaml-free-cli`, `mutants-diff`). Operator action required: GitHub does not allow marking a status check as required until it has been observed at least once on a PR or push event, so the sequencing is (1) land #23 with the new jobs, (2) verify they pass on the merged push-to-main, (3) add the three checks to branch protection. dst-lint requires no new check — both new crates declare non-`core` `crate_class` and dst-lint skips them automatically per ADR-0038 §8. (see `branching-strategy.md` §2; `ci-cd-pipeline.md` §4)
- **[D4] Lima image change is one line.** `infra/lima/overdrive-dev.yaml` line 205: append `bpf-linker` to the existing `cargo install --locked cargo-deny cargo-nextest cargo-mutants` line. No system-mode apt edits — the existing system-mode toolchain (clang/lld/llvm/libelf-dev/libbpf-dev/linux-libc-dev/bpftool/xdp-tools/qemu/kvm/lvh/virtme-ng) covers everything else aya needs. (see `infrastructure-integration.md` §2, §3)
- **[D5] CI/Lima parity is a structural property.** Both surfaces use `cargo install --locked bpf-linker` as the sole install primitive. The xtask `which_or_hint` at the top of `cargo xtask bpf-build` is the single diagnostic surface — a missing tool produces the same one-line actionable error on every developer surface. (see `infrastructure-integration.md` §5)
- **[D6] BPF object artifact is uploaded by `bpf-build` and consumed by `bpf-unit` + `integration-test-vm-latest` via `actions/download-artifact@v4`.** Each downstream job declares `needs: bpf-build` and materialises the ELF at `target/xtask/bpf-objects/overdrive_bpf.o` — the stable path mandated by ADR-0038 §3 that the loader's `include_bytes!` references. This avoids re-running `cargo xtask bpf-build` in each test job (saves ~2 min per job after the first). (see `ci-cd-pipeline.md` §2.1, §2.2, §2.3)
- **[D7] Tier 4 jobs (`verifier-regress`, `xdp-perf`, PREVAIL) explicitly NOT added.** ADR-0038 §6 keeps these as `tracing_placeholder` stubs with `// TODO(#29)` markers. Wiring them against a no-op program produces meaningless baselines and a gate that catches nothing. Deferred to #29 alongside the kernel matrix expansion. (see `ci-cd-pipeline.md` §5)

---

## Infrastructure Summary

- **Pattern:** CI extension; no platform deployment. Overdrive IS the platform-as-a-product per project SSOT (whitepaper). #23 ships workspace crates that downstream Phase-2 issues (#24–#37) extend; the DEVOPS surface is GitHub Actions + the Lima dev VM image, nothing else.
- **Deployable artifacts:** none. The new `overdrive-bpf` crate produces an ELF object consumed via `include_bytes!` by `overdrive-dataplane`; the loader is a Rust library shipped as part of the `overdrive` workspace. There is no container, no cloud resource, no installer, no VM image.
- **Runtime surfaces (post-#23):**
  - `macos-dev` — compile-only via `cargo check --workspace --no-run`; `overdrive-bpf` excluded from `default-members`; `overdrive-dataplane` compiles via `#[cfg(target_os = "linux")]` stub bodies.
  - `linux-lima-dev` — full build + load + attach via `cargo xtask lima run --`; canonical inner-loop path on macOS hosts.
  - `linux-native-dev` — full build + load + attach without Lima; provisioning via `cargo xtask dev-setup`.
  - `github-actions-ci` — three new jobs on `ubuntu-latest`; LVH boots a VM with `--qemu-disable-kvm` for Tier 3.
- **CI jobs added:**
  - `bpf-build` (≤8 min) — produces `target/xtask/bpf-objects/overdrive_bpf.o`.
  - `bpf-unit` (≤5 min) — Tier 2 `BPF_PROG_TEST_RUN` against the no-op program.
  - `integration-test-vm-latest` (≤10 min) — Tier 3 LVH smoke; load → attach → counter > 0 → detach.
- **Lima image change:** one line — append `bpf-linker` to the `cargo install --locked` line at `infra/lima/overdrive-dev.yaml:205`.
- **Branch protection delta:** three required-status-checks added to `main` (operator action after #23 merges).

---

## Constraints Established

- **PR critical path ≤ 15 min** — the three new jobs run in parallel with the existing seven; the longest single job remains under the budget. `integration-test-vm-latest` (≤10 min) is the largest single contributor.
- **Single-kernel CI for #23** — full matrix is scope of #29.
- **CAP_BPF + CAP_NET_ADMIN required at runtime on Linux** — handled transparently by `cargo xtask lima run --` (runs as root inside Lima) and the LVH harness (boots VM with the test process as root). Production-deployment capability declaration is flagged for Phase 7+ in ADR-0038 §9.
- **macOS dev preserved** — `default-members` excludes `overdrive-bpf`; `overdrive-dataplane` compiles via `#[cfg(target_os = "linux")]` stub bodies. The existing `--no-run` macOS quality gate continues to be authoritative.
- **No new metrics/dashboards/alerting** — CI-level signal is GitHub Actions job duration + pass/fail + JUnit reports + failure-artifact uploads (matching the existing `dst` and `mutants-diff` patterns). Application-level eBPF telemetry is the scope of #31.
- **No platform-as-a-product deployment** — Overdrive is the platform; #23 ships workspace crates, not infrastructure operating something else.

---

## Skipped Standard Deliverables (rationale per skill)

The DEVOPS skill enumerates several standard deliverables. Several do not apply to #23 and are skipped per the orchestrator brief:

- **`platform-architecture.md`** — N/A. The platform-as-a-product architecture lives in the whitepaper SSOT; #23 changes nothing at that level. The DESIGN wave's `architecture.md` and ADR-0038 are the authoritative architecture artifacts for this issue.
- **`observability-design.md` / `monitoring-alerting.md`** — N/A. The eBPF telemetry pipeline ships in #31 (whitepaper §12). #23 has no application-level observability surface. CI-level observability is "existing GH Actions logs + cargo nextest JUnit + bpftool dump on test failure" — described inline in `ci-cd-pipeline.md` §3.
- **`continuous-learning.md`** — N/A. No production deployment, no operational learning loop in scope.
- **`kpi-instrumentation.md`** — N/A. No `outcome-kpis.md` was produced (no DISCUSS wave was run for #23 — foundational infra). The skill explicitly says "do not block on it" when the KPI artifact is absent.
- **`security-design.md` (as a separate doc)** — folded into ADR-0038 §5 (capability requirements, identity boundaries) and `environments.yaml` (CAP_BPF/CAP_NET_ADMIN at runtime). No standalone security-design doc warranted for a CI extension that ships no production posture changes.

---

## Upstream Changes

- (no upstream changes — DESIGN-wave artifacts are consistent with what #23's DEVOPS surface needs; no decision in this wave reshapes a prior wave)

See `upstream-changes.md` for the placeholder.

---

## References

- `docs/feature/phase-2-aya-rs-scaffolding/devops/ci-cd-pipeline.md` — three CI jobs in detail.
- `docs/feature/phase-2-aya-rs-scaffolding/devops/infrastructure-integration.md` — Lima image change + toolchain audit.
- `docs/feature/phase-2-aya-rs-scaffolding/devops/branching-strategy.md` — branch-protection delta.
- `docs/feature/phase-2-aya-rs-scaffolding/devops/environments.yaml` — environment inventory.
- `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md` §3, §4, §6 — DESIGN-wave authoritative spec.
- `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md` — DESIGN-wave ADR; CI work implements its build-pipeline contract.
- `.claude/rules/testing.md` — Tier 2 / Tier 3 / Workspace convention / Running tests on macOS.
- `.github/workflows/ci.yml` — existing six-job baseline.
- `infra/lima/overdrive-dev.yaml` — Lima image config.
- Issue #29 — kernel-matrix expansion + Tier 4 wiring (out of scope for #23).
- `feedback_no_user_install_instructions` user memory.
