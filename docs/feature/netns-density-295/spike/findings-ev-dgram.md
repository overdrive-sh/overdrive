# V-EV-DGRAM findings (netns-density-295, EVMOD-DGRAM / ADR-0201)

**Verdict: WORKS.** Datagram mode as pinned carries 0 to 59,000 B datagrams exactly once and in order, both ways, with empties in order. It needs two additions to the contract, two pins, and no ENOBUFS handling. A hypothesis I held on that last point was falsified.

**Gate recommendation: PROMOTE EVMOD-DGRAM with D1 to D4 below folded in.** Nothing here reopens D4 or UDP-SEQ.

## Substrate and artifacts

- **Physical host:** AMD EPYC 8024P, `uname -r` = 7.0.0-38-generic, `systemd-detect-virt` = none, `/dev/kvm` API 12 (preflight failed closed otherwise).
- **VMs and guest:** `uname -r` = 7.0.0-29-generic in the private QEMU VM and the Cloud Hypervisor guest.
- **Isolation:** the module loaded only inside the QEMU VM or the CH guest. `physical-after.json` shows no module added or evmod/shmproxy present in any run. No code under `crates/`, `specs/` or `docs/` was touched.
- **Module:** `module/evmod.c` is the evmod inc2 module (`d5710d7c`) plus datagram mode; `evmod.ko a3e389b8df55`, source `3c0f27d260b8`. Identical in every final run. The ioctl, the 512-entry `/proc/evmod_ends` ring and the module params are spike stand-ins for `/dev/ovd_fwd` and the 32-byte end record.
- **Probe and BPF:** static Rust probe `dgprobe`; aya-rs BPF with sendmsg4/recvmsg4 modelled on `cgroup_recvmsg4_service.rs`, plus a cgroup_skb egress drop.
- **Two environments:**
  - **Env 1:** QEMU 4 vCPU, loopback vsock, chain A → S → fwd1 → vsock → fwd2 → U → D. Both forwarder pairs run the new module, so the host-side code path is exercised, but over loopback, not vhost.
  - **Env 2:** a CH guest (fork, `--vsock backend=vhost-kernel`) with the module in the guest. The host side is an independent userspace OVD1 parser/writer on the physical host, so framing is checked against a second implementation. The host-side module on a real vhost transport was not run.
- **Final evidence** (`spike-scratch/netns-density-295-ev-dgram/evidence/`): `qemu-corr5-20261017T005034Z`, `qemu-churn5-…T005140Z`, `qemu-stall5-…T005149Z`, `ch-ch5-…T005231Z`, `qemu-tp5-…T005404Z`, `qemu-attach6-…T010206Z`, `ch-chtp4-…T002857Z`. Earlier runs (`corr1` to `corr4`, `ch1` to `ch4`) are kept, including the failed ones.
- **Repeats:** 4 earlier Env 1 correctness runs (`corr1` to `corr4`) and 4 earlier CH batteries (`ch1` to `ch4`) are kept as repeats; no case failed on a module defect. Their harness-only failures are listed under Harness defects.

## 1. Byte-exact, ordered, both ways (0 to 59,000 B, empties, connected and unconnected)

**Hypothesis:** every datagram is delivered once, in order, unmodified, with empties in order. **Prediction:** zero lost, duplicated, reordered or corrupt, and no drops. **Falsification:** any such datagram outside a UDP receive-queue drop.

The verifier compares every full payload against a position-dependent pattern, classifies mismatches by embedded sequence number, and checks each direction separately. The data below is Env 1 (`qemu-corr5`) except where noted.

| Case | n each way | Result | Module counters |
|---|---|---|---|
| sizes 0 / 1 / 1,431 / 4,096 / 4,097, unconnected and connected | 1,500 | all exact | `dg_ab` = `dg_ba` = 2n; size 0: `dg_empty` 6,000 (= 4n); partial resumes 0 / 0 / 0 / 21–28 / 23–24 |
| size 59,000, unconnected and connected | 300 | exact | partial 143 / 144, `dg_wait` 285 / 288 |
| 10,000 mixed sizes (about 10% empty, edge sizes, random 1 to 59,000), unconnected and connected | 10,000 | exact, both directions at once | `dg_empty` 4,050 (= 2 × 2,025 empties, each counted at both forwarders), bytes 180,743,710 and 178,924,658, partial 1,726 / 1,645 |
| Env 2 (CH guest, vhost; host peer is an independent userspace parser): all sizes, plus 1,431 and 59,000 connected | as above | exact | host `rx_ok` equals the sent count |
| Env 2 mixed, unconnected and connected | 10,000 | exact | `dg_empty` 1,997; partial 761 / 698 |

- **Empties:** D15-R4 ("empties may be reordered") is not needed. The strict-order check passed across all empties, including the 4,050 in the mixed run.
- **Reply source** with no rewrite hook: the slot address, as expected.
- **Slot-socket drops:** zero in every lossless case.

## 2. Partial frames

- **UDP → vsock under credit pressure** (Env 2 `credit`): peer window 65,536 (confirmed by `getsockopt`), peer reads 4,096 B per 300 µs, 1,500 mixed frames (26,087,481 B). Result: **6,079 partial-send resumes**, host `rx_ok` 1,500, lost 0, dup 0, corrupt 0. Env 1 gave 6,233.
- **vsock → UDP**, frames written in random 1 to 5,000 B pieces, a third starting with a 1 to 7 B header split (`chunked`): 1,200 of 1,200 exact, `dg_wait` 8,464 in CH and 8,472 in Env 1.
- **Frames sent early:** frames and datagrams queued before the attach are forwarded by the attach-time run (Env 1, `early`, 20 + 20 each way): `peer_ok` 40, `app_ok` 40. The slot queue held 46,080 B and the vsock queue 28,780 B before the attach.

## 3. Oversize and bad frames

| Case | Result |
|---|---|
| Datagrams of 59,001 / 60,000 / 65,000 / 65,507 B interleaved with normal ones, both directions | 80 oversize consumed and counted (`dg_oversize` = 80), 400 + 400 normal datagrams exact, no desync after an oversize |
| One-direction peer case | 20 of 20; Env 2: `dg_oversize` 20, host `rx_ok` 200 of 200 |
| Peer sends header with length 59,001 | pair aborts, end record `kind=2 cause=3 side=1 errno=0`, `dg_invalid` = 1; the 10 earlier datagrams were delivered |
| Peer sends bad magic | same end record |
| Peer clean half-close after 20 frames | `kind=1` (completed) |

## 4. Stalled target: memory flat, drops only at UDP receive queues

**Prediction:** memory flat; datagrams lost only at a UDP receive queue; every datagram accounted for. **Falsification:** growth, or an unaccounted datagram.

| Stall (10 s blast of 1,431 B, nothing reading) | Sent | Delivered on resume | Drops | Memory |
|---|---|---|---|---|
| vsock peer not reading (Env 1) | 2,746,272 | 366 | 2,745,906 at the slot socket | SUnreclaim spread 1,156 KiB, MemAvailable 656 KiB |
| same, Env 2 (real vhost) | 2,772,800 | 363 | 2,772,437 at the slot socket | SUnreclaim 352 KiB, MemAvailable 544 KiB |
| host destination D not reading | 2,321,942 | 218 | 1,162,217 at S + 1,159,507 at D | SUnreclaim 372 KiB |
| guest app A not reading | 2,302,244 | 150 | 1,158,279 at A + 1,143,815 at the host socket | SUnreclaim 4 KiB |

- **Accounting** is exact: sent = delivered + drops in all four rows. The `/proc/net/snmp` `RcvbufErrors` delta equals the sum of socket drops in all four.
- **Env 2 stall cross-check:** the host peer's received count (363) equals the module's `dg_ab` (363).
- **Source citations:** the queue drop is `net/ipv4/udp.c:1721-1731 @ v7.0`; it also accepts an oversized datagram when the queue is empty.
- **Caveat:** `MemAvailable` and `SUnreclaim` are batched per CPU, so they have KiB-level noise. Per-association kernel memory was not resolved (the measured value was 0 to 11 KiB, which is below the noise floor).

## 5. Error cases

| Case | End record of the host/UDP pair (pair 2 in the chain; the pair was the one on the failing UDP half) |
|---|---|
| Closed destination port (ICMP port unreachable) | `kind=2 cause=2 side=0 errno=111` |
| nft output `drop` and `reject` | `kind=2 cause=2 side=0 errno=1` (EPERM) |
| cgroup_skb egress drop (aya-rs) | `errno=1` |
| Guest app closes its socket, then the destination replies (slot-socket send hits the closed port) | guest pair `errno=111 side=0` |

- **Effect on the other pair:** in each chain case the guest-side pair then reads vsock EOF and ends `kind=1`. This module has no vsock end classification (that is V-EV-END's job), so it counts this as "completed". The contract text wants `Aborted{PeerAbort}` there.
- **After the end:** later datagrams from the app wait in the slot queue (8,640 B queued in the ECONNREFUSED case) until the owner releases the slot.

## 6. Reply source (recvmsg4)

A cgroup BPF pair is attached: unconnected sends to O = 198.51.100.7:53 are steered to S, and the reply source is rewritten back to O.

| Run | Reply sources seen (200 replies) | Hook hits |
|---|---|---|
| with recvmsg4 | `{"198.51.100.7:53": 200}` | send hit 200, recv hit 200 |
| control, recvmsg4 not attached | `{"127.0.0.2:47261": 200}` (the slot address) | recv hit 0 |

All 200 datagrams each way were exact.

## 7. Slot release

**Prediction:** abort the pair, discard the slot queue without reading payload, disconnect, then reuse the slot; no stale datagram reaches the next association.

| Variant | Result |
|---|---|
| Abort, discard 1,018 queued datagrams, reconnect | `assoc2_markers_ok` 20, stale 0; slot port 47985 before and after |
| Control: no discard | **1,018 stale datagrams from association 1 reached association 2's peer**; port kept |
| Slot socket bound with port 0 (autobind), disconnect, reconnect | **port changed 59778 → 57681** |

- **Why the third row happens:** `__udp_disconnect` unhashes and zeroes `inet_sport` unless `SOCK_BINDPORT_LOCK` is set (`net/ipv4/udp.c:2209-2235`), and a port-0 bind does not set it (`net/ipv4/af_inet.c:562-563`).
- **Consequence:** a slot's address is the key of the BPF rewrite maps, so slot sockets must bind an explicit port.
- **Churn:** 300 cycles under traffic with a detach at a random moment, 600 pairs: 600 aborted, live pairs 0 after every cycle, worst detach of a pair 4 ms, no oops or warning in dmesg.

## 8. Defects and hazards found

- **Truncated frame hangs the pair (found by reading, reproduced, fixed).**
  - The vsock → UDP path waits for a whole frame to be queued. If the peer sends a header plus 700 of 1,431 payload bytes (or 5 header bytes) and then half-closes, nothing ever completes the frame.
  - Reproduction (`trunc_fix=0`): `dg_wait` 3, the pair did not end in 4 s and only the owner's detach ended it (`end_needed_the_owners_detach: true`); the peer never saw the end.
  - Fix: read `peer_shutdown & SEND_SHUTDOWN` before the queued-bytes count. `peer_shutdown` is set in `virtio_transport_common.c:1443-1448` and `:1250`. With the fix, both cases end at once: `kind=2 cause=3 side=1`, `dg_trunc` = 1.
  - Passes in Env 1 and Env 2.
- **A frame larger than the vsock half's buffer can never be queued (reproduced).**
  - Buffer 32,768 B, frame 59,008 B: three frames delivered, then the pair stayed live for 4 s, the peer's write never finished, `dg_wait` 3.
  - The contract's "always fits the 256 KiB buffer" (`af_vsock.c:199` default) holds only if nobody lowers `SO_VM_SOCKETS_BUFFER_SIZE`.
- **ENOBUFS handling is not needed (hypothesis falsified).**
  - I predicted a dropping qdisc would make the UDP send fail with ENOBUFS and abort healthy associations. A tbf with limit 30,000 B dropped 1,086,869 packets and `err` stayed 0; the pair kept running.
  - Cause: `udp_send_skb` turns ENOBUFS into 0 and bumps `SndbufErrors` unless `IP_RECVERR` is set (`net/ipv4/udp.c:1195-1201`). The residual (sent − slot drops − egressed) equals the qdisc drop count exactly (1,086,869) and equals the `SndbufErrors` delta.
  - The module's `enobufs_drop` branch was never reached (`dg_enobufs` 0) and should not be built.
- **EAGAIN from a held send buffer resumes correctly.**
  - A slow link (tbf 8 Mbit/s, 10 MB limit) holds sent skbs against the UDP send buffer. `egress-eagain`: 10,371,393 B egressed in 10 s, 5,002,308 B in the first 5 s (theory about 5 MB), `stall_dst` 8,554, no abort, residual 0 (the drops all at the source queue).
  - The wake works because overriding `sk_write_space` takes `sock_wfree` off its fast path and onto our hook (`net/core/sock.c:2668-2695`).

## 9. Attach refusals

All 12 refusals match the ADR-0199 table (Env 1, `attach-errnos`):

| Attach | errno |
|---|---|
| UDP not connected | ENOTCONN |
| UDP + TCP, two UDP, two vsock, UDP in stream mode | ESOCKTNOSUPPORT (4 cases) |
| Same socket twice, unknown mode | EINVAL (2 cases) |
| Not a socket / bad fd | ENOTSOCK / EBADF |
| UDP half already attached, vsock half already attached | EBUSY (2 cases) |

The first pair still forwarded 10 of 10 datagrams afterwards.

## 10. Throughput versus the earlier Aya UDP results

Closed loop, one request outstanding per association, the host echoes. "Delivered" counts request plus reply, verified, as in `kernel-vs-aya-benchmark.md`. Median of 3, 8 s windows, 0 corrupt datagrams, 0 socket drops, 0 resend timeouts.

- **Env 1:** loopback vsock, 4 vCPU QEMU, module at both ends.
- **Aya reference:** the corrected Aya UDP cohort (`samples` in `corrected-udp/summary.csv`), a different harness and transport emulation on 4 vCPUs, unrestricted load.
- **Caveat:** generators differ. At population 64 my 4 + 4 threads do not saturate the VM (1.7 to 2.5 busy cores of 4; Aya reports 3.7 to 4.0), so compare CPU per datagram more than raw rate.

| Pop | Payload | evmod dgram dg/s | GiB/s | cores | µs/dg | Aya dg/s | Aya µs/dg |
|---|---|---|---|---|---|---|---|
| 1 | 0 | 39,221 | 0 | 1.12 | 28.5 | 19,001 | 64.9 |
| 1 | 64 | 38,104 | 0.002 | 1.11 | 29.3 | 18,331 | 70.0 |
| 1 | 1,431 | 37,589 | 0.050 | 1.10 | 29.3 | 15,373 | 75.9 |
| 1 | 59,000 | 12,519 | 0.688 | 1.05 | 83.6 | 1,097 (0.060 GiB/s) | 935.6 |
| 64 | 0 | 75,232 | 0 | 1.66 | 22.0 | 114,804 | 32.0 |
| 64 | 64 | 74,543 | 0.004 | 1.64 | 22.8 | 124,600 | 31.1 |
| 64 | 1,431 | 70,561 | 0.094 | 1.66 | 23.3 | 108,125 | 35.9 |
| 64 | 59,000 | 30,264 | 1.663 | 2.49 | 83.9 | 16,941 (0.931 GiB/s) | 234.0 |

At population 64 the earlier kernel-module cohort (shmproxy) measured 71,073 dg/s at 0 B (47.1 µs) and 15,078 at 59,000 B (0.829 GiB/s, 257 µs).

**Env 2** (real vhost transport, module only in the guest, userspace host echo; guest-VM cores only), `ch-chtp4`:

| Pop | Payload | dg/s | GiB/s | guest cores | µs/dg |
|---|---|---|---|---|---|
| 1 | 64 | 29,776 | 0.002 | 0.87 | 29.2 |
| 1 | 1,431 | 28,887 | 0.038 | 0.82 | 28.3 |
| 1 | 59,000 | 8,489 | 0.466 | 0.75 | 88.7 |
| 64 | 64 | 79,792 | 0.005 | 1.56 | 19.6 |
| 64 | 1,431 | 81,631 | 0.109 | 1.68 | 20.5 |
| 64 | 59,000 | 25,212 | 1.385 | 2.16 | 86.4 |

Read: per-datagram CPU is lower than the Aya path at every payload and population, and at 59,000 B the delivered rate is higher, which is the case the sockmap/SEQPACKET path was weakest at. The 0 to 1,431 B populations at 64 deliver fewer dg/s than Aya because my generator, not the forwarder, limits them. I did not rerun Aya.

## Design implications for the EVMOD-DGRAM contract

**Holds as written:**
- The frame layout and the peek-`MSG_TRUNC` / resume / consume-after-send procedure.
- Empties in order, with D15-R4 withdrawn.
- Oversize consumed and counted.
- A bad magic or length aborts.
- The UDP receive-queue bound.
- EAGAIN waits on `sk_write_space`.
- Errors end the association with `SocketError` carrying the errno (ECONNREFUSED 111, EPERM 1).
- The guest slot release (abort, then discard) replaces CELL-RECREATE.
- The recvmsg4 rewrite works unchanged on forwarder-sent replies.
- The attach errnos.

**Must change or be pinned:**
- **D1.** A vsock half whose peer has ended its direction while a frame is incomplete (partial header or partial payload) aborts the pair as `FrameInvalid`, whatever the class of the end. Implement it as `peer_shutdown & SEND_SHUTDOWN` read before the queued-bytes count, or from the sk_state_change callback V-EV-END already needs. Without it the pair is stuck until the owner's abort.
- **D2.** Slot sockets are bound to an explicit port, and the release sequence is abort first, then discard, then disconnect. Add the discard to the contract as load-bearing, with the control result above (1,018 stale datagrams leaked without it).
- **D3.** The vsock half's buffer size must be at least `MAX_GUEST_DATAGRAM_FRAME` (59,008 B), by owner guarantee or by attach refusal (`vsk->buffer_size`, `include/net/af_vsock.h:71`). Otherwise a large frame can never be queued.
- **D4.** Pin the wait for a whole frame to `vsock_stream_has_data` (exported GPL, `net/vmw_vsock/af_vsock.c:1030-1037`) so no payload peek is repeated while a frame accumulates.
- **Coupling to V-EV-END, not decided here:** the contract's "Completed for a classified half-close, Aborted{PeerAbort} otherwise" needs vsock end classification. This spike's unclassified EOF yields `kind=1` for the guest pair after the host pair aborted (section 5). The datagram pair's end class depends on V-EV-END's mechanism.
- **Do not build** ENOBUFS handling or `IP_RECVERR`. `IP_RECVERR` must stay off, because turning it on makes qdisc drops visible as errors.
- Datagram-mode ids and end records were not exercised through the real `/dev/ovd_fwd` context; my pair ids were reused and the harness had to floor end records by sequence number.

## Not covered

- The host-side module on a real vhost transport (the host forwarders ran over loopback vsock).
- IPv6, `UDP_SEGMENT`/GRO.
- kTLS (EVMOD-KTLS) and the real `/dev/ovd_fwd` context semantics.
- A natural vsock RST in datagram mode (V-EV-END).
- A hardware NIC path for the EAGAIN case (a dummy device with tbf stood in).
- Per-association kernel memory (below the vmstat noise floor).

## Harness defects found and fixed along the way

Each of these cost a failed run. None was a module defect.
- The host peer's rate limiter panicked on a `Duration` subtraction (CH `mixed-c`, `ch1`).
- The first slot-release test bound its slot socket with port 0 and lost the port on disconnect (which became finding D2).
- Module pair ids are reused, so an end-record lookup matched the previous pair's record (fixed with an attach-time sequence floor).
- The `truncated-nofix` pass logic was inverted for the peer half.
- The initial `enobufs-abort` / `enobufs-drop` pass criteria encoded the falsified hypothesis.
