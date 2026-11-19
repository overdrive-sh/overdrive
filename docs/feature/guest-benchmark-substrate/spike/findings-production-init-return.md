# V-B2 (review finding B3) — Returning a framed sample block from a production-`overdrive-init` guest, read after VMM exit

**Verdict: the return path WORKS, but the framing grammar as the design pins it (DDD-7: the block starts with `bench-series-begin:` and nothing precedes it) is FALSIFIED.** The block is complete and correct when the host reads the console only after EXIT, power-off and VMM exit: 180 of 180 replicates returned every end line, and no byte was missing at the tail. But the production `overdrive-init` writes its own log lines to the same console in unterminated fragments, and in **24 of 60 replicates (40%)** one of those fragments was the open console line when the program's first line arrived, so the first `bench-series-begin:` line did not start a line and the strict host parse found no begin line. One leading `\n` before the block removes it (0 of 30 failed). No production-code change is needed. The fix is a one-character change to the program's output grammar (or a tolerant begin-line match) and goes back to DESIGN as a finding, below.

Host and guest: native Metal host, `uname -r` = `7.0.0-38-generic` (AMD EPYC 8024P); the guest reports the same `7.0.0-38-generic` (its own `/proc/sys/kernel/osrelease`, printed by the program). Commit `5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b`, working tree dirty (docs and spike only). Run 2026-11-19. No file under `crates/` or `xtask/` was changed.

Probe: `spike-scratch/guest-bench-init-return/` (`src/lib.rs` host half, `src/init_return_guest.rs` in-guest program, `benches/benchmarks.rs`). Evidence kept: `spike-scratch/guest-bench-init-return/results/` (records and reports) and `spike-scratch/guest-bench-init-return/evidence/<ts>-<pid>/` (every replicate's full console, `rep-NNN-console.console.log`, and `summary.tsv`).

## Hypothesis, prediction, falsification (written before the first run)

- **Hypothesis:** a guest whose PID 1 is the production `overdrive-init` (`GuestInit::OverdriveInit`) can run an operator program that drives the guest flow owner over loopback vsock, prints its framed series block on its stdout (the guest console) followed at once by exit, and the host, reading the console only after EXIT, power-off and VMM exit, gets the complete block: every begin line, value line and end line, with count and FNV-1a checksum intact, no tail loss, and no kernel printk line inside a block.
- **Prediction:** 60 of 60 replicates (30 per arm, a fresh guest each) return both series intact; the block is about 38 KB or more; the console after the last end line carries only later kernel/init lines (power-off messages), proving the console was drained to the end; the echo path moves bytes in both directions; the whole cycle is about 1.3 to 2.5 s.
- **Falsification:** any replicate with a missing or short block (tail loss), a checksum or count mismatch, a kernel line interleaved inside a block, a reverse-direction (app to vsock) failure, or a console that is incomplete when read after VMM exit, in the production-init guest even though the scripted-peer guest passed.

Result against the triple: tail loss, count or checksum mismatch, interleaving inside a block, reverse-direction failure and an incomplete console were all absent. The falsification fired on one clause the triple did cover ("a missing block") by a cause the triple did not predict: a head splice.

## What the probe does

Host half (mirrors `guest_lane::run_in_guest_cell_attempt`; `Holder` and `read_line` are private there, so re-implemented): `GuestImageSpec { init: OverdriveInit, forwarder_module, loopback_module, extra: [GuestFile init-return-guest] }`; `HostListeners::bind`, token on the kernel command line, `kit.boot` (512 MiB, 2 vCPUs, as the lane), READY, `EXEC [overdrive-guest-peer run-with-modules vsock_loopback.ko -- /opt/spike/init-return-guest ...]`, wait for `EXIT`, `vm.wait_exit(60 s)`, and only then `vm.console()`. A fresh image and guest per replicate. The cwd adaptation of the prior spike is repeated.

In-guest program (`init_return_guest.rs`): loopback vsock `VMADDR_CID_LOCAL` to `GUEST_INBOUND_PORT` with `FlowRequest::TcpAccept`, an application listener on `workload_addr:8080`, timing on `Instant` (CLOCK_MONOTONIC).
1. `accept_ns`: 20 unrecorded then 500 timed flows (request written to the application's `accept()` returning).
2. One paired flow: the application first writes 16 unprompted bytes that the vsock side must read (**app to vsock-side direction**), then 200 unrecorded and 5,000 timed closed-loop 64-byte round trips: write on the vsock side, an application thread echoes, read the reply on the vsock side, bytes compared (**vsock to app and back**).
3. It prints, in one `write_all` to stdout, the DDD-7 grammar (`bench-series-begin: <metric> <n>`, `bench-values: <metric> <v,...>` lines of 16 values, `bench-series-end: <metric> <n> <fnv1a-hex16>`), plus trailer lines, and exits at once (`process::exit`). No sleep, no drain step.

Host parse is strict: a begin, its value lines and an end line must each start a console line; the end line's `n` must equal the begin line's `n` and the parsed count, and the FNV-1a over the little-endian `u64` bytes must match. Non-matching lines inside a block are counted as interleaved.

Size: the block is about 41 KB (about 5,500 values); the console is about 72 KB with the kernel and init output.

## Commands

```
cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build        # run bd489dce58ae0763cc46e589
cargo xtask metal run -- cargo xtask guest-kit-prepare                            # run cd2b8fd282b675e101ee1979
cargo xtask metal run --benchmark guest-init-return --spike guest-bench-init-return          # run b560f548b3a447af1e79e12f
cargo xtask metal run --benchmark guest-init-return-lead --spike guest-bench-init-return     # run cf2a4bea07be31301b460ec2
cargo xtask metal pull spike-scratch/guest-bench-init-return/results/guest-init-return/b560f548b3a447af1e79e12f spike-scratch/guest-bench-init-return/evidence
cargo xtask metal pull spike-scratch/guest-bench-init-return/results/guest-init-return-lead/cf2a4bea07be31301b460ec2 spike-scratch/guest-bench-init-return/evidence
```

`kmod-load` was not needed: the guests load `ovd_fwd` and the loopback module themselves from their images. One run (`fe14615ba1df3679403102f0`) exited 64 before any replicate because I named a family the crate had not yet declared (my edit had not been applied); it is not evidence.

## Observed

### Run 1 (`b560f548b3a447af1e79e12f`): block with a header line first. 90 of 90 replicates ok, 0 failed

All 90 consoles read after VMM exit: `exit=0`, both series intact (count and checksum recomputed on the host), `both_dirs=true`, `interleaved=0+0`, `after_block_lines` 5 to 8.

```
V-B2 rep 0 arm=init-return-control cid=0x1003000 exit=0 rtt_intact=true() accept_intact=true() both_dirs=true interleaved=0+0 after_block_lines=5 console_bytes=71051 guest_kernel=7.0.0-38-generic cycle_ms=3380
...
V-B2 rep 89 arm=init-return cid=0x1003059 exit=0 rtt_intact=true() accept_intact=true() both_dirs=true interleaved=0+0 after_block_lines=5 console_bytes=71719 guest_kernel=7.0.0-38-generic cycle_ms=3043
```

In this run the program printed a one-line `init-return-guest-kernel:` header before the first begin line. **35 of 90 consoles show that header line spliced** onto an unfinished `overdrive-init` log line (the first run's blocks survived only because that header took the splice):

```
overdrive-init: guest-flow forward-end id=init-return-guest-kernel: 7.0.0-38-generic
overdrive-init: guest-flow forward-end id=ForwardId(3) outcome=Aborted { init-return-guest-kernel: 7.0.0-38-generic
overdrive-init: guest-flow forward-end id=ForwardIdinit-return-guest-kernel: 7.0.0-38-generic
```

### Run 2 (`cf2a4bea07be31301b460ec2`): `bare` (DDD-7 as written: begin line first) against `lead-newline` (one `\n` first). 90 of 90 replicates ok

| arm | replicates | `rtt_ns` block intact | `accept_ns` block intact | echo both directions | kernel/other line inside a block |
|---|---|---|---|---|---|
| `bare` (60 replicates in the `bare` bucket; the report tabulates 30 as the arm, the other 30 are presumably the runner's A/A control draws) | 60 | 60 | **36 (24 lost, 40%)** | 60 | 0 |
| `lead-newline` | 30 | 30 | **30 (0 lost)** | 30 | 0 |

Every lost `accept_ns` block failed the same way: `0 begin lines`. The value lines and the end line were intact; only the begin line did not start a line:

```
overdrive-init: guest-flow handle-end id=ForwardId(2) outcome=Aborted { cause: PeerAbort { side: A } } tcp_pairs=1 pending_end_closes=0
overdrive-init: guest-flow deferred-close stream id=ForwardId(2) clear_linger=false outer=GuestPairSocket(Handle { seam: SeamInstance(1), id: SocketId(831) }) vsock=Guest...
overdrive-init: guest-flow forward-end id=bench-series-begin: accept_ns 500
bench-values: accept_ns 1924687,1973691,...
```

and the same position in a `lead-newline` replicate, intact:

```
overdrive-init: guest-flow forward-end id=ForwardId(3) outcome=Aborted { cause: PeerAbort
bench-series-begin: accept_ns 500
bench-values: accept_ns 2371507,1978468,...
```

The tail of that console (the program's last lines, then init's remainder of the fragment, then power-off), showing the console is complete to the reboot message when read after VMM exit:

```
775:bench-series-end: rtt_ns 5000 91bb64984041834b
776:direction-check: app_to_vsock=ok
777:direction-check: vsock_to_app_echoed=ok
778:init-return-guest-kernel: 7.0.0-38-generic
779:init-return-workload-ns: 883741320
780:init-return-print-ns: 628047095
781: { side: A } }
782:overdrive-init: guest-flow handle-end id=ForwardId(3) outcome=Aborted { cause: PeerAbort { side: A } } tcp_pairs=1 pending_end_closes=0
783:overdrive-init: guest-flow deferred-close stream id=ForwardId(3) ...
784:overdrive-init: group-complete elapsed_ms=1533
785:overdrive-init: poweroff-requested elapsed_ms=1534
786:overdrive-init: beacon boundary tx EXIT
787:[    2.249192] ACPI: PM: Preparing to enter system sleep state S5
788:[    2.250121] reboot: Power down
```

### The questions you asked

- **Intact-block rate, all 180 replicates.** `rtt_ns`: 180 of 180. `accept_ns`: 156 of 180 as the grammar is pinned (24 lost to the head splice, all in the `bare` arm); 156 + 24 recoverable = 180 of 180 once the begin line starts a line (`lead-newline` 30 of 30; the run-1 header line absorbed the splice in the other 90). The blocks were never short or corrupt: no count or checksum mismatch in 180 replicates.
- **Tail loss.** None. Every replicate has the end lines and 5 to 13 console lines after the last `bench-series-end:` (init's remaining log lines, `ACPI: PM: Preparing to enter system sleep state S5`, `reboot: Power down`). Reading only after VMM exit gave a complete console, with no flush or drain step in the program.
- **Kernel printk interleaving.** None observed inside any block: `interleaved_lines_in_blocks` is 0 in 180 of 180. The 40 KB block is written in one `write_all`; no kernel line and no init line landed between its first and last byte in 180 replicates. (The init fragments that did land always arrived before the program's first byte or after its last, consistent with the write being applied as a unit; this is observed, not read from the kernel source: the Linux tree at `vendors/linux` was not checked out in this workspace, so no `path:line @ v7.0` is cited.) A kernel message that interleaved mid-block was not provoked by this workload and is not excluded by it.
- **Echo path, both directions.** Works. App to vsock side: 16 unprompted bytes read intact in 180 of 180. Vsock to app and back: 200 + 5,000 closed-loop 64-byte round trips per replicate, every reply byte-equal to its request, 180 of 180 replicates with 0 mismatches. This is the direction the existing in-guest cell does not assert.
- **Whole-cycle time per replicate** (run 1 / run 2 medians): 2.80 s / 2.75 s (IQR about 0.16 to 0.2 s). Parts (run 1, medians): image build 64 ms; launch to READY 1.10 s; EXEC to EXIT 1.58 s; EXIT to VMM exit 40 ms. Inside EXEC to EXIT (run 2, guest-measured): the workload (520 flows and 5,200 round trips) 0.89 s median (0.76 to 1.37 s), and the console print 0.65 s median (0.54 to 0.66 s) for the 41 KB block, about 63 KB/s, matching V-B's 59 KB/s and PD-6's 0.65 s estimate. The design's 1.3 to 1.5 s expectation plus 0.65 s return is 2.0 s; the measured cycle is about 0.75 s longer because the 520 timed accepts and 5,200 round trips cost 0.89 s of workload.
- **Round trip and accept (guest CLOCK_MONOTONIC; percentiles histogram-derived, error at most 3.125%).** `rtt_ns`: p50 40.7 to 51.3 us, p90 53 to 59 us, p95 59.8 to 63.7 us, p99 70.8 to 76.5 us, 150,000 values per arm per run (the A/A comparisons call the arms indistinguishable). `accept_ns`: p50 0.94 to 0.95 ms, p95 1.96 to 1.97 ms, p99 2.3 to 2.9 ms (bimodal, about 1 ms and 2 ms).
- **`uname -r`.** Host `7.0.0-38-generic`; guest `7.0.0-38-generic` (guest-reported in every readable replicate).

## Cause of the head splice

`overdrive-init` logs to the same console from its own process with unbuffered fragments: a line such as `forward-end id=ForwardId(2) outcome=Aborted { cause: PeerAbort { side: A } }` is emitted as several writes (its `{:?}` pieces). Our program's last flows close just before it prints, so init is still writing the end-of-flow log lines as the program prints. If init's last fragment has no newline yet when the program's first byte lands, the program's first line is glued to it. The remainder of init's line arrives after the program's whole write. The splice only ever **prefixes** the first line; it never cut a value line, an end line or a second block's begin line, in 180 replicates.

The DDD-7 text says lines that do not begin with the prefixes are ignored. A begin line that starts mid-line is one of those lines, so the block's metric is `record_failure("<metric>", "the guest returned no intact bench-series block")` in 40% of replicates, and the first series is the one hit. The second block's begin line follows the first block's end line in the same write, so it is not exposed.

## Production-code changes required

None. A fix is in the program's output grammar, which is test-support surface the crafter owns, but DDD-7 pins that grammar and the host parse rule, so this is a design finding and the user should rule on it:

1. **Preferred (verified 0 of 30 lost):** the program writes one `\n` before its first block line. The partial init line then ends, the begin line starts a line, and the strict line-start parse is unchanged. DDD-7's "console lines that do not begin with those prefixes are ignored" still holds (the orphaned init fragment and the blank line are ignored).
2. A tolerant begin match (find `bench-series-begin:` anywhere in a line and parse from there) would also have recovered every lost block here, but it weakens the strict grammar and would accept text from an unrelated line, so it is the weaker choice.
3. Do not rely on a header line to absorb the splice (run 1): it works only while a sacrificial line comes first.

## Edge cases and limits of this evidence

- 30 `lead-newline` replicates, 0 lost: with 30 trials this bounds the splice-after-fix rate at about 10% (95%), not zero. A fix that removes the cause by construction, rather than one that merely reduces its rate, is the leading newline (it terminates whatever fragment is open), but a mid-block splice cannot be excluded by this sample alone. A cheap host-side defence is the count-and-checksum check DDD-7 already pins: a spliced value or end line fails that metric rather than shortening the series.
- The workload ends with flow teardown immediately before the print, which is the worst case for the splice. A program that sleeps before printing would hide it, and would add a timer to the design for no reason.
- The probe's sample-return mechanism (one stdout write) is not the design's `forwarding-bench` subcommand; its grammar, sizes and framing are DDD-7/DDD-8's, its flows are the same production owner path.
- Kernel printk lines were not seen between block lines. Whether a printk can interleave under heavier kernel logging was not tested.
- The console is the VMM's file; the only reader here is the host after VMM exit, as the design specifies.

## Design implications

1. The post-exit console read is sound: 180 of 180 consoles were complete to the `reboot: Power down` line. No drain step, flush, sleep or alternative return path is needed (PD-6 option (a) stands).
2. DDD-7's grammar needs one amendment: the program writes a leading `\n` before the first `bench-series-begin:` line. Add it to the ADR-0330 grammar and to the host-half acceptance (a console whose first block line follows an unterminated `overdrive-init` fragment must parse).
3. The two-direction echo through the production owner works, so DDD-8's `forwarded_round_trip_ns` is measurable as designed. The cycle is about 2.75 s (about 83 s per 30 replicates), not the 1.3 to 1.5 s plus 0.65 s the design estimated; the bound in PD-5 (30 s) still holds by a wide margin.

## Gate recommendation

PROMOTE the console return from a production-`overdrive-init` guest, **with the leading-newline amendment to DDD-7/ADR-0330 as a precondition**; do not promote the grammar as currently written. Probe `spike-scratch/guest-bench-init-return/` is throwaway and is deleted when the implementation lands.
