# V-13 — `SO_LINGER{1,0}` on owner-held sockets (H-5, G-CRASH, A-15, K-D5)

**Verdict: FALSIFIED as stated ("reset everywhere").**
- **HOLDS** where the far end belongs to the **dying** owner: its `SO_LINGER{1,0}` socket resets, the observer sees `ECONNRESET`, never EOF (110/110 per cell, mid-transfer; kTLS 20/20).
- **FALSIFIED** where the far end belongs to the **surviving** owner: the survivor sees the dead owner's flow vsock shut (`EPOLLRDHUP`) first, treats it as a clean half-close, drains and sends a FIN; the abort on control-session loss (D20, ADR-0166) arrives later and its RST is too late — the observer has already read EOF.

Probe and raw captures: `spike-scratch/netns-density-295-v13/` (`run.py`, `launch.py`, `build.sh`, `ktls_holder.py`, `vfwd/`, `bpf/`, `analyze.py` → `analysis.txt`, `evidence/<stage>-<stamp>/`, `out/*.stdout`).

## Setup

- Native x86_64 metal (AMD EPYC 8024P), fail-closed probes passed every run. `7.0.0-29-generic` host and guest (not the 6.18 pin). CH fork `41a619d19` (`overdrive/vhost-kernel-vsock`), `--vsock cid=3280,backend=vhost-kernel`, binary from the v24 build (sha256 `eadddf1b…`). Superproject HEAD `f6157044f`.
- Owners: `vfwd guest` / `vfwd host` stand-ins (copied from `netns-density-295-guest-vsock-capture/`), pairing in-kernel with `SockHash` + `SK_SKB` + `sock_ops` (aya-rs). Control-session loss aborts every flow on the survivor. Half-close: `EPOLLRDHUP` → wait for drain → `shutdown_write` on the partner.
- `SO_LINGER{1,0}` set at creation (read-back asserted) on guest intake child A_g, host intake child A, guest C, host D, leg-F's remote socket R; cleared before a clean close. `--linger no` is the negative control. Leg-F stand-in has no TLS and no FIN propagation.
- Remotes in netns `v13-rem` behind a veth (172.29.131.0/29). Observers are single-threaded AF_INET streamers; verdict = first terminal event; a trial counts only if ≥ 200 kB moved each way before the kill. Fresh owners/observers/VM state per trial. Vsock ports moved to 13341–13343 / 13350 after a clash with another spike.

## Verdicts (N = 110 per observer, three pooled baseline runs)

| # | Observer | Owner killed | Whose socket resets it | RESET | EOF-class | Verdict |
|---|---|---|---|---|---|---|
| 1 | guest app, outbound non-mesh | guest | dying (A_g) | 110 | 0 | HOLDS |
| 2 | guest app, outbound mesh | guest | dying (A_g) | 110 | 0 | HOLDS |
| 3 | guest app, inbound | guest | dying (C) | 110 | 0 | HOLDS |
| 4 | non-mesh remote destination | host | dying (D) | 110 | 0 | HOLDS |
| 5 | mesh remote peer | host | dying (R) | 110 | 0 | HOLDS (kTLS 20/20) |
| 6 | inbound external client | host | dying (A) | 110 | 0 | HOLDS |
| 7 | non-mesh remote destination | guest | survivor (D, abort) | 12 | 98 | **FALSIFIED** |
| 8 | inbound external client | guest | survivor (A, abort) | 7 | 103 | **FALSIFIED** |
| 9 | guest app, outbound non-mesh | host | survivor (A_g, abort) | 106 | 4 | **FALSIFIED** |
| 10 | guest app, outbound mesh | host | survivor (A_g, abort) | 106 | 4 | **FALSIFIED** |
| 11 | guest app, inbound | host | survivor (C, abort) | 61 | 49 | **FALSIFIED** |
| 12 | mesh remote peer | guest | survivor (R, abort) | 110 | 0 | **INCONCLUSIVE** (stand-in leg-F lacks half-close) |

Both owners killed (10 trials each order, 0 ms and 40 ms apart): the split matches the first kill alone — the survivor's FIN is out 2–10 ms after the first kill.

Row 7, retained trial (`evidence/matrix-20261009T011951Z`, `out_nonmesh-killG`, trial 1):
```
remote_nonmesh: {"kind":"rx_eof", ...}   +2.01 ms after the kill   (first event: EOF)
                {"kind":"tx_err","errno":32,"name":"EPIPE"}         later
host owner:  +1.85 leg_hup {flow 1, vsock true, rdhup true, hup false, mask 8192}
             +2.01 drain_shutdown_write {flow 1}                    FIN to the remote
             +11.41 ctl_closed
             +11.55 flow_closed {reset true, why "ctl_lost"}        RST, 9.4 ms after the flow RDHUP
```

## Cause: two notifications of one death, flow `RDHUP` always first

| Survivor | Flow `RDHUP` → control loss | Flow `RDHUP` first |
|---|---|---|
| host owner (guest killed) | p50 9.1 ms (7.3–19.9) outbound; p50 5.5 ms (0.2–9.6) inbound | 208/208 |
| guest owner (host killed) | p50 58 µs (max 201 µs) outbound; p50 129 µs (max 2.9 ms) inbound | 207/207 |

Drain completes in 0.1–0.2 ms, so the survivor nearly always sends its FIN before learning of control loss. Once a FIN is delivered, `recv()` keeps returning 0; a later RST in `CLOSE_WAIT` reaches the sender as `EPIPE`, not `ECONNRESET`.

## Controls and variants (exploratory, not decisions)

- **No `SO_LINGER` anywhere** (15 trials/cell, 180 observers): 178 EOF-class, 2 RESET — the probe discriminates.
- **kTLS holder** (TX ULP AES-GCM-128, 20 each): linger + kTLS → RESET 20/20; no linger + kTLS → EOF 20/20; linger no TLS → RESET 20/20. RX configuration and a real handshake not covered.
- **Variant B — treat `EPOLLHUP` as abort:** no effect; `EPOLLHUP` never delivered (79/79 events were `mask 8192`, `EPOLLRDHUP` only). Does not work.
- **Variant C — no half-close without an explicit signal:** before `shutdown_write` on a flow vsock the owner sends `HalfClose(flow)` on the control session; a bare `RDHUP` stays pending until it arrives; control loss aborts; a pending `RDHUP` with no message aborts after 10 s. Kill test: RESET 360/360 observers incl. survivor cells. Clean-close regression (2 MB each way, half-close from both apps, 10 trials non-mesh and inbound, with/without signal): CLEAN 80/80 byte-exact.

## Design implications

1. **G-CRASH, A-15, D20 stand only in part.** `SO_LINGER{1,0}` is validated for sockets the dying owner holds (incl. kTLS); it is not sufficient for sockets the survivor holds. "An owner exit resets the remote destination" (H-5) holds only when the dying owner holds the remote-facing socket.
2. **Half-close needs an explicit signal.** (a) an explicit half-close message on the control session worked, incl. clean close — changes the ADR-0166 message set and the *Flow end* rule, and needs a pending-`RDHUP` deadline decision; (b) a timer between `RDHUP` and `shutdown_write` is a heuristic (design.md § fallback) and was not tested; (c) poll-flag discrimination does not work. Option (a) is contract-shaping (ADR-0158/0160/0166).
3. **R5-10 and Quint.** R5-10 covers only a host-owner SIGKILL; it needs a guest-owner SIGKILL scenario and the survivor-held cells (rows 7–11). The guest-flow-owner model needs "flow-leg shutdown reaches the survivor before control-session loss" as an action; the assumption that the abort wins is undischarged.

## Limits

Stand-in single-threaded epoll owners (1 ms tick), not the production tokio owner — the kernel delivery order is a kernel-path property, the per-implementation race probability is not measured. Row 12 open (leg-F stand-in has no close propagation). One VM, 7.0.0-29, remotes in a netns on the same host, one flow per matrix trial. K-D5 not exercised. Reconnect not tested.

**Gate recommendation:** do not accept the owner-crash-visibility and H-5 text as written; DISTILL specifies the survivor-held reset cells against the approved half-close signal.
