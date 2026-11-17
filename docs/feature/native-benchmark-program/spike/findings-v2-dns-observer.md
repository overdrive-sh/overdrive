# V-2 — DNS query-observer cost

**Verdict: HOLDS.** Shapes DDD-12 (`QueryObserver` on `DnsResponder`).

Host: AMD EPYC 8024P (8-core), `uname -r` 7.0.0-38-generic, Cloud Hypervisor v53.0.0, rustc 1.95.0, commit e3b42c92e (DESIGN docs dirty). Run 2026-11-16/17 on the native Metal host with `cargo xtask metal run --exclusive`.

Probe: `spike-scratch/nbp-v2-dns-observer/` (`results/runs.csv`, `results/run-1.log`).

Command: `cargo xtask metal run --exclusive -- cargo run --release --manifest-path spike-scratch/nbp-v2-dns-observer/Cargo.toml -- 20261116 30 10 5000 spike-scratch/nbp-v2-dns-observer/results/runs.csv`

## What was copied

The `recvmsg` + `IP_PKTINFO` serve loop of `responder.rs` (`serve_one_socket`, lines 354-420), with `SO_RCVTIMEO` and `sendmsg` source pinning. The hickory decode/answer/encode step is replaced by a fixed 70-byte reply, which overstates the observer's share of service time (conservative). Arms: A, A2 (A/A control), B (`NoQueryObserver`), C (recording sink). Scheduled arrival at 5,000 q/s for 10 s; 30 replicates per arm in seeded blocks; server, sender and receiver threads pinned to separate CPUs.

## Predicted vs actual

Prediction: under 100 ns per datagram; C−A interval includes the A/A interval. Falsification: C−A distinguishable with point estimate above 200 ns.

| Measure | Result |
|---|---|
| Isolated instrumentation cost (10M-iteration microbench) | 62.7 ns (`NoQueryObserver`), 75.2 ns (recorder) |
| Client p50, median of replicates | A 33,991 / A2 34,054 / B 33,953 / C 34,102 ns |
| A/A (A2−A) | +63 ns, 95% CI [−223, +710] |
| C−A | +111 ns, CI [−165, +848] |
| B−A | −38 ns, CI [−349, +649] |
| p99 C−A | +51 ns, CI [−343, +308] |
| Lost queries | 0 of 6,000,000 |
| Observer-recorded service time | ≈11.2 µs |

All deltas are not distinguishable under the DDD-5 rule. The end-to-end A/A floor (≈±700 ns) only rules out costs above ≈0.7 µs; the microbench gives the per-datagram bound (63–75 ns).

## Recommendation

DDD-12 stands; its hot-path budget (two clock reads plus one dynamic call) is confirmed. Cite the microbench for the bound.
