# Findings: kTLS splice relay for the public ingress gateway

Date: 2026-11-02. Substrate: native x86_64 metal host `ubuntu@151.115.99.251`, `uname -r` = `7.0.0-38-generic`
(the pinned appliance kernel line, ADR-0068). Source tree: HEAD `ff7cfec53691b7b1034aab8ae1be1d50bd63afee`, dirty with
untracked spike code only; `crates/` untouched. rustls `=0.23.39` (workspace `Cargo.lock`), aya 0.13.1.
Probe code: `spike-scratch/gw-splice-a/` (a, b, c) and `spike-scratch/gw-ktls-bpf/` (d, e). Raw logs: `spike-scratch/*/out/` (gitignored).
Every endpoint is a real loopback TCP socket with a real rustls TLS 1.3 handshake, then kTLS TX+RX; byte streams are pattern-checked at the far peer.
Kernel citations are `path:line @ tag`; v7.0 from `vendors/linux` (tag `v7.0`), v7.3 from a depth-1 read of tag `v7.3`.

| Item | Verdict | Gate |
|---|---|---|
| (a) splice kTLS-RX -> pipe -> kTLS-TX | WORKS on 7.0 under a pipe rule; kernel data-loss defect found | PROMOTE with condition |
| (b) TLS 1.3 kTLS incl. KeyUpdate mid-relay | WORKS (rustls `kernel::KernelConnection`; `dangerous_extract_secrets` is NOT enough) | PROMOTE |
| (c) exact-CL / per-chunk splice, backpressure, keep-alive | WORKS, bounded memory, no over-read; NO CPU win | PROMOTE |
| (d) connect4 rewrite + TLS handshake + kTLS on the same fd | WORKS | PROMOTE |
| (e) kTLS+sockmap rejection on 7.0.0-38 | NOT present on the metal kernel; present at v7.3 | INFORMATIONAL (G1 uses no sockmap) |

## (a) splice kTLS-RX -> pipe -> kTLS-TX

Hypothesis: one splice-in returns at most one decrypted record and never crosses a record boundary; a non-DATA record yields `-EINVAL`
and stays readable via `recvmsg` with a `TLS_GET_RECORD_TYPE` cmsg. Falsification: a call returns bytes of two records, or the record is lost.

Record-boundary rule (actual): one splice-in returns `min(len, bytes left in that record)`, never crossing a record boundary. A `len` smaller
than the record leaves the rest readable. With `SPLICE_F_NONBLOCK`, `EAGAIN` means no complete record yet.

Non-DATA record (log `a-20261102T105724Z.log`):
```
splice#2 (next record is HANDSHAKE) Err(EINVAL) pipe_bytes=0
splice#3 same again (sticky)         Err(EINVAL);  splice#3b BLOCKING flags=0 -> EINVAL, no hang
recv() without cmsg buffer           n=-1 EIO(5)
recvmsg+cmsg                         type=22 n=9 bytes=[04,00,00,05,01,02,03,04,05]
splice#4                             Ok(60) pattern-exact;  ALERT -> type=21 [01,00];  after FIN -> Ok(0)
```
Fallback procedure: on `EINVAL`, `recvmsg` with a ~20000-byte buffer and a control buffer of at least `CMSG_SPACE(1)`; read the `SOL_TLS/TLS_GET_RECORD_TYPE`
cmsg. The record is requeued, so `EINVAL` is sticky and never blocks; a plain `recv` without the cmsg buffer returns `EIO` but the record stays readable.
This is routine traffic: an upstream's TLS 1.3 NewSessionTickets arrive as a type-22 record (162 bytes for two tickets) before the first response.

DEFECT (kernel, v7.0): data loss when the pipe cannot map a whole record (`a-20261102T100545Z.log`):
```
record=16384 pipe=4096 len=4096: moved=49137 recv=49137 total=49152 first_mismatch=Some(16379) MISMATCH/LOSS
record=16384 pipe=8192 ... 1048576: EXACT
FUZZ2 pipecap=65536 drain<=100% bytes=31512610 runs_with_loss=0
FUZZ2 pipecap=65536 drain<=10%  runs_with_loss=73   (it=0 lost=20, mismatch=Some(151849))
```
Source: `net/tls/tls_sw.c` `tls_sw_splice_read` @ v7.0 computes `chunk = min(full_len, len)`, ignores a short `skb_splice_bits` return, advances `rxm->offset += len`
when `chunk < full_len`, and `consume_skb`s the record when `chunk == full_len` even if `copied < chunk`. Loss is 5 bytes per record (the TLS header length), with no error on either side.
Fixed at v7.3: `if (copied < rxm->full_len) { rxm->offset += copied; rxm->full_len -= copied; goto splice_requeue; }` (`net/tls/tls_sw.c` @ v7.3, `splice_read` tail).
Consequence for the design: the relay on kernel 7.0 MUST enforce the invariant "private pipe of at least 64 KiB, splice-in only into an EMPTY pipe,
`len = min(remaining, pipe_capacity)`" (held across 4 pipe sizes x 80 randomized runs of ~30 MB, and in the 256 MiB / 64 MiB relay runs), or the appliance kernel carries the `copied` fix.
This is an empirical envelope on one kernel build, not a proof; the design should name a Tier-3 RED witness with a 4 KiB pipe. Nonblocking mode must finish the pending splice-out before polling the source.

## (b) TLS 1.3 kTLS RX/TX and KeyUpdate mid-relay

KeyUpdate with a real rustls peer (`b-20261102T105955Z.log`):
```
gw: KeyUpdate(request=1) received; before TLS_RX re-arm: recvmsg=Err(errno 127 EKEYEXPIRED) splice=Err(errno 127)
gw: answered with KeyUpdate(not_requested) and re-armed TLS_TX
R(upstream) received n=15000 mismatch=None => EXACT
tls_stat: TlsDecryptError=0 TlsRxRekeyOk=2 TlsTxRekeyOk=2 TlsRxRekeyReceived=2
```
Negative control (record fetched, `TLS_RX` not re-armed): sticky 127 and `poll(POLLIN)` returns 0. KeyUpdates at a message boundary, inside a head, and inside chunk framing stayed byte-exact (`TlsRxRekeyOk=30`, `TlsDecryptError=0`).
Procedure:
1. Fetch the type-22 record (payload `18 00 00 01 rr`) via the `EINVAL` -> `recvmsg` path.
2. `kconn.update_rx_secret()` then `setsockopt(TLS_RX)` with rec_seq 0. Until then reads and splice return `EKEYEXPIRED` (127) and poll reports nothing.
3. If `rr == 1`: send `18 00 00 01 00` as record type 22 under the old TX key, then `update_tx_secret()` and `setsockopt(TLS_TX)`.
4. Never forward the KeyUpdate; each leg rekeys independently.
Requirements: rustls `kernel::KernelConnection` via the unbuffered handshake API, reading one TLS record at a time so nothing is over-read; `enable_secret_extraction=true`; AES-128/256-GCM tested.
`dangerous_extract_secrets` has no rekey path. TX rekey must be serialised with all writers of that fd. The kernel does not track AES-GCM confidentiality limits: a long-lived relay counts records and originates its own KeyUpdate.
Kernel: rekey is `setsockopt(TLS_TX/TLS_RX)` on an existing TLS socket (`net/tls/tls_main.c:670-762 @ v7.3`); RX pauses with `-EKEYEXPIRED` while `key_update_pending` (`net/tls/tls_sw.c:1531-1558, 1104-1105 @ v7.3`).

## (c) Exact-length, per-chunk splice, backpressure, keep-alive

```
after req1 body: PEEK on A -> "POST /m2 HTTP/1.1\r\nHost:"   (next head untouched)
c1: 4 exchanges (CL, chunked, CL0, 204, ...) EXACT end-to-end
c6: 64 MiB, U stalled 3 s: C_sent frozen at 447 KiB, RSS 4684 KiB flat; max(C_sent-U_recv) 0.46-0.48 MiB
```
Loop: head or chunk-size line: `MSG_PEEK`, then `recv` exactly up to the terminator (a partial head is consumed into an accumulator because peek cannot pass a control record; skipping this livelocks when a KeyUpdate sits inside a head).
Send the head/line with `send`. Body: while `remaining > 0`, splice-in `len = min(remaining, pipe_cap)` into an EMPTY pipe (>= 64 KiB), splice-out until empty (`SPLICE_F_MORE` while `remaining > 0`); on `EAGAIN` poll `POLLIN`/`POLLOUT`.
Chunked: per chunk, size line, `size` bytes spliced, CRLF, final `0\r\n\r\n` through userspace. Nothing is read past `remaining`; no leftover management; CL 0 splices nothing.
Backpressure: the blocking relay blocks in splice-out; the nonblocking relay gets `EAGAIN` from splice-out (443 in the 32 MiB run) and polls; both lossless and bounded.

CPU (gateway thread, 256 MiB, 16 KiB records, indicative, loopback): splice 340 ms (356) / 66.8 calls per MiB vs copy 318 ms (317) / 16.0.
Record-size sweep, splice vs copy ms: 16 KiB 93/87; 8 KiB 115/91; 4 KiB 154/118; 1 KiB 388/197; 256 B 1314/600 (splice calls/MiB 66.8, 129.5, 259, 1028, 4098).
Implication: G1 is a bounded-memory, no-userspace-payload-copy mechanism. It MUST NOT claim a CPU gain; per-call cost scales with the peer's record size.

## (d) connect4 backend selection with a kTLS upstream socket

Result: 20 of 20 connections to VIP `10.96.0.9:443` were rewritten to the loopback backend by an aya connect4 program; `getpeername` = backend; ULP `''` -> `'tls'`; 1 MiB moved through kernel TLS.
The control process outside the cgroup timed out (`HITS` = 20). A pre-connect BPF rewrite composes with a later TLS handshake and kTLS install on the same fd. Caveat: a minimal rewrite program, not the production selection arm.

## (e) kTLS + sockmap on 7.0.0-38

- E1, kTLS first then sockmap: `EINVAL` from `net/core/skmsg.c:756 @ v7.0` (rejects any socket that already has a ULP; `:757 @ v7.3`).
- E2, sockmap first then ULP/TLS_TX/TLS_RX: all OK, 4000 bytes one way and 3000 the other exact (no verdict program attached).
- v7.3 rejects the reverse order in `do_tls_setsockopt_conf` (`net/tls/tls_main.c:636-641 @ v7.3`); v7.0 has no such check (`:635 @ v7.0`).
- The `-29` and `-38` package changelogs contain no sockmap line. v7.2 was not fetched; the claim "from 7.2" rests on the research doc; v7.3 is verified.
G1 uses no sockmap, so (e) changes nothing in the design; it confirms the forbidden combination must stay out of any future change.

## Design implications

1. Add the pipe invariant (or the kernel `copied` fix) to the relay contract; Tier-3 witness with a 4 KiB pipe.
2. Both handshakes use the rustls unbuffered API; the `KernelConnection` lives for the connection's life.
3. The control-record path (`EINVAL` -> `recvmsg` -> dispatch) serves both legs and the head/chunk-line readers; `close_notify` maps to a half-close of the other leg.
4. TX rekey is serialised with all writers of that fd; the relay originates KeyUpdate by record count.
5. No CPU-gain claim.

Caveats: loopback only, single host; no IPv6, TLS 1.2, ChaCha20, or real NIC layout.
Leftovers on the metal host: none after the final run (no netns, no `gwk*` cgroup, no extra listeners, empty nft ruleset, no loaded `gw_connect4`/`CFG`/`HITS` maps or programs); synced source and `out/` logs remain under `/home/ubuntu/overdrive/spike-scratch/gw-*`.
Gate recommendation: PROMOTE (a with the pipe condition, b, c, d); G1 stands.
