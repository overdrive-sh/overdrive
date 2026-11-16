# Native benchmark program — intake

Feature ID: `native-benchmark-program` · Anchor issue: [#287](https://github.com/overdrive-sh/overdrive/issues/287)
Classification: infrastructure / tooling, brownfield. Gathered 2026-11-13.

## User requirements (from the request)

1. Implement #287: a native performance-benchmark program with a catalogue,
   versioned methodology, machine-readable raw results, distributions and
   environment metadata. Regression gating is a later, separate decision.
2. Include benchmark candidates already named in other GitHub issues.
3. Support **userspace vs in-kernel** comparisons as used in spikes
   (A/B arms of one boundary).
4. **One framework, two uses:** usable from tests (repeatable, rerunnable lanes)
   and from spikes (throwaway `spike-scratch/` probes that still produce
   comparable, retained evidence).

## Benchmark candidates found in GitHub issues

| Source | Benchmark | Boundary / arms | Notes |
|---|---|---|---|
| #287 (family 1), #283 | VM lifecycle stages | admission/queue, VMM launch, guest READY, Stable, stop submit, guest receipt, workload exit, poweroff, VMM reaper exit, cleanup | cold/warm image cache, concurrency, CH + kernel versions; healthy-path vs grace/failure kept distinct |
| #287 (family 2) | DNS responder | client-observed vs resolver-internal | query shape, cache hit/miss, cardinality, transport, concurrency, response size |
| #287 (family 3), ADR-0069 | Transparent mTLS / kTLS / agent-light L4 proxy | plaintext baseline vs proxy+kTLS; both directions | handshake, first byte, throughput, tail latency, CPU, memory, sockets; forward userspace vs return splice work |
| #287 comment (#54 P-41) | `cgroup/connect4` hook per-connect cost | arm A: service hook only; arm B: service + gateway hook | `bpf_stats_enabled=1`, `run_time/run_count` + wall time; 30 interleaved runs/arm; first data point in `docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md`, probe `spike-scratch/pig-connect-cost/` |
| #326 (#295 V-9) | vsock attachment density | 1 / 64 / 1,024 / 4,096 / 16,384 attachments | per-attachment kernel memory, tasks, setup/teardown, connect p50/p99, throughput; through `serve` + `deploy`; starting point `spike-scratch/netns-density-295-kernel-vs-aya-benchmark/`, `spike-scratch/netns-density-295-evmod/` |
| #300 | kTLS/splice pump scaling | thread-per-pump vs io_uring / bounded executor / alternatives | FDs, threads, `pids.max`, stacks, CPU, memory, handshake cost at concurrent-flow profiles |
| #303 | In-guest kernel mTLS (Camblet-style module) vs host proxy | in-guest kTLS vs host userspace proxy | handshake over vsock (spike: 19–28 ms single samples, explicitly "not a benchmark"); per-flow host cost |
| #305 | Firecracker vs Cloud Hypervisor | VMM A/B | cold boot, footprint, density — "quantify with the #287 benchmark program" |
| #96 / #100 | Snapshot/restore | `memory_restore_mode` copy vs ondemand; prefault | restore latency, guest resume, RSS, disk; prior method: 5 interleaved trials/mode, cold page cache, controls, nothing discarded |
| #329 | vDPA / SR-IOV hardware path | hardware vs software path | wire-speed/latency claims need raw output; methodology deferred to #326 |
| #310 / #309 | vsock UDP datagram path | frame size up to 65,507 B; inbound association | prior kernel-vs-Aya benchmark (commit b63b1507d) |
| #146 | Linker comparison | default / lld / mold / wild | existing `scripts/benchmark-linker/` (hyperfine); build-time benchmark, not product |
| #157 | XDP perf-baseline update helper | — | existing Tier-4 `xdp-perf` gate + `perf-baseline/main/`; #287 must not absorb it |

## Userspace vs in-kernel (spike comparisons)

Recurring shape across #295 (`ovd_fwd` kernel forwarder vs Aya/userspace),
#303 (in-guest kernel mTLS vs host userspace proxy), #300 (splice pumps vs
io_uring), #310 (no userspace payload relay) and `.claude/rules/design.md`
§ "Look for a kernel (BPF) mechanism before designing a userspace one". The
framework needs a first-class **A/B arm** model over one fixed boundary and
workload, with interleaved runs and attribution of CPU/memory to kernel vs
userspace.

## Existing assets to account for

- Tier-4 gates: `cargo verifier-regress`, `cargo xtask xdp-perf`, `perf-baseline/main/{verifier-budget,xdp-perf}`.
- Spike harnesses: `spike-scratch/{boot-cost,pig-connect-cost,netns-density-295-kernel-vs-aya-benchmark,netns-density-295-evmod}`.
- `scripts/benchmark-linker/` (hyperfine).
- Native metal runner (`cargo xtask metal run`), its per-run node-view isolation and run records.

## Boundaries already set by rules and issues

- Benchmarks are not EDD expectations (#288) and not E2E tests (#289); `.claude/rules/testing.md` § "Classify external execution".
- Tests never spawn the `overdrive` binary; spike code lives in `spike-scratch/`, never `crates/`.
- Regression gating and baseline promotion are proposed separately.

## Proposal — declare benchmarks with an attribute macro (not yet approved)

Modelled on the placement macro in [#354](https://github.com/overdrive-sh/overdrive/issues/354),
but the expansion differs: a benchmark is not a pass/fail `#[test]`. The macro
registers a scenario into a link-time list (`linkme` / `inventory`, as
`#[divan::bench]` does) consumed by one runner `main` in a `harness = false`
target — the measurement core.

```rust
// Illustrative only; DESIGN pins the real surface.
#[overdrive_bench(
    family = "cgroup-connect-hook",
    placement = native,              // or guest = "cgroup-default" (#354 vocabulary)
    replicates = 30,
    arms = ["service-only", "service+gateway"],
)]
fn connect_hook_cost(arm: Arm, run: &mut Run) {
    let fx = Fixture::fresh(arm);    // built per replicate, never shared between them
    run.span("connect", || fx.connect_and_close(5_000));
    run.sample_bpf_stats(&fx.prog_ids());
}
```

- The runner owns arm interleaving, the A/A noise-floor control and the
  provenance block (commit, dirty flag, methodology version,
  host state), so no scenario can skip them.
- The body runs once per replicate, so a per-replicate fresh fixture is
  structural.
- Named `run.span(...)` boundaries give stage timing (VM lifecycle).
- **Dual use:** the maintained lane is a crate's bench target; a spike is a
  `spike-scratch/` crate depending on the bench crate by path, using the same
  macro and record schema (marked as spike evidence). The macro may also emit a
  `#[test]` smoke variant (one replicate per arm, no statistics, no
  performance assertion, #354 placement) so a benchmark cannot rot between
  metal runs.

### Relationship to the shipped placement attribute (#354)

#354 landed on `main` (finalized 2026-11-16, `docs/evolution/2026-11-16-metal-test-placement-attribute.md`):
`#[overdrive_test(...)]`, `placement!`, `overdrive_test_default!` and
`shared_guest!` in `crates/overdrive-testing-macros`. The attribute writes a
record into the `overdrive_test_placement` ELF section that the Metal wrapper
reads; `cargo xtask placement-lint` checks every listed test before any guest
boots; groups and profiles are a closed registry checked at compile time
(ADR-0294); the attribute never inserts `#[test]` (ADR-0282).

Consequences for this proposal:

- **The smoke variant needs no new placement mechanism.** The generated smoke
  `#[test]` carries `#[overdrive_test(...)]` with the benchmark's placement and is
  linted like any other test.
- **The benchmark's own placement reuses the closed registry** (ADR-0294) rather
  than a second vocabulary. Whether the bench runner also writes and reads the
  ELF placement record, or only the smoke test does, is a DESIGN question.
- **A `harness = false` bench target is not in nextest's default listing**, so
  `placement-lint` does not see it. DESIGN decides whether the bench target
  needs its own placement check or whether only the smoke test is placed.
- ADR-0295 already allows `xtask` and `core` crates a dev-dependency on the
  placement macros; a bench macro crate would need its own ruling.

## User rulings (2026-11-16)

| # | Question | Ruling |
|---|---|---|
| 1 | Metal admission for benchmark runs | **Exclusive.** |
| 3 | May a benchmark launch the `overdrive` binary | **No.** Benchmarks drive the production composition in-process, as tests do. |
| 5 | Statistical defaults (≥ 9 replicates per arm, A/A control, canary, nonparametric A/B with bootstrap CIs) | **Accepted.** |
| 7 | First slice | **Connect-hook cost** (#287 comment, #54 P-41). |
| 2 | Host settings | **Run at the appliance's production settings; record them and refuse a drifted host.** `kernel.bpf_stats_enabled` stays **0** on the Metal host. A benchmark that needs BPF `run_time_ns`/`run_cnt` enables stats for its own process with the fd-scoped `BPF_ENABLE_STATS` command, which the kernel reverts when the process exits (`kernel/bpf/syscall.c:5990-6040 @ v7.0`), so nothing is toggled or restored and a crashed run cannot leave stats on. The result records whether stats were on. Ruled 2026-11-16 after the overhead spike (`spike/findings-bpf-stats-overhead.md`: +67 ns per program run, +0.48% on a two-hook loopback connect). |
| 4 | Measurement core location | **A new dev-dependency-only `adapter-host` crate, shaped like `overdrive-testing`.** |
| 6 | Substrate | **Each family declares its substrate, the same way the test placement macros do; the result records it; results from different substrates are never compared.** First slice runs native, for direct comparison with the spike. |
| 8 | Macro approach | **Approved**, reusing the #354 placement registry and `#[overdrive_test]` for the smoke variant. |
| 9 | Metal grammar | **Approved**: add a benchmark form to `cargo xtask metal run`, with benchmarks run exclusive (the `testing.md` exclusive rule gains a benchmark exemption alongside expectation captures). |
| 10 | Macro crate | **Delegated** ("whatever"). Proposed: extend `overdrive-testing-macros`, which already owns the placement parsing and the closed registry, so no new dev-dependency ruling is needed. DESIGN confirms. |

## Open decisions before DISCUSS

None. All ten are ruled above.
