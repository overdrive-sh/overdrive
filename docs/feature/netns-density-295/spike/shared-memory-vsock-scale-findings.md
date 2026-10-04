# Stock virtio-vsock transport-device capacity — 16,384

**Verdict: WORKS at the user-approved transport-device boundary.** On the native
metal host, the standalone harness concurrently held **16,384 activated stock
Cloud Hypervisor virtio-vsock devices**, each with an independent stock Unix
muxer and peer virtqueue/memory context. **Every held device exchanged distinct
CID/epoch-tagged bytes in both directions through its actual stock virtqueues
and muxer at full population**, then passed another complete pool recheck.
An independent frozen kernel-object audit reconciled all 16,384 owners.

**Zero KVM VMs and zero Linux guests were created.** The user approved the exact
stock-device/muxer boundary, then explicitly excluded a new running-guest
cohort. This result answers transport-device capacity; it does not declare
16,384 resident microVMs. The prior [64/128 real Linux guest traffic results](shared-memory-vsock-findings.md)
remain separate existing evidence and were not rerun. Combined guest-mTLS
testing was explicitly excluded and **NOT RUN**.

## Question, corrected scope and prediction

The bounded question was whether the candidate's actual host-side per-VM
virtio-vsock device/muxer owners can reach 16,384 concurrently on this machine,
without the Linux bridge-port ceiling, and remain operational at that held
population. The final scope pins the counted transport-device layer and permits
synthetic peer drivers; it requires neither guest boot nor guest RAM residency.
No production API, accepted DESIGN, roadmap validation or DES log was changed.

Prediction before the large run: the stock per-device constructors and activation
path would allocate independent owners beyond 1,023, with memory, FDs and worker
threads determining admission. The harness would first prove that exact seam at
4, then retain 1,024 / 4,096 / 8,192 / 16,384 owners and exchange tagged bytes
through every device at each stage. The stock muxer's **1,023 tracked connections
per muxer** was not treated as a host-wide device limit. One established
connection per independent muxer was exercised.

Actual: every predicted stage passed, including the final 16,384. No resource
ceiling was hit, and no count beyond 16,384 was attempted. There is no inferred
higher capacity or aggregate throughput promise.

## Exactly what was counted

Each retained owner contains an **unmodified upstream**
`Vsock<VsockUnixBackend>` from CH v53.0, revision
`9ed824d6d08df3e96f7d5f50795d9449ac99f431`. The probe calls the public stock
`VsockUnixBackend::new`, `Vsock::new`, and `VirtioDevice::activate` APIs.
The stock VMM DeviceManager uses those same device/backend constructors.

| Object or lifecycle | Actual at final held population |
|---|---:|
| Activated stock `Vsock` owners with distinct retained addresses | 16,384 |
| Independent stock Unix muxers / base backend listeners | 16,384 |
| Distinct device-config CIDs, also verified in stock-produced RX headers | 16,384; 100,000–116,383 |
| Independent peer `GuestMemoryMmap<AtomicBitmap>` contexts | 16,384; 128KiB each |
| Stock virtqueues owned by those activated devices | 49,152; three 256-entry queues per device |
| Independently verified kernel listening socket inodes | 16,384 |
| Independently verified stock muxer nested epoll FDs | 16,384 |
| Independently verified stock worker outer epoll FDs | 16,384 |
| Independently verified distinct exit eventfd identities | 16,384 |
| Held established host↔peer transport connections | 16,384; one per device |
| Stock worker threads / total capacity-process threads | 16,384 / 16,385 |
| Running guest OSes / KVM VMs / guest vCPUs | 0 / 0 / 0 |
| Created TAPs / bridge ports / virtio-net devices | 0 / 0 / 0 |

The peer driver and interrupt callback are probe code. Peer drivers place actual
virtio descriptor chains and `virtio_vsock_hdr` packets into each context's
memory. Stock worker code processes the RX/TX queues, decodes packets, dispatches
the stock muxer and drives its stock connection state machine. Host endpoints
use the real Unix backend and its `CONNECT 5001` handshake. Tags include the
individual CID and stage/epoch, so traffic is correlated to each owner.

After each full-pool exchange, the separate Python observer sends SIGSTOP to
the owned capacity process, verifies all its threads are stopped, and reads
`/proc/PID/fd`, `fdinfo`, `net/unix`, `task`, `status`, `smaps_rollup` and `maps`.
For every owner it correlates the backend pathname's kernel listening socket
inode to its nested muxer epoll, verifies that epoll is registered in exactly
one stock worker outer epoll, and reconciles exit eventfd identities, held host
connection socket identities, device/context addresses, config/RX CIDs and
thread cardinality. SIGCONT resumes the owner before further work.

This is not a listener-only population, aliases behind one device, many ports
behind one peer, a logical CID registry, or an in-process replacement muxer.
The CH DeviceManager/PCI transport, KVM/vCPU composition, Linux guest socket
implementation and guest workload memory are not exercised. Seccomp is `Allow`
through the existing stock constructor; production confinement is not tested.
There are no host kernel AF_VSOCK CID registrations in this userspace backend.
The [source boundary audit](../../../../spike-scratch/netns-density-295-vsock-scale/source-boundary.md)
pins those distinctions and the exact upstream production path.

## Measured stages and resource budget

Native host: x86_64 Linux **7.0.0-29-generic**, AMD EPYC 8024P, 8 cores / 16
logical CPUs, **65,401,772KiB RAM (62.372GiB)**, **zero swap**. This qualifies
the tested native kernel, not the pinned 6.18 appliance kernel.

| Held activated devices | Process PSS, KiB | Process RSS, KiB | External FD count | Threads | Memory mappings | Host MemAvailable, KiB | Exchange + identity capture/write + FD/task enumeration |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 4 | 1,313 | 3,284 | 87 | 5 | 58 | 62,738,252 | 0.000960s |
| 1,024 | 79,969 | 81,940 | 21,507 | 1,025 | 4,437 | 62,653,004 | 0.244360s |
| 4,096 | 309,329 | 311,300 | 86,019 | 4,097 | 13,665 | 62,408,688 | 0.943923s |
| 8,192 | 615,121 | 617,092 | 172,035 | 8,193 | 25,960 | 62,034,452 | 1.704718s |
| 16,384 | **1,226,449** | **1,228,420** | **344,067** | **16,385** | **50,541** | **61,118,072** | **3.282924s** |

The stage timer includes every-device bidirectional exchange, capture and
serialization/write of all owner identities, and process FD/task enumeration.
Its 3.282924353s value at 16,384 is an upper bound on the sequential exchange's
wall time, not an isolated traffic benchmark. The later final complete pool
recheck passed but was not separately timed.

At 16,384, measured PSS is **1.170GiB**, RSS **1.172GiB**, VmPTE **77.688MiB**,
and VmSize **46.424GiB**. The 2GiB total peer-memory mapping and thread-stack /
allocator reservations contribute to virtual address space; virtual size is
not physical RAM usage. The process includes the stock muxers' preallocated
maps, stock queue/worker resources, synthetic driver state and held transport
connections. Approximately 74.9KiB average PSS per device describes this
specific lightly loaded population, not a loaded-service or real-VM budget.
Kernel socket/thread/slab cost and the observer are not included in that
process PSS; host MemAvailable is a separate whole-host observation.

The observer sees **21 × count + 3** open FDs throughout the stages. The Rust
process reports one more because its own `/proc/self/fd` enumeration temporarily
opens that directory; external frozen counts are the table's values. The
capacity process's existing NOFILE hard limit was 524,288, and only its soft
limit was raised from 1,024 to that inherited hard limit before exec. At final
population 180,221 FD slots remained under that process limit. No global FD,
thread, socket, ARP, CPU, memory or sysctl setting changed.

Admission used actual MemAvailable: a 16GiB entry threshold and an 8GiB bounded
host reserve checked at each held stage. At 16,384, **58.287GiB MemAvailable**
remained. CPU quotas were neither configured nor summed as reservations; zero
guest CPUs ran, and stock workers used the existing host scheduler. The tested
process was in `user.slice/user-1000.slice/session-29353.scope`; its live
cgroup identity is retained. Post-release surviving ancestor readbacks show
unlimited memory/CPU and `user-1000.slice` pids.max 155,206. The terminated
session cgroup had already disappeared, so its exact former limits are not
claimed from that later readback.

Observed host limits: threads-max 470,322, pid_max 4,194,304, max_map_count
1,048,576, epoll max_user_watches 14,538,195, somaxconn 4,096; these settings
were read and left untouched. The post-release disk readback has approximately
649.8GiB free; the probe created no guest images. Full raw resource, kernel FD /
epoll and ancestry receipts are retained, including the explicit timing of
post-release observations.

## Predicted versus actual native output

Expected: 16,384 separately activated stock owners, all tagged exchanges pass,
all independent owner counts agree, then stock cleanup returns to the initial
FD/thread population and removes every owned socket pathname.

Actual output from attempt c:

```json
{"actual_kvm_vms":0,"actual_linux_guests":0,"actual_stock_activated_devices":16384,"actual_stock_muxers":16384,"elapsed_s":21.226878916,"event":"stage_held","every_peer_tagged_bidirectional_exchange_passed":true,"full_pool_recheck_s":3.282924353,"held_transport_connections":16384,"independent_synthetic_driver_contexts":16384,"process_fd_count":344068,"process_task_count":16385}
{"all_checks_passed":true,"at_s":55.849601212888956,"distinct_device_addresses":16384,"distinct_exit_eventfd_ids":16384,"distinct_memory_contexts":16384,"distinct_peer_connections":16384,"distinct_stock_muxer_epoll_fds":16384,"distinct_worker_outer_epoll_fds":16384,"event":"independent-frozen-owner-audit","held_stock_activated_devices":16384,"host_memavailable_kib":61118072,"kernel_unique_listening_socket_inodes":16384,"native_process_fds":344067,"native_process_tasks":16385}
{"event":"final_recheck_begin","held":16384}
{"elapsed_s":56.451896895,"event":"cleaned","final_fds":4,"final_tasks":1,"initial_fds":4,"initial_tasks":1,"released_devices":16384,"remaining_socket_paths":0}
```

Native dependency build: **28.476242s**, the difference between the retained
`build-start.at_s=0.09396740514785051` and
`build-complete.at_s=28.570209346711636` events. The latter is a runner-relative
completion timestamp, not the build duration. Device experiment including all stages,
independent holds and final recheck/cleanup: 56.451897s. Native runner:
85.757187s, exit **0**. Canonical launcher including lease/source sync/preflight:
**145.503525s**, from the retained launch log's `wall_s=145.50352470899816`,
exit **0**. These are individual mechanism observations, not an
invented SLO or sustained-throughput claim. The stage timestamp includes prior
observer holds; it is not a pure device-construction benchmark.

## Provenance, failed attempts and exact cleanup

Canonical command:

```sh
python3 spike-scratch/netns-density-295-vsock-scale/launch.py increment-c
cargo xtask metal run -- python3 spike-scratch/netns-density-295-vsock-scale/increment-c/run.py
```

Attempt a failed before build/device creation because the copied snapshot
helper lacked its output binding; canonical exit 1. Attempt b's canonical
source sync failed with rsync exit 23 because attempt a had generated one
root-owned Python import cache. The exact witnessed cache was removed under
the existing canonical lease-holder protocol, with its own retained script /
receipt. Attempt c binds the output and disables import-cache writes. No prior
source or failed receipt was overwritten; none of these preparation failures
is a transport negative.

| Executed item | SHA-256 |
|---|---|
| Rust probe source | `8363e2849f4a701e3aa081cd15a62e20ac8da58a3697f3a3d1eb8214f5629356` |
| Native runner | `d095c593d97d6be9de21d3c0b4a691c549a574eb53fa8f6e55146f0705ddba34` |
| Built helper and independently read loaded `/proc/814497/exe` | `4d0b81949b62283fe31396ec0e83cf2aab10e452002122d41442fc4b3a19c11e` |

The retained native dependency lock/build receipt pins CH to revision
`9ed824d6d08df3e96f7d5f50795d9449ac99f431`. After release, a read-only attestation
checked the actual native Cargo git checkout HEAD and independently hashed
the seven inspected upstream files; every hash matched the read-only local
v53.0 source audit. No tracked upstream source was modified; the checkout's
only reported untracked item was Cargo's `.cargo-ok` marker. Rustc/version,
source manifests, executed lock, native hashes, command/exits and complete
owner/resource receipts are preserved.

Attempt c held exclusive lease token `366ba1d33a9392b0b674f30f`. Stock reset
signaled and joined all 16,384 worker threads; stock shutdown removed their
base sockets. The capacity process returned from 16,385 to 1 thread and from
344,068 to its initial 4 self-enumerated FDs before exiting. Owned capacity PID
814497 and runner PID 811758 exited; `/run/vs295-scale-811758` was absent.
Before/after administrative configuration complements matched for links,
addresses, routes, rules, nftables, BPF maps/links, namespaces, host module
identities and sampled sysctls. The host OOM counter increased by **zero**.
No foreign dynamic neighbor/counter/timer/process equality is claimed.

The [post-release attestation](../../../../spike-scratch/netns-density-295-vsock-scale/post-release-readback.json)
independently confirms all seven recorded task/lease PIDs absent, both exact
owned socket-directory paths absent, the exact prior import-cache directory
absent, and the canonical lease-owner file absent. Build caches and private
archives remain intentionally in ignored `target/` and `out/`; no live runtime
owner remains. Large receipts use verified lossless gzip; shared text redacts
only the configured metal address, while exact native archives remain private.
The [preservation manifest](../../../../spike-scratch/netns-density-295-vsock-scale/evidence-preservation.json)
records original/compressed hashes and byte-round-trip verification.

## Capacity implications for #303 and PR #306 — documentary only

The prior bidirectional [#303/#306 assessment](shared-memory-vsock-findings.md#bidirectional-interaction-with-issue-303-and-pr-306)
continues to apply. Current supplied metadata pins [PR #306](https://github.com/overdrive-sh/overdrive/pull/306)
to `e0c1b4190cbcfaf44c82d325f405a224f2cc8ffc`; it is spike evidence, not an accepted
production design. [Issue #303](https://github.com/overdrive-sh/overdrive/issues/303)
proposes guest TCP-socket/kTLS work with host-held SVID key, host rustls/policy,
and vsock resolution/handshake channels. No combined module, handshake,
encryption, workload, restart or contention test ran here.

The newly measured implication is bounded: **creating the stock per-VM vsock
device/muxer population itself does not hit the bridge's roughly 1,023-port
limit at 16,384**. Whether used for handshake/control, bulk data or both, this
backend retains per-device queues, maps, a stock worker and owned kernel FDs.
Moving TLS work into the guest would not remove those observed transport costs.
Conversely, the 16,384 lightly loaded device result does not establish the CPU,
memory, flow-admission or congestion budget for resolution plus handshake plus
bulk traffic. Each muxer still has its own 1,023 tracked-entry budget, and this
probe used one connection per muxer. Existing queue/map ownership, shared
per-device contention and host-transport survival distinctions remain explicit;
no new channel/persistence/recovery architecture is invented.

## Gate

**Retain shared-memory/virtio-vsock as a viable 16,384 stock transport-device
capacity candidate.** This corrected capacity question is validated on the
tested native host. Production guest adaptation, policy/TLS/DNS composition,
observation and lifecycle contracts remain separate design work. No promotion,
implementation or relaxation of accepted zero/never/must outcomes follows.
