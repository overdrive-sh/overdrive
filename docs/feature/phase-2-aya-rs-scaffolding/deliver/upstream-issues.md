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
