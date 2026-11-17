# Spike wave decisions

## V-6 — programmed service map read (2026-11-17)

- Gate: **PROMOTE**. See [findings-programmed-service-map-read.md](findings-programmed-service-map-read.md).
- Basis: on kernel 7.0.0-38-generic, with production weights, the read reproduced the programmed set exactly. It read empty after a restart over the same pin directory and after a reboot analogue. With the apply-commit guard held there were no torn reads.
- Conditions for D-20:
  - the read is exact only while every backend owns a Maglev slot, which uniform weight 1 guarantees today;
  - the read must hold the apply-commit guard;
  - it costs about 5 ms per frontend.
- Read mechanism (recommended, pending user approval): **fingerprint slot**, measured against the full-table scan and an mmap read (1,000 frontends × 16 backends).
  - Read p50: fingerprint 4.7 µs, mmap 107 µs, scan 5.41 ms.
  - Sweep of 1,000: 6.2 ms, 112 ms and 5.84 s respectively.
  - See "Comparison of read mechanisms" in the findings.
