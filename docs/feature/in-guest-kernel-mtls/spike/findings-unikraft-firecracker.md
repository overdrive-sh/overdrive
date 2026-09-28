# Spike A3 findings: Unikraft under Firecracker (boot + virtio-net + virtio-vsock)

GH #303, feature `in-guest-kernel-mtls`. PROBE phase, throwaway. This is the Unikraft
counterpart of Spike A (`findings.md`, Nanos on Cloud Hypervisor) and Spike A2
(`findings-firecracker.md`, Nanos on Firecracker). Probe sources, the resolved `.config` and
raw captures: `spike-scratch/in-guest-kernel-mtls/increment-c/` (runs `runs/0001`–`runs/0015`,
each `{meta,stdout,stderr}`, append-only, host identifiers redacted). The increment `README.md`
holds the pre-run predictions and the full run log; this file summarizes it.

The probe agent was interrupted after its last run and before writing findings. All runs and
both host-state checks had completed. The orchestrator wrote this file from the increment's
README and the run captures, and re-checked the key lines below against those captures.

## Does Unikraft work on Firecracker?

**Yes, with no source changes, provided the host passes the right kernel command line.**
Unikraft `eb8fa236` with lib-lwip `ec55ae17`, built for Unikraft's Firecracker target, boots
under Firecracker v1.17.0 (and v1.11.0). virtio-net (lwIP) and virtio-vsock both work, and a
guest `AF_VSOCK` connect to CID 2 reaches the host on `<uds_path>_5000` with bytes both ways.
Firecracker exits on its own with code 0 when the app returns.

Unlike Nanos, Unikraft needs no boot fix: it has no PVH note, so Firecracker boots it through
the Linux 64-bit boot protocol, which is the entry Unikraft's Firecracker target provides.

**The catch is the command line.** Unikraft parses kernel parameters only when the command line
contains `--` after at least one parameter, and it never parses `argv[0]`. Firecracker's default
command line has no `--`, so with no `boot_args` Unikraft finds no devices at all (pass 2). The
only working shape is `boot_args = "<name> <libparams> -- <app args>"`.

## Verdict

| Part | FC v1.17.0, `boot_args` = `igkmc netdev.ip=… -- probe` (runs 0007, 0009) | FC v1.17.0, no `boot_args` (run 0010) | FC v1.17.0, `netdev.ip=… -- probe` (run 0014) | FC v1.11.0, `igkmc netdev.ip=… -- probe` (run 0011) |
|---|---|---|---|---|
| Boot | **WORKS** (Linux 64-bit boot → `_lxboot_entry`) | WORKS | WORKS | **WORKS** |
| virtio-MMIO devices found | **yes**, from `virtio_mmio.device=` on the command line | **no** | yes | yes |
| virtio-net (lwIP) | **WORKS** | FAIL (no interface) | FAIL (no IPv4 address: `netdev.ip` was `argv[0]`) | **WORKS** |
| vsock guest → host | **WORKS** | FAIL (`EAFNOSUPPORT`) | **WORKS** | **WORKS** |
| Firecracker exits on its own when `main` returns | **yes**, code 0 | yes, code 0 even though `main` returned 1 | yes, code 0 | yes, code 0 |

Runs 0010 and 0014 were deliberate negative controls for the command-line rules; both results
matched their pre-run predictions.

Substrate: host `uname -r` = `7.0.0-29-generic` (x86_64, AMD EPYC 8024P, `systemd-detect-virt`
= `none`). Guest: Unikraft core `eb8fa2368618cea11c9bde196f79e6e6b9caeed5` (tree `48639ed4`),
`unikraft/lib-lwip` `ec55ae17618feeb57c8c10109bcf5c42723e8e95` (tree `16e828f2`), both checked
against the laptop read copies. Firecracker `v1.17.0` (commit `95f868c8`) and `v1.11.0` (commit
`4e40479d`), the same verified binaries as Spike A2 (`99ad0f5c…`, `8f0ea0c5…`).

## Assumption under test (written before any run)

- **Hypothesis:** a Unikraft unikernel built for Unikraft's Firecracker target boots under
  Firecracker v1.17.0, brings up virtio-net (lwIP) and virtio-vsock, and a guest `AF_VSOCK`
  connect to CID 2 reaches the host on `<uds_path>_<port>` with bytes both ways.
- **Prediction (pass 1, from source):**
  - Unikraft has no PVH note; `KVM_VMM_FIRECRACKER` selects `KVM_BOOT_PROTO_LXBOOT`
    (`unikraft:plat/kvm/Config.uk:119-121`), linked with `--entry=_lxboot_entry`
    (`plat/kvm/Linker.uk:6-7`). Firecracker v1.17.0 falls back to Linux 64-bit boot for an ELF
    without the PVH note (`firecracker:src/vmm/src/arch/x86_64/mod.rs:514-530`), entering at
    `e_entry` with `rsp = rbp = 0x8ff0` and `rsi` = the zero page (`regs.rs:95-108`).
  - Unikraft on x86 finds virtio-MMIO devices only from `virtio_mmio.device=` command-line
    entries (`unikraft:drivers/virtio/mmio/virtio_mmio_cmdl.c:21-24,55-109`); there is no ACPI
    path. Firecracker v1.17.0 still appends those entries on x86_64 in addition to the DSDT
    (`firecracker:src/vmm/src/device_manager/mmio.rs:231-278`), so the ACPI move does not
    remove them.
  - The real risk is Unikraft's command-line split: parameters are parsed only if the command
    line contains `--` after at least one parameter, and `argv[0]` is never parsed
    (`unikraft:lib/ukboot/early_init.c:64-89`). Firecracker (linux-loader 0.14.0) splits
    `boot_args` at the first ` -- `, appends its own items (`pci=off`, `virtio_mmio.device=…`)
    to the first half, and re-joins with ` -- ` only if the second half is non-empty.
  - Expected: `Kernel loaded using Linux 64-bit boot protocol`; two `virtio-mmio device` lines;
    lwIP `Set IPv4 address 192.168.203.2`; both legs `PASS`; power-off through the PS/2 sysreset
    driver (`unikraft:drivers/input/ukps2/sysreset.c:20-35`) → Firecracker's i8042 → exit 0.
- **Falsification:** no serial output; a crash before `main`; no device lines; either leg
  `FAIL`; Firecracker still running after 60 s.

## Evidence

### Build (runs 0001–0008)

- Run 0001: `SUBSTRATE OK`. flex, bison and m4 were missing on the host (gcc 15.2.0, make 4.4.1
  present).
- Run 0002: m4 1.4.19, bison 3.8.2 (GNU signatures verified) and flex 2.6.4 (sha256 checked)
  built in user space under the scratch tree. No system packages were installed.
- Run 0003 (harness failure): `lex: not found` while building Kconfig `conf`. Before a `.config`
  exists, the top-level Makefile has not set `LEX := flex` (`unikraft:Makefile:589,677-678`).
  Fix: `LEX=flex YACC=bison` on the make command line.
- Run 0004 (compile failure under gcc 15):
  `lib/ukboot/boot.c:489: too many arguments to function '*ctorfn'; expected 0, have 2`. gcc 15
  defaults to C23, where `typedef void (*uk_ctor_func_t)();` (`include/uk/ctors.h:45`) means
  `(void)`. Fix: `UK_CFLAGS=-std=gnu17`.
- Run 0006: image built with Unikraft's own make/Kconfig build (`kraft` not used). ELF64, entry
  `0x171ffa` = `_lxboot_entry`, first PT_LOAD at `0x100000`, **Xen notes: 0** (no PVH), not a
  bzImage. Zero compiler warnings. Run 0008 rebuilt it with one extra diagnostic line (the wall
  clock).

Key options from `config/igkmc.config`:

    CONFIG_ARCH_X86_64=y
    CONFIG_PLAT_KVM=y
    CONFIG_KVM_BOOT_PROTO_LXBOOT=y
    CONFIG_KVM_VMM_FIRECRACKER=y
    CONFIG_LIBUKPS2_SYSRESET=y
    CONFIG_LIBVIRTIO_MMIO=y
    CONFIG_VIRTIO_MMIO_LINUX_COMPAT_CMDLINE=y
    CONFIG_LIBPOSIX_SOCKET=y
    CONFIG_LIBUKNETDEV=y
    CONFIG_LIBUKVSOCKDEV=y
    CONFIG_LIBUKVSOCKDEV_VSOCK_BUF_SIZE=65536

### Unikraft on Firecracker v1.17.0 works (runs 0007, 0009)

Run 0007 (Firecracker log, Unikraft klog, guest app, host listener):

    [igkm-c:main:DEBUG:src/vmm/src/arch/x86_64/mod.rs:524] Kernel loaded using Linux 64-bit boot protocol
    [    0.108481] Info: [libvirtio_mmio] <virtio_mmio_cmdl.c @   93> virtio-mmio device 4K@0xc0001000:5
    [    0.114976] Info: [libvirtio_mmio] <virtio_mmio_cmdl.c @   93> virtio-mmio device 4K@0xc0002000:6
      1: Set IPv4 address 192.168.203.2 mask 255.255.255.0 gw 192.168.203.1
    [    0.156930] Info: [libukboot] <boot.c @  504> Calling main(2, ['igkmc', 'probe'])
    IGKM-C: VERDICT vsock=PASS
    IGKM-C: VERDICT net=PASS
    [    0.170432] Info: [libukboot] <boot.c @  514> main returned 0
    HOST: VERDICT vsock=PASS t+0.253295s
    HOST: VERDICT net=PASS t+0.253304s
    [igkm-c:main:INFO:src/firecracker/src/main.rs:116] Firecracker exiting successfully. exit_code=0
    firecracker exited on its own: exit=0 at t+0.301s (<=0.05s poll)
       i8042.reset_count = 1

Run 0009 repeated this on the rebuilt image with the same results, plus:

    IGKM-C: wall clock ukplat_wall_clock()=946598400.057428 s since 1970-01-01

### No `boot_args`: no devices (run 0010, negative control)

    [    0.109763] Warn: [liblwip] <init.c @  460> No network interface attached!
    IGKM-C: VERDICT vsock=FAIL
    IGKM-C: VERDICT net=FAIL
    HOST: VERDICT vsock=FAIL t+60.115897s
    HOST: VERDICT net=FAIL t+60.115910s
    [igkm-c:main:INFO:src/firecracker/src/main.rs:116] Firecracker exiting successfully. exit_code=0

The README records the rest of this run: no `virtio-mmio device` line;
`Calling main(11, ['reboot=k', …, 'pci=off', 'virtio_mmio.device=4K@0xc0001000:5', 'virtio_mmio.device=4K@0xc0002000:6'])`,
so Firecracker's device entries arrived as application arguments; `socket(AF_VSOCK)` failed with
errno 97 (`EAFNOSUPPORT`); TCP connect failed with errno 113; `main returned 1`, yet Firecracker
still exited with code 0.

### Parameter as the first token (run 0014, negative control)

With `boot_args = "netdev.ip=192.168.203.2/24:192.168.203.1 -- probe"`, `netdev.ip` became
`argv[0]` and was never parsed, while the Firecracker-appended device entries (before `--`) were:

    IGKM-C: VERDICT vsock=PASS
    IGKM-C: VERDICT net=FAIL
    HOST: VERDICT vsock=PASS t+60.147736s
    HOST: VERDICT net=FAIL t+60.147754s

Run 0012 was the first attempt at this pass; it produced no guest result because the SSH
connection to the metal host was reset after the rsync. Run 0013 confirmed the host was clean,
and run 0014 is the retry.

### Firecracker v1.11.0 (run 0011)

    [    0.108367] Info: [libvirtio_mmio] <virtio_mmio_cmdl.c @   93> virtio-mmio device 4K@0xd0000000:5
    [    0.114784] Info: [libvirtio_mmio] <virtio_mmio_cmdl.c @   93> virtio-mmio device 4K@0xd0001000:6
    IGKM-C: VERDICT vsock=PASS
    IGKM-C: VERDICT net=PASS
    firecracker exited on its own: exit=0 at t+0.296s (<=0.05s poll)

Devices sit at `0xd0000000` on v1.11.0 (vs `0xc0001000` on v1.17.0), there is no
`Kernel loaded using …` line (added with PVH in 1.12), and there are no CMOS warnings (v1.11.0
drops unmatched port I/O silently).

## Firecracker configuration

`firecracker --no-api --config-file <run dir>/vm.json --id igkm-c` (no jailer), 1 vCPU, one
virtio-net on tap `igkmc0` (192.168.203.1/24, created and deleted per run), one vsock (guest CID
3, `uds_path` in the run directory), **no drives**: the native app is linked into the kernel
image. `boot_args` per preset:

| Preset | `boot_args` |
|---|---|
| `uk` | `igkmc netdev.ip=192.168.203.2/24:192.168.203.1 -- probe` |
| `fcdefault` | (none: Firecracker's default command line) |
| `noargv0` | `netdev.ip=192.168.203.2/24:192.168.203.1 -- probe` |

The exact `vm.json` and all commands are in `run-boot-fc.sh` and the `runs/*.meta` files.

## Comparison with the Nanos probes

| Aspect | Nanos on CH v53.0 (Spike A) | Nanos on FC v1.17.0 (Spike A2) | Unikraft on FC v1.17.0 (this spike) |
|---|---|---|---|
| Stock guest boots | no (PVH stack bug) | no (same bug) | **yes** |
| Boot route | PVH | PVH | Linux 64-bit boot (`_lxboot_entry`) |
| Guest source fix needed | yes, one instruction | yes, the same one | **no** |
| Host must supply | disk image with `boot.img` MBR | disk image with `boot.img` MBR | `boot_args` in Unikraft's `<name> <params> -- <args>` shape; no disk |
| Device discovery | virtio-PCI | virtio-MMIO via ACPI | virtio-MMIO via `virtio_mmio.device=` on the command line |
| vsock guest → host | works | works | works |
| VMM exits when the program exits | no | yes, code 0 | yes, code 0 |
| Program exit status visible to the host | no | no | no (exit 0 even when `main` returned 1) |
| Guest wall clock | not checked | not checked | **wrong: 1999-12-31** (edge case 1) |
| Guest program start (one sample, guest time) | 51.3 ms | 29.2 ms | 157 ms (`Calling main`) |

The timings are single samples, not a benchmark. The cause of Unikraft's later `main` was not
investigated.

## Edge cases observed

1. **The guest wall clock is wrong on Firecracker.** Unikraft reads the CMOS RTC
   (`unikraft:plat/kvm/x86/tscclock.c:163-196`), which Firecracker does not emulate on x86.
   Firecracker logs failed `IO write @ 0x70` accesses and answers unhandled port reads with zeros
   (`firecracker:src/vmm/src/arch/x86_64/vcpu.rs:759-766`), so the RTC reads as year 2000 with
   all other fields 0, which converts to 1999-12-31T00:00:00Z
   (`unikraft:lib/uktimeconv/timeconv.c:175-220`). Run 0009 measured
   `ukplat_wall_clock()=946598400.057428`.
2. **`argv[0]` is never parsed** (run 0014): the first token must be a program name, or a
   parameter placed there is silently ignored.
3. **Without `--`, no parameters are parsed at all** (run 0010), including the device entries
   Firecracker appends. A bare trailing `--` also fails by source reading: the device entries
   are then appended after it and become application arguments. That shape was not run.
4. **Firecracker's exit code does not reflect the program's.** `main returned 1` in runs 0010
   and 0014, and Firecracker still exited 0.
5. **vsock local port is 0.** Unikraft's port allocator starts at 0
   (`unikraft:lib/ukvsockdev/vsock.c:205,215-244`); `getsockname` reports CID 3.
6. **virtio-net acknowledges a feature Firecracker does not offer**
   (`Received acknowledge request for unknown feature: 0x202089a3`): Unikraft's virtio-net always
   acks `GUEST_ANNOUNCE`, bit 21 (`unikraft:drivers/virtio/net/virtio_net.c:1032`). Harmless.
7. **The `Boot loader :` and `Command line:` klog lines do not appear**: they print before the
   console driver is up.
8. **Current toolchains need two build workarounds**: `LEX=flex YACC=bison` for the first
   Kconfig build, and `UK_CFLAGS=-std=gnu17` under gcc 15 (C23 changes the meaning of `()`).

## Design implications

**For GH #303:**
- The vsock host channel works on Unikraft/Firecracker exactly as on Nanos: a per-VM Unix
  listener at `<uds_path>_<port>`.
- *Analysis (interpretation):* the wrong guest clock (edge case 1) supports keeping the TLS
  handshake on the host. Certificate validity checks run in the host's rustls with the host's
  clock, so a guest that believes it is 1999 does not break them. A guest-side TLS handshake
  would.
- The research's Unikraft prerequisites for the socket-layer port are unchanged by this probe:
  a core `lib/posix-socket` patch plus a `lib-lwip` patch for readiness suppression. This probe
  tested boot and transport only.

**For an Overdrive Unikraft VM driver:**
- On Firecracker, Unikraft needs **no boot patch**. On Cloud Hypervisor it has no confirmed path,
  because it has no PVH entry (research doc, F-CH-3). For Unikraft, Firecracker is the working
  VMM today.
- The driver must build `boot_args` in Unikraft's shape: a program-name token, then library
  parameters (for example `netdev.ip=`), then `--`, then application arguments.
- The driver cannot read the program's exit status from Firecracker's exit code (edge case 4).
  It needs vsock or serial for that, as with Nanos.
- Guest time needs a source other than the RTC on Firecracker, for example a host-provided time
  over vsock or a paravirtual clock. This probe did not test any.
- The image build needs flex/bison (buildable in user space) and `-std=gnu17` under gcc 15.
- Running an **unmodified Linux binary** was not tested. That needs an `app-elfloader` build
  (pinned `a6c9dc2b655572bee1f9657cbcc2d1eca8f6cb73`), an initrd carrying a static Linux binary,
  and the same vsock and lwIP libraries, so that the binary's socket syscalls go through
  `syscall_shim` → `posix-socket`.

## Host state created and cleanup

- **Per boot run:** one tap `igkmc0` (192.168.203.1/24), one Firecracker process (no jailer, no
  API socket), one Python listener, and a per-run directory. Each run's cleanup removed them.
- **Runs 0013 and 0015** (read-only checks) found no firecracker, jailer or listener processes,
  no tap (`igkmc0`, `igkmb0` or `igkma0`), no tun/tap links, no `192.168.203.x` address, no Unix
  sockets, no per-run directories and no root-owned files.
- **Host kernel and packages:** untouched.
- **Retained on purpose:** the user-owned tree `~/igkm-spike-a` on the metal host, now 728 MB,
  of which 226 MB is `unikraft/` (sources, the user-space build tools and the build). Remove it
  with `rm -rf ~/igkm-spike-a`.

## Gate recommendation

**No walking skeleton yet (the probe is kept per `spike.md`).** Unikraft on Firecracker works
without a guest patch and its vsock host channel matches the Nanos results. A walking skeleton
would be a Unikraft VM driver through `overdrive deploy`, which is #112 scope, not #303.
