# Spike B findings: the in-guest mTLS mechanism built into Unikraft, on Firecracker

GH #303, feature `in-guest-kernel-mtls`. PROBE phase, throwaway. Probe sources, patches, the
resolved `.config` and raw captures are in `spike-scratch/in-guest-kernel-mtls/increment-d/`:
- runs `runs/0001` to `runs/0013`, each `{meta,stdout,stderr}`, append-only, host identifiers
  redacted
- tap captures `runs/0009.tap.pcap` and `runs/0012.tap.pcap`

The increment `README.md` has the design, every pre-run hypothesis, prediction and
falsification, and the full run log. Run 0009 is the primary evidence for items 1–8. Run 0012
covers item 9 and re-runs 1–8 under a corrected relay protocol.

The probe agent could not write under `docs/`, so the orchestrator wrote this file from its
report and re-checked the verdict, scan, custody and relay-kill lines against runs 0009 and 0012.

## Verdict: WORKS (blocking sockets)

A plain-socket native Unikraft program with no TLS code gets transparent mutual TLS 1.3 on its
mesh connections:
- A hook in Unikraft's socket layer holds `connect()`, and `accept()` in the stretch.
- The TLS handshake runs in a host rustls stub over vsock, with the certificate key on the host.
- The host sends back only per-direction traffic keys, and the guest carries the connection with
  its own TLS 1.3 record layer.
- Policy is decided on the host from the verified peer SPIFFE ID.

The whole Unikraft change is a 49-line patch to `lib/posix-socket` plus an out-of-tree library.
`lib-lwip` is not patched.

| # | Checklist item | Verdict | Run |
|---|---|---|---|
| 1 | mTLS handshake through the relay; the peer logs the client SPIFFE ID from the host-held cert; the guest's 1999 clock is irrelevant | **WORKS** | 0009, 0012 |
| 2 | tap: only TLS records after the handshake; app data is `0x17`; REQUEST/RESPONSE plaintext appears 0 times | **WORKS** | 0009, 0012 |
| 3 | client-speaks-first REQUEST written right after `connect()` returns arrives byte-exact | **WORKS** | 0009, 0012 |
| 4 | server-speaks-first completes | **WORKS** | 0009, 0012 |
| 5 | relay stub killed after the handshake; second exchange on the same connection works | **WORKS** | 0009, 0012 |
| 6 | deny fails `connect()`; no application data records on the wire | **WORKS** | 0009, 0012 |
| 7 | pass-through (non-mesh) connect works in plaintext, unaffected | **WORKS** | 0009, 0012 |
| 8 | key custody: vsock carries only handshake records and traffic secrets | **WORKS** | 0009, 0012 |
| 9 | stretch: the same mechanism on `accept()` (guest server, host rustls client) | **WORKS** (allowed and denied caller) | 0012 |

Out of scope and **not** shown:
- non-blocking sockets and poll/epoll readiness (a non-blocking mesh `connect`/`accept` is
  refused with `EOPNOTSUPP`)
- KeyUpdate
- session resumption
- post-handshake NewSessionTicket handling (the guest deliberately fails closed on it)
- multi-threaded applications
- unmodified Linux binaries through `app-elfloader`

**Substrate** (run 0001, fail-closed substrate probes):
- native metal host, x86_64, AMD EPYC 8024P
- `uname -r` = `7.0.0-29-generic`
- `systemd-detect-virt` = `none`, `/dev/kvm` API 12, `aes` CPU flag present
- Firecracker v1.17.0 (`99ad0f5c…`)

**Guest:**
- Unikraft `eb8fa236` (boot banner `Ijiraq 0.21.0~eb8fa236-custom`), lib-lwip `ec55ae17`
- Mbed TLS 3.6.7 LTS, nolibc
- 1 vCPU, 512 MiB, no drives

## Question tested

Does the #303 mechanism work inside Unikraft on Firecracker, where there is no kTLS and the
record layer must run in the unikernel? The mechanism:
- hook `connect`
- hold it
- relay the handshake over vsock to a host rustls agent that holds the key and decides policy
- encrypt in the guest with traffic secrets supplied by the host

## Design as built

### Core patch 0001: connect post-hook

26 lines in 3 files of `lib/posix-socket`. `connect()` calls
`uk_socket_connect_hook(sock, addr, addr_len, blocking, ret)` after the lwIP connect and the
existing blocking `EINPROGRESS` wait, and before `uk_ofile_release`. A `__weak` default returns
`ret`.

The research had already ruled out the alternatives:
- a second `POSIX_SOCKET_FAMILY_REGISTER(AF_INET)` is forbidden
- the socket-events payload carries no socket handle

This call site is the smallest one that has the connected `struct uk_file *` in hand:

```c
	if (ret == 0 || ret == -EINPROGRESS)
		ret = uk_socket_connect_hook(of->file, addr, addr_len,
					     _SHOULD_BLOCK(mode), ret);
	uk_ofile_release(of);
```

The rest of the patch is:
- the prototype in `include/uk/socket.h`, with a `struct uk_file;` forward declaration
- the weak default in `socket.c`
- one line in `lib/posix-socket/exportsyms.uk` (see edge case 1)

### Core patch 0002: accept post-hook (stretch)

23 lines in the same 3 files. `uk_sys_accept` calls
`uk_socket_accept_hook(listener, sock, blocking, flags)` after `uk_socket_accept` returns the new
socket and before the socket gets an fd. A non-zero return releases the socket and fails
`accept()`.

### lib-mtlsguard: the out-of-tree library

Everything else is an out-of-tree Unikraft library: `guard.c` (1 038 lines including logging)
plus the Mbed TLS 3.6.7 AES-GCM subset. `libmtlsguard.o` is 35 893 B of text and 74 472 B of
bss. For a mesh destination the library does five things.

1. **Holds.** The hook runs inside the `connect` syscall with TCP already up. It takes the
   socket's write lock and does not return until SECRETS, DENY, ERROR, or a 10 s timeout. For
   `accept()`, the hold happens before the fd exists.
2. **Relays.** It opens `AF_VSOCK` with `uk_socket_create` (no fd) to (CID 2,
   `mtlsguard.agent_port`) and follows the host's lock-step frames. It touches the TCP peer only
   when told to.
3. **Swaps the driver.** On SECRETS it writes a per-socket `struct posix_socket_driver` into
   `posix_sock_get_node(sock)->driver`. That driver has the guard ops, the per-socket state in
   `.private`, and lwIP's allocator.
   - lwIP's `sock_data` is untouched, so the guard ops call lwIP's ops on the same file.
   - `close` restores lwIP's driver before freeing.
   - Blocking I/O needs no extra code. lwIP sockets are non-blocking underneath, posix-fdio and
     posix-socket already loop on `-EAGAIN` plus `uk_file_poll`, and the guard `read` returns
     `-EAGAIN` until a whole record has arrived.
4. **Runs the record layer.** TLS 1.3 with `TLS_AES_128_GCM_SHA256` only, pinned in the rustls
   provider on both host ends.
   - nonce = IV XOR sequence number; AAD = the 5-byte record header; inner type
     `application_data`
   - one record per `write` (at most 16 KiB)
   - received records must have outer type `0x17` and a valid tag
   - `close_notify` maps to EOF; any other inner type fails the socket closed (sticky)
5. **Applies the policy outcome.**
   - DENY: `connect()` returns -1 `EACCES`; `accept()` returns -1 `ECONNABORTED`.
   - ERROR, or relay unreachable: `ECONNABORTED`.
   - TCP is shut down in all three cases.

### Configuration

Configuration uses Unikraft library parameters, not compile-time constants:
- `mtlsguard.mesh=192.168.203.1:6443-6446`
- `mtlsguard.mesh_in=192.168.203.2:7443`
- `mtlsguard.agent_port=7100`

The full `boot_args` line is:

```
igkmd netdev.ip=192.168.203.2/24:192.168.203.1 mtlsguard.mesh=192.168.203.1:6443-6446 mtlsguard.mesh_in=192.168.203.2:7443 mtlsguard.agent_port=7100 -- run
```

### Crypto library

Upstream Mbed TLS 3.6.7 LTS (release tarball sha256 `a7e8bcbe…d11f6`):
- Only `aes.c aesni.c gcm.c block_cipher.c constant_time.c platform_util.c` are compiled, under
  a five-line config (`HAVE_ASM AES_C AESNI_C GCM_C SELF_TEST`).
- `unikraft/lib-mbedtls` was not used: that port pins Mbed TLS 2.18.1 (2019, end of life) and
  builds the whole library.
- The built-in known-answer tests pass in the guest, and AES-NI runs through Mbed TLS's
  inline-assembly path with CPUID detection:

```
GCM note: using AESNI.
MTLSGUARD: gcm self test result=0, AES-NI in use=yes, mesh="192.168.203.1:6443-6446", agent vsock port=7100
```

### Relay protocol (vsock, host-driven, lock-step)

One frame is `type:u8 | len:u32 BE | payload`.

| type | name | direction | payload and meaning |
|---|---|---|---|
| 0x01 | OPEN | guest → host | `4`, IPv4, port (7 B). The host becomes the TLS client with the workload's client SVID. |
| 0x08 | ACCEPT | guest → host | `4`, local ip, local port, remote ip, remote port (13 B). The host becomes the TLS server with the workload's server SVID and requires a client cert. |
| 0x02 | TO_PEER | host → guest | TLS bytes the guest writes to the TCP peer |
| 0x03 | NEED_PEER | host → guest | Empty: read exactly one TLS record from the TCP peer |
| 0x04 | FROM_PEER | guest → host | That record (0 bytes means peer EOF) |
| 0x05 | SECRETS | host → guest | Suite `0x1301`; tx seq (8), key length (1), key (16), IV (12); the same for rx; peer ID length (2) plus peer SPIFFE ID (informational). 78 B plus the ID. |
| 0x06 | DENY | host → guest | Reason text |
| 0x07 | ERROR | host → guest | Reason text |

Rules around the protocol:
- The host refuses to extract secrets unless `tx_seq == rx_seq == 0` and rustls holds no
  received plaintext.
- The guest closes vsock after SECRETS, DENY or ERROR.
- On a failed handshake, the alert rustls queues is not relayed; the guest just shuts TCP down.
- In run 0009, NEED_PEER meant "read once, whatever is available". For run 0012 it became one
  record per frame (edge case 3).

## Evidence (predicted vs actual, real output)

The test topology is on tap `igkmd0` (host 192.168.203.1, guest .2).

The host `peer` program (rustls) runs these listeners:
- `:6443` TLS echo
- `:6444` TLS echo with a denied identity
- `:6445` TLS, server speaks first
- `:6446` TLS echo that sends one NewSessionTicket
- `:5001` plain TCP

Every TLS listener requires a client cert from the throwaway CA and sends no 0.5-RTT data. None
sends tickets except `:6446`. The `relay` program (rustls) listens on `<uds_path>_7100`.

The guest runs these cases in order: A pass-through, B deny, C NST control, D server-first,
S inbound (stretch), E client-first plus relay kill, F agent-down, G non-blocking.

### 1. Handshake through the relay, client identity from the host-held cert: WORKS

Predicted: the peer logs `verified client SPIFFE ID=…/guest-client`, and the relay reports ALLOW
with `tx_seq=0 rx_seq=0`. Actual (run 0009):

```
RELAY[c4] vsock rx OPEN payload=7 dst=192.168.203.1:6443 local identity=spiffe://overdrive.test/ns/default/sa/guest-client t+0.649284s
RELAY[c4] handshake complete: policy ALLOW local spiffe://overdrive.test/ns/default/sa/guest-client -> peer spiffe://overdrive.test/ns/default/sa/peer-allowed; suite=TLS13_AES_128_GCM_SHA256 tx_seq=0 rx_seq=0 buffered_plaintext=false t+0.654655s
PEER[6443#1] handshake OK version=TLSv1_3 suite=TLS13_AES_128_GCM_SHA256 verified client SPIFFE ID=spiffe://overdrive.test/ns/default/sa/guest-client t+0.656905s
MTLSGUARD: [c4] record layer installed: TLS_AES_128_GCM_SHA256 tx_seq=0 rx_seq=0, peer identity (from host) spiffe://overdrive.test/ns/default/sa/peer-allowed; socket driver lwip -> mtlsguard t=0.321089
```

The 1999 guest clock did not matter, by design:
- The guest reads `ukplat_wall_clock()=946598400.056401`, which is 1999-12-31.
- Every certificate is valid only from 2026-09-27 to 2026-10-28, a window that excludes the
  guest's date, and every handshake still succeeded.
- rustls checked validity on the host clock. The guest never parses a certificate.

### 2. Only TLS records on the tap, plaintext 0 times: WORKS

`tcpdump -i igkmd0` captured the whole boot. `pcap_scan.py` reassembles every TCP stream by
sequence number and parses the record framing.

Predicted:
- `unframed_bytes=0` in every TLS direction
- the marker `IGKM-D-` appears 0 times on TLS streams
- plaintext is found on `:5001`, the positive control

Actual (run 0009, client-first connection):

```
SCAN conn guest:51828 -> peer:6443 (TLS)
SCAN   guest->peer: 956 bytes flags[FIN,RST,SYN] marker(IGKM-D-)=0 records=6 (0x17 count=4) unframed_bytes=0
SCAN     records: 0x16/189 0x14/1 0x17/605 0x17/54 0x17/58 0x17/19
SCAN   peer->guest: 976 bytes flags[FIN,SYN] marker(IGKM-D-)=0 records=6 (0x17 count=4) unframed_bytes=0
SCAN     records: 0x16/122 0x14/1 0x17/690 0x17/55 0x17/59 0x17/19
SCAN   tls streams: 6685 bytes, marker(IGKM-D-) occurrences=0
SCAN   plain streams: 85 bytes, marker(IGKM-D-) occurrences=2
```

The records after the handshake match the guest's own record log byte for byte:

| Wire record | Guest log line | Content |
|---|---|---|
| guest→peer `0x17/54` | `tx record seq=0 … wire=59` | REQ1 |
| guest→peer `0x17/58` | `tx record seq=1 wire=63` | REQ2 |
| guest→peer `0x17/19` | `close_notify wire=24` | close_notify |
| peer→guest `0x17/55` | `rx record seq=0 wire=60` | RESP1 |
| peer→guest `0x17/59` | `rx record seq=1 wire=64` | RESP2 |

The `0x17/605` before them is the client's encrypted Certificate, CertificateVerify and Finished,
which the relay produced (`TO_PEER … records[0x14/1 0x17/605]`).

In run 0012 the scan covered 10 152 TLS bytes: marker count 0 and `unframed_bytes=0` everywhere,
including `:7443`. Both captures are kept. Each matches the hash the runner printed on the host:
`runs/0009.tap.pcap` sha256 `a3781d54…` and `runs/0012.tap.pcap` sha256 `80bb7a00…`.

### 3. Client-speaks-first REQUEST right after `connect()` returns: WORKS

```
MTLSGUARD: [c4] connect 192.168.203.1:6443: RELEASE connect() = 0 after 18609 us hold t=0.326796
IGKM-D: E client-first connect() returned 0 errno=0 (-) after 20388 us t=0.328448
MTLSGUARD: [c4] tx record seq=0 appdata plaintext=37 wire=59 (outer 0x17, inner 0x17) t=0.329817
PEER[6443#1] rx plaintext line 1 (37 bytes): IGKM-D-REQ1 client-first guest->peer\n t+0.697776s
```

The hold is the blocking `connect()` itself: the app has no socket to write on until the record
layer is installed. The task accepted this as the hold for blocking sockets. It does not show
that an early write is buffered without loss, which is how #303 words the item. Nothing can be
written early here, so there is nothing to buffer.

### 4. Server-speaks-first: WORKS

```
PEER[6445#1] sent greeting first (41 bytes): IGKM-D-GREETING server-first peer->guest\n t+0.621244s
MTLSGUARD: [c3] rx record seq=0 wire=63 inner=0x17 plaintext=41 t=0.295839
IGKM-D: D server-first read 41 bytes: IGKM-D-GREETING server-first peer->guest
PEER[6445#1] rx plaintext line 1 (37 bytes): IGKM-D-REQS server-first guest->peer\n t+0.635950s
PEER[6445#1] clean close (close_notify received); application plaintext bytes received=37 t+0.642229s
```

### 5. Relay killed after the handshake, same connection keeps working: WORKS

Predicted: exchange 2 succeeds on sequence 1 after the relay is SIGKILLed. The runner kills the
relay only after the peer has answered REQ1 and the relay has closed that connection. Actual
(run 0009):

```
RELAY[c4] guest closed the vsock connection (agent out of the data path) t+0.664296s
RUNNER: client-first exchange 1 answered and relay connection c4 finished -> SIGKILL relay pid 258209 t+0.742045s
RUNNER: relay killed t+0.755822s; kill -0: gone
RUNNER: relay processes now: []
RUNNER: unix listeners on the agent path now: []
IGKM-D: E woke; exchange 2 on the SAME connection t=8.364815
MTLSGUARD: [c4] tx record seq=1 appdata plaintext=41 wire=63 (outer 0x17, inner 0x17) t=8.366954
PEER[6443#1] rx plaintext line 2 (41 bytes): IGKM-D-REQ2 after-relay-kill guest->peer\n t+8.704957s
MTLSGUARD: [c4] rx record seq=1 wire=64 inner=0x17 plaintext=42 t=8.372971
IGKM-D: VERDICT E after-relay-kill=PASS
```

Case F then shows that the guest fails closed when the agent is gone, rather than falling back
to plaintext:

```
MTLSGUARD: [c5] host relay unreachable on vsock cid 2 port 7100 (-111) -> fail closed t=8.394584
IGKM-D: F agent-down connect() returned -1 errno=103 (Software caused connection abort) after 14790 us t=8.403764
SCAN   guest->peer: 0 bytes flags[FIN,SYN] …        (that connection carried no bytes)
```

### 6. Deny: WORKS

Predicted: `connect()` returns -1 `EACCES`, and guest→peer carries only the ClientHello. Actual
(run 0009):

```
RELAY[c1] handshake NOT completed: policy deny: local spiffe://overdrive.test/ns/default/sa/guest-client -> peer spiffe://overdrive.test/ns/default/sa/peer-denied (handshake aborted: invalid peer certificate: ApplicationVerificationFailure) t+0.561688s
MTLSGUARD: [c1] connect 192.168.203.1:6444: FAIL closed, connect() returns -13, TCP shut down, no application data sent (held 17207 us) t=0.232378
IGKM-D: B deny connect() returned -1 errno=13 (Permission denied) after 19901 us t=0.234865
PEER[6444#1] handshake FAILED: unexpected end of file; application plaintext bytes received=0 t+0.569926s
SCAN   guest->peer: 194 bytes flags[FIN,SYN] marker(IGKM-D-)=0 records=1 (0x17 count=0) unframed_bytes=0
SCAN     records: 0x16/189
```

In run 0012, with one record per NEED_PEER, the same connection shows `0x16/189 0x14/1`. There is
still no `0x17` from the guest (edge case 3).

### 7. Pass-through: WORKS

```
MTLSGUARD: connect 192.168.203.1:5001: not a mesh destination -> pass-through, driver untouched t=0.207517
PEER[5001#1] rx plaintext line 1 (42 bytes): IGKM-D-PLAIN-REQ pass-through guest->host\n t+0.548250s
SCAN     contains plaintext b'IGKM-D-PLAIN-REQ pass-through guest->host\n'
```

This is also the scanner's positive control: the same scan that reports 0 on the TLS streams
finds both plaintext lines here.

### 8. Key custody: WORKS

Frame types and sizes from both ends, for relay connection `c4` in run 0009:

```
RELAY[c4] tally guest->host OPEN: frames=1 wire_bytes=12
RELAY[c4] tally guest->host FROM_PEER: frames=1 wire_bytes=833
RELAY[c4] tally host->guest TO_PEER: frames=2 wire_bytes=820
RELAY[c4] tally host->guest NEED_PEER: frames=1 wire_bytes=5
RELAY[c4] tally host->guest SECRETS: frames=1 wire_bytes=133
RELAY[c4] vsock tx SECRETS payload=128 wire=133 (suite=0x1301 tx: seq 0 key 16B iv 12B; rx: seq 0 key 16B iv 12B; peer id 50B) t+0.654667s
RELAY[c4] custody: host->guest total 958 bytes; occurrences of client key PKCS#8 DER=0 PEM=0 private scalar=0
MTLSGUARD: [c4] vsock summary: rx TO_PEER x2 (810 B), NEED_PEER x1; tx FROM_PEER 828 B; closing vsock (agent leaves the data path) t=0.324441
```

Host→guest frames were only these types:
- TO_PEER: TLS handshake records, encrypted after ServerHello
- NEED_PEER: empty
- SECRETS: 78 B plus the peer ID
- DENY and ERROR: text

The relay scans every byte it sends to the guest for the PKCS#8 DER, the PEM file and the raw
32-byte P-256 scalar of each key it holds. The counts were 0/0/0 on all four relay connections
in run 0009. In run 0012 they were 0/0/0 on all six connections, for both the `client` and
`guest-server` keys. The keys exist only in the relay process (`load_key` reads them from the
certificate directory). The guest image contains no certificate and no key.

### 9. Stretch: the same mechanism on `accept()`: WORKS

Allowed caller (`inclient` presenting `peer-client`), run 0012:

```
MTLSGUARD: [c4] accept 192.168.203.2:7443 <- 192.168.203.1:55586: mesh listener -> HOLD the new socket (no fd yet) until the host handshake completes t=0.405322
RELAY[c4] vsock rx ACCEPT payload=13 local=192.168.203.2:7443 remote=192.168.203.1:55586 role=TLS server, local identity=spiffe://overdrive.test/ns/default/sa/guest-server t+0.749242s
RELAY[c4] handshake complete: policy ALLOW local spiffe://overdrive.test/ns/default/sa/guest-server <-> peer spiffe://overdrive.test/ns/default/sa/peer-client; suite=TLS13_AES_128_GCM_SHA256 tx_seq=0 rx_seq=0 buffered_plaintext=false t+0.761759s
MTLSGUARD: [c4] accept 192.168.203.2:7443 <- 192.168.203.1:55586: RELEASE, accept() gets the socket after 24749 us hold t=0.430072
IGKM-D: S1 read 32 bytes: IGKM-D-REQI inbound peer->guest
INCLIENT[peer-client] client-side handshake done version=TLSv1_3 suite=TLS13_AES_128_GCM_SHA256 server SPIFFE ID=spiffe://overdrive.test/ns/default/sa/guest-server t+0.755187s
INCLIENT[peer-client] read 33 bytes: IGKM-D-RESPI inbound guest->peer\n t+0.969064s
```

Denied caller (`peer-client-denied`):

```
RELAY[c5] handshake NOT completed: policy deny: local spiffe://overdrive.test/ns/default/sa/guest-server <-> peer spiffe://overdrive.test/ns/default/sa/peer-client-denied (handshake aborted: invalid peer certificate: ApplicationVerificationFailure) t+1.015430s
IGKM-D: S2 accept() returned -1 errno=103 (Software caused connection abort) t=0.691950
SCAN conn client peer:55600 -> server guest:7443 (TLS)
SCAN   guest->peer: 827 bytes flags[FIN,RST,SYN] marker(IGKM-D-)=0 records=3 (0x17 count=1) unframed_bytes=0
SCAN     records: 0x16/122 0x14/1 0x17/689
```

- The `0x17/689` from guest to caller is the server's encrypted handshake flight, produced by the
  relay.
- The caller is a TLS 1.3 client, which is done once it has sent Finished, so it wrote its REQI
  (`0x17/49` caller→guest) before it learned of the rejection.
- The guest application never saw a byte, because `accept()` never returned that connection.

All ten guest verdicts in run 0012 are `PASS` (A, B, C, D, S1, S2, E, E after kill, F, G), and
Firecracker exited 0 on its own. Run 0009 has the same eight without S1 and S2.

## Edge cases discovered

1. **A weak hook is silently never overridden unless it is in `exportsyms.uk`.** Unikraft makes
   every library symbol not listed there local.
   - Run 0005 built cleanly with the strong hook present, but `nm` showed a local
     `t uk_socket_connect_hook` next to the `T`.
   - Disassembly in run 0007 showed `connect()` calling the local weak default:
     `128e51: call 128d10 <uk_socket_connect_hook>`.
   - The cause is the per-library `objcopy --keep-global-symbols` step
     (`support/build/Makefile.rules:1036-1050`).
   - Adding the hook to `lib/posix-socket/exportsyms.uk` fixed it: `128e51: call 166cc0`.
   - With green tests and no disassembly check, this would have looked like "the hook does
     nothing".
2. **Circular header order.** `socket_driver.h` includes `socket.h` before `file.h`, so a
   prototype that takes `struct uk_file *` needs a forward declaration. Run 0004 showed the
   warning from `lib-lwip/sockets.c`.
3. **The relay must give the host exactly one TLS record per request.**
   - On the server side, a TLS 1.3 client sends application data right after its Finished. "Read
     whatever TCP has" would hand the host the first application record, the host would consume
     it (rx seq 1), and the guest would lose it.
   - The host's `rx_seq == 0 && no buffered plaintext` check turns that into an ERROR rather than
     silent loss.
   - With one record per NEED_PEER, S1 shows `buffered_plaintext=false`, and REQI stayed in lwIP
     for the record layer.
   - Cost: the server's first flight now takes three vsock round trips instead of one, and the
     connect hold went from 18.6 ms to 27.8 ms. These are single samples with verbose serial
     logging, not a benchmark.
   - The finer interleaving also moves rustls's middlebox-compatibility CCS. On the deny
     connection it now goes out right after ServerHello (`0x16/189 0x14/1`), still with no
     `0x17`.
4. **lwIP's delayed ACK outlasts Linux's minimum RTO.**
   - In S1, the caller's final flight (614 B at relative sequence 194) was sent at 10.720 ms and
     retransmitted at 219.585 ms.
   - The guest had already consumed it (relay FROM_PEER at t+0.758–0.762 s) but did not ACK. The
     server has nothing to send after the client Finished, so lwIP delays the ACK past the host's
     200 ms RTO.
   - Nagle then held REQI until the ACK at 219.868 ms, so REQI reached the guest 193 ms after
     `accept()` released the socket.
   - Neither the relay nor the record layer is on that path, and a plain lwIP TLS server would
     behave the same. It is a latency cliff for inbound request/response.
5. **NewSessionTicket fails the connection closed, by design, and production cannot do that.**
   - Case C logged
     `rx post-handshake handshake message type 4 (NewSessionTicket) -> unsupported, fail closed`,
     and `read()` returned -1 `EPROTO`.
   - Most TLS 1.3 servers send tickets by default (rustls sends 2).
   - A production record layer must parse and discard NST. The host could suppress tickets only
     on its own side, and it does not control the far end.
6. **The deny alert is not relayed.** The peer sees EOF (`unexpected end of file`) instead of a
   TLS alert. The host could relay the alert rustls queued, which is encrypted under handshake
   keys; that would give the peer better diagnostics.
7. **TCP and the ClientHello reach the peer even on deny.** Policy needs the peer's certificate,
   which only arrives in the handshake. No application bytes cross. The #303 host-proxy design
   has the same property.
8. **vsock close noise.** Every guest vsock close is followed by
   `libvirtio_vsock … Got packet to port N without socket`: the host's close arrives after the
   guest has freed the socket. It is harmless.
9. **The guest wall clock reads 1999** (Spike A3 edge case 1). It does not affect this mechanism,
   because the guest never validates a certificate.
10. **The application can read its own traffic keys.** Unikraft is a single address space, so the
    AES keys sit in memory the app can read. Only the SVID private key is protected, as #303
    already says for unikernels. The custody scan showed this; no isolation test was run.
11. **Harness issues.**
    - The metal host's shared `~/.cargo/registry` has root-owned entries (run 0002), so the host
      build uses a spike-private `CARGO_HOME`.
    - The runner's final `grep VERDICT` needed `-a`, because the serial log contains NUL bytes.

## What we assumed wrong

- **The relay protocol needs more than "relay the handshake records".** #303 and the research
  describe the relay that way. The protocol also has to be record-granular and host-driven, or
  the host consumes application data (edge case 3). Neither document calls this out.
- **"Fail closed on any non-application record" breaks against ordinary TLS 1.3 peers.** That is
  the task's spec, and a natural reading of #303's "KeyUpdate stays unsupported". It is
  incompatible with ordinary peers because they send tickets (edge case 5).
- **The core change is smaller than the research sized it.** The research expected one call-out
  per dispatch point: `connect`, `accept`, and `read`/`write`. The driver swap makes the
  `read`/`write` call-outs unnecessary. Only `connect` and `accept` need hooks, 49 lines in
  total. The care needed is in `exportsyms.uk` and weak linkage (edge case 1).
- **No `lib-lwip` patch was needed for blocking sockets.** The research said one is needed for
  readiness suppression. That still holds for non-blocking sockets, which this spike did not
  test.

## Design implications for #303

- **Option 2 of the Unikernels section is buildable on Unikraft and Firecracker for blocking
  sockets.** Option 2 ports the mechanism into the unikernel's socket layer. It needs:
  - a small core patch: `connect` and `accept` hooks, 49 lines in `lib/posix-socket`, carried
    across pin advances
  - an out-of-tree library: about 1 000 lines of C plus the Mbed TLS AES-GCM subset
  - a host agent

  Together with Spike A3 (Unikraft needs no boot patch on Firecracker), this is real input to D1.
- **D2 (handshake on the host) is confirmed as the right split for unikernels.**
  - The guest needs only AEAD: no X.509, no time, no key.
  - The key stays on the host: every custody scan counted 0.
  - The agent leaves the data path after the handshake, as the relay kill shows.
- **D3 (hook choice) for Unikraft:** a post-hook in `connect` and `accept` plus a per-socket
  driver swap. There is no family re-registration and no global table overwrite, and other
  sockets are untouched (pass-through).

A production version still needs:
- **Non-blocking sockets and poll/epoll readiness.** lwIP pushes readiness straight to the
  `uk_file` (research, "Additional finding"). This needs a `lib-lwip` patch or a proxy `uk_file`.
  Buffered plaintext in the guard also needs to raise `EPOLLIN`. Today these sockets are refused
  with `EOPNOTSUPP`.
- **Post-handshake control records.**
  - discard NewSessionTicket
  - KeyUpdate (#229): either implement it in the guest, which needs the TLS 1.3 key schedule
    (HKDF) or a host round trip, or refuse it honestly
  - alert semantics beyond close_notify
- **Multi-threaded applications.** The hold takes the socket write lock, but a second thread
  blocking on that lock was not tested.
- **Binary compatibility.** Unmodified Linux binaries go through `app-elfloader` →
  `syscall_shim` → the same `connect`/`accept` (research F-UK-5). This spike ran only a native
  app.
- **Stream semantics.**
  - `MSG_PEEK`
  - `FIONREAD`, which reports ciphertext bytes today
  - `sendfile`/`splice`
  - TCP Fast Open, which bypasses `connect`
  - `shutdown` half-close ordering
- **Relay protocol hardening.**
  - authenticate the guest on the vsock channel; today the per-VM Unix path is the only binding
  - per-handshake timeouts
  - relay the deny alert
- **Inbound latency.** Tune lwIP's delayed ACK, or quick-ACK after the handshake, to avoid the
  200 ms RTO cliff (edge case 4).
- **Service-name resolution and load balancing (D4).** This spike did not touch them.
- **Performance relative to the host proxy (#300, #287).** Not measured. Each connection's
  handshake costs about 5–7 vsock round trips.

## Commands

All metal runs go through `capture.sh` → `cargo xtask metal run`, which does the rsync with the
`.env` guard, takes the shared lease and runs the fail-closed native preflight. From the repo
root:

```
bash spike-scratch/in-guest-kernel-mtls/increment-d/capture.sh 0001 inventory --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-d/host-inventory.sh
bash spike-scratch/in-guest-kernel-mtls/increment-d/capture.sh 0010 build-host-accept --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-d/build-host.sh
bash spike-scratch/in-guest-kernel-mtls/increment-d/capture.sh 0011 build-guest-accept --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-d/build-guest.sh
OVERDRIVE_METAL_KERNEL=<metal-home>/igkm-spike-a/unikraft/out-igkmd/igkmd_fc-x86_64 \
  bash spike-scratch/in-guest-kernel-mtls/increment-d/capture.sh 0012 boot-mesh-accept-fc1.17.0 -- bash spike-scratch/in-guest-kernel-mtls/increment-d/run-boot-fc.sh mesh
bash spike-scratch/in-guest-kernel-mtls/increment-d/capture.sh 0013 host-state-check-final --no-sudo -- bash spike-scratch/in-guest-kernel-mtls/increment-d/host-state-check.sh
```

The guest is built with Unikraft's own make/Kconfig (no kraft):

```
make -C unikraft-igkmd A=app-igkmd L=lib-lwip:lib-mtlsguard O=build-igkmd C=app-igkmd/.config LEX=flex YACC=bison UK_CFLAGS=-std=gnu17 MTLSGUARD_MBEDTLS_DIR=mbedtls/mbedtls-3.6.7
```

- **Firecracker:** `firecracker --no-api --config-file vm.json --id igkm-d`, with 1 vCPU,
  512 MiB, one virtio-net on the tap, vsock guest CID 3, and no drives.
- **Host programs:** `cargo build --release --locked` in `host/`, which is its own workspace and
  not a member of the repo workspace.

## Versions

| Component | Version |
|---|---|
| Host kernel (`uname -r`) | `7.0.0-29-generic`; never modified (no modules, sysctls or packages) |
| Firecracker | v1.17.0, sha256 `99ad0f5c…` (Spike A2's verified binary) |
| Unikraft core | `eb8fa2368618cea11c9bde196f79e6e6b9caeed5` plus patches 0001 (sha256 `e3b35083…`) and 0002, in worktree `unikraft-igkmd` of the pristine A3 clone |
| lib-lwip | `ec55ae17618feeb57c8c10109bcf5c42723e8e95`, unpatched |
| Mbed TLS | 3.6.7 LTS release tarball, sha256 `a7e8bcbec0e6f761b4af24f25677626b35f762f68eef79c08677a363212d11f6` |
| Guest image | run 0009 `a43bee60…`; run 0012 `c07f7684…` (628 448 B, entry `0x17967a`, `_lxboot_entry`, no PVH note) |
| Host stubs | rustls 0.23.45 (ring provider), ring 0.17.14, rustls-webpki 0.103.15, rustls-pki-types 1.15.1, rcgen 0.14.10, x509-parser 0.18.1 (from `host/Cargo.lock`); rustc/cargo 1.95.0 |
| Guest toolchain | gcc 15.2.0 (`-std=gnu17`), GNU ld 2.46, make 4.4.1; flex 2.6.4, bison 3.8.2 and m4 1.4.19 built in user space by Spike A3 |
| Capture | tcpdump 4.99.6, libpcap 1.10.6 |

## Host state created and cleanup

- **Per boot run:** each boot run (0009 and 0012) created:
  - tap `igkmd0` (192.168.203.1/24)
  - one tcpdump on it (`-Z root -w -`, written to a file in the run directory)
  - the `peer` and `relay` processes, plus one `inclient` subshell in run 0012
  - one Firecracker process (no jailer, no API socket)
  - a per-run directory

  The runner's cleanup removed all of them and diffed host state: no links added or removed, and
  no processes, listeners or run directory left.
- **Run 0013 (read-only check):** found nothing left.
  - no firecracker, jailer, peer, relay, tcpdump or listener processes
  - no `igkm*` tap and no tun/tap links
  - no `192.168.203.x` address and no listeners on 5001/6443-6446
  - no Unix sockets, no run directories, 0 root-owned files
- **Host kernel and packages:** untouched.
- **Kept on purpose** (user-owned, outside `~/overdrive`):
  - `~/igkm-spike-a`, now 1.1 GB (728 MB after A3). It includes the spike-private `cargo-home`,
    `igkmd-host-target`, the Unikraft worktree `unikraft-igkmd` (registered in the pristine
    clone), the build directory and Mbed TLS.
  - The throwaway test PKI, in the gitignored
    `~/overdrive/spike-scratch/in-guest-kernel-mtls/increment-d/out/certs` on the host.
  - Remove both with `rm -rf ~/igkm-spike-a ~/overdrive/spike-scratch/in-guest-kernel-mtls/increment-d/out`.

## Gate recommendation

WORKS for blocking sockets. Keep the probe, and use "unikernel option 2 is buildable" as input to
D1 and D2. Do not start a walking skeleton until two things happen:
- a follow-up probe covers non-blocking/epoll readiness (the `lib-lwip` patch) and post-handshake
  NewSessionTicket/KeyUpdate handling
- D1 is decided
