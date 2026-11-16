<!-- markdownlint-disable MD024 -->
# Feature delta — `native-benchmark-program`

DISCUSS, 2026-11-16. Infrastructure (developer tooling), brownfield. Density `lean`, Tier-1 `[REF]` sections only. One job (`J-PLAT-008`), nine stories in nine slices; the first four slices are the walking skeleton (connect-hook cost). Anchor: [GitHub issue #287](https://github.com/overdrive-sh/overdrive/issues/287). Inputs: `intake.md` (ten user rulings), `docs/research/testing/native-benchmark-program-best-practices-research.md`, `spike/findings-bpf-stats-overhead.md`, `docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md`. Requirements only: DESIGN owns every interface shape; this document names behaviours and observable outputs.

## Wave: DISCUSS / [REF] Persona ID

`marcus-platform-verification-engineer` — Marcus, the Overdrive engineer who decides between mechanisms or checks a primitive's cost; coding agents run the same commands for him, so every pitch is also what an agent sees. Extended with a trigger and a characteristic for this feature. [Persona](../../product/personas/marcus-platform-verification-engineer.yaml).

## Wave: DISCUSS / [REF] JTBD one-liner

`J-PLAT-008`: When I must choose between two mechanisms or check what a primitive costs end to end, I want to declare the boundary and its arms once and run it on the shared native host under one fixed method, so I can decide from retained raw samples and a report with distributions and host state that anyone can reproduce and compare. [Job register](../../product/jobs.yaml). Journey: [measure a boundary or compare two mechanisms natively](../../product/journeys/measure-a-boundary-natively.yaml).

## Wave: DISCUSS / [REF] Premise and evidence

| Fact | Source |
|---|---|
| No general native benchmark exists. Tier-4 `xdp-perf` is XDP-only and gates a mean; correctness tests, expectations and E2E tests are explicitly not benchmarks. | #287; `.claude/rules/testing.md` § "Classify external execution" |
| Spikes already measure well (interleaved A/B, fresh fixture per run, raw rows kept) but each hand-writes sampler, recorder and report; `bpf-stats-overhead` is a copy of `pig-connect-cost`. | `spike-scratch/pig-connect-cost/`, `spike-scratch/bpf-stats-overhead/`; research F1.7 |
| First data point exists: with both production connect hooks attached, the gateway program adds a median 117.0 ns hook time per connect (arm A 108.2 ns, arm B 225.2 ns) on a 28 µs loopback connect-and-close; the wall delta (about 168 ns) is inside run noise. | `findings-connect-hook-cost.md` |
| The measurement itself disturbs a short program: `bpf_stats_enabled=1` adds +67 ns per program run (+94% on a 71 ns XDP run, +0.48% on a connect). | `findings-bpf-stats-overhead.md` |
| The production path exists: the gateway connect program is in tree and attached by the production dataplane composition beside the service program. | `crates/overdrive-bpf/src/programs/cgroup_connect4_gateway.rs`; `crates/overdrive-dataplane/src/lib.rs` |
| VM lifecycle, DNS responder and transparent-mTLS paths exist in production composition (#283 closed with a stage list; DNS responder ADR-0072; mTLS ADR-0069). DESIGN must name, per family, the production entry point and the observable boundaries; where a stage has no observable boundary today, that is a DESIGN gap, not something DISCUSS invents. | #287; `.claude/rules/reconcilers.md` precedent notes |

Ground-the-premise check (`development.md`): the connect-hook benchmark's arms (service only; service plus gateway) are both reachable on the production path: the production composition attaches both. Arm A (gateway detached) is a benchmark configuration of a real component, not a production state; it is the A side of an A/B comparison and is labelled as such in the record. The mTLS/proxy family measures a boundary (plaintext baseline against the composed mTLS path), so it survives the planned move from the host proxy to in-guest kernel mTLS (#303).

DIVERGE artifacts are absent; the approach comes from #287 and the rulings. Noted risk: the alternatives (a criterion-style harness; per-spike scripts) are recorded in the research, not in a DIVERGE comparison.

## Wave: DISCUSS / [REF] Locked decisions

| ID | Decision | Source |
|---|---|---|
| D1 | Feature type: infrastructure / developer tooling, brownfield. UX research depth: lightweight. JTBD: yes (`J-PLAT-008`). Walking skeleton: yes, the connect-hook benchmark, serving both a maintained lane and a `spike-scratch/` probe from the first slice. | User, 2026-11-16 (dispatch) |
| D2 | Benchmark runs take **exclusive** Metal admission. | Ruling 1 |
| D3 | Run at the appliance's production host settings; record them and refuse a drifted host. `kernel.bpf_stats_enabled` stays 0; a benchmark needing BPF `run_time_ns`/`run_cnt` enables stats for its own process (fd-scoped `BPF_ENABLE_STATS`), which the kernel reverts at process exit; the result records whether stats were on. | Ruling 2 |
| D4 | A benchmark never launches the `overdrive` binary; it drives the production composition in-process, as tests do. | Ruling 3 |
| D5 | The measurement core is a new dev-dependency-only `adapter-host` crate, shaped like `overdrive-testing`. | Ruling 4 |
| D6 | Statistical defaults accepted: at least 9 replicates per arm, A/A noise-floor control, canary, nonparametric A/B comparison with bootstrap confidence intervals. | Ruling 5 |
| D7 | Each family declares its substrate the way the placement macros do; the result records it; results from different substrates are never compared. The first slice runs native, for direct comparison with the spike. | Ruling 6 |
| D8 | First slice: connect-hook cost (#287 comment, #54 P-41). | Ruling 7 |
| D9 | Benchmarks are declared with an attribute macro reusing the placement registry; the generated smoke variant is `#[overdrive_test]`-placed. | Ruling 8 |
| D10 | A benchmark form is added to `cargo xtask metal run`; benchmarks run exclusive; `testing.md`'s "no test runs `--exclusive`" rule gains a benchmark exemption beside the expectation-capture one. | Ruling 9 |
| D11 | Macro crate: delegated. Proposed: extend `overdrive-testing-macros`; DESIGN confirms. | Ruling 10 |
| D12 | Out of the program: regression gating and baseline promotion (#287 proposes them separately), the XDP baseline helper (#157), and correctness expectations (#288) and E2E tests (#289). | #287 non-goals and ACs |
| D13 | Names describe behaviour, never roadmap steps or issue numbers in identifiers (`domain-naming.md`). | Project rule |

## Wave: DISCUSS / [REF] User stories with elevator pitches

All stories: `job_id: J-PLAT-008`, persona Marcus. The benchmark form of `cargo xtask metal run` is written "the benchmark form"; its spelling is DESIGN's.

### US-BENCH-01 — Declare a benchmark and get retained raw samples and a report (walking skeleton)

Slice: [01](slices/slice-01-declare-and-run-a-benchmark.md). Must-have.

Marcus wants the per-connect cost of the gateway hook again. Today he copies a spike crate and hand-writes the sampler, the recorder and the report.

### Elevator Pitch
Before: a cgroup connect-hook cost is measured by copying a spike crate and re-implementing its sampler, recorder and report, and the output is a stdout table.
After: run the benchmark form of `cargo xtask metal run` for family `cgroup-connect-hook` → sees the run id, then a retained machine-readable record holding every sample of both arms and a report with per-arm median, IQR and replicate count, the substrate and the commit with its dirty flag.
Decision enabled: Marcus decides the declared benchmark is the way to take this number and stops writing per-spike harnesses.

Domain examples:
1. Marcus declares `connect_hook_cost` with arms `service-only` and `service+gateway`; the run records 30 replicates per arm, each on a fresh dataplane, and the report lists arm A median hook time near 108 ns and arm B near 225 ns.
2. A replicate fails (the cgroup cannot be created); it is counted as a failure in the record and the report, and the other replicates remain.
3. Marcus runs the same declaration twice; the two records carry different run ids and identical methodology versions.
4. Marcus names a substrate the registry does not know (`"guest-xyz"`); the build fails naming it.

```gherkin
Scenario: A declared benchmark runs on the exclusive native host and retains raw samples
  Given Marcus declared the connect-hook benchmark with two arms and native substrate
  When he runs the benchmark form for family cgroup-connect-hook
  Then every sample of every replicate of both arms is retained in a machine-readable record
  And a report shows per-arm median, IQR and replicate count with the substrate and the commit

Scenario: A failed replicate is counted, never dropped
  Given one replicate cannot create its fixture
  When the run completes
  Then the record and the report list the failure with its replicate and arm
  And the remaining replicates are still reported

Scenario: The run holds the host exclusively
  Given another worktree has an ordinary Metal run in progress
  When Marcus starts a benchmark
  Then the benchmark waits for exclusive admission and never overlaps an ordinary run

Scenario: A benchmark does not start the overdrive binary
  Given the connect-hook benchmark ran
  Then its retained command log shows no launch of the overdrive binary

Scenario: An unknown substrate is rejected by name
  Given a benchmark declares an unregistered substrate
  When Marcus builds it
  Then the failure names the substrate
```

AC: (a) a declared benchmark runs through the benchmark form on the exclusive host and retains all samples plus a report; (b) a failed replicate is counted and reported; (c) exclusive admission, with the `testing.md` exemption recorded; (d) no launch of the `overdrive` binary; (e) unknown substrate rejected by name; (f) the catalogue has a row for the family naming its workload matrix, clock boundary and result owner; (g) the record carries the methodology version. Notes: the retained result is brought back with `cargo xtask metal pull`; the methodology document is versioned (US-BENCH-02).

### US-BENCH-02 — Compare two arms under the fixed statistical protocol (walking skeleton)

Slice: [02](slices/slice-02-ab-comparison-protocol.md). Must-have.

Marcus needs to know whether arm B's cost is real or noise. The report must separate the effect from the run-to-run floor and place it beside the spike's number.

### Elevator Pitch
Before: the spike reported two medians and a hand-computed delta, and its wall-time delta (about 168 ns) was not distinguishable from noise by inspection.
After: run the benchmark form for `cgroup-connect-hook` → sees a report with the B-minus-A hook-time delta and its 95% bootstrap interval, the A/A noise-floor interval, the canary result, the replicates per arm (at least 9), and the spike's 117 ns shown as a reference point.
Decision enabled: Marcus decides whether the gateway hook's added cost is an effect or noise, and can state it in one line with its interval.

Domain examples:
1. The report shows hook-time delta of about 117 ns with an interval that excludes zero, an A/A interval around zero, and the spike's 117.0 ns beside it.
2. The wall-time delta of about 168 ns has an interval that includes the A/A floor; the report labels it "not distinguishable from noise" rather than a difference.
3. A declaration asks for 5 replicates per arm; the run is refused (the protocol requires at least 9).
4. The canary shifts between the start and end of the run; the report flags the run as unstable, and the record keeps every sample.

```gherkin
Scenario: Arms are interleaved and an A/A control is taken
  Given a two-arm benchmark with 30 replicates per arm
  When the benchmark runs
  Then replicate order alternates between arms and an A/A control of the same arm is recorded in the same run

Scenario: The delta is reported with an interval beside the noise floor
  When the run completes
  Then the report states B minus A with a bootstrap interval, the A/A interval, and a nonparametric test result

Scenario: A difference inside the noise floor is labelled as noise
  Given the wall-time delta interval includes the A/A interval
  Then the report says the difference is not distinguishable from noise

Scenario: Too few replicates are refused
  Given a declaration asks for 5 replicates per arm
  When Marcus starts the run
  Then it is refused naming the minimum of 9

Scenario: A shifting canary flags the run
  Given the canary measurement differs between the start and the end of the run
  Then the report flags the run as unstable and the record keeps every sample

Scenario: The spike's first data point is shown as a reference
  Then the report places the earlier 117 ns hook-time delta beside the new one, labelled as spike evidence from a different run
```

AC: (a) interleaved arm order and A/A control in one run; (b) delta with bootstrap interval, noise-floor interval and a nonparametric result in the report; (c) a delta inside the floor is labelled noise; (d) fewer than 9 replicates refused; (e) canary shift flags the run; (f) the spike reference is shown and labelled; (g) the methodology document (runner isolation, host eligibility, clock boundaries, cold/warm handling, sample and percentile method, raw-result format, failure accounting) exists at a named version (#287 AC 1). Notes: ruling 5; replicates are independent runs, not iterations inside a run (research F1.1).

### US-BENCH-03 — Record the host's production settings and refuse a drifted host (walking skeleton)

Slice: [03](slices/slice-03-host-settings-and-bpf-stats.md). Must-have.

Marcus must trust that two results were taken on a host in the same state, and that measuring did not change the node.

### Elevator Pitch
Before: the spike recorded governor, SMT and sysctl by hand, flipped `kernel.bpf_stats_enabled` for its whole run with a restore guard, and a crash could leave it on.
After: run the benchmark form for `cgroup-connect-hook` on a host whose setting differs from the production settings → sees a refusal before any sample naming the setting with both values; on a conforming host the report's host block lists the settings and `bpf stats: on (per process)`, and `sysctl kernel.bpf_stats_enabled` still reads 0 afterwards.
Decision enabled: Marcus decides whether a result is comparable with another from the recorded host state, and trusts that no run left the node altered.

Domain examples:
1. The host reads `bpf_stats_enabled` 1 before the run; the run refuses naming `kernel.bpf_stats_enabled` (found 1, production 0).
2. A conforming run records kernel `7.0.0-38-generic`, governor `schedutil`, SMT on, boost on, and `bpf_stats_enabled` 0, and reports hook times with `bpf stats: on (per process)`.
3. The benchmark process is killed mid-run; `sysctl` still reads 0 and no stats remain enabled once the process is gone.

```gherkin
Scenario: A drifted host is refused before any sample
  Given a host setting differs from the production settings
  When Marcus starts a benchmark
  Then the run refuses naming the setting with its found and production values
  And no sample is taken

Scenario: Host state is recorded with the result
  Given a conforming host
  When a benchmark completes
  Then the record and the report list kernel version, CPU model and count, governor, SMT, boost and bpf_stats_enabled

Scenario: BPF statistics are enabled for the benchmark process only
  Given a benchmark that needs per-program run time
  When it runs
  Then kernel.bpf_stats_enabled reads 0 before, during and after the run
  And the record states that statistics were on for that process

Scenario: A killed run leaves no statistics enabled
  Given the benchmark process is killed mid-run
  Then no BPF statistics remain enabled and the host settings are unchanged

Scenario: A benchmark that needs no BPF statistics records them as off
  Then the record states statistics were off
```

AC: (a) refusal before any sample naming setting and values; (b) host block in record and report; (c) `bpf_stats_enabled` never changed, stats scoped to the process; (d) a killed run leaves nothing enabled; (e) the result records whether stats were on. Notes: the production-settings reference set is an open point (O1).

### US-BENCH-04 — Use the same declaration and record from a spike probe, and keep the benchmark from rotting (walking skeleton)

Slice: [04](slices/slice-04-spike-use-and-smoke-variant.md). Must-have.

Marcus runs a throwaway probe under `spike-scratch/` and wants evidence in the same record schema as the maintained lane, without copying a harness. He also wants the maintained benchmark to keep compiling and running between Metal runs.

### Elevator Pitch
Before: each spike is a copy of an earlier spike's harness, and a maintained benchmark would only be exercised on a Metal run.
After: run a `spike-scratch/` crate's run (it depends on the benchmark crate by path and carries the same attribute) on the benchmark form → sees a record in the same schema marked `spike evidence` and the same report; run `cargo nextest run` on the benchmark's crate → sees a smoke test PASS that took one replicate per arm and asserts nothing about speed.
Decision enabled: Marcus decides a probe is enough for a design decision, with the record still comparable to the maintained lane, and trusts a benchmark that fails to compile or run is caught by the ordinary test lane.

Domain examples:
1. The existing `pig-connect-cost` probe is re-expressed with the attribute; its record validates against the same schema and the report renders it labelled spike evidence.
2. A probe's record and the maintained lane's record for the same family are shown side by side by their shared schema; the report refuses to compare them if the substrates differ.
3. The smoke test of the connect-hook benchmark runs one replicate per arm and passes; a deliberately broken fixture makes it fail in the ordinary lane.
4. The smoke test carries a placement declaration and `placement-lint` accepts it.

```gherkin
Scenario: A spike crate produces the same record schema
  Given a crate under spike-scratch depends on the benchmark crate by path and declares a benchmark
  When Marcus runs it through the benchmark form
  Then the record validates against the same schema and is marked as spike evidence

Scenario: Results from different substrates are never compared
  Given a maintained record on the native substrate and a record on another substrate
  When Marcus asks the report to compare them
  Then it refuses and names both substrates

Scenario: The smoke variant runs one replicate per arm with no performance assertion
  When the ordinary test lane runs the benchmark crate
  Then the smoke test passes after one replicate per arm and asserts nothing about timing

Scenario: A broken benchmark fails in the ordinary lane
  Given the benchmark's fixture no longer builds a dataplane
  When the ordinary test lane runs
  Then the smoke test fails naming the benchmark

Scenario: The smoke test is placed like any other test
  Then cargo xtask placement-lint accepts it
```

AC: (a) a `spike-scratch/` crate using the same attribute produces a schema-identical record marked spike evidence, with no change under `crates/` for the probe; (b) cross-substrate comparison refused by name; (c) smoke variant: one replicate per arm, no performance assertion, `#[overdrive_test]`-placed and lint-clean; (d) a broken benchmark fails the smoke test. Notes: `spike.md` isolation holds; the spike's Cargo.toml depends on workspace crates by path only.

### US-BENCH-05 — Report each VM-lifecycle stage separately

Slice: [05](slices/slice-05-vm-lifecycle-host-stages.md). Must-have (#287 AC 4).

Marcus needs to know where deploy and stop time goes, not one end-to-end number (#283 lacked it). This story covers the stages a host process can observe with one host clock.

### Elevator Pitch
Before: VM lifecycle latency is one wall number from a test or a single trace; writer success, guest READY and Service Stable are easy to confuse.
After: run the benchmark form for family `vm-lifecycle` → sees a report with one row per host-observable stage (admission/queue delay, VMM launch, guest READY as received by the host, workload `Stable`, stop request submission, workload exit, VMM reaper exit, cleanup), each with p50/p90/p95/p99 and the sample and failure counts, and a header naming cache state, VM resources, concurrency and Cloud Hypervisor and kernel versions.
Decision enabled: Marcus decides which stage to attack first and whether a change moved the stage it targeted.

Domain examples:
1. A run with 1 VM at a time and a warm image cache shows each stage's distribution separately and no stage called "ready" that merges READY and Stable.
2. A deploy where the guest never reports READY is a failure in the READY row, counted, with the later stages absent for that replicate.
3. A stop that reaches the grace deadline is kept in a separate failure/grace profile, never mixed into the healthy-path distribution.
4. A run at concurrency 4 records the concurrency in the header and is a distinct profile from concurrency 1.

```gherkin
Scenario: Each host-observable stage is reported separately
  When the VM-lifecycle benchmark completes
  Then the report has one row per stage with p50, p90, p95 and p99, sample count and failure count

Scenario: READY and Stable are never merged
  Then no row represents guest READY, writer success or Stable as a broader completion signal

Scenario: A stage that never completes is a counted failure
  Given a guest that never reports READY
  Then that replicate is a failure in the READY row and later stages are absent for it

Scenario: Healthy-path and grace-deadline stops are distinct profiles
  Given a stop that waits for its grace deadline
  Then it is reported in a separate profile from healthy stops

Scenario: The header names the run's conditions
  Then the report records image cache state, VM resources, concurrency, Cloud Hypervisor version and kernel version
```

AC: (a) each listed host-observable stage reported separately with the four percentiles, counts and failures; (b) no merged completion signal; (c) failures counted per stage; (d) healthy and grace/failure profiles distinct; (e) header lists cache state, resources, concurrency, VMM and kernel versions; (f) all stage timestamps taken by one host process on one host clock; (g) drives the production composition in-process (D4). Notes: each stage needs an observable boundary on a production surface; a missing boundary is a DESIGN gap (O3).

### US-BENCH-06 — Report guest-measured stages and cold versus warm cache for VM lifecycle

Slice: [06](slices/slice-06-vm-lifecycle-guest-stages.md). Must-have (#287 AC 4).

The guest's clock has a different zero point from the host's, so guest-internal stages cannot be subtracted from host instants.

### Elevator Pitch
Before: guest receipt and poweroff time cannot be placed against host stages without an unstated clock assumption, and cold versus warm image cache is an accident of run order.
After: run the benchmark form for `vm-lifecycle` with the cold and the warm profile → sees guest receipt, workload exit and poweroff as guest-reported durations, each next to the host-stamped stage it sits in, and separate cold-cache and warm-cache result sets, each naming its preparation step.
Decision enabled: Marcus decides whether shutdown delay is host-side or guest-side and whether the image cache matters.

Domain examples:
1. Poweroff is a duration measured inside the guest and reported as a duration; the report never subtracts a guest instant from a host instant.
2. The cold profile records "page cache dropped before each replicate"; the warm profile records "image pre-read".
3. A guest that does not return a duration reports the stage as absent, not zero.

```gherkin
Scenario: Guest stages are reported as durations
  Then guest receipt and poweroff appear as guest-measured durations, never as host instants

Scenario: Cold and warm are separate profiles with named preparation
  When both profiles run
  Then each result set names its preparation step and is never merged with the other

Scenario: A missing guest duration is absent, not zero
  Given a guest that returns no duration for a stage
  Then the stage is reported as absent for that replicate

Scenario: Clock provenance is recorded
  Then the record names the host and guest clock sources
```

AC: (a) guest-internal stages reported as guest-measured durations; (b) cold and warm profiles separate, each with its recorded preparation; (c) absent is not zero; (d) clock provenance (host and guest clock sources) recorded. Notes: research F1.9; the guest's actual clocksource is not settled (research Knowledge Gap 7).

### US-BENCH-07 — Report DNS responder timings: client-observed apart from resolver-internal

Slice: [07](slices/slice-07-dns-responder.md). Must-have (#287 AC 5).

### Elevator Pitch
Before: DNS responder cost has no number; a client-side timing would include everything and say nothing about the resolver's own work.
After: run the benchmark form for family `dns-responder` → sees a report with client-observed latency and throughput per profile (query shape, cache hit or miss, record cardinality, transport, client concurrency, response size) and, separately, resolver-internal timing, with the profile named in every row.
Decision enabled: Marcus decides whether the responder or the network path dominates and which query profile to optimise.

Domain examples:
1. Profile "A record, cache hit, 100 names, UDP, 1 client, small response" and profile "A record, cache miss, 10,000 names, UDP, 16 clients" are two result sets.
2. Client-observed p99 is reported in one table and resolver-internal time in another; neither is a column of the other.
3. A load generator that waits for each reply is not used for tail latency; the benchmark's load is scheduled-arrival (research F1.6) and the report states the arrival model.

```gherkin
Scenario: Client-observed and resolver-internal timings are separate
  Then the report has distinct tables for client-observed results and resolver-internal timing

Scenario: Profiles are named and distinct
  Then each result set names query shape, cache state, cardinality, transport, concurrency and response size

Scenario: The arrival model is stated
  Then the report states whether load was scheduled-arrival or closed-loop

Scenario: A failed query is counted
  Given a query times out
  Then it is counted as a failure in its profile and never as a latency sample
```

AC: (a) client-observed and resolver-internal reported separately; (b) profile named per result set; (c) arrival model stated; (d) failures counted; (e) the fixture conditions (host, kernel, network fixture) recorded; (f) the family has a catalogue row. Notes: drives the production responder in-process (D4).

### US-BENCH-08 — Compare plaintext and composed mTLS for connection cost and throughput, both directions

Slice: [08](slices/slice-08-mtls-latency-and-throughput.md). Must-have (#287 AC 6).

### Elevator Pitch
Before: the mTLS and proxy path has no number against a plaintext baseline, and the one handshake figure on record is a single sample the spike itself calls "not a benchmark".
After: run the benchmark form for family `mtls-path` → sees, for each traffic direction, plaintext-baseline arm against composed-mTLS arm: handshake, first-byte latency, sustained throughput and tail latency, with the interval of the difference and the noise floor.
Decision enabled: Marcus decides what the transparent mTLS path costs per direction and whether it meets what the platform claims.

Domain examples:
1. Egress (workload to peer) and ingress (peer to workload) are two result sets, each with its own baseline.
2. The plaintext baseline is a defined direct path between the same two endpoints; the report states its definition.
3. Handshake time and first-byte latency are separate rows.

```gherkin
Scenario: Both directions are reported
  Then the report has a plaintext-baseline arm and a composed-mTLS arm for each direction

Scenario: Handshake, first byte, throughput and tail are separate
  Then connection establishment, first-byte latency, sustained throughput and tail latency are separate rows

Scenario: The baseline is defined
  Then the report states what the plaintext baseline is

Scenario: A direction's difference carries its noise floor
  Then each direction's difference shows its interval and its A/A floor
```

AC: (a) both directions, each with a baseline; (b) the four measures separate; (c) baseline defined in the report; (d) interval and A/A floor per direction; (e) wire-capture correctness stays a Tier-3 test, not part of this benchmark. Notes: the egress dialer is plaintext by design (`CLAUDE.md` § East-west mTLS tests); the benchmark client follows it; drives the production composition in-process (D4).

### US-BENCH-09 — Report resource cost and the forward-versus-return work split for mTLS

Slice: [09](slices/slice-09-mtls-resources-and-split.md). Must-have (#287 AC 3 and 6).

### Elevator Pitch
Before: the claim that the proxy is "agent-light" (little userspace work on the forward path, splice on the return) has no measurement behind it.
After: run the benchmark form for `mtls-path` resource profile → sees CPU (whole-host, with the kernel and userspace split), memory, socket count and connection concurrency per arm and direction, plus forward-path userspace work reported apart from return-path splice work.
Decision enabled: Marcus decides whether the agent-light claim holds and where a future mechanism (for example an in-guest kernel path) must beat the current one.

Domain examples:
1. Whole-host CPU is the primary scope; a per-cgroup figure is shown as secondary and labelled, because interrupt and kernel-worker time is charged elsewhere.
2. At 1,000 concurrent connections the report records sockets and memory per arm.
3. Forward-path userspace CPU and return-path splice CPU are separate rows.

```gherkin
Scenario: CPU scope is stated and whole-host is primary
  Then CPU is reported whole-host with a kernel and userspace split, and any cgroup-scoped figure is labelled secondary

Scenario: Memory, sockets and concurrency are reported per arm
  Then each arm and direction records memory, socket count and connection concurrency

Scenario: Forward and return work are separate
  Then forward-path userspace work and return-path splice work are separate rows

Scenario: A scope that cannot be measured is absent
  Given a metric unavailable on the host
  Then the report names it absent and never reports zero
```

AC: (a) CPU scope stated, whole-host primary; (b) memory, sockets, concurrency per arm and direction; (c) forward and return work separate; (d) unavailable metrics named absent. Notes: research F3.2 caveats on cgroup accounting.

## Wave: DISCUSS / [REF] Story map and scope assessment

Backbone: Declare the benchmark, Run it on the exclusive host, Trust the conditions, Read the result, Extend to a family.

| Declare | Run | Trust the conditions | Read | Extend to a family |
|---|---|---|---|---|
| WS: attribute on a function (01) | WS: exclusive benchmark form (01) | WS: host settings, stats scope (03) | WS: raw record and report (01), A/B protocol (02) | VM lifecycle host stages (05), guest stages and cold/warm (06) |
| WS: spike probe, same attribute (04) | | | WS: spike-labelled record (04) | DNS (07) |
| WS: smoke variant (04) | | | | mTLS latency and throughput (08), mTLS resources (09) |

Walking skeleton = slices 01 to 04 (connect-hook cost), delivering a maintained lane and a spike-usable path from the same declaration. Priority rationale: 01 retires the riskiest assumption (a macro-declared function runs on an exclusive Metal benchmark form and its records come back). 02 follows because the statistical protocol is what makes the number comparable and puts the spike data point beside it. 03 follows because the production-settings check and the per-process statistics are what the spike showed can bend a result. 04 closes the dual-use requirement. Families follow in order of #287's acceptance criteria and learning leverage: 05 (host-stamped stages, no clock problem) before 06 (guest clocks, the harder question), then DNS (07, self-contained), then mTLS (08 before 09 so the baseline exists before resource attribution).

Scope assessment: **PASS (sliced)**: 9 stories, 3 bounded contexts (measurement core crate, macro crate, xtask/Metal runner), integration surfaces about 5 (macro, core crate, Metal benchmark form, wrapper or admission, record and report). Effort is the borderline signal: the four skeleton slices plus three families plausibly exceed two weeks in total. It is mitigated by nine independently shippable slices, the family slices being separate deliverables, and the skeleton-first order. Families named in #287 but not in this feature are listed under out of scope.

## Wave: DISCUSS / [REF] Outcome KPIs

Objective: Marcus obtains a comparable, reproducible native number for a boundary without writing a harness.

| KPI | Who does what | Target | Baseline | Measured by |
|---|---|---|---|---|
| K1 (primary) | Marcus adds a new benchmark family | 0 lines of arm-interleaving, A/A control, provenance capture or report code in the family's diff | Each spike hand-wrote sampler, recorder and report (`bpf-stats-overhead` is a copy of `pig-connect-cost`) | Diff of the DNS family slice (the first family added after the skeleton is verified) |
| K2 | The framework reproduces the spike's first data point | New median gateway hook-time delta lies within the spike's per-run spread of that program's hook time (115.5 to 126.5 ns; spike median delta 117.0 ns) | One point-in-time spike result, no retained rerun | Report of the connect-hook benchmark against `findings-connect-hook-cost.md` |
| K3 | Every retained record carries its provenance | 100% of records have commit, dirty flag, methodology version, substrate, host settings and statistics flag | Spike findings record these by hand, inconsistently | Schema validation over every retained record |
| K4 (guardrail) | A drifted host is refused | 100% of runs on a host differing from the production settings refuse before any sample | No check exists | One deliberate drift (for example `bpf_stats_enabled` set to 1) in the acceptance scenario |
| K5 | A spike uses the same schema | 1 existing probe (`pig-connect-cost`) re-expressed with the same attribute; 0 schema forks | 0 spikes share a record schema | Schema validation of the spike's record |
| K6 | #287's separation requirements hold | VM lifecycle: 8 host-observable stages plus guest-measured durations reported as separate rows; DNS: client-observed and resolver-internal in separate tables; mTLS: 2 directions each with a baseline | No such separation exists | Report structure check per family |
| K7 (guardrail) | Benchmarks stay outside correctness gates | 0 benchmark results used as pass or fail; smoke variant asserts no timing | n/a | Review of the smoke variants and lanes |

Owner: implementer captures, independent reviewer evaluates; no dashboard. K2's tolerance is derived from the spike's observed spread, not invented.

## Wave: DISCUSS / [REF] Definition of Done

- [ ] US-01 to US-04 (skeleton): a declared connect-hook benchmark runs on the exclusive host, retains raw samples, reports the protocol results beside the spike reference, refuses a drifted host, and a spike crate and a smoke test use the same declaration.
- [ ] Methodology document at a named version; catalogue lists each family with workload matrix, clock boundary and result owner (a row added per family slice).
- [ ] US-05 and US-06: each VM-lifecycle stage reported separately; no merged completion signal; cold and warm separate.
- [ ] US-07: DNS client-observed and resolver-internal separate; profiles named.
- [ ] US-08 and US-09: mTLS both directions against a plaintext baseline; resources and forward/return split reported.
- [ ] `testing.md` and the `metal run` grammar rule amended for the benchmark form and the exclusive exemption (orchestrator process).
- [ ] Regression gating and baseline promotion are not implemented.
- [ ] No issue number or roadmap-step id in any new name.

## Wave: DISCUSS / [REF] Out of scope

- Regression gating and baseline promotion: tracked by [#361](https://github.com/overdrive-sh/overdrive/issues/361) (user-approved 2026-11-16); #157 remains the XDP helper.
- #157 (XDP perf-baseline update helper) and the Tier-4 `xdp-perf` gate: unchanged.
- #326 (vsock attachment density lane), #300 (pump scaling), #303 (in-guest kernel mTLS against the host proxy), #305 (Firecracker against Cloud Hypervisor): each is tracked by its own open issue and is a consumer of this program, not part of it. #303 and #305 already say to use #287 for the comparison.
- #357 (deploy latency under reconcile load): its own issue; it asks for a benchmark per `testing.md`.
- Snapshot/restore (#96): not implemented yet, so there is nothing to benchmark; not part of this feature (user, 2026-11-16).
- #329 (vDPA feasibility), #309 and #310 (UDP guest transport): feasibility or feature issues.
- #146 (linker build-time comparison, hyperfine): a build-time benchmark, not product.
- #288 (EDD), #289 (E2E runner): separate artifacts.
- Changing lifecycle semantics, DNS behaviour, the kTLS or proxy architecture, or published latency targets (#287 non-goals).

## Wave: DISCUSS / [REF] Walking skeleton strategy

Real, end to end, not a mock, native substrate. Slices 01 to 04 are the skeleton, promoted as production code under `crates/` for the maintained lane; the `spike-scratch/` use goes through the same declaration and record. Comparison point: the existing spike data. The spike's `pig-connect-cost` and `bpf-stats-overhead` probes stay as evidence and are not deleted by this feature. Run on the native Metal host per `testing.md`; Lima is no signal.

## Wave: DISCUSS / [REF] Driving ports

Developer-facing: (1) the attribute on a benchmark function; (2) the benchmark form of `cargo xtask metal run`; (3) `cargo xtask metal pull` of the retained result; (4) `cargo nextest run` on a benchmark crate for the smoke variant; (5) a `spike-scratch/` crate's run on the benchmark form. No HTTP or operator surface.

## Wave: DISCUSS / [REF] Pre-requisites

Metal host provisioned, `OVERDRIVE_METAL_TARGET`, `OVERDRIVE_METAL_KERNEL`, `OVERDRIVE_METAL_ROOTFS` set; guest kit prepared and module loaded for the VM-lifecycle family; the shipped placement attribute and its closed registry (`overdrive-testing-macros`, ADR-0294); the existing probes `pig-connect-cost` and `bpf-stats-overhead`; the research and spike findings listed above.

## Wave: DISCUSS / [REF] Open points

- O1 (**accepted by the user 2026-11-16**): what are "the appliance's production settings" the host is compared with? Recommendation: the methodology version pins an expected-settings list (kernel 7.0, `bpf_stats_enabled` 0, CPU governor, SMT, boost) taken from the appliance image configuration (ADR-0068) where it defines a value, and otherwise from the settings recorded on the first accepted host; a drift list is part of the methodology version. This follows ruling 2's "production settings", which sets no noise controls (turbo, SMT or ASLR are not changed).
- O2 (DESIGN): whether the bench target (`harness = false`) needs its own placement check or only the smoke test is placed (intake).
- O3 (DESIGN): VM-lifecycle stage boundaries and the DNS resolver-internal timing need observable points on production surfaces. Where one is missing, DESIGN defines it and surfaces it as a decision; DISCUSS does not invent it. This is contract-shaping and must be settled before DISTILL.
- O4 (DESIGN): where retained results live in the repository and their naming.
- O5 (DESIGN): the smoke variant needs a `#[test]`; the placement attribute never inserts `#[test]` (ADR-0282), so the benchmark macro, not the placement attribute, emits it. DESIGN confirms.
- Deferrals: regression gating and baseline promotion, tracked by #361. Every other scope limit is listed under out of scope with its issue.

## Wave: DISCUSS / [REF] DoR validation

| Item | 01 | 02 | 03 | 04 | 05 | 06 | 07 | 08 | 09 |
|---|---|---|---|---|---|---|---|---|---|
| 1 Problem in domain language | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 2 Persona specific | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 3 3+ examples, real data | PASS (4) | PASS (4) | PASS (3) | PASS (4) | PASS (4) | PASS (3) | PASS (3) | PASS (3) | PASS (3) |
| 4 UAT 3-7 scenarios incl. error | PASS (5) | PASS (6) | PASS (5) | PASS (5) | PASS (5) | PASS (4) | PASS (4) | PASS (4) | PASS (4) |
| 5 AC derived from UAT | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 6 Right-sized (1-3 days) | PASS (2-3 d) | PASS | PASS | PASS | PASS with risk: split by stage if over 3 d | PASS | PASS | PASS | PASS |
| 7 Technical notes | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 8 Dependencies tracked | PASS | PASS (01) | PASS (01) | PASS (01, 02) | PASS (01-03, O3) | PASS (05) | PASS (01-03, O3) | PASS (01-03) | PASS (08) |
| 9 KPIs numeric | PASS (K3) | PASS (K2) | PASS (K3, K4) | PASS (K5, K7) | PASS (K6) | PASS (K6) | PASS (K1, K6) | PASS (K6) | PASS (K6) |

Elevator pitch present on all; each "After" names the benchmark form, a spike crate's run, or `cargo nextest run`, with concrete output. `job_id: J-PLAT-008` on all. Verdict: PASSED, with O1 and O3 as DESIGN decisions. Per-wave peer review not invoked (optional; the consolidated review fires at end of DISTILL).
