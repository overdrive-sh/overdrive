# V-A (O3) — What one guest-kit guest costs on Metal: launch to ready, exec, shutdown

**Verdict: WORKS.** A fresh guest-kit guest per replicate costs about 1.08 s at p50 (1.13 s at p95); 60 fresh guests (30 replicates x 2 arms) cost about 65 s of guest lifecycle. One guest per run with 60 execs costs about 1.2 s. Both are affordable; the choice between them is a methodology decision, not a cost constraint.

Host: native Metal host, AMD EPYC 8024P, `uname -r` = `7.0.0-38-generic`, Cloud Hypervisor from the guest kit. Commit `5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b`, working tree dirty with docs only (`git status` shows no `crates/` change). Run 2026-11-19 through the benchmark form (exclusive).

Probe: `spike-scratch/guest-bench-return/` (family `guest-boot-cost`; records in `spike-scratch/guest-bench-return/results/guest-boot-cost/e65831289881af9b4e474326/`).

## Existing evidence first (debugging.md §6)

- `benchmarks/results/vm-lifecycle/ab04acf6fd5ba76ef370ac0f` (commit `6ef7014f0`): `guest_ready_ns` p50 1,279,495,180 ns (p95 1,291,859,456). That is the production `serve` + `VmDriver` + `overdrive-init` READY path, not the scripted-peer kit path a benchmark body would boot.
- `git log 6ef7014f0..HEAD -- crates/overdrive-testing/src/{guest_kit,guest_peer,vm_fixture}.rs crates/overdrive-init crates/overdrive-host/src/vmm.rs crates/overdrive-worker/src/vm_driver.rs xtask/src/guest_kit_prepare.rs` prints nothing, so that record is still valid for its own path.
- V-10 (`findings-v10-guest-boot.md`) reports `group-complete − shutdown-received` of 11 to 17 ms inside the guest; it does not time launch to ready or the host-side VMM exit.

Neither covers launch to peer-ready, exec and VMM exit on the kit's scripted path, so it was measured.

## Hypothesis, prediction, falsification

- **Hypothesis:** a guest-kit scripted guest boots in about the production READY time, so per-replicate guests are affordable.
- **Prediction:** boot to peer-dialled within 0.8 to 1.4 s (vm-lifecycle `guest_ready_ns` is 1.28 s), exec of a trivial payload under 50 ms, shutdown under 100 ms; 60 fresh guests under 2 minutes.
- **Falsification:** boot p50 over 3 s, or any boot or shutdown failure in 90 replicates, or 60 guests over 5 minutes.

## Method

`guest-boot-cost` is a native benchmark (`substrate = native`, the closed registry) whose body runs, per replicate: `GuestKit::image` (scripted-peer image carrying the prepared in-guest payload), `boot_scripted`, one `peer.exec` of the payload, then 20 further `exec`s in the same guest (warm round trip), `power_off`, `wait_exit`. Two arms with identical bodies (an A/A control); the runner interleaves them in blocks. 30 replicates per arm; the runner produced 90 replicate records (3 positions per block, 30 blocks), all `ok`, 0 failed. Because both arms are the same treatment, the figures below pool all 90.

Commands (all on the Metal box):

```
cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build
cargo xtask metal run -- cargo xtask guest-kit-prepare
cargo xtask metal run --benchmark guest-boot-cost --spike guest-bench-return
cargo xtask metal pull spike-scratch/guest-bench-return/results/guest-boot-cost/e65831289881af9b4e474326
```

## Observed (pooled 90 replicates, computed from `record.jsonl`)

```
image_ns               n= 90 p50=    74.50ms p95=    78.56ms max=    87.94ms min=    70.60ms
boot_to_peer_ns        n= 90 p50=   947.92ms p95=  1004.16ms max=  1030.18ms min=   905.03ms
first_exec_ns          n= 90 p50=     8.60ms p95=    10.91ms max=    13.11ms min=     7.14ms
shutdown_ns            n= 90 p50=    40.21ms p95=    60.29ms max=    60.34ms min=    40.17ms
total_ns               n= 90 p50=  1120.11ms p95=  1183.25ms max=  1685.01ms min=  1072.98ms
warm_exec_ns(1800)     n=1800 p50=     2.21ms p95=     2.36ms max=     8.70ms min=     2.04ms
image+boot+exec+shut   n= 90 p50=  1077.77ms p95=  1131.64ms max=  1173.70ms min=  1028.88ms
```

From the report: A/A arms are "not distinguishable from noise" on every metric (for example `boot_to_peer_ns` delta −4.6 ms, 95% interval [−34.6, +12.9] ms; the A/A interval is [−24.7, +26.1] ms).

Terms: `image_ns` = `GuestKit::image` (rootfs build and copy of the payload plus its shared libraries). `boot_to_peer_ns` = `boot_scripted` call to the scripted peer dialling the host control port (launch, kernel boot, module loads, peer up). `first_exec_ns` = one `exec` round trip of the 1-sample payload including its process start. `shutdown_ns` = `power_off` sent to the VMM observed exited.

Derived cost (p50 of `image+boot+exec+shutdown` = 1.078 s; p95 = 1.132 s):

| Granularity | Guest lifecycle for 30 replicates x 2 arms |
|---|---|
| Fresh guest per replicate (60 guests) | 60 x 1.078 s = **64.7 s** (p95-based: 67.9 s) |
| One guest per run, 60 replicate payloads exec'd in it | image 0.075 + boot 0.948 + shutdown 0.040 + 60 x 2.21 ms = **about 1.2 s** |

The first replicate of the run was the slowest (1.685 s total: `GuestKit::provision`, cold caches); steady state is 1.07 to 1.18 s.

## Edge cases and notes

- `shutdown_ns` clusters at 40 ms and 60 ms: `GuestKitVm::wait_exit` polls every 20 ms (`guest_kit.rs`, `std::thread::sleep(Duration::from_millis(20))`). Shutdown is quantised to that step; a body that needs finer shutdown timing cannot take it from `wait_exit`.
- `boot_to_peer_ns` (0.95 s) is lower than vm-lifecycle `guest_ready_ns` (1.28 s) because the production path also includes admission, `overdrive-init` READY and the driver's own steps. The two are different paths, so do not substitute one for the other.
- This is a fresh image per boot (`GuestBoot` says one boot per image), so `image_ns` (75 ms) is a per-guest cost even for per-replicate guests.
- Fixture adaptation, spike-local (see `findings-guest-return-path.md`): `vm_fixture::workspace_root()` resolves from the cwd, so a `--spike` crate must `chdir` to the repository root before `GuestKit::provision`.

## Design implications

- Per-replicate guests keep the parent rule ("every replicate builds a fresh fixture, no state survives") and cost about one minute per 60 replicates; they also fit inside the `--benchmark` runner's exclusive window with room to spare. The one-guest-per-run alternative saves about 64 s but lets replicates share kernel and page-cache state, which breaks that rule.
- The record must state which granularity ran (O3 asks for this); the method used here is fresh guest per replicate.
- Guest-substrate benchmarks that boot a guest per replicate have a hard floor of about 1.1 s per replicate; a family with thousands of replicates per arm is not feasible per-replicate and would have to choose one guest per run with in-guest repetition.

## Gate recommendation

PROMOTE: fresh guest per replicate is affordable (about 65 s per 60 replicates); no production-code change is needed to measure it.
