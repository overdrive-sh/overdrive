# Linker benchmark — Overdrive workspace

**Date:** 2026-05-03
**Host:** Apple Silicon → Lima VM (Ubuntu 24.04, kernel 6.8, aarch64,
8 vCPU, 16 GiB RAM)
**Toolchain:** rustc 1.95.0 (59807616e 2026-04-14)
**Workspace:** 8 crates × 62 test binaries (with `--features
integration-tests`)

## Linkers

| Variant | Version | Invocation |
|---|---|---|
| `default` | GNU `ld.bfd` (binutils, via gcc) | _no `RUSTFLAGS`_ |
| `lld` | Ubuntu LLD 18.1.3 | `RUSTFLAGS="-C link-arg=-fuse-ld=lld"` |
| `mold` | mold 2.30.0 | `RUSTFLAGS="-C link-arg=-fuse-ld=mold"` |
| `wild` | wild 0.8.0 | `RUSTFLAGS="-C linker=clang -C link-arg=--ld-path=$(which wild)"` |

`wild` is invoked via clang's `--ld-path` because Ubuntu 24.04's gcc
does not yet recognise `-fuse-ld=wild` (only `bfd|gold|lld|mold`).

## Workload 1 — clean full-workspace test build

`cargo nextest run --no-run --workspace --features integration-tests`
after `cargo clean`. Single timed run per variant with a per-variant
`CARGO_TARGET_DIR` (no shared cache). One run only — clean builds are
expensive enough that hyperfine multi-run was infeasible inside the
30-min Bash window.

| Variant | Time | vs default |
|---|---:|---:|
| default | 226 s | 1.00× |
| lld | 171 s | 0.76× (-24%) |
| mold | 152 s | 0.67× (-33%) |
| wild | **99 s** | **0.44× (-56%)** |

All four produced 62 executable binaries; total binary size 1665–1709 MB
(variation ≤ 2.6%); a wild-built `overdrive_core-*` binary `--list`'d 26
tests cleanly, confirming the artifacts are functional.

## Workload 2 — incremental rebuild after one-line change

Append `// bench touch <RANDOM>` to `crates/overdrive-core/src/lib.rs`
(bottom of the dep tree — every other crate rebuilds), then
`cargo nextest run --no-run --workspace --features integration-tests`.
Reuses the warm `target-bench-X/` from workload 1.
hyperfine `--warmup 2 --runs 5`.

| Variant | Mean ± σ | Min / Max | vs default |
|---|---:|---:|---:|
| default | 40.886 ± 1.446 s | 39.42 / 42.69 | 1.00× |
| lld | 13.546 ± 0.420 s | 12.82 / 13.81 | **0.33× (-67%)** |
| mold | 13.477 ± 0.600 s | 12.75 / 14.12 | **0.33× (-67%)** |
| wild | 12.941 ± 0.338 s | 12.52 / 13.25 | **0.32× (-68%)** |

Tight stddev across all variants (≤ 4.5%). The three alternatives are
statistically indistinguishable from one another; default is in a
different league of slow.

## What this means for `cargo xtask mutants`

Not measured directly, but the mutants gate is a loop of
(touch source → cargo build → relink test binary → nextest run).
The "compile + link" phase per mutant is the same shape as workload 2;
the test-execution phase is independent of linker. With ~70% of the
per-mutant time saved on the compile+link half, even the most
test-execution-heavy mutants run gets a substantial win — and a
diff-scoped `cargo xtask mutants --diff origin/main` (compile-link
heavy, short test runs) sees most of the workload-2 saving directly.

## What this does NOT speed up

- `cargo check` / `cargo clippy` — never invoke the linker. Confirmed
  unchanged.
- DST simulation runtime, integration test execution time — those are
  CPU-bound test bodies, linker-independent.
- Host-side macOS builds — mold and wild are Linux-only (mold's macOS
  port was abandoned; wild has no Mach-O target). The benefit applies
  only inside Lima for the user's current setup.

## Recommendation

**Switch to mold for Linux builds.** It hits the ≥20% threshold on both
workloads (-33% clean / -67% incremental), is the mature production-ready
choice (wild is still 0.x), and integrates with the standard
`-C link-arg=-fuse-ld=mold` invocation that gcc and clang both understand
without the `--ld-path` workaround wild currently needs.

`wild` is a touch faster but the marginal win (≤ 5% in workload 2,
larger in workload 1 but a single-run measurement) does not justify
the version-skew risk on a 0.x linker. Worth re-evaluating after wild
1.0 ships.

`lld` is essentially equivalent to mold here; mold has better
single-binary throughput on x86_64 in published benchmarks and is
slightly easier to install (apt-only vs needing LLVM). No reason to
prefer lld.

## Reproducing

Inside Lima (mold + wild + hyperfine pre-installed via the dev VM yaml):

```
# Workload 1 — clean full-workspace test build, one shot per variant
for v in default lld mold wild; do
  cargo xtask lima run --no-sudo -- bash scripts/benchmark-linker/w1.sh $v
done

# Workload 2 — incremental rebuild, hyperfine multi-run per variant
# (must follow workload 1 — reuses target-bench-$v as warm cache)
for v in default lld mold wild; do
  cargo xtask lima run --no-sudo -- bash scripts/benchmark-linker/w2.sh $v
done

# Cleanup the per-variant target dirs (~16 GB total)
cargo xtask lima run --no-sudo -- rm -rf target-bench-{default,lld,mold,wild}
```

Per-run logs and hyperfine JSON/Markdown output land in
`target/bench-linker/` (gitignored under `target/`).

## CI is a separate question

GitHub-hosted Ubuntu runners have a different cache topology, runner
hardware varies by time of day, and the linker's share of CI wall-clock
is smaller than incremental dev (rust-cache restore + test execution
dominate). The local Lima win does not extrapolate directly. Worth a
matrix benchmark on a temporary workflow before changing CI config —
not done in this round.
