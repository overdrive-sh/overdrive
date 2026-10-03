# increment-i — Spike D: move service-name resolution + first-healthy backend selection onto the host-agent vsock channel (in-guest kernel-module path, Firecracker)

Overdrive GH #303 (`in-guest-kernel-mtls`), decision **D4** ("Choose where
service-name resolution and service load balancing live under this model").
THROWAWAY PROBE CODE. Nothing here ships; nothing builds or gates on it; it is
deleted when the implementation it validates lands.

## What this spike proves

Spike C (increment-h) proved the in-guest-kTLS module for a FIXED mesh
destination. Spike D adds the one L4 job that lives in the host proxy's capture
path today and must be re-homed once the proxy is deleted (ADR-0072
`MtlsResolve`): **service-name resolution + first-healthy backend selection**,
moved onto the SAME host-agent vsock channel the mTLS mechanism already uses.

The new loop no earlier spike exercised: **guest dials a stable VIP → the guest
module asks the host agent over vsock → the agent picks a first-healthy backend
from a backend set → the guest's TCP connection lands on THAT backend and
exchanges byte-exact → when a backend goes unhealthy, the next connect moves to
a healthy one.** The SVID key still never crosses vsock; the mTLS handshake runs
exactly as in Spike C.

## Shape 2 (resolve at connect time) — the shape this probe leads with

The guest app connects to a stable VIP (an address, not a backend). The module's
`connect` hook:

1. recognises the VIP (the `mesh=` module param lists the VIPs), and sends a
   `RESOLVE{vip}` frame to the agent over a **separate** vsock socket — this
   runs UNDER `lock_sock(sk)` (the connect hook is called with sk locked), which
   is safe because vsock is a different socket; the only thing forbidden under
   the sk lock is raw I/O on sk itself;
2. receives `RESOLVED{backend_addr, expected_peer}` (or `DENY`);
3. **rewrites the connecting socket's destination sockaddr** to the chosen
   backend BEFORE calling the original `tcp_prot.connect` — the in-guest
   equivalent of the host's old `cgroup_connect4` / XDP destination rewrite;
4. the lossless HOLD + mTLS handshake + kTLS install then run lazily on the
   first I/O exactly as Spike C, now to the chosen backend, and the module
   checks the handshake's verified peer SPIFFE ID matches what RESOLVE promised.

A policy-denied service returns `DENY` at resolve, so the `connect()` itself
fails with `-EACCES` and no TCP SYN is ever sent.

Shape 1 (resolve at name-lookup time, no connect-time rewrite) is the fallback
if Shape 2's connect-hook resolve cannot be done from a module; it is only built
if Shape 2 fails.

## New vsock frames (added to the Spike C protocol in `host/src/lib.rs`)

```
0x09 RESOLVE   guest->host  family·vip_ip[4]·vip_port:u16 BE                 (7 B, same shape as OPEN)
0x0a RESOLVED  host->guest  family·backend_ip[4]·backend_port:u16 BE·peer_id_len:u16 BE·peer_id
     (DENY 0x06 reused for a policy-denied service; ERROR 0x07 for no-healthy-backend)
```
A RESOLVE carries only a VIP in and a backend addr + an expected-peer SPIFFE-ID
string back — no key material in either direction.

## Backend set / health (agent side, mirrors `service_backends`)

Two host-side TLS backends for one service `svc-a`, both presenting the service
identity `peer-allowed` (a service's instances share the service SPIFFE ID) on
`192.168.204.1` (the cert's IP SAN) at distinct ports — the serving port is the
distinguishing evidence, and each backend's echo embeds `from-<label>`:

```
  A = 192.168.204.1:6443   (Ord-first)
  B = 192.168.204.1:6444
```
The agent selects **first-healthy by `Ord`** over the set (exactly `MtlsResolve`).
Health is a flag file per backend (`health/<ip>_<port>`), read fresh on every
resolve — the test drives it by removing backend A's flag between the two
connects, so connect-2 re-resolves to B.

`svc-denied` (VIP `10.80.0.9:9443`) is a policy-denied service: the agent returns
DENY at resolve.

## Run plan / what actually ran (all via capture.sh, from the repo root; append-only)

```
0001 inventory         --no-sudo  host-inventory.sh       substrate OK, reusable scratch present, host clean
0002 build-all         --no-sudo  build-all.sh            built (2 cosmetic cc warnings in the module)
0003 build-all-clean   --no-sudo  build-all.sh            rebuilt clean after fixing the 2 warnings
0004 mesh-run          (root)     run-mesh-fc.sh          RESOLVE+DENY worked; all TCP failed -- runner bug: the
                                                           "create tap" section had been dropped, so 192.168.204.1
                                                           was unassigned (peer EADDRNOTAVAIL, guest EHOSTUNREACH).
                                                           vsock is tap-independent -> isolates it as a runner bug.
0005 mesh-run          (root)     run-mesh-fc.sh          tap restored: full loop GREEN; connect-2(B) wire tail lost
                                                           to the guest's hard reboot flushing the NIC too soon.
0006 build-all-retimed --no-sudo  build-all.sh            rebuilt: app pause 6s->3s + 2s settle after connect-2.
0007 mesh-run          (root)     run-mesh-fc.sh          COMPLETE GREEN -- both backends A(6443)/B(6444) fully on
                                                           the wire, VIP never dialed, mTLS/custody/freshness/deny.
0008 mesh-run-repro    (root)     run-mesh-fc.sh          reproduces 0007 identically.
0009 host-state-check  --no-sudo  host-state-check.sh     VERDICT host clean = YES.
```
The authoritative evidence runs are **0007** (mesh-run) and **0008** (mesh-run-repro).

## Pass log (hypothesis / prediction / falsification — spike.md §4/§10)

### Boot pass (runs 0004 mesh-run, 0005 mesh-run-repro) — Shape 2

- **Hypothesis.** A guest kernel module can, from inside the `connect()` hook
  (called under `lock_sock(sk)`), do a RESOLVE round trip on a *separate* vsock
  socket, rewrite the connecting socket's destination sockaddr to the
  agent-chosen backend, and let the original `tcp_prot.connect` dial it — so the
  connection lands on the chosen backend, mTLS holds, a fresh connect after a
  health change re-resolves to the healthy backend, and a policy-denied service
  fails `connect()` with no TCP.
- **Predicted outcome.**
  - Serial: `IGKMD: [resolve] vip 10.80.0.1:9443 -> asking agent …`, `agent chose
    backend 192.168.204.1:6443 expected_peer=…/peer-allowed; rewriting connect dst`,
    `connect VIP 10.80.0.1:9443 returned 0`; `lb-c1 read … from-A`; then
    `PAUSE-FOR-HEALTH-TOGGLE`; connect-2 resolves to `192.168.204.1:6444`,
    `lb-c2 read … from-B`; deny VIP → `[resolve] … DENY … -EACCES` and
    `connect VIP 10.80.0.9:9443 FAILED errno=13`. `HANDSHAKE DONE` +
    `RESOLVE-MATCH … match=1` on both allowed connects; `/proc/net/tls_stat`
    TxSw/RxSw nonzero.
  - Relay: `rx RESOLVE vip=10.80.0.1:9443` → `chose 192.168.204.1:6443 (first-healthy)`;
    after toggle → `chose 192.168.204.1:6444`; `svc-denied … DENY`. Handshakes ALLOW
    peer-allowed. Custody: SVID key occurrences 0.
  - Peer: backend `A:6443` serves connect-1; `B:6444` serves connect-2.
  - Wire: 0 packets to any `10.80.0.x` VIP; packets to `:6443` and `:6444`; every
    mesh plaintext marker 0×; `:5001` PT markers ≥1; pcap_scan TLS records only,
    `unframed_bytes=0`.
- **Falsification.** `connect()` hangs (a vsock-under-sk-lock deadlock) → boot
  times out → Shape 2 is infeasible from a module → fall back to Shape 1. Or
  connect-2 still lands on A (health ignored/stale) → freshness fails. Or mesh
  plaintext on the wire → mTLS broken. Or an SVID key appears on vsock → custody
  broken.
- **ACTUAL (runs 0007 / 0008, identical).** Hypothesis CONFIRMED, nothing
  falsified. `connect()` did NOT hang — the vsock RESOLVE round trip ran fine
  under `lock_sock(sk)`. Serial: `[resolve] … agent chose backend
  192.168.204.1:6443 … rewriting connect dst`, `connect VIP 10.80.0.1:9443
  returned 0`, `lb-c1 read … from-A`; after the toggle `[resolve] … chose
  192.168.204.1:6444`, `lb-c2 read … from-B`; deny VIP → `connect … FAILED
  errno=13`. `RESOLVE-MATCH … match=1` on both. Wire: 10 pkts to :6443, 9 to
  :6444, **0 to any 10.80.0.x VIP**; every TLS stream `unframed_bytes=0`,
  `marker(IGKM-D-) occurrences=0`; `:5001` plaintext present. Custody: SVID key
  0× on vsock. `TlsTxSw=2 TlsRxSw=2`.

## Hard constraints (see `.claude/rules/spike.md`)

- GUEST kernel is STOCK and UNMODIFIED (Firecracker boots the host's own
  `/boot/vmlinuz-<rel>` directly). No kernel patch/rebuild.
- HOST kernel is never touched. The `.ko` is `insmod`'d only inside the
  disposable Firecracker guest.
- Build as the LOGIN USER (`--no-sudo`); boot as root. Build outputs live in the
  UNSYNCED scratch tree `~/igkm-spike-a/igkmd/` and
  `~/igkm-spike-a/igkmd-host-target/` (CARGO_HOME, Firecracker and the stock
  kernel are reused from the Spike C scratch `~/igkm-spike-a/`), never the
  rsync'd repo tree.
- One uniquely named tap per boot (`igkd<...>`); delete it after. Clean up every
  Firecracker/tap/socket/listener/run-dir/health-dir; prove it with a final
  host-state-check.
