# Spike B2 findings: non-blocking sockets, poll/epoll readiness, NewSessionTicket and KeyUpdate for the in-guest mTLS mechanism on Unikraft/Firecracker

GH #303, feature `in-guest-kernel-mtls`. PROBE phase, throwaway. Builds on Spike B
(`findings-unikraft-hook.md`, `spike-scratch/in-guest-kernel-mtls/increment-d/`).
Probe sources, the new lib-lwip patch, the resolved `.config` and raw captures are in
`spike-scratch/in-guest-kernel-mtls/increment-e/`:
- runs `runs/0001` to `runs/0008`, each `{meta,stdout,stderr}`, append-only, host
  identifiers redacted
- tap captures `runs/0005.tap.pcap`, `runs/0006.tap.pcap`, `runs/0007.tap.pcap`

The increment `README.md` has the design, every pre-run hypothesis, prediction and
falsification, and the full run log. Run 0005 is the primary evidence for items 1–10,
run 0006 for item 11, and run 0007 repeats 0005 on the same image.

The probe agent could not write under `docs/`, so the orchestrator wrote this file from its
report and re-checked the verdict counts, wire scans, KeyUpdate counts and custody lines against
runs 0005–0007.

## Verdict: WORKS (all eleven items)

A plain-socket native Unikraft program that uses non-blocking sockets with `epoll`
(level- and edge-triggered) or `poll()` gets transparent mutual TLS 1.3 on its mesh
connections, with Linux-like readiness. Its guest record layer now survives ordinary
TLS 1.3 peers that send NewSessionTickets and KeyUpdates.

The Unikraft change is Spike B's unchanged 49-line `lib/posix-socket` patch plus a new
42-line `lib-lwip` patch, and the out-of-tree library.

| # | Checklist item | Verdict | Runs |
|---|---|---|---|
| 1 | non-blocking `connect()` = EINPROGRESS; EPOLLOUT only after the host reports the handshake complete and the record layer is installed | **WORKS** | 0005, 0007 |
| 2 | `write()` before EPOLLOUT = EAGAIN; no application data record on the tap before the handshake completes | **WORKS** | 0005, 0007 |
| 3 | client-first and server-first byte-exact under epoll | **WORKS** | 0005, 0007 |
| 4 | edge-triggered epoll, 1-byte `read()`s: every byte of multi-byte and multi-record data arrives, no lost readiness | **WORKS** | 0005, 0007 |
| 5 | `poll()` shows the same readiness | **WORKS** | 0005, 0007 |
| 6 | tap: only TLS records; REQUEST/RESPONSE markers 0 times | **WORKS** | 0005, 0007 |
| 7 | NewSessionTicket discarded, including several tickets and a ticket coalesced with app data; other post-handshake messages still fail closed | **WORKS** | 0005, 0007 |
| 8 | peer-initiated KeyUpdate with `update_requested`; exchanges continue both ways; key change visible on the wire without plaintext | **WORKS** | 0005, 0007 |
| 9 | deny under non-blocking connect → socket error (EPOLLERR, SO_ERROR), no application bytes | **WORKS** | 0005, 0007 |
| 10 | Spike B blocking regression: pass-through, deny, client-first, server-first, relay kill, agent-down, accept | **WORKS** | 0005, 0007 |
| 11 | stretch: non-blocking listener under epoll with the accept hook | **WORKS** (the hold does not block a non-blocking `accept()`) | 0006 |

All 17 guest verdicts `PASS` in run 0005 and again in run 0007; both stretch verdicts
`PASS` in run 0006. Firecracker exited 0 on its own every time.

Not shown (out of scope): multi-threaded applications, SMP, `app-elfloader`, session
resumption, the lwIP delayed-ACK latency cliff, performance. `FIONREAD` and `MSG_PEEK`
are implemented in the guard, but no case exercised them.

**Substrate** (run 0001 fail-closed probes; the harness preflight re-checked them on every
run):
- native metal host, x86_64, AMD EPYC 8024P
- `uname -r` = `7.0.0-29-generic`
- `systemd-detect-virt` = `none`, `/dev/kvm` API 12, `aes` CPU flag
- Firecracker v1.17.0 (`99ad0f5c…`)

**Guest:** Unikraft `eb8fa236` (`Ijiraq 0.21.0~eb8fa236-custom`), 1 vCPU, 512 MiB,
cooperative scheduler, no drives.

## Design choices

### Handshake driver: one guard thread per mesh connection

At `connect()` and `accept()` of a mesh socket, the library now installs its per-socket
driver immediately, in state HANDSHAKING, and starts a guard thread
(`uk_sched_thread_create`). The thread waits for TCP, runs Spike B's lock-step vsock
relay, installs the record layer, and then stays on as the connection's RX pump.
- A non-blocking `connect()` returns `EINPROGRESS` at once.
- A blocking `connect()` or `accept()` waits for the thread's completion bit. Spike B's
  hold now runs through the same code, and it no longer holds the socket's write lock
  across the handshake.
- A non-blocking listener's `accept()` returns the fd at once (item 11).

Why a thread and not a state machine driven from the socket's own read/write/poll path:
- An application that sits in `epoll_wait` does not call into the socket, and
  `epoll_wait` only sleeps on the pollqueue. Nothing on the application's path would
  move the handshake forward.
- lwIP's event callback is the only other context that runs. It executes with
  interrupts disabled inside lwIP (edge case 5), where a vsock round trip or an lwIP
  read would block or deadlock.

Trade-off, computed from the config and buffer constants, not measured:
- one thread per mesh connection: 64 KiB stack plus 64 KiB aux stack in this config
- about 73 KiB of guard buffers: the RX record, plaintext, handshake reassembly and a
  two-record TX queue
- a 64 KiB frame buffer during the handshake
- one switch to the pump for every received record

At unikernel density, a production design would run one guard thread per guest as an
event loop.

### Readiness fix: a lib-lwip patch, not a proxy `uk_file`

lwIP assigns its readiness directly to the socket's `uk_file`, past any installed driver.
Patch `patches/0003-lib-lwip-readiness-interposition.patch` changes that (2 files, +42/−2):
- When the socket's current driver is not lwIP's own, lwIP passes the events it computed
  to a weak hook, `lwip_posix_socket_events_hook(sock, lwip_events)`, and assigns what
  the hook returns.
- This applies in both `lwip_posix_socket_event_callback` and `poll_setup`.
- Ordinary sockets are unchanged.
- The hook prototype is in a new header, `include/uk/lwip_readiness.h`.

lib-lwip has no `exportsyms.uk`, so the weak default stays global and the strong
definition in lib-mtlsguard wins. Disassembly confirmed this in the build, which is
Spike B's lesson:

```
lwip_posix_socket_event_callback -> lwip_posix_socket_events_hook (strong at 0x0000000000171780):
  151863:	call   171780 <lwip_posix_socket_events_hook>
lwip_posix_socket_poll_setup -> lwip_posix_socket_events_hook (strong at 0x0000000000171780):
  151743:	call   171780 <lwip_posix_socket_events_hook>
```

The hook copies lwIP's raw readiness into a private pollqueue that the guard thread
waits on, and returns the application's readiness. That readiness is a pure function of
guard state:
- **HANDSHAKING:** nothing.
- **FAILED:** EPOLLERR|EPOLLHUP.
- **ESTABLISHED:**
  - EPOLLIN when decrypted plaintext is buffered, or on EOF or error.
  - EPOLLOUT when the TX queue can take a full record.

The guard republishes the same function after every state change. Only the RX pump
decrypts, and `read()` only copies plaintext out. Draining the plaintext clears EPOLLIN,
so each newly decrypted record is a fresh rising edge for edge-triggered epoll.

The proxy-`uk_file` alternative was not needed: with the hook, the guard owns the
readiness of the one existing `uk_file`.

### KeyUpdate secret source: rustls `KeyLog`

What rustls 0.23.45 offers, checked in the crate source:
- `dangerous_extract_secrets()` returns key, IV and sequence number only.
- `KeyLog` exposes `CLIENT_TRAFFIC_SECRET_0` and `SERVER_TRAFFIC_SECRET_0`.
- `kernel::KernelConnection` (via `dangerous_into_kernel_connection` on an unbuffered
  connection) keeps the secrets inside rustls. It returns new keys from
  `update_tx_secret()` and `update_rx_secret()`.

The relay now captures the traffic secrets with a per-connection `KeyLog` and sends
SECRETS v2: for each direction, the 32-byte traffic secret plus rustls' key and IV. The
guest then:
- derives key and IV itself with HKDF-Expand-Label over SHA-256, using Mbed TLS
  `hkdf.c`, `md.c` and `sha256.c`
- fails closed unless its derived key and IV match rustls'
- runs the KeyUpdate schedule itself

`KernelConnection` would keep the secrets on the host. But it puts the agent back in the
data path for every KeyUpdate, and an agent restart would lose the ability to rekey.
KeyLog keeps Spike B's property that the agent leaves the data path after the handshake.

## Evidence (predicted vs actual, real output)

Topology:
- tap `igke<ddHHMMSS>`, created fresh for each boot run and deleted afterwards
- host 192.168.204.1, guest 192.168.204.2
- vsock CID 3, relay listening on `<uds_path>_7100`

Host `peer` (rustls) listeners:

| Port | Behaviour |
|---|---|
| `:6443` | echo |
| `:6444` | echo, denied identity |
| `:6445` | server speaks first |
| `:6446` | echo that calls `refresh_traffic_keys()` before every response |
| `:6447` | echo followed by 40 000 bytes of bulk data |
| `:6448` | raw: crafted tickets, then KeyUpdate(update_not_requested) |
| `:6449` | raw: crafted unexpected CertificateRequest |
| `:6450` | echo, used for the relay-kill case |
| `:5001` | plain TCP |

- Every rustls listener keeps rustls' **default** ticket behaviour:
  `PEER listening tls 192.168.204.1:6443 … tickets=2 (rustls default)`.
- On the two raw ports, rustls runs the handshake. A hand-rolled record layer (ring
  AES-128-GCM with the extracted keys) then sends crafted records.

### 1. EINPROGRESS, and EPOLLOUT only after the record layer is installed: WORKS

Predicted:
- `connect()` returns -1 EINPROGRESS at once.
- The guard sees lwIP's raw EPOLLOUT when TCP comes up; the app does not.
- The install line comes before the app's EPOLLOUT.
- On the host clock, the relay's SECRETS comes before the first guest app record on the
  tap.

Actual (case N1, run 0005, guest clock):

```
IGKM-E: N1 epoll-LT client-first connect() returned -1 errno=115 (Operation now in progress) after 6970 us t=0.764248
IGKM-E: N1 epoll-LT client-first poll(POLLIN|POLLOUT, 0 ms) right after connect() = 0 revents=0x0 t=0.766252
MTLSGUARD: [c5] TCP connected after 14591 us (lwIP raw EPOLLOUT at t=0.772021); the app still sees no EPOLLOUT -> relay the handshake
MTLSGUARD: [c5] vsock rx SECRETS payload=194 wire=199 t=0.792973
MTLSGUARD: [c5] record layer installed: TLS_AES_128_GCM_SHA256 tx_seq=0 rx_seq=0; key/iv derived in the guest by HKDF-Expand-Label(SHA-256) from the traffic secrets and equal to rustls' key/iv (tx and rx); peer identity (from host) spiffe://overdrive.test/ns/default/sa/peer-allowed; app write() EAGAINs during the handshake=2, read() EAGAINs=0 t=0.794070
MTLSGUARD: [c5] app readiness none-> OUT (published by the guard) t=0.802293
IGKM-E: N1 epoll-LT client-first epoll_wait -> OUT (LT) after 7 empty 1-ms probes t=0.805291
IGKM-E: N1 epoll-LT client-first EPOLLOUT observed 42588 us after connect() returned; during the wait: 7 empty epoll_wait probes, early write() -> EAGAIN x2, other results x0, EPOLLIN-before-EPOLLOUT x0 t=0.806837
```

- TCP was up at guest t=0.772. EPOLLOUT was withheld for 22 ms, until install at
  t=0.794.
- The guard published EPOLLOUT at t=0.802, and the app observed it at t=0.805.
- Seven empty 1-ms `epoll_wait` probes returned in between.

Host clock (relay log and tap, both relative to the runner's T0):

```
RELAY[c5] handshake complete: policy ALLOW local spiffe://overdrive.test/ns/default/sa/guest-client <-> peer spiffe://overdrive.test/ns/default/sa/peer-allowed; suite=TLS13_AES_128_GCM_SHA256 tx_seq=0 rx_seq=0 buffered_plaintext=false; traffic secrets from rustls KeyLog tx=CLIENT_TRAFFIC_SECRET_0 rx=SERVER_TRAFFIC_SECRET_0 (32 B each), host check HKDF(secret)==rustls key/iv tx=true rx=true t+1.144750s
RELAY[c5] vsock tx SECRETS payload=194 wire=199 (v2 suite=0x1301 tx: seq 0 secret 32B key 16B iv 12B; rx: seq 0 secret 32B key 16B iv 12B; peer id 50B) t+1.144762s
SCAN conn client guest:54377 -> server peer:6443 (TLS) first packet t+1.111746s
SCAN       seg#3 t+1.179047s: 0x17/606 0x17/57 COALESCED
```

- The first guest→peer application record is `0x17/57`; the guest logged
  `tx record … plaintext=40 wire=62` for it. It went on the wire 34 ms after SECRETS.
- It shares a segment with the client Finished flight `0x17/606`, which the relay
  produced (`TO_PEER … records[0x17/606] t+1.144734s`). In stream order it follows the
  Finished; lwIP's Nagle coalesced the two (edge case 4).
- The same ordering held for all eight non-blocking connects, and again in run 0007.

### 2. `write()` before EPOLLOUT = EAGAIN, no bytes on the wire: WORKS

```
MTLSGUARD: [c5] write() of 40 B before the record layer is installed -> EAGAIN, 0 bytes to lwIP t=0.768061
IGKM-E: N1 epoll-LT client-first write(40 bytes) before EPOLLOUT returned -1 errno=11 (Resource temporarily unavailable) t=0.769836
```

- Every non-blocking case tries its first write right after `connect()`, and again on
  every fourth empty probe.
- All tries returned EAGAIN: `early write() -> EAGAIN x2, other results x0` in all six
  epoll cases of run 0005, and the same in run 0007.
- The early payload `IGKM-E-EARLY must-not-leave guest->peer\n` never left the guest:
  it occurs 0 times in the TLS streams, and no peer log line mentions it.

### 3. Client-first and server-first under epoll: WORKS

N1 is client-first (`:6443`) and N2 is server-first (`:6445`), both with level-triggered
epoll.

```
IGKM-E: N1 epoll-LT client-first read 41 bytes: IGKM-E-RESP1 nb-client-first peer->guest
IGKM-E: N2 epoll-LT server-first read 41 bytes: IGKM-E-GREETING server-first peer->guest
IGKM-E: N2 epoll-LT server-first read 38 bytes: IGKM-E-RESPS server-first peer->guest
PEER[6443#1] rx plaintext line 1 (40 bytes): IGKM-E-REQ1 nb-client-first guest->peer\n t+1.179460s
PEER[6445#2] rx plaintext line 1 (37 bytes): IGKM-E-REQS server-first guest->peer\n t+1.531760s
```

### 4. Edge-triggered epoll with 1-byte reads, multi-record data: WORKS

In N3 (`:6447`), the peer answers with one line and then 40 000 bytes in the pattern
`(i*7+3)%251`, which rustls splits into records. The app uses epoll with
`EPOLLIN|EPOLLOUT|EPOLLET` and, after each wakeup, reads one byte per `read()` until
EAGAIN.

```
MTLSGUARD: [c7] rx record gen=0 seq=1 wire=55 inner=0x17 plaintext=33 t=1.291562
MTLSGUARD: [c7] app readiness OUT -> IN OUT (published by the guard) t=1.292924
MTLSGUARD: [c7] app readiness IN OUT -> OUT (published by the guard) t=1.294368
MTLSGUARD: [c7] rx record gen=0 seq=2 wire=16406 inner=0x17 plaintext=16384 t=1.295785
…
MTLSGUARD: [c7] rx record gen=0 seq=4 wire=7254 inner=0x17 plaintext=7232 t=1.306134
IGKM-E: N3 received 40033 bytes in 40033 one-byte read()s over 4 EPOLLIN (ET) wakeups, spurious=0, in 36 ms; bulk 40000 bytes fnv1a32=0xccf62e40, bytes differing from the pattern=0
PEER[6447#1] tx bulk 40000 bytes (pattern (i*7+3)%251, fnv1a32=0xccf62e40; rustls splits it into 16384-byte records) t+1.640899s
```

- There is one rising edge per application record: the 4 records on the tap
  (`0x17/50 16401 16401 7249`) produced 4 wakeups.
- No wakeup found nothing to read: 0 spurious wakeups.
- The checksum matches the peer's.
- Run 0007 gave the same numbers.

### 5. `poll()`: WORKS

N4 (`:6445`, server-first) used non-blocking `poll()` for the connect phase, then read
**one byte per `poll()` wakeup**. This only works if `poll()` keeps reporting POLLIN while
plaintext stays buffered in the guard, even though lwIP's own EPOLLIN is already clear.

```
IGKM-E: N4 poll -> revents=0x4 after 7 empty 1-ms polls, 41020 us after connect(); early write() -> EAGAIN x2 t=1.369550
IGKM-E: N4 POLLIN wakeups=77 for 77 bytes (level-triggered readiness kept while plaintext stays buffered), spurious=0
```

### 6. Tap: only TLS records, markers 0 times: WORKS

```
SCAN   tls streams: 64336 bytes, marker(IGKM-E-) occurrences=0
SCAN   plain streams: 85 bytes, marker(IGKM-E-) occurrences=2
SCAN     plain: b'IGKM-E-PLAIN-REQ pass-through guest->host\n' occurrences=1
```

- Every TLS direction shows `unframed_bytes=0`: 30 directions in run 0005, and 30 in
  run 0007.
- Each of the 28 REQUEST/RESPONSE lines in `pcap_scan.py` occurs 0 times on TLS streams.
- `:5001` is the positive control: the same scan finds its plaintext.
- The capture hashes match the ones printed on the host:
  - `0005.tap.pcap` `723e2f96…`
  - `0006.tap.pcap` `ba56a60b…`
  - `0007.tap.pcap` `c2941d52…`

### 7. NewSessionTicket works; other post-handshake messages still fail closed: WORKS

**Default rustls tickets.** Every rustls peer sends two tickets, as two handshake
messages in **one** record (edge case 2). The guard discards both, and they raise no
EPOLLIN:

```
MTLSGUARD: [c5] rx record gen=0 seq=0 wire=184 inner=0x16 plaintext=162 t=0.825046
MTLSGUARD: [c5] rx post-handshake NewSessionTicket #1 (77 B body) in record seq=0 -> discarded (no resumption) t=0.826427
MTLSGUARD: [c5] rx post-handshake NewSessionTicket #2 (77 B body) in record seq=0 -> discarded (no resumption) t=0.828441
MTLSGUARD: [c5] rx record gen=0 seq=1 wire=63 inner=0x17 plaintext=41 t=0.912400
MTLSGUARD: [c5] app readiness OUT -> IN OUT (published by the guard) t=0.914359
IGKM-E: N1 EPOLLIN wakeups=1 spurious (EPOLLIN but first read() = EAGAIN)=0
```

In Spike B, the one-ticket control case failed closed with EPROTO. Every blocking case
now passes against peers that send tickets.

**Crafted tickets on `:6448`.** The raw peer wrote four records in one write, and they
arrived in one TCP segment:
- NST1 plus the first 20 bytes of NST2
- the rest of NST2
- NST3 and NST4 together
- the greeting

```
PEER[6448#1] raw tx ONE write of 352 bytes = records [0x17/94 0x17/54 0x17/131 0x17/53] = [inner 0x16 77B, inner 0x16 37B, inner 0x16 114B, inner 0x17 36B]: NST1 (57 B) + first 20 B of NST2 | remaining 37 B of NST2 | NST3 + NST4 | greeting IGKM-E-GREETING raw-nst peer->guest\n t+2.207525s
SCAN       seg#2 t+2.207513s: 0x17/94 0x17/54 0x17/131 0x17/53 COALESCED
MTLSGUARD: [c10] rx post-handshake NewSessionTicket #1 (53 B body) in record seq=0 -> discarded (no resumption) t=1.854478
MTLSGUARD: [c10] rx handshake message continues in the next record (20 B buffered)
MTLSGUARD: [c10] rx post-handshake NewSessionTicket #2 (53 B body) in record seq=1 -> discarded (no resumption) t=1.859241
MTLSGUARD: [c10] rx post-handshake NewSessionTicket #3 (53 B body) in record seq=2 -> discarded (no resumption) t=1.862723
MTLSGUARD: [c10] rx post-handshake NewSessionTicket #4 (53 B body) in record seq=2 -> discarded (no resumption) t=1.864753
IGKM-E: R1 raw-nst read 36 bytes: IGKM-E-GREETING raw-nst peer->guest
```

**Unexpected message on `:6449`.** The peer sent a ticket, a CertificateRequest
(type 13) and application data, all in one segment (`0x17/74 0x17/24 0x17/62 COALESCED`):

```
MTLSGUARD: [c11] rx post-handshake NewSessionTicket #1 (53 B body) in record seq=0 -> discarded (no resumption) t=1.968449
MTLSGUARD: [c11] FAIL CLOSED: unexpected post-handshake handshake message type 13 (CertificateRequest) -> read()/write() now return -71, fatal alert sent t=1.971837
IGKM-E: R2 epoll events=IN ERR  read result=1 n=-71 (Protocol error); application bytes delivered=0
PEER[6449#1] raw rx record inner 0x15: 020a (0x15 0232 = fatal unexpected_message) t+2.330946s
```

- The application data behind the CertificateRequest was never decrypted or delivered.
- The guest also fails closed on the following, but no case exercised them:
  - a KeyUpdate that is not at a record boundary
  - application data inside a fragmented handshake message
  - any other inner content type

### 8. KeyUpdate with `update_requested`: WORKS

In case K (`:6446`), the rustls peer calls `refresh_traffic_keys()` before each of three
responses.

```
PEER[6446#1] refresh_traffic_keys(): KeyUpdate(update_requested) queued ahead of response 1; peer tx keys advance t+1.856543s
MTLSGUARD: [c9] rx record gen=0 seq=1 wire=27 inner=0x16 plaintext=5 t=1.662426
MTLSGUARD: [c9] rx KeyUpdate(update_requested) in record seq=1 -> rx traffic secret N+1 = HKDF-Expand-Label(secret_N, "traffic upd"), rx keys now generation 1, rx seq 0 t=1.664338
MTLSGUARD: [c9] tx record gen=0 seq=1 KeyUpdate(update_not_requested) plaintext=5 wire=27 (outer 0x17, inner 0x16) t=1.669084
MTLSGUARD: [c9] sent KeyUpdate(update_not_requested) under the old tx keys, then tx keys -> generation 1, tx seq 0 t=1.672416
MTLSGUARD: [c9] rx record gen=1 seq=0 wire=58 inner=0x17 plaintext=36 t=1.675745
MTLSGUARD: [c9] tx record gen=1 seq=0 appdata plaintext=35 wire=57 (outer 0x17, inner 0x17) t=1.684030
PEER[6446#1] rx plaintext line 2 (35 bytes): IGKM-E-REQK2 keyupdate guest->peer\n t+2.073296s
PEER[6446#1] rx plaintext line 3 (35 bytes): IGKM-E-REQK3 keyupdate guest->peer\n t+2.104997s
MTLSGUARD: [c9] close: state=established tx records=3 rx records=3 NewSessionTickets discarded=2 KeyUpdates rx=3 tx=3
```

- The guest decrypted each response under its advanced receive keys (generations 1, 2
  and 3).
- The peer decrypted REQK2 and REQK3, which the guest sent under transmit keys of
  generations 1 and 2. So rustls applied the guest's KeyUpdates and derived the same
  keys.
- On the wire, the key changes are 22-byte ciphertext records (`0x17/22`) in both
  directions, with no plaintext:

```
SCAN     records: 0x16/189 0x14/1 0x17/605 0x17/52 0x17/22 0x17/52 0x17/22 0x17/52 0x17/22 0x17/19      (guest->peer)
SCAN       seg#3 t+2.016770s: 0x17/22 0x17/53 COALESCED                                                 (peer->guest: KeyUpdate + response)
```

**`update_not_requested`**, tested by R1. rustls cannot send this on demand, so the raw
peer sent it:

```
PEER[6448#1] raw tx ONE write: KeyUpdate(update_not_requested) record 0x17/22 under tx generation 0, then tx keys -> generation 1 (seq 0), then response record 0x17/50 under the NEW key: IGKM-E-RESPR raw-nst peer->guest\n t+2.228424s
MTLSGUARD: [c10] rx KeyUpdate(update_not_requested) in record seq=4 -> rx traffic secret N+1 = HKDF-Expand-Label(secret_N, "traffic upd"), rx keys now generation 1, rx seq 0 t=1.876293
MTLSGUARD: [c10] tx record gen=0 seq=1 appdata plaintext=41 wire=63 (outer 0x17, inner 0x17) t=1.884555
PEER[6448#1] raw rx record rx_seq=1 wire=63 inner 0x17 plaintext (41 bytes): IGKM-E-REQR2 after-keyupdate guest->peer\n t+2.240877s
```

Only the guest's receive keys advanced. Its transmit keys stayed at generation 0, and the
peer decrypted REQR2 with its unchanged key.

**Key-schedule self-test.**
- Key and IV derivation are checked against RFC 8448 §3. The RFC text was fetched for
  this (sha256 `6564d137…`).
- The `traffic upd` value was computed off-guest with Python `hmac`/`hashlib`, because
  RFC 8448 has no KeyUpdate vector.

```
MTLSGUARD: self test: Mbed TLS 3.6.7 gcm=0 sha256=0 AES-NI=yes; HKDF-Expand-Label RFC 8448 key=OK iv=OK, traffic upd (python vector)=OK
```

### 9. Deny under non-blocking connect: WORKS

Case N5 (`:6444`):

```
MTLSGUARD: [c12] host DENY: policy deny: local spiffe://overdrive.test/ns/default/sa/guest-client <-> peer spiffe://overdrive.test/ns/default/sa/peer-denied (handshake aborted: invalid peer certificate: ApplicationVerificationFailure)
MTLSGUARD: [c12] connect 192.168.204.1:6444: FAIL closed (-13), TCP shut down, no application data sent (after 40495 us) t=2.024682
IGKM-E: N5 nb-deny epoll_wait -> ERR HUP (LT) after 7 empty 1-ms probes t=2.026914
IGKM-E: N5 nb-deny wait result=1 events=ERR HUP  SO_ERROR=13 (Permission denied), SO_ERROR again=0, write() afterwards=-1 errno=13 (Permission denied) t=2.028292
SCAN conn client guest:54384 -> server peer:6444 (TLS)
SCAN   guest->peer: 200 bytes flags[FIN,SYN] marker(IGKM-E-)=0 records=2 (0x17 count=0) unframed_bytes=0
SCAN     records: 0x16/189 0x14/1
```

- The socket never reported EPOLLOUT.
- SO_ERROR returned the error once and was then cleared, as on Linux.
- The guest sent no `0x17` record.
- N6 (agent down, non-blocking) behaves the same:
  `events=ERR HUP  SO_ERROR=103 (Software caused connection abort)`.

### 10. Spike B blocking regression: WORKS

```
IGKM-E: VERDICT A pass-through=PASS
IGKM-E: VERDICT B deny=PASS                  (connect() returned -1 errno=13)
IGKM-E: VERDICT D server-first=PASS          (now against a peer that sends 2 tickets)
IGKM-E: VERDICT S1 inbound-allowed=PASS      (blocking accept() gets the socket after 34558 us)
IGKM-E: VERDICT S2 inbound-denied=PASS       (accept() returned -1 errno=103)
IGKM-E: VERDICT E client-first=PASS
IGKM-E: VERDICT E after-relay-kill=PASS      (RUNNER: relay killed t+2.495663s; PEER[6450#1] rx plaintext line 2 … t+10.532406s)
IGKM-E: VERDICT F agent-down=PASS            (host relay unreachable … (-111); connect() -1 errno=103)
```

Key custody, from run 0005's relay log:

```
RELAY[c5] custody: host->guest total 1040 bytes; client SVID key occurrences PKCS#8 DER=0 PEM=0 private scalar=0; guest-server SVID key occurrences PKCS#8 DER=0 PEM=0 private scalar=0; application traffic secrets sent (changed vs Spike B: expected 1 each when ALLOW) = [1,1]
```

Across all 13 relay connections in run 0005:
- SVID key occurrences (DER/PEM/scalar) were 0/0/0 for both keys.
- The 10 allowed connections sent the traffic secrets once each (`[1,1]`).
- The 3 denied connections sent none (`[]`).

### 11. Stretch: non-blocking listener under epoll: WORKS

The hold now depends on whether the listener is blocking, so a non-blocking `accept()` is
not held.

```
MTLSGUARD: [c1] accept 192.168.204.2:7443 <- 192.168.204.1:43158: returning the fd now; handshake continues in the guard thread (app readiness withheld) t=0.195732
IGKM-E: SN accept4(SOCK_NONBLOCK) on the non-blocking listener returned 2 errno=0 after 16363 us (no handshake hold) t=0.198448
MTLSGUARD: [c1] record layer installed: … peer identity (from host) spiffe://overdrive.test/ns/default/sa/peer-client … t=0.216635
MTLSGUARD: [c1] accept 192.168.204.2:7443 <- 192.168.204.1:43158: handshake done after 35893 us; RX pump running t=0.226227
IGKM-E: SN fd 2 events=IN  read()=32 errno=0 (-) t=0.422708
IGKM-E: SN fd 2 read: IGKM-E-REQI inbound peer->guest
IGKM-E: SN accept4(SOCK_NONBLOCK) on the non-blocking listener returned 2 errno=0 after 16293 us (no handshake hold) t=0.459315
MTLSGUARD: [c2] vsock rx DENY payload=213 wire=218 t=0.490201
IGKM-E: SN fd 2 events=ERR HUP  read()=-1 errno=103 (Software caused connection abort) t=0.500556
```

- `accept4` returned before the record layer was installed on the allowed connection,
  and before the DENY on the denied one.
- The allowed fd's first event was EPOLLIN, after REQI was decrypted.
- The denied fd reported an error.

The probe predicted "a few ms" for `accept4`. It took 16.3 ms both times, mostly the hook's
own three serial log lines (the install line alone took 4.5 ms), not a hold. For
comparison, the blocking hold in S1 took 34.6 ms.

On Spike B's question: Spike B refused non-blocking accepts with EOPNOTSUPP. Applying its
hold there would have blocked a non-blocking `accept()` for the whole handshake. The fix
was cheap: the hook no longer waits when the listener is non-blocking. The cost is edge
case 7.

## The exact patches and their sizes

| Change | Where | Size |
|---|---|---|
| connect post-hook (Spike B 0001, unchanged, sha256 `e3b35083…`) | `unikraft/lib/posix-socket` | 26 lines, 3 files |
| accept post-hook (Spike B 0002, unchanged, sha256 `9c798cce…`) | `unikraft/lib/posix-socket` | 23 lines, same files |
| **readiness interposition (new 0003, sha256 `a4857fce…`)** | `lib-lwip`: `sockets.c` (+23/−2) and new `include/uk/lwip_readiness.h` (+19) | 2 files, +42/−2 |
| lib-mtlsguard (out of tree) | `guard.c` | 1 837 lines, including 58 log statements (Spike B: 1 038) |

- `libmtlsguard.o` is 60 122 B of text and 8 872 B of bss. Spike B's was 35 893 B and
  74 472 B; the per-connection state moved from bss to the heap.
- The Mbed TLS files compiled are `aes`, `aesni`, `gcm`, `block_cipher`,
  `constant_time`, `platform_util`, `sha256`, `md` and `hkdf`.
- The core `lib/posix-socket` patch did not grow: non-blocking support needed no new core
  hook. The readiness patch is one routing decision at two call sites.

## What changed versus Spike B

- **Key custody changed.** The application traffic secrets of each connection now cross
  vsock: SECRETS v2 carries, per direction, the 32-byte secret plus rustls' key and IV.
  Spike B sent only key and IV.
  - With the secrets, the guest can derive every future traffic key of that one
    connection. It could already decrypt that connection.
  - The SVID private key still never crosses: 0 occurrences of both keys' PKCS#8 DER,
    PEM and raw scalar on every connection.
- **Guard driver timing.** The guard driver is now installed at connect/accept time, not
  after SECRETS. Until then it returns EAGAIN for `read` and `write`.
- **Blocking hold.** The Spike B blocking hold is now the guard thread plus a wait. It no
  longer holds the socket's write lock across the handshake.
- **Readiness.** The guard owns the readiness applications see, through the lib-lwip
  hook. lwIP's own readiness is only an input.
- **Record layer.**
  - reassembles post-handshake messages
  - discards tickets
  - implements KeyUpdate in both directions
  - sends a fatal alert when it fails closed (Spike B relayed none)
- **Relay.**
  - runs one thread per vsock connection
  - captures the traffic secrets with `KeyLog`
  - sends SECRETS v2
- **Peers.** They use rustls' default tickets. Spike B's "NewSessionTicket fails closed"
  control became a "NewSessionTicket works" case.

## Edge cases discovered

1. **A Unikraft thread entry function must never return.**
   - `uk_thread_fn1_t` is `__noreturn`, and `ukarch_ctx_init_entry1`
     (`arch/x86/ctx.c:126`) pushes a return address of 0. A returning `guard_thread`
     would have jumped to address 0 when the first mesh connection closed.
   - The only evidence was the build's single warning in run 0003:
     `passing argument 2 of 'uk_sched_thread_create_fn1' makes '__attribute__((noreturn))' qualified function pointer from unqualified`.
   - The fix is `uk_sched_thread_exit()`. This is the same lesson as Spike B's
     `exportsyms.uk`: a clean-looking build hid a mechanism that would not work.
2. **rustls sends its two tickets as two messages in one record.** A reassembler that
   assumed one message per record would break.
3. **Tickets add latency to the first response.**
   - The ticket record goes out in its own segment. Linux Nagle then holds the server's
     first response until lwIP's delayed ACK of that segment, which added 20–155 ms in
     run 0005. For example: `PEER[6445#1] sent greeting first … t+0.627513s`, but on the
     tap `seg#3 t+0.766689s: 0x17/58`.
   - This is Spike B's edge case 4, now triggered on the outbound path by every ordinary
     TLS 1.3 server.
4. **lwIP coalesces the relayed client Finished flight with the application's first
   record** into one segment. The stream order is correct, and the app wrote only after
   EPOLLOUT.
5. **The readiness hook runs with interrupts disabled** inside lwIP's `event_callback`
   (`SYS_ARCH_PROTECT` is `uk_lcpu_save_irqf`).
   - It may only record lwIP's events, compute the application's readiness and wake the
     guard thread.
   - Reads from lwIP and all vsock I/O must happen in the thread.
6. **The design relies on the single-vCPU cooperative scheduler.** The hook reads guard
   state without a lock. SMP, or a multi-threaded application, would need atomics or a
   lock the hook can take with interrupts off.
7. **A non-blocking listener hands the application an fd before policy is decided.**
   - A denied caller becomes an fd that later reports `ERR HUP` and ECONNABORTED. With a
     blocking listener, `accept()` fails instead.
   - This is Linux-like (it looks like a reset after accept), but it is a difference.
   - A guard-side accept queue that surfaces only completed handshakes would remove it.
8. **Closing while the peer's close_notify is still unread sends RST, not FIN.** lwIP
   does what Linux does here, and the peer still logs a clean close first.
9. **The handshake hold is about 33–59 ms per connection** in these runs, with verbose
   serial logging on. These are single samples, not a benchmark.

## What we assumed wrong

- **The research called the proxy `uk_file` the larger fallback. It was not needed.** A
  routing hook in lib-lwip lets the guard own the readiness of the one existing
  `uk_file`: 23 changed lines in `sockets.c` plus a header.
- **#303 says "KeyUpdate stays unsupported" (Costs and risks, citing #229).**
  - On the unikernel path this is no longer true. With the traffic secrets, the guest
    runs the KeyUpdate schedule itself: about 100 lines plus Mbed TLS HKDF.
  - The Linux kTLS path (#229) was not examined.
- **"The key never enters the guest" needs to be more precise.** The SVID key never
  does. The per-connection traffic secrets now do; Spike B sent only per-connection keys.
- **"Hold the application's first read or write" is the wrong shape for non-blocking
  I/O.** The right contract is Linux's still-connecting socket:
  - `connect()` returns EINPROGRESS
  - neither readiness event is reported
  - `write()` returns EAGAIN
  - the socket becomes writable only once the record layer is installed
  - if policy denies the connection, the socket reports an error through SO_ERROR
- **The probe's `accept4` estimate was wrong.** It predicted "a few ms"; it took 16 ms,
  mostly logging.

## Design implications for #303

- **Option 2 of the Unikernels section now covers non-blocking I/O and post-handshake
  messages on Unikraft.**
  - Now demonstrated: non-blocking sockets with poll/epoll readiness, discarding
    NewSessionTicket, and KeyUpdate.
  - Still needed before production:
    - multi-threaded applications and SMP (edge case 6)
    - `app-elfloader`
    - authenticating the guest on the vsock channel
    - fixing the lwIP delayed-ACK latency cliff, which now hits both directions (edge
      case 3)
    - a performance comparison against the host proxy (#287)
- **Two repositories to carry, as the research predicted (R1).** The patches are
  `unikraft` `lib/posix-socket` (49 lines) and `lib-lwip` `sockets.c` (+42/−2). Both are
  small, and each adds a single hook point.
- **D2 (handshake on the host) still holds, with an amended custody statement.**
  - The guest still needs no X.509 code, no clock and no key, and the SVID key stays on
    the host.
  - The per-connection traffic secrets now cross vsock, so the guest can rekey without
    putting the agent back in the data path.
  - If traffic secrets must stay on the host, rustls' `KernelConnection` is the supported
    API. It costs a host round trip per KeyUpdate, and rekeying then depends on the agent
    being up.
- **D3 on Unikraft is now three pieces:**
  - the connect and accept post-hooks
  - a per-socket driver swap, installed at connect/accept time
  - lwIP readiness routed to the installed driver, so the mechanism owns the readiness
    applications see
- **Density.** This probe runs one guard thread per mesh connection, about 200 KiB
  including stacks (computed, not measured). At the unikernel density tier (#295), the
  design should run one guard event loop per guest. That is the next structural change,
  not a feasibility question.

## Commands

All metal runs go through `capture.sh`, which calls `cargo xtask metal run`. That command
rsyncs with the `.env` guard, takes the shared lease and runs the fail-closed native
preflight. From the repo root:

```
bash spike-scratch/in-guest-kernel-mtls/increment-e/capture.sh 0001 inventory --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-e/host-inventory.sh
bash spike-scratch/in-guest-kernel-mtls/increment-e/capture.sh 0002 build-host --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-e/build-host.sh
bash spike-scratch/in-guest-kernel-mtls/increment-e/capture.sh 0004 build-guest-noreturn-fix --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-e/build-guest.sh
OVERDRIVE_METAL_KERNEL=<metal-home>/igkm-spike-a/unikraft/out-igkme/igkme_fc-x86_64 \
  bash spike-scratch/in-guest-kernel-mtls/increment-e/capture.sh 0005 boot-mesh-fc1.17.0 -- bash spike-scratch/in-guest-kernel-mtls/increment-e/run-boot-fc.sh mesh
OVERDRIVE_METAL_KERNEL=… capture.sh 0006 boot-stretch-fc1.17.0 -- … run-boot-fc.sh stretch
OVERDRIVE_METAL_KERNEL=… capture.sh 0007 boot-mesh-repeat-fc1.17.0 -- … run-boot-fc.sh mesh
bash spike-scratch/in-guest-kernel-mtls/increment-e/capture.sh 0008 host-state-check-final --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-e/host-state-check.sh
```

The guest is built with Unikraft's own make and Kconfig, in new worktrees; Spike B's
trees are untouched:

```
make -C unikraft-igkme A=app-igkme L=lib-lwip-igkme:lib-mtlsguard-igkme O=build-igkme C=app-igkme/.config LEX=flex YACC=bison UK_CFLAGS=-std=gnu17 MTLSGUARD_MBEDTLS_DIR=mbedtls/mbedtls-3.6.7
```

Firecracker runs as `firecracker --no-api --config-file vm.json --id igkm-e`, with the
same VM shape as Spike B. The `boot_args` are:

```
igkme netdev.ip=192.168.204.2/24:192.168.204.1 mtlsguard.mesh=192.168.204.1:6443-6450 mtlsguard.mesh_in=192.168.204.2:7443 mtlsguard.agent_port=7100 -- run
```

The stretch run passes `-- stretch` instead.

## Versions

| Component | Version |
|---|---|
| Host kernel (`uname -r`) | `7.0.0-29-generic`; never modified (no modules, sysctls or packages) |
| Firecracker | v1.17.0, sha256 `99ad0f5c…` |
| Unikraft core | `eb8fa2368618cea11c9bde196f79e6e6b9caeed5` + increment-d 0001/0002, worktree `unikraft-igkme` |
| lib-lwip | `ec55ae17618feeb57c8c10109bcf5c42723e8e95` + increment-e 0003, worktree `lib-lwip-igkme` |
| Mbed TLS | 3.6.7 LTS release tarball, sha256 `a7e8bcbec0e6f761b4af24f25677626b35f762f68eef79c08677a363212d11f6` (shared with Spike B) |
| Guest image | runs 0005–0007: `a1d09df7…` (681 696 B, entry `0x18577a`); resolved config `config/igkme.config` |
| Host stubs | rustls 0.23.45 (ring provider), ring 0.17.14 (also a direct dependency), rustls-webpki 0.103.15, rustls-pki-types 1.15.1, rcgen 0.14.10, x509-parser 0.18.1; rustc and cargo 1.95.0 |
| Guest toolchain | gcc 15.2.0 (`-std=gnu17`), GNU ld 2.46, make 4.4.1; flex 2.6.4, bison 3.8.2 and m4 1.4.19 (built in user space by Spike A3) |
| Capture | tcpdump 4.99.6, libpcap 1.10.6 |
| KAT source | RFC 8448 text, sha256 `6564d1376d1ec744fc7a9993da15ebc1b9be361908b166091f47ef605c537fba` |

## Host state created and cleanup

**Per boot run (0005, 0006, 0007):**
- one uniquely named tap (`igke29101928`, `igke29102245`, `igke29102358`) on
  192.168.204.1/24
- one tcpdump on the tap
- the `peer` and `relay` processes, plus the `inclient` subshell
- one Firecracker process (no jailer, no API socket)
- a per-run directory

Each runner's cleanup removed all of these, and its before/after host-state diff showed
no leftovers: no links added or removed, no processes, no listeners and no run directory.

**Run 0008 (read-only) found nothing left:**
- no firecracker, jailer, peer, relay, inclient or tcpdump processes
- no `igk*` links and no tun/tap links
- no 192.168.203.x or 192.168.204.x address
- no listeners on 5001, 6443-6450 or 7100
- no Unix sockets and no run directories
- 0 root-owned files in the scratch tree and in increments d and e
- the pristine clones are clean, and the Spike B worktree is still exactly 0001+0002

**Host kernel and packages:** untouched. No system packages were installed.

**Kept on purpose** (user-owned, outside `~/overdrive`):
- `~/igkm-spike-a`, now 1.4 GB (was 1.1 GB). B2 added:
  - `igkme-host-target` (163 MB)
  - `unikraft/build-igkme` (110 MB), `out-igkme` (5.1 MB), `app-igkme` and
    `lib-mtlsguard-igkme`
  - worktrees `unikraft-igkme` (17 MB) and `lib-lwip-igkme` (0.5 MB), both registered
    in the pristine clones
- the throwaway test PKI, in the gitignored
  `~/overdrive/spike-scratch/in-guest-kernel-mtls/increment-e/out/certs` on the host

To remove only B2's additions:

```
git -C ~/igkm-spike-a/unikraft/unikraft worktree remove --force ~/igkm-spike-a/unikraft/unikraft-igkme
git -C ~/igkm-spike-a/unikraft/lib-lwip worktree remove --force ~/igkm-spike-a/unikraft/lib-lwip-igkme
rm -rf ~/igkm-spike-a/igkme-host-target ~/igkm-spike-a/unikraft/{build,out,app,lib-mtlsguard}-igkme
```

To remove everything: `rm -rf ~/igkm-spike-a`.

## Gate recommendation

WORKS for all eleven items. Keep the probe, and use "option 2 on Unikraft covers
non-blocking I/O, NewSessionTicket and KeyUpdate" as input to D1, D2 and D3. Amend D2's
custody wording: the per-connection traffic secrets now cross vsock.

Do not start a walking skeleton until both of these are done:
- D1 is decided.
- A follow-up probe covers multi-threaded applications and SMP, the guard as one event
  loop per guest, and `app-elfloader` binary compatibility.
