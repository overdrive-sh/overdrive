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

## A3 — LVH `images pull` requires Docker; Lima dev VM lacks Docker

**Discovered by:** Step 03-02 crafter
**Affected files:**
- `docs/feature/phase-2-aya-rs-scaffolding/design/architecture.md` §6.2 (Tier 3 — LVH smoke harness)
- `docs/feature/phase-2-aya-rs-scaffolding/devops/ci-cd-pipeline.md` §2.3 (`integration-test-vm-latest` step graph)
- `infra/lima/overdrive-dev.yaml` (provisioning: lvh installed, docker NOT installed)

**Issue:** The Tier 3 LVH harness is documented as `lvh kernels pull <tag>` + `lvh run --kernel <path> --image <path>`. The kernel pull works on Lima (already verified end-to-end — `lvh kernels pull 6.12-main` produces `target/xtask/lvh-cache/6.12-main/boot/vmlinuz-6.12.83`). The `lvh run` step requires a rootfs `--image <path>`; LVH only provides `lvh images pull <URL>` to fetch one, and `lvh images pull` invokes `docker` to talk to an OCI registry. The Lima dev VM (`infra/lima/overdrive-dev.yaml`) does not provision Docker, so `lvh images pull` fails with "Cannot connect to the Docker daemon". CI on `ubuntu-latest` HAS Docker preinstalled, so the same `cargo xtask integration-test vm latest` invocation succeeds in CI.

The architecture and devops docs do not surface this dependency. Three reasonable resolutions, none blocking step 03-02:

1. **Provision Docker in Lima** (`infra/lima/overdrive-dev.yaml`) so the Lima dev path matches CI. Cost: extra ~200 MB image weight; conflicts with the existing user feedback "Don't tell the user to install tools" on their machine, but the Lima VM is platform-managed.
2. **Pre-bake a rootfs image into the Lima VM** at provisioning time, sidestepping `lvh images pull` entirely. Cost: bigger Lima image; tighter coupling to a specific LVH base-image version.
3. **Migrate the harness to direct QEMU + initramfs** (the pattern aya itself uses, per `aya-rs/aya/xtask/src/run.rs`). Aya does not actually use LVH — it builds an initramfs with `gen_init_cpio` and invokes `qemu-system-<arch>` directly with `-kernel <vmlinuz> -initrd <cpio>`. Cost: larger implementation surface (~500 LOC); benefits: zero docker dependency, faster boot, exactly the harness shape aya documents.

**Resolution chosen (2026-05-04, after step-back):** Drop step 03-02 from #23 entirely; defer to **#152** ([2.7-followup] Defer nested-VM kernel matrix harness — single-kernel Tier 3/Tier 4 lands in-host; matrix expansion ships separately).

The deeper issue surfaced once #29's "single-kernel only" deferral was reread: for a no-op `xdp_pass` program, real `xdp::attach(iface)` adds zero coverage over Tier 2's `BPF_PROG_TEST_RUN` from step 03-01. The attach path is aya's code, the verifier path is exercised at load-time by Tier 2, the counter increment is identical between PROG_TEST_RUN and real-attach. The first slice where Tier 3 attach earns its keep is one with actual XDP branching logic (#24 POLICY_MAP onward), not no-op.

The nested VM (LVH or QEMU+initramfs) ONLY earns its keep when the kernel must differ from the host environment — the kernel matrix [5.10, 5.15, 6.1, 6.6, latest LTS, bpf-next], which was already deferred to a follow-up. So step 03-02 was carrying nested-VM machinery whose only justification was matrix expansion, against a program that doesn't need real-attach coverage at all.

**Resolution applied in DELIVER (2026-05-04):**
- Step 03-02's partial implementation deleted: `xtask/src/integration_vm.rs`, `xtask/test-bin/integration_vm_latest/main.rs`, `xtask/tests/integration/integration_vm_latest_smoke.rs`, plus the LVH-related Cargo.toml deps (overdrive-dataplane optional dep, tokio optional dep, default-run, second `[[bin]]` entry).
- `xtask::integration_vm` task handler reverted to a `tracing_placeholder` shape pointing at #152 with reasoning baked into the comment.
- Roadmap step 03-02 to be removed; step 03-03 contracted to TWO GHA jobs (bpf-build + bpf-unit) instead of three; step 03-04 comment-block to reflect two new required checks instead of three. Architect dispatch queued.

**Recommended upstream amendment (architect, post-DELIVER):**
1. Amend `architecture.md` §6.2 to drop the LVH framing and reference #152 for the nested-VM harness when it lands.
2. Amend `ADR-0038` §6 similarly.
3. Amend `devops/ci-cd-pipeline.md` §2.3 — the `integration-test-vm-latest` GHA job is removed from the per-PR critical path; the section can be retained as documentation for what #152 will land.
