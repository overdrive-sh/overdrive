# Benchmark report: guest-flow-forwarding / guest_flow_forwarding

- run id: 507cd3e5a68532d2bedd7e86
- family: guest-flow-forwarding
- benchmark: guest_flow_forwarding
- evidence: maintained
- substrate: guest
- commit: 36618cba7a86588dce3608ed3f898890017b9ef4
- dirty: true
- methodology: 1
- seed: 2100612783549208868
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
- guest launch: metal host (not nested)
- guest boot: fresh guest per replicate
- guest vcpu scheduling: unpinned
- guest governor: absent: not visible from a guest
- guest smt: absent: not visible from a guest
- guest boost: absent: not visible from a guest
- guest vcpus: absent: no replicate returned a guest view
- guest memory mib: absent: no replicate returned a guest view
- guest kernel: absent: no replicate returned a guest view
- guest clocksource: absent: no replicate returned a guest view
- guest artifact overdrive_init: absent: no replicate returned a guest view
- guest artifact overdrive_guest_peer: absent: no replicate returned a guest view
- guest artifact ovd_fwd: absent: no replicate returned a guest view
- bpf stats: off
- clock: host CLOCK_MONOTONIC_RAW; service none; guest CLOCK_MONOTONIC (in-guest Instant, 1 ns)
- canary: steady
- replicates ok: 0
- replicates failed: 30
- status: finished
## Profile: default

### Table: forwarded-path

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| forwarded_round_trip_ns | forwarding | absent: no stored value | absent: no stored value | absent: no stored value | absent: no stored value | 0 | 0 | 30 |

## Failures

- profile default block 0 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 1 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 2 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 3 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 4 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 5 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 6 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 7 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 8 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 9 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 10 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 11 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 12 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 13 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 14 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 15 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 16 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 17 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 18 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 19 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 20 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 21 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 22 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 23 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 24 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 25 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 26 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 27 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 28 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
- profile default block 29 position 0 arm forwarding: guest boot: the guest-kit lane is not prepared: no static overdrive-guest-peer for x86_64-unknown-linux-musl at /home/ubuntu/overdrive-worktrees/1979cf3037635b207acfa805597fb66447abd1fd68b30be7f6a336192158f5ab/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer; run `cargo xtask guest-kit-prepare` (under `cargo xtask metal run --`) before the lane
