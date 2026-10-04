# Stock Cloud Hypervisor virtio-vsock device capacity probe

Standalone NW-SPIKE PROBE for the corrected 16,384 **transport-device** boundary.
The user approved this boundary and explicitly excluded a new running-guest cohort.
No KVM VM or Linux guest is created by this harness. Prior 64/128 real-guest
transport receipts remain in the separate shared-memory-vsock probe.

The counted owner is an unmodified upstream CH v53.0 `Vsock<VsockUnixBackend>`
activated through public `VirtioDevice::activate`, with its own stock Unix muxer,
CID, backend listener/nested epoll, exit eventfd, stock worker/outer epoll, three
256-entry virtqueues and separate 128KiB `GuestMemoryMmap<AtomicBitmap>` context.
The standalone program implements the peer virtio driver and interrupt callback.
It does not implement a replacement muxer or copy the transport implementation.
Every retained device exchanges distinct CID/epoch-tagged bytes through the
actual stock RX/TX virtqueue and Unix muxer paths at each held cardinality.

Each stage freezes all owned threads with SIGSTOP before a separate Python
observer reads `/proc/PID/fd`, `fdinfo`, `net/unix`, `task`, maps and memory.
The observer reconciles distinct device/context addresses and config/RX CIDs
with distinct kernel listening socket inodes, stock muxer nested epolls,
outer worker epolls, exit eventfd identities and held transport connections.
SIGCONT resumes the owned process before the next stage. The final pool is
exercised again before stock `reset`/`shutdown` releases every owner.

The 16,384-stage timer is 3.282924353s for bidirectional exchange **plus owner
identity capture/write and process FD/task enumeration**; it is not an isolated
traffic benchmark. The final complete pool recheck was not separately timed.
Receipt-derived build duration is 28.476242s; retained canonical-launcher
duration is 145.503525s.

Upstream dependency: `https://github.com/cloud-hypervisor/cloud-hypervisor`,
revision `9ed824d6d08df3e96f7d5f50795d9449ac99f431` (tag v53.0).
The source manifest records the inspected stock source hashes; native
`Cargo.lock.executed`, build receipts, executed source hashes and loaded
`/proc/PID/exe` fingerprint identify the code that ran. No upstream changes,
kernel fork/patch/module, guest-mTLS test, production change or commit occurs.

## Attempts

| Preserved increment | Result |
|---|---|
| [a](increment-a/README.md) | Native preparation failed before build because the snapshot helper had no output binding. Zero devices. |
| [b](increment-b/README.md) | Canonical source sync failed on attempt a's exact root-owned import cache. Zero devices. |
| [c](increment-c/README.md) | 16,384 activated stock devices/muxers; every device's tagged bidirectional stock virtqueue/muxer traffic, frozen independent owner audits and complete stock cleanup passed. |

[Canonical lease cleanup](cleanup-owned-preflight-cache.py) removed only the
exact cache witnessed in attempt b's error; its receipt proves lease release.
Prior source/evidence is never overwritten. Each attempted native run has its
own increment and immutable receipt directory.

## Run

```sh
python3 spike-scratch/netns-density-295-vsock-scale/launch.py increment-c
```

The launcher uses `cargo xtask metal run` for the canonical exclusive lease,
source sync and native preflight. Existing IDs refuse evidence/output reuse.
Only the owned capacity process's NOFILE soft limit is raised to its inherited
hard limit of 524,288; no host-global setting changes. Admission uses measured
MemAvailable with an 8GiB host reserve, not CPU-quota summation. There is no
per-guest CPU quota because no guest CPU runs.

At the final population: PSS 1,226,449KiB, RSS 1,228,420KiB, 344,067 externally
observed FDs, 16,385 threads and 58.287GiB host MemAvailable. Native runner and
canonical launcher exited 0. Stock reset/shutdown restored initial FD/thread
counts; all owned runtime socket paths and lease metadata were absent.

Build output and private archives live in ignored `target/` and `out/`.
Sources, scripts, exact commands and captured evidence remain for the
orchestrator's scoped commit. See the [findings](../../docs/feature/netns-density-295/spike/shared-memory-vsock-scale-findings.md)
for the final verdict and limits, [measurements](measurement-summary.json),
[source boundary](source-boundary.md), and [post-release attestation](post-release-readback.json).
