# ADR-0330 — A guest returns its samples on its console as framed series lines

## Status

**Accepted (2026-11-19).** Approved by the user as PD-6 (console). DDD-6 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md). Both candidate transports were measured through the benchmark form with a scripted-peer guest (validation item V-B, held). The console return from a production-`overdrive-init` guest, read after the VMM exits, was proven over 180 fresh-guest replicates (V-B2, held, [findings](../../feature/guest-benchmark-substrate/spike/findings-production-init-return.md)). The console's throughput is a property of this mechanism, not a scope limitation (user ruling 2026-11-19).

## Context

The in-guest program measures with the guest's own clock and must hand every sample to the host half, which records them. The in-guest lane's result channel is the guest console: the operator command's stdout reaches the VMM's serial file, which the host half reads once the VMM has exited (`CellRun.console`). The VMM writes that file whether or not the host is reading it (`crates/overdrive-host/src/vmm.rs:296`). The console is also what a failed guest leaves behind, and its retention is required (US-GBS-03).

Measured on the Metal host (Linux 7.0.0-38-generic, medians, count and checksum intact in every case): the console carries about 59 KB/s, so 1,000 samples take 61 ms, 100,000 samples (307 KB) 5.15 s and 1,000,000 samples (3.07 MB) 52.3 s; vsock carries 8 KB in 8.2 ms, 800 KB in 14.9 ms and 8 MB in 88 ms, but needs a host listener read concurrently with the run (the guest blocks once the vsock buffer fills while the host waits for the program to exit).

## Decision

The in-guest program prints, after its timed phase, one framed block for its series metric (begin line, value lines, end line carrying a checksum of the values), preceded by one `\n` so the block starts on a fresh console line. It exits 0 when it ran to the end and non-zero when it could not drive its flow; there is no per-failure line. The leading `\n` is part of the grammar because the production `overdrive-init` writes unterminated log fragments to the same console: when one is open as the program prints its first byte, the first block line is glued to it (24 of 60 replicates without the newline, 0 of 30 with it; V-B2). The host half matches a begin line only at the start of a console line (the strict grammar is kept), parses the block out of the console, checks that exactly one begin line, its value lines and one end line exist, that the parsed value count equals the count the family asked for, and that the checksum matches the recomputed one. A missing, short or disagreeing block (a residual splice of an `overdrive-init` fragment or a kernel message into a block line is caught by the count and checksum) makes the replicate failed; it is never recorded as a smaller series. The program needs no drain, flush or sleep: the console is complete after the VMM exits (V-B2). The same program prints its two guest readings, kernel release and clocksource (ADR-0328), after that newline and ahead of the block, under the same strict start-of-line rule. The exact grammar is pinned in the feature delta's DDD-6, and the grammar's printer and the host half's parser are owned together in `overdrive_testing::guest_app`.

## Alternatives

- **A tolerant begin-line match** (find `bench-series-begin:` anywhere in a line and parse from there): it would also recover the spliced blocks, but it weakens the strict grammar and would accept text from an unrelated line that happens to contain the prefix. Rejected for the leading newline, which removes the cause by construction and keeps every other rule.
- **A vsock block from the program to a host listener**: about 8 to 15 ms per replicate against about 0.65 s on the console (measured for a 41 KB block, V-B2), so about 20 s less over 30 replicates (about 63 s against about 83 s for the run). It needs a host listener and a port passed to the program, a reader running before the program starts, and listener plumbing and a received-block field in `run_in_guest_program` (more public test-support surface); the samples are then not in the retained console, so the raw evidence needs a second artifact. A guest-to-host connection uses the guest-to-host transport (`net/vmw_vsock/af_vsock.c:584-586 @ v7.0`). Rejected as the default for the mechanism it adds against a saving that does not change how the run is used.
- **A virtio-fs or disk file the guest writes**: needs a writable shared volume or reading back a guest-modified rootfs clone; the clone is removed with the guest. Rejected.
- **The beacon `EXIT` status as the channel**: one integer. Rejected.

## Consequences

The payload per replicate is bounded by the family's flow counts: the V-B2 block of about 41 KB (about 5,500 values) cost 0.65 s at about 63 KB/s on the production-init guest; 15,000 values (about 105 KB) would cost about 1.7 s, and the console costs about 16 s per MB. The cost is paid in the guest after the timed phase, so it never perturbs a measured interval, and it lengthens the replicate's guest life. A whole replicate cycle (launch, workload, return, exit) measured about 2.75 s at the median, about 83 s per 30 replicates. A residual splice of a console fragment into the block is possible at a rate bounded at about 10% (95%, 30 trials) and is a failed replicate, never a short series. The samples are also in the retained `console.log` (ADR-0333). The proof ran a production-init guest read after VMM exit (V-B2).

## Links

- [feature delta, DDD-6, V-B, V-B2](../../feature/guest-benchmark-substrate/feature-delta.md)
- [V-B findings](../../feature/guest-benchmark-substrate/spike/findings-guest-return-path.md)
- [V-B2 findings](../../feature/guest-benchmark-substrate/spike/findings-production-init-return.md)
- [ADR-0333](adr-0333-a-replicate-retains-its-guest-console.md)
