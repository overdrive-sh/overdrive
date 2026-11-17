# V-4 — Guest shutdown durations

**Verdict: FALSIFIED as written; the console-log alternative HOLDS.** Shapes DDD-11 (VM-lifecycle seams).

Host: AMD EPYC 8024P (8-core), `uname -r` 7.0.0-38-generic, Cloud Hypervisor v53.0.0, rustc 1.95.0, commit e3b42c92e (DESIGN docs dirty). Run 2026-11-16/17 on the native Metal host with `cargo xtask metal run --exclusive`.

Probe: `spike-scratch/nbp-v4-guest-shutdown/` (`results/run-1.log`, `results/diag-*.log`, `results/smoke*.log`).

Command: `cargo xtask metal run --exclusive -- cargo run --release --manifest-path spike-scratch/nbp-v4-guest-shutdown/Cargo.toml -- 30` — 30 cycles of a spin guest and 30 of a slow-exit guest (SIGTERM handler, exit delay 0–40 ms), through `run_with_adapters` and operator `deploy`/`stop`, with a `Vmm` decorator over `CloudHypervisorVmm`.

## Predicted vs actual

Prediction: both `overdrive-init` shutdown lines in the `VmmDiagnostics` tail in ≥ 29 of 30 cycles, ≥ 3 distinct durations.

- **As specified (falsified):** `VmmDiagnostics::console_tail()` was empty in 60/60 cycles; the lines appeared in 0/60. Cause (`overdrive-host/src/vmm.rs`): Cloud Hypervisor runs with `--serial file=<run_dir>/console.log --console off`, and the handle captures only the VMM's stderr pipe (a failed-boot cycle showed only VMM stderr). The 5-line / 8 KiB tail bound never mattered.
- **Alternative (holds):** reading `config.run_dir.console_log()` in the decorator when the exit fires (before forwarding it, as `GuestControlVmm` does) found both lines in 60/60 cycles (≈28 KB per run).

| | Spin guest | Slow-exit guest |
|---|---|---|
| `group-complete` − `shutdown-received` | 30 values across 11, 12, 13 ms | 11 distinct values: 11, 12, 13, 22, 23, 31, 32, 33, 43, 53, 54 ms |
| Values of 0 or 1 ms | 0/30 | 0/30 |

Slow-guest durations track the programmed delay on a ≈12 ms floor; the spin guest's 11–13 ms floor means `guest.shutdown-handling` reports a runtime-independent floor for trivially stoppable guests.

## Fixture findings (probe-side adaptations, no `crates/` change)

1. The probe's `<data_dir>` came out `root:root 0750` because it was created under the Metal runner's umask (027); `serve` sets no mode on `<data_dir>` itself, and only `<data_dir>/vm` and `clone-staging` get the 4200-group traverse grant (`prepare_clone_staging_root`, `crates/overdrive-control-plane/src/lib.rs`). The uid-dropped VMM therefore got EACCES on its disk clone (`results/smoke3.log` `namei` capture). The probe ran `chmod 0711` on the data dir after boot; a family fixture sets `umask(0o022)` before creating it, as `guest_listener_real_boot.rs` does.
2. The staged VM fixture's Ubuntu-kernel guest cannot run production `overdrive-init`: init needs the `dummy` link driver and `/lib/overdrive/ovd_fwd.ko`. The probe injected a wrapper PID 1 that loads `dummy.ko` and execs the real init, plus `ovd_fwd.ko` from `cargo xtask kmod-build`, into a copy of `/srv/vm/overdrive-testing/rootfs.ext4` with the kit kernel.

## Recommendation

DDD-11's "no production change" holds, but its seam changes from `VmmDiagnostics::console_tail()` to the console log at `VmConfig::run_dir.console_log()`, read in the `Vmm` decorator when the exit watch fires (the run directory is removed afterwards). The VM-lifecycle family needs a natively prepared guest image matching the guest kit (open design item).
