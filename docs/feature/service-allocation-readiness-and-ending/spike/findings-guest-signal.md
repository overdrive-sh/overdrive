# Spike findings: guest-signal (V34 and V36) for D13

> Numbering: the feature delta renumbered the D13 spike items to avoid a collision. This file's
> "V34" (no missed listener at boot) is **V36** there, and its "V36" (vsock transport) is **V38**.
> The text below keeps the numbers the spike was run under.

Feature `service-allocation-readiness-and-ending`, proposed decision D13 (ADR-0261 the beacon report, ADR-0265 the kernel observation).
Probe: `spike-scratch/guest-signal/` (self-contained workspace, path-depends on `overdrive-core` and `overdrive-host`; nothing under `crates/` touched).
Driver: `spike-scratch/guest-signal/run.py`. Raw run logs: `spike-scratch/guest-signal/evidence/`.

## Substrate (real guest, real virtio hop)

Run on the native metal host through `cargo xtask metal run --`, as root, under the lease.

```
SUBSTRATE {"uname_r": "7.0.0-38-generic", "machine": "x86_64", "systemd_detect_virt": "none",
  "kernel_file": "/srv/vm/overdrive-testing/kernel", "ch": "/usr/local/bin/cloud-hypervisor",
  "ch_sha256": "6d54fbd2611e692fda00e476fc0723e9502bfd8acd53c8bc46edd96e84f3e842",
  "ch_version": "cloud-hypervisor v53.0.0 ..."}
GS_BOOT uname_r=7.0.0-38-generic        <- the guest kernel (the metal fixture kernel; the guest uname)
```

- Guest: the fixture kernel (`OVERDRIVE_METAL_KERNEL`) plus an initramfs holding busybox, the three vsock modules and the spike guest binary
  (`gsguest`, static, PID 1 after `/init` mounts proc, sys, dev and cgroup2). CH is the vendored-fork build (`--vsock cid=N,backend=vhost-kernel`).
  The guest is NOT the repo's `overdrive-init` (see "What this does not prove").
- Host side: the production `overdrive_host::VsockBeaconIo` (`accept` / `read_line` / `write_line`) and the production `overdrive_core::vm::beacon::BeaconMessage`
  codec classify every line. The spike's own parser handles `LISTEN`.
- Guest eBPF (aya-rs, `bpf/src/main.rs`, built `bpfel-unknown-none` on `nightly-2026-08-05`): sockops arm `TCP_LISTEN_CB` and `CgroupSock` `post_bind4`/`post_bind6` (UDP only),
  attached `Single` at the cgroup2 root, events through a `RingBuf`. Loaded and attached inside the real guest on kernel 7.0.0-38 every boot:
  `GS_HOOKS_ATTACHED ms=78.79 / 75.17 / 61.50 / 51.73 / 72.09` (five sampled boots; 50 to 80 ms, on the pre-`READY` path).

## Verdicts

| Item | Verdict | Environment |
|---|---|---|
| V36 delivery latency | **WORKS** | real guest, real virtio-vsock hop |
| V36 line integrity, one ordered writer | **WORKS** | real guest |
| V36 line integrity, two unsynchronised concurrent writers | **DOESN'T-WORK** (corruption found) | real guest |
| V34 no missed listener at boot | **WORKS** (1000/1000 real boots) | real guest |
| Mid-life guest line vs the exit report in today's host reader | **DOESN'T-WORK today** (read from code) | code citation plus codec run |

Overall for D13: **WORKS, with three constraints D13 must pin** (single ordered writer on the guest, drain before `EXIT`, reader fix on the host).

## V36: vsock transport (`LISTEN <tcp|udp> <port>` interleaved with a concurrent `EXIT`)

Prediction: p99 < 5 ms, zero corruption. Falsification: p99 >= 10 ms or any corruption.

### Latency and ordering (paced, production shape)

Design: one boot, 100 beacon connections, each `READY` then `EXEC ["lat", 1000, ...]`. Per connection the guest writes 1000 `LISTEN` lines closed-loop (each `LISTEN` write is
timed from just before `write(2)` to the host's `ACK <idx>` line being read), and a second guest thread writes exactly one `EXIT <session>` at a pseudo-random point in the
stream, so the `EXIT` lands concurrently with the `LISTEN` stream (positions 1 to 987 observed). 100 x 1000 = 100,000 iterations. Both threads write with one `write(2)` per line and no user-space lock.
The measured quantity is the round trip (guest `LISTEN` write to host `ACK` read in the guest), an upper bound on the one-way send to receive time. The clocks of host and guest are not comparable, so a one-way number was not taken.

```
V36_LAT_SUMMARY {"sessions": 100, "lines_total": 100600, "listen_ok_in_order": 100000, "listen_out_of_order": 0,
 "malformed": 0, "exits_total": 100, "sessions_with_exactly_one_exit": 100,
 "exit_position_in_stream_min_max": [1, 987], "today_codec_unknown_kind": 100000,
 "rtt_n": 100000, "rtt_us": {"min": 35.3, "p50": 45.6, "p90": 50.9, "p99": 138.8, "p999": 283.3, "max": 2175.3}}
```

- Round trip p50 45.6 us, p99 138.8 us, p99.9 283 us, max 2.18 ms, all 100,000 iterations. One-way is below that. p99 is about 36 times under the 5 ms prediction. **Prediction held.**
- 100,000 of 100,000 `LISTEN` lines parsed and arrived in order; 0 malformed; every connection carried exactly one `EXIT`, intact.
- `today_codec_unknown_kind: 100000`: the production `BeaconMessage::from_str` rejects every one of those valid `LISTEN` lines as `UnknownKind` today. Expected (the variant does not exist yet); it confirms the codec change is required and nothing else parses the line.
- Per-session p99 ranged 100 to 350 us (`V36_LAT_PER_SESSION_P99_US` in `evidence/v36-run2.log`).

### Integrity under saturation (two writers, 100,000 `LISTEN` + 100,000 `EXIT` each, no pacing, no ack)

```
sat-free phase wall=3.0s
V36_SAT_FREE {"lines": 200001, "exits": 99994, "malformed_n": 10, "ready": 1, "stats": ["STATS {\"sat_done\":100000,\"partial_writes\":0}"], ...
  "malformed": ["\"LISTEN udp 1EXIT 2493\": unrecognised beacon message kind \"LISTEN\" ...",
                "\"9972\": unrecognised beacon message kind \"9972\" ...",
                "\"LISTEN udp 3EXIT 24197\": ..."]}
sat-lock phase wall=2.9s
V36_SAT_LOCK {"lines": 200001, "exits": 100000, "malformed_n": 0, "listen_ok": 100000, "listen_out_of_order": 0, "exit_out_of_order": 0, ...}
```

- **Unsynchronised concurrent writers corrupt the stream**: bytes of an `EXIT` line landed inside a `LISTEN` line (`LISTEN udp 1EXIT 2493` then `9972`), 10 malformed lines and 6 lost `EXIT`s per 200,000 (the very high `listen_out_of_order` figure
  in the raw record is the harness's index tracker losing sync after a merged line, not extra corruption). The guest reported `partial_writes: 0`: every `write(2)` returned the full length, so the split happens inside the kernel call, not as a short write the caller could see.
  A first, buggy harness run repeated the saturation 770 times and showed 5 to 13 malformed lines every time (`evidence/v36-run1.log`).
- **With one user-space lock around each whole-line write: 200,000 of 200,000 lines intact, in per-sender order, 0 malformed.**
- Mechanism (Linux v7.0 `net/vmw_vsock/af_vsock.c`, read in a sibling workspace's initialised `vendors/linux`, tag v7.0; this worktree's submodule is not initialised):
  `vsock_connectible_sendmsg` holds `lock_sock` (`:2163`) but loops `while (total_written < len)` (`:2206`) and, when the send queue has no credit, runs `release_sock(sk)` / `wait_woken` / `lock_sock(sk)` (`:2228-2230`) inside that loop. A second sender can enqueue between two chunks of the first sender's single line.
  The paced run above never hit a credit wait, so it never interleaved. Credit exhaustion needs a burst bigger than the send credit, which a handful of listen reports never reaches; the corruption is a latent hazard, not a today-reachable one, but it is cheap to make unrepresentable.
- Edge: the host adapter caps a line at 4096 bytes (`overdrive-host/src/beacon_io.rs:30,186-193`). A first harness run sent 7 KB lines and got `InvalidData: beacon line exceeds 4096 bytes` x100, the adapter discarded the line and kept reading. The wire format needs no change; `LISTEN` lines are under 32 bytes.

**V36 verdict: WORKS for latency and ordered delivery. The "zero corruption" prediction is falsified for unsynchronised concurrent writers (DOESN'T-WORK) and holds with a single ordered writer.**

## V34: no missed listener at boot (init's ordering, real guest boots)

Prediction: 1000/1000, every report emitted only after `EXEC`. Falsification: any miss.

Sequence per boot (imitating `overdrive-init`): load and attach both hook families at the cgroup2 root, start the forwarder thread, dial the beacon, `READY`, host writes `EXEC`, guest `fork+exec`s `/gswork`,
whose first application code is `socket(AF_INET,SOCK_STREAM)` + `bind(tcp_port)` + `listen()`, then `socket(AF_INET,SOCK_DGRAM)` + `bind(udp_port)`, then exit. After `waitpid` the guest drains the ring buffer, sends `NOTE`, then `EXIT 0`, then the VM powers off.
Ports are unique per boot. 1000 real VM boots, 8 in parallel, each a fresh VMM and a fresh kernel.

```
V34_SUMMARY {"drain_before_exit": 1, "boots": 1000, "parallel": 8, "wall_s": 125.2, "records": 1000, "no_record_n": 0,
 "tcp_reported": 1000, "udp_reported": 1000, "missed_tcp_cids": [], "missed_udp_cids": [],
 "any_listen_before_exec_write": 0, "listens_total": 2000, "listens_per_boot_distribution": {"2": 1000},
 "listen_after_exit_total": 0, "both_reported_before_exit": 1000, "exit_status_counts": {"0": 1000},
 "boots_with_preexec_zero_note": 1000, "boots_with_preexec_nonzero_note": 0, "malformed_total": 0,
 "natural_poweroff": 1000, "boot_wall_s": {"p50": 1.0, "p99": 1.15, "max": 1.36},
 "exec_write_to_first_listen_us": {"p50": 3093, "p99": 7272, "max": 11949}}
```

- **1000/1000 TCP listens and 1000/1000 UDP binds reported; exactly 2 reports per boot; 0 misses.** Prediction held.
- **Reports only after `EXEC`**, two independent checks: the host saw 0 of 1000 boots with a `LISTEN` received before it wrote `EXEC`; and in the guest, every kernel `bpf_ktime_get_ns` event time was >= the monotonic time `EXEC` was read (`preexec=0` in 1000 of 1000 `NOTE` lines).
- `exec_write_to_first_listen_us` p50 3.1 ms, p99 7.3 ms is EXEC delivery plus fork/exec plus the workload's start plus the report, with 8 VMs booting in parallel. It is not the channel latency (that is V36, under 150 us at p99); do not read it as the cost of the report.
- **Negative control (teeth): hooks attached after the workload ran.** 50 boots with `gs.late=1` (workload first, attach second): `tcp_reported: 0, udp_reported: 0, listens_total: 0` for 50 of 50. The kernel does not replay a listen to a program attached later, so attaching before `READY` is load-bearing, which is exactly D3/D13's ordering.
- **Ordering of the last report against `EXIT` is not free.** Same 300-boot test with the exit path sending `EXIT` straight after `waitpid` and a separate forwarder thread racing it (`drain_before_exit: 0`):
  ```
  V34_SUMMARY {"drain_before_exit": 0, "boots": 300, "tcp_reported": 300, "udp_reported": 300,
   "listen_after_exit_total": 343, "boots_with_listen_after_exit": 185, "both_reported_before_exit": 115, ...}
  ```
  In 185 of 300 boots at least one `LISTEN` reached the host after the `EXIT` (343 lines). Nothing was lost on the wire (all 300 still got both) but the stream order was `EXIT` before the report. Because the BPF hook runs synchronously inside `listen()`/`bind()`,
  a drain after `waitpid` returns every event (the 1000/1000 run above): the fix is purely a guest ordering rule.

**V34 verdict: WORKS (real guest boots, 1000/1000).** Not PARTIAL: no fallback was needed.

## Can today's host beacon reader consume a mid-life guest line without mistaking it for the exit report? No.

Read from the code (all in `crates/overdrive-worker/src/vm_driver.rs` unless stated):

- The adapter itself is fine: `VsockBeaconIo::read_line` is `\n`-framed, buffered, cancellation-safe (`overdrive-host/src/beacon_io.rs:173-207`), and handled my mixed `LISTEN`/`EXIT`/`ACK` stream. The problem is one layer up.
- `spawn` creates the guest-line channel with capacity 1 (`:580`, `mpsc::channel(1)`). `run_beacon_writer`'s read arm forwards EVERY line the guest sends, of any kind, with a blocking `state.lines.send(Ok(Some(line))).await` (`:775`) from inside the `select!` arm body. A second mid-life line while the first is unconsumed blocks the whole arm, and with it the stop and command arms of the same `select!`.
- The only consumer is `drain_guest_report` (`:2165`, called from the exit watcher at `:2252`). Its first, biased arm returns on the FIRST line received, whatever it is, then waits for the VMM exit and returns. `finish_guest_report_line` (`:2209-2217`) maps anything other than `BeaconMessage::Exit` to `(None, vmm_signal)`.
  So a mid-life `LISTEN` is consumed as "the guest's report", classified as no report, and the real `EXIT` that follows is never read: the exit status is lost and the allocation is classified from the VMM signal alone.
- The production codec rejects the line outright today: `today_codec_unknown_kind: 100000` (V36) and `2000` (V34), i.e. `BeaconParseError::UnknownKind`, which `finish_guest_report_line` also maps to `(None, ...)`.

D13 must pin the fix: the reader classifies each line by kind as it arrives; `Listen` goes to the listener set, only `Exit` (and EOF) ends the guest-report wait; `drain_guest_report` must keep reading past non-`Exit` lines until `Exit` or EOF; and the line channel must not be able to stall the `select!` (the `send().await` on a capacity-1 channel from inside the arm).

## Design implications for D13 and D14

1. **Wire (D13 item 2):** `LISTEN <tcp|udp> <port>\n` works as specified, parses with the three-field rule, fits the 4096-byte line cap with a wide margin. No framing change needed.
2. **New contract: one ordered writer on the guest.** Today `overdrive-init::send` takes `&mut File` (`main.rs:654-656`, `write_all`), which makes concurrent writers unrepresentable. A forwarder thread that writes `LISTEN` while the main path writes `EXIT` breaks that. D13 must require that every beacon write goes through one owner (a single writer task fed by a channel, or one lock held across the whole `write_all`). With that, `EXIT` after the final `LISTEN` is a property of one ordered stream; without it the kernel can interleave two lines (measured, 10 per 200,000 at saturation).
3. **New contract: drain before `EXIT`.** After `waitpid`, init drains the listen event ring and sends every pending `LISTEN` before `EXIT`. Then "a `LISTEN` never follows `EXIT`" holds (0 of 1000) and the host's late-subscriber argument (level-readable set) stays simple. Measured counter-case: 185 of 300 boots violated it without the drain.
4. **Reader fix (D13 item 3):** pinned above. Required for correctness the first time any mid-life line exists.
5. **Hook attach order (D3/D13 item 4) is load-bearing, confirmed by the negative control (0/50 reported when attached late).**
6. **Cold-start budget:** loading and attaching the three listen programs cost 50 to 80 ms per boot in this guest, before `READY`. The real init also loads seven existing programs; the listen programs add their share. Relevant to the millisecond cold-start goal; fold into the D13 consequences.
7. **No change needed to D14:** the report arrives about 3 ms after `EXEC` is written and well before the VMM exits; `ProbeResultRow` plumbing can rely on `Listen` being level-readable after delivery.

## What this does not prove

- The guest init is the spike's imitation, not `overdrive-init`. Real init attaches seven other programs and starts the flow-owner thread before `READY`, which costs more than the 50 to 80 ms measured here and adds a second kernel-side workload to the same cgroup root.
- The workload's "first syscalls" are the first application syscalls after the static glibc start-up, not the first instruction after `execve`.
- One guest kernel: the metal fixture kernel `7.0.0-38-generic`. If the appliance guest kernel differs, V37 still applies.
- Round-trip, not one-way, latency (the host and guest monotonic clocks are unrelated). One-way is bounded above by the round trip.
- Saturation corruption was found with plain blocking `write(2)`; the non-blocking/`AsyncFd` writer shape of the host side was not exercised (the host was the sender only for `ACK` lines, one writer).

## Gate recommendation

PROMOTE D13's wire and guest/host ordering to the design, adding three pinned constraints (single ordered writer on the guest, drain-before-`EXIT`, reader reads past non-`Exit` lines and cannot stall the `select!`).

## Cleanup

All CH processes and `gshost` listeners exited (`pgrep cloud-hyp` and `pgrep gshost` both 0 after the last run). BPF links are owned by the guest VMs and died with them; no host cgroups, programs, or vsock listeners were created. Build output remains under the excluded, gitignored `spike-scratch/guest-signal/out/` and `target/` on the metal host.
