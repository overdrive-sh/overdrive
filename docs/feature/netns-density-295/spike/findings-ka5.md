# K-A5: the guest observes a host end of a guest-opened control connection

**Verdict: WORKS.** Every case delivered an observable end to the guest with no guest-side timer. All 4,750 trials ended correctly. Neither a hang nor a false end occurred.

K-A5 (`feature-delta.md` ~8259, `owner_flows.qnt:116`) holds for guest-opened connections. This is the same mechanism V-24 proved for host-opened flow vsocks.

I did not commit anything. The probe and its evidence are untracked in the working tree.

## Environment

- **Host:** native x86_64 AMD EPYC 8024P, `systemd-detect-virt` = `none`, KVM API 12. The fail-closed preflight passed.
- **Kernels:** host `7.0.0-29-generic` and guest `7.0.0-29-generic`. This is the pinned 7.0 line, and `vendors/linux` is at tag `v7.0`.
- **VMM:** Cloud Hypervisor fork `41a619d19` (v53.0 + 2), launched with `--vsock cid=3260,backend=vhost-kernel`. The guest is 2 vCPU, 1 GiB, stock kernel from `/srv/vm/overdrive-testing/kernel`, with an initramfs carrying the stock vsock modules. This is V-24's harness shape.
- **Superproject HEAD:** `b71c77488`.
- **Where it ran:** inside a real microVM under the metal lease, via `cargo xtask metal run`.

## Commands run

```
python3 spike-scratch/netns-density-295-ka5/launch.py increment-c suite=full n=250
  -> cargo xtask metal run -- python3 spike-scratch/netns-density-295-ka5/increment-c/run.py suite=full n=250
```

Earlier increments:
- **`increment-a`:** failed because the guest `main.rs` was missing. That was my error.
- **`increment-b`:** a smoke run with 3 trials per case, which passed.
- **`increment-c`:** the verdict run.

A transient `No route to host` killed my first attempt's SSH session and left its lease holder (pid 2057248) orphaned. A retry then sat waiting on the lease. I killed only my own orphaned holders (2057248, 2057243 and the waiting 2057701/2057696) and re-ran. No other spike's run was touched.

## Probe

- **Probe code** is in `spike-scratch/netns-density-295-ka5/`, its own Cargo workspace:
  - `guest/` (static Rust)
  - `holder.py`
  - `increment-{a,b,c}/run.py`
  - `launch.py`
- **Evidence for the verdict run** is in `increment-c/evidence/`:
  - `trials.jsonl` (4,750 per-trial lines)
  - `results.json`
  - `reports.jsonl`
  - `run.log`
  - `artifact-pins.json`
  - serial and CH logs
- **Guest side:** in each trial the guest `connect()`s to CID 2 and then waits with no timer. In `read` mode it blocks in `recv()`. In `epoll` mode it blocks in `epoll_wait(-1)` on `EPOLLIN|EPOLLRDHUP`.
- **After the wake** the guest records the first return, drains the socket, tries a write, and reads `poll` bits and `SO_ERROR`.
- **Latency** is from the host's action to the guest's wake. Host and guest `CLOCK_MONOTONIC` are aligned by 5 ping-pongs per trial. The offset error bound has a median of 35 µs and a maximum of 91 µs over 3,750 calibrations.
- **Abandon command:** the guest accepts an "abandon" command that does a local `shutdown()` to reap a hung thread. It never fired.

## Hypothesis, prediction, falsification

All cases share one hypothesis: the host's `close()` or process exit sends a vsock SHUTDOWN(RCV|SEND) (or an RST when there is no listener). The guest's connected socket records it and wakes. These are the per-case predictions and falsifications.

| Case | Prediction | Falsified if |
|---|---|---|
| 1. Host accepts, then `close()` | `recv()` returns 0, or epoll reports `IN\|RDHUP`; write gives `EPIPE` | any trial has no wake |
| 2. Host `close()` with `SO_LINGER{1,0}` | Same as case 1. Source reading says vsock linger with timeout 0 returns immediately, so it is not an RST. | `ECONNRESET` or a missing wake |
| 3. SIGKILL of the host process holding the accepted socket (3b: holding an unaccepted queued connection) | Process exit closes the socket, which sends SHUTDOWN, so the same as case 1, slower by exit time | no wake |
| 4a. No listener | `connect()` fails fast with `ECONNRESET` | a hang or a timeout |
| 4b. Backlog full, `listen(0)` plus one queued filler | The second `connect()` gets `ECONNRESET`. Closing the listener ends the queued filler. | a hang or no end |
| 4c. Listener closed with a queued, unaccepted child | The child is released, so SHUTDOWN reaches the guest | no end |
| 4d. Race: listener closed during the in-flight `connect()` (0–1000 µs jitter) | Either `connect()` fails with `ECONNRESET`, or it succeeds and then ends | any hang |
| 5. Host ends while the guest is blocked in `read()` or idle in `epoll_wait`; also data then close | Same as case 1 in both wait modes | no wake |
| Control: unrelated idle connection | No end event, ever | any `HUP`/`RDHUP`/`ERR`/EOF |

## Results (`increment-c`, n=250 per row)

Latency is in µs, host action to guest wake.

| Case | Mode | Trials | Guest observed | Latency p50 / p99 / max |
|---|---|---|---|---|
| 1 close | read | 250 | `read()=0` (EOF), poll `IN\|OUT\|RDHUP`, write `EPIPE` | 149 / 228 / 306 |
| 1 close | epoll | 250 | `epoll=IN\|RDHUP`, then EOF, `EPIPE` | 156 / 260 / 304 |
| 2 linger{1,0} | read | 250 | `read()=0`, `EPIPE` | 151 / 244 / 303 |
| 2 linger{1,0} | epoll | 250 | `IN\|RDHUP`, EOF, `EPIPE` | 137 / 240 / 245 |
| 3 SIGKILL, accepted | read | 250 | `read()=0`, `EPIPE` | 866 / 1251 / 1286 |
| 3 SIGKILL, accepted | epoll | 250 | `IN\|RDHUP`, EOF, `EPIPE` | 884 / 1207 / 1310 |
| 3b SIGKILL, unaccepted | read | 250 | `read()=0`, `EPIPE` | 796 / 960 / 1016 |
| 3b SIGKILL, unaccepted | epoll | 250 | `IN\|RDHUP`, EOF, `EPIPE` | 764 / 1003 / 1140 |
| 5c 16 bytes then close | read | 250 | `read()=0` after the 16 data bytes | 189 / 270 / 336 |
| 5c 16 bytes then close | epoll | 250 | `IN\|RDHUP`, EOF | 179 / 273 / 277 |
| 4a no listener | epoll | 250 | `connect()` = `ECONNRESET` (guest-local) | 63 / 192 / 208 |
| 4b backlog full, refused | epoll | 250 | `connect()` = `ECONNRESET` | 76 / 122 / 154 |
| 4b filler after listener close | epoll | 250 | `IN\|RDHUP`, EOF, `EPIPE` | 102 / 230 / 242 |
| 4c listener closed, queued | epoll | 250 | `IN\|RDHUP`, EOF, `EPIPE` | 201 / 330 / 332 |
| 4c listener closed, queued | read | 250 | `read()=0`, `EPIPE` | 225 / 317 / 376 |
| 4d race | epoll | 500 | 424 connected-then-ended (`IN\|RDHUP`); 76 `connect()` = `ECONNRESET` | 51 / 87 / 202 (end); `connect()` 66 / 193 / 211 |
| Control: idle, no host action | read | 250 | no end (`STILL-OPEN`), no `HUP`/`RDHUP` | n/a |
| Control: idle, no host action | epoll | 250 | no end (`STILL-OPEN`), no `HUP`/`RDHUP` | n/a |

- **Unrelated idle connection:** held open for the whole run (about 4 minutes, thousands of other connections ending). It saw 0 false-end events, and a final byte from the host was still received.
- **Failures:** 0 `NO-END`, 0 `SETUP-FAIL`, 0 `FALSE-END`.
- **Leftovers:** the host `ss --vsock` after the run shows only the command and control sockets. The harness left no leaked modules.
- **Case 4d** took both branches, so the race was genuinely exercised.

## Kernel source (`vendors/linux` @ v7.0)

- **Host close sends SHUTDOWN with both directions.**
  - `virtio_transport_close` (`virtio_transport_common.c:1281-1310`) sends it, at `:1295`.
  - It goes out via `virtio_transport_shutdown` (`:1084-1098`).
  - The host's `release` reaches it from `__vsock_release` (`af_vsock.c:952-992`).
  - SIGKILL takes the same path through process-exit fd release.
- **Guest receipt:** `virtio_transport_recv_connected` `case SHUTDOWN` (`:1443-1461`).
  - It sets `peer_shutdown`.
  - It resets and closes when the peer shutdown is complete and no data is left (`:1449-1452`).
  - It fires `sk_state_change` (`:1461`).
- **`do_close`** (`:1244-1256`) is the path an RST takes (`:1463-1464`). It sets `SOCK_DONE` and `peer_shutdown = SHUTDOWN_MASK`.
- **Guest-side observables.**
  - `vsock_poll` sets `EPOLLRDHUP` when `peer_shutdown & SEND_SHUTDOWN` (`af_vsock.c:1256-1258`).
  - `EPOLLIN` comes from `sk_is_readable` (`:1261`).
  - `vsock_connectible_wait_data` breaks on `peer_shutdown & SEND_SHUTDOWN` (`:2326-2329`).
  - `recv` returns 0 when `sk_shutdown & RCV_SHUTDOWN` (`:2441`).
  - `EPIPE` on write is observed. I did not trace its source line.
- **`SO_LINGER{1,0}` is not an RST.** `vsock_linger` returns immediately when the linger time is 0 (`af_vsock.c:1153-1157`), and the SHUTDOWN is sent first (`:1295`). Case 2 is therefore identical to case 1 on vsock, which matches the measurements. An established guest socket never sees `ECONNRESET` from a host close.
- **No listener, or a full backlog.**
  - The host replies RST via `virtio_transport_reset_no_sock` (`:1175-1214`).
  - It is called at `:1544-1545` for a full accept queue, and at `:1630-1633` for no bound socket.
  - The guest's `recv_connecting` `case RST` sets `ECONNRESET` and `sk_error_report` (`:1345-1348`, `:1356-1362`).
- **Listener close with queued children.** `__vsock_release` loops `vsock_dequeue_accept` and releases each child (`af_vsock.c:983-987`). A child is inserted as connected before it is queued (`virtio_transport_common.c:1589-1590`), so its release sends SHUTDOWN.
- **Packets from the guest** reach `virtio_transport_recv_pkt` from vhost at `drivers/vhost/vsock.c:555`.

## Design implications

- **No change to the control protocol or the model.** K-A5 can be marked proven (this spike) for guest-opened control connections. `GuestSessionLossObserved` is deliverable by the kernel with no guest timer.
- **Wording for DISTILL and the K-A5 cell:** the guest owner must watch for an end through a readable EOF or `EPOLLRDHUP`. It should not wait for `ECONNRESET` or `EPOLLERR`.
- **`ECONNRESET` is a connect-time outcome only.** It means refused, no listener, or backlog full. An established end is always EOF or `RDHUP`.
- **A reset (`SO_LINGER{1,0}`) is indistinguishable from a graceful close.** The design must not rely on the guest telling the two apart.
- **Process-kill latency is about 1 ms**, dominated by host process exit. A normal close is about 0.15 ms.

## Limits

- Kernel is 7.0.0-29 on both sides, which is the pinned line.
- The guest owner is a userspace model, not the production guest owner.
- Not probed:
  - Host kernel panic or VMM death. K-A2′/K-C4 and V-22/V-30 cover those; a lost session whose VM ended has no guest to observe it.
  - Accept-queue overflow beyond the single-filler `listen(0)` case.
  - Guest-owner restart.
  - Host `shutdown()` without `close()`.
- Latency is a calibrated cross-clock estimate with a ±35 µs median error and a ±91 µs maximum.

**Gate recommendation:** PROMOTE. K-A5 is discharged for guest-opened control connections. Carry the EOF/`RDHUP` and no-`ECONNRESET` wording into DISTILL.
