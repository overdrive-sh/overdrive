# increment-h — Spike C RE-RUN: in-guest transparent mTLS as an out-of-tree LKM in a STOCK Linux guest on Firecracker, production-faithful TLS + full run capture

Overdrive GH #303 (`in-guest-kernel-mtls`). THROWAWAY PROBE CODE. Nothing here
ships; nothing builds or gates on it; it is deleted when the implementation it
validates lands.

## Why a re-run of increment-g

increment-g already proved the mechanism: an out-of-tree `.ko` on a STOCK Linux
guest kernel hooks `tcp_prot` connect/sendmsg/recvmsg, HOLDs the app's first
I/O, relays the TLS 1.3 handshake over vsock to the host rustls relay (which
holds the SVID key and decides policy), installs the returned session keys into
the kernel's kTLS ULP on the workload's OWN socket, and steps out of the data
path. The module ships NO record layer, NO crypto, NO KeyUpdate code — the
kernel carries TLS. increment-g's module (`module/igkmh_mtls.c`, unchanged here
except the log prefix) is reused verbatim.

Two things change in this re-run:

### Change 1 — production-faithful TLS config (no post-handshake control records)

increment-g found that with the host peers at rustls' DEFAULT
`send_tls13_tickets=2`, the post-handshake NewSessionTicket records land on the
workload's own kTLS socket and an unmodified plain `read()` gets `EIO` (only a
kTLS-aware `recvmsg`+cmsg reader skips them). In production Overdrive controls
BOTH mesh agents, so it configures the handshake to emit none of these. Here:

- every clean mesh `peer` TLS listener sets `ServerConfig.send_tls13_tickets = 0`
  and never calls `refresh_traffic_keys()` (no KeyUpdate); the `relay`'s server
  side already set `send_tls13_tickets = 0`. (`host/src/bin/peer.rs`,
  `host/src/bin/relay.rs`.)
- the payoff: an **unmodified plain blocking `read()`/`write()`** app
  (`app/igkmh_app.c` modes `echo`/`serverfirst`/`twostep`/`passthrough`, NOT
  kTLS-aware, no `recvmsg`+cmsg, no ticket draining) exchanges byte-exact over
  the mesh with no `EIO`.
- WHY (not luck): one contrast port `:6452` keeps rustls' default
  `send_tls13_tickets=2`; a plain `read()` there still hits the control record
  and returns `EIO`, while the `drain` mode (`recvmsg`+cmsg) skips the two NSTs.

### Change 2 — capture every run (like increments a–f)

increment-g did not persist its run logs; this re-run routes EVERY
`cargo xtask metal run` through `capture.sh`, producing append-only
`runs/NNNN.{meta,stdout,stderr}` with the metal target/host/login-home redacted.
The boot run embeds the tap `.pcap` as base64; `capture.sh` decodes it into
`runs/NNNN.tap.pcap` and records its sha256 in the `.meta`.

## Hard constraints (see `.claude/rules/spike.md`)

- GUEST kernel is STOCK and UNMODIFIED (Firecracker boots the host's own
  `/boot/vmlinuz-<rel>` directly). No kernel patch/rebuild. If the mechanism
  genuinely needs a kernel change, that is the finding — report it.
- HOST kernel is never touched. The `.ko` is `insmod`'d only inside the
  disposable Firecracker guest. No `insmod`/`modprobe`/sysctl/kernel install on
  the metal host.
- Build as the LOGIN USER (`--no-sudo`); boot as root. Build outputs live in the
  UNSYNCED scratch tree `~/igkm-spike-a/igkmh/` and `~/igkm-spike-a/igkmh-host-target/`,
  never the rsync'd repo tree, so they can't break a later rsync.
- One uniquely named tap per boot (`igkh<...>`); delete it after. Clean up every
  Firecracker/tap/socket/listener/run-dir; prove it with a final host-state-check.

## Run plan (all via capture.sh, from the repo root)

```
bash .../increment-h/capture.sh 0001 inventory        --no-sudo -- bash .../increment-h/host-inventory.sh
bash .../increment-h/capture.sh 0002 build-all        --no-sudo -- bash .../increment-h/build-all.sh
bash .../increment-h/capture.sh 0003 mesh-run                   -- bash .../increment-h/run-mesh-fc.sh   # root
bash .../increment-h/capture.sh 0004 mesh-run-repro             -- bash .../increment-h/run-mesh-fc.sh   # reproduction
bash .../increment-h/capture.sh 0005 host-state-check           -- bash .../increment-h/host-state-check.sh
```

## Vsock relay protocol (SSOT: `host/src/lib.rs`, unchanged from increments e–g)

```
0x01 OPEN  guest->host  family·ipv4·port     0x05 SECRETS   host->guest  suite·{seq,secret,key,iv}×(tx,rx)·peer_id
0x02 TO_PEER host->guest TLS bytes to peer    0x06 DENY      host->guest  reason
0x03 NEED_PEER host->guest read one record    0x07 ERROR     host->guest  reason
0x04 FROM_PEER guest->host bytes from peer     0x08 ACCEPT    guest->host  (inbound; stretch only)
```
The SVID private key never crosses vsock; SECRETS carries only per-direction
TLS 1.3 traffic secret + key + IV.
