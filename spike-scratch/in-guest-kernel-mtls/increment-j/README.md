# increment-j — Spike E: the inbound accept() server path + non-blocking / epoll readiness for the Linux kernel-module mTLS mechanism (Firecracker)

Overdrive GH #303 (`in-guest-kernel-mtls`). THROWAWAY PROBE CODE. Nothing here
ships; nothing builds or gates on it; it is deleted when the implementation it
validates lands.

## What this spike proves (closing Spike C's two deferrals)

Spike C (increment-h) proved the OUTBOUND/client slice of the Linux kernel
module: hook `tcp_prot.connect`, hold the first I/O, relay the TLS 1.3 handshake
over vsock to the host agent that holds the SVID key, install guest kTLS.
Spike D (increment-i) added connect-time resolve + first-healthy LB over the
same vsock channel. Both used BLOCKING sockets and the client role only. Spike C
explicitly deferred two items a production module needs:

> "the `accept()` / inbound path (hook `tcp_prot.accept` / `inet_csk_accept`;
> the relay `ACCEPT` frame exists but was not exercised here)"
> "non-blocking / epoll sockets ... the hold must integrate with `O_NONBLOCK` /
> epoll readiness without spurious `EAGAIN` ... the kTLS path must be
> re-characterised"

Spike E closes both, on the same stock kernel (`7.0.0-29-generic`), Firecracker
v1.17.0, host kernel untouched.

### PART 1 — inbound accept() server path (lower risk, led with)

The module now hooks `tcp_prot.accept` (== `inet_csk_accept`, signature on 7.0:
`struct sock *(*accept)(struct sock *sk, struct proto_accept_arg *arg)`). When a
mesh LISTENER (a local port listed in the new `mesh_listen=` param) accepts an
inbound connection, the accepted CHILD socket is tagged ROLE_SERVER. On first
I/O the module relays the **server-role** TLS 1.3 handshake over vsock — sending
the **ACCEPT frame (0x08)** that the host relay already understood (it selects
the rustls SERVER role holding the `guest-server` SVID, client-cert required,
zero tickets) but that the Linux module never exercised — then installs kTLS on
the accepted socket exactly as the client path does. The guest server workload
reads/writes a plaintext-looking socket; the kernel carries the record layer.

The host side was already complete from Spike B/C/D: `relay.rs` handles the
ACCEPT frame → rustls `ServerConnection`; `inclient.rs` is a host rustls client
that dials INTO the guest; `gencerts` mints `guest-server` (IP SAN .2) and
`peer-client` / `peer-client-denied`. Only the GUEST module + app + runner wiring
are new.

### PART 2 — non-blocking / epoll readiness (the real unknown)

A blocking `connect()`/`accept()`/`read()` thread runs the handshake inline
(Spike C). A non-blocking workload sits in `epoll_wait` and never calls into the
socket, so nothing drives the multi-round-trip vsock handshake, and the socket
must not report ready before kTLS is installed. The module solves both with ONE
mechanism (the Linux-kTLS analogue of B2's Unikraft guard-thread + lib-lwip
readiness hook):

- **Background-handshake drive = a per-socket KERNEL THREAD (kthread).** Kicked
  at `connect()` time (non-blocking client) or `accept()` time (server). It
  waits for TCP to reach `TCP_ESTABLISHED` (client), runs the whole vsock relay
  (`do_handshake`), installs kTLS, then wakes the poll waiters. It runs in its
  own task context, so it NEVER blocks the hooked syscall and NEVER holds
  `lock_sock` across the vsock round trips (`install_ktls` takes it only
  briefly). This is the exact "who drives the handshake while the app is in
  epoll_wait" answer — `module/igkmd_mtls.c` `igkm_drive()` / `spawn_driver()`.
- **Readiness suppression = a per-socket `proto_ops` swap.** The module builds a
  private copy of `inet_stream_ops` whose `->poll` (`igkm_poll`) calls the real
  `tcp_poll` to register the poll_wait but MASKS OUT `EPOLLIN|EPOLLOUT` while the
  socket is PENDING/HANDSHAKING, adds `EPOLLERR|EPOLLHUP` on FAILED, and passes
  through once READY. A mesh socket's `struct socket ->ops` pointer is swapped to
  this copy (the pointer field is writable; only the pointee is const — no rodata
  patching). For the client this happens at connect (no spurious `EPOLLOUT` when
  TCP comes up); for the server, future children are born-masked because
  `do_accept` copies the listener's `->ops`, and the first child is masked by the
  kthread right after graft.
- **Non-blocking I/O before kTLS returns `EAGAIN`** (the hooked sendmsg/recvmsg
  `io_gate`), a blocking caller on a kthread-driven socket WAITS on the entry
  waitqueue, and a DENY sets `sk_err` (surfaced to the app via `SO_ERROR`) +
  reports the error so poll wakes with `EPOLLERR`.

## Module source delta vs Spike D (increment-i `igkmd_mtls.c`)

- New `mesh_listen=` param + `is_mesh_listen()`.
- `hooked_accept()` on `tcp_prot.accept`; records the child (ROLE_SERVER, ACCEPT
  addrs from `inet_sk(child)`), swaps the listener's ops, spawns the driver.
- `do_handshake()` gains a role: ROLE_SERVER sends the 13-byte ACCEPT frame
  (0x08); ROLE_CLIENT sends the 7-byte OPEN frame (unchanged). The TO_PEER /
  NEED_PEER / SECRETS / DENY relay loop is identical for both.
- Per-socket `igkm_stream_ops` copy + `igkm_poll()` readiness mask; `igkm_swap_ops()`.
- Per-socket kthread driver `igkm_drive()` + `spawn_driver()`; entry gains role,
  kd (kthread-driven), ready/failed state, a waitqueue, and server ACCEPT addrs.
- `io_gate()` unifies first-I/O handling: inline (blocking client, Spike C/D
  regression), EAGAIN (non-blocking), wait-on-wq (blocking caller on a
  kthread-driven socket), DENY→`sk_err`.
- Blocking CLIENT path is byte-for-byte the Spike C/D behaviour (kd=0, inline),
  so the regression phase is unchanged.

## Phases (one initramfs, dispatched by `igkm_phase=` on the kernel cmdline)

`run-fc.sh <phase>` boots Firecracker with `igkm_phase=<phase>` and drives the
host side. Services registry (agent): `svc-a` (allow → peer-allowed 6443/6444),
`svc-hs-deny` (allow at resolve → the peer-DENIED backend :6445 → the relay
denies at handshake), `svc-denied` (deny at resolve).

| phase      | checklist | what runs |
|------------|-----------|-----------|
| regression | item 6    | guest dials out (passthrough/lbfresh/deny); health toggle (Spike C/D). |
| accept     | items 1,2 | guest is a BLOCKING server on :8443; runner dials `inclient peer-client` (allowed) then `inclient peer-client-denied` (denied). |
| nbconnect  | item 3    | guest non-blocking `connect()` + epoll to svc-a (ok), svc-hs-deny (SO_ERROR), svc-denied (resolve EACCES). |
| nbaccept   | item 4    | guest is a NON-BLOCKING ET-epoll server on :8444; runner floods it 40000 B (multi-record). |

Item 5 (the background-drive + readiness mechanism) is reported from nbconnect /
nbaccept. The serial log interleaves kernel `pr_info` and app stdout on ttyS0 in
real time (both unbuffered), so event ORDERING in the serial log is the timing
proof (the app's "EPOLLOUT seen" line appears AFTER the module's "kTLS installed
/ waking poll" line).

## Pass log — hypothesis / prediction / falsification (spike.md §4/§10)

### Pass BUILD
- **H.** The accept hook (`proto_accept_arg *` signature, confirmed from the
  stock headers, run 0002) + kthread + `proto_ops` poll swap compile clean as an
  OOT `.ko` against `/lib/modules/7.0.0-29-generic/build`; host Rust (peer +6445,
  inclient +flood) builds.
- **Pred.** `igkmd_mtls.ko` built, vermagic `7.0.0-29-generic`; 4 host bins; app
  static; initramfs packed.
- **Fals.** A signature mismatch on `tcp_prot.accept` / `proto_ops.poll`, or a
  missing symbol, fails the build → re-read the header and fix.

### Pass ACCEPT (Part 1, items 1-2)
- **H.** Hooking `tcp_prot.accept` and sending the ACCEPT frame on first I/O
  gives a guest BLOCKING server transparent server-side mTLS: the accepted
  socket's handshake is relayed to the host agent (guest-server SVID, client
  cert verified), kTLS is installed, and the guest app reads the client's request
  as plaintext and writes a response — lossless, mTLS-only on the wire, SVID key
  never on vsock. A policy-denied inbound caller is rejected: the guest's
  accepted socket fails its first read with no application bytes.
- **Pred.** Serial: `SERVER-LISTENING-8443`; `[accept] inbound on mesh listen
  :8443`; `HANDSHAKE DONE role=server`; `SERVER-IDENTITY verified-client-peer=
  …/peer-client`; `server conn#1 read … PLAINTEXT-IN-GUEST: IGKM-E-REQI …`; then
  for the denied caller `server conn#2 read FAILED` (no bytes). Relay: `rx ACCEPT
  … role=TLS server`; `handshake complete policy ALLOW guest-server <-> peer-client`
  then `policy deny … peer-client-denied`. inclient: allowed logs `server SPIFFE
  ID=…/guest-server` + response; denied logs handshake FAILED. Wire: TLS 1.3
  records on :8443, `IGKM-E-REQI`/`from-guest-server` occurrences 0×,
  `unframed_bytes=0`; `/proc/net/tls_stat` TxSw/RxSw ≥ 1. Custody: SVID key 0×.
- **Fals.** Accept hook never fires (no server tag) → guest read returns raw TLS
  / hangs. Or the ACCEPT handshake stalls (server role mis-driven). Or the denied
  caller's bytes reach the app. Or cleartext on the wire. Or a kernel panic from
  the accept-hook signature.

### Pass NBCONNECT (Part 2, item 3 + SO_ERROR)
- **H.** A non-blocking `connect()` to a mesh VIP returns EINPROGRESS; the socket
  does NOT report `EPOLLOUT` and `write()` returns `EAGAIN` until the kthread
  completes the handshake and installs kTLS; then it becomes writable and the
  first app write is byte-exact and encrypted. A handshake-time deny surfaces via
  `SO_ERROR=EACCES` with no app bytes on the wire; a resolve-time deny fails
  `connect()` with EACCES directly.
- **Pred.** Serial ORDER: `nbclient connect() rc… errno=115 (EINPROGRESS)`;
  `pre-ready write() … errno=11 (EAGAIN)`; module `[connect] NON-BLOCKING … EINPROGRESS;
  readiness withheld`; module `HANDSHAKE DONE role=client`; module `[kthread] …
  waking poll`; THEN app `nbclient EPOLLOUT … FIRST seen … AFTER the module's
  kTLS install line`; `read … PLAINTEXT`. hsdeny: `EPOLLERR -> SO_ERROR=13`,
  app bytes written=0. rdeny: `connect() … errno=13` immediately. Wire: first
  0x17 appdata record AFTER the handshake records; mesh markers 0×.
- **Fals.** `EPOLLOUT` reported before the kTLS line (readiness leak) → mask
  failed. Or `write()` succeeds before kTLS (early plaintext on the wire). Or the
  kthread deadlocks under `lock_sock` / the hooked syscall blocks → boot hangs.
  Or deny does not set `SO_ERROR`. Or app plaintext on the wire.

### Pass NBACCEPT (Part 2, item 4)
- **H.** A non-blocking `accept4(SOCK_NONBLOCK)` server under edge-triggered epoll
  with 64-byte reads receives EVERY byte of a 40000-byte multi-record stream,
  readiness withheld until kTLS, one rising edge per decrypted record, no byte
  lost while plaintext is buffered.
- **Pred.** Serial: `SERVER-LISTENING-8444 nonblocking+ET`; `accept4(NONBLOCK)
  child`; module `[accept] … handshake driven by kthread` + `HANDSHAKE DONE
  role=server`; several `ET edge#N drained … small reads`; `RECEIVED total=40000
  … match=1`. Wire: multi-record TLS on :8444, `IGKM-E-FLOOD` 0× cleartext,
  `unframed_bytes=0`.
- **Fals.** total < 40000 (bytes lost under ET) → readiness lost while buffered.
  Or readiness reported before the handshake. Or a cleartext `IGKM-E-FLOOD` on
  the wire. Or deadlock/panic.

### Pass REGRESSION (item 6)
- **H.** The extended module leaves the Spike C/D blocking outbound path
  byte-identical: passthrough, lbfresh (resolve + first-healthy + re-resolve
  after health toggle), deny (resolve EACCES).
- **Pred.** Same as increment-i run 0007/0008: `lb-c1 … from-A`, `lb-c2 … from-B`,
  deny `connect … errno=13`, 0 packets to any 10.80.0.x VIP, mesh markers 0× on
  the wire, `:5001` plaintext present, custody SVID key 0×.
- **Fals.** Any Spike C/D case regresses (hang, wrong backend, cleartext, custody).

## Run plan / what actually ran (all via capture.sh from repo root; append-only)

```
0001 inventory       --no-sudo  host-inventory.sh   FAIL: metal lease held by another workspace (wellington-v2); 120s timeout (not our box)
0002 inventory       --no-sudo  host-inventory.sh   SUBSTRATE OK; kernel-header signatures captured (accept/poll/inet_sock) -> module ground truth
0003-0005,0011 build  --no-sudo build-all.sh        lease contention with wellington-v2 (recorded; our build never ran)
0006 build-all       --no-sudo  build-all.sh        module+host built; app FAILED (accept4 needs _GNU_SOURCE) -> fixed
0012 build-all       --no-sudo  build-all.sh         BUILT clean (module vermagic 7.0.0-29-generic, static app, initramfs)
0020 accept-boot     (root)     run-fc.sh accept    GREEN items 1+2: inbound server mTLS + accept deny; custody 0; wire TLS-only
0021 nbconnect-boot  (root)     run-fc.sh nbconnect  item 3 OK proven, THEN kernel PANIC on hsdeny: stale-READY sk-pointer reuse -> hooked_sendmsg self-recursion -> stack overflow
0025 build-all2      --no-sudo  build-all.sh         rebuilt with stale-entry fix (tbl_add evict + connect/accept evict + io_gate READY=stale)
0030 nbconnect-boot  (root)     run-fc.sh nbconnect  GREEN item 3: EINPROGRESS->EAGAIN->EPOLLOUT-after-kTLS byte-exact; hsdeny SO_ERROR=13; rdeny connect EACCES; no panic
0035 nbaccept-boot   (root)     run-fc.sh nbaccept   GREEN item 4: non-blocking ET server received 40000/40000 B (match=1), 4 app records on the wire, custody 0
0040 regression-boot (root)     run-fc.sh regression GREEN item 6: passthrough + lbfresh(from-A->from-B after health toggle) + deny; identical to Spike D 0007/0008
0045 host-state-check --no-sudo host-state-check.sh  VERDICT host clean = YES (host kernel untouched; no leaked tap/FC/listener/run-dir)
```
Authoritative evidence: **0020** (accept), **0030** (nbconnect), **0035** (nbaccept), **0040** (regression), **0045** (clean). Panic reproduced+diagnosed in **0021**; fixed and reverified in **0030**.

## Hard constraints (see `.claude/rules/spike.md`)
- GUEST kernel is STOCK/UNMODIFIED (Firecracker boots the host's own vmlinuz).
- HOST kernel never touched; the `.ko` is `insmod`'d only in the disposable guest.
- Build as the login user (`--no-sudo`) into the unsynced scratch tree
  `~/igkm-spike-a/igkmd/` + `~/igkm-spike-a/igkmd-host-target/`; boot as root.
- One uniquely named tap per boot; clean up every FC/tap/socket/listener/run-dir;
  prove it with a final host-state-check.
