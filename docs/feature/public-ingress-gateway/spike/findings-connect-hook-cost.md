# Findings: per-connect cost of the gateway selection hook (R-55, report-only)

Status: report-only, point-in-time. This is a benchmark-style receipt. It is not an
EDD expectation, not a test, and not a gate. It has no baseline, no lane and no
harness binary in `crates/`. Tracking: GH #287. Verdict: not applicable (no
WORKS/DOESN'T-WORK claim is made).

## Question

What does attaching `cgroup_connect4_gateway` beside `cgroup_connect4_service` add to
a loopback connect-and-close that no gateway intent is registered for (the case R-1
accepts for every unregistered workload connect)?

## Setup

| Item | Value |
|---|---|
| Kernel (`uname -r`) | `7.0.0-38-generic` (the pinned appliance kernel, native x86_64 metal host, 16 CPUs) |
| Source | commit `bb0b52244`, which adds the program; the measured program is the one in that commit. The object measured is the final program (125 verified instructions) |
| Probe | `spike-scratch/pig-connect-cost/` (throwaway, own `Cargo.toml`/workspace; links the production `overdrive-dataplane` by path only to load and attach the real object) |
| `kernel.bpf_stats_enabled` | `1` for the run (prior value `0`, restored; verified `0` after) |
| Cgroup | a dedicated `/sys/fs/cgroup/pig-connect-cost-<pid>` that the probe process joins and passes as the dataplane's `cgroup_attach_path`; removed afterwards |
| Workload | per run: 500 warm-up connects, then 5 000 timed loopback `connect(2)` + close (`SO_LINGER` 0, so no TIME_WAIT accumulation) to an in-process listener that accepts and drops |
| Arm A | `cgroup_connect4_service` only: the dataplane is built, then the gateway program's link is detached with `bpftool link detach` (its `run_count` stays 0) |
| Arm B | both programs attached (the production composition) |
| Runs | 30 per arm, interleaved (A,B on even runs; B,A on odd runs); a fresh `EbpfDataplane` per run |
| Metrics | per program `ProgramInfo::run_time() / run_count()` delta across the timed loop; wall time per connect over the same loop |

The two `connect4` programs share the 15-character kernel program name
`cgroup_connect4`; they are told apart by program id (the service program loads
first, so it has the lower id). Program index 0 is `cgroup_connect4_service`,
index 1 is `cgroup_connect4_gateway`.

## Result (30 runs per arm)

Hook time per connect (`run_time / run_count`, ns):

| Program | median | mean | min | max |
|---|---|---|---|---|
| `cgroup_connect4_service`, arm A | 108.1 | 107.7 | 103.6 | 115.8 |
| `cgroup_connect4_service`, arm B | 108.2 | 107.9 | 104.6 | 111.3 |
| `cgroup_connect4_gateway`, arm B | 116.8 | 117.8 | 115.5 | 126.5 |

Total connect4 hook time per connect: arm A median 108.2 ns, arm B median 225.2 ns,
so the second program adds a median of 117.0 ns per connect.

Wall time per connect (ns, includes the whole connect-and-close and the
`bpf_stats` accounting itself):

| Arm | median | mean | min | max | stdev |
|---|---|---|---|---|---|
| A | 27 947.7 | 27 943.7 | 27 142.9 | 30 264.7 | 676.2 |
| B | 28 115.8 | 28 368.5 | 27 402.5 | 30 167.8 | 731.3 |

Median wall delta B minus A: about 168 ns per connect (0.6% of a 28 us connect),
inside one standard deviation of either arm (about 700 ns). The wall-time delta is
not distinguishable from run-to-run noise at this sample size; the per-program
`run_time` delta (about 117 ns) is the stable number.

Interpretation, limited to what was measured: for a socket with no registered
intent the added work is one program run costing about 117 ns of hook time, on a
connect that costs about 28 us end to end on this host. Not measured: connects
that are registered (the selection arm: outer, inner and backend lookups, receipt
write, rewrite), non-loopback connects, other CPU counts or loads, and any
workload other than connect-and-close.

## Raw per-run numbers

Columns: arm, run, connect4 programs that ran during the timed loop, wall ns per
connect (same value on both rows of a run), program index, `run_count` delta,
`run_time` delta in ns. In arm A the gateway row (index 1) has `run_count` 0: the
program was detached.

```
arm,run,connect4_programs_that_ran,wall_ns_per_connect,prog_idx,run_count,run_time_ns
A,0,1,27273.7,0,5000,542832
A,0,1,27273.7,1,0,0
B,0,2,27702.0,0,5000,527312
B,0,2,27702.0,1,5000,589758
B,1,2,28017.2,0,5000,539526
B,1,2,28017.2,1,5000,579975
A,1,1,30264.7,0,5000,541332
A,1,1,30264.7,1,0,0
A,2,1,28141.9,0,5000,540898
A,2,1,28141.9,1,0,0
B,2,2,30013.0,0,5000,542541
B,2,2,30013.0,1,5000,585027
B,3,2,30156.2,0,5000,545650
B,3,2,30156.2,1,5000,607798
A,3,1,28181.3,0,5000,555351
A,3,1,28181.3,1,0,0
A,4,1,27787.7,0,5000,537818
A,4,1,27787.7,1,0,0
B,4,2,27925.6,0,5000,533229
B,4,2,27925.6,1,5000,580540
B,5,2,28021.6,0,5000,528153
B,5,2,28021.6,1,5000,605741
A,5,1,27723.2,0,5000,532750
A,5,1,27723.2,1,0,0
A,6,1,27905.7,0,5000,546499
A,6,1,27905.7,1,0,0
B,6,2,28091.3,0,5000,535867
B,6,2,28091.3,1,5000,579541
B,7,2,28120.8,0,5000,548739
B,7,2,28120.8,1,5000,603210
A,7,1,28039.9,0,5000,522393
A,7,1,28039.9,1,0,0
A,8,1,27231.4,0,5000,523432
A,8,1,27231.4,1,0,0
B,8,2,27749.7,0,5000,535180
B,8,2,27749.7,1,5000,579190
B,9,2,28317.6,0,5000,536169
B,9,2,28317.6,1,5000,589530
A,9,1,27953.7,0,5000,521499
A,9,1,27953.7,1,0,0
A,10,1,27979.1,0,5000,524032
A,10,1,27979.1,1,0,0
B,10,2,30167.8,0,5000,552233
B,10,2,30167.8,1,5000,584056
B,11,2,28553.1,0,5000,545132
B,11,2,28553.1,1,5000,577521
A,11,1,28005.6,0,5000,538662
A,11,1,28005.6,1,0,0
A,12,1,27831.3,0,5000,547774
A,12,1,27831.3,1,0,0
B,12,2,28102.6,0,5000,526189
B,12,2,28102.6,1,5000,579326
B,13,2,28462.9,0,5000,547844
B,13,2,28462.9,1,5000,590872
A,13,1,27148.2,0,5000,541227
A,13,1,27148.2,1,0,0
A,14,1,27587.7,0,5000,545150
A,14,1,27587.7,1,0,0
B,14,2,28167.3,0,5000,540964
B,14,2,28167.3,1,5000,632685
B,15,2,28149.5,0,5000,528531
B,15,2,28149.5,1,5000,584098
A,15,1,27912.0,0,5000,536338
A,15,1,27912.0,1,0,0
A,16,1,28018.0,0,5000,537460
A,16,1,28018.0,1,0,0
B,16,2,28102.9,0,5000,556446
B,16,2,28102.9,1,5000,579879
B,17,2,28111.1,0,5000,540927
B,17,2,28111.1,1,5000,583843
A,17,1,28001.2,0,5000,548128
A,17,1,28001.2,1,0,0
A,18,1,28007.7,0,5000,543859
A,18,1,28007.7,1,0,0
B,18,2,28404.6,0,5000,536793
B,18,2,28404.6,1,5000,594825
B,19,2,28115.4,0,5000,549578
B,19,2,28115.4,1,5000,582464
A,19,1,28306.0,0,5000,578929
A,19,1,28306.0,1,0,0
A,20,1,29967.1,0,5000,518144
A,20,1,29967.1,1,0,0
B,20,2,30104.4,0,5000,542669
B,20,2,30104.4,1,5000,588331
B,21,2,28461.6,0,5000,539825
B,21,2,28461.6,1,5000,607967
A,21,1,27941.6,0,5000,541907
A,21,1,27941.6,1,0,0
A,22,1,27988.3,0,5000,540308
A,22,1,27988.3,1,0,0
B,22,2,27992.4,0,5000,543547
B,22,2,27992.4,1,5000,607566
B,23,2,28116.3,0,5000,541894
B,23,2,28116.3,1,5000,583862
A,23,1,28011.2,0,5000,543190
A,23,1,28011.2,1,0,0
A,24,1,27663.9,0,5000,547121
A,24,1,27663.9,1,0,0
B,24,2,28266.8,0,5000,546317
B,24,2,28266.8,1,5000,584054
B,25,2,28089.4,0,5000,522768
B,25,2,28089.4,1,5000,590502
A,25,1,27472.8,0,5000,536754
A,25,1,27472.8,1,0,0
A,26,1,27142.9,0,5000,528152
A,26,1,27142.9,1,0,0
B,26,2,28020.3,0,5000,538663
B,26,2,28020.3,1,5000,579863
B,27,2,27402.5,0,5000,528959
B,27,2,27402.5,1,5000,577335
A,27,1,27185.6,0,5000,521139
A,27,1,27185.6,1,0,0
A,28,1,27530.2,0,5000,529184
A,28,1,27530.2,1,0,0
B,28,2,27988.8,0,5000,542212
B,28,2,27988.8,1,5000,580918
B,29,2,28160.0,0,5000,542492
B,29,2,28160.0,1,5000,583035
A,29,1,28108.7,0,5000,544335
A,29,1,28108.7,1,0,0
```
