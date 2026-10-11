# V-28 findings: leg-S and VM probes over an owner-opened vsock flow

Probe increment: `spike-scratch/netns-density-295-v28-legs-vsock/` (raw evidence in `evidence/`). Question source: `.context/over-engineering-audit-review.md` finding 1; leg-S is `crates/overdrive-dataplane/src/mtls/inbound.rs` + `mtls/splice.rs`; ADR-0173 §3 (`TcpAccept`), ADR-0174. Recorded by the orchestrator from the spike agent's report (the agent's Write tool was refused).

## Verdict

| # | Question | Verdict |
|---|---|---|
| 1 | kTLS ↔ vsock STREAM forwarding, byte-exact, throughput | WORKS for correctness. Costs 12–37 % of bulk client-to-guest throughput and ~0.3 ms per small request vs loopback TCP. |
| 2a | Half-close, both directions | WORKS. 0 bad, 0 hang over 2,016 stress flows plus the single-flow matrix; same outcomes as the TCP baseline. |
| 2b | Abort: TLS-peer RST, guest-app RST | DOESN'T-WORK with the plain vsock socket API: a guest reset arrives as clean EOF, so the peer sees silent truncation (192/192). Works only with a discriminator: a tracepoint BPF classifier on `virtio_transport_recv_pkt`. |
| 2c | Guest app crash (SIGKILL), VMM death | WORKS, parity with TCP baseline, no hang. One client byte-counter anomaly unexplained (see Not run). |
| 3 | Peer address in `TcpAccept` | WORKS. Supply the guest gateway address with port 0 — a per-lease constant available at leg-S. |
| 4 | HTTP and TCP-connect probes | WORKS. Same answer as a direct probe in every state, including a guest app that listens but never accepts. |

**PIVOT.** The vsock path is correct for streams, half-close, probes and peer address. TCP reset fidelity is not carried by vsock STREAM, so keeping today's leg-S semantics needs either the tracepoint classifier on host and guest, or an in-band abort signal the design pins. Not "just splice kTLS to vsock".

## Substrate

- Native metal (AMD EPYC 8024P, `systemd-detect-virt` = none). Host and guest `uname -r` = `7.0.0-29-generic`.
- Vendored Cloud Hypervisor fork (binary pin `eadddf1bbcdc`, `--vsock cid=4280,backend=vhost-kernel`), real microVM.
- Guest configured as in ADR-0150: `10.77.0.2/32` on `lo`, fwmark rule + local table, `fwmark_reflect=1`, `tcp_fwmark_accept=1`.
- Relay is a leg-S stand-in: real kTLS 1.3 on leg C, `splice(kTLS-RX → pipe → vsock)` and mirror, one thread per direction, EOF forwarded as `shutdown(SHUT_WR)`.
- Guest side: `legs gagent` reads a 40-byte `TcpAccept`-shaped header, binds `IP_TRANSPARENT` to the requested peer, connects to the app, replies `P`, `R:errno` or `T:errno`, then splices.
- The "today" baseline is a loopback plaintext TCP app (best case for TCP), not the real route to the VM address.

Kernel facts (`vendors/linux` @ v7.0):
- kTLS RX has `splice_read`: `net/tls/tls_main.c:946,951`, `net/tls/tls_sw.c:2267`.
- vsock STREAM has no `splice_read`; `sock_splice_read` falls back to `copy_splice_read` (`net/socket.c:1129-1138`) — vsock→pipe is a kernel copy.
- pipe→vsock uses `splice_to_socket` (`fs/splice.c:795`) over `vsock_connectible_sendmsg` (`net/vmw_vsock/af_vsock.c:2624`).
- Established-state RST: `virtio_transport_common.c:1463` → `virtio_transport_do_close` (`:1244-1256`) sets `SOCK_DONE` and `peer_shutdown = SHUTDOWN_MASK` but never `sk_err`, so `read()` returns 0. Only a RST while connecting sets `ECONNRESET` (`:1345-1347`).
- `vsock_poll` exposes only `EPOLLRDHUP`/`EPOLLHUP` (`af_vsock.c:1247-1259`); half-close vs full close indistinguishable afterwards. The `virtio_transport_recv_pkt` tracepoint (`include/trace/events/vsock_virtio_transport_common.h:89`) is the only place ops and flags are visible.

## Q1 — correctness and throughput

Every bench rep checked a position-dependent pattern both directions: `fail = 0` in all 6 configurations at every size; `splice_einval` 0 in every `pump_end` event. Median per transfer, `evidence/bench-20261011T100813Z` (100–300 reps per cell, 5 ms pause). `vsock-app` = relay → vsock → guest agent → guest app.

| dir | size | tcp-splice | vsock-app-splice | vsock-app-copy | Δ splice vs tcp |
|---|---|---|---|---|---|
| up | 1 KiB | 0.147 ms | 0.446 ms | 0.438 ms | +0.30 ms |
| up | 64 KiB | 273 MB/s | 145 MB/s | 106 MB/s | −47 % |
| up | 1 MiB | 650 MB/s | 432 MB/s | 346 MB/s | −34 % |
| up | 64 MiB | 803 MB/s | 506 MB/s | 792 MB/s | −37 % |
| down | 1 KiB | 0.133 ms | 0.346 ms | 0.328 ms | +0.21 ms |
| down | 64 KiB | 504 MB/s | 158 MB/s | 196 MB/s | −69 % |
| down | 1 MiB | 1308 MB/s | 1276 MB/s | 1285 MB/s | −2 % |
| down | 64 MiB | 992 MB/s | 873 MB/s | 1064 MB/s | −12 % |

- Repeat `bench-20261011T103007Z` (1 MiB / 64 MiB, 100 reps): vsock-app-splice up 392 / 615 MB/s vs tcp-splice 579 / 908 MB/s (~−32 %). Ordering stable; 64 MiB absolutes vary 10–20 % between runs.
- Per-connection setup (`legs_open_us`): vsock 0.60–0.85 ms vs TCP 0.07–0.13 ms (~+0.55 ms).
- `splice` buys nothing on the vsock leg; `copy` equal or faster in most vsock cells.
- Not the production relay (no rustls handshake or `PumpHandle`): shape of the cost, not a budget.

## Q2 — half-close and abort

Evidence: single flow `half-20261011T085703Z`; stress `stress-20261011T115716Z` (tracepoint classifier), `-115752Z` (EOF always clean), `-115826Z` (zero-length-send probe). Each stress group 12 rounds × 16 parallel flows (192; 480 for the small group), TCP baseline alongside.

- **Half-close** (client FIN then reply, close_notify then FIN, app FIN first with client still sending, simultaneous, app close after reply, server speaks first): all byte-exact, identical to TCP. Stress: `client_bad` 0, `app_bad` 0, `hang_clients` 0 for every group with the classifier or EOF-always-clean. The zero-length-send classifier mis-classified clean half-closes (`client_bad` 192, 192, 480) — unusable.
- **Abort**, 192 flows each:

| scenario | TCP baseline | vsock, EOF always clean | vsock, tracepoint classifier |
|---|---|---|---|
| A1 TLS peer RST mid-upload: guest app sees | `ECONNRESET` 192 | clean `eof` 192 (truncation 192) | `ECONNRESET` 192 |
| A2 guest app RST mid-download: TLS client sees | `reset` 192 | clean `eof` 192 (truncation 192) | `reset` 192 |
| A2b guest app RST, client idle | `reset` 192 | `eof` 186, `reset` 6 (truncation 186) | `reset` 192 |
| A3 app `close()` with unread upload | full reply + EOF 192; upload error EPIPE 172, none 19, other 1 | same classes (EPIPE 54, none 137, other 1) | same classes (EPIPE 45, none 147) |

- Classifier is aya-rs (`bpf/src/main.rs`, userspace `probe/src/tp.rs`), runs on host and guest; the stock guest kernel allowed it.
- A2b with classifier: 5/192 refused at pairing with `R:104` (guest `connect()` got `ECONNRESET`); client still saw `reset`, not a hang. TCP baseline 0. Not investigated.
- A4/A5 guest app SIGKILL: vsock and TCP identical; client sees EOF on download within 0.07 s; relay upload ends `write_err:32` (A4) or `eof_clean` (A5); no hang.
- D1 VMM SIGKILL mid-upload (`death-20261011T115902Z`): client done 0.25 s after the kill, no hang; relay ended `write_err:104` and aborted both legs; a probe right after got `vsock_connect_err:19` (ENODEV), unhealthy in 0.04 ms.

## Q3 — peer address in `TcpAccept`

Evidence: `peer-20261011T091446Z`.

| header peer | app `getpeername` | matches today (ADR-0174)? |
|---|---|---|
| gateway `198.18.28.254:0` | `198.18.28.254:49425` (guest picks the port) | yes — platform client appears from the gateway |
| leg-C TLS peer `198.18.28.1:47371` | `198.18.28.1:47371` exactly | the "real peer" shape, not today's |
| none | `10.77.0.2:34784` (guest's own address) | no |

40 concurrent flows from one source IP with distinct ports: 40/40 ok, 40 distinct peers equal to the clients' local addresses. Keeping today's semantics: leg-S supplies the guest gateway with port 0 (per-lease constant). The leg-C peer is also available at leg-S (`getpeername`). Requires the ADR-0150 guest setup; without it the transparent bind fails.

## Q4 — probes over an owner-opened flow

Evidence: `probes-20261011T091742Z`.

| guest app state | TCP-connect probe | HTTP probe (2xx healthy) |
|---|---|---|
| HTTP 200 | healthy, 0.4 ms | healthy, 1.2 ms |
| HTTP 500 | healthy (connect only) | unhealthy |
| no listener | unhealthy `R:111`, 0.6 ms | unhealthy `R:111` |
| listens, never accepts, backlog 128 | healthy, 0.5 ms (guest kernel completes the handshake) | unhealthy after 2 s read timeout |
| listens, never accepts, backlog 1 | healthy, healthy, then `T:110` unhealthy at 1.50 s | n/a |
| accepts, never answers | healthy | unhealthy after 2 s |
| listener removed after healthy | `R:111` unhealthy | n/a |
| VMM dead | `vsock_connect_err:19` unhealthy, 0.04 ms | same |

A TCP-connect probe on a never-accepting app is healthy while the backlog has room and unhealthy once full — same as a direct TCP probe today. The host-side flow timeout must exceed the guest agent's connect timeout (3 s vs 1.5 s here).

## Design implications

1. Reset fidelity is the open design item. Pin one: (a) tracepoint-BPF classifier on host and guest (works; a kernel tracepoint program in both data paths), or (b) an explicit owner↔guest-agent abort signal (not tested). Without either, a reset becomes clean EOF and the peer sees truncation.
2. No zero-copy on the vsock leg; `splice` helps only on the kTLS side.
3. `TcpAccept` for leg-S carries the gateway address with port 0; the pairing reply carries paired / refused(errno) / timeout(errno), which makes probes correct.
4. Owner-side flow timeout > guest agent `connect()` timeout.
5. Budget ~+0.5 ms per connection and 12–37 % less client-to-guest bulk throughput vs loopback TCP.

## Not run

- Bench with the classifier attached — all throughput numbers are classifier-off (metal lease held by v31 at the time).
- Death-stage re-run with the instrumented client. In D1, A4 and the TCP-baseline A4 the client reports `up_sent` equal to the full 64–256 MiB with `up_err = 0` although the relay forwarded only 2.7–3.0 MB; it appears in the TCP baseline too, so vsock-vs-TCP parity stands, but that counter is unverified.
- The out-of-band abort signal (implication 1b).
- The production relay and the real route to the VM address (loopback TCP stands in).

Cleanup: no v28 processes, VMs, netns or cgroups remain on the host; `/tmp/v28-work` keeps built binaries and sources for the two missing runs. Nothing under `crates/`, `specs/` or `tests/` touched.
