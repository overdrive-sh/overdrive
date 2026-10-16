# V-4 findings: guest `close(V_g)` is seen by the host as one `SHUTDOWN 0x3` with nothing earlier. A later `RST` can follow it.

## Verdict: WORKS for the question as asked, with two qualifications

- **The claim holds.** In 2,000 of 2,000 trials the first and only end record the host received before any host action was a single `SHUTDOWN flags=0x3`. No `RST` and no `0x3` reached the host earlier. This held for guest-opened and host-opened flows, after a clean exchange, with unread bytes on either side, and after a half-close.
- **Qualification 1: a second end record can arrive later.** If the host holds unread bytes, its kernel does not answer the `0x3` with a reset. The guest's zombie socket then times out and sends an `RST`. The host receives that `RST` about 8.1 to 8.4 s after the `0x3`, and it did so in 400 of 400 long-phase flows with host-unread bytes. A classifier that treats every end record as an event must treat this `RST` as a no-op on an already-ended flow.
- **Qualification 2: `close()` cannot say clean or abort.** `close(V_g)` emits `0x3` whatever the history. After a clean exchange, with unread bytes, and after a full `0x2`-each-way exchange, the host sees the same `0x3`. Only the earlier record history distinguishes them. Under R2-END's table (`0x3` while any direction is open means abort), a plain `close` after a clean exchange is classified as an abort. Only the "both sides sent `0x2`, then `close`" history leaves no direction open. This is my reading of the R2-END wording in `feature-delta.md`. I did not check it against ADR-0180.

## Environment

| Item | Value |
|---|---|
| Host kernel | `7.0.0-29-generic` (`vendors/linux` is at tag `v7.0`) |
| Guest kernel | `7.0.0-29-generic` (stock bzImage `b51367c7…`, from the guest's own `hello` report) |
| Host | native x86_64 AMD EPYC 8024P. Preflight passed: `systemd-detect-virt=none`, KVM API 12, `/dev/kvm` opened, vmx/svm present |
| VMM | Cloud Hypervisor fork `41a619d19`, `--vsock cid=3270,backend=vhost-kernel`, 2 vCPU, 1 GiB |
| Superproject HEAD | `3bab7b132` |
| Pins (increment-c) | guest probe `65d54ba3…`, fork CH `99dd3d07…`, guest kernel `b51367c7…` |
| Guest agent | static Rust, a stand-in for the guest owner. It never reads or writes unless commanded. |

## Commands

```
python3 spike-scratch/netns-density-295-v4/launch.py increment-a suite=smoke
python3 spike-scratch/netns-density-295-v4/launch.py increment-b suite=full n=100 long_m=50
python3 spike-scratch/netns-density-295-v4/launch.py increment-c suite=full n=100 long_m=50   # verdict run
  -> cargo xtask metal run -- python3 spike-scratch/netns-density-295-v4/increment-c/run.py suite=full n=100 long_m=50
```

- **Increment-a** was a smoke run.
- **Increment-b** was the full run without the `both-halfclosed` cell. It is consistent with increment-c.
- **Increment-c** is the verdict run.
- Jobs ran one at a time. No lease contention occurred. I started and killed no process other than my own.
- Probe code is in `spike-scratch/netns-density-295-v4/`. Evidence is in `…/increment-c/evidence/`.
- Evidence files: `results.json`, `trials.jsonl` (2,000 lines, one per trial), `host-vsock-trace.log` (117,725 parsed packet events, 0 lost), `long-flows.jsonl`, `run.log`.
- Nothing is committed.

## Method

- **Host kernel's own packet record.** I used the vsock tracepoints through tracefs: `virtio_transport_recv_pkt` for every packet the host receives (op and flags), and `virtio_transport_alloc_pkt` for every packet the host sends. `trace_clock=mono` puts them on the same clock as the harness's `CLOCK_MONOTONIC`. Resolution is 1 µs.
- **Host application's view.** An edge-triggered `epoll` registered before the close command, giving wake time and mask. After a 150 ms window I recorded the `poll` mask, bytes still readable (`MSG_PEEK`), the drain result, a write attempt and `SO_ERROR`.
- **Guest close timing.** The guest `close(V_g)` entry is timed on the guest clock and mapped to the host clock by a per-trial ping-pong calibration. Calibration error bound is median 46 µs, max 97 µs. Close-to-record values below about 50 µs are therefore sign-uncertain.
- **Matrix.** 2 establishment directions (`g2h` guest-opened, `h2g` host-opened) × 10 pre-states × N=100 gives 2,000 trials. A guest-side `Q` check confirmed the unread bytes were present before each close.
- **Long phase.** 50 flows per cell (1,000 flows) were closed by the guest at once. The host end was left untouched for 10.5 s. Then I recorded the trace and host state.

### Hypothesis, prediction, falsification

| | |
|---|---|
| Hypothesis | The guest `close(V_g)` makes the guest kernel send exactly one `SHUTDOWN` with `flags=0x3` to the host. The guest sends nothing earlier and no automatic reset arrives before it. |
| Predicted | Host trace shows `…REQUEST/RESPONSE/RW/CREDIT…` and then one `SHUTDOWN 0x3`. Host `epoll` wakes after the record, within tens of µs. If the host has no unread bytes, its own kernel replies `RST`. If it has unread bytes, it does not, and the guest's `close_work` sends `RST` after `VSOCK_CLOSE_TIMEOUT` (8 s). Unread guest bytes are dropped silently with no extra record. |
| Falsified by | An `RST` or `0x3` before the guest `close()`, a close that produces only `RST`, nothing at all, or unread bytes producing a different or extra end record. |

## Per-cell results (N=100 each; all 2,000 trials OK, 0 setup failures)

Each row's result is the same for `g2h` and `h2g` and is identical across all 100 trials of each direction.

| Pre-state at guest close | Host end records, first window | Host kernel sends | Host `epoll` wake | Host after window |
|---|---|---|---|---|
| `clean` (16 B each way, consumed) | `SHUTDOWN:0x3` | `RST` about 4 µs later | `IN\|RDHUP` | read EOF, write `EPIPE`, `SO_ERROR=0` |
| `guest-unread` 1 KiB and 128 KiB | `SHUTDOWN:0x3` | `RST` about 4 µs later | `IN\|RDHUP` | read EOF, write `EPIPE`, `SO_ERROR=0` |
| `host-unread` 1 KiB and 128 KiB | `SHUTDOWN:0x3` | nothing until the host closes | `IN\|RDHUP` | all bytes still readable, then EOF. `EPIPE`, `SO_ERROR=0`. |
| `both-unread` 1 KiB and 128 KiB | `SHUTDOWN:0x3` | nothing until the host closes | `IN\|RDHUP` | all bytes readable, then EOF. `EPIPE`, `SO_ERROR=0`. |
| `own-halfclose-first` (guest `shutdown(WR)` already sent) | earlier `SHUTDOWN:0x2`, then `0x3` | `RST` | `IN\|RDHUP` | EOF |
| `peer-halfclose-first` (host `shutdown(WR)` first) | `SHUTDOWN:0x3` | `RST` | `IN\|HUP\|RDHUP` | EOF |
| `both-halfclosed` (`0x2` each way, then guest `close`) | earlier `0x2`, then `0x3` (a third end record) | `RST` | `IN\|HUP\|RDHUP` | EOF |

Aggregates over all 2,000 trials:
- **End records:** the host received exactly one `SHUTDOWN 0x3` per trial (2,000/2,000) and no `RST` inside the 150 ms window.
- **Earlier end records:** none in 1,800 of 2,000 trials. In the 200 `own-halfclose-first` trials and the 200 `both-halfclosed` trials, the only earlier record was the expected `0x2` that the test itself sent.
- **`close()` itself:** the syscall returned 0 in every trial.
- **Data order:** bytes the guest wrote before `close` were readable after the `0x3`. There was no `ECONNRESET` and `SO_ERROR` was 0. In the trace the `RW` packet precedes `SHUTDOWN`.
- **Unread guest bytes:** they are dropped without any extra record (the `guest-unread` cells show only the same `0x3`). The host cannot tell data was discarded.
- **No `EPOLLHUP` from the `0x3` alone.** `HUP` appears only when the host's own write side was already shut down.

### Latency and order

All values are µs relative to the guest `close()` entry, over the 2,000 trials.

| Interval | n | min | p50 | p99 | max |
|---|---|---|---|---|---|
| `close()` entry to `recv_pkt SHUTDOWN` record | 2,000 | −21.9 (calibration noise) | 23.4 | 68.8 | 85.0 |
| `close()` entry to first host `epoll` wake | 2,000 | 4.1 | 47.4 | 127.0 | 418.5 |
| record to first host wake | 2,000 | 11.0 | 23.2 | 101.5 | 396.7 |
| record to host kernel's `tx RST` | 1,200 | 2.0 | 4.0 | 10 | 80 |

- **Record precedes wake:** in 2,000/2,000 trials. This is the order the R2-END classifier needs.
- **Double wake:** a few trials (2 seen in the 1,800-trial increment-b run) had two `IN|RDHUP` wakes. This is the host kernel's own auto-`RST` and close changing socket state. It does no harm.
- **Extra host `RST` after host close:** after the host app closes its end, the host sends a second `RST` about 150 ms later in the cells where it had already sent one. I did not investigate it.

Example raw trace for one `g2h/clean` flow (timestamps in seconds, mono). The guest sent `0x3` at `.091560`. The host kernel's `RST` follows 5 µs later. The later `RST` at `.242` comes from the host app's `close`, not from the guest.

```
5352647.058243: recv_pkt: 3270:2873046160 -> 2:201001 len=0 op=REQUEST flags=0x0
5352647.058252: alloc_pkt: 2:201001 -> 3270:2873046160 len=0 op=RESPONSE
5352647.058557: alloc_pkt: 2:201001 -> 3270:2873046160 len=16 op=RW
5352647.059022: recv_pkt: ... op=CREDIT_UPDATE fwd_cnt=16
5352647.059186: recv_pkt: ... len=16 op=RW
5352647.059300: alloc_pkt: ... op=CREDIT_UPDATE
5352647.091560: recv_pkt: 3270:2873046160 -> 2:201001 len=0 op=SHUTDOWN flags=0x3
5352647.091565: alloc_pkt: 2:201001 -> 3270:2873046160 len=0 op=RST
5352647.242298: alloc_pkt: ... op=RST            (host app close(), +150 ms)
```

Flow with host-unread bytes. No host `RST` follows the `0x3`. The later `CREDIT_UPDATE` and `RST` at `.0926` come from the host app draining and closing.

```
5352717.909439: recv_pkt: ... -> 2:201301 len=1024 op=RW
5352717.941601: recv_pkt: ... -> 2:201301 len=0 op=SHUTDOWN flags=0x3
5352718.092612: alloc_pkt: ... op=CREDIT_UPDATE
5352718.092707: alloc_pkt: ... op=RST
```

### Long phase: host end untouched for 10.5 s (50 flows per cell, 1,000 flows)

| Cells | Records seen by host | Later end record |
|---|---|---|
| 12 cells with no host-unread bytes (`clean`, `guest-unread`, both half-close cells, both establishment directions) | `rx SHUTDOWN:0x3`, then the host's own `tx RST` | none after 1 s (0 of 600 flows) |
| 8 cells with host-unread bytes (`host-unread`, `both-unread`, 1 KiB and 128 KiB, both directions) | `rx SHUTDOWN:0x3`, then `rx RST:0x0` | `rx RST` at 8.14 to 8.19 s (increment-c) in 400 of 400 flows |

- **Timing of the late `RST`:** it was 8.36 to 8.39 s in increment-b, 8.14 to 8.19 s in increment-c and about 8.21 s in the smoke run. The 8 s is `VSOCK_CLOSE_TIMEOUT`. I did not isolate the 0.14 to 0.39 s excess. Timer-wheel rounding of the delayed work is a plausible cause, unverified.
- **Host state at 10.5 s:** after that `RST`, the host socket polled `IN|OUT|RDHUP`, `SO_ERROR=0`, and all unread bytes were still drainable followed by EOF. The late `RST` neither discards data nor raises an error.

## Kernel citations (`vendors/linux` @ v7.0)

| Behaviour | Source |
|---|---|
| Tracepoint fires on every received packet, before the socket lookup | `net/vmw_vsock/virtio_transport_common.c:1622` |
| Host receive path for guest packets | `drivers/vhost/vsock.c:555` |
| `close` sends `SHUTDOWN` with the full mask unless the local socket is already fully shut down. If the peer already sent the full mask, it replies `RST`. | `virtio_transport_common.c:1280-1296` |
| `close` then schedules `close_work` for `VSOCK_CLOSE_TIMEOUT`, which sends `RST` if no `SOCK_DONE` | `virtio_transport_common.c:1306-1308` (schedule), `:1258-1275` (timeout handler), `:24` (`8 * HZ`) |
| Shutdown flag values (RCV=1, SEND=2, so full mask = `0x3`) | `include/uapi/linux/virtio_vsock.h:94-95` |
| Receiving `SHUTDOWN` with `peer_shutdown == mask`: if no unread data and not `SOCK_DONE`, the receiver replies `RST` and closes. Unread data defers this. | `virtio_transport_common.c:1443-1456` |
| `RST` received calls `virtio_transport_do_close`, which keeps the state `CLOSING` only if there is no unread data | `virtio_transport_common.c:1463-1464`, `:1244-1256` |
| `poll`: `EPOLLRDHUP` from `peer_shutdown & SEND_SHUTDOWN`. `EPOLLHUP` only if the local socket is fully shut down or both sides shut `SEND`. | `net/vmw_vsock/af_vsock.c:1250-1259` |

The traces match the code path:
- The `0x3` arrives first.
- With no unread bytes the host replies `RST` within about 4 µs.
- With unread bytes the host stays silent and the guest `RST` follows about 8 s later.
- The host `EPOLLHUP` appears only when the host had already shut its write side.

## Design implications

1. **R2-END's `0x3` rule is sound for the guest-owner close.** The `0x3` is always the first end record. It is the host's only reliable end signal on this path, since `EPOLLHUP` is not raised. The classifier should key on the record, not on `EPOLLHUP`.
2. **A trailing `RST` must be a no-op.** An `RST` can arrive up to about 8.4 s after a classified `0x3` when the host has unread bytes. Per-flow end handling must be idempotent. It must not reclassify as abort, must not double-release the flow slot or double-charge a counter, and must not touch a recycled flow id.
3. **Clean end needs `0x2` from both sides first.** `close(V_g)` after a clean exchange is `0x3` with a direction open, which R2-END classifies as abort. A guest owner that wants a clean full end should half-close its write side (`0x2`) and observe the host's `0x2` before `close`. The `both-halfclosed` cell, with `0x2` each way before `close`, is the only history in which `0x3` follows two `0x2` records.
4. **Unread guest bytes are silently lost.** The guest kernel emits no abort signal when it closes with unread bytes. The drain goal and sent counter (R2-DRAIN) must account for this. Nothing at the transport level tells the host the guest never read what it sent.
5. **Unread host bytes survive the end record.** Data the guest sent before `close` stays readable, followed by EOF, even after the `0x3`. The host owner must read to EOF and not discard on the `0x3`.
6. **The host's own socket does not time out.** In 4 of 6 timeout-bound combinations the late `RST` is the only reset the host sees. The host kernel itself never aborts the host socket within 10.5 s.

## Limitations

- **Observation method:** I used the ftrace tracepoint, not the production raw-tracepoint BPF classifier from V-31. It reads the same tracepoint arguments. The ring and BPF path of the classifier was not exercised. This is the same tracepoint, but a loss mode of the BPF classifier is outside this spike.
- **Resolution:** the trace has 1 µs resolution and the cross-clock error bound is 46 to 97 µs. Close-to-record times under about 50 µs are indicative only. The record-before-wake ordering and the ordering within the host trace are exact.
- **Single harness:** one VM, one guest kernel, a single-threaded stand-in guest (not the production owner), stock vsock modules.
- **Not tested:**
  - `SO_LINGER` on the guest.
  - Guest `close` racing a host `close`.
  - `close` while guest TX data is in flight and unacknowledged.
  - Guest process death or SIGKILL (V-13 covers it).
  - A reused flow id after a trailing `RST`.
  - Host-initiated `close` ordering.
- **Late RST excess:** the 0.14 to 0.39 s excess over 8.000 s was not isolated.

## Gate recommendation

**PROMOTE V-4 as holding**, on two conditions to carry into DELIVER:

- **(a)** The classifier and flow table treat a trailing `RST` after a classified `0x3` as a no-op, for at least 9 s after the `0x3`.
- **(b)** The design states that a guest owner wanting a clean (non-abort) end must emit `0x2`, observe the host's `0x2`, and only then `close`. Plain `close` classifies as abort under R2-END.

Evidence is at `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-v4/increment-c/evidence/`.

Cleanup: the module set before and after is identical (`modules-before.txt` equals `modules-after.txt`). Tracefs events are disabled and `trace_clock` is restored. No cloud-hypervisor process remains.
