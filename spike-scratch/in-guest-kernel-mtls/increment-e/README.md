# Spike B2, increment e: non-blocking sockets, poll/epoll readiness, NewSessionTicket and KeyUpdate for the in-guest mTLS mechanism on Unikraft/Firecracker

Throwaway probe for GH #303 (`in-guest-kernel-mtls`). Extends Spike B
(`../increment-d/`) from blocking sockets to non-blocking sockets driven by
`epoll` (level- and edge-triggered) and `poll()`, and makes the guest record
layer survive post-handshake NewSessionTicket and TLS 1.3 KeyUpdate. Runs on the
native metal host (Firecracker v1.17.0), reusing the scratch tree
`~/igkm-spike-a` that Spikes A3 and B left.

Pinned: Unikraft core `eb8fa2368618cea11c9bde196f79e6e6b9caeed5` + increment-d
patches 0001/0002 (reused read-only, sha256 `e3b35083…`, `9c798cce…`), lib-lwip
`ec55ae17618feeb57c8c10109bcf5c42723e8e95` + increment-e patch 0003, Mbed TLS
3.6.7 LTS (release tarball sha256 `a7e8bcbe…d11f6`), Firecracker v1.17.0,
rustls 0.23.45 / ring 0.17.14 / rcgen 0.14.10 / x509-parser 0.18.1
(`host/Cargo.lock`).

Spike B's trees are left untouched: B2 builds in NEW worktrees
(`unikraft-igkme` of the pristine Unikraft clone, `lib-lwip-igkme` of the
pristine lib-lwip clone), new build/out dirs (`build-igkme`, `out-igkme`), a new
host target dir (`igkme-host-target`), its own subnet (192.168.204.0/24) and a
uniquely named tap per boot run (`igke<ddHHMMSS>`).

## Design

### Readiness: a lib-lwip patch, not a proxy uk_file

lwIP pushes readiness for its sockets straight into the socket's `uk_file`
(`lib-lwip/sockets.c` `lwip_posix_socket_event_callback` →
`posix_sock_event_assign`), bypassing the per-socket driver. Patch
`patches/0003-lib-lwip-readiness-interposition.patch` (2 files, 42 insertions,
2 deletions): when the socket's installed driver is **not** lwIP's own, lwIP
passes the events it computed to a weak hook
`lwip_posix_socket_events_hook(sock, lwip_events)` and assigns what the hook
returns; for every ordinary socket nothing changes. The same routing is applied
in `poll_setup`. The hook is declared in a new header
`include/uk/lwip_readiness.h`. lib-lwip has no `exportsyms.uk`, so the weak
default stays global and the strong definition in lib-mtlsguard wins at the
final link (checked by disassembly in the build, the Spike B lesson).

The proxy-`uk_file` alternative (a second, guard-owned file wrapping the lwIP
socket) was not needed: with the hook, the guard owns the app-visible readiness
of the one existing `uk_file`, and lwIP's own readiness is still available to
the guard through the hook's argument.

### Guard thread per mesh connection

Every mesh `connect()` and mesh `accept()` gets the per-socket guard driver
immediately (state HANDSHAKING) and its own guard thread
(`uk_sched_thread_create`). The thread:
1. waits for TCP (non-blocking connect) on a private pollqueue `rawq` that
   mirrors lwIP's raw readiness (fed by the hook);
2. runs the Spike B vsock relay unchanged in shape (lock-step, one TLS record per
   NEED_PEER);
3. installs the record layer on SECRETS (v2, below), publishes EPOLLOUT;
4. stays on as the connection's RX pump: it reads ciphertext from lwIP,
   decrypts one complete record at a time, discards NewSessionTickets, applies
   KeyUpdates, and publishes EPOLLIN only when application plaintext is
   buffered.

A non-blocking `connect()` returns `EINPROGRESS` at once. A blocking `connect()`
or `accept()` (Spike B's hold) waits for the thread's HSDONE bit, so the blocking
regression runs through the same machinery. A non-blocking listener's
`accept()` returns the fd at once (stretch).

Why a thread and not a state machine driven from the socket's own
read/write/poll path: an application that sits in `epoll_wait` does not call
into the socket, and `epoll_wait` only sleeps on the pollqueue, so nothing on
the application's path would advance the handshake. lwIP's event callback is
the only other context that runs, and it executes with interrupts disabled
inside lwIP's core (a relay round trip or an lwIP read from it would block or
deadlock). A thread is therefore the smallest mechanism that makes progress on
its own. Trade-off: one thread (64 KiB stack + 64 KiB aux stack) and ~76 KiB of
buffers per mesh connection, and one extra context switch per received record.
A production design at unikernel density would use one guard thread per guest
driving all mesh connections as an event loop.

### Application readiness (a pure function of guard state)

| State | App readiness |
|---|---|
| HANDSHAKING | none: no EPOLLOUT, no EPOLLIN; `write()` = EAGAIN with 0 bytes to lwIP; `read()` = EAGAIN |
| FAILED | EPOLLERR \| EPOLLHUP; `SO_ERROR` returns the errno once; `read()`/`write()` return it |
| ESTABLISHED | EPOLLIN iff decrypted plaintext is buffered, or EOF / error; EPOLLOUT iff the TX ciphertext buffer can take a full record |

The application's `read()` only copies plaintext; only the RX pump decrypts. So
EPOLLIN never means "some ciphertext arrived", and under edge-triggered epoll
every newly decrypted record is a fresh rising edge after the previous one was
drained (the drain clears EPOLLIN). The guard republishes the same function
after every state change (`uk_file_event_assign`), and the lwIP hook returns it.

### Post-handshake handshake messages

Inner-type 0x16 records are reassembled (a message may span records, a record
may carry several). NewSessionTicket (4) is discarded. KeyUpdate (24):
`application_traffic_secret_N+1 = HKDF-Expand-Label(secret_N, "traffic upd",
"", 32)`, new key/IV, sequence 0; if `update_requested`, a KeyUpdate
(`update_not_requested`) is queued under the old transmit keys and then the
transmit keys advance. A KeyUpdate that is not at a record boundary, application
data inside a fragmented handshake message, and any other message type fail the
connection closed with an `unexpected_message` alert.

### SECRETS v2 and key custody (changed versus Spike B)

rustls 0.23.45 offers three relevant APIs (checked in the crate source):
- `dangerous_extract_secrets()` → key + IV + sequence per direction only (Spike B);
- `KeyLog` (`CLIENT_TRAFFIC_SECRET_0` / `SERVER_TRAFFIC_SECRET_0`) → the
  application traffic secrets;
- `kernel::KernelConnection` (via `dangerous_into_kernel_connection` on an
  *unbuffered* connection) → keeps the secrets inside rustls and returns new
  key/IV on `update_tx_secret()` / `update_rx_secret()`.

B2 uses `KeyLog` + `dangerous_extract_secrets()`: SECRETS v2 carries, per
direction, the 32-byte traffic secret plus rustls' key/IV; the guest derives
key/IV itself (Mbed TLS HKDF over SHA-256) and fails closed unless they match.
`KernelConnection` would keep the secrets on the host but puts the agent back in
the data path for every KeyUpdate and loses that ability across an agent
restart. **Key custody changes:** the traffic secrets now cross vsock (Spike B
sent only key/IV). They let the guest derive all future traffic keys of that
one connection, which it could already decrypt; the SVID private key still
never crosses.

## Test topology

- tap `igke<ddHHMMSS>` host 192.168.204.1, guest 192.168.204.2, vsock CID 3,
  relay on `<uds_path>_7100`.
- `peer` (rustls, every listener requires a client cert; rustls' DEFAULT ticket
  behaviour, i.e. 2 NewSessionTickets per connection, on every rustls listener):
  `:6443` echo, `:6444` echo with a denied identity, `:6445` server-first,
  `:6446` echo that calls `refresh_traffic_keys()` before every response,
  `:6447` echo + 40 000-byte bulk, `:6448` RAW crafted NSTs (split + two per
  record + coalesced with app data in ONE write) then a KeyUpdate
  (`update_not_requested`), `:6449` RAW unexpected CertificateRequest, `:6450`
  echo (relay-kill case), `:5001` plain.
- `relay` (rustls): one thread per vsock connection, KeyLog capture, SECRETS v2,
  custody scan (SVID keys must be 0, traffic secrets 1 each).
- guest `app/main.c` cases: see its header comment.

## Files

| File | Role |
|---|---|
| `capture.sh`, `rsync-without-local-env.sh` | copied from increment-d (paths renamed) |
| `host-inventory.sh`, `host-state-check.sh` | read-only substrate / end-of-probe checks |
| `patches/0003-lib-lwip-readiness-interposition.patch` | the lib-lwip readiness patch |
| `lib-mtlsguard/` | the out-of-tree library (guard thread, readiness, record layer, NST/KeyUpdate) |
| `app/` | the guest program + `defconfig` |
| `host/` | own Cargo workspace: `gencerts`, `peer`, `relay`, `inclient` |
| `build-host.sh`, `build-guest.sh`, `run-boot-fc.sh` | builds (no sudo) and the root boot runner |
| `pcap_scan.py` | tap capture scanner (records with host-clock timestamps, per-segment grouping) |
| `runs/NNNN.{meta,stdout,stderr}` | append-only captures |

## Run log (append-only)

| Run | What | Result |
|---|---|---|
| 0001 | inventory | `SUBSTRATE OK`; `uname -r` `7.0.0-29-generic`, AMD EPYC 8024P, `systemd-detect-virt` = `none`, KVM API 12, `aes` flag; Firecracker v1.17.0 `99ad0f5c…`; pristine Unikraft/lib-lwip clean at the pinned SHAs; Spike B worktree `unikraft-igkmd` = patches 0001+0002 (49 insertions); Mbed TLS tarball sha256 matches; ring 0.17.14 and rustls 0.23.45 already in the spike-private CARGO_HOME; no `igkme0`, no 192.168.204.x, no listeners |

## Pass 1 — host build (written before run 0002)

- **Hypothesis:** the host programs build with `cargo build --release --locked`
  (ring added as a direct dependency; the lockfile only gained that edge), and
  `gencerts` produces the six-leaf PKI with 192.168.204.x IP SANs.
- **Prediction:** build OK offline from the spike CARGO_HOME; `openssl verify`
  OK for all six leaves; peer certs carry IP SAN 192.168.204.1, `guest-server`
  192.168.204.2.
- **Falsification:** `--locked` refuses the lockfile, a compile error, or wrong
  SANs.

## Pass 2 — guest build (written before run 0003)

- **Hypothesis:** patches 0001+0002 apply to a fresh Unikraft worktree and 0003
  to a fresh lib-lwip worktree; the image links with strong definitions of the
  three hooks from lib-mtlsguard, the lib-lwip callback and poll_setup call the
  strong `lwip_posix_socket_events_hook`, and Mbed TLS MD/SHA-256/HKDF compile
  under the minimal config.
- **Prediction:** `3 files changed, 49 insertions(+)` (core) and
  `2 files changed, 42 insertions(+), 2 deletions(-)` (lib-lwip); make
  succeeds; `nm` shows exactly one `T` per hook and no local `t` copy;
  `objdump` shows `lwip_posix_socket_event_callback` and
  `lwip_posix_socket_poll_setup` calling the strong hook address.
- **Falsification:** a patch that does not apply, a compile/link error in
  guard.c or Mbed TLS (e.g. md.c wanting PSA), or the weak lwIP default being
  bound locally (which would make the readiness fix a silent no-op).

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0002 | host build | **pass-1 prediction held**: `--locked` OK, built in 14 s from the spike CARGO_HOME (rustls 0.23.45, ring 0.17.14, rustls-webpki 0.103.15, rcgen 0.14.10, x509-parser 0.18.1); six leaves, `openssl verify` OK ×6, peer IP SAN 192.168.204.1, guest-server 192.168.204.2; client key DER sha256 `47afc7db…`, guest-server `5931fa7c…` |
| 0003 | guest build | built; both patch stats as predicted (`3 files changed, 49 insertions(+)`, `2 files changed, 42 insertions(+), 2 deletions(-)`); one strong `T` per hook and every call site bound to it (`151863: call 171e10 <lwip_posix_socket_events_hook>` in the lwIP callback, `151743` in poll_setup, connect `12f511`, accept `129ed4`). **But its one compiler warning was load-bearing:** `passing argument 2 of 'uk_sched_thread_create_fn1' makes '__attribute__((noreturn))' qualified function pointer from unqualified`. Unikraft thread entries are `__noreturn` and `ukarch_ctx_init_entry1` (`arch/x86/ctx.c:126`) pushes a 0 return address, so a `guard_thread` that returned would have jumped to address 0 after the first connection closed. Not booted |
| 0004 | guest build, `guard_thread` ends in `uk_sched_thread_exit()` | **pass-2 prediction holds**: 0 warnings; hooks strong and bound (`151863: call 171780`, `151743: call 171780`, `12f511: call 1718d0`, `129ed4: call 171e00`); `libmtlsguard.o` text 60 122 B, bss 8 872 B (the per-connection buffers are now heap, not bss); `liblwip.o` text 132 493 B; image `a1d09df7…` (681 696 B), entry `0x18577a`; resolved `.config` saved as `config/igkme.config` (`LIBPOSIX_POLL`, `LIBUKFILE_CHAINUPDATE`, `LIBUKSCHEDCOOP` = y) |

## Pass 3 — the checklist on Firecracker (written before run 0005)

`boot_args = "igkme netdev.ip=192.168.204.2/24:192.168.204.1 mtlsguard.mesh=192.168.204.1:6443-6450 mtlsguard.mesh_in=192.168.204.2:7443 mtlsguard.agent_port=7100 -- run"`.

- **Hypothesis:** with the lib-lwip hook routing readiness through the guard
  and a guard thread per mesh connection, a non-blocking mesh connect behaves
  like a still-connecting Linux socket until the record layer is installed,
  EPOLLIN tracks decrypted plaintext only, NST/KeyUpdate are handled in the
  guest, and Spike B's blocking behaviour is unchanged.
- **Predictions (per checklist item):**
  1. N1: `connect()` = -1 EINPROGRESS in well under 1 ms; guard logs "TCP
     connected … lwIP raw EPOLLOUT" (raw OUT seen) while the app keeps getting
     empty 1-ms epoll probes; the guard's `record layer installed` line precedes
     its `app readiness none -> OUT` line, and the app's `epoll_wait -> OUT`
     timestamp is later still; the relay's `handshake complete` (host clock)
     precedes the first guest→peer application record on the tap (same host
     clock).
  2. `write()` before EPOLLOUT = -1 EAGAIN every time it is tried (initial try
     plus every fourth empty probe); the guard counts them; the tap shows no
     guest→peer `0x17` application record before SECRETS and the EARLY marker
     line 0 times anywhere.
  3. N1 (client-first) and N2 (server-first) byte-exact under epoll.
  4. N3: 40 034 bytes (response line + 40 000 bulk) received with one
     `read()` per byte; EPOLLIN (ET) wakeups ≈ number of application records
     (1 + 3), spurious 0, pattern mismatches 0, fnv1a32 equal to the peer's.
  5. N4: one POLLIN per byte (wakeups = 41 + 37 = 78), spurious 0.
  6. Scan: every TLS direction `unframed_bytes=0`; `IGKM-E-` 0 times on TLS
     streams; the positive control finds the plaintext on :5001.
  7. D/N1/N2/N4: 2 NSTs per rustls connection discarded (rustls emits both in
     ONE record: the guard logs two messages from one record seq); R1: NST1 +
     half of NST2 | rest of NST2 | NST3+NST4 | greeting, all in one tap
     segment (`COALESCED`), 4 NSTs discarded, greeting delivered; R2 fails
     closed with `read()` = -1 EPROTO, EPOLLIN|EPOLLERR, the data after the
     CertificateRequest never delivered.
  8. K: each response preceded by a peer KeyUpdate(update_requested); guest rx
     generation 1→2→3, guest sends KeyUpdate(update_not_requested) and its tx
     generation 1→2→3; all 3 exchanges byte-exact (the peer decrypting REQK2/3
     proves it applied the guest's KeyUpdate); `0x17/22` records in both
     directions on the wire; R1's KeyUpdate(update_not_requested) advances only
     the guest's rx keys.
  9. N5: epoll → `ERR HUP` without OUT; SO_ERROR = 13 then 0; `write()` fails;
     guest→peer on that connection carries only `0x16`/`0x14`, no `0x17`.
  10. A, B, D, S1, S2, E, E-after-relay-kill, F all PASS as in Spike B run 0012
     (B `connect()` = -1 EACCES; F ECONNABORTED; N6: ERR, SO_ERROR = 103).
  - Relay custody: SVID key occurrences 0/0/0 for both keys on every
    connection; traffic secrets sent = 1 each on every ALLOW connection.
- **Falsification:** any VERDICT `FAIL`; an EPOLLOUT/POLLOUT before the guard's
  install line; a `write()` before EPOLLOUT that does anything but EAGAIN; an
  EPOLLIN wakeup whose first `read()` is EAGAIN; a readiness timeout in N3/N4
  with bytes outstanding; any `IGKM-E-` byte on a TLS stream; a non-zero SVID
  custody count; a KeyUpdate exchange that fails to decrypt on either side.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0005 | preset `mesh`, Firecracker v1.17.0, image `a1d09df7…`, tap `igke29101928` | **pass-3 prediction held: all 17 guest verdicts `PASS`**, Firecracker exit 0 on its own. Self test `gcm=0 sha256=0 AES-NI=yes; HKDF-Expand-Label RFC 8448 key=OK iv=OK, traffic upd (python vector)=OK`. N1: `connect()` -1 errno 115; `poll(…, 0)` = 0; early `write()` -1 errno 11 (×2, guard counted 2); TCP up (raw lwIP EPOLLOUT) at guest t=0.772021 while 7 empty 1-ms epoll probes followed; install t=0.794070 → `app readiness none-> OUT` t=0.802293 → app `epoll_wait -> OUT` t=0.805291; host clock: relay SECRETS t+1.144762, first guest→peer app record (`0x17/57`) on the tap t+1.179047, after the client Finished flight in the same segment (lwIP Nagle). 2 NSTs per rustls connection, both in ONE record (`rx record … wire=184 inner=0x16`, NST #1 and #2 from seq 0), no EPOLLIN for them (N1 wakeups 1, spurious 0). N3: 40 033 bytes in 40 033 one-byte reads over 4 ET wakeups (one per app record: `0x17/50 16401 16401 7249`), spurious 0, fnv1a32 `0xccf62e40` = the peer's. N4: 77 POLLIN wakeups for 77 bytes, spurious 0. K: three rounds of `rx KeyUpdate(update_requested)` → rx gen 1,2,3 and `sent KeyUpdate(update_not_requested)` → tx gen 1,2,3; the rustls peer decrypted REQK2/REQK3 (sent under guest gen 1/2); `0x17/22` ×3 each way on the wire. R1: `NST #1` + `continues in the next record (20 B buffered)` → `NST #2` → `NST #3`, `#4` from one record → greeting; the 4 crafted records in ONE tap segment (`0x17/94 0x17/54 0x17/131 0x17/53 COALESCED`); `rx KeyUpdate(update_not_requested)` → rx gen 1 only (guest tx stayed gen 0 and the raw peer decrypted REQR2 with its old rx key). R2: `FAIL CLOSED: unexpected post-handshake handshake message type 13 (CertificateRequest)`, fatal alert (peer logged `020a`), app `read()` -1 errno 71 with `IN ERR`, 0 bytes delivered. N5: `ERR HUP` without OUT, SO_ERROR 13 then 0, `write()` -1 errno 13; wire guest→peer `0x16/189 0x14/1` only. Regression: B -1 EACCES, S1/S2, E after relay SIGKILL (t+2.495663) exchange 2 at t+10.532, F -1 ECONNABORTED, N6 SO_ERROR 103. Scan: 64 336 TLS bytes, `IGKM-E-` 0, `unframed_bytes=0` in every direction; plaintext positive control on :5001. Custody: SVID key DER/PEM/scalar 0/0/0 on all 13 relay connections; traffic secrets `[1,1]` on the 10 ALLOW connections, `[]` on the 3 DENY ones. Capture `runs/0005.tap.pcap` sha256 `723e2f96…` (matches the host). Host state after cleanup identical. **Not predicted:** the peer's first response after the tickets is delayed 20–155 ms on the tap (e.g. 6445#1 greeting written by the peer at t+0.627513, on the wire at t+0.766689): Linux Nagle holds it until lwIP's delayed ACK of the ticket segment; the same lwIP delayed-ACK cliff as Spike B edge case 4, now triggered by ordinary tickets |

## Pass 4 — stretch: non-blocking listener under epoll (written before run 0006)

Preset `stretch` (`-- stretch`): a `SOCK_NONBLOCK` listener on :7443 registered
with epoll; `accept4(…, SOCK_NONBLOCK)` on EPOLLIN; the runner starts the allowed
and then the denied inbound caller.

- **Hypothesis:** with the hold keyed on the listener's blocking mode, the
  accept hook returns the fd at once for a non-blocking listener and the
  handshake finishes in the guard thread; readiness of the new fd follows the
  same guard rules as a connect.
- **Prediction:** `accept4` returns in a few ms (the hook's own serial log
  lines, not the ~35 ms handshake of run 0005's blocking S1); the guard's
  `handshake done after …` line appears AFTER `accept4` returned; the allowed
  fd first reports EPOLLIN only after the record layer is installed and REQI is
  decrypted, then REQI/RESPI byte-exact; the denied fd reports `ERR HUP` and
  `read()` = -1 ECONNABORTED; the caller sees the same as in S2.
- **Falsification:** `accept4` taking the handshake duration, an EPOLLIN on the
  new fd before install, a denied fd that delivers data or reports readable
  plaintext.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0006 | preset `stretch`, tap `igke29102245` | **pass-4 prediction held on ordering, missed on magnitude.** Both verdicts `PASS`, Firecracker exit 0. Allowed caller: `accept4(SOCK_NONBLOCK)` returned fd 2 at guest t=0.198448 (`after 16363 us`), BEFORE `record layer installed` t=0.216635 and `handshake done after 35893 us` t=0.226227; the fd's first event was `IN` at t=0.422708 after `rx record … plaintext=32` (REQI) — no event before install; REQI/RESPI byte-exact, `inclient` saw server ID `…/guest-server` and EOF on close_notify. Denied caller: `accept4` returned at t=0.459315, relay DENY at t=0.490201, the fd reported `ERR HUP` and `read()` -1 errno 103; `inclient` saw EOF without close_notify, 0 bytes. **Missed:** I predicted "a few ms" for `accept4`; it was 16.3 ms both times, dominated by the hook's own three serial log lines (the `guard driver installed` line alone took 4.5 ms in c2), not by a hold; the blocking hold in run 0005 S1 was 34.6 ms |

## Pass 5 — repeat of pass 3 on the same image (written before run 0007)

- **Hypothesis:** run 0005 is not a one-off: the guard's behaviour does not
  depend on the particular interleaving of that run (ticket arrival relative to
  the app's epoll interest change, pump vs. app thread ordering).
- **Prediction:** all 17 verdicts `PASS` again with the same structural
  numbers (N3: 40 033 bytes, 4 ET wakeups, spurious 0; N4: 77 wakeups for 77
  bytes; K: rx/tx generations 1→3; custody 0), timings different.
- **Falsification:** any `FAIL`, or a structural number that differs.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0007 | preset `mesh` again, same image, tap `igke29102358` | **pass-5 prediction held**: all 17 verdicts `PASS`; N3 40 033 bytes / 40 033 reads / 4 ET wakeups / spurious 0 / fnv1a32 `0xccf62e40`; N4 77 wakeups for 77 bytes; K `KeyUpdates rx=3 tx=3`; R2 `IN ERR`, errno 71, 0 bytes; N5/N6 as before; every N case `EPOLLOUT observed` after 7 empty probes with `early write() -> EAGAIN x2, other results x0`; 64 327 TLS bytes, marker 0, `unframed_bytes=0` in all 30 directions; custody 0 everywhere, secrets `[1,1]` on ALLOW. Capture `runs/0007.tap.pcap` sha256 `c2941d52…` (matches) |
| 0008 | final host-state check | nothing left: no firecracker/jailer/peer/relay/inclient/tcpdump processes, no `igk*` links, no tun/tap links, no 192.168.203.x/204.x, no listeners on 5001/6443-6450/7100, no Unix sockets, no run dirs, 0 root-owned files in the scratch tree and in increments d/e; pristine clones clean; Spike B worktree still exactly 0001+0002. Retained (user-owned, outside `~/overdrive`): `~/igkm-spike-a` 1.4 GB (was 1.1 GB; added `igkme-host-target` 163 MB, `unikraft/build-igkme` 110 MB, `out-igkme` 5.1 MB, worktrees `unikraft-igkme` 17 MB and `lib-lwip-igkme` 0.5 MB, both registered in the pristine clones) |

## Edge cases discovered

1. **A Unikraft thread entry must not return.** `uk_thread_fn1_t` is
   `__noreturn` and `ukarch_ctx_init_entry1` (`arch/x86/ctx.c:126`) pushes a 0
   return address; a returning `guard_thread` would jump to address 0 when the
   first mesh connection closed. The only evidence was one compiler warning in
   run 0003; the fix is `uk_sched_thread_exit()`.
2. **rustls puts both tickets in one record** (the server's post-Finished
   flight is coalesced): the guard's reassembler has to process several
   handshake messages per record, not one message per record.
3. **Tickets move Spike B's latency cliff onto the first response.** The
   ticket record goes out in its own segment; Linux Nagle then holds the
   server's first response until lwIP's delayed ACK of that segment
   (20–155 ms on the tap in run 0005).
4. **lwIP coalesces the relay-produced client Finished flight with the
   application's first record** into one TCP segment (Nagle): the first app
   record follows the Finished in stream order and was written after SECRETS,
   but shares its segment.
5. **The readiness hook runs with interrupts disabled** inside lwIP's
   `event_callback` (`SYS_ARCH_PROTECT` = `uk_lcpu_save_irqf`): it may only
   record lwIP's events, compute the app readiness and wake the guard thread.
   Anything that reads from lwIP or waits must happen in the guard thread.
6. **The design assumes the single-vCPU cooperative scheduler**: the hook reads
   guard fields without a lock. SMP (and multi-threaded apps) would need
   atomics or a lock the hook may take with interrupts off.
7. **A non-blocking listener hands the app an fd before policy is decided**:
   a denied caller is an fd that later reports `ERR HUP` / ECONNABORTED, where a
   blocking `accept()` fails instead. An accept-side queue that only surfaces
   completed handshakes would remove the difference.
8. **Guest close with the peer's close_notify unread sends RST** (lwIP, like
   Linux, resets a connection closed with unread data): guest→peer flags show
   `RST`; the peer still logs a clean close_notify first.
9. **Untested code**: the guard also answers `FIONREAD` with the buffered
   plaintext and supports `MSG_PEEK` on `recv*`; no case exercised either.
