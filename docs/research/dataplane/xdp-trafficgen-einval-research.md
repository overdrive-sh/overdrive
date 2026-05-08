# xdp-trafficgen EINVAL on BPF_F_TEST_XDP_LIVE_FRAMES — Root Cause Analysis

**Date**: 2026-05-08
**Status**: Resolved — root cause identified, fix available

## Problem

`xdp-trafficgen udp -n 1000000 xdp1` fails with EINVAL on Lima VM
(kernel 6.8.0-111-generic, xdp-tools 1.4.2-1ubuntu4):

```
bpf(BPF_PROG_TEST_RUN, {
  test={prog_fd=30, data_size_in=4, ctx_size_in=24,
        flags=BPF_F_TEST_XDP_LIVE_FRAMES, batch_size=1}
}, 80) = -1 EINVAL
```

## Root Cause

**xdp-trafficgen v1.4.2's `probe_kernel_support()` sends a 4-byte
data buffer; the kernel requires >= 14 bytes (`ETH_HLEN`).**

### Kernel side

Kernel commit `6b3d638ca897` ("bpf: fix KMSAN uninit-value in
bpf_prog_test_run_xdp") added a minimum size check to
`bpf_test_init()` in `net/bpf/test_run.c`:

```c
if (user_size < ETH_HLEN || user_size > PAGE_SIZE - headroom - tailroom)
    return ERR_PTR(-EINVAL);
```

This check fires regardless of `BPF_F_TEST_XDP_LIVE_FRAMES` — it
applies to both standard and live-frame test runs.

### xdp-trafficgen side

v1.4.2's probe function (the code that tests whether the kernel
supports live frames before using them) does:

```c
int data = 0;
.pkt      = &data,
.pkt_size = sizeof(data),   // 4 bytes — below ETH_HLEN threshold
```

The kernel returns EINVAL (from the size check, not from
live-frames validation). xdp-trafficgen misinterprets this as
"kernel doesn't support live packet mode" and bails.

### Fix in xdp-tools

Commit `4f7f5cb` ("xdp-trafficgen: Fix data size when probing for
kernel support") by Toke Hoiland-Jorgensen:

```c
__u8 data[ETH_HLEN] = {};
.pkt      = data,
.pkt_size = sizeof(data),   // 14 bytes — passes validation
```

**First release with fix**: xdp-tools v1.5.4 (April 2024).
Ubuntu Noble ships v1.4.2, which predates the fix.

## Solutions

### Option A: Build xdp-tools from source (>= v1.5.4)

Build xdp-trafficgen from the xdp-tools repo at tag v1.5.4+.
Requires libxdp, libbpf, clang build dependencies in the Lima VM.

Tradeoffs: exact fix for the exact tool; live-frame mode achieves
~9 Mpps/core (Toke's benchmarks). Adds a build dependency to
provisioning.

### Option B: Standard BPF_PROG_TEST_RUN with repeat=N (no live frames)

Use `BPF_PROG_TEST_RUN` without `BPF_F_TEST_XDP_LIVE_FRAMES`. The
kernel runs the XDP program N times in a tight loop against a
synthetic packet. The `repeat` field controls iteration count.

`ProgramInfo::run_time()` / `run_count()` increment for standard
test runs — the `BPF_PROG_RUN()` macro unconditionally accumulates
stats when `BPF_ENABLE_STATS` is active. No distinction between
test-run and real-traffic invocations in the stats accounting.

Tradeoffs: zero external dependency; entirely in-kernel; maximum
throughput; more deterministic (no veth/NIC jitter). Packets are
synthetic (verdict discarded, never hit the network stack) — fine
for measuring program execution cost, not for end-to-end flow.

The project already has a `prog_test_run()` helper at
`crates/overdrive-dataplane/src/sys/prog_test_run.rs`.

### Option C: Raw socket traffic generator

Send real UDP frames through the veth pair using AF_PACKET sockets
(Rust or nping/hping3). Lower throughput (~1-2 Mpps vs 9 Mpps)
but exercises real packet flow.

## Recommendation

**Option B** (standard BPF_PROG_TEST_RUN) for the Tier 4 perf gate.
The gate measures program execution cost, not end-to-end packet flow
(that's Tier 3). Standard test_run is:

- Zero external dependencies
- More deterministic (no kernel networking stack jitter)
- Higher throughput than veth-based approaches
- Already supported by the project's `prog_test_run()` helper
- Stats (`run_time_ns`/`run_cnt`) accumulate identically

Build xdp-trafficgen from source for Tier 3 when live end-to-end
packet flow validation is needed (separate concern from perf gate).

## Sources

| Source | Reputation |
|--------|-----------|
| Linux kernel v6.8 `net/bpf/test_run.c` (github.com/torvalds/linux) | High |
| Kernel commit `6b3d638ca897` | High |
| xdp-tools commit `4f7f5cb` | High |
| xdp-tools releases (github.com/xdp-project/xdp-tools) | High |
| Kernel docs `bpf_prog_run` (docs.kernel.org) | High |
| LWN: Live packet mode patch series | High |
