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

Job: J-PLAT-008. Slice: [01](slices/slice-01-declare-and-run-a-benchmark.md). Must-have.

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

Job: J-PLAT-008. Slice: [02](slices/slice-02-ab-comparison-protocol.md). Must-have.

Marcus needs to know whether arm B's cost is real or noise. The report must separate the effect from the run-to-run floor and place it beside the spike's number.

### Elevator Pitch
Before: the spike reported two medians and a hand-computed delta, and its wall-time delta (about 168 ns) was not distinguishable from noise by inspection.
After: run the benchmark form for `cgroup-connect-hook` → sees a report with the B-minus-A hook-time delta and its 95% bootstrap interval, the A/A noise-floor interval, the canary result, the replicates per arm (at least 9), and the spike's 117 ns shown as a reference point.
Decision enabled: Marcus decides whether the gateway hook's added cost is an effect or noise, and can state it in one line with its interval.

Domain examples:
1. The report shows hook-time delta of about 117 ns with an interval that excludes zero, an A/A interval around zero, and the spike's 117.0 ns beside it.
2. The wall-time delta of about 168 ns has an interval that includes the A/A floor; the report labels it "not distinguishable from noise" rather than a difference.
3. A declaration asks for 5 replicates per arm; the build fails naming the minimum of 9 (CA-12), and no run starts.
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

Scenario: Too few replicates fail the build
  Given a declaration asks for 5 replicates per arm
  When Marcus builds it
  Then the build fails naming the minimum of 9 and no run record is written

Scenario: A shifting canary flags the run
  Given the canary measurement differs between the start and the end of the run
  Then the report flags the run as unstable and the record keeps every sample

Scenario: The spike's first data point is shown as a reference
  Then the report places the earlier 117 ns hook-time delta beside the new one, labelled as spike evidence from a different run
```

AC: (a) interleaved arm order and A/A control in one run; (b) delta with bootstrap interval, noise-floor interval and a nonparametric result in the report; (c) a delta inside the floor is labelled noise; (d) fewer than 9 replicates is a build-time error naming the minimum (CA-12); (e) canary shift flags the run; (f) the spike reference is shown and labelled; (g) the methodology document (runner isolation, host eligibility, clock boundaries, cold/warm handling, sample and percentile method, raw-result format, failure accounting) exists at a named version (#287 AC 1). Notes: ruling 5; replicates are independent runs, not iterations inside a run (research F1.1).

### US-BENCH-03 — Record the host's production settings and refuse a drifted host (walking skeleton)

Job: J-PLAT-008. Slice: [03](slices/slice-03-host-settings-and-bpf-stats.md). Must-have.

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

Job: J-PLAT-008. Slice: [04](slices/slice-04-spike-use-and-smoke-variant.md). Must-have.

Marcus runs a throwaway probe under `spike-scratch/` and wants evidence in the same record schema as the maintained lane, without copying a harness. He also wants the maintained benchmark to keep compiling and running between Metal runs.

### Elevator Pitch
Before: each spike is a copy of an earlier spike's harness, and a maintained benchmark would only be exercised on a benchmark run.
After: run a `spike-scratch/` crate's run (it depends on the benchmark crate by path and carries the same attribute) on the benchmark form → sees a record in the same schema marked `spike evidence` and the same report; run `cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests` → sees the guest-placed smoke test PASS, having taken one replicate per arm and asserting nothing about speed.
Decision enabled: Marcus decides a probe is enough for a design decision, with the record still comparable to the maintained lane, and trusts a benchmark that fails to build or run is caught by the Metal integration test run.

Domain examples:
1. The existing `pig-connect-cost` probe is re-expressed with the attribute; its record validates against the same schema and the report renders it labelled spike evidence.
2. A probe's record and the maintained lane's record for the same family share one schema; each record and its report state their substrate, and nothing compares across substrates.
3. The smoke test of the connect-hook benchmark runs one replicate per arm and passes; a deliberately broken fixture makes it fail on the Metal integration run.
4. The smoke test is gated on `integration-tests`, carries a guest placement (`cgroup-default` for connect-hook) and `placement-lint` accepts it.

```gherkin
Scenario: A spike crate produces the same record schema
  Given a crate under spike-scratch depends on the benchmark crate by path and declares a benchmark
  When Marcus runs it through the benchmark form
  Then the record validates against the same schema and is marked as spike evidence

Scenario: Every record and report states the substrate it was measured on
  Given a maintained record on the native substrate
  When Marcus reads the record and its report
  Then both state the native substrate and no command compares records across substrates

Scenario: The smoke variant runs one replicate per arm with no performance assertion
  When Marcus runs cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests
  Then the smoke test passes after one replicate per arm and asserts nothing about timing

Scenario: A broken benchmark fails the smoke test
  Given the benchmark's fixture no longer builds a dataplane
  When the same Metal integration run executes
  Then the smoke test fails naming the benchmark

Scenario: The smoke test is placed like any other test
  Then cargo xtask placement-lint accepts it
```

AC: (a) a `spike-scratch/` crate using the same attribute produces a schema-identical record marked spike evidence, with no change under `crates/` for the probe; (b) every record and report state their substrate, and no command compares records across substrates (CA-10); (c) smoke variant: one replicate per arm, no performance assertion, gated on `integration-tests`, guest-placed with `#[overdrive_test]` and lint-clean, run by `cargo xtask metal run -- cargo nextest run -p <crate> --features integration-tests` (CA-11); (d) a broken benchmark fails the smoke test. Notes: `spike.md` isolation holds; the spike's Cargo.toml depends on workspace crates by path only.

### US-BENCH-05 — Report each VM-lifecycle stage separately

Job: J-PLAT-008. Slice: [05](slices/slice-05-vm-lifecycle-host-stages.md). Must-have (#287 AC 4).

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

Job: J-PLAT-008. Slice: [06](slices/slice-06-vm-lifecycle-guest-stages.md). Must-have (#287 AC 4).

The guest's clock has a different zero point from the host's, so guest-internal stages cannot be subtracted from host instants.

### Elevator Pitch
Before: the guest's share of shutdown time cannot be placed against host stages without an unstated clock assumption, and cold versus warm image cache is an accident of run order.
After: run the benchmark form for `vm-lifecycle` with the cold and the warm profile → sees one guest-measured duration, `guest.shutdown-handling`, beside the host-stamped shutdown-delivery and guest-exit stage intervals (guest receipt and poweroff appear only as those host-stamped intervals), and separate cold-cache and warm-cache result sets, each naming its preparation step.
Decision enabled: Marcus decides whether shutdown delay is host-side or guest-side and whether the image cache matters.

Domain examples:
1. Shutdown handling is a duration measured inside the guest (`shutdown-received` to `group-complete`) and reported as a duration; the report never subtracts a guest instant from a host instant, and poweroff is only the host-stamped interval.
2. The cold profile records "page cache dropped before each replicate"; the warm profile records "image pre-read".
3. A guest that does not return a duration reports the stage as absent, not zero.

```gherkin
Scenario: The guest duration is reported as a duration
  Then guest.shutdown-handling appears as one guest-measured duration, never as a host instant
  And guest receipt and poweroff appear only as host-stamped stage intervals

Scenario: Cold and warm are separate profiles with named preparation
  When both profiles run
  Then each result set names its preparation step and is never merged with the other

Scenario: A missing guest duration is absent, not zero
  Given a guest that returns no duration for a stage
  Then the stage is reported as absent for that replicate

Scenario: Clock provenance is recorded
  Then the record names the host and guest clock sources
```

AC: (a) one guest-measured duration, `guest.shutdown-handling`, with receipt and poweroff reported only as host-stamped stage intervals (CA-2); (b) cold and warm profiles separate, each with its recorded preparation; (c) absent is not zero; (d) clock provenance (host and guest clock sources) recorded. Notes: research F1.9; the guest's actual clocksource is not settled (research Knowledge Gap 7).

### US-BENCH-07 — Report DNS responder timings: client-observed apart from resolver-internal

Job: J-PLAT-008. Slice: [07](slices/slice-07-dns-responder.md). Must-have (#287 AC 5).

### Elevator Pitch
Before: DNS responder cost has no number; a client-side timing would include everything and say nothing about the resolver's own work.
After: run the benchmark form for family `dns-responder` → sees a report with client-observed latency and throughput per profile (query shape, name cardinality, client count, arrival rate) and, separately, resolver-internal timing, with the profile named in every row.
Decision enabled: Marcus decides whether the responder or the network path dominates and which query profile to optimise.

Domain examples:
1. Profile "A record, 100 names, 1 client, 1,000 queries per second" and profile "unknown name, 10,000 names, 16 clients, 5,000 queries per second" are two result sets.
2. Client-observed p99 is reported in one table and resolver-internal time in another; neither is a column of the other.
3. A load generator that waits for each reply is not used for tail latency; the benchmark's load is scheduled-arrival (research F1.6) and the report states the arrival model.

```gherkin
Scenario: Client-observed and resolver-internal timings are separate
  Then the report has distinct tables for client-observed results and resolver-internal timing

Scenario: Profiles are named and distinct
  Then each result set names query shape, name cardinality, client count and arrival rate

Scenario: The arrival model is stated
  Then the report states whether load was scheduled-arrival or closed-loop

Scenario: A failed query is counted
  Given a query times out
  Then it is counted as a failure in its profile and never as a latency sample
```

AC: (a) client-observed and resolver-internal reported separately; (b) profile named per result set; (c) arrival model stated; (d) failures counted; (e) the recorded fixture fields: host block (kernel, CPU, governor, SMT, boost), the network namespace fixture the responder and clients ran in, the arrival model, scheduled rate and duration, client socket count and name-table cardinality; (f) the family has a catalogue row. Notes: drives the production responder in-process (D4); cache hit or miss, transports other than UDP and response size are not profile axes (CA-3).

### US-BENCH-08 — Compare plaintext and composed mTLS for connection cost and throughput, both directions

Job: J-PLAT-008. Slice: [08](slices/slice-08-mtls-latency-and-throughput.md). Must-have (#287 AC 6).

### Elevator Pitch
Before: the mTLS and proxy path has no number against a plaintext baseline, and the one handshake figure on record is a single sample the spike itself calls "not a benchmark".
After: run the benchmark form for family `mtls-path` → sees, for each traffic direction, plaintext-baseline arm against composed-mTLS arm: handshake, first-byte latency, sustained throughput and tail latency, with the interval of the difference and the noise floor.
Decision enabled: Marcus decides what the transparent mTLS path costs per direction.

Domain examples:
1. Egress (workload to peer) and ingress (peer to workload) are two result sets, each with its own baseline.
2. The plaintext baseline is a defined direct path between the same two endpoints; the report states its definition.
3. Handshake time and first-byte latency are separate rows.
4. In the egress composed-mTLS arm, 12 of 5,000 connection attempts time out at the handshake; they are counted as failures in that arm and direction and are not latency samples.

```gherkin
Scenario: Both directions are reported
  Then the report has a plaintext-baseline arm and a composed-mTLS arm for each direction

Scenario: Handshake, first byte, throughput and tail are separate
  Then connection establishment, first-byte latency, sustained throughput and tail latency are separate rows

Scenario: The baseline is defined
  Then the report states what the plaintext baseline is

Scenario: A direction's difference carries its noise floor
  Then each direction's difference shows its interval and its A/A floor

Scenario: A failed or timed-out connection is a counted failure
  Given a connection or handshake fails or times out in the composed-mTLS arm of one direction
  Then it is counted as a failure in that arm and direction
  And it is never a latency sample
```

AC: (a) both directions, each with a baseline; (b) the four measures separate; (c) baseline defined in the report; (d) interval and A/A floor per direction; (e) a failed or timed-out connection or handshake is counted as a failure in its arm and direction, never a latency sample; (f) wire-capture correctness stays a Tier-3 test, not part of this benchmark. Notes: the egress dialer is plaintext by design (`CLAUDE.md` § East-west mTLS tests); the benchmark client follows it; drives the production composition in-process (D4).

### US-BENCH-09 — Report resource cost and the forward-versus-return work split for mTLS

Job: J-PLAT-008. Slice: [09](slices/slice-09-mtls-resources-and-split.md). Must-have (#287 AC 3 and 6).

### Elevator Pitch
Before: the claim that the proxy is "agent-light" (little userspace work on the forward path, splice on the return) has no measurement behind it.
After: run the benchmark form for `mtls-path` resource profile → sees CPU (whole-host from `/proc/stat`, with the kernel and userspace split), memory, socket count and connection concurrency per arm and direction, plus, for egress, forward-path userspace work reported apart from return-path splice work (absent for ingress, CA-6).
Decision enabled: Marcus decides whether the agent-light claim holds and where a future mechanism (for example an in-guest kernel path) must beat the current one.

Domain examples:
1. CPU is whole-host only; no cgroup figure is reported, and the report states that interrupt and kernel-worker time is charged outside the benchmark's cgroup (CA-9).
2. At 1,000 concurrent connections the report records sockets and memory per arm.
3. Forward-path userspace CPU and return-path splice CPU are separate rows.

```gherkin
Scenario: CPU scope is stated and is whole-host
  Then CPU is reported whole-host with a kernel and userspace split, and no cgroup-scoped figure is reported

Scenario: Memory, sockets and concurrency are reported per arm
  Then each arm and direction records memory, socket count and connection concurrency

Scenario: Forward and return work are separate
  Then for egress, forward-path userspace work and return-path splice work are separate rows
  And for ingress they are reported absent with the reason

Scenario: A scope that cannot be measured is absent
  Given a metric unavailable on the host
  Then the report names it absent and never reports zero
```

AC: (a) CPU scope stated, whole-host only (CA-9); (b) memory, sockets, concurrency per arm and direction; (c) forward and return work separate for egress, absent for ingress (CA-6); (d) unavailable metrics named absent. Notes: research F3.2 caveats on cgroup accounting.

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
| K1 (primary) | Marcus adds a new benchmark family | 0 lines of arm-interleaving, A/A control, provenance capture or report code in the family's declaration and fixture files; no use of runner, record or report internals from the family module | Each spike hand-wrote sampler, recorder and report (`bpf-stats-overhead` is a copy of `pig-connect-cost`) | Diff of the DNS family slice (the first family added after the skeleton is verified), limited to the family's declaration and fixture files and excluding production-surface changes (the seams the family needs) |
| K2 | The framework reproduces the spike's first data point | New median gateway hook-time delta lies within the spike's per-run spread of that program's hook time (115.5 to 126.5 ns; spike median delta 117.0 ns) | One point-in-time spike result, no retained rerun | Report of the connect-hook benchmark against `findings-connect-hook-cost.md` |
| K3 | Every retained record carries its provenance | 100% of records have commit, dirty flag, methodology version, substrate, host settings and statistics flag | Spike findings record these by hand, inconsistently | Schema validation over every retained record |
| K4 (guardrail) | A drifted host is refused | 100% of runs on a host differing from the production settings refuse before any sample | No check exists | One deliberate drift (for example `bpf_stats_enabled` set to 1) in the acceptance scenario |
| K5 | A spike uses the same schema | 1 existing probe (`pig-connect-cost`) re-expressed with the same attribute; 0 schema forks | 0 spikes share a record schema | Schema validation of the spike's record |
| K6 | #287's separation requirements hold | VM lifecycle: 8 host-observable stages plus one guest-measured duration (`guest.shutdown-handling`) reported as separate rows; DNS: client-observed and resolver-internal in separate tables; mTLS: 2 directions each with a baseline | No such separation exists | Report structure check per family |
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
- Guest and other substrates: the substrate registry is closed to `native`, tracked by [#362](https://github.com/overdrive-sh/overdrive/issues/362).
- Snapshot/restore (#96): not implemented yet, so there is nothing to benchmark; not part of this feature (user, 2026-11-16).
- #329 (vDPA feasibility), #309 and #310 (UDP guest transport): feasibility or feature issues.
- #146 (linker build-time comparison, hyperfine): a build-time benchmark, not product.
- #288 (EDD), #289 (E2E runner): separate artifacts.
- Changing lifecycle semantics, DNS behaviour, the kTLS or proxy architecture, or published latency targets (#287 non-goals).

## Wave: DISCUSS / [REF] Walking skeleton strategy

Real, end to end, not a mock, native substrate. Slices 01 to 04 are the skeleton, promoted as production code under `crates/` for the maintained lane; the `spike-scratch/` use goes through the same declaration and record. Comparison point: the existing spike data. The spike's `pig-connect-cost` and `bpf-stats-overhead` probes stay as evidence and are not deleted by this feature. Run on the native Metal host per `testing.md`; Lima is no signal.

## Wave: DISCUSS / [REF] Driving ports

Developer-facing: (1) the attribute on a benchmark function; (2) the benchmark form of `cargo xtask metal run`; (3) `cargo xtask metal pull` of the retained result; (4) `cargo xtask metal run -- cargo nextest run -p <crate> --features integration-tests` for the smoke variant; (5) a `spike-scratch/` crate's run on the benchmark form. No HTTP or operator surface.

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
| 4 UAT 3-7 scenarios incl. error | PASS (5) | PASS (6) | PASS (5) | PASS (5) | PASS (5) | PASS (4) | PASS (4) | PASS (5) | PASS (4) |
| 5 AC derived from UAT | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 6 Right-sized (1-3 days) | PASS (2-3 d) | PASS | PASS | PASS | PASS with risk: split by stage if over 3 d | PASS | PASS | PASS | PASS |
| 7 Technical notes | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 8 Dependencies tracked | PASS | PASS (01) | PASS (01) | PASS (01, 02) | PASS (01-03, O3) | PASS (05) | PASS (01-03, O3) | PASS (01-03) | PASS (08) |
| 9 KPIs numeric | PASS (K3) | PASS (K2) | PASS (K3, K4) | PASS (K5, K7) | PASS (K6) | PASS (K6) | PASS (K1, K6) | PASS (K6) | PASS (K6) |

Elevator pitch present on all; each "After" names the benchmark form, a spike crate's run, or `cargo nextest run`, with concrete output. `job_id: J-PLAT-008` on all. Verdict: PASSED, with O1 and O3 as DESIGN decisions. Per-wave peer review not invoked (optional; the consolidated review fires at end of DISTILL).

---

# DESIGN

DESIGN, 2026-11-16. Application scope, propose mode, density `lean`, paradigm object-oriented. Decisions DDD-1 to DDD-14 were approved by the user on 2026-11-17, with the rulings recorded under "User approval" below; independent design review (adversarial and subtractive) was resolved on 2026-11-17 and ADR-0296 to ADR-0309 are **Accepted** (`.claude/rules/design.md`; reviews in `design/review-design.md` and `design/review-design-subtractive.md`). Scope is all nine slices and open points O1 to O5. Architecture style: ports-and-adapters around a pure measurement core inside two dev-only `adapter-host` crates; no new production runtime component, and two small production-surface changes (DDD-12, DDD-14), both approved.

## Wave: DESIGN / [REF] Inputs read

✓ `intake.md` (ten rulings), `slices/slice-01..09`, `spike/findings-bpf-stats-overhead.md`, the research document, DISCUSS sections above. ✓ `.claude/rules/{design,testing,development,rust,spike,reconcilers}.md`, CLAUDE.md. ✓ `docs/product/architecture/brief.md`, ADR-0003, ADR-0068, ADR-0282 to ADR-0295, `docs/evolution/2026-11-16-metal-test-placement-attribute.md`. ✓ `spike/findings-v1..v5-*.md` (revision after the contract-shaping spikes), `crates/overdrive-testing/src/guest_kit.rs`, `crates/overdrive-control-plane/src/lib.rs:3500-3761`. ✓ Code: `crates/overdrive-testing-macros/src/lib.rs`, `crates/overdrive-testing/src/placement/`, `infra/metal/{bootstrap.sh,test-wrapper.sh}`, `xtask/src/{main.rs,metal_pull.rs,placement_lint.rs}`; VM stage seams (`overdrive-control-plane/src/lib.rs`, `overdrive-worker/src/vm_driver.rs`, `overdrive-core/src/traits/{vmm.rs,clock.rs}`, `overdrive-init/src/main.rs`); DNS (`overdrive-control-plane/src/dns_responder/`); mTLS (`overdrive-dataplane/src/mtls/`, `overdrive-worker/src/mtls_intercept_worker.rs` and their Tier-3 rigs). ✓ Prior art: `spike-scratch/{pig-connect-cost,bpf-stats-overhead}`. ⊘ `discuss/*.md` per-file artifacts (this feature keeps DISCUSS in this file). ⊘ `spike-scratch/{netns-density-295-kernel-vs-aya-benchmark,boot-cost}` read through the research summary only. Kernel claims are read in `vendors/linux` (v7.0) and cited as `path:line @ v7.0`. Contradictions with DISCUSS requirements and with the rulings are listed under Changed Assumptions; none contradicts a numbered ruling.

## Wave: DESIGN / [REF] Reuse Analysis

Contract shape (principle 12): P = pure-function, B = bounded-change (declared mutation set), U = unbounded-preservation (returns a plan, never mutates).

| Existing | Path | Overlap | Decision | Justification and contract shape |
|---|---|---|---|---|
| Placement attribute, parser, closed guest registry | `crates/overdrive-testing-macros/src/lib.rs`, `crates/overdrive-testing/src/placement/groups.rs` | declaration parsing, `guest` group and profile validation by constant evaluation, record emission | EXTEND | New attribute `overdrive_bench` in the same crate; the smoke `smoke(guest = ...)` argument reuses the group parser and registry unchanged. No new record kind, so `test-wrapper.sh` and `placement-lint` are untouched. Shape: B (writes only the expansion). |
| Substrate vocabulary | same registry | closed list of named environments | CREATE NEW | The registry lists guest groups; `native` is not a guest group and mixing them breaks ADR-0294's meaning. A second closed list of one entry in `overdrive-bench`. |
| Metal runner | `xtask/src/main.rs` (`MetalAction::Run`), `infra/metal/bootstrap.sh` | `--run`, `--exclusive`, lease, run record, rsync | EXTEND | One flag and one closed mode; no second runner. Shape: B (remote run directory and `benchmarks/results/<family>/<run-id>/`). |
| `metal pull`, `metal collect` | `xtask/src/metal_pull.rs`, `bootstrap.sh` | retrieving remote outputs | EXTEND (unchanged) | The result path obeys the existing path rules. |
| Test placement, `overdrive_test` | ADR-0282..0295 | smoke test placement | EXTEND (reuse) | The smoke test is an ordinary placed test. |
| Connect-hook probes | `spike-scratch/pig-connect-cost/src/main.rs`, `spike-scratch/bpf-stats-overhead/src/main.rs` | fixture, arms, warm-up, hook-time and wall-time reading | CREATE NEW | A spike is throwaway and may not be moved into `crates/` (`spike.md`); the family re-implements the fixture against the framework and the spikes stay as evidence. Shape: B (one cgroup, one pin directory, one veth pair per replicate, removed on drop). |
| Native lifecycle benchmark test and its layer | `crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs` (`LifecycleTraceLayer`, `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials`) | event-stamping layer, stage distributions | CREATE NEW | Test-local code in a delivered gate; moving it changes that gate. The family re-creates the layer (not the targets). The test stays as it is and does not move onto this framework (user ruling 2026-11-17); two layers observe the same events. |
| Lifecycle events, `BeaconIo`, `Vmm`, console log file, observation subscription, guest kit image (`GuestImageSpec::workload()`) | `overdrive-control-plane/src/lib.rs`, `overdrive-worker/src/vm_driver.rs`, `overdrive-core/src/traits/` | every VM stage boundary | EXTEND (consume as is) | No production change (DDD-11). |
| Test fixtures | `overdrive-testing` (`NetNs`, `TestCidrLease`, `RemotePeer`, `VmFixture`, `guest_kit`) | cgroup, netns, veth, VM image staging | EXTEND (consume as is) | No duplicate fixture. |
| DNS responder | `crates/overdrive-control-plane/src/dns_responder/responder.rs` | per-query service time | EXTEND | Adds an observer parameter (DDD-12); the alternative of timing pure functions is weaker. Shape: P for `NoQueryObserver`, B for a recorder (appends to its own buffer); asserted by an observer test that the responder's replies are byte-identical with and without it. |
| mTLS pump threads | `crates/overdrive-dataplane/src/mtls/splice.rs` | per-direction CPU | EXTEND | Thread names only (DDD-14). Shape: B (thread name); asserted by reading `/proc/self/task/*/comm` in the dataplane Tier-3 rig. |
| mTLS rigs | `overdrive-worker/tests/integration/vsock_mesh_egress_mtls.rs`, `overdrive-dataplane/tests/integration/mtls_inbound_guest_relay.rs` | composed egress and inbound paths | CREATE NEW | Test-local rigs cannot be imported; the families crate composes the same pieces from their public constructors. |
| `xdp-perf`, Tier-4 gates | `overdrive-dataplane/bin/`, `perf-baseline/` | native performance numbers | EXTEND (unchanged) | A gate, not a benchmark; D12. |
| EDD catalogue | `verification/` | native evidence | EXTEND (unchanged) | Different artifact (`testing.md` § "Classify external execution"). |

## Wave: DESIGN / [REF] Components and ports

| Component | Path | Change | Role |
|---|---|---|---|
| `overdrive-bench` | `crates/overdrive-bench/` | CREATE NEW | Measurement core: declaration types, registry, schedule (pure), statistics (pure), canary verdict (pure function over the start and end samples), record writer and validator, report renderer (pure), runner, host eligibility, host clock, BPF stats hold, `Error` and `Result`, `bench-report` tool |
| `overdrive-bench-families` | `crates/overdrive-bench-families/` | CREATE NEW | Four families, bench target `benchmarks` |
| `overdrive_bench` attribute | `crates/overdrive-testing-macros/` | EXTEND | Declaration macro |
| Metal benchmark form | `xtask/src/main.rs`, `infra/metal/bootstrap.sh` | EXTEND | Closed exclusive mode |
| Catalogue, methodology, results | `benchmarks/` | CREATE NEW (docs and data) | `catalogue.md`, `methodology.md`, `results/<family>/<run-id>/` |
| `DnsResponder` | `overdrive-control-plane` | EXTEND | Observer parameter |
| mTLS pump threads | `overdrive-dataplane` | EXTEND | Thread names |
| `testing.md` | `.claude/rules/` | amend (orchestrator) | Exclusive-run exemption, benchmark form |

**Driving ports:** (1) the `overdrive_bench` attribute on a function; (2) `cargo xtask metal run --benchmark ...`; (3) `cargo xtask metal pull <result dir>`; (4) the generated smoke tests of `overdrive-bench-families`, run as `cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests` (the smoke tests are `integration-tests`-gated and guest-placed, so a plain `cargo nextest run` does not build or run them, CA-11); (5) a spike crate's `benchmarks` bench target run through (2) with `--spike`; (6) `cargo bench-report` (alias for the `bench-report` binary): `validate <record>` and `render <record>`, host-independent.

**Driven ports and adapters** (each with a `probe()` that proves it can honour its contract where the environment can lie):

| Port | Production adapter | Test double | What `probe()` proves |
|---|---|---|---|
| `HostProbe` (read-only; no write methods) | procfs and sysfs reader | fixture reader | absent-by-design is distinguished from unreadable; the `bpf-stats` descriptor scan can see a held descriptor |
| `StatsEnabler` | `aya::sys::enable_stats` | fake | n/a: a requested-but-ineffective statistics setting is a failed replicate (`run_count` delta of 0, DDD-8) |
| `RecordSink` | append-only file in the result directory | in-memory | create-new semantics; a flush after every line |
| `HostClock` | `clock_gettime(CLOCK_MONOTONIC_RAW)` | scripted | n/a (no probe: the call cannot be made to lie about monotonicity on the pinned kernel) |

The canary is a loopback connect-and-close loop run by the runner; its verdict is a pure function over the start and end samples.

Wiring order in `run_main`: read the checkout marker, then probe, then read every host setting (including the `bpf-stats` descriptor scan, which therefore runs before the runner holds its own descriptor) and decide eligibility, then enable statistics (only when eligible and declared), then write `run_started` (so its `bpf_stats` field states what the run holds), then, when refused, write `run_refused` and stop, else canary, run, canary, finish ("wire then probe then use"). A failed marker read, probe or eligibility check refuses with a typed error and never falls back; DDD-6 pins which refusals leave a record line.

## Wave: DESIGN / [REF] Decisions table

| ID | Decision | ADR | Status |
|---|---|---|---|
| DDD-1 | Two dev-only `adapter-host` crates: `overdrive-bench` (core) and `overdrive-bench-families` | [0296](../../product/architecture/adr-0296-benchmarks-are-measured-by-a-dev-only-core-crate-with-families-in-a-separate-crate.md) | approved by the user 2026-11-17 |
| DDD-2 | `#[overdrive_bench]` in `overdrive-testing-macros`; emits the smoke `#[test]` with its own placement | [0297](../../product/architecture/adr-0297-a-benchmark-is-declared-with-an-attribute-that-also-emits-its-smoke-test.md) | approved by the user 2026-11-17 |
| DDD-3 | Link-time registration with `linkme` | [0298](../../product/architecture/adr-0298-declared-benchmarks-register-at-link-time-in-a-distributed-slice.md) | approved by the user 2026-11-17 |
| DDD-4 | Closed substrate registry with one entry, `native` | [0299](../../product/architecture/adr-0299-benchmark-substrates-are-a-closed-registry-and-the-first-entry-is-native.md) | approved by the user 2026-11-17 |
| DDD-5 | Runner-owned statistical protocol, methodology version 1 | [0300](../../product/architecture/adr-0300-the-runner-owns-the-statistical-protocol-and-versions-it-as-the-methodology.md) | approved by the user 2026-11-17 |
| DDD-6 | Append-only JSONL record, derived report, results under `benchmarks/results` (O4) | [0301](../../product/architecture/adr-0301-a-benchmark-result-is-an-append-only-record-with-a-derived-report.md) | approved by the user 2026-11-17 |
| DDD-7 | Host eligibility against a pinned list; refuse before any sample (O1) | [0302](../../product/architecture/adr-0302-a-benchmark-refuses-a-host-that-differs-from-the-pinned-test-host-settings.md) | approved by the user 2026-11-17 |
| DDD-8 | Per-process BPF statistics through the descriptor-scoped command | [0303](../../product/architecture/adr-0303-bpf-run-statistics-are-enabled-for-the-benchmark-process-only.md) | approved by the user 2026-11-17 |
| DDD-9 | `metal run --benchmark`: closed command, exclusive by construction | [0304](../../product/architecture/adr-0304-the-metal-benchmark-form-runs-a-closed-command-under-exclusive-admission.md) | approved by the user 2026-11-17 |
| DDD-10 | Host stamps on `CLOCK_MONOTONIC_RAW`; guest stages are durations only | [0305](../../product/architecture/adr-0305-stage-boundaries-are-stamped-on-the-host-and-guest-stages-are-durations.md) | approved by the user 2026-11-17 |
| DDD-11 | VM-lifecycle stages observed through existing seams; no new production API (O3a) | [0306](../../product/architecture/adr-0306-vm-lifecycle-stages-are-observed-through-existing-production-seams.md) | approved by the user 2026-11-17 |
| DDD-12 | DNS responder reports per-query service time to a `QueryObserver` (O3b) | [0307](../../product/architecture/adr-0307-the-dns-responder-reports-per-query-service-time-to-an-observer.md) | approved by the user 2026-11-17 (production change) |
| DDD-13 | mTLS family measures the host agent hop against a plaintext baseline | [0308](../../product/architecture/adr-0308-the-mtls-benchmark-measures-the-host-agent-hop-against-a-plaintext-baseline.md) | approved by the user 2026-11-17 |
| DDD-14 | mTLS outbound pump threads are named `mtls-out-fwd` and `mtls-out-ret` | [0309](../../product/architecture/adr-0309-mtls-pump-threads-carry-their-role-in-their-name.md) | approved by the user 2026-11-17 (production change) |

Resolution of the open points: O1 by DDD-7 (list below); O2 by DDD-2 (the bench target is not placed; only the smoke test is); O3 by DDD-11, DDD-12, DDD-14 (and DDD-13); O4 by DDD-6; O5 by DDD-2 (the benchmark macro emits `#[test]`).

## Wave: DESIGN / [REF] User approval

2026-11-17, all on the recommended option:

- DDD-1 to DDD-14 approved as proposed, including the two production changes DDD-12 (`QueryObserver` on `DnsResponder::new`) and DDD-14 (pump thread names). ADRs 0296 to 0309 are Accepted: approved by the user 2026-11-17; independent design review resolved 2026-11-17.
- CA-1 (smoke test placement; connect-hook in `cgroup-default`, DNS and mTLS in a guest group chosen in DISTILL), CA-2 (one guest-clock duration, `guest.shutdown-handling`; receipt and poweroff are host-stamped stage intervals; no host/guest clock-offset protocol) and CA-3 (DNS profile axes are those the responder has; a property of the mechanism, not a scope limit) accepted.
- The existing `native_lifecycle_profiles_meet_stage_targets_without_dropping_trials` test stays as it is and does not move onto this framework.
- `SimKek` (`overdrive-sim`) is allowed in `overdrive-bench-families`' `[dependencies]` (dev-only crate).
- `record.jsonl` and `report.md` are both committed under `benchmarks/results/<family>/<run-id>/`.
- Scope limits: the `native`-only substrate (DDD-4) is tracked by [#362](https://github.com/overdrive-sh/overdrive/issues/362). The ingress exclusion of the guest transport and the absent ingress forward/return split (DDD-13, CA-6, CA-8) are properties of the host-hop benchmark and have no issue: the ADR-0069 host proxy it measures is being replaced by in-guest kernel mTLS ([#303](https://github.com/overdrive-sh/overdrive/issues/303)), which names #287 as the way to compare the two. Guest receipt and poweroff durations have no issue (CA-2).
- The VM-lifecycle family fixture creates its own data directory (DDD-11).
- User ruling 2026-11-17: the runner's end-to-end run stays out of nextest; its evidence is a retained `metal run --benchmark` run under exclusive admission, and the nextest runner coverage is the smoke variant plus the record and report tests (DDD-9, evidence lane). The DISTILL placement decision D-1 (`native_launcher` for the families tests) and the long-run budget D-2 therefore lapse.

## Wave: DESIGN / [REF] Interface contracts

Rust signatures below are contracts pinned by DESIGN; private structure belongs to the crafter.

### DDD-1 — Two dev-only adapter-host crates

- `crates/overdrive-bench`: `crate_class = "adapter-host"`, `[features] integration-tests = []`, `publish = false`. Dependencies: `overdrive-testing-macros`, `serde`, `serde_json`, `rand` (seeded `StdRng`), `aya` (`aya::sys::enable_stats`), `libc`, `linkme`, `thiserror`. Binary `bin/bench_report.rs` with a `[[bin]]` entry; alias `bench-report = "run -p overdrive-bench --bin bench-report --"`.
- `crates/overdrive-bench-families`: `crate_class = "adapter-host"`, `[features] integration-tests = []`, `[[bench]] name = "benchmarks" harness = false path = "benches/benchmarks.rs"` whose `main` is `use overdrive_bench_families as _;` followed by `fn main() -> std::process::ExitCode { overdrive_bench::run_main(overdrive_bench::Evidence::Maintained) }` (the reference to the declaring crate is required, DDD-3). Family modules live in `src/families/<family>.rs`. Dependencies: `overdrive-bench`, `overdrive-testing`, `overdrive-core`, `overdrive-control-plane`, `overdrive-worker`, `overdrive-dataplane`, `overdrive-host`, `overdrive-cli` (library target: the operator driving port, called in process), `overdrive-sim` (`SimKek`, the hermetic key adapter only; approved for this dev-only crate's `[dependencies]` 2026-11-17), `aya`, `tracing`, `hickory-proto`.
- Neither crate appears in the `[dependencies]` of any production crate. Enforcement: `crate_class` dispatch by dst-lint and review of `[dependencies]`. No mechanical gate keeps `overdrive-testing` or these crates dev-only today (DISTILL confirmed none exists), and none is added here.
- A spike crate depends on `overdrive-bench` and `overdrive-testing` by path, declares `[[bench]] name = "benchmarks" harness = false`, and its `main` references the crate that declares its benchmarks (`use <crate> as _;`, DDD-3) and calls `run_main(Evidence::Spike)`. It adds no file under `crates/`.

### DDD-2 — The `overdrive_bench` attribute and its smoke test

```text
#[overdrive_bench(
    family     = "<kebab>",
    substrate  = native,                             // a bare identifier, like `host_pure`; never a string literal
    arms       = ["<arm>", ...],                     // one or more, unique, [a-z0-9][a-z0-9+._-]*; the first is the reference arm
    replicates = <u32 literal>,                      // build error below 9, naming the minimum
    profile(name = "<kebab>", preparation = "<text>")*,   // none => one implicit profile "default"
    metric(name = "<snake>", unit = <ns|bytes|bytes_per_second|qps|ratio|count>,
           table = "<kebab>" [, series])+,           // every duration is recorded in ns; `series` marks a metric recorded with `record_series` (DDD-6), else it is scalar
    bpf_stats,                                       // optional flag
    reference(metric = "<snake>", value = <float>,   // compared with that metric's A/B delta
              label = "<text>", source = "<repo-relative path>")*,
    smoke(host_pure | guest = "<group>" [, profile = "<profile>"] | none),   // required
)]
fn <name>(rep: &mut Replicate) -> Result<(), ReplicateFailure> { ... }
```

Unknown or repeated keys and malformed values are build errors that name the offending key. The pinned `error:` message for each rejection (`<...>` is the offending text; trybuild fixtures assert these lines, and rustc's span and label lines are regenerated once):

| Rejection | Message |
|---|---|
| unknown key | ``unknown key `<key>` in `#[overdrive_bench(...)]`; the keys are family, substrate, arms, replicates, profile, metric, bpf_stats, reference, smoke`` |
| repeated key (only `profile`, `metric`, `reference` repeat) | ``key `<key>` is given more than once`` |
| missing required key | ``missing required key `<key>` `` (for `smoke`: ``missing required key `smoke`; give smoke(host_pure), smoke(guest = "<group>") or, in a spike, smoke(none)``) |
| malformed value | ``key `<key>`: expected <what>`` |
| substrate given as a string literal | ``key `substrate`: expected an identifier such as `native`, found a string literal`` (a substrate is a bare identifier like the placement kinds `host_pure` and `native_launcher`; a string is the form of a guest group name, `guest = "<group>"`, which a substrate is not) |
| substrate identifier not registered | ``substrate `<value>` is not registered; the registered substrates are: native`` |
| substrate identifier `guest` | ``guest substrates are not available to benchmarks (substrate `guest`); the registered substrates are: native`` |
| `replicates` below 9 | ``replicates = <n> is below the minimum of 9 per arm`` |
| repeated arm | ``arm `<name>` is declared more than once`` |
| arm outside the grammar | ``arm `<name>` does not match [a-z0-9][a-z0-9+._-]*`` |
| unknown metric unit | ``unit `<value>` is not one of ns, bytes, bytes_per_second, qps, ratio, count`` |
| `smoke(none)` outside a spike | ``smoke(none) is allowed only in a crate under spike-scratch/`` |
| unknown guest group in `smoke` | the placement registry's existing diagnostic (ADR-0294), unchanged |

The smoke test is the only consumer of `smoke`: the macro exports nothing else, and a test that needs to exercise the smoke path runs the generated `smoke_<name>` test, not `smoke` on a declaration it chooses. `substrate` is checked by constant evaluation against DDD-4; `guest` in `smoke` against ADR-0294's registry (the existing check, unchanged). `smoke(none)` is accepted only when `CARGO_MANIFEST_DIR` contains `/spike-scratch/` (constant evaluation). The expansion: the function unchanged; one registration (DDD-3); and, unless `smoke(none)`, `#[cfg(feature = "integration-tests")] #[::overdrive_testing::overdrive_test(<smoke placement>)] #[test] fn smoke_<name>() { ::overdrive_bench::smoke(&DECL) }` (placement attribute first, per its attribute-order rule). The benchmark crate therefore depends on `overdrive-testing`. The macro adds no ELF record kind.

### DDD-3 — Link-time registration

`overdrive-bench` owns one `linkme` distributed slice of `BenchmarkDecl`; the macro adds one entry per declaration. A family has exactly one benchmark: the runner selects the one entry whose `family` equals `--family`, and more than one is a refusal (`Error::Usage`, exit 64, naming the family and the benchmark names). Spike V-5 (`linkme` 0.3.38, thin LTO, one codegen unit, `7.0.0-38-generic`) found every entry in the library's unit-test binary and in a `harness = false` bench binary, with one edge: rustc does not link a dependency crate nothing references, and its entries vanish without an error. Two rules follow. (1) The bench `main` of the families crate, and the `main` of any spike crate, references the declaring crate (`use <crate> as _;`). (2) A `--family` that no registered benchmark declares is a refusal (`Error::FamilyNotDeclared`) naming the requested family and the families found, exit 64; the message states rule (1) as the usual cause. The smoke test binaries are unaffected, since the declarations sit in the crate under test.

### DDD-4 — Substrate registry

`overdrive-bench` holds a closed registry `["native"]` with a constant-evaluable validity check. `native` means the exclusive Metal host with no namespace and no wrapper. The attribute takes the substrate as a bare identifier (`substrate = native`); a string literal is a build error (DDD-2). Any other identifier, including `guest`, is a build error naming it ("substrate `x` is not registered" / "guest substrates are not available to benchmarks"). The `native`-only registry is a scope limitation tracked by [#362](https://github.com/overdrive-sh/overdrive/issues/362). The record carries the substrate. Records of different substrates are never compared because no command takes two records: `bench-report` has `validate <record>` and `render <record>` only (DDD-6), and the report of one record states its substrate in its header.

### DDD-5 — The statistical protocol, methodology version 1

| Parameter | Value |
|---|---|
| Replicate | one fresh fixture; the body runs once |
| Minimum replicates per arm | 9 (refused below) |
| Block | every arm once, plus one A/A control (a second run of the reference arm, which is the first declared arm) when there are 2 or more arms; none for one arm |
| Order inside a block | seeded shuffle; the seed comes from OS entropy and is always recorded; there is no seed override |
| Series representation | no value is ever dropped. A series metric (declared `series`) of one replicate is stored as its raw values when it has at most 1,000, else as a histogram of all of its values (below). The switch is a representation choice, not a cap: the stored size is bounded (at most 2,049 sparse buckets) and the whole run's distribution is kept at any load |
| Histogram | log-linear, defined exactly in "Histogram and percentile computation" below. Stored sparse as `[bucket, count]` pairs with the exact `count`, `min` and `max` |
| Percentiles and quartiles | raw values: linear interpolation between closest ranks (the "R-7" method). A histogram, or pooled values of which any replicate is a histogram: the same R-7 over ranks, computed as pinned below. A histogram percentile is within one bucket width of the exact one (3.125% relative for values of 1 and above; absolute error below 1 for smaller values) |
| A/B statistic | difference of per-replicate medians (a series metric uses each replicate's median, exact from raw values, histogram-derived otherwise), non-reference arm minus reference arm. Only `ok` replicates feed any statistic: a `failed` replicate's partial metrics are recorded and never used |
| Interval | 95% percentile bootstrap, 10,000 seeded resamples of replicates, for the delta and for the A/A pair |
| Nonparametric test | Mann-Whitney U with a seeded 10,000-permutation p-value |
| Verdict | `distinguishable` iff the delta interval excludes 0 and the absolute point estimate exceeds the larger absolute bound of the A/A interval; else `not distinguishable from noise`; `insufficient replicates` if an arm, or the A/A control, has fewer than 9 `ok` replicates holding a stored value for that metric. A metric with no stored value at all in either compared arm has no comparison (DDD-6 item 4) |
| Distribution (one arm) | pooled values per metric: p50, p90, p95, p99, value count, replicate count, failure count, absent count |
| Canary | loopback TCP connect-and-close, 200 warm-up then 2,000 timed connects per sample, 16 samples at the start and 16 at the end, run before any fixture is built; start vs end under the verdict rule above, with the A/A floor taken from the alternating halves of the start set; a shift marks the run `unstable` |
| Failure accounting | a failed replicate, a failed metric and an absent metric are recorded and counted; none is dropped or zero-filled |

**Histogram and percentile computation (normative).** Two implementations must produce byte-identical `report.md` for the same record, so every step is fixed. All arithmetic is IEEE-754 `f64`, evaluated in the order written, with no fused multiply-add.

1. **Buckets.** Bucket 0 is `[0, 1)`: `lower` 0, `upper` 1. A value `v` below 1 (including 0 and every subnormal) falls in bucket 0. For `v` of at least 1, `e` is the unbiased binary exponent read from the `f64` bits (the `frexp` exponent minus 1; never computed with `log2`), and `s` is the top 5 bits of the 52-bit fraction, that is `floor((v / 2^e - 1) * 32)` exactly. The value falls in bucket `1 + 32e + s`, covering `lower = 2^e (1 + s/32)` and `upper = 2^e (1 + (s+1)/32)`. Bucket indices run `0..=2048`; a value at or above 2^64 clamps to bucket 2048. A negative, NaN or infinite value is a `ReplicateFailure` (measurement).
2. **Ranks.** Ranks are 0-based. For a histogram of `n` values, the value at integer rank `k` (`0 <= k < n`) lies in the bucket holding it; `r0` is the count of values in the lower buckets, `c` the count in that bucket, and `v(k) = lower + ((k - r0) + 0.5) / c * (upper - lower)`, evaluated as `((k - r0) as f64 + 0.5) / (c as f64) * (upper - lower)` and then added to `lower`.
3. **Clamp.** Each `v(k)` is clamped to `[min, max]` (the stored values) before it is used in step 4.
4. **R-7.** For a percentile `p` in `[0, 1]` (p50 is 0.5, p99 is 0.99): `h = (n - 1) * p`, `f = floor(h)`. If `f = n - 1`, the result is `v(n - 1)`; otherwise it is `v(f) + (h - f) * (v(f + 1) - v(f))`, with that expression order. Quartiles are `p` of 0.25 and 0.75. Raw values use the same formula with `v(k)` the `k`-th smallest value.
5. **Pooling.** Pooled values of an arm, where any replicate is a histogram, are binned: each raw value goes to its bucket, counts add per bucket, `min` is the minimum of the minimums, `max` the maximum of the maximums (a raw value is its own minimum and maximum) and `count` the sum of the counts.
6. **Error.** The percentile is within one bucket width of the exact one: at most 3.125% relative for values of 1 and above, and an absolute error below 1 for values under 1 (bucket 0). The report note says both (DDD-6 item 3).

The schedule is a pure function: `plan(decl, profile, seed) -> Schedule` returns the ordered runs and performs no effect; `execute` is the only impure step ("Plan-value pattern"). The property: every block holds every arm exactly once. The schedule, the statistics (percentiles, bootstrap, Mann-Whitney, verdict), the canary verdict, the host probe and the eligibility decision are internal structure of `overdrive-bench`: their names and signatures are not contracts, and their unit and proptest coverage is owned by the crafter in DELIVER. The pinned, observable surfaces are the record and the report (DDD-6).

### DDD-6 — Record, report and retention

Public API of `overdrive-bench` (contract):

```rust
pub enum Evidence { Maintained, Spike }
pub fn run_main(evidence: Evidence) -> std::process::ExitCode;   // maps `Error` to the exit statuses of DDD-9
pub fn smoke(decl: &BenchmarkDecl);          // called only by the macro-generated smoke test (DDD-2); panics naming the benchmark on any replicate failure; writes nothing; holds no statistics descriptor and asserts no timing

/// Crate error (`thiserror`); every variant names its cause, none flattens a typed source.
pub enum Error {
    Usage { reason: String },                                  // exit 64; no record, no result directory
    FamilyNotDeclared { family: String, found: Vec<String> },  // exit 64 (DDD-3); no record, no result directory
    Refused { setting: String, found: String, pinned: String },// exit 78 (DDD-7); `run_started` then `run_refused { setting, found, pinned }`
    Unreadable { setting: String, source: std::io::Error },    // exit 78 (DDD-7); `setting` `.overdrive-metal-source` writes no record (stderr only); any other setting writes `run_started` then `run_refused { setting, reason }`
    StatsEnable { source: aya::sys::SyscallError },            // exit 70 (DDD-8); `run_started` then `run_refused { setting: "bpf_stats", reason }`
    Record { path: std::path::PathBuf, source: std::io::Error },// exit 70; the record cannot be written, so it ends incomplete
    InvalidRecord { line: usize, reason: String },             // `bench-report validate`/`render`: exit 65
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub struct Replicate { /* opaque */ }
impl Replicate {
    pub fn arm(&self) -> &str;               // for the A/A control: the reference arm's name (the body cannot tell)
    pub fn profile(&self) -> &str;
    pub fn clock(&self) -> &HostClock;
    pub fn record(&mut self, metric: &str, value: f64) -> Result<(), ReplicateFailure>;                // a scalar metric, once per replicate; a second `record` of the metric, or a `record` of a `series` metric, is a `ReplicateFailure`
    pub fn record_series(&mut self, metric: &str, values: &[f64]) -> Result<(), ReplicateFailure>;   // a `series` metric; may be called more than once and the values accumulate; a scalar metric is a `ReplicateFailure`; every value is kept, raw or in the histogram (DDD-5)
    pub fn record_failure(&mut self, metric: &str, reason: &str) -> Result<(), ReplicateFailure>;
    pub fn record_absent(&mut self, metric: &str, reason: &str) -> Result<(), ReplicateFailure>;
    pub fn note(&mut self, key: &str, value: &str);                  // header context (cache state, versions, concurrency)
    pub fn exec(&mut self, program: &str, args: &[&str]) -> Result<String, ReplicateFailure>; // logged
    pub fn bpf_stats(&self) -> Option<&BpfStatsHold>;               // Some iff declared and run by the runner; None under smoke, where a stats-dependent metric is recorded with `record_absent`
}
pub struct HostClock; impl HostClock { pub fn now(&self) -> HostInstant; }
pub struct HostInstant; impl HostInstant { pub fn duration_since(self, earlier: HostInstant) -> std::time::Duration; }
pub struct BpfStatsHold; impl BpfStatsHold { pub fn read(&self, program_id: u32) -> Result<RunStats, StatsError>; }
pub struct RunStats { pub run_count: u64, pub run_time_ns: u64 }   // cumulative; callers take deltas
pub struct ReplicateFailure; // constructors: fixture(msg), measurement(msg); typed variants for undeclared metric and stats-not-effective
```

Contract notes: `record*` on an undeclared metric returns `ReplicateFailure` (the smoke test finds the typo). A replicate that returns `Err` is a failed replicate, counted, and the run continues. `exec` appends the program, arguments and status to the replicate's command log. The body never observes block, position or role. Ruling 3 (a benchmark never launches the `overdrive` binary) is enforced by the families' code, which drives in-process entry points, and by review; there is no runtime guard, and the command log records every `exec` for review. The connect-hook family calls no `exec` (it detaches through raw `bpf(2)`), so its command log is empty and the "no `overdrive` command appears in the command log" check holds vacuously for it; the ruling is enforced there by review of the family code.

**Record** `record.jsonl`, schema version 1, one JSON object per line, field `t` the line type; unknown fields refused on read; written append-only with a flush per line. Types: `string`, `bool`, `u64`/`u32`/`i32` (JSON integers), `f64` (JSON numbers), `null`; timestamps are RFC 3339 UTC to the second (`2026-11-17T10:00:00Z`); every field below is required unless marked optional; no field is ever omitted to mean `null`. Line order: `run_started`, then, unless refused, `canary` start, the `replicate` lines in schedule order and `canary` end, then exactly one terminal line (`run_finished` or `run_refused`) and nothing after it. A refused run is `run_started` then `run_refused` and nothing else. The terminal-line accounting: an error before `run_started` (`Usage`, `FamilyNotDeclared`, an unreadable checkout marker) writes no record and creates no result directory, prints one stderr diagnostic (`bench: refused: <setting>: <reason>` for the marker) and exits with the status of DDD-9; `Refused`, `Unreadable` (any setting but the marker) and `StatsEnable` write `run_started` then `run_refused`; `Record` and a kill leave a record with no terminal line (incomplete). Every record that exists has `run_started` first, so `commit`, `dirty` and `host` are never null.

| `t` | Fields |
|---|---|
| `run_started` | `schema` u32 (`1`); `methodology` u32 (`1`); `run_id` string (24 lowercase hex; from `OVERDRIVE_METAL_RUN_ID`, else generated); `family`, `benchmark` string; `evidence` `"maintained"` or `"spike"`; `substrate` `"native"`; `commit` string (40 hex) and `dirty` bool, both from the checkout marker `$OVERDRIVE_METAL_SOURCE_ROOT/.overdrive-metal-source` (`commit=` line; `dirty` is true iff its `source_digest=` differs from the SHA-256 of empty input, which is what `bootstrap.sh` computes for a clean tree); a missing, unreadable or malformed marker is `Error::Unreadable` (setting `.overdrive-metal-source`) read before the record exists, so it writes no record (above); `debug_assertions` bool (the closed `cargo bench` command prevents a debug build; the field records the bench binary's setting); `seed` u64; `started` timestamp; `host` object (below); `clock` object `{ "host": "CLOCK_MONOTONIC_RAW", "service": string or null, "guest": string or null }` (`service` is `"Clock::now (monotonic)"` for a benchmark that reads the injected clock, else null; `guest` names the guest duration source, else null); `bpf_stats` `"per_process"` iff the benchmark declares `bpf_stats` and the runner holds the descriptor for the run, else `"off"` (always `"off"` on a refused run, where none was held); `declaration` object (below) |
| `canary` | `phase` `"start"` or `"end"`; `samples_ns` array of f64: 16 values, each the mean connect-and-close time of one 2,000-connect sample, in ns (DDD-5) |
| `replicate` | `profile`, `arm` string; `block`, `position` u32; `role` `"arm"` or `"control"` (a control carries the reference arm's name in `arm`); `outcome` `"ok"` or `"failed"`; `failure` string, non-null iff `outcome` is `"failed"`; `metrics` object, metric name to `{ "values": [f64], "histogram": null or { "count": u64, "min": f64, "max": f64, "buckets": [[u32, u64]] }, "failures": [string], "absent": string or null }` (`values` holds the raw values, at most 1,000; `histogram` is non-null only for a `series` metric of more than 1,000 values, with `values` then empty; `buckets` are the sparse `[bucket, count]` pairs of DDD-5, at most 2,049, in ascending bucket order); `notes` object, string to string; `commands` array of `{ "program": string, "args": [string], "status": i32 or null }` (`null`: killed by a signal) |
| `run_finished` | `finished` timestamp; `replicates_ok`, `replicates_failed` u64 |
| `run_refused` | `setting` string (an identifier of the DDD-7 table, or `bpf_stats` for a statistics-enable failure), and either both `found` and `pinned` (strings) or `reason` (string); the others are omitted (the only optional fields in the schema). `found` and `pinned` for a difference or a setting with no source (`found` = `absent`); `reason` for an unreadable source or a failed statistics enable. A failed host probe (`host_probe`) is a `found` / `pinned` refusal (DDD-7) |

`host` has exactly these keys: `kernel`, `cpu_model`, `cpu_count`, `governor`, `smt`, `boost`, `bpf_stats_enabled`, `clocksource`, `irq_time_accounting` (DDD-7). Each value is one of three forms: a string; `{ "absent": "<reason>" }` for a setting with no source (absent by design, or, for a pinned setting, the source is missing and the run is refused); `{ "unreadable": "<error>" }` for a source that exists and cannot be read. Every key is always present: the runner reads every setting before it decides eligibility, so a refused run carries the full block and `run_refused` names the setting that refused (the first differing one in the DDD-7 table order). `cpu_count` is the decimal count of online CPUs, `boost` is `on` or `off` whichever source supplied it, `governor` is the governor shared by every online CPU, or, when they disagree, the distinct governors sorted and joined by `,` (for example `performance,schedutil`), which then also is the `found` of the refusal. `declaration` is `{ "arms": [string], "replicates": u32, "profiles": [{ "name", "preparation" }], "metrics": [{ "name", "unit", "table", "series": bool }], "references": [{ "metric", "value": f64, "label", "source" }] }`, an echo of the attribute (`unit` is one of `ns bytes bytes_per_second qps ratio count`; no profile declared echoes one profile `default` with empty `preparation`). A metric entry in an `ok` replicate holds exactly one of: non-empty `values`, a non-null `histogram`, a non-empty `failures`, or a non-null `absent`; a scalar metric's entry never holds a histogram and holds at most one value; a declared metric a replicate never recorded is written as one failure `not recorded`. A `failed` replicate may hold a partial `metrics` object.

A record without `run_finished` and without `run_refused` is reported incomplete. Intervals, p-values and verdict labels are not stored. `bench-report validate <record>` parses the typed schema and the semantic rules: first line `run_started`, line order above, `schema` and `methodology` equal to `1`, every replicate names a declared profile, arm and metric, a control carries the reference arm, `failure` non-null iff failed, `run_finished` counts equal the `replicate` lines, a histogram's bucket counts sum to its `count`, and each histogram satisfies: every bucket index is at most 2048, indices ascend and are distinct, every bucket `count` is at least 1, `histogram` is non-null only when `count` exceeds 1,000 and then `values` is empty, `min` is at most `max`, and `min` lies within the range of the lowest occupied bucket and `max` within the range of the highest occupied bucket (bucket 0 is `[0, 1)`; for the last bucket the upper end is unbounded). It prints nothing and exits 0 on success; on a violation it prints `bench-report: invalid record: line <n>: <reason>` to stderr, the reason naming the offending field or name, and exits 65 (`Error::InvalidRecord`). `bench-report render <record>` validates, then prints the report to stdout (same exit statuses). Exit statuses of `bench-report`: 0 success, 64 usage, 65 invalid record, 70 the record is unreadable.

**Report** `report.md`, a pure function of the record (same bytes for the same record). Structure, in this order; the wording quoted is asserted by tests:

1. `# Benchmark report: <family> / <benchmark>`, then a bullet list, one `- <key>: <value>` per line: `run id`, `family`, `benchmark`, `evidence` (`maintained`, or `spike evidence` for a spike record), `substrate`, `commit` (the hash), `dirty` (`true`/`false`), `methodology`, `seed`, `debug_assertions`, one line per `host` key (`kernel`, `cpu_model`, `cpu_count`, `governor`, `smt`, `boost`, `bpf_stats_enabled`, `clocksource`, `irq_time_accounting`; an absent value prints `absent: <reason>`, an unreadable one `unreadable: <error>`), `bpf stats` (`off` or `on (per process)`, from the record's `bpf_stats`; `off` on a refused run), `clock` (host, service and guest sources), `canary` (`steady`, or `unstable (start median <a> ns, end median <b> ns)`; the word `unstable` appears nowhere else; `not run` when the record has no `canary` line, as in a refused run; `not finished` when it has the start line and no end line, as in a killed run), `replicates ok`, `replicates failed` (counted from the `replicate` lines by `outcome`, so a killed run shows the count so far and a refused run shows 0 and 0; on a finished run they equal `run_finished`), and `status` (`finished`, `refused`, or `incomplete (no run_finished or run_refused)`; the word `incomplete` appears nowhere else). A refused run adds `- refused: <setting> found <found> pinned <pinned>` or `- refused: <setting>: <reason>` and has no further section.
2. For each profile, `## Profile: <name>` and its `preparation` text, then for each declared `table` label in declaration order `### Table: <label>`; a metric appears only under the table it declares.
3. Each table is a Markdown table with one row per arm (the A/A control is not a row). A metric is a series metric iff the declaration echo marks it `series`; otherwise it is scalar. Only `ok` replicates of the arm feed a row (a `failed` replicate's partial metrics are never used). Scalar columns: `metric | arm | median | IQR | replicates | failures`, the median and IQR (R-7 quartiles) taken over the `ok` replicates' values. Series columns: `metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures`, over the pooled values of the `ok` replicates (DDD-5). `replicates` is the number of `ok` replicates of the arm holding a stored value for the metric; `values` (series) is the pooled value count; `failures` is the number of `failed` replicates of the arm plus the number of `ok` replicates whose entry for the metric holds `failures`; an absent entry is counted in neither. When any entry of a series metric in the table is a histogram, a line `percentiles of <metric> are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)` follows the table. Numbers print with Rust's `f64` `Display` (no rounding, no thousands separators, an integral value without a decimal point). An arm with no stored value for the metric (every entry absent, failed or the replicate failed) prints one row whose value cells are `absent: <reason>`, the `<reason>` being the first absent reason in schedule order, else `no stored value`, and whose `replicates` is 0 and `failures` the count above.
4. With two or more arms, below each table a comparison table, one row per non-reference arm and metric for which both that arm and the reference arm have at least one stored value (a metric with no stored value in either arm, for example the mTLS handshake on the baseline arm or the ingress forward and return rows, has no comparison row: the arm row above prints `absent: <reason>` and no delta, interval or verdict is computed; a metric with one to eight values in an arm or the control keeps its row with the verdict `insufficient replicates`), columns `metric | arm | delta (<arm> minus <reference>) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict`; intervals print as `[lo, hi]`; the verdict cell is exactly `distinguishable`, `not distinguishable from noise` or `insufficient replicates` (DDD-5). A one-arm benchmark has no comparison table and no A/A text.
5. Each declared `reference` prints under its metric's table as `reference: <label>: <value> <unit> (source: <source>)`; the `label` carries the "different run" wording (the connect-hook label is "spike evidence, different run").
6. `## Failures`: one line per failed replicate, `- profile <p> block <b> position <n> arm <arm>: <reason>`, and one per metric failure, `- profile <p> block <b> position <n> arm <arm> metric <m>: <reason>`; the section is omitted when there are none.

**Retention (O4):** maintained results at `benchmarks/results/<family>/<run-id>/{record.jsonl,report.md}`, both committed (the raw record is the evidence, the report is derived from it); a spike writes under `spike-scratch/<name>/results/<family>/<run-id>/`. `benchmarks/catalogue.md` has one row per benchmark (family, boundary and arms, workload matrix, clock boundary, result owner; the substrate, the statistics flag and the caveats are in each record's header). A retained series is bounded by its representation (DDD-5): raw values up to 1,000 per series metric per replicate, else a histogram of at most 2,049 buckets, so a committed record stays small at any load and no value is dropped. `benchmarks/methodology.md` states version 1: runner isolation, host eligibility, clock boundaries, cold and warm handling, the sample and percentile method, the record format and failure accounting; a test asserts its stated version equals the code constant.

### DDD-7 — Host eligibility and the pinned test-host settings

Pinned list, methodology version 1 (O1 applied: ADR-0068 defines the kernel; the other values are the test host's, taken from the first accepted host recorded in `spike/findings-bpf-stats-overhead.md` and re-confirmed by the first accepted run; ADR-0068 defines no governor, SMT or boost value):

| Setting (the `setting` identifier in `run_refused`, in this order) | Source read | Pinned | On difference |
|---|---|---|---|
| `host_probe` (no host key): the probe's own self-check | `HostProbe::probe()` before any setting is read | probe succeeds | refuse; `found` = the probe's error, `pinned` = `probe succeeds`; every `host` key is `{"unreadable": "<probe error>"}` because no setting was read |
| `kernel` (`host.kernel`): kernel release series | `/proc/sys/kernel/osrelease` | `7.0` | refuse |
| `bpf_stats_enabled` (`host.bpf_stats_enabled`): `kernel.bpf_stats_enabled` | `/proc/sys/kernel/bpf_stats_enabled` | `0` | refuse |
| `governor` (`host.governor`): CPU governor, every online CPU | `cpu*/cpufreq/scaling_governor` | `schedutil` | refuse |
| `smt` (`host.smt`) | `/sys/devices/system/cpu/smt/control` | `on` | refuse |
| `boost` (`host.boost`): CPU boost | `cpufreq/boost`, else `intel_pstate/no_turbo` | boost on | refuse |
| `bpf_stats_holder` (no host key): `bpf-stats` descriptor holder | scan of `/proc/*/fd` for `anon_inode:bpf-stats` (name from `kernel/bpf/syscall.c:6014`, rendering at `fs/anon_inodes.c:76` @ v7.0) | none | refuse; `found` = `held by pid <n>`, `pinned` = `none` |
| `debug_assertions` (the `run_started` field): build | `cfg!(debug_assertions)` of the bench binary | off | refuse; `found` = `on`, `pinned` = `off` |
| Recorded only: `cpu_model`, `cpu_count`, `clocksource`, `irq_time_accounting` (image `CONFIG_IRQ_TIME_ACCOUNTING` where readable) | CPU model, online CPU count, clocksource | | record; never refuses |

The refusal (`Error::Refused`) names the first differing setting in the order above with the found and pinned values, writes `run_started` and `run_refused`, takes no sample, and exits 78; the full `host` block is in `run_started`, so every other value is retained. The probe runs first. When it succeeds, every setting is read before the decision, so no `host` key is ever missing. When it fails, no setting is read: the run writes `run_started` with every `host` key set to `{"unreadable": "<probe error>"}`, then `run_refused` with setting `host_probe`; `host_probe` is therefore first in the order above. A source absent by design is `{"absent": ...}` in `host`: a recorded-only row, and the unused alternative source of CPU boost (the `intel_pstate` directory on an AMD host). A pinned setting for which no source exists (no cpufreq directory, so no governor and no boost source; no `smt/control`) cannot be verified, so it is a difference: `host.<key>` is `{"absent": "<reason>"}` and the refusal is `Error::Refused` with `found` = `absent`. A pinned setting whose source cannot be read (permission, I/O) is `host.<key>` = `{"unreadable": "<error>"}` and refuses as `Error::Unreadable` with its own reason; an unreadable recorded-only source is recorded as `{"unreadable": ...}` and does not refuse. A failed `HostProbe::probe()` refuses as `Error::Refused` with `setting` = `host_probe`, `found` the probe's error and `pinned` = `probe succeeds`. When online CPUs disagree on the governor, `host.governor` and `found` hold the distinct governors sorted and joined by `,` (DDD-6). The `run_started` of a refused run has `bpf_stats` `off` (nothing was held).

### DDD-8 — Per-process BPF statistics

A benchmark that declares `bpf_stats` makes the runner call `aya::sys::enable_stats(Stats::RunTime)` once and hold the descriptor for the run. The sysctl is never written. Source: the command returns an anonymous-inode descriptor and raises the global key (`kernel/bpf/syscall.c:6002-6020` @ v7.0), release lowers it (`:5990-5996`), `CAP_SYS_ADMIN` is required (`:6030`), and the sysctl handler's saved value does not reflect descriptor holders (`:6533-6558`). Every arm of every comparison in the run therefore has the same setting. `smoke` does not hold the descriptor: `Replicate::bpf_stats()` is `None`, a stats-dependent metric is recorded `absent`, and the smoke test asserts no timing. The record states `per_process` or `off`. Spike V-1 (`7.0.0-38-generic`, AMD EPYC 8024P) confirmed it: the sysctl read 0 in every phase; 5,000 connects with the descriptor held gave a `run_count` delta of 5,000 per connect program (about 103 and 117 ns per run), and 1,000 connects with no holder gave 0; a scan from another process found the holder's pid with the exact target `anon_inode:bpf-stats`; after `kill -9` or a clean exit the scan was empty and the delta 0; with two holders, killing one left statistics on (reference-counted). The scan matches the target string exactly (other BPF descriptors carry `bpf-map`, `bpf-prog`, `bpf_link`, `btf`, so there is no false positive) and runs as root (0 unreadable `/proc/*/fd` directories).

### DDD-9 — The Metal benchmark form

`cargo xtask metal run --benchmark <FAMILY> [--spike <NAME>] [--no-sync] [--no-sudo]`. In `xtask/src/main.rs` the `Run` variant gains `benchmark: Option<String>` and `spike: Option<String>`; `--benchmark` conflicts with `--exclusive` and with any trailing command and requires none of them. `bootstrap.sh <target> --run --benchmark <FAMILY> [--spike <NAME>]` validates `FAMILY` against `^[a-z][a-z0-9-]{0,62}$` and `NAME` against `^[a-z0-9][a-z0-9-]{0,62}$` (and that `spike-scratch/<NAME>/Cargo.toml` exists after sync), sets execution class `exclusive`, and runs exactly (`$ROOT` is the exported `OVERDRIVE_METAL_SOURCE_ROOT`, `infra/metal/bootstrap.sh:779`):

```text
cargo bench -p overdrive-bench-families --bench benchmarks -- run --family <FAMILY> --results-root $ROOT/benchmarks/results
cargo bench --manifest-path spike-scratch/<NAME>/Cargo.toml --bench benchmarks -- run --family <FAMILY> --results-root $ROOT/spike-scratch/<NAME>/results   # with --spike
```

`cargo bench` runs with the package directory as its working directory, so the results root is always an absolute path under `$ROOT`; the runner refuses a relative `--results-root`, and a root outside `OVERDRIVE_METAL_SOURCE_ROOT`, as `Error::Usage` (exit 64). There is no seed option: the seed comes from OS entropy and is recorded (DDD-5). `bootstrap.sh` refuses a malformed benchmark request before the selected command runs, in the existing `OVERDRIVE_METAL_RUNNER_ERROR error=<name>` shape, status 64: `benchmark_family_invalid` (`FAMILY` fails its grammar, including empty), `benchmark_spike_invalid` (`NAME` fails its grammar, or `spike-scratch/<NAME>/Cargo.toml` does not exist after sync), and the existing `invalid_mode` for `--spike` without `--benchmark`, and for `--benchmark` beside `--exclusive` or a trailing command (where `xtask`'s own argument parser already rejects the combination first, so the refusal never reaches the box). The runner accepts and ignores the `--bench` flag Cargo injects and writes `<results-root>/<family>/<run-id>/record.jsonl` while it runs. Once that directory exists, and before the canary, it prints one line to stdout, `OVERDRIVE_BENCH_RESULT_DIR=<path>`, with `<path>` relative to `OVERDRIVE_METAL_SOURCE_ROOT` (so repo-relative: `benchmarks/results/<family>/<run-id>` or `spike-scratch/<name>/results/<family>/<run-id>`); it is therefore printed for a refused (other than an unreadable checkout marker, which writes nothing, DDD-6) or killed run too. After `run_finished` or `run_refused` the runner writes `report.md` in the same directory through the renderer `bench-report render` uses (one function, no subprocess); a killed run has no `report.md`, and `bench-report render <record>` produces its report on stdout. Retrieval is `cargo xtask metal pull <path>`. Runner exit status: 0 the run finished (failed replicates and an unstable canary are in the report, not the status); 78 refused (DDD-7, including an unreadable checkout marker); 64 usage; 70 internal (including a statistics-enable failure). Prerequisites stay separate commands (`bpf-build`, `guest-kit-prepare`, `kmod-load`).

**Evidence lane (user ruling 2026-11-17).** The runner's end-to-end run over a real replicate cohort is a benchmark, not a test (`.claude/rules/testing.md` § "Classify external execution"), and a product test is never `native_launcher`. No nextest test runs the runner over a cohort. DDD-7 and DDD-9 are evidenced by retained `cargo xtask metal run --benchmark <family>` runs under exclusive admission: a finished run of the connect-hook family (its committed record and report), and one run on a host deliberately drifted from a pinned setting whose `run_refused` record and exit 78 are retained (K4); host-eligibility refusal is shown only by that real run. In nextest the runner path is covered by the generated smoke tests (one replicate per arm, in a guest placement, no timing assertions, DDD-2) and by the record and report tests (`bench-report validate` and `render` on crafted records). The `xtask` benchmark-form grammar tests (the Metal runner's own behaviour, host-pure) are unaffected. No family test is `native_launcher`, and no nextest override for a families run is needed.

### DDD-10 — Clocks: host stamps and guest durations

One `HostClock` over `clock_gettime(CLOCK_MONOTONIC_RAW)` stamps every cross-boundary instant in a run. Guest-internal stages are durations the guest measured; they are never subtracted from or added to a host instant, and a missing one is `absent`. The DNS service time uses the injected `Clock::now()` (monotonic, slew-affected; `timekeeping.c:2163` @ v7.0) and the record names that clock.

### DDD-11 — VM lifecycle stage observation

Family `vm-lifecycle`, benchmark `vm_lifecycle_stages`, arms `["lifecycle"]` (one arm: a distribution benchmark), profiles `healthy-concurrency-1`, `healthy-concurrency-4`, `cold-cache`, `warm-cache`, `stop-grace-deadline` (names and preparation text pinned in DISTILL). Instruments (all existing public seams; no production change): a timestamping `BeaconIo` decorator installed in `NodeAdapters.vm.beacon`; a `Vmm` decorator installed in `NodeAdapters.vm.vmm` that reads the guest console log file `VmConfig::run_dir.console_log()` when the exit watch fires, before it forwards the exit (the run directory is removed afterwards); a `tracing` layer for the named events; a subscription to `ObservationStore::subscribe_all_events` held by the benchmark's injected observation store; the operator driving port (`overdrive-cli` deploy and stop calls) stamped before the call.

| # | Stage row | Start boundary | End boundary |
|---|---|---|---|
| 1 | admission | deploy call entry (benchmark stamp) | event `convergence.evaluation.admitted` for the workload target |
| 2 | vmm-launch | event `vm.lifecycle.create_enter` | event `vm.lifecycle.created` (process spawned; Cloud Hypervisor boots on its own after spawn) |
| 3 | guest-ready | `vm.lifecycle.created` | `READY` line received (decorator) |
| 4 | stable | `READY` received | allocation row with `terminal == Stable` received on the subscription (the allocation stays `Running`; `Stable` and `Running` are different states) |
| 5 | stop-submission | stop call entry (benchmark stamp) | event `vm.lifecycle.stop_enter` |
| 6 | shutdown-delivery | event `vm.lifecycle.stop_enter` | event `vm.lifecycle.writer_finished` (`SHUTDOWN` delivered to the guest and the beacon closed); metric `shutdown_delivery_ns` |
| 7 | guest-exit | event `vm.lifecycle.writer_finished` | event `vmm.process.reaped` (the guest handles the stop, powers off and the VMM exits; the in-guest part is the `guest.shutdown-handling` row); metric `guest_exit_ns` |
| 8 | cleanup-calls | `vmm.process.reaped` | event `vm.lifecycle.cleanup_calls_finished` (the calls returned; absence is not verified) |

The guest `EXIT` beacon line is not a boundary on the operator-stop path: the host never receives it (0 of 60 cycles, V-10). `VmDriver::stop` closes the beacon after delivering `SHUTDOWN` (`stop_beacon_writer`, `vm_driver.rs:874`), and the guest powers off only after the workload's process group completes (`overdrive-init/src/main.rs:939-943`), so the VMM reap follows workload exit. Holding the beacon open until `EXIT` was considered and rejected: no production consumer uses the stop-path exit status (stop moves the allocation to `EndingInFlight` first, so the exit watcher's `try_begin_ending` returns `None` and emits nothing, `vm_driver.rs:2349-2353`), so it would shape production for a benchmark. Ruled by the user 2026-11-17.

Guest-measured row: `guest.shutdown-handling` = `group-complete` minus `shutdown-received` from the guest lines `overdrive-init: <stage> elapsed_ms=<n>` (`crates/overdrive-init/src/main.rs:1000`), read from the console log file by the `Vmm` decorator (above); resolution 1 ms. `VmmDiagnostics::console_tail()` is not a source: it holds only the VMM's stderr, never the guest serial (V-4). Floor: a trivially stoppable guest reports 11 to 13 ms (V-4, 30 of 30 values), so the row is a runtime-independent floor for such a guest and tracks the guest's own exit delay above it (slow-exit guest: 11 to 54 ms across 11 distinct values). Header notes: cache state and its preparation, VM resources, concurrency, Cloud Hypervisor version (`exec`), host kernel. A stage that does not complete is a `record_failure` on that metric and the later stages of that replicate are absent. Stages with no boundary today, and why none is added: "API socket up" does not exist (the VMM boots from its command line, `overdrive-host/src/vmm.rs:272-310`); per-artifact cleanup has one aggregate event and the aggregate is what is reported.

**Guest image and data directory (V-4 fixture findings; no new pipeline; V-10 validates the boot).** The family boots the image the guest kit already builds for a `[vm]` spec: `GuestKit::provision(staging_root)` then `GuestKit::image(dir, name, &GuestImageSpec::workload())` (`crates/overdrive-testing/src/guest_kit.rs:358`, "the image a `[vm]` spec names"). That image carries the guest peer as the module-loading PID 1 at `/sbin/init` and `/init`, which loads the stock `dummy.ko` and execs the production `overdrive-init`, and carries `ovd_fwd.ko`; its guest program (`overdrive-guest-peer`, for example the `idle` subcommand) is the workload. The shared VM fixture's own image cannot run production `overdrive-init` (it lacks `dummy.ko` and `ovd_fwd.ko`) and is not used. The prerequisites are the existing separate commands, `cargo xtask kmod-build` and `cargo xtask guest-kit-prepare`; when they have not run, `provision` returns `GuestKitError::LaneNotPrepared` and the family refuses naming the command. Nothing is added to the kit. The confined VMM identity traverses every ancestor of its clone by name. The fixture therefore creates `<data_dir>` and the directories it makes beneath it under a scoped `umask(0o022)` that an RAII guard restores on drop (the mechanism `crates/overdrive-control-plane/tests/integration/guest_listener_real_boot.rs:143-149` uses, because the Metal runner's umask is 027 and `serve` runs under 022), and makes the root of `<data_dir>` mode `0755`. Group 4200 with mode `0710` stays only for the directories the guest kit owns (`WorkDir::create`, `guest_kit.rs:1288-1301`, under the staging root `provision` already makes traversable). The family never relies on the runner's umask for a directory the VMM must cross.
### DDD-12 — DNS per-query observer

Production-surface contract (crate `overdrive-control-plane`, module `dns_responder`):

```rust
pub trait QueryObserver: Send + Sync {
    /// Called once per datagram the responder handles, on the serving thread.
    /// `service_time` runs from the instant the receive returned to the instant the
    /// reply send (or the drop decision) completed, on the injected `Clock`.
    /// Must not block, panic or allocate unboundedly.
    fn observe(&self, outcome: QueryOutcome, service_time: std::time::Duration);
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueryOutcome { Answered, NoData, NameError, Dropped }
pub struct NoQueryObserver;   // QueryObserver with an empty body; the production composition passes it
// DnsResponder::new(store, clock, gateway, frontend, observer: Arc<dyn QueryObserver>)
```

The observer enters through `DnsResponder::new` only. `GuestDnsDeps` and `GuestDnsFactory` are unchanged; `HostGuestDnsFactory` (`dns_responder/mod.rs:102`) passes `NoQueryObserver`, and the benchmark constructs `DnsResponder` directly with its recorder. Behaviour with `NoQueryObserver` is byte-identical to today's. A send failure still calls `observe` with the attempted outcome (the existing `dns.responder.sendmsg_failed` event is unchanged). Hot-path budget: two `Clock::now()` reads and one dynamic call per datagram, no allocation, no lock. Confirmed by spike V-2 (`7.0.0-38-generic`, AMD EPYC 8024P): 62.7 ns per datagram with `NoQueryObserver` and 75.2 ns with a recording sink, in a 10-million-iteration microbenchmark. The end-to-end comparison (5,000 queries per second, 30 replicates, 6 million queries, none lost) found no difference distinguishable from noise (C minus A +111 ns, 95% interval [-165, +848]); its A/A floor of about 700 ns only rules out costs above that, so the microbenchmark is the cited bound. Family `dns-responder`, benchmark `dns_query_latency`, one arm `responder`; tables `client-observed` (`client_latency_ns` declared `series`, `client_throughput_qps` scalar) and `resolver-internal` (`resolver_service_ns` series). Load is scheduled-arrival (fixed schedule, latency from the scheduled send time, replies matched by DNS id, a timeout counted as a failure), with the arrival model, rate and duration written in each profile's `preparation`. The responder and the clients run in a netns the family creates (`overdrive-testing::NetNs`), so the host's own `:53` listener is irrelevant. Profile axes that exist in production: query shape (A on a resolvable name, AAAA on a resolvable name giving no data, an unknown name giving a name error), name-table cardinality, client socket count, arrival rate. Axes that do not: cache hit or miss (the name index is the authoritative map, `name_index.rs:257`), transport other than UDP (no TCP listener, `responder.rs:275-290`), response size (set by the query shape and recorded as observed).

### DDD-13 — mTLS boundary, rigs and baseline

Family `mtls-path`, benchmark `mtls_hop`; profiles `egress` and `ingress`; arms `["plaintext-baseline", "composed-mtls"]` (the baseline is the reference arm). Rigs (built in the families crate from public constructors): egress is the worker rig (`HostMtlsEnforcement`, real `HostMtlsIntercept`, `MtlsInterceptWorker`, a rustls mesh peer on a veth over a `TestCidrLease`; the dialer is plaintext, per CLAUDE.md); ingress is `HostMtlsEnforcement` serving `Routed::Inbound` over a loopback leg C with the guest side as an in-memory `GuestStreamDial`. Baselines: egress, the same client to a plaintext TCP peer on the same peer host and lease; ingress, a plaintext client to an equivalent in-memory sink. Measures, each its own row: connection establishment (client socket, both arms), handshake (`MtlsEnforcement::enforce` duration through a timing decorator injected into the worker; absent on the baseline arm), first-byte latency, sustained throughput, tail latency (closed-loop request and response, stated as closed-loop). Resource rows: whole-host CPU from `/proc/stat` with the user, system, IRQ and softirq split, memory, socket count and connection concurrency from the host and `/proc/self`. Forward and return rows (egress only) sum per-thread user and system time from `/proc/self/task/*/stat` by thread name over the sustained window; for ingress they are `absent: asynchronous relay on shared runtime threads`. The ingress number excludes the guest transport and ingress has no forward/return split: both are properties of this host-hop benchmark, the baseline for the comparison with in-guest kernel mTLS in [#303](https://github.com/overdrive-sh/overdrive/issues/303), which names #287 for that comparison. The ADR-0069 host proxy it measures is being replaced by #303, so widening this measurement is not planned work. Spike V-3 (egress enforcement path, one connection, 10 s per direction, `7.0.0-38-generic`) adds three requirements. (1) The family's own client and server threads are named (the `lg-` prefix in the spike) and the pump share is reported excluding them: pumps were 1.000 and 0.999 to 1.000 of the process delta excluding those threads, but 0.86 and 0.57 to 0.60 of the whole process. (2) A pump saturates one core (about 890 MB/s), so the lighter thread is below the 10 ms tick of `/proc/<tid>/stat` and the heavy-to-light ratio is a lower bound (at least 100 in both directions); the report says so. (3) The mesh-peer fixture sets `send_tls13_tickets = 0`, as production peers do; with tickets enabled the kTLS-RX return `splice` fails with EINVAL and the connection tears down. Each connection creates exactly two pump threads, which inherit the name `tokio-rt-worker` without DDD-14. The rig takes the node-global kernel-state lock and cleans the shared nft state, as the Tier-3 rigs do.

### DDD-14 — mTLS pump thread names

In `crates/overdrive-dataplane/src/mtls/splice.rs` the two synchronous pump constructors take one required thread-name argument, `name: &'static str`, and build their thread with `std::thread::Builder::new().name(name.to_owned())`:

```rust
pub(super) fn spawn_decrypt(src_fd: RawFd, dst_fd: RawFd, early: Vec<u8>, now_unix_nanos: u64, name: &'static str) -> std::io::Result<PumpHandle>;
pub(super) fn spawn_encrypt(src_fd: RawFd, dst_fd: RawFd, prelude: Vec<u8>, now_unix_nanos: u64, name: &'static str) -> std::io::Result<PumpHandle>;
```

`Builder::spawn` can fail, so the constructors return `io::Result`. A failure maps to `MtlsEnforcementError::Io` (the variant the enforcement trait documents for "`splice` pump setup", `?` through its `#[from] std::io::Error`), the existing per-connection establish error; the establish path closes both legs and, when the forward pump already started, stops and joins it before returning (fail-closed, no cleartext). Names are fixed strings under the 15-byte `comm` limit. The callers, enumerated from the code:

| Caller | Pump | Name |
|---|---|---|
| `outbound.rs:138` (`establish`) | forward encrypt | `mtls-out-fwd` |
| `outbound.rs:147` (`establish`) | return decrypt | `mtls-out-ret` |
| `outbound.rs:371` (outbound unit test, forward pump over a kTLS pair) | encrypt | `mtls-test-fwd` |
| `mod.rs:1496` and `mod.rs:1503` (`outbound_shaped_conn_state` test helper) | encrypt and decrypt | `mtls-test-primary` and `mtls-test-aux` |
| `mod.rs:1604` (reclaim-while-blocked unit test) | encrypt | `mtls-test-blocked` |

The inbound path calls neither constructor: its only pump is `PumpHandle::spawn_async` (`splice.rs:267`, caller `inbound.rs:85`), which is not changed and keeps its unnamed thread. No other behaviour changes. V-3 confirmed the heavy thread follows the transfer direction and is the first- or second-created pump. Asserted by a Tier-3 test that reads `/proc/self/task/*/comm` during an active egress connection.

### Connect-hook family (walking skeleton)

Family `cgroup-connect-hook`, benchmark `connect_hook_cost`, arms `["service-only", "service+gateway"]`, `replicates = 30`, `bpf_stats`, metrics `hook_ns_per_connect` (ns, table `hook-time`) and `wall_ns_per_connect` (ns, table `wall-time`), reference `hook_ns_per_connect` value `117.0` labelled "spike evidence, different run" with source `docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md`. Replicate body (the spike's fixture): a fresh cgroup and bpffs pin directory, `EbpfDataplane::new_with_pin_dir` with both connect programs. Arm A detaches the gateway link in process. `EbpfDataplane` holds its links in private fields (`_cgroup_connect4_gateway_link`, `lib.rs:418`) that only `Drop` releases, so it exposes no detach handle; the family adds none. It detaches through the kernel's link commands over raw `bpf(2)`, with no `bpftool` and no `rep.exec`: it finds the gateway program's cgroup link by enumerating link ids and reading each link's program name (`BPF_LINK_GET_NEXT_ID`, `BPF_LINK_GET_FD_BY_ID`, `BPF_OBJ_GET_INFO_BY_FD`) and issues `BPF_LINK_DETACH` on it (`kernel/bpf/syscall.c:5893-5900`, `:6324-6337`; cgroup links implement `.detach`, `kernel/bpf/cgroup.c:1464` @ v7.0). This works in a guest as well as on the host. Statistics are read through `bpf_stats()`, which is `None` under `smoke`: the metric `hook_ns_per_connect` is then `absent` and the smoke test checks the fixture, the detach and the connect loop, not timing. The loop is 500 warm-up then 5,000 timed loopback connect-and-close with `SO_LINGER` zero; `hook_ns_per_connect` is the summed `run_time_ns` delta of the connect programs over the largest `run_count` delta; `wall_ns_per_connect` is the timed loop divided by 5,000; teardown on drop. `smoke(guest = "cgroup-default")` is the placement (DISTILL confirms the group provides cgroup v2, bpffs and veth).

### Fault paths that have no nextest obligation

Four fault paths of the families need a fault no existing fixture produces. Each is ruled here; none adds a seam, a flag or a config surface. All four sit inside a family replicate or the runner, which run only in a retained `--benchmark` run (DDD-9, evidence lane), so none is a nextest obligation. What is contractual about them, that a failed or absent stage or metric is recorded and counted and never dropped or zero-filled, is covered by the record and report tests on crafted records (failed replicates, `failures`, `absent`) and by the crafter-owned runner unit tests (G1 ruling in DDD-5).

| Fault path | Reachable on a production path? | Ruling |
|---|---|---|
| Guest never reports `READY` (OQ-4) | Yes, by an image whose init never sends it, but no existing fixture image does that (the guest kit's workload image reports it) | No obligation. The family records the stage as a `record_failure` and the later stages absent (DDD-11); shown only if a retained run meets it. |
| Stop reaching its grace deadline (profile `stop-grace-deadline`) | Yes: a workload that ignores the stop signal. No existing guest-peer subcommand does | No nextest obligation. The profile's fixture is a guest-peer subcommand that ignores the stop signal, test-support surface (not a production seam) shaped in DELIVER as OQ-4 already states; it runs in the retained VM-lifecycle run only. |
| DNS query timing out | Yes, naturally: a datagram the responder drops (a malformed query) gets no reply through the real responder socket (`QueryOutcome::Dropped`) | No family-level obligation; the client counts a timeout as a failure (DDD-12). The responder's own `Dropped` observation belongs to the DNS observer tests, not to the family. |
| Pump thread spawn failing (DDD-14, `MtlsEnforcementError::Io`) | Only under thread or memory exhaustion; no existing fixture provokes it deterministically | No obligation: the mapping is `?` through `#[from] std::io::Error` by construction, and the close-both-legs and stop-and-join ordering is reviewed by the DELIVER reviewer. This is a residual: the branch has no executing test, and a test would need a seam, which is excluded. |

### Rule text the orchestrator edits (not the architect)

`.claude/rules/testing.md`: (d) the status-64 runner diagnostics list gains `benchmark_family_invalid` and `benchmark_spike_invalid` (DDD-9); (a) § "Running tests — native x86_64 bare-metal KVM host": the sentence "No test is ever run with `--exclusive`" gains "Benchmarks (the benchmark form) are exempt beside expectation captures"; (b) the command-grammar section names the benchmark form; (c) § "Classify external execution" row "Benchmark" cites `benchmarks/` and `cargo xtask metal run --benchmark`. `.claude/rules/development.md` § "xtask": no change (the Metal form adds no `overdrive-*` dependency to xtask).

## Wave: DESIGN / [REF] Technology choices

| Choice | Version and licence | Rationale and rejected alternatives |
|---|---|---|
| Rust, `serde`/`serde_json`, `rand 0.8`, `libc`, `thiserror`, `aya 0.13`, `tracing`, `hickory-proto` | existing workspace dependencies | no new dependency for these roles |
| `linkme` | MIT OR Apache-2.0, maintained by dtolnay | ADR-0298; rejected `inventory`, explicit list |
| Statistics in-crate (bootstrap, Mann-Whitney U, permutation p) | n/a | `statrs` and `criterion` rejected: Criterion measures an in-process closure; a permutation p-value needs no distribution function; the code is small and property-tested |
| JSON Lines record | n/a | append-only and crash-tolerant (`testing.md` diagnostic-history rule); rejected single JSON document |
| `clock_gettime(CLOCK_MONOTONIC_RAW)` | kernel | ADR-0305 |

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

| Signal or state | Owning component | Promise | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| "this run may take samples" | `overdrive-bench` runner | the host matches the pinned settings and the probes passed | `HostProbe`, build flags | any production lifecycle, health or reconciler state (none is touched) |

### Gate G-1 — host eligibility

- **Existing evidence:** none in production; the benchmark process starts under exclusive Metal admission (`bootstrap.sh`, execution class `exclusive`) and `run_main` is its only caller path.
- **Owner:** the `overdrive-bench` runner.
- **Promise:** every pinned setting matched and every probe passed before the first sample.
- **Affected state:** whether this benchmark process proceeds to the canary.
- **Failure projection:** setting differs, has no source, is unreadable, or the probe failed (`host_probe`, first in the DDD-7 order, with every `host` key `{"unreadable": ...}`) gives `run_started` then `run_refused` naming the setting and exit 78; an unreadable checkout marker gives no record, a stderr diagnostic and exit 78 (DDD-6); a statistics-enable failure gives `run_refused` with setting `bpf_stats` and exit 70; malformed input gives exit 64; cancellation (kill) leaves an incomplete record and, by kernel descriptor release, no statistics held.
- **Explicitly unaffected:** every production state and every other Metal run.
- **Ordering:** usage checks, marker, probe, settings read and eligibility, statistics, create the result directory and print `OVERDRIVE_BENCH_RESULT_DIR`, `run_started`, canary, run; no timeout of its own beyond the Metal lease.
- **Counterexample:** gating `kernel.bpf_stats_enabled` by the sysctl alone would pass a host where another process holds a descriptor; hence the descriptor scan.
- **Evidence lane:** crafter-owned unit tests with a fixture host probe (drift, absent, unreadable); a retained `--benchmark` run on a host with one deliberate drift (K4), the only evidence of the refusal on a real host (DDD-9, evidence lane).

## Wave: DESIGN / [REF] Formal protocol model

Not applicable: the runner is one sequential process with no state shared between owners, and its only crash-sensitive artifacts are an append-only file and a kernel descriptor that the kernel releases at exit (`kernel/bpf/syscall.c:5990-5996` @ v7.0); the Metal lease already serialises runs.

## Wave: DESIGN / [REF] Validation items

Contract-shaping items (a falsification changes a contract DISTILL pins) run on the Metal host before approval, as spikes in `spike-scratch/<name>/`, recorded in `spike/findings-<name>.md`. The orchestrator ran V-1 to V-5; the architect did not. No contract-shaping item is Open; V-6 to V-10 are DELIVER evidence, and V-10 blocks only the VM-lifecycle step; its result is recorded in its row.

| ID | Item | Class | Status |
|---|---|---|---|
| V-1 | Descriptor-scoped statistics behave as the source says (DDD-8, DDD-7 scan pattern) | contract-shaping | **HELD**, proven fact. [findings](spike/findings-v1-bpf-stats-fd.md) |
| V-2 | The DNS observer's hot-path cost is below the noise floor (DDD-12) | contract-shaping | **HELD**, proven fact: 62.7 / 75.2 ns per datagram by microbenchmark; the end-to-end comparison is not distinguishable from noise. [findings](spike/findings-v2-dns-observer.md) |
| V-3 | Per-thread CPU by thread role accounts for the pump work (DDD-14, DDD-13) | contract-shaping | **HELD, with caveats** (DDD-13). [findings](spike/findings-v3-pump-threads.md) |
| V-4 | Guest shutdown durations are recoverable from the diagnostics handle at usable resolution (DDD-11) | contract-shaping | **FALSIFIED as written**: the handle never holds the guest serial. **The console-log alternative HELD** and DDD-11 now uses it. [findings](spike/findings-v4-guest-shutdown.md) |
| V-5 | `linkme` collects registrations under the bench profile (DDD-3) | contract-shaping | **HELD, with one edge** (DDD-3 rules). [findings](spike/findings-v5-linkme.md) |
| V-6 | Connect-hook family reproduces the spike's hook-time delta within 115.5 to 126.5 ns (K2) | not contract-shaping | DELIVER evidence |
| V-7 | VM stage boundaries arrive in the table's order in every replicate | not contract-shaping | DELIVER; the existing native lifecycle test already depends on these events |
| V-8 | The egress and ingress rigs run unchanged under exclusive admission (no node view), and leftover nft state is cleaned | not contract-shaping | DELIVER |
| V-9 | The runner tolerates Cargo's injected `--bench` argument | not contract-shaping | DELIVER |
| V-10 | The guest-kit workload image (`GuestImageSpec::workload()`, `overdrive-guest-peer idle`) boots under production `overdrive-init` with the console-log `Vmm` decorator installed, and the `<data_dir>` fixture of DDD-11 lets the confined VMM identity reach its clone | not contract-shaping: the seam (the console log file) and the image source are pinned; a falsification changes only the VM-lifecycle fixture. Blocks only the VM-lifecycle DELIVER step | DELIVER. **Hypothesis:** the image reaches `READY` and both `overdrive-init: <stage> elapsed_ms=<n>` shutdown lines appear in `console.log`. **Prediction:** 60 of 60 cycles show `READY` and both lines (V-4 measured the shutdown lines in 60 of 60 cycles). **Falsification:** any cycle without `READY`, or without both lines, or a permission error on `<data_dir>`. **Result (`spike/findings-v10-guest-boot.md`, commit 0e024ab85):** `READY` and both shutdown lines 60/60; the guest `EXIT` line reaches the host 0/60 on an operator stop. DDD-11 stages 6 and 7 therefore end and start at `vm.lifecycle.writer_finished` instead of `EXIT` (see the DDD-11 table) |

### Proven facts

All five probes ran 2026-11-16/17 on the native Metal host (AMD EPYC 8024P, 8 cores, `uname -r` 7.0.0-38-generic, Cloud Hypervisor v53.0.0, rustc 1.95.0) at commit `e3b42c92e` with the DESIGN documents dirty, through `cargo xtask metal run --exclusive`; probes are `spike-scratch/nbp-v{1..5}-*/` and raw output sits beside them.

- **V-1:** `BPF_ENABLE_STATS` held by a descriptor makes `cgroup/connect4` programs count runs while the sysctl stays 0; it ends with the holder; the `anon_inode:bpf-stats` scan finds the holder (DDD-7, DDD-8).
- **V-2:** the `QueryObserver` hot-path budget is 63 to 75 ns per datagram (DDD-12).
- **V-3:** the two outbound pumps hold all of the process's non-fixture CPU, the heavy thread follows the transfer direction; the three caveats are pinned in DDD-13. Composition caveat: the probe called `HostMtlsEnforcement::enforce(Routed::Outbound)` in the `mtls_guardrails.rs` shape and reached the same `spawn_encrypt` / `spawn_decrypt` sites, but omitted the worker's leg-F accept loop; V-8 (DELIVER) runs the full worker rig.
- **V-4:** guest shutdown lines are recoverable from the console log file, not from `console_tail()` (DDD-11).
- **V-5:** `linkme` collects entries under the bench profile once the declaring crate is referenced (DDD-3).

## Wave: DESIGN / [REF] Open questions

None is contract-shaping and none needs a user decision; each goes to DISTILL or DELIVER.

- OQ-3 (DISTILL): the smoke placement for the DNS and mTLS families (connect-hook is `cgroup-default`, CA-1). They need a netns, so they are never `host_pure`; a guest group is chosen against the registry.
- OQ-4 (DISTILL): a fixture workload that reaches the stop grace deadline, and confirmation that production reaches that path for it. The guest peer's subcommands as read (`round-trip`, `hold`, `flood`, `udp-churn`, `idle`, `bind`, `listen`, `v6-peer`, `v6-client`, `run-with-modules`) include none documented to ignore the stop signal; a new subcommand is test-support surface for DISTILL to shape.
- OQ-5 (DISTILL): final profile and metric names for DNS, VM lifecycle and mTLS; the DNS scheduled-arrival rate and duration per profile.

## Wave: DESIGN / [REF] Changed Assumptions

- **CA-1.** `intake.md`: "The generated smoke `#[test]` carries `#[overdrive_test(...)]` with the benchmark's placement and is linted like any other test." Now: the smoke test carries its own placement from the `smoke(...)` argument, because the benchmark's substrate is native and `testing.md` forbids a product test from being `native_launcher`. Ruling 8 is unchanged. Accepted by the user 2026-11-17; the connect-hook smoke test is placed in `cgroup-default`, the DNS and mTLS smoke tests in a guest group chosen in DISTILL.
- **CA-2.** US-BENCH-06 elevator pitch: "sees guest receipt, workload exit and poweroff as guest-reported durations, each next to the host-stamped stage it sits in". Now: the guest reports one duration (shutdown handling: `shutdown-received` to `group-complete`); guest receipt is a host-send to guest-receive latency and cannot be a guest-clock duration, and `poweroff-requested` is emitted at the same instant as `group-complete` (`overdrive-init/src/main.rs:939-943`). Receipt and poweroff appear only as the host-stamped stage intervals (shutdown-delivery, guest-exit). Accepted by the user 2026-11-17; there is no host/guest clock-offset protocol (research F1.9 item 3 is not used).
- **CA-3.** US-BENCH-07: "profile (query shape, cache hit or miss, record cardinality, transport, client concurrency, response size)". Now: no cache concept exists in the responder, only UDP is served, and response size follows from the query shape; the profile axes are query shape, name-table cardinality, client count and arrival rate. Accepted by the user 2026-11-17; dropping the absent axes is a property of the mechanism, not a scope limit.
- **CA-4.** US-BENCH-02 scenario: "replicate order alternates between arms". Now: each block holds every arm once in a recorded seeded order (DDD-5).
- **CA-5.** Locked decision D5 and ruling 4: "The measurement core is a new dev-dependency-only `adapter-host` crate". Now: that crate plus a second dev-only crate for the families (DDD-1); the core remains the crate ruling 4 describes.
- **CA-6.** US-BENCH-09: forward-path userspace work apart from return-path splice work. Now: reported for egress; for ingress it is absent, because the inbound path is an asynchronous relay on shared runtime threads. This is a property of the host-hop benchmark, with no issue; see DDD-13 and [#303](https://github.com/overdrive-sh/overdrive/issues/303).
- **CA-7.** US-BENCH-05 stage "cleanup". Now: "cleanup-calls", the point at which the driver's cleanup calls returned; absence of every artifact is not verified.
- **CA-8.** US-BENCH-08: the ingress comparison. Now: it measures the host agent hop without the guest transport (DDD-13). This is a property of the host-hop benchmark, the baseline for the in-guest kernel mTLS comparison in [#303](https://github.com/overdrive-sh/overdrive/issues/303); no issue.

- **CA-9.** US-BENCH-09 acceptance criterion 1: "a per-cgroup figure is shown as secondary and labelled". Now: CPU is whole-host from `/proc/stat` only; no cgroup figure is reported (the report states why whole-host is the scope: interrupt and kernel-worker time is charged outside the benchmark's cgroup).

- **CA-10.** US-BENCH-04 domain example 2 and scenario "Results from different substrates are never compared": "the report refuses to compare them if the substrates differ". Now: no command takes two records, so no comparison exists to refuse; the substrate is in every record and report header (DDD-4). The scenario is dropped.

- **CA-11.** Port (4) and the DISTILL smoke scenarios: "`cargo nextest run` on `overdrive-bench-families`". Now: the generated smoke test is `#[cfg(feature = "integration-tests")]` and guest-placed (DDD-2), so a plain `cargo nextest run` neither builds nor runs it; it runs on a Metal integration run, `cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests`.

- **CA-12.** US-BENCH-03 scenario "Too few replicates are refused" (a run refused at run time) and the benchmark-level wording "refused below 9". Now: `replicates` is a literal in the attribute, so a value below 9 is a build-time error naming the minimum (DDD-2); no runtime refusal exists and no run record is written for it.

No `design/upstream-changes.md` is written; these entries are the requested story and acceptance-criteria changes for the product owner.

---

# DISTILL

DISTILL, 2026-11-17. Density `lean`, Tier-1 `[REF]` sections only. Rust, project rules override the skill's Gherkin and Python conventions: no `.feature` files; scenarios are specified in [distill/test-scenarios.md](distill/test-scenarios.md) and executed as Rust tests. Reconciliation: 0 contradictions between DISCUSS and DESIGN (CA-1 to CA-10 are recorded changes, approved); revised 2026-11-17 for the user ruling on the runner's end-to-end run, the DESIGN pins, and the final-review findings (semantic RED, Contract Shape per test, the histogram series, the refused-host and canary header shapes, only-`ok` statistics). Gaps and decisions: [distill/upstream-issues.md](distill/upstream-issues.md). Proof of RED: [distill/red-classification.md](distill/red-classification.md).

## Wave: DISTILL / [REF] Scenario list

Revised 2026-11-17 after the user ruling that the runner's end-to-end run is a benchmark, not a test (DDD-9, "Evidence lane"): 70 tests, 48 error or edge (69%); 67 pending RED, 3 GREEN guards (V1, V17, P3); every test carries a `CONTRACT_SHAPE:` line, recorded per row in `distill/test-scenarios.md`. The 21 Metal tests that ran the runner over a cohort are removed from nextest; their scenarios, and W12 (a missing checkout marker refuses with exit 78 and no record, which no nextest test can reach without running the runner), are in `distill/test-scenarios.md` § "Benchmark-run evidence (DELIVER, retained, not tests)", discharged by eight retained runs. Aligned the same day with the DDD-5 histogram pin and the DDD-7 `host_probe` row: R14 pins the report note and the exact percentile computation, R12c adds the failed-probe refusal, V17-V24 add the histogram validate rules, and the smoke probes each sit in their own family. Full table with traces, driving port, owner path, fault, oracle and lane per scenario: `distill/test-scenarios.md`. Summary:

| Group | Tests | Lane | Where |
|---|---|---|---|
| Walking skeleton, nextest side (benchmark form, smoke variant) | W7-W11 | xtask / int-host | `xtask/tests/acceptance/metal_run_isolation/benchmark_form.rs`, `overdrive-bench/tests/integration/smoke_variant.rs` |
| `bench-report render` / `validate`, methodology and catalogue | 52 (report 26, validate 24, docs 2) | int-host | `overdrive-bench/tests/integration/{report_rendering,report_validation,methodology_documents}.rs` |
| Declaration rules | 5 tests, 13 fixtures | trybuild | `overdrive-bench/tests/compile_fail.rs`, `tests/compile_fail/` |
| DNS observer, pump thread names | 4 | int-guest | `overdrive-control-plane/tests/integration/dns_query_observer.rs`, `overdrive-dataplane/tests/integration/mtls_guardrails.rs` |
| Metal benchmark form grammar | 4 (X1a-c, X2; + W7, W8) | xtask | `benchmark_form.rs` |
| Families, runner end to end | 0 tests; 22 evidence scenarios | retained `--benchmark` runs | `distill/test-scenarios.md` |

Tags: no Gherkin tags; the scenario table carries the lane and the state (RED or GREEN). Property-shaped criteria and pure internals (schedule, eligibility, statistics, canary, the writer's raw-or-histogram choice and the histogram encoder, the marker reader, argument parser) are crafter-owned unit and proptest coverage (G1, DDD-5). The substrate-comparison scenario is dropped (CA-10). Four fault paths carry no-obligation rulings (DESIGN, G6), recorded in `test-scenarios.md`.

## Wave: DISTILL / [REF] WS strategy

The walking skeleton is the connect-hook benchmark. Its end-to-end run (the runner binary against the real host: cgroup, bpffs, production connect programs, per-process statistics) is evidence: a finished `cargo xtask metal run --benchmark cgroup-connect-hook` run with its committed record and report, and one run on a deliberately drifted host that retains the `run_refused` record and exit 78 (K4). In nextest the skeleton is covered by the benchmark-form argv tests (W7, W8), the generated smoke tests (one replicate per arm, no timing; W9-W11 on synthetic benchmarks, the family's own smoke test generated from its declaration in `cgroup-default`) and the record and report tests on crafted records. Port treatment: driving adapters (the `bench-report` binary, the xtask and `bootstrap.sh` command lines, the attribute) are real; the DNS observer and pump-name tests run the real production owners in a guest. Policy file `docs/architecture/atdd-infrastructure-policy.md` is not used in this repository (the project's `testing.md` tiers are the policy). Slice 01 is the first to activate.

## Wave: DISTILL / [REF] Adapter coverage

| Driven adapter / boundary | Real-I/O scenario |
|---|---|
| `HostProbe` (procfs, sysfs, `/proc/*/fd` scan) | evidence run E-2 (descriptor holder refused), E-1 (host block recorded); unit coverage crafter-owned (G1) |
| `StatsEnabler` (`aya::sys::enable_stats`) | evidence runs E-1, E-3 |
| `RecordSink` (append-only file, flush per line) | evidence runs E-1, E-3 (incomplete record after SIGKILL); V1-V24 on crafted records |
| `HostClock` | evidence run E-1 (block, seed, canary retained); no probe (DDD) |
| `EbpfDataplane` + cgroup + bpffs (connect-hook fixture) | evidence run E-1; smoke test generated for the family (`cgroup-default`) |
| `DnsResponder` + `QueryObserver` | P1-P3; evidence run E-6 |
| mTLS pump threads | P4; evidence run E-7 |
| VM lifecycle seams (`BeaconIo`, `Vmm`, subscription) | evidence run E-5; smoke test generated for the family (`cgroup-default`) |

Costly-external adapters: none. The retained runs are point-in-time evidence, not the nextest-lane Mandate 6 scenarios; the guest-placed tests (P1-P4 and the generated family smoke tests) are the `@real-io` nextest coverage.

## Wave: DISTILL / [REF] Scaffolds

Exact shapes from the DESIGN contract only; each is marked `SCAFFOLD: true` and raises nothing a test cannot reach.

- `crates/overdrive-bench/` (new, `adapter-host`, dev-only): `src/lib.rs` (`Evidence`, `run_main` exits 70 printing a RED line, `smoke`, `Error` with the seven DDD-6 variants (`SubstrateMismatch` removed), `Result`, `Replicate`, `HostClock`, `HostInstant`, `BpfStatsHold`, `RunStats`, `ReplicateFailure`, `BenchmarkDecl` opaque), `bin/bench_report.rs` (accepts anything, prints nothing); alias `bench-report` in `.cargo/config.toml`.
- `crates/overdrive-bench-families/` (new): `benches/benchmarks.rs` as DDD-1, `src/families/cgroup_connect_hook.rs` carrying the pinned declaration with a body that returns a fixture failure. No `tests/` directory and no dev-dependency: the crate keeps no hand-written test. Each family slice adds the production crates it drives.
- `crates/overdrive-testing-macros`: `overdrive_bench` as a passthrough attribute (validates nothing, registers nothing, emits no smoke test).
- `crates/overdrive-control-plane/src/dns_responder/`: `QueryObserver`, `QueryOutcome`, `NoQueryObserver` as DDD-12, and the `observer` parameter on `DnsResponder::new` (stored, not yet called). Two callers updated.
- Workspace: both crates in `members` and `default-members`. No nextest override (the families `host-kernel-shared` assignment and 300 s slow-timeout were removed with the families tests).
- Not scaffolded: DDD-14's pump signatures (the test asserts behaviour through the public enforcement port), `linkme`, statistics, eligibility, the Metal `--benchmark` flag.

## Wave: DISTILL / [REF] Test placement

Precedent: `overdrive-testing/tests/{compile_fail,integration}.rs`, `xtask/tests/{acceptance,integration}`, `overdrive-control-plane/tests/integration/dns_responder_bind.rs`. `overdrive-bench` integration binary: `overdrive_test_default!(host_pure)` (subprocess of `bench-report` and of the test binary, no kernel state); trybuild tests `#[overdrive_test(host_pure)]` + `#[serial(trybuild)]`. xtask module: `placement!(host_pure)`. DNS observer: `guest = "cgroup-default", profile = "normal-gate"` (OQ-3 resolved for DNS). Pump names: `guest = "mtls-relay"`. Family smoke tests (macro-generated from each declaration, DDD-2): connect-hook, `vm-lifecycle` and `dns-responder` in `cgroup-default`, `mtls-path` in `mtls-relay` (OQ-3 resolved). No test is `native_launcher` and the families crate has no test binary. `cargo xtask placement-lint`: clean.

## Wave: DISTILL / [REF] Driving adapter coverage

| Driving port (DESIGN) | Scenario |
|---|---|
| 1 `#[overdrive_bench]` attribute | T1-T4, W9-W11 |
| 2 `cargo xtask metal run --benchmark` | W7, W8, X1, X2 (grammar and argv); the full run is evidence runs E-1 to E-7 |
| 3 `cargo xtask metal pull <dir>` | unchanged existing contract; not re-tested |
| 4 `cargo nextest run` smoke | W9-W11 |
| 5 spike crate run with `--spike` | W8 (argv); evidence run E-4 (record) |
| 6 `cargo bench-report` | R1-R19 (R12c: failed host probe), V1-V24 (V18-V24: one malformed case per histogram rule) |

Every CLI/command-line entry point that nextest can reach has a subprocess scenario; none spawns the `overdrive` binary (the command logs of evidence run E-1 are read for that).

## Wave: DISTILL / [REF] Pre-requisites

`OVERDRIVE_METAL_TARGET`, `OVERDRIVE_METAL_KERNEL`, `OVERDRIVE_METAL_ROOTFS`; `cargo xtask bpf-build` and `cargo xtask guest-kit-prepare` on the host before the guest-placed tests (DNS observer, pump names, the generated family smoke tests); `kmod-load` and a prepared guest kit before the VM-lifecycle evidence run. DEVOPS artifacts absent: default matrix. DELIVER regenerates the trybuild `.stderr` span lines once (`TRYBUILD=overwrite`) and reviews that the pinned message lines are unchanged. The pig-connect-cost re-expression (K5) is a DELIVER change under `spike-scratch/`.

## Wave: DISTILL / [REF] Completeness audit

15-item checklist, applicable items: C1a, C1b (8/5 replicates, family grammar 63/64, 1,000 raw values against a 2,000-value histogram in R14; the writer's switch at 1,000 is crafter-owned and observed in evidence run E-6), C3 (one-arm, absent, no stored value in a compared arm, empty series) pass; C2a/C2b n/a (no state machine); C4a (render twice) pass, C4b n/a; C5a (spike/maintained, stats on/off) pass, C5b n/a; C6a, C6b, C6c (malformed record, host value forms, metric entry shape, usage errors never 101; the no-marker refusal is evidence W12) pass; C7a (absent metrics, no cpufreq, an unreadable recorded-only setting) partial (G8), C7b (SIGKILL: incomplete record with `not finished`, R11) pass, C7c n/a (exclusive admission). 12 pass, 3 n/a counted as passing with the rationale above: ACCEPTABLE_WITH_DOCUMENTED_GAPS. Gaps G1 (crafter-owned unit coverage), G3 (dropped, CA-10), G6 (no-obligation rulings) and G8 (host probe unit coverage plus evidence run E-2) are resolved or ruled; C7b (SIGKILL) is evidence run E-3 with R11 in nextest. Per-wave review not run by this agent.
