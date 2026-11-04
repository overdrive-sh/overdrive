# Spike findings: boot-cost of the D13 listen programs (cold-start budget for ADR-0265)

Feature `service-allocation-readiness-and-ending`, decision D13 (ADR-0265, the kernel observation).
Question: how much do the listen-observation programs add to the guest's boot-to-READY time, and what does today's pre-READY BPF loading already cost?
Probe: `spike-scratch/boot-cost/` (self-contained workspace; path-depends on `overdrive-core`, `overdrive-fwd`, `overdrive-testing`; nothing under `crates/` touched).
Run: `OVERDRIVE_BPF_NATIVE=1 cargo xtask metal run -- bash spike-scratch/boot-cost/run.sh all 40 3` (after `cargo xtask bpf-build` and `cargo xtask guest-kit-prepare` on the same tree).
Evidence: `spike-scratch/boot-cost/evidence/` (`run-stdout-2.log` is the raw record, one `BCJSON` line per boot; `run-stdout-2-summary.txt` is `analyze.py`'s full table).

## Verdict: PARTIAL (prediction held; the init is a spike-local instrumented copy of the real one, proven equivalent by a control)

- **B minus A is sub-millisecond, not "a few ms".** The listen additions cost **+0.36 ms at p50** (init start to READY sent, B vs the same-toolchain baseline A2), p90 of paired per-boot differences +1.29 ms, max +2.10 ms, over 40 interleaved boots each. The falsification bar was +20 ms at p50; it is missed by a factor of about 50. The prediction's direction held and its magnitude was 5 to 10 times too high.
- **What the platform already pays today before READY is about 117 ms of BPF work** (p50 116.8 ms, p90 117.9, max 120.5), inside a guest that reaches READY 131 ms after init starts and about 1297 ms after the VMM start. So the listen programs are about 0.3 percent of the BPF work already there and about 0.03 percent of VMM-start to READY.
- PARTIAL because the guest PID 1 is a copy of `overdrive-init` (see "Real init versus copy"), not the repo binary. A control run of the unmodified repo binary through the same fixture is statistically identical on the host-side measure (A minus R = -1.1 ms at p50), so the copy's load sequence is the real one.

## Substrate (real guest, real Cloud Hypervisor, the kvm-fixture boot path)

```
SUBSTRATE uname_r=7.0.0-38-generic machine=x86_64 virt=none date=2026-11-03T21:24:24Z
BCENV {"kernel_release":"7.0.0-38-generic","kernel":"/srv/vm/overdrive-testing/kernel",
       "ch":"/usr/local/bin/cloud-hypervisor", "real_init":".../target/x86_64-unknown-linux-musl/release/overdrive-init"}
```

- Host and guest kernel: `7.0.0-38-generic` (the guest kit stages the metal host's own kernel; native x86_64, `systemd-detect-virt` = `none`). Guest: 256 MiB, 1 vCPU (the kit's `boot_to_ready` shape), the kit's rootfs with `ovd_fwd.ko`, `overdrive.transport=` token on the command line.
- Boot path: `overdrive_testing::guest_kit::GuestKit::boot` (production `CloudHypervisorVmm::create`, the confined, FICLONE-cloned-rootfs path the kvm fixtures use). The driver changes only `GuestKit.init_bin` (PID 1) per variant and plays the node beacon (accept, READY) as `boot_to_ready` does, holding the guest owner's control session like the lane's `Holder`.
- Host timestamp: `Instant` taken immediately before `kit.boot(...)`; READY is the instant the full `READY` line is read from the beacon. This is "VMM start to READY" and includes the VMM spawn (about 10 ms for the `boot` call itself), firmware and kernel boot (guest `CLOCK_MONOTONIC` at init start is 615 ms), and init.
- Guest timestamps: `CLOCK_MONOTONIC` ns via `nix::time::clock_gettime`, recorded in memory by `bc::span`/`bc::mark` and printed to the console once, after READY is sent (nothing is printed on the measured path).
- Order: 3 warm-up rounds discarded, then 40 measured rounds, each round booting R, A, A2, B in that order, so host drift hits every variant equally. 172 boots, 0 failures.

## Variants

| Label | Guest init | Guest BPF object | What it is |
|---|---|---|---|
| R | the repo's real `overdrive-init`, unmodified (built by `cargo build --package overdrive-init --target x86_64-unknown-linux-musl --release`, 2,405,272 bytes) | `target/bpf/overdrive_guest_bpf.o` | control; host-side numbers only (no instrumentation) |
| **A** | spike copy of `overdrive-init` + timing marks | the repo's real `overdrive_guest_bpf.o` (sha256 `d64aa168...`, 10,848 bytes) | **baseline: today's object and load sequence** |
| A2 | same copy as A | spike rebuild of the same sources (sha256 `040d9c42...`, 10,696 bytes) | same-toolchain baseline for the clean B delta |
| **B** | same copy, `--features listen` (nine programs) | spike build with the merged `TCP_LISTEN_CB` arm + `post_bind4`/`post_bind6` + 1 MiB ringbuf + counters (sha256 `d40e8513...`, 12,928 bytes) | **design: D13 attached before READY** |

Why A2 exists: the repo's `overdrive_guest_bpf.o` and a rebuild of the identical sources in this spike differ in codegen, and the `sock_release` program (a 1,024-iteration slot loop) verifies 2.9 ms slower in the repo's object (38.96 vs 36.06 ms, A vs A2, constant across 40 boots). B is compared with both: A for "versus today's object", A2 for "the cost of the additions and nothing else". The `listen` additions are the listen-hooks spike's design object (`spike-scratch/listen-hooks/bpf`), minus that spike's V35-only extra `listen_only` sockops program, which the design does not contain.

## Breakdown, variant A (today), milliseconds, 40 boots

| Step (guest `CLOCK_MONOTONIC` span) | p50 | p90 | max |
|---|---:|---:|---:|
| `Ebpf::load` guest object (parse, relocate, create maps) | 21.53 | 21.73 | 21.98 |
| `Ebpf::load` candidate object (`fentry tcp_check_req`) | 21.92 | 22.07 | 22.98 |
| other prep (mount bpffs, staging dir, `ACTIVE_CORRELATION` pin, init maps) | 0.89 | 0.97 | 0.99 |
| **prepare total** | **44.37** | 44.64 | 45.44 |
| program load (verifier): `sock_release` | **38.96** | 39.08 | 39.89 |
| program load: `tcp_correlation_candidate` (fentry, includes a `Btf::from_sys_fs` of 15.30 p50) | **24.51** | 24.81 | 27.37 |
| program load: `tcp_correlation` (sockops) | 1.09 | 1.10 | 1.15 |
| program loads: `connect4` + `sendmsg4` + `getpeername4` + `recvmsg4` | 0.40 + 0.24 + 0.19 + 0.11 | | |
| **sum of program loads** | **65.51** | 66.07 | 69.11 |
| attach: `tcp_correlation_candidate` (fentry) | 6.67 | 6.69 | 6.77 |
| attach: all six other programs together | 0.29 | | |
| **sum of attaches** | **6.94** | 6.97 | 7.10 |
| **pre-READY BPF total (prepare + loads + attaches)** | **116.83** | 117.86 | 120.54 |
| `configure_guest_transport` whole (adds netlink, sysctls, binds, forwarder module, vsock listener) | 126.73 | 127.82 | 130.54 |
| init start to READY sent | 130.99 | 132.03 | 134.77 |
| host: VMM start to READY | 1296.52 | 1304.92 | 1313.68 |

Paste (from `run-stdout-2-summary.txt`):

```
### variant A: p50 / p90 / max / min / mean (ms)
prep:total                                                    44.368    44.639    45.437    44.152    44.437
verifier+load:tcp_correlation_candidate                       24.511    24.809    27.366    24.305    24.626
verifier+load:sock_release                                    38.956    39.076    39.893    38.816    39.002
attach:tcp_correlation_candidate                               6.669     6.693     6.767     6.640     6.672
load:sub:btf_from_sys_fs (inside candidate load)              15.298    15.472    18.147    15.191    15.408
PRE-READY BPF TOTAL (prep+loads+attaches)                    116.831   117.864   120.535   116.364   117.046
transport_total (configure_guest_transport)                  126.734   127.816   130.536   125.966   126.860
init_start_to_ready_sent                                     130.993   132.032   134.769   130.023   131.123
```

What dominates the 117 ms:

| Cost | ms (p50) | share |
|---|---:|---:|
| `sock_release` verifier (the 1,024-iteration loop over `SLOT_META`) | 38.96 | 33 percent |
| two `Ebpf::load` calls | 43.45 | 37 percent |
| `fentry` candidate load (excluding its BTF read) plus its attach | 9.2 + 6.67 | 14 percent |
| `Btf::from_sys_fs` parses (3 in total, 15.3 ms each, see below) | 45.9 (inside the rows above) | 39 percent |
| everything else (sockops and four cgroup programs, prep) | about 2.5 | 2 percent |

BTF: one `Btf::from_sys_fs` is timed directly (15.30 ms, inside the candidate load). The two `Ebpf::load` calls each parse kernel BTF as well, because aya 0.13.1's `EbpfLoader` defaults to `Btf::from_sys_fs()` (`aya-0.13.1/src/bpf.rs:171`); that is inferred from the source and from the 21.5 ms per `Ebpf::load` (15.3 ms BTF plus about 6 ms ELF parse, relocation and map creation), not separately timed. Three parses of the same vmlinux BTF is therefore about 46 ms, 39 percent of the pre-READY BPF cost.

## Breakdown, B minus the baselines (the answer to the question)

| Metric (ms) | A p50 | A2 p50 | B p50 | **B - A2** p50 | B - A2 paired p90 / max | B - A p50 |
|---|---:|---:|---:|---:|---:|---:|
| `Ebpf::load` guest object | 21.53 | 21.53 | 21.60 | +0.07 | +0.25 / +0.86 | +0.06 |
| load `tcp_correlation` (merged arm: +112 insns) | 1.09 | 1.10 | 1.26 | **+0.16** | +0.18 / +0.22 | +0.17 |
| load `post_bind4` | n/a | n/a | 0.245 | +0.245 | | |
| load `post_bind6` | n/a | n/a | 0.104 | +0.104 | | |
| attach `post_bind4` + `post_bind6` | n/a | n/a | 0.017 + 0.016 | +0.033 | | |
| **pre-READY BPF total** | 116.83 | 113.91 | 114.32 | **+0.42** | +1.05 / +2.31 | -2.51 |
| **init start to READY sent** | 130.99 | 128.08 | 128.44 | **+0.36** | +1.29 / +2.10 | -2.56 |
| host: VMM start to READY | 1296.52 | 1295.23 | 1295.88 | +0.65 | +10.6 / +23.3 (noise) | -0.63 |

- The sum of the measured parts is 0.07 + 0.16 + 0.245 + 0.104 + 0.033 = 0.61 ms; the whole-run delta is +0.36 to +0.42 ms. They agree within the run-to-run noise of the unchanged steps (the candidate load alone moves by about 0.1 ms between variants).
- The ringbuf and counter maps (`LISTEN_EVENTS` 1 MiB ringbuf, `LISTEN_COUNTERS`) cost +0.07 ms inside `Ebpf::load`. The ringbuf's 1 MiB is kernel memory allocated on map create; it did not show up in any timing here.
- B minus A is negative (-2.5 ms) only because the repo's production object is 2.9 ms slower on `sock_release` than the identical sources rebuilt in the spike. That is not a property of the additions; B-A2 is the like-for-like number.
- The host-side VMM-start-to-READY delta is not resolvable: variant-to-variant host noise (p90 minus p50 of about 7 ms; kernel boot varies by up to 13 ms between boots) is 20 times the effect. The guest-side `CLOCK_MONOTONIC` split is the measurement; the host number is reported for completeness and as the control.

Paste:

```
### delta B - A2  (p50 of per-metric distributions, and paired by round: median / p90 / max of per-round difference) (ms)
verifier+load:tcp_correlation                              p50Δ=   0.164 p90Δ=   0.167  paired: med=   0.165 p90=   0.180 max=   0.223
SUM program loads (verifier)                               p50Δ=   0.355 p90Δ=   0.160  paired: med=   0.338 p90=   0.756 max=   1.744
PRE-READY BPF TOTAL (prep+loads+attaches)                  p50Δ=   0.419 p90Δ=   0.045  paired: med=   0.343 p90=   1.048 max=   2.314
init_start_to_ready_sent                                   p50Δ=   0.362 p90Δ=   0.118  paired: med=   0.241 p90=   1.294 max=   2.102
host_vmm_start_to_READY                                    p50Δ=   0.649 p90Δ=   2.673  paired: med=   1.667 p90=  10.608 max=  23.336
### delta B - A  ...
verifier+load:sock_release                                 p50Δ=  -2.885 p90Δ=  -2.906  paired: med=  -2.901 p90=  -2.780 max=  -2.621
PRE-READY BPF TOTAL (prep+loads+attaches)                  p50Δ=  -2.508 p90Δ=  -3.072  paired: med=  -2.581 p90=  -2.070 max=  -0.809
init_start_to_ready_sent                                   p50Δ=  -2.555 p90Δ=  -3.061  paired: med=  -2.591 p90=  -2.015 max=  -1.164
### instrumentation control: host VMM-start->READY, real unmodified init R vs instrumented A
R p50=1297.604 p90=1304.550 max=1317.858;  A p50=1296.517 p90=1304.919 max=1313.675;  A-R p50Δ=-1.087
```

## Predicted versus actual

Prediction: B minus A is a few ms, dominated by two extra program loads and attaches; the merged arm adds negligible verifier time. Falsification: B minus A >= 20 ms at p50.

- **Held, with a smaller magnitude.** B-A2 = +0.36 ms (init start to READY, p50). The two extra loads are 0.35 ms of it (0.245 + 0.104), attaches 0.03 ms, the merged arm's verifier time +0.16 ms (negligible but not zero; more than the two extra programs' attaches), the maps +0.07 ms. Not "a few ms" and not 20.
- The listen-hooks and guest-signal spikes measured "50 to 80 ms" for loading and attaching the listen programs in a spike guest (`GS_HOOKS_ATTACHED ms=78.79 / 75.17 / 61.50 / 51.73 / 72.09`). This spike shows why that figure is not the marginal cost: that init loaded its own object, and a whole object load (one `Ebpf::load` including a kernel-BTF parse) is 22 ms here, plus program verification and a non-LTO guest binary (the first run of this spike, see below, was 28 ms slower end to end from the build profile alone). The programs themselves cost 0.6 ms when they ride in the object init already loads.

## Real init versus copy

- The guest PID 1 for A, A2 and B is `spike-scratch/boot-cost/init/`, a **copy** of `crates/overdrive-init` (`src/` copied whole; test modules and in-guest test cells removed; `Cargo.toml` rewritten with explicit dependency versions and path dependencies on `overdrive-core` and `overdrive-fwd`; `build.rs` replaced so the object path comes from `$BC_GUEST_OBJ`; the repo's `Cargo.lock` reused so aya, nix and friends resolve to the repo's versions). It was **not** possible to instrument the real init without editing `crates/`, and its `build.rs` pins the object path.
- The only source differences are: `src/bc.rs` (in-memory spans), `bc::span`/`bc::mark` calls in `main.rs`, `transport/mod.rs` (around each load/attach) and `transport/kernel.rs` (around the two `EbpfLoader::load` calls, `Btf::from_sys_fs` and the cgroup file), and, behind `--features listen`, two `GuestProgram` variants (`PostBind4`, `PostBind6`) with their `load`/`attach` arms (`CgroupSock`, attach at the cgroup2 root, `Single`). The load order, the staging-dir pin, the mounts and everything else are the real code, unmodified.
- **Equivalence control (R).** The unmodified repo binary, booted through the same driver in the same rounds, reaches READY at p50 1297.6 ms against the copy's 1296.5 ms (A-R = -1.1 ms; the paired spread is the host noise above). A first run compiled the copy with cargo's default release profile instead of the repo's (`lto = "thin"`, `codegen-units = 1`, `strip`) and A was **+27.9 ms** slower than R, almost all in BPF loading (`Btf::from_sys_fs` 25.1 ms against 15.3 ms with the repo profile). That run is kept as `evidence/run1-default-profile-INVALID-instrumentation-control-failed.log` and is not used for any number here; the control caught it.
- The B object is not the repo's production source: it is the listen-hooks spike's copy of the guest BPF sources plus the listen additions. A is the real production object.
- Not measured: the forwarder thread that drains the ringbuf (a thread spawn, tens of microseconds), the beacon `LISTEN` write path, and any runtime cost per event (listen-hooks measured the per-event cost under a 20,000-socket flood: no slowdown).

## Design implications for D13 / ADR-0265

1. **Attach all three listen hooks before READY, in the same object. The cost is about 0.4 ms (guest) and is below host noise.** Nothing needs to move after READY for cold-start reasons; and V36 (guest-signal) showed that attaching after the workload starts misses listeners (0 of 50 reported), so anything moved after READY would still have to precede EXEC. There is nothing to gain from it.
2. **Keep it in the object init already loads.** The marginal cost of a hook is its program verification and a link create (0.1 to 0.25 ms load, 0.02 ms attach). The cost of a *separate* BPF object is about 22 ms (one `Ebpf::load`, dominated by a kernel-BTF parse). The D13 design (merge the TCP arm into `tcp_correlation`; put `post_bind4`/`post_bind6` and the ringbuf in the same object) is exactly the cheap shape. A second object for the listen programs would cost about 50 times as much.
3. **The `tcp_correlation` merge costs +0.16 ms of verifier time** (+112 verified instructions); the correlation path itself is unchanged.
4. **The pre-READY BPF budget the platform already pays is 117 ms, of which 46 ms is the same kernel BTF parsed three times and 39 ms is one program's verifier loop.** These are existing costs, outside D13, but they set the scale: the listen programs are 0.4 percent of it. Two unmeasured observations for whoever owns the millisecond cold-start goal, to be tested rather than assumed: (a) aya's `EbpfLoader::btf(Some(&btf))` takes a shared `Btf`; passing one parsed `Btf` to both loads and to the fentry load would remove two of the three 15 ms parses (about 30 ms) if the API behaves as documented; (b) `sock_release`'s 1,024-iteration loop is 33 percent of the pre-READY BPF time. Neither is a D13 matter, neither was tried here, and neither should be added to D13's scope without the user's decision.
5. **Budget line for the D13 consequences.** "Adds about 0.4 ms (p50), at most about 2 ms (paired max over 40 boots) to guest boot-to-READY on a 7.0.0-38 guest; today's pre-READY BPF load is about 117 ms." Replace the "50 to 80 ms" figure carried in `findings-guest-signal.md`, which measured a whole separate object in a non-LTO spike guest.

## What this does not prove

- One guest kernel (`7.0.0-38-generic`, the fixture kernel, same as the host) and one VMM build; 1 vCPU, 256 MiB. A slower guest CPU scales the 117 ms and, proportionally, the 0.4 ms.
- The copy's equivalence to the real init is shown by the host-side control and by reading the diff, not by timestamps inside the real init (it has none).
- B's object is built from the listen-hooks design object, not from a landed production change; the production merge may differ in instruction count (the 112-insn figure is from listen-hooks V35/V37).
- Timings are from a warm host (3 discarded warm-up rounds, page cache hot); the guest's first-ever boot on a cold host cache is not measured.
- 40 boots per variant; the p90 of 40 samples is the 36th value.

## Cleanup

No VMM survives (`pgrep cloud-hyp` = 0, no `/dev/vhost-vsock` holders), no guest cgroup scopes remain, no BPF programs of the spike remain (guest programs live and die with each guest VM; nothing is attached on the host). Removed from metal: `/tmp/boot-cost-target`, `/srv/vm/overdrive-testing/bootcost-*`, and the leaked per-launch runtime dir `/run/ovd-gk-771623-6` (a boot of this run's driver). Two other `/run/ovd-gk-*` directories (`1519761-1`, `diagnostic-3903596`) belong to other runs and were left alone. The trap in `run.sh` chowns `spike-scratch/boot-cost` back to `ubuntu` after each run (`stat` shows owner `ubuntu`). No GitHub issues, feature-delta or ADR edits.

## Probe source retained

The measurement used a spike-local copy of `crates/overdrive-init/src` with timing spans added, because `overdrive-init` is a binary-only crate and spike code may not edit `crates/`. That copy has been deleted from `spike-scratch/boot-cost/init/` (user ruling 2026-11-03) so that no duplicate of production source stays in the repository. The copy was validated against the unmodified repo binary booted in the same rounds (READY p50 1297.6 ms for the repo binary vs 1296.5 ms for the copy). The driver, the analysis script, the BPF object sources and the raw evidence under `spike-scratch/boot-cost/evidence/` are retained; the numbers above stand.
