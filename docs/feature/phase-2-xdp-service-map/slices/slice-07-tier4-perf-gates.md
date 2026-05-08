# Slice 07 — Tier 4 perf gates + veristat baseline land on `main`

**Story**: US-07
**Backbone activity**: 7 (Enforce perf gates)
**Effort**: 1 day
**Depends on**: Slice 02 (baseline source #1), Slice 04 (baseline source #2), Slice 05 (baseline source #3), Slice 06 (final baseline state). This slice closes #24.

## Outcome

Phase 2.1's `cargo xtask verifier-regress` and `cargo xtask xdp-perf`
stubs (at `xtask/src/main.rs:588-594`, marked `// TODO(#29): wire
when first real program lands` per ADR-0038 D8) are filled in:

- **`cargo xtask verifier-regress`**: runs `veristat` against every
  compiled BPF program; compares against
  `perf-baseline/main/veristat-{program}.txt`; fails the build if
  any program's instruction count exceeds its baseline by >5% or
  approaches the per-program complexity ceiling by >10%.
- **`cargo xtask xdp-perf`**: runs `xdp-trafficgen` + `xdp-bench`
  (DROP + TX + LB-forward modes) against the loaded program inside
  the calling host's Lima VM; measures pps and p99 latency; compares
  against `perf-baseline/main/xdp-perf-{mode}.txt`; fails the build
  if pps drops by >5% or p99 latency rises by >10%.

Baseline files land under `perf-baseline/main/`:
`veristat-service-map.txt`, `veristat-reverse-nat.txt`,
`xdp-perf-drop.txt`, `xdp-perf-tx.txt`, `xdp-perf-lb-forward.txt`.

Both xtask subcommands wire into the per-PR CI workflow per
`.claude/rules/testing.md` § "CI topology" Job E. Single-kernel
in-host per #152 — the developer's Lima VM (or CI's `ubuntu-latest`
runner) is the canonical target; the kernel-matrix variant is
deferred to #152's slice.

A light DST-invariant-shaped self-test (`PerfBaselineGatesEnforced`)
asserts the gate logic itself works: the xtask subcommand returns
non-zero on a synthetic >5% regression input, zero on a synthetic
2% input. This is the gate's own correctness check, not a measurement.

## Value hypothesis

*If* there's no real PR-blocking perf gate, every later Phase 2
slice (POLICY_MAP, IDENTITY_MAP, FS_POLICY_MAP, conntrack #154,
sockops, kTLS) has no way to detect verifier or pps regressions
before merge — the baselines die of attrition. *Conversely*, with
the gates real and structured, the first hand-validated false-
positive trip on a follow-on PR is the disproof attempt: if the gate
fires on a PR that is genuinely not regressing, the gate logic is
wrong; if it correctly catches a true regression, the gate is paying
off.

## Disproves (named pre-commitment)

- **"Tier 4 can stay deferred to a later phase."** No — every Phase
  2.3+ slice will keep adding eBPF code; without enforcement the
  baseline dies of attrition.
- **"Absolute-pps thresholds work."** No, per `.claude/rules/testing.md`
  — relative-delta only.

## Scope (in)

- `cargo xtask verifier-regress` real implementation (replaces Phase 2.1 stub at `xtask/src/main.rs:588`).
- `cargo xtask xdp-perf` real implementation (replaces Phase 2.1 stub at `xtask/src/main.rs:594`).
- Baseline files under `perf-baseline/main/`: `veristat-service-map.txt`, `veristat-reverse-nat.txt`, `xdp-perf-drop.txt`, `xdp-perf-tx.txt`, `xdp-perf-lb-forward.txt`.
- Both xtask subcommands wired into per-PR CI workflow Job E.
- Single-kernel in-host execution per #152.
- Structured-output failure messages (program / metric / baseline / measured / threshold).
- Baseline-update commit-message convention documented in slice brief and CONTRIBUTING.md (or equivalent).
- xtask self-test invariant `PerfBaselineGatesEnforced`.
- `xdp-tools` install added to `infra/lima/overdrive-dev.yaml` (alongside the existing `bpf-linker` install).

## Scope (out)

- Kernel matrix (#152) — `cargo xtask integration-test vm` LVH harness from Phase 2.1 stays in place but is not exercised by this slice.
- Conntrack-related metrics (#154).
- Optional `cargo xtask perf-baseline-update` helper (DESIGN may add as a future ergonomic improvement; not required).
- Trend visualisation / DuckLake telemetry of CI metrics (whitepaper § 22 stretch goal; not in this slice).

## Target KPI

- 100% of PRs that breach 5% / 10% / 5% thresholds fail CI (no false negatives).
- 0 false positives on the first three Phase 2.3+ follow-on PRs (hand-validated by author).
- Both xtask subcommands return well-formed structured output on every run.

## Acceptance flavour

See US-07 scenarios. Focus: real CI gate; structured output; relative
deltas only; baseline-update commit-message convention; single-kernel
in-host execution per #152.

## Failure modes to defend

- Runner-class variance produces flaky absolute pps numbers: gate is
  on RELATIVE delta only per `.claude/rules/testing.md` § Tier 4.
- Baseline-update PR lands without rationale: contributor convention
  (commit message must explain). Not a mechanical check; reviewer
  flags it.
- xtask self-test fails on a synthetic >5% regression: the gate
  logic itself is wrong — slice's CI run catches this at PR time.

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — `verifier-regress` body + `xdp-perf` body + baseline directory + CI workflow wiring (4) |
| No hypothetical abstractions landing later | PASS — replaces existing Phase 2.1 stubs; uses existing xtask plumbing |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — real veristat / real xdp-bench against compiled programs; real CI workflow integration |
| Demonstrable in single session | PASS — `cargo xtask verifier-regress` and `cargo xtask xdp-perf` run on developer's Lima VM in single session; structured output renders in terminal |
| Same-day dogfood moment | PASS — Linux developer drops a deliberate +5% regression, watches the gate fire |
