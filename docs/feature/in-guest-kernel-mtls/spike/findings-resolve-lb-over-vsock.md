# Spike D findings — service-name resolution + first-healthy backend selection moved onto the host-agent vsock channel, in the in-guest kernel-module path on Firecracker

GH #303 (`in-guest-kernel-mtls`), decision **D4** ("Choose where service-name
resolution and service load balancing live under this model"). PROBE phase,
throwaway. Probe: `spike-scratch/in-guest-kernel-mtls/increment-i/`. Evidence:
`increment-i/runs/0001..0009.{meta,stdout,stderr}` plus `runs/0005.tap.pcap`,
`runs/0007.tap.pcap`, `runs/0008.tap.pcap` (committed, append-only, host
identifiers redacted). Authoritative boot runs: **0007** (`mesh-run`) and **0008**
(`mesh-run-repro`), identical in verdict.

The probe agent could not write under `docs/`, so the orchestrator wrote this file
from its report and re-checked the headline lines (resolve→chose, the health-toggle
move to backend B, `RESOLVE-MATCH`, custody 0, wire markers 0, VIP packets 0, deny
errno=13, host-clean) against the committed `runs/` captures.

## Verdict: WORKS — Shape 2 (resolve at connect time, with destination rewrite)

A stock-guest in-guest kernel module CAN re-home the one L4 job that lives in the
host proxy's capture path today — ADR-0072 `MtlsResolve` (service-name resolution +
first-healthy backend selection) — onto the **same host-agent vsock channel** the
in-guest mTLS mechanism already uses. The guest app dials a stable VIP; the module's
`connect()` hook asks the agent over vsock to resolve the VIP to a first-healthy
backend, **rewrites the connecting socket's destination to that backend before the
TCP connect**, and the lossless hold + mTLS handshake + kTLS install then run exactly
as in Spike C, to the chosen backend. A fresh connect after a backend goes unhealthy
re-resolves and lands on the healthy one; a policy-denied service fails `connect()`
with `-EACCES` and no TCP. The SVID private key never crosses vsock, and a RESOLVE
carries no key material.

**The load-bearing unknown is answered YES:** a guest kernel module can redirect a
connecting socket's destination to an agent-chosen address from inside the `connect`
hook. The hook runs under `lock_sock(sk)`, but the RESOLVE round trip uses a
**separate** `AF_VSOCK` socket that never touches `sk`, so it does not deadlock. This
is the in-guest equivalent of the host's old `cgroup_connect4` / XDP rewrite.

Shape 1 (resolve at name-lookup time, no connect-time rewrite) was **not built**: the
task led with Shape 2 and made Shape 1 optional if Shape 2 worked. Shape 2 worked.
Its trade is under "Shape 1 (not built)" below.

## Substrate + versions (run 0001)

- Metal host: AMD EPYC 8024P, x86_64, `systemd-detect-virt=none`, `/dev/kvm` usable → SUBSTRATE OK (fail-closed per testing.md).
- Host kernel `uname -r` = `7.0.0-29-generic` (Ubuntu 7.0.0-29.29).
- Guest kernel `uname -r` = `7.0.0-29-generic` — the host's own stock `/boot/vmlinuz-7.0.0-29-generic` bzImage booted directly. No kernel patch/rebuild.
- Firecracker v1.17.0, sha256 `99ad0f5cd0514a88aad0e9ae8cfdb3cc3b4ab9d190e1194602406c786b5de7a5`.
- gcc `(Ubuntu 15.2.0-16ubuntu1) 15.2.0`; rustc/cargo `1.95.0`.
- Module `module/igkmd_mtls.c` = 614 lines (Spike C's 523 + 91); built `.ko` 786,864 B, vermagic matches the guest.

## The new vsock resolve frame (wire shape)

Added to the Spike C relay protocol (`host/src/lib.rs`), reusing its `type:u8 | len:u32 BE | payload` framing. A RESOLVE is a short lookup — no TLS state machine, no key material either way.

```
0x09 RESOLVE   guest->host  family:u8(=4) · vip_ip[4] · vip_port:u16 BE                 (7 B, same shape as OPEN)
0x0a RESOLVED  host->guest  family:u8(=4) · backend_ip[4] · backend_port:u16 BE
                           · peer_id_len:u16 BE · peer_id (UTF-8)                         (observed 59 B)
     DENY  (0x06, reused)  host->guest  UTF-8 reason  — policy-denied service; guest connect() -> -EACCES
     ERROR (0x07, reused)  host->guest  UTF-8 reason  — no-healthy-backend / no-such-service
```

The guest module opens a fresh vsock connection per resolve (CID 2, port 7100), sends
RESOLVE, reads one reply, closes. The same agent process that runs the mTLS handshake
answers RESOLVE on the same vsock port — one channel, D4 as #303 proposed.

## The destination-rewrite mechanism (Shape 2) — exact

`tcp_prot.connect` is invoked by `inet_stream_connect` with `lock_sock(sk)` held.
`hooked_connect` (`igkmd_mtls.c`):

1. if the dialed `sockaddr_in` is a mesh VIP (the `mesh=` module param lists VIPs), calls `do_resolve(sk, vip_ip, vip_port, …)` — a blocking RESOLVE round trip on a separate `AF_VSOCK` socket made with `sock_create_kern(sock_net(sk), AF_VSOCK, …)`;
2. on `RESOLVED`, writes the chosen backend into the caller's sockaddr — `sin->sin_addr.s_addr = backend_ip; sin->sin_port = backend_port;` — and records `(sk → backend, expected_peer)` in the per-socket table;
3. calls the original `tcp_prot.connect(sk, uaddr, addr_len)` with the rewritten address, so the kernel dials the backend, not the VIP;
4. on `DENY`, returns `-EACCES` from the hook WITHOUT calling the original connect — so `connect()` fails and no SYN is emitted.

The lossless HOLD + TLS 1.3 handshake relay + kTLS install run lazily on the first
`sendmsg`/`recvmsg` (called without the sk lock), exactly as Spike C, now to the
chosen backend; the module then checks the handshake's verified peer SPIFFE ID equals
the one RESOLVE promised (`RESOLVE-MATCH … match=1`).

## The relocated `MtlsResolve` selection (agent side)

The agent (`host/src/bin/relay.rs`) holds a service registry (from a services file)
and reads per-backend health from a health directory on every resolve — mirroring
`service_backends` + the `healthy` column `MtlsResolve` reads. Selection is
**first-healthy by `Ord`** over the backend set, exactly `MtlsResolve`:

```
RELAY   service svc-a vip 10.80.0.1:9443 policy=allow backends=[192.168.204.1:6443,192.168.204.1:6444] expected_peer=spiffe://overdrive.test/ns/default/sa/peer-allowed
RELAY   service svc-denied vip 10.80.0.9:9443 policy=deny backends=[] expected_peer=spiffe://overdrive.test/ns/default/sa/peer-denied
```

The two backends present the SAME service SPIFFE ID (`peer-allowed`) on the same IP
(192.168.204.1, the cert IP SAN) at distinct ports — a service's instances share the
service identity; the serving port is the distinguishing evidence, and each backend's
echo embeds `from-A` / `from-B` so the guest's own plain `read()` proves which backend
served. Health is driven from the test by removing backend A's flag file between the
two connects.

## Checklist — binary verdicts (each tied to the run that evidences it)

Cited from run 0007; 0008 is equivalent (A=10 / B=9 wire packets, VIP=0, both
`match=1`, custody 0, `unframed_bytes=0`).

| # | Item | Verdict | Evidence (runs/0007) |
|---|------|---------|----------------------|
| 1 | Guest dials a VIP; module asks the agent over vsock; agent logs the resolve request + chosen backend | PASS | guest `[resolve] agent chose backend 192.168.204.1:6443 … rewriting connect dst`; relay `RELAY[c1] resolve service=svc-a backends(by Ord)[…=healthy …=healthy] -> chose 192.168.204.1:6443 (first-healthy, MtlsResolve-style)` |
| 2 | Connection lands on the chosen backend — serving backend log AND the wire; req/resp byte-exact | PASS | `PEER[A:6443#1] tx … IGKM-D-RESP-svca-c1 from-A …`; guest `lb-c1 read(52) PLAINTEXT: IGKM-D-RESP-svca-c1 from-A … lb-connect-1`; wire 10 pkts to :6443 |
| 3 | mTLS holds: TLS 1.3 only, plaintext 0×, kTLS ULP, SVID key 0× on vsock, expected-peer observed to match backend (`match=1`; logged, not enforced — see Design implications) | PASS | `SCAN tls streams: 3641 bytes, marker(IGKM-D-) occurrences=0` (`unframed_bytes=0` all streams); `/proc/net/tls_stat TlsTxSw=2 TlsRxSw=2`; custody `client/guest-server SVID key occurrences PKCS#8 DER=0 PEM=0 private scalar=0`; `RESOLVE-MATCH … match=1` |
| 4 | Freshness: mark first-choice unhealthy → next connect selects the other, byte-exact | PASS | `RELAY[c3] resolve … backends(by Ord)[192.168.204.1:6443=unhealthy 192.168.204.1:6444=healthy] -> chose 192.168.204.1:6444`; guest `lb-c2 read(72) PLAINTEXT: IGKM-D-RESP-svca-c2 from-B … after-health-toggle`; `PEER[B:6444#1] …` |
| 5 | Deny: resolve for a policy-denied service fails connect, no app bytes on the wire | PASS | guest `connect VIP 10.80.0.9:9443 FAILED errno=13 (Permission denied)`, `DENY-CASE OK … no TCP SYN, no application bytes`; `packets to any VIP 10.80.0.x: 0` |
| 6 | Which shape + rewrite mechanism + vsock resolve frame | PASS | Shape 2 (resolve at connect time + sockaddr rewrite before `orig_connect`); frames RESOLVE 0x09 / RESOLVED 0x0a |
| — | VIP never dialed on the wire (rewrite happened pre-connect) | PASS | `packets to any VIP 10.80.0.x: 0 (expected 0)`; only SYNs are to 192.168.204.1 :5001/:6443/:6444 |
| — | Pass-through (non-mesh :5001) unaffected, plaintext (positive control) | PASS | `SCAN plain: IGKM-D-REQ-PT-pt … occurrences=1` / `IGKM-D-RESP-PT-pt from-PT … occurrences=1` |
| — | Host kernel untouched / no leaks | PASS | run 0009: `igkm* modules on host: []`, `host tainted? 0`, no firecracker/peer/relay/tcpdump, no `igkd*`/`igkh*` taps, no `192.168.204.x`, no run/health-dir residue → `host clean = YES` |

## Edge cases / observations

- **The vsock RESOLVE runs under `lock_sock(sk)` and does not deadlock** — the whole Shape-2 question. Safe because the resolve uses a separate `AF_VSOCK` socket and never does raw I/O on `sk`; `sock_create_kern` + `kernel_connect` + blocking frame I/O complete while `sk` is held, and `sk` is the just-created connecting socket nothing else references yet.
- **The VIP never reaches the wire.** The rewrite happens before `orig_connect`, so route lookup / ARP / SYN all use the backend address. `tcpdump` saw 0 packets to any `10.80.0.x`. A VIP needs no guest route — it is pure input to the hook.
- **Deny is stronger than Spike C's.** Spike C's deny failed the first I/O (lazy handshake). Here a policy-denied *service* fails `connect()` itself with `-EACCES` and emits no SYN, because the denial is decided at resolve, before the TCP connect. Both the service-level resolve DENY and Spike C's handshake-time peer DENY remain and agree.
- **kTLS is software kTLS** (`TlsTxSw=2 TlsRxSw=2`) — TLS 1.3, not NIC-offloaded. Same as Spike C.
- **Capture-tail timing (harness, not a mechanism finding).** Run 0005 lost connect-2's wire tail because the guest's hard `reboot -f` tore the NIC down before the pcap flushed; a 3 s pause + 2 s settle (runs 0006→0007/0008) fixed it. Run 0004 had a dropped "create tap" section in the runner, so all TCP failed `EHOSTUNREACH` while RESOLVE still worked over the tap-independent vsock — which isolated it as a runner bug. `0004.tap.pcap` is a 0-byte artifact (no tap existed).

## Shape 1 (not built) and the staleness trade

Shape 1 resolves at name-lookup time: the agent surfaces the backend to the guest (a
DNS answer or a vsock name-query), so the guest connects straight to the backend with
no connect-time rewrite — a simpler datapath (no sockaddr rewrite, no vsock round trip
under the connect lock). Its risk is **health staleness**: a name→backend answer
computed at lookup can outlive that backend's health, so a connection opened later
against a cached answer can land on a now-unhealthy backend until the answer is
re-resolved. Shape 2 has no such window because every `connect()` re-resolves against
live health — which is what made the freshness move (item 4) land on B with no stale-A
attempt. Shape 2 works, so Shape 1 was not needed; a future design wanting the simpler
datapath must add a cache-invalidation/TTL story for health that Shape 2 avoids by
construction.

## Design implications for #303 D4

- **D4 answer:** service-name resolution + first-healthy backend selection can live on
  the host agent, reached over the same vsock channel, resolved at the guest module's
  `connect()` hook with a destination rewrite. This re-homes the one L4 job the host
  proxy does today (ADR-0072 `MtlsResolve`) into the in-guest path, so **the host
  proxy's resolution/LB job can be deleted** along with the proxy once in-guest mTLS
  lands — the selection logic is relocated, not reinvented (first-healthy by `Ord`,
  unchanged).
- **The agent stays the single policy + identity + resolution authority.** It already
  held the SVID key and decided handshake policy; D4 adds service→backend resolution
  and the expected-peer identity to the same channel. **Honest scope:** the module only
  *logs* whether the handshake peer matches what resolution promised (`RESOLVE-MATCH
  match=%d`, `match=1` in every run here, `igkmd_mtls.c:604`) — it does **not yet
  enforce** it; a `match=0` is logged, not rejected, and kTLS is already installed by
  that point. mTLS itself is still enforced (the handshake peer is an identity the
  relay authenticated), but binding the connection to the *resolved* peer — intended-peer
  pinning — is production work, the same concern deferred in the transparent-mtls arc
  (#236). (Surfaced in PR #306 review, finding 2.)
- **Health is an input the agent reads at resolve time** (here a flag file; production
  reads `service_backends.healthy`). Because Shape 2 resolves per connect, health
  changes take effect on the next connect with no cached-answer staleness.
- **No key custody change.** A RESOLVE carries only a VIP in and a backend addr +
  expected-peer string back; the SVID private key still never crosses vsock (0× in
  every custody scan).
- **Open items (inherited from Spike C, unchanged by D4):** the `accept()`/inbound
  path, non-blocking/epoll readiness, IPv6 (`tcpv6_prot`), robust per-socket teardown +
  safe `rmmod`, the `sockptr.h:49` fortify warning on kernel-context kTLS install. D4
  adds: **enforce `RESOLVE-MATCH`** (reject a handshake-peer ≠ resolved-peer mismatch
  rather than only logging it — intended-peer pinning, #236); the resolve round trip
  adds one vsock hop to `connect()` latency before the handshake's hops; and the agent
  needs the production service registry + health feed (from `service_backends`) rather
  than a flag-file/stub. Shape 2 assumes the module
  can rewrite the connect destination on the pinned 6.18 appliance kernel; proven here
  on the stock 7.0.0-29 test kernel (the ADR-0068 §4 pinned-kernel boot is still unrun,
  same caveat as Spike C).

## Gate recommendation

PROMOTE. D4 is settled by evidence reproduced across two boots: the host proxy's
service-name resolution and first-healthy LB can be relocated onto the in-guest
kernel-module path via the host agent's vsock channel, resolved at `connect()` time
with a destination rewrite, with mTLS, key custody, connect-time re-resolution
freshness, and policy-deny all intact. This removes the D4 blocker to deleting the
host proxy's resolution/LB job. Shape 1 is unnecessary for Linux VMs and carries a
health-staleness window Shape 2 does not.
