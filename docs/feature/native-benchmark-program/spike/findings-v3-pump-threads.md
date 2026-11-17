# V-3 — Per-thread attribution of mTLS pump work

**Verdict: HOLDS, with two caveats.** Shapes DDD-14 (named pump threads) and DDD-13's per-thread rows.

Host: AMD EPYC 8024P (8-core), `uname -r` 7.0.0-38-generic, Cloud Hypervisor v53.0.0, rustc 1.95.0, commit e3b42c92e (DESIGN docs dirty). Run 2026-11-16/17 on the native Metal host with `cargo xtask metal run --exclusive`.

Probe: `spike-scratch/nbp-v3-pump-threads/` (`results/run-2.log`, `results/runs.csv`, `results/strace-1.log`).

Command: `cargo xtask metal run --exclusive -- cargo run --release --manifest-path spike-scratch/nbp-v3-pump-threads/Cargo.toml -- run 10 3 spike-scratch/nbp-v3-pump-threads/results/runs.csv`

## What was composed

Production `HostMtlsEnforcement::enforce(Routed::Outbound)` in the `mtls_guardrails.rs` shape (lines 125-380), not the full worker egress rig (`MtlsInterceptWorker` + `HostMtlsIntercept` + `start_alloc`). It reaches the same `spawn_encrypt` / `spawn_decrypt` sites in `outbound.rs`; it omits the worker's leg-F accept loop. `mtls_pki.rs` and `guest_dial.rs` included by `#[path]`. Mesh peer in a child process; plaintext workload client in-process on named `lg-*` threads; sampling via `/proc/self/task/<tid>/stat` at window edges. 3 replicates per direction.

## Predicted vs actual

Prediction: pumps ≥ 90% of process CPU delta; heavy:light ratio ≥ 3, flipping with direction. Falsification: < 50% in the pumps, or ratio < 1.5 in both runs.

| | forward-heavy | return-heavy |
|---|---|---|
| Throughput | 889–894 MB/s | 855–872 MB/s |
| Heavy pump CPU over 10 s | ≈10,010 ms (99% system) | ≈9,930 ms (96% system) |
| Light pump CPU | below one 10 ms tick | below one 10 ms tick |
| Heavy:light | ≥ 100 (lower bound) | ≥ 100 |
| Heavy thread | first-created (encrypt) | second-created (decrypt) |
| Pumps / whole-process delta | 0.86 | 0.57–0.60 |
| Pumps / (process − `lg-*` threads) | 1.000 | 0.999–1.000 |

Each connection creates exactly two std pump threads; their `comm` is inherited as `tokio-rt-worker`, so name-based attribution needs DDD-14's names. `mtls-out-fwd` / `mtls-out-ret` are 12 bytes, under the 15-byte `comm` limit.

## Caveats

1. The literal 90%-of-whole-process bar is not met because in-process client threads burn CPU (the 50% falsification bar is cleared). The family must name its fixture threads and report the pump share excluding them.
2. A pump saturates one core (≈890 MB/s), so the light thread is below tick resolution; the ratio is a lower bound.

## Fixture requirement

The first run failed: the peer sent a TLS 1.3 NewSessionTicket, the kTLS-RX return `splice` returned EINVAL and the connection tore down (`results/strace-1.log`). Production peers set `send_tls13_tickets = 0`; the DDD-13 mesh-peer rig must too.

## Recommendation

DDD-14 and DDD-13's per-thread rows stand; add the two caveats and the ticket requirement to the family design.
