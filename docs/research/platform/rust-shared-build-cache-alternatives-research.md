# Research: Sharing Rust compilation output across many concurrent build environments (as of 2026-11-17)

**Date**: 2026-11-17 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium (strong on status and maintenance facts, weak on measured performance, which is mostly vendor-reported) | **Sources**: 60 cited URLs

## Executive Summary

The goal is that when one microVM builds a cargo workspace at some input state, every other VM with identical inputs reuses the result, including workspace crates, linked test binaries, build-script outputs and ideally test results, while each VM keeps a fast incremental edit loop. As of 2026-11-17 no single option delivers all of that cheaply.

**Evidence summary.**
- **Cargo-native sharing is not available.** The per-user artifact cache (rust-lang/cargo#5931) is still `S-needs-design`. The 2026 "cross workspace cache" goal targets a first nightly that excludes build scripts and proc macros. A remote cache is only a "future possibility". [1][2][3]
- **sccache is maintained but its limits are unchanged.** v0.19.0 (2026-10-24) did not touch them. Incremental crates, anything that invokes the linker (bin, proc-macro, dylib) and test binaries remain uncached. [4][5]
- **Bazel + rules_rust and Buck2 are the only options with hermetic, content-addressed caching of everything, including test results.** Both need the build graph rewritten outside cargo. Neither caches incrementally compiled output. rules_rust incremental support is an open third-party PR, and the Buck2 prelude deliberately never uploads incremental actions to the shared cache. [6][7][8]
- **Nix per-crate builds are the least mature.** crate2nix is maintained (527 stars, pushed 2026-11-11, 152 open issues). cargo2nix has been idle since 2025-06-19. The newest approaches (cargo-dyndrv, cargo-schnee, cargo-nix-plugin) depend on experimental Nix features that upstream says will be redesigned before stabilisation. [9][10][11][12][13]
- **One new option appeared in 2026 that was not on the original candidate list.** `kache` (created 2026-02-15, v2.1.1 on 2026-11-12) is a cargo `RUSTC_WRAPPER`-style cache that claims to cache Rust executables, build-script runs and (via private state) incremental edits, with path normalisation and S3/GCS/filesystem/OCI remotes. Its performance claims are vendor-reported, and the benchmark page contains no numbers. [14][15][16][17]

**Evidence-based ranking for this setup (reasoning is in "Recommendation").**
1. **kache, as a measured pilot.** Lowest migration cost: cargo, nextest, cargo-mutants and the Metal runner grammar stay intact.
2. **Bazel + rules_rust,** only if hermetic test-result caching and CI-grade reproducibility are worth a multi-month migration. The workspace's process-per-test, node-global-kernel-state test model is the biggest semantic obstacle.
3. **Nix crane / crate2nix,** a good fit for CI artifacts but weak for the VM edit loop.
4. **Buck2,** which has the same costs as Bazel with less ecosystem and a pre-1.0 Cargo bridge.
5. **sccache,** as the mature baseline and fallback, not a way to beat its own limits.

Cargo-native caching is not usable yet. Re-check it in 2027.

**Migration headline (estimates, see §"Migration cost").** A kache pilot is roughly 1 week of integration plus measurement with no source changes. A Bazel build for the default test lane is roughly 10–18 engineer-weeks, with the Tier-3 lanes staying on cargo. Buck2 is roughly 40% more than that. A Nix CI path is roughly 3–6 engineer-weeks.

**Biggest unverified items.** Real cross-VM hit rates on this workspace, kache correctness under `build.rs` absolute paths, and whether any non-cargo build can model the BPF pipeline natively (`bpfel-unknown-none` with `-Z build-std`). A measurement plan is at the end.

## Research Methodology

**Search Strategy**: Web search plus direct fetches of primary sources: project repositories via the GitHub REST API, official docs, release notes, rust-lang project-goal pages and maintainer posts. The local workspace was read, not built or run. Nothing was cloned and no third-party code was executed.

**Tooling limitation**: No shell was available in this dispatch, so the `gh` CLI could not be run. GitHub metadata (stars, push dates, issues, PRs, releases) was fetched through `https://api.github.com/...` with WebFetch, which returns the same data `gh api` would. Because `gh issue view --comments` was not available, long issue threads were read only through the first API page. This is noted where it limits a claim (cargo#5931 comments were visible only up to 2024).

**Source Selection**: Primary sources are preferred (repos, release notes, rust-lang goals and blog, official docs). Secondary sources (blogs, recaps) are labelled with their bias. Vendor and self-reported numbers are marked **[vendor]** or **[self-reported]**. Interpretation and estimates are marked **[analysis]**.

**Quality Standards**: Target 3 sources per major claim, 1 authoritative minimum. Where only one source exists the finding is rated Low or Medium.

## Local workspace facts (read-only; used for migration estimates)

| Fact | Evidence |
|---|---|
| Root `Cargo.toml` lists **19 workspace members** (16 under `crates/`, plus `tests/ovd-fwd-frame-parser`, `tests/conformance`, `xtask`), 17 `default-members` (excludes `overdrive-bpf` and `tests/conformance`). The task brief said 31; the 19 figure is what the file shows. | `/Users/marcus/conductor/workspaces/helios/antananarivo/Cargo.toml` |
| Toolchain pinned `channel = "1.95.0"` plus `rust-src`, musl targets. | `rust-toolchain.toml` |
| Profiles: `[profile.dev] debug = "line-tables-only"`, `[profile.dev.package."*"] opt-level = 2`, release `lto = "thin"`, `codegen-units = 1`. Resolver 3, edition 2024. | `Cargo.toml` |
| Linker is mold on x86_64/aarch64 Linux via `-C link-arg=-fuse-ld=mold`. Incremental is on by default; CI sets `CARGO_INCREMENTAL=0`. | `.cargo/config.toml` |
| **Three build scripts**: `overdrive-dataplane/build.rs`, `overdrive-init/build.rs`, `tests/ovd-fwd-frame-parser/build.rs` (uses the `cc` crate). | Glob `crates/*/build.rs`, `Cargo.toml` (`cc`), `tests/ovd-fwd-frame-parser/build.rs` |
| `overdrive-dataplane/build.rs` derives an **absolute** workspace root from `CARGO_MANIFEST_DIR`, emits `cargo:rustc-env=CARGO_WORKSPACE_DIR=<abs path>` and `OVERDRIVE_BPF_OBJECT_PATH=<abs path>`, reads `target/bpf/overdrive_bpf.o`, compares file mtimes against `crates/overdrive-bpf/src/**/*.rs`, and calls `process::exit(1)` if the object is missing or stale. It is overridable via `OVERDRIVE_BPF_OBJECT`. The object is consumed through `include_bytes!`. | `crates/overdrive-dataplane/build.rs` |
| `overdrive-init/build.rs` requires two objects, `target/bpf/overdrive_guest_bpf.o` and `target/bpf/overdrive_guest_tcp_correlation_candidate_bpf.o`, and emits their absolute paths via `rustc-env`. | `crates/overdrive-init/build.rs` |
| The BPF crate is `#![no_std]`, built only by `cargo xtask bpf-build` (`cargo +nightly build --target bpfel-unknown-none --features build-bpf-target -Z build-std=core`), with the bins gated by `required-features`. The two bins are `overdrive-bpf` and `overdrive-guest-bpf`. | `crates/overdrive-bpf/Cargo.toml` |
| **One workspace proc-macro crate**: `overdrive-testing-macros` (`#[overdrive_test]`, writes placement records into an ELF section read by `placement-lint` and the Metal wrapper). | Grep `proc-macro = true`; `.claude/rules/testing.md` § Test placement |
| `trybuild` (pinned `=1.0.101`) is a dev-dependency of at least `overdrive-core`, `overdrive-control-plane`, `overdrive-store-local` and `overdrive-testing`. Nextest config also names `overdrive-worker` compile_fail. Each fixture spawns `cargo build` internally. | `Cargo.toml`, per-crate `Cargo.toml`, `.config/nextest.toml` |
| `integration-tests` and `kvm-tests` are cargo features, unified workspace-wide. Every member declares `integration-tests = []`. | `.claude/rules/testing.md` § Integration vs unit gating |
| `.config/nextest.toml` is ~1,060 lines. It defines the `host-kernel-shared` (`max-threads = 1`) and `trybuild` test groups and dozens of per-test overrides. It enables `wrapper-scripts` for the Metal run wrapper and states that nextest runs **each test in its own process**, which `serial_test` cannot span. | `.config/nextest.toml` |
| `cargo-mutants` runs `cargo test --locked` per mutant on a copy of the tree in `/tmp`, sets `OVERDRIVE_BPF_OBJECT` for the dataplane build script, and excludes `**/build.rs`, `crates/overdrive-bpf/**`, `xtask/**` and others. | `.cargo/mutants.toml`, `crates/overdrive-dataplane/build.rs` |
| `xtask` shells out to `cargo` in at least 5 source files (21 files across the workspace mention `Command::new("cargo")`-style calls, including tests such as `dst_clean_clone_green.rs`, `build_rs_artifact_check.rs`, `openapi_gate.rs`, `vm_fixture.rs`). | Grep over `*.rs` |
| The Metal runner accepts a **closed** command grammar: `cargo build/check/clippy/fmt/test --doc/nextest run/xtask/run`, with a fixed `env` key allowlist (no `RUSTC_WRAPPER`, `SCCACHE_*` or `KACHE_*`). `--archive-file`, `--binaries-metadata` and `--target-dir-remap` are refused. | `.claude/rules/testing.md` § Running tests, native x86_64 |
| Hooks (`lefthook.yml`) run `cargo clippy -p`, `cargo test --doc -p`, `cargo nextest run -E 'rdeps(...)'`, `cargo xtask kmod-build`, `placement-lint`, `conformance-lint`, all routed through `infra/hooks/linux-run.sh`. | `lefthook.yml` |
| Third-party dependencies include `wasmtime 26`, `aws-lc-rs`, `ring`, `libsql` (core, C), `rustls`, `ktls`, `aya`, `tonic`/`prost`, `hickory-proto`, `rtnetlink`, `utoipa`, `rkyv`, `quint-connect`. | `Cargo.toml` `[workspace.dependencies]` |

**[analysis]** Three properties of this workspace shape every candidate:
1. The dev profile compiles all third-party crates at `opt-level = 2`, so cold dependency builds are expensive and cross-VM reuse of dependencies is the largest single saving.
2. The foundational crate `overdrive-core` has a very large reverse-dependency set (the lefthook comments say a touch to it expands `rdeps` to "most of the workspace"). Edits there invalidate almost everything downstream in any scheme.
3. The heaviest tests mutate node-global kernel state and are not hermetic. Test-result caching is therefore only meaningful for the default-lane (unit, acceptance, proptest) tests.

## Findings

### Finding 1: sccache is actively maintained, but its Rust limits are unchanged
**Evidence**: Release v0.19.0 (2026-10-24) changes: dep-info is written locally instead of restored from cache (#2894), `.rmeta` notifications stream to cargo early (#2875), Rust outputs are hashed concurrently, `--show-stats` breaks down non-cacheable Rust compilations by crate type. For "Incremental" the release lists no notable changes. v0.18.0 was 2026-09-14. [4]
The docs on `main` state: with `CARGO_INCREMENTAL=1` sccache errors unless `SCCACHE_ALLOW_INCREMENTAL=1`, which leaves workspace and path crates uncached. Crates that call the system linker (bin, dylib, cdylib, proc-macro) cannot be cached. Proc-macros that read files or `std::env::var` may be cached wrongly. `SCCACHE_BASEDIRS` strips base dirs, but crates embedding `CARGO_MANIFEST_DIR` can get a hit that reuses another checkout's path unless `SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS` is set. The page does not mention build scripts. [5]
**Source**: [sccache releases](https://api.github.com/repos/mozilla/sccache/releases?per_page=6), [sccache docs/Rust.md](https://raw.githubusercontent.com/mozilla/sccache/main/docs/Rust.md) - accessed 2026-11-17
**Confidence**: High (two primary sources)
**Distributed mode**: v0.19.0 fixes list scheduler/toolchain items (job-ID collisions across scheduler restarts #2895, toolchain repackaging after eviction #2893, chroot overlay builder #2908), indicating sccache-dist is still being repaired. [4] I did not find a third-party report of sccache-dist in production on a single node, so its value for the microVM setup is unverified.
**cachepot fork**: `paritytech/cachepot` is archived, last push 2023-02-28 (**unmaintained**). [18]
**Analysis**: sccache is a sound baseline for third-party dependency crates but cannot meet three of the four stated goals: workspace-crate incremental output, linked binaries/test binaries, and build scripts.

### Finding 2: Cargo-native sharing is not available in 2026 and will start with a narrow subset
**Evidence**:
- Issue rust-lang/cargo#5931 "Per-user compiled artifact cache" is open, updated 2026-10-06, labelled `A-caching` and `S-needs-design`. The MVP covers only immutable registry/git dependencies and idempotent builds, excluding build scripts and proc macros. The roadmap lists `build-dir` split (#14125) and the per-unit layout (#15010) as done and a coarse-locked shared cache, GC and granular locking as open. A remote cache plugin system is a "future possibility". [1]
- The 2026 project goal "Cargo cross workspace cache" is **Accepted**. The first nightly version supports "basic crates" and excludes build scripts and proc macros. A remote cache is only "could be extended in the future ... for CI use cases". Stabilisation is "likely not completed within the goal period". Funding is $30,000 part-time (partially funded), owner Ross Sullivan, champion Ed Page. [2]
- Build dir layout v2 was put out for testing on 2026-03-13 (nightly `-Zbuild-dir-new-layout`). Each unit lives in `build/<package>/<hash>/`. The post says the Cargo team "does not officially endorse sharing a build dir across workspaces". A separate `build-dir` setting has existed since Cargo 1.91. [3]
- Early comments on #5931 (through Aug 2024; later pages not read) record that relative-path hashing causes collisions in shared target dirs (#12516), and that the design deliberately excludes build scripts and proc macros until sandboxing exists. [1b]
- rustc's incremental system: a 2026 goal "Incremental Systems Rethought" (owner Alejandra González, partially funded) aims to overhaul it. I found no rust-lang/rust work making the incremental cache relocatable across checkouts. [19][20]
**Source**: [cargo#5931](https://api.github.com/repos/rust-lang/cargo/issues/5931), [cross-workspace cache goal](https://goals.rust-lang.org/2026/cargo-cross-workspace-cache.html), [build-dir v2 post](https://blog.rust-lang.org/2026/03/13/call-for-testing-build-dir-layout-v2), [cargo#5931 comments](https://api.github.com/repos/rust-lang/cargo/issues/5931/comments?per_page=100&page=1&direction=desc), [program management Jul-Aug 2026](https://blog.rust-lang.org/inside-rust/2026/08/31/program-management-2026-jul-aug/) - accessed 2026-11-17
**Confidence**: High for status; Low for any timeline.
**Analysis**: Treat Cargo-native sharing as a 2027+ watch item. Its first form would not cover workspace crates (path dependencies), linked binaries or build scripts, which are exactly the stated gaps.

### Finding 3 (NEW): kache is a cargo-wrapper cache that claims to close sccache's gaps; evidence is mostly vendor-reported
**Evidence**: Repository `kunobi-ninja/kache`: created 2026-02-15, last push 2026-11-16, 1,552 stars, 29 open issues, Apache-2.0, Rust; crates.io v2.1.1 dated 2026-11-12; commit authorship points to a Zondax address (company backing unconfirmed). [14][15][21]
Documented behaviour (vendor docs):
- **Executables and build scripts**: "Rust executables are cached by default on Linux and macOS"; build-script runs are cached on Linux and macOS. [15][16]
- **Path normalisation**: workspace, Cargo home, Rustup home, temp and target dirs map to stable sentinels (`/kache/workspace` etc.), so equivalent checkouts share entries. Exceptions use real paths and separate keys: coverage builds, **trybuild** builds, and some explicit `-L native=` dirs inside `OUT_DIR`. Crates that read `CARGO_MANIFEST_DIR` at compile time re-key per build directory. [17][22]
- **Key contents**: rustc identity/target, crate name, edition, cfg, features, codegen/lint flags, contents of `--extern` deps, **linker identity for link steps**, `-Z` flags. Workspace/path-crate records are used only when the workspace and `OUT_DIR` match the recording build. Proc macros that read untracked inputs need explicit `cache.key_env_vars`/`key_salt`/`workspace.extra_inputs`. [17]
- **Build-script inputs**: for workspace/path packages keyed on files and variables the script declares with `cargo:rerun-if-changed` / `rerun-if-env-changed`; with none declared, the package's non-Rust files count. [17]
- **Incremental**: not an artifact-cache hit. Edited units get a private incremental dir ("adaptive mode": skips the cache for up to eight builds or 15 minutes of idleness). "Donor seeds" let an edited crate import another checkout's finalised incremental session **with rustc 1.98 or later** (default on). [23][24]
- **Remotes**: S3-compatible, GCS, shared filesystem, OCI registry; local store first, remote read on miss and written in the background. [14][25]
- **Concurrency**: per-machine; after a local+remote miss a compile joins a machine-wide flight for its key, takes a memory-weighted permit and a per-key lock, so concurrent identical misses share one compile. The docs explicitly **do not address multiple VMs or shared-filesystem safety**. [26]
- **cargo-mutants**: documented; each mutant's build directory maps to a shared `<WORKSPACE>` sentinel, so unmutated crates (and their proc-macros and test binaries) hit across `--jobs N`; the mutated crate and everything downstream rebuild. [27]
Benchmarks: the benchmark page describes methodology (cold / warm_same_tree / warm / pull phases, randomised block design, 18-comparison Bonferroni family) but contains **no numeric results**, and states the default six-sample design often gives an "inconclusive" verdict. The README's headline claim is a Firefox 151 run on macOS/APFS where a second worktree added ~3 GB with kache versus 16.7 GB with sccache. This is a disk-use claim, **[vendor]**, on macOS, not a time claim. [14][28]
Open issues relevant to risk: #1719 (pull-request jobs could write objects `main` restores, a cache-trust-boundary concern), #1698 (`index.db: unable to open database file`, a locking failure), #1649 (truncated archive on an incremental build, Windows), #1944 (clippy units pass through with a named codegen backend). [29]
**Source**: [kache repo](https://api.github.com/repos/kunobi-ninja/kache), [docs home](https://ninja.kunobi.com/docs/kache), [cache key](https://ninja.kunobi.com/docs/kache/how-it-works/cache-key), [concurrency](https://ninja.kunobi.com/docs/kache/how-it-works/limits), [worktrees](https://ninja.kunobi.com/docs/kache/getting-started/worktrees), [mutation testing](https://ninja.kunobi.com/docs/kache/getting-started/mutation-testing), [benchmarks](https://ninja.kunobi.com/docs/kache/benchmarks), [issues](https://api.github.com/repos/kunobi-ninja/kache/issues?state=open&per_page=30), [crate page](https://docs.rs/kache) - accessed 2026-11-17
**Confidence**: Low-Medium. Behaviour is documented by one vendor with no independent reproduction found; maintenance is very active, which cuts both ways.
**Analysis [analysis]**: kache is the only option found that claims to cover all of: workspace crates, linked binaries, build scripts, path normalisation, a remote backend and cargo-mutants, without leaving cargo. It still does **not** cache test *results*. Note a repo-specific interaction: this workspace pins rustc **1.95.0**, so the "donor seed" incremental feature (1.98+) would be unavailable until the toolchain is bumped.

### Finding 4: cargo-reapi and other REAPI-backed cargo wrappers are early
**Evidence**: `cargo-reapi` (github.com/TamedTornado/cargo-reapi) v0.2.0 2026-10-04, v0.3.0 2026-10-15. Described as "Cargo-authoritative shared build caching with experimental REAPI execution". Cargo stays the planner and `RUSTC_WRAPPER` observes compiler actions. It supports verified linked outputs and whole-gate snapshots in the local cache. A reclient adapter can send eligible actions to a REAPI service, but only metadata-only compile actions are eligible, **link actions are not remotely eligible**, and the adapter was tested only against a fake `rewrapper`: "live REAPI service validation is outstanding". Tested filesystems: macOS APFS and Linux XFS. [30]
`objfs` (nnunley/objfs) is described in search results as an early-stage REAPI v2 build cache for Rust with no releases. `cargo-remote` (sgeisler) is an ssh/rsync remote-build wrapper, not REAPI, last released ~3 years ago (**stale**). [31][32]
**Source**: [cargo-reapi on docs.rs](https://docs.rs/crate/cargo-reapi/0.2.0), [objfs](https://github.com/nnunley/objfs), [cargo-remote listing](https://packages.ecosyste.ms/registries/crates.io/packages/cargo-remote)
**Confidence**: Medium (one primary source each; descriptions from docs).
**Analysis**: Not ready as a REAPI bridge. Revisit only if Bazel-style remote *execution* of cargo actions is wanted.

### Finding 5: Bazel + rules_rust is active and the only mainstream path to test-result caching, but incremental support is an open PR
**Evidence**:
- Releases: rules_rust 0.75.0 (2026-10-26), 0.74.0 (2026-08-28), 0.73.0 (2026-07-31). Notes mention crate_universe splicer following `.bazelignore` and a smaller `cargo-bazel-lock.json`. [6]
- **Incremental**: PR #3925 "Add incremental Rust compilation via Bazel-managed tree artifacts" (opened 2026-03-24, updated 2026-10-17, **open, 2 comments, no reviews**) adds `--@rules_rust//rust/settings:incremental=true`. Workspace crates only; external deps, proc macros and build scripts excluded; state is seeded by hardlink; test plan unchecked. Older persistent-worker approaches (2020–21) note rustc has no service mode and the sandbox's changing directory is the main obstacle. [7][33]
- **crate_universe**: `crate.from_cargo` ingests `Cargo.toml`/`Cargo.lock` manifests to generate third-party repositories with build scripts on by default; `crate.annotation` customises per-crate settings (`gen_build_script`, `build_script_env/data/deps/tools`, `rustc_flags`, `patches`...). `all_crate_deps` macros let Cargo.toml be the source of truth for workspace crate deps; the workspace crates' own BUILD files are written separately. [34]
- **Bazel test caching**: `--cache_test_results=auto` (default) reuses a test result unless the test or its dependencies changed, it is `external`, multiple runs were requested, or it failed. [35]
- **Remote cache correctness**: only `--action_env` variables enter an action; "Bazel currently does not track tools outside a workspace", so differing `/usr/bin` compilers can give false hits. The HTTP/gRPC protocol is a CAS plus an action cache. [36]
- **rust-analyzer**: `bazel run @rules_rust//tools/rust_analyzer:setup`; the whole project is one workspace unless `--per-package-workspaces`, with the documented drawback that dependents are not indexed. `--clippy` runs clippy on save. [37]
- **Cross/no_std**: I found no rules_rust example combining a custom `no_std` target with `-Z build-std`; one third-party repo (colatkinson/bazel-rust-cross) builds a custom-target toolchain extracted from `cargo build -Z build-std`. [38]
**Source**: [rules_rust releases](https://api.github.com/repos/bazelbuild/rules_rust/releases?per_page=5), [PR #3925](https://api.github.com/repos/bazelbuild/rules_rust/pulls/3925), [crate_universe docs](https://bazelbuild.github.io/rules_rust/crate_universe_bzlmod.html), [Bazel user manual](https://bazel.build/docs/user-manual), [Bazel remote caching](https://bazel.build/remote/caching), [rust-analyzer docs](https://bazelbuild.github.io/rules_rust/rust_analyzer.html), [bazel-rust-cross](https://github-redirect.dependabot.com/colatkinson/bazel-rust-cross)
**Confidence**: High for the facts above; Low for how well they compose for a bpfel `build-std` crate (no evidence found).

### Finding 6: Real-world Bazel/Rust migrations report qualitative wins and a recurring dual-build pattern
**Evidence**:
- **DFINITY (Internet Computer)**: ~600k lines of Rust (March 2023); Bazel became the primary build in 2022; migration took "several months" and "a few hundred Rust packages". They wrapped cargo as a black-box rule first, automated `Cargo.toml` to `BUILD` conversion (about 90% of output), built a progress visualiser and a cargo-vs-Bazel test-parity checker, and **kept Cargo files so IntelliJ Rust works**, with CI only requiring `cargo check --tests --benches`. rules_rust's rust-analyzer support "worked perfectly in the prototype but choked on our code base" (2023). No benchmark numbers; "a definitive success" is the author's qualitative view. They tried sccache and "saw no improvement" on CI servers. Listed gaps: no `cargo check` equivalent, no `cargo publish`, lost cargo ecosystem tools. [39]
- **Materialize**: epic "Build Materialize with Bazel" (#26796): CI builds from scratch took 25-30 minutes; also covers C-library annotations for crates and `cxx`; one nightly run showed Bazel returning cached test results. A `cargo_gazelle` crate exists in the Materialize tree (generator of Bazel targets from Cargo metadata), but I could not open its source or README. [40][41]
- **Google Quantum AI**: BazelCon 2025 recap says an 80% CI-time reduction after migration (one engineer); no methodology and no Rust-specific detail. **[self-reported]** [42]
- **Others**: a 2022 guided experiment found the tooling assumed a specific directory layout (the main pain point; no outcome stated). `gazelle_rust` (Calsign) is not archived, 53 stars, pushed 2026-08-24. A RustConf 2026 talk "2 Hours to 10 Minutes: Building Rust Monorepos at Scale with Bazel RBE" exists; the page returned HTTP 403, so its numbers are **unverified**. [43][44][45]
**Source**: [mmapped.blog](https://mmapped.blog/posts/17-scaling-rust-builds-with-bazel), [Materialize #26796](https://github-redirect.dependabot.com/MaterializeInc/materialize/issues/26796), [cargo_gazelle docs](https://dev.materialize.com/api/rust/src/cargo_gazelle/targets.rs.html), [BazelCon 2025 recap](https://jmmv.dev/2025/11/bazelcon-2025-recap.html), [prestontw](https://prestontw.github.io/blog/a-guided-bazel-migration-in-rust), [gazelle_rust](https://api.github.com/repos/Calsign/gazelle_rust), [RustConf 2026 listing](https://rustconf2026.sched.com/event/2KHtn/2-hours-to-10-minutes-building-rust-monorepos-at-scale-with-bazel-rbe)
**Confidence**: Medium. Three independent adopters, but few measurements and none with methodology.

### Finding 7: Self-hostable REAPI backends: bazel-remote is the simplest; NativeLink's licence matters
**Evidence**: `buchgr/bazel-remote`: Apache-2.0, 792 stars, pushed 2026-11-11, 89 open issues; supports HTTP and gRPC. `TraceMachina/nativelink`: 1,646 stars, pushed 2026-11-16, 197 open issues; licence is **FSL-1.1-Apache-2.0** (competing-use ban, Apache-2.0 after two years; internal production use is permitted). Bazel's community list names Buildbarn and BuildBuddy among self-service options; BuildBuddy OSS is described as open-core. A 2026 comparison guide positions bazel-remote as the lightweight option. [46][47][48][49]
**Source**: [bazel-remote](https://api.github.com/repos/buchgr/bazel-remote), [NativeLink](https://api.github.com/repos/TraceMachina/nativelink), [NativeLink LICENSE](https://raw.githubusercontent.com/TraceMachina/nativelink/main/LICENSE), [remote execution services](https://bazel.build/remote-execution-services.html), [2026 self-hosted guide](https://www.pistack.xyz/posts/2026-05-03-self-hosted-remote-build-cache-buildbuddy-bazel-remote-nativelink-guide/)
**Confidence**: High for repo and licence facts; Low for relative performance (no single-node head-to-head found).
**Analysis [analysis]**: For one physical node, bazel-remote (cache only) is enough. Remote *execution* is unnecessary when every VM builds locally. Content-addressed storage makes concurrent writers safe: identical digest means identical bytes.

### Finding 8: Buck2 is active, with a Rust prelude that does not share incremental output; third-party crates need Reindeer vendoring and fixups
**Evidence**:
- `facebook/buck2`: 4,532 stars, pushed 2026-11-16, 428 open issues, Apache-2.0. Releases are date-tagged (latest tag 2026-11-15). [8][50]
- The Rust prelude's `build.bzl`: incremental rustc actions run locally (`local_only = incremental_enabled`), keep outputs, and carry the comment that incremental compilation "should not publish any action output to a shared cache". Pipelined builds use the unstable `-Zno-codegen` flag (nightly). Proc-macro crates get a host-agnostic hash only when not incremental. [8]
- `facebookincubator/reindeer`: MIT, 279 stars, pushed 2026-11-16. Any crate with a `build.rs` needs a `fixups.toml` with `buildscript.run` set; vendoring is "strongly recommended"; crates needing no-std/cross feature resolution must set `execution-platform` on every platform; native libraries need explicit Buck targets. [51][52]
- `cargo-buckal` (buck2hub): v0.1.0 (2026-01-31) to v0.1.4 (2026-09-04), pre-1.0; `migrate` converts a Cargo dependency graph into `BUCK` files. Supported hosts include x86_64 Linux. [53]
- Buck2 REAPI: documented servers are EngFlow, Buildbarn and BuildBuddy; NativeLink advertises Buck2 compatibility. The docs page does not cover cache-only mode, caveats or test-result caching. [54][47]
- Maturity outside Meta: a 2023 BuildBuddy engineer's review (vendor with a Bazel product; corrected by the Bazel team after publication) concluded "not yet" for enterprise, flagged Reindeer vendoring as awkward for open-source git repos, and said there was no test caching by default (2023, possibly stale). Meta's own page calls Buck2 "early-stage" for open-source users and claims 2x speed over Buck1 **[vendor, no methodology]**. [55][56]
- Adopters named in search results (k23, crates-pro, System Initiative's `si` repo) are undated community projects; I found no 2026 adopter report with numbers. [57]
**Source**: [buck2](https://api.github.com/repos/facebook/buck2), [prelude/rust/build.bzl](https://raw.githubusercontent.com/facebook/buck2/main/prelude/rust/build.bzl), [reindeer manual](https://raw.githubusercontent.com/facebookincubator/reindeer/main/docs/MANUAL.md), [cargo-buckal](https://docs.rs/crate/cargo-buckal/latest), [Buck2 RE docs](https://buck2.build/docs/users/remote_execution/), [BuildBuddy Buck2 review](https://www.buildbuddy.io/blog/buck2-review), [Buck2 why](https://buck2.build/docs/about/why/)
**Confidence**: Medium-High on mechanics (code read), Low on current outside-Meta maturity (no recent independent report).

### Finding 9: Nix per-crate options: crate2nix is maintained; cargo2nix is stale; the newest approaches rely on experimental Nix
**Evidence**:
- **crane** v0.24.0 (2026-08-21). `buildDepsOnly` builds dependencies from a dummy source, so workspace source edits do not invalidate it; the application builds in a downstream derivation inheriting `cargoArtifacts`. It sets `CARGO_BUILD_INCREMENTAL=false` unless already defined. `installCargoArtifactsMode` is `use-zstd` (default) or `use-symlink`. Workspace crates are not separately cached. [58][59]
- **crate2nix**: 527 stars, pushed 2026-11-11, 152 open issues; "builds your cargo-based rust project crate-by-crate with nix". **cargo2nix**: 470 stars, last push **2025-06-19 (about 17 months, stale)**, 79 open issues. [11][12]
- **Benchmarks (`dpc/crane-eval`, single developer machine, one Rust app "SelfCI")**: warm-store medians of 5 samples: cargo-nix-plugin 5.29 s (clean) / 4.91 s (source edit) / 5.40 s (dep change); crane 7.4–7.7 s; crate2nix 7.1 / 7.2 / 9.52 s (graph regeneration ~2 s). Project-cold: crane `use-zstd` 21.5 s, `use-symlink` 25.5 s, cargo-nix-plugin 26.6 s, crate2nix 31.8 s. The author warns against choosing from the table alone: uncontrolled store, some rows from separate historical runs. **[self-reported, small project]** [13]
- **cargo-schnee**: per-unit content-addressed derivations from cargo's unit graph, requiring experimental `dynamic-derivations`, `ca-derivations`, `recursive-nix`. Its README (one project, Just 1.40.0, **[self-reported]**): clean 58.1 s versus cargo 37.8 s and crane 44.5 s; incremental 18.1 s versus cargo 33.0 s, crane 45.4 s, and cargo2nix 31.5 s. 77 commits, 1 star, no releases. [10]
- **cargo-dyndrv** (announced 2026-09-16): per-crate derivations from cargo's unit graph using dynamic derivations. No benchmarks; the post calls it "A Beginning". [9]
- **Nix experimental status**: `dynamic-derivations`, `ca-derivations`, `recursive-nix` are all listed experimental and "can be changed or removed at any time". Lix removed dynamic derivations in 2.94 (Nov 2025), and the upstream design is expected to be replaced before stabilisation. [60][61]
- **Binary caches**: attic ("multi-tenant Nix binary cache", 2,164 stars, pushed 2026-11-16) and harmonia (657 stars, pushed 2026-11-15) are maintained self-hosted options. [62][63]
**Confidence**: Medium (repo facts High; the benchmarks are weak evidence).
**Analysis [analysis]**: Granularity is the trade-off. crane = 2 derivations, coarse reuse, but works with cargo incremental via `cargoArtifacts`. crate2nix = per crate, no incremental. Per-unit dynamic-derivation tools are experimental. Nix sandboxes cannot run the repo's Tier-3 tests.

## Comparison Table

| | **sccache** | **kache** | **Cargo-native (2026)** | **Bazel + rules_rust** | **Buck2 + reindeer** | **Nix: crane / crate2nix** |
|---|---|---|---|---|---|---|
| Deps cached | Yes | Yes (vendor-documented) | Planned (immutable only, nightly) | Yes | Yes | crane: whole dep set as one derivation; crate2nix: per crate |
| Workspace crates | Not when incremental; yes at `CARGO_INCREMENTAL=0` | Yes by key; edited units use private incremental | No (path deps excluded at first) | Yes, action-level | Yes, action-level | crane: no (rebuilt as one); crate2nix: yes |
| Linked outputs (bin, test bin, proc-macro) | No | Yes (executables default on Linux/macOS) | No | Yes | Yes | Yes (a derivation output) |
| Build scripts | Always re-run | Cached (keyed by declared inputs) | Excluded in v1 | Modelled (`cargo_build_script`), cached | Modelled (reindeer fixups), cached | Run once inside a derivation |
| Test results | No | No | No | **Yes** (test target caching) | Unverified (2023 review: not by default) | Only as derivations (hermetic tests only) |
| Incremental compat | Errors or skips | Private per-unit + 1.98+ donor seeds (vendor) | Local only | Open PR #3925 (workspace only); not cached remotely | Local only, never uploaded (code) | crane via `cargoArtifacts`; crate2nix none |
| Path sensitivity | `SCCACHE_BASEDIRS`; `CARGO_MANIFEST_DIR` caveat | Sentinel normalisation; exceptions listed | Open design issue (#12516) | Sandboxed execroot; `PATH`/tool leaks possible | Hermetic action model | Nix sandbox, fixed store paths |
| Concurrent writers | Server-managed | Per-machine flights; multi-VM not documented; `index.db` lock issue #1698 | File lock per build dir | CAS is content-addressed (safe by construction) | Same CAS model | Store is immutable and path-addressed |
| Self-hostable on-node backend | Redis/S3/local disk/sccache-dist | S3-compatible/FS/OCI; planner service preview | n/a | bazel-remote, Buildbarn, BuildBuddy OSS, NativeLink (FSL) | Buildbarn / BuildBuddy / EngFlow documented | attic, harmonia |
| Maturity / maintenance | v0.19.0 2026-10-24; mature | v2.1.1 2026-11-12; 9 months old; very active | Goals accepted; narrow start | rules_rust 0.75.0 2026-10-26; mature | Active; pre-1.0 Cargo bridge | crane v0.24.0 2026-08-21; crate2nix active; **cargo2nix stale (2025-06-19)**; dyndrv/schnee experimental |
| Stale / unmaintained | **cachepot archived 2023-02-28** | - | - | - | **buck-reindeer crate (0.0.1, 2023) is a different, old crate** | **cargo2nix** |
| Keeps cargo workflow | Yes | Yes | Yes | Partly (dual path) | Partly (dual path) | Partly |
| Measured benchmarks | None current found | **[vendor]** disk claim; no timing numbers | None | **[self-reported]** DFINITY (qualitative), Quantum AI 80% CI | **[vendor]** Meta 2x vs Buck1 | **[self-reported]** crane-eval, cargo-schnee (single projects) |

## Migration Cost for This Workspace

All estimates in this section are **[analysis]**, derived from the local facts above and the cited behaviours. They are ranges for planning, not measurements. Engineer-weeks assume one engineer already fluent in the tool.

### A. kache pilot (cargo stays)
- **To write**: `kache init` on the VM image (cargo wrapper config); a store location policy; optionally `kache.toml` entries for untracked proc-macro inputs (`#[overdrive_test]` reads nothing outside rustc's tracked files as far as I could tell; verify). Rust 1.95.0 is inside kache's stated support (CLI needs Rust 1.95+; repo pins 1.99). [64]
- **What breaks / needs care**:
  - The Metal runner `env` allowlist excludes `KACHE_*`, `RUSTC_WRAPPER`, `SCCACHE_*`, so configuration has to live in a config file on the box (`~/.cargo/config.toml` or the shim from `kache init`), not on the command line. Confirm the runner's `.cargo/config*` check (it forbids `runner`) tolerates a wrapper. **[analysis, unverified]**
  - `crates/overdrive-dataplane/build.rs` emits absolute `CARGO_WORKSPACE_DIR` and `OVERDRIVE_BPF_OBJECT_PATH` through `rustc-env`. kache lists `CARGO_MANIFEST_DIR` readers as re-keying per build directory. If VMs mount checkouts at different paths, `overdrive-dataplane` and all of its dependents (cli, control-plane, sim, worker...) could miss across VMs. The simplest mitigation is giving every VM the **same checkout path**. Whether kache normalises `rustc-env` values is unverified.
  - trybuild builds use real paths and separate keys in kache (no cross-VM sharing for those tests; they are serialised anyway by the nextest `trybuild` group).
  - `placement-lint` and the Metal wrapper read ELF sections from test binaries. Restoring a cached test binary must preserve the `overdrive_test_placement` section. This should hold (restore returns the same bytes) but must be checked.
  - Incremental: the dev profile has incremental on; kache's adaptive mode toggles per-unit private state. The "donor seeds" feature needs rustc >= 1.98 (toolchain is 1.95.0).
- **Dual path**: not needed; it is a wrapper. Rollback is removing the wrapper.
- **Effort**: 2–5 days integration, 1–2 weeks to run the measurement plan.
- **Riskiest parts**: multi-VM shared-store semantics (undocumented), young codebase with 29 open issues including a cache-poisoning trust-boundary issue, and vendor-only evidence.

### B. Bazel + rules_rust (cargo stays as parallel path)
- **To write**:
  1. `MODULE.bazel`, pinned Rust toolchain 1.95.0 and the exec/target platforms; `crate.from_cargo` over `Cargo.toml`/`Cargo.lock`. 1–2 weeks.
  2. BUILD files for the 19 members. Each crate needs a `rust_library`, bins, and one `rust_test` per test target (acceptance/integration/compile_fail binaries), with `integration-tests`/`kvm-tests` modelled as Bazel flags because cargo unifies features workspace-wide. Adapting an existing generator (Materialize's `cargo_gazelle`, DFINITY's ~90% automatic conversion, `gazelle_rust`) is realistic: 1–2 weeks if adapted, 2–3 weeks by hand.
  3. Third-party annotations for crates with build scripts or native code: `ring`, `aws-lc-sys`, `libsql-ffi`, `wasmtime` and its dependencies, `linux-keyutils`, `ktls`. 1–3 weeks. (Which crates need fixes is inferred from the dependency list, not tested.)
  4. The three `build.rs` files. The two BPF build scripts become declared data dependencies (`compile_data` plus a `rustc_env` entry) in Bazel and stay as they are for cargo. The `cc` test crate maps to `cargo_build_script` with `build_script_use_cc_toolchain`. 3–5 days.
  5. **The BPF object pipeline** (`bpfel-unknown-none`, nightly, `-Z build-std=core`). Two options: (i) an action that wraps `cargo +nightly build` with vendored dependencies and declared inputs (DFINITY did the black-box-cargo pattern first), which yields one cacheable artifact but is only hermetic if the sandbox gets vendored crates and the toolchain; or (ii) a native custom-target toolchain, for which I found no rules_rust example combining a `no_std` custom target with build-std. Option (i): 1–2 weeks. Option (ii): 3–4 weeks and unproven.
  6. Dev tooling: rust-analyzer setup, a clippy aspect or config, rustfmt. About 1 week. DFINITY found rules_rust's rust-analyzer support choked on a large codebase in 2023; 2026 state not measured.
- **Test model (largest risk, 3–6 weeks and several user decisions)**:
  - The repo's test isolation is **process-per-test under nextest**, with `serial_test` explicitly insufficient and `host-kernel-shared` / `trybuild` groups (`max-threads = 1`). A Bazel `rust_test` runs a whole libtest binary in one process, so in-process global state (`OnceLock`, env vars, process-global kernel state) behaves differently. Restoring parity means a per-test runner, which Bazel does not provide for rules_rust by default. **[analysis]**
  - Serialising node-global kernel tests maps roughly to `exclusive`/`local` tags, which serialise the whole machine, a coarser contract than today's group.
  - The Metal runner grammar is closed: `bazel` is not an accepted command, and `--archive-file`/`--binaries-metadata` are refused, so Bazel-built binaries cannot be fed to nextest or Metal without a user-approved grammar change (a design decision this document only flags).
  - Test-result caching would apply only to hermetic default-lane tests; tests that touch kernel state must be tagged non-cacheable.
- **What breaks or needs replacing**: `cargo nextest` (hooks, the rdeps filter), `cargo-mutants` (cargo-only, stays on cargo), `cargo xtask` subcommands that call cargo, lefthook gates, `placement-lint` (reads test ELFs; Bazel-built test ELFs would need the same section, which `#[overdrive_test]` emits regardless of build system, but the lint's listing step uses nextest), trybuild (spawns cargo; keep on cargo path).
- **Dual path viability**: yes in principle. crate_universe consumes Cargo manifests and the workspace BUILD files can be generated. DFINITY did this, running `cargo check --tests --benches` in CI plus a parity checker. The cost is permanent: every change to `Cargo.toml` features, targets, `build.rs` or dev-dependencies must be mirrored, and feature unification differences are a silent-divergence risk.
- **Effort**: ~10–18 engineer-weeks to a Bazel build and test of the default lane with remote caching; Tier-3 lanes, mutants, trybuild and hooks stay on cargo.
- **Riskiest parts**: BPF build-std toolchain, the per-test process model, Metal grammar incompatibility, permanent dual-path maintenance, no incremental support for the VM edit loop (a Bazel build would be slower than cargo incremental for edits).

### C. Buck2 + reindeer
- **Same list as Bazel plus**: committed `vendor/` managed by `reindeer vendor` (the vendor dir is "fully managed" and strongly recommended), a `fixups.toml` for every crate with a `build.rs`, prelude via git submodule or bundled, nightly-only pipelining flag (`-Zno-codegen`) in the prelude, and `cargo-buckal` still pre-1.0. No test caching confirmed in current docs. The nextest/Metal/mutants incompatibilities are identical to Bazel's.
- **Effort**: ~40% above Bazel, about 14–26 engineer-weeks **[analysis]**.
- **Riskiest parts**: thinner documentation and community, REAPI backends documented only for EngFlow, Buildbarn, BuildBuddy (bazel-remote not listed), and no outside-Meta Rust migration report with numbers found.

### D. Nix (crane) for CI/artifact sharing, cargo stays in VMs
- **To write**: a flake with the pinned toolchain (1.95.0 plus a bpfel/nightly BPF derivation), `buildDepsOnly` plus workspace derivations, a patch or env override for the `build.rs` absolute-path and mtime check (the existing `OVERDRIVE_BPF_OBJECT` override helps), attic or harmonia as the binary cache.
- **What breaks**: Nix sandboxes cannot run the Tier-3 tests; coarse reuse (all workspace crates rebuild on any workspace change); Nix would not serve the in-VM edit loop.
- **Effort**: ~3–6 engineer-weeks **[analysis]**.
- **Riskiest parts**: the BPF derivation (build-std on a pinned nightly), and the temptation to adopt experimental per-unit tools.

## Recommendation (reasoning kept separate from the evidence above)

**Ranked for this setup**
1. **Run a measured kache pilot (with sccache as the control).** It is the only option that targets all four sccache limits without leaving cargo.
2. **Keep Bazel + rules_rust as the strategic fallback,** to be chosen only if (a) kache fails the measurement plan, or (b) the team decides hermetic test-result caching and CI-grade reproducibility are worth 10–18 engineer-weeks and a permanent dual-path cost.
3. **Nix crane for CI artifact reuse,** independent of the VM decision.
4. **Buck2:** no advantage over Bazel for this workspace (same costs, thinner ecosystem, no confirmed test caching).
5. **sccache** remains the baseline.
6. **Cargo-native:** watch, not usable.

**Reasoning**
- Hermetic systems (Bazel, Buck2) give the strongest correctness and are the only route to test-result caching, but this workspace's tests are mostly not hermetic (node-global kernel state, process-per-test, Metal runner), so the main extra benefit of a hermetic system lands on the minority default-lane tests.
- The Bazel/Buck2 incremental story works against the stated goal of a fast per-VM edit loop: Bazel's incremental support is an open PR, and Buck2 explicitly refuses to share incremental output. Cargo plus a wrapper keeps cargo's incremental compilation in the edit loop and uses the shared cache for non-edited units.
- The migration cost asymmetry is large (days versus months), so the rational sequence is pilot the cheap option, measure, and only then decide on migration.
- kache's evidence is the weakest of any candidate (one vendor, nine months, no timing numbers published), so adoption must be gated on own-hardware measurement and a trust-boundary review. The kache issue about pull-request jobs writing objects that `main` restores is directly relevant if the VMs differ in trust.
- A pragmatic zero-code lever independent of tool choice **[analysis]**: give every VM an identical checkout path, which removes the path-sensitivity problem that sccache's `BASEDIRS` and kache's sentinels both work around, and removes the risk from the dataplane `build.rs` absolute `rustc-env` paths.

**Decision criteria after measurement**: adopt kache if cross-VM hit rate on workspace crates plus linked test binaries at identical states is high enough that the median cold-to-warm time on a rebased VM drops meaningfully (suggested threshold: warm build of a rebased tree under 25% of a cold build) with zero wrong-hit incidents; otherwise write a Bazel spike under `spike-scratch/` per `.claude/rules/spike.md`.

## Source Analysis

Bias notes: kache docs, Buck2 "why" page and the cargo-schnee README are by the tool's own authors. The BuildBuddy Buck2 review comes from a vendor selling a Bazel service (and was corrected after publication). The crane-eval and cargo-schnee benchmarks cover one project each. The EngFlow and Google claims in recaps lack methodology.

| Source (short) | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| sccache releases / docs | api.github.com, raw.githubusercontent.com (mozilla) | High | official | 2026-11-17 | Y |
| cargo#5931, goals page, build-dir post | github.com/rust-lang, goals.rust-lang.org, blog.rust-lang.org | High | official | 2026-11-17 | Y |
| kache repo/docs | github.com, ninja.kunobi.com, docs.rs | Medium (vendor) | vendor | 2026-11-17 | N (no independent report) |
| cargo-reapi | docs.rs | Medium | vendor | 2026-11-17 | N |
| rules_rust releases, PR, docs | github.com/bazelbuild, bazelbuild.github.io | High | official | 2026-11-17 | Y |
| Bazel docs | bazel.build | High | official | 2026-11-17 | Y |
| Buck2 code, docs, Reindeer manual | github.com/facebook*, buck2.build | High (mechanics), Medium (claims) | official/vendor | 2026-11-17 | Y (code) |
| DFINITY blog | mmapped.blog | Medium-High | practitioner | 2026-11-17 | Partial |
| Materialize issue | github.com | Medium-High | practitioner | 2026-11-17 | N |
| Nix experimental docs, Discourse | nix.dev, discourse.nixos.org | High/Medium | official/community | 2026-11-17 | Y |
| crane docs/releases, crane-eval | crane.dev, github.com | Medium-High | official/self-reported | 2026-11-17 | Partial |
| bazel-remote, NativeLink, attic, harmonia | api.github.com | High (metadata) | official | 2026-11-17 | Y |

Reputation: High ~55% | Medium-high ~25% | Medium ~20%. Avg ~0.85.

## Knowledge Gaps

### Gap 1: Real cross-VM hit rates and speedups on this workspace
**Issue**: No independent benchmark for kache, and none with methodology for rules_rust Rust migrations. **Attempted**: kache benchmark page (no numbers), DFINITY/Materialize/Quantum AI sources. **Recommendation**: run the measurement plan.

### Gap 2: kache behaviour with the dataplane `build.rs` absolute `rustc-env` paths
**Issue**: Docs say `CARGO_MANIFEST_DIR` readers re-key per directory; nothing says whether `rustc-env` values emitted by build scripts are normalised. **Recommendation**: test directly (measurement plan S5).

### Gap 3: kache across multiple microVMs sharing one store
**Issue**: Concurrency docs are per machine and "do not address multiple VMs or shared-filesystem safety". Zero-copy reflink restore also needs one filesystem. **Recommendation**: test a local S3-compatible remote or OCI registry on the host rather than a shared filesystem; check #1698-style lock failures under load.

### Gap 4: Bazel native build of a `bpfel-unknown-none` + `-Z build-std` crate
**Issue**: No rules_rust example found. **Attempted**: web search, rules_rust cross-compile examples, bazel-rust-cross. **Recommendation**: spike only if Bazel is pursued.

### Gap 5: Buck2 test-result caching and a current outside-Meta Rust adopter report
**Issue**: The 2023 review says no test caching by default; the 2026 docs page for RE does not address it; no 2026 adopter with numbers. **Recommendation**: read buck2 test docs if Buck2 is considered further.

### Gap 6: rules_rust incremental PR timeline and rustc "donor seed" semantics
**Issue**: PR #3925 has no reviews. kache's rustc 1.98+ donor-seed claim was not corroborated by any rust-lang source I found. **Recommendation**: do not plan on either feature.

### Gap 7: Items not retrievable
RustConf 2026 Bazel RBE talk (HTTP 403), Sourcegraph's Bazel-for-Rust guide (HTTP 404 on the new URL), `cargo_gazelle` source (page only found through a search hit), rest of the cargo#5931 thread after 2024, Determinate Nix position, Depot/Namespace self-hosting (searches returned no relevant documents).

### Gap 8: Workspace member count
The brief says 31 members; `Cargo.toml` lists 19. The estimates use 19.

## Conflicting Information

### Conflict 1: Is rustc incremental state shareable?
**Position A**: kache says donor seeds can import another checkout's incremental session on rustc 1.98+. [24]
**Position B**: No rust-lang source found shows relocatable incremental state; the 2026 goal "Incremental Systems Rethought" is a redesign goal, and Cargo's team says sharing build dirs across workspaces is unsupported. [3][19]
**Assessment**: Position A is a vendor claim about its own implementation, and could be consistent with B if kache copies state and validates it itself. Treat as unverified.

### Conflict 2: Is sccache effective for Rust on CI?
**Position A**: sccache docs present broad Rust caching. [5]
**Position B**: DFINITY "saw no improvement" with sccache on CI servers (2023). [39]
**Assessment**: Not contradictory; DFINITY's workspace crates and linked outputs fall in sccache's uncached categories.

## Recommendations for Further Research

1. Run the measurement plan below (own hardware).
2. If kache passes, review its trust model against the VMs' trust levels (issue #1719).
3. Re-check Cargo's cross-workspace cache nightly status around Q2 2027.
4. If Bazel is pursued, spike (i) the BPF object as a cargo-wrapping action and (ii) one `rust_test` per-process runner, under `spike-scratch/` per `.claude/rules/spike.md`, run on the Metal host, not Lima.

## Measurement Plan (run on our own hardware before deciding)

Methodology: pin rustc 1.95.0 and mold; record `uname -r`, filesystem, CPU count and RAM; at least 5 samples per cell, medians and spread; one warm-up discarded; every scenario also run with the control (plain cargo, then sccache with `CARGO_INCREMENTAL=0`). Keep full logs (append-only, per `.claude/rules/testing.md` § Diagnostic history). Run on the Metal host. Spikes and scratch configuration belong in `spike-scratch/`, not `crates/`.

| # | Scenario | What to measure | Against |
|---|---|---|---|
| S0 | Cold build, no cache, `cargo nextest run --no-run`-equivalent compile of the default lane and `cargo check` | Wall time, peak RSS, CPU-seconds | Baseline for all ratios |
| S1 | VM A builds `main@X`; empty VM B (same checkout path) builds the same commit | Hit rate by unit (deps / workspace / bins / test bins / build scripts), wall time | S0; sccache control |
| S2 | VM B rebases to `main@X+n` where the diff touches only leaf crates (`overdrive-cli`, `overdrive-gateway`) | Units rebuilt vs restored | S1 |
| S3 | Same, but the diff touches `overdrive-core` (worst-case fan-out) | Units rebuilt; time | S2 |
| S4 | **Edit loop**: in VM B, edit one function body in `overdrive-control-plane`, rebuild the test binary, 20 iterations, incremental on, with and without the wrapper | Median and p95 rebuild time; wrapper overhead | Plain cargo incremental |
| S5 | **Path sensitivity**: checkout at identical path vs different paths per VM; specifically `overdrive-dataplane` (build.rs absolute `rustc-env`) and its reverse dependencies | Hit/miss per unit | S1 |
| S6 | **Concurrency**: 8 VMs build the same commit simultaneously from a cold store | Total CPU-seconds, duplicate compiles, any lock errors (`index.db`-style), corrupted entries | S0 |
| S7 | **Correctness**: restore cached test binaries into VM B; run default-lane tests; run `placement-lint`; compare ELF `overdrive_test_placement` section and test results bytes against a locally built binary | Any divergence = fail | - |
| S8 | **cargo-mutants** on one crate with `--jobs 4` (uses `OVERDRIVE_BPF_OBJECT`) | Hit rate for unmutated crates across the temp copies; kill-rate unchanged | Plain cargo |
| S9 | Proc-macro and build-script behaviour: `overdrive-testing-macros`, the `cc` build script, `overdrive-init` build.rs | Hit/miss and correctness | - |
| S10 | Disk and remote: store size after S1–S3; local S3-compatible remote vs shared filesystem between VMs; time to restore | GB per VM; restore time | - |
| S11 | Toolchain bump to rustc >= 1.98 (spike branch only) | Whether kache donor seeds produce a measurable gain on S4 | S4 |

**Commits**: pick `main` HEAD (X), X+n with a small leaf diff, X+m with an `overdrive-core` diff, and a feature-branch commit that already carries incremental state.
**Crates/targets**: `overdrive-core` (foundation), `overdrive-control-plane` (largest), `overdrive-dataplane` and `overdrive-init` (build scripts, absolute paths), `overdrive-testing-macros` (proc-macro), `overdrive-cli` (bin), `overdrive-core --test acceptance` (linked test binary).
**Go / no-go**: adopt the wrapper only with zero wrong hits in S7, no data-loss/lock failures in S6, S4 overhead under about 10%, and a clear win in S1–S3. If not met, open a Bazel spike, and re-state the decision to the user.

## Full Citations

[1] rust-lang/cargo. "Per-user compiled artifact cache (#5931)". github.com (API). 2026-10-06. https://api.github.com/repos/rust-lang/cargo/issues/5931. Accessed 2026-11-17.
[1b] rust-lang/cargo. "#5931 comments". https://api.github.com/repos/rust-lang/cargo/issues/5931/comments?per_page=100&page=1&direction=desc. Accessed 2026-11-17.
[2] Rust Project Goals. "Cargo cross workspace cache". 2026. https://goals.rust-lang.org/2026/cargo-cross-workspace-cache.html. Accessed 2026-11-17.
[3] Ed Page. "Call for Testing: Build Dir Layout v2". blog.rust-lang.org. 2026-03-13. https://blog.rust-lang.org/2026/03/13/call-for-testing-build-dir-layout-v2. Accessed 2026-11-17.
[4] mozilla/sccache. "Releases". 2026. https://api.github.com/repos/mozilla/sccache/releases?per_page=6. Accessed 2026-11-17.
[5] mozilla/sccache. "docs/Rust.md". https://raw.githubusercontent.com/mozilla/sccache/main/docs/Rust.md. Accessed 2026-11-17.
[6] bazelbuild/rules_rust. "Releases". https://api.github.com/repos/bazelbuild/rules_rust/releases?per_page=5. Accessed 2026-11-17.
[7] bazelbuild/rules_rust. "PR #3925 incremental compilation". https://api.github.com/repos/bazelbuild/rules_rust/pulls/3925. Accessed 2026-11-17.
[8] facebook/buck2. "prelude/rust/build.bzl". https://raw.githubusercontent.com/facebook/buck2/main/prelude/rust/build.bzl. Accessed 2026-11-17.
[9] Ericson2314 / artemist. "Cargo-dyndrv". NixOS Discourse. 2026-09-16. https://discourse.nixos.org/t/cargo-dyndrv-a-new-way-of-building-cargo-packages-with-nix/80112. Accessed 2026-11-17.
[10] poly2it. "cargo-schnee". https://github.com/poly2it/cargo-schnee. Accessed 2026-11-17.
[11] nix-community/crate2nix. https://api.github.com/repos/nix-community/crate2nix. Accessed 2026-11-17.
[12] cargo2nix/cargo2nix. https://api.github.com/repos/cargo2nix/cargo2nix. Accessed 2026-11-17.
[13] dpc. "crane-eval". https://github.com/dpc/crane-eval. Accessed 2026-11-17.
[14] kunobi-ninja/kache. https://api.github.com/repos/kunobi-ninja/kache and https://raw.githubusercontent.com/kunobi-ninja/kache/main/README.md. Accessed 2026-11-17.
[15] kache docs. https://ninja.kunobi.com/docs/kache. Accessed 2026-11-17.
[16] kache crate. https://docs.rs/kache. Accessed 2026-11-17.
[17] kache. "Cache key". https://ninja.kunobi.com/docs/kache/how-it-works/cache-key. Accessed 2026-11-17.
[18] paritytech/cachepot. https://api.github.com/repos/paritytech/cachepot. Accessed 2026-11-17.
[19] Rust Project Goals 2026 index and program update. https://blog.rust-lang.org/inside-rust/2026/08/31/program-management-2026-jul-aug/. Accessed 2026-11-17.
[20] rustc incremental design. https://doc.rust-lang.org/stable/nightly-rustc/rustc_incremental/persist/fs/index.html. Accessed 2026-11-17.
[21] kache commits. https://api.github.com/repos/kunobi-ninja/kache/commits?per_page=1. Accessed 2026-11-17.
[22] kache path normalisation, same as [17].
[23] kache. "Mutation testing". https://ninja.kunobi.com/docs/kache/getting-started/mutation-testing. Accessed 2026-11-17.
[24] kache. "Worktrees". https://ninja.kunobi.com/docs/kache/getting-started/worktrees. Accessed 2026-11-17.
[25] kache. "Kache or sccache?". https://ninja.kunobi.com/docs/kache/getting-started/comparison. Accessed 2026-11-17.
[26] kache. "Concurrent builds". https://ninja.kunobi.com/docs/kache/how-it-works/limits. Accessed 2026-11-17.
[27] same as [23].
[28] kache. "Benchmarks". https://ninja.kunobi.com/docs/kache/benchmarks. Accessed 2026-11-17.
[29] kache open issues. https://api.github.com/repos/kunobi-ninja/kache/issues?state=open&per_page=30. Accessed 2026-11-17.
[30] TamedTornado. "cargo-reapi 0.2.0". https://docs.rs/crate/cargo-reapi/0.2.0. Accessed 2026-11-17.
[31] nnunley. "objfs". https://github.com/nnunley/objfs (search result; not fetched). Accessed 2026-11-17.
[32] sgeisler. "cargo-remote". https://packages.ecosyste.ms/registries/crates.io/packages/cargo-remote. Accessed 2026-11-17.
[33] rules_rust persistent worker / incremental discussions. https://github.com/bazelbuild/rules_rust/pull/3925 and https://groups.google.com/g/rules_rust/c/E3MJtgamlD4. Accessed 2026-11-17.
[34] rules_rust. "crate_universe (bzlmod)". https://bazelbuild.github.io/rules_rust/crate_universe_bzlmod.html. Accessed 2026-11-17.
[35] Bazel. "User manual". https://bazel.build/docs/user-manual. Accessed 2026-11-17.
[36] Bazel. "Remote caching". https://bazel.build/remote/caching. Accessed 2026-11-17.
[37] rules_rust. "rust-analyzer". https://bazelbuild.github.io/rules_rust/rust_analyzer.html. Accessed 2026-11-17.
[38] colatkinson. "bazel-rust-cross". https://github-redirect.dependabot.com/colatkinson/bazel-rust-cross. Accessed 2026-11-17.
[39] Roman Kashitsyn. "Scaling Rust builds with Bazel". mmapped.blog. 2023-03-20. https://mmapped.blog/posts/17-scaling-rust-builds-with-bazel. Accessed 2026-11-17. (Published 2023; qualitative.)
[40] MaterializeInc/materialize. "Build Materialize with Bazel (#26796)". https://github-redirect.dependabot.com/MaterializeInc/materialize/issues/26796. Accessed 2026-11-17.
[41] Materialize. "cargo_gazelle". https://dev.materialize.com/api/rust/src/cargo_gazelle/targets.rs.html. Accessed 2026-11-17.
[42] Julio Merino. "BazelCon 2025 recap". 2025-11. https://jmmv.dev/2025/11/bazelcon-2025-recap.html. Accessed 2026-11-17.
[43] P. Williams. "Introducing Bazel to a Rust Project". 2022-06-19. https://prestontw.github.io/blog/a-guided-bazel-migration-in-rust. Accessed 2026-11-17.
[44] Calsign. "gazelle_rust". https://api.github.com/repos/Calsign/gazelle_rust. Accessed 2026-11-17.
[45] RustConf 2026. "2 Hours to 10 Minutes". https://rustconf2026.sched.com/event/2KHtn/2-hours-to-10-minutes-building-rust-monorepos-at-scale-with-bazel-rbe. HTTP 403, abstract only via search. Accessed 2026-11-17.
[46] buchgr/bazel-remote. https://api.github.com/repos/buchgr/bazel-remote. Accessed 2026-11-17.
[47] TraceMachina/nativelink and LICENSE. https://api.github.com/repos/TraceMachina/nativelink ; https://raw.githubusercontent.com/TraceMachina/nativelink/main/LICENSE. Accessed 2026-11-17.
[48] Bazel. "Remote Execution Services". https://bazel.build/remote-execution-services.html. Accessed 2026-11-17.
[49] pistack. "Self-hosted remote build cache guide". 2026-05-03. https://www.pistack.xyz/posts/2026-05-03-self-hosted-remote-build-cache-buildbuddy-bazel-remote-nativelink-guide/. Accessed 2026-11-17. (Blog, directional.)
[50] facebook/buck2. https://api.github.com/repos/facebook/buck2. Accessed 2026-11-17.
[51] facebookincubator/reindeer. https://api.github.com/repos/facebookincubator/reindeer. Accessed 2026-11-17.
[52] Reindeer manual. https://raw.githubusercontent.com/facebookincubator/reindeer/main/docs/MANUAL.md. Accessed 2026-11-17.
[53] buck2hub. "cargo-buckal". https://docs.rs/crate/cargo-buckal/latest. Accessed 2026-11-17.
[54] Buck2. "Remote execution". https://buck2.build/docs/users/remote_execution/. Accessed 2026-11-17.
[55] Son Luong Ngoc. "Buck2 Unboxing". BuildBuddy. 2023-05-30. https://www.buildbuddy.io/blog/buck2-review. Accessed 2026-11-17. (Vendor, 2023.)
[56] Buck2. "Why Buck2". https://buck2.build/docs/about/why/. Accessed 2026-11-17.
[57] Reindeer adopters (k23, crates-pro) via search. https://jonaskruckenberg.github.io/k23/contributing/adding-a-dependency.html. Accessed 2026-11-17.
[58] ipetkov/crane releases. https://api.github.com/repos/ipetkov/crane/releases?per_page=3. Accessed 2026-11-17.
[59] crane API. https://crane.dev/API.html. Accessed 2026-11-17.
[60] Nix manual. "Experimental features". https://nix.dev/manual/nix/latest/development/experimental-features. Accessed 2026-11-17.
[61] "Dynamic Derivations + Dynamic build graphs: next steps". NixOS Discourse. 2026. https://discourse.nixos.org/t/dynamic-derivations-dynamic-build-graphs-next-steps/79409 ; Lix 2.94 notes https://docs.lix.systems/manual/lix/2.94/release-notes/rl-2.94.html. Accessed 2026-11-17 (search results only).
[62] zhaofengli/attic. https://api.github.com/repos/zhaofengli/attic. Accessed 2026-11-17.
[63] nix-community/harmonia. https://api.github.com/repos/nix-community/harmonia. Accessed 2026-11-17.
[64] kache compatibility. https://ninja.kunobi.com/docs/kache/compatibility. Accessed 2026-11-17.

## Research Metadata
Duration: one dispatch | Examined: ~75 pages/API responses | Cited: 64 | Cross-refs: kache vs sccache limits, Cargo goals vs issue thread, Buck2 code vs docs, Nix status across 3 sources | Confidence: High 35%, Medium 45%, Low 20% | Tool failures: no shell (`gh` unavailable; used GitHub REST API), HTTP 403/404 on 3 pages (see Gap 7) | Output: /Users/marcus/conductor/workspaces/helios/antananarivo/docs/research/platform/rust-shared-build-cache-alternatives-research.md
