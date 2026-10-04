# Shared-memory/vsock with host in-kernel TCP/UDP proxying

Date: 2026-10-04. Wave: NW-SPIKE PROBE. No promotion or DESIGN decision.

## Verdict and exact question

**WORKS:** stock `vhost-vsock` shared virtqueues carried real application bytes
and UDP datagrams through a loaded host kernel TCP/UDP adapter, without TAPs,
userspace application forwarding, or a kernel patch. The qualified metal run
held **16,384 live kernel transport/adapters**, with a different vhost device,
CID, memory context and kernel UDP socket for every counted owner. Every owner
passed a tagged bidirectional datagram at the final population.

**DOESN'T-WORK:** this bounded prototype does not deliver UDP datagrams above its
60,000-byte frame bound; a real 61,000-byte datagram was consumed whole and
rejected without partial delivery. It also does not demonstrate fast complete
transport teardown: retirement after the final population took approximately
267.343s and stock transport references remained before private VM shutdown.

Ordinary guest TCP/UDP transparency, unmodified guest sshd reachability, the
Cloud Hypervisor backend integration, guest kTLS and host-held key composition,
and production lifecycle/policy guarantees remain **unvalidated**, not disproven.

## Topology and ownership

Final evidence is **metal**, not Lima. The physical qualified fixture runs
Ubuntu 7.0.0-29-generic. QEMU with **KVM**, q35, the unmodified installed stock
kernel image and matching stock modules/headers boots one disposable host-role
VM. The experimental module executes in that VM's **7.0.0-29-generic kernel**;
it is never loaded into the physical host kernel. i reserves 16GiB and four
online/64 possible CPUs; stock defaults are PID max 65,536 and threads max
115,140. The outer-host admission check reserves an additional 8GiB. There is
no host sysctl tuning and no 16,384 guest-OS boot cohort.

| Component | Placement and role |
|---|---|
| Stock `vhost_vsock`, `vhost`, `vsock`, virtio transport common | Private host-role kernel; actual shared-virtqueue transport and AF_VSOCK implementation. |
| Scratch `shmproxy.ko` | Private host-role kernel; accepts AF_VSOCK streams, creates/connects/binds ordinary TCP/UDP sockets, forwards payloads and peer metadata. |
| Standalone Rust synthetic virtio peer | Private host-role userspace; produces/consumes endpoint bytes through actual vhost queues. It does not forward ordinary TCP/UDP payloads to vsock. |
| Rust TCP/UDP endpoint actors | Ordinary final source/sink sockets in the private VM; echo on those sockets, with no vsock socket or bridging relay. |
| QEMU/KVM | Physical fixture userspace/kernel; CPU, RAM and boot hardware for isolation. No application payload relay. |
| Python/canonical launcher | Build, control, source pins, capture and exclusive lease. No application payload relay. |

The real path is synthetic guest endpoint → shared virtqueue → stock vhost-vsock
kernel → accepted kernel AF_VSOCK socket → **kernel module** → kernel TCP/UDP
socket → ordinary endpoint. Reverse traffic traverses the same kernel-owned
adapter. A kernel module receiver and sender own both sides; no userspace
process reads a socket and forwards that payload into another transport.

Every owner opens its own `/dev/vhost-vsock`, registers its own CID and
256KiB guest-memory mapping, configures two split virtqueues, creates independent
kick/call eventfds and starts the stock backend. Queue descriptors and packet
payloads are in memory registered through `VHOST_SET_MEM_TABLE`; release/acquire
ordering surrounds queue indices. The kernel's vhost worker accesses that owner
memory. This is actual stock virtqueue execution, not AF_VSOCK local loopback,
a plain scratch ring, logical IDs, or 16,384 streams behind one peer.

## Stock backend seam

This backend differs from CH v53's `VsockUnixBackend`: CH constructs a Unix
muxer and executes its userspace virtio-vsock device/connection state machine.
Host AF_VSOCK sockets do not automatically join those Unix contexts. The
previous [CH device capacity proof](shared-memory-vsock-scale-findings.md)
proved 16,384 CH transport devices; it did not prove kernel adaptation.
The present 16,384 count is independently proven stock **kernel vhost** owners.
No CH production backend integration is claimed and the old userspace
TCP↔vsock relay is not used.

The read-only Linux reference is revision
`502d45774af09f1c681c754c4b7cdfb5d7f72fd9`, a **7.2 development tree**.
It is separate from the actually executed Ubuntu 7.0.0-29 kernel. Relevant
socket API, vhost, transport, BPF and TLS source files and hashes are retained in
[the source manifest](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/linux-reference-manifest.json).
Native module link/load and traffic prove the required socket symbols exist
in the executed stock kernel; the reference tree was neither built nor patched.

## Functional predictions and actual metal evidence

Predicted: 262,144 TCP bytes in each direction, independent of virtio packet and
ordinary TCP read boundaries; output after half-close; bounded peer loss; and
kernel backpressure when RX descriptors are withheld.

Actual i output:

```text
"bytes_each_direction":262144,"event":"tcp_pass","half_close_tail":"after-half-close"
"split_sizes":[1,2,3071,1733]
"stats":"accepted=1 closed=1 to_net=262144 to_peer=262160 ..."
```

Input wire headers are split across one-byte packets; TCP payload chunks use
3,071-byte packet writes, the ordinary TCP endpoint reads 1,733-byte chunks,
and returned data is reassembled byte-exactly. The 16 extra reverse bytes are
the verified tail sent by the TCP endpoint after receiving EOF. The input worker
shuts down TCP write while the independent reverse worker drains to EOF.
With no RX descriptors posted for 300ms, kernel output stopped advancing and
`send_waits` rose; subsequently posting buffers/credit updates resumed delivery.
The exact original i counters were `accepted=1 closed=0 to_net=262144 to_peer=51733 zero_udp=0 truncated=0 rejected=0 send_waits=3` before the hold and `accepted=1 closed=0 to_net=262144 to_peer=51733 zero_udp=0 truncated=0 rejected=0 send_waits=152` after it.

Predicted: UDP preserves zero-length, small and 59,000-byte messages; inbound
server traffic returns to the originating address/port, including multiple peers.

Actual i ran outgoing lengths 0/1/1431/59000 and inbound lengths 0/17/59000/1500
from two source ports. The small final **metal k** run additionally varied IPs:

```text
"event":"udp_outbound_pass","remote_address":[127,0,0,2]
"event":"udp_inbound_pass","source_peer_address":[127,0,0,2]
"event":"udp_inbound_pass","source_peer_address":[127,0,0,3]
"input_len":61000,"bound":60000,"partial_payload_delivered":false
accepted=4 closed=4 to_net=383093 to_peer=383109 zero_udp=2 truncated=1 rejected=1
SHMPROXY_HARNESS_EXIT=0
SHMPROXY_MODULE_UNLOAD_EXIT=0
SHMPROXY_COMPLETE_EXIT=0
```

Each received datagram's exact payload, whole length, source address and port
were asserted; every inbound response was observed from the actual kernel UDP
server port. The oversized datagram uses `MSG_TRUNC`: the whole datagram is
consumed, its source is retained, and no prefix becomes a successful message.
An unsupported registration is rejected; actual peer reset drains its kernel
socket owner. These are bounded SPIKE checks, not a production acceptance suite.

## Framing, copying and resource implications

AF_VSOCK is **STREAM** in this probe. UDP is not claimed to be native vsock
SOCK_DGRAM. A scratch 12-byte frame carries kind, IPv4 address, port and length,
followed by at most 60,000 payload bytes. The kernel assembles fragmented stream
reads and performs one UDP send per frame; UDP receive emits one frame per
whole datagram. Zero payload length remains a real datagram. This protocol is
test-local and creates no production API or approved wire format.

Ordering and head-of-line blocking are those of a reliable per-peer stream;
partial/large frames can delay later datagrams. One synthetic flow is used per
device. Capacity is a population of UDP adapters; it does not prove 16,384
concurrent TCP flows, fairness, throughput or latency. The prototype uses two
kernel module workers plus a stock vhost worker per live owner, fixed buffers,
blocking receives and a bounded 300s registration idle interval. The final
112.938s population check reconciled accepted−closed and all actual bindings,
so no early owner had expired while being counted.

Shared memory is not a zero-copy or fastest-path claim. Vhost reads/writes queue
buffers; kernel socket receive copies into the module's bounded kernel buffer,
and socket send copies/queues the ordinary TCP/UDP payload. The Rust peer and
ordinary endpoint also copy their endpoint bytes. No throughput comparison was
performed. Guest kTLS/control-key modules were not loaded or composed here.

## Actual population and measurements

Selected values below come from the original **metal i** output. Timers include
serial endpoint exchange and subsequent independent socket/worker enumeration;
they are not isolated network benchmarks.

| Held real owners | Live adapter sockets | Full-pool check s | Stage elapsed s |
|---:|---:|---:|---:|
| 16 | 16 | 0.023880420 | 0.097206476 |
| 64 | 64 | 0.083681135 | 0.458207542 |
| 256 | 256 | 0.334943200 | 1.600795801 |
| 1,024 | 1,024 | 1.674496229 | 6.054455132 |
| 4,096 | 4,096 | 6.761367450 | 24.091239074 |
| 8,192 | 8,192 | 13.763968456 | 53.277455015 |
| **16,384** | **16,384** | **28.565969839** | **112.937676243** |

Final exact raw fields:

```text
"actual_started_vhost_devices":16384
"actual_kernel_udp_adapter_sockets":16384
"distinct_open_vhost_fds":16384
"independent_guest_memory_contexts":16384
"independently_observed_udp_bindings":16385
"module_input_kernel_workers":16384
"module_reverse_kernel_workers":16384
"first_cid":300000,"last_cid":316383
"held_process_fd_count":81925,"held_process_task_count":16386
accepted=16388 closed=4 to_net=1930693 to_peer=1930709 zero_udp=2 truncated=1 rejected=1
MemAvailable: 9199764 kB
SUnreclaim: 5585008 kB
KernelStack: 788736 kB
VmRSS: 406356 kB
```

The additional UDP binding is the ordinary endpoint actor, not another adapter.
The input/reverse workers are independently enumerated in `/proc`; each owner
also has its actual vhost worker. The process task count includes 16,384 vhost
workers, driver main and ordinary endpoint actor. There are approximately 49,155
owned live tasks across those scopes, including the module acceptor. Kernel
slab/stack memory is separate from harness RSS and must not be omitted from the
population cost. Original i outer-QEMU CPU samples are unavailable because of
the preservation issue below; no CPU-utilization claim is made.

## Teardown and warning disposition

The harness sends stop frames, releases each actual vhost owner and checks the
closed session count; module unload stops and joins its accept/session/reverse
workers before releasing its own sockets. No unsafe forced module unload occurs.

Actual metal i output:

```text
"capacity_elapsed_s":380.28041197699997,"released_vhost_devices":16384
"process_fds":4,"process_tasks":1
accepted=16388 closed=16388 to_net=1930693 to_peer=1930709
SHMPROXY_HARNESS_EXIT=0
shmproxy: drained accepted=16388 closed=16388
SHMPROXY_MODULE_UNLOAD_EXIT=0
vhost_vsock 28672 1044 ... Live
SHMPROXY_COMPLETE_EXIT=0
```

Approximately **267.343s** elapsed between final held-stage output and capacity
cleanup. Stock `virtio_transport_close_timeout` emitted repeated workqueue
CPU-hog warnings, eventually 67 occurrences; these are retained, not suppressed
or treated as a production success. The stock vhost-vsock module still had
1,044 delayed transport references after the harness and own module drained.
It was not unloaded. Normal shutdown destroyed the entire private VM/kernel,
then QEMU exited 0. This proves isolated cleanup and own-module unload; it does
not prove all stock transport references become zero promptly in a surviving
host kernel. The small metal k functional run drained four sessions/unloaded
normally and QEMU exited 0 in 2.856858s.

[Post-release readback](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/post-release-readback.json)
confirms no owned QEMU process, no experimental module in the physical host
kernel and no canonical lease-owner metadata. Both canonical launches exited 0.
Owned builds/images/private archives remain intentionally under ignored `out/`.

## eBPF, kTLS and SSH boundaries

The Linux **7.2 reference** has `vsock_bpf_update_proto` and virtio/vhost
`read_skb` support. Sockmap admission includes established STREAM/SEQPACKET
vsock and bound UDP sockets. Therefore a categorical statement that vsock
cannot use eBPF would be incorrect. `SK_SKB` redirect supports egress to vsock
but rejects `BPF_F_INGRESS` targeting vsock; `SK_MSG` redirect rejects vsock
targets and restricts non-ingress targets to TCP. Parser/pull/change-head/tail
helpers exist for SK_SKB; message push/pop/cork helpers are a different SK_MSG
interface. Their existence does not prove bidirectional cross-protocol framing.

eBPF data redirection between existing sockets is distinct from socket creation,
connect/accept, ownership, lifecycle and ordinary guest address-family
adaptation. Userspace socket/map **control** could coexist with a kernel payload
path; it is not a forbidden userspace application relay. This spike builds and
loads no BPF program and makes no native BPF forwarding claim.

The 7.2 reference `net/tls/tls_main.c` explicitly rejects upgrading an attached
psock to TLS, and `net/core/skmsg.c` rejects inet ULP sockets. That is a
version-specific source constraint on sockmap and an existing kTLS socket.
Separate non-TLS host sockets carrying opaque guest ciphertext are a different
boundary. Neither guest ordinary TCP/kTLS adaptation nor the host-held key
control channel is validated here; no combined #303/#306 test is claimed.

TCP byte forwarding could carry SSH once ordinary guest TCP/sshd listener
adaptation exists. The current synthetic endpoint proof does **not** prove
unmodified guest sshd is reachable. No SSH server or userspace vsock relay was
added to manufacture that claim.

## Sources, receipt integrity and limitations

Primary executable sources are
[metal i Rust](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/src/main.rs),
[metal i kernel glue](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/module/shmproxy.c),
[metal i runner](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-i/run.py),
and [metal k functional Rust](../../../../spike-scratch/netns-density-295-shm-vsock-kernel-proxy/increment-k/src/main.rs).
The exact original i launcher is retained as `increment-i/evidence/launch.executed.py`.

The original i launcher retrieved an ignored out archive instead of native
diagnostic JSON. Later k source sync replaced remote i JSON receipts. The
complete original stdout/serial, every live audit event, source manifest and
exact executed kernel/module/harness/rootfs archive survived. The archive SHA256
is `9ce703db16577ac0c35e28d31182c8f155d277f191bd5b6cc4c570b4e4e9546e`.
Extracted copies explicitly say `from-launch` or `derived`; they are not presented
as original native JSON. i outer CPU samples and original command-result JSON
are missing. The later launcher issue that overwrote k's local bootstrap log
is also disclosed in `launcher-result.recorded.json`; k's complete native
serial, command results, hashes and resource samples remain intact. The current
launcher retrieves evidence only and preserves its local launcher log.

| Evidence | Location under the scratch tree |
|---|---|
| Original complete metal i output | `increment-i/evidence/native-launch.log` |
| Exact parsed i events | `increment-i/evidence/events.from-original-launch.json` |
| Retained actual executable hashes | `increment-i/evidence/retained-executed-artifact-pins.derived.json` |
| Exact metal k native serial/commands/pins | `increment-k/evidence/native-serial.log`, `commands.jsonl`, `executed-artifact-pins.json` |
| Actual metal k QEMU topology/results | `increment-k/evidence/qemu-command.json`, `native-result.json`, `host-admission-budget.json` |
| Linux reference source hashes | `linux-reference-manifest.json` and `linux-reference/` |
| Final physical-host cleanup attestation | `post-release-readback.json` and its executed script |
| Prior failed/preparation attempts | Preserved increments a–h/j; historical Lima evidence does not establish final claims. |

Executed metal i kernel image SHA256:
`b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`.
Its actual module SHA256:
`736801736b5ee5468e0f6c190474d84c56afb07cc047d536003122367be36df5`.
Its actual Rust binary SHA256:
`3e5c566b32a498c10c019632739733dbafda1d84a2936fe525cd4c4027682a7f`.
The module/harness hashes match those printed by the private kernel's init
before execution. No measured source was rewritten after its run.

**Gate recommendation:** the host kernel forwarding feasibility and 16,384
UDP-adapter population assumptions pass; keep VMM integration, guest ordinary
socket/kTLS adaptation and production teardown guarantees open for separate
bounded validation. No DESIGN promotion is implied.
