# V-18 — a frame-shaped application payload above the egress MTU under hybrid (A-22, D5a, ADR-0165)

**Verdict: V-18 HOLDS.** Adjacent finding **FALSIFIED**: the fragment-tracking map has no lifecycle.

Probe and raw captures: `spike-scratch/netns-density-295-v18/` (increments a–e, each with `evidence/`; start with `increment-c/evidence/results.json`). Reproduce: `spike-scratch/netns-density-295-v18/launch.sh increment-c`.

## Verdicts

| Sub-claim | Verdict |
|---|---|
| V-18 as written: frame-shaped payload (escaped frame) above MTU, fragmented, fragment-shifted, byte-exact | **HOLDS** (8,565 datagrams, 0 corrupted, 0 lost in the unframe path) |
| Fragment-shifted edge at every 8-byte boundary ±2 at MTU 1500, and MTU 1280/1400/1492/9000 | **HOLDS** |
| Frame-looking but not frame-shaped payloads (wrong length, near-miss magic, chains, declared = first-fragment size) never stripped | **HOLDS** |
| Concurrent interleaved fragment trains | **HOLDS for the unframe path**; a separate ~0.05% concurrency-only loss upstream of the host verdict and on the reply leg, independent of fragmentation (INCONCLUSIVE as to cause; not a V-18 failure) |
| Wire: application datagram on the wire, reassembles from capture alone, valid IP checksums, consistent UDP length | **HOLDS** (21/21) |
| Control: egress program absent | as ADR-0165 predicts (only empty and frame-shaped arrive +8) |
| **Adjacent:** the fragment-tracking map keeps no stale state | **FALSIFIED** |

## Substrate

Host and guest `7.0.0-29-generic` x86_64, `systemd-detect-virt` none, fail-closed probes OK (`increment-c/evidence/substrate-probes.json`). Prebuilt CH from the fork (`overdrive/vhost-kernel-vsock`), `--vsock cid=3312,backend=vhost-kernel`. Off-host peer: netns behind veth `v18-h` 10.250.218.1/24 ↔ `v18-p0` (MTU 1500, then 1280/1400/1492/9000). No `UDP_SEGMENT`. scp + ssh, no tree sync; all resources `v18-`; harness's global `pkill -f vfwd` and vsock `rmmod` removed; `sch_tbf`/`sch_netem` unloaded afterwards.

Mechanism: the V-11 spike path (guest capture hooks, SK_SKB framing verdict, SEQPACKET leg over vhost-vsock, host owner, host verdict hybrid, `eg_decode_frag` on `v18-h`), with production magic `OVD1`. Oracle: peer logs length + SHA-256 of what it received (self-tested SHA, cross-checked with `hashlib` on capture-reassembled datagrams) and acks; `hybrid_escaped` counter delta must equal frame-shaped sent (29/29 phases).

## Results

```
phases 29   datagrams sent 8565   ack ok 8565   peer received 8565   missing 0   extra 0
above 1472 B: 7160   frame-shaped (escaped): 4352   escape counter == frame-shaped sent in 29/29 phases
tc_err_frag + bad_frame + helper_fail + redirect_fail + tc_err = 0
frag_first 3958  frag_shift 39170  tc_lenient_pass 4207
```
| Lane | Datagrams | Missing | Extra |
|---|---|---|---|
| frame-shaped, 4 kinds × 78 sizes × 2, conn/unconn, fresh socket | 1,216 | 0 | 0 |
| same-association mixed sequences 8 B–59,000 B | 276 | 0 | 0 |
| empty | 6 | 0 | 0 |
| lookalikes (15 kinds × 13 sizes) | 390 | 0 | 0 |
| random / zero / ones | 106 | 0 | 0 |
| MTU 1280/1400/1492/9000 | 550 | 0 | 0 |
| concurrent | 6,000 | 0 | 0 |
| wire capture | 21 | 0 | 0 |

The verdict's escape test and the egress program's validity test use the same predicate on the same bytes (payload's first 8 bytes vs `n-16` = `UDP length - 16`); the egress program takes the whole-datagram length from the first fragment's UDP header, so `decl_1464/1472/1480` are not confused.

Wire layout of escaped trains: first fragment 8 bytes short, later offsets one 8-byte unit lower, UDP length rewritten to `8 + payload` (e.g. `frame 4096 udp_len 4104 [0,1472,MF] [1472,1480,MF] [2952,1152,F]`; bare `rand 4096 [0,1480,MF] [1480,1480,MF] [2960,1144,F]`).

Control (no egress program): `frame 8 → 16, 100 → 108, … 59000 → 59008`, empty → 8-byte datagrams; everything else unaffected.

Concurrency loss (4 threads × 250, increment e): small sizes (no fragmentation) 6/11,795 failures, big 6/12,000, serial 0/6,000; classified `ack_lost_after_delivery` or `never_reached_egress` (host `noroute` counter), never `lost_after_egress`, no inconsistent wire train. Belongs with the pairing/parking validations (V-5(c) family).

## Adjacent: fragment-tracking map lifecycle — FALSIFIED

Increments b and c produced **invalid** H7 results (raw-socket fragments re-assembled by the host's netfilter defrag); d and e redo it with `AF_PACKET` injection.

`FRAGS` is a plain BPF hash (4,096): a stripped train's first fragment inserts `(src,dst,ip id)`, later fragments with that key shift, the entry is removed only when an MF=0 fragment passes; insert errors ignored.

```
S1  stale entry (first fragment only) then an unrelated bare datagram with the same id:
    received_byte_exact = false   wire [0,1480,MF] [1472,1480,MF] [2952,48,F] (shifted)
    control other id: true; same id after the entry is consumed: true
    peer: ReasmFails 2, ReasmTimeout 1 → shifted train fails UDP length check: lost, not delivered corrupt
S2  fill the map (4,300 stripped first fragments): entries 4096
    frame-shaped 4096 B after fill: 40 sent, 37 delivered; bare 400: 379 delivered (21 shifted by stale ids)
    entries 40 s later: 4068 (no expiry); after owner restart: 0, 80/80 delivered
S3  ordinary congested queue (netem delay 20ms limit 1), no crafting:
    run 0: 40 frame-shaped sent, 0 delivered, entries 40; run 1: 80; run 2: 120 (qdisc dropped 120)
    after qdisc removed: entries stay 120; next 200: 199 delivered, entries 119
```
A lost final fragment leaves an entry forever; it is consumed one-shot by the next train with the same `(src,dst,id)`, losing that datagram; after 4,096 stale entries new kept-frame datagrams lose their tails.

## Design implications

1. V-18 closes as HOLDS; D5a's structure unchanged.
2. ADR-0165 needs a pinned **fragment-tracking map lifecycle** (decisions for the architect/user): every train's first fragment is authoritative for its key (strip inserts, not-stripped removes any entry); the map is bounded without blocking inserts (LRU hash, or entry timestamp vs reassembly lifetime); the forwarder model carries "a fragment of an unrelated train never changes because of another train's state" as an invariant with S1 as a hazard.
3. New validation item for the fixed map: S1, S2, S3 as regressions on the real egress program, `FRAGS` entry counts as oracle.
4. The ~0.05% concurrency-only loss is not D5a; track it under pairing/parking.

## Not covered

6.18; a physical NIC / second machine; USO; multiple egress interfaces; IP options; wire reordering/duplication; IPv6 (#308); > 59,000 B (#309).

**Gate recommendation:** PROMOTE V-18; add the map-lifecycle contract before DELIVER step 4.
