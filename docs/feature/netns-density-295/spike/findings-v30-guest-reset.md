# V-30: guest reset with and without the `fd=` guard

Probe increment: `spike-scratch/netns-density-295-v30-guest-reset/` (`run.py`, `launch.py`, `build.sh`, `guest/`, `analyze.py`, `analysis.txt`, `fd-mode.patch`, `evidence/` — 143 evidence directories). Question source: `.context/over-engineering-audit-review.md` findings 3 and 6. Recorded by the orchestrator from the spike agent's report (the agent's Write tool was refused).

**Verdict: the concern is half right.** Without the once-only `fd=` guard the fork reboots a reset guest in place, inside the same VMM process, under the same CID; a crashing PID 1 never ends the VMM. Host flows from the old incarnation ARE reset, because the fork's reboot closes the old vhost fd before opening the new one. The fork has no flag or API to exit on guest reset.

## Setup

- Native x86_64 metal (AMD EPYC 8024P), canonical lease, `v30-` tokens; fail-closed substrate probes passed every run (`systemd-detect-virt` none, `/dev/kvm` API 12).
- Host and guest `7.0.0-29-generic`; kernel citations against `vendors/linux` @ `v7.0`.
- Cloud Hypervisor fork `41a619d19`, built on the host from sources whose `vhost_kernel.rs`, `lib.rs`, `vm_config.rs` match the repo's sha256. `ch-stock` opens `/dev/vhost-vsock` itself; `ch-fd` is the V-25 `fd-mode.patch` unchanged, including the once-only guard.
- Guest: busybox initramfs + a static vsock probe; each boot reports a new `boot_id`, listens on port 5000, connects to the host on 6000, acts on `GO <case>`. Cmdline `panic=1`, plus `reboot=t` for triple-fault cases.
- Modes: `stock`; `fd-keep` (launcher claims the CID, passes the fd and keeps its own copy, as ADR-0170's `serve` does); `fd-close` (launcher closes its copy right after spawn).
- Cases: PID 1 exits ("Attempted to kill init!"); `sysrq-c`; busybox `reboot` through a real init; `reboot -f`; `sysrq-b`; triple fault (`reboot=t`; CH logs "Guest likely triple-faulted", then `VmExit::Reset`); panic then triple fault; `poweroff` control.
- 4–5 runs per cell, 96 matrix cells; every cell identical across repeats. CIDs 3700–3723.

## Verdict table

| # | Question | Verdict |
|---|---|---|
| Q1a | `stock`: does a guest reset end the VMM? | **No.** 0/29 exits across 7 reset cases; guest reboots in place, VMM pid unchanged. |
| Q1b | `fd=` guard: does a guest reset end the VMM? | **Yes, rc=1.** 56/56 across `fd-keep` and `fd-close`. The exit is a failed reboot (`ClaimedFdConsumed`), not a clean exit. `poweroff` exits rc=0 in every mode (12/12). |
| Q1c | Guest PID 1 crashes on every boot | `stock`: infinite in-place reboot loop, 9 reboots in 15 s, VMM never exits. `fd-keep`: VMM exits rc=1 at the first reset (2.0 s). |
| Q2a | `stock`: host flows into the old incarnation | **Reset.** ECONNRESET 22–45 ms after the reset request; busybox `reboot` gives orderly EOF at 22–43 ms; panic cases reset at 1.09–1.12 s when the guest actually restarts. |
| Q2b | `stock`: host listener under the same CID | **Accepts the new incarnation:** 29/29 `HELLO` with a new `boot_id`; host→guest `connect()` reaches the new incarnation 29/29. |
| Q2c | `fd-keep`: host flows after the VMM exits | **Stay ESTABLISHED to a dead guest** while the launcher holds its fd copy (observed 7–8 s); `send` succeeds; they reset the moment the launcher closes; `claim(CID)` is `EADDRINUSE` until then. |
| Q2d | `fd-close`: host flows after the VMM exits | Reset at VMM exit with ECONNRESET, same timings as `stock`; CID claimable immediately (32/32). |
| Q3a | Fork flag/API to exit on guest reset? | **No.** |
| Q3b | Event-monitor supervisor as the pin | **Works, but a race:** 30/30 kills landed before `rebooted`. |

## Key evidence

`stock` rebootf — VMM stays up, guest returns with a new `boot_id`:
```
GUEST_BOOT boot_id=930d8178-…   GUEST_GO rebootf   [ 1.431873] reboot: Restarting system
==== SERIAL FILE TRUNCATED (VM re-created) ====
GUEST_BOOT boot_id=5adea822-…   GUEST_HELLO_SENT
ch.log: 1.655815s VM reset event / 1.655875s Event: … rebooting / 1.683052s Booting VM from config
g2h#1 error@0.043s recv: ECONNRESET    h2g#1 error@0.043s recv: ECONNRESET
```

Crash loop (PID 1 exits every boot):
```
stock   vmm_exited=false rebooted_events=9 secs=15.01 cpu_s_ch=14.72
fd-keep vmm_exited=true rc=1 at 2.0s
        ch.log: Fatal error: VmmThread(VmReboot(DeviceManager(CreateVhostKernelVsock(ClaimedFdConsumed))))
```

`fd-keep` flows after the VMM has exited:
```
g2h#1 open, h2g#1 open, send probe "send ok", claim_cid_while_launcher_holds=EADDRINUSE
launcher_closed_at_s=8.198 -> both flows recv: ECONNRESET at 8.705s, claim=OK
```

Supervisor experiment (CH `--event-monitor fd=<pipe>`, a thread SIGKILLs the VMM on the first `rebooting` event):
```
rebootf, idle:                  10/10, kill 0.029–0.204 ms after the event was read
rebootf, 64 busy procs/16 cpu:  10/10, 0.037–0.064 ms
sysrq-b, 64 busy procs/16 cpu:  10/10, 0.040–0.059 ms
'rebooted' never seen, no new incarnation. CH's own rebooting->rebooted window: 21.3–40.6 ms.
```

## Why it behaves this way

- **Reset becomes reboot.** A guest reset becomes `reset_evt` then `vm_reboot` (`vmm/src/lib.rs:2088-2094`). Triple fault: `hypervisor/src/kvm/mod.rs:2530-2533` (`VcpuExit::Shutdown` → `VmExit::Reset`), then `vmm/src/cpu.rs:1432-1437`. i8042, CMOS and ACPI reset registers also write `reset_evt` (`devices/src/legacy/i8042.rs:49`, `cmos.rs:81`, `devices/src/acpi.rs:65`). Only ACPI S5 poweroff ends the VMM (`devices/src/acpi.rs:82`, `lib.rs:2096-2106`).
- **Reboot reopens the device by path.** `vm_reboot` (`lib.rs:2427-2498`) shuts down and drops the old `Vm`, then `Vm::new` and `vm.boot()`, reaching `VhostKernelVsock::new` (`virtio-devices/src/vsock/vhost_kernel.rs:209-213`: open, `set_owner`, `set_guest_cid`). `/proc/<vmm>/fd` shows exactly one vhost-vsock fd before and after; the new `SET_GUEST_CID` never returned `EADDRINUSE`, so the old device was fully closed first.
- **Flow reset comes from the release path.** `vhost_vsock_dev_release` (`drivers/vhost/vsock.c:753-786 @ v7.0`) unhashes the CID, waits for RCU, calls `vhost_vsock_reset_orphans` (`:765-768`; sweep at `:721-748`), setting `ECONNRESET` on every flow whose remote CID no longer has a device. It runs only on the last close of a vhost-vsock fd: `stock` and `fd-close` close the last fd in the VMM; `fd-keep` leaves a launcher reference, so no sweep until the launcher closes. (Inferred from the differential and source; the kernel function was not traced directly.)
- **`ss --vsock` is not an oracle:** after the sweep it still prints `ESTAB` (`SS_UNCONNECTED` and `TCP_ESTABLISHED` are both 1). The harness reads `recv` and `SO_ERROR`.

## What the fork offers (Q3)

- No reboot policy; no CLI argument (`cloud-hypervisor/src/main.rs`) selects exit-on-reset; the `Reset` dispatch always calls `vm_reboot`.
- `--no-shutdown` (`main.rs:390`) is the opposite knob (keeps the VMM alive after guest shutdown, `lib.rs:2099`).
- `--watchdog` (`virtio-devices/src/watchdog.rs:170-177`) writes `reset_evt` → reboot; `--pvpanic` (`devices/src/pvpanic.rs:150-152`) only emits `guest/panic`.
- `--event-monitor` is the only observable: `vm/rebooting` at `lib.rs:2428` (before teardown), `vm/rebooted` at `lib.rs:2495`. It writes pretty-printed JSON objects (not JSONL), via `path=` or `fd=`.
- Untested: a source-level fork change sending `EpollDispatch::Reset` down the exit path instead of `vm_reboot` — a reading, not a result.

## Design implications

1. Dropping the CID claim keeps flow hygiene: with CH as the only holder, the fd closes at VMM exit and at every in-place reboot, and the kernel resets host flows before the new device exists.
2. Dropping the claim loses "a guest reset ends the VMM": a PID-1 crash is an in-place reboot (same VMM pid, cgroup, netns, CID, API socket, launch token; only `boot_id` changes). A driver watching for VMM exit cannot see it; a crash loop never charges a restart budget (9 reboots in 15 s, one core busy).
3. The `fd=` guard is a poor pin: it works only as a failed reboot with rc=1, and while `serve` keeps its fd copy host flows stay ESTABLISHED to a dead guest.
4. Pin the crash where it is observable. Options: supervise `vm/rebooting` from the driver and kill the VMM (30/30, a race with ~100× margin), or the deterministic fork change above.
5. `panic=1` is what turns a PID-1 crash into a reset; without it the guest would hang after a panic with no reset event (not tested).

**Recommendation:** drop the CID claim if wanted — flow reset is guaranteed by the single-holder close. Do not rely on the fork to end the VMM on guest reset; pin it by a small fork change (deterministic, untested) or by supervising `vm/rebooting` (tested, a race). Never keep a claim-holder fd copy past VMM exit. (User ruling 2026-10-11: change the fork; recorded in CLAUDE.md § "A guest reset ends the VMM — enforced in the fork".)

## Not run

`--landlock` in `stock` mode (the fork grants `/dev/vhost-vsock` for every non-`fd` vsock, `vmm/src/vm_config.rs:905-910`, so the reopen should be allowed); the V-25 unguarded `fd=` variant; a real Overdrive rootfs on `/dev/vda` with the production driver; aarch64; CID lease allocation (`BASE + lease offset`); direct kernel tracing of `vhost_vsock_reset_orphans`; repeats are 4–5 per cell (not a distribution).

Cleanup: all VMM processes killed by the harness; host `/tmp/v30-work` removed under the lease; `vhost_vsock` modules were already loaded and left loaded; no `v30` processes or CIDs remain.
