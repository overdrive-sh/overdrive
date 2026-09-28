# Spike A2, increment b: Nanos under Firecracker (boot + virtio-blk/-net/-vsock)

Throwaway probe for GH #303 (`in-guest-kernel-mtls`), the Firecracker counterpart
of Spike A (`../increment-a/`, Cloud Hypervisor). Tests one assumption, asked by
the user as "does Nanos work out of the box with Firecracker?".

Reuses Spike A's build tree on the metal host (`~/igkm-spike-a`): the stock and
patched Nanos kernels, the disk image with the probe program
(`nanos-vsock.img`, sha256 `27157a22…`) and the guest program itself. Nothing is
rebuilt. The guest program still prints `IGKM-A:` markers and uses the Spike A
REQUEST/RESPONSE strings; `host_listener.py` here is a copy of Spike A's.

Source anchors below: Firecracker `v1.17.0` (tag object `29d66eb2`, commit
`95f868c8`), Nanos `aad473aad2f73a9893d5c32be333f2796725ab82`.

## Pass 1 — written before any run (debugging.md § 4)

- **Hypothesis (the user's):** STOCK Nanos `aad473aa` (no patches) boots under
  Firecracker, and its virtio-blk root, virtio-net and virtio-vsock all work; a
  guest `AF_VSOCK` connect to CID 2 reaches the host on Firecracker's Unix socket.
- **Static read before running (which predicts the hypothesis FAILS):**
  - Firecracker v1.17.0 prefers the PVH entry whenever the ELF carries the
    `XEN_ELFNOTE_PHYS32_ENTRY` note (`firecracker:src/vmm/src/arch/x86_64/mod.rs:518-521`).
    PVH boot was added in Firecracker 1.12.0 (`CHANGELOG.md:476-480`, PR #5048).
    The Nanos `kernel.img` carries that note (`nanos:src/x86_64/crt0.s:393-401`,
    entry `0x2000dc`, Spike A run 0002).
  - For PVH, Firecracker sets only `rflags`, `rbx` and `rip`; RSP stays 0
    (`firecracker:src/vmm/src/arch/x86_64/regs.rs:88-93`). The Linux-boot arm
    sets `rsp = rbp = 0x8ff0` (`:95-108`). This is the same PVH register state
    Cloud Hypervisor v53.0 provides.
  - Nanos `pvh_start32` calls `pvh_zero_page` at `nanos:src/x86_64/init.s:116`
    before its first stack setup at `:142`. So the first `call` pushes to
    `0xfffffffc`, which on Firecracker is in the 32-bit MMIO gap, not RAM.
- **Prediction (stock kernel, Firecracker v1.17.0):** Firecracker logs
  `Kernel loaded using PVH boot protocol`; then an unhandled MMIO write and read
  at `0xfffffffc` (Firecracker logs both as `Invalid MMIO read @ 0xfffffffc:0x4`,
  because the write arm reuses the read message, `vstate/vcpu.rs:453-462`); no
  Nanos output on the serial console; the vCPU dies with an error exit
  (`KVM_EXIT_INTERNAL_ERROR`, or a triple-fault `Shutdown` exit) and Firecracker
  exits on its own with a non-zero code; the host listener reports
  `vsock=FAIL`, `net=FAIL`.
- **Falsification (of the prediction; = the user's hypothesis holds):** Nanos
  output appears on the serial console, the `IGKM-A:` markers print, and both
  legs pass.

## Pass 2 — patched kernel (planned if pass 1 fails as predicted)

Boot Spike A's `kernel-pvhfix.img` (one instruction, `mov esp, 0xa000`, first in
`pvh_start32`; `../increment-a/patches/0001-…`) under the same Firecracker config.

- **Hypothesis:** the PVH stack ordering is the only Firecracker boot blocker; with
  the patch the chain works as it did on Cloud Hypervisor.
- **Prediction:**
  - Nanos boots and runs `/vsock_probe` from the TFS root on virtio-blk.
  - Transport is virtio-MMIO: Firecracker without `--enable-pci` registers
    each device on MMIO and describes it in the DSDT as `LNRO0005`
    (`firecracker:src/vmm/src/device_manager/mmio.rs:92,254-280`); Nanos
    enumerates `LNRO0005` from ACPI (`nanos:src/drivers/acpi.c:455-457`,
    `src/virtio/virtio_mmio.c:95-99`). Under PVH Nanos has no command line
    (`nanos:platform/pc/service.c:436-446` passes no zero page), so ACPI is the
    only discovery path.
  - Firecracker vsock offers `VERSION_1 | IN_ORDER | EVENT_IDX` and no vsock
    feature bit (`firecracker:src/vmm/src/devices/virtio/vsock/device.rs:56-58`);
    as on CH this is not a blocker.
  - The guest connect to CID 2:5000 lands on `<uds_path>_5000`
    (`firecracker:src/vmm/src/devices/virtio/vsock/unix/muxer.rs:638`); exact
    REQUEST in, exact RESPONSE out; same for TCP over the tap. Both `PASS`.
  - **Power-off differs from CH.** Firecracker's DSDT has no `_S5` object (the
    only `_S5` in the tree is a unit test, `src/acpi-tables/src/aml.rs:1510`), so
    Nanos's `acpi_powerdown_init` returns early (`nanos:src/drivers/acpi.c:369-372`)
    and `vm_halt` stays unset. `vm_exit` then calls `vm_shutdown`
    (`nanos:src/kernel/init.c:945-949`) → `QEMU_HALT`: `out 0x501` (no device on
    Firecracker, logged as a failed IO write) then `out 0x64, 0xfe`
    (`nanos:src/kernel/kvm_platform.h:8,13`). Firecracker's i8042 turns the CPU
    reset into its exit event (`firecracker:src/vmm/src/devices/legacy/i8042.rs:258-266`)
    and the VMM stops with exit code 0 (`src/vmm/src/lib.rs:797-822`). Predicted:
    Firecracker exits on its own, code 0, shortly after `program exiting`;
    metrics show `i8042.reset_count = 1`.
- **Falsification:** another early crash, a device that never attaches (no traffic
  in the metrics, `vsock/net FAIL`), or Firecracker still running after 60 s.

## Pass 3 — optional, only if pass 1 fails at PVH entry

Firecracker ≤ 1.11 has no PVH support, so it would enter the stock ELF at its
`e_entry` (`_start`, `0x201d87`) through the Linux 64-bit boot path with
RSP = `0x8ff0` and RSI = the zero page. Nanos has a dedicated branch for exactly
that (`nanos:platform/pc/service.c:377-393`, "loaded directly by the hypervisor")
and carries Firecracker-specific quirks (`service.c:342`, "Firecracker v1.6.0").

- **Hypothesis:** stock Nanos works out of the box on Firecracker v1.11.0 (the last
  release before PVH).
- **Prediction:** no PVH path exists in that release (to be confirmed in its
  source before the run; the `Kernel loaded using …` debug line is itself part of
  the 1.12 PVH change, so it is not expected in the 1.11 log); the probe runs; both
  legs `PASS`; exit via i8042 as in pass 2. Because the stock PVH entry is proven
  broken by pass 1, a successful stock boot here can only have come through the
  64-bit `_start` entry.
- **Falsification:** no serial output or either leg fails.

## Files

| File | Role |
|---|---|
| `capture.sh` | Append-only capture of one `cargo xtask metal run` → `runs/NNNN.{meta,stdout,stderr}` (refuses overwrite; redacts host). Copied from Spike A. |
| `rsync-without-local-env.sh` | Keeps the workspace `.env` (SSH target) off the host during sync. Copied from Spike A. |
| `host-inventory.sh` | Substrate probes (testing.md, fail closed) + Firecracker / Spike A artifact inventory. Creates nothing. |
| `fetch-firecracker.sh` | Login user, no sudo: downloads the official static Firecracker release tarball(s) into `~/igkm-spike-a/firecracker/<ver>/`, verifies the published SHA-256, records the binary SHA-256 and `--version`. No jailer. |
| `run-boot-fc.sh` | Root: creates one tap (`igkmb0`), boots one Firecracker microVM from a JSON config (`--no-api`), runs the host listener, prints every log, cleans up and diffs host state. |
| `host_listener.py` | Host side (copy of Spike A's): Unix listener on `<uds_path>_5000`, TCP listener on the tap IP. |
| `host-state-check.sh` | Read-only end-of-probe check. |

Findings: `docs/feature/in-guest-kernel-mtls/spike/findings-firecracker.md`.

## Run log (append-only)

| Run | What | Result |
|---|---|---|
| 0001 | inventory (substrate probes, artifacts) | `SUBSTRATE OK`; Spike A artifacts unchanged (hashes match) |
| 0002 | fetch Firecracker v1.17.0 | published tarball checksum `OK`; binary sha256 `99ad0f5c…` |
| 0003 | stock `kernel.img` on Firecracker v1.17.0 | pass-1 prediction held: PVH route, write+read at `0xfffffffc`, `Unexpected exit reason on vcpu run: Shutdown`, Firecracker exit 1, no Nanos output, both legs `FAIL` |

After run 0003 the metrics section of `run-boot-fc.sh` was changed to print every
flushed metrics line (non-zero counters only) instead of the last line pretty-printed:
Firecracker metrics counters are per-flush deltas, so the last line alone can read
zero. Diagnostic output only; no change to the VM, config or cleanup.
| 0004 | patched `kernel-pvhfix.img` on Firecracker v1.17.0 | pass-2 prediction held: boots via PVH; virtio-MMIO devices from ACPI carry traffic; vsock + net `PASS`; Firecracker exits on its own, code 0, `i8042.reset_count = 1` |

### Pass 3 source confirmation (written after run 0004, before run 0005)

Firecracker `v1.11.0` (tag object `80ad6ebc`, commit `4e40479d`): no `pvh` anywhere
under `src/vmm/src`; `load_kernel` returns linux-loader's `kernel_load`, i.e. the
ELF `e_entry` (`src/vmm/src/builder.rs:565-589`); `setup_regs` always sets
`rsp = rbp = 0x8ff0`, `rsi = 0x7000` (`src/vmm/src/arch/x86_64/regs.rs:83-100`);
the zero page carries `boot_flag = 0xaa55` and `header = HdrS`
(`src/vmm/src/arch/x86_64/mod.rs:121-122,140-141`), which is exactly what Nanos's
direct-load branch checks (`nanos:platform/pc/service.c:377-378`). The JSON config
used for v1.17.0 is valid for v1.11.0 (`vsock_id` optional, `is_read_only`
`Option<bool>`). The pass-3 prediction stands unchanged.

| Run | What | Result |
|---|---|---|
| 0005 | fetch Firecracker v1.11.0 | published tarball checksum `OK`; binary sha256 `8f0ea0c5…` |
| 0006 | stock `kernel.img` on Firecracker v1.11.0 | **harness error, no guest verdict**: v1.11.0 opens `log_path` without `O_CREAT` (`src/vmm/src/vmm_config/mod.rs:171-177`) → `Logger error: Failed to open target file`, Firecracker exit 1 before building the VM. `run-boot-fc.sh` now pre-creates `fc.log`/`fc.metrics`; re-run as 0007 |
| 0007 | stock `kernel.img` on Firecracker v1.11.0 (retry of 0006) | pass-3 prediction held: stock kernel boots (only possible via `_start`, since the PVH entry is broken and 1.11 has no PVH path); vsock + net `PASS`; exits on its own, code 0, `i8042.reset_count = 1` |
| 0008 | end-of-probe host-state check | no firecracker/jailer/listener processes, no tap, no `192.168.203.x`, no sockets, no run dirs, no root-owned files; retained user-owned tree 502 MB (58 MB of it the two Firecracker releases) |
