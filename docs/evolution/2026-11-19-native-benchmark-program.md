# Evolution — A native benchmark program for Overdrive's primitives

**Feature:** `native-benchmark-program` ([GH #287](https://github.com/overdrive-sh/overdrive/issues/287)). **Finalized:** 2026-11-19.
**Scope:** developer verification tooling, one job (`J-PLAT-008`), nine stories in nine slices, seventeen implementation steps in four phases.
**Waves run:** DISCUSS (2026-11-16), DESIGN (2026-11-16 to 2026-11-18), DISTILL (2026-11-17), DELIVER (2026-11-17 to 2026-11-18). No DIVERGE: the alternatives are recorded in the research document, not in a comparison.
The [feature contract and wave history](../feature/native-benchmark-program/feature-delta.md), [roadmap](../feature/native-benchmark-program/deliver/roadmap.json) and [execution log](../feature/native-benchmark-program/deliver/execution-log.json) remain the delivery records.
Website documentation: [`website/content/docs/how-to/write-and-run-a-benchmark.mdx`](../../website/content/docs/how-to/write-and-run-a-benchmark.mdx), [`website/content/docs/concepts/benchmark-methodology.mdx`](../../website/content/docs/concepts/benchmark-methodology.mdx), and the benchmark exemption in `website/content/docs/how-to/run-verification-on-metal.mdx`. Blog post: [`website/content/blog/measuring-hooks-vms-dns-and-mtls-on-one-runner.mdx`](../../website/content/blog/measuring-hooks-vms-dns-and-mtls-on-one-runner.mdx). Documentation and blog review verdict: APPROVED in round 2 by an independent documentation reviewer, review artifact [`docs/feature/native-benchmark-program/finalize/review-website.md`](../feature/native-benchmark-program/finalize/review-website.md). Validation: `bun run typecheck`, `lint`, `build` (49 pages) and `assert:one-index` pass. `test:blog` was not run, because it builds and serves the Cloudflare Worker through `opennextjs-cloudflare` and `wrangler dev`.

## Summary

Overdrive now has one fixed way to measure what a boundary costs on the shared native Metal host. A developer declares a benchmark with `#[overdrive_bench]`, runs `cargo xtask metal run --benchmark <family>` under exclusive admission, and gets two retained files: an append-only `record.jsonl` with every sample, and a `report.md` derived from it. The runner interleaves the arms in seeded blocks, adds an A/A control of the reference arm, brackets the run with a canary, and compares arms with bootstrap intervals and a permutation Mann-Whitney test. It refuses a host that differs from a pinned settings list before it takes a sample. It enables BPF run statistics for its own process only, so `kernel.bpf_stats_enabled` stays 0.

Four families ship on that core:

- `cgroup-connect-hook`: the gateway `cgroup/connect4` program against no program.
- `vm-lifecycle`: eight host-observable stages and one guest-measured duration, over five profiles.
- `dns-responder`: client-observed latency beside the responder's own per-datagram service time, over four profiles.
- `mtls-path`: the host agent's mTLS hop against a plaintext baseline, egress and ingress, with whole-host CPU, memory, sockets and the egress forward and return pump split.

The same declaration runs from a `spike-scratch/` crate and produces a schema-identical record marked spike evidence. Each declaration also generates a smoke test that runs one replicate per arm in a guest and asserts nothing about speed. Regression gating and baseline promotion are not part of this feature.

## Business context

Before this feature, every native measurement was a spike that copied an earlier spike's harness. `bpf-stats-overhead` was a copy of `pig-connect-cost`, and each hand-wrote its sampler, recorder and report. #287 asked for one program that measures VM lifecycle, DNS and mTLS, keeps the raw samples, states the conditions and separates the stages, so that a decision between two mechanisms, such as the host proxy against in-guest kernel mTLS ([#303](https://github.com/overdrive-sh/overdrive/issues/303)), rests on numbers anyone can reproduce. The persona is the engineer, or the coding agent acting for them, who must choose between mechanisms or check what a primitive costs end to end.

## Key decisions

All fourteen DESIGN decisions (DDD-1 to DDD-14) were approved by the user on 2026-11-17 and independently reviewed (adversarial and subtractive, resolved the same day). The ADRs read Accepted. The ten DISCUSS rulings are in [intake.md](../feature/native-benchmark-program/intake.md).

| Decision | Outcome | ADR |
|---|---|---|
| DDD-1 | Two dev-only `adapter-host` crates: `overdrive-bench` (core) and `overdrive-bench-families` | [0296](../product/architecture/adr-0296-benchmarks-are-measured-by-a-dev-only-core-crate-with-families-in-a-separate-crate.md) |
| DDD-2 | `#[overdrive_bench]` in `overdrive-testing-macros`, which also emits the smoke test with its own placement | [0297](../product/architecture/adr-0297-a-benchmark-is-declared-with-an-attribute-that-also-emits-its-smoke-test.md) |
| DDD-3 | Declared benchmarks register at link time with `linkme` | [0298](../product/architecture/adr-0298-declared-benchmarks-register-at-link-time-in-a-distributed-slice.md) |
| DDD-4 | Closed substrate registry with one entry, `native` | [0299](../product/architecture/adr-0299-benchmark-substrates-are-a-closed-registry-and-the-first-entry-is-native.md) |
| DDD-5 | The runner owns the statistical protocol and versions it as the methodology | [0300](../product/architecture/adr-0300-the-runner-owns-the-statistical-protocol-and-versions-it-as-the-methodology.md) |
| DDD-6 | A result is an append-only JSONL record with a derived report, under `benchmarks/results/` | [0301](../product/architecture/adr-0301-a-benchmark-result-is-an-append-only-record-with-a-derived-report.md) |
| DDD-7 | Refuse a host that differs from the pinned settings, before any sample | [0302](../product/architecture/adr-0302-a-benchmark-refuses-a-host-that-differs-from-the-pinned-test-host-settings.md) |
| DDD-8 | BPF run statistics are enabled for the benchmark process only, through a descriptor | [0303](../product/architecture/adr-0303-bpf-run-statistics-are-enabled-for-the-benchmark-process-only.md) |
| DDD-9 | `metal run --benchmark` runs a closed command under exclusive admission | [0304](../product/architecture/adr-0304-the-metal-benchmark-form-runs-a-closed-command-under-exclusive-admission.md) |
| DDD-10 | Stage boundaries are stamped on the host. Guest stages are durations | [0305](../product/architecture/adr-0305-stage-boundaries-are-stamped-on-the-host-and-guest-stages-are-durations.md) |
| DDD-11 | VM-lifecycle stages come from existing production seams, with no new production API | [0306](../product/architecture/adr-0306-vm-lifecycle-stages-are-observed-through-existing-production-seams.md) |
| DDD-12 | The DNS responder reports per-query service time to a `QueryObserver` (production change) | [0307](../product/architecture/adr-0307-the-dns-responder-reports-per-query-service-time-to-an-observer.md) |
| DDD-13 | The mTLS family measures the host agent hop against a plaintext baseline | [0308](../product/architecture/adr-0308-the-mtls-benchmark-measures-the-host-agent-hop-against-a-plaintext-baseline.md) |
| DDD-14 | Outbound mTLS pump threads are named `mtls-out-fwd` and `mtls-out-ret` (production change) | [0309](../product/architecture/adr-0309-mtls-pump-threads-carry-their-role-in-their-name.md) |

ADR numbers 0296 to 0299 are each held by two files in `docs/product/architecture/`. The other four belong to `reconciler-convergence-guarantees`. This feature's ADRs are the ones named above. The collision is not fixed here.

Decisions taken during DESIGN review and DELIVER, all recorded in the feature delta's Changed Assumptions:

- **CA-1:** the smoke test carries its own placement, because a native benchmark may not be a `native_launcher` product test.
- **CA-2:** a guest reports one duration, shutdown handling. Receipt and poweroff appear only as host-stamped stage intervals. There is no host and guest clock-offset protocol.
- **CA-3:** the DNS profiles carry no cache, transport or response-size axes. The responder has no cache, serves only UDP, and its response size follows from the query shape. This is a property of the mechanism, not a scope limit.
- **CA-12:** fewer than 9 replicates is a build-time error naming the minimum, because `replicates` is a literal in the attribute.
- **CA-13** (user, 2026-11-18): the connect-hook arms became `no-hook` and `gateway-hook` (see Lessons).
- **CA-14** (user, 2026-11-18): a series metric may hold measured values and failures together, and the replicate stays `ok`.
- The existing `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials` test stays as it is and does not move onto this framework (user, 2026-11-17).
- The runner's end-to-end run stays out of nextest. Its evidence is a retained `--benchmark` run, and the nextest coverage is the smoke variant plus the record and report tests (user, 2026-11-17).

## Work completed per step

Source: `deliver/execution-log.json` and the `Step-Id` trailers in `git log`. Every step reached COMMIT with status PASS. DISTILL authored the acceptance tests and scaffolds before DELIVER (`8665798c8`), and the DISTILL run proved 67 of 70 tests RED with a named semantic reason.

| Step | What landed | Commit | RED |
|---|---|---|---|
| 01-01 | Record schema, writer and `bench-report validate` | `fbf1e8309` | executed |
| 01-02 | Statistics and `bench-report render` | `a7f722a27` | executed |
| 01-03 | `overdrive_bench` attribute, link-time registration, substrate registry, generated smoke test | `bf6433571` | executed |
| 01-04 | Runner: schedule, host eligibility, per-process BPF statistics, canary, retention | `dbdf79bf2` | not applicable: no pre-authored acceptance test exists (the runner's evidence is a retained run), and the pure internals were written together with their unit and proptest suites |
| 01-05 | `metal run --benchmark` in xtask and `bootstrap.sh` | `5ce6b508b` | not applicable: the pre-authored `should_panic` markers had already shown the semantic RED on HEAD (`unknown arg: --benchmark`), and were removed with the implementation |
| 01-06 | Connect-hook family, methodology, catalogue, retained runs E-1, E-2, E-3, E-8 | `d15bab5f7`, `dcce79fe8` | executed |
| 01-07 | `pig-connect-cost` re-expressed on the core, retained run E-4 | `ea3d2d1fb` | not applicable: evidence-only step, discharged by retained run E-4 |
| 01-08 | Connect-hook arms become `no-hook` and `gateway-hook`, reference cites the spike re-run | `05a77066b`, `d62130f8e`, `063cbce0d`, `8e47de74e`, `4b4f7e732`, `5e44ab6b8` | executed |
| 02-01 | `vm-lifecycle` host-observable stages and profiles | `6ef7014f0` | executed after a checkpoint (see Lessons) |
| 02-02 | Guest-measured shutdown handling, grace-deadline fixture, retained run E-5 | `1f4397fc7`, `ec52bab6d` | executed |
| 02-03 | A benchmark declares its service and guest clock sources | `f864e46fb` | executed |
| 02-04 | A partly failed series keeps its measured values | `2c7218d43` | executed |
| 03-01 | `DnsResponder` per-query observer (DDD-12) | `1645ecf91` | executed |
| 03-02 | `dns-responder` family, retained run E-6 | `96e4f2358` | not applicable: a new family's smoke test is emitted by the macro from the declaration, so it does not exist before the family |
| 04-01 | Named mTLS outbound pump threads (DDD-14) | `b15dfcca8` | executed |
| 04-02 | `mtls-path` rigs, plaintext baseline arm, latency and throughput measures | `b920ad460` | not applicable: the acceptance test is the macro-generated smoke test, which does not exist before the declaration |
| 04-03 | `mtls-path` resource cost and pump split, retained run E-7 | `73dd90f72`, `c5edf53b2` | not applicable: the pure `/proc` parsers and their unit tests were written together, and no acceptance test exists for the step |

Outside the steps: `873f9a1c2` recorded the Metal benchmark form and its exclusive exemption in `.claude/rules/testing.md`, and `a50ff5934` (named the lease holder on `checkout_timeout`, blocked metal-run misuse) landed on the branch beside this work.

## Evidence runs

The families have no nextest test beyond their generated smoke tests. Their evidence is retained `--benchmark` runs, taken on the native Metal host (AMD EPYC 8024P, 16 CPUs, kernel 7.0.0-38-generic, governor `schedutil`, SMT on, boost on, `bpf_stats_enabled` 0). Each run is a directory with `record.jsonl` and `report.md` under `benchmarks/results/<family>/<run-id>/`. Every report states `dirty: true`, so the recorded commit is the nearest base and not the exact tree. Intervals are 95% bootstrap intervals over per-replicate medians. "A/A" is the interval of the reference arm against its own control in the same run.

| Run | Directory | What it shows | Headline numbers, as measured |
|---|---|---|---|
| E-1 | [`cgroup-connect-hook/9e740a31…`](../../benchmarks/results/cgroup-connect-hook/9e740a312f48cddc18c56ee3/report.md) | The walking skeleton: 30 replicates per arm plus 30 A/A, 90 ok, 0 failed, canary steady, statistics on per process | Hook time on `gateway-hook`: median 137.4 ns per connect. Wall time per connect: `no-hook` 30,108.7 ns, `gateway-hook` 30,354.5 ns, delta 245.8 ns, interval [131.9, 332.9], A/A [-115.9, 89.3], verdict distinguishable. The hook time is absent on `no-hook` |
| E-2 | [`cgroup-connect-hook/05da8b43…`](../../benchmarks/results/cgroup-connect-hook/05da8b439057b1b516b28784/report.md) | A drifted host is refused before any sample. A second process held a `BPF_ENABLE_STATS` descriptor, which the sysctl cannot see | `bpf_stats_holder found held by pid 1205413 pinned none`. Exit 78. The record holds `run_started` and `run_refused` only. Receipts under `receipts/` |
| E-3 | [`cgroup-connect-hook/ae57c429…`](../../benchmarks/results/cgroup-connect-hook/ae57c4291bbc63794c1dc126/record.jsonl) | A killed run leaves no statistics enabled | SIGKILL after the first replicate left `run_started`, `canary`, `replicate` and no terminal line. `bench-report render` prints `status: incomplete (no run_finished or run_refused)`. The post-run probe read `kernel.bpf_stats_enabled=0`, 0 `bpf-stats` descriptor holders and 0 benchmark processes |
| E-4 | [`spike-scratch/pig-connect-cost/results/…/112c3104…`](../../spike-scratch/pig-connect-cost/results/cgroup-connect-hook/112c3104a22101479a180f5c/report.md) | The `pig-connect-cost` probe, re-expressed with the attribute, yields a schema-identical record marked spike evidence (K5). It is also the re-run the connect-hook family cites | Hook time on `gateway-hook`: median 137.6 ns. Wall-time delta 185.7 ns, interval [31.0, 334.0], A/A [-107.8, 212.0], verdict not distinguishable from noise. `bench-report validate` exits 0 |
| E-5 | [`vm-lifecycle/1759869c…`](../../benchmarks/results/vm-lifecycle/1759869cfd254906cd263e60/report.md) | Eight host-observable stages and one guest-measured duration, per profile, never merged. 150 ok, 0 failed, 30 replicates in each of five profiles | Profile `healthy-concurrency-1`, p50: admission 2.81 ms, VMM launch 14.1 ms, guest READY 1.280 s, Stable 1.017 s, stop submission 2.80 ms, shutdown delivery 64.5 µs, guest exit 61.5 ms, cleanup calls 0.61 ms. Guest shutdown handling p50 18 ms (1 ms resolution). With four VMs together, VMM launch p50 is 31.5 ms. `stop-grace-deadline`: guest exit p50 5.07 s, guest shutdown handling p50 5.024 s |
| E-6 | [`dns-responder/7db6b39d…`](../../benchmarks/results/dns-responder/7db6b39da5ddf46a646bdb21/report.md) | Client-observed and resolver-internal timings in separate tables. 120 ok, 0 failed, four profiles. Series above 1,000 values are stored as histograms, so percentiles carry at most 3.125% relative error | `a-record-1-name-1-client-2000qps`: client p50 38.8 µs, service p50 14.0 µs. `a-record-1000-names-4-clients-8000qps`: client p50 99.9 µs, service p50 77.0 µs, throughput 8,000 qps. `aaaa-no-data-100-names-1-client-2000qps`: 47.9 µs and 22.9 µs. `unknown-name-1-client-2000qps`: 40.0 µs and 15.6 µs. Every profile delivered its scheduled rate |
| E-7 | [`mtls-path/5bf66f2f…`](../../benchmarks/results/mtls-path/5bf66f2f594d8d8c1bd5676e/report.md) | Egress and ingress, each against a plaintext baseline. 180 ok, 0 failed, canary steady | See below |
| E-8 | [`cgroup-connect-hook/e-8-marker-refusal/`](../../benchmarks/results/cgroup-connect-hook/e-8-marker-refusal/) | A checkout with a missing or malformed marker refuses before any record exists | Both cases exit 78 with one stderr line: `bench: refused: .overdrive-metal-source: …`. No result directory and no stdout result line |

**Reading E-1 against E-4.** The two runs measure the same two arms and agree on the hook time (137.4 ns and 137.6 ns). They disagree on the wall-time verdict: E-1 says distinguishable, E-4 says not distinguishable from noise. The deltas (245.8 ns and 185.7 ns) have overlapping intervals, and the E-1 delta sits inside E-4's interval. The honest statement is a wall-time cost of a few hundred nanoseconds per 30 µs connect with a wide interval. Neither run settles whether it exceeds the noise floor. The hook time, from kernel run statistics, is the sharper figure. Under the K2 criteria the delta verdict is reported as observed, never retried.

**E-7, egress** (baseline `plaintext-baseline`, then `composed-mtls`):

- Connection establishment p50 18.8 µs to 28.8 µs, delta 10.1 µs, interval [9.1, 10.7], A/A [-0.5, 1.5].
- First byte p50 118 µs to 2.96 ms, delta 2.85 ms. Handshake p50 2.85 ms (absent on the baseline, never zero).
- Tail latency p50 17.4 µs to 64.6 µs, delta 48.8 µs, interval [40.5, 54.3].
- Throughput median 2.67 GB/s to 0.868 GB/s, delta -1.80 GB/s, interval [-1.98, -1.76].
- Whole-host CPU: user ratio delta 0.016 (distinguishable), system ratio not distinguishable from noise, softirq distinguishable. Memory delta 1.5 MB, not distinguishable. Sockets 6 to 11.
- Pump split: `forward_userspace_cpu_ratio` 1 and `return_splice_cpu_ratio` 0, with `pump_heavy_to_light_lower_bound` 30 (see Lessons).

**E-7, ingress** (guest transport excluded):

- Handshake p50 0.82 ms, first byte p50 71.8 µs to 948 µs, tail latency p50 27.6 µs to 38.2 µs, delta 9.2 µs, interval [7.9, 13.7].
- Throughput median 1.03 GB/s to 1.21 GB/s, delta +175 MB/s, interval [10.1, 220.5]. The composed arm measured higher. The baseline bridges to the same in-memory guest sink through different code from the enforcement path, so this is not a claim that mTLS is faster.
- Memory not distinguishable. The pump split is absent on both arms (`asynchronous relay on shared runtime threads`).

Earlier runs stay in the tree as history and are not the current evidence: `cgroup-connect-hook/c591ccdb…` (the two-program arms, hook-time delta 120.0 ns), `vm-lifecycle/ab04acf6…`, `dns-responder/52f2cb04…` and `adbbc5b7…`, `mtls-path/f4bf18f7…`, and `spike-scratch/pig-connect-cost/results/…/afda96dd…`.

## Lessons learned

- **A concurrent change removed one of the two arms.** Commit `cfcc67612` (from `reconciler-convergence-guarantees`) deleted the service connect program while this feature was in DELIVER. The dataplane boot now attaches only the gateway program, so the planned arms (`service-only`, `service+gateway`) no longer existed. We re-arranged the family around `no-hook` (the gateway link detached) and `gateway-hook`. The hook-time metric is recorded on `gateway-hook` only and is absent on `no-hook`, and the comparison is judged on wall time. The 117 ns figure of the first spike was measured against a two-program dataplane and is no longer comparable, so the family cites the spike re-run on the same arms (E-4). Step 01-08, CA-13.
- **The DNS service-time boundary moved to the instant the receive returns.** The responder read its clock after the datagram copy and the cmsg parse, so the first retained run started the interval later than DDD-12 says. Commit `1348c61de` reads the injected clock as soon as `recvmsg` returns. The same commit counts client send errors in the failure reason, so a refused send is told apart from a responder that did not answer. E-6 was re-taken after the fix.
- **The `mtls-path` smoke test runs in `cgroup-default`, and the mesh peer is on loopback.** The egress rig runs the real `MtlsInterceptWorker`, whose shared nft program uses the `fib` expression, and `nft_fib` modules are staged only for `cgroup-default` guests. On both profiles the TLS peer or client shares the host with the agent, so the relative overhead in E-7 is an upper bound and the absolute numbers are not pure agent cost.
- **A metric was named wrongly and a count was declared.** The family recorded connection establishment as `connect_ns`. The DISTILL-pinned name is `connection_establishment_ns`, and `c5edf53b2` renames it. `connection_concurrency` was the constant 1. It is now the peak number of live client sessions the family held open during the sustained window, kept by a guard that counts a session from open to drop. It still reads 1 in E-7 because the family opens one session at a time, and that figure is now a measurement.
- **Rewritten commits left three records naming unreachable commits.** The earlier part of the branch reached `main` as patch-equivalent commits with new ids, and the final rebase rewrote the rest. The DNS and mTLS runs were re-taken on reachable commits (records name `c5edf53b2` and `20d93006b`; evidence committed in `a3b4d8465` and `afc3f4170`), and the earlier directories stay as history. E-1, E-4 and E-5 were kept as taken. Their records name `8dabedfc3`, `5f78426f7` and `284f2a659`, which are not ancestors of `main`. The patch-equivalent commits on `main` are `063cbce0d`, `05a77066b` and `d078b90c3`. `crates/overdrive-bench` and `crates/overdrive-bench-families` are byte-identical between each pair. The 16 other files that differ belong to the concurrent reconciler-convergence work (reconcilers, sim adapters, observation store), none on the connect-hook path. This mapping was accepted as the provenance of the three records on 2026-11-19, in place of re-running them.
- **A zero reads as a bound, not as absence.** In E-7 the egress return-pump ratio is 0. The sustained window carries the 256 MiB frame forward only, so the return pump does almost no work, and per-thread CPU is sampled on a 10 ms tick. A zero means below one tick, and the heavy-to-light ratio (30) is a lower bound. The profile text in the report says so.
- **A falsified boundary changes the stage table, not only the fixture.** V-10 showed the guest `EXIT` line reaches the host in 0 of 60 operator stops. DDD-11 stages 6 and 7 ended and started at that line. Step 02-01 stopped at a checkpoint, and `67baac0e9` moved the two stages to the `SHUTDOWN` delivery and the VMM reap before the step resumed. Running the contract-shaping probes in DESIGN caught V-4 the same way, but V-10 was classed as not contract-shaping and cost a pause in DELIVER.
- **Unit tests over crafted traces found a deadlock.** The `vm-lifecycle` Stable-row lookup took the trace mutex twice in one expression, so a Stable row that never arrived hung the replicate instead of failing the stage (`d078b90c3`).
- **The report shows preparation text, not record notes.** The mTLS caveats (whole-host CPU scope, tick-sampled irq and softirq, the pump split as a lower bound, the ingress handshake including the wait for the `ClientHello`, the co-located peer) sit in each profile's preparation text, once, because that is the only text that reaches `report.md`.
- **A design rule hid a data-loss path.** The record rule that a metric entry holds exactly one of values, histogram, failures or absent meant a failed contribution to a series discarded the measured ones. CA-14 and step 02-04 allow measured values and failures together.

## Issues encountered

- Step 01-08 COMMIT first logged FAIL, then CHECKPOINT_PENDING. The catalogue test still asserted the old arms and needed the acceptance designer to update it. It closed PASS in `4b4f7e732`.
- Step 02-01 was blocked on V-10 (above) and re-ran the same evening.
- Step 04-03 COMMIT was held until the retained E-7 run completed.
- The E-3 capture's first attempt reused E-2's run id, and the runner refused to reuse the result directory (exit 70). The second attempt used its own id. Both receipts are kept.
- A mutation-testing run was not made. The user ruled it out for this feature, so no kill rate is claimed.

## Known residuals

- **Known pre-existing defect, left unfixed.** `establish` in `crates/overdrive-dataplane/src/mtls/outbound.rs` leaks leg B's file descriptor on an early kTLS-arm or timeout error. DDD-14 touches that function for thread names only, and the defect predates this feature.
- E-1, E-4 and E-5 name commits not on `main`; the patch-equivalent `main` commits are listed above.
- Every retained report carries `dirty: true`, so the commit field is a base and not an exact tree.
- No full-suite nextest comparison was made for this record. The DELIVER evidence is the per-step runs and the retained benchmark runs above.

## Scope limits and deferrals

Each issue below was read with `gh issue view <N>` on 2026-11-19 (body and comments). Every one is open and covers the item named.

- [#362](https://github.com/overdrive-sh/overdrive/issues/362): the substrate registry is closed to `native`. A guest substrate for components that run inside a workload VM is tracked there. No command compares records across substrates, because none takes two records (CA-10).
- [#361](https://github.com/overdrive-sh/overdrive/issues/361): baseline promotion and the regression-gate policy. This feature compares nothing against an earlier run and fails nothing.
- [#288](https://github.com/overdrive-sh/overdrive/issues/288) and [#289](https://github.com/overdrive-sh/overdrive/issues/289): point-in-time verification expectations and the black-box E2E runner are separate artifacts. A benchmark is neither.
- [#157](https://github.com/overdrive-sh/overdrive/issues/157): the XDP perf-baseline helper and the Tier-4 `xdp-perf` gate are unchanged.
- [#303](https://github.com/overdrive-sh/overdrive/issues/303): the mTLS family measures the ADR-0069 host agent hop, without the guest transport. In-guest kernel mTLS replaces that hop, and #303 names #287 as the way to compare the two. The ingress exclusion of the guest transport and the absent ingress forward and return split (CA-6, CA-8) are properties of the host-hop benchmark and have no issue of their own.
- No cache, transport or response-size axes in the DNS profiles (CA-3): a property of the responder, not a limit.
- Guest receipt and poweroff have no guest-clock duration (CA-2): a clock boundary, not a deferral.

## Verification expectations (EDD)

None authored. The retained benchmark runs are not EDD expectations. They answer how fast and how variable, and `.claude/rules/verification.md` § "Boundary" keeps those apart from point-in-time feature receipts.

## Permanent artifacts and rule changes

- ADRs 0296 to 0309 (benchmark side) in `docs/product/architecture/`, linked above.
- [`benchmarks/catalogue.md`](../../benchmarks/catalogue.md): one row per benchmark with its workload matrix, clock boundary and result owner.
- [`benchmarks/methodology.md`](../../benchmarks/methodology.md): methodology version 1.
- Retained results: [`benchmarks/results/`](../../benchmarks/results/) (one directory per run) and the spike runs under `spike-scratch/pig-connect-cost/results/`.
- Code: `crates/overdrive-bench/`, `crates/overdrive-bench-families/`, the `overdrive_bench` attribute in `crates/overdrive-testing-macros/`, the benchmark form in `xtask/src/main.rs` and `infra/metal/bootstrap.sh`, the DNS `QueryObserver` and the named pump threads (the two production changes).
- Rule change: `.claude/rules/testing.md` records the benchmark form of `metal run` and the benchmark exemption beside the expectation-capture one for the "no test runs `--exclusive`" rule (`873f9a1c2`).
- Website documentation: [`website/content/docs/how-to/write-and-run-a-benchmark.mdx`](../../website/content/docs/how-to/write-and-run-a-benchmark.mdx), [`website/content/docs/concepts/benchmark-methodology.mdx`](../../website/content/docs/concepts/benchmark-methodology.mdx), and the benchmark exemption in `website/content/docs/how-to/run-verification-on-metal.mdx`. Blog post: [`website/content/blog/measuring-hooks-vms-dns-and-mtls-on-one-runner.mdx`](../../website/content/blog/measuring-hooks-vms-dns-and-mtls-on-one-runner.mdx).

## Migration of lasting artifacts

This feature uses the lean feature-delta layout, so none of the destination-map sources exist: no `design/architecture-design.md`, `component-boundaries.md`, `technology-stack.md` or `data-models.md`, no `design/adrs/`, no `distill/walking-skeleton.md`, no `discuss/journey-*`. The ADRs, catalogue, methodology and results already sit at permanent homes and were not copied. The feature directory [`docs/feature/native-benchmark-program/`](../feature/native-benchmark-program/) keeps the wave history, the spike findings (V-1 to V-5, V-10 and the `bpf_stats` overhead probe) and the design reviews as delivery history.

## Metal box cleanup

After every retained record had been pulled back, the feature's remote checkout `/home/ubuntu/overdrive-worktrees/d6e116fa0e342e06ad0b58474ab66e6f68a1cc611c87fc78ba4451a071a928c7` (checkout, build and scratch output, 99 GB) was removed with an exclusive Metal command on 2026-11-19 (run `b3f35b0cabc3b24ce3116e07`). Root filesystem use went from 527 GB to 429 GB. The durable run records under `/var/lib/overdrive-metal/runs/` are kept, as the runner intends.
