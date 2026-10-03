# Spike B3 findings: the in-guest mTLS mechanism for unmodified Linux binaries under Unikraft app-elfloader on Firecracker

GH #303, feature `in-guest-kernel-mtls`. PROBE phase, throwaway. Builds on Spike B2
(`findings-unikraft-hook-nonblocking.md`, `spike-scratch/in-guest-kernel-mtls/increment-e/`).

Probe sources, the resolved configs and raw captures are in
`spike-scratch/in-guest-kernel-mtls/increment-f/`:
- runs `runs/0001` to `runs/0027`, each `{meta,stdout,stderr}`, append-only, host
  identifiers redacted
- tap captures `runs/{0010,0012,0014,0017,0021,0023,0025}.tap.pcap`

The increment `README.md` has every pre-run hypothesis, prediction and falsification,
and the full run log. The key runs:
- run 0012: primary evidence for items 1–9
- run 0014: a repeat of 0012
- run 0010: the syscall trace
- run 0013: the non-blocking accept
- runs 0017 and 0025: the stretch

The probe agent could not write under `docs/`, so the orchestrator wrote this file from its
report and re-checked the verdicts, initrd hash match, wire scans, HTTP results and custody
lines against runs 0012, 0013, 0014, 0017 and 0025.

## Verdict: WORKS (all eleven items)

An unmodified Linux x86_64 program runs under Unikraft's `app-elfloader` and gets the
same transparent mutual TLS 1.3 as Spike B2's native app. The program was built with
the host's stock gcc 15.2.0 and glibc 2.43 as a static-pie ELF, and handed to
Firecracker as the initrd.

Its socket, poll and epoll syscalls reach the same hooked `posix-socket` functions.
B2's checklist passes with identical structural numbers. Two real-world programs work
through the mesh: BusyBox `wget` and a Go `net/http` client.

**No new Unikraft source patch.** The mechanism is Spike B2's patches and library,
unchanged. What the binary needed was configuration app-elfloader does not set:
- `arch_prctl`, for every glibc program
- a VFS library, for any program that starts threads

| # | Checklist item | Verdict | Runs |
|---|---|---|---|
| 1 | unmodified Linux ELF (file, SHA-256 on host = in guest); app-elfloader loads and runs it | **WORKS** after enabling `LIBPOSIX_PROCESS_ARCH_PRCTL` (config) | 0004, 0006, 0008, 0010, 0012 |
| 2 | hooks fire for the binary's `connect()`/`accept()`; relay handshake with the host-held client identity | **WORKS** | 0010, 0012, 0014 |
| 3 | tap: only TLS records on mesh connections, markers 0; pass-through plaintext | **WORKS** | 0012, 0014 |
| 4 | B2 readiness: EINPROGRESS, no EPOLLOUT and `write()` = EAGAIN before install, ET 1-byte reads, `poll()` | **WORKS**, B2's numbers exactly | 0012, 0014 |
| 5 | NewSessionTicket and KeyUpdate peers | **WORKS** | 0012, 0014 |
| 6 | deny: blocking `connect()` fails, non-blocking gives SO_ERROR, no app bytes | **WORKS** | 0010, 0012, 0014 |
| 7 | relay kill after the handshake; agent down fails closed | **WORKS** | 0012, 0014 |
| 8 | accept side, blocking and non-blocking | **WORKS** | 0010, 0012, 0013 |
| 9 | key custody: SVID keys never cross vsock | **WORKS** (0/0/0 on every relay connection) | 0010, 0012–0014, 0017, 0023, 0025 |
| 10 | syscall gaps | two blocking gaps, both config; the rest harmless | 0008, 0010, 0021, 0023 |
| 11 | stretch: real-world binaries | **WORKS**: BusyBox 1.37.0 `wget` on the checklist image; Go 1.27.1 `net/http` client on an image with futex, eventfd, pipe and posix-vfs added | 0017, 0023, 0025 |

Verdict counts:
- all 17 guest verdicts `PASS` in run 0012, and again in run 0014
- both non-blocking accept verdicts `PASS` in run 0013
- all 7 trace verdicts `PASS` in run 0010
- Firecracker exited 0 on its own in every boot

Not shown (out of scope):
- SMP, and one guard event loop per guest
- the lwIP delayed-ACK latency
- performance
- dynamically linked binaries (app-elfloader's VFS load mode)

**Substrate** (run 0001 fail-closed probes, re-checked by the harness preflight on
every run):
- native metal host, x86_64, AMD EPYC 8024P
- `uname -r` = `7.0.0-29-generic`
- `systemd-detect-virt` = `none`, `/dev/kvm` API 12
- Firecracker v1.17.0 (`99ad0f5c…`)

**Guest:**
- Unikraft `eb8fa236` (`Ijiraq 0.21.0~eb8fa236-custom`), app-elfloader `a6c9dc2b`
- 1 vCPU, 512 MiB, cooperative scheduler
- `uname(2)` inside the guest: `sysname="Unikraft" release="5.15.148-Ijiraq"`,
  `pid=1`, `AT_SYSINFO_EHDR=0` (no vDSO)

## How the binary is run

- **Load mode:** `APPELFLOADER_INITRDEXEC`. The Firecracker initrd IS the ELF file,
  with no filesystem. This mode loads only static-pie executables.
- **Command line:** with `APPELFLOADER_CUSTOMAPPNAME=y`, everything after `--` is the
  program's argv:

  ```
  igkmf netdev.ip=192.168.204.2/24:192.168.204.1 mtlsguard.mesh=192.168.204.1:6443-6451 mtlsguard.mesh_in=192.168.204.2:7443 mtlsguard.agent_port=7100 -- igkmf-app run
  ```

- **Exit:** the program's `exit_group` ends the init process, `LIBUKBOOT_MAINTHREAD`
  shuts the guest down, and Firecracker exits 0.

The test program `app-linux/igkmf_app.c` is B2's `app/main.c` ported to Linux. The only
functional changes are:
- `clock_gettime(2)` instead of Unikraft's clock calls
- `#define _GNU_SOURCE`, because glibc declares `accept4` only under it
- a line-buffered stdout
- an identity banner and presets

The wire lines are B2's, so B2's peer and scanner apply unchanged.

## Evidence (predicted vs actual, real output)

### 1. An unmodified ELF, loaded and run: WORKS (after one config change)

Predicted: a static-pie ELF from the host toolchain, the same SHA-256 in the guest,
and a clean start.

Actual, run 0004 (host):

```
gcc -O2 -g0 -Wall -Wextra -fPIE -static-pie -o …/out/bin/igkmf-app …/app-linux/igkmf_app.c
…/igkmf-app: ELF 64-bit LSB pie executable, x86-64, version 1 (GNU/Linux), static-pie linked, BuildID[sha1]=26c851e4…, for GNU/Linux 3.2.0, not stripped
043e34e746ef86eb2a97c3334920edc1022ef34e54186213eb33571f6357b179  …/igkmf-app
  Type:                              DYN (Position-Independent Executable file)
--- dynamic section NEEDED entries (none expected):
  (none)
--- strings containing 'unikraft' / 'ukplat' (0 expected):
0
GNU C Library (Ubuntu GLIBC 2.43-2ubuntu2.4) stable release version 2.43.
IGKM-F: uname(2): sysname="Linux" release="7.0.0-29-generic" …
```

The source includes only standard headers. Before each boot, the runner ran the same
file natively on the host, and case A passed:
`IGKM-F: VERDICT native A pass-through=PASS`.

In the guest, lib-mtlsguard hashes the initrd before app-elfloader parses it. This is
evidence code only. The runner compares the hash with the host file:

```
MTLSGUARD: evidence: initrd0 (the ELF app-elfloader will load) vbase=0x1ff26000 pg_off=0 len=890080 sha256=043e34e746ef86eb2a97c3334920edc1022ef34e54186213eb33571f6357b179
[    0.170969] Info: [appelfloader] <main.c @  299> Image at 0x1ff26000, len 890080 bytes
[    0.173730] Info: [appelfloader] <main.c @  358> igkmf-app: ELF program loaded to 0x41f801000-0x41f8d4000 (864256 B), entry at 0x41f809840
initrd sha256 match: yes
```

**The first boot failed** (run 0006), before any socket call:

```
Fatal glibc error: Cannot allocate TLS block
```

The strace image (run 0008) showed that glibc allocated the TLS block but could not
install it:

```
brk(0x0, 0xd40, ...) = Function not implemented (-38)
brk(0xd1a, 0xd40, ...) = Function not implemented (-38)
mmap(NULL, 3392, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, fd:-1, 0) = va:0x100028e000
arch_prctl(0x1002, 0x100028e3c0, ...) = Function not implemented (-38)
Fatal glibc error: Cannot allocate TLS block
```

- `ARCH_SET_FS` (0x1002) is how x86_64 glibc sets its thread pointer.
- `arch_prctl` is at `lib/posix-process/arch/x86_64/arch_prctl.c:15`, behind
  `LIBPOSIX_PROCESS_ARCH_PRCTL` (`lib/posix-process/Config.uk:85-87`, default n).
- app-elfloader's `Config.uk:1-25` does not select it.
- app-elfloader's Firecracker defconfigs (`defconfigs/fc-x86_64-initrd*`) name
  symbols this Unikraft no longer has: `LIBVFSCORE_AUTOMOUNT_ROOTFS`,
  `LIBVFSCORE_ROOTFS_INITRD`, `LIBUKSIGNAL`, `LIBPOSIX_EVENT`. So nothing turns
  `arch_prctl` on.
- The pass-4 prediction, that the mmap fallback also fails, was wrong: mmap worked.

With `LIBPOSIX_PROCESS_ARCH_PRCTL=y`, plus `LIBPOSIX_PROCESS_BRK=y` for a Linux-like
brk, glibc starts. Run 0010:

```
brk(NULL) = va:0x1000174000
brk(va:0x1000174d40) = va:0x1000174d40
arch_prctl(0x1002, 0x10001743c0, ...) = 0x0
mprotect(va:0x41f8be000, 20480, PROT_READ) = OK
IGKM-F: uname(2): sysname="Unikraft" release="5.15.148-Ijiraq" version="0.21.0~eb8fa236-custom" machine="x86_64"
IGKM-F: libc: glibc 2.43 (stable), statically linked into this ELF; pid=1 ppid=0
IGKM-F: auxv: AT_ENTRY=0x41f809840 AT_PHDR=0x41f801040 AT_BASE=0 AT_PAGESZ=4096 AT_SYSINFO_EHDR=0 AT_RANDOM=set AT_EXECFN="igkmf-app"
```

### 2. The hooks fire for the binary's syscalls: WORKS

In the strace image, each guard line is printed inside the syscall, before the strace
line of that syscall (strace prints on return). Run 0010:

```
MTLSGUARD: [c1] connect 192.168.204.1:6444: mesh destination, BLOCKING connect (lwIP connect returned 0) t=0.245258
MTLSGUARD: [c1] host DENY: policy deny: local spiffe://overdrive.test/ns/default/sa/guest-client <-> peer spiffe://overdrive.test/ns/default/sa/peer-denied (…)
MTLSGUARD: [c1] connect 192.168.204.1:6444: blocking hold released: connect() = -13 after 32947 us t=0.278206
connect(fd:3, sockaddr:{…}, 16) = Permission denied (-13)
MTLSGUARD: [c3] accept 192.168.204.2:7443 <- 192.168.204.1:55930: blocking hold released: accept() gets the socket after 34623 us t=0.615102
accept(0x3, 0x1000489a60, ...) = 0x4
MTLSGUARD: [c5] connect 192.168.204.1:6443: returning EINPROGRESS now; the guard thread drives TCP + handshake t=0.817264
connect(fd:3, sockaddr:{…}, 16) = Operation now in progress (-115)
```

The build confirms the dispatch path. `uk_syscall_r_connect` calls the hooked
function between the syscall enter and exit tables (run 0011 disassembly):

```
0000000000139670 <uk_syscall_r_connect>:
  139687:  call 154160 <_uk_syscall_wrapper_do_entertab>
  139695:  call 139150 <__uk_syscall_r_connect>
  13969e:  call 1541d0 <_uk_syscall_wrapper_do_exittab>
__uk_syscall_r_connect  139281: call 1abfd0 <uk_socket_connect_hook>
do_accept4              133c98: call 133b90 <uk_sys_accept>
```

glibc's `accept()` is syscall 43 and `accept4()` is syscall 288. Both reach
`do_accept4` → `uk_sys_accept` → the accept hook (runs 0010 and 0013).

The relay completes every allowed handshake with the host-held client identity. Run
0012:

```
RELAY[c5] handshake complete: policy ALLOW local spiffe://overdrive.test/ns/default/sa/guest-client <-> peer spiffe://overdrive.test/ns/default/sa/peer-allowed; suite=TLS13_AES_128_GCM_SHA256 … host check HKDF(secret)==rustls key/iv tx=true rx=true t+1.290491s
```

### 3. Tap: only TLS, markers 0 times: WORKS

Run 0012 (run 0014 gave the same):

```
SCAN   tls streams: 64348 bytes, marker(IGKM-E-) occurrences=0, marker(IGKM-F-) occurrences=0, marker(igkm-f) occurrences=0
SCAN   plain streams: 85 bytes, marker(IGKM-E-) occurrences=2, …
SCAN     plain: b'IGKM-E-PLAIN-REQ pass-through guest->host\n' occurrences=1
```

- All 30 TLS directions show `unframed_bytes=0`, in both runs.
- The `:5001` pass-through is the positive control: the scan finds its plaintext.
- The capture hashes match the ones printed on the host: `0012.tap.pcap`
  `2507a0a4…`, `0014.tap.pcap` `409128d4…`.

### 4. B2's readiness behaviour: WORKS, with B2's numbers

Case N1 (non-blocking, level-triggered epoll), run 0012:

```
IGKM-F: N1 epoll-LT client-first connect() returned -1 errno=115 (Operation now in progress) after 6984 us t=0.778180
IGKM-F: N1 epoll-LT client-first poll(POLLIN|POLLOUT, 0 ms) right after connect() = 0 revents=0x0 t=0.780183
IGKM-F: N1 epoll-LT client-first write(40 bytes) before EPOLLOUT returned -1 errno=11 (Resource temporarily unavailable) t=0.783769
MTLSGUARD: [c5] TCP connected after 14636 us (lwIP raw EPOLLOUT at t=0.780180); the app still sees no EPOLLOUT -> relay the handshake
MTLSGUARD: [c5] record layer installed: … app write() EAGAINs during the handshake=2, read() EAGAINs=0 t=0.808028
MTLSGUARD: [c5] app readiness none-> OUT (published by the guard) t=0.816247
IGKM-F: N1 epoll-LT client-first epoll_wait -> OUT (LT) after 6 empty 1-ms probes t=0.819261
```

On the host clock:
- the relay sent SECRETS at t+1.290502
- the first guest→peer application record, `0x17/57`, is on the tap at t+1.325286
- it shares a TCP segment with the relayed Finished record (`0x17/603`), exactly as
  in B2

Cases N3 (edge-triggered epoll, 1-byte reads) and N4 (`poll()`, one byte per wakeup):

```
IGKM-F: N3 received 40033 bytes in 40033 one-byte read()s over 4 EPOLLIN (ET) wakeups, spurious=0, in 50 ms; bulk 40000 bytes fnv1a32=0xccf62e40, bytes differing from the pattern=0
IGKM-F: N4 POLLIN wakeups=77 for 77 bytes (level-triggered readiness kept while plaintext stays buffered), spurious=0
```

These are B2 run 0005's numbers exactly. Run 0014 printed them again.

### 5. NewSessionTicket and KeyUpdate: WORKS

Run 0012:

```
MTLSGUARD: [c10] rx post-handshake NewSessionTicket #1 (53 B body) in record seq=0 -> discarded (no resumption)
MTLSGUARD: [c10] rx handshake message continues in the next record (20 B buffered)
MTLSGUARD: [c10] rx post-handshake NewSessionTicket #3 … #4 (53 B body) in record seq=2 -> discarded
MTLSGUARD: [c10] rx KeyUpdate(update_not_requested) in record seq=4 -> … rx keys now generation 1, rx seq 0
MTLSGUARD: [c9] close: state=established tx records=3 rx records=3 NewSessionTickets discarded=2 KeyUpdates rx=3 tx=3
PEER[6446#1] rx plaintext line 3 (35 bytes): IGKM-E-REQK3 keyupdate guest->peer\n
PEER[6448#1] raw rx record rx_seq=1 wire=63 inner 0x17 plaintext (41 bytes): IGKM-E-REQR2 after-keyupdate guest->peer\n
MTLSGUARD: [c11] FAIL CLOSED: unexpected post-handshake handshake message type 13 (CertificateRequest) -> read()/write() now return -71, fatal alert sent
IGKM-F: R2 epoll events=IN ERR  read result=1 n=-71 (Protocol error); application bytes delivered=0
```

### 6. Deny: WORKS

```
IGKM-F: B deny connect() returned -1 errno=13 (Permission denied) after 34815 us
IGKM-F: N5 nb-deny wait result=1 events=ERR HUP  SO_ERROR=13 (Permission denied), SO_ERROR again=0, write() afterwards=-1 errno=13
SCAN conn client guest:60534 -> server peer:6444 (TLS) …
SCAN   guest->peer: 200 bytes flags[FIN,SYN] … records=2 (0x17 count=0) unframed_bytes=0
SCAN     records: 0x16/189 0x14/1
```

No `0x17` record left the guest on either denied connection.

### 7. Relay kill and agent down: WORKS

```
RUNNER: relay killed t+2.635765s; kill -0: gone
PEER[6450#1] rx plaintext line 2 (41 bytes): IGKM-E-REQ2 after-relay-kill guest->peer\n t+10.694427s
MTLSGUARD: [c14] host relay unreachable on vsock cid 2 port 7100 (-111) -> fail closed
IGKM-F: F agent-down connect() returned -1 errno=103 (Software caused connection abort)
IGKM-F: N6 nb-agent-down wait result=1 events=ERR HUP  SO_ERROR=103 …, SO_ERROR again=0
```

### 8. Accept side: WORKS

Blocking listener, run 0012:
- `S1 accept() returned 4`. The caller saw `server SPIFFE ID=…/guest-server` and the
  RESPI line.
- `S2 accept() returned -1 errno=103`. The denied caller got 0 bytes.

Non-blocking listener, run 0013:

```
MTLSGUARD: [c1] accept …: returning the fd now; handshake continues in the guard thread (app readiness withheld) t=0.251641
IGKM-F: SN accept4(SOCK_NONBLOCK) on the non-blocking listener returned 5 errno=0 after 16410 us (no handshake hold) t=0.254389
MTLSGUARD: [c1] record layer installed: … peer identity (from host) spiffe://overdrive.test/ns/default/sa/peer-client … t=0.272579
IGKM-F: SN fd 5 events=IN  read()=32 errno=0 (-) t=0.479673
IGKM-F: SN fd 5 events=ERR HUP  read()=-1 errno=103 (Software caused connection abort) t=0.558583
```

### 9. Key custody: WORKS

Every relay connection in runs 0012–0014, 0017, 0023 and 0025 reported:

```
RELAY[c5] custody: host->guest total 1037 bytes; client SVID key occurrences PKCS#8 DER=0 PEM=0 private scalar=0; guest-server SVID key occurrences PKCS#8 DER=0 PEM=0 private scalar=0; application traffic secrets sent … = [1,1]
```

In run 0012:
- the 10 allowed connections sent the traffic secrets once each (`[1,1]`)
- the 3 denied connections sent none (`[]`)

This is B2's custody model, unchanged.

### 10. Syscall gaps

**Two gaps block programs. Both are configuration, not missing code.**

| Syscall / behaviour | Result | Where | Effect |
|---|---|---|---|
| `arch_prctl(ARCH_SET_FS)` | `-ENOSYS` | `lib/posix-process/arch/x86_64/arch_prctl.c:15`, behind `LIBPOSIX_PROCESS_ARCH_PRCTL` (`lib/posix-process/Config.uk:85-87`, default n; registered at `lib/posix-process/Makefile.uk:92`); not selected by `app-elfloader/Config.uk:1-25` | **blocks every glibc program**: `Fatal glibc error: Cannot allocate TLS block` (run 0008) |
| `clone(0x3d0f00)` from glibc `pthread_create` | `-ENOTSUP` | posix-process accepts only clone flags that some library registered a handler for (`lib/posix-process/events.c:34-60`). The only `CLONE_FS` handlers are posix-vfs (`lib/posix-vfs/vfs.c:329`, under `LIBPOSIX_VFS_MULTICTX`) and vfscore (`lib/vfscore/syscalls.c:1264`) | **blocks any threaded glibc program** without a VFS: `pthread_create failed: Operation not supported` (run 0021) |

**The rest were harmless for these programs:**

| Syscall | Result | Why | Where |
|---|---|---|---|
| `brk` | `-ENOSYS`; glibc falls back to mmap | `LIBPOSIX_PROCESS_BRK` is off by default; enabled here | `lib/posix-process/Config.uk:18`, `Makefile.uk:63` |
| `set_tid_address` | `-ENOSYS` in the checklist image | provided by posix-futex | `lib/posix-futex/futex.c:390`, `Makefile.uk:15` |
| `set_robust_list`, `rseq`, `clone3` | `-ENOSYS` | not implemented at `eb8fa236`. glibc tolerates the first two, and falls back from `clone3` to `clone` | — |
| `readlinkat`, `openat` | `-ENOSYS` without a VFS; `-ENOENT` with posix-vfs's empty root | VFS syscalls | `lib/posix-vfs/Makefile.uk:42`, `lib/vfscore/Makefile.uk:33` |
| `ioctl(1, TCGETS2)` | `-EINVAL` | glibc then treats stdout as a non-tty and block-buffers it | posix-tty |
| `rt_sigaction`, `rt_sigprocmask`, `sigaltstack`, `tgkill` | "stubbed": return 0, deliver nothing | real signals need `LIBPOSIX_PROCESS_SIGNAL`, which selects EXECVE, which needs `HAVE_VFS` | `lib/posix-process/signal/rt_sigaction.c:64-75`, `lib/posix-process/Config.uk:63-66,78-83` |
| `sched_getaffinity` | "stubbed": reports CPU0 only (Go runs with `GOMAXPROCS=1`) | — | `lib/uksched/sched.c:431-443` |
| `setsockopt(IPPROTO_TCP, TCP_KEEPIDLE)` | `-ENOPROTOOPT` from lwIP (the guard passes setsockopt through) | `LWIP_TCP_KEEPALIVE` is off by default | `lib-lwip/Config.uk:255-257`, `include/lwipopts.h:168-170` |

**Nothing the mechanism relies on behaves differently for the binary.** These calls
reached the same handlers and gave B2's results: `connect`, `accept`, `accept4`,
`getsockopt(SO_ERROR)`, `epoll_create1`, `epoll_ctl`, `epoll_wait`, `epoll_pwait`,
`poll`, `read`, `write`, `close`, `clock_nanosleep`.

### 11. Stretch: real-world binaries: WORKS

**BusyBox wget** (run 0017, on the unchanged checklist image):
- Source: the upstream `busybox-1.37.0.tar.bz2`. Its sha256 `3311dff3…` equals
  busybox.net's published `.sha256` file.
- Configuration: `allnoconfig` plus the `busybox` and `wget` applets, no source
  change.
- Built as static-pie with `CONFIG_PIE=y` and `make CFLAGS_busybox=-static-pie`.
  Result: sha256 `f062e1b8…`, 1 223 344 B.

Native control: the same file on the host, without the mechanism, against the TLS
port. It fails because the program speaks plain HTTP:

```
native exit=1
PEER[6451#1] handshake FAILED: received corrupt message of type InvalidContentType; application plaintext bytes received=0
```

In the guest:

```
Calling main(5, ['igkmf', 'wget', '-O', '-', 'http://192.168.204.1:6451/igkm-f-http'])
Connecting to 192.168.204.1:6451 (192.168.204.1:6451)
MTLSGUARD: [c1] connect 192.168.204.1:6451: mesh destination, BLOCKING connect (lwIP connect returned 0)
MTLSGUARD: [c1] connect 192.168.204.1:6451: blocking hold released: connect() = 0 after 37679 us
IGKM-F-HTTP-BODY served by the host peer over the mesh (TLS terminated at the peer)
PEER[6451#2] handshake OK … verified client SPIFFE ID=spiffe://overdrive.test/ns/default/sa/guest-client mode=Http
PEER[6451#2] http rx plaintext request (92 bytes, 4 lines): GET /igkm-f-http HTTP/1.1   (User-Agent: Wget)
SCAN     tls: b'GET /igkm-f-http' occurrences=0
SCAN     tls: b'HTTP/1.1 200 OK' occurrences=0
SCAN   plain streams: 0 bytes
```

**Go `net/http` client** (run 0023 traced, run 0025 untraced):
- Standard library only. It uses Go's own netpoller (non-blocking sockets,
  edge-triggered epoll, an eventfd) and several OS threads.
- Toolchain: go1.27.1. The tarball sha256 `63d339f0…` came from go.dev's release
  JSON.
- go1.27.1 refuses an external link without cgo: `-linkmode=external requires
  external (cgo) linking, but cgo is not enabled`.
- Its internal linker's `-buildmode=pie` adds an `INTERP`, which initrd mode cannot
  load.
- So the client is built with `CGO_ENABLED=1 -buildmode=pie -tags netgo,osusergo
  -ldflags='-linkmode=external -extldflags=-static-pie'`. Result: static-pie,
  sha256 `1d39ec35…`, 10 MB. It starts its threads through glibc `pthread_create`.
- The first attempt hit the `CLONE_FS` gate (run 0021). Its image therefore adds
  posix-vfs (a built-in empty root, nothing mounted), posix-futex, posix-eventfd and
  posix-pipe.

Run 0025:

```
IGKM-F-GO: go1.27.1 linux/amd64 GOMAXPROCS=1 NumCPU=1 args=["igkmf-go" "http://192.168.204.1:6451/igkm-f-http"]
MTLSGUARD: [c1] connect 192.168.204.1:6451: mesh destination, NON-BLOCKING connect (lwIP connect returned -115)
MTLSGUARD: [c1] app readiness none-> OUT (published by the guard) t=0.330075
IGKM-F-GO: GET http://192.168.204.1:6451/igkm-f-http -> "200 OK" proto=HTTP/1.1 content-length=84, read 84 body bytes (err=<nil>) after 66.048595ms
PEER[6451#2]   header: User-Agent: Go-http-client/1.1
SCAN   guest->peer: 965 bytes flags[SYN] … records=5 (0x17 count=3) unframed_bytes=0
```

Run 0023 (traced) shows the path in Go's own syscalls:

```
clone3(0x1000489830, 0x58, ...) = Function not implemented (-38)
gettid() = pid:3 … gettid() = pid:5
epoll_create1(0x80000, 0x0, ...) = 0x3
eventfd2(0, O_RDONLY|O_CLOEXEC|O_NONBLOCK) = fd:4
socket(AF_INET, SOCK_NONBLOCK|SOCK_CLOEXEC|SOCK_STREAM, 0) = fd:5
connect(fd:5, …) = Operation now in progress (-115)
epoll_pwait(0x3, 0x10004893cc, ...) = 0x1
setsockopt(fd:5, 6, 4, "\x1E\x00\x00\x00", 4) = Protocol not available (-92)
write(fd:5, "GET /igkm-f-http HTTP/1."..., 110) = 110
read(fd:5, <out>"HTTP/1.1 200 OK\x0D\x0AContent"..., 4096) = 168
```

- Go's netpoller behaved like B2's edge-triggered epoll cases.
- Its five threads ran on the single-vCPU cooperative scheduler without starving the
  guard thread or lwIP. Every thread blocks in a syscall that yields (futex,
  nanosleep, epoll_pwait).

## Build and Kconfig changes versus Spike B2

**Unikraft source: none.** B2's patches were applied read-only to new worktrees:
- 0001 and 0002: core `lib/posix-socket`, 49 lines (increment d)
- 0003: lib-lwip, +42/−2 (increment e)

**lib-mtlsguard:** B2's `guard.c` is unchanged apart from `initrd_evidence()`. The
diff is 39 lines added, 0 removed: a header comment, two includes, and a 32-line
`uk_late_initcall` that hashes the initrd. It is evidence code only.

**Build inputs:**
- `A=` is app-elfloader `a6c9dc2b`, unmodified.
- `L=lib-libelf:lib-lwip:lib-mtlsguard`.
- lib-libelf `c03d4ceb` vendors its sources; nothing is fetched for it.

**Kconfig for the checklist image** (fragment `config/igkmf.defconfig`, resolved
`config/igkmf.config`). B2's platform, console, virtio-net/vsock, posix-socket, lwIP,
mtlsguard, posix-time and posix-poll settings are unchanged. Added:
- `APPELFLOADER_INITRDEXEC`, `APPELFLOADER_CUSTOMAPPNAME`, `APPELFLOADER_STACK_NBPAGES=256`
- `LIBPOSIX_PROCESS` and `LIBPOSIX_PROCESS_MULTITHREADING`
- **`LIBPOSIX_PROCESS_ARCH_PRCTL`** (required) and `LIBPOSIX_PROCESS_BRK`
- `LIBUKRANDOM` with `LIBUKRANDOM_GETRANDOM`
- VFS off

**Go image only** (fragment `config/igkmf-go.fragment`, resolved
`config/igkmf-go.config`), added on top:
- `LIBPOSIX_FUTEX`, `LIBPOSIX_EVENTFD`, `LIBPOSIX_PIPE`
- **`LIBPOSIX_VFS`** (required for threads)
- `APPELFLOADER_AUTOGEN` off

**Images:**
- checklist `cb6ba9ee…`, and its strace variant `47b067f9…`
- Go `7d0da310…`, and its strace variant `19fab2cb…`
- the first checklist image, `273d7744…`, lacked `arch_prctl`

**Build warnings:** 2, neither in the mechanism:
- `lib/posix-tty/serial.c:162 'tty_cons' may be used uninitialized`
- `app-elfloader/elf_load.c:71 'get_phdr_mmap_prot' defined but not used`

**Host side:**
- B2's host crate and lockfile, unchanged except `peer.rs`, which gains the `:6451`
  HTTP-over-TLS responder.
- `pcap_scan.py` gains port 6451 and the HTTP markers.

## Edge cases discovered

1. **app-elfloader does not configure what glibc needs, and the failure is
   misleading.** The message is `Cannot allocate TLS block`, but the block was
   allocated; `arch_prctl` was missing. Only the strace image showed this.
2. **Threads need a filesystem library.** The `CLONE_FS` gate refuses glibc's
   standard `pthread_create` flags unless posix-vfs or vfscore is configured.
3. **Signals are stubs without a VFS.** Handlers are registered but never invoked.
   Go ran without async preemption. A workload that relies on a delivered signal
   would not work: SIGALRM timers, SIGCHLD, SIGTERM for graceful shutdown.
4. **Initrd mode loads only static-pie executables** (`app-elfloader/elf_load.c:148`):
   - Ubuntu's static busybox is `ET_EXEC`, and elfloader refuses it.
   - Go's default PIE has an `INTERP`.
   - Go 1.27 needs cgo for a static-pie link.
5. **glibc block-buffers stdout on the Unikraft console**, as it does on Linux when
   stdout is piped, because `TCGETS2` returns `EINVAL`.
6. **The strace printer mis-decodes clone flags.** For glibc's `0x3d0f00` it prints
   `CLONE_NEWTIME|CLONE_FS|…`. The raw value appears in posix-process's error line.
7. **lwIP rejects `TCP_KEEPIDLE`** unless `LWIP_TCP_KEEPALIVE` is enabled.
8. **Go's large address-space reservations work.** In a 512 MiB guest, the runtime
   reserved 2 × 512 MiB plus 64 MiB of `PROT_NONE` space, served by `LIBUKVMEM`
   demand paging.
9. **The handshake hold is about 26–61 ms per connection** in run 0012, with verbose
   serial logging on. These are single samples, not a benchmark.

## What we assumed wrong

- **The research's Gap 2 closure is correct about dispatch but not sufficient.**
  Binary syscalls do reach the same `posix-socket` handlers; the disassembly and the
  strace confirm it, and the mechanism needed no change. But the closure said
  nothing about the loader's own prerequisites. A stock-configured app-elfloader on
  this Unikraft cannot start any glibc program (no `arch_prctl`), nor any threaded
  program (no `CLONE_FS` handler without a VFS).
- **The pass-4 prediction** expected both brk and mmap to fail. mmap worked; the cause
  was `arch_prctl`.

## Design implications for #303 and #259

**#303:**
- **Option 2 on Unikraft now covers unmodified Linux binaries.** The mechanism sits
  below the syscall boundary, so it does not depend on the libc or the language
  runtime. glibc C, BusyBox and Go, with its own netpoller and threads, all worked
  with no change to B2's patches or library.
- **D3 on Unikraft is unchanged:** the connect/accept post-hooks, the per-socket
  driver swap, and the lwIP readiness hook.
- **D2 custody is as amended by B2:** the SVID keys never cross vsock; the
  per-connection traffic secrets do.
- **The guest configuration becomes part of the design.** A Unikraft profile for
  Linux binaries needs:
  - `arch_prctl`
  - a VFS library, for threads
  - `LIBPOSIX_PROCESS_SIGNAL` (which needs a VFS), for workloads that rely on
    signals
  - futex and eventfd

  These are Unikraft library choices, not mechanism code. They should be pinned and
  tested per image.

**#259:**
- **Initrd mode loads only static-pie executables.** Plain static (`ET_EXEC`) and
  ordinary dynamically linked binaries need app-elfloader's VFS mode, which loads
  from a filesystem and side-loads the dynamic loader named in `PT_INTERP`. That mode
  was not tested here.
- **Most Dockerfile images are dynamically linked,** so the image factory will need
  the VFS mode: a rootfs (for example a cpio initrd mounted by posix-vfs) with the
  binary, its loader and its libraries.
- **Go binaries** need cgo plus an external `-static-pie` link, or must ship
  dynamically linked with a loader.

**Remaining work before production:**
- SMP, and one guard event loop per guest
- dynamically linked binaries through the VFS mode
- authenticating the guest on the vsock channel
- the lwIP delayed-ACK latency
- a performance comparison with the host proxy (#287)

## Commands

All metal runs go through `capture.sh`, which calls `cargo xtask metal run`. That
command rsyncs with the `.env` guard, takes the shared lease and runs the fail-closed
native preflight. From the repo root, with `I=spike-scratch/in-guest-kernel-mtls/increment-f`:

```
bash $I/capture.sh 0001 inventory --no-sudo -- bash $I/host-inventory.sh
bash $I/capture.sh 0002 build-host --no-sudo -- bash $I/build-host.sh
bash $I/capture.sh 0004 build-binary-gnu-source --no-sudo -- bash $I/build-binary.sh
bash $I/capture.sh 0011 build-guest-main-archprctl --no-sudo -- bash $I/build-guest.sh main
OVERDRIVE_METAL_KERNEL=/home/<user>/igkm-spike-a/unikraft/out-igkmf/igkmf-strace_fc-x86_64 bash $I/capture.sh 0010 … -- bash $I/run-boot-fc.sh trace
OVERDRIVE_METAL_KERNEL=/home/<user>/igkm-spike-a/unikraft/out-igkmf/igkmf-main_fc-x86_64 bash $I/capture.sh 0012 … -- bash $I/run-boot-fc.sh mesh   (0014 repeat; 0013 stretch; 0017 http-busybox)
bash $I/capture.sh 0016 … --no-sudo -- bash $I/build-busybox.sh
bash $I/capture.sh 0019 … --no-sudo -- bash $I/build-go-client.sh
bash $I/capture.sh 0024 … --no-sudo -- bash $I/build-guest.sh go
OVERDRIVE_METAL_KERNEL=…/igkmf-go_fc-x86_64 bash $I/capture.sh 0025 … -- bash $I/run-boot-fc.sh http-go
bash $I/capture.sh 0026 host-state-check-final --no-sudo -- bash $I/host-state-check.sh
```

The guest is built with Unikraft's own make:

```
make -C unikraft-igkmf A=app-elfloader L=lib-libelf:lib-lwip-igkmf:lib-mtlsguard-igkmf O=build-igkmf-<v> C=config-igkmf-<v>/.config LEX=flex YACC=bison UK_CFLAGS=-std=gnu17 MTLSGUARD_MBEDTLS_DIR=mbedtls/mbedtls-3.6.7
```

Firecracker runs as `firecracker --no-api --config-file vm.json`, with B2's VM shape
plus `"initrd_path"`.

## Versions

| Component | Version |
|---|---|
| Host kernel | `7.0.0-29-generic`; never modified |
| Firecracker | v1.17.0, sha256 `99ad0f5c…` |
| Unikraft | `eb8fa2368618cea11c9bde196f79e6e6b9caeed5` + 0001/0002 |
| lib-lwip | `ec55ae17618feeb57c8c10109bcf5c42723e8e95` + 0003; lwIP zip sha256 `1cf15ac8…` |
| app-elfloader | `a6c9dc2b655572bee1f9657cbcc2d1eca8f6cb73` (unmodified) |
| lib-libelf | `c03d4ceb22a2b9e9799238f9c277a14284d313ad` |
| Mbed TLS | 3.6.7, tarball sha256 `a7e8bcbe…` |
| Test binary toolchain | gcc 15.2.0-16ubuntu1, GNU ld 2.46, glibc 2.43-2ubuntu2.4; `igkmf-app` sha256 `043e34e746ef86eb2a97c3334920edc1022ef34e54186213eb33571f6357b179` |
| BusyBox | 1.37.0 (tarball `3311dff3…`), binary `f062e1b88be4a70c…` |
| Go | go1.27.1 (tarball `63d339f0…`), client `1d39ec35bf7e0d1e…` |
| Host stubs | rustls 0.23.45, ring 0.17.14, rcgen 0.14.10, x509-parser 0.18.1; rustc 1.95.0 |
| Guest toolchain | gcc 15.2.0 (`-std=gnu17`), make 4.4.1, flex 2.6.4, bison 3.8.2, m4 1.4.19, wget 1.25.0 |

## Host state created and cleanup

**Per boot run** (runs 0006, 0008, 0010, 0012–0014, 0017, 0021, 0023, 0025):
- one uniquely named tap `igkf<ddHHMMSS>` on 192.168.204.1/24
- one tcpdump on the tap
- the peer and relay processes, and the inclient subshell
- one Firecracker (no jailer, no API socket)
- the native run of the test binary
- a per-run directory

Each runner's before/after diff showed nothing left behind.

**Run 0026 (read-only) found nothing left:**
- no processes: firecracker, jailer, peer, relay, inclient, tcpdump or test binaries
- no `igk*` or tun/tap links
- no 192.168.203.x or 192.168.204.x address
- no listeners on 5001, 6443–6451 or 7100
- no Unix sockets and no run directories
- 0 root-owned files in the scratch tree and in increments d, e and f
- the pristine clones, app-elfloader and lib-libelf are clean
- the B, B2 and B3 worktrees contain exactly their patches

**Stray file in the host mirror:** an editor diagnostics hook compiled `main.go`
locally and produced a Mach-O file, `app-go/main`, which was synced into the host's
`~/overdrive` mirror. The probe agent deleted it, and run 0027's sync removed it from the
host.

**Kept on purpose** (user-owned, outside `~/overdrive`): `~/igkm-spike-a` is now
3.5 GB (was 1.4 GB). B3 added:
- `igkmf-host-target` (163 MB)
- `stretch` (505 MB)
- the build directories `build-igkmf-{main,strace,go,go-strace}` (about 1.35 GB)
- `out-igkmf` (36 MB)
- the new worktrees and clones

Also kept: the gitignored `increment-f/out` on the host (12 MB).

To remove only B3's additions:

```
git -C ~/igkm-spike-a/unikraft/unikraft worktree remove --force ~/igkm-spike-a/unikraft/unikraft-igkmf
git -C ~/igkm-spike-a/unikraft/lib-lwip worktree remove --force ~/igkm-spike-a/unikraft/lib-lwip-igkmf
rm -rf ~/igkm-spike-a/igkmf-host-target ~/igkm-spike-a/stretch ~/igkm-spike-a/unikraft/{build,out,config}-igkmf* ~/igkm-spike-a/unikraft/lib-mtlsguard-igkmf ~/igkm-spike-a/unikraft/igkmf-*.log ~/igkm-spike-a/unikraft/app-elfloader ~/igkm-spike-a/unikraft/lib-libelf
```

## Gate recommendation

WORKS for all eleven items. Keep the probe. Use "option 2 on Unikraft covers
unmodified static-pie Linux binaries (glibc C, BusyBox, Go) with no mechanism change"
as input to D1 and D3.

Before a walking skeleton, the next probe should cover:
- dynamically linked binaries from a real Dockerfile image, through app-elfloader's
  VFS mode
- SMP, with one guard event loop per guest
