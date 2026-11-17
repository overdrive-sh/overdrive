# V-10 — Guest-kit workload image boot, console log and beacon lines

**Verdict: PARTLY FALSIFIED.** The image boots and `READY` and both shutdown lines are present in 60 of 60 cycles, but the guest `EXIT` line is received by the host in 0 of 60 cycles on an operator stop. Stage rows 6 (workload-exit) and 7 (reaper-exit) of DDD-11 end or start at "`EXIT` received (decorator)", a boundary that does not occur on this path. This changes the stage table, not only the fixture, so it is returned as a design gap.

Host: native Metal host, AMD EPYC 8024P, `uname -r` 7.0.0-38-generic, Cloud Hypervisor from the guest kit (`cloud_hypervisor_bin`), commit 3ee66453b (working tree dirty with step 02-01 work). Run 2026-11-17 with `cargo xtask metal run --exclusive`.

Probe: `spike-scratch/nbp-v10-guest-boot/` (`results/run-1.log`). It boots one in-process `serve` through `run_with_adapters`, the guest-kit image `GuestKit::image(dir, "workload", &GuestImageSpec::workload())` with `overdrive-guest-peer idle` as the `[job]` workload, a `Vmm` decorator that reads `config.run_dir.console_log()` when the exit watch fires, and a `BeaconIo` decorator that records every line the driver reads. `<data_dir>` and the directories beneath it are created under `umask(0o022)` and set to `0755`.

Command: `cargo xtask metal run --exclusive -- cargo run --release --manifest-path spike-scratch/nbp-v10-guest-boot/Cargo.toml -- 60`.

## Hypothesis, prediction, falsification

- **Hypothesis:** the image reaches `READY` and both `overdrive-init: <stage> elapsed_ms=<n>` shutdown lines appear in `console.log`.
- **Prediction:** 60 of 60 cycles show `READY`, `EXIT` and both lines.
- **Falsification:** any cycle without `READY`, or without both lines, or a permission error on `<data_dir>`.

## Observed

```
# uname -r: 7.0.0-38-generic
--- SUMMARY ---
cycles=60 running=60 READY=60 EXIT=0 both_shutdown_lines=60 durations_ms(value:count)={11: 4, 12: 22, 13: 32, 14: 1, 17: 1}
```

A representative cycle:

```
CYCLE nbp-v10-000: running=Some(Running) terminal=Some(Terminated) READY=true EXIT=false sr=Some(457) gc=Some(470) beacon_lines=["READY pid=1 port=1053098523 launch=477d2b9b96e651b17025627d546a1fbd"]
```

The tail of the first cycle's console log (full log in `results/run-1.log`):

```
overdrive-init: beacon boundary tx READY
overdrive-init: beacon boundary rx EXEC
overdrive-init: shutdown-received elapsed_ms=457
overdrive-init: group-complete elapsed_ms=470
overdrive-init: poweroff-requested elapsed_ms=472
overdrive-init: fatal: beacon connection I/O failed: Broken pipe (os error 32)
```

- Held: 60 of 60 cycles reached `Running`, delivered `READY` (line format `READY pid=<n> port=<n> launch=<hex>`), and showed both shutdown lines. No permission error on `<data_dir>`; the DDD-11 fixture (umask 022, `0755`) lets the confined VMM identity reach its clone. `group-complete` minus `shutdown-received` is 11 to 17 ms (11: 4, 12: 22, 13: 32, 14: 1, 17: 1), consistent with V-4's 11 to 13 ms floor.
- Falsified: `EXIT` is never received. The guest's `fatal: beacon connection I/O failed: Broken pipe` after `group-complete` shows the host had already closed the beacon connection when the guest wrote `EXIT`.

## Cause (source reading, `crates/overdrive-worker/src/vm_driver.rs`)

`VmDriver::stop` logs `vm.lifecycle.stop_enter` and then asks the beacon writer to stop (`request_stop`). `stop_beacon_writer` writes `SHUTDOWN` and calls `state.close_socket()` immediately; the writer is the only reader of the beacon after `READY`. The guest therefore finishes the group, tries to write `EXIT` on a closed connection, and no `BeaconIo::read_line` ever returns it. The `EXIT` line reaches the driver only when the workload ends on its own (the writer is still reading). The events on the operator-stop path are `vm.lifecycle.stop_enter`, `vm.lifecycle.writer_finished`, `vmm.process.reaped` and `vm.lifecycle.cleanup_calls_finished`; none is an `EXIT` boundary.

## Design implication

DDD-11 stages 6 (`stop_enter` to `EXIT` received) and 7 (`EXIT` received to `vmm.process.reaped`) need a different observable boundary for an operator stop. The decorator seams named in DDD-11 cannot supply it: `BeaconIo::read_line` never sees `EXIT`. Which boundary to use (for example `vm.lifecycle.writer_finished`, or a boundary the design adds) is a design decision for the stage table, not a fixture adaptation.
