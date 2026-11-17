# V-5 — linkme registration across crates

**Verdict: HOLDS, with one edge.** Shapes DDD-3 / ADR-0298 (`linkme` distributed slice).

Host: AMD EPYC 8024P (8-core), `uname -r` 7.0.0-38-generic, Cloud Hypervisor v53.0.0, rustc 1.95.0, commit e3b42c92e (DESIGN docs dirty). Run 2026-11-16/17 on the native Metal host with `cargo xtask metal run --exclusive`.

Probe: `spike-scratch/nbp-v5-linkme/` (`results.txt`). Workspace profile: `lto = "thin"`, `codegen-units = 1` for bench and release. `linkme` 0.3.38; the slice is owned by a library.

Command: `cargo xtask metal run --exclusive -- bash spike-scratch/nbp-v5-linkme/run.sh`

## Predicted vs actual

Prediction: 3 entries declared in a dependency library are seen in both a `harness = false` bench binary and the library's unit-test binary.

| Binary | Entries seen |
|---|---|
| Library unit-test binary | 3 of 3 |
| Separate declaring crate's unit-test binary | 6 of 6 (3 own + 3 library) |
| `harness = false` bench, declaring crate referenced (`extern crate`) | 6 of 6 |
| `harness = false` bench, declaring crate not referenced | 3 of 6 — the dependency's 3 entries silently dropped |

## Edge case

rustc does not link an unreferenced dependency crate, so its `linkme` entries vanish with no error. The bench `main` (and any `spike-scratch` equivalent) must name the declaring crate, e.g. `use overdrive_bench_families as _;`. The smoke unit-test binaries are unaffected, since the declarations sit in the crate under test.

## Recommendation

DDD-3 and ADR-0298 stand; the explicit-list fallback is not needed. Add the "bench main references the declaring crate" rule and a runner assertion on the expected registered-family count.
