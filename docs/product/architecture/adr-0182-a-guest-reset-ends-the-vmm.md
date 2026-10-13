# ADR-0182 — A guest reset ends the VMM, enforced by a change in the vendored Cloud Hypervisor fork

## Status

**Accepted — decision RESET-EXIT approved by user 2026-10-11.** GH #295.
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN* (§ *VMM fork provisioning contract*). Depends on ADR-0146 (the VMM
opens its own vhost-vsock device) and ADR-0161 (the pinned fork build).

## Context

Every VM runs on the vendored Cloud Hypervisor fork (ADR-0161). In that fork
a guest reset does not end the VMM; it reboots the guest in place:

- Every reset source writes the VMM's `reset_evt`: a triple fault
  (`VcpuExit::Shutdown` becomes `VmExit::Reset`,
  `hypervisor/src/kvm/mod.rs:2530-2533`, written at `vmm/src/cpu.rs:1432-1436`),
  the i8042 and CMOS reset registers (`devices/src/legacy/i8042.rs:49`,
  `devices/src/legacy/cmos.rs:81`), the ACPI reset register
  (`devices/src/acpi.rs:65`) and the virtio watchdog
  (`virtio-devices/src/watchdog.rs:176`).
- The VMM's control loop answers `EpollDispatch::Reset` with `vm_reboot`
  (`vmm/src/lib.rs:2088-2094`), which drops the VM and builds and boots a new
  one in the same process (`vmm/src/lib.rs:2427-2495`), re-creating the vsock
  device, which reopens `/dev/vhost-vsock` and sets the same CID
  (`virtio-devices/src/vsock/vhost_kernel.rs:209-213`). Only a guest poweroff
  (ACPI S5, `devices/src/acpi.rs:82`) or a VMM exit event ends the process
  (`vmm/src/lib.rs:2079-2086`, `:2096-2106`).
- The guest kernel turns a PID-1 death into a reset: the exit of the last
  thread of global init panics (`kernel/exit.c:924-931 @ v7.0`), and under
  `panic=1`, which Overdrive's guest command line sets, the panic restarts
  the machine (`kernel/panic.c:718-742 @ v7.0`). PID 1 is `overdrive-init`.

V-30 (`spike/findings-v30-guest-reset.md`, 7.0.0-29, fork `41a619d19`)
measured the result: in 0 of 29 runs across seven reset cases did the VMM
exit; the guest returned with a new `boot_id` under the same VMM pid, CID and
launch token, and a guest whose PID 1 crashed on every boot rebooted 9 times
in 15 s with the VMM never exiting. A driver that watches for VMM exit cannot
see such a crash, a crash loop never charges a restart budget, and the
launch token on the kernel command line (ADR-0177) stays the same across the
in-place reboot. The fork has no flag or API that selects exit on reset.
V-30 also measured that the reboot closes the old vhost-vsock instance before
opening the new one, so host flows into the old incarnation are reset
(Q2a); flow hygiene is not the problem, observability and launch identity are.

## Decision

- **The fork ends the VMM on a guest reset.** The fork routes
  `EpollDispatch::Reset` to the VMM exit path instead of to `vm_reboot`.
  Every reset source therefore ends the VMM process; no guest runs a second
  incarnation inside one VMM.
- **A reset is classified as a crash by the driver's existing exit
  classification; no new exit status is introduced.** That a reset is a
  crash is the purpose of this decision (CLAUDE.md: a crash loop must charge
  the restart budget), and the existing classification already yields it. The
  driver never consults the VMM's exit status; the guest's own `EXIT <status>`
  report from `overdrive-init` is authoritative, and a VMM exit with no such
  report is a crash (ADR-0082 § D3;
  `crates/overdrive-worker/src/vm_driver.rs:2030-2042`, the no-report row at
  `:2041`). A reset produces no report: PID 1 died, or the machine reset
  without PID 1 reporting a workload exit. Routed to the fork's existing exit
  path, which ends the process with its ordinary status
  (`cloud-hypervisor/src/main.rs:960-968` in the fork), the reset therefore
  lands as the existing unreported-guest-exit start failure before READY and
  as an ordinary crash after READY.
- **The change lives in the fork.** It is a fork property carried by the
  pinned fork build (ADR-0161), not a driver-side behaviour. The driver gains
  no reset supervision; it sees the VMM's exit through its existing VMM-exit
  handling, before or after READY.
- **One incarnation per VMM process.** A VMM process runs exactly one guest
  boot. A guest that needs to start again is started by its lifecycle owner
  as a new launch.

## Alternatives considered

- **The driver supervises the VMM's `vm/rebooting` event and kills the VMM.**
  Worked in V-30 (30/30 kills landed before the reboot completed), but it is
  a race between the driver and the VMM's own reboot window (21-41 ms
  measured), and it puts a correctness obligation on an event stream.
  Rejected.
- **A once-only guard on a device descriptor handed to the VMM.** Ends the
  VMM only as a failed reboot (exit status 1), and requires a process other
  than the VMM to claim the device, whose descriptor copy keeps host flows
  ESTABLISHED to a dead guest (V-30 Q2c). Rejected (ADR-0146).
- **Accept in-place reboot.** A crash loop is invisible to the driver and the
  restart budget, and the launch token no longer identifies one guest boot.
  Rejected.
- **Drop `panic=1` from the guest command line.** A PID-1 crash then hangs the
  guest with no reset event: the VMM stays up and the driver sees nothing.
  Rejected.

## Consequences

- A guest PID-1 crash, triple fault, watchdog expiry or guest-requested
  reboot ends the VMM with no guest exit report. The driver observes a VMM
  exit classified as a crash; post-READY it counts against the workload's
  restart budget like any other crash.
- The VMM's exit releases its vhost-vsock device, which resets every host
  flow of that guest and frees the CID (ADR-0146). A guest whose PID 1
  crashed sends no `SHUTDOWN` for its flows; the device release is what ends
  them on the host (ADR-0180).
- The launch token stays one-per-VMM-process, which ADR-0177's attribution
  requires.
- The fork diverges from upstream Cloud Hypervisor in its reset handling; a
  rebase of the fork carries the change (ADR-0161).
- The rule is covered by the feature delta's § *Formal protocol model*.
- The change is untested: V-30's reading of it is a reading, not a result.
  Its validation (V-30-FORK in the feature delta's validation plan) is not
  contract-shaping and blocks DELIVER of the fork step only: the driver's
  classification depends on the absence of a guest report, not on any
  property of the patch, so a falsification changes the fork patch, not a
  contract DISTILL pins. The pinned fork build `41a619d19` still answers a
  reset with `vm_reboot` (`vmm/src/lib.rs:2088-2094`); the change is this
  ADR's, not yet the fork's.
