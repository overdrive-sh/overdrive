# Spike V2 findings: K-PS2 / V-PSO2(a), a released flow's end precedes the next request

## Verdict

**WORKS (K-PS2 holds). 0 reordering in 8,275 timed trials.**

- In every trial the guest kernel received the released flow's end record before any byte of the later request.
- The same order held for the later request's connection request when it was a fresh dial.
- The user-space check agreed: the earlier flow always read as ended once the later request was readable.
- A deliberate reversed-order control was flagged 250 of 250 times, so the oracle can see a reorder.

**Gate recommendation: PROMOTE.** Candidate B ("ends before claims") may rest on K-PS2. PSO-2 is not returned to DESIGN on account of K-PS2.

## Environment

| Item | Value |
|---|---|
| Host | native x86_64, `systemd-detect-virt` = `none`, KVM API 12 |
| Host `uname -r` | `7.0.0-29-generic` |
| Guest `uname -r` (reported by the guest) | `7.0.0-29-generic`, 2 vCPUs, CID 3270 |
| VMM | Cloud Hypervisor fork, `--vsock cid=3270,backend=vhost-kernel`, v53.0 + 2 fork commits |
| Fork submodule HEAD | `41a619d191f6472f79a3f22bcb9576c2a516906c`, branch `overdrive/vhost-kernel-vsock`, clean |
| Superproject HEAD | `3bab7b132ee81cd04a210ca207c281fda72f9b66` |
| Kernel source | `vendors/linux` at tag `v7.0` |
| Guest kernel sha256 | `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682` |

- The harness is the ka5 one (launch, boot, preflight).
- Guest tracefs mounted and the tracepoint was available. No LOST EVENTS in any trial.
- Modules were left clean after the run: `modules_added_and_left: []`.
- Only one metal job ran at a time, under the lease.

## Hypothesis, prediction, falsification

- **Hypothesis:** the host's release of flow A (close of V_h, which sends SHUTDOWN) and the host's next request B share one host-to-guest queue. The guest therefore receives A's end before any byte of B.
- **Prediction:** 0 violations over all trials, for every release kind, B mode and gap in 0–1000 µs.
- **Falsification:** one trial where any B request byte is readable at the guest before A's end record, as seen in either of two ways:
  - the guest kernel's packet receive order, or
  - A not yet ended in a user-space sample taken after B's bytes were readable.

## Method

- **Exact oracle.** The guest enables `vsock:virtio_transport_recv_pkt` with `trace_clock=global`.
  - It fires at `net/vmw_vsock/virtio_transport_common.c:1622 @ v7.0`, at the top of `virtio_transport_recv_pkt`. That is the site the design says the guest `flow_end_classifier` hooks.
  - Trace order is the kernel's receive order, so "A's end packet earlier than B's first payload packet in the trace" is the same fact as "the classifier record precedes B's bytes".
  - A trial is `ORDERED` only if the end-op index (`SHUTDOWN` or `RST`) is below the first B `RW len>0`. For a fresh dial it must also be below B's `REQUEST`.
- **Observability oracle.** A guest spinner polls with timeout 0, in this order each iteration:
  1. B's listener, B's socket, then B's data.
  2. When B's first byte is readable, it samples A.
  - A's end is monotone, so A being not ended after B was readable proves A was not ended at that moment.
  - "Ended" means `POLLRDHUP`, `POLLHUP` or `POLLERR`.
  - This check is one-sided: it can only catch a reorder if the guest wakes in between.
- **Release kinds (A):**

| Kind | Release |
|---|---|
| v1 | dial-drop: host dials A, writes the request, closes at once |
| v2 | paired-drop: host dials A, the guest acks the request, then the host closes |
| v3 | host closes a guest-opened flow |
| v4 | host aborts a guest-opened flow with `SO_LINGER{1,0}` |

- **B modes:**
  - `fresh`: connect B after the release, then send.
  - `pre`: B already established, so the request is sent right after the release. This is the worst case, one `send()` after one `close()`.
- **Timing:** back to back (no added gap), uniform random 0–1000 µs busy-wait, and a fixed grid of 25, 100, 250, 500 and 1000 µs.
- **Control:** B's request sent first, then A released, which must be flagged `REORDERED`.
- **Passes:**
  - idle: 250 trials per cell;
  - loaded: 12 host busy-loop processes plus 2 guest burner threads on 2 vCPUs, 150 per cell.

## Commands

Launched via the retrieval wrapper, which runs `cargo xtask metal run --` and fetches the evidence:

```
python3 spike-scratch/netns-density-295-v2/launch.py increment-a suite=smoke
python3 spike-scratch/netns-density-295-v2/launch.py increment-b suite=full n=250
```

- Increment-a (54 trials: 48 pass, 6 control detected) was the smoke run. Increment-b is the verdict.
- Increment-b's inner command was `cargo xtask metal run -- python3 spike-scratch/netns-density-295-v2/increment-b/run.py suite=full n=250`.

## Results (increment-b)

Totals: `{'n': 8525, 'results': {'CONTROL-REORDER-DETECTED': 250, 'PASS': 8275}}`. Timed trials: 8,275 PASS, 0 violation, 0 incomplete, 0 setup failure.

Pass cells ran 250 trials idle and 150 loaded; grid cells ran 125. Every cell returned `trace: ORDERED` and `user: A-ENDED` for all its trials.

| Cell group | Trials | Result | Trace margin, A-end to B-data (µs, min / p50) |
|---|---|---|---|
| idle v1–v4 × fresh/pre × back-to-back | 8 cells × 250 = 2000 | all PASS | `pre` 2/3 (v1) to 12/23 (v4); `fresh` 58/73 to 73/88 |
| idle v1–v4 × fresh/pre × jitter 0–1000 µs | 2000 | all PASS | 3/556 (v1-pre) up to 82/600 (v3-fresh) |
| idle fixed-gap grid (v1-pre, v3-pre, v1-fresh × 5 gaps) | 15 cells × 125 = 1875 | all PASS | scales with the gap: gap 25 µs → 3/32 (v1-pre); gap 1000 µs → 942/1031 (v1-pre) |
| loaded v1–v4 × fresh/pre × back-to-back and jitter | 16 cells × 150 = 2400 | all PASS | tightest: v1-pre 1/2; v2-pre 7/10 |
| control, reversed order: v1-pre | 125 | all `CONTROL-REORDER-DETECTED` | −1015/−475 |
| control, reversed order: v3-fresh | 125 | all `CONTROL-REORDER-DETECTED` | −1008/−553 |

Other counts:

- The smallest margin was **1 µs**, in the loaded v1-pre cells. That is the trace-clock resolution, not a gap in the ordering. The verdict uses the trace sequence, not the timestamp.
- A sample of the spinner at B-data time: `{'IN|OUT|RDHUP': 8275}`.
- On fresh dials the spinner also sampled A at B's accept: `A-ENDED` in 3,817 of 3,817.
- In 8 trials the spinner had not yet accepted A when B's data was readable. A was accepted then and was already ended.
- Every first end record for A was `SHUTDOWN`: 2850 (v1), 1600 (v2), 2225 (v3), 1600 (v4). `SO_LINGER{1,0}` also yields SHUTDOWN, not RST.
- Control: in the reversed runs the spinner often saw A "ended" anyway (5 of 250, with the host closing A within 1 ms). The trace caught all 250. This shows the trace is the exact oracle and the spinner is a one-sided cross-check.
- Host-side measured gaps (`gap_actual_us`):
  - back to back: about 1–20 µs;
  - `rand` cells: up to 1000 µs;
  - one loaded v2 cell overshot to 2725 µs, because the busy host preempted the Python process.

Pasted guest trace, v1 `pre`, trial 5501 (A's ports are 316503 and B's 316504; fields are seq, ts, src_port, dst_port, dst_cid, len, op). The A→SHUTDOWN is received before B's first RW:

```
[5004,"21.330818",1369208354,316504,3270,0,"REQUEST"]      B established earlier
[5005,"21.330931",1369208355,316503,3270,0,"REQUEST"]      A
[5006,"21.331009",1369208355,316503,3270,9,"RW"]           A request
[5007,"21.331023",1369208355,316503,3270,0,"SHUTDOWN"]     A end   <-- before
[5008,"21.331026",1369208354,316504,3270,9,"RW"]           B request bytes (3 µs later)
```

Pasted control trace, reversed order (B sent first, A released after):

```
[57131,"247.159212",...,332628,3270,9,"RW"]      A request
[57132,"247.159226",...,332629,3270,9,"RW"]      B request bytes
[57133,"247.159661",...,332628,3270,0,"SHUTDOWN"] A end   <-- after B: REORDERED, flagged
```

## Citations (`vendors/linux` at `v7.0`)

- `net/vmw_vsock/virtio_transport_common.c:1280-1295`: `virtio_transport_close` sends SHUTDOWN on close of an established flow, synchronously.
- `drivers/vhost/vsock.c:296-297`: `vhost_transport_send_pkt` queues the packet on the device's one `send_pkt_queue` tail.
- `drivers/vhost/vsock.c:125-140`, `:238`: the consumer takes packets from the head, and a stalled packet is requeued at the head, so FIFO holds.
- `net/vmw_vsock/virtio_transport.c:624`, `:680`: the guest `rx_work` handles the ring in order, one packet at a time through `virtio_transport_recv_pkt`.
- `net/vmw_vsock/virtio_transport_common.c:1622`: the tracepoint and the classifier site.
- `net/vmw_vsock/virtio_transport_common.c:1443-1452`: the SHUTDOWN handler sets `peer_shutdown`.
- `net/vmw_vsock/af_vsock.c:1250-1258`: that state surfaces as `EPOLLHUP` / `EPOLLRDHUP`, which is what the spinner sampled.

## Design implications

- K-PS2 holds on a microVM. A's end is queued before B's request and is processed in order, so the guest can know at B's install which earlier flows the host ended. No request-side release-horizon field and no wire change are needed. PSO-2's typed refusal can still be dropped on this ground (candidate B).
- It holds across all four release kinds the design lists (dropped `GuestStream`, dial dropped after its request write, host abort of a guest-opened flow with and without linger-0), for fresh and pre-established B, over gaps from a few microseconds to 1 ms, and under CPU contention.
- A host abort with `SO_LINGER{1,0}` produces a SHUTDOWN record like a plain close. The classifier sees the same end packet kind in both.

## Limits

- One guest, 2 vCPUs, one kernel.
- The probe is a user-space model, not the production guest owner. The classifier site is observed through the tracepoint, not a BPF raw tracepoint. Tracing may perturb timing slightly.
- The host side is Python. The tightest back-to-back is about 2–3 µs from `close()` to `send()`. A faster production path would only shrink the gap further.
- Trace timestamps resolve to 1 µs. The trace sequence order, not the timestamp, decides the verdict.
- The case "classifier skipped, end pending as V_g readiness (A4)" is not exercised. It needs the production raw-tracepoint build (A-34, DELIVER).

## Files

All under `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-v2/`:

- `launch.py`
- `guest-init.sh`
- `rsync-owned-filter.sh`
- `guest/src/main.rs`
- `increment-a/run.py` and `increment-b/run.py`
- `increment-b/evidence/`:
  - `results.json`
  - `trials.jsonl` (8,525 lines)
  - `reports.jsonl` (full per-trial trace lists)
  - `run.log`
  - `commands.jsonl`
  - `source-manifest.json`
  - `artifact-pins.json`
  - `vm3270-serial.log`
  - `vm3270-ch.log`
  - `vm3270-ch.stdout`

Nothing under `crates/`, `specs/` or `docs/` was touched. Build output is git-ignored (`out/`).
