# Spike B3, increment f: the in-guest mTLS mechanism for UNMODIFIED Linux binaries under Unikraft app-elfloader on Firecracker

Throwaway probe for GH #303 (`in-guest-kernel-mtls`). Spikes B and B2
(`../increment-d/`, `../increment-e/`) proved the mechanism for a *native*
Unikraft app, linked against Unikraft's own libc and syscalls. Overdrive's
workloads are built from Dockerfiles (#259), so they are ordinary Linux
binaries. This probe runs one unchanged Linux ELF, built with the host's stock
gcc and glibc, through Unikraft's `app-elfloader` on Firecracker v1.17.0, with
Spike B2's mechanism underneath it, and repeats B2's checklist from that binary.

Pinned: Unikraft core `eb8fa2368618cea11c9bde196f79e6e6b9caeed5` + increment-d
patches 0001/0002 (read-only), lib-lwip `ec55ae17618feeb57c8c10109bcf5c42723e8e95`
+ increment-e patch 0003 (read-only), `app-elfloader`
`a6c9dc2b655572bee1f9657cbcc2d1eca8f6cb73` (unmodified), `lib-libelf` (HEAD at
first clone, recorded in run 0005: `c03d4ceb`), Mbed TLS 3.6.7, Firecracker v1.17.0,
rustls 0.23.45 / ring 0.17.14 (`host/Cargo.lock`, B2's lockfile unchanged).

Spike B/B2 trees are untouched. B3 builds in new worktrees (`unikraft-igkmf`,
`lib-lwip-igkmf`), new clones (`app-elfloader`, `lib-libelf`), new build/out
dirs (`build-igkmf-<variant>`, `out-igkmf`), a new host target dir
(`igkmf-host-target`), and a uniquely named tap per boot run
(`igkf<ddHHMMSS>`, 192.168.204.0/24).

## What is reused and what is new

| Piece | Source |
|---|---|
| core `lib/posix-socket` connect/accept post-hooks | increment-d patches 0001/0002, applied read-only |
| lib-lwip readiness hook | increment-e patch 0003, applied read-only |
| `lib-mtlsguard` | B2's `guard.c` unchanged, plus `initrd_evidence()` (hashes the initrd for evidence; not mechanism) |
| host `peer`, `relay`, `inclient`, `gencerts` | B2's crate; only `peer.rs` gains the `:6451` HTTP responder for the stretch |
| `pcap_scan.py`, `capture.sh`, rsync guard | B2's, with increment-f names (and the stretch's markers/port) |
| guest application | **new:** app-elfloader (unmodified) loading `app-linux/igkmf_app.c` built by the host gcc as `-static-pie`; the Firecracker initrd IS that ELF (`APPELFLOADER_INITRDEXEC`) |
| guest config | `config/igkmf.defconfig`: B2's platform/net/vsock/socket/lwIP/mtlsguard settings + elfloader, posix-process multithreading, getrandom, no VFS |

The test program is B2's `app/main.c` ported to Linux. The only functional
changes are `clock_gettime(2)` for Unikraft's clock calls, a line-buffered
stdout, an identity banner and the `native`/`id`/`trace` presets. The wire
lines are B2's `IGKM-E-...` so the B2 peer and scanner apply unchanged; the
program's own log prefix is `IGKM-F:`.

## Files

| File | Role |
|---|---|
| `capture.sh`, `rsync-without-local-env.sh` | B2's, increment-f names |
| `host-inventory.sh`, `host-state-check.sh` | read-only substrate / end-of-probe checks |
| `app-linux/igkmf_app.c`, `build-binary.sh` | the unmodified Linux test program and its host build + native smoke test |
| `config/igkmf.defconfig`, `build-guest.sh` | the elfloader guest image (`main` and `strace` variants) |
| `lib-mtlsguard/` | B2's library + the evidence hash |
| `host/`, `build-host.sh` | host programs (B2 + `:6451`) |
| `run-boot-fc.sh`, `pcap_scan.py` | the root boot runner and the tap scanner |
| `build-busybox.sh`, `build-go-client.sh`, `app-go/` | stretch: BusyBox 1.37.0 wget and a Go HTTP client, both built as static-pie on the host |
| `config/igkmf-go.fragment` | stretch only: futex, eventfd, pipe and posix-vfs for the Go client's image |
| `config/igkmf.config`, `config/igkmf-go.config` | resolved `.config` of the checklist image (run 0011) and the Go image (run 0024); the login home is redacted in `CONFIG_UK_BASE`/`CONFIG_UK_APP`, and the sha256 matches the logged one (`dfe2714b…`, `d2c4ec26…`) after substituting it back |
| `runs/NNNN.{meta,stdout,stderr}` | append-only captures |

## Run log (append-only)

| Run | What | Result |
|---|---|---|
| 0001 | inventory | `SUBSTRATE OK`; `uname -r` `7.0.0-29-generic`, AMD EPYC 8024P, `systemd-detect-virt` = `none`, KVM API 12; Firecracker v1.17.0 `99ad0f5c…`; pristine Unikraft/lib-lwip clean at the pinned SHAs; B and B2 worktrees exactly their patches; Mbed TLS tarball sha256 matches; host gcc 15.2.0, GNU ld 2.46, glibc 2.43 (`libc6-dev 2.43-2ubuntu2.4`), `rcrt1.o` present (static-pie available); wget/curl/git/cpio present; **no Go toolchain**; `/usr/bin/busybox` is a static **non-PIE** `ELF 64-bit LSB executable` (app-elfloader only loads ET_DYN, `elf_load.c:148`); no `igk*` links, no 192.168.204.x, no listeners on 5001/6443-6451/7100 |

## Pass 1 — host programs and the Linux test binary (written before runs 0002, 0003)

- **Hypothesis:** B2's host crate builds unchanged apart from the `:6451`
  responder (`--locked`, offline from the spike CARGO_HOME); the host gcc
  builds `igkmf_app.c` as a static-pie ELF that runs natively on the host.
- **Prediction:** 0002: build OK; six-leaf PKI, `openssl verify` OK x6.
  0003: 0 compiler warnings; `file` = `ELF 64-bit LSB pie executable … static-pie
  linked`; `readelf -h` Type `DYN`; no `INTERP` program header, no `NEEDED`;
  0 `unikraft`/`ukplat` strings; `./igkmf-app id` prints `sysname="Linux"
  release="7.0.0-29-generic"`, `glibc 2.43` and exits 0.
- **Falsification:** `--locked` refusal or a compile error in `peer.rs`; a
  non-PIE or dynamically linked result; the native run failing.

| Run | What | Result |
|---|---|---|
| 0002 | host build | **pass-1 prediction held**: `--locked` OK in 14 s from the spike CARGO_HOME (rustls 0.23.45, ring 0.17.14, rcgen 0.14.10, x509-parser 0.18.1); six leaves, `openssl verify` OK x6; client key DER sha256 `aee0f2c7…`, guest-server `0a6ad060…` (fresh throwaway PKI in `out/certs`) |
| 0003 | Linux binary build | **failed to compile, my error**: `implicit declaration of function 'accept4'` at `igkmf_app.c:915`. glibc declares `accept4` only under `_GNU_SOURCE` (B2 compiled against Unikraft's nolibc headers, which declare it unconditionally). Fixed with `#define _GNU_SOURCE`, as any glibc program that calls `accept4` does. Not a Unikraft finding |
| 0004 | Linux binary build, `_GNU_SOURCE` | **pass-1 prediction held**: 0 warnings; `ELF 64-bit LSB pie executable, x86-64, version 1 (GNU/Linux), static-pie linked … for GNU/Linux 3.2.0, not stripped`; Type `DYN`; LOAD/DYNAMIC/TLS/GNU_RELRO, no `INTERP`; no `NEEDED`; 0 `unikraft`/`ukplat` strings; `GNU C Library (Ubuntu GLIBC 2.43-2ubuntu2.4) stable release version 2.43` embedded; **sha256 `043e34e746ef86eb2a97c3334920edc1022ef34e54186213eb33571f6357b179`, 890 080 B**. Native `./igkmf-app id`: `sysname="Linux" release="7.0.0-29-generic"`, `glibc 2.43`, auxv with a vDSO (`AT_SYSINFO_EHDR` set), exit 0 |

## Pass 2 — the elfloader guest image (written before run 0005)

- **Hypothesis:** app-elfloader at `a6c9dc2b` builds against Unikraft
  `eb8fa236` with B2's patches, lib-lwip patch and lib-mtlsguard, in the
  initrd-exec configuration without a VFS; the syscall-shim binary handler is
  linked, and its socket handlers are the same hooked functions B2 used.
- **Prediction:** the three patch stats as in B2; `make defconfig` resolves every
  required symbol (APPELFLOADER_INITRDEXEC, LIBSYSCALL_SHIM_HANDLER(_ULTLS),
  LIBPOSIX_PROCESS_MULTITHREADING, LIBUKBOOT_MAINTHREAD, …) with VFS off; lib-libelf's
  make fetches elftoolchain with wget; the image links; one strong `T` per hook,
  every call site bound to it; `ukplat_syscall_handler`, `uk_syscall6_do_e`,
  `uk_syscall_r_connect`, `uk_syscall_r_accept4`, `uk_syscall_r_epoll_wait`,
  `uk_syscall_r_poll`, `uk_syscall_r_clock_nanosleep` and
  `uk_syscall_r_exit_group` present.
- **Falsification:** app-elfloader's Kconfig or sources no longer build against
  this Unikraft (its own defconfigs name symbols this Unikraft no longer has:
  `LIBVFSCORE_AUTOMOUNT_ROOTFS`, `LIBVFSCORE_ROOTFS_INITRD`, `LIBUKSIGNAL`,
  `LIBPOSIX_EVENT` — none exist at `eb8fa236`); a required symbol that
  cannot be selected; a weak hook default bound locally.

| Run | What | Result |
|---|---|---|
| 0005 | guest build, `main` variant | **pass-2 prediction held; the falsification risk did not fire.** app-elfloader `a6c9dc2b` builds against Unikraft `eb8fa236` with its OWN stale defconfigs unused: `config/igkmf.defconfig` resolves every required symbol (APPELFLOADER_INITRDEXEC/CUSTOMAPPNAME, LIBELF, LIBSYSCALL_SHIM(_HANDLER, _HANDLER_ULTLS), LIBPOSIX_PROCESS_MULTITHREADING, LIBUKBOOT_MAINTHREAD, LIBUKRANDOM_GETRANDOM, LIBPOSIX_MMAP) with LIBVFSCORE and LIBPOSIX_VFS off. lib-libelf `c03d4ceb22a2b9e9799238f9c277a14284d313ad` (vendored sources; nothing fetched besides lib-lwip's `UNIKRAFT-2_1_x.zip`, sha256 `1cf15ac8…`, as in B2). Patch stats as B2. 2 warnings, neither in the mechanism: `lib/posix-tty/serial.c:162 'tty_cons' may be used uninitialized` and `app-elfloader/elf_load.c:71 'get_phdr_mmap_prot' defined but not used`. One strong `T` per hook, every call site bound (`__uk_syscall_r_connect` `139281: call 1abfd0`, `uk_sys_accept` `133bc4: call 1ac500`, lwIP callback `18be63` and poll_setup `18bd43` -> `1abe80`); `ukplat_syscall_handler`, `uk_syscall6_do_e`, `uk_syscall_r_{connect,accept,accept4,getsockopt,epoll_wait,epoll_pwait,poll,ppoll,read,write,sendto,recvfrom,clock_nanosleep,exit_group}` all present; `uk_syscall_r_connect` calls `__uk_syscall_r_connect` (the hooked function) between the enter/exit tables, and `do_accept4` calls `uk_sys_accept` (the hooked function). Image `273d7744…` (981 656 B); resolved `.config` sha256 `95edf088…` |

## Pass 3 — B2's checklist from the unmodified binary (written before run 0006)

`boot_args = "igkmf netdev.ip=192.168.204.2/24:192.168.204.1 mtlsguard.mesh=192.168.204.1:6443-6451 mtlsguard.mesh_in=192.168.204.2:7443 mtlsguard.agent_port=7100 -- igkmf-app run"`,
`initrd_path` = `out/bin/igkmf-app` (sha256 `043e34e7…`).

- **Hypothesis:** app-elfloader loads the static-pie glibc ELF, and every
  socket/poll/epoll call it makes arrives through the syscall-shim binary
  handler at the same posix-socket functions B2's native app reached, so the
  mechanism behaves exactly as in B2 run 0005.
- **Predictions:**
  - before boot, the same file run natively on the host prints
    `sysname="Linux"` and passes case A;
  - in the guest: `MTLSGUARD: evidence: initrd0 … sha256=043e34e7…` equal to the
    host file; elfloader's `ELF program loaded to …`; the binary's banner with
    `sysname="Unikraft" release="5.15.148-…"`, `glibc 2.43`,
    `AT_SYSINFO_EHDR=0` (no vDSO), `argv[0]="igkmf-app" argv[1]="run"`;
  - `MTLSGUARD: [cN] connect …: mesh destination` / `accept …: mesh listener`
    lines for the binary's connections (the hooks fire), and the relay's
    `handshake complete: policy ALLOW local …guest-client` with the host-held
    client identity;
  - all 18 guest verdicts `PASS`, with B2 run 0005's structural numbers: N1
    `connect()` -1 errno 115, `poll(…,0)`=0, early `write()` -1 errno 11; N3
    40 033 bytes in 40 033 one-byte reads over 4 ET wakeups, spurious 0,
    fnv1a32 `0xccf62e40`; N4 77 POLLIN wakeups for 77 bytes; K `KeyUpdates
    rx=3 tx=3`; R1 4 NSTs discarded; R2 `IN ERR` errno 71; N5 `ERR HUP`,
    SO_ERROR 13 then 0; E exchange 2 after the relay SIGKILL; F/N6 errno 103;
    S1/S2 as B2;
  - scan: every TLS direction `unframed_bytes=0`, `IGKM-E-` 0 times on TLS
    streams, the plaintext positive control on :5001;
  - custody: SVID key DER/PEM/scalar 0/0/0 on every relay connection;
  - Firecracker exits 0 on its own once the binary's `exit_group` ends the
    init process (`LIBUKBOOT_MAINTHREAD` shutdown).
- **Falsification:** elfloader refuses or crashes on the ELF; glibc start-up
  dies on a missing syscall; a mesh connection with no guard line (hook
  bypassed) or with plaintext on the wire; any `FAIL`; a readiness difference
  from B2 (EPOLLOUT before install, a spurious wakeup, a lost ET edge); an
  initrd hash mismatch.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0006 | preset `mesh`, image `273d7744…`, tap `igkf29105328` | **Falsified at glibc start-up, before any socket call.** Before boot, the same file run natively on the host passed case A (`sysname="Linux"`, `VERDICT native A pass-through=PASS`, exit 0). In the guest: `MTLSGUARD: evidence: initrd0 … len=890080 sha256=043e34e7…` = the host file (`initrd sha256 match: yes`); `Calling main(3, ['igkmf', 'igkmf-app', 'run'])`; `appelfloader … ELF program loaded to 0x41f801000-0x41f8d4000 (864256 B), entry at 0x41f809840`; then **`Fatal glibc error: Cannot allocate TLS block`** and a clean shutdown (Firecracker exit 0 at t+0.76 s). No socket was opened: 0 bytes on the tap. The resolved config has `# CONFIG_LIBPOSIX_PROCESS_BRK is not set` (brk(2) absent) while posix-mmap is on; glibc 2.43's static start-up (`__libc_setup_tls` -> `_dl_early_allocate`) tries brk and then an anonymous mmap, so the mmap fallback must also have failed. Next: the strace image to see both return values |

## Pass 4 — why glibc cannot allocate its TLS block (written before runs 0007, 0008)

- **Hypothesis:** with `LIBPOSIX_PROCESS_BRK` off, glibc's `brk(0)` gets
  `-ENOSYS`, and its mmap fallback fails too; the strace image shows which call
  fails and with what.
- **Prediction:** the strace lines show `brk(…) = -ENOSYS` (twice), then
  `mmap(NULL, <size>, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`
  with an error return, then `write(2, "Fatal glibc error: Cannot allocate TLS block"…)`.
- **Falsification:** brk succeeds (then the message has another cause), or no
  mmap is attempted (glibc 2.43 dropped the fallback).

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0007 | guest build, `strace` variant | built; image `aba1ae64…`; same 2 warnings; `CONFIG_LIBSYSCALL_SHIM_STRACE=y` |
| 0008 | preset `trace` on the strace image, tap `igkf29105437` | **pass-4 prediction half right; the cause is a different syscall.** The strace lines: `brk(0x0, 0xd40, ...) = Function not implemented (-38)`, `brk(0xd1a, 0xd40, ...) = Function not implemented (-38)`, `mmap(NULL, 3392, PROT_READ\|PROT_WRITE, MAP_PRIVATE\|MAP_ANONYMOUS, fd:-1, 0) = va:0x100028e000` (**the fallback worked**), then **`arch_prctl(0x1002, 0x100028e3c0, ...) = Function not implemented (-38)`**, then `Fatal glibc error: Cannot allocate TLS block`. glibc allocated the block and failed to install it: `ARCH_SET_FS` (0x1002) is how x86_64 glibc sets its TLS pointer, and `TLS_INIT_TP`'s failure is reported with the same message. `arch_prctl` lives in `lib/posix-process/arch/x86_64/arch_prctl.c:15` behind `LIBPOSIX_PROCESS_ARCH_PRCTL` (`lib/posix-process/Config.uk:85`, default n, registered at `lib/posix-process/Makefile.uk:92`); app-elfloader's `Config.uk:1-25` does not select it, and its FC defconfigs (`defconfigs/fc-x86_64-initrd*`) name symbols this Unikraft no longer has, so nothing turns it on. Fix: `CONFIG_LIBPOSIX_PROCESS_ARCH_PRCTL=y`, plus `CONFIG_LIBPOSIX_PROCESS_BRK=y` for a Linux-like brk (config only, no source change) |

## Pass 5 — the syscall trace with arch_prctl and brk (written before runs 0009, 0010)

- **Hypothesis:** with `arch_prctl` and `brk` provided, glibc starts, and the
  trace preset's socket calls reach the hooked posix-socket path.
- **Prediction:** `arch_prctl(0x1002, …) = OK`; the banner prints
  `sysname="Unikraft"`; the strace shows `connect(fd:…, …:6443 …) = -EINPROGRESS`
  (N1) and blocking `connect()` = OK / -EACCES (B), each preceded by the
  guard's `connect … mesh destination` line; `accept(…)`/`accept4` for S1/S2 with
  the guard's accept lines; all 8 trace verdicts `PASS`; any remaining `-38`
  lines name the next gaps (expected harmless ones: `set_robust_list`, `rseq`,
  `prlimit64`, `readlink`-style probes).
- **Falsification:** another start-up abort; a socket syscall that the strace
  shows returning without a guard line (hook bypassed); a trace verdict `FAIL`.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0009 | guest build, `strace` variant + ARCH_PRCTL + BRK | built; image `47b067f9…`; same 2 warnings |
| 0010 | preset `trace` on the strace image, tap `igkf29105702` | **pass-5 prediction held: all 7 trace verdicts `PASS`**, Firecracker exit 0 on its own at t+1.64 s. glibc start-up: `brk(NULL) = va:0x1000174000`, `arch_prctl(0x1002, 0x10001743c0, ...) = 0x0`, then `set_tid_address`, `set_robust_list`, `rseq`, `readlinkat` = `-38` and `ioctl(0x1, 0x802c542a) = -22` (TCGETS2 on the console: glibc treats stdout as a non-tty), `prlimit64 = 0`, `getrandom(…, 8) = 8`, `mprotect(…, 20480, PROT_READ) = OK` (RELRO). Banner: `sysname="Unikraft" release="5.15.148-Ijiraq"`, `glibc 2.43`, `pid=1 ppid=0`, `AT_SYSINFO_EHDR=0`, `AT_EXECFN="igkmf-app"`, wall clock `946598400` (1999-12-31, as Spike A3). **Every socket syscall of the binary went through the hooks**: the guard's `connect …: mesh destination, BLOCKING` / `NON-BLOCKING … lwIP connect returned -115` lines appear before the strace line of the same syscall (`connect(fd:3, …) = Permission denied (-13)` for B, `= OK` for D, `= Operation now in progress (-115)` for N1/N5), and `accept(0x3, …) = 0x4` / `= Software caused connection abort (-103)` follow the guard's accept-hook hold. glibc's `accept()` issues syscall `accept` (43), not `accept4`. N1: `poll(…) = 0x0`, `write(fd:3, "IGKM-E-EARLY…", 40) = Resource temporarily unavailable (-11)` twice, EPOLLOUT after the guard's `record layer installed`. N5: `ERR HUP`, `SO_ERROR=13`, then 0. Scan: 9 574 TLS bytes, `IGKM-E-` 0; plaintext positive control on :5001. Custody 0/0/0 on all 6 relay connections, secrets `[1,1]` on the 3 ALLOW ones |

## Pass 6 — B2's full checklist from the binary (written before runs 0011, 0012)

Same predictions as pass 3 (all 18 verdicts `PASS` with B2 run 0005's
structural numbers; scan clean; custody 0; Firecracker exit 0 on its own), on
a `main` image rebuilt with ARCH_PRCTL + BRK. Falsification as in pass 3.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0011 | guest build, `main` + ARCH_PRCTL + BRK | built; image `cb6ba9ee…` (985 752 B); resolved `.config` sha256 `dfe2714b…`; same 2 warnings |
| 0012 | preset `mesh`, tap `igkf29105816` | **pass-6 prediction held: all 17 guest verdicts `PASS`** (A, B, D, S1, S2, N1–N6, K, R1, R2, E, E-after-relay-kill, F), Firecracker exit 0 on its own at t+10.85 s; native case A before boot `PASS`; `initrd sha256 match: yes`. B2 run 0005's structural numbers reproduced exactly: N1 `connect()` -1 errno 115, `poll(…,0)` = 0, early `write()` -1 errno 11 x2, `record layer installed` t=0.808028 → `app readiness none-> OUT` t=0.816247 → app `epoll_wait -> OUT` t=0.819261 after 6 empty 1-ms probes; host clock: relay SECRETS t+1.290502, first guest→peer app record `0x17/57` on the tap t+1.325286 (coalesced after the relayed Finished `0x17/603`, as B2); N3 `40033 bytes in 40033 one-byte read()s over 4 EPOLLIN (ET) wakeups, spurious=0`, fnv1a32 `0xccf62e40`; N4 `POLLIN wakeups=77 for 77 bytes … spurious=0`; K rx gen 1→3 and tx gen 1→3, `KeyUpdates rx=3 tx=3`, peer decrypted REQK2/REQK3; R1 NST #1 split, #2, #3+#4 in one record, then `KeyUpdate(update_not_requested)` rx gen 1 only, peer decrypted REQR2; R2 `FAIL CLOSED: … type 13 (CertificateRequest)`, app `IN ERR` errno 71, 0 bytes; N5 `ERR HUP SO_ERROR=13`, then 0; E exchange 2 at t+10.69 after the relay SIGKILL at t+2.62; F errno 103; N6 `SO_ERROR=103`. Scan: 64 348 TLS bytes, `IGKM-E-` 0, `unframed_bytes=0` in all 30 TLS directions; plaintext control on :5001. Custody: 0/0/0 on all 13 relay connections, secrets `[1,1]` on the 10 ALLOW, `[]` on the 3 DENY. Capture `runs/0012.tap.pcap` sha256 `2507a0a4…` (matches the host). Host state after cleanup identical |

The trace capture `runs/0010.tap.pcap` (sha256 `8162ac03…`) is kept too.

## Pass 7 — non-blocking accept from the binary (written before run 0013)

Preset `stretch`: B2's non-blocking listener under epoll, with glibc's
`accept4(…, SOCK_NONBLOCK)` (syscall 288, whereas glibc's `accept()` in run
0010 was syscall 43).

- **Hypothesis:** `accept4` from the binary reaches `do_accept4` →
  `uk_sys_accept` → the accept hook, and a non-blocking listener is not held.
- **Prediction:** as B2 run 0006: `accept4` returns an fd before the guard's
  `record layer installed` / `handshake done` lines; the allowed fd's first
  event is `IN` after REQI is decrypted; the denied fd reports `ERR HUP` and
  `read()` -1 errno 103; both verdicts `PASS`.
- **Falsification:** `accept4` blocking for the handshake, an event on the
  new fd before install, the denied fd delivering data.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0013 | preset `stretch`, tap `igkf29110003` | **pass-7 prediction held**: both verdicts `PASS`, Firecracker exit 0. Allowed caller: `accept4(SOCK_NONBLOCK) … returned 5 errno=0 after 16410 us (no handshake hold)` at t=0.254389, BEFORE `record layer installed` t=0.272579 and `handshake done after 35903 us` t=0.282121; first event `IN` at t=0.479673 with REQI. Denied caller: `accept4` returned at t=0.516914, `vsock rx DENY` t=0.548097, the fd reported `ERR HUP`, `read()` -1 errno 103. Same shape and the same 16.3 ms `accept4` as B2 run 0006 (the hook's own serial log lines) |

## Pass 8 — repeat of pass 6 on the same image (written before run 0014)

- **Hypothesis:** run 0012 is not a one-off interleaving.
- **Prediction:** all 17 verdicts `PASS` with the same structural numbers
  (N3 40 033 / 40 033 / 4 wakeups / spurious 0 / `0xccf62e40`; N4 77 for 77;
  K rx=3 tx=3; custody 0), timings different.
- **Falsification:** any `FAIL` or a different structural number.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0014 | preset `mesh` again, same image, tap `igkf29110053` | **pass-8 prediction held**: all 17 verdicts `PASS`, Firecracker exit 0 at t+10.83 s; N3 `40033 bytes in 40033 one-byte read()s over 4 EPOLLIN (ET) wakeups, spurious=0`, `0xccf62e40`; N4 77 for 77, spurious 0; K `KeyUpdates rx=3 tx=3`; 64 345 TLS bytes, `IGKM-E-` 0, `unframed_bytes=0` in all 30 TLS directions; custody 0/0/0 on 13 relay connections, secrets `[1,1]` x10, `[]` x3. Capture `runs/0014.tap.pcap` sha256 `409128d4…` (matches) |

**The main checklist passes from the unmodified binary.** On to the stretch.

## Pass 9 — stretch: a real-world binary, BusyBox wget (written before runs 0015, 0016)

The host's `/usr/bin/busybox` is static but not PIE (`ELF 64-bit LSB
executable`, run 0001), which app-elfloader refuses (`elf_load.c:148`:
`ELF executable is not position-independent!`). So BusyBox 1.37.0 is built
from the upstream tarball (sha256 checked against busybox.net's published
file) with the host gcc/glibc as static-pie, `allnoconfig` + the `busybox` and
`wget` applets, no source change. Preset `http-busybox`: the guest runs
`wget -O - http://192.168.204.1:6451/igkm-f-http` (plain HTTP); the peer's
`:6451` terminates TLS (identity peer-allowed) and answers one GET.

- **Hypothesis:** an off-the-shelf program that knows nothing about TLS gets
  its HTTP request carried over mutual TLS by the mechanism.
- **Prediction:** 0015: a static-pie busybox (`--list` shows `busybox`,
  `wget`), native `wget --help` prints usage. 0016: the native control (same
  file on the host, against `:6451`, no guest mechanism) FAILS: the peer logs
  `PEER[6451#1] handshake FAILED` (it received plaintext HTTP), proving the
  binary itself speaks plain HTTP. In the guest: the guard's `connect
  192.168.204.1:6451: mesh destination, BLOCKING` line, the relay's `handshake
  complete: policy ALLOW local …guest-client`, `PEER[6451#2] http rx plaintext
  request … GET /igkm-f-http HTTP/1.1`, and the body
  `IGKM-F-HTTP-BODY served by the host peer …` printed on the guest console by
  wget; the tap shows only TLS records on `:6451` (`GET /igkm-f-http`,
  `IGKM-F-` and `HTTP/1.1 200 OK` 0 times); Firecracker exits 0.
- **Falsification:** busybox fails to load or dies on a missing syscall;
  plaintext HTTP on the tap; no guard line for its connect.

| Run | What | Result |
|---|---|---|
| 0015 | BusyBox build | tarball `busybox-1.37.0.tar.bz2` sha256 `3311dff32e746499f4df0d5df04d7eb396382d7e108bb9250e7b519b837043a4` = busybox.net's published `.sha256` (`OK`). **Build failed, my error**: `CONFIG_EXTRA_LDFLAGS="-static-pie"` also reaches BusyBox's partial links, `ld.bfd: -r and -pie may not be used together`. Fixed by using `CONFIG_PIE=y` (`-fpie` on every object) and giving only the final link `-static-pie` via `make CFLAGS_busybox=-static-pie`. Not a Unikraft finding |
| 0016 | BusyBox build, `CONFIG_PIE=y` + `CFLAGS_busybox=-static-pie` | built (9 warnings, BusyBox's own); `trylink`: `Final link with: <none>`; **`ELF 64-bit LSB pie executable, x86-64, version 1 (GNU/Linux), static-pie linked … stripped`, Type DYN, no INTERP, no NEEDED, sha256 `f062e1b88be4a70ca00afec4ab8228a3a083e76875481da51ada675807e2440f`, 1 223 344 B**; `--list`: `sh`, `wget`; native run prints `BusyBox v1.37.0 … multi-call binary.` (`allnoconfig` has no usage texts, so `wget --help` prints nothing) |
| 0017 | preset `http-busybox`, image `cb6ba9ee…` (the checklist image, unchanged), tap `igkf29110401` | **pass-9 prediction held.** Native control, same file on the host against `:6451` without the mechanism: `native exit=1`, `PEER[6451#1] handshake FAILED: received corrupt message of type InvalidContentType; application plaintext bytes received=0` (the binary itself speaks plain HTTP). Guest: `initrd0 … len=1223344 sha256=f062e1b8…` (`match: yes`), `Calling main(5, ['igkmf', 'wget', '-O', '-', 'http://192.168.204.1:6451/igkm-f-http'])`, `wget: ELF program loaded to 0x41fa01000-0x41fb32000 (1249280 B)`, busybox prints `Connecting to 192.168.204.1:6451`, `MTLSGUARD: [c1] connect 192.168.204.1:6451: mesh destination, BLOCKING connect`, hold released after 37 679 us, `tx record … appdata plaintext=92` (the request), 2 NSTs discarded, `rx record … plaintext=168` (the response); busybox prints `IGKM-F-HTTP-BODY served by the host peer over the mesh (TLS terminated at the peer)` and `written to stdout`; Firecracker exit 0 at t+1.35 s. Relay: `handshake complete: policy ALLOW local …/guest-client <-> peer …/peer-allowed`, custody 0/0/0, secrets `[1,1]`. Peer: `PEER[6451#2] handshake OK … verified client SPIFFE ID=spiffe://overdrive.test/ns/default/sa/guest-client`, `http rx plaintext request (92 bytes, 4 lines): GET /igkm-f-http HTTP/1.1`, headers `Host: 192.168.204.1:6451`, `User-Agent: Wget`, `Connection: close`; `http tx 200 OK (168 bytes incl. body 84 bytes)`. Tap: `guest->peer … records=5 … unframed_bytes=0`, `peer->guest … records=6 … unframed_bytes=0`; `GET /igkm-f-http`, `IGKM-F-HTTP-BODY…`, `HTTP/1.1 200 OK` 0 times on TLS; plain streams 0 bytes. Capture `runs/0017.tap.pcap` sha256 `30c94ad3…` (matches) |

## Pass 10 — stretch: a Go HTTP client (written before runs 0018+)

`app-go/main.go`: standard-library `http.Client.Get` of the same URL. The Go
runtime uses its own netpoller (non-blocking sockets, edge-triggered epoll via
`epoll_pwait`, an eventfd to wake it), several OS threads (`clone`), futexes,
`sched_yield`, `nanosleep`, `sigaltstack`/`rt_sigaction`/`rt_sigprocmask` and
`tgkill` (async preemption). Built with the official Go toolchain (sha256 from
go.dev) as `CGO_ENABLED=0 -buildmode=pie -ldflags='-linkmode=external
-extldflags=-static-pie'`.

Source reading before the run: without `LIBPOSIX_PROCESS_SIGNAL` (which
requires a VFS through EXECVE), `rt_sigaction` is a stub that returns 0
(`lib/posix-process/signal/rt_sigaction.c:64-75`), so the runtime should not
abort at `initsig`, but signals are never delivered (no async preemption).
`futex` and `set_tid_address` need `LIBPOSIX_FUTEX`; `eventfd2` needs
`LIBPOSIX_EVENTFD`; `clone` is in `LIBPOSIX_PROCESS_MULTITHREADING` (on);
`sched_yield`/`sched_getaffinity` are in uksched (on).

- **Hypothesis:** app-elfloader runs the Go binary on the single-vCPU
  cooperative scheduler, and the Go netpoller's non-blocking connect/read
  path behaves under the guard's readiness like B2's epoll cases.
- **Prediction:** 0018: a static-pie Go ELF, no INTERP; runs natively (banner,
  exit 2 without an argument). Then a `go` image variant (main + FUTEX +
  EVENTFD, strace on first) prints `IGKM-F-GO: go1.x linux/amd64 GOMAXPROCS=1
  NumCPU=1`, the guard's `NON-BLOCKING connect` line, and `GET … -> "200 OK"
  … read 84 body bytes`; the tap shows only TLS on `:6451`.
- **Falsification (any of these is a precise DOESN'T WORK):** the loader
  refuses the ELF; the runtime aborts on a missing syscall; the process hangs
  because a Go thread spins without yielding on the cooperative scheduler;
  the request never completes because a readiness edge is lost.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0018 | Go toolchain + client build | Go **go1.27.1** `go1.27.1.linux-amd64.tar.gz`, sha256 `63d339f0da5ab53635a56f2490a7984dfe12dfcff22ad749f63edaf590168445` from go.dev's release JSON: `OK`. **Build refused:** `-linkmode=external requires external (cgo) linking, but cgo is not enabled`. go1.27.1 no longer allows an external (static-pie) link of a cgo-free program. Next: record what the internal linker's `-buildmode=pie` produces, and build with `CGO_ENABLED=1` (runtime/cgo + static glibc) for the static-pie |
| 0019 | Go client, `CGO_ENABLED=1` | the internal linker's `-buildmode=pie` gives `dynamically linked, interpreter /lib64/ld-linux-x86-64.so.2` (INTERP), which `APPELFLOADER_INITRDEXEC` cannot load (it loads no interpreter; the VFS mode side-loads one from a filesystem). With `CGO_ENABLED=1 -buildmode=pie -tags netgo,osusergo -ldflags='-linkmode=external -extldflags=-static-pie'`: **`ELF 64-bit LSB pie executable … static-pie linked`**, no INTERP, no NEEDED, go1.27.1, sha256 `1d39ec35bf7e0d1e7252ddf454c73990e026814a20e46a4e3c3d10a8d7c52651`, 10 145 848 B. Native: `IGKM-F-GO: go1.27.1 linux/amd64 GOMAXPROCS=16 NumCPU=16`, exit 2 without a URL. With runtime/cgo, Go's OS threads are created through glibc `pthread_create` |

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0020 | guest build, `go-strace` variant (main + FUTEX + EVENTFD + PIPE + strace) | built; image `da182069…` |
| 0021 | preset `http-go-trace`, tap `igkf29110911` | **Go stretch falsified at the first thread.** Native control: the same file on the host, without the mechanism, fails as expected (`malformed HTTP response "\x15\x03\x03\x00\x02\x022"`; the peer logs `handshake FAILED: received corrupt message of type InvalidContentType`). In the guest: `initrd sha256 match: yes`; `igkmf-go: ELF program loaded to 0x418001000-0x41a771000 (41353216 B)`; glibc start-up as run 0010 (now `set_tid_address(…) = 0x2` with posix-futex); the Go runtime reserves its arenas (`mmap(NULL, 536870912, PROT_NONE, …) = va:0x1004dee000` twice, `mmap(va:0x25fbd4000000, 67108864, …)`, all OK), `sched_getaffinity … stubbed … = 0x2000`, `sigaltstack`/`rt_sigprocmask`/`rt_sigaction` "stubbed" but returning 0 for every signal (no abort), then glibc `pthread_create`: `clone3(…) = Function not implemented (-38)`, fallback `clone(0x3d0f00, …)` and **`ERR: [libposix_process] <events.c @ 57> posix_clone … (igkmf-go): Unsupported clone flags requested: 0x3d0f00`**, `runtime/cgo: pthread_create failed: Operation not supported`, the runtime aborts (`tgkill(0x1, 0x2) … stubbed`), clean shutdown, Firecracker exit 0 at t+2.09 s. No socket opened. Cause: posix-process only accepts clone flags that some library registered a handler for (`lib/posix-process/events.c:34-60`); 0x3d0f00 = CLONE_VM\|FS\|FILES\|SIGHAND\|THREAD\|SYSVSEM\|SETTLS\|PARENT_SETTID\|CHILD_CLEARTID (glibc's standard set), and the only CLONE_FS handlers are posix-vfs's (`lib/posix-vfs/vfs.c:329`, under `LIBPOSIX_VFS_MULTICTX`) and vfscore's. With no VFS configured, no multithreaded glibc program can start a thread. Fix (config only): `CONFIG_LIBPOSIX_VFS=y` (built-in empty root, nothing mounted), `# CONFIG_APPELFLOADER_AUTOGEN is not set` |
| 0022 | guest build, `go-strace` + posix-vfs | built; image `19fab2cb…`; `HAVE_VFS=y` with `APPELFLOADER_INITRDEXEC=y` kept, `LIBPOSIX_VFS_MULTICTX=y`, AUTOGEN off; 1 warning |
| 0023 | preset `http-go-trace`, tap `igkf29111000` | **Go client WORKS (traced image).** Native control fails as in 0021. In the guest: `initrd sha256 match: yes`; glibc `pthread_create` now succeeds through the `clone(0x3d0f00)` fallback after `clone3 = -38`: four more threads (`gettid() = pid:2 … pid:5`), futex waits/wakes, `nanosleep` from sysmon; `IGKM-F-GO: go1.27.1 linux/amd64 GOMAXPROCS=1 NumCPU=1`; the netpoller: `epoll_create1(0x80000) = 0x3`, `eventfd2(0, …O_NONBLOCK) = fd:4`, `socket(AF_INET, SOCK_NONBLOCK\|SOCK_CLOEXEC\|SOCK_STREAM, 0) = fd:5`, guard `connect 192.168.204.1:6451: mesh destination, NON-BLOCKING connect (lwIP connect returned -115)`, `connect(fd:5, …) = Operation now in progress (-115)`, `epoll_pwait = 0x0` while the guard relays, guard `record layer installed` → `app readiness none-> OUT` → `epoll_pwait(…) = 0x1`, `getsockopt(SO_ERROR) = 0`, `getpeername`/`getsockname` OK, `setsockopt(TCP_NODELAY)` OK, `setsockopt(SO_KEEPALIVE)` OK, **`setsockopt(fd:5, 6, 4 = TCP_KEEPIDLE) = Protocol not available (-92)`** (lwIP; Go ignores it), `read = EAGAIN`, `write(fd:5, "GET /igkm-f-http HTTP/1."…, 110) = 110` → guard `tx record … plaintext=110`, 2 NSTs discarded, `rx record … plaintext=168` → `app readiness OUT -> IN OUT` → `epoll_pwait = 0x1` → `read(fd:5, "HTTP/1.1 200 OK\r\nContent"…) = 168`; **`IGKM-F-GO: GET http://192.168.204.1:6451/igkm-f-http -> "200 OK" proto=HTTP/1.1 content-length=84, read 84 body bytes (err=<nil>) after 148.926948ms`**, the body printed, `done, goroutines=1`; Firecracker exit 0 at t+1.81 s. Peer: `PEER[6451#2] handshake OK … verified client SPIFFE ID=…/guest-client`, `http rx plaintext request (110 bytes, 4 lines): GET /igkm-f-http HTTP/1.1`, `User-Agent: Go-http-client/1.1`, `Accept-Encoding: gzip`. Relay ALLOW, custody 0/0/0, secrets `[1,1]`. Tap: both directions `unframed_bytes=0`, `GET /igkm-f-http`/`IGKM-F-HTTP-BODY`/`HTTP/1.1 200 OK` 0 times; plain 0 bytes. Remaining gaps: `clone3`, `set_robust_list`, `rseq` = -38 (glibc falls back / tolerates); `openat`/`readlinkat` = -2 (empty root); `rt_sigaction`, `rt_sigprocmask`, `sigaltstack`, `tgkill`, `sched_getaffinity` "stubbed" (return success; no signal is ever delivered) |

## Pass 11 — the Go client on the non-traced image (written before runs 0024, 0025)

- **Hypothesis:** run 0023 did not depend on the strace printing (which
  slows every syscall and yields on the serial console).
- **Prediction:** `GET … -> "200 OK" … read 84 body bytes (err=<nil>)`, the same
  guard/relay/peer lines, tap clean, Firecracker exit 0.
- **Falsification:** a hang (a Go thread spinning without a syscall would
  starve the cooperative scheduler once strace no longer yields), a timeout,
  or plaintext on the wire.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0024 | guest build, `go` variant (no strace) | built; image `7d0da310…` |
| 0025 | preset `http-go`, tap `igkf29111133` | **pass-11 prediction held.** `IGKM-F-GO: GET http://192.168.204.1:6451/igkm-f-http -> "200 OK" proto=HTTP/1.1 content-length=84, read 84 body bytes (err=<nil>) after 66.048595ms`, body printed, `done, goroutines=3`; guard `NON-BLOCKING connect (lwIP connect returned -115)` → `record layer installed` t=0.321819 → `app readiness none-> OUT` t=0.330075; relay ALLOW with `…/guest-client`; peer `PEER[6451#2] handshake OK … verified client SPIFFE ID=…/guest-client`, `GET /igkm-f-http HTTP/1.1`, `User-Agent: Go-http-client/1.1`; Firecracker exit 0 at t+1.51 s; tap `unframed_bytes=0` both ways, HTTP lines 0 times on TLS, plain 0 bytes. Capture `runs/0025.tap.pcap` sha256 `e9ea348f…` (matches). Captures `runs/0021.tap.pcap` (`171d859a…`, no guest traffic) and `runs/0023.tap.pcap` (`97dba0ed…`) are kept |
| 0026 | final host-state check | nothing left: no firecracker/jailer/peer/relay/inclient/tcpdump or test-binary processes, no `igk*` or tun/tap links, no 192.168.203.x/204.x, no listeners on 5001/6443-6451/7100, no Unix sockets, no run dirs, 0 root-owned files in the scratch tree and in increments d/e/f; pristine clones clean; app-elfloader `a6c9dc2b` and lib-libelf `c03d4ceb` clean; B, B2 and B3 worktrees exactly their patches. Retained (user-owned, outside `~/overdrive`): `~/igkm-spike-a` 3.5 GB (was 1.4 GB): `igkmf-host-target` 163 MB, `stretch` 505 MB (Go 1.27.1, BusyBox source, Go cache), `build-igkmf-{main,strace,go,go-strace}` 316/316/361/361 MB, `out-igkmf` 36 MB, worktrees `unikraft-igkmf` 17 MB and `lib-lwip-igkmf` 0.5 MB, clones `app-elfloader` 1.3 MB and `lib-libelf` 4.3 MB; gitignored `increment-f/out` 12 MB (certs, `igkmf-app`, `busybox`, `igkmf-go`) |

## Result summary

| Item | Verdict | Runs |
|---|---|---|
| 1 unmodified Linux ELF, loaded and run by app-elfloader | WORKS, after enabling `LIBPOSIX_PROCESS_ARCH_PRCTL` (config only) | 0004, 0006, 0008, 0010, 0012 |
| 2 hooks fire for the binary's connect()/accept(); relay handshake with the host-held client identity | WORKS | 0010 (strace), 0012, 0014 |
| 3 tap: only TLS on mesh connections, markers 0; pass-through plaintext | WORKS | 0012, 0014 |
| 4 B2 readiness: EINPROGRESS, no EPOLLOUT and EAGAIN before install, ET 1-byte reads, poll() | WORKS, identical numbers to B2 | 0012, 0014 |
| 5 NewSessionTicket and KeyUpdate peers | WORKS | 0012, 0014 |
| 6 deny: blocking connect() fails / SO_ERROR, no app bytes | WORKS | 0010, 0012, 0014 |
| 7 relay kill after handshake; agent down fails closed | WORKS | 0012, 0014 |
| 8 accept side, blocking and non-blocking | WORKS | 0010, 0012, 0013 |
| 9 key custody | WORKS (0/0/0 on every relay connection) | 0010, 0012–0014, 0017, 0023, 0025 |
| 10 syscall gaps | listed in the findings; two blocking (`arch_prctl`, and for threaded programs the `CLONE_FS` gate), both config | 0008, 0010, 0021, 0023 |
| 11 stretch: real-world binaries | WORKS: BusyBox wget on the checklist image; Go `net/http` client on the Go image (+ futex, eventfd, pipe, posix-vfs) | 0017, 0023, 0025 |

## Edge cases discovered

1. **app-elfloader does not configure the syscalls glibc needs.** Its
   `Config.uk:1-25` selects the syscall shim, ULTLS and posix-time, and only
   *implies* the rest; it does not select `LIBPOSIX_PROCESS_ARCH_PRCTL`
   (default n), and its FC defconfigs name symbols Unikraft `eb8fa236` no longer
   has (`LIBVFSCORE_AUTOMOUNT_ROOTFS`, `LIBVFSCORE_ROOTFS_INITRD`,
   `LIBUKSIGNAL`, `LIBPOSIX_EVENT`). glibc's static start-up then dies with a
   misleading `Fatal glibc error: Cannot allocate TLS block` (the block WAS
   allocated; `arch_prctl(ARCH_SET_FS)` failed). Only the strace image showed it.
2. **Threads need a filesystem library.** posix-process refuses any clone flag
   no library registered a handler for (`lib/posix-process/events.c:34-60`),
   and only posix-vfs/vfscore register `CLONE_FS`. glibc's `pthread_create`
   always passes it, so without a VFS no multithreaded glibc program can start
   a thread (`pthread_create failed: Operation not supported`).
3. **Signals are stubs without a VFS.** `LIBPOSIX_PROCESS_SIGNAL` requires
   EXECVE, which requires `HAVE_VFS`; without it `rt_sigaction`,
   `rt_sigprocmask`, `sigaltstack` and `tgkill` return success and deliver
   nothing. Go tolerates it (no async preemption); a program that relies on a
   delivered signal would not work.
4. **Only static-pie ELFs load from the initrd.** Ubuntu's static busybox (ET_EXEC)
   is refused (`elf_load.c:148`); Go's internal linker's `-buildmode=pie` adds an
   INTERP, which initrd mode cannot satisfy; go1.27.1 refuses an external
   (static-pie) link without cgo, so the Go client carries runtime/cgo + static
   glibc and starts its threads through glibc `pthread_create`.
5. **glibc's stdout on the Unikraft console is block-buffered**: `ioctl(1,
   TCGETS2) = -EINVAL`, so glibc treats it as a non-tty (as when piped on Linux).
   The test program sets line buffering; BusyBox and Go write directly.
6. **The strace printer decodes clone flags wrongly** (prints
   `CLONE_NEWTIME|CLONE_FS|…` for glibc's 0x3d0f00); the raw value is in
   posix-process's error line.
7. **lwIP rejects `TCP_KEEPIDLE`** with -ENOPROTOOPT (`LWIP_TCP_KEEPALIVE`
   default n, `lib-lwip/Config.uk:255-257`); the guard passes setsockopt
   through, and Go ignores the error.
8. **glibc's `accept()` is syscall 43 and `accept4()` 288**; both reach
   `do_accept4` → `uk_sys_accept` → the accept hook.
9. **The Go runtime's large PROT_NONE reservations work** in a 512 MiB guest
   (2 × 512 MiB plus 64 MiB arenas, `LIBUKVMEM` demand paging); it runs with
   `GOMAXPROCS=1` from the `sched_getaffinity` stub (CPU0 only).
| 0027 | sync + read-only check | an 8 MB macOS `arm64` Mach-O `app-go/main`, produced locally by an editor diagnostics hook compiling `main.go` (not by this probe), had been rsynced into the host's `~/overdrive` mirror by runs 0018–0026; deleted locally, and this sync removed it from the host mirror (`app-go/` now holds only `go.mod`, `main.go`); no firecracker, 0 `igk*` links |
