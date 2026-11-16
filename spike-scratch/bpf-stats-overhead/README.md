Throwaway probe: cost of `kernel.bpf_stats_enabled=1`. Findings:
`docs/feature/native-benchmark-program/spike/findings-bpf-stats-overhead.md`.
Run (Metal, exclusive; it flips a node-global sysctl and restores it via an RAII guard):

    cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build
    cargo xtask metal run --exclusive -- cargo run --release \
      --manifest-path spike-scratch/bpf-stats-overhead/Cargo.toml -- 20261116 30 \
      spike-scratch/bpf-stats-overhead/results/runs.csv
    cargo xtask metal pull spike-scratch/bpf-stats-overhead/results/runs.csv

Derived from `../pig-connect-cost` (unmodified). `results/runs.csv` = raw per-run rows, `results/run.log` = full stdout.
