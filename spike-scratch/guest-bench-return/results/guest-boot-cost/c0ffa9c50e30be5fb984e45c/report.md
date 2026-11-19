# Benchmark report: guest-boot-cost / guest_boot_cost

- run id: c0ffa9c50e30be5fb984e45c
- family: guest-boot-cost
- benchmark: guest_boot_cost
- evidence: spike evidence
- substrate: native
- commit: 5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b
- dirty: true
- methodology: 1
- seed: 9483754556213611528
- debug_assertions: false
- kernel: 7.0.0-38-generic
- cpu_model: AMD EPYC 8024P 8-Core Processor
- cpu_count: 16
- governor: schedutil
- smt: on
- boost: on
- bpf_stats_enabled: 0
- clocksource: tsc
- irq_time_accounting: n
- bpf stats: off
- clock: host CLOCK_MONOTONIC_RAW; service none; guest none
- canary: steady
- replicates ok: 0
- replicates failed: 90
- status: finished
## Profile: default

### Table: lifecycle

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| image_ns | fresh-guest | absent: no stored value | absent: no stored value | 0 | 30 |
| image_ns | fresh-guest-control | absent: no stored value | absent: no stored value | 0 | 30 |
| boot_to_peer_ns | fresh-guest | absent: no stored value | absent: no stored value | 0 | 30 |
| boot_to_peer_ns | fresh-guest-control | absent: no stored value | absent: no stored value | 0 | 30 |
| first_exec_ns | fresh-guest | absent: no stored value | absent: no stored value | 0 | 30 |
| first_exec_ns | fresh-guest-control | absent: no stored value | absent: no stored value | 0 | 30 |
| shutdown_ns | fresh-guest | absent: no stored value | absent: no stored value | 0 | 30 |
| shutdown_ns | fresh-guest-control | absent: no stored value | absent: no stored value | 0 | 30 |
| total_ns | fresh-guest | absent: no stored value | absent: no stored value | 0 | 30 |
| total_ns | fresh-guest-control | absent: no stored value | absent: no stored value | 0 | 30 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| warm_exec_ns | fresh-guest | absent: no stored value | absent: no stored value | absent: no stored value | absent: no stored value | 0 | 0 | 30 |
| warm_exec_ns | fresh-guest-control | absent: no stored value | absent: no stored value | absent: no stored value | absent: no stored value | 0 | 0 | 30 |

## Failures

- profile default block 0 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 0 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 0 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 1 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 1 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 1 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 2 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 2 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 2 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 3 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 3 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 3 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 4 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 4 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 4 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 5 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 5 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 5 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 6 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 6 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 6 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 7 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 7 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 7 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 8 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 8 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 8 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 9 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 9 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 9 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 10 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 10 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 10 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 11 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 11 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 11 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 12 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 12 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 12 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 13 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 13 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 13 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 14 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 14 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 14 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 15 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 15 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 15 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 16 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 16 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 16 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 17 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 17 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 17 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 18 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 18 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 18 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 19 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 19 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 19 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 20 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 20 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 20 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 21 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 21 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 21 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 22 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 22 position 1 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 22 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 23 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 23 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 23 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 24 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 24 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 24 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 25 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 25 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 25 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 26 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 26 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 26 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 27 position 0 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 27 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 27 position 2 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 28 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 28 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 28 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 29 position 0 arm fresh-guest-control: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 29 position 1 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

- profile default block 29 position 2 arm fresh-guest: kit provision: building overdrive-init for target x86_64-unknown-linux-musl failed (exit=Some(101)): error: package ID specification `overdrive-init` did not match any packages

help: a package with a similar name exists: `overdrive-host`

