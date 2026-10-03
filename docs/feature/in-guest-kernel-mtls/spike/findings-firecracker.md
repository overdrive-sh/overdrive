# Spike A2 findings: Nanos under Firecracker (boot + virtio-blk/-net/-vsock)

GH #303, feature `in-guest-kernel-mtls`. PROBE phase, throwaway. This is the Firecracker
counterpart of Spike A (`findings.md`, Cloud Hypervisor). Probe scripts and raw captures:
`spike-scratch/in-guest-kernel-mtls/increment-b/` (runs `runs/0001`–`runs/0008`, each
`{meta,stdout,stderr}`, append-only, host identifiers redacted). The Spike A build tree on the
metal host was reused; nothing was rebuilt.

## Does Nanos work out of the box with Firecracker?

**Not with current Firecracker.** Stock Nanos `aad473aa` does not boot under Firecracker
v1.17.0, the latest release. It fails the same way it fails under Cloud Hypervisor, and the
cause is the same Nanos defect. Firecracker added PVH boot in 1.12.0 and prefers PVH whenever
the kernel ELF has a PVH note, which the Nanos kernel does.

**It did work with older Firecracker.** On Firecracker v1.11.0, the last release before PVH
support, stock Nanos boots with no changes and everything works. That release enters the ELF
at `_start` through the Linux 64-bit boot path, which sets a stack, and Nanos has a dedicated
branch for that path.

**With Spike A's one-instruction PVH fix, Nanos works fully on Firecracker v1.17.0.** Unlike
Cloud Hypervisor, Firecracker also exits on its own when the guest program exits.

## Verdict

| Part | FC v1.17.0, stock Nanos | FC v1.17.0, Nanos + PVH stack patch | FC v1.11.0, stock Nanos |
|---|---|---|---|
| Boot | **DOESN'T WORK**: triple fault at the first `call` in the PVH entry stub; Firecracker exits 1 | **WORKS** | **WORKS** |
| virtio-blk root image | not reached | **WORKS** | **WORKS** |
| virtio-net | not reached | **WORKS** (TCP both ways over the tap) | **WORKS** |
| vsock guest → host | not reached | **WORKS** (guest `AF_VSOCK` connect to CID 2:5000 lands on `<uds_path>_5000`, bytes both ways) | **WORKS** |
| Power-off on program exit | n/a | **WORKS**: Firecracker exits on its own with code 0 | **WORKS**, same |
| Boot protocol | PVH | PVH | Linux 64-bit boot at ELF `e_entry` |
| RSP at entry | 0 | 0 | `0x8ff0` |
| virtio transport | — | virtio-MMIO, discovered from ACPI `LNRO0005` | virtio-MMIO |

Substrate: host `uname -r` = `7.0.0-29-generic` (x86_64, AMD EPYC 8024P, `systemd-detect-virt`
= `none`, KVM API 12). Guest: Nanos `aad473aad2f73a9893d5c32be333f2796725ab82`.

## Assumption under test (written before any run)

The full pre-run text, with source anchors, is in the increment `README.md` (Pass 1).

- **Hypothesis (the user's):** stock Nanos `aad473aa` boots under Firecracker, and its
  virtio-blk root, virtio-net and virtio-vsock all work. A guest `AF_VSOCK` connect to CID 2
  reaches the host on Firecracker's Unix socket.
- **Prediction, from reading the source before running: the hypothesis fails.**
  - Firecracker v1.17.0 prefers PVH for an ELF with the PVH note
    (`firecracker:src/vmm/src/arch/x86_64/mod.rs:518-521`).
  - For PVH it sets only `rflags`, `rbx` and `rip`, so RSP is 0
    (`firecracker:src/vmm/src/arch/x86_64/regs.rs:88-93`).
  - Nanos `pvh_start32` calls before it sets a stack (`nanos:src/x86_64/init.s:116` vs `:142`).
  - Expected symptoms: an unhandled MMIO write and read at `0xfffffffc`; no serial output; an
    error vCPU exit; Firecracker exits non-zero on its own; both legs `FAIL`.
- **Falsification:** Nanos output on the console, `IGKM-A:` markers, and both legs `PASS`.

Passes 2 (patched kernel) and 3 (Firecracker v1.11.0) were each predicted in the README before
their run.

## Predicted vs actual

| Prediction | Actual | Evidence |
|---|---|---|
| Firecracker v1.17.0 takes the PVH route | Yes: `Kernel loaded using PVH boot protocol` | runs 0003, 0004 |
| Stock Nanos crashes at the first `call` (RSP = 0) | Yes: the write and read at `0xfffffffc`, then `Unexpected exit reason on vcpu run: Shutdown` (a triple fault). Firecracker exited with code 1, about 32 ms after its start banner. | run 0003 |
| No Nanos output; both legs FAIL | Yes | run 0003 |
| With the patch, the stack ordering is the only blocker | Yes. Boot, mount root, run the program. | run 0004 |
| Devices use virtio-MMIO, discovered from ACPI (`LNRO0005`) | Yes: three `_SB_.V00x` MMIO devices in the DSDT, MMIO transport activations, no PCI | run 0004 |
| Guest vsock connect lands on `<uds_path>_5000` | Yes | runs 0004, 0007 |
| Exact REQUEST in, exact (different) RESPONSE out, vsock and TCP | Yes, on host and guest | runs 0004, 0007 |
| No `_S5` → Nanos falls back to `QEMU_HALT` → i8042 reset → Firecracker exits 0 | Yes: Firecracker exited on its own, code 0; `i8042.reset_count = 1`. The preceding `out 0x501` was not seen in the log (see edge case 2). | runs 0004, 0007 |
| Firecracker v1.11.0 has no PVH path; stock Nanos boots there | Yes: source has no PVH; the stock kernel booted and both legs passed | source + run 0007 |

## Evidence

### Substrate probes (run 0001)

    uname -m: x86_64
    uname -r: 7.0.0-29-generic
    systemd-detect-virt: none (exit 1)
    cpu vmx/svm flag count: 16
    crw-rw---- 1 root kvm 10, 232 Sep 28 06:42 /dev/kvm
    KVM_GET_API_VERSION=12 KVM_CREATE_VM=ok
    model name	: AMD EPYC 8024P 8-Core Processor
    ...
    firecracker on PATH: MISSING
    ...
    SUBSTRATE OK

Run 0001 also confirmed that the reused Spike A artifacts still have Spike A's hashes. The
`cargo xtask metal run` fail-closed preflight passed on every run.

### Firecracker binaries (runs 0002, 0005)

These are the official static release tarballs from GitHub releases. Each download was checked
against the release's published `.sha256.txt` before extraction. That file comes from the same
release page, so it proves the download is intact, not who published it. The jailer was never
used.

    firecracker-v1.17.0-x86_64.tgz: OK
    99ad0f5cd0514a88aad0e9ae8cfdb3cc3b4ab9d190e1194602406c786b5de7a5  .../firecracker-v1.17.0-x86_64
    Firecracker v1.17.0

    firecracker-v1.11.0-x86_64.tgz: OK
    8f0ea0c508d690b288079709830ca6aa037f75cea3dc9ddd48b2aa0ab0b448d5  .../firecracker-v1.11.0-x86_64
    Firecracker v1.11.0

### Stock Nanos on Firecracker v1.17.0: PVH boot fails (run 0003)

Firecracker log (Debug):

    06:43:02.119815153 [igkm-b:main:DEBUG:src/vmm/src/arch/x86_64/mod.rs:524] Kernel loaded using PVH boot protocol
    06:43:02.120934678 [igkm-b:main:INFO:src/firecracker/src/main.rs:610] Successfully started microvm that was configured from one single json
    06:43:02.121098495 [igkm-b:fc_vcpu 0:WARN:src/vmm/src/vstate/vcpu.rs:457] Invalid MMIO read @ 0xfffffffc:0x4: Failed to find address range.
    06:43:02.121174689 [igkm-b:fc_vcpu 0:WARN:src/vmm/src/vstate/vcpu.rs:447] Invalid MMIO read @ 0xfffffffc:0x4: Failed to find address range.
    06:43:02.121233478 [igkm-b:fc_vcpu 0:ERROR:src/vmm/src/arch/x86_64/vcpu.rs:782] Unexpected exit reason on vcpu run: Shutdown
    06:43:02.147128392 [igkm-b:main:ERROR:src/firecracker/src/main.rs:113] Firecracker exiting with error. exit_code=1

Runner and host listener:

    firecracker exited on its own: exit=1 at t+0.136s (<=0.05s poll)
    HOST: vsock-leg NO CONNECTION before deadline t+60.114323s
    HOST: VERDICT vsock=FAIL t+60.114554s
    HOST: VERDICT net=FAIL t+60.114567s

The only line on Firecracker's stdout, which carries the guest serial console, was
Firecracker's own banner `Running Firecracker v1.17.0`. Nanos printed nothing.

**Mechanism.** It is the same as Spike A run 0003.
- The first line comes from the MMIO *write* arm (`vstate/vcpu.rs:453-462`, which reuses the
  "read" message). The second comes from the read arm (`:442-451`). Together they are the
  `call`/`ret` pair running with ESP = 0.
- Firecracker returns zeros for an unhandled read (`data.fill(0)`, `:443`), so `ret` jumps to
  address 0.
- The vCPU then triple-faults (`KVM_EXIT_SHUTDOWN`). Cloud Hypervisor reported
  `InternalError` at this point instead.
- As on CH, the root cause is Nanos's `pvh_start32`. It calls `pvh_zero_page` at
  `nanos:src/x86_64/init.s:116`, `:121` and `:129`, but first loads ESP at `:142`. The PVH ABI
  leaves ESP unspecified, and Firecracker leaves it at 0:

      BootProtocol::PvhBoot => kvm_regs {
          // Configure regs as required by PVH boot protocol.
          rflags: 0x0000_0000_0000_0002u64,
          rbx: super::layout::PVH_INFO_START,
          rip: entry_point.entry_addr.raw_value(),
          ..Default::default()
      },

  (`firecracker:src/vmm/src/arch/x86_64/regs.rs:88-93` at v1.17.0.) The Linux-boot arm, by
  contrast, sets `rsp`/`rbp` to `BOOT_STACK_POINTER` (`0x8ff0`) and `rsi` to the zero page
  (`:95-108`).

**Why Firecracker picks PVH.** `load_kernel` falls back to Linux boot only when the ELF has no
PVH note:

    if let PvhBootCapability::PvhEntryPresent(pvh_entry_addr) = elf_result.pvh_boot_cap {
        // Use the PVH kernel entry point to boot the guest
        entry_point_addr = pvh_entry_addr;
        boot_prot = BootProtocol::PvhBoot;
    }

(`firecracker:src/vmm/src/arch/x86_64/mod.rs:518-521`.) Nanos always emits the note
(`nanos:src/x86_64/crt0.s:393-401`). The run 0003 header shows
`Xen 0x00000004 Unknown note type: (0x00000012)`, `description data: dc 00 20 00` (entry
`0x2000dc`), and ELF `Entry point address: 0x201d87`. Firecracker has no option to force Linux
boot for an ELF that has the note. PVH boot arrived in Firecracker 1.12.0 (`CHANGELOG.md:476-480`,
PR #5048), so every Firecracker from 1.12.0 on should behave this way. Only v1.17.0 was run.

### Patched Nanos on Firecracker v1.17.0: everything works (run 0004)

Kernel: Spike A's `kernel-pvhfix.img` (sha256 `792bf6e7…`, the same bytes as Spike A). It is
`aad473aa` plus `mov esp, 0xa000` as the first instruction of `pvh_start32`.

Guest serial console (Firecracker stdout):

    [0.022340] NET: static IP config for interface en1:
    [0.024456] en1: assigned 192.168.203.2
    [0.025995] gitversion: aad473aad2f73a9893d5c32be333f2796725ab82
    IGKM-A: BOOT-MARKER guest program started t=0.029227
    IGKM-A: vsock socket(AF_VSOCK=40, SOCK_STREAM) t=0.030396
    IGKM-A: vsock connected local cid=4294967295 port=1024 -> cid=2 port=5000 t=0.031549
    IGKM-A: vsock wrote 33 bytes t=0.033017
    IGKM-A: vsock read 34 bytes: IGKM-A-VSOCK-RESPONSE host->guest
    IGKM-A: VERDICT vsock=PASS
    IGKM-A: net connected local 192.168.203.2:60079 -> 192.168.203.1:5001 t=0.036168
    IGKM-A: net read 32 bytes: IGKM-A-NET-RESPONSE host->guest
    IGKM-A: VERDICT net=PASS
    IGKM-A: program exiting t=0.039799

Host listener (Unix socket `<run dir>/vsock.sock_5000`; TCP `192.168.203.1:5001`):

    HOST: vsock-leg accepted connection peer='' t+0.095615s
    HOST: vsock-leg received 33 bytes: b'IGKM-A-VSOCK-REQUEST guest->host\n' t+0.097059s
    HOST: vsock-leg replied 34 bytes: b'IGKM-A-VSOCK-RESPONSE host->guest\n' t+0.097075s
    HOST: net-leg accepted connection peer=('192.168.203.2', 60079) t+0.100217s
    HOST: VERDICT vsock=PASS t+0.101689s
    HOST: VERDICT net=PASS t+0.101693s

Firecracker's own view of the vsock handshake:

    muxer.send[rxq.len=0]: VsockPacketHeader { src_cid: 3, dst_cid: 2, src_port: 1024, dst_port: 5000, len: 0, type_: 1, op: 1, ... }
    vsock muxer: RX pkt: VsockPacketHeader { src_cid: 2, dst_cid: 3, src_port: 5000, dst_port: 1024, len: 0, type_: 1, op: 2, ... }
    muxer.send[rxq.len=0]: VsockPacketHeader { src_cid: 3, dst_cid: 2, src_port: 1024, dst_port: 5000, len: 33, type_: 1, op: 5, ... }
    vsock muxer: RX pkt: VsockPacketHeader { src_cid: 2, dst_cid: 3, src_port: 5000, dst_port: 1024, len: 34, type_: 1, op: 5, ... }

- The first packet is the guest's `OP_REQUEST` from CID 3 to CID 2, port 1024 → 5000.
  Firecracker answers `OP_RESPONSE`, then 33 bytes go up and 34 bytes come down.
- The connection went through Firecracker's `<uds_path>_<port>` convention:
  `let port_path = format!("{}_{}", self.host_sock_path, pkt.hdr.dst_port());`
  (`firecracker:src/vmm/src/devices/virtio/vsock/unix/muxer.rs:638`), the same convention CH
  uses.

**Transport: virtio-MMIO, discovered from ACPI.** Firecracker was started without
`--enable-pci`. It described three MMIO devices in the DSDT:

    acpi: Building AML for VirtIO device _SB_.V000. memory range: 0xc0001000:4096 gsi: 5
    acpi: Building AML for VirtIO device _SB_.V001. memory range: 0xc0002000:4096 gsi: 6
    acpi: Building AML for VirtIO device _SB_.V002. memory range: 0xc0003000:4096 gsi: 7

When Nanos set `DRIVER_OK`, the MMIO transport activated each device. The `notifying queues`
line is emitted right after a successful activation, and its caller is
`firecracker:src/vmm/src/devices/virtio/transport/mmio.rs:214`:

    [Block:rootfs] notifying queues
    [Vsock:vsock] notifying queues
    [Net:net0] notifying queues

The vCPU metrics show `exit_mmio_read = 132` and `exit_mmio_write = 137` (virtio-MMIO register
traffic). On the Nanos side, under PVH `init_service` gets no zero page, so no command line
(`nanos:platform/pc/service.c:436-446`). `virtio_mmio_enum_devs` therefore found the devices
only through ACPI `LNRO0005` (`nanos:src/virtio/virtio_mmio.c:95-99`,
`src/drivers/acpi.c:455-457`). ACPI worked: Nanos found Firecracker's RSDP at `0xe0000` with
its first BIOS-area scan (`nanos:src/x86_64/acpi.c:35-36,57-64`). Nanos also probed PCI config
space, found nothing, and carried on (edge case 1).

Per-device traffic (Firecracker metrics, flushed at exit):

    block_rootfs.read_count = 53    block_rootfs.read_bytes = 1278976   block_rootfs.write_count = 1
    net_net0.tx_packets_count = 7   net_net0.rx_packets_count = 7
    vsock.conns_added = 1   vsock.tx_bytes_count = 33   vsock.rx_bytes_count = 34
    i8042.reset_count = 1   i8042.write_count = 1

What each line of evidence shows:
- **virtio-blk root**: 53 reads (1.28 MB). `/vsock_probe` exists only in the image's TFS root
  partition, and it ran.
- **virtio-net**: `en1` got its address, and TCP data flowed both ways.
- **vsock**: the connection arrived on the Unix socket, and data flowed both ways.

**The guest powers off and Firecracker exits on its own:**

    IGKM-A: program exiting t=0.039799
    06:44:49.099106959 [igkm-b:main:INFO:src/vmm/src/lib.rs:693] Vmm is stopping.
    06:44:49.115495646 [igkm-b:main:INFO:src/firecracker/src/main.rs:116] Firecracker exiting successfully. exit_code=0
    firecracker exited on its own: exit=0 at t+0.135s (<=0.05s poll)

The chain, with each step's source:
- Firecracker's DSDT has no `_S5` object (the only `_S5` in the Firecracker tree is a unit test
  in `src/acpi-tables/src/aml.rs:1510`).
- So Nanos's `acpi_powerdown_init` fails to evaluate `\_S5` and returns
  (`nanos:src/drivers/acpi.c:369-372`) without setting `vm_halt`.
- `vm_exit` therefore calls `vm_shutdown` (`nanos:src/kernel/init.c:945-949`), which is
  `QEMU_HALT`: `out 0x501, code`, then `out 0x64, 0xfe` (`nanos:src/kernel/kvm_platform.h:8,13`).
- Firecracker's i8042 turns `0xfe` on port `0x64` into its VMM exit event
  (`firecracker:src/vmm/src/devices/legacy/i8042.rs:258-266`).
- The VMM stops with `FcExitCode::Ok` (`src/vmm/src/lib.rs:797-822`).

The metrics confirm exactly one i8042 write, and it was the reset. This differs from Spike A:
CH gives Nanos an `_S5` package, so Nanos takes its PM1 path, the writes go to port 0, and the
guest idles forever.

### Stock Nanos on Firecracker v1.11.0 works out of the box (run 0007)

Firecracker v1.11.0 source (commit `4e40479d`):
- There is no `pvh` anywhere under `src/vmm/src`.
- `load_kernel` returns linux-loader's `kernel_load`, which is the ELF `e_entry`
  (`src/vmm/src/builder.rs:565-589`).
- `setup_regs` always sets `rsp = rbp = 0x8ff0` and `rsi = 0x7000`
  (`src/vmm/src/arch/x86_64/regs.rs:83-100`).
- The zero page carries `boot_flag = 0xaa55` and `header = HdrS`
  (`src/vmm/src/arch/x86_64/mod.rs:121-122,140-141`). That is exactly what Nanos's
  direct-load branch in `init_service` checks (`nanos:platform/pc/service.c:377-378`), reached
  from `_start` (`nanos:src/x86_64/crt0.s:354-357`).

Serial and listener, same stock `kernel.img` (sha256 `c99b7da3…`) as run 0003:

    [0.022699] gitversion: aad473aad2f73a9893d5c32be333f2796725ab82
    IGKM-A: BOOT-MARKER guest program started t=0.026636
    IGKM-A: VERDICT vsock=PASS
    IGKM-A: VERDICT net=PASS
    IGKM-A: program exiting t=0.037060
    HOST: VERDICT vsock=PASS t+0.100628s
    HOST: VERDICT net=PASS t+0.100633s
    firecracker exited on its own: exit=0 at t+0.137s (<=0.05s poll)

The metrics again show `i8042.reset_count = 1`. v1.11.0 put the MMIO devices at
`0xd0000000`–`0xd0002000` (IRQs 5–7) and also passes them on the kernel command line
(`src/vmm/src/device_manager/mmio.rs:256`). Nanos, booted through the zero page, reads both
sources (`virtio_mmio.c:97-98`). `vtmmio_probe_devs` skips a device whose status already has
`DRIVER` set (`:112`), so a second registration of the same device would be ignored. That
duplicate case was not observed directly. Because run 0003 proves the stock PVH entry is
broken, and v1.11.0 has no PVH path, this boot can only have come through `_start`.

Run 0006 was a harness failure, not a guest result. v1.11.0 opens `log_path` without
`O_CREAT` (`src/vmm/src/vmm_config/mod.rs:171-177`), so it died with
`Logger error: Failed to open target file` before building the VM. The runner now pre-creates
the log and metrics files, and run 0007 is the retry.

## Exact commands and configuration

Each run went through `spike-scratch/in-guest-kernel-mtls/increment-b/capture.sh`, which wraps
`cargo xtask metal run` (rsync with the `.env` guard, shared lease, fail-closed preflight) and
writes `runs/NNNN.*`:

    capture.sh 0001 inventory --no-sudo -- bash .../host-inventory.sh
    capture.sh 0002 fetch-fc-v1.17.0 --no-sudo -- bash .../fetch-firecracker.sh v1.17.0
    capture.sh 0005 fetch-fc-v1.11.0 --no-sudo -- bash .../fetch-firecracker.sh v1.11.0
    # boots (root); OVERDRIVE_METAL_KERNEL=<kernel> OVERDRIVE_METAL_ROOTFS=<metal-home>/igkm-spike-a/out/nanos-vsock.img
    capture.sh 0003 boot-stock-fc1.17.0  -- bash .../run-boot-fc.sh <fc v1.17.0> <metal-home>/igkm-spike-a/nanos/output/platform/pc/bin/kernel.img
    capture.sh 0004 boot-pvhfix-fc1.17.0 -- bash .../run-boot-fc.sh <fc v1.17.0> <metal-home>/igkm-spike-a/out/kernel-pvhfix.img
    capture.sh 0007 boot-stock-fc1.11.0-retry -- bash .../run-boot-fc.sh <fc v1.11.0> <metal-home>/igkm-spike-a/nanos/output/platform/pc/bin/kernel.img
    capture.sh 0008 host-state-check --no-sudo -- bash .../host-state-check.sh

Tap (the only host network change, made per boot run and deleted in cleanup):

    ip tuntap add dev igkmb0 mode tap
    ip addr add 192.168.203.1/24 dev igkmb0
    ip link set igkmb0 up
    ...
    ip link del igkmb0

Firecracker command line (no jailer, no API socket; stdin `/dev/null`; stdout, which carries
the guest serial console, redirected to `serial.log`):

    firecracker-v1.17.0-x86_64 --no-api --config-file <run dir>/vm.json --id igkm-b

`vm.json` (identical for every boot run except `kernel_image_path`; valid for v1.11.0 too):

    {
      "boot-source": { "kernel_image_path": "<kernel>" },
      "drives": [ { "drive_id": "rootfs", "path_on_host": "<run dir>/disk.img",
                    "is_root_device": false, "is_read_only": false } ],
      "machine-config": { "vcpu_count": 1, "mem_size_mib": 512 },
      "network-interfaces": [ { "iface_id": "net0", "guest_mac": "52:54:00:1a:03:02",
                                "host_dev_name": "igkmb0" } ],
      "vsock": { "vsock_id": "vsock0", "guest_cid": 3, "uds_path": "<run dir>/vsock.sock" },
      "logger": { "log_path": "<run dir>/fc.log", "level": "Debug",
                  "show_level": true, "show_log_origin": true },
      "metrics": { "metrics_path": "<run dir>/fc.metrics" }
    }

No `boot_args` were set, so Firecracker's default command line applies. Under PVH, Nanos does
not read the command line. `disk.img` is a per-run copy of the Spike A image. The guest IP comes
from the Nanos manifest, not the command line.

## Artifact versions

| Artifact | Identity |
|---|---|
| Firecracker (primary) | `v1.17.0`, official static release. Tarball sha256 `06094a1108ae9e82aa4c23a775aa92758f53f1175d422270d9d6162cb9ade558` (matches the published `.sha256.txt`). Binary sha256 `99ad0f5cd0514a88aad0e9ae8cfdb3cc3b4ab9d190e1194602406c786b5de7a5`. Source anchors at tag `v1.17.0` (tag object `29d66eb2`, commit `95f868c8e345b1cc8faccd1a3c910b4989dc3f58`). |
| Firecracker (pass 3) | `v1.11.0`. Tarball sha256 `38ad6fb34273b2fa616956237b15ea6e064cf21336b0d990d5de347b35b9328b`. Binary sha256 `8f0ea0c508d690b288079709830ca6aa037f75cea3dc9ddd48b2aa0ab0b448d5`. Source at commit `4e40479d1533a4e8e5aca3430f736474d7868bf4` (tag object `80ad6ebc`). |
| Nanos source | `aad473aad2f73a9893d5c32be333f2796725ab82`, clean tree (Spike A build, reused) |
| `kernel.img` (stock) | sha256 `c99b7da38017ee74d5e1999ae1f76654420ffca859972a25a1637aa87ef9f521` |
| `kernel-pvhfix.img` | sha256 `792bf6e7fa5b190d435d229a7c07608b783f3388cf68897aab9b7f86870f259d` |
| Disk image | sha256 `27157a22bd2ceb9d0167248b3b04bb68add394368c2840e2f8ffce75a542beec` (the same image as every Spike A boot) |
| Guest program | sha256 `d541c4a7e452e23a2021e31a08a44503df8ff7c0b5a590c6757a653dec39992a` (Spike A's `vsock_probe`) |
| Host kernel | `7.0.0-29-generic`, not modified |

## Comparison with Spike A (Cloud Hypervisor v53.0)

| Aspect | Cloud Hypervisor v53.0 (Spike A) | Firecracker v1.17.0 | Firecracker v1.11.0 |
|---|---|---|---|
| Route for the Nanos ELF | PVH only: an ELF without the note is refused (`KernelMissingPvhHeader`, `cloud-hypervisor:vmm/src/vm.rs:1657-1675` at v53.0) | PVH (preferred whenever the note exists) | Linux 64-bit boot at `e_entry` (`_start`) |
| RSP at entry | 0 (`regs.rs:100-104`) | 0 (`regs.rs:88-93`) | `0x8ff0` (`regs.rs:83-100`) |
| Stock Nanos | **fails**: `InternalError` after the `0xfffffffc` write/read | **fails**: triple-fault `Shutdown` after the `0xfffffffc` write/read; Firecracker exit 1 | **works** |
| Nanos + PVH stack patch | works | works | not tested (not needed) |
| virtio transport | virtio-PCI | virtio-MMIO via ACPI `LNRO0005` (PCI off by default; `--enable-pci` not tested) | virtio-MMIO (ACPI + command line) |
| vsock features offered | `VERSION_1`, `IN_ORDER` | `VERSION_1`, `IN_ORDER`, `EVENT_IDX` (`vsock/device.rs:56-58`) | `VERSION_1`, `IN_ORDER` |
| vsock host endpoint | `<socket>_<port>` Unix listener | `<uds_path>_<port>` Unix listener (`muxer.rs:638`) | same |
| Tap | CH creates and removes it | caller must create and delete it | same |
| ACPI RSDP | found by the EBDA fallback | `0xe0000`, found by the first BIOS-area scan | same |
| Guest power-off on exit | **no**: `_S5` present → PM1 writes to port 0, VMM keeps running | **yes**: no `_S5` → `QEMU_HALT` → i8042 reset → VMM exits 0 | **yes**, same |
| One-sample timing | VMM start → `vm booted` 21.8 ms; program start at guest 51.3 ms | FC "started microvm" → vsock `OP_REQUEST` 27.9 ms; program start at guest 29.2 ms; FC process 58 ms end to end | vsock `OP_REQUEST` 27.7 ms after start; program start at guest 26.6 ms |

The timings come from one sample each. They are not a benchmark and not comparable across VMMs
beyond order of magnitude.

## Edge cases observed

1. **Nanos probes PCI config space on a Firecracker without PCI.** Nanos runs `pci_discover`
   before virtio-MMIO enumeration. On v1.17.0 each unanswered `0xcf8`/`0xcfc` access is logged
   (`vcpu: IO write @ 0xcf8:0x4 failed: Failed to find address range.`, `arch/x86_64/vcpu.rs:764,774`):
   - 20 such warnings were logged, and Firecracker's log rate limiter dropped another 1521
     lines (`logger.rate_limited_log_count = 1521`).
   - The run has `vcpu.exit_io_in = 1608` and `exit_io_out = 1616`.
   - v1.11.0 drops unmatched port I/O silently (`src/vmm/src/vstate/vcpu/x86_64.rs:634-649`).

   This is harmless for boot, but the probes flood a v1.17 log.
2. **The `out 0x501` in `QEMU_HALT` was not seen directly.** Firecracker has no device on
   `0x501`. On v1.17.0 the write would log a failed IO write, but that line was probably among
   the 1521 rate-limited lines. On v1.11.0 it is silently dropped. The claim that Nanos went
   through `QEMU_HALT` rests on the rest of the chain: exactly one i8042 write, which was the
   reset, and a clean `exit_code=0`.
3. **The guest's exit code is not visible to the host.** Firecracker ends with `FcExitCode::Ok`
   after any i8042 reset, and it has no `isa-debug-exit` device. So Firecracker's exit status
   does not reflect the program's exit status. This is inferred from source. The probe program
   exited 0 in both passing runs, and a failing program was not tested.
4. **Vsock close differs slightly from CH.**
   - After the host closed, Firecracker sent `OP_SHUTDOWN` (flags 3).
   - The guest sent one `OP_RST`, then 34 `OP_CREDIT_UPDATE`s, one per 1-byte `read()` (Spike A
     edge case 3, `nanos:src/virtio/virtio_socket.c:479-485`).
   - Firecracker sent nothing back, where CH answered each update with `OP_RST`. The
     host-to-guest packets were only `OP_RESPONSE`, `OP_RW` and `OP_SHUTDOWN` (runs 0004 and
     0007).
   - The guest still read all 34 bytes.
5. **`getsockname` reports local CID `4294967295` (`VMADDR_CID_ANY`), not 3.** This is the
   same as Spike A edge case 2. On the wire the guest sends `src_cid: 3`.
6. **Firecracker writes its own banner onto the stdout that carries the guest console.** The
   first line of `serial.log` is `Running Firecracker v1.17.0`. A driver that captures the
   console from Firecracker's stdout must expect Firecracker lines mixed in.
7. **Firecracker v1.11.0 needs the log and metrics files to exist; v1.17.0 creates them.** This
   was the run 0006 harness failure.
8. **Cosmetic, the same as Spike A:** Nanos prints the netmask and gateway as the address
   (`192.168.203.2`), and it starts DHCPv6 because no `ip6addr` is set.

## Design implications

**For GH #303 (in-guest mTLS on Nanos):**
- The host channel #303 needs works on Firecracker exactly as on CH. A guest `AF_VSOCK`
  connect to CID 2 port N reaches a host process listening on `<uds_path>_N`, as an ordinary
  Unix-socket accept. The host-side agent design from Spike A carries over unchanged: a per-VM
  Unix listener at a path Overdrive chooses.
- No vsock feature negotiation problem on either VMM.
  - Firecracker additionally offers `EVENT_IDX`.
  - Nanos's MMIO negotiation keeps only its requested mask, `F_STREAM | VERSION_1`
    (`nanos:src/virtio/virtio_mmio.c:130,144`, `virtio_socket.c:51`). By source reading that
    leaves `VERSION_1`, as on CH. The negotiated value was not captured on Firecracker.
  - The device attached and carried traffic.
- The relay should still use large reads. Nanos sends a credit update per `read()` on both
  VMMs (edge case 4).

**For an Overdrive Nanos VM driver:**
- **The PVH stack fix is needed on every current VMM.** The VMMs tested, CH v53.0 and
  Firecracker ≥ 1.12, enter PVH with RSP = 0. Stock Nanos `aad473aa` cannot boot through its
  PVH entry on either. The one-instruction fix from Spike A is sufficient on Firecracker as
  well. It should be carried in the #259 image factory or upstreamed to nanovms/nanos.
- **Pinning an old Firecracker is not a real alternative.** Stock Nanos works on Firecracker
  1.11.0 only because that release predates PVH. Pinning it would freeze the VMM at a March
  2025 release.
- **Removing the PVH note would, by source reading, make current Firecracker use the Linux
  boot path.** In that case Firecracker enters `_start` with a stack, and Nanos has a
  Firecracker-tested branch for that. This option was **not tested**. It would also break
  Cloud Hypervisor, which refuses an ELF kernel without the PVH note
  (`cloud-hypervisor:vmm/src/vm.rs:1657-1675`, v53.0). The source fix is the one that works on
  both VMMs.
- **Exit detection works on Firecracker, not on CH.**
  - On Firecracker the guest program's exit ends the Firecracker process with code 0, through
    the i8042 reset.
  - A driver can treat "Firecracker exited 0" as "guest halted".
  - It cannot read the guest's exit status from it (edge case 3). The status needs another
    channel, such as vsock or serial.
  - On CH, exit detection still needs the Nanos ACPI fix or another signal (Spike A edge case 1).
- **The device model differs by VMM.** On CH, Nanos uses virtio-PCI. On Firecracker, by
  default, it uses virtio-MMIO discovered from ACPI. Both worked here. A driver that targets
  both exercises both Nanos transports. Firecracker v1.17.0's `--enable-pci` was not tested.
- **The driver owns the tap on Firecracker.** Firecracker only opens an existing tap by name.
  Creating it, addressing it and deleting it are the caller's job; CH did all three itself.
- Direct boot on Firecracker needs the same things as on CH:
  - `kernel_image_path` pointing at the Nanos ELF.
  - A disk image built with `boot.img`, which provides the MBR partition table Nanos uses to
    find root (Spike A edge case 4).

## Host state created and cleanup

- **Per boot run:**
  - one tap `igkmb0` (`192.168.203.1/24`), created with `ip tuntap add` and deleted in cleanup
  - one `firecracker` process (no jailer, no API socket)
  - one `python3` listener
  - Unix sockets, the JSON config, logs, metrics and a disk-image copy in a per-run directory
    under `~/igkm-spike-a/runs-fc/`

  Every run's cleanup printed `links added: (none)`, `links removed: (none)`,
  `firecracker processes before: [] after: []`, `tap igkmb0 present: no`,
  `192.168.203.x on host: 0` and `run dir present: no`.
- **Run 0008 re-checked independently:** no firecracker, jailer or listener processes, no tap
  (`igkmb0` or `igkma0`), no tun/tap links, no `192.168.203.x` address, no Unix sockets, no
  per-run directories, and no root-owned files in the scratch tree.
- **Host kernel:** untouched. No modules, sysctls or packages. No other networking was changed.
- **Retained on purpose:** the user-owned tree `~/igkm-spike-a` on the metal host, outside
  the rsynced `~/overdrive` tree, now 502 MB. That is Spike A's 445 MB plus 58 MB for the two
  Firecracker releases under `~/igkm-spike-a/firecracker/{v1.17.0,v1.11.0}/`. Remove the whole
  tree with `rm -rf ~/igkm-spike-a`, or only the Firecracker downloads with
  `rm -rf ~/igkm-spike-a/firecracker`.

## Gate recommendation

**PROMOTE, with the same named prerequisite as Spike A.** Nanos on Firecracker v1.17.0, with
direct boot, a virtio-blk root, virtio-net and a vsock host channel, is viable and exits cleanly.
It needs the one-instruction Nanos PVH fix, which Overdrive must carry or upstream. Stock Nanos
works out of the box only on Firecracker ≤ 1.11.
