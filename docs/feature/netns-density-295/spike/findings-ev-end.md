# V-EV-END findings: end classification with module-to-module chaining on a real microVM

**Verdict: WORKS, with one falsified sentence in the contract.** The EVMOD-END classification rule holds unchanged across a real Cloud Hypervisor microVM with the forwarder on both sides. The rule is computed per received SHUTDOWN or RST in `sk_state_change`, plus `sk_err` for the vsock orphan sweep. One part of the contract text is falsified: "a TCP half needs no classifier". A TCP reset on a half whose source direction already ended is never noticed unless `sk_error_report` reads `sk_err`.

In the final configuration every pass/fail check below passed, with `unclassified_eof` at 0. Two items are measurements rather than proofs: the sweep-path record ordering (see "Watch record ordering" under What holds) and the 18 failures in the first smoke runs, which were harness errors and a contract gap (see Design implications, items 1 and 2).

## Substrate and artifacts

All paths below are under `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-ev-end/`.

- **L0 (metal host):** AMD EPYC 8024P, `uname -r` 7.0.0-38-generic, `systemd-detect-virt` = `none`, KVM api 12, `kvm_amd nested=1`. The preflight fails closed on all of these.
- **L1 (private QEMU/KVM VM, host role):** 7.0.0-29-generic, 3 GiB, 4 vCPU. It loads `vhost_vsock` and `ovd_fwd.ko`, and runs the orchestrator `evend host`, the remote peer R and the host context.
- **L2 (Cloud Hypervisor fork microVM):** `--vsock backend=vhost-kernel`, guest kernel 7.0.0-29-generic, 512 MiB, 2 vCPU. It loads `ovd_fwd.ko` and runs `evend agent`. The CH binary sha is `eadddf1b…`.
- **Module placement:** after every leased run the physical host's module names and links were unchanged, with no `ovd_fwd`, `evmod`, `tls` or `shmproxy` entry (the last check was in `finalD`).
- **Sources:**
  - `module/ovd_fwd.c` and `module/ovd_fwd.h` implement the proposed ABI. It is stream mode and watch only: `/dev/ovd_fwd` contexts, 32-byte end records preallocated with the pair, read/poll, `VERSION`/`ATTACH`/`WATCH`/`ABORT`/`ABORT_ALL`/`INVENTORY`/`PAIR_STATE`, a release that aborts all and queues nothing, and `module_init` with no `module_exit`.
  - `harness/evend.c` is the harness. `driver/run.py` builds, assembles the images and runs QEMU. `launch.py` is the lease launcher.
- **Evidence:** `evidence/mainA-20261016T232657Z`, `mainB-20261016T234656Z`, `mainC-20261017T011444Z`, `finalD-20261017T014256Z`, hazard controls `hz1-20261017T003546Z`, `hz2-20261017T003643Z`, `hz16-20261017T004507Z`, `hz4-20261017T004842Z`, `hz8-20261017T004932Z`. Each has `serial.log`, the executed sources, the artifact pins and `result.json`. Progress file: `out/REPORT-progress.md`.
- **Permanence:** `/proc/modules` shows `ovd_fwd … [permanent]` on both sides, and `delete_module` returned -1 with `errno=16` (EBUSY) on both host and guest (`kernel/module/main.c:818-826 @ v7.0`).

## Hypothesis, prediction, falsification

- **Hypothesis:** per-transition classification in the callbacks, chained host and guest, yields exactly the pinned R2-END outcome at every far end.
- **Prediction:** every abort reaches the other far end as a reset, never EOF. Every clean end reaches it as EOF after the last byte. Each id yields exactly one record. Transitions after an end change nothing. `unclassified_eof` stays 0. A watch record is queued before the holder's read or poll observes the end.
- **Falsified by:** any EOF after a reset, any reset after a clean full end, a truncated or corrupt stream, a missing, duplicate or extra record, or a pair left live.
- **Result:** the prediction held in every case once the module read `sk_err` for TCP halves. Before that fix it was falsified in 2 shapes.

## Results with the final module and harness (hazards off)

Stream bytes are splitmix64 streams verified at the receiver. A "trial" is one flow, or one batched chain in the race shapes.

### Clean ends and trailing events

| Shape | Trials | Pass | Expected, and what was checked |
|---|---:|---:|---|
| clean, guest first / remote first / simultaneous | 3×1000 | 3000 | Both records `completed`, `moved_ab`/`moved_ba` equal the sent bytes, both far ends EOF with the full stream |
| clean, then owners `SHUT_RDWR` both vsock halves (trailing 0x3 and RST) | 1000 | 1000 | No new record, `queued_ends` 0, counters unchanged |
| CLEAN-FULL-END, host run lagging 15 ms: 0x2 then 0x3 reach the host callback before its run | 1000 | 1000 | Host `completed`, R reads EOF with every byte (not reset) |
| CLEAN-FULL-END, guest run lagging 15 ms (mirror) | 1000 | 1000 | Same, at the guest |

### Resets and aborts through both forwarders

The "ex" columns are the expected host and guest record.

| Shape | Trials | Pass | Expected host record | Expected guest record | Far end |
|---|---:|---:|---|---|---|
| guest app RST, both directions open | 1000 (+200) | all | aborted, PeerAbort, side A | aborted, SocketError, side A, errno 104 | R: verified prefix then reset |
| guest app RST, only host→guest open | 1000 (+200) | all | PeerAbort, side A | SocketError, side A, **EPIPE (32)** | R: TCP_CLOSE (reset) |
| guest app `close()` with unread data | 1000 | all | PeerAbort, side A | SocketError, side A | R: reset |
| remote RST, both directions open | 1000 (+200) | all | SocketError, side B, 104 | PeerAbort, side B | G: prefix then reset |
| remote RST, only guest→host open | 1000 (+200) | all | SocketError, side B, **EPIPE** | PeerAbort, side B | G: reset |
| remote `close()` with unread data | 1000 | all | SocketError, side B | PeerAbort, side B | G: reset |
| host `abort` | 1000 | all | Detached | PeerAbort, side B | both far ends reset |
| guest `abort` | 1000 | all | PeerAbort | Detached | both far ends reset |
| guest abort, direct 0x3, host direction closed | 1000 | all | PeerAbort | Detached | R reset |
| guest abort, 0x2 then 0x3, host direction open | 1000 | all | PeerAbort | Detached | R reset |
| host abort, direct 0x3, guest direction closed | 1000 | all | Detached | PeerAbort | G reset |
| host abort, 0x2 then 0x3, guest direction open | 1000 | all | Detached | PeerAbort | G reset |
| stalled pipeline (6 MiB offered) then guest RST / remote RST / host abort | 3×200 (+50) | all | as above | as above | reset reaches a non-reading far end |

The "(+200)" counts come from `finalD`.

The errno split from `finalD`:
- Host records: ECONNRESET 200 (remote RST, both directions) and EPIPE 200 (remote RST, guest→host only).
- Guest records: ECONNRESET 250 (guest RST, both directions, plus the stalled shape) and EPIPE 200 (guest RST, host→guest only).

### Concurrent and exactly-once checks

| Shape | Trials | Pass | What was checked |
|---|---:|---:|---|
| 48 pairs reset at once, guest side / remote side | 2×50 rounds (4,800 pairs) | all | One record per id on both sides, correct cause and side, far ends reset |
| context release (host / guest), 16 live pairs | 2×50 rounds (1,600 pairs) | all | Far ends reset, no record queued by the released context, the other module classifies PeerAbort, fresh context empty |
| `race-random`: 1–3 random events per chain (RST, abort, send, shut, in any order), then clean closers | 1500 sequential + 3200 in batches of 16 + 2880 in batches of 48 | all | Every chain terminates (no hang), no stream mismatch, exactly one record per id on each side, nothing live afterwards |
| VMM `kill -9`, 32 pairs with traffic both ways | 40 rounds (1,280 pairs) | all | See VMM kill below |
| VMM `kill -9`, 16 pairs and 16 watches | 40 rounds | all | See VMM kill below |
| VMM `kill -9` after every pair completed cleanly | 30×16 | all | Sweep touches completed pairs' sockets, no record, counters unchanged |
| VMM `kill -9` with every pair half-closed (G→R ended, R at EOF) | 30×32 | all | R reaches TCP_CLOSE (a reset), never stays in CLOSE_WAIT |

**VMM kill.** Each pair gets exactly one host record `aborted` / SocketError / side A / errno 104. Each watch gets one `watch aborted` / SocketError / 104 record. No remote peer read a clean EOF (0 of 1,280 plus 640). Inventory was zero afterwards and `unclassified_eof` did not move.

### Watch (dialled V_h) shapes

The holder's read or poll result is recorded with the instant record check.

| Shape | Trials | Pass | Record at holder's observation |
|---|---:|---:|---|
| guest half-close, poll holder | 1000 | all | present at poll wake in 1000 |
| guest half-close, blocking `read()` holder | 1000 | all | present when `read()` returned 0 in 1000 |
| guest reset, poll holder | 1000 | all | present at poll wake in 1000, **holder read EOF, not an error, in 1000/1000** |
| guest reset, blocking holder | 1000 | all | present at read, holder read EOF in 1000/1000 |
| guest half-close, then reset | 1000 | all | first class `kind 3` recorded, no second record |
| host drop (`abort` of the watch) | 1000 | all | record Aborted/Detached, guest app reset, guest pair aborted/PeerAbort |
| holder shuts first, then guest half-closes (clean) | 1000 | all | `kind 3`, guest pair `completed` |
| holder shuts first, then guest resets (direct 0x3, host direction closed) | 1000 | all | `kind 4` / PeerAbort |
| VMM kill with 16 watches | 640 | all | `kind 4` / SocketError / 104; record present when the holder observed the end (640 of 640) |

After each dial the owners close their vsock halves and the harness checks for a second record: 0 in all runs.

## Negative controls (hazard variants)

Each control proves a check has teeth. The runs use short waits and small counts, so the failing numbers are counts, not rates over 1,000.

| Variant | Trials | Failures |
|---|---|---|
| hazard 1: forward every vsock end as a FIN (rejected alternative b) | reset shapes 6–10 each | 100%: "R/G saw clean EOF after a reset". Clean shapes pass 10 of 10. VMM kill fails 1 of 2 rounds |
| hazard 2: classify at the EOF read from accumulated `peer_shutdown` | 100 each | CLEAN-FULL-END host-lag 100/100, guest-lag 100/100 (a clean flow is aborted and R reads a reset), 0x2-then-0x3 shapes fail. Only the CLEAN-FULL-END and 0x2-then-0x3 shapes fail; direct 0x3 and plain clean shapes pass |
| hazard 16: TCP `sk_err` not read in `sk_error_report` | 20 each | `g-rst-hg-only` 20/20 and `r-rst-gh-only` 20/20 fail; `g-rst-both-dirs` passes; watch half-close-then-reset fails 10/10 |
| hazard 4: watch record queued from the work item | 300 each | record absent at the holder's end observation in 8% to 68% (reset/poll 53 of 300, half-close 25, reset-poll 204, record-at-end 181) |
| hazard 8: original callback called before queueing the watch record | 300 each | **0 failures in 1,200**; ordering is held by the socket lock on the `sk_state_change` path |

## Kernel facts (vendors/linux @ v7.0)

- `net/vmw_vsock/virtio_transport_common.c:1443-1464`: a SHUTDOWN sets `peer_shutdown` and calls `sk_state_change`. A 0x3 with an empty queue sends an RST reply and runs `virtio_transport_do_close` (`:1244-1253`), which calls it again.
- `net/vmw_vsock/virtio_transport_common.c:1280-1325`: `close()` sends SHUTDOWN 0x3 unless the peer already sent 0x3, in which case it replies RST. This is the source of every trailing 0x3 and RST.
- `drivers/vhost/vsock.c:721-751`: the orphan sweep sets `SOCK_DONE`, `peer_shutdown = SHUTDOWN_MASK`, `sk_err = ECONNRESET`, and calls `sk_error_report`. It takes no sock lock and does not call `sk_state_change`, so `sk_err` is the only signal on this path. It is invoked from the device release at `:753-768`.
- `net/vmw_vsock/af_vsock.c:1230-1330`: for connectible sockets `vsock_poll` takes `lock_sock` before returning, which is why a poller cannot return before the callback completes.
- `net/vmw_vsock/af_vsock.c:2326-2328`: the reader sees end-of-stream only once the queue is empty.
- `net/ipv4/tcp_input.c:4890-4902` (`tcp_done_with_error` calls `sk_error_report`) and `:4905-4930` (`tcp_reset`): the error is **EPIPE in CLOSE_WAIT, ECONNRESET otherwise**. This is the EPIPE seen in the half-closed shapes.
- `net/ipv4/tcp.c:5087-5141` (`tcp_abort`) and `:3400-3425` (`tcp_disconnect`, RST for a zero-linger close in ESTABLISHED, CLOSE_WAIT, FIN_WAIT1, FIN_WAIT2): the `SO_LINGER{1,0}` close used for the RST shapes sends the reset.
- `kernel/module/main.c:818-826`: an init without an exit gives -EBUSY.

## Design implications for the EVMOD contract

### What holds

- **EVMOD-END's rule, unchanged.** Per-transition classification in `sk_state_change` works chained across both modules. "Own direction closed" is `sk->sk_shutdown & SEND_SHUTDOWN` on the vsock half, read under the socket lock. The 2×2 of {direct 0x3, 0x2 then 0x3} × {receiver's direction open, closed} behaves exactly as the table says, at both the host and the guest forwarder.
- **Classifying in the callback is necessary, not just stylistic.** Hazard 2 turns a clean flow into a reset in 100% of the lag shapes. "Forward the end as a FIN" (alternative b) resets nothing.
- **Exactly one record per id**, and abort propagation across both modules.
- **Pair record ordering.** A pair's record is queued after its abort actions, so a reader of the record knows the far-end reset was sent.
- **Watch ordering and the EOF ambiguity.** The record is queued before the holder observes the end, in 100% of 6,640 observations. An aborted dial reads as EOF at the holder in 100% of 2,000 trials, so `GuestStream` must never infer a clean end from `read() == 0`; the record is the only discriminator.
- **Context release**, **VMM kill** and **permanence** behave as pinned.

### What must change

1. **Add a TCP rule to EVMOD-END (mandatory).** The text "A TCP half needs no classifier: … `recvmsg` returns the error" is falsified for a half whose source direction already ended.
   - After a FIN the forwarder stops reading that half. A later RST sets `sk_err` and calls `sk_error_report`, but nobody calls `recvmsg` or `sendmsg`.
   - Result without the rule: the pair stays live and the far end stays in CLOSE_WAIT indefinitely, while a direct TCP connection would have been reset (hazard 16: 20/20 and 20/20).
   - The rule: in `sk_error_report` of a TCP half, `sk_err != 0` decides abort with `SocketError { side, errno }`, exactly like the vsock half. The module in `module/ovd_fwd.c` implements this.
2. **The end record's errno is ECONNRESET or EPIPE for a TCP reset.** Owners and `ForwardOutcome` consumers must treat both as a reset (EPIPE is the CLOSE_WAIT case, `tcp_input.c:4905-4930`).
3. **Pin the watch callback order and the `abort` semantics.**
   - Queue the watch record before calling the original callback, in both `sk_state_change` and `sk_error_report`. This is correct by construction on the sweep path, which takes no socket lock. The contract's "originals first" also held in 1,200/1,200 on the locked path, but that rests on `lock_sock` ordering, not on the callback order.
   - `abort(id)` of a watch is not specified. The spike's behavior: `shutdown(SHUT_RDWR)` on the vsock, then record `Aborted { Detached }`, and the guest pair aborts.
4. **`GuestStream`'s end contract needs one sentence.** `Ok(0)` is valid only after the watch record says `HalfClose`; an abort must surface as `Err(ConnectionReset)` from the record.
5. **Attach-time classification is untested.** The module classifies a vsock half's `peer_shutdown` at attach under `lock_sock` (needed for EVMOD-PAIR's early end). That path was not exercised here.

## Limits and not covered

- **Not covered by V-EV-END:** datagram mode, kTLS, the attach errno matrix, 16,384-pair close storms and 10^6-id reuse (V-EV-CTL), attach-before-data (V-EV-PAIR), and guest PID-1 crash.
- **Payload size and nesting:** payloads are ≤ 32 KiB per direction except the stalled shapes (6 MiB offered). The stack is nested virtualization at 4 L1 vCPUs and 2 L2 vCPUs, so timing is not representative of production latency. Classification correctness does not depend on timing, and the work-delay hazard shapes force the ordering windows explicitly.
- **Kernel version:** guest and L1 are Ubuntu 7.0.0-29-generic, not necessarily the exact appliance build. VMM kill ran 40 rounds per shape, not 1,000.
- **Watch ordering on the sweep path is measured, not proven.** The orphan sweep takes no socket lock, so 640 of 640 on this path is evidence, not a guarantee. "Queue before the original callback" is the only order that is correct by construction there.
- **Maintenance incident, disclosed.**
  - I killed a launcher locally while its remote job (the first hazards script) was running. The remote payload survived the SSH drop and kept starting 3 GiB QEMU jobs, one at a time, for about 45 minutes **without the lease**. The old `hz1-…000337Z`, `hz2-…001841Z`, `hz16-…003345Z`, and the `hz4-…004849Z` and `hz8-…004940Z` evidence directories are from that run. They overlapped my own later leased job and possibly other spikes.
  - No module was loaded on the physical host, and a later check found no stray `ev-end` process. I cite only the leased runs above. Remote scripts now use `timeout`.
  - The cleanup job I ran to check for strays killed its own shell because `pgrep -f` matched itself (`launcher_rc` 255). No other process was affected.

## Gate recommendation

**V-EV-END: PROMOTE to acceptance.** The EVMOD-END classification rule is confirmed, and the amendments are small and local to the contract text. Amend EVMOD-END with changes 1 and 2 (the TCP `sk_error_report` rule and the errno), and pin changes 3 and 4 in the watch and `GuestStream` text. No redesign is needed. Re-run this harness against the production module and `KernelFlowForwarder` as the A-40 conformance driver. The harness's hazard variants make a ready-made set of negative controls for that driver.
