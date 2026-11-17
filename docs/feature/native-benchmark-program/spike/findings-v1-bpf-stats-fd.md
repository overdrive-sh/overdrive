# V-1 — Descriptor-scoped BPF stats

**Verdict: HOLDS.** Shapes DDD-8 (per-process `BPF_ENABLE_STATS`) and the DDD-7 holder scan.

Host: AMD EPYC 8024P (8-core), `uname -r` 7.0.0-38-generic, Cloud Hypervisor v53.0.0, rustc 1.95.0, commit e3b42c92e (DESIGN docs dirty). Run 2026-11-16/17 on the native Metal host with `cargo xtask metal run --exclusive`.

Probe: `spike-scratch/nbp-v1-bpf-stats-fd/` (raw output `results/run-1.log`). Fixture: the pig-connect-cost fixture, two production `cgroup_connect4` programs, loopback connect-and-close.

Command: `cargo xtask metal run --exclusive -- cargo run --release --manifest-path spike-scratch/nbp-v1-bpf-stats-fd/Cargo.toml` (after `env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build`).

## Predicted vs actual

| Phase | Predicted | Actual |
|---|---|---|
| Sysctl, every phase | 0 | 0 |
| No holder, 1,000 connects | `run_count` delta 0 | `[0, 0]` |
| Holder live, 5,000 connects | delta 5,000 per program | `[(5000, 516265), (5000, 583485)]` (≈103 and ≈117 ns per run) |
| Scan from another process during hold | target `anon_inode:bpf-stats` | `holders=[(3234538, 3)]`, the holder's pid |
| After `kill -9`, 1,000 connects | no holder, delta 0 | scan empty, `[0, 0]` |

## Edge cases

- Two holders: killing one leaves stats on (reference-counted); the scan shows only the survivor.
- Clean exit behaves like a kill.
- Scan run as root: 0 unreadable `/proc/*/fd` directories.
- Other BPF descriptors carry different targets (`bpf-map`, `bpf-prog`, `bpf_link`, `btf`), so an exact match has no false positives.

## Recommendation

DDD-8 and the DDD-7 scan stand. Match the target string exactly; run the scan as root.
