# Benchmark catalogue

One row per benchmark. The substrate, the statistics flag and the caveats of a run are in its
record's header; methodology version 1 applies to every row (see `methodology.md`).

| Family and benchmark | Boundary and arms | Workload matrix | Clock boundary | Result owner |
|---|---|---|---|---|
| `cgroup-connect-hook` / `connect_hook_cost` | per-connect cost of the gateway `cgroup/connect4` program; arms `service-only` (reference, gateway link detached) and `service+gateway` | 1 profile; per replicate a fresh cgroup, bpffs pin directory and veth pair, both connect programs loaded, 500 warm-up then 5,000 timed loopback connect-and-close (`SO_LINGER` 0); 30 replicates per arm | `hook_ns_per_connect`: kernel per-program `run_time_ns` over `run_count` (BPF statistics); `wall_ns_per_connect`: host `CLOCK_MONOTONIC_RAW` stamps around the timed loop divided by 5,000 | public ingress gateway (`overdrive-gateway`, `overdrive-dataplane`) |
