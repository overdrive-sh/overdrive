# E-G1, E-G2, E-G5 and V-D receipt: walking-skeleton guest run, drift refusal, CID wrap, cost

Commit 36618cba7 (dirty tree) for both runs. Box kernel 7.0.0-38-generic, AMD EPYC 8024P, 16 CPUs.
Every figure below is read from the committed files under `benchmarks/results/guest-flow-forwarding/`.

## E-G1: guest walking-skeleton run (run 605495babcbab0f445b702c3)

Record `605495babcbab0f445b702c3/record.jsonl` (20357 bytes): 1 `run_started`, 2 `canary`, 1 `guest_observed`, 30 `replicate`, 1 `run_finished`.

- `run_started` (started 2026-11-20T00:39:15Z): `"substrate":"guest"`, `"evidence":"maintained"`, host `bpf_stats_enabled:"0"`, and the guest block
  `"guest":{"boot":"per_replicate","launch":"metal_host","settings":{"boost":{"absent":"not visible from a guest"},"governor":{"absent":"not visible from a guest"},"smt":{"absent":"not visible from a guest"}},"vcpu_scheduling":"unpinned"}`.
- One `guest_observed` line: `{"t":"guest_observed","vcpus":2,"memory_mib":512,"kernel":"7.0.0-38-generic","clocksource":"tsc","artifacts":{"overdrive_init":"010f5819...b63d0","overdrive_guest_peer":"537b1597...fcdfaa","ovd_fwd":"0e64db08...a615fca1"}}` (full digests in the report, `guest artifact` lines).
- `run_finished`: `{"finished":"2026-11-20T00:40:24Z","replicates_ok":30,"replicates_failed":0}`.
- All 30 replicates: `role arm`, `outcome ok`, 5000 values each (150000 in total), and `commands` is `[]` in every one (0 commands in total). No `overdrive` binary was launched by any replicate.
- `report.md` header: `substrate: guest`, `guest launch: metal host (not nested)`, `guest boot: fresh guest per replicate`, `guest vcpu scheduling: unpinned`, the three `guest governor/smt/boost: absent: not visible from a guest` lines, `guest vcpus: 2`, `guest memory mib: 512`, `guest kernel: 7.0.0-38-generic`, `guest clocksource: tsc`, `bpf_stats_enabled: 0`, `canary: steady`, `replicates ok: 30`, `replicates failed: 0`, `status: finished`.
- `report.md` table forwarded-path:
  `| forwarded_round_trip_ns | forwarding | 50548.19092595716 | 57239.5006833713 | 62337.90020618555 | 74186.20061538463 | 150000 | 30 | 0 |`
  (columns p50, p90, p95, p99, values, replicates, failures), followed by the note that percentiles are histogram-derived (relative error at most 3.125%).
- Dirty flag: `run_started` and the report carry `"dirty":true`. The tree has one untracked path now, `docs/feature/guest-benchmark-substrate/deliver/.develop-progress.json` (`git status --short`), the DES progress file. That it was the cause at run time is the evidence agent's account; this receipt cannot re-observe the run-time tree. The same flag appears on the E-G2 record.

## E-G5: CID band wrap

The family's own replicate counter modulo the band width picks the slot (`crates/overdrive-testing/src/guest_kit.rs`, `cid_bands::GUEST_BENCHMARK`: `first_offset 0x2050, width 16`). The record has 30 replicates (blocks 0 to 29) all `outcome ok`, 0 failed, so replicate 17 (counter 17, slot 1) ran on the wrapped CID of the benchmark band after slots 0 to 15 were used, and replicates 16 to 29 reused slots 0 to 13. The record does not store the CID per replicate; the claim follows from 30 > 16 replicates with 0 failures and the counter-modulo rule in the source, not from a recorded CID.

## V-D: cost and spread (computed from the record)

Script: `.context/vd_medians.py` (throwaway, decodes the sparse `[bucket, count]` histograms with the bucket layout of `crates/overdrive-bench/src/histogram.rs`, interpolating linearly inside a bucket, clamped to min/max). Check: merging all 30 histograms reproduces the report's percentiles, p50 50548, p90 57240, p95 62338, p99 74187 (rounded), matching the report to the unit.

- Pooled p50: 50548 ns (report).
- Per-replicate medians (30): min 39540 ns, median 45296 ns, max 52215 ns; max/min 1.32.
- Two clusters, 15 replicates each: a low cluster with medians 39540 to 39709 ns (replicates 4, 5, 7, 10, 11, 14, 15, 16, 17, 19, 21, 23, 25, 26, 27) and a high cluster with medians 50884 to 52215 ns (the other 15). The median of the per-replicate medians (45296) falls between the clusters and describes neither; the pooled p50 sits inside the high cluster. The record gives no cause for the split.
- Cycle time: `run_started.started` 00:39:15Z to `run_finished.finished` 00:40:24Z = 69 s; 69 / 30 = 2.3 s per replicate. This includes the two canaries and the guest boot per replicate. Prior measurement (about 2.75 s per replicate, about 83 s for 30): this run is faster by about 0.45 s per replicate (about 14 s in total).
- Failed-replicate rate: 0 / 30 = 0%.
- Console-return time per replicate is not derivable from the record: replicate lines carry no timestamps, and the `artifacts/default/<n>-0-forwarding/console.log` files are guest serial output only. Only the whole-run wall time above is available.

## E-G2: host drift refusal (run ebc396d84cdf0fed3a1f3dd4)

Record `ebc396d84cdf0fed3a1f3dd4/record.jsonl` has exactly two lines:

- `run_started` (started 2026-11-20T00:45:49Z, `substrate:"guest"`, same guest block as E-G1, host `"bpf_stats_enabled":"1"`).
- `{"t":"run_refused","setting":"bpf_stats_enabled","found":"1","pinned":"0"}`.

There is no `guest_observed` line and no `artifacts/` directory (directory contains only `record.jsonl` and `report.md`). Report: `bpf_stats_enabled: 1`, every guest-view line `absent: no replicate returned a guest view`, `canary: not run`, `replicates ok: 0`, `replicates failed: 0`, `status: refused`, `refused: bpf_stats_enabled found 1 pinned 0`.
Host restore (as reported by the evidence agent, not verifiable from the committed files): run a771875d700b0f89c2ca59f7 set `bpf_stats_enabled` back to 0. The later E-G1 run recorded `bpf_stats_enabled:"0"`, but E-G1 ran before E-G2 (00:39 against 00:45), so it does not confirm the restore.

## Not holding / limits

- The per-replicate CID is not in the record (E-G5 rests on the counter rule).
- The dirty-flag cause and the host restore are the evidence agent's reports.
- Per-replicate medians are interpolated from histogram buckets (relative error at most 3.125%).
