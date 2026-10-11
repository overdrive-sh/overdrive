# V-29 findings: owner-sent empty UDP datagrams instead of the egress TC unframe chain

Probe increment: `spike-scratch/netns-density-295-v29-udp-empty/` (final evidence `increment-d/evidence/results.json` and pcaps; `increment-c` hazard run; `increment-a` dev smoke; `increment-b` first evidence run with harness flaws — its throttled, ring-overflow and mixed-size numbers are not used). Question source: `.context/over-engineering-audit-review.md` finding 2 and gaps 3–4; ADR-0165, ADR-0181, ADR-0148/0149. Recorded by the orchestrator from the spike agent's report (the agent's Write tool was refused).

**PIVOT.** Strip every frame in the host verdict, `SK_DROP` the declared-length-0 frame with a ringbuf notice, and let the owner `send(fd, NULL, 0)`. That works and is cheaper than the egress TC chain — but only if the user rules that an empty datagram may be reordered against its neighbours. If the ruling is "no reordering", DISCARD this and keep the TC chain.

## Verdicts

| # | Question | Verdict | Evidence |
|---|---|---|---|
| 1a | Every empty emitted exactly once as a real zero-length datagram | HOLDS | 3000 sent, 3000 `UDP, length 0` in tcpdump on `v29-h`, 3000 at the peer. Mixed-size run 440/440. ~98,000 empties across 35 verdict-notify phases, none duplicated. |
| 1b | No interaction with the psock | HOLDS | `skb_send_sock` fexit saw 0 errors and 0 zero-returns in every verdict-notify phase; 0 data datagrams missing or duplicated. Control redirecting the zero-length skb (`verdict-naive`): `sent_zero` 1 and 5 of 6 later data lost. |
| 1c | Reorder rate of owner-sent empty vs redirected datagrams | FALSIFIED as exact ordering | Ungated, no load: 31–70 % of empties overtake their predecessor. Ungated, throttled: 98.8 %, overtaking up to 2,423 datagrams. A gate on the flow's sent-count removes every early case (17/17 gated phases); late tail stays 0.1–57 % (no load) and 0.8–17 % (throttled). |
| 2 | Ring full: a drop counter makes every lost notice observable | HOLDS | 1000 empties into a 4096-byte ring: `seen 1000 = ok 102 + lost 898`; per-flow counter, owner count (both 898) and peer (102 empties; 1000 with compensation) agree. |
| 3 | Zero-length `SK_PASS` into a psock → `recvmsg` `-EFAULT` forever | CONFIRMED | Source and experiment: every `recv`/`recvmsg` returns `-EFAULT`, data behind it unreadable, `poll` stays readable; clears only when the socket leaves the map, queued data lost. |
| 4 | One redirected skb > ~32 KiB leaves as several datagrams; linearising fixes it | CONFIRMED / HOLDS | Without `bpf_skb_pull_data`, every size ≥ 32,768 B split into 2–4 datagrams (30/30). With it, 65/65 arrived as one byte-exact datagram, 1 KiB to 65,507 B. |

## Reorder detail

Early = the empty arrived before the data datagram sent just before it; late = after the next one. Run-to-run variance is large (55 % vs 34 % early at identical settings).

| Regime | Owner | Pacing | Early | Late | Max displacement |
|---|---|---|---|---|---|
| no load | ungated | 1000 µs | 65.5 % | 0.1 % | 2 |
| | ungated | 500 µs | 70.4 % | 0 | 6 |
| | ungated | 200 µs (two runs) | 55.4 % / 34.4 % | 0.1 % / 0.4 % | 12 |
| | ungated | 100 µs | 65.5 % | 0.5 % | 32 |
| | ungated | 200 µs, empty every 8th | 30.8 % | 0 | 22 |
| | gate, 1 ms retry | 1000 / 500 / 200 / 100 µs | 0 | 16 / 44 / 33 / 57 % | up to 32 |
| | gate, spinning | 1000 / 500 / 200 / 100 µs | 0 | 0.1 / 3.8 / 17–19 / 55 % | 1 to 28 |
| throttled (5 Mbit) | ungated | 500 µs | 98.8 % | 0 | 2,423 |
| throttled (5 Mbit) | gate, 1 ms retry | 500 µs | 0 | 10.2 % | 4 |
| throttled (5 Mbit) | gate, spinning | 500 µs | 0 | 0.8 % | 2 |
| throttled (5 Mbit) | gate, spinning, `SO_SNDBUF` 16 KiB | 500 µs | 0 | 16.8 % | 2 |

- **Why the empty overtakes:** the verdict only queues the skb and schedules work (`net/core/skmsg.c:989-990`); the backlog defers while the socket is not writable, i.e. wmem < sndbuf/2 (`skmsg.c:649-651`, `include/net/sock.h:2631-2633`); the owner's empty needs only wmem < sndbuf (`net/core/sock.c:2983`). Under a backlog the empty passes the whole queue. The owner's non-blocking send never hit `EAGAIN`.
- **The gate** makes the owner wait until every earlier redirected skb of the flow was sent (fexit on `skb_send_sock` counts them). It cannot stop the successor's redirect running first, so no owner-side way to exact order was found. Exact-order mechanisms: the existing TC chain (the empty rides the same psock queue), or holding successor frames (the verdict cannot).
- Spinning gate CPU cost not measured.

## Kernel citations (`@ v7.0`)

- Zero-length redirect: `net/core/skbuff.c:3336-3337,3400` (returns 0); `net/core/skmsg.c:730-732` (EPIPE, TX cleared).
- `udp_bpf` overrides no sendmsg: `net/ipv4/udp_bpf.c:128-134`.
- Zero-length `SK_PASS`: enqueue `skmsg.c:1038-1062`, `:544-577`; `-EFAULT` at `:444-445`, past the dequeue at `:488-491`; `udp_bpf.c:79-85`. (The review's `skmsg.c:437-446,489-491` map to `:444-445` and `:488-491`.)
- Paged skbs: `include/linux/virtio_vsock.h:61-92`, `include/linux/mmzone.h:62` (frag skb above ~32 KiB); `net/core/skbuff.c:3305,3322,3361` (one `sendmsg` per head or frag, `MSG_MORE` TCP-only); `net/core/filter.c:1907-1918`, `:3666-3690` (SK_SKB pull_data / adjust_room).

## Hazard output (item 3, identical on two runs)

```
control (no psock)  recv AAAA→4 ; recv(empty)→0 ; B→4, C→4 ; D→4, E→4
psock + SK_PASS     recv AAAA→4 ; recv(empty datagram)→-1 EFAULT ; recvmsg→EFAULT
                    after B,C: 6×recv→EFAULT ; after 1.5 s + D,E: 3×recv→EFAULT ; poll revents 0x1
                    after map remove: recv→EAGAIN ×4 (queued B,C,D,E gone)
```

## Paged-skb piece lengths without linearising (item 4)

```
32768→[32760, 8]   32769→[32760, 9]   40000→[32760, 7240]
49152→[32760, 16384, 8]   59000→[32760,16384,8192,1664] or [32760,32747]
```

The existing V-11/V-18 modes already called `pull_data` after the strip, which is why they were byte-exact to 59,000.

## Design implications

1. Pin two verdict invariants: never `SK_PASS` or redirect a zero-length skb, and drop any frame whose declared length is 0; `pull_data(len)` before every redirect to a UDP or SEQPACKET target.
2. D15 needs a user ruling on empty-vs-neighbour ordering (numbers above). With the gate, displacement ≤ 4 datagrams at 500 µs pacing and up to 32 at 100 µs. A real gate needs a wake on `skb_send_sock` completion, not polling.
3. Notices pending at association teardown are dropped (`notice_stale_flow`), as queued redirected data is. The epoch-mismatch path was not exercised (no flow id reused).
4. Ring sized at 40 bytes per record; the per-flow lost counter is the observable. Compensating by count restores the number of empties, not their position. The owner must read the counter before removing the flow.
5. Adjacent, independent of empties: without a linger, a burst followed by immediate close loses the tail (428/2000 in `increment-b`; 2000/2000 with a 1 s linger in `increment-d`). `bad_frame` counts from burst-then-close in `increment-a`/`b` not investigated.

## Not run

Second machine / physical NIC (destination is a netns behind a veth); IPv6 and `UDP_SEGMENT`; concurrent associations with empties; the epoch-mismatch notice path; spinning-gate CPU cost; owner `EAGAIN` retry (never triggered); vsock-source psock under `SK_PASS` (hazard run on a UDP receiver only, same `sk_msg` code); the SEQPACKET duplicate-emission item (see V-32); sizes above 65,507; production code.

## Substrate and cleanup

Host and guest `7.0.0-29-generic`, x86_64, `systemd-detect-virt` none, fail-closed probes passed. VM: Cloud Hypervisor fork (`v53.0.0` build) `--vsock cid=3329,backend=vhost-kernel`. Every run held the canonical lease; files in private `/tmp/v29-work`, no tree rsync, names prefixed `v29-`. No TC program attached on the host. All netns, links, cgroups, qdiscs, processes removed; `/tmp/v29-work` and the lock file deleted.
