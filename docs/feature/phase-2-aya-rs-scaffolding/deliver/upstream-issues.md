# Upstream Issues — DELIVER wave findings

Issues discovered during DELIVER that need amendment in upstream wave artifacts.

## A1 — architecture.md §4 / ADR-0038 §4 toolchain provisioning incomplete

**Discovered by:** Step 02-01 / 02-02 crafters
**Affected files:**
- `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md` §4 (Toolchain provisioning — bpf-linker)
- `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md` §4

**Issue:** The architecture wave enumerated `bpf-linker` as the only platform-managed toolchain dependency for `cargo xtask bpf-build`. In practice, the bpf-build invocation uses `cargo +nightly build ... -Z build-std=core` (architecture.md §3.1), which requires:

1. The `nightly` rustup toolchain to be installed.
2. The `rust-src` component to be installed on the `nightly` toolchain (NOT just the default toolchain).

The current architecture text mentions `-Z build-std=core` in §3.1 but does not surface "nightly + rust-src on nightly" as a dependency in §4 alongside bpf-linker.

**Resolution applied in 02-02:** Lima YAML extended with `rustup toolchain install nightly --component rust-src --profile minimal || true` (alongside the original bpf-linker append).
**Resolution applied in 02-03 (planned):** dev-setup xtask handler does the same install for non-Lima Linux developers.

**Recommended upstream amendment (architect, post-DELIVER):** Update architecture.md §4 and ADR-0038 §4 to enumerate three toolchain dependencies: `bpf-linker`, `rustup toolchain nightly`, `rust-src component on nightly`. The Lima YAML, dev-setup xtask, and `cargo xtask bpf-build` runtime checks all consume this list.

## A2 — devops/ci-cd-pipeline.md §2.2 Tier 2 capability claim is wrong on Ubuntu 22.04+

**Discovered by:** Step 03-01 dispatch review (orchestrator pre-flight)
**Affected files:**
- `docs/feature/phase-2-aya-rs-scaffolding/devops/ci-cd-pipeline.md` §2.2 lines 138-143
- `docs/feature/phase-2-aya-rs-scaffolding/deliver/roadmap.json` — phase 03 step 03-01 acceptance criteria bullet 4
- (consequently) `architecture.md` §6.1 if it carries the same claim

**Issue:** The devops doc claims "None at the runner level — `BPF_PROG_TEST_RUN` does not require CAP_BPF; aya's `Program::test_run` runs as the runner user. This is the explicit Tier-2 boundary." This is the source of roadmap step 03-01's AC bullet 4 ("test runs without CAP_BPF").

The claim is empirically false on locked-down kernels:

1. **`kernel.unprivileged_bpf_disabled` defaults.** Ubuntu 22.04+, Debian 12+, RHEL 9+ all ship with `kernel.unprivileged_bpf_disabled = 2` (fully disabled). The Lima image (`infra/lima/overdrive-dev.yaml`) is Ubuntu 24.04 → `unprivileged_bpf_disabled = 2`.
2. **aya's `Program::test_run` calls `bpf(BPF_PROG_LOAD, …)` before the test_run syscall.** Even if `BPF_PROG_TEST_RUN` itself were unprivileged-friendly on a stock kernel, `BPF_PROG_LOAD` requires `CAP_BPF` (or `CAP_SYS_ADMIN` on pre-5.8 kernels) when `kernel.unprivileged_bpf_disabled != 0`, which is the default. The load fails with `EPERM` before `test_run` is ever reached.
3. **The Tier-2 / Tier-3 capability split is not where the doc draws it.** The reality:
   - **Tier 2 (`cargo xtask bpf-unit`):** Needs `CAP_BPF` to load programs into the kernel for `BPF_PROG_TEST_RUN`. Not just attach.
   - **Tier 3 (`cargo xtask integration-test vm`):** Needs `CAP_BPF + CAP_NET_ADMIN` (load + xdp::attach to an interface).

**Resolution applied in 03-01:** The `cargo xtask bpf-unit` invocation uses the canonical `cargo xtask lima run -- ...` wrapper which `sudo`s by default per `.claude/rules/testing.md` § "Cgroup writes need root or delegation" (the Lima wrapper already implements this for the workload-cgroup tests; same shape applies). CI on `ubuntu-latest` runs the GHA job as the runner user; the GHA job step gets `sudo` prefix on the `cargo xtask bpf-unit` invocation OR sets ambient CAP_BPF on the runner user (cleaner: prefix sudo, since the runner is ephemeral).

**Recommended upstream amendment (architect, post-DELIVER):**
1. Rewrite `ci-cd-pipeline.md` §2.2 "Capability requirements" to: "`cargo xtask bpf-unit` requires CAP_BPF (kernel.unprivileged_bpf_disabled = 2 default on Ubuntu 22.04+). The GHA job runs the step under `sudo`. Locally, `cargo xtask lima run -- cargo xtask bpf-unit` sudos by default. Tier 3 additionally requires CAP_NET_ADMIN."
2. Amend roadmap step 03-01 AC bullet 4 to drop the "without CAP_BPF" claim; replace with "test runs under `sudo` via the canonical Lima wrapper / GHA `sudo` prefix."
3. If `architecture.md` §6.1 echoes the unprivileged claim, amend it the same way.
