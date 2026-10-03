# Spike A findings: Nanos under Cloud Hypervisor (PVH boot + virtio-vsock)

GH #303, feature `in-guest-kernel-mtls`. PROBE phase, throwaway. Probe sources and raw
captures: `spike-scratch/in-guest-kernel-mtls/increment-a/` (runs `runs/0001`–`runs/0008`,
each `{meta,stdout,stderr}`, append-only, host identifiers redacted).

## Verdict

| Part | Stock Nanos `aad473aa` | Nanos `aad473aa` + one-instruction patch |
|---|---|---|
| PVH direct boot under CH | **DOESN'T WORK**: triple-faults at the first `call` in the PVH entry stub | **WORKS** |
| virtio-blk root image | not reached (untested, not refuted) | **WORKS** |
| virtio-net | not reached | **WORKS** (TCP both ways over the CH tap) |
| vsock guest → host | not reached | **WORKS** (guest `AF_VSOCK` connect to CID 2:5000 lands on `<socket>_5000`, bytes both ways) |
| `VIRTIO_VSOCK_F_STREAM` a hard requirement? | — | **No.** CH offers no vsock feature bits; Nanos negotiates `VERSION_1` only and attaches |

Kernel under test: host `uname -r` = `7.0.0-29-generic` (x86_64, AMD EPYC 8024P,
`systemd-detect-virt` = `none`). Cloud Hypervisor `v53.0`. Guest: Nanos built from
`aad473aad2f73a9893d5c32be333f2796725ab82`.

The stock-Nanos failure is a Nanos defect against the PVH boot ABI, not a Cloud Hypervisor
defect. Nanos uses the stack before it sets one up; the ABI says the OS must set up its own
stack.

## Assumption under test (written before any run)

- **Hypothesis:** a Nanos unikernel image boots under Cloud Hypervisor through Nanos's PVH
  entry, and Nanos's virtio-vsock reaches the host.
- **Prediction:** the guest boots; virtio-blk, virtio-net and vsock come up; a guest
  `AF_VSOCK` connect to CID 2 lands on CH's host-side Unix socket and bytes flow both ways.
- **Falsification:** no boot; or vsock never attaches, including Nanos treating
  `VIRTIO_VSOCK_F_STREAM` as a hard requirement.

The pass-2 prediction (after run 0003, before run 0006) is recorded in the increment `README.md`.

## Predicted vs actual

| Prediction | Actual | Evidence |
|---|---|---|
| CH takes the PVH route for the Nanos ELF | Yes: `PVH kernel loaded: entry_addr = 0x2000dc` | run 0003 |
| Stock Nanos boots | **No.** vCPU dies about 0.3 ms after `vm booted`; serial console empty | run 0003 |
| (pass 2) The stack ordering is the only boot blocker | Yes. With the patch, the kernel boots, mounts root and runs the program | run 0006 |
| virtio-blk / -net / -vsock activate | Yes: CH `virtio-device activated` for `_disk0`, `_net1`, `_vsock2` (and `__rng`) | runs 0006, 0007 |
| Guest vsock connect lands on `<socket>_5000` | Yes: host listener on the Unix socket `vsock.sock_5000` accepted it | runs 0006, 0007 |
| Exact REQUEST in, exact (different) RESPONSE out | Yes, both on host and guest | runs 0006, 0007 |
| No `F_STREAM` failure | Correct: `dev_features 0x900000000, features 0x100000000` | run 0007 |

## Evidence

### Substrate probes (run 0001)

    uname -m: x86_64
    uname -r: 7.0.0-29-generic
    systemd-detect-virt: none (exit 1)
    cpu vmx/svm flag count: 16
    crw-rw---- 1 root kvm 10, 232 Sep 28 06:03 /dev/kvm
    KVM_GET_API_VERSION=12 KVM_CREATE_VM=ok
    model name	: AMD EPYC 8024P 8-Core Processor
    /usr/local/bin/cloud-hypervisor
    cloud-hypervisor v53.0
    ...
    SUBSTRATE OK

The `cargo xtask metal run` fail-closed preflight also passed on every run.

### Kernel artifact is a PVH ELF (run 0002)

    kernel.img: ELF 64-bit LSB executable, x86-64, version 1 (SYSV), statically linked, not stripped
      Owner                Data size 	Description
      Xen                  0x00000004	Unknown note type: (0x00000012)
       description data: dc 00 20 00
    --- bzImage setup-header magic at 0x202 (expect NOT 'HdrS' for PVH route)
      \0  \0  \0  \0

Note type `0x12` is `XEN_ELFNOTE_PHYS32_ENTRY`, so the PVH entry is `0x2000dc`. There is no
Linux setup header, so CH v53.0 routes the image to `configure_pvh`
(`cloud-hypervisor:arch/src/x86_64/mod.rs:1109` at tag v53.0).

### Stock Nanos: PVH boot fails (run 0003)

Cloud Hypervisor log (`-vv`):

    0.005880s: <payload_loader> INFO:vmm/src/vm.rs:1659 -- PVH kernel loaded: entry_addr = 0x2000dc
    0.022933s: <vmm> INFO:event_monitor/src/lib.rs:113 -- Event: source = vm event = booted
    0.023094s: <vcpu0> INFO:vmm/src/vm.rs:485 -- Guest MMIO write to unregistered address 0xfffffffc
    0.023141s: <vcpu0> INFO:vmm/src/vm.rs:476 -- Guest MMIO read from unregistered address 0xfffffffc
    0.023199s: <vcpu0> ERROR:vmm/src/cpu.rs:1460 -- VCPU generated error: VcpuRun(Failed to run vcpu
        Unexpected exit reason on vcpu run: InternalError)

The guest serial log was empty. The host listener got no connection within 60 s:
`HOST: VERDICT vsock=FAIL`, `HOST: VERDICT net=FAIL`.

**Mechanism.** A write, then a read, of `0xfffffffc` is a `call`/`ret` pair running with
`ESP = 0`:

- Nanos `pvh_start32` calls `pvh_zero_page` at `nanos:src/x86_64/init.s:116`, `:121` and
  `:129`. It first sets a stack at `:142` (`mov esp, 0xa000`), after those calls.
- CH v53.0 enters PVH with only `rflags`, `rip` and `rbx` set, so RSP is 0
  (`cloud-hypervisor:arch/src/x86_64/regs.rs:100-104` at tag v53.0). The first `call` pushes
  its return address to `0xfffffffc`, which is unbacked. The write is dropped, `ret` pops
  garbage, and KVM exits with `InternalError`.
- The PVH boot ABI (xenbits `docs/misc/pvh`, fetched 2026-09-28) specifies `ebx`, `cr0`,
  `cr4`, the segments, `tr` and `eflags`, then states: "All other processor registers and flag
  bits are unspecified. The OS is in charge of setting up it's own stack, GDT and IDT." CH is
  conformant. Nanos relies on unspecified state.

This probe did not investigate why the defect does not show on Nanos's other PVH loaders.

### Patched Nanos boots and vsock works (run 0006)

`patches/0001-pvh-set-stack-before-first-call.patch` adds one instruction, `mov esp, 0xa000`,
as the first instruction of `pvh_start32`. That is the same value the code already loads at
`:142`, just before the first `call`. The patch is in a separate clone; the stock build stays
untouched. Disassembly check (run 0005):

    00000000002000dc <pvh_start32>:
      2000dc:	bc 00 a0 00 00       	mov    $0xa000,%esp
      2000e1:	b8 20 00 00 00       	mov    $0x20,%eax

Guest serial console:

    [0.044241] NET: static IP config for interface en1:
    [0.046826] en1: assigned 192.168.203.2
    [0.048702] gitversion: aad473aad2f73a9893d5c32be333f2796725ab82
    IGKM-A: BOOT-MARKER guest program started t=0.051299
    IGKM-A: vsock socket(AF_VSOCK=40, SOCK_STREAM) t=0.052575
    IGKM-A: vsock connected local cid=4294967295 port=1024 -> cid=2 port=5000 t=0.054294
    IGKM-A: vsock wrote 33 bytes t=0.056095
    IGKM-A: vsock read 34 bytes: IGKM-A-VSOCK-RESPONSE host->guest
    IGKM-A: VERDICT vsock=PASS
    IGKM-A: net connected local 192.168.203.2:52477 -> 192.168.203.1:5001 t=0.074552
    IGKM-A: net read 32 bytes: IGKM-A-NET-RESPONSE host->guest
    IGKM-A: VERDICT net=PASS

`gitversion` reports the base SHA. The patch is an uncommitted change in the probe clone.

Host listener (Unix socket `<run dir>/vsock.sock_5000`; TCP `192.168.203.1:5001`):

    HOST: vsock-leg accepted connection peer='' t+0.170292s
    HOST: vsock-leg received 33 bytes: b'IGKM-A-VSOCK-REQUEST guest->host\n' t+0.172449s
    HOST: vsock-leg replied 34 bytes: b'IGKM-A-VSOCK-RESPONSE host->guest\n' t+0.172467s
    HOST: net-leg accepted connection peer=('192.168.203.2', 52477) t+0.190760s
    HOST: VERDICT vsock=PASS t+0.192995s
    HOST: VERDICT net=PASS t+0.193000s

CH event monitor (seconds since VMM start):

    0.021794393s vm booted
    0.030460826s virtio-device activated {'id': '_disk0'}
    0.037075572s virtio-device activated {'id': '__rng'}
    0.042609981s virtio-device activated {'id': '_vsock2'}
    0.047087320s virtio-device activated {'id': '_net1'}

The CH muxer's own view of the handshake is below. The header fields are
`src_cid(8) dst_cid(8) src_port(4) dst_port(4) len(4) type(2) op(2)`:

    muxer.send[rxq.len=0]: [3,0,0,0,0,0,0,0, 2,0,0,0,0,0,0,0, 0,4,0,0, 136,19,0,0, 0,0,0,0, 1,0, 1,0, ...]
    vsock muxer: RX pkt:   [2,0,0,0,0,0,0,0, 3,0,0,0,0,0,0,0, 136,19,0,0, 0,4,0,0, 0,0,0,0, 1,0, 2,0, ...]

The guest sent CID 3 → CID 2, port 1024 → 5000 (`136,19` = `0x1388`), stream, `OP_REQUEST`.
CH replied `OP_RESPONSE`.

What each line of evidence shows:
- **virtio-blk root**: `_disk0` activated. `/vsock_probe` exists only in the image's TFS
  root partition, and it ran.
- **virtio-net**: `_net1` activated, `en1` got its address, and TCP data flowed both ways.
- **vsock**: `_vsock2` activated, the connection arrived on the Unix socket, and data flowed
  both ways.

The connection went through the `<socket>_<port>` convention:
`format!("{}_{}", self.host_sock_path, pkt.dst_port())`
(`cloud-hypervisor:virtio-devices/src/vsock/unix/muxer.rs:716` at tag v53.0).

### `VIRTIO_VSOCK_F_STREAM` is not a hard requirement (run 0007)

Run 0007 used the same patched kernel plus
`patches/0002-diagnostic-enable-virtio-sock-debug.patch`, which uncomments Nanos's existing
`VIRTIO_SOCK_DEBUG` switch and changes no logic:

    [0.039532, 0, virtio_sock] dev_features 0x900000000, features 0x100000000
    [0.041118, 0, virtio_sock]   guest CID 3

- CH offers bits 32 (`VERSION_1`) and 35 (`IN_ORDER`), and no vsock-specific bit
  (`cloud-hypervisor:virtio-devices/src/vsock/device.rs:388` at v53.0).
- Nanos requests `VIRTIO_VSOCK_F_STREAM`, bit 0 (`nanos:src/virtio/virtio_socket.c:51`). It
  negotiates `dev_features & feature_mask`, with no check that requested bits survived
  (`nanos:src/virtio/virtio_pci.c:444`), so the result is `VERSION_1` only.
- The device attached, read CID 3 from config, and carried traffic. There was no CH
  `Received acknowledge request for unknown feature` warning in either run. Run 0007 passed
  both legs again.

## Exact commands

Each run went through `spike-scratch/in-guest-kernel-mtls/increment-a/capture.sh`, which
wraps `cargo xtask metal run` (rsync, shared lease, fail-closed preflight) and writes
`runs/NNNN.*`:

    # 0001 inventory, 0002 build, 0005 patched build (0004 = failed worktree attempt), 0008 end state
    capture.sh 0001 inventory -- bash .../host-inventory.sh
    capture.sh 0002 build --no-sudo -- bash .../build.sh
    capture.sh 0005 build-patched-retry --no-sudo -- bash .../build-patched.sh
    # boots (root): 0003 stock kernel, 0006 patched, 0007 patched + diagnostic
    OVERDRIVE_METAL_KERNEL=<kernel> OVERDRIVE_METAL_ROOTFS=<metal-home>/igkm-spike-a/out/nanos-vsock.img \
      capture.sh 0006 boot-pvhfix -- bash .../run-boot.sh <metal-home>/igkm-spike-a/out/kernel-pvhfix.img
    capture.sh 0008 host-state-check --no-sudo -- bash .../host-state-check.sh

Cloud Hypervisor command line (run 0006; run 0003 differs only in `--kernel`):

    cloud-hypervisor --kernel <metal-home>/igkm-spike-a/out/kernel-pvhfix.img \
      --disk path=<run dir>/disk.img --cpus boot=1 --memory size=512M \
      --net tap=igkma0,ip=192.168.203.1,mask=255.255.255.0,mac=52:54:00:1a:03:02 \
      --vsock cid=3,socket=<run dir>/vsock.sock \
      --serial file=<run dir>/serial.log --console off \
      --event-monitor path=<run dir>/events.json --log-file <run dir>/ch.log -vv

## Artifact versions

| Artifact | Identity |
|---|---|
| Cloud Hypervisor | `v53.0` (`/usr/local/bin/cloud-hypervisor` on the host). Source anchors checked at tag `v53.0` (`9ed824d6d`), not at the research's `ae14eb5d`. The cited behaviours are identical at both. |
| Nanos source | `aad473aad2f73a9893d5c32be333f2796725ab82` (2026-09-19), clean tree |
| Nanos vendored | acpica `e01cc6b3`, lwip `e27f9b7a`, mbedtls `729a04f6` (the same bytes in all three kernels) |
| `kernel.img` (stock) | sha256 `c99b7da38017ee74d5e1999ae1f76654420ffca859972a25a1637aa87ef9f521` |
| `kernel-pvhfix.img` | sha256 `792bf6e7fa5b190d435d229a7c07608b783f3388cf68897aab9b7f86870f259d` |
| `kernel-pvhfix-sockdebug.img` | sha256 `4746fa6181934d840b8c42b522d589971a964d79ccf7dc0bb68992f6fee9c7b0` |
| `boot.img` | sha256 `9a6c4e6b817be8b057b734b5481cb02e2b1acb8dbbb13dbfa4950f81dc72a4bc` |
| Disk image (`mkfs -b boot.img -k kernel.img`) | sha256 `27157a22bd2ceb9d0167248b3b04bb68add394368c2840e2f8ffce75a542beec` (the same image in every boot) |
| Guest program | static glibc ELF, gcc 15.2.0, sha256 `d541c4a7e452e23a2021e31a08a44503df8ff7c0b5a590c6757a653dec39992a` |
| nasm (build-only) | 2.16.03 from `nasm.us` source tarball, sha256 `1412a1c760bbd05db026b6c0d1657affd6631cd0a63cddb6f73cc6d4aa616148`, built in user space |

No official Nanos release was used. Everything was built from the SHA above, with no system
packages installed.

## Edge cases observed

1. **The guest does not power off when the program exits.** CH kept running and was
   SIGTERM'd by the runner's cleanup in runs 0006 and 0007. The CH log shows two
   `Guest PIO write to unregistered address 0x0` at 0.0856 s, right after `program exiting`.
   - Nanos sets its power-off handler to `acpi_powerdown_pm1` whenever the `_S5` package has
     two or more elements (`nanos:src/drivers/acpi.c:376-393`).
   - CH's `_S5_` is `Package{5,5,0,0}` (`cloud-hypervisor:vmm/src/device_manager.rs:5894` at
     v53.0). Its FADT is `HW_REDUCED_ACPI` with no PM1 blocks (`vmm/src/acpi.rs:324-325`), so
     the PM1a/PM1b writes go to port 0.
   - Nanos does not use CH's sleep-control register. After the failed power-off it halts
     (`nanos:src/kernel/init.c:944-947`).
2. **`getsockname` reports local CID `4294967295` (`VMADDR_CID_ANY`), not 3.**
   `vsock_connect` auto-binds to `VMADDR_CID_ANY` (`nanos:src/unix/vsock.c:535`) and
   `getsockname` reports the bound CID (`:675`). On the wire the guest still sends
   `src_cid=3` (muxer dump above).
3. **Each successful vsock `read()` sends a CREDIT_UPDATE.** `virtio_sock_recved` sends
   `OP_CREDIT_UPDATE` unconditionally (`nanos:src/virtio/virtio_socket.c:479-485`, called
   from `src/unix/vsock.c:243`). The probe reads one byte at a time, so run 0007 shows 34
   credit updates for the 34-byte response. Because the host had already closed, each drew an
   `OP_RST` from CH. That is harmless, but it doubled the packet count, and with debug
   printing it delayed the read by about 200 ms. The data survived the host's half-close: the
   guest read all 34 bytes after `OP_SHUTDOWN`.
4. **Root discovery needs the MBR.** Nanos finds root through an MBR `PARTITION_ROOTFS` entry
   (`nanos:src/kernel/init.c:659-686`), and `mkfs` writes that MBR only when given
   `-b boot.img` (`tools/mkfs.c:810-811`). Direct kernel boot therefore still ships
   `boot.img` in the disk image.
5. **ACPI RSDP lookup worked through the EBDA fallback.** CH puts the RSDP at `0xa0000` and
   writes the EBDA pointer (`arch/src/x86_64/mod.rs:1083`, v53.0). Nanos ignores
   `hvm_start_info.rsdp_paddr` in `pvh_start` and finds the RSDP by its EBDA scan
   (`nanos:src/x86_64/acpi.c:66-76`). This was inferred from source; the boot's success is
   consistent with it.
6. **Cosmetic:** Nanos prints netmask and gateway as the address (`192.168.203.2`). All three
   values are formatted into one shared buffer (`nanos:src/net/net.c:256-261`). The real
   netmask and gateway work: TCP to `.1` succeeded.
7. CH warns that auto-detecting the disk image type is deprecated. A driver should pass
   `image_type=raw`.
8. Timing, from one sample and not a benchmark:
   - VMM start to `vm booted`: 21.8 ms.
   - `vsock` activated at 42.6 ms after VMM start.
   - The guest program started at a guest-reported monotonic 51.3 ms.
   - The host-side T0-to-accept figure (0.170 s) includes a ≤0.1 s launcher poll before CH
     starts, so it overstates boot time.

## Design implications

**For GH #303 (in-guest mTLS on Nanos):**
- The host channel #303 needs works on CH as it stands. A guest-initiated `AF_VSOCK` connect
  to CID 2 port N reaches a host process listening on `<vsock socket>_N`, as an ordinary
  Unix-socket accept. The host-side agent is a per-VM Unix listener at a path Overdrive
  chooses, next to the `--vsock` socket. The connection took the userspace-muxer path end to
  end. This probe did not check whether the host has `vhost_vsock` loaded, so it does not
  independently confirm the research's claim that no host `vhost-vsock` is needed. The CH
  source still shows no vhost path.
- `F_STREAM` is not a problem, so no Nanos driver change is needed for vsock.
- The relay should use large reads. Nanos sends a credit update per `read()` (edge case 3),
  so byte-at-a-time reads double vsock packet traffic.
- The guest cannot learn its own CID from `getsockname` (edge case 2). The host assigned the
  CID, so the protocol should key on host-side knowledge, not on a guest-reported CID.

**For an Overdrive Nanos VM driver on Cloud Hypervisor:**
- **Stock Nanos at `aad473aa` does not boot under CH.** The driver needs a Nanos kernel with
  the PVH stack fix, either carried as a patch in whatever builds guest images (the #259
  image factory is the natural owner) or upstreamed to nanovms/nanos. The fix is one
  instruction and brings Nanos in line with the PVH ABI.
- **Exit detection cannot rely on the CH process exiting.** With default config, a Nanos
  guest whose program exits idles forever (edge case 1). A driver needs a Nanos fix that
  prefers the FADT sleep-control register when `HW_REDUCED_ACPI` is set, or another exit
  signal such as vsock or serial. This probe tested neither alternative, nor Nanos's
  `reboot_on_exit` option.
- Direct boot needs `--kernel` (the PVH ELF) plus a disk image built with `boot.img`, for the
  MBR partition table.
- CH created and tore down the tap (non-persistent). The static IP came from the Nanos
  manifest (`ipaddr`/`netmask`/`gateway`). Nanos also started DHCPv6 because no `ip6addr`
  was set.

## Host state created and cleanup

- **Per boot run:**
  - one `cloud-hypervisor` process
  - a non-persistent tap `igkma0` (192.168.203.1/24)
  - one `python3` listener
  - Unix sockets and a copy of the disk image in a per-run directory
  Every run's cleanup printed `links added: (none)`, `tap igkma0 present: no`,
  `run dir present: no`, and no leftover processes. Run 0008 re-checked independently: no CH
  or listener processes, no tap, no `192.168.203.x` address, no sockets, no root-owned files.
- **Host kernel:** untouched. No modules, sysctls or packages.
- **Retained on purpose:** a 445 MB user-owned build tree at `~/igkm-spike-a` on the metal
  host, outside the rsynced `~/overdrive` tree. It holds the nasm binary, the two Nanos
  clones, the kernels, the guest and the image, for any follow-up increment. Remove it with
  `rm -rf ~/igkm-spike-a`.

## Gate recommendation

**PROMOTE, with a named prerequisite.** Nanos on Cloud Hypervisor, with PVH direct boot,
virtio-blk/-net and a vsock host channel, is viable. It works only with a one-instruction
Nanos PVH-entry fix, which Overdrive must carry or upstream. The ACPI power-off behaviour
(edge case 1) must be decided before a driver relies on VM exit.
