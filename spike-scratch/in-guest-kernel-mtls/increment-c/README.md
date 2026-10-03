# Spike A3, increment c: native Unikraft under Firecracker (boot + virtio-net + virtio-vsock)

Throwaway probe for GH #303 (`in-guest-kernel-mtls`). The Unikraft counterpart
of Spike A (`../increment-a/`, Nanos on Cloud Hypervisor) and Spike A2
(`../increment-b/`, Nanos on Firecracker). Tests one assumption: does a Unikraft
unikernel built for Unikraft's Firecracker target boot under Firecracker
v1.17.0, bring up virtio-net (lwIP) and virtio-vsock, and reach the host over
vsock?

Reuses from Spike A2 the verified Firecracker releases in
`~/igkm-spike-a/firecracker/{v1.17.0,v1.11.0}` on the metal host, and copies
`capture.sh`, `rsync-without-local-env.sh` and `host_listener.py` (the listener
byte-identical, so the REQUEST/RESPONSE litmus is the Nanos probes').

Source anchors: Unikraft core `eb8fa2368618cea11c9bde196f79e6e6b9caeed5`
(tree `48639ed4`), `unikraft/lib-lwip` `ec55ae17618feeb57c8c10109bcf5c42723e8e95`
(tree `16e828f2`), Firecracker `v1.17.0` (commit `95f868c8`) and `v1.11.0`
(commit `4e40479d`), linux-loader 0.14.0 (Firecracker v1.17.0's `Cargo.lock`).

## Static read before running (debugging.md § 4)

**Boot protocol.** Unikraft has no PVH note. `KVM_VMM_FIRECRACKER` selects
`KVM_BOOT_PROTO_LXBOOT` (`unikraft:plat/kvm/Config.uk:119-121`), which links the
image with `--entry=_lxboot_entry` (`unikraft:plat/kvm/Linker.uk:6-7`) at
`0x100000` (`plat/kvm/x86/link64.lds.S:29`). Firecracker v1.17.0 falls back to
the Linux 64-bit boot protocol when the ELF has no PVH note
(`firecracker:src/vmm/src/arch/x86_64/mod.rs:514-530`), enters at `e_entry` with
`rsp = rbp = 0x8ff0` and `rsi = 0x7000` (`regs.rs:95-108`), and writes a zero
page with `boot_flag = 0xaa55`, `header = HdrS`, `cmd_line_ptr = 0x20000` and
`cmdline_size` (`mod.rs:400-421`). `_lxboot_entry` checks `HdrS` at zero page
offset 514 (`unikraft:plat/kvm/x86/lxboot.S:31-32`, `include/kvm-x86/lxboot.h:10-11`),
sets its own stack, and `lxboot_entry` checks `boot_flag == 0xAA55`, reads the
command line, initrd and e820, and records `bootprotocol = "lxboot"`
(`lxboot.c:135-152`).

**Device discovery.** Unikraft on x86 finds virtio-MMIO devices only on the
kernel command line: `virtio_mmio_cmdl.c` parses `virtio_mmio.device=<size>@<base>:<irq>`
(`unikraft:drivers/virtio/mmio/virtio_mmio_cmdl.c:21-24,55-109`); the only other
source is FDT (`virtio_mmio.c:422-467`, arm64); there is no ACPI path. Firecracker
v1.17.0 still appends `virtio_mmio.device=` for every virtio-MMIO device on
x86_64, in addition to the DSDT AML (`firecracker:src/vmm/src/device_manager/mmio.rs:231-278`,
test at `builder.rs:1238-1243`), and appends `pci=off` when PCI is disabled
(`builder.rs:217-219`). So the v1.17.0 ACPI move does not remove the command-line
entries.

**The real risk is Unikraft's command-line split, not Firecracker's device
list.** Unikraft parses kernel parameters only if the command line contains the
stop token `--` after at least one parameter, and it never parses `argv[0]`
(`unikraft:lib/ukboot/early_init.c:64-89`: the scan pass must return
`0 < rc < argc-1`, and the parser is fed `&boot_argv[1]`). Firecracker builds the
command line with linux-loader, which splits `boot_args` at the first ` -- `
into boot args and init args, appends every inserted item (`pci=off`,
`virtio_mmio.device=...`) to the boot-args half, and re-joins with ` -- ` only
if the init-args half is non-empty (`linux-loader 0.14.0 src/cmdline/mod.rs:
265-316,384-397,431-450,503-531`). Consequences, by source:

- `boot_args` = `<name> <libparams> -- <app arg>`: the device entries land
  before `--` and are parsed. **This is the only shape that works.**
- no `boot_args` (Firecracker's `DEFAULT_KERNEL_CMDLINE`, `boot_source.rs:19-20`):
  no `--` anywhere, so Unikraft parses no parameters and finds no virtio devices.
- `boot_args` ending in a bare `--` (no init arg): the `--` stays in the boot-args
  half and the devices are appended after it, so they become application
  arguments.
- `boot_args` whose first token is a parameter: that token is `argv[0]` and is
  never parsed.

## Pass 1 — primary run, Firecracker v1.17.0, preset `uk` (written before run 0007)

`boot_args = "igkmc netdev.ip=192.168.203.2/24:192.168.203.1 -- probe"`, one
virtio-net (tap `igkmc0`, 192.168.203.1/24), one vsock (CID 3), no drives.

- **Hypothesis (the task's):** the Unikraft Firecracker-target image boots under
  Firecracker v1.17.0, brings up virtio-net (lwIP) and virtio-vsock, and a guest
  `AF_VSOCK` connect to CID 2 reaches the host on `<uds_path>_<port>` with bytes
  both ways.
- **Prediction:**
  - Firecracker log: `Kernel loaded using Linux 64-bit boot protocol`.
  - Unikraft klog: `Boot loader : unknown-lxboot`; `Command line: igkmc netdev.ip=192.168.203.2/24:192.168.203.1 pci=off virtio_mmio.device=4K@0xc0001000:5 virtio_mmio.device=4K@0xc0002000:6 -- probe`;
    two `virtio-mmio device 4K@0xc000N000:M` lines (from the command line); lwIP
    `Set IPv4 address 192.168.203.2`.
  - App: `argv[0]="igkmc"`, `argv[1]="probe"`; vsock connects, `getsockname`
    reports local CID 3 (Unikraft sets it from the device, `lib/ukvsockdev/vsock.c:1196`
    connect path), 33 bytes out, 34 bytes in, `VERDICT vsock=PASS`; TCP to
    192.168.203.1:5001, `VERDICT net=PASS`. Host listener both `PASS`.
  - Power-off: `main` returns, ukboot requests `SYSHALT`, the PS/2 sysreset
    driver writes `0xfe` to port `0x64` (`unikraft:drivers/input/ukps2/sysreset.c:20-35`),
    Firecracker's i8042 turns that into its exit event; Firecracker exits on its
    own with code 0; metrics `i8042.reset_count = 1`.
- **Falsification:** no serial output; a crash before `main`; no
  `virtio-mmio device` lines or no vsock device (`socket(AF_VSOCK)` fails with
  `EAFNOSUPPORT`); either leg `FAIL`; Firecracker still running after 60 s.

## Pass 2 — negative control, Firecracker v1.17.0, preset `fcdefault` (written before run 0008)

No `boot_args`, so Firecracker's default command line plus its own insertions.

- **Hypothesis:** without a ` -- ` in the command line Unikraft parses no kernel
  parameters, so it discovers no virtio device even though Firecracker lists them.
- **Prediction:** boot as in pass 1 (lxboot); `Command line: reboot=k panic=1 nomodule 8250.nr_uarts=0 i8042.noaux i8042.nomux i8042.dumbkbd swiotlb=noforce pci=off virtio_mmio.device=... virtio_mmio.device=...`;
  no `virtio-mmio device` klog line; the app's `argv[0]` is `reboot=k` and the
  `virtio_mmio.device=` tokens appear as application arguments; `socket(AF_VSOCK)`
  fails with `EAFNOSUPPORT` (97); the TCP leg fails (no netif); both `FAIL`;
  `main` returns 1 yet Firecracker still exits on its own with code 0 (the guest's
  exit status is not visible through Firecracker's exit code).
- **Falsification:** the devices are found (virtio-mmio lines, vsock `PASS`).

## Files

| File | Role |
|---|---|
| `capture.sh` | Append-only capture of one `cargo xtask metal run` → `runs/NNNN.{meta,stdout,stderr}` (refuses overwrite; redacts host). Copied from increment-b. |
| `rsync-without-local-env.sh` | Keeps the workspace `.env` (SSH target) off the host during sync. Copied from increment-b. |
| `host-inventory.sh` | Substrate probes (testing.md, fail closed), Firecracker binaries, user-space build tooling. Creates nothing. |
| `build-tools.sh` | Login user, no sudo: builds m4 1.4.19, bison 3.8.2 and flex 2.6.4 from upstream tarballs into `~/igkm-spike-a/unikraft/tools/prefix`; clones Unikraft and lib-lwip at the pinned SHAs and checks HEAD and tree against the laptop read copies. |
| `app/` | The native Unikraft app: `main.c`, `Makefile.uk`, `Config.uk`, `defconfig` (Kconfig fragment), `Makefile`. |
| `build.sh` | Login user, no sudo: `make defconfig` + `make` with Unikraft's own build system (kraft not used); prints the full resolved `.config` and the image's ELF identity. |
| `config/igkmc.config` | The resolved `.config`, extracted from run 0006's capture. Only `CONFIG_UK_BASE`/`CONFIG_UK_APP` differ from the metal file (login home redacted), so its SHA-256 differs from the printed `89e56902…`. |
| `run-boot-fc.sh` | Root: creates tap `igkmc0`, boots one Firecracker microVM from a JSON config (`--no-api`), runs the host listener, prints every log, cleans up and diffs host state. |
| `host_listener.py` | Host side (byte-identical copy of increment-b's): Unix listener on `<uds_path>_5000`, TCP listener on the tap IP. |
| `host-state-check.sh` | Read-only end-of-probe check. |

## Run log (append-only)

| Run | What | Result |
|---|---|---|
| 0001 | inventory (substrate probes, tools) | `SUBSTRATE OK`; Firecracker v1.17.0/v1.11.0 binaries unchanged (`99ad0f5c…`, `8f0ea0c5…`); **flex, bison, m4 missing** (gcc 15.2.0, make 4.4.1, python3, unzip, patch, gawk present) |
| 0002 | build m4/bison/flex in user space; clone Unikraft + lib-lwip | m4 1.4.19 and bison 3.8.2 GNU signatures `Good` (gpgv + `gnu-keyring.gpg`); flex 2.6.4 sha256 `e87aae03…` (GitHub API gives no asset digest; matches Homebrew's `flex.rb`, checked from the laptop); both clones at the pinned SHA with the laptop's tree object |
| 0003 | build | **harness failure**: `lex: not found` building Kconfig `conf`. Before a `.config` exists the top-level Makefile has not set `LEX := flex` (inside the `UK_HAVE_DOT_CONFIG` block, `unikraft:Makefile:589,677-678`), so the sub-make gets make's default `lex`. Fix: `LEX=flex YACC=bison` on the make command line |
| 0004 | build (retry) | `defconfig` OK, all required symbols `=y`; **compile failure under gcc 15**: `lib/ukboot/boot.c:489: too many arguments to function '*ctorfn'; expected 0, have 2`. gcc 15 defaults to C23, where `typedef void (*uk_ctor_func_t)();` (`include/uk/ctors.h:45`) means `(void)`. Fix: `UK_CFLAGS=-std=gnu17` |
| 0005 | build (retry, clean build dir) | make succeeded; the script then exited 1 on its own `grep -c` (0 matches → exit 1 under `pipefail`). Script bug, not a build failure |
| 0006 | build (retry) | **image built**: `igkmc_fc-x86_64` sha256 `daee7d78…`, ELF64 EXEC, entry `0x171ffa` = `_lxboot_entry`, first PT_LOAD at `0x100000`, **Xen notes: 0** (no PVH), no `HdrS` at 0x202 (plain ELF, not a bzImage); 0 compiler warnings |
| 0007 | preset `uk` on Firecracker v1.17.0 (image `daee7d78…`) | **pass-1 prediction held**: `Kernel loaded using Linux 64-bit boot protocol`; `virtio-mmio device 4K@0xc0001000:5` and `…0xc0002000:6` from the command line; lwIP `Set IPv4 address 192.168.203.2`; `Calling main(2, ['igkmc', 'probe'])`; vsock + net `PASS` on guest and host; Firecracker exited on its own, code 0, `i8042.reset_count = 1`. Not as predicted: the `Boot loader :`/`Command line:` klog lines did not appear (printed before the console driver is up); vsock local port is **0** (Unikraft's allocator starts at 0, `lib/ukvsockdev/vsock.c:205,215-244`), CID 3 as predicted. Unpredicted: Firecracker logged 7 failed `IO write @ 0x70`/`IO read @ 0x71` pairs (Unikraft reading the CMOS RTC, `plat/kvm/x86/tscclock.c:163-196`, which Firecracker does not emulate on x86) and `acknowledge request for unknown feature: 0x202089a3` (Unikraft virtio-net always acks `GUEST_ANNOUNCE`, bit 21, `drivers/virtio/net/virtio_net.c:1032`) |

After run 0007, one diagnostic line was added to `app/main.c`: it prints
`ukplat_wall_clock()`. Firecracker answers every unhandled port read with zeros
(`firecracker:src/vmm/src/arch/x86_64/vcpu.rs:759-766`), so the RTC reads give
second/minute/hour/day/month 0 and year 2000, and
`uktimeconv_bmkclock_to_nsec` turns that into day `DAYSTO2000 + 0 - 1`
(`unikraft:lib/uktimeconv/timeconv.c:175-220`), i.e. 1999-12-31T00:00:00Z.
Nothing else in the app, the configuration or the runner changed.

## Pass 1b — rebuilt image, preset `uk`, Firecracker v1.17.0 (written before runs 0008-0009)

- **Prediction:** identical to pass 1 (both legs `PASS`, exit 0), plus
  `wall clock ukplat_wall_clock()=946598400.x` (1999-12-31T00:00:00Z plus the
  time since the RTC read, under a second): the guest's wall clock is not the
  host's.
- **Falsification:** a wall clock near the real date (2026-09-28, about
  `1790580000`), or any leg failing.

Pass 2 (preset `fcdefault`, above) runs on the rebuilt image as run 0010; its
prediction is unchanged apart from the extra wall-clock line.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0008 | rebuild with the wall-clock line | image `675c53f0…` (main.c `e3104fa7…`, same `.config` `89e56902…`, same lwIP zip `1cf15ac8…`); entry `0x171ffa`, Xen notes 0 |
| 0009 | preset `uk` on Firecracker v1.17.0 (image `675c53f0…`) | **pass-1b prediction held**: as run 0007, both legs `PASS` on guest and host, Firecracker exit 0 on its own, `i8042.reset_count = 1`; `wall clock ukplat_wall_clock()=946598400.057428` (1999-12-31T00:00:00.057Z) |
| 0010 | preset `fcdefault` on Firecracker v1.17.0 | **pass-2 prediction held**: no `virtio-mmio device` line; lwIP `No network interface attached!`; `Calling main(11, ['reboot=k', …, 'pci=off', 'virtio_mmio.device=4K@0xc0001000:5', 'virtio_mmio.device=4K@0xc0002000:6'])`; `socket(AF_VSOCK)` errno 97; TCP connect errno 113 (`No route to host`); both `FAIL`; `main returned 1` yet Firecracker exited on its own with code 0 |

## Pass 3 — Firecracker v1.11.0, preset `uk` (written before run 0011)

Not required (v1.17.0 passed), run for the comparison with the Nanos probes.
Firecracker v1.11.0 has no PVH path, enters every ELF at `e_entry` through the
Linux 64-bit boot path, and puts each virtio-MMIO device on the command line
(`firecracker@v1.11.0:src/vmm/src/device_manager/mmio.rs:224-238`, MMIO base
`0xd0000000` per Spike A2), with linux-loader 0.13.0.

- **Prediction:** the same as pass 1b, except the devices are at
  `virtio-mmio device 4K@0xd0000000:5` and `4K@0xd0001000:6`, the Firecracker log
  has no `Kernel loaded using …` line (added with PVH in 1.12), and the CMOS
  warnings are absent (v1.11.0 drops unmatched port I/O silently, Spike A2 edge
  case 1). Both legs `PASS`; exit 0 on its own; wall clock again `946598400.x`.
- **Falsification:** any leg `FAIL`, no device lines, or Firecracker not exiting.

## Pass 4 — preset `noargv0`, Firecracker v1.17.0 (written before run 0012)

`boot_args = "netdev.ip=192.168.203.2/24:192.168.203.1 -- probe"`: the naive
shape without a program-name token.

- **Hypothesis:** Unikraft never parses `argv[0]`
  (`unikraft:lib/ukboot/early_init.c:64-89`), so a leading `netdev.ip=` is lost
  while the Firecracker-appended device entries (before `--`) still parse.
- **Prediction:** `Calling main(2, ['netdev.ip=192.168.203.2/24:192.168.203.1', 'probe'])`;
  both `virtio-mmio device` lines; no `Set IPv4 address` line; vsock `PASS` (host
  too); TCP connect fails (lwIP has a netif with no IPv4 address; expected
  errno 113); net `FAIL`; exit 0 on its own.
- **Falsification:** `Set IPv4 address 192.168.203.2` appears, or net `PASS`.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0011 | preset `uk` on Firecracker v1.11.0 | **pass-3 prediction held**: `virtio-mmio device 4K@0xd0000000:5` and `4K@0xd0001000:6`; no `Kernel loaded using …` line; no CMOS warnings; both legs `PASS` on guest and host; exit 0 on its own, `i8042.reset_count = 1`; wall clock `946598400.056941` |
| 0012 | preset `noargv0` on Firecracker v1.17.0 | **no guest result**: `Connection reset by <redacted-metal-host> port 22` after the rsync, before the preflight output; `cargo xtask metal run` exit 255. Retried as 0014 after 0013 confirmed the host was clean |
| 0013 | host-state check after the 0012 reset | no firecracker/jailer/listener processes, no tap, no `192.168.203.x`, no sockets, no run dirs, no root-owned files |
| 0014 | preset `noargv0` on Firecracker v1.17.0 (retry of 0012) | **pass-4 prediction held**: `Calling main(2, ['netdev.ip=192.168.203.2/24:192.168.203.1', 'probe'])`; both `virtio-mmio device` lines; `en1: Added`/`Interface is up` but no `Set IPv4 address`; vsock `PASS` (guest and host); TCP connect errno 113; net `FAIL`; `main returned 1`, Firecracker exit 0 on its own |
| 0015 | final host-state check | same as 0013: nothing left running or attached; retained user-owned tree `~/igkm-spike-a` 728 MB, of which `unikraft/` 226 MB |
