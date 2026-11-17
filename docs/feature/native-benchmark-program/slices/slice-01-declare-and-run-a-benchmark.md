# Slice 01 — Declare a benchmark and run it on the exclusive host (walking skeleton)

Story: US-BENCH-01. Job: J-PLAT-008. Family: `cgroup-connect-hook`, native substrate.

## Goal
A benchmark function carrying the attribute runs through the benchmark form of `cargo xtask metal run` on the exclusive host and returns retained raw samples and a per-arm report.

## In scope
- The attribute on a function (family, substrate, arms, replicates), checked against a closed substrate registry (native only).
- The measurement core crate (dev-dependency-only, `adapter-host`): per-replicate fresh fixture, sample recording, failure counting.
- The benchmark form of `metal run`, exclusive admission, retained record brought back with `metal pull`.
- The connect-hook benchmark: arm A service hook only, arm B service plus gateway hook, loopback connect-and-close, hook time per connect and wall time per connect.
- Report: per-arm median, IQR, replicate count, substrate, commit and dirty flag, methodology version.
- Catalogue row for the family; `testing.md` exemption for exclusive benchmarks (orchestrator process).

## Out of scope
Interleaving, A/A, canary, intervals, host-settings refusal, spike use, smoke variant (slices 02 to 04).

## Learning hypothesis
Disproves: that a macro-declared function can run on an exclusive Metal benchmark form with its records retrievable. Confirms the shape of rulings 4, 8, 9 end to end.

## Acceptance
- [ ] Records hold every sample of both arms; report derived from them.
- [ ] A failed replicate is counted and reported.
- [ ] Exclusive admission; no ordinary run overlaps.
- [ ] No launch of the `overdrive` binary.
- [ ] Unknown substrate rejected by name.
- [ ] Catalogue row names workload matrix, clock boundary, result owner.

## Dependencies
Shipped placement attribute and registry; Metal host. None on other slices.

## Estimate
2 to 3 days. Reference class: the placement-attribute skeleton (1 to 3 days). Pre-slice spike: none; the spike data already exists.
