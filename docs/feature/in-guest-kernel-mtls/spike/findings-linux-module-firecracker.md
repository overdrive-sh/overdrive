# Spike C findings — in-guest transparent mTLS as an out-of-tree loadable kernel module in a STOCK Linux guest on Firecracker (production-faithful TLS)

GH #303 (`in-guest-kernel-mtls`). PROBE phase, throwaway. Probe:
`spike-scratch/in-guest-kernel-mtls/increment-h/`. Evidence:
`increment-h/runs/0001..0005.{meta,stdout,stderr}` plus `runs/0003.tap.pcap` and
`runs/0004.tap.pcap` (committed, append-only, host identifiers redacted).

The probe agent could not write under `docs/`, so the orchestrator wrote this file from
its report and re-checked the headline lines (plain `read()` byte-exact, the tickets=2
`EIO` contrast, the wire scan, `TlsTxSw`, relay-kill survival, custody, the host-clean
verdict, and the `0003.meta`) against the committed `runs/` captures.

A first attempt (`increment-g`) proved the mechanism but did not persist its run logs, so
it could not be verified; it was dropped and re-run as `increment-h` with full capture and
production-faithful TLS. This file is the verified re-run.

## Verdict: WORKS

An out-of-tree `.ko`, `insmod`'d into a STOCK, UNMODIFIED Linux guest kernel on
Firecracker, hooks `tcp_prot`, holds the workload's first I/O, relays the TLS 1.3
handshake over vsock to the host rustls agent (which holds the SVID key and decides
policy), installs the session keys into the kernel's kTLS ULP on the workload's OWN
socket, and steps out of the data path. **With the mesh configured production-faithfully
(zero TLS 1.3 session tickets, no KeyUpdate), an unmodified plain blocking `read()` /
`write()` application exchanges data byte-exact over the mesh with no `EIO`** — the result
the re-run set out to prove. Both boot runs (0003, 0004) gave identical verdicts.

**No kernel patch.** The guest runs an off-the-shelf bzImage; the mechanism is a loadable
module built out-of-tree against that kernel's headers. This is the whole point versus the
Unikraft spikes: the Linux guest has kTLS, so the kernel carries the record layer and the
module ships no crypto.

## Substrate + versions (run 0001, `runs/0001.stdout`)

- Metal host: AMD EPYC 8024P, x86_64, `systemd-detect-virt=none`, `/dev/kvm`
  `KVM_GET_API_VERSION=12 KVM_CREATE_VM=ok` → SUBSTRATE OK (fail-closed per testing.md).
- Host kernel `uname -r` = `7.0.0-29-generic` (Ubuntu 7.0.0-29.29).
- **Guest kernel `uname -r` = `7.0.0-29-generic`** — the STOCK kernel is the host's own
  `/boot/vmlinuz-7.0.0-29-generic` (a bzImage), which Firecracker v1.17.0 boots directly.
  No kernel patch, no rebuild, no custom image (Gate A).
- Gate B: `/lib/modules/7.0.0-29-generic/build` has `Makefile` + `Module.symvers` +
  `.config`; the OOT `.ko` builds with vermagic `7.0.0-29-generic SMP preempt mod_unload
  modversions`, matching the guest.
- VMM: Firecracker v1.17.0, sha256 `99ad0f5cd0514a88aad0e9ae8cfdb3cc3b4ab9d190e1194602406c786b5de7a5`.
- gcc `(Ubuntu 15.2.0-16ubuntu1) 15.2.0`; rustc/cargo `1.95.0`.
- Module source: `module/igkmh_mtls.c` = 523 lines; built `.ko` = 760,232 B. Reused
  verbatim from the first attempt except the log prefix (`IGKMG:` → `IGKMH:`).

## The two changes this re-run made

### Change 1 — production-faithful TLS (no post-handshake control records)

In production Overdrive controls BOTH mesh agents, so it configures the handshake to emit
no post-handshake control records. The spike mimics that:
- Every host rustls `peer`'s clean mesh listener and the `relay`'s server side set
  `ServerConfig.send_tls13_tickets = 0` and never call `refresh_traffic_keys()` (no
  KeyUpdate). Verified at listen time: `PEER listening tls …:6443 … send_tls13_tickets=0
  (production-faithful: no NewSessionTicket)`.
- The payoff is proven by an UNMODIFIED plain `read()` / `write()` app (`app/igkmh_app.c`,
  modes `echo` / `serverfirst` / `twostep` / `passthrough` — plain blocking `read()`, NOT
  kTLS-aware, no `recvmsg`+cmsg, no ticket draining).
- The WHY (not luck) is pinned by one contrast port `:6452` left at rustls' default
  `send_tls13_tickets=2`: there a plain `read()` still hits the kTLS control record and
  returns `EIO`, while a `recvmsg`+cmsg `drain` reader skips the NewSessionTicket. The
  module's `HANDSHAKE DONE` line is identical on the tickets=0 and tickets=2 ports, so the
  `EIO` is purely post-handshake control-record delivery — a mesh-config property, not a
  module difference.

### Change 2 — every run captured

Every `cargo xtask metal run` went through `increment-h/capture.sh`, producing
append-only `runs/NNNN.{meta,stdout,stderr}` with the metal target / host / login-home
redacted (verified: no provider IP, no full target, no `/home/<user>` in any `runs/`
file). The boot runs embed the tap `.pcap` as `runs/NNNN.tap.pcap`, sha256 in the `.meta`
(0003 `tap_pcap_sha256=5547963272…`, confirmed to match the file).

## Module design (hook / hold / kTLS install)

`tcp_prot.connect` is invoked by `inet_stream_connect` with `lock_sock(sk)` HELD, so the
`connect` hook only records the mesh destination for `sk` — it cannot do socket I/O there
(that re-locks `sk` and deadlocks; observed as a hang in the first attempt). The HOLD +
handshake run lazily in the first `sendmsg` / `recvmsg`, which the kernel calls WITHOUT the
socket lock. This is the Camblet shape. There the module:

1. opens a kernel `AF_VSOCK` socket to the host relay (CID 2, port 7100 → host
   `<uds>_7100`) and relays the TLS 1.3 handshake: `OPEN` → (`TO_PEER` / `NEED_PEER` /
   `FROM_PEER`, one TLS record per `NEED_PEER`) → `SECRETS` | `DENY`. Raw TCP I/O on the
   workload `sk` during the handshake calls the saved original `sendmsg` / `recvmsg`
   directly, bypassing its own hook; vsock uses a separate socket.
2. installs kTLS. Ordering (load-bearing): ULP first, then keys —
   `lock_sock(sk); tcp_set_ulp(sk,"tls"); release_sock(sk);` then
   `setsockopt(sk, SOL_TLS, TLS_TX, KERNEL_SOCKPTR(&crypto_info))` then `TLS_RX`. The
   SECRETS → `tls12_crypto_info_aes_gcm_128` mapping sets `key`, `salt`=iv[0:4],
   `iv`=iv[4:12], `rec_seq`=seq, `version=TLS_1_3`.
3. steps out: the kernel's kTLS carries the AES-128-GCM record layer on the workload's own
   socket; the module forwards the held app I/O to the now-kTLS proto. On `DENY` it
   `shutdown()`s the socket and the first I/O returns `-EACCES`. **The module ships no
   record layer, no crypto, no KeyUpdate code** — the kernel does all of it.

The SVID private key never crosses vsock; `SECRETS` carries only the per-direction TLS 1.3
traffic secret + key + IV.

## Checklist — binary verdicts (each tied to the run that evidences it)

Runs 0003 (`mesh-run`) and 0004 (`mesh-run-repro`) are the boot runs; verdicts were
identical across both. Byte counts cited from 0003.

| # | Item | Verdict | Evidence (runs/0003 unless noted) |
|---|------|---------|-----------------------------------|
| 1 | Module loads in stock guest; `connect()` to mesh dst triggers vsock relay; host logs client SPIFFE ID from host-held cert | PASS | `IGKMH: [c1] HOLD on first I/O … relaying TLS handshake over vsock`; relay `handshake complete: policy ALLOW local spiffe://…/guest-client <-> peer spiffe://…/peer-allowed; suite=TLS13_AES_128_GCM_SHA256` |
| 2 | tcpdump only TLS 1.3 records; `tls` ULP in guest; mesh plaintext 0× on wire; passthrough plaintext (positive control) | PASS | `SCAN tls streams: 10514 bytes, marker(IGKM-H-) occurrences=0`; `unframed_bytes=0` on all 12 TLS directions; wire string scan `IGKM-H-REQ-PT=1 RESP-PT=1`, every mesh string `=0`; `/proc/net/tls_stat TlsTxSw=5 TlsRxSw=5` |
| 3 | Unmodified plain `read()`/`write()` byte-exact over mesh, no `EIO`; client-first REQ right after `connect()` byte-exact (lossless hold) | PASS (headline) | `:6443` tickets=0: `write(33) rc=33: IGKM-H-REQ-e1…` then `echo read(34) PLAINTEXT: IGKM-H-RESP-e1 peer->guest line 1` |
| 4 | Server-speaks-first with a plain `read()` (failed in the first attempt; passes with tickets off) | PASS | `:6445` tickets=0: `greeting read(41) PLAINTEXT: IGKM-H-GREETING…`, REQ, `sf-resp read(45) PLAINTEXT: IGKM-H-RESP-SF-sf…` |
| 5 | Relay killed after handshake → second exchange on same connection still works | PASS | `:6450`: `exch1 read PLAINTEXT…`, `PAUSE-FOR-RELAY-KILL`, runner `SIGKILL relay … unix listeners now: []`, `exch2 read(57) PLAINTEXT: IGKM-H-RESP-B-ts … exchange-2-after-relay-gone` |
| 6 | Deny → first mesh I/O `EACCES`, no application bytes on the wire | PASS | `:6444`: `DENY from relay: policy deny … -> first I/O returns -EACCES`; app `DENY-CASE OK: first write refused errno=13 (Permission denied); no application bytes on the wire`; relay custody secrets `[]` |
| 7 | Pass-through (non-mesh) unaffected, plaintext | PASS | `:5001`: `write(39)…IGKM-H-REQ-PT-pt`, `pt read(40) PLAINTEXT: IGKM-H-RESP-PT-pt`; cleartext on the wire (positive control) |
| 8 | Custody: vsock carries only handshake records + traffic secrets; SVID private key 0 occurrences | PASS | all 6 relay connections: `client SVID key occurrences PKCS#8 DER=0 PEM=0 private scalar=0; guest-server SVID key … =0; application traffic secrets sent … = [1,1]` (ALLOW) / `[]` (DENY) |
| 9 | kTLS ordering (ULP before TLS_TX/RX) | PASS | module `install_ktls`: `tcp_set_ulp(sk,"tls")` → `TLS_TX` → `TLS_RX`; `HANDSHAKE DONE … kTLS installed`; `TlsTxSw=5 TlsRxSw=5`. (The first attempt measured `TLS_TX` before ULP → `-92 ENOPROTOOPT`.) |
| — | Host kernel untouched / no leaks (cleanup) | PASS | run 0005: `igkmh modules on host: []`, `host tainted? 0`, no firecracker/peer/relay/tcpdump procs, no `igkh*` taps, no `192.168.204.x`, no run dirs → `VERDICT: host clean = YES`. Per-run 0003/0004 cleanup also deleted their tap. |

### Contrast sub-case (the WHY — run 0003, `:6452` tickets=2)

- Plain `read()`: `IGKMH: [c4] HANDSHAKE DONE … kTLS installed` then `echo read FAILED
  errno=5 (Input/output error)`.
- `recvmsg`+cmsg `drain`: `drain recvmsg skipped a CONTROL record type=22 (162 bytes)
  [NewSessionTicket=22]` then `drain recvmsg(36) APP-DATA (drained tickets):
  IGKM-H-RESP-DR-dr peer->guest drain`.

Turning tickets off on both agents (which production does) makes the workload's kTLS
socket carry `application_data` only, and the unmodified plain `read()` works.

## Edge cases / observations

- **`sockptr.h:49` fortify `WARN_ONCE` (expected, non-fatal).** On the first `TLS_TX`
  install the kernel logs `memcpy: detected field-spanning write (size 36) … at
  include/linux/sockptr.h:49` / `WARNING … do_tls_setsockopt_conf`. It is `WARN_ONCE`
  (one per boot) and non-fatal: kTLS installed on all 5 allowed sockets and data flowed
  byte-exact. It comes from passing a `KERNEL_SOCKPTR` to the TLS setsockopt path, where
  fortify-source sees the 36-byte `tls12_crypto_info_aes_gcm_128` copy against a 0-sized
  field view. A production module must install kTLS via a path that does not trip fortify,
  or suppress the warning deliberately.
- **kTLS is software kTLS** (`TlsTxSw`/`TlsRxSw`, `TlsTxDevice=0`). NIC offload does not
  apply: Linux offloads TLS 1.2 only, the mesh is TLS 1.3 (CLAUDE.md).
- busybox `ss` has no `-K`; the in-guest ULP evidence is `/proc/net/tls_stat`. A
  production node with full iproute2 would also show the `tls` ULP via `ss -K`.
- Each handshake relayed in `TO_PEER=3 NEED_PEER=3` records; relay host-side self-check
  `HKDF(secret)==rustls key/iv tx=true rx=true`, `tx_seq=0 rx_seq=0
  buffered_plaintext=false`.
- Deny fails the first I/O, not `connect()`, because the handshake is lazy (forced by the
  `connect`-under-lock constraint). Zero application bytes cross. This is the Camblet
  property.

## Design implications for #303

- **Host-proxy comparison.** This is the in-guest kernel-mediated alternative to a
  host-side proxy: the workload opens an ordinary socket; the module holds the first I/O,
  has the host agent run the mTLS handshake (SVID key stays on the host), and installs the
  keys into the guest kernel's kTLS so the kernel — not a userspace proxy, not the
  workload — carries the record layer. No per-connection userspace copy after install, and
  the agent is out of the path and can die without breaking established connections
  (item 5). It needs a kTLS-capable guest kernel (`CONFIG_TLS`) and `AF_VSOCK` to the host
  agent.
- **The mesh MUST configure both agents for zero tickets / no KeyUpdate** (Change 1). This
  is a control-plane configuration invariant for the in-guest-kTLS design, not optional
  tuning — otherwise unmodified workloads doing plain `read()` get `EIO`. It is also the
  main difference from the Unikraft path, whose userspace record layer could swallow
  tickets and run KeyUpdate itself.
- **What a production module still needs** (beyond this proven outbound/client slice):
  - the `accept()` / inbound path (hook `tcp_prot.accept` / `inet_csk_accept`; the relay
    `ACCEPT` frame exists but was not exercised here) — server-side termination in the
    guest kernel
  - non-blocking / epoll sockets — this probe used blocking sockets; the hold must
    integrate with `O_NONBLOCK` / epoll readiness without spurious `EAGAIN` (the Unikraft
    B2 spike characterised this for the userspace variant; the kTLS path must be
    re-characterised)
  - IPv6 — hook `tcpv6_prot` as well as `tcp_prot`
  - robust per-socket teardown and a safe `rmmod` under concurrency — the probe swaps
    `tcp_prot` function pointers globally with a static `NENT=64` table; production needs
    real per-socket lifecycle and an `rmmod` that cannot race in-flight holds
  - resolve the `sockptr.h:49` fortify path on kernel-context kTLS install
- **Key custody holds:** only per-direction traffic secret + key + IV cross vsock; the SVID
  private key never does (0 occurrences across all runs).

## Gate recommendation

PROMOTE the in-guest-kTLS-module mechanism as a viable candidate for #303: the outbound /
client slice is proven end-to-end on a stock, unmodified Linux kernel with
production-faithful TLS. The `accept()` / inbound path, non-blocking / epoll behaviour,
IPv6, safe teardown, and the `sockptr.h:49` kTLS-install path are the open items a
production module must close.
