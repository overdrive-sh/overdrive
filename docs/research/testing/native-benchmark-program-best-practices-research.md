# Research: Best practices for a native performance-benchmark program (Overdrive)

**Date**: 2026-11-13 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High overall (per-finding ratings below) | **Sources**: 40 (35 external, 4 in-repo, 1 vendored kernel source)

Scope: `docs/feature/native-benchmark-program/intake.md` and anchor issue #287. This is research input for the DISCUSS/DESIGN waves. Everything under "Recommendations for Overdrive" is a **proposal**, not a decision; the user decides.

## Executive Summary

Mature benchmark practice converges on a small set of rules that map directly onto #287. (1) Treat the *independent run* (a fresh process/fixture/VM) as the unit of replication; iterations inside a run are correlated, so repetition counts must be chosen per level, not by total iteration count [3][22]. (2) Keep every raw sample and report distributions with confidence intervals and effect sizes, not a best-of or a bare mean [2][22][28]. (3) For A vs B, interleave and randomise arm order within one environment, use a nonparametric two-sample test with enough repetitions (Google Benchmark states "no less than 9") and report an effect size with its interval [7][8][18][2]. (4) For latency under load, use open-loop (scheduled-arrival) generation or a correction, otherwise coordinated omission hides the tail [1][11][29]. (5) Control the noise you can (frequency/turbo, SMT, ASLR, affinity, IRQ, logging) and *record* what you could not [24][12][13][10][5]; Brendan Gregg's "active benchmarking" adds: watch the system during the run to confirm what is actually being measured [37].

Established tools converge on a two-part result record: raw samples (or a mergeable histogram) plus a context block with host/build/revision metadata [7][6][9][21][31]. None of them version the *methodology* itself in the file; that is a gap Overdrive's "versioned methodology" requirement has to fill on its own (Knowledge Gap 2).

For regression handling (explicitly a later decision in #287), MongoDB's experience is the strongest primary evidence: fixed thresholds produced up to "99%" false positives depending on how one counts, and could not serve both noisy and quiet tests with one value; they moved to E-Divisive-means change-point detection with human triage [4]. Bencher offers per-metric statistical thresholds over a sliding baseline [20]. Overdrive's current Tier-4 gate is a fixed-fraction gate on a mean [in-repo 38], which is the shape MongoDB moved away from — but it is a deterministic `BPF_PROG_TEST_RUN` loop, a very different noise regime.

In-process Rust frameworks (Criterion, Divan, Tango, iai-callgrind) are strong on local pure code and weak or inapplicable for cross-process, kernel- and VM-crossing boundaries: they measure a closure in one process, and iai-callgrind explicitly offers no wall-clock time [2][17][18][19]. The reusable part is their *analysis design* (warm-up, bootstrap, noise threshold, paired interleaving), not their harness. The in-repo spikes already use most of the good practices (interleaved A/B, fresh fixture per run, kept raw rows, control runs, failed attempts retained) but each re-implements sampler, recorder and report by hand [38]. That is the argument for a shared measurement core with per-scenario fixtures.

The kernel claims in this document were first written from `docs.kernel.org` only, then re-grounded by reading `vendors/linux` (tag v7.0, commit 028ef9c96); source is now the authority and kernel citations are `path:line @ v7.0`, relative to `vendors/linux` [40]. Three results change the methodology. (a) `bpf_stats_enabled` is a **global** static key, not a per-program or per-namespace switch; when on, every program run reads `sched_clock()` twice and bumps per-CPU counters, with no IRQ-save or seqcount on x86-64; the cost is therefore small and bounded, but the key is shared by every concurrent run on the node, and `BPF_ENABLE_STATS` offers an fd-scoped alternative that reverts on process exit. (b) The stated cgroup-accounting bias is **partly right and more general than stated**: with IRQ-time accounting on (enabled at TSC init on x86 unless `no_sched_irq_time` is given, and only if the image is built with `CONFIG_IRQ_TIME_ACCOUNTING`), hard-IRQ and softirq time is charged to no non-root cgroup at all, and `ksoftirqd`/`kworker` time is charged to the root cgroup; but this hits *both* arms, so its direction is not "toward the kernel arm" in general, and vhost workers are charged to the VMM's cgroup, not to the root. Whole-host `/proc/stat` stays the right primary CPU scope. (c) A guest and the host do **not** share a timebase for timestamps: the guest's `kvm-clock` runs at the rate of host `CLOCK_MONOTONIC_RAW` but is zeroed at VM creation, and the guest's own `CLOCK_MONOTONIC` is zeroed again at guest boot, so host→guest stage boundaries need one-sided timestamping or an explicit offset estimate (F1.9). Residual questions that source reading could not settle are listed in Knowledge Gaps 6 and 7.

## Research Methodology

**Search strategy**: primary documentation and README of each named tool; peer-reviewed methodology papers; kernel.org documentation; vendor engineering write-ups; repo-local prior art. WebSearch used to locate papers; WebFetch used to extract text. **Source selection**: official/primary first (kernel.org, tool repos, arXiv/ACM papers); secondary blogs flagged. **Verification**: where possible two independent sources per claim; single-source claims are rated Low/Medium. **Adversarial check**: fetched pages were scanned for embedded directives; none found. A WebSearch result summary is treated as a pointer, not evidence, unless noted.

**Coverage limits** (see Research Metadata): several papers were read via abstract/search snippet only; one page (Gregg) failed to fetch. Kernel behaviour was subsequently verified by reading the vendored Linux v7.0 source (`vendors/linux`), per `.claude/rules/design.md` § "Ground every kernel claim in the Linux source Overdrive runs"; where source and `docs.kernel.org` differ, the source is followed and the change is stated.

---

## Findings

### Q1. Methodology for end-to-end, cross-process, kernel- and VM-crossing benchmarks

**F1.1 — Repetition must be planned per level of the experiment; the independent run is the unit.**
Kalibera and Jones present a cost model identifying "the number of repetitions at each level of an experiment necessary and sufficient to obtain a given level of precision", and recommend reporting an effect-size confidence interval [3]. Georges et al. argue that "best of N" and similar practices do not control non-determinism and advocate 95% confidence intervals [22]. Hoefler and Belli surveyed 120 papers and found reporting practice "lacking"/"not sufficient" (abstract wording differs between copies) and propose guidelines and a library [28].
*Interpretation (mine)*: in the existing `pig-connect-cost` probe a run is one fresh `EbpfDataplane` with 5,000 timed connects [38]; the 5,000 are correlated within the run, so the replicate count is 30 per arm, not 150,000. The benchmark design should state the replication unit per family (a VM launch; a fresh daemon+fixture; a fresh cgroup).
**Confidence**: High on the principle (3 independent sources), Medium on how the cost model applies to Overdrive (abstracts only).

**F1.2 — Measurement bias from setup is real; randomise setup, do not just minimise it.**
Mytkowicz et al. show that innocuous setup changes (their examples include link order; environment size is cited in the title/abstract of the paper but I only confirmed link order via a secondary summary) can flip conclusions, found none of 133 surveyed papers adequately considered it, and propose *setup randomisation* and causal analysis [23]. LLVM's benchmarking guide cites the same paper and states low noise is "necessary but not sufficient" [24].
**Confidence**: High (two sources); the environment-size detail is Low (unverified).

**F1.3 — Warm-up and cold/warm are different things; do both deliberately.**
Criterion warms up by running the routine with doubling iteration counts until a time budget passes, then measures [2]. hyperfine offers `--warmup` for warm runs and `--prepare` to run a command before each run, with `sync; echo 3 | sudo tee /proc/sys/vm/drop_caches` as its cold-cache example [6]. Cloud Hypervisor's metrics run 10 iterations for boot time and restore latency, 5 for network/block tests, and report mean, std-dev, min, max [9]. The repo's snapshot-restore prior method (#96) is "5 interleaved trials/mode, cold page cache, controls, nothing discarded" [intake].
*Interpretation*: for VM lifecycle and image-cache benchmarks, "cold" and "warm" must be separate named profiles, each with an explicit preparation step recorded in the result (what was dropped, what was pre-pulled), not an accident of run order.
**Confidence**: High on tool behaviour (3 sources); the cold/warm profile rule is a proposal.

**F1.4 — Outliers: keep them, flag them.**
Criterion classifies outliers with a modified Tukey IQR method, warns, but "outlier samples are not dropped from the data" [2]. hyperfine advertises outlier *detection* "to detect interference from other programs and caching effects" without discarding [6]. MongoDB's change-point system treats correlated noise as something to "learn to live with", not remove [4]. This matches the repo's own "nothing discarded" policy [intake; 38].
**Confidence**: High (3 sources agree: detect and flag, do not delete).

**F1.5 — Bootstrap confidence intervals and nonparametric comparison are the common analysis.**
Criterion bootstraps resamples to produce a CI and compares old vs new with a t-test on bootstrap distributions, treating changes within a noise threshold (example ±1%) as noise [2]. Google Benchmark's `compare.py` uses Mann–Whitney U with null hypothesis "no difference", requires "no less than 9" repetitions, reports relative delta `(new - old)/|old|`, and warns that significance does not prove a true difference [8]. Tango applies a configurable noise threshold (default 0.5%) [18]. The Chromium HistogramSet format keeps `sampleValues` "to support Mann-Whitney U hypothesis testing" [31].
**Disagreement**: Criterion uses a t-test on a bootstrap; Google Benchmark and Chromium use Mann–Whitney; Georges et al. use Student-t CIs under a normality assumption [22]. They are not interchangeable for skewed latency data.
**Assessment**: for latency distributions (right-skewed, multimodal — see F1.6) the nonparametric route is better supported; I could not retrieve Hoefler and Belli's body text to confirm their specific recommendation on medians/nonparametric CIs [unverified from fetched text].
**Confidence**: Medium-High.

**F1.6 — Coordinated omission (CO) and open- vs closed-loop load.**
wrk2's README: wrk-style generators measure from send to full response and wait per connection, producing "a strong Coordinated Omission effect"; wrk2 sends a fixed rate (`-R`) and measures latency "from when a request *should* have been sent", and in its 1.4 s-stall example the corrected p99 was ~200x the uncorrected [1]. Tene's talk slides and a mailing-list post: omission matters when it is *coordinated* with the events being observed; HdrHistogram's expected-interval recording back-fills missed samples [29][11]. HdrHistogram has constant memory, configurable significant digits, and is additive across threads/time [11]. wrk2 limits: ±1 ms accuracy from OS sleep behaviour, 10 s calibration, "experimental" status [1].
**Confidence**: High (3 sources, though [29] and [11] are the same author community; wrk2 is independent code).

**F1.7 — Hierarchical, paired, interleaved designs reduce environment drift.**
Tango measures two functions "simultaneously" in one process by interleaving, claiming ~1% detectability in ~1 s; limited to in-process comparison and requires building baseline separately [18]. Google Benchmark's `--benchmark_enable_random_interleaving` "can help reduce the impact of system state changes" [7]. The repo spike used 30 runs/arm, A,B on even runs and B,A on odd runs, fresh dataplane per run [38].
**Confidence**: Medium-High (3 sources; Tango's detectability number is the vendor's own claim, unverified).

**F1.8 — Active benchmarking: verify what you measure while you measure.**
Gregg: observe the system with other tools during the run to establish what limits the benchmark, versus passive "fire and forget" [37]. Matches the repo's own diagnostic culture (`debugging.md` § probes, populations) and its use of `/proc/stat`, `perf stat -a` and inventory in the density benchmark [38].
**Confidence**: Medium — I could not fetch Gregg's page (connection refused); this rests on a search-result summary of it. Re-fetch before citing as evidence in a DESIGN artifact.

**F1.9 — Clock sources, and why host and guest timestamps cannot be subtracted (Linux v7.0 source [40]).**
*Host clocks* (all paths `@ v7.0`, relative to `vendors/linux`):
- `CLOCK_MONOTONIC` reads the timekeeper's `tkr_mono` (`kernel/time/posix-timers.c:210-220`, `kernel/time/timekeeping.c:955`). Its multiplier is **NTP-adjusted**: `timekeeping_adjust` edits `tkr_mono.mult` (`timekeeping.c:2163`), so the clock can be slewed while a benchmark runs.
- `CLOCK_MONOTONIC_RAW` reads `tkr_raw` (`posix-timers.c:222-227,1462`; `timekeeping.c:929-944`). Same clocksource, but `tkr_raw.mult` is set only when the clocksource is (re)selected and never NTP-adjusted (`timekeeping.c:366-367`).
- `CLOCK_BOOTTIME` is `tkr_mono` plus `offs_boot` (`timekeeping.c:857-875`; `posix-timers.c:249-259`); `offs_boot` changes only when suspend sleep time is injected (`timekeeping.c:267-276,1918`). On a host that never suspends, BOOTTIME equals MONOTONIC.
- All three are read through `ktime_get*` and share the underlying clocksource (TSC on the metal host when selected; which clocksource is active is runtime state, not source-determinable).

*Guest clocks.*
- The guest's `kvm-clock` clocksource has rating 400, but the rating is dropped to 299 when the VMM exposes constant and non-stop TSC and TSC is not marked unstable, so the TSC clocksource (rating 300) wins (`arch/x86/kernel/kvmclock.c:158-166,343-346`; `arch/x86/kernel/tsc.c:1199`). The guest `sched_clock()` is redirected to `kvm_sched_clock_read` with an offset captured at guest init either way (`kvmclock.c:90-100,322`). Which of the two the Overdrive guest uses depends on the CPUID bits the Cloud Hypervisor fork exposes; that fork is not vendored in this worktree, so it is **not settled** (Knowledge Gap 7).
- What KVM hands the guest: `system_time = kernel_ns + kvmclock_offset` (`arch/x86/kvm/x86.c:3392`), where `kernel_ns` is the host's `CLOCK_MONOTONIC_RAW` plus boot offset (`do_kvmclock_base`, `x86.c:2967-2984`; fallback `get_kvmclock_base_ns`, `:2398-2406`) and `kvmclock_offset` is set to the negative of that value when the VM is created (`x86.c:13253`). So guest kvm-clock runs at the rate of host `CLOCK_MONOTONIC_RAW` ("a simple function of the TSC without any such adjustment", `x86.c:3443-3445`) and reads about zero at **VM creation**.
- That relation holds tightly only when KVM uses the "master clock": host clocksource TSC-based and all vCPU TSCs matched (`x86.c:2605-2607,3118-3148`). Only then is `PVCLOCK_TSC_STABLE_BIT` set (`x86.c:3397-3398`), and the guest only trusts pvclock in the vDSO or as a stable sched clock when that bit is present (`kvmclock.c:95-98,250-257`). Without it `kernel_ns` and the TSC are read at different instants (`x86.c:3347-3350`).
- The guest's own `CLOCK_MONOTONIC`/`CLOCK_MONOTONIC_RAW` start near zero at guest-kernel timekeeping init (`timekeeping_init`, `timekeeping.c:1801-1843`: `raw_sec = 0`, `wall_to_mono = boot_offset - wall_time`), which is later than VM creation by firmware and bootloader time. (That the value continues unbroken across the switch from the early jiffies clocksource to kvm-clock or TSC is my inference, not read here.)

*What follows.* Host `CLOCK_MONOTONIC_RAW` and guest `CLOCK_MONOTONIC_RAW` tick at (nearly) the same rate but have different, unrecorded zero points: one is near VM creation, the other near guest boot. A guest timestamp therefore cannot be subtracted from a host timestamp. Guest `CLOCK_MONOTONIC` can additionally be slewed by guest NTP, and `CLOCK_REALTIME` can be stepped; neither suits stage boundaries. The guest's wall-clock epoch is computed host-side once, precisely only under the master clock (`kvm_get_wall_clock_epoch`, `x86.c:3457-3512`), and is no better than that.

*Proposal (stage timing; the user decides).*
1. **Stamp every cross-boundary stage boundary on one side only.** Host-observable boundaries (request accepted, VMM spawned, VMM API ready, vsock connect established, guest READY *received*) are stamped by the observing host process with one host clock for the whole run (`CLOCK_MONOTONIC_RAW`, so a mid-run NTP slew cannot bend an interval). "Guest READY" is then defined as host receipt of the READY message and includes one-way vsock delivery; the result names it that way.
2. **Guest-internal stages are reported as durations, not timestamps.** The guest measures each as a difference of two reads of one guest clock (`CLOCK_MONOTONIC_RAW`) and reports the duration; the unknown zero point cancels. Durations from the two sides are summed or compared as intervals, never as instants.
3. **If an absolute guest-to-host mapping is required**, estimate the offset with repeated host↔guest exchanges over vsock and keep the minimum-round-trip sample; record the residual bound (at most half the round trip) in the result. This is a standard technique, not something the kernel source provides, and it adds a measurement step; skip it unless a stage truly needs an instant on the other side.
4. **Record clock provenance in the context block**: host and guest current clocksource (`/sys/devices/system/clocksource/clocksource0/current_clocksource`), and whether the host used the KVM master clock (tracepoint `kvm_update_master_clock`, `x86.c:3145`).
**Confidence**: High on the host-clock and KVM-to-guest-clock relations (read in source); Medium on guest zero-point behaviour (one inference flagged above); the Overdrive guest's actual clocksource is not settled.

---

### Q2. Noise control on bare-metal Linux

| Control | Source-backed guidance | Confidence |
|---|---|---|
| Frequency / turbo | `intel_pstate`: setting `no_turbo` to 1 forbids turbo P-states; "turbo P-states… are not sustainable" and max turbo depends on active core count [13]. LLVM: governor `performance`, disable turbo, write `no_turbo` [24]. Google Benchmark warns "CPU scaling is enabled… may be noisy" [7]. | High (3) |
| SMT | LLVM: take the SMT sibling of benchmark CPUs offline via `thread_siblings_list`/`online` [24]. Firecracker recommends disabling SMT for *tenant separation* (a security, not benchmarking, reason) [10]. | Medium |
| ASLR | LLVM: set `/proc/sys/kernel/randomize_va_space` to 0 [24]. **Tension**: Mytkowicz argues for *randomising* setup to expose bias [23]; disabling ASLR reduces variance but may hide layout sensitivity. | Medium (sources disagree in emphasis) |
| CPU shielding / affinity | LLVM: `cset shield -c N -k on`, keep ≥2 cores free for `perf`; claims "<0.1%" perf variation with these settings [24]. Kernel docs: `nohz_full=`, `rcu_nocbs=`, bind kthreads (`ksoftirqd`, `kworker`, `rcuc/rcuo*`) elsewhere, IRQ affinity, `sched_setaffinity`/cgroups [12]. | High (2 authoritative) |
| IRQ affinity | `/proc/irq/N/smp_affinity`/`smp_affinity_list` select permitted CPUs; default mask applies to not-yet-active IRQs; controllers lacking affinity support keep all CPUs [36]. The page does not discuss `irqbalance`. | High (kernel doc); irqbalance interaction Low |
| Host logging noise | Firecracker: host console logging "degraded the snapshot restore time from 3ms to 8.5ms on aarch64"; recommends `quiet loglevel=1`; KVM PIT timer interrupts raise host CPU use [10]. | Medium (1 source, vendor primary) |
| Boot-time regressions from host config | Firecracker: Linux 6.1 regressed x86_64 boot time; mitigations `nx_huge_pages=never` or cgroup `favordynmods` [10]. | Medium (1 source) |
| `bpf_stats_enabled` | Kernel sysctl doc: "Enabling statistics causes a slight reduction in performance on each program run"; default 0 [5]. Source: a global static key, default off; when on, each run reads `sched_clock()` twice and updates per-CPU counters (F3.3) [40]. | High (doc and source agree); the cost is bounded by the code path but its wall-time magnitude is still **not measured** — measure it (Q3). |
| Swap / KSM | Firecracker recommends disabling swap and KSM (security rationale) [10]. | Low for benchmarking relevance |
| Recording the environment | Google Benchmark's JSON `context` records `host_name`, `num_cpus`, `mhz_per_cpu`, `cpu_scaling_enabled`, `library_build_type`, `load_avg`, `caches`, `date` [7]. Cloud Hypervisor records `git_revision`, `git_human_readable`, `git_commit_date`, `date`, but its page names no reference hardware [9]. | High |

**Kernel-source check of the controls above** (all `@ v7.0`, relative to `vendors/linux`; each exists as the sources describe) [40]:
- `no_turbo`: `/sys/devices/system/cpu/intel_pstate/no_turbo`, created by `intel_pstate_attr_group` under a kobject named `intel_pstate` (`drivers/cpufreq/intel_pstate.c:1813-1833`); `store_no_turbo` clamps input to 0/1, refuses to clear it when turbo is disabled by BIOS (`:1598-1645`). The driver must be active, else the write returns `-EAGAIN` (`:1609-1610`).
- SMT: `/sys/devices/system/cpu/smt/control` accepts `on`, `off`, `forceoff` and reads back `notsupported`/`notimplemented` (`kernel/cpu.c:2898-2962`, group name `smt` at `:3006`); `nosmt` boot parameter at `kernel/cpu.c:661`. `forceoff` cannot be reverted at runtime (`:2904`).
- ASLR: `/proc/sys/kernel/randomize_va_space` (`mm/memory.c:121-131`).
- `nohz_full=` and `isolcpus=` (`kernel/sched/isolation.c:312,367`), `rcu_nocbs` (`kernel/rcu/tree_nocb.h:71`), `irqaffinity=` (`kernel/irq/irqdesc.c:39`); per-CPU kthread names `ksoftirqd/%u` (`kernel/softirq.c:1107`), `rcuc/%u` (`kernel/rcu/tree.c:2977`), `rcuog/%d` (`kernel/rcu/tree_nocb.h:1367`). `/proc/irq/N/smp_affinity` and `smp_affinity_list` (`kernel/irq/proc.c:363,370`). SMT sibling list file `thread_siblings_list` (`drivers/base/topology.c:82`).
- `perf_event_paranoid` defaults to **2** (`kernel/events/core.c:495`); system-wide (`-a`) CPU events need the value ≤ 0 or `perfmon_capable()` (`include/linux/perf_event.h:1795-1801`: `perf_allow_cpu` returns `-EACCES` when > 0 without the capability). A root run on the metal host passes through the capability check regardless of the value; a non-root run does not.

**Noise sources decomposed.** MongoDB models measured noise as platform + software + workload-generator components (N = Np + Ns + Nw), notes the noise "is not clear a priori" to be IID, has *correlated* noise (cloud time-varying behaviour, code-alignment effects on caches), and uses **canary workloads** whose change "tell us nothing about the software we are testing, but a lot about the test system itself" [4]. This is the best primary-source pattern for host-eligibility checks: run a fixed canary before/with a benchmark and refuse or flag the run if the canary shifts.
**Confidence**: High on the decomposition and canary idea (primary paper); Medium on transfer to a single dedicated metal host.

**Concurrency noise specific to Overdrive.** The repo's metal runner admits concurrent *ordinary* runs from different worktrees and isolates them via namespaces, not CPU partitioning [in-repo 39]. Namespaces isolate state, not CPU, cache, memory bandwidth or IRQ load. Concurrent runs are therefore a noise source for latency/throughput benchmarks that the cited sources' controls (cpuset shielding, IRQ affinity) do not address by themselves. See Open Question 1.

**Not found**: authoritative guidance for noise of *nested* virtualization (benchmark guests as nested Cloud Hypervisor guests inside a KVM guest). Overdrive's rules already treat the native metal host as the only signal [39]; whether a benchmark may run inside a guest-group is undecided (Knowledge Gap 4).

---

### Q3. Userspace vs in-kernel comparisons

**F3.1 — Mature projects fix the boundary and the load, and vary only the mechanism.**
Cilium's published benchmark uses netperf `TCP_RR` (round-trip latency/efficiency) and `TCP_CRR` (connections in sequence) with a 100 Gbit/s NIC so "the network card is not the bottleneck", a single netperf process over one TCP connection so processing lands on one core, and varies the datapath (iptables/kube-proxy vs eBPF host routing); it uses CPU flamegraphs to attribute differences [26]. The XDP paper benchmarks against DPDK using 64-byte packets and reports per-core packet rates, and publishes datasets, experimental notes and the kernel branch used [32]. Netflix's kTLS slides compare configurations on one server and report throughput *with CPU utilisation* (375 Gb/s at 53% CPU; sendfile+kTLS 240 Gb/s at ~80%; kTLS without sendfile 75 Gb/s at ~80%) [27]. The Intel netdev 0x14 slides compared kTLS sendfile and userspace TLS and found kTLS used "considerably more memory bandwidth" in media streaming [27]; the Facebook kTLS paper reported "up to 7% CPU copy overhead" saved and "up to 10% latency" improvement [27].
**Pattern**: report throughput *and* CPU (and memory/memory-bandwidth where relevant), same workload, same host, mechanism as the only variable.
**Confidence**: High on pattern; individual numbers are vendor-reported and not transferable. Netflix numbers are FreeBSD, not Linux. The Cilium search summary is from a search result, with the docs page as the primary source [26].

**F3.2 — Attribution of CPU/memory to kernel vs userspace has several instruments, each with a blind spot.**

| Instrument | What it gives | Documented caveat | Source |
|---|---|---|---|
| BPF stats (`run_time_ns`, `run_cnt`) | cumulative per-program time and count, summed over all possible CPUs → **mean** per run only. Exported by `bpf_prog_info` as `run_time_ns`, `run_cnt` (plus `recursion_misses`) [40: `kernel/bpf/syscall.c:2459-2484,5066-5069`] | needs the global stats key on (sysctl or `BPF_ENABLE_STATS` fd); two `sched_clock()` reads per run; no per-invocation latency (only two cumulative sums exist), so no true p99; the timed span is the program body only, not the surrounding hook wrapper (F3.3) | [5]; [40]; repo Tier-4 baseline files state mean-only [38] |
| cgroup v2 `cpu.stat` | `usage_usec`, `user_usec`, `system_usec`, `nice_usec` per cgroup incl. descendants. Non-root: `usage_usec` is the cgroup's summed task `sum_exec_runtime`; user/system are tick-sampled and scaled to that sum by `cputime_adjust` [40: `kernel/cgroup/rstat.c:713-744`] | **Answered from source (F3.2 below):** with IRQ-time accounting, hard-IRQ and softirq time is charged to no non-root cgroup; `ksoftirqd`/`kworker` time goes to the root cgroup; vhost workers go to the VMM's cgroup; guest-mode vCPU time is charged as *user* time to the VMM's cgroup. The root cgroup's `cpu.stat` is built from per-CPU `kcpustat` and **does** include IRQ and softirq as system time, so root ≠ sum of children | [35]; [40] |
| `/proc/<pid>/schedstat` | three fields: `sum_exec_runtime` (ns on CPU), `run_delay` (ns runnable-but-waiting), `pcount` (times scheduled in) [40: `fs/proc/base.c:511-523`] | doc confirmed. Present only with `CONFIG_SCHED_INFO` (else the file is absent; prints `0 0 0` if `sched_info_on()` is false). `sum_exec_runtime` accrues from `rq_clock_task` (IRQ time already subtracted when IRQ-time accounting is on) while `run_delay` uses `rq_clock` (not subtracted) [40: `kernel/sched/fair.c:1234-1250`, `kernel/sched/stats.h:253,277`, `kernel/sched/core.c:794-819`]; counters are monotonic, need baseline deltas | [30]; [40] |
| PSI (`/proc/pressure/*`, `cpu.pressure`) | stall time (`some`/`full`, `total=` in µs) per resource/cgroup. `cpu some` = runnable tasks exceed on-CPU tasks; `cpu full` = at least one runnable task and none on CPU [40: `kernel/sched/psi.c:259-263`] | CPU `full` undefined at system level: the file still prints a `full` line, but with zero averages and `total=0` [40: `kernel/sched/psi.c:1276-1289`]. `/proc/pressure/irq` exists only when IRQ-time accounting is enabled (else `-EOPNOTSUPP`, `:1254-1257`) and has only a `full` line | [25]; [40] |
| Whole-VM `/proc/stat` + `perf stat -a` | all user/kernel/IRQ/softIRQ time on all CPUs | system-wide cpu-clock is not busy CPU; idle must be excluded | [38] (spike README) |
| `memory.stat` / `memory.peak` / `pids.peak` | anon/file/kernel memory, peak memory and tasks | **Corrected:** `memory.peak` is resettable, per open fd (a write on that fd resets that fd's baseline, `mm/memcontrol.c:4255-4334`); `pids.peak` is **not** resettable (read-only, `kernel/cgroup/pids.c:402-406,353-359`). Both are absent on the root cgroup (`CFTYPE_NOT_ON_ROOT`) | [35]; [40] |

The in-repo density benchmark chose whole-VM `/proc/stat` as primary CPU "including all user/kernel/IRQ/softIRQ/vhost/module/BPF work", with generator CPU diagnostic only and physical-QEMU/cgroup/host scopes secondary and "never silently merged" [38]. That is consistent with the table's caveat column: a cgroup-scoped number can omit kernel work charged elsewhere.
**Pitfall (checked against the v7.0 source; the original wording was partly wrong and is corrected here)**. The original claim was that per-cgroup accounting under-attributes interrupt/softirq and kernel-thread work for a *kernel* arm relative to a *userspace* arm, biasing toward the kernel arm. The source shows who is charged for each kind of work, and the bias is real but not specific to the kernel arm:

| Work | Charged to | Source (`@ v7.0`) |
|---|---|---|
| Syscall-context kernel work (e.g. `connect()` and the `cgroup/connect4` program, which runs from `tcp_v4_pre_connect` inside `__inet_stream_connect` with the caller's socket) | the calling task, so the caller's cgroup, as system time | `net/ipv4/af_inet.c:679-680`, `net/ipv4/tcp_ipv4.c:206-218`; accounting via `update_se` → `cgroup_account_cputime(donor, …)`, `kernel/sched/fair.c:1232-1256` |
| Hard-IRQ and softirq time, when IRQ-time accounting is on (`CONFIG_IRQ_TIME_ACCOUNTING`, enabled for x86 at TSC init unless `no_sched_irq_time`) | **per-CPU `kcpustat` only** (`CPUTIME_IRQ`/`CPUTIME_SOFTIRQ`), subtracted from the interrupted task's runtime; **no non-root cgroup is charged** | `kernel/sched/cputime.c:57-82` (`irqtime_account_irq`, writes `kcpustat_this_cpu`, not `task_group_account_field`); `kernel/sched/core.c:794-819` (`update_rq_clock_task` removes `irq_delta` from `clock_task`); `kernel/sched/cputime.c:378-379` (comment: IRQ/softirq "do not count in task exec_runtime any more"); `arch/x86/kernel/tsc.c:1560-1561`; softirq entry/exit hooks `kernel/softirq.c:600,648`, `include/linux/vtime.h:133-142` |
| Same, when IRQ-time accounting is off | the **interrupted** task's cgroup (an arbitrary victim, not the cause), as system time | `kernel/sched/cputime.c:190-206` (`account_system_time` → `account_system_index_time` → `task_group_account_field`), `:479-504` |
| `ksoftirqd/N` time (softirqs deferred out of interrupt context) | the `ksoftirqd` task's own cgroup. `irqtime_account_irq` deliberately skips softirq run *from* `ksoftirqd` so that time stays task time. Kthreads descend from `kthreadd`, which is kept in the root cgroup. | `kernel/sched/cputime.c:72-81,399-405`; `include/linux/cgroup.h:640-657`; `kernel/softirq.c:1107` |
| `kworker` time | a kthread created with `kthread_create_on_node`, so the root cgroup (the workqueue code never attaches workers to a cgroup) | `kernel/workqueue.c:2827` (no cgroup call anywhere in the file); child inherits the creator's `css_set`, `kernel/cgroup/cgroup.c:6769-6789`; `kthreadd` is pinned to the root, `kernel/kthread.c:798` |
| `vhost` worker (e.g. `vhost-vsock`) | the **VMM's** cgroup. Default mode forks a `vhost_task` from the `VHOST_SET_OWNER` caller, which inherits the caller's `css_set`; the alternative kthread mode explicitly `cgroup_attach_task_all(owner, worker)`. The sending host process only queues work and does not run the copy. | `drivers/vhost/vhost.c:45,602,761-813,1093-1136`; `kernel/vhost_task.c:119-154`; `kernel/cgroup/cgroup.c:6769-6789` (child gets `task_css_set(current)`); `kernel/cgroup/cgroup-v1.c:66-88`; `drivers/vhost/vsock.c:277-297,702-704` |
| Guest-mode vCPU time (PF_VCPU) | the VMM vCPU thread's cgroup, as **user** time (plus `CPUTIME_GUEST` in `kcpustat`) | `kernel/sched/cputime.c:145-161,194-197,410-411` |

*Verdict.* (1) Confirmed: per-cgroup `cpu.stat` systematically misses softirq/IRQ time and `ksoftirqd`/`kworker` time for the workload cgroup. (2) Corrected: it is not specific to a kernel arm. A userspace-proxy arm adds socket hops whose RX processing also runs in softirq, so it loses attribution too; the net direction depends on how much of each arm's work runs in softirq or kthread context and is **not determinable without measuring**. The claim "biases toward the kernel arm" is therefore not supported; "makes a cgroup-scoped comparison unreliable in an unknown direction" is. (3) Corrected: vhost work is not lost to the root cgroup; it is charged to the VMM, so a cgroup scope that excludes the VMM omits it and one that includes the VMM charges it to the VMM, not to the workload that caused it. (4) The `cgroup/connect4` hook's own execution is in the caller's syscall context, so that cost is attributed to the caller's cgroup; any softirq work that follows the connect (SYN/ACK processing) is not. (5) Root `cpu.stat` and `/proc/stat` do include IRQ and softirq (`kernel/cgroup/rstat.c:669-699`), which is why the density spike's whole-VM `/proc/stat` scope is the right primary [38]. (6) Residual: whether the pinned image sets `CONFIG_IRQ_TIME_ACCOUNTING`, and whether kTLS record processing runs in process or softirq context, were not traced here; record the image's setting (`/proc/config.gz` or boot log) in the result context.
**Confidence**: High on the instrument list and on the charging rules (read in source); Medium on the net direction of bias (explicitly not determinable from source).

**F3.3 — `bpf_stats_enabled`: mechanism and per-run cost read from source; wall-time perturbation must still be measured.**
The sysctl doc says stats slow each program run slightly [5]. Reading v7.0 [40] gives the mechanism:
- **Switch.** `kernel.bpf_stats_enabled` is a `ctl_table` entry whose `.data` is the static key `bpf_stats_enabled_key` and whose handler is `bpf_stats_handler` (`kernel/bpf/syscall.c:6602-6607`, registered under `kernel` at `:6612`). The key is `DEFINE_STATIC_KEY_FALSE`, so the default is off (`kernel/bpf/core.c:3305`; `static int saved_val` starts at 0, `syscall.c:6537`). A write needs `CAP_SYS_ADMIN` and accepts only 0 or 1 (`:6543-6548`); the handler bumps or drops the key only on a 0↔1 change of its own `saved_val` (`:6553-6558`), so repeated writes of 1 do not stack.
- **Second, fd-scoped switch.** `bpf(BPF_ENABLE_STATS, BPF_STATS_RUN_TIME)` returns an anonymous-inode fd and increments the same key; closing the fd, including at process exit, decrements it (`syscall.c:5990-6040`). The key is a counter, so stats are on while the sysctl or any fd holds it. **Consequence:** reading the sysctl returns `saved_val` (`:6551`), which does not reflect fd holders; a recorded sysctl value of 0 does not prove stats were off.
- **Scope.** A static key is global to the node: it covers every BPF program, in every cgroup and network namespace. A concurrent ordinary run from another worktree that enables stats changes the cost of this run's programs, and a stats-off control arm is only off if nobody else holds the key.
- **Per-run cost.** `__bpf_prog_run` tests `static_branch_unlikely(&bpf_stats_enabled_key)` (`include/linux/filter.h:707`; a patched jump, so the off path adds no work). When on, it takes `start = sched_clock()`, runs the program, takes `duration = sched_clock() - start`, then updates the program's per-CPU `cnt` and `nsecs` (`filter.h:700-726`). On x86-64 the `u64_stats_sync` is empty, the counters are `local64_t`, and `u64_stats_update_begin_irqsave` does not disable interrupts or touch a seqcount (`include/linux/u64_stats_sync.h:64-124`, `:113`). Per run that is two `sched_clock()` calls plus two per-CPU read-modify-writes. The per-CPU `stats` block is allocated for every program at load whether or not stats are on (`kernel/bpf/core.c:150-175`).
- **Which clock.** `sched_clock()` on x86 is TSC-based: `native_sched_clock` is `rdtsc` scaled to ns (`arch/x86/kernel/tsc.c:237-244`), called through a paravirt static call with preemption disabled (`:270-275,293-300`). In a KVM guest using `kvm-clock` the call is redirected to `kvm_sched_clock_read`, which reads the pvclock page (`arch/x86/kernel/kvmclock.c:90-100`). It is not `CLOCK_MONOTONIC` and is not NTP-adjusted. The `rdtsc` cost is hardware-specific and is not derivable from source.
- **What is timed.** Only the program body (plus roughly one `sched_clock()` call's latency at each edge). The counter update falls outside `duration` but inside the caller's wall time, so wall delta includes the full accounting cost and `run_time_ns` does not. This is a reading of the code, not a measurement.
- **Readout.** `bpf_prog_info.run_time_ns` and `run_cnt` (and `recursion_misses`) are per-CPU sums taken with a retry loop (`syscall.c:2459-2484,5066-5069`; also in fdinfo, `:2501-2503`). Only the two cumulative sums exist, so the readout is a **cumulative mean-only source**, confirmed. They count only runs executed while the key was on, so use before/after deltas and hold the key on for the whole measured window.
- **cgroup `sock_addr` programs use the same wrapper.** `connect()` reaches `tcp_v4_pre_connect` (`net/ipv4/tcp_ipv4.c:206-218`, called from `net/ipv4/af_inet.c:679-680`), which expands `BPF_CGROUP_RUN_PROG_INET4_CONNECT` → `BPF_CGROUP_RUN_SA_PROG` (gated by the separate static key `cgroup_bpf_enabled`) → `__cgroup_bpf_run_filter_sock_addr` (`include/linux/bpf-cgroup.h:78,237-245,286-287`; `kernel/bpf/cgroup.c:1667-1702`). That calls `bpf_prog_run_array_cg` with `run_prog = bpf_prog_run` (`kernel/bpf/cgroup.c:61-93`, call at `:81`), and `bpf_prog_run` is `__bpf_prog_run` (`filter.h:728-731`). So each attached program accumulates its own stats. The RCU read lock, effective-array dereference, run-context set/reset and context setup in `bpf_prog_run_array_cg` and `__cgroup_bpf_run_filter_sock_addr` are outside the timed span, so `run_time_ns` understates the hook's total cost on the connect path.
- **Other program types.** Trampoline-based programs (fentry, fexit, `fmod_ret`, BPF-LSM) test the same key in their own enter/exit code instead (`kernel/bpf/trampoline.c:1122,1183`).

The existing connect-hook receipt leaves stats on for both arms and notes the wall time "includes… the `bpf_stats` accounting itself" and that the wall delta was within noise while the `run_time` delta (~117 ns) was stable [38]; the source reading confirms that statement. *Proposal*: add a stats-off wall-time control per arm so the accounting cost is separately visible, and enable stats through the fd-scoped `BPF_ENABLE_STATS` rather than the sysctl where the harness can, because it reverts when the harness process exits (no restore step to leak after a crash) and does not depend on the sysctl's global value; either way record the key's holders, not only the sysctl reading.
**Confidence**: High (kernel doc, v7.0 source and repo evidence agree). The magnitude of the wall-time cost remains unmeasured.

---

### Q4. Raw-result formats and provenance

| Tool | Raw samples? | Metadata | Notes |
|---|---|---|---|
| Google Benchmark JSON | per-benchmark entries; aggregates (mean, median, stddev, CV) when repetitions >1 | `context`: host, CPUs, MHz, scaling flag, build type, caches, load average, `json_schema_version` [7] | `UseManualTime` lets the code report its own clock-boundary time [7] |
| hyperfine `--export-json` | "all available metrics" per run, warm-up excluded; field names not documented on the README page [6] | command, runs | `--shell=none`, `--prepare`, first command is the comparison reference [6] |
| Criterion | raw samples + bootstrap estimates in its output dir | — | outliers kept [2] |
| Cloud Hypervisor metrics | mean, std_dev, min, max, status per test | `git_revision`, `git_human_readable`, `git_commit_date`, `date` [9] | no raw samples; no hardware record; regression handling "not covered" [9] |
| Bencher (BMF) | `{benchmark: {measure: {value, lower_value, upper_value}}}` — one aggregate per metric, no raw samples [21] | stored server-side | thresholds: percentage, z-score, t-test, log-normal, IQR, delta-IQR, static; `min_sample_size`, `max_sample_size`, `window` [20] |
| Chromium/catapult HistogramSet | `sampleValues` plus bins and running stats (count, max, meanlogs, mean, min, sum, variance) [31] | typed *diagnostics*; shared diagnostics by GUID; GenericSet/DateRange/RelatedNameMap; "Scalar diagnostics… cannot be safely merged" [31] | explicit unit strings with improvement direction (`smallerIsBetter`) [31] |
| HdrHistogram | lossless counts at fixed precision, mergeable [11] | — | |
| MongoDB change points | stores per-(variant, task, test, config) series, computed change points and stable-region stats (min/max/median/variance/count) in a database [4] | git hash, build | |
| Phoronix/OpenBenchmarking | **not retrieved** (docs URL 404) | — | Knowledge Gap 1 |

**Findings**
- Raw samples (or a mergeable histogram) plus a typed metadata block is the dominant shape (Google Benchmark, hyperfine, Chromium, HdrHistogram) [7][6][31][11]. Aggregate-only formats (Cloud Hypervisor, Bencher BMF) cannot be re-analysed, which conflicts with #287's "machine-readable raw results" and "distributions" criteria. **Confidence**: High (5 sources).
- Units and improvement direction are first-class in Chromium's format; Google Benchmark has a `json_schema_version` [31][7]. Both support versioned, self-describing results.
- Provenance minimum observed across tools: revision, date, host/CPU count, build type. Missing from all of them: commit *dirty* state and *methodology version*. Overdrive's own rules require "SHA, dirty state, and any relevant seed" for captures [39: verification.md]. **Confidence**: Medium (absence-of-evidence over the tools I read).
- The repo's existing spike receipts already retain `uname -r`, commit, kernel sysctl state with prior value and restore, cgroup path, run counts [38]. The density spike additionally keeps failed attempts, source-selection, cohort accounting and integrity receipts [38].

---

### Q5. One framework for tests and spikes; measurement core vs scenario

**F5.1 — In-process Rust frameworks and why they fit or do not fit.**

| Tool | Measures | Fits cross-process/kernel/VM? | Source |
|---|---|---|---|
| Criterion | wall time of an in-process closure; warm-up, bootstrap, t-test, noise threshold | No: one process, closure timing; useful for pure local functions (#287 itself says so) | [2][intake] |
| Divan | in-process; `Instant` or TSC timer; groups iterations to beat timer resolution; allocation counting; multi-thread contention | No: same limitation; its timer-resolution scaling does not apply to ms-scale single-shot events | [17] |
| Tango | paired A/B, interleaved in one process or in two threads; noise threshold | No for cross-process; the *paired interleaved* idea transfers | [18] |
| iai-callgrind/Gungraun | Valgrind instruction counts; deterministic, no wall time | No: "cannot help you much" for wall time; no kernel, no VM | [19] |
| hyperfine | wall time of external commands; warm-up, prepare, JSON export | Partly: process-level black box; no clock boundaries inside the target; used already for linker benchmark [6][38] | [6] |
| Custom harness | any boundary | Yes; cost is re-implementing analysis | [38] |

**F5.2 — Separation of measurement core from scenario is the common structure.**
Evidence by example, not a documented standard: Google Benchmark separates the benchmark function and its manual-timing hook from the runner, repetitions, aggregation and JSON output [7]; Chromium separates metric computation (histograms+diagnostics) from the result format [31]; MongoDB's pipeline is explicitly "Setup a system under test; Run a workload; Report; Decide (alert); Visualize" and the detection stage is independent of the test [4]. hyperfine's `--prepare` shows the per-run setup/teardown hook as a first-class concept [6].
**Confidence**: Medium (three consistent examples; no source states it as a general rule).

**F5.3 — Dual use (maintained lane vs throwaway probe).**
I found no tool that explicitly documents serving both "baselined CI lane" and "throwaway exploratory probe". The nearest practice: Cloud Hypervisor runs its metrics from a dev script (`dev_cli.sh tests --metrics`) with a JSON report file, no gate described [9]; the repo's own approach is a throwaway probe with retained receipts, deliberately not a lane [38][39 spike.md]. This is a genuine gap in prior art (Knowledge Gap 3) and means the dual-use design must be derived from Overdrive's own rules: probes live in `spike-scratch/` and may link crates by path; production crates must not gain probe code; benchmarks are distinct from tests and EDD expectations [39: spike.md, testing.md, verification.md].
**Confidence**: Low-Medium (absence of prior art; reasoning from repo rules).

---

### Q6. Baselines and regression gating (input only)

**F6.1 — Fixed thresholds fail across heterogeneous noise.**
MongoDB: a static 10% threshold generated many false alerts on noisy tests while missing real small changes in quiet tests; no single threshold could "both catch the change in insert_ttl and not constantly flag the map_reduce test"; the three-comparison scheme (previous run, one week ago, last release) had false positives "up to 99% as discussed in Section 5 depending on how you count"; worst-case run-to-run variation "currently less than 10%, but some sensitive tests can fluctuate as much as 20% or more" [4]. After adopting E-Divisive means, false positives "dramatically dropped" [4]. Properties they cite: no distributional assumption beyond a finite mean, recursively finds multiple change points, no training data, retroactive not predictive [4].
**Caveats from the same paper**: change points are retrospective — "the change point algorithm will never find that newest result to be a change point", with a 3-5 day delay common; points can move as data accumulates; correlated noise needs human triage; hazard level = log ratio of means before/after [4].
**Confidence**: High on the claims (primary paper, read directly pages 1-5); Medium on transfer (MongoDB runs thousands of CI results, Overdrive will have few series).

**F6.2 — Other gating designs.**
Bencher's thresholds are per metric over a sliding baseline with `min_sample_size`, `max_sample_size` or time `window`; "if this minimum is not met, the Test will not run" [20]. Google Benchmark `compare.py` is a pairwise test, not a gate [8]. Criterion/Tango use a relative noise threshold [2][18]. Overdrive's Tier-4 `xdp-perf` gate is a fixed fraction (5% pps / 10% mean latency) over a one-file baseline, with comments explaining the mean-only limitation and that deltas, not absolute numbers, gate [in-repo 38; testing.md].
**Disagreement/assessment**: relative-delta-over-baseline is the common denominator; absolute thresholds are rejected by Overdrive's own testing rule ("Never gate on absolute numbers") and are not recommended by any retrieved source for variable hardware. The choice between fixed fraction and change-point detection depends on observed repeatability, which #287 says must be demonstrated first.
**Confidence**: Medium-High.

**F6.3 — Baseline promotion.** No retrieved source documents a baseline *promotion policy* in detail (Bencher's windowing is the nearest) [20]. Knowledge Gap 5.

---

### Q7. Load generators and measurement tools

| Tool | Use in catalogue | Coordinated omission / loop model (as documented) | Notes | Source |
|---|---|---|---|---|
| wrk2 | HTTP-ish request load; fixed-rate | Corrects CO via scheduled send time; HdrHistogram | ±1 ms accuracy, ≥10-20 s runs, experimental | [1] |
| dnsperf | DNS latency/QPS | "self-paces"; supports per-query latency reporting; whether a fixed-QPS option exists is not stated on the page | repo archived, moved to codeberg; each client needs two cores | [15] |
| resperf | DNS capacity | ramps query rate while monitoring response rate | models caching resolver | [15] |
| flamethrower | DNS (UDP/TCP/DoT/DoH) | open by default (as fast as possible), `-Q` caps QPS, `--qps-flow` changes rate over time; JSON per-sender metrics (counts, timeouts, min/max/avg latency) | single-threaded; one process per CPU; no percentiles listed | [16] |
| neper | TCP `tcp_rr`, `tcp_crr`, `tcp_stream`; multi-thread/flow with epoll; `--percentiles` | closed-loop per flow (my inference; page does not say); percentiles only in `samples.csv` | per-sample CSV | [14] |
| netperf `TCP_RR`/`TCP_CRR` | latency / connection rate | closed-loop request-response | used by Cilium | [26] |
| iperf3 | throughput | `-O` omit slow-start seconds; `-A` affinity; `-J` JSON; bandwidth limit applied per stream | schema not documented on page | [33] |
| h2load | HTTP/2 | `--duration`, `--warm-up-time`, `-c`, `-m`; reports min/max/mean/sd, **no percentiles** on the how-to page; open/closed model not stated | | [34] |
| `perf stat -r N` | process-level CPU counters | — | LLVM example | [24] |
| Firecracker/CH boot/restore timing | VM lifecycle | CH: 10 iterations, mean/std-dev, no start/end event definition on the page; Firecracker: console logging and PIT noise affect restore time | both lack a documented *stage* breakdown | [9][10] |

**Findings**
- Of the tools above, only wrk2 explicitly documents CO correction [1]; for DNS tools the retrieved pages do not say, so they must be treated as *unverified for CO* and tested (e.g. inject a server stall and compare uncorrected vs scheduled-time latency, which is exactly the wrk2 README's own validation example [1]). **Confidence**: High for wrk2, Low for others.
- VM boot methodology: neither Firecracker nor Cloud Hypervisor documents boot-time start/end markers on the pages I could retrieve [9][10]. vHive-style work is cited in search results as breaking restore latency into state restore, network setup and function processing and measuring both isolated boot and end-to-end user-visible latency [38b]; practitioner posts split restore into process start, mmap, device-state restore, vsock reconnect [38b]. The key discipline is a consistent definition of "ready" (e.g. guest agent responds over vsock) [38b]. **Confidence**: Low-Medium (secondary/search summaries; vendor figures not comparable). Overdrive's own #287 lifecycle stages are *more* granular than any retrieved prior art.
- Connection-hook cost and density sweeps are not covered by established external tools; the in-repo probes are the prior art [38].

---

## In-repo prior art: what already exists and where it differs

| Artifact | Methodology it already uses | Difference from a shared framework |
|---|---|---|
| `spike-scratch/pig-connect-cost/` + `findings-connect-hook-cost.md` | 2 arms, 30 runs/arm, A/B order alternated by run parity, fresh `EbpfDataplane` per run, 500 warm-up + 5,000 timed connects, `bpf_stats_enabled=1` set and restored, dedicated cgroup, `run_time/run_count` and wall time, raw CSV in the findings, explicit "not measured" list; no CI, only median/mean/min/max/stdev [38] | alternation is A,B / B,A, not randomised; no bootstrap CI or test; stats-on for both arms; bespoke recorder |
| `spike-scratch/netns-density-295-kernel-vs-aya-benchmark/` | populations 1/64/1024/4096/16384; paced vs unrestricted closed loop; open-loop UDP send pacing "independent of reply completion"; whole-VM `/proc/stat` CPU; idle CPU by stat/sleep/stat; separate memory kinds; immutable increments; failed attempts indexed; gzip + SHA-256 evidence; selection/exclusion recorded; byte-identical captures [38] | very rich, Python analysis scripts per spike; no shared schema; the density README itself notes pilot and failed windows excluded explicitly |
| `spike-scratch/boot-cost/` | instrumented guest-init variants A/A2/B/R, control R is the unmodified product, records `uname -r`, `systemd-detect-virt`, CH version and sha256, object hashes; run log retained; an INVALID run kept with reason in its filename [38] | single-purpose driver; N=40, WARM=3 parameters |
| `scripts/benchmark-linker/` | hyperfine `--warmup 2 --runs 5`; one run only for clean builds; records host, toolchain, versions [38] | build-time; n=1 for the expensive workload, which this research would rate Low-evidence |
| Tier-4 `xdp-perf` + `perf-baseline/main/xdp-perf/*.txt` | `BPF_PROG_TEST_RUN repeat=1000000`; reads `run_time_ns/run_cnt` (mean only); gate: pps -5% or mean +10%; records kernel, tool, commit, date [38] | a *gate*, not a benchmark program; mean-only; #287 and #157 say it must not be absorbed |

The repo's own statements that the report is "report-only… not an EDD expectation, not a test, and not a gate" [38] already encode the classification the program needs.

---

## Recommendations for Overdrive (proposals for DISCUSS/DESIGN; the user decides)

Mapped to the intake catalogue. Each states the evidence behind it.

**R1 — Separate a measurement core from scenarios.** Core responsibilities: clock-boundary definition and stamping, sampler/schedule (run order, interleaving, warm-up, cold/warm preparation), recorder (raw samples + typed metadata), analysis (bootstrap CIs, nonparametric A/B), report. A *scenario* supplies: boundary, workload, arm set, environment profile, fixture setup/teardown per run. Evidence: F5.2, F5.1. This also serves dual use (R8). The core's location (crate vs `spike-scratch/` library) is a DESIGN decision (Open Question 4); `xtask` cannot depend on `overdrive-*` crates [39: development.md].

**R2 — Result record = raw samples + context + methodology id.** One record per (scenario, run): unit-bearing samples with improvement direction (Chromium) [31]; context block like Google Benchmark's plus: commit SHA, **dirty flag**, `uname -r`, firmware/microcode, CPU model, SMT/turbo/governor state, `bpf_stats_enabled` prior/applied value, tool/CH/guest-kernel versions, image digests, cold/warm profile, run id, admission to the metal runner; and a **methodology version string** (no retrieved tool has one: Knowledge Gap 2). Failures are records, not omissions (matches the density spike's cohort accounting [38]). Never aggregate-only (rejects the BMF/Cloud Hypervisor shape for storage; BMF can be a *derived export* if gating is later adopted) [21][9].

**R3 — Arm model for userspace-vs-kernel (and any A/B).** Define `Boundary` + `Workload` once; an `Arm` varies only the mechanism (setup/teardown/attach). Scheduler: block-randomised arm order per replicate (generalise the spike's A,B/B,A alternation, F1.7), a fresh fixture per run (the spike's practice), and a mandatory **A/A control** (same arm twice) to measure the noise floor and a **null/plaintext baseline arm** where the boundary has one (#287 family 3 mandates a defined plaintext baseline). Analysis per metric: median and quantile differences or ratios with bootstrap CI plus Mann–Whitney p-value, with at least 9 replicates, noting that significance is not proof (F1.5, [8]). Add a stats-off arm/control for BPF-hook benchmarks (F3.3), recognising that the stats key is node-global, so the control is only valid if no other run holds it.

**R4 — CPU/memory attribution plan per family.** Record several instruments rather than one: whole-host or whole-VM `/proc/stat` (primary, as in the density spike), per-cgroup `cpu.stat`/`memory.stat`/`memory.peak`/`pids.peak`, BPF `run_time_ns/run_cnt` (mean only), `/proc/<pid>/schedstat`, PSI for contention [35][30][25][5][38][40]. Label which one is primary and never silently merge scopes (the density README's rule). Source-grounded constraints on each (F3.2, F3.3): `/proc/stat` and the root cgroup include IRQ and softirq time, a non-root cgroup's `cpu.stat` does not (with IRQ-time accounting) and never includes `ksoftirqd`/`kworker`; the VMM's cgroup carries its vhost workers and guest-mode vCPU time (as user time); `schedstat` needs `CONFIG_SCHED_INFO`; `memory.peak` is resettable per open fd (a write on that fd resets its baseline, `mm/memcontrol.c:4299-4334`) while `pids.peak` has **no write handler and cannot be reset** (`kernel/cgroup/pids.c:402-406`), and neither file exists on the root cgroup (`CFTYPE_NOT_ON_ROOT`; `memcontrol.c:4631-4637`). For #303's "forward-path userspace work versus return/deliver splice work", measure the proxy *process* user/system time separately from kernel-side kTLS/softirq time; the softirq part will be missing from the proxy's cgroup, so take it from the whole-host scope and do not assume which arm the omission favours (F3.2 verdict).

**R5 — Load model per family.**
- *VM lifecycle (#283)*: stage timestamps from one monotonic clock domain per stage pair, each stage a separate series; boundaries that cross the host/guest line are stamped on the host only, and guest-internal stages are reported by the guest as durations, never as timestamps compared with host ones (F1.9); failure/grace paths as a separate series; concurrency N as an explicit profile; arrival of launch requests open-loop at a stated rate for concurrency profiles. Cold = image cache dropped via a recorded preparation step; warm = pre-populated. Define "ready" per stage (the issue's non-conflation requirement). Evidence: F1.3, Q7 VM row.
- *DNS*: client-observed with open-loop scheduled-send latency (wrk2 principle) plus resolver-internal timing as a separate series; named query shapes and cache hit/miss as separate profiles. Because dnsperf/flamethrower CO behaviour is undocumented in retrieved pages, validate any chosen generator with a deliberate server stall (F-Q7). Latency vs offered load needs open-loop; a closed-loop run answers a different question (per-connection latency at a fixed concurrency).
- *mTLS/kTLS (family 3)*: handshake/first-byte via `tcp_crr`-shaped runs, throughput via `tcp_stream`-shaped runs, tail latency via `tcp_rr`-shaped runs, each in both directions; CPU, memory, socket count recorded alongside. Netflix/Cilium/Intel-style reporting: throughput *and* CPU on the same row [27][26].
- *Connect-hook, density, pump scaling, vsock UDP, vDPA, snapshot/restore, Firecracker-vs-CH*: arms/profiles as in the intake; reuse the spikes' populations (1/64/1,024/4,096/16,384) and "paced vs unrestricted" distinction [38].

**R6 — Statistics defaults (to be ratified in the methodology doc).** Keep every sample; flag, never delete, outliers [2][6]; report p50/p90/p95/p99 only when sample count supports them — *interpretation*: with n replicates a q-quantile is meaningful only if roughly n·(1−q) ≥ a handful of samples (30 runs make p99 equal to the maximum, so p99 of per-run values is not meaningful, whereas p99 over thousands of within-run requests is, subject to within-run correlation, F1.1); bootstrap CIs; ≥9 replicates per arm for any A/B claim [8]; decide replicate counts per level using the Kalibera-Jones approach after a pilot measures between-run vs within-run variance [3]. The spike used 30 runs/arm and Cloud Hypervisor uses 5-10 iterations [38][9]; neither justification is documented.

**R7 — Host eligibility and noise control as a recorded gate, not a silent assumption.** Refuse-or-flag checks recorded in the result: governor, `no_turbo` (`/sys/devices/system/cpu/intel_pstate/no_turbo`; absent if `intel_pstate` is not the active driver), SMT state (`/sys/devices/system/cpu/smt/control`), ASLR, `perf_event_paranoid` (default 2; `perf stat -a` needs ≤ 0 or `CAP_PERFMON`), swap, BPF stats state (the sysctl value *and* any `BPF_ENABLE_STATS` holders, since the sysctl read ignores fd holders; F3.3), `CONFIG_IRQ_TIME_ACCOUNTING` and `CONFIG_SCHED_INFO` of the running kernel (F3.2), host and guest clocksource (F1.9), `systemd-detect-virt == none` (already required by the metal runner's preflight [39]), host kernel version (7.0 pinned [39]), console log level (Firecracker's finding that logging inflated restore time [10]). Run a short **canary** (fixed workload, e.g. a pure CPU loop and a loopback connect loop) before each run set and record it, per MongoDB's canary practice [4]. Whether to *change* host settings (turbo off, SMT off) or only *record* them is a decision for the user (Open Question 2). Include active-benchmarking observation (vmstat/perf/PSI sampling during the run) as an optional recorded side-channel [37].

**R8 — Tests-and-spikes dual use.** Proposal: one core and one record schema; two *entry shapes* that differ in lifecycle, not in measurement:
- *Maintained lane*: scenario code under the framework's owning location, rerunnable, with a versioned methodology id, a declared environment profile, and (later, separately) an optional baseline. Not a correctness test: it asserts the harness's own validity (all arms ran, sample counts reached, controls consistent) but never a performance threshold, preserving the testing.md boundary.
- *Spike probe*: scenario code lives in `spike-scratch/<increment>/` with its own `Cargo.toml`, linking the measurement core **by path** (permitted by spike.md), producing the *same* record schema into a retained evidence directory; the probe's receipt is a human report pointing at the raw records. Throwaway status is unchanged: the probe is deleted when the implementation lands, but its raw records remain comparable because the schema is shared. Evidence from [39: spike.md] (sources/captures committed; probes may depend on workspace crates by path; build output ignored).
Prior art does not exist for this dual use (F5.3), so this is Overdrive-derived reasoning, not a pattern to copy.

**R9 — Defer gating; if later chosen, evidence points to relative-delta over baseline with a change-point or per-metric statistical rule rather than a fixed fraction for noisy end-to-end series, and to a fixed fraction only where the measurement is deterministic like `BPF_PROG_TEST_RUN`** (F6.1, F6.2; #287 and #157 keep the current Tier-4 gate out of scope). Absolute thresholds are rejected by the repo's testing rule. A baseline needs a promotion rule (who, how many stable runs, which environment profile); none was found in external sources (Knowledge Gap 5).

**R10 — Keep existing assets distinct.** `xdp-perf`/`verifier-regress` remain gates; `benchmark-linker` is build-time and not product; the framework may *import* hyperfine for the build-time family but is not required to (#146 is a catalogue entry, not a design driver).

---

## Source Analysis

Reputation tiers per `nw-source-verification`: High = academic/official/standards/kernel.org; Medium-High = established industry engineering sources; Medium = vendor blogs/slides, community. In-repo documents are treated as primary for repo facts only.

| # | Source | Domain | Reputation | Type | Cross-verified |
|---|---|---|---|---|---|
| 1 | wrk2 README | github.com/giltene | Medium-High | tool docs | Y ([11],[29]) |
| 2 | Criterion.rs book, Analysis | bheisler.github.io | Medium-High | tool docs | Y ([6],[18]) |
| 3 | Kalibera & Jones, ISMM 2013 | kar.kent.ac.uk / ACM | High | academic | Y ([22],[28]); abstract only |
| 4 | Daly et al., ICPE 2020, arXiv 2003.00584 | arxiv.org | High | academic/industry | partial (single primary; read pages 1-5) |
| 5 | Linux admin-guide sysctl/kernel | docs.kernel.org | High | official | single authoritative |
| 6 | hyperfine README | github.com/sharkdp | Medium-High | tool docs | Y |
| 7 | Google Benchmark user guide | google.github.io | Medium-High | tool docs | Y |
| 8 | Google Benchmark tools.md | github.com/google | Medium-High | tool docs | Y ([2]) |
| 9 | Cloud Hypervisor performance_metrics.md | github.com/cloud-hypervisor | High (project docs) | tool docs | single |
| 10 | Firecracker prod-host-setup.md | github.com/firecracker-microvm | High (project docs) | tool docs | single |
| 11 | HdrHistogram README | github.com/HdrHistogram | Medium-High | tool docs | Y ([1]) |
| 12 | Per-CPU kthreads | docs.kernel.org | High | official | single authoritative |
| 13 | intel_pstate | docs.kernel.org | High | official | Y ([24]) |
| 14 | neper README | github.com/google | Medium-High | tool docs | single |
| 15 | dnsperf README | github.com/DNS-OARC | Medium-High | tool docs | single (partial) |
| 16 | flamethrower README | github.com/DNS-OARC | Medium-High | tool docs | single |
| 17 | Divan intro | nikolaivazquez.com | Medium | author blog | single |
| 18 | Tango README | github.com/bazhenov | Medium | tool docs | single |
| 19 | iai-callgrind/Gungraun README | github.com/iai-callgrind | Medium-High | tool docs | single |
| 20 | Bencher thresholds | bencher.dev | Medium (vendor) | tool docs | single |
| 21 | Bencher Metric Format | bencher.dev | Medium (vendor) | tool docs | single |
| 22 | Georges et al., OOPSLA 2007 | dri.es / ACM | High | academic | Y ([3]); abstract-level |
| 23 | Mytkowicz et al., ASPLOS 2009 | sape.inf.usi.ch / ACM | High | academic | Y ([24]) |
| 24 | LLVM Benchmarking | llvm.org | High (OSS foundation) | project docs | Y ([13],[12]) |
| 25 | PSI | docs.kernel.org | High | official | single authoritative |
| 26 | Cilium CNI benchmark docs and blog | docs.cilium.io, cilium.io | Medium-High | vendor primary | partial (via search summary) |
| 27 | Netflix EuroBSDCon 2022 slides; Intel netdev 0x14 slides; Facebook ktls netdev 1.2 | people.freebsd.org, netdevconf.info | Medium-High | industry primary | partial (search summaries) |
| 28 | Hoefler & Belli, SC15 | evaluate.inf.usi.ch / ACM | High | academic | abstract-level |
| 29 | Tene, "How NOT to Measure Latency" slides; mailing-list post | qconsf.com, groups.google.com | Medium-High | industry talk | Y ([1]) |
| 30 | sched-stats | docs.kernel.org | High | official | single authoritative |
| 31 | catapult HistogramSet JSON format | chromium.googlesource.com | Medium-High | project docs | single |
| 32 | XDP paper, CoNEXT 2018 | research.redhat.com, ACM | High | academic | partial (search summary) |
| 33 | iperf3 doc | iperf.fr | Medium | tool docs (third-party mirror) | single |
| 34 | h2load how-to | nghttp2.org | Medium-High | tool docs | single |
| 35 | cgroup v2 | docs.kernel.org | High | official | single authoritative |
| 36 | IRQ affinity | docs.kernel.org | High | official | single authoritative |
| 37 | Gregg, Active Benchmarking | brendangregg.com | Medium-High | practitioner | fetch failed; search summary only |
| 38 | In-repo artifacts: findings-connect-hook-cost.md, spike READMEs and scripts, `perf-baseline/main/xdp-perf`, `scripts/benchmark-linker/RESULTS.md`, `xdp_perf_gate.rs` | repo | primary (repo facts) | internal | n/a |
| 38b | vHive / snapshot-restore practitioner write-ups (via search summary) | arxiv.org/abs/2101.09355 and blogs | Medium / Low | mixed | Low |
| 39 | In-repo rules: testing.md, spike.md, verification.md, development.md, CLAUDE.md | repo | primary (repo rules) | internal | n/a |
| 40 | Linux v7.0 source (`vendors/linux`, commit 028ef9c96) | vendored submodule | High (authoritative; the kernel Overdrive runs) | primary source code | Y for `bpf_stats_enabled`, cgroup charging and `no_turbo`/SMT/`perf_event_paranoid` (agrees with [5][13][35]); single-source otherwise |

Reputation (external plus [40], 36 sources): High 15 (42%), Medium-High 17 (47%), Medium 4 (11%). Average ≈ 0.90. Commercial-interest bias noted for [20][21] (Bencher), [26] (Cilium), [27] (Netflix/Intel/Facebook), and [9][10] (project docs, no independent benchmark). No sources excluded.

## Knowledge Gaps

1. **Phoronix Test Suite / OpenBenchmarking result schema and dynamic run-count rules.** Attempted: PTS documentation URL (404). Not otherwise searched. Recommend a targeted fetch of the PTS GitHub docs if the DESIGN wave wants that prior art.
2. **Versioned methodology identifiers inside result files.** None of the formats read (Google Benchmark, hyperfine, Cloud Hypervisor, BMF, HistogramSet) carries a methodology version; only `json_schema_version` for the file schema [7]. Overdrive must define this itself.
3. **Dual-use (maintained lane + throwaway probe) frameworks.** No documented prior art found. Treated as Overdrive-derived design (R8).
4. **Benchmarking inside nested virtualization / guest groups.** No authoritative guidance found; repo rules treat only native metal as signal [39].
5. **Baseline promotion policy.** Only Bencher's windowing and MongoDB's retrospective change points were found; no promotion/ratification procedure documented.
6. **Kernel-source grounding — resolved, with residuals.** Resolved from `vendors/linux` at v7.0 [40]: the `bpf_stats_enabled` mechanism and per-run cost (F3.3), charging of softirq/kthread/vhost/guest-mode work (F3.2), clock definitions and the KVM-to-guest clock relation (F1.9), and the existence and location of the host controls (Q2 source check). **Not settled from source:** (a) the wall-time magnitude of the stats cost (needs measurement; `rdtsc` cost is hardware-specific); (b) whether the pinned image enables `CONFIG_IRQ_TIME_ACCOUNTING` and `CONFIG_SCHED_INFO` (image config is not in this checkout; read the running kernel's config); (c) whether kTLS record processing runs in process or softirq context, which decides how much of the kernel-mTLS arm falls outside a cgroup scope (not traced); (d) the net direction of the cgroup-attribution bias (depends on measured softirq share per arm); (e) the `TSC`-vs-`kvm-clock` choice inside the Overdrive guest (depends on Cloud Hypervisor's exposed CPUID; `vendors/cloud-hypervisor` is not checked out here).
7. **Guest clock alignment across the host/guest boundary** for VM-lifecycle stages (guest receipt, guest READY): the source shows the *relation* (guest kvm-clock = host `CLOCK_MONOTONIC_RAW` rate, zeroed at VM creation; guest `CLOCK_MONOTONIC` zeroed again at guest boot; F1.9) but gives no error budget. The residual error of any offset estimate over vsock (round-trip asymmetry, vhost worker queueing) is a measurement question, not a source one. F1.9's proposal avoids the need for an offset.
8. **Official Firecracker/CH boot-time measurement definitions** (what marks start and end) were not retrievable; CH's metrics page does not say [9].
9. **Hoefler & Belli, Georges et al. and Kalibera & Jones bodies** were read only as abstracts/search records; their specific recommendations (e.g. nonparametric CIs, exact repetition formula) are not verified beyond the abstracts.
10. **Gregg's active-benchmarking page** could not be fetched (connection refused); claim F1.8 rests on a search-result summary.

## Conflicting Information

**Conflict 1 — Parametric vs nonparametric comparison.** Position A: Criterion compares via t-test on bootstrap distributions [2]; Georges et al. advocate Student-t confidence intervals [22]. Position B: Google Benchmark and Chromium use Mann-Whitney U and keep raw samples for it [8][31]. Assessment: latency data is skewed and multimodal (Tene's talk emphasises "strongly multi-modal nature of latency" via search description [29]); nonparametric/quantile-based comparison is better matched, but both sides state their method's assumptions; choice should be validated on Overdrive's own pilot data.

**Conflict 2 — ASLR and setup randomisation.** LLVM says disable ASLR to cut variance [24]; Mytkowicz argues for randomising setup to expose and average bias [23]. Assessment: compatible only if the design separates a "low-noise A/B" mode from a "bias-exposing" mode; the repo has no stated position.

**Conflict 3 — Fixed thresholds.** MongoDB found fixed thresholds inadequate [4]; Tango/Criterion and the repo's Tier-4 gate use fixed relative noise thresholds or gates [2][18][38]. Assessment: not a contradiction of fact — different noise regimes (deterministic `BPF_PROG_TEST_RUN` vs thousands of noisy CI series) — but it determines how gating should differ per family.

**Conflict 4 — Sources of CPU attribution (not a source conflict, a design tension).** The density spike chose whole-VM `/proc/stat` over cgroup scopes [38] while cgroup `cpu.stat` is the standard per-workload kernel counter [35]. See F3.2.

## Recommendations for Further Research

1. Done for `bpf_stats_enabled`, cgroup CPU charging, vhost worker accounting and clock sources (F3.2, F3.3, F1.9). Still open from source: trace whether kTLS record encrypt/decrypt (`net/tls/`) runs in process or softirq context, since that sets how much of the kernel-mTLS arm a cgroup scope can see; and read the Cloud Hypervisor fork's CPUID/clock setup once `vendors/cloud-hypervisor` is checked out. Cite `path:line @ v7.0` in DESIGN.
2. Run a *pilot* (not a benchmark result) on the metal host to measure between-run vs within-run variance per family, so replicate counts follow F1.1/R6 rather than convention.
3. Verify the CO behaviour of the chosen DNS and TCP generators empirically with a deliberate stall (wrk2's own validation shape [1]).
4. Retrieve primary text of Kalibera & Jones, Hoefler & Belli, and Gregg's page before they are cited in an ADR.
5. Search for guest/host clock-alignment practice (kvmclock, vsock timestamping) for the VM-lifecycle clock boundaries.

## Open Questions Requiring a User Decision

1. **Exclusive host for benchmarks?** `testing.md` says no *test* runs `--exclusive` and ordinary runs from other worktrees execute concurrently with namespace (not CPU) isolation; benchmarks are explicitly not tests, and concurrent runs are an uncontrolled noise source (Q2). Should benchmark runs get exclusive or CPU-partitioned admission, or must they stay in the ordinary grammar (which refuses scripts/non-`cargo` entry points)?
2. **Control or only record the host state?** May the program change governor/turbo/SMT/ASLR/`bpf_stats_enabled` on the metal host (and restore them), or only refuse/flag ineligible hosts? Source facts that bear on this: `bpf_stats_enabled` is node-global and has a crash-safe fd-scoped alternative (F3.3); `smt/control` `forceoff` cannot be undone at runtime (`kernel/cpu.c:2904`).
3. **May a benchmark spawn the `overdrive` binary?** `testing.md`: tests never spawn it and only EDD captures run the CLI; benchmarks are a separate class but the rule is silent on them. In-process composition vs a black-box binary changes what clock boundaries are measurable (and #287 mentions "through `serve` + `deploy`" for density).
4. **Where does the measurement core live?** A new `adapter-host` crate, `overdrive-testing`, or a `spike-scratch` library (never `crates/` for probe code); xtask cannot depend on `overdrive-*` crates. Also the crate class and whether `xtask` merely fronts it via a cargo alias.
5. **Statistical defaults to ratify:** replicate counts (R6), A/B test and effect-size convention, whether p99 is reported per-run or pooled, and the A/A control and canary requirements (R3, R7) — these add validation cost; per `design.md` § "No unrequested scope" they are proposals needing your approval with cost/alternatives.
6. **Methodology versioning scheme:** how methodology ids and result-schema versions are named and bumped, given no external precedent (Gap 2), and consistent with `domain-naming.md` (no roadmap step IDs in names).
7. **Scope of nested-virtualization benchmarking:** may guest-group placements (nested CH guests) carry benchmark results, or only native-host measurements?
8. **Which catalogue entries are in the first slice?** The intake lists 12 candidates; the vertical-slice rule in `CLAUDE.md` implies a first thin, production-drivable loop. This research gives no ranking; user input needed.

## Research Metadata

Duration: ~45 tool calls for the web pass plus a source-reading pass over `vendors/linux` | Examined: ~40 pages/papers and ~35 kernel source files | Cited: 40 | Cross-refs: ≈ 20 claims with ≥2 sources | Confidence mix (findings): High ≈ 45%, Medium ≈ 40%, Low ≈ 15% | Output: `docs/research/testing/native-benchmark-program-best-practices-research.md`

Tool failures: Gregg `activebenchmarking.html` fetch (ECONNREFUSED); Phoronix Test Suite documentation (HTTP 404); MongoDB arXiv PDF text extraction failed in WebFetch, then read directly via PDF pages 1-5 (pages 6-9, including false-positive tables in section 5, were not read; the "99%" figure comes from the quoted text on page 2); `vendors/linux` was absent during the first pass (kernel claims then rested on `docs.kernel.org`) and was read in a second pass (Linux v7.0, commit 028ef9c96); `vendors/cloud-hypervisor` is not checked out (no Cloud Hypervisor source read); Kalibera-Jones, Georges, Mytkowicz, Hoefler-Belli read through search records/abstracts only.

## Full Citations

[1] Gil Tene. "wrk2 README". GitHub. https://github.com/giltene/wrk2. Accessed 2026-11-13.
[2] Brook Heisler. "Criterion.rs Book: Analysis Process". https://bheisler.github.io/criterion.rs/book/analysis.html. Accessed 2026-11-13.
[3] Tomas Kalibera, Richard Jones. "Rigorous Benchmarking in Reasonable Time". ISMM 2013, DOI 10.1145/2464157.2464160. https://kar.kent.ac.uk/id/document/3127886 (also https://evaluate.inf.usi.ch/node/364). Accessed 2026-11-13 (abstract/record only).
[4] David Daly, William Brown, Henrik Ingo, Jim O'Leary, David Bradford. "The Use of Change Point Detection to Identify Software Performance Regressions in a Continuous Integration System". ICPE 2020. arXiv:2003.00584. https://arxiv.org/pdf/2003.00584. Accessed 2026-11-13.
[5] Linux kernel documentation. "Documentation for /proc/sys/kernel/ (bpf_stats_enabled, perf_event_paranoid)". https://docs.kernel.org/admin-guide/sysctl/kernel.html. Accessed 2026-11-13.
[6] David Peter (sharkdp). "hyperfine". https://github.com/sharkdp/hyperfine. Accessed 2026-11-13.
[7] Google. "Benchmark User Guide". https://google.github.io/benchmark/user_guide.html. Accessed 2026-11-13.
[8] Google. "Benchmark: Additional Tooling (compare.py)". https://github.com/google/benchmark/blob/main/docs/tools.md. Accessed 2026-11-13.
[9] Cloud Hypervisor project. "Performance Metrics". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/performance_metrics.md. Accessed 2026-11-13.
[10] Firecracker project. "Production Host Setup Recommendations". https://github.com/firecracker-microvm/firecracker/blob/main/docs/prod-host-setup.md. Accessed 2026-11-13.
[11] HdrHistogram. "HdrHistogram". https://github.com/HdrHistogram/HdrHistogram. Accessed 2026-11-13.
[12] Linux kernel documentation. "Reducing OS jitter due to per-cpu kthreads". https://docs.kernel.org/admin-guide/kernel-per-CPU-kthreads.html. Accessed 2026-11-13.
[13] Linux kernel documentation. "intel_pstate CPU Performance Scaling Driver". https://docs.kernel.org/admin-guide/pm/intel_pstate.html. Accessed 2026-11-13.
[14] Google. "neper". https://github.com/google/neper. Accessed 2026-11-13.
[15] DNS-OARC. "dnsperf". https://github.com/DNS-OARC/dnsperf. Accessed 2026-11-13.
[16] DNS-OARC. "flamethrower". https://github.com/DNS-OARC/flamethrower. Accessed 2026-11-13.
[17] Nikolai Vazquez. "Divan: Comfy Rust benchmarking". https://nikolaivazquez.com/blog/divan/. Accessed 2026-11-13.
[18] Denis Bazhenov. "tango-bench". https://github.com/bazhenov/tango. Accessed 2026-11-13.
[19] iai-callgrind / Gungraun. https://github.com/iai-callgrind/iai-callgrind. Accessed 2026-11-13.
[20] Bencher. "Thresholds & Alerts". https://bencher.dev/docs/explanation/thresholds/. Accessed 2026-11-13.
[21] Bencher. "Bencher Metric Format". https://bencher.dev/docs/reference/bencher-metric-format/. Accessed 2026-11-13.
[22] Andy Georges, Dries Buytaert, Lieven Eeckhout. "Statistically Rigorous Java Performance Evaluation". OOPSLA 2007. https://dri.es/files/oopsla07-georges.pdf (record: https://evaluate.inf.usi.ch/node/2). Accessed 2026-11-13 (abstract-level).
[23] Todd Mytkowicz, Amer Diwan, Matthias Hauswirth, Peter Sweeney. "Producing Wrong Data Without Doing Anything Obviously Wrong!". ASPLOS 2009, DOI 10.1145/1508244.1508275. https://sape.inf.usi.ch/publications/asplos09. Accessed 2026-11-13.
[24] LLVM Project. "Benchmarking tips". https://llvm.org/docs/Benchmarking.html. Accessed 2026-11-13.
[25] Linux kernel documentation. "PSI - Pressure Stall Information". https://docs.kernel.org/accounting/psi.html. Accessed 2026-11-13.
[26] Cilium. "CNI Performance Benchmark" docs and "CNI Benchmark: Understanding Cilium Network Performance" blog. https://docs.cilium.io/en/latest/operations/performance/benchmark/ ; https://cilium.io/blog/2021/05/11/cni-benchmark. Accessed 2026-11-13 (via search summary).
[27] Drew Gallatin (Netflix). EuroBSDCon 2022 slides. https://people.freebsd.org/~gallatin/talks/euro2022.pdf ; Intel. "TLS Perf Characterization", Netdev 0x14. https://netdevconf.info/0x14/pub/slides/25/TLS%20Perf%20Characterization%20slides%20-%20Netdev%200x14%20v2.pdf ; Dave Watson (Facebook). "kTLS", Netdev 1.2. https://netdevconf.info/1.2/papers/ktls.pdf. Accessed 2026-11-13 (via search summary).
[28] Torsten Hoefler, Roberto Belli. "Scientific Benchmarking of Parallel Computing Systems: Twelve ways to tell the masses when reporting performance results". SC15. https://evaluate.inf.usi.ch/node/366. Accessed 2026-11-13 (abstract-level).
[29] Gil Tene. "How NOT to Measure Latency". QCon SF 2012 slides. https://qconsf.com/sf2012/dl/qcon-sanfran-2012/slides/GilTene_HowNotToMeasureLatency.pdf ; mechanical-sympathy post https://groups.google.com/d/msg/mechanical-sympathy/icNZJejUHfE/BfDekfBEs_sJ. Accessed 2026-11-13 (via search summary).
[30] Linux kernel documentation. "Scheduler Statistics". https://docs.kernel.org/scheduler/sched-stats.html. Accessed 2026-11-13.
[31] Chromium catapult. "HistogramSet JSON Format". https://chromium.googlesource.com/catapult/+/HEAD/docs/histogram-set-json-format.md. Accessed 2026-11-13.
[32] Toke Høiland-Jørgensen et al. "The eXpress Data Path: Fast Programmable Packet Processing in the Operating System Kernel". CoNEXT 2018, DOI 10.1145/3281411.3281443. https://research.redhat.com/?p=22875. Accessed 2026-11-13 (via search summary).
[33] iPerf. "iPerf3 documentation". https://iperf.fr/iperf-doc.php. Accessed 2026-11-13.
[34] nghttp2. "h2load HOW-TO". https://nghttp2.org/documentation/h2load-howto.html. Accessed 2026-11-13.
[35] Linux kernel documentation. "Control Group v2". https://docs.kernel.org/admin-guide/cgroup-v2.html. Accessed 2026-11-13 (first 100,000 characters read).
[36] Linux kernel documentation. "SMP IRQ affinity". https://docs.kernel.org/core-api/irq/irq-affinity.html. Accessed 2026-11-13.
[37] Brendan Gregg. "Active Benchmarking". https://www.brendangregg.com/activebenchmarking.html. Accessed 2026-11-13 (fetch failed; search summary only).
[38] In-repo: `docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md`; `spike-scratch/pig-connect-cost/`; `spike-scratch/netns-density-295-kernel-vs-aya-benchmark/README.md`; `spike-scratch/boot-cost/run.sh`; `scripts/benchmark-linker/RESULTS.md`; `perf-baseline/main/xdp-perf/xdp_service_map_lookup.txt`; `crates/overdrive-dataplane/bin/xdp_perf_gate.rs`.
[38b] vHive (arXiv:2101.09355) and snapshot-restore practitioner/vendor write-ups surfaced by search (Princeton SnapFaaS poster, SYSTOR'22, Dedalus Labs). https://alphaxiv.org/abs/2101.09355 . Accessed 2026-11-13 (search summary; low weight).
[39] In-repo: `CLAUDE.md`; `.claude/rules/testing.md`; `.claude/rules/spike.md`; `.claude/rules/verification.md`; `.claude/rules/development.md`; `.claude/rules/design.md`; `.claude/rules/domain-naming.md`; `docs/feature/native-benchmark-program/intake.md`; `.context/issue-287.md`.
[40] Linux kernel source, tag v7.0 (commit 028ef9c96), vendored at `vendors/linux`. Paths relative to it, all `@ v7.0`: `include/linux/filter.h`; `include/linux/u64_stats_sync.h`; `include/linux/bpf-cgroup.h`; `include/linux/cgroup.h`; `include/linux/vtime.h`; `include/linux/perf_event.h`; `kernel/bpf/syscall.c`; `kernel/bpf/core.c`; `kernel/bpf/cgroup.c`; `kernel/bpf/trampoline.c`; `net/ipv4/tcp_ipv4.c`; `net/ipv4/af_inet.c`; `kernel/sched/cputime.c`; `kernel/sched/core.c`; `kernel/sched/fair.c`; `kernel/sched/stats.h`; `kernel/sched/psi.c`; `kernel/sched/isolation.c`; `kernel/softirq.c`; `kernel/cgroup/rstat.c`; `kernel/cgroup/cgroup.c`; `kernel/cgroup/cgroup-v1.c`; `kernel/cgroup/pids.c`; `kernel/kthread.c`; `kernel/vhost_task.c`; `kernel/workqueue.c`; `drivers/vhost/vhost.c`; `drivers/vhost/vsock.c`; `fs/proc/base.c`; `mm/memcontrol.c`; `mm/memory.c`; `kernel/time/timekeeping.c`; `kernel/time/posix-timers.c`; `arch/x86/kernel/tsc.c`; `arch/x86/kernel/kvmclock.c`; `arch/x86/kvm/x86.c`; `kernel/cpu.c`; `drivers/cpufreq/intel_pstate.c`; `kernel/events/core.c`; `kernel/rcu/tree.c`; `kernel/rcu/tree_nocb.h`; `kernel/irq/proc.c`; `kernel/irq/irqdesc.c`; `drivers/base/topology.c`. Read 2026-11-13.
