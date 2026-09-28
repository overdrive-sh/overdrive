# Spike B, increment d: the in-guest mTLS mechanism built into Unikraft, on Firecracker

Throwaway probe for GH #303 (`in-guest-kernel-mtls`). Builds the proposed
mechanism (hook `connect`, hold, relay the handshake to a host rustls stub over
vsock, record layer in the guest with host-supplied traffic keys, policy on the
host) into a Unikraft unikernel and runs it on Firecracker v1.17.0 on the native
metal host. Reuses Spike A3 (`../increment-c/`): its user-space build tools, its
Unikraft/lib-lwip clones, the verified Firecracker binary, the `boot_args` shape
(`<name> <libparams> -- <app args>`), the capture wrapper and the `.env` rsync
guard.

Pinned: Unikraft core `eb8fa2368618cea11c9bde196f79e6e6b9caeed5`, lib-lwip
`ec55ae17618feeb57c8c10109bcf5c42723e8e95`, Mbed TLS 3.6.7 LTS (release tarball
sha256 `a7e8bcbe…d11f6`), Firecracker v1.17.0, rustls 0.23.45 / ring 0.17.14 /
rcgen 0.14.10 / x509-parser 0.18.1 (`host/Cargo.lock`).

## Design

### Hook point (why `connect()` post-hook + per-socket driver swap)

The research (`docs/research/networking/unikernel-socket-layer-mtls-port-research.md`
F-UK-1) found that nothing outside `lib/posix-socket/socket.c` gets the socket's
`struct uk_file *` at the moment of the connect decision: a second
`POSIX_SOCKET_FAMILY_REGISTER(AF_INET)` is forbidden (`driver.c:93-115`), and the
`LIBPOSIX_SOCKET_EVENTS` payload carries no handle. So the minimal core patch is
one call-out in `connect()`:

- `patches/0001-posix-socket-connect-post-hook.patch` (final: 26 added lines,
  3 files, all in `lib/posix-socket`): declares
  `uk_socket_connect_hook(sock, addr, addr_len, blocking, ret)` in
  `include/uk/socket.h` (with a `struct uk_file;` forward declaration), adds a
  `__weak` default that returns `ret`, calls it in `connect()` after the driver
  connect (and the existing blocking `EINPROGRESS` wait) when `ret` is 0 or
  `-EINPROGRESS`, before `uk_ofile_release`, and lists the hook in
  `exportsyms.uk` (without that line the build localizes the weak default and
  `connect()` binds to it; runs 0005–0007). No `lib-lwip` change. The first
  revision had 23 lines in 2 files; see the run log.

Everything else lives in an out-of-tree Unikraft library, `lib-mtlsguard`, which
provides the strong definition of the hook. For a mesh destination it:

1. **holds**: the hook runs inside the `connect` syscall with TCP already up; it
   takes the socket's write lock (so another thread's `read`/`write` on the fd
   blocks too) and does not return until the handshake is done;
2. **relays**: opens an `AF_VSOCK` socket with `uk_socket_create` (no fd) to
   (CID 2, `mtlsguard.agent_port`), and follows the host's frames;
3. **swaps the driver**: on SECRETS it allocates a per-socket
   `struct posix_socket_driver` whose `.ops` are the guard ops and whose
   `.private` is the per-socket state, and writes it into
   `posix_sock_get_node(sock)->driver` (the per-instance field the research found
   mutable). lwIP's `sock_data` is untouched, so the guard ops call lwIP's ops on
   the same file. Only this one socket changes; `close` restores lwIP's driver
   before freeing (`socket_release` frees with `node.driver->allocator`).

Blocking works without extra code because lwIP sockets are always
non-blocking underneath and `posix-fdio`/`posix-socket` already loop on
`-EAGAIN` + `uk_file_poll`: the guard `read` returns `-EAGAIN` until a whole
record has arrived.

Mesh destinations are a Unikraft **library parameter** (not a compile-time
constant): `mtlsguard.mesh=192.168.203.1:6443-6446`, plus
`mtlsguard.agent_port=7100`. A non-blocking mesh connect is refused with
`EOPNOTSUPP` (fail closed) because readiness suppression is out of scope.

### Relay protocol (vsock, host-driven, lock-step)

One frame = `type:u8 | len:u32 BE | payload`.

| type | name | dir | payload |
|---|---|---|---|
| 0x01 | OPEN | guest → host | `4`, IPv4 (4 B), port (2 B), network order |
| 0x02 | TO_PEER | host → guest | TLS bytes to write to the TCP peer |
| 0x03 | NEED_PEER | host → guest | empty: read once from TCP, reply FROM_PEER |
| 0x04 | FROM_PEER | guest → host | bytes read from TCP (0 = peer EOF) |
| 0x05 | SECRETS | host → guest | suite `0x1301`; tx seq(8) keylen(1) key(16) iv(12); rx the same; peer-id len(2) + peer SPIFFE ID (informational) — 78 B + id |
| 0x06 | DENY | host → guest | reason text |
| 0x07 | ERROR | host → guest | reason text |

The guest touches the TCP peer only when told to, so it never reads past the
server's handshake flight (the peer sends no 0.5-RTT data and no tickets). The
host checks `rx_seq == tx_seq == 0` and no buffered plaintext at extraction.
After SECRETS/DENY/ERROR the guest closes the vsock connection: the host agent is
out of the data path. On DENY the host does not relay the alert rustls queues;
the guest shuts the TCP connection down.

### Record layer (guest)

TLS 1.3, `TLS_AES_128_GCM_SHA256` only (pinned in the host rustls provider on
both ends), Mbed TLS 3.6.7 LTS `mbedtls_gcm_*` (only `aes.c aesni.c gcm.c
block_cipher.c constant_time.c platform_util.c` compiled; AES-NI through Mbed
TLS's inline-assembly path with CPUID detection). Nonce = IV XOR seq (64-bit BE
right-aligned), AAD = the 5-byte header (`17 03 03 len`), inner type
`application_data`, one record per `write` (≤ 16384 B), per-direction sequence
numbers from SECRETS. Received records must have outer type 0x17 and a valid
tag; inner `application_data` is delivered, `close_notify` maps to EOF, and any
other inner type (NewSessionTicket, KeyUpdate, other alerts) fails the socket
closed (`EPROTO`/`ECONNRESET`, sticky). Why Mbed TLS 3.6.7 and not
`unikraft/lib-mbedtls`: the Unikraft port pins Mbed TLS 2.18.1 (2019, end of
life) and compiles the whole library; 3.6 is the current LTS, and only the
AES-GCM subset is needed.

### Test topology

- **peer** (`host/src/bin/peer.rs`): rustls TLS 1.3 servers on the tap address,
  each requiring a client cert from the test CA: `:6443` echo (identity
  `peer-allowed`), `:6444` echo (identity `peer-denied`), `:6445`
  server-speaks-first, `:6446` echo that sends one NewSessionTicket (fail-closed
  control); plain `:5001` echo (pass-through). No tickets elsewhere, no 0.5-RTT
  data. Logs the verified client SPIFFE ID and the exact plaintext.
- **relay** (`host/src/bin/relay.rs`): Unix listener at `<uds_path>_7100`, rustls
  client with the client SVID (`spiffe://overdrive.test/ns/default/sa/guest-client`),
  policy allow-list `(guest-client → peer-allowed)` decided in the certificate
  verifier after chain/IP validation, secret extraction, per-connection frame
  tally and a custody scan of every byte sent to the guest for the client key's
  PKCS#8 DER, PEM and raw P-256 scalar.
- **guest** (`app/main.c`): cases A pass-through, B deny, C NST control,
  D server-first, E client-first + relay kill + second exchange on the same
  connection, F agent-down, G non-blocking.
- `pcap_scan.py`: reassembles every TCP stream on the tap, parses TLS record
  framing and counts the plaintext markers.

## Files

| File | Role |
|---|---|
| `capture.sh`, `rsync-without-local-env.sh` | copied from increment-c (paths renamed) |
| `host-inventory.sh` | read-only substrate probes + toolchain inventory |
| `patches/0001-posix-socket-connect-post-hook.patch` | core change 1: connect post-hook (26 lines, `lib/posix-socket` only) |
| `patches/0002-posix-socket-accept-post-hook.patch` | core change 2 (stretch): accept post-hook (23 lines, same files) |
| `config/igkmd.config` | resolved Unikraft `.config` from run 0011 (login home redacted) |
| `lib-mtlsguard/` | the out-of-tree Unikraft library (hook, relay, driver swap, record layer) |
| `app/` | the guest program (plain sockets) + `defconfig` |
| `host/` | own Cargo workspace (NOT a repo workspace member): `gencerts`, `peer`, `relay`, `inclient` (stretch) |
| `build-host.sh`, `build-guest.sh` | login-user builds on the metal host, no sudo |
| `run-boot-fc.sh` | root runner: tap, tcpdump, peer, relay, Firecracker, relay kill, scan, cleanup |
| `pcap_scan.py` | tap capture scanner |
| `host-state-check.sh` | read-only end-of-probe check |
| `runs/NNNN.{meta,stdout,stderr}`, `runs/0009.tap.pcap`, `runs/0012.tap.pcap` | append-only captures; the pcaps are decoded from the base64 in the run stdout and match the hashes printed on the host |

## Run log (append-only)

| Run | What | Result |
|---|---|---|
| 0001 | inventory | `SUBSTRATE OK`; host `7.0.0-29-generic`, AMD EPYC 8024P, `systemd-detect-virt` = `none`, `aes` CPU flag present; Firecracker v1.17.0 `99ad0f5c…` unchanged; A3 clones clean at the pinned SHAs; rustc/cargo 1.95.0 in `~/.cargo/bin`; tcpdump 4.99.6, ss, openssl present; crates.io reachable |

## Pass 1 — host build (written before run 0002)

- **Hypothesis:** the three host programs build on the metal host with
  `cargo build --release --locked` from the laptop-generated lockfile, and
  `gencerts` produces a CA and three leaves with one `spiffe://` URI SAN each.
- **Prediction:** build OK; `openssl x509` shows the URI SANs, IP SAN
  192.168.203.1 on the two peer certs, clientAuth/serverAuth EKUs, validity
  2026-09-27..2026-10-28; `openssl verify` OK for all three leaves.
- **Falsification:** a compile error, a lockfile mismatch (`--locked` fails), or
  a cert without the expected SAN.

## Pass 2 — guest build (written before run 0003)

- **Hypothesis:** the one-call-out core patch applies to `eb8fa236`, and the
  image links with the strong `uk_socket_connect_hook` from `lib-mtlsguard`
  overriding the weak default, with the Mbed TLS AES-GCM subset compiled under
  nolibc.
- **Prediction:** `git apply --check` clean; `git diff --stat` = 2 files, 23
  insertions; `make` succeeds; `nm` shows exactly one `T uk_socket_connect_hook`
  and the four Mbed TLS symbols; entry `_lxboot_entry`, no Xen note.
- **Falsification:** the patch does not apply, a compile/link error, the weak
  default winning (a `W` symbol, or none from `lib-mtlsguard`), or missing
  nolibc prototypes that force changes to Mbed TLS.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0002 | host build | **harness failure**: `Permission denied` opening `~/.cargo/registry/cache/…/cfg-if-1.0.5.crate`; the shared `~/.cargo/registry` has root-owned entries. Not touched; `build-host.sh` now uses a spike-private `CARGO_HOME=~/igkm-spike-a/cargo-home` with the shared rustup toolchain |
| 0003 | host build (retry) | **pass-1 prediction held**: `cargo build --release --locked` OK (rustls 0.23.45, rustls-webpki 0.103.15, ring 0.17.14, rcgen 0.14.10, x509-parser 0.18.1); certs as predicted (URI SANs, IP SAN on the peer certs, EKUs, 2026-09-27..2026-10-28), `openssl verify` OK ×3; client key P-256, PKCS#8 DER sha256 `28b12d34…` |
| 0004 | guest build | **compile failure, my bug**: the header comment in `guard.c` contained `recv*/send*`, whose `*/` closed the comment (the `errno` errors were cascade). Also a real patch defect: `struct uk_file declared inside parameter list` when `lib-lwip/sockets.c` includes `socket_driver.h` first (circular include order). Fix: comment text, and a `struct uk_file;` forward declaration in the patch (now 25 lines) |
| 0005 | guest build | built, 0 warnings, **but the pass-2 falsifier fired**: `nm` shows `t uk_socket_connect_hook` (local) at `0x128d10` besides `T` at `0x166cc0`. Unikraft's per-library `objcopy --keep-global-symbols=exportsyms.uk` (`support/build/Makefile.rules:1036-1050`) localizes every symbol not in the library's `exportsyms.uk`, so posix-socket's weak default became a local symbol |
| 0006–0007 | disassembly of the 0005 image | confirms it: `__uk_syscall_r_connect` has `128e51: call 128d10 <uk_socket_connect_hook>`, the local weak default. The hook would never have run. Fix: `uk_socket_connect_hook` added to `lib/posix-socket/exportsyms.uk` (patch now 3 files, 26 insertions) |
| 0008 | guest build | **pass-2 prediction holds after the two fixes**: patch applies; 0 compiler warnings; exactly one `T uk_socket_connect_hook` at `0x166cc0`; `128e51: call 166cc0 <uk_socket_connect_hook>`; the four Mbed TLS symbols present; `libmtlsguard.o` text 33 398 B, bss 74 440 B; image `a43bee60…` (628 448 B), entry `0x178cfa`, no Xen note |

## Pass 3 — the mechanism on Firecracker (written before run 0009)

`boot_args = "igkmd netdev.ip=192.168.203.2/24:192.168.203.1 mtlsguard.mesh=192.168.203.1:6443-6446 mtlsguard.agent_port=7100 -- run"`,
tap `igkmd0`, CID 3, relay on `<uds_path>_7100`, tcpdump on the tap.

- **Hypothesis:** a plain-socket Unikraft program's mesh connections are held in
  `connect()`, handshaken by the host rustls relay over vsock with the
  host-held client key, and then carried by the guest's own TLS 1.3 record layer;
  policy is enforced at handshake time; non-mesh traffic is untouched; the
  record layer survives the relay's death.
- **Prediction:**
  - Boot as in A3; `gcm self test result=0`, AES-NI in use `yes` (the host has
    `aes`; Firecracker passes it through); guest wall clock `946598400.x`.
  - **A** `not a mesh destination -> pass-through`; plain REQ/RESP; the peer logs
    the plaintext; the scan finds `IGKM-D-PLAIN-REQ`/`-RESP` on the `:5001`
    stream (positive control for the scanner).
  - **B** relay `c1`: OPEN `:6444`, TO_PEER (ClientHello, `0x16`), NEED_PEER,
    FROM_PEER (`0x16` ServerHello, `0x14` CCS, `0x17` × several), then
    `handshake NOT completed: policy deny: … -> peer …/peer-denied`, DENY. Guest
    `connect()` = -1 `EACCES`. Peer `handshake FAILED`, 0 plaintext bytes. On the
    wire guest→peer: one `0x16` record, **no `0x17`** (rustls sends its CCS only
    before its second flight, which never happens).
  - **C** relay `c2` ALLOW, SECRETS `tx_seq=0 rx_seq=0`; guest writes REQN
    (tx seq 0); first rx record has inner `0x16` type 4 → `NewSessionTicket ->
    fail closed`; `read()` = -1 `EPROTO`.
  - **D** relay `c3`; guest reads the greeting first (rx seq 0, inner `0x17`,
    41 B plaintext), then REQS/RESPS.
  - **E** relay `c4`; `connect()` returns after a hold of a few ms; REQ1 written
    immediately (tx seq 0, plaintext 38, wire 60); RESP1 (rx seq 0). The runner
    SIGKILLs the relay after the peer's `tx plaintext line 1` and the relay's
    `c4` custody line; ~8 s later REQ2 (tx seq 1) and RESP2 (rx seq 1) succeed
    on the same connection, and the peer logs `rx plaintext line 2` after the
    `RUNNER: relay killed` timestamp.
  - **F** TCP connects, vsock connect refused (no listener behind the stale
    socket file) → `connect()` = -1 `ECONNABORTED`; no relay log line (it is
    dead); the peer sees a TCP connection with no TLS bytes.
  - **G** `connect()` = -1 `EOPNOTSUPP` with no relay contact.
  - Firecracker exits 0 on its own; all eight guest verdicts `PASS`.
  - Relay custody per connection: occurrences of the key's PKCS#8 DER, PEM and
    P-256 scalar in host→guest bytes = 0; host→guest frame types only
    TO_PEER/NEED_PEER/SECRETS/DENY; SECRETS payload = 78 + id length.
  - Peer: `verified client SPIFFE ID=spiffe://overdrive.test/ns/default/sa/guest-client`
    on :6443/:6445/:6446.
  - Scan: every TLS stream direction `unframed_bytes=0`, marker count 0 on all
    TLS streams; guest→peer `0x17` records after the handshake match the guest's
    `tx record … wire=` lengths.
- **Falsification:** any verdict `FAIL`; a mesh connect returning before the
  relay's SECRETS/DENY; any `IGKM-D-` byte on a TLS-port stream; a `0x17`
  record guest→peer on the deny connection; exchange 2 failing after the relay
  kill; a non-zero custody count; the peer seeing a client identity other than
  the relay's SVID.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0009 | preset `mesh` on Firecracker v1.17.0 (image `a43bee60…`) | **pass-3 prediction held, all eight guest verdicts `PASS`**, Firecracker exit 0 on its own. Self test `result=0`, `GCM note: using AESNI.`; wall clock `946598400.056401`. A plaintext on `:5001` (scanner positive control finds both lines). B relay `c1` DENY (`peer-denied`), `connect()` = -1 errno 13 after a 17.2 ms hold; wire guest→peer `0x16/189` only. C NST record (rx seq 0, wire 103, inner `0x16` type 4) → `read()` = -1 errno 71. D greeting read first (rx seq 0, wire 63). E hold 18.6 ms; REQ1 = tx seq 0 wire 59; relay SIGKILLed at t+0.756 s (after the peer's `tx plaintext line 1` at t+0.698 s and relay `c4`'s custody line); REQ2/RESP2 on the same connection at t+8.705 s. F vsock connect `-111` → `connect()` = -1 errno 103. G errno 95. Custody 0/0/0 on all four relay connections. Scan: every TLS direction `unframed_bytes=0`, marker 0 on 6 685 TLS bytes. Tap capture retained as `runs/0009.tap.pcap` (sha256 `a3781d54…`, matches the runner's). Not predicted: `libvirtio_vsock … Got packet to port N without socket` after each guest vsock close (the host's close arriving after the guest freed the socket; harmless); the runner's final `grep VERDICT` printed nothing because the serial log contains NUL bytes (`grep` treats it as binary); the verdicts are in the serial dump. Host state after cleanup identical to before. |

## Pass 4 — stretch: the same mechanism on `accept()` (written before runs 0010–0012)

Checklist items 1–8 passed in run 0009, so item 9 is in scope.

**Design additions.**
- `patches/0002-posix-socket-accept-post-hook.patch` (on top of 0001; 3 files,
  23 insertions, all in `lib/posix-socket`): `uk_socket_accept_hook(listener,
  sock, blocking, flags)` with a `__weak` default returning 0, called in
  `uk_sys_accept` after `uk_socket_accept` returns the new socket and **before**
  it gets an fd; a non-zero return releases the socket (TCP closed) and makes
  `accept()` fail. Exported in `exportsyms.uk` (the run-0005 lesson).
- `lib-mtlsguard`: `mtlsguard.mesh_in=192.168.203.2:7443`; for an accepted
  socket whose local address matches, the hook holds it and relays with a new
  frame `ACCEPT (0x08)` = `4 · local ip · local port · remote ip · remote port`;
  the host then runs the rustls **server** with a `guest-server` SVID (held only
  by the relay; IP SAN 192.168.203.2), requires a client certificate and decides
  policy on the client's SPIFFE ID (allow `guest-server ← peer-client`). DENY
  maps to `accept()` = -1 `ECONNABORTED`.
- **Relay granularity change, both roles:** NEED_PEER is now answered with
  exactly one TLS record (header + body). With "whatever TCP has", the server
  role would forward the client's first application record together with its
  Finished and the host would consume it. The outbound cases are re-run under the
  changed protocol.
- Host: `inclient` (rustls client presenting `peer-client` or
  `peer-client-denied`, verifying the guest's server cert against the test CA
  and IP SAN 192.168.203.2); the runner starts it when the guest prints
  `READY-FOR-INBOUND`. Guest case **S** (between D and E): plain
  `bind/listen/accept` on `0.0.0.0:7443`; S1 reads `IGKM-D-REQI…`, answers
  `IGKM-D-RESPI…`; S2 expects `accept()` = -1 `ECONNABORTED`.

**Predictions.**
- Build: 0002 applies after 0001 (combined 49 insertions in 3 files); one
  strong `T uk_socket_accept_hook`; `uk_sys_accept` calls it; 0 warnings. Host
  build OK; the PKI is regenerated with six leaves.
- Outbound cases A–G: same outcomes as run 0009. The relay now logs one
  FROM_PEER per record (server flight `0x16/122`, `0x14/1`, `0x17/~690` → three
  NEED_PEER round trips instead of one), so the connect hold grows by two vsock
  round trips (from ~18.6 ms to roughly 25 ms).
- S1: relay `ACCEPT … role=TLS server`; NEED_PEER → ClientHello; TO_PEER server
  flight; NEED_PEER → client CCS; NEED_PEER → client `0x17` flight; `policy
  ALLOW local …/guest-server <-> peer …/peer-client`, `tx_seq=0 rx_seq=0
  buffered_plaintext=false` (the REQI record stayed in the guest's TCP buffer);
  `accept()` returns an fd only after SECRETS; REQI read as rx seq 0, RESPI
  written as tx seq 0; `inclient` logs server SPIFFE ID `…/guest-server`, reads
  RESPI, then EOF on close_notify.
- S2: relay DENY (`peer-client-denied`); `accept()` = -1 errno 103. `inclient`
  finishes its side of the handshake (a TLS 1.3 client is done once it has sent
  Finished), may write REQI, then sees EOF/reset. On the wire, guest→peer on that
  connection carries only the server flight; the guest application never sees a
  byte.
- Custody 0 for both held keys on every relay connection; every TLS direction
  on the tap, including `:7443`, `unframed_bytes=0` and marker 0.
- **Falsification:** relay `invariant broken` on S1 (the guest over-read), an
  `accept()` that returns before SECRETS, S2 returning a socket, any `IGKM-D-`
  byte on a TLS stream, a non-zero custody count, or an outbound regression.

### Run log, continued

| Run | What | Result |
|---|---|---|
| 0010 | host build (inclient, six-leaf PKI) | OK; PKI regenerated (guest-server IP SAN 192.168.203.2; peer-client / peer-client-denied clientAuth); `openssl verify` OK ×6; client key DER sha256 `1ec38021…`, guest-server `8b387b25…` |
| 0011 | guest build, patches 0001 + 0002 | **pass-4 build prediction held**: both patches apply, `3 files changed, 49 insertions(+)`; 0 warnings; one `T uk_socket_connect_hook` (`0x1680f0`, called at `1290b1`) and one `T uk_socket_accept_hook` (`0x168660`, called from `uk_sys_accept` at `123a74`); `libmtlsguard.o` text 35 893 B; image `c07f7684…`; resolved `.config` saved as `config/igkmd.config` (printed sha `59fe2bbc…`; the saved copy has the login home redacted) |
| 0012 | preset `mesh` + inbound, Firecracker v1.17.0 | **pass-4 prediction held, all ten verdicts `PASS`**, Firecracker exit 0. S1: relay `c4` `ACCEPT … role=TLS server`, three one-record FROM_PEERs (`0x16/189`, `0x14/1`, `0x17/603`), `policy ALLOW local …/guest-server <-> peer …/peer-client`, `tx_seq=0 rx_seq=0 buffered_plaintext=false`; `accept()` returned fd 1 after a 24.7 ms hold; REQI rx seq 0, RESPI tx seq 0; `inclient` saw server SPIFFE ID `…/guest-server`, read RESPI, EOF on close_notify. S2: relay `c5` DENY (`peer-client-denied`), `accept()` = -1 errno 103; `inclient` finished its side and wrote REQI (`0x17/49` caller→guest on the wire), then `peer closed connection without sending TLS close_notify`; guest→caller only `0x16/122 0x14/1 0x17/689`. Outbound A–G as in 0009; connect hold 27.8 ms (was 18.6 ms; predicted ~25 ms). Relay killed at t+1.160 s, REQ2 at t+9.097 s. Custody 0 for both keys on all six relay connections; every TLS direction `unframed_bytes=0`, marker 0 on 10 152 TLS bytes. Capture `runs/0012.tap.pcap` (sha256 `80bb7a00…`, matches). **Not predicted:** (1) the outbound deny connection now shows `0x16/189 0x14/1` guest→peer (still no `0x17`): with one record per FROM_PEER, rustls emits its middlebox-compat CCS after processing ServerHello, before the certificate is rejected (relay `c1`: `TO_PEER payload=6 records[0x14/1]` right after `FROM_PEER records[0x16/122]`). (2) S1's REQI reached the guest 193 ms after `accept()` released it; the tap shows the caller's 614-byte final flight (rel seq 194) sent at 10.720 ms and **retransmitted** at 219.585 ms, REQI (rel seq 808) only after the guest's first ACK at 219.868 ms. The guest had consumed that flight at t+0.758–0.762 s (relay FROM_PEER), so the ACK was lwIP's delayed ACK outlasting Linux's 200 ms minimum RTO, with Nagle holding REQI; neither the relay nor the record layer is on that path |
| 0013 | final host-state check | no firecracker/jailer/peer/relay/tcpdump/listener processes, no tap (`igkmd0`…`igkma0`), no tun/tap links, no `192.168.203.x`, no listeners on 5001/6443-6446, no Unix sockets, no run dirs, 0 root-owned files in the scratch tree or the increment. Retained (user-owned, outside `~/overdrive`): `~/igkm-spike-a` 1.1 GB (was 728 MB; added `cargo-home` 48 MB, `igkmd-host-target` 163 MB, `unikraft/{build-igkmd,mbedtls,unikraft-igkmd,out-igkmd,lib-mtlsguard,app-igkmd}`), incl. a registered git worktree `unikraft-igkmd` of the pristine clone. Remove with `rm -rf ~/igkm-spike-a` |
