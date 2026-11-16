# Findings: overhead of `kernel.bpf_stats_enabled=1` (native Metal host, Linux 7.0)

Probe: `spike-scratch/bpf-stats-overhead/` (copy of `spike-scratch/pig-connect-cost/`, original untouched).
Raw rows: `spike-scratch/bpf-stats-overhead/results/runs.csv`; full stdout: `results/run.log`.

## Verdict: NOT-NEGLIGIBLE under the strict A/A criterion, but small (about 0.5% of a connect); about 67 ns per program run

The strict test was "ON-OFF within the A/A noise floor". It is only borderline met in one direction and failed in the other:

- ON-OFF wall delta CI95 `[+10, +216] ns` **excludes zero**; the A/A CI `[-20, +125] ns` includes zero, and the ON-OFF point estimate (+145 ns) is above the A/A upper bound (+125 ns). So the effect is detectable, not noise.
- The detected effect is 0.48% of a loopback connect-and-close (30.4 us) and is fully explained by the direct per-run cost: 2 production connect4 programs x ~67 ns = ~134 ns.
- It is directly measurable and large *relative to a tiny program*: the production XDP lookup (miss path) goes 71 ns to 138 ns per run (+94%).

## Predicted vs actual

| | Predicted | Actual |
|---|---|---|
| per-run cost | ~20-60 ns; falsified above ~200 ns | **+67 ns** (CI [67,68]); slightly above the predicted range, far below the 200 ns falsifier |
| connect wall delta | under 2%, inside A/A spread | **+0.48%** [0.03%, 0.71%]: under 2% (held), but just outside the A/A spread (not held) |

The hypothesis "small constant, tens of ns" held in order of magnitude (67 ns); "within noise on connect" did not hold strictly.

## Environment

- `uname -r`: 7.0.0-38-generic; CPU AMD EPYC 8024P 8-Core (16 CPUs, SMT on); governor `schedutil`, cpufreq boost=1 (no intel_pstate), cpu0 ~2.997 GHz at start.
- Commit d1953f3f7 (dirty tree: untracked docs + this probe). Run id `58f7cb10e8e9aca2b576eb6d`, `metal run --exclusive`.
- `bpf_stats_enabled` prior value 0; restored to 0 by the RAII guard (verified afterwards: `sysctl=0`).
- Seed 20261116; 30 blocks; each block runs OFF1, OFF2, ON once in a seeded-shuffled order (90 runs); fresh cgroup-attached `EbpfDataplane` per run, both production connect4 programs (service + gateway) attached in every arm; sysctl set before each run. Per run: 500 warm-up connects, 5000 timed loopback connect-and-close (SO_LINGER 0), then `BPF_PROG_TEST_RUN` of `xdp_service_map_lookup` (64-byte TCP SYN to a VIP not in SERVICE_MAP: miss, XDP_PASS, retval 2) with repeat 1,000,000 (after a 100,000 warm-up); `duration` is kernel-reported ns/run. The sample is the run (median over its 5000 connects); the statistic is the median of the 30 run medians; CIs are percentile bootstrap (10,000 resamples of runs) of the difference of medians.
- fd-scoped `BPF_ENABLE_STATS` lie check: in all 60 OFF-arm runs `ProgramInfo::run_count/run_time` of the three programs grew by 0 (total run_time_ns growth 0) and the sysctl read back as requested in all 90 runs. Nothing held an fd-scoped enable. (Checked through aya `ProgramInfo` rather than `bpftool prog show`; same kernel fields.)

## Per-arm table (probe output)

```
########## [bootstrap] preflight: ssh to ubuntu@151.115.99.251
########## [bootstrap] remote user=ubuntu home=/home/ubuntu sudo='sudo -n'
########## [bootstrap] acquire exclusive physical-maintenance barrier (/run/lock/overdrive-metal-shared.lock)
########## [bootstrap] sync /Users/marcus/conductor/workspaces/helios/islamabad-v1 -> ubuntu@151.115.99.251:/home/ubuntu/overdrive-worktrees/d6e116fa0e342e06ad0b58474ab66e6f68a1cc611c87fc78ba4451a071a928c7
########## [bootstrap] materialising standalone .git (common dir 2.3G)
## marcus-sa/native-benchmark-program
########## [bootstrap] fail-closed native x86_64/KVM preflight
# seed=20261116 blocks=30 connects/run=5000 warmup=500 xdp_repeat=1000000
# uname -r: 7.0.0-38-generic
# sysctl bpf_stats_enabled prior value: 0
# cpu governor cpu0: schedutil
# intel_pstate no_turbo: <No such file or directory (os error 2)>
# cpufreq boost: 1
# smt active: 1
# cpu0 cur_freq kHz: 2997563
# CPU(s):                                  16
# Model name:                              AMD EPYC 8024P 8-Core Processor
# Thread(s) per core:                      2
# --- wall_median_ns/connect ---
# OFF1  n=30 median-of-runs=30366.00 IQR=[30207.50,30451.00] (243.50)
# OFF2  n=30 median-of-runs=30416.00 IQR=[30310.00,30473.50] (163.50)
# ON    n=30 median-of-runs=30511.00 IQR=[30313.50,30563.50] (250.00)
# ON-OFF1  diff=145.00 CI95=[10.00,216.00]  (0.48% [0.03%,0.71%])
# OFF2-OFF1 (A/A) diff=50.00 CI95=[-20.00,125.50]  (0.16% [-0.07%,0.41%])
# --- wall_mean_ns/connect ---
# OFF1  n=30 median-of-runs=30993.21 IQR=[30874.67,31093.26] (218.59)
# OFF2  n=30 median-of-runs=31021.41 IQR=[30907.20,31111.62] (204.42)
# ON    n=30 median-of-runs=31169.62 IQR=[30992.68,31252.31] (259.63)
# ON-OFF1  diff=176.41 CI95=[20.14,290.41]  (0.57% [0.06%,0.94%])
# OFF2-OFF1 (A/A) diff=28.21 CI95=[-56.16,130.16]  (0.09% [-0.18%,0.42%])
# --- xdp_test_run_ns/run ---
# OFF1  n=30 median-of-runs=71.00 IQR=[71.00,71.00] (0.00)
# OFF2  n=30 median-of-runs=71.00 IQR=[71.00,71.00] (0.00)
# ON    n=30 median-of-runs=138.00 IQR=[138.00,139.00] (1.00)
# ON-OFF1  diff=67.00 CI95=[67.00,68.00]  (94.37% [94.37%,95.77%])
# OFF2-OFF1 (A/A) diff=0.00 CI95=[0.00,0.00]  (0.00% [0.00%,0.00%])
# OFF-arm run_time_ns growth total (must be 0 if stats truly off): 0
# runs where sysctl readback != requested: 0
```

| Metric | OFF1 median [IQR] | OFF2 median [IQR] | ON median [IQR] | ON-OFF1 (CI95) | A/A OFF2-OFF1 (CI95) |
|---|---|---|---|---|---|
| connect wall median ns | 30366 [30208, 30451] | 30416 [30310, 30474] | 30511 [30314, 30564] | +145 [+10, +216] (+0.48%) | +50 [-20, +126] (+0.16%) |
| connect wall mean ns | 30993 | 31021 | 31170 | +176 [+20, +290] (+0.57%) | +28 [-56, +130] |
| XDP test-run ns/run | 71 [71, 71] | 71 [71, 71] | 138 [138, 139] | **+67 [+67, +68]** | 0 [0, 0] |

Cross-check from the kernel's own stats in ON runs (aggregated from the CSV): connect4 reports 111.3 ns per run and XDP lookup 85.7 ns per run (`run_time/run_count`); these are the stats' own measurement and include part of the added cost, so they are larger than the OFF-arm true cost and are not the overhead figure.

## Per-program `BPF_PROG_TEST_RUN` delta

Feasible and used: stats off 71 ns/run, stats on 138 ns/run, so **+67 ns per program run** (A/A exactly 0 ns across 30 runs: the control is quiet). This is consistent with the source: two `sched_clock()` reads plus per-CPU stat updates per run (`include/linux/filter.h:700-726 @ v7.0`). The 2 connect4 programs at +67 ns each predict +134 ns per connect, matching the measured +145 ns.

## Raw rows (`results/runs.csv`)

```
block,pos,arm,sysctl_readback,wall_median_ns,wall_mean_ns,xdp_ns_per_run,xdp_retval,connect4_run_count_delta,connect4_run_time_ns_delta,xdp_run_count_delta,xdp_run_time_ns_delta
0,0,ON,1,29946.0,30693.6,137,2,10000,1104999,1100000,94015280
0,1,OFF2,0,30496.0,31114.3,71,2,0,0,0,0
0,2,OFF1,0,30135.0,31146.0,71,2,0,0,0,0
1,0,OFF1,0,30406.0,31181.7,71,2,0,0,0,0
1,1,ON,1,30526.0,31225.4,139,2,10000,1095727,1100000,94154655
1,2,OFF2,0,30416.0,31040.1,71,2,0,0,0,0
2,0,OFF1,0,33110.0,33857.4,71,2,0,0,0,0
2,1,ON,1,32899.0,33700.2,138,2,10000,1111349,1100000,94025512
2,2,OFF2,0,32789.0,33489.2,71,2,0,0,0,0
3,0,OFF2,0,32870.0,33800.2,71,2,0,0,0,0
3,1,ON,1,30126.0,30983.8,139,2,10000,1134752,1100000,94678945
3,2,OFF1,0,29805.0,30499.8,71,2,0,0,0,0
4,0,OFF1,0,29915.0,30793.0,69,2,0,0,0,0
4,1,ON,1,29945.0,30642.6,139,2,10000,1128641,1100001,94216683
4,2,OFF2,0,29694.0,30365.6,69,2,0,0,0,0
5,0,OFF2,0,29824.0,30680.5,69,2,0,0,0,0
5,1,OFF1,0,29695.0,30507.6,71,2,0,0,0,0
5,2,ON,1,29805.0,30471.8,138,2,10000,1101830,1100000,94076115
6,0,OFF2,0,29965.0,30703.6,71,2,0,0,0,0
6,1,ON,1,30556.0,31343.2,139,2,10000,1108460,1100000,94471428
6,2,OFF1,0,30456.0,31129.7,71,2,0,0,0,0
7,0,ON,1,30546.0,31543.5,138,2,10000,1109174,1100000,94119996
7,1,OFF2,0,30526.0,31109.5,71,2,0,0,0,0
7,2,OFF1,0,30496.0,31075.3,71,2,0,0,0,0
8,0,OFF1,0,30325.0,30898.9,71,2,0,0,0,0
8,1,ON,1,30106.0,30848.8,137,2,10000,1132627,1100000,94050597
8,2,OFF2,0,30476.0,31130.0,70,2,0,0,0,0
9,0,OFF1,0,30325.0,30915.6,71,2,0,0,0,0
9,1,OFF2,0,30456.0,31010.9,71,2,0,0,0,0
9,2,ON,1,30216.0,31527.5,137,2,10000,1137872,1100000,94105653
10,0,ON,1,30296.0,31128.0,139,2,10000,1104239,1100000,94444787
10,1,OFF2,0,30456.0,31031.2,71,2,0,0,0,0
10,2,OFF1,0,30386.0,30979.4,71,2,0,0,0,0
11,0,ON,1,30366.0,31029.3,138,2,10000,1100168,1100000,94062260
11,1,OFF2,0,30366.0,30961.6,71,2,0,0,0,0
11,2,OFF1,0,30496.0,31099.2,71,2,0,0,0,0
12,0,OFF2,0,30436.0,31014.8,71,2,0,0,0,0
12,1,OFF1,0,30396.0,31031.1,71,2,0,0,0,0
12,2,ON,1,30566.0,31166.6,137,2,10000,1089853,1100000,94047079
13,0,OFF1,0,30145.0,30794.1,71,2,0,0,0,0
13,1,ON,1,30556.0,31640.7,139,2,10000,1120571,1100000,94324311
13,2,OFF2,0,29925.0,30767.8,69,2,0,0,0,0
14,0,ON,1,30606.0,31207.5,139,2,10000,1095252,1100000,94441428
14,1,OFF2,0,30476.0,31377.3,71,2,0,0,0,0
14,2,OFF1,0,30426.0,31066.7,70,2,0,0,0,0
15,0,OFF2,0,30386.0,30980.8,71,2,0,0,0,0
15,1,ON,1,30596.0,31434.7,138,2,10000,1119501,1100000,94056440
15,2,OFF1,0,29835.0,30572.0,71,2,0,0,0,0
16,0,OFF1,0,30296.0,30874.1,71,2,0,0,0,0
16,1,OFF2,0,30436.0,31317.9,71,2,0,0,0,0
16,2,ON,1,30396.0,30892.6,139,2,10000,1116613,1100000,94415343
17,0,ON,1,30446.0,31025.0,137,2,10000,1095474,1100000,94440093
17,1,OFF1,0,30185.0,30811.3,71,2,0,0,0,0
17,2,OFF2,0,30325.0,30855.7,71,2,0,0,0,0
18,0,ON,1,30536.0,31172.6,139,2,10000,1136994,1100000,94103521
18,1,OFF1,0,30476.0,31063.5,71,2,0,0,0,0
18,2,OFF2,0,30486.0,31112.3,71,2,0,0,0,0
19,0,OFF1,0,30436.0,31021.8,69,2,0,0,0,0
19,1,ON,1,30376.0,31254.8,138,2,10000,1114378,1100000,94054160
19,2,OFF2,0,30305.0,30889.1,71,2,0,0,0,0
20,0,ON,1,30556.0,31225.7,138,2,10000,1124010,1100000,94028767
20,1,OFF2,0,30466.0,31092.5,71,2,0,0,0,0
20,2,OFF1,0,30475.0,31404.2,71,2,0,0,0,0
21,0,OFF2,0,30405.0,31006.7,71,2,0,0,0,0
21,1,ON,1,30636.0,31244.8,138,2,10000,1135119,1100000,94033984
21,2,OFF1,0,30356.0,30876.4,71,2,0,0,0,0
22,0,ON,1,30606.0,31244.2,139,2,10000,1091521,1100000,94074579
22,1,OFF1,0,30456.0,31192.7,71,2,0,0,0,0
22,2,OFF2,0,30446.0,31076.5,71,2,0,0,0,0
23,0,ON,1,30576.0,31143.2,138,2,10000,1101059,1100000,93969778
23,1,OFF1,0,30275.0,30977.1,71,2,0,0,0,0
23,2,OFF2,0,30406.0,31028.0,72,2,0,0,0,0
24,0,ON,1,30486.0,31125.3,139,2,10000,1095463,1100000,94665164
24,1,OFF2,0,30416.0,30994.2,71,2,0,0,0,0
24,2,OFF1,0,30366.0,31062.4,71,2,0,0,0,0
25,0,OFF2,0,30476.0,31650.5,71,2,0,0,0,0
25,1,ON,1,30536.0,31203.3,137,2,10000,1121171,1100001,94401968
25,2,OFF1,0,30366.0,31007.0,71,2,0,0,0,0
26,0,OFF1,0,30466.0,31434.5,70,2,0,0,0,0
26,1,OFF2,0,29784.0,30513.2,71,2,0,0,0,0
26,2,ON,1,29885.0,30689.7,137,2,10000,1120330,1100000,94043761
27,0,ON,1,30446.0,31019.3,138,2,10000,1126325,1100000,94004208
27,1,OFF1,0,30366.0,30941.7,71,2,0,0,0,0
27,2,OFF2,0,30386.0,31012.7,70,2,0,0,0,0
28,0,OFF1,0,30386.0,30953.2,71,2,0,0,0,0
28,1,ON,1,30606.0,31308.6,139,2,10000,1104257,1100000,94654916
28,2,OFF2,0,29965.0,30659.4,70,2,0,0,0,0
29,0,OFF1,0,29825.0,30687.8,71,2,0,0,0,0
29,1,ON,1,30496.0,30980.6,139,2,10000,1102299,1100000,94399665
29,2,OFF2,0,30286.0,31059.1,71,2,0,0,0,0
```

Columns: `connect4_run_count_delta` / `_run_time_ns_delta` and `xdp_run_*` are `ProgramInfo` deltas over the timed connect loop plus the XDP test-run (0 when stats are off).

## Things that went wrong

1. First run panicked at the XDP program lookup (kernel truncates program names to 15 chars: `xdp_service_map`, not `xdp_service_map_lookup`). The RAII guard restored the sysctl, but the cgroup and bpffs pin of that process leaked; removed by hand afterwards. Fixed the filter and reran; only the second run's data is used.
2. Two `metal run` invocations were killed client-side by the tool time cap while the first sync materialised the standalone `.git` (~570 MB at ~400 KB/s); the remote run itself is durable and was collected (`metal collect`). A pipe through `tail` on the first attempt hung. `.env` was missing in this worktree; copied from a sibling workspace (it is gitignored).

## Recommendation

Leaving it on permanently is acceptable for macro benchmarks (a +0.5% constant offset) provided every arm of every comparison runs with the same setting and the setting is recorded in the environment capture; it must NOT be left on, or must be toggled off, for micro-benchmarks of individual BPF program cost (it roughly doubles a ~70 ns XDP run).
