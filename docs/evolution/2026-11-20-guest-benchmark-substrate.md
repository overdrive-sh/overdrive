# Evolution — A guest substrate for the benchmark program

**Feature:** `guest-benchmark-substrate` ([GH #362](https://github.com/overdrive-sh/overdrive/issues/362)). **Finalized:** 2026-11-20. Child of [`native-benchmark-program`](2026-11-19-native-benchmark-program.md).
**Scope:** developer verification tooling. One new substrate, `guest`, and one family on it, `guest-flow-forwarding`. Twelve implementation steps in three phases.
**Waves run:** DISCUSS (2026-11-19), DESIGN (2026-11-19, user approval PD-1 to PD-9 the same day, one amendment 2026-11-20), DISTILL (2026-11-19), DELIVER (2026-11-19 to 2026-11-20). No DIVERGE.
The [feature contract and wave history](../feature/guest-benchmark-substrate/feature-delta.md), [roadmap](../feature/guest-benchmark-substrate/deliver/roadmap.json) and [execution log](../feature/guest-benchmark-substrate/deliver/execution-log.json) remain the delivery records. The retained-run receipts are in [`deliver/evidence/`](../feature/guest-benchmark-substrate/deliver/evidence/), the spike findings in [`spike/`](../feature/guest-benchmark-substrate/spike/).

## Summary

The benchmark program could only measure components that run on the Metal host. This feature adds the `guest` substrate: a family declared `guest` boots a fresh kit guest on the exclusive Metal host for every replicate, runs a prepared program inside it, and gets the samples back on the guest console as one framed, checksummed series block. The same `cargo xtask metal run --benchmark <family>` form produces the same append-only `record.jsonl` and derived `report.md`. The record gains an optional `guest` block on `run_started` (launch, boot granularity, vCPU scheduling, the three host settings a guest cannot see) and one `guest_observed` line holding the guest's view (vCPUs, memory, kernel, clocksource, SHA-256 of each staged artifact). `schema` and the methodology version stay 1.

The first guest family is `guest-flow-forwarding`. It drives the production guest flow owner through its vsock wire seam and measures one series, `forwarded_round_trip_ns`, in one arm. Its generated smoke test runs in `cgroup-default` with profile `nested-launch`. A `spike-scratch/` crate declares a guest benchmark through `--spike` and yields a schema-identical record marked spike evidence. Native families, their records and their reports are unchanged.

## Business context

#362 was opened by the parent feature (CA-10 there): components that live inside a workload VM, the guest flow owner first, cannot be measured by a native-only runner. In-guest kernel mTLS ([#303](https://github.com/overdrive-sh/overdrive/issues/303)) is the stated first consumer. It is not implemented, so the family measures the component that exists today and #303 compares later. The persona is the engineer, or the coding agent acting for them, who must choose between mechanisms or check what a guest-side primitive costs.

## Key decisions

All nine DESIGN decisions were put to the user and approved on 2026-11-19 (PD-1 to PD-9). The ADRs read Accepted. DDD-10 and DDD-11 are choices within existing scope and stand with the design.

| Decision | Outcome | ADR |
|---|---|---|
| PD-1 / DDD-1 | `guest` joins the closed substrate registry. It means a kit guest launched directly on the Metal host. A guest benchmark declares no `bpf_stats` (BPF statistics in a guest: [#371](https://github.com/overdrive-sh/overdrive/issues/371)) | [0325](../product/architecture/adr-0325-the-guest-substrate-joins-the-closed-benchmark-registry.md) |
| PD-2 / DDD-2 | Guest conditions go in `run_started.guest`. The guest's view, with the artifact digests, goes in one `guest_observed` line. Digests are recorded, not refused on | [0326](../product/architecture/adr-0326-a-guest-benchmark-record-states-the-guests-conditions-in-run-started-and-its-view-in-one-guest-observed-line.md) |
| PD-3 / DDD-3 | Adding a substrate does not bump the methodology version | [0327](../product/architecture/adr-0327-adding-a-substrate-does-not-bump-the-methodology-version.md) |
| PD-4 / DDD-4 | The host is refused in full before any guest boots. There is no probe guest and no guest pin. Each replicate reports the guest's view, and an unprepared kit is a failed replicate. User's reason: the guest kernel is never stale, because we control it | [0328](../product/architecture/adr-0328-a-guest-benchmark-refuses-the-host-before-it-boots-a-guest-and-takes-the-guests-view-from-its-replicates.md) |
| PD-5 / DDD-5 | A fresh guest for every replicate | [0329](../product/architecture/adr-0329-a-guest-benchmark-boots-a-fresh-guest-for-every-replicate.md) |
| PD-6 / DDD-6 | A guest returns its samples on its console as one framed series block | [0330](../product/architecture/adr-0330-a-guest-returns-its-samples-on-its-console-as-framed-series-lines.md) |
| PD-7 / DDD-7 | The family launches its own guests. The benchmark core has no guest port and no new dependency. Includes DDD-11, the exported source root also resolves in class `exclusive` | [0331](../product/architecture/adr-0331-a-guest-benchmark-family-launches-its-own-guests-and-the-benchmark-core-has-no-guest-port.md) |
| PD-8 / DDD-8 | The first family drives the production guest flow owner through its vsock wire seam and measures one series | [0332](../product/architecture/adr-0332-the-first-guest-family-drives-the-production-guest-flow-owner-through-its-vsock-wire-seam.md) |
| PD-9 / DDD-9 | A replicate retains its guest console beside the record, in every outcome, uncommitted. `benchmarks/results/**/artifacts/` goes into `.gitignore` | [0333](../product/architecture/adr-0333-a-replicate-retains-its-guest-console.md) |

**DDD-9 amendment, approved by the user 2026-11-20.** `retain_console` records the console, and the runner writes `artifacts/<profile>/<block>-<position>-<arm>/console.log` after the body returns. A write failure fails the replicate. Nothing is written under smoke. The reason: writing in place would make `Replicate` borrow the sink, so a body would become non-`Send`. A companion changed assumption, same date: `run_in_guest_cell` shares `run_in_guest_program`'s post-EXIT rule (a VMM still running after the post-EXIT wait is an error, and the wait result is never discarded), so DDD-5's "keeps its behaviour" is reworded.

Changed assumptions CA-1 to CA-4 (feature delta): a guest's death is a failed replicate, not an incomplete run; `run_started.guest` and `guest_observed` are a second optional place and a new line kind in a record contract that had none; workspace resolution by exported source root now also applies in class `exclusive`; an unprepared kit is a failed replicate and the run exits 0, not a refusal.

## Work completed per step

Source: `deliver/execution-log.json` and the `Step-Id` trailers. All twelve steps reached COMMIT with status PASS. Four steps log RED as skipped with a reason (below).

| Step | What landed | Commits |
|---|---|---|
| 01-01 | Registry accepts `guest` and the `guest_monotonic` clock source; trybuild fixtures activated | `5603a8412` |
| 01-02 | `GuestMemoryMib` validated positive newtype, text and JSON round trips | `8f2b5f31a` |
| 01-03 | A guest record validates and its report states the guest's conditions | `ae30c2d71` (first attempt blocked, see Issues) |
| 01-04 | Runner records the guest's view once and retains every replicate's console | `2b6465d91`, `bd0a8ca74`, `13a6bdaf0`; later fixes `f0a321ea6` (a failed body's reason stays first when its console write also fails; lane stalls no longer labelled image staging) and `2fbef0740` (the runner writes the retained console after the body returns, the DDD-9 amendment) |
| 01-05 | Benchmark guest CID band registered; the kit finds the checkout under the exclusive class | `0f98129b9` |
| 01-06 | Guest program return printed and parsed under one strict console grammar | `44665962b` |
| 01-07 | A guest program run reports its exit and console in every outcome and leaves no VMM | `82485313e`, preceded by the regression test `e9a335da6` |
| 01-08 | The `guest-flow-forwarding` family, fresh guest per replicate, with its smoke test | `a3c11c575`, `0b1ac99aa` |
| 01-09 | Retained run E-G1, drift refusal E-G2, cost figures V-D | `810cad27f` (roadmap), `76a5d5853`, `e20a45741` |
| 02-01 | Retained failure runs E-G3, E-G4, E-G7 | `dc63a95f2`, `721431e37` |
| 03-01 | A spike crate declares the guest substrate and runs through `--spike`, retained run E-G6 | `f15c1e661` |
| 03-02 | Methodology and catalogue state the guest substrate, the family row and the nested-virtualization rule; native families unchanged | `d4be59960` |

RED skipped as not applicable: 01-07 (the semantic RED for GF1-GF6 and GK3 against the inert scaffold is in `distill/red-classification.md`; markers removed, bodies untouched), 01-08 (implementation survived an interrupted dispatch; all 32 family tests passed on the first Metal run), and 03-01, 02-01 and 01-09 (evidence steps with no acceptance test). DISTILL proved 66 of 73 tests RED and 7 GREEN guards (GR13, GM1, GV7, GP7, GP8, GK2, GW2).

## Evidence runs

All runs are on the native Metal host (AMD EPYC 8024P, 16 CPUs, kernel 7.0.0-38-generic), commit `36618cba7`, tree dirty. Result directories are under `benchmarks/results/guest-flow-forwarding/<run-id>/`. Each receipt in `deliver/evidence/` was checked against the committed record.

| Run | Run id | What it shows |
|---|---|---|
| E-G1 | [`605495babcbab0f445b702c3`](../../benchmarks/results/guest-flow-forwarding/605495babcbab0f445b702c3/report.md) | Walking skeleton: 30 replicates, 30 ok, 0 failed, 150,000 values (5,000 each), `status: finished`, one `guest_observed` line (2 vCPUs, 512 MiB, kernel 7.0.0-38-generic, clocksource `tsc`). Every replicate's `commands` is `[]`, so no `overdrive` binary was launched. `forwarded_round_trip_ns`: p50 50,548 ns, p90 57,240, p95 62,338, p99 74,187 (histogram-derived, relative error at most 3.125%) |
| E-G2 | [`ebc396d84cdf0fed3a1f3dd4`](../../benchmarks/results/guest-flow-forwarding/ebc396d84cdf0fed3a1f3dd4/report.md) | Host drift refused before any guest boots: `bpf_stats_enabled` found 1, pinned 0. Record is `run_started` and `run_refused` only, no `guest_observed`, no `artifacts/`. Report `status: refused`, every guest-view line `absent: no replicate returned a guest view`. Exit 78 as reported in the evidence agent's account |
| E-G3 | [`507cd3e5a68532d2bedd7e86`](../../benchmarks/results/guest-flow-forwarding/507cd3e5a68532d2bedd7e86/report.md) | Unprepared guest kit (CA-4): 30 failed replicates, each `guest boot: the guest-kit lane is not prepared: ... run cargo xtask guest-kit-prepare ...`. `run_finished` `replicates_ok 0, replicates_failed 30`, no `guest_observed`, no `artifacts/`, run exit 0 |
| E-G4 / V-C | [`41a4823eb9e8f15dcedb3622`](../../benchmarks/results/guest-flow-forwarding/41a4823eb9e8f15dcedb3622/record.jsonl) | Benchmark process SIGKILLed after the CID wrap: 24 lines, replicate blocks 0 to 20 all ok, no terminal line, no `report.md`. A box-side probe about 35 s later found no `cloud-hypervisor` process, no stopped process and 0 holders of `/dev/vhost-vsock`. That probe is reported, not committed. A clean run followed ([`62cf21ac0f4b8793e31ec89d`](../../benchmarks/results/guest-flow-forwarding/62cf21ac0f4b8793e31ec89d/report.md)): 30 ok, 0 failed, `status: finished`, over 16 CIDs |
| E-G5 | same record as E-G1 | CID band wrap: the band is 16 wide, the family's replicate counter modulo the width picks the slot, and 30 replicates ran with 0 failures, so replicates 16 to 29 reused slots. The record stores no per-replicate CID, so this follows from 30 > 16 with 0 failures and the counter rule in `crates/overdrive-testing/src/guest_kit.rs`, not from a recorded CID |
| E-G6 | [`spike-scratch/guest-flow-spike/results/…/3855f6aa963e74504de7ac65`](../../spike-scratch/guest-flow-spike/results/guest-flow-forwarding/3855f6aa963e74504de7ac65/report.md) | A spike crate declares the guest substrate and runs through `--spike`: `evidence: spike evidence`, 9 of 9 replicates ok, 45,000 values, p50 40,453 ns, p90 56,992, p95 61,367, p99 73,894 |
| E-G7 | [`58e217e3ee6e494b10b120eb`](../../benchmarks/results/guest-flow-forwarding/58e217e3ee6e494b10b120eb/report.md) | A guest stalled by SIGSTOP of its VMM: block 10 failed (`the guest sent no beacon within 60s`), 29 ok and 1 failed, `status: finished`, the console retained. The post-run probe (reported) found no VMM and 0 vhost-vsock holders |

**V-D, cost and spread (E-G1, computed from the record).** The 30 per-replicate medians run from 39,540 ns to 52,215 ns (max/min 1.32) in two clusters of 15: about 39.5 µs (39,540 to 39,709 ns) and about 51 µs (50,884 to 52,215 ns). The median of medians, 45,296 ns, falls between the clusters and describes neither. The pooled p50 (50,548 ns) sits at the lower edge of the high cluster (lowest median 50,884 ns), within the 3.125% histogram error of it. The record gives no cause for the split. The run took 69 s for 30 replicates, about 2.3 s per replicate including two canaries and the guest boot. The earlier production-init spike measured about 2.75 s per replicate, an estimate of about 83 s for 30 replicates before this run. Merging the 30 histograms reproduces the report's percentiles to the unit. Per-replicate console-return time is not derivable from the record. Per-replicate medians are interpolated from histogram buckets.

Earlier spikes behind the design: [`spike/findings-guest-boot-cost.md`](../feature/guest-benchmark-substrate/spike/findings-guest-boot-cost.md), [`spike/findings-guest-return-path.md`](../feature/guest-benchmark-substrate/spike/findings-guest-return-path.md), [`spike/findings-production-init-return.md`](../feature/guest-benchmark-substrate/spike/findings-production-init-return.md). They decided V-A (fresh guest per replicate) and V-B, V-B2 (console return) before the user approved the design.

## Issues encountered and lessons

- **Two DISTILL body defects were routed back to the acceptance designer, not fixed by the crafter.** GP3 (`a_guest_run_refused_on_a_drifted_host_reports_the_refusal_and_the_static_conditions`) asserted that the status line carries the refusal detail, which contradicts the native test and DDD-2. Step 01-03 stopped as BLOCKED_BY_DEPENDENCY on all three phases, and the work sat uncommitted until the body was corrected. The second, GP8 (`every_committed_native_report_re_renders_byte_identical`), scanned records as if all were native. The guard was adjusted to admit guest records and keep asserting native ones in `dc63a95f2`.
- **The DDD-9 deferred-write amendment.** The first design had `retain_console` write the file in place. Writing in place makes `Replicate` borrow the sink, which makes a body non-`Send`. The user approved recording the console and writing it after the body returns. A write failure fails the replicate, and `f0a321ea6` keeps the body's own failure reason first when both fail.
- **`run_in_guest_cell` now shares the post-EXIT rule.** A VMM that outlives its guest's EXIT is a failed run in both entry points, and the wait result is never discarded (`e9a335da6`, `82485313e`). The prior text said the cell path kept its behaviour.
- **An untracked DES progress file made every evidence record dirty.** `docs/feature/guest-benchmark-substrate/deliver/.develop-progress.json` was the only untracked path. That it caused `dirty: true` at run time is the evidence agent's account. The receipt cannot re-observe the run-time tree. Every retained guest report carries `dirty: true`, so the commit field is a base.
- **`metal run` re-sync deletes un-pulled remote result directories.** Pull each result immediately, before the next Metal command. E-G4 was pulled right after its run for this reason.
- **A failure label misled.** E-G7's reason prefix `guest run: staging the guest image: guest program:` described a guest that had booted and stalled. `f0a321ea6` stopped labelling lane stalls as image staging. The retained E-G7 record predates that fix and still carries the old prefix.
- **A bimodal series.** E-G1 splits into two clusters of 15 replicates with no recorded cause. A pooled percentile hides this, and only the per-replicate medians show it. A second clean run on the same machine and day, `62cf21ac0f4b8793e31ec89d` (started 01:15Z, E-G1 at 00:39Z, 30 of 30 ok), pooled to p50 40,046 ns, p90 53,998, p95 59,976, p99 71,745. Binning its replicate histograms puts about 20 medians near the low level and about 10 near the high one, so the roughly 10 µs p50 difference between the runs is the same split with a different mix. The cause is unexplained. The hypothesis is unpinned vCPUs ([#368](https://github.com/overdrive-sh/overdrive/issues/368)), untested. Two runs on one day give no measure of stability across days. The family reports the pooled distribution as the design says, and the split is an open observation about the measured system.
- **Open observation, not investigated.** The `overdrive-init` lane test `a_burst_of_host_aborts_resets_every_connection_in_a_real_guest` failed on Metal in a code path this feature did not touch (`run_in_guest_owner_cell`). It was not investigated and no cause is claimed.

## What was not done

- **Mutation testing was skipped by user direction, 2026-11-20.** The roadmap's per-step mutation gates were not run, so no kill rate is claimed.
- The retained consoles under `artifacts/` are gitignored. Only the pulled local directories hold them.
- The post-kill and post-stall VMM probes (E-G4, E-G7), the E-G2 exit status and the E-G2 host restore are box-side reports, not committed records.
- No full-suite nextest comparison was made.

## Scope limits and deferrals

Each issue was read with `gh issue view <N> --comments` on 2026-11-20 and is open except #362, which this feature closes. The feature delta records each limitation beside its issue.

- [#368](https://github.com/overdrive-sh/overdrive/issues/368): pinning the benchmark guest's vCPUs to dedicated host CPUs. Today `vcpu_scheduling` is `unpinned`.
- [#369](https://github.com/overdrive-sh/overdrive/issues/369): a family declaring the guest's vCPU count and memory size. Every guest benchmark runs in the kit's default shape.
- [#370](https://github.com/overdrive-sh/overdrive/issues/370): the family's unmeasured paths (the host-driven vsock leg, guest-originated TCP and datagram flows, the owner's accept-and-pair work).
- [#371](https://github.com/overdrive-sh/overdrive/issues/371): reading BPF program run statistics inside a guest. A guest benchmark declares no `bpf_stats`.
- [#347](https://github.com/overdrive-sh/overdrive/issues/347): nested virtualization for VM workloads. Until it lands, VM-lifecycle families stay native (D9, enforced by the catalogue, the methodology and review, with no mechanical check).
- [#361](https://github.com/overdrive-sh/overdrive/issues/361): baseline promotion and regression gating. This feature compares nothing against an earlier run.
- [#303](https://github.com/overdrive-sh/overdrive/issues/303): in-guest kernel mTLS and its comparison with the host proxy. The mTLS family measures the host hop; this feature adds the substrate #303 will need.
- [#362](https://github.com/overdrive-sh/overdrive/issues/362): the anchor issue, closed by this feature. Every item under its "What lifting it takes" is delivered: the `guest` registry entry run through the guest kit on Metal, the in-guest program returning samples in the same record schema, the guest view and `unpinned` vCPU scheduling in the record, substrates never compared, and VM-lifecycle families kept native.

## Verification expectations (EDD)

None authored. The retained runs answer how fast and how variable, which `.claude/rules/verification.md` § "Boundary" keeps apart from point-in-time feature receipts.

## Permanent artifacts

- ADRs 0325 to 0333 in `docs/product/architecture/`, linked above.
- [`benchmarks/catalogue.md`](../../benchmarks/catalogue.md) (the `guest-flow-forwarding` row and the nested-virtualization rule) and [`benchmarks/methodology.md`](../../benchmarks/methodology.md) (the "Guest substrate" section; the version stays 1).
- Retained results under `benchmarks/results/guest-flow-forwarding/` and `spike-scratch/guest-flow-spike/results/`.
- Code: `crates/overdrive-bench/`, `crates/overdrive-bench-families/`, the guest-program, guest-kit and CID-band surface in `crates/overdrive-testing/`, the `overdrive_bench` attribute in `crates/overdrive-testing-macros/`.

## Migration of lasting artifacts

Phase B scan of `docs/feature/guest-benchmark-substrate/` against the destination map: it holds `intake.md`, `feature-delta.md`, `slices/`, `distill/`, `deliver/` and `spike/`. There is no `design/` directory, so no `architecture-design.md`, `component-boundaries.md`, `technology-stack.md`, `data-models.md` or `design/adrs/`. There is no `distill/walking-skeleton.md` and no `discuss/journey-*`. The ADRs (0325 to 0333) already sit at their permanent home in `docs/product/architecture/`, and the catalogue, methodology and results are already permanent. Nothing was copied. Retained only as delivery history: the feature delta, intake, slices, DISTILL scenarios and RED classification, DISTILL upstream issues, the three spike findings, the roadmap, the execution log and the five evidence receipts (including `validation.md`, which records `every_retained_record_still_validates` passing on Metal in run `227585af7539ff8f9519e9c8`). The DES progress file `deliver/.develop-progress.json` is session state for Phase C and was not touched.

## Finalization record

- **Mutation testing:** not run, by user direction on 2026-11-20.
- **DES integrity:** `des-verify-integrity docs/feature/guest-benchmark-substrate/deliver/` reports all 12 steps with complete traces.
- **Website documentation:** `website/content/docs/how-to/write-and-run-a-benchmark.mdx` (declaring and running a guest benchmark, the kit, the report's guest lines, retained consoles, `--spike`) and `website/content/docs/concepts/benchmark-methodology.mdx` (the guest substrate).
- **Blog post:** `website/content/blog/measuring-a-component-inside-a-workload-vm.mdx`; `website/content/blog/measuring-hooks-vms-dns-and-mtls-on-one-runner.mdx` had one sentence corrected (it said a component inside a VM could not be benchmarked).
- **Validation:** `bun run typecheck`, `bun run lint`, `bun run build` and `bun run assert:one-index` pass. `bun run test:blog`, `test:mcp` and `test:mcp:analytics` fail on one assertion: the top docs hit for "deterministic simulation" is `/docs/concepts/workload-identity-lifecycle`, not `/docs/concepts/deterministic-simulation-testing`. This failure is pre-existing: it reproduces in a clean worktree at the pre-content HEAD, and the four content changes never change the order (a short link-text list item in the workload-identity page outscores the DST page's title chunk under BM25). Not fixed here.
- **Review:** independent documentation review, two rounds: CHANGES_REQUESTED (four major findings: the second clean run unreported, cost figures without provenance, undefined terms, an incomplete finalization record), then APPROVED with four low nits, two applied. Artifact: `docs/feature/guest-benchmark-substrate/deliver/review-finalize-content.md`.
- **Issues closed:** #362.
- **Open observation:** the `overdrive-init` lane test `a_burst_of_host_aborts_resets_every_connection_in_a_real_guest` failed on Metal ("every burst flow ended not reached within 1s") in `run_in_guest_owner_cell`, which this feature does not change. It was not investigated and has no issue.
- **Metal box cleanup:** this checkout's remote directory on the Metal box removed after every result was pulled (see the commit that records this section).
