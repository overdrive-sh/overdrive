<!-- markdownlint-disable MD024 -->
# Feature delta — `guest-benchmark-substrate`

DISCUSS, 2026-11-19. Infrastructure (developer tooling), brownfield; extends `native-benchmark-program`. Density `lean`, Tier-1 `[REF]` sections only. One job (`J-PLAT-008`, extended, not new), four stories in four slices; slices 01 and 02 are the walking skeleton. Anchor: [GitHub issue #362](https://github.com/overdrive-sh/overdrive/issues/362). Inputs: `intake.md`, `docs/feature/native-benchmark-program/` (DISCUSS, ADR-0299, ADR-0308, `benchmarks/methodology.md`, `benchmarks/catalogue.md`), the guest kit (`crates/overdrive-testing/src/guest_kit.rs`, `guest_lane.rs`, `placement/groups.rs`), the shipped benchmark crates (`overdrive-bench`, `overdrive-bench-families`) and `.claude/rules/testing.md`. Requirements only: DESIGN owns every interface shape; this document names behaviours and observable outputs. DIVERGE and DISCOVER artifacts are absent; the approach comes from #362. Noted risk: no alternative to the guest-kit approach is compared in a DIVERGE record.

## Wave: DISCUSS / [REF] Persona ID

`marcus-platform-verification-engineer` — Marcus, the Overdrive engineer who decides between mechanisms or checks what a primitive costs; coding agents run the same commands for him, so every pitch is also what an agent sees. Trigger added for this feature: a component he wants to measure runs inside a workload VM in production, so a native number does not describe it. [Persona](../../product/personas/marcus-platform-verification-engineer.yaml).

## Wave: DISCUSS / [REF] JTBD one-liner

`J-PLAT-008` (existing, contract text unchanged): When I must choose between two mechanisms or check what a primitive costs end to end, I want to declare the boundary and its arms once and run it on the shared native host under one fixed method, so I can decide from retained raw samples and a report with distributions and host state that anyone can reproduce and compare. For a component that runs inside a workload VM, "run it" means run it in a guest the same way production runs one. [Job register](../../product/jobs.yaml). Journey: [measure a boundary or compare two mechanisms natively](../../product/journeys/measure-a-boundary-natively.yaml) (extended with the guest-view and vCPU-scheduling artifacts).

## Wave: DISCUSS / [REF] Premise and evidence

| Fact | Source |
|---|---|
| The substrate registry is closed to `native`; a benchmark declaring `guest` fails to build with "guest substrates are not available to benchmarks". | `crates/overdrive-bench/src/registry.rs` (`SUBSTRATES`, `refusal_pieces`); `tests/compile_fail/guest_substrate.rs`; ADR-0299 |
| The record has a `Substrate` type with one variant, and a fixed nine-key `host` block that refuses unknown keys. Every committed record says `native`. | `crates/overdrive-bench/src/record.rs` (`Substrate`, `HOST_KEYS`); `benchmarks/results/**/record.jsonl` |
| Host eligibility refuses unless governor, SMT and boost read as pinned values, and "a pinned setting with no readable source cannot be verified and refuses". A guest has no cpufreq or boost source, so the existing list would refuse every guest. | `crates/overdrive-bench/src/host.rs` (`decide`); `benchmarks/methodology.md` § Host eligibility |
| The benchmark form runs `cargo bench -p overdrive-bench-families --bench benchmarks -- run --family <F> --results-root <checkout>/benchmarks/results` as one host process under exclusive admission. | `infra/metal/bootstrap.sh` (benchmark form); `.claude/rules/testing.md` |
| A runner half inside a guest has a precedent: the in-guest lane launches a guest-kit microVM, plays the node beacon, runs one prepared libtest cell inside it and reads its result from the console. The kit launches the guest through the production `CloudHypervisorVmm`, defaults to 2 vCPUs and sets no pinning. `cargo xtask guest-kit-prepare` stages the kernel, the module and the in-guest binaries; a missing one refuses `LaneNotPrepared` naming that command. | `crates/overdrive-testing/src/guest_lane.rs`, `guest_kit.rs` (`vcpus: extra.vcpus.unwrap_or(2)`), `xtask/src/guest_kit_prepare.rs` |
| What runs inside a workload VM in production today: `overdrive-init` as PID 1 (transport configuration, the guest flow owner on its own thread, READY beacon, workload exec) over the `ovd_fwd` forwarder module. The guest flow owner is the guest half of the vsock attachment path. | `crates/overdrive-init/src/main.rs` (`start_guest_flow_owner`); `crates/overdrive-init/src/guest_flow/mod.rs`; `guest_kit.rs` module doc |
| In-guest kernel mTLS ([#303](https://github.com/overdrive-sh/overdrive/issues/303)), the stated first consumer, is not implemented. The host proxy and a temporary vsock relay carry mesh traffic until it lands; ADR-0186 "is deleted when #303 lands" and ADR-0308 calls the host proxy "being replaced". `grep` finds no in-guest mTLS code under `crates/`. | ADR-0186; ADR-0308; ADR-0069 |
| The shipped mTLS family measures the host agent hop and excludes the guest transport; it is the baseline #303 compares against. | ADR-0308; `benchmarks/catalogue.md` row `mtls-path` |
| `smoke(guest = "cgroup-default")` already runs a native family's body inside an outer test guest for one replicate per arm. That is a smoke placement, not the measuring substrate. | `crates/overdrive-bench-families/src/families/cgroup_connect_hook.rs:56`; methodology § intro |

Ground-the-premise check (`development.md`): every guest family must measure a component that runs inside a workload VM in production, reached through the production benchmark form. The first consumer named in #362, in-guest kernel mTLS, has no production code yet, so it cannot be the first guest family; a family built on it would defend a state no production path produces. The component that does run in production guests today is the `overdrive-init` guest flow owner over `ovd_fwd`. Which observable of it the first guest family measures is open point O1 and needs the user's confirmation. The `cgroup-connect-hook`, `dns-responder`, `mtls-path` and `vm-lifecycle` families measure host components and stay native; none runs in a workload VM in production.

## Wave: DISCUSS / [REF] Locked decisions

Requirement-level only; each is either from #362, the intake or a parent ruling that still stands. Nothing architectural is locked here.

| ID | Decision | Source |
|---|---|---|
| D1 | Feature type: infrastructure / developer tooling, brownfield. UX research depth: lightweight. JTBD: yes (`J-PLAT-008`). Walking skeleton: yes, real and end to end. | Dispatch |
| D2 | A second substrate, `guest`, is added to the closed registry; `native` is unchanged. A benchmark declares it the same way it declares `native`. The "guest substrates are not available" refusal goes away; an unknown substrate is still refused by name. | #362 |
| D3 | A guest benchmark runs through the existing benchmark form, `cargo xtask metal run --benchmark <family> [--spike <name>]`, under exclusive admission. No new command. | #362; parent D2, D10 |
| D4 | The runner measures inside a guest-kit guest on the Metal host. A host half collects what the guest half measured and writes it in the same record schema and report. Statistics, interleaving, the A/A control (where the family has two arms) and the 9-replicate minimum apply unchanged. | #362 |
| D5 | The guest benchmark's guest is launched directly on the Metal host by the benchmark form, as production launches a workload VM. It is not run inside an outer test guest, so the measurement is not nested. The record states this. | #362 ("nested virtualization, which production does not have"); ADR-0299 |
| D6 | The record carries the guest's view of its settings. A setting a guest cannot see (turbo, SMT, frequency) is recorded absent with the reason, never defaulted or zero-filled. The host half's reading of the real host is recorded beside it, and a drifted host is refused before any guest boots. | #362; methodology § Failure accounting |
| D7 | The record states the guest's vCPU count and that vCPU scheduling is unpinned (the guest kit does not pin today). | #362 |
| D8 | Results of different substrates are never compared. The record and report state the substrate and no command takes two records. | #362; ADR-0299; parent CA-10 |
| D9 | VM-lifecycle families are not guest benchmarks: a guest that launches VMs measures nested virtualization, which production does not have until nested virtualization for VM workloads lands (#347). `vm-lifecycle` stays native. Enforced by the catalogue, the methodology and review; no mechanical check (user ruling 2026-11-19). | #362, #347 |
| D10 | Every guest family measures a component that runs inside a workload VM in production, driven in-process (a benchmark never launches the `overdrive` binary). | `development.md` § Ground the premise; parent D4 |
| D11 | Out of the feature: regression gating and baselines ([#361](https://github.com/overdrive-sh/overdrive/issues/361)), in-guest kernel mTLS itself and its comparison with the host proxy ([#303](https://github.com/overdrive-sh/overdrive/issues/303)), and any change to the existing native families or their committed records. | #362; parent D12 |
| D12 | Names describe behaviour, never roadmap steps or issue numbers in identifiers (`domain-naming.md`). | Project rule |

## Wave: DISCUSS / [REF] User stories with elevator pitches

All stories: `job_id: J-PLAT-008`, persona Marcus. The guest family in the examples uses the working name `guest-flow-forwarding`; the real family, its arms and its observables are open point O1. Output lines are examples of content; DESIGN fixes exact formatting.

### US-GBS-01 — Run a component that lives in a workload VM inside a guest and get the same record (walking skeleton)

Job: J-PLAT-008. Slice: [01](slices/slice-01-guest-family-runs-in-a-guest.md). Must-have (#362 items 1 and 2).

Marcus wants the cost of the guest flow owner's forwarding as it runs inside a workload VM. Today every benchmark runs on the host, so a number for an in-guest component would describe the wrong place, and a hand-written spike would need its own harness again.

### Elevator Pitch
Before: a benchmark declaring a guest substrate fails to build, so a component that runs inside a workload VM can only be measured by a hand-written spike on the host or in an unrecorded guest.
After: run `cargo xtask metal run --benchmark guest-flow-forwarding` → sees the run id, then `OVERDRIVE_BENCH_RESULT_DIR=benchmarks/results/guest-flow-forwarding/<run-id>`, and after `cargo xtask metal pull` a `record.jsonl` and `report.md` whose header reads `substrate guest` with per-arm median, IQR and replicate count derived from the retained series, and the commit with its dirty flag.
Decision enabled: Marcus decides the number for an in-guest component can be taken with the declared benchmark and stops writing per-spike guest harnesses.

Domain examples:
1. Marcus declares `guest-flow-forwarding` with `substrate = guest` and 30 replicates; the run boots a guest, measures inside it, and `report.md` lists the arm's median and IQR with `substrate guest`.
2. The same record passes `bench-report validate`, the validator that checks native records, with no guest-only flag.
3. Marcus runs `cargo xtask metal run --benchmark vm-lifecycle`; its report still says `substrate native` and its committed records still validate.
4. The guest kit was not prepared; the run refuses before any sample, naming `cargo xtask guest-kit-prepare` (superseded by CA-4: every replicate fails with `guest boot: <cause>` naming that command, no guest boots, no sample is taken; not a refusal).

```gherkin
Scenario: A guest benchmark runs inside a guest and returns a retained record
  Given Marcus declared the guest-flow-forwarding benchmark with the guest substrate
  When he runs the benchmark form for family guest-flow-forwarding
  Then each replicate's series is retained in the machine-readable record (raw values up to 1,000 per replicate, a histogram above) and the raw block is in the retained console
  And the report shows per-arm median, IQR and replicate count with substrate guest and the commit

Scenario: The guest record is the same schema as a native record
  Given a retained guest record
  When Marcus validates it with the same validator as a native record
  Then it validates and the report statistics are derived only from its retained samples

Scenario: The guest is launched directly on the host, not inside a test guest
  When the guest benchmark runs
  Then the record states the guest was launched on the Metal host and was not nested

Scenario: The existing native families are unchanged
  Given the committed records of the four native families
  Then each still validates and states substrate native

Scenario: VM-lifecycle stays a native benchmark
  Then the vm-lifecycle family declares the native substrate
  And the catalogue states that a family that launches VMs is not a guest benchmark

Scenario: An unprepared guest kit refuses before any sample (superseded by CA-4: failed replicates, not a refusal)
  Given the guest kit has not been prepared
  When Marcus starts the guest benchmark
  Then the run refuses naming cargo xtask guest-kit-prepare and takes no sample
```

AC: (a) a family declared `guest` runs through the benchmark form and retains each replicate's series in the record (raw values up to 1,000 per replicate, a histogram above; the raw block lives in the retained, uncommitted console, PD-9) plus a report, with the substrate stated; (b) the record validates with the existing validator and no second schema; (c) the record states the guest was launched on the host and not nested (D5); (d) native families and their committed records are unchanged; (e) `vm-lifecycle` declares native and the catalogue states the D9 rule; (f) an unprepared kit refuses naming the setup command, before any sample (superseded by CA-4: failed replicates, not a refusal); (g) the catalogue has a row for the guest family naming workload matrix, clock boundary and result owner; (h) the guest family drives the production in-guest component in-process and no launch of the `overdrive` binary appears in its command log. Notes: the guest clock is the guest's own; a guest duration is never added to or subtracted from a host instant (methodology § Clock boundaries).

### US-GBS-02 — Know what the guest could see, what the host was, and how the vCPUs were scheduled (walking skeleton)

Job: J-PLAT-008. Slice: [02](slices/slice-02-guest-view-and-vcpu-scheduling.md). Must-have (#362 item 3).

A guest cannot see turbo, SMT or frequency. Marcus must be able to tell from the record which host state the number came from, and that two vCPUs sharing host CPUs with other threads may have moved it.

### Elevator Pitch
Before: a guest's reading of governor, SMT and boost is empty or misleading, and nothing in a record says how its vCPUs were scheduled.
After: run `cargo xtask metal run --benchmark guest-flow-forwarding` on a conforming host → sees a report host block listing the real host's kernel, governor, SMT and boost, a guest block listing the guest's kernel, clocksource and vCPU count `2` (read from the `guest_observed` line; `absent: no replicate returned a guest view` when every replicate failed before reporting one, CA-4) and `vcpu scheduling: unpinned`, with the settings the guest cannot see shown as `absent (not visible from a guest)`; on a host whose `kernel.bpf_stats_enabled` reads 1 → sees a refusal naming the setting with both values and no guest booted.
Decision enabled: Marcus decides whether a guest result is comparable with another guest result, and whether unpinned vCPUs could explain a spread he sees.

Domain examples:
1. A conforming run records host kernel `7.0.0-38-generic`, governor `schedutil`, SMT on, boost on, and a guest block with 2 vCPUs, `unpinned`, governor `absent (not visible from a guest)`.
2. The host reads `bpf_stats_enabled` 1; the run refuses naming `kernel.bpf_stats_enabled` (found 1, pinned 0) before booting any guest.
3. The report places the guest block under the host block; nothing in either is zero-filled where a source did not exist.

```gherkin
Scenario: A drifted host is refused before any guest boots
  Given a host setting differs from the pinned settings
  When Marcus starts a guest benchmark
  Then the run refuses naming the setting with its found and pinned values
  And no guest is booted and no sample is taken

Scenario: The record holds the real host's settings beside the guest's view
  Given a conforming host
  When a guest benchmark completes
  Then the record and report list the host's settings and, separately, the guest's kernel, vCPU count and clock source

Scenario: Settings a guest cannot see are recorded absent with a reason
  When the guest block is written
  Then governor, SMT and boost appear as absent with the reason "not visible from a guest" and never as a default or zero

Scenario: vCPU count and scheduling are marked
  Then the record and report state the guest's vCPU count (from the guest_observed line) and that vCPU scheduling is unpinned
  And when every replicate failed before reporting a guest view, the kernel, clocksource and vCPU count read "absent: no replicate returned a guest view" (CA-4)

Scenario: A guest result is never compared with a native result
  Given a guest record and a native record for different families
  Then each states its own substrate and no command takes both records
```

AC: (a) a drifted host is refused before any guest boots, naming setting, found and pinned value; (b) host block and guest block both in record and report; (c) guest-unseeable settings absent with the stated reason, none defaulted; (d) vCPU count and `unpinned` scheduling stated; the kernel, clocksource and vCPU count come from the `guest_observed` line and read `absent: no replicate returned a guest view` when every replicate failed before reporting a view (CA-4); (e) no command compares records across substrates. Notes: the pinned list as applied to a guest is open point O2; the existing rule would refuse every guest. The record's host block currently refuses unknown keys, so adding the guest block must keep every committed native record valid (O4).

### US-GBS-03 — See a guest that fails to boot, dies or returns nothing as a reported failure

Job: J-PLAT-008. Slice: [03](slices/slice-03-guest-failures-are-reported.md). Must-have (failure accounting is a methodology rule: nothing dropped or zero-filled).

A guest adds ways to fail that a host process does not have. Marcus must not mistake a guest that never ran for a fast one, and a killed run must leave no guest behind.

### Elevator Pitch
Before: a guest benchmark has no defined failure shape; a guest that hangs could be read as an empty or fast result and could survive the run.
After: run `cargo xtask metal run --benchmark guest-flow-forwarding` where the guest dies during replicate 12 → sees replicate 12 counted failed with its reason and its console retained, replicates 13 to 30 run, the run finishes with its terminal line, and no guest VMM is left running afterwards; a report is "incomplete" only when the benchmark process itself is killed before its terminal line (CA-1).
Decision enabled: Marcus decides whether a result is complete enough to cite and where to look (console) when it is not.

Domain examples:
1. The guest kernel image is missing; the record holds a `guest boot` failure naming the missing path and no sample, and the run does not fall back to measuring on the host.
2. The guest dies at replicate 12 of 30; replicate 12 is a counted failure with its console retained, replicates 13 to 30 run and the run finishes with its terminal line (CA-1). A run is reported incomplete only when the benchmark process itself is killed.
3. One replicate's fixture fails inside the guest; it is a counted failure with its arm and replicate, and the remaining replicates are reported.
4. The run is killed from the host; the next run finds no leftover guest VMM.

```gherkin
Scenario: A guest that cannot boot is a reported failure, not a host measurement
  Given the guest cannot boot
  When Marcus runs the guest benchmark
  Then the record names the boot failure and holds no sample
  And no measurement is taken on the host in its place

Scenario: A guest that dies mid-run fails that replicate and the run finishes (CA-1)
  Given the guest dies during replicate 12
  Then replicate 12 is a counted failure with its console retained and replicates 13 to 30 run
  And the report says the run is incomplete only if the benchmark process itself was killed

Scenario: A failed replicate inside the guest is counted
  Given one replicate's fixture fails in the guest
  Then the record and report list the failure with its replicate and arm and the other replicates are reported

Scenario: The guest console is retained with the record
  When a guest run ends in any state
  Then the guest's console output is retained with the result directory

Scenario: No guest survives the run
  Given the benchmark process is killed
  Then no guest VMM from the run is still running afterwards
```

AC: (a) a boot failure is a named failure with no host fallback; (b) a mid-run guest death fails that replicate and the run finishes with its terminal line, reported incomplete only when the benchmark process itself is killed (CA-1); (c) a failed replicate is counted with replicate and arm; (d) the guest console is retained with the result; (e) no guest VMM outlives the run, including a killed run. Notes: the host-side process that holds the guest owns the leak guarantee (`testing.md` § leaked state).

### US-GBS-04 — Keep a guest family buildable between Metal runs and prove native is unaffected

Job: J-PLAT-008. Slice: [04](slices/slice-04-guest-family-smoke-and-native-unchanged.md). Must-have (parent AC: a benchmark cannot rot between runs; the declaration requires a smoke placement).

Marcus wants to learn of a broken guest fixture from the Metal integration run, not from the next benchmark run, and to be sure that adding a substrate left the native families alone.

### Elevator Pitch
Before: the smoke test of a benchmark declaration exists for native families; a guest family has no defined smoke behaviour, and no check says adding the substrate left the native families alone.
After: run `cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests` → sees the smoke test of `guest-flow-forwarding` and of each native family PASS, each after one replicate per arm with no timing assertion, and `cargo xtask placement-lint` accepting them.
Decision enabled: Marcus decides the substrate addition is safe to merge, and learns of a broken guest fixture from the Metal integration run, not from the next benchmark run.

Domain examples:
1. The guest family's smoke test runs one replicate per arm and passes without asserting a time.
2. A deliberately broken in-guest fixture makes that smoke test fail naming the benchmark.
3. The four native smoke tests pass unchanged.

```gherkin
Scenario: The guest family's smoke test runs one replicate per arm with no timing assertion
  When Marcus runs the Metal integration test run for overdrive-bench-families
  Then the guest family's smoke test passes after one replicate per arm and asserts nothing about speed

Scenario: A broken in-guest fixture fails the smoke test
  Given the guest family's fixture no longer works in the guest
  When the same run executes
  Then the smoke test fails naming the benchmark

Scenario: The native families' smoke tests are unchanged
  Then each native family's smoke test passes as before

Scenario: The smoke test is placed like any other test
  Then cargo xtask placement-lint accepts it
```

AC: (a) the guest family has a smoke variant: one replicate per arm, no performance assertion, gated on `integration-tests`, lint-clean; (b) a broken guest fixture fails it naming the benchmark; (c) native smoke tests pass unchanged; (d) the build-time refusal test for `guest` is replaced by one that asserts the new acceptance and still refuses an unregistered substrate by name. Notes: how the smoke variant gets a guest is open point O5.

## Wave: DISCUSS / [REF] Story map and scope assessment

Backbone: Declare it, Run it in a guest, Trust the conditions, Read the result, Keep it building.

| Declare | Run | Trust the conditions | Read | Keep it building |
|---|---|---|---|---|
| WS: `guest` accepted by the attribute (01) | WS: benchmark form boots a guest, collects (01) | WS: host refusal, guest view, vCPU scheduling (02) | WS: same record, report states `guest` (01) | Smoke variant, native unchanged (04) |
| | Failure shapes: boot, death, console, leak (03) | | Incomplete record (03) | |

Walking skeleton = slices 01 and 02: a real in-guest component measured through the real benchmark form, with the conditions recorded. Priority rationale: 01 retires the riskiest assumption (a runner half inside a guest returns the unchanged record schema through the Metal benchmark form). 02 follows because the record is not comparable without the guest view and vCPU marking, and because the existing eligibility rule cannot apply to a guest as written. 03 follows because it is the failure shape of a mechanism 01 introduced. 04 closes the declaration's smoke requirement and the native-unchanged guarantee.

Scope assessment: **PASS**: 4 stories, 3 bounded contexts (measurement core crate and its record, the benchmark family crate, the xtask/Metal benchmark form with the guest kit), integration surfaces about 4 (registry and attribute, in-guest runner half and its return path, benchmark form, record and report). Estimated effort 5 to 7 days in total. No oversized signal fires beyond the borderline integration count, mitigated by the four independently shippable slices.

## Wave: DISCUSS / [REF] Outcome KPIs

Objective: Marcus obtains a comparable, reproducible number for an in-guest component through the benchmark form, without a hand-written guest harness.

| KPI | Who does what | Target | Baseline | Measured by |
|---|---|---|---|---|
| K1 (primary) | Marcus measures an in-guest component | 1 command (`cargo xtask metal run --benchmark <family>`) yields a validating record and report; 0 guest-only validators, report tools or record schemas | Declaring `guest` is a build error; each guest spike hand-wrote its harness | Run the form for the first guest family, then `bench-report validate` on the record |
| K2 | Every guest record carries its conditions | 100% of guest records state substrate `guest`, the host block, the guest block, vCPU count, `unpinned`, and not-nested | No guest records exist | Schema validation plus a check over every retained guest record |
| K3 (guardrail) | A drifted host is refused | 100% of guest runs on a host differing from the pinned settings refuse before any guest boots | The refusal exists for native only | One deliberate drift (`bpf_stats_enabled` set to 1) in the acceptance scenario |
| K4 (guardrail) | Existing native results are untouched | 0 committed native records fail validation; 0 native families change substrate | n/a | `bench-report validate` over every committed record |
| K5 | Guest failures are not silent | 100% of boot failures, guest deaths and failed replicates appear in the record or report; 0 guests alive after a run | No guest benchmark failure shape exists | Fault scenarios of US-GBS-03 |
| K6 (guardrail) | No guest family launches VMs and none compares substrates | 0 families declared `guest` that launch a VM; 0 commands taking records of two substrates | n/a | Review of the declarations; the cross-substrate scenario of US-GBS-02 |

Owner: implementer captures, independent reviewer evaluates; no dashboard.

## Wave: DISCUSS / [REF] Definition of Done

- [ ] US-GBS-01 and US-GBS-02 (skeleton): a guest family runs through the benchmark form on the exclusive host, returns a record in the existing schema with the guest view, host block, vCPU count, `unpinned` and not-nested, and a drifted host is refused before any guest boots.
- [ ] US-GBS-03: boot failure, mid-run guest death, replicate failure and console retention behave as stated; no guest survives a run.
- [ ] US-GBS-04: smoke variant of the guest family passes, a broken fixture fails it, native smoke tests unchanged, `placement-lint` clean.
- [ ] `benchmarks/methodology.md` states the guest substrate (isolation, eligibility as applied to a guest, clock boundaries, vCPU marking) and its version policy follows O4; `benchmarks/catalogue.md` has a row for the guest family and states the D9 rule.
- [ ] ADR-0299 is revised by the architect to the two-entry registry (not edited inline); `testing.md` and the `metal run` grammar rule are amended if DESIGN changes them (orchestrator process).
- [ ] Every committed native record still validates.
- [ ] Regression gating and baselines are not implemented.
- [ ] No issue number or roadmap-step id in any new name.
- [ ] Every scope limitation in the artifacts cites an approved GitHub issue, or the user dropped the language (CLAUDE.md).

## Wave: DISCUSS / [REF] Out of scope

- Regression gating and baseline promotion: tracked by [#361](https://github.com/overdrive-sh/overdrive/issues/361).
- VM-lifecycle families, and any family that launches VMs, in a guest: excluded by #362 because production has no nested virtualization; nested virtualization for VM workloads is tracked by #347, after which such a family measures a production state.
- In-guest kernel mTLS and its comparison with the host proxy: [#303](https://github.com/overdrive-sh/overdrive/issues/303) owns the mechanism; it is the intended later consumer of this substrate, not part of it. How #303 expresses an A/B whose arms sit on different substrates is open point O7.
- The native families and their committed records (`cgroup-connect-hook`, `dns-responder`, `mtls-path`, `vm-lifecycle`): unchanged. They measure host components.
- Running a benchmark inside an outer test guest group: not a measuring substrate (nested virtualization, ADR-0299). The smoke variant is the only benchmark body that runs in a test guest.
- Vsock attachment density ([#326](https://github.com/overdrive-sh/overdrive/issues/326)), pump scaling ([#300](https://github.com/overdrive-sh/overdrive/issues/300)), Firecracker against Cloud Hypervisor ([#305](https://github.com/overdrive-sh/overdrive/issues/305)): their own issues; consumers of the program, not part of this feature.
- Pinning the guest's vCPUs, and choosing a guest shape other than the guest kit's default: tracked by #368 (pin vCPUs) and #369 (declared guest shape); not delivered here.
- The first guest family's unmeasured paths (the host-driven vsock leg, guest-originated TCP and datagram flows, the owner's accept-and-pair work): tracked by [#370](https://github.com/overdrive-sh/overdrive/issues/370). Reading BPF program statistics inside a guest: tracked by [#371](https://github.com/overdrive-sh/overdrive/issues/371).

## Wave: DISCUSS / [REF] Walking skeleton strategy

Real, end to end, not a mock, guest substrate on the native x86_64 Metal host (Lima is no signal). Slices 01 and 02 are the skeleton: the first guest family is a real in-guest production component (O1), run through the real benchmark form under exclusive admission, in a guest launched by the production launcher. The comparison point for the schema and statistics is the existing native records; there is no earlier guest figure to compare with.

## Wave: DISCUSS / [REF] Driving ports

Developer-facing: (1) the attribute on a benchmark function with the `guest` substrate; (2) the benchmark form `cargo xtask metal run --benchmark <family> [--spike <name>]`; (3) `cargo xtask metal pull <result dir>`; (4) `bench-report validate|render <record>`; (5) `cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests` for the smoke variant; (6) `cargo xtask placement-lint`. Setup step, not a behaviour port: `cargo xtask guest-kit-prepare`. No HTTP or operator surface.

## Wave: DISCUSS / [REF] Pre-requisites

Metal host provisioned with `OVERDRIVE_METAL_TARGET`, `OVERDRIVE_METAL_KERNEL`, `OVERDRIVE_METAL_ROOTFS`; guest kit prepared (`cargo xtask metal run -- cargo xtask guest-kit-prepare`) and the module loaded (`cargo xtask metal run --exclusive -- cargo xtask kmod-load`); the shipped benchmark crates, methodology and catalogue (`native-benchmark-program`); the guest kit and in-guest lane (`overdrive-testing`); ADR-0299 (to be revised). #303 is not a prerequisite and is not implemented.

## Wave: DISCUSS / [REF] Open points

User rulings (2026-11-19):

- **O1 — ruled: option (a) of the recommendation.** The first guest family measures the guest flow owner's forwarding path in `overdrive-init` over `ovd_fwd` as it runs in a workload VM (working name `guest-flow-forwarding`), one arm, observable and clock chosen by DESIGN from what the component exposes. Waiting for #303 and a synthetic guest family were rejected. DESIGN must name the production seam the family needs; if the component's internals are not reachable from outside the `overdrive-init` binary, that is a surfaced design decision.
- **Unpinned guest vCPUs — tracked by #368.** Guest benchmarks run with the guest kit's unpinned vCPUs (`vcpus` default 2, no CPU affinity); this feature records `unpinned` only.
- **Fixed guest shape — tracked by #369.** All guest benchmarks run in the guest kit's default shape; a family cannot declare another vCPU count or memory size.
- **No mechanical D9 check — ruled.** D9 is stated in the catalogue and methodology and enforced by review. Nested virtualization for VM workloads is tracked by #347.

DESIGN questions (no user decision needed unless noted):

- **O2: eligibility as applied to a guest.** Today's pinned list (governor, SMT, boost) cannot be read in a guest, and "no readable source" refuses. Recommendation: the host half applies the full existing list on the real host before the guest boots; the guest half checks only what it can see (kernel series, `bpf_stats_enabled`, a release build) and records the rest absent with the reason. DESIGN confirms and states it in the methodology.
- **O3: guest boot granularity (contract-shaping, spike before approval).** One guest per run, or a fresh guest per replicate. The parent rule "every replicate builds a fresh fixture, no state survives" is the argument for per-replicate; boot time for 30 replicates per arm is the cost. Needs measurement on the Metal host before DESIGN is approved, and the record states which was used.
- **O4: record and methodology evolution.** The `host` block refuses unknown keys and `Substrate` has one variant; the guest block and the second substrate must keep every committed native record valid. Whether this bumps the methodology version (a change to any rule bumps it) is a DESIGN call.
- **O5: the smoke variant of a guest family.** The declaration requires a smoke placement. The smoke test runs inside a test guest; a guest family's body needs its own guest. DESIGN decides how one replicate per arm runs without making the smoke measurement nested or the benchmark substrate nested.
- **O6: the in-guest runner half and return path.** How the guest-side code is built and staged (the kit prepares in-guest libtest binaries today), how samples return to the host half (console, vsock), and how the benchmark form's closed command reaches a guest. The in-guest lane is the precedent.
- **O7: how #303 compares an in-guest mechanism with the host proxy.** The two mechanisms sit on different substrates, which D8 forbids comparing. One reading: both arms are measured from a client inside a guest, the host-proxy arm having the host half run the proxy. This belongs to #303's DESIGN; recorded so this feature's registry does not foreclose it.
- **O8: `--spike` with a guest family.** The benchmark form already takes `--spike`; DESIGN confirms a `spike-scratch/` crate can declare `guest` with the same schema and the spike evidence marker.
- Deferrals: regression gating and baselines, tracked by #361. Every other scope limit is tracked by #368, #369 or #347, or is out of scope; none carries a "future" pointer without an issue.

## Wave: DISCUSS / [REF] DoR validation

This table is the DISCUSS-time assessment; the current status of each dependency follows it.

| Item | 01 | 02 | 03 | 04 |
|---|---|---|---|---|
| 1 Problem in domain language | PASS | PASS | PASS | PASS |
| 2 Persona specific | PASS | PASS | PASS | PASS |
| 3 3+ examples, real data | PASS (4) | PASS (3) | PASS (4) | PASS (3) |
| 4 UAT 3-7 scenarios incl. error | PASS (6) | PASS (5) | PASS (5) | PASS (4) |
| 5 AC derived from UAT | PASS | PASS | PASS | PASS |
| 6 Right-sized | PASS with risk: 2 to 3 days, depends on O1 and O6 | PASS (1 to 2 d) | PASS (1 to 2 d) | PASS (1 d) |
| 7 Technical notes | PASS | PASS | PASS | PASS |
| 8 Dependencies tracked | PASS (O1, O6) | PASS (01, O2, O4) | PASS (01) | PASS (01, O5) |
| 9 KPIs numeric | PASS (K1, K2) | PASS (K2, K3) | PASS (K5) | PASS (K4, K6) |

Elevator pitch present on all; each "After" names the benchmark form or the Metal nextest run with concrete output; `job_id: J-PLAT-008` on all. Verdict at DISCUSS time: **PASSED for DESIGN, with O1 requiring the user's decision and O3 requiring a spike before the design is approved.** Current status: O1 ruled (option (a), 2026-11-19); O3 spiked, V-A held (DDD-5); O2 resolved by DDD-4, O4 by DDD-2 and DDD-3, O5 by DDD-10, O6 by DDD-6, DDD-7 and DDD-8; no dependency is open. Contradictions DESIGN found are CA-1 to CA-4.

Evidence per story (one line each, quoted from the current design):
- 01: "A fresh guest for every replicate"; the family drives the production guest flow owner and measures one series (DDD-5, DDD-8, ADR-0329, ADR-0332).
- 02: "The host is refused in full before any guest boots ... the guest's view comes from the replicates" (DDD-4, ADR-0328); the record states the guest's conditions in `run_started` and its view in one `guest_observed` line (DDD-2).
- 03: "A replicate retains its guest console beside the record"; a guest that cannot boot, dies or returns no intact result is a failed replicate and the run continues (DDD-9, DDD-5; CA-1, CA-4).
- 04: "The guest family's smoke test is `smoke(guest = "cgroup-default", profile = "nested-launch")`; no new mechanism" (DDD-10).

Per-wave peer review not invoked (optional; the consolidated review fires at end of DISTILL).

## Wave: DISCUSS / [REF] Wave decisions summary

- Scope: add a `guest` substrate to the closed registry and run a guest family through the existing benchmark form, in the same record schema, with the guest view and vCPU scheduling recorded. Four stories, four slices, one job (extended).
- Constraints: guest launched directly on Metal, not nested; host half enforces eligibility before any guest boots; guest-unseeable settings recorded absent; no cross-substrate comparison; no VM-lifecycle family in a guest; no new command; no regression gating.
- Honest first consumer: #303 has no code. The first live guest family measures the guest flow owner's forwarding path in `overdrive-init` (O1, user ruling 2026-11-19).
- Upstream changes: ADR-0299 ("guest not available", "a scope limitation of this feature") is superseded in part by this feature's two-entry registry; revision goes through the architect in DESIGN. No parent ruling is contradicted: parent ruling 6 (each family declares its substrate) is what this extends.
- Scope limitations: unpinned vCPUs (#368), fixed guest shape (#369); VM-launching guest families wait on #347.

## Wave: DESIGN / [REF] Inputs read

DESIGN, 2026-11-19, application scope, propose mode, lean density. Status of every decision below: **ACCEPTED, 2026-11-19**, on the user's rulings recorded in "User approval" (`.claude/rules/design.md`). DDD numbering is this feature's own and does not continue the parent's.

✓ This feature's DISCUSS (this file), `intake.md`, `slices/slice-01..04`. ✓ Parent `docs/feature/native-benchmark-program/feature-delta.md` (DESIGN sections) and `spike/findings-v10-guest-boot.md`, `spike/findings-v4-guest-shutdown.md` (read through the parent's DDD-11 account; their facts concern a production `serve` boot, not the guest-kit lane). ✓ ADR-0296 to ADR-0309 (ADR-0299 in full), `docs/product/architecture/brief.md` (benchmark section). ✓ Code: `crates/overdrive-bench/src/{registry,record,runner,host,replicate,lib}.rs`, `crates/overdrive-bench/Cargo.toml`, `crates/overdrive-bench-families/src/families/cgroup_connect_hook.rs`, `benchmarks/{methodology,catalogue}.md`, `crates/overdrive-testing/src/{guest_kit,guest_lane,guest_app,guest_suite,vm_fixture}.rs` and `placement/groups.rs`, `crates/overdrive-init/{Cargo.toml,src/main.rs,src/guest_flow/{mod,real_kernel}.rs,tests/integration/in_guest_lane.rs}`, `crates/overdrive-host/src/vmm.rs` (confined command, `kill_on_drop(false)`), `xtask/src/guest_kit_prepare.rs`, `infra/metal/{bootstrap.sh,native-preflight.sh}`. ⊘ `benchmarks/results/**` (read through the catalogue and `record.rs`), ⊘ `crates/overdrive-bench/bin/bench_report.rs`, `report.rs`, `args.rs` bodies (report structure taken from the parent's DDD-6 text).

**Kernel claims.** Every kernel claim the design rests on is read in the Linux v7.0 tree (`vendors/linux` is not checked out in this worktree; the identical tree of a sibling worktree is read) and cited `path:line @ v7.0`:

1. *A guest-local vsock connection to `VMADDR_CID_LOCAL` uses the loopback transport.* `vsock_use_local_transport` returns true for `remote_cid == VMADDR_CID_LOCAL` when a local transport is registered (`net/vmw_vsock/af_vsock.c:513-521 @ v7.0`), and `vsock_assign_transport` then selects `transport_local` for stream sockets (`net/vmw_vsock/af_vsock.c:580-583 @ v7.0`). The loopback module registers itself as that local transport (`net/vmw_vsock/vsock_loopback.c:159-160 @ v7.0`), which is why the in-guest program loads it before it connects.
2. *The loopback leg reaches the listener through the receive function a real host connection also takes.* The loopback worker hands each queued packet to `virtio_transport_recv_pkt` (`net/vmw_vsock/vsock_loopback.c:136-144 @ v7.0`); the guest's virtio receive path calls the same function (`net/vmw_vsock/virtio_transport.c:680 @ v7.0`) and so does the vhost host path (`drivers/vhost/vsock.c:555 @ v7.0`). The measured path shares the transport's receive logic with production and omits the virtqueue and vhost legs ([#370](https://github.com/overdrive-sh/overdrive/issues/370)).
3. *A VMM the benchmark process spawns stays in the benchmark unit's control group.* A task forked without `CLONE_INTO_CGROUP` takes its parent's css_set (`kernel/cgroup/cgroup.c:6783-6793 @ v7.0`), and `CloudHypervisorVmm` spawns the VMM as an ordinary child with `kill_on_drop(false)` and no cgroup move (`crates/overdrive-host/src/vmm.rs:309`). That the unit's stop then kills every task of the group is systemd's `KillMode=mixed` (`infra/metal/bootstrap.sh:870`), not a kernel claim; V-C checks it on the host.

No kernel claim is deferred to the DELIVER reviewer.

**Contradictions with DISCUSS.** Four, listed under Changed Assumptions (CA-1 guest death and "incomplete", CA-2 ADR-0301's "no field omitted" sentence, CA-3 workspace resolution in the `exclusive` class, CA-4 an unprepared kit is a failed replicate, not a refusal). None contradicts a numbered ruling or O1.

## Wave: DESIGN / [REF] Reuse Analysis

Contract shape (principle 12): P = pure-function, B = bounded-change (declared mutation set), U = unbounded-preservation (returns a plan, never mutates).

| Existing | Path | Overlap | Decision | Justification and contract shape |
|---|---|---|---|---|
| Substrate registry and attribute checks | `crates/overdrive-bench/src/registry.rs` (`SUBSTRATES`, `refusal_pieces`), `overdrive-testing-macros` | closed list checked by constant evaluation | EXTEND | One entry added, the `guest` refusal text removed. Shape: P (const functions). |
| Record schema, validator, report renderer | `crates/overdrive-bench/src/{record,report}.rs` | `Substrate`, `run_started`, validation, header lines | EXTEND | `Substrate::Guest`, one optional `run_started.guest`, one `guest_observed` line kind, a second `clock.guest` string, guest header lines. Shape: P (validate, render); the writer is B (append-only file). A native record's bytes and report are unchanged (asserted by validating every committed record and re-rendering one). |
| `HostValue` (the record's three-form setting value) | `crates/overdrive-bench/src/record.rs` | text / absent / unreadable | EXTEND (reuse) | `GuestView` carries `HostValue`s; no second reading type. Made `pub` and re-exported so a family can build a `GuestView` (ADR-0331). Shape: P. |
| Host eligibility | `crates/overdrive-bench/src/host.rs` | the pinned list, the probe, the `host` block | EXTEND (unchanged, applied to the real host first) | The existing `HostReading` is the host half's check. Shape: P over a read-only port. |
| Runner | `crates/overdrive-bench/src/runner.rs` | wiring order, refusal lines, ports | EXTEND | Writes the one `guest_observed` line when the first replicate reports a view, and fails a guest-substrate replicate whose body returned `Ok` without reporting one. No guest step before the replicates; the wiring order is the native one. Shape: B (result directory). |
| `Replicate`, `RecordSink` | `crates/overdrive-bench/src/replicate.rs`, `runner.rs` | what a body can write | EXTEND | `Replicate::report_guest_view`; `Replicate::retain_console`; `RecordSink::write_artifact`. Shape: B (writes only under the result directory, create-new). |
| Guest kit | `crates/overdrive-testing/src/guest_kit.rs` | image build, VMM launch through `CloudHypervisorVmm`, console, CID bands | EXTEND | `GuestImageSpec::benchmark()`, the `GUEST_BENCHMARK` band. The launch path is consumed as is. Shape: B (VMM process, rootfs clone, run directory, CID; removed on drop). |
| In-guest lane | `crates/overdrive-testing/src/guest_lane.rs` (`run_in_guest_cell_attempt`, `HostListeners`, beacon play, `Holder`) | boot the production-init guest, play the beacon, run one operator command, read the console | EXTEND | `run_in_guest_program` shares the existing body with `run_in_guest_cell`; the lane's inline `512` and `2` become two constants both runs read. A parallel implementation would duplicate the beacon exchange. Shape: B. |
| `overdrive-guest-peer` | `crates/overdrive-testing/src/guest_app.rs` and its bin | the release-built static guest program already in every kit image; already hosts benchmark workloads (`listen-ignoring-stop`) | EXTEND | One subcommand (`forwarding-bench`) that prints the two guest readings and the series block, the grammar's printer and the host half's parser of it (one owner holds both); `sha2` joins `overdrive-testing`'s dependencies. CREATE NEW (a bin in the families package or a new crate) rejected in ADR-0332. Shape: B (guest sockets, closed with the guest). |
| `overdrive-init` guest flow owner | `crates/overdrive-init/src/guest_flow/` | the measured production component | EXTEND (consume as is; no change) | Reached through its vsock wire seam only; internals stay `pub(crate)` of a binary crate. Shape: U for this feature (never modified). |
| In-guest libtest cells and lanes | `xtask/src/guest_kit_prepare.rs` (`IN_GUEST_LANES`), `guest_lane::InGuestTests` | staging a program into the guest | not used | Libtest cells are dev-profile builds (debug assertions on) and `--exact` test selection; neither fits a measured program. The peer is already staged by the existing release step, so `guest-kit-prepare` is unchanged. |
| `LaneScript`, `OwnerLaneHost` | `crates/overdrive-testing/src/guest_lane.rs` | host-to-guest line channel | not used | Needs a scripted-peer PID 1 and a kernel command-line port; not the production-init guest this family needs. |
| Metal benchmark form, preflight | `infra/metal/bootstrap.sh` (benchmark form), `infra/metal/native-preflight.sh` (virtualization probe) | closed command, exclusive admission, no-nesting guard | EXTEND (unchanged) | The form already runs the host process as root on the exclusive host in a `systemd-run` unit with `KillMode=mixed`; the preflight refuses a virtualized host, which is what makes "launched on the Metal host, not nested" true. |
| `vm_fixture::workspace_root` | `crates/overdrive-testing/src/vm_fixture.rs:1086-1097` | resolves the checkout the kit stages artifacts from | EXTEND | One condition widened (DDD-11). Crate-private. Shape: P over the environment (reads two variables, a `cargo locate-project` call otherwise). |
| `smoke(...)` and the `cgroup-default` profiles | `overdrive-testing-macros`, `crates/overdrive-testing/src/placement/groups.rs` | running a guest-launching body in a test guest | EXTEND (reuse; no new mechanism) | `smoke(guest = "cgroup-default", profile = "nested-launch")` (DDD-10). |
| Families crate | `crates/overdrive-bench-families/` | where families live | EXTEND | One family module `guest_flow_forwarding.rs`; the bench `main` is unchanged. |
| Compile-fail fixtures | `crates/overdrive-bench/tests/compile_fail/{guest_substrate,unknown_substrate,unknown_guest_clock_source}.rs` | build-time refusals | EXTEND | `guest_substrate` is replaced by an acceptance fixture; two `.stderr` files regenerate. |

## Wave: DESIGN / [REF] Components and ports

Pattern: the parent's modular, ports-and-adapters shape, unchanged; no new crate. Paradigm: object-oriented, per `CLAUDE.md`. Enforcement: `crate_class` dispatch and review keep both crates dev-only; the existing compile-fail and unit tests are extended; no new enforcement tool is needed (ADR-0296's position stands).

| Component | Path | Change | Role |
|---|---|---|---|
| Substrate registry, attribute checks | `crates/overdrive-bench/src/registry.rs` | EXTEND | `["native", "guest"]`; guest clock source `guest_monotonic` |
| Record, validator, report | `crates/overdrive-bench/src/{record,report}.rs` | EXTEND | `Substrate::Guest`; optional `guest` block; the `guest_observed` line; guest lines; `HostValue` made `pub` |
| Runner | `crates/overdrive-bench/src/runner.rs` | EXTEND | the `guest_observed` line and failing a guest replicate that reports no view; no guest port, no guest step before the replicates |
| `Replicate::report_guest_view`, `Replicate::retain_console`, `RecordSink::write_artifact`, `GuestView` | `replicate.rs`, `runner.rs`, `record.rs` | EXTEND | the guest's view and its console beside the record |
| Guest kit and lane | `crates/overdrive-testing/src/{guest_kit,guest_lane,vm_fixture}.rs` | EXTEND | `GuestImageSpec::benchmark`, `GUEST_BENCHMARK` band, `LANE_GUEST_VCPUS` and `LANE_GUEST_MEMORY_MIB`, `run_in_guest_program`, `ProgramRun`, workspace resolution |
| Guest peer | `crates/overdrive-testing/src/guest_app.rs` and bin | EXTEND | `forwarding-bench`; output grammar (two readings and series block): printer and host-half parser together |
| Family `guest-flow-forwarding` | `crates/overdrive-bench-families/src/families/guest_flow_forwarding.rs` | CREATE NEW (family) | host half: artifact digests once per run, per-replicate guest, parse, report the view, record |
| Catalogue, methodology | `benchmarks/{catalogue,methodology}.md` | EXTEND | row and guest section (DELIVER writes) |

**Driving ports:** unchanged from DISCUSS: (1) the attribute with `substrate = guest`; (2) `cargo xtask metal run --benchmark <family> [--spike <name>]`; (3) `cargo xtask metal pull <result dir>`; (4) `bench-report validate|render <record>`; (5) `cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests` (the smoke variant); (6) `cargo xtask placement-lint`. Setup step: `cargo xtask guest-kit-prepare` (unchanged). Read/write split holds: `bench-report` is read-only; the runner is the only writer of the record.

**Driven ports and adapters** (each with a `probe()` where the environment can lie):

| Port | Production adapter | Test double | What `probe()` proves |
|---|---|---|---|
| `HostProbe` (existing) | procfs/sysfs reader | fixture reader | unchanged |
| `RecordSink` (existing, one method added) | append-only file | in-memory | create-new semantics, now also for the console file |
| Guest kit launch (`GuestKit::boot`, via the production `CloudHypervisorVmm`) | production | none (the family runs only against it) | proved on every replicate, not by a separate probe: a replicate whose guest does not boot to READY, or does not print its readings and series block, fails; each replicate's guest reports the kernel and clocksource it actually ran, and the record carries the digests of the staged files the kit copies into the image, so the specific lie "the kit's staged artifacts are stale or absent" is exercised by booting them and recording their digests, not by checking paths |

There is no `GuestSubstrate` port: nothing in the runner needs a guest before the replicates (DDD-7).

**Wiring order in `run_main` for a `guest` benchmark** (the runner's existing order, `runner.rs` `Runner::run`, unchanged): usage and family selection, source root and results root, checkout marker; host probe, every host setting and the host decision (no side effect); create the result directory and announce `OVERDRIVE_BENCH_RESULT_DIR` (before any refusal, as today); **if the host is refused**: `run_started` (with the static `guest` block), `run_refused`, report, stop; no guest has booted. **Else**: statistics enable if declared, `run_started`, canary, replicates (each a fresh guest; the line of the first replicate that reports a guest view is preceded by the one `guest_observed` line), canary, `run_finished`, report. A guest benchmark's only refusals are host refusals, and the directory-line-first invariant is the native one.

**C4 (L1 System Context, L2 Container).** The diagrams live in [c4-diagrams.md](../../product/architecture/c4-diagrams.md#guest-benchmark-substrate-gh-362); they are not repeated here.

## Wave: DESIGN / [REF] Decisions table

| ID | Decision | ADR | Status |
|---|---|---|---|
| DDD-1 | `guest` joins the closed substrate registry; `guest` means a kit guest the benchmark launches directly on the Metal host; a guest benchmark declares no `bpf_stats` (BPF statistics inside a guest: [#371](https://github.com/overdrive-sh/overdrive/issues/371)) | [0325](../../product/architecture/adr-0325-the-guest-substrate-joins-the-closed-benchmark-registry.md) | ACCEPTED 2026-11-19 (PD-1) |
| DDD-2 | The record states the guest's conditions in the optional `guest` block of `run_started` and the guest's view (shape, kernel, clocksource, staged-artifact digests) in one `guest_observed` line; `schema` stays 1 | [0326](../../product/architecture/adr-0326-a-guest-benchmark-record-states-the-guests-conditions-in-run-started-and-its-view-in-one-guest-observed-line.md) | ACCEPTED 2026-11-19 (PD-2, PD-4) |
| DDD-3 | Adding a substrate does not bump the methodology version | [0327](../../product/architecture/adr-0327-adding-a-substrate-does-not-bump-the-methodology-version.md) | ACCEPTED 2026-11-19 (PD-3) |
| DDD-4 | The host is refused in full before any guest boots; there is no probe guest and no guest pin: each replicate reports the guest's view, the first fixes it, later ones are not compared; an unprepared kit is a failed replicate | [0328](../../product/architecture/adr-0328-a-guest-benchmark-refuses-the-host-before-it-boots-a-guest-and-takes-the-guests-view-from-its-replicates.md) | ACCEPTED 2026-11-19 (PD-4, option (b)) |
| DDD-5 | A fresh guest for every replicate | [0329](../../product/architecture/adr-0329-a-guest-benchmark-boots-a-fresh-guest-for-every-replicate.md) | ACCEPTED 2026-11-19 (PD-5); V-A held |
| DDD-6 | A guest returns its samples on its console as one framed series block | [0330](../../product/architecture/adr-0330-a-guest-returns-its-samples-on-its-console-as-framed-series-lines.md) | ACCEPTED 2026-11-19 (PD-6); V-B held; V-B2 held |
| DDD-7 | The family launches its own guests; the benchmark core has no guest port and no new dependency | [0331](../../product/architecture/adr-0331-a-guest-benchmark-family-launches-its-own-guests-and-the-benchmark-core-has-no-guest-port.md) | ACCEPTED 2026-11-19 (PD-7, as PD-4 option (b) leaves it) |
| DDD-8 | The first family drives the production guest flow owner through its vsock wire seam and measures one series | [0332](../../product/architecture/adr-0332-the-first-guest-family-drives-the-production-guest-flow-owner-through-its-vsock-wire-seam.md) | ACCEPTED 2026-11-19 (PD-8) |
| DDD-9 | A replicate retains its guest console beside the record | [0333](../../product/architecture/adr-0333-a-replicate-retains-its-guest-console.md) | ACCEPTED 2026-11-19 (PD-9) |
| DDD-10 | The guest family's smoke test is `smoke(guest = "cgroup-default", profile = "nested-launch")`; no new mechanism (resolves O5) | none: a choice within the existing placement registry | not a material decision; stands with the design |
| DDD-11 | `vm_fixture::workspace_root` accepts the exported source root in class `exclusive` as well as `ordinary` (resolves O8's cwd finding) | none: a one-condition change inside a crate-private function | ACCEPTED 2026-11-19 (approved with PD-7) |

Resolution of the DISCUSS design questions: O2 by DDD-4; O3 by DDD-5 (V-A, held); O4 by DDD-2 and DDD-3; O5 by DDD-10; O6 by DDD-6, DDD-7 and DDD-8 (the program is the guest peer, already staged by `guest-kit-prepare`; samples return on the console; the form's closed command is unchanged because the guest is launched inside the family body; V-B and V-B2, held); O7 below; O8 by DDD-11 and below.

- **O7 (#303's A/B across mechanisms).** The substrate is a property of the benchmark declaration and the record, not of an arm. A benchmark whose arms are an in-guest mechanism and the host proxy is one `guest` benchmark whose host-proxy arm's fixture runs the proxy on the host half; both arms are measured from the same in-guest client, so the comparison stays inside one record and D8 is not crossed. Nothing in the registry forecloses it, and no mechanism is added for it here; it belongs to #303's DESIGN.
- **O8 (`--spike` with a guest family).** A spike crate declares `substrate = guest` and calls `run_main(Evidence::Spike)` exactly as today: `run_main` and the spike's `main` are unchanged (DDD-7: the family, here the spike's body, launches its guests), and the form's `--spike` grammar, the result root and the evidence marker are unchanged. A spike that needs a guest program of its own builds it as a second `[[bin]]` of the spike crate (`cargo bench` builds it beside the bench binary) and stages it with a `GuestFile` in the image spec's `extra` list over `GuestImageSpec::benchmark()`, then passes it to `run_in_guest_program`; no prepare step and no change to `guest-kit-prepare`. A spike whose payload is a guest-peer subcommand needs neither. The guest kit finds the checkout through the form's source root, so a spike's cwd does not matter (DDD-11 states the finding). V-B ran a spike crate through `--benchmark <family> --spike <name>` with its own staged payload and returned samples intact.

## Wave: DESIGN / [REF] Decisions put to the user

Status of every item: **APPROVED 2026-11-19** (verdicts in "User approval"). Each is stated so it can be read without the ADR. The ADR and the interface contract that work out a decision's details do not need a second approval. The contract-shaping spikes V-A, V-B and V-B2 have run and held; no contract-shaping item is open. Options a decision did not take stay listed as the alternatives that lost.

**PD-1 — The `guest` substrate joins the registry.** ADR-0325. **APPROVED 2026-11-19, option (a).**
- *Context.* A benchmark declares its measuring environment with `substrate = <name>`. ADR-0299 registers only `native` (the exclusive Metal host) and makes `substrate = guest` a build error.
- *Problem.* The component we were asked to measure runs inside a workload VM in production; a native number describes the wrong place, so today it takes a hand-written spike with its own harness. Adding `guest` forces us to say what it means so it cannot be confused with the placement-group guests that tests run in, and to say what a guest benchmark may declare.
- *Options.* (a) `guest` means the measuring half runs in a guest-kit guest that the benchmark process launches directly on the exclusive Metal host; the attribute takes it as a bare identifier, like `native`; an unknown name is still a build error naming the registered ones; a guest benchmark declares no `bpf_stats` (the host's statistics descriptor does not cover a guest kernel), stated in the catalogue and methodology and enforced by review, the way the D9 rule is; reading BPF statistics inside a guest is tracked by [#371](https://github.com/overdrive-sh/overdrive/issues/371). (b) `substrate = guest("<group>")`, naming a test placement group. (c) also register a nested-guest substrate. (d) as (a) plus a build error for `bpf_stats` on a `guest` benchmark: one constant-evaluated message and one compile-fail fixture for a state no declared family reaches.
- *Recommendation.* (a). (b) and (c) measure inside an outer test guest, which is nested virtualization that production does not have (ADR-0299's reason, #362); (d) adds a mechanism for an unreached state. `native` is unchanged.
- *Depends on.* nothing.

**PD-2 — The record states the guest's conditions in `run_started` and its view, with the identity of the staged artifacts, in one `guest_observed` line.** ADR-0326. **APPROVED 2026-11-19: record the artifact SHA-256 digests, do not refuse on them. The guest view arrives with the replicates (PD-4).**
- *Context.* A record's `run_started` has a nine-key `host` block that refuses unknown keys, and every committed record is native with no `guest` key. The native record contract (the parent feature delta's DDD-4) says no field is ever omitted to mean `null`; the only optional fields are three on `run_refused`. The benchmark form rebuilds the benchmark binary at the checkout's commit but not the guest artifacts: `cargo xtask guest-kit-prepare` stages `overdrive-init`, `overdrive-guest-peer` and the `ovd_fwd` module in a separate step, and nothing in the kit checks them against the commit.
- *Problem.* The record of a guest run must show the real host's settings, what the guest could see (and that it could not see governor, SMT and boost), the guest's vCPU count, that its scheduling is unpinned, and that it was launched on the host and not nested, without a second schema and without making any committed native record invalid. It must also say what ran: if Marcus edits the guest flow owner and benchmarks without preparing the kit again, the record would state the new commit while the guest ran the old `overdrive-init`.
- *Options.* (a) One optional `guest` object on `run_started`, present exactly when the substrate is `guest`, holding what is true before any guest boots (launch, boot granularity, vCPU scheduling, the three settings a guest cannot see), plus one `guest_observed` line (kernel and clocksource read in the guest, the guest shape the kit booted, and `artifacts`: the SHA-256 of the three staged files the kit copies into the image, computed once per run on the host) written when the first replicate reports its view (PD-4); `schema` stays 1; native records and reports stay byte-identical. (b) `guest: null` on native records: every committed record becomes invalid or must be rewritten. (c) A separate guest schema or record file: contradicts "no guest-only schema or validator" (K1). (d) As (a) without `artifacts`: a stale artifact is recorded under the current commit and no reader can tell. (e) As (a) and refuse when the staged artifacts differ from the commit: needs a stamp `guest-kit-prepare` writes and a rule for a dirty checkout, and still misses an edit after the stamp.
- *Recommendation.* (a). It costs one more exception to the native record contract's "never omitted" sentence (a second optional place; CA-2), one line kind that only guest records carry, and `sha2` in the family's host half; a validator built before this change refuses a guest record, which is correct. A stale artifact is visible in the record, not refused: a reader compares the digests with a fresh `guest-kit-prepare` at the record's commit. (e) would prevent it but changes the setup step.
- *Depends on.* nothing.

**PD-3 — Adding a substrate does not bump the methodology version.** ADR-0327. **APPROVED 2026-11-19, option (a).**
- *Context.* `methodology.md:3-5` says a change to any rule bumps the version; the validator requires `methodology == 1`; committed native records say 1.
- *Problem.* The guest substrate adds rules. If it bumps, either the four native families' next runs say 2 although nothing about them changed (comparing records then looks like comparing two methods), or the version depends on the substrate. The premise of "no bump" is that no rule governing a native record changes. These are the sentences of `benchmarks/methodology.md` the change touches, and the other sentences it leaves alone:

| Lines | Sentence | Effect of the change |
|---|---|---|
| 3-5 | "a change to any rule below bumps the version" | Reworded to "a change to or removal of a rule below bumps the version; an addition for a new substrate does not". This is the version policy itself; it constrains no record's numbers. |
| 8-11 | "Each benchmark's declaration also generates a smoke test that runs one replicate per arm in a guest and asserts no timing." | Unchanged. Gains a clause for a guest benchmark: its smoke test boots the family's measuring guest inside that test guest and takes no measurement. Addition. |
| 17-19 | "The benchmark runs on the native substrate (the host itself, no namespace, no wrapper)." | Reworded to "A native benchmark runs on the native substrate (...); a guest benchmark runs in a guest the benchmark process launches directly on that host (§ Guest substrate)". Substrate-conditional: the requirement on a native run is word for word the same. |
| 20-21, 22-24, 25-26 | fresh fixture per replicate; no `bpf_stats_enabled` writes; seeded shuffle | Unchanged. The guest is part of the fresh fixture. |
| 28-38 | Host eligibility | Unchanged; the full list applies to the real host. No guest setting is pinned; the guest's readings are recorded (new section). |
| 42-49 | Clock boundaries | Unchanged. The guest clock source set gains `guest_monotonic`, which is a duration the guest measured, the category the sentence already names. |
| 51-78 | Cold and warm handling; samples and percentiles; canary | Unchanged. A one-arm family has no A/A control, as for any one-arm family; the canary is a host measurement and the new section says so. |
| 80-88 | Record format | `run_started` gains the optional guest block. Addition. |
| 90-98 | Failure accounting | Unchanged. A guest failure is a failed replicate, which the section already counts. |

  So two sentences are reworded (`:3-5` and `:17-19`) and each is substrate-conditional or the policy itself; the rest are unchanged or additions.
- *Options.* (a) Stay at 1; the version bumps when a rule that already governed some record's production changes or is removed; a section for a new substrate and a sentence made substrate-conditional are not such a change. (b) Bump to 2 for every new record; the validator accepts 1 and 2. (c) Bump to 2 for guest records only.
- *Recommendation.* (a), with the sentence at `:3-5` amended. (b) makes identical native methods look different; (c) adds a version axis the schema has no field for. If the user reads the `:17-19` rewording as a change to a rule, (b) is the alternative.
- *Depends on.* nothing.

**PD-4 — The host is refused in full before any guest boots; there is no probe guest; the guest's view comes from the replicates.** ADR-0328. **APPROVED 2026-11-19 as option (b).** The user's reason: "the guest kernel would never be stale... we control it."
- *Context.* The existing eligibility list pins governor, SMT, boost, kernel series, `bpf_stats_enabled`, no `bpf-stats` holder and a release build, and refuses a setting with no readable source. A guest has no cpufreq, boost or SMT source. `run_started` is the first line of an append-only record, written before any replicate.
- *Problem.* Applied inside a guest the list refuses every guest; skipped, a drifted host would go unnoticed although the guest's vCPUs run on its CPUs. What Marcus sees today on a drifted host is a refusal naming the setting; he must see the same here, with no guest booted. The guest's own readings (kernel and clocksource) exist only once a guest has booted, which is after `run_started`.
- *Options.* (a) Apply the full list to the real host first (unchanged); then boot one probe guest, read the guest's view in it, pin the guest kernel series `7.0`, and refuse on `guest_probe` or `guest.kernel`: one more boot per run (about 1.1 s of about 84 s), a probe port, a probe error type and two refusal settings. (b) **Chosen.** No probe guest and no guest pin. Apply the full list to the real host first (unchanged); each replicate's in-guest program prints the kernel and clocksource and the family's host half reports them, with the staged artifacts' digests and the guest shape, as the replicate's view; the first reported view is appended as one `guest_observed` line and later views are not compared (the operator fixes the kernel, the artifacts and the shape, and the run holds the host exclusively on a bound checkout, so nothing can differ: the reason of this ruling). governor, SMT and boost are recorded `absent` with `not visible from a guest`. An unprepared kit is a failed replicate with the typed reason (CA-4), not a refusal. (c) Check only the host and record no guest view: no reader can tell what ran. (d) Apply the host list in the guest: refuses every guest.
- *Decision.* (b). It removes the probe guest, the `GuestSubstrate` port and its adapter, `GuestProbeError`, the `guest_probe` and `guest.kernel` refusal settings, the guest kernel pin and the `guest-readings` program, at the cost of one new record line kind and the refusal timing for an unprepared kit (failed replicates, not a refusal before the record starts). DISCUSS O2's recommendation also listed the guest's `bpf_stats_enabled` and a release build; neither is pinned in the guest: no guest benchmark declares `bpf_stats` (PD-1) and the guest programs are built `--release` by `guest-kit-prepare`, so no reachable failure motivates either.
- *Depends on.* nothing.

**PD-5 — A fresh guest for every replicate.** ADR-0329. V-A held. **APPROVED 2026-11-19, option (a).**
- *Context.* The methodology rule is one fresh fixture per replicate, no state surviving. For a guest benchmark the guest is part of the fixture: its socket tables, the guest owner's flow table and UDP slots, the forwarder's pair state and the page cache.
- *Problem.* One guest per run is cheap but lets replicate 30 differ from replicate 1 for reasons the A/A control cannot separate; a guest per replicate is clean but costs one boot and shutdown each, and it changes what a guest death means.
- *Options.* (a) A fresh guest per replicate; a guest that cannot boot, dies, or returns no intact result is a failed replicate (counted with its reason) and the run continues; the record states `boot: per_replicate`; a run is "incomplete" only when the benchmark process itself dies. (b) One guest per run, a replicate being a fixture inside it; a guest death ends the run incomplete; the record states `boot: per_run`. (c) One guest per profile.
- *Measured cost (V-A, Metal host, Linux 7.0.0-38-generic, commit `5eaa4f7e4`).* One whole guest lifecycle (image build and copy, launch to peer ready, one trivial exec, shutdown to VMM exit) is p50 1,078 ms, p95 1,132 ms, max 1,174 ms over 90 sequential cycles with 0 failures; the first cycle of the run, with cold caches, was 1,685 ms. Sixty fresh guests cost about 65 s of lifecycle; this feature's first family has one arm, so the lifecycle of 30 replicates costs about 33 s (the whole cycle with workload and return is measured below). The bound is a cycle of at most 30 s at the median: the measurement is about 28 times inside it. The spike booted the kit's scripted-peer image. *Measured for the production-init guest (V-B2, same host, kernel and commit, 180 fresh guests):* a whole replicate cycle (launch, workload, console return, exit) is about 2.75 s at the median: launch to READY 1.10 s, EXEC to EXIT 1.58 s (workload about 0.89 s, console print about 0.65 s for a 41 KB block), EXIT to VMM exit 40 ms. Thirty replicates cost about 83 s, about 11 times inside the 30 s bound. The floor of about 1.1 s of lifecycle per replicate follows from the fresh-fixture rule, so a family with thousands of replicates per arm would cost hours; a family chooses its `replicates` value knowing this.
- *Recommendation.* (a). The cost is affordable by a wide margin and keeps the methodology's fresh-fixture rule. This changes US-GBS-03 scenario 2 and AC (b) (CA-1): guest death at replicate 12 leaves 1 to 11 retained and 12 failed, and the run finishes.
- *Depends on.* nothing.

**PD-6 — A guest returns its samples on its console as one framed series block.** ADR-0330. V-B and V-B2 held; both transports work, so this is a choice between two measured costs. **APPROVED 2026-11-19, option (a), console.**
- *Context.* The in-guest program measures on the guest clock and must hand every sample to the host half. The in-guest lane's result channel is the guest console, read once the VMM has exited, and it is also what a failed guest leaves behind. The VMM writes the console to a file whether or not the host is reading it (`crates/overdrive-host/src/vmm.rs:296`).
- *Problem.* Samples must arrive intact (none dropped, none altered), and a short or corrupt return must make the replicate fail, not shrink the series. Both candidate transports deliver intact blocks (V-B); they differ in what they cost and what they need.
- *Measured (V-B, Metal host, Linux 7.0.0-38-generic, commit `5eaa4f7e4`, medians over 10 replicates per case, count and checksum recomputed on the host, 70 of 70 replicates intact, scripted-peer guest).*

| Samples in the block | Console: bytes, time | vsock: bytes, time |
|---|---|---|
| 1,000 | 3.1 KB, 61 ms | 8 KB, 8.2 ms |
| 100,000 | 307 KB, 5.15 s | 800 KB, 14.9 ms |
| 1,000,000 | 3.07 MB, 52.3 s | 8 MB, 88 ms |

  The console runs at about 59 KB/s, linear in bytes, and its cost is paid in the guest after the timed phase, so it never enters a measured interval. The spike's samples were short (about 3 bytes each); the family's values are 5 to 6 digits, about 7 bytes each with the separator. *Measured on the production-init guest (V-B2, 180 replicates, console read after EXIT, power-off and VMM exit):* a 41 KB block (about 5,500 values) prints in 0.65 s median at about 63 KB/s and arrives whole; the console costs about 0.65 s per replicate against about 8 ms on vsock, 15,000 values (about 105 KB) would cost about 1.7 s on the console. A whole replicate cycle is about 2.75 s, about 83 s per 30 replicates on the console; vsock would save about 0.65 s per replicate, about 20 s per run (about 63 s against about 83 s).
- *Options.* (a) Console: after the timed phase the program prints a `\n` and then one series block (a begin line, value lines of at most 16 values, an end line carrying a SHA-256 checksum), and exits non-zero when it could not drive its flow. The host half matches the begin line only at the start of a console line and requires exactly one intact block of the expected count; a missing, short or disagreeing block, or a non-zero exit, fails the replicate. ADR-0330 is the one place that states why the leading `\n` is part of the grammar (V-B2) and why a tolerant begin match is rejected. Needs nothing else: no listener, no port, no reader thread. The samples are also in the retained `console.log` (PD-9). (b) vsock: the program connects to the host (CID 2, `net/vmw_vsock/af_vsock.c:584-586 @ v7.0`) and writes the block as binary with a trailing checksum. Needs a host listener and a port passed to the program, and the listener must be read concurrently with the run, because `GuestPeer::exec`-style waits return only at guest-process exit and the guest blocks once the vsock buffer fills (V-B finding: 10 of 10 vsock 100k replicates timed out until the host read on a thread started before `exec`). `run_in_guest_program` gains the listener plumbing and a result field for the received block, which is more public test-support surface than (a). A guest that dies mid-send leaves a short block, caught by the count and checksum. The samples are not in `console.log`. (c) A file the guest writes into a shared volume. (d) The beacon exit status.
- *Decision.* (a): (a) adds about 20 s to a 30-replicate run, all outside any timed phase, in exchange for no listener, no concurrent reader, no port parameter and the samples living in the retained console. (b) saves that time but is the larger mechanism, and a run of about 63 s against about 83 s does not change what Marcus can do with it. (c) and (d) do not fit. V-B2 showed the production-init guest's console is complete after the VMM exits, so (a) needs no drain step.
- *Depends on.* nothing; V-B2 held.

**PD-7 — The family launches its own guests; the benchmark core has no guest port.** ADR-0331. **APPROVED 2026-11-19 as recommended, including the workspace-root change (DDD-11).** The user's PD-4 ruling removes the probe the recommended port existed for, so the approved decision takes the shape below; the part of the recommendation that stands is that no spike `main` changes.
- *Context.* Launching guests is `overdrive-testing`'s job; `overdrive-bench` has only a dev-dependency on it; the families crate already depends on it; a spike crate must declare a guest benchmark by calling `run_main` as today. Every guest benchmark boots in the in-guest lane's existing production-init shape (2 vCPUs, 512 MiB, `guest_lane.rs:467-468`); the record states it and #369 tracks declaring another. With no probe guest (PD-4) the runner needs nothing from a guest before the replicates.
- *Problem.* Where guest launching lives decides what a spike crate and `bench-report` must depend on and compile.
- *Options.* (a) **Chosen.** The family's host half boots each replicate's guest through the guest kit and hands the guest's view and console to the runner through `Replicate`; `overdrive-bench` gains record vocabulary and two `Replicate` methods and no dependency; `run_main` is unchanged. (b) A crate-internal `GuestSubstrate` port in `overdrive-bench` with a production adapter over the kit (the recommended shape before PD-4): `overdrive-testing` becomes a normal dependency of the benchmark core and the port has no method left to call. (c) The adapter in `overdrive-bench-families`, passed through a new `run_main` variant: a spike crate then depends on the whole families closure. (d) The port in `overdrive-testing`: a dependency cycle.
- *Decision.* (a). `DDD-11` (the kit finds the checkout through the form's source root in class `exclusive`) is approved with it.
- *Depends on.* PD-4.

**PD-8 — The first guest family drives the production guest flow owner through its vsock wire seam and measures one series.** ADR-0332. **APPROVED 2026-11-19, option (a): one metric, `forwarded_round_trip_ns`.**
- *Context.* O1 ruled that the first guest family measures the guest flow owner's forwarding path in `overdrive-init` over `ovd_fwd`. `overdrive-init` is a binary crate whose owner types are `pub(crate)`; its in-guest cells are dev-profile libtest binaries.
- *Problem.* The family must drive the production component through a production seam and must not invent a test-only branch. The owner's internals are not reachable from a benchmark crate, so a seam has to be named; and the number of metrics decides how much the in-guest program has to do.
- *Options.* (a) Boot a guest whose PID 1 is the production `overdrive-init` and run a subcommand of the existing release-built `overdrive-guest-peer` as its operator command. The peer plays the host's flow peer (a vsock connection over loopback to the guest's inbound port, carrying the `TcpAccept` request) and the application (a listener on the workload address), on the guest's `CLOCK_MONOTONIC`. Family `guest-flow-forwarding`, one arm `forwarding`, one series metric `forwarded_round_trip_ns` (64-byte request to echoed reply after the pair is attached, closed loop), declared guest clock `guest_monotonic`. No production change. (a2) As (a) plus a second series metric `accept_ns` (request written to the application's `accept()` returning): measures the owner's accept work, at the price of an accept loop in the program, a second series block and a second failure path. (b) Split `overdrive-init` into a library and call `GuestFlowOwner` from the benchmark: publishes internals and measures a second owner beside PID 1's. (c) A cell in `overdrive-init`'s own test binary: debug assertions on. (d) Drive the host leg over vhost-vsock and stamp on the host clock: measures the host path with the owner and cannot attribute cost.
- *Decision.* (a). Its consequence: after the pair is attached the measured path is the `ovd_fwd` forwarder under an owner-established pair, and the owner's own accept-and-pair work is setup, not measured; (a2) was the option that measured it. The measured path is the guest side only: the vsock leg is loopback in the guest, so the vhost transport, the host flow owner, guest-originated TCP flows, datagram flows and the owner's accept-and-pair work are not measured. These scope limitations are tracked by [#370](https://github.com/overdrive-sh/overdrive/issues/370) ("Benchmark the host-driven vsock leg and the other flow kinds of the guest flow owner").
- *Payload staging.* The program is a subcommand of `overdrive-guest-peer`, which `cargo xtask guest-kit-prepare` builds static and release (`xtask/src/guest_kit_prepare.rs:615`) and every kit image installs at `/sbin/overdrive-guest-peer` (`crates/overdrive-testing/src/guest_kit.rs:563`, `guest_app.rs:46`); the in-guest lane already runs peer subcommands as the operator command (`guest_lane.rs:501`). So the first family needs no prepare step, no `IN_GUEST_LANES` entry, no `GuestFile` and no new bin target. A spike (or a later family whose program is not a peer subcommand) stages a program as in V-B: a second `[[bin]]` plus `GuestFile`, with no prepare step. An unprepared kit (no peer built) is `LaneNotPrepared` naming `cargo xtask guest-kit-prepare`, which fails each replicate before any guest launches (CA-4).
- *Depends on.* nothing. The seam is one an existing in-guest cell already uses, and V-B showed the staging and return work under the benchmark form.

**PD-9 — A replicate retains its guest console beside the record.** ADR-0333. **APPROVED 2026-11-19, option (a): retain every replicate's console in every outcome, not committed; `benchmarks/results/**/artifacts/` goes into `.gitignore`.**
- *Context.* US-GBS-03 requires the guest console to be retained with the result in every outcome (AC (d): "when a guest run ends in any state"). A body cannot write a file next to the record. A series metric keeps raw values only up to 1,000 per replicate and a histogram above that; the family returns about 5,000 values, so a replicate's console is the only raw copy of its samples.
- *Problem.* Without retention a guest that boots but misbehaves leaves nothing to look at; the record only says it failed. Retaining every console costs about 40 KB per replicate (about 1.2 MB per run of 30, up to about 3 MB at 105 KB) and the directory is pulled with the result.
- *Options.* (a) `Replicate::retain_console(text)` writes `artifacts/<profile>/<block>-<position>-<arm>/console.log` under the result directory, create-new, for every replicate in every outcome; no schema change and no note; a no-op under smoke; `benchmarks/results/**/artifacts/` goes into `.gitignore` (the record and report stay committed as today, the consoles are kept with the pulled directory). (b) The same call, made only for failed replicates; AC (d) is reworded to "a failed guest's console is retained", the result directory shrinks, and a successful replicate's raw values survive only as the histogram. (c) As (a) and commit the consoles: about 1.2 to 3 MB per committed run. (d) The console as a record field: grows every line by tens of kilobytes. (e) The family writes the file itself: the directory is not exposed to bodies.
- *Decision.* (a). It keeps AC (d) as written and keeps the raw evidence of a surprising result. The `.gitignore` line is approved with it.
- *Depends on.* nothing.

## Wave: DESIGN / [REF] User approval

The user ruled on every decision on 2026-11-19.

| ID | Verdict | Date | ADR |
|---|---|---|---|
| PD-1 | APPROVED as recommended (option (a)) | 2026-11-19 | 0325 |
| PD-2 | APPROVED: record the staged artifacts' SHA-256, do not refuse on them | 2026-11-19 | 0326 |
| PD-3 | APPROVED: no methodology bump | 2026-11-19 | 0327 |
| PD-4 | APPROVED as option (b): no probe guest; the guest's view comes from each replicate. Reason: "the guest kernel would never be stale... we control it" | 2026-11-19 | 0328 |
| PD-5 | APPROVED as recommended (fresh guest per replicate) | 2026-11-19 | 0329 |
| PD-6 | APPROVED: console return | 2026-11-19 | 0330 |
| PD-7 | APPROVED as recommended, including the workspace-root change (DDD-11); the shape follows PD-4 (no port) | 2026-11-19 | 0331 |
| PD-8 | APPROVED: option (a), one metric `forwarded_round_trip_ns` | 2026-11-19 | 0332 |
| PD-9 | APPROVED: option (a), every replicate's console retained and not committed; `benchmarks/results/**/artifacts/` in `.gitignore` | 2026-11-19 | 0333 |

Scope-limit rulings (2026-11-19): the first family's unmeasured paths (host-driven vsock leg, other flow kinds, the owner's accept-and-pair work) are tracked by [#370](https://github.com/overdrive-sh/overdrive/issues/370); BPF program statistics inside a guest by [#371](https://github.com/overdrive-sh/overdrive/issues/371); the console's throughput is ruled a property of the mechanism and has no issue. Already tracked: unpinned vCPUs #368, fixed guest shape #369, VM-launching guest families #347, regression gating #361, in-guest kernel mTLS #303.

## Wave: DESIGN / [REF] Interface contracts

Rust signatures are contracts pinned by DESIGN for public and cross-crate API; private structure belongs to the crafter. Names describe behaviour (`domain-naming.md`). New public surface carries domain values in types, not raw primitives (`.claude/rules/rust.md` § Newtypes), reusing what exists: a vCPU count is `std::num::NonZeroU8`, the type `VmConfig.vcpus` and `vcpus_for` already use for it (`crates/overdrive-core/src/vm/config.rs:877`, `:367`); a vsock CID is `overdrive_core::guest_transport::GuestCid` (`crates/overdrive-core/src/guest_transport/mod.rs:51`; `CidBand::cid(n)` yields one); a guest memory size has no existing type (`MemoryPlan` is a derived layout, `Resources.memory_bytes` and `GuestBoot.memory_mib` are raw), so this feature adds one, `GuestMemoryMib` (DDD-4 below). The kit's own raw fields (`GuestBoot.vcpus`, `.memory_mib`, `.cid`, `CidBand::raw`) are unchanged; this feature's functions convert at that boundary.

### DDD-1 — Registry and attribute

`SUBSTRATES = ["native", "guest"]` in `overdrive-bench::registry`. `substrate = guest` is accepted as a bare identifier; every other rejection row of the parent's DDD-2 table is unchanged except these three:

| Rejection | Message |
|---|---|
| substrate identifier not registered | ``substrate `<value>` is not registered; the registered substrates are: native, guest`` |
| substrate identifier `guest` | removed (accepted) |
| unknown guest clock source | ``guest clock source `<value>` is not one of init_elapsed_ms, guest_monotonic`` |

`BenchmarkDecl.substrate` stays the registry string; the runner maps `"guest"` to `Substrate::Guest`. The guest clock vocabulary gains `GuestClock::Monotonic` (identifier `guest_monotonic`, record string `"CLOCK_MONOTONIC (in-guest Instant, 1 ns)"`): a duration the guest program measured with `Instant` on the guest's `CLOCK_MONOTONIC`, in whole nanoseconds, never added to or subtracted from a host instant (DDD-10 of the parent, unchanged). `bench-report validate` accepts `clock.guest` as `null` or either of the two strings. A `guest` benchmark declares no `bpf_stats` (stated in the catalogue and methodology, with [#371](https://github.com/overdrive-sh/overdrive/issues/371) as the tracking issue for reading BPF statistics inside a guest; review enforces it).

### DDD-2 — Record: the optional `guest` block and the `guest_observed` line (schema 1)

`run_started` gains `guest`, present iff `substrate == "guest"`; absent otherwise (a native record is byte-identical to today's). It holds what is true before any guest boots:

```json
"guest": {
  "launch": "metal_host",
  "boot": "per_replicate",
  "vcpu_scheduling": "unpinned",
  "settings": {
    "governor": { "absent": "not visible from a guest" },
    "smt":      { "absent": "not visible from a guest" },
    "boost":    { "absent": "not visible from a guest" }
  }
}
```

A guest record also carries at most one `guest_observed` line, the guest's view as the first reporting replicate reported it (a record line like the others, `"t": "guest_observed"`). `vcpus` and `memory_mib` are configuration, not observation: the shape the kit booted the guest in, reported by the family with the view because `overdrive-bench` has no kit dependency to read them from; they stay on this line (no comparison, no dependency added) rather than in `run_started`, which would need the benchmark core to hold a copy of the kit's constants:

```json
{
  "t": "guest_observed",
  "vcpus": 2,
  "memory_mib": 512,
  "kernel": "7.0.0-38-generic",
  "clocksource": "kvm-clock",
  "artifacts": {
    "overdrive_init": "<64 lowercase hex digits>",
    "overdrive_guest_peer": "<64 lowercase hex digits>",
    "ovd_fwd": "<64 lowercase hex digits>"
  }
}
```

Validator rules (each names the offending field): `guest` present iff the substrate is `guest`; `launch` is `metal_host`; `boot` is `per_replicate`; `vcpu_scheduling` is `unpinned` (#368 adds `pinned`); `settings` has exactly the three keys and each is exactly `{"absent": "not visible from a guest"}`; unknown keys refused. `guest_observed`: accepted only after the start canary of a record whose `run_started` carries `guest`; at most once; the first `ok` replicate line of a guest record is preceded by it (the report reads the guest view for every record that has ok samples, and the runner fails a replicate that reports none, so the runner's own record always satisfies this); `vcpus` and `memory_mib` positive integers within the writer's types (`vcpus` 1..=255, `memory_mib` 1..=4294967295); `kernel`, `clocksource` and each of the exactly three `artifacts` keys in the host block's three forms; a text artifact value is 64 lowercase hex digits; unknown keys refused. No rule requires the line to precede the end canary or a terminal line: the record's existing line-order rules already place every line before the terminal line. `launch = metal_host` is true by construction: the benchmark form runs on the exclusive host, and `native-preflight.sh` refuses a virtualized host (`probe=virtualization`). `vcpus` and `memory_mib` are the values of `LANE_GUEST_VCPUS` and `LANE_GUEST_MEMORY_MIB`, which the family reports with the view. A guest record with no ok replicate may have no `guest_observed` line.

`run_refused` is unchanged: a guest benchmark's refusals are the host's, with the host settings, the exit statuses (`Refused`, `Unreadable` 78) and the `Error` enum as today. No guest setting is pinned.

Report (a pure function of the record): the header gains, after the `host` lines and before `bpf stats`, only for a record with a `guest` block: `- guest launch: metal host (not nested)`, `- guest boot: fresh guest per replicate`, `- guest vcpu scheduling: unpinned`, one `- guest <key>: <value>` line each for governor, smt and boost (printing `absent: <reason>`), then the observed lines `- guest vcpus: 2`, `- guest memory mib: 512`, `- guest kernel: <value>`, `- guest clocksource: <value>` and one `- guest artifact <key>: <value>` line per artifact. A guest record with no `guest_observed` line prints each of the seven observed lines (`vcpus`, `memory mib`, `kernel`, `clocksource`, and one `artifact <key>` line per artifact) as `absent: no replicate returned a guest view`. The `substrate` line prints `guest`. A native report is byte-identical.

### DDD-3 — Methodology

`METHODOLOGY_VERSION` and `schema` stay 1. `methodology.md` gains the policy sentence at `:3-5` and the `:17-19` rewording (the table in PD-3), and a section "Guest substrate": isolation (a guest launched on the exclusive Metal host, fresh per replicate, never nested), eligibility (the full host list on the real host before any guest boots; no guest setting is pinned; the guest's kernel and clocksource, the shape and the staged-artifact digests are recorded from the first replicate that reports them, and later replicates are not compared with it), clock boundaries (the guest clock is the guest's own and is never combined with a host stamp), vCPU marking (count recorded, scheduling `unpinned`), staged artifacts (the record names what ran by digest; the benchmark form does not rebuild the staged guest artifacts, so run `cargo xtask guest-kit-prepare` at the commit being measured), canary (the loopback canary is a host measurement and says nothing about the guest), comparability (never across substrates; arms are compared inside one record), `bpf_stats` (a guest benchmark declares none; reading BPF statistics inside a guest is tracked by #371), failure accounting (a guest failure, including an unprepared kit, is a failed replicate; the console is retained; a run whose replicates all failed still exits 0, and exit status 0 does not imply that any replicate was ok: read `replicates_ok`).

### DDD-4 and DDD-7 — The guest's view (`overdrive-bench`, public, used by families and spikes)

There is no guest port and no probe guest. A replicate reports what its guest saw and ran through `Replicate`:

```rust
// overdrive_bench (re-exported at the crate root)
/// The guest's view of a run. `HostValue` is the record's existing three-form value
/// (text, absent with reason, unreadable with error); it becomes `pub` and is re-exported.
/// `vcpus` and `memory_mib` are configuration, not observation: the shape the kit booted the
/// guest in, carried from the kit's `LANE_GUEST_VCPUS` and `LANE_GUEST_MEMORY_MIB`. `kernel` and
/// `clocksource` are read in the guest.
pub struct GuestView {
    pub vcpus: NonZeroU8,               // std::num::NonZeroU8, as VmConfig.vcpus
    pub memory_mib: GuestMemoryMib,
    pub kernel: HostValue,
    pub clocksource: HostValue,
    pub artifacts: GuestArtifacts,
}
/// The three staged files the kit copies into the guest image, each its SHA-256, computed once
/// per run by the family's host half (not in the guest).
pub struct GuestArtifacts {
    pub overdrive_init: HostValue,
    pub overdrive_guest_peer: HostValue,
    pub ovd_fwd: HostValue,
}

/// A guest memory size in MiB: positive. Owner: `overdrive_bench::record` (the record's
/// vocabulary; re-exported). No existing type names this concept and `overdrive-bench` takes no
/// dependency to borrow one. `new(NonZeroU32)` is infallible; `FromStr` accepts decimal digits
/// only and rejects zero and anything else with a typed `GuestMemoryMibError`; `Display` is the
/// decimal digits; `Serialize`/`Deserialize` are the JSON integer the record carries (the same
/// digits as `Display`; deserialising 0 is refused naming `memory_mib`).
pub struct GuestMemoryMib(/* NonZeroU32 */);

impl Replicate {
    /// Reports the guest view of this run.
    /// Precondition: the benchmark declares the `guest` substrate; otherwise a `ReplicateFailure`
    /// (a native record may not carry a `guest_observed` line).
    /// Postcondition: the first call of the run fixes the run's guest view and the runner appends
    /// the `guest_observed` line (DDD-2) before the line of the replicate that made the call. A
    /// later call returns `Ok(())` and changes nothing: the operator fixes the kernel, the staged
    /// artifacts and the guest shape, and the benchmark runs under exclusive admission on a bound
    /// checkout (ADR-0328), so later reports are not compared. Under `smoke` only the effect (the record line) is
    /// skipped and `Ok(())` is returned: the precondition failure and the runner rule below apply
    /// exactly as in a real run. A source that does not exist in the guest is
    /// `HostValue::Absent`; one that exists and cannot be read is `HostValue::Unreadable`.
    /// Runner rule: a guest-substrate replicate whose body returns `Ok` without having called
    /// this method is failed by the runner (reason `the replicate reported no guest view`), so the
    /// record the runner writes always satisfies the validator's rule that the first `ok`
    /// replicate line follows the `guest_observed` line. A replicate that fails without
    /// reporting stays `failed` with its own reason.
    pub fn report_guest_view(&mut self, view: GuestView) -> Result<(), ReplicateFailure>;
}
```

A view reported by a replicate that later fails stays the run's view. No guest setting is pinned and the guest kernel and clocksource never refuse. A kit that is not prepared, a guest that does not reach READY, a program that exits non-zero and output that does not parse are replicate failures (DDD-5, DDD-6), never run-level refusals. `overdrive-bench` takes no new dependency; the family's host half launches its guests through the kit and a spike's body does the same (ADR-0331).

### DDD-5 and DDD-6 — The replicate and the guest program

Per replicate (family host half), in order, each step's failure returns `Err(ReplicateFailure::fixture(..))` with the named reason, so the replicate is `failed` (counted, excluded from `replicates_ok` and every statistic) and a smoke run panics naming the benchmark: provision the kit (`GuestKitError::LaneNotPrepared` names `cargo xtask guest-kit-prepare`; it fails the replicate as `guest boot: <cause>` before any guest launches, so an unprepared kit fails every replicate at once, CA-4); `run_in_guest_program` with the family's program (an outer `Err` is `guest boot: <cause>`); `rep.retain_console(console)` in every outcome that produced a console; `ProgramRun.exit` `Err` is `guest run: <cause>`; a non-zero exit is `the in-guest program exited <n>`; parse the two readings (kernel and clocksource, each exactly once; a missing, repeated or malformed reading is `the guest returned no intact readings: <detail>`) and call `rep.report_guest_view(view)?` with the run's artifact digests (computed once per run in this host half from the staged files `GuestKit` names: `init_bin`, `peer_bin`, `forwarder_module`); parse the series block (below); a missing, short or corrupt series block is `the guest returned no intact forwarded_round_trip_ns block: <detail>`; record the series. A guest never outlives its replicate. No step records the failure as a metric failure and returns `Ok`.

```rust
// overdrive_testing::guest_lane
/// The shape `run_in_guest_cell` boots a production-init guest in; `run_in_guest_program`
/// and the benchmark record read the same values. The family builds `GuestMemoryMib` from
/// `LANE_GUEST_MEMORY_MIB`; the kit's raw `GuestBoot` fields take `.get()` of each.
pub const LANE_GUEST_VCPUS: NonZeroU8 = NonZeroU8::new(2).unwrap();       // std::num
pub const LANE_GUEST_MEMORY_MIB: NonZeroU32 = NonZeroU32::new(512).unwrap(); // std::num

pub struct ProgramRun {
    /// The program's exit status as the guest's `EXIT` reported it, or why none arrived
    /// (no READY within READY_BOUND, no EXIT within `bound`, a beacon violation, the VMM
    /// still running after the post-EXIT wait).
    pub exit: Result<i32, GuestKitError>,
    /// The guest console as it stands when the run ends, in every outcome.
    pub console: String,
}

/// Boots `spec` (PID 1 must be `GuestInit::OverdriveInit`) at `cid`, plays the node beacon as
/// `run_in_guest_cell` does (holds the guest owner's control session), runs `program` as the
/// operator command after loading the vsock loopback module, waits for EXIT within `bound`,
/// and ends the guest.
/// Postcondition: the VMM has exited and been reaped before return, in every outcome. The
/// console is complete only when the VMM exited on its own after EXIT; a VMM that did not exit
/// within the post-EXIT wait makes `exit` an `Err` (the wait result is never discarded).
/// The outer `Err` is only for a failure before a VMM exists (image build, listener bind,
/// launch); a failure after the VMM exists is `ProgramRun.exit = Err(..)` with its console.
pub fn run_in_guest_program(
    kit: &GuestKit, spec: &GuestImageSpec, program: &[String],
    cid: GuestCid, work: &Path, bound: Duration,
) -> Result<ProgramRun, GuestKitError>;

// overdrive_testing::guest_kit
impl GuestImageSpec { pub const fn benchmark() -> Self }   // OverdriveInit, forwarder_module, loopback_module, no extra files
// cid_bands: GUEST_BENCHMARK = CidBand { name: "overdrive-bench guest benchmark", first_offset: 0x2050, width: 16 }; REGISTERED grows to 6 entries (registry_is_sound still holds)
```

`run_in_guest_cell` keeps its behaviour and shares its body with `run_in_guest_program`.

CID per replicate: the family takes `GUEST_BENCHMARK.cid(n)` with `n` a family-private counter of the replicates this process has started, modulo the band width (16). Replicates run sequentially (one guest at a time on an exclusive host), so a CID is in use by at most one guest, and from the seventeenth replicate on a CID is reused. Reuse is safe because `run_in_guest_program` reaps the VMM before it returns (its postcondition), the VMM's exit releases its vhost-vsock file and removes the CID from the hash (`drivers/vhost/vsock.c:946`, body `:753-786 @ v7.0`), and V-C checks that the CID is reusable after the VMM is gone, including the wrap to replicate 17. The V-A and V-B2 spikes do not show reuse: they cycled 256-wide and unbounded CID ranges (V-A's probe `next_cid` modulo 256; V-B2's CIDs `0x1003000` to `0x1003059` over 90 cycles), so no CID was used twice in 90 cycles; the reuse rests on V-C and the kit's unchanged launch path, and a launch that fails on a busy CID is a failed replicate (`guest boot: <cause>`).

Guest program output grammar (`overdrive_testing::guest_app`; test-support surface whose exact argv is the crafter's, but whose output grammar is pinned because the record's integrity rests on it):

- guest readings (printed by every program a guest benchmark runs; the first family's program prints them through `overdrive_testing::guest_app`'s printer, which DISTILL scaffolds beside the parser so one owner holds the grammar, and a spike's program may use that printer or print the pinned lines itself): after the program's timed phase and the leading `\n` below, one line `guest-reading: <key>=<value>` for each of `kernel` and `clocksource`; a source that does not exist is `absent:<reason>`, one that cannot be read `unreadable:<error>`. They sit between the leading `\n` and the series block. The artifact digests are not guest readings: the family's host half computes them (DDD-4).
- subcommand `forwarding-bench`: after its timed phase writes one `\n`, then the two reading lines, then the series block for `forwarded_round_trip_ns`: `bench-series-begin: <metric>`; one or more `bench-values: <metric> <v1>,<v2>,...` lines of at most 16 decimal nanosecond values each, in measurement order; `bench-series-end: <metric> <checksum>` where `<checksum>` is the SHA-256, 64 lowercase hex digits, of the values as consecutive little-endian `u64` bytes. The count is not printed: the host half checks the parsed count against the `R` it asked for. The leading `\n` ends any unterminated `overdrive-init` log fragment open on the shared console (ADR-0330 states why), so the first reading line starts a console line. The program needs no drain, flush or sleep before it exits: the console is complete once the VMM has exited (V-B2). It exits 0 when it ran to the end and non-zero when a flow could not be driven; no failure is printed as a series line. The number of timed round trips `R` reaches the program as an argument.
- Host half, one owner with the printer: the parser lives beside the printer in `overdrive_testing::guest_app`, so one owner holds the grammar for the first family's program and its parser:

```rust
// overdrive_testing::guest_app
/// One guest reading as the guest printed it.
pub enum GuestReading { Text(String), Absent(String), Unreadable(String) }
/// What an intact program return holds.
pub struct ProgramReturn {
    pub kernel: GuestReading,
    pub clocksource: GuestReading,
    pub values: Vec<u64>,
}
/// Why a console holds no intact return; each variant's `Display` names its cause.
pub enum ProgramReturnError {
    ReadingMissing { key: String }, ReadingRepeated { key: String }, ReadingMalformed { key: String },
    BlockMissing, BlockRepeated, BlockMalformed,
    CountMismatch { expected: usize, found: usize }, ChecksumMismatch,
}
/// Parses `console` for the two readings and exactly one block of `metric` holding
/// `expected_count` values. Postcondition on `Ok`: every reading appeared once and the block's
/// checksum equals the checksum recomputed over the parsed values. Anything else is the matching
/// error; there is never a short series.
pub fn parse_program_return(console: &str, metric: &str, expected_count: usize)
    -> Result<ProgramReturn, ProgramReturnError>;
```

  A reading line and a begin line are matched only at the start of a console line (the strict grammar). Each of the two reading keys must appear exactly once. Exactly one begin line for the metric, the value lines that follow it and one end line must exist; the parsed value count must equal `expected_count` and the end line's checksum must equal the checksum recomputed over the parsed values; any other outcome fails the replicate (above): the family maps `ReadingMissing`, `ReadingRepeated` and `ReadingMalformed` to `the guest returned no intact readings: <cause>` and the block variants to `the guest returned no intact forwarded_round_trip_ns block: <cause>`. Console lines that do not begin with those prefixes are ignored; a kernel message or `overdrive-init` fragment that splices into a value or end line breaks the block's count or checksum and fails the replicate (B2 semantics), never a short series. Host-half acceptance case: a console whose first reading line follows an unterminated `overdrive-init` fragment (the program's `\n` ends the fragment) parses to intact readings and an intact block; a console whose first reading or begin line is glued to a fragment finds no such line and fails the replicate.

### DDD-8 — The first guest family

Family `guest-flow-forwarding`, benchmark `guest_flow_forwarding`.

```text
#[overdrive_bench(
    family = "guest-flow-forwarding", substrate = guest, arms = ["forwarding"], replicates = 30,
    metric(name = "forwarded_round_trip_ns", unit = ns, table = "forwarded-path", series),
    clock(guest = guest_monotonic),
    smoke(guest = "cgroup-default", profile = "nested-launch"),
)]
```

No `bpf_stats`, no `reference`, one implicit profile. One arm, so no A/A control and no comparison table; the report shows each arm's distribution (p50, p90, p95, p99, values, replicates, failures). The in-guest program, in one fresh guest per replicate, with the production `overdrive-init` as PID 1 and `ovd_fwd` and the vsock loopback module loaded: pairs one flow (the `TcpAccept` request on the loopback vsock connection to `GUEST_INBOUND_PORT`, answered by the owner's connection to the application listener on `workload_addr:port`), sends `W` unrecorded and `R` timed 64-byte requests that the application echoes, and times each (`forwarded_round_trip_ns`: write on the vsock side to the echoed reply read on the vsock side, closed loop). Indicative sizes, pinned by the catalogue row: `W` = 200, `R` = 5,000 (stored as a histogram, parent DDD-5). A replicate returns about 5,000 values, about 36 KB on the console, about 0.6 s outside the timed phase (PD-6; 0.65 s measured for a 41 KB block, V-B2); a whole replicate cycle is about 2.75 s. The workload address, the launch token and the port reach the program as arguments; the family's host half derives them from `lane_workload_addr(cid)` and `LANE_LAUNCH_TOKEN` as the in-guest lane does.

Catalogue row (DELIVER writes it): boundary "the `ovd_fwd` forwarder's data path under a flow the guest flow owner paired, as `overdrive-init` runs it in a workload VM"; arm `forwarding`; workload matrix as above, 2 vCPUs, 512 MiB, fresh guest per replicate; clock boundary "guest `CLOCK_MONOTONIC` read by the guest program; both stamps of a measurement are in the same process; no host stamp is combined with them"; result owner `overdrive-init` (guest flow owner) and `overdrive-fwd`. Caveat stated in the row: the owner's accept-and-pair work is setup and not measured, the vsock leg is loopback inside the guest, the vhost transport, the host flow owner, guest-originated TCP flows and datagram flows are not measured (tracked by #370), the family declares no `bpf_stats` (BPF statistics inside a guest: #371), and the two vCPUs are shared by the owner thread, the peer's threads and the guest kernel (unpinned, #368). The row also states the D9 rule: a family that launches VMs is not a guest benchmark and stays `native` (#347).

### DDD-9 — The retained console

```rust
impl Replicate {
    /// Writes `text` to `<result dir>/artifacts/<profile>/<block>-<position>-<arm>/console.log`
    /// (create-new). A second call by the same replicate, or a write failure, is a
    /// `ReplicateFailure`. Under `smoke` only the effect (the file) is skipped and `Ok(())` is returned: a guest-substrate precondition failure, the M-1 failure and a second `retain_console` call fail the replicate exactly as in a real run.
    pub fn retain_console(&mut self, text: &str) -> Result<(), ReplicateFailure>;
}
// RecordSink gains: fn write_artifact(&self, dir: &Path, relative: &Path, bytes: &[u8]) -> io::Result<()>;  // create-new, creates parents
```

### DDD-10 — Smoke placement

The guest family declares `smoke(guest = "cgroup-default", profile = "nested-launch")`. `nested-launch` stages the VMM, the node setup and everything the inner `GuestKit::provision` resolves inside the outer guest: the static `overdrive-init` at `/opt/overdrive-testing/prebuilt-overdrive-init` (`guest_suite.rs:1120-1124`, found first by `vm_fixture.rs:1051-1055`), the forwarder module (`guest_suite.rs:1097`, found first by `guest_kit.rs:1248-1252`), the `vsock_loopback` module source (`guest_suite.rs:271`, in `NESTED_GUEST_KIT_MODULE_SOURCES` staged at `:1099-1101`), the host kernel image and the VMM; the static `overdrive-guest-peer` is in the outer guest because every kit image installs it at `/sbin/overdrive-guest-peer` (`guest_kit.rs:563`), where `prepared_guest_peer` finds it first (`guest_kit.rs:1260-1264`). `nested-launch-lane` adds exactly one more file, the prepared `overdrive-init` libtest binary (`guest_suite.rs:1217-1224`), which the smoke does not run: its program is a peer subcommand. So the smoke needs no staging beyond `nested-launch`, and no open item remains. The smoke boots a nested kit guest inside the outer test guest; the nesting is immaterial because the smoke takes no measurement and asserts no timing. The artifact digests are computed from the paths the inner kit names, so the smoke exercises that code too.

### DDD-11 — Spike-crate workspace resolution

`overdrive_testing::vm_fixture::workspace_root()` (crate-private; no signature change) returns the exported `OVERDRIVE_METAL_SOURCE_ROOT` when it is set and `OVERDRIVE_METAL_EXECUTION_CLASS` is `ordinary` or `exclusive`; in every other context it resolves `cargo locate-project --workspace` from the cwd, as today. Postcondition: under the benchmark form the kit's staged artifacts (the static peer, `overdrive-init`, the forwarder module, the in-guest binaries) are resolved under the bound checkout whatever the process's cwd, for a maintained family and for a `--spike` crate alike. The kit's four call sites (`guest_kit.rs:1266`, `guest_lane.rs:156`, `forwarder_module.rs:56`, `vm_fixture.rs:1057`) all reach it, so none changes and a benchmark adapter never changes directory. A test covers the `exclusive` case; the `ordinary` and unset behaviours are unchanged. The benchmark form exports the bound checkout as the source root in every run (`infra/metal/bootstrap.sh:482-485` and `:842`). The change serves a `--spike` crate. V-B found that `cargo bench` runs it with cwd = the spike's own directory, which is its own workspace, where the kit cannot find its artifacts: without the change every guest boot there failed (90 of 90 replicates), and a spike author would have to change directory to the repository root before the first boot, a workaround the design does not require of anyone. A maintained family's cwd is its package directory inside the workspace, where `cargo locate-project` already finds the right root.

### Other pinned behaviour

- **Failure shapes (US-GBS-03).** Boot failure (no READY within `READY_BOUND`, a launch error): the replicate is `failed` with `guest boot: <cause>`, its console retained, and no host measurement is taken in its place. Guest death after READY or no EXIT within `bound`: `failed`, console retained. A flow the program could not drive: non-zero exit, `failed`. A killed benchmark process leaves a record with no terminal line (reported incomplete); no guest VMM outlives it because the VMM is a child in the benchmark unit's control group and the unit is `KillMode=mixed` (`infra/metal/bootstrap.sh:870`); `GuestKitVm::drop` ends it on any normal or unwinding exit. The child-stays-in-the-group half is a kernel fact (Kernel claims, item 3); the unit-stop half is systemd's, and V-C checks it.
- **No change** to the benchmark form (`bootstrap.sh`), to `guest-kit-prepare`, to the exit statuses, to `Error`, to the statistics, to the canary, or to any native family.

### Rule text the orchestrator edits (not the architect)

`.claude/rules/testing.md`: § "Classify external execution" / benchmark row and the native-host section's benchmark-form paragraph: one sentence each that a benchmark's `guest` substrate is a guest launched directly on the exclusive host and is never a placement group, and that only the smoke variant runs a benchmark body in a test guest. ADR-0299's status line pointing at ADR-0325 is written (by the architect, with this design). ADR-0301 has no "no field is omitted" sentence; that sentence is in the parent feature delta (`docs/feature/native-benchmark-program/feature-delta.md:735`), a wave record that is not edited: CA-2 records the second optional place. `.gitignore` gains `benchmarks/results/**/artifacts/` (approved with PD-9; DELIVER lands it with the first retained console). ADR-0296's dependency list is unchanged (ADR-0331). `benchmarks/methodology.md` and `benchmarks/catalogue.md` text is DELIVER's, per DDD-3 and DDD-8.
## Wave: DESIGN / [REF] Technology choices

| Choice | Version and licence | Rationale and rejected alternatives |
|---|---|---|
| `sha2` in `overdrive-testing` (the peer's series checksum and the host-half parser beside it) and in `overdrive-bench-families` (artifact digests, computed once per run); added to a crate's dependencies where not already present | workspace dependency `sha2 = "0.10"`, MIT OR Apache-2.0 | a standard crate already in the workspace; a hand-rolled FNV checksum (what the spike used) is rejected by `rust.md` § Dependencies |
| No new dependency for `overdrive-bench` | n/a | ADR-0331; the family launches its guests, so the benchmark core has no guest port and `overdrive-testing` stays a dev-dependency; rejected: a port and adapter in the core (needs the kit as a normal dependency, no method left to call), an adapter in the families crate (spike dependency weight), a port in `overdrive-testing` (cycle) |
| No new crate or tool | existing workspace dependencies | the guest kit, the guest peer, `serde`, `thiserror` and the production `CloudHypervisorVmm` already exist; the alternatives (a new in-guest binary crate, a package bin in the families crate, a lib split of `overdrive-init`) are rejected in ADR-0332 |

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| "this run may take samples" (existing) | `overdrive-bench` runner | the host matches the pinned settings | `HostProbe`, build flags | any production state |
| "this replicate has a guest" (new) | `run_in_guest_program` | the guest reached READY within `READY_BOUND` and ended | the beacon exchange | the run, other replicates, the host canary |

There is one gate beyond host eligibility. A gate for "this run may boot measured guests" does not exist: no guest boots before the replicates, and a guest that cannot boot fails its replicate (ADR-0328).

**Gate G-1 — host eligibility.** Unchanged (parent G-1). It decides whether any replicate, and so any guest, runs: a refused host ends the run before the family body is called.

**Gate G-2 — a replicate's guest.**
- *Existing evidence:* the in-guest lane boots a production-init guest, reaches READY and runs one operator command (`crates/overdrive-testing/src/guest_lane.rs`, `run_in_guest_cell`); production entry for the benchmark is `run_main` under `cargo xtask metal run --benchmark`; no production lifecycle is touched.
- *Owner:* `run_in_guest_program` for the guest's life; the family's host half for the result's integrity. *Promise:* the kit is prepared, the guest reached READY, the program's EXIT arrived within `bound`, the VMM exited, the console holds the two readings and one intact series block of the expected count, and the replicate reported a guest view (`report_guest_view`; the runner fails a guest-substrate replicate whose body returned `Ok` without calling it, reason `the replicate reported no guest view`, so the record the runner writes always satisfies the validator). Later views are not compared with the first (ADR-0328). *Affected state:* the replicate's outcome only.
- *Failure projection:* an unprepared kit, boot failure, timeout, guest death, beacon violation, a VMM that does not exit, a non-zero program exit, missing or malformed readings, a missing, short or disagreeing series block: the family returns `Err(ReplicateFailure::fixture(..))`, so the replicate is `failed` with the cause, the retained console (where a guest ran) and no statistic; a body that returns `Ok` without reporting a view: the runner fails it (`the replicate reported no guest view`); a clean exit with intact readings, a reported view and one intact block: `ok`.
- *Explicitly unaffected:* the run, other replicates, the canary, the host half's own process, host eligibility.
- *Ordering:* inside the replicate; one `READY_BOUND` and one program bound per replicate.
- *Counterexample:* ending the run on the first guest failure, or on an unprepared kit, would turn one transient boot failure into a lost run and would hide how many replicates are sound; recording a bad block as a metric failure and returning `Ok` would let the smoke test pass over a guest that returned nothing; gating on a pinned guest kernel would need a guest booted before the record starts, which the user ruled out (PD-4).
- *Evidence lane:* seeded fixture tests of the host half's parsing and failure mapping (`parse_program_return`: missing block, missing or repeated reading, a first reading line or begin line glued to an unterminated `overdrive-init` fragment, a first reading line following one after the program's `\n`, wrong count, mismatched checksum, non-zero exit; each failure makes the replicate fail and a smoke run panic); crafter-owned unit tests of the runner with replicate bodies that report scripted views (the first view writes one `guest_observed` line before that replicate's line, a later view writes none and changes nothing, a body returning `Ok` without a view is failed `the replicate reported no guest view`, a guest record with no view writes no line and the report says so); a retained `--benchmark guest-flow-forwarding` run on the Metal host, with one deliberate host drift retained as a refusal with no guest booted (K3) and one unprepared kit retained as a run of failed replicates naming `cargo xtask guest-kit-prepare`.

Required boundary scenarios (DISTILL turns each into an executable obligation): (1) gate available: a conforming host and prepared kit proceed to samples and the record validates, with one `guest_observed` line before the first `ok` replicate; (2) gate unavailable or timed out: an unprepared kit fails every replicate with `guest boot: <cause>` naming `cargo xtask guest-kit-prepare`, takes no sample and boots no guest, and a guest that never reaches READY fails its replicate; (3) unrelated state: a native family run beside this change still validates and renders byte-identically; (4) late success: a guest that reaches READY after `READY_BOUND` is ended and its result is never used; (5) disconnect: a guest that dies after READY is a failed replicate, replicates 1 to 11 stay and 13 to 30 run; (6) feature disabled: with no guest benchmark selected no VMM exists and no `guest` block or `guest_observed` line is written; with a drifted host the family body is never called and no guest boots; (7) the smoke boots a nested kit guest inside the outer guest, passes one replicate with no timing assertion, and fails naming the benchmark when the block is missing; (8) no guest view: a guest-substrate replicate whose body returns `Ok` without reporting a view is failed `the replicate reported no guest view`, and the record still validates. For ordering and cancellation the handoff names a seeded fixture invariant over the runner (the family body is never called when the host is refused, the directory line before `run_started`, `run_started` before any replicate, at most one `guest_observed` line, the first `ok` replicate line of a guest record preceded by it, no replicate line after a terminal line); the real-guest effects are Tier-3 evidence on the Metal host through the benchmark form (a test never spawns the benchmark binary; the retained run is the evidence lane, as in the parent's DDD-9).

## Wave: DESIGN / [REF] Formal protocol model

Not applicable: the host half and the in-guest half run one strictly sequential script per replicate (boot, READY, EXEC, program, EXIT, console read, end) with no state shared between owners and no concurrent actors, and the only crash-sensitive artifacts are an append-only record and a disposable guest whose lifetime the benchmark unit's control group bounds (V-C), so there is no interleaving a model would enumerate.

## Wave: DESIGN / [REF] Validation items

The contract-shaping items V-A, V-B and V-B2 were spiked on the Metal host and held; no contract-shaping item is open. The orchestrator ran the spikes; the architect ran nothing.

| ID | Item | Class | Status |
|---|---|---|---|
| V-A | Cost and repeatability of one guest cycle on the Metal host: image build, launch to peer ready, one trivial exec, shutdown to VMM exit, repeated sequentially. **Prediction:** a cycle of 0.8 to 1.4 s boot, 60 guests under 2 minutes. **Falsification:** boot p50 over 3 s, any boot or shutdown failure in 90 replicates, or 60 guests over 5 minutes. | contract-shaping: decides DDD-5, the record's `boot` value and CA-1 | **Held.** Metal host, AMD EPYC 8024P, Linux 7.0.0-38-generic, commit `5eaa4f7e4`, through the benchmark form. Lifecycle p50 1,078 ms, p95 1,132 ms, max 1,174 ms over 90 cycles, 0 failed (image 75 ms, boot to peer 948 ms, first exec 8.6 ms, shutdown 40 ms at p50, quantised by `wait_exit`'s 20 ms poll); 60 fresh guests about 65 s. Findings: [findings-guest-boot-cost.md](spike/findings-guest-boot-cost.md); probe `spike-scratch/guest-bench-return/`. Proof scope: the scripted-peer kit path; the production-init guest this family boots is bracketed by the retained production READY cost of 1,279 ms and the in-guest lane that already boots it, and a 28-fold margin to the bound means a different figure would not change the decision. |
| V-B | A `spike-scratch/` benchmark crate run through `cargo xtask metal run --benchmark <family> --spike <name>` boots a guest-kit guest on the exclusive host, runs a prepared in-guest program staged without changing `guest-kit-prepare`, and receives its sample block back with count and checksum intact. **Falsification:** a launch or permission failure under the benchmark form, a payload that cannot be staged without editing `xtask` or `crates/`, or a block that is short, corrupt or missing. | contract-shaping: decides DDD-6 (console or vsock), the staging of the program, O8 and DDD-11 | **Held**, with two findings that are design inputs (cwd: DDD-11; concurrent vsock reader: PD-6 option (b)). Same host, kernel and commit. 70 of 70 replicates intact: 1,000, 100,000 and 1,000,000 samples intact on both console and vsock (console 61 ms, 5.15 s and 52.3 s; vsock 8.2 ms, 14.9 ms and 88 ms). The host-side prerequisites under the benchmark form (umask, the confined VMM's traversal of the staging root, `/dev/vhost-vsock`) held. The payload was staged as a second `[[bin]]` plus `GuestFile` with no change to `IN_GUEST_LANES` or `guest-kit-prepare` and no production change. Findings: [findings-guest-return-path.md](spike/findings-guest-return-path.md); probe `spike-scratch/guest-bench-return/`. Proof scope: a scripted-peer guest through `GuestPeer::exec`, and the 1,000,000-sample arms did not go through `record_series`; the console file and the VMM launch are the same code as the production-init guest's. |
| V-B2 | The production-init guest's console return is complete after the VMM exits. A production-init guest (PID 1 `overdrive-init`, operator command through the beacon, as `run_in_guest_program` runs it) prints a framed series block of the family's size (about 5,000 values) and of 100,000 values after its timed phase, exits, and the guest powers off immediately after `EXIT`; the host reads the console file after `wait_exit`. **Prediction:** the block is whole (count and SHA-256 match) in every replicate. **Falsification:** a short or missing block in any replicate of at least 30, or a console read before the VMM exited. | contract-shaping: decided that PD-6 option (a) holds for the production-init guest, that the program needs no drain step, and the leading newline in the grammar | **Held**, with a precondition now pinned in the grammar (the program's leading `\n`). Metal host, Linux 7.0.0-38-generic (guest the same), commit `5eaa4f7e4`. 180 replicates, each a fresh production-`overdrive-init` guest, console read after EXIT, power-off and VMM exit: no tail loss, no count or checksum mismatch, no kernel or init line inside a block, and echo works in both directions (5,200 closed-loop 64-byte round trips per replicate, byte-equal). `overdrive-init` writes unterminated log fragments to the same console; one open as the program prints its first byte glues the first block line to it: 24 of 60 replicates lost the block without a leading newline, 0 of 30 with one (residual rate bounded at about 10% at 95%; a residual splice is caught by count and checksum as a failed replicate). Whole cycle about 2.75 s median per replicate (launch to READY 1.10 s; EXEC to EXIT 1.58 s, workload about 0.89 s, console print about 0.65 s at about 63 KB/s for a 41 KB block of about 5,500 values), about 83 s per 30 replicates, far inside PD-5's 30 s bound. Round trip p50 41 to 51 us, p95 60 to 64 us. No production change and no drain or flush step are needed. Findings: [findings-production-init-return.md](spike/findings-production-init-return.md); probe `spike-scratch/guest-bench-init-return/`. |
| V-C | A benchmark process killed with `SIGKILL` during a replicate leaves no VMM: after the unit ends, `vhost_vsock_holders()` is empty and the guest's CID is reusable; and a 17th sequential replicate reuses the first replicate's CID without a launch failure (the 16-wide band wraps). **Hypothesis:** the VMM is a child in the unit's control group (`CloudHypervisorVmm` sets `kill_on_drop(false)` and does not move the child; `bootstrap.sh:870` sets `KillMode=mixed`), so the unit's stop kills it; the VMM's exit releases its vhost-vsock file (`.release` is `vhost_vsock_dev_release`, `drivers/vhost/vsock.c:946 @ v7.0`, body `:753-786`), which removes the CID from the hash, waits for RCU readers and sweeps every connected vsock socket through `vhost_vsock_reset_orphans` (`:721-751`, `ECONNRESET` on a socket whose peer CID is no longer registered), so the CID is free and the leaked guest's host flows are reset. **Falsification:** a holder remains, or the 17th replicate's launch fails on its CID. | not contract-shaping: a falsification adds a launch-side guard in test-support code, not a contract DISTILL pins; but a falsified US-GBS-03 AC (e) blocks that step | DELIVER evidence |
| V-D | Reporting items of the first retained `guest-flow-forwarding` run (no gate): the metric's median and spread across replicates; the per-replicate cycle of the production-init guest (launch to EXIT) and the console-return time, so the catalogue row states them; the failed-replicate rate (replicates failed, out of all). | not contract-shaping: changes workload sizes or the catalogue caveat, not an interface | DELIVER evidence |

Proven facts carried from the spikes (kernel 7.0.0-38-generic, commit `5eaa4f7e4`): a kit guest cycles in about 1.1 s with no failure over 90 sequential boots (V-A; its probe cycled a 256-wide CID range, so the 90 cycles reused no CID, and CID reuse across boots rests on V-C and the unchanged kit); a spike crate under `--benchmark ... --spike` boots a guest, runs a staged program and gets 1,000 to 1,000,000 samples back intact over the console and over vsock (V-B); a production-`overdrive-init` guest returns a 41 KB block on the console, read after VMM exit, intact in 180 of 180 replicates once the block starts on its own line, with the two-direction echo through the production owner intact (V-B2). Facts the design leans on that were not re-run: the existing in-guest lane's (a production-init guest reaches READY and runs an operator command; a cell reaches the guest's own inbound listener over vsock loopback and plays the application), read in `guest_lane.rs` and `guest_flow/real_kernel.rs`; the kernel facts are read in v7.0 (Inputs read, Kernel claims). The smoke variant's nested kit guest (DDD-10) is DISTILL scope, not a validation item: the in-guest lane tests already boot a guest in the same outer-guest placement family and the smoke takes no measurement.

## Wave: DESIGN / [REF] Open questions

None open. The user ruled on the scope limitations on 2026-11-19; each is tracked by an existing issue.

**Scope limitations, tracked.**

- **The first family measures the guest side of the forwarding path only: [#370](https://github.com/overdrive-sh/overdrive/issues/370)** ("Benchmark the host-driven vsock leg and the other flow kinds of the guest flow owner"). `guest-flow-forwarding` measures the data path of one flow the owner paired, over a loopback vsock leg. It does not measure the vhost-vsock transport and the host flow owner (the host-driven leg), guest-originated TCP flows (`TcpConnect`) or datagram flows, nor the owner's own accept-and-pair work. Pinned in ADR-0332 Consequences, PD-8 and the catalogue row's caveat; each cites #370.
- **A guest benchmark cannot read the BPF program statistics of programs in the guest kernel: [#371](https://github.com/overdrive-sh/overdrive/issues/371)** ("Benchmark program: read BPF program run statistics inside a guest"). A `guest` benchmark declares no `bpf_stats` (ADR-0325); the runner's statistics descriptor is held by the host process and does not cover the guest kernel. Pinned in ADR-0325, PD-1, the methodology section and the catalogue row; each cites #371.

**Classified as properties of the chosen mechanism (no issue).**

- *Console return throughput.* The console carries about 59 KB/s, so a replicate's return costs about 17 s per MB (PD-6). The user ruled this a property of the mechanism on 2026-11-19; no issue.
- *A fresh guest per replicate, about 1.1 s of lifecycle and about 2.75 s per whole replicate cycle, so a family with thousands of replicates per arm costs hours.* A consequence of the methodology's fresh-fixture rule, chosen for correctness (PD-5); a family declares the `replicates` it can afford. Same footing as "serialising work that would race".
- *One guest at a time, on an exclusive host.* Concurrent guests would load the CPUs the measurement runs on; exclusive admission and sequential replicates are the method, not a cap.

**Already tracked.** Unpinned vCPUs: #368. Fixed guest shape: #369. VM-launching guest families: #347. Regression gating and baselines: #361. In-guest kernel mTLS and its A/B against the host proxy: #303.

## Wave: DESIGN / [REF] Changed Assumptions

- **CA-1. A guest's death is a failed replicate, not an incomplete run.** DISCUSS US-GBS-03 scenario: "Given the guest dies during replicate 12 / Then replicates 1 to 11 are retained and the report says the run is incomplete", and AC (b): "a mid-run guest death leaves the partial record, reported incomplete" (`docs/feature/guest-benchmark-substrate/feature-delta.md`). New: with a fresh guest per replicate (PD-5), a guest that dies in replicate 12 fails that replicate (counted, console retained); replicates 13 to 30 still run and the run finishes. "Incomplete" is reserved for a benchmark process that ends without a terminal line. The scenario and AC (b) read as the new text (approved in PD-5). Slice [03](slices/slice-03-guest-failures-are-reported.md) and the journey `docs/product/journeys/measure-a-boundary-natively.yaml` carry the superseded wording and are updated to this reality; the slice file's text reads as CA-1 and CA-4 state.
- **CA-2. The native record contract's "no field is ever omitted to mean `null`".** The sentence is in the parent feature delta's DDD-4 record paragraph (`docs/feature/native-benchmark-program/feature-delta.md:735`), not in ADR-0301; ADR-0301 is not amended. The parent states: "every field below is required unless marked optional; no field is ever omitted to mean `null`" and "the only optional fields in the schema" are on `run_refused`. New: `run_started.guest` is a second optional place, present exactly when the substrate is `guest`, and a guest record may carry one `guest_observed` line, a line kind native records never have (PD-2).
- **CA-3. Workspace resolution is by source root only in ordinary Metal runs.** `crates/overdrive-testing/src/vm_fixture.rs:1086-1097`: the exported source root is used only when the execution class is `ordinary`; other contexts resolve from the cwd. The benchmark form's class is `exclusive` (finding and reason: DDD-11). New: the source root is also used in class `exclusive`. No ordinary-run or non-Metal behaviour changes.
- **CA-4. An unprepared guest kit is a failed replicate, not a refusal before any sample; only host settings refuse before any guest boots.** DISCUSS US-GBS-01 scenario "An unprepared guest kit refuses before any sample / Then the run refuses naming cargo xtask guest-kit-prepare and takes no sample", example 4 ("the run refuses before any sample, naming `cargo xtask guest-kit-prepare`"), AC (f) ("an unprepared kit refuses naming the setup command, before any sample") and slice 01's "Refusal naming `cargo xtask guest-kit-prepare` when the kit is not prepared"; US-GBS-02 scenario and AC (a) ("refused before any guest boots", "no guest is booted and no sample is taken") and elevator pitch, which hold for the host's settings and for nothing guest-side. New (PD-4 option (b)): with no probe guest there is no guest-side refusal and no guest pin. An unprepared kit makes every replicate fail with `guest boot: <cause>`, the cause naming `cargo xtask guest-kit-prepare`; each fails before a guest launches, so no guest boots, no sample is taken and the run finishes with every replicate counted failed and no `guest_observed` line. The run is not refused and exits 0, as any run whose replicates failed does; exit status 0 does not imply any replicate was `ok`. Two DISCUSS statements change with it: D7 ("the record states the guest's vCPU count") and K2 ("100% of guest records state ... vCPU count") hold for every guest record that has a `guest_observed` line, which carries the shape; a record in which every replicate failed before reporting a view (an unprepared kit, a guest that never boots) states `launch`, `boot` and `vcpu_scheduling` in `run_started` and no vCPU count, and its report prints `absent: no replicate returned a guest view`. The chosen alternative to a run-level abort: the runner has no abort a body can request, and a guest that cannot boot is already a failed replicate (CA-1). A host refusal still happens before any guest boots, with no sample (US-GBS-02 AC (a)). Any "no guest booted" acceptance for a guest-side refusal is removed.

## Wave: DISTILL / [REF] Scenario list

DISTILL, 2026-11-19. Reconciliation passed: DEVOPS artifacts absent (warning, default Metal matrix); DISCUSS and DESIGN disagree only where DESIGN's Changed Assumptions CA-1 to CA-4 say so, and the tests follow DESIGN; 0 unresolved contradictions. 72 tests, 65 pending RED and 7 GREEN guards; every pending test has a full executable body and `#[should_panic(expected = ...)]` naming its own failing assertion; every test carries its contract shape; about half of the 72 are error, edge or boundary cases (the refusals GV2-GV6, GA6-GA14, GA16, GA17, GM4-GM5, GR3, GR5-GR7 and GR9-GR10, the smoke refusals GR14-GR16, the guest failure cases GF1-GF4 and GF6, the unprepared-kit, killed and refused-host reports). Only GF1-GF6 boot a guest, each in its own `cgroup-default` `nested-launch` guest; no test spawns the `overdrive` or benchmark binary or is `native_launcher`. Seventeen tests (GV1-GV6, GP1-GP6, GA6, GA7, GA10, GA11, GA17) share a control precondition and activate together; see `distill/red-classification.md` "Activate together". Full table with traces, owner path, fault, oracle, lane and state per test: `distill/test-scenarios.md`. Summary:

| Group | Tests | Lane | Where |
|---|---|---|---|
| Runner over a guest benchmark: the `guest_observed` line, M-1, view-then-fail, refused host, native untouched, console retention, sink artifacts, ordering invariant (proptest) | GR1-GR16 (15 RED, 1 GREEN) | unit | `crates/overdrive-bench/src/{runner,replicate}.rs` (`mod tests`) |
| `GuestMemoryMib` newtype: display, text and JSON round trips (proptest), refusals | GM1-GM5 | unit | `crates/overdrive-bench/src/record.rs` |
| Registry and attribute: two-entry registry and message, a guest declaration builds, the guest clock set | GS1-GS2, GT1-GT3 | unit, trybuild | `registry.rs`, `tests/compile_fail.rs`, `tests/compile_fail/guest_substrate_accepted.rs` |
| Record validation: the `guest` block, `guest_observed`, the guest clock string, no command takes two records | GV1-GV7 | int-host | `tests/integration/report_validation.rs`, `support.rs` |
| Report: guest conditions between host lines and `bpf stats`, no-view, refused, incomplete, one arm, byte-identical, native untouched | GP1-GP8 | int-host | `tests/integration/report_rendering.rs` |
| Methodology and catalogue | GD1-GD4 | int-host | `tests/integration/methodology_documents.rs` |
| Guest program return: strict console grammar, B2 semantics, round trip and damage (proptest) | GA1-GA17 | unit | `crates/overdrive-testing/src/guest_app/program_return_tests.rs` |
| Guest kit band and image, workspace root in the `exclusive` class | GK1-GK3, GW1-GW2 | unit | `crates/overdrive-testing/src/{guest_kit,vm_fixture}.rs` |
| `run_in_guest_program` failure handling in real guests: no READY, a guest stalled past the READY bound, death after READY, a VMM that outlives its guest's EXIT, a missed EXIT bound, and the clean-exit control, each with the console retained, the VMM reaped and the CID reusable (F3 ruling) | GF1-GF6 (all RED) | int-guest (Metal, `integration-tests,kvm-tests`, `cgroup-default` `nested-launch`) | `crates/overdrive-testing/tests/integration/guest_program_failures.rs` |
| Real-guest runs (walking skeleton, K3 drift, unprepared kit, killed run, CID reuse, spike, V-D) | 7 evidence runs | retained Metal runs | `distill/test-scenarios.md` "Benchmark-run evidence" |

Tags: no Gherkin tags; the scenario table carries lane and state. Property-shaped criteria are proptests at layers 1-2 only (GR12, GM2, GM3, GA15, GA16); layer 3+ sad paths, GF1-GF4 and GF6 among them, are named examples (Mandate 11). A READY after the bound has a reasoned no-obligation ruling (`distill/test-scenarios.md`). The family-private host half is crafter-owned unit coverage.

## Wave: DISTILL / [REF] WS strategy

The walking skeleton is the guest family's benchmark run: a production-`overdrive-init` guest per replicate on the exclusive Metal host, through `cargo xtask metal run --benchmark guest-flow-forwarding`. Its end-to-end run is evidence (E-G1 retained record and report, E-G2 the K3 drift refusal), as for the native skeleton: a benchmark is not a test. In nextest the skeleton is the runner (GR1-GR12) and the record, report and parser contracts (GV, GP, GA) driven through their real ports (the runner's `RecordSink`, the `bench-report` binary, `parse_program_return`) with the real renderer and validator. Port treatment (Architecture of Reference): driving adapters (`bench-report`, the attribute, the xtask form) are real; the one driven port with non-deterministic effects, the guest itself, is not faked in nextest: it is exercised only through the retained runs, because the design makes the family launch its own guests (ADR-0331) and keeps real-guest boots out of the test lanes. The project policy file `docs/architecture/atdd-infrastructure-policy.md` is not used in this repository (the tiers of `testing.md` are the policy). Slice 01 activates first.

## Wave: DISTILL / [REF] Adapter coverage

| Driven adapter / boundary | Real-I/O scenario |
|---|---|
| `RecordSink` (`FileSink`: append-only record, create-new artifacts) | GR11 (real directory, create-new, parents); GR8-GR10 against the in-memory sink; E-G1 |
| `HostProbe`, `StatsEnabler`, canary (unchanged) | the existing runner tests; G-1 on a guest benchmark: GR6 (refusal); E-G2 on the real host |
| Guest kit launch (`GuestKit::boot` through `CloudHypervisorVmm`) | E-G1, E-G3, E-G4, E-G5, E-G7 (retained runs; the design places real-guest effects in Tier-3 evidence); not a nextest scenario |
| Guest console (the serial file the VMM writes) | the parser's strict grammar against crafted consoles incl. unterminated fragments (GA1-GA16); E-G1 for the real console |
| `vm_fixture::workspace_root` (process environment) | GW1, GW2 (real environment, serialised) |
| Document store (`benchmarks/*.md`, committed records) | GD1-GD4, GP8, the existing `every_retained_record_still_validates` |

Costly-external adapters: none. The Metal guest boot is the only adapter the nextest lane does not touch, by design (U9).

## Wave: DISTILL / [REF] Scaffolds

Exact shapes from the approved Interface contracts only; each is marked `SCAFFOLD: true`.

- `crates/overdrive-bench/src/record.rs` (re-exported at the crate root): `HostValue` made `pub`; `GuestView`, `GuestArtifacts`, `GuestMemoryMib` (`new` and `Display` complete; `FromStr` refuses every text, `Serialize` writes `0`, `Deserialize` refuses every value), `GuestMemoryMibError` (unit struct; shape is crafter detail, U5).
- `crates/overdrive-bench/src/replicate.rs`: `Replicate::report_guest_view` and `Replicate::retain_console`, inert (`Ok(())`).
- `crates/overdrive-bench/src/runner.rs`: `RecordSink::write_artifact` and `FileSink::write_artifact` (inert).
- `crates/overdrive-testing/src/guest_app.rs`: `GuestReading`, `ProgramReturn`, `ProgramReturnError` (scaffold `Display`), `parse_program_return` (always `Err(BlockMissing)`), `print_program_return` (inert: prints nothing; the printer beside the parser, so one owner holds the grammar).
- `crates/overdrive-testing/src/guest_kit.rs`: `cid_bands::GUEST_BENCHMARK` (complete, not yet in `REGISTERED`), `GuestImageSpec::benchmark()` (inert: returns `workload()`).
- `crates/overdrive-testing/Cargo.toml`: `sha2` (pinned in Technology choices; the parser tests use it). `Cargo.lock` updated.
- `crates/overdrive-testing/src/guest_lane.rs` (F3): `LANE_GUEST_VCPUS` (2) and `LANE_GUEST_MEMORY_MIB` (512), `ProgramRun` (`exit: Result<i32, GuestKitError>`, `console: String`) and `run_in_guest_program` with the pinned signature (`kit, spec, program, cid: GuestCid, work, bound`); the body is inert (boots nothing, returns `Ok(0)` and an empty console). `run_in_guest_cell` is not touched (DELIVER makes it read the two constants).
- Not scaffolded: the family module, `Substrate::Guest`, the validator and report changes, `GuestClock::Monotonic`, the macro's guest clock entry. The compile-fail and document tests are RED against the missing behaviour itself.

## Wave: DISTILL / [REF] Test placement

Precedent: the parent feature's `overdrive-bench` layout. Unit tests stay in-crate beside the code they drive (the runner, record, registry and replicate unit tests already live in `src/`; the parser tests sit in `src/guest_app/program_return_tests.rs` so the crafter's edits of `guest_app.rs` do not collide). Integration tests extend the existing `tests/integration/*` files and `support.rs` builders (`guest_started`, `guest_observed`, `guest_record`). Placement: every test except GF1-GF6 is host-pure through the crate defaults (`overdrive_test_default!(host_pure)` in the `cfg(test)` of `overdrive-bench` and `overdrive-testing` and in the `overdrive-bench` integration binary; trybuild tests carry `#[overdrive_test(host_pure)]` and `#[serial(trybuild)]`); GW1 and GW2 mutate the process environment under `#[serial_test::serial(env)]` with an RAII guard. No test is `native_launcher`. GF1-GF6 (F3) are `#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]` in `crates/overdrive-testing/tests/integration/guest_program_failures.rs`, the precedent being `overdrive-host`'s `vsock_beacon_accept_path.rs` (a guest-kit launch from a `nested-launch` guest) and `overdrive-init`'s `in_guest_lane.rs`; the binary is compiled only with `integration-tests,kvm-tests` and is in the `host-kernel-shared` group with a 120 s x 6 slow-timeout. `cargo xtask placement-lint`: exit 0. The guest family's smoke test will be placed `guest = "cgroup-default", profile = "nested-launch"` by its declaration (DDD-10).

## Wave: DISTILL / [REF] Driving adapter coverage

| Driving port (DESIGN) | Scenario |
|---|---|
| 1 the attribute with `substrate = guest` | GT1-GT3, GS1-GS2 |
| 2 `cargo xtask metal run --benchmark <family> [--spike <name>]` | unchanged by this feature (`bootstrap.sh` untouched); the run is evidence E-G1 to E-G7; `--spike` with a guest family E-G6 |
| 3 `cargo xtask metal pull <result dir>` | unchanged existing contract; not re-tested |
| 4 `bench-report validate\|render` | GV1-GV7, GP1-GP8 |
| 5 `cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests` (smoke variant) | GR13-GR16 and the generic smoke tests; the guest family's own smoke test is generated at DELIVER (U8) |
| 6 `cargo xtask placement-lint` | run, exit 0 |
| setup `cargo xtask guest-kit-prepare` | unchanged; its absence is E-G3 |

Every command-line entry point that nextest can reach has a subprocess scenario (`bench-report`); none spawns the `overdrive` binary (the evidence run's command log is read for that, US-GBS-01 AC (h)).

## Wave: DISTILL / [REF] Pre-requisites

`OVERDRIVE_METAL_TARGET`, `OVERDRIVE_METAL_KERNEL`, `OVERDRIVE_METAL_ROOTFS` for every Metal run. The nextest tests above need nothing else (no BPF object, no guest kit, no kernel module) except GF1-GF6, which need `cargo xtask guest-kit-prepare` and run with `--features integration-tests,kvm-tests` in nested-launch guests. The evidence runs need `cargo xtask guest-kit-prepare` at the measured commit and `cargo xtask metal run --exclusive -- cargo xtask kmod-load`. DEVOPS artifacts absent: default environment matrix. DELIVER removes the `should_panic` of the three trybuild tests and checks the pinned `error:` lines are unchanged. GT2's activation is that its fixture builds whole with no `.stderr` file written or changed. The guest family's declaration and its macro-generated smoke test land in one DELIVER step, gated by `cargo xtask placement-lint` and a Metal nextest run of `overdrive-bench-families`. DELIVER also lands `.gitignore` `benchmarks/results/**/artifacts/` with the first retained console (DESIGN-approved).

## Wave: DISTILL / [REF] Completeness audit

15-item checklist. Passing: C1a (GA4, GA12 one and two values, GM4 empty and zero, GV5 zero vcpus), C1b (GA4 15/16/17, GA12 4,999/5,001, GM4 `u32::MAX` and one past, GV5 digest of 63 and 65), C2a (the guest record's line order documented in the validation module docstring), C2b (GV4: before the start canary, twice, after the first ok replicate, on a native record), C3 (view 0/1/many: GR3, GR1, GR2; series 0/1/many: GA7, GA4, GA15), C4a (view reported twice GR2, console retained twice GR9, artifact written twice GR11, rendered twice GP6), C4b (n/a: no inverse operation; counted as passing), C5a (substrate x terminal state x view present or absent: GV1, GP1-GP4, GR6, GR7), C5b (the guest block leaves a native record and report untouched: GP7, GP8, GR7), C6a (GV3, GV5, GM4, GM5, GA10, GA11), C6c (GA16 and the existing never-101 validator tests: no input yields a series other than the one the checksum covers, and no refusal is a panic), C7a (a failed console write GR10; failed boots GR5), C7b (GP4 incomplete record; E-G4 killed process), C7c (n/a: the form holds the host exclusively and replicates run one guest at a time; counted as passing). C6b (GA17: a reading line with no `=` is `Err`; the variant is the crafter's, U2). 15 of 15: COMPLETE. C7a/C7b now also cover GF1-GF6 (a guest that never READYs, stalls, dies, misses EXIT or outlives its EXIT is a failed run with its console and a reaped VMM). Gaps recorded in `distill/upstream-issues.md`: U11 (F3, resolved; one case, a late READY, carries a reasoned no-obligation ruling), U12, U13; U1 resolved by DISTILL (the printer is scaffolded beside the parser); U3 and U6 pinned by DESIGN and covered by GP2 and GR14-GR16; U2, U4 and U5 are crafter detail; U8 and U9 (deliberate evidence-lane assignments). Per-wave review not run by this agent; the consolidated final review is the orchestrator's.
