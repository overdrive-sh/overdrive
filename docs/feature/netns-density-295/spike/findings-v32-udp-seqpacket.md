# V-32: guest UDP strparser cell into vsock SEQPACKET (netns-density-295)

Probe increment: `spike-scratch/netns-density-295-v32-udp-seqpacket/` (`bpf/`, `probe/`, `vmrun.py`, `cells/`, `report.py`, `evidence/vm-matrix-*/`). Question source: `.context/over-engineering-audit-review.md`, "Strparser / guest SEQPACKET duplicate emission". Kernel claims read in `vendors/linux` at `v7.0`. Recorded by the orchestrator from the spike agent's report (the agent's Write tool was refused).

## Verdicts

| # | Claim or candidate | Verdict | Evidence |
|---|---|---|---|
| 1a | Under credit pressure a SEQPACKET send emits a prefix without EOM and returns `-EAGAIN`; the backlog resends | **CONFIRMED (mechanism)** | 406,799 non-EOM prefix packets and 411,347 `-EAGAIN` across 36 SEQPACKET cells; up to 99,988 prefixes per 100k datagrams (`C-seq-59000-hb64k`). |
| 1b | The host sees duplicated datagrams | **REFUTED on the sockmap host verdict** | 0 duplicated in 3,520,000 datagrams. The prefix arrives as its own short skb and the ADR-0149 length check drops it; the whole retry is delivered once. |
| 1c | "The host concatenates fragments until EOM" (`virtio_transport_common.c:740-788`) | **REFUTED for the sockmap path, CONFIRMED for a `recv()` consumer** | The verdict is fed by `virtio_transport_read_skb` (`:1751`), one packet at a time. A userspace `recv()` reader does concatenate: 1, 187, 307 and 49,976 merged records per 100k in the four `seqrecv` cells. |
| 1d | The length check turns a merged record into datagram loss | **CONFIRMED wherever a merged record exists** | Every merged record dropped whole. With the check off, the 64 KiB credit regime lost 19,994 of 20,000. |
| 1e | Impact on the production host path is datagram loss | **REFUTED as stated** | 0 lost in every credit and throttle regime on the sockmap host. Real loss source is 1f. |
| 1f | (new) The guest strparser cell kills itself on variable-size datagrams | **FOUND, 7 of 7 cells** | Default TCP options stall after 7,558–8,679 delivered; `strp_bad_frame=2`. Same for SEQPACKET and STREAM. |
| 2 | STREAM both ways with a host TCP strparser cell | **Works under conditions; not a fix** | Fixed sizes: 0 dup, 0 merge, 0 loss. Mixed sizes pass only with `TCP_NODELAY`. Needs the vsock skb linearised first and repeats flaw 1f on a second cell. |
| 3a | `TCP_NODELAY` on the parking cell's sender | **MITIGATES 1f** | 15 mixed-size cells (1.5M datagrams): 0 stalls, 0 parser errors. |
| 3b | No parking cell: UDP intake straight into SEQPACKET | **Clean 6/6; not a drop-in** | Cannot hold the first datagram before pairing. |
| 3c | AF_UNIX `SOCK_DGRAM` parking cell | **Clean in steady state; late install fails** | 5 cells clean; after a late install 0 of 200 delivered (cause not diagnosed). |
| 3d | Larger host credit window | **Reduces cost only** | 59,000 B prefixes per 100k: 99,988 at 64 KiB, 17 at 128 KiB, 0 at 256 KiB. |
| 3e | Blocking send in the psock backlog | **Impossible** | `MSG_DONTWAIT` fixed at `net/core/skbuff.c:3322`. |

**Recommendation:** keep SEQPACKET with the ADR-0149 length check as the normative host defence; reject the STREAM + host-strparser rewrite. Fix the real defect — the guest strparser cell — by setting `TCP_NODELAY` on its sender or removing the TCP cell. Add variable-size datagrams to the cell's validation.

## Substrate

Native x86_64 metal, AMD EPYC 8024P, `systemd-detect-virt` none, fail-closed probes passed (`evidence/*/substrate-probes.json`). Host and guest `7.0.0-29-generic`. Guest kernel: stock bzImage `/srv/vm/overdrive-testing/kernel` (sha256 `b51367c7dab2…`). VMM: patched Cloud Hypervisor fork `41a619d19`, `--vsock cid=3232,backend=vhost-kernel`, 4 vCPU, 2 GiB (sha256 `eadddf1bbcdc…`). BPF object sha256 `ec1ae5b05414…`; `v32probe` sha per run in `evidence/*/artifact-pins.json`. Runs in private `/tmp/v32-work` under the lease (`launch.py` → `infra/metal/lease-holder.sh`); no `cargo xtask metal run`.

## What was built

Derived from the guest-capture spike's forwarder.
- **Guest cell:** UDP intake verdict prepends the 8-byte `ZUD1`+length frame, `pull_data`, redirects into loopback TCP socket P1. P2 has strparser + verdict (mode 4: `pull_data`, redirect to the vsock leg); mode 0 (no `pull_data`) kept as a control.
- **Host SEQPACKET:** verdict is the ADR-0149 check (`skb.len - 8 == declared` and magic), counting `ok / short / long / lt8 / badmagic`, then redirect to a loopback TCP sink; a `long` record is tested for a second magic.
- **Host STREAM:** vsock STREAM leg → host cell H1 over TCP → H2 (strparser, mode 5) → same sink.
- **Counters:** `fexit(skb_send_sock)` records every guest vsock send outcome; `vsock:virtio_transport_recv_pkt` records host RW packets by EOM flag.
- **App/sink:** plain UDP sender; datagram `i` carries seq, size and a position-dependent pattern; the host sink is the only payload observer and counts per-seq copies.
- **Pairing:** mirrors ADR-0158; host installs the leg then releases the guest over a control STREAM; host never writes on the SEQPACKET.
- **Variants:** `seq` (sockmap verdict, the ADR design), `stream`, `seqrecv` (host reads SEQPACKET with `recv()`).
- **Knobs:** size (`64 … 59000`, `mix` 64..59,000 random, `mixs` 64..2,047 random), host buffer, sink pause, `TCP_NODELAY`, check on/off, cell kind (`tcp`, `direct`, `unix`), guest bound.
- **Model limits:** one association per VM; plain loopback UDP intake (no capture hooks); TCP sink observer instead of ADR-0165's TCX frame removal; spike verdict, not production code.

## Probe 1 — SEQPACKET as designed

Fixed sizes, 256 KiB host buffer, 100,000 per cell (`A-seq-*`):

| size | once | dup | lost | short (prefix) | merged | `-EAGAIN` |
|---|---|---|---|---|---|---|
| 64 B | 100,000 | 0 | 0 | 0 | 0 | 0 |
| 1,024 B | 100,000 | 0 | 0 | 1 | 0 | 6 |
| 4,096 B | 100,000 | 0 | 0 | 0 | 0 | 0 |
| 16,384 B | 100,000 | 0 | 0 | 0 | 0 | 0 |
| 32,768 B | 100,000 | 0 | 0 | 0 | 0 | 0 |
| 59,000 B | 100,000 | 0 | 0 | 0 | 0 | 0 |
| 61,440 B (bound lifted to 65,536; 20,000 sent) | 20,000 | 0 | 0 | 0 | 0 | 0 |

With the ADR's 59,000 B bound, 61,440 B datagrams are dropped at the guest verdict, 2,000/2,000, counted.

Credit pressure: host buffer 64 KiB, nodelay, 100,000 per cell (`C-seq-*-hb64k`):

| size | once | dup | lost | prefix packets (all dropped by the check) | `-EAGAIN` |
|---|---|---|---|---|---|
| 64 B | 100,000 | 0 | 0 | 0 | 0 |
| 1,024 B | 100,000 | 0 | 0 | 101 | 210 |
| 4,096 B | 100,000 | 0 | 0 | 242 | 255 |
| 16,384 B | 100,000 | 0 | 0 | 33,331 | 33,762 |
| 32,768 B | 100,000 | 0 | 0 | 99,999 | 101,192 |
| 59,000 B | 100,000 | 0 | 0 | 99,988 | 100,615 |
| mix | 100,000 | 0 | 0 | 60,249 (+10 `lt8`) | 61,951 |
| mixs | 100,000 | 0 | 0 | 43 | 85 |

```
OUTCOME once=100000 dup_seqs=0 dup_extra=0 lost=0
HOST verdict ok=100000 short=99988 long=0 (2nd-magic 0) lt8=0 badmagic=0 | sink frames=100000 ok=100000 bad=0
GUEST send_outcomes={'errno11': 100615, 'ok_full': 100000}
HOST rx_pkts seq_eom=100000 seq_noeom=99988 peer_buf_alloc=262144 host_child_buf=65536
EVENTS [{"declared": 59000, "kind": "short", "n": 6528, ...}, ...]
```

Each `short` event is a 6,528-byte prefix of a 59,008-byte frame; short count equals non-EOM packet count in every cell. Cost is bandwidth: 199.7 s at 64 KiB vs 99.9 s at 256 KiB. At 128 KiB: 16,384 B 0 prefixes, 59,000 B 17; all delivered once.

**Destination throttled** (`B-seq-*`, sink paused 3 s): almost no credit pressure (0 prefixes at 1,024 B, 12 at 59,000 B, all once). The vsock leg is consumed in the vhost worker and credit returns before the verdict runs (`virtio_transport_common.c:1751-1773`); the redirect target's backlog gives no feedback to the source. A throttled userspace `recv()` reader does exhaust credit: 1,509–101,608 `-EAGAIN` per 100k in the `seqrecv` cells.

### Where merged records come from

1. Sockmap host, credit regimes: none (0 `long` in 36 cells).
2. Userspace `recv()` reader — the audit's mechanism:

| `seqrecv` cell (100,000 each) | ok | merged = lost |
|---|---|---|
| 1,024 B, paused 3 s | 99,999 | 1 |
| 16,384 B, paused 3 s | 99,813 | 187 |
| 59,000 B, paused 3 s | 99,693 | 307 |
| 59,000 B, 64 KiB buffer | 50,024 | 49,976 |

3. Guest strparser mis-cut (1f): `A-seq-mix` one `long` (`declared=5391, n=42054, second_magic=1`); `A-seq-mixs` one (`declared=659, n=1238`); both dropped by the check.
4. Synthetic records written straight onto the guest SEQPACKET (`E-seq-synth`):

| record | host verdict |
|---|---|
| two whole frames in one record (n=316, declared 100) | `long`, dropped |
| frame plus 3 trailing bytes (n=111) | `long`, dropped |
| frame cut at 150 of 300 B | `short`, dropped |
| header declaring 500 with 20 B present | `short`, dropped |
| one 3,000 B frame sent as two records | `short` + `badmagic`, **both dropped: datagram lost** |
| outer datagram whose bytes at the split form a valid inner frame | first piece dropped; **second fragment passes the check and is delivered as a datagram the app never sent** |
| whole frames (100, 100, 59,000 B) | `ok` |

The last two need a send split into two skbs without an `-EAGAIN` between them (credit returns between loop iterations, `vsock_connectible_sendmsg` continues, `af_vsock.c:2206-2272`). Synthetic only — **0 observed naturally** in 3,520,000 datagrams with 406,799 forced prefixes.

**The check is load-bearing:** `E-seq-59000-hb64k-check-off` (20,000 datagrams): 6 delivered, 19,994 lost; sink `pattern_corrupt 19,994`, `stream_desync 39,985`. Same regime with the check: 100,000/100,000.

**Other failure modes:**
- Missing `pull_data` on guest P2 (`E-seq-*-p2mode0-control`): paged TCP skb leaves as several SEQPACKET records (`__skb_send_sock` sends head and each fragment separately, `skbuff.c:3292-3396`). 1,024 B lost 471/20,000; 59,000 B lost 20,000/20,000 (records 32,741 + 26,267 B).
- Host buffer below the frame (`E-seq-59000-hb32k-emsgsize`, 32,768 B buffer, 59,008 B frames): first send `-EMSGSIZE` (errno 90), a hard error; psock stops transmitting; 0/300 delivered, sender stalled after 160. Source: `virtio_transport_seqpacket_enqueue` refuses `len > min(peer_buf_alloc, buf_alloc)` (`:825-846`). STREAM has no such limit (`E-stream-59000-hb32k` 20,000/20,000 with 35,932 partial writes).

## Finding 1f — the guest strparser cell dies on variable-size datagrams

| cell (default TCP options) | sent before stall | delivered | `strp_bad_frame` |
|---|---|---|---|
| `A-seq-mix` | 8,960 | 8,677 | 2 |
| `A-seq-mixs` (repeats r1, r2 identical) | 15,264 | 7,558 | 2 |
| `A-stream-mix` (repeat r1) | 8,960 | 8,679 | 2 |
| `A-stream-mixs` | 15,264 | 7,612 | 2 |

Seven of seven variable-size cells with default options stall permanently; every fixed-size cell (six sizes, 100,000 each) passes.

Mechanism (verified by logging what the parser reads): `__strp_recv` clones the original skb for a message that starts mid-skb and records the start only in `strp_msg(skb)->offset` (`strparser.c:169-213`); `sk_psock_strp_parse` runs the BPF parser on that skb unmodified (`skmsg.c:1143-1158`), and BPF cannot read `strp_msg`. So the parser reads the first bytes of the skb, not of the message. The failing parse in `A-seq-mix` showed `ctx.len()=23926` and first 8 bytes `2a2b2c2d2e2f3031` (mid-payload), returned `-1`, strparser aborted. An earlier mid-skb message reads a stale header and returns the wrong length, cutting the `long` record above. Fixed-size frames pass by accident (the stale header declares the same length). Messages start mid-skb because the sender coalesces back-to-back small writes while earlier data is unacknowledged (`tcp.c:1243-1262`, Nagle at `tcp_output.c:2205-2217`).

`TCP_NODELAY` on P1 removes the stall: 15 cells (`A2-*-mix/mixs`, `A3-seq-mix` r1–r2, `A3-seq-mixs` r1–r3, `A3-stream-mixs` r1–r2, four mixed `C-*-hb64k`), 1.5M datagrams, 0 stalls, 0 parser errors, 0 lost. A measured mitigation, not a proof (`strp_data_ready` defers work when the socket is user-owned, `strparser.c:372-390`).

## Probe 2 — STREAM both ways with a host TCP strparser cell

| regime (100,000 each unless noted) | once | dup | lost |
|---|---|---|---|
| fixed 64 B–59,000 B, 256 KiB (`A-stream-*`) | 100,000 each | 0 | 0 |
| fixed, host buffer 64 KiB (`C-stream-*`) | 100,000 each | 0 | 0 |
| mixed, 64 KiB, nodelay | 100,000 each | 0 | 0 |
| mixed, nodelay (`A2`, `A3`) | 100,000 each | 0 | 0 |
| mixed, default options | 7,612–8,679 | 0 | stall (guest cell dies, 1f) |
| destination paused 3 s (`B-stream-*`) | 100,000 each | 0 | 0 |
| 61,440 B, bound 65,536 (20,000) | 20,000 | 0 | 0 |
| host buffer 32 KiB, 59,008 B frames (20,000) | 20,000 | 0 | 0 |

STREAM partial sends are harmless (91,145 `-EAGAIN`, 89,744 partial writes at 59,000 B / 64 KiB, nothing lost; `af_vsock.c:2289-2296`, `skmsg.c:700-735`). Conditions it needs: (1) the host must `pull_data` the vsock skb before redirecting to H1 (without it `d1-stream-59000` delivered 1/200 — vhost-built skb is paged, boundary lands mid-skb, parser read `ctx.len()=47616` starting `f8f9fafbfcfdfeff`, then `parse_bad`); (2) the same parser flaw on the second cell, needing `TCP_NODELAY` on H1; (3) no `ctx.len()` length test at a strparser verdict (the skb is the accumulated TCP skb: `n=80,384` for a 59,008-byte frame in `s2-stream-59000-pause`). Buys: no prefix packets (~400,000 fewer over the matrix), no `EMSGSIZE` ceiling. Costs: a second TCP cell and strparser per association, and a second instance of 1f. Fixes no duplicate, merge or loss Probe 1 showed on the sockmap host.

## Probe 3 — other alternatives from the kernel source

| alternative | result | notes |
|---|---|---|
| Blocking send in the backlog | impossible | `MSG_DONTWAIT` fixed at `skbuff.c:3322`; vsock loop returns `-EAGAIN` when `timeout == 0` (`af_vsock.c:2206-2219`). |
| Larger host credit window | reduces prefix rate | Buffers above 256 KiB need `…MAX_SIZE` first (`af_vsock.c:2032-2044`). 59,000 B prefixes per 100k: 99,988 / 17 / 0 at 64 / 128 / 256 KiB; 1 MiB smoke of 3,000 showed 0. |
| Direct intake into SEQPACKET (no cell) | clean | 100,000/100,000 for 1,024 B, mix, 59,000 B (2 prefixes); at 64 KiB 16,384 B (25,458 prefixes) and mix (54,421). Cannot hold the first datagram before pairing; UDP cannot be installed late (`udp_read_skb` takes one skb per `data_ready`). A pre-paired per-slot leg would work, not tested. |
| AF_UNIX `SOCK_DGRAM` parking cell | steady state clean, late install broken | 100,000/100,000 for 1,024 B, mix, 59,000 B, and 16,384 B at 64 KiB (32,893 prefixes). With 100 datagrams queued before install (`k1`, `k1-rearm`) 0/200 delivered, no verdict ran. `af_unix.c` has no `set_rcvlowat` (TCP has, `tcp.c:1846-1859`). The datagram skb is owned by its sender, so the route must key on the sender's cookie (early `F-seq-unix-*` runs failed `noroute` for that reason, kept as evidence). |

## Kernel facts used (`v7.0`)

- `net/core/sock_map.c:305-309` strparser + verdict only on TCP; `:541-551` vsock STREAM/SEQPACKET need ESTABLISHED, unix datagram and UDP allowed.
- `net/vmw_vsock/af_vsock.c:2206-2219,2262-2272,2289-2296` non-blocking loop; SEQPACKET returns the byte count only when `total_written == len`; `:931` child inherits `buffer_size`.
- `virtio_transport_common.c`: `:298-305` EOM only on the skb that exhausts the message; `:339-362,495-509` `pkt_len` clamped to credit; `:825-846` `-EMSGSIZE`; `:723-788` `recvmsg` assembly to EOM; `:1751-1773` `read_skb` hands one packet to the verdict and returns credit.
- `net/core/skmsg.c:700-735` retry of the same range, `:1236-1290` verdict path, `:1143-1158` parser on the unmodified skb.
- `net/core/skbuff.c:3292-3396` `__skb_send_sock` sends head and each fragment separately.
- `net/strparser/strparser.c:169-250,372-390`; `net/core/filter.c:1907-1919` (`pull_data` counts from `skb->data`, not the message start).
- `net/ipv4/tcp.c:1243-1262`, `tcp_output.c:2205-2217` write-queue tail append and Nagle.
- `net/unix/af_unix.c:2675-2686,992` `unix_read_skb` takes one datagram; datagram ops define no `set_rcvlowat`.
- `vsock:virtio_transport_alloc_pkt` fires before EOM is set, so guest-side EOM counts are meaningless; the host's `recv_pkt` flags are the record of what was sent.

## Design implications

1. ADR-0148/0149/0165: the SEQPACKET duplicate is real on the guest and harmless on the sockmap host only because of the length check. Make the check normative (every SEQPACKET skb length-checked; a `short` record is an expected, counted prefix). ADR-0148's "never write on SEQPACKET" is a host-side rule and does not constrain the guest sender.
2. Do not replace SEQPACKET with STREAM plus a host strparser.
3. Fix the guest cell: `TCP_NODELAY` on the sender of every strparser parking cell (P1, and P3 if it has the same shape), or remove the TCP cell. Add `mix`/`mixs` datagrams to validation; every fixed-size test passes by accident.
4. Pin the buffer invariant: `min(peer_buf_alloc, buf_alloc)` ≥ the largest frame (59,008 B) on both ends, or the first oversized send is a silent permanent `-EMSGSIZE`.
5. Pin the linearise step: the guest P2 verdict must `pull_data` the whole frame range before redirecting (2.4 %–100 % loss otherwise).
6. Counters: `short` is the credit-pressure signal; alert on `long`, `lt8`, `badmagic`, `strp_bad_frame`.
7. Residual hazard: the split-without-`EAGAIN` pair loses one datagram and can deliver a forged one into the sender's own association; if that matters, add a sequence number or checksum to ADR-0149's header.

## Not run

Concurrent associations on one VM; ADR-0165's TCX frame removal and a real destination (TCP sink stands in); repeats of every cell (only parser-flaw cells repeated: 7 stalled, 15 passed, default-option stalls deterministic); a natural split-without-`EAGAIN` pair (synthetic only, 0 in 3.52M); host memory growth under a throttled destination and CPU/latency cost of prefix retries; unix late-install root cause; TCP cell `SO_RCVLOWAT` re-arm re-measurement; pre-paired per-slot leg; IPv6; datagrams above 61,440 B; aarch64.

## Commands and evidence

```
bash sync.sh                                 # scp sources and cells to /tmp/v32-work
ssh <host> 'bash /tmp/v32-work/build.sh'     # BPF + probe build, private dir
python3 launch.py <name> 'cd /tmp/v32-work && V32_PARSELOG=1 python3 vmrun.py matrix cells/<set>.json'   # under the lease
python3 report.py [label-substring]          # offline table
```

Matrix runs: `A` = `…084557Z` (16 cells), `A2` = `…091458Z` (6), `C1` interrupted (8 cells kept in `…094446Z`), `R1` = `…101017Z` (28: F/E/B/D), `R2` = `…112457Z` (29: F2/A3/C). Development: `…080006Z`, `…081226Z`, `…082241Z`, `…082926Z`, `…084140Z`. Per cell in `evidence/vm-matrix-*/`: `<label>-summary.json`, `<label>-host.json`, `proc-*.log`, `run.log`, `substrate-probes.json`, `artifact-pins.json`. Cell definitions in `cells/*.json`. Cleanup: `/tmp/v32-work` removed; no processes, bpffs pins, cgroups or netns left.
