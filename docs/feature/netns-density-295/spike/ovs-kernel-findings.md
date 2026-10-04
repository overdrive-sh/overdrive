# OVS stock-kernel datapath comparison: 16,384 real TAP attachments

**Verdict: WORKS for the bounded comparison.** The distribution's stock
`openvswitch` kernel module held **16,384 distinct TAP netdevices, 16,384 NETDEV
vports and 16,384 live TAP owner FDs**, plus its separate internal local vport.
At that population every TAP completed gateway ARP and ICMP exchanges, and all
16,384 TAPs exchanged byte-identical TCP Ethernet frames through kernel OUTPUT
actions in 8,192 bidirectional pairs. The forwarding path used no userspace
application relay and no OVS userspace packet-processing daemon.

This is the requested OVS/TAP comparison. It does **not** establish the selected
no-TAP shared-memory/vsock application path, its ordinary-TCP adaptation, or its
kernel forwarding. No production direction is selected or promoted here.

## Question and boundary

Can the stock Linux OVS module hold 16,384 actual independent TAP/netdevice/vport
attachments and forward real traffic in the kernel, using an exact stock
control model without a userspace application forwarding proxy?

The capacity objects are nonpersistent, single-queue `IFF_TAP | IFF_NO_PI`
devices. Each has an independent `/dev/net/tun` open file description, a
distinct ifindex within one isolated namespace, and an explicit stock OVS
NETDEV vport numbered 1–16,384. They are real held objects, not OpenFlow flows,
aliases, logical endpoints, booted VMs or sparse high-ID samples. **Guest boots:
zero. VMM processes and virtio-net devices: zero.** The synthetic TAP writers
and readers stand in for the two endpoint queues; they do not forward between
the queues.

The native probe completed on 2026-10-04, within the 60-minute spike budget.
The executable imports only `libc` and `serde_json`, in a standalone Cargo
workspace. It imports no Overdrive crate and changes no production source,
ADR, roadmap, DES event or existing dirty file.

## Native platform and provenance

- Physical x86_64 canonical metal host; Linux `7.0.0-29-generic`, 16 logical CPUs,
  `MemTotal=65,401,772 KiB` (62.372 GiB), no swap.
- Kernel image and module package version `7.0.0-29.29`; `modinfo` reports
  `intree: Y`, matching vermagic and srcversion
  `1385EBD8343BF79830D5E14`. The loaded module's taint field is empty.
- `/lib/modules/7.0.0-29-generic/kernel/net/openvswitch/openvswitch.ko.zst`:
  SHA-256 `5325df6f19026a892e5df54e4f03a56ec62ae7b9044301be4d067666b4b74234`.
- OVS userspace tools were absent. No `ovs-vswitchd`, `ovsdb-server`, `ovs-dpctl`,
  netdev datapath, DPDK, kernel patch, custom module or eBPF program was used.
- Final executed Rust binary SHA-256:
  `d0b515251843f182eda348f82a8316bc3b463084f5d5490ec5b06e5f9c3729d2`.
- Final executed Rust source SHA-256:
  `cddc205e5f58684c1e6da7f9417e0ace5f742905d2b72833d682f48762549770`.
  Post-release native source hashes match the retained local increment-d
  sources; every attempt retains its executed Cargo lock and build output.

Primary receipts: [module attestation](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/kernel-module-attestation.txt),
[module file hash](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/module-file-sha256.json),
[native events](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/events.jsonl),
and [post-release readback](../../../../spike-scratch/netns-density-295-ovs-kernel/post-release-readback.json).
The package readback's exit 1 is specifically the absent optional
`linux-modules-extra` package; its installed image/module package results are
preserved, and no claim depends on that optional package being installed.

## Exact forwarding and control model

The stock Generic Netlink families `ovs_datapath`, `ovs_vport`, and `ovs_flow`
create and configure one datapath named `ok295dp` inside the attempt's private
network namespace. Its local vport is INTERNAL port 0. Every vport's upcall PID
array is exactly `[0]`; the datapath is created with upcall PID 0 and user
features 0. Configuration calls request ACK and the documented `NLM_F_ECHO`
object reply. The [Linux UAPI](https://github.com/torvalds/linux/blob/v7.0/include/uapi/linux/openvswitch.h)
defines these existing interfaces and distinguishes upcalls, USERSPACE actions
and userspace packet EXECUTE requests.

During the gateway exercise, there are exactly two preinstalled, disjoint
wildcard flows per TAP:

| Match | Sole kernel action |
|---|---|
| Exact guest ingress vport; other fields wildcarded | OUTPUT to local port 0 |
| Exact local ingress port 0 and exact guest destination MAC | OUTPUT to that guest's vport |

The flow's supplied key includes Ethernet and ethertype `0x88b5`, but its
ethertype mask is wildcarded: the actual ARP and IPv4 traffic exercises that
mask. Independent dumps retain the kernel's returned keys, masks and actions;
all installed actions are OUTPUT, never USERSPACE. Gateway MAC is
`02:aa:29:50:00:01`, gateway address `198.18.0.1/16`, and guest identities cover
`198.18.0.2` through `198.18.64.1` with MAC `02:00` followed by their IPv4 bytes.
The Linux kernel local port answers ARP and ICMP. Every guest has an owned
PERMANENT neighbor on the local port; independent readback counts all 16,384.
No global neighbor thresholds, routes, forwarding sysctls or limits change.

At held population 16,384, the guest-ingress flows are temporarily changed to
OUTPUT directly to the paired guest vport, `peer_index = index XOR 1`.
Each endpoint writes one valid Ethernet/IPv4/TCP segment with a unique identity
and 32-byte payload. The opposite TAP reads the complete 86-byte frame, which
must equal the original exactly. Both directions of every pair are exercised.
No TCP socket is opened or terminated by the host or an OVS userspace process.
These are valid synthetic TCP segments, not established workload TCP sessions
or a kTLS/mTLS test. Flows are then restored before the removal/drop control.

This permissive forwarding fixture proves capacity and packet preservation;
it does not implement production tenant isolation, policy or topology.

## Held populations and traffic

Independent audits use a separate Python Generic Netlink socket, `ip -j -d
link`, and `/proc/<owner-pid>/fdinfo` while the Rust harness waits. Each TAP's
name and ifindex agree with its kernel NETDEV vport and the owner FD's `iff`
readback. Netlink confirms TAP mode and the OVS master. Port 0 is counted
separately: at N=16,384 the datapath has **16,385 total vports**, and the
namespace has 16,386 netdevices including loopback.

Final increment-d results:

| Held TAPs / NETDEV vports / TAP FDs | ARP gateway round trips | ICMP payload round trips | Kernel flows | Incremental attach + flow install | Gateway traffic wall time |
|---:|---:|---:|---:|---:|---:|
| 4 | 4 | 8 | 8 | 0.153 s | 0.000884 s |
| 1,024 | 1,024 | 2,048 | 2,048 | 0.779 s | 0.169 s |
| 4,096 | 4,096 | 8,192 | 8,192 | 2.524 s | 0.676 s |
| 8,192 | 8,192 | 16,384 | 16,384 | 3.893 s | 1.354 s |
| 16,384 | 16,384 | 32,768 | 32,768 | 9.815 s | 2.738 s |

All identities were exercised at each row. The full-population TCP phase adds
**16,384 exact frames, 8,192 bidirectional pairs and 524,288 TCP payload bytes**
in 0.767 s. These serial microprobe timings include receipt writing and, in
increment-d, control-wire logging. They are not throughput, E18, production
owner, recovery-window or VM density measurements.

[Full-population kernel dump](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/stage-16384-kernel-dump.json.gz),
[TAP link dump](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/stage-16384-links.json.gz),
[owner FD readback](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/stage-16384-tap-fdinfo.json.gz),
[identity list](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/identities-16384.json.gz),
[all full-population gateway frames](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/gateway-traffic-16384.jsonl.gz),
and [all TCP pair frames](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/paired-tcp-traffic-16384.jsonl.gz)
are retained losslessly. ICMP payload and endpoint bytes are exact; the response
IPv4 ID is kernel-selected and its independently checked header checksum is
allowed to differ. TCP comparison covers the entire frame without masking.

## First packets, steady packets, misses and userspace handling

The final native output is:

```text
cold, before any flow: hit=0 missed=1 lost=1 flows=0; no reply
all gateway cohorts: hit=178200 missed=1 lost=1 flows=32768
all paired TCP frames: hit=194584 missed=1 lost=1 flows=32768
remove TAP-0 ingress flow: hit=194584 missed=2 lost=2 flows=32767; no reply
actual sent control datagrams=81943; received control datagrams=163886
packet EXECUTE requests=0; received ovs_packet-family messages=0
USERSPACE actions across all flow installs=0
```

Each port's first configured frame is an ARP broadcast; subsequent ICMP and TCP
traffic also matches the preinstalled kernel flows. Configured traffic adds
zero misses. The two intentionally unconfigured frames are the only misses;
both increment `n_lost` and produce no reply. With upcall PID 0, they are
dropped rather than delivered to a userspace handler. There is no fallback.

The [complete wire capture](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/netlink-wire.jsonl.gz)
covers the harness control socket from first family resolution through the
acknowledged datapath deletion. A separate offline parser checks every sent
datagram, all flow actions, all vport upcall PID attributes, every captured
packet, endpoint coverage and checksums. Successful miss delivery is zero over
the datapath lifetime: `n_missed == n_lost`, with exact intended miss controls.
The control socket's zero packet-family messages is supplementary evidence,
not a claim that it monitored every system socket.

Ordinary OVS deployments may send cache misses to a userspace handler and use
packet EXECUTE commands; matched flows run in kernel. That is described by the
[official OVS datapath guide](https://github.com/openvswitch/ovs/blob/b89dc80f0143f5bf3f58c48d54e82cc7f8d29da8/Documentation/topics/datapath.rst).
An OVS packet slow path and a TCP connection-terminating application relay are
different mechanisms. This direct-UAPI experiment uses neither, and does not
measure or adopt the standard OVS daemon/control stack.

## Resources and cleanup

| Held TAPs | Rust owner FDs | Rust owner threads | Rust RSS | Rust PSS | Whole-host MemAvailable |
|---:|---:|---:|---:|---:|---:|
| 4 | 10 | 1 | 3,768 KiB | 1,856 KiB | 63,339,668 KiB |
| 1,024 | 1,030 | 1 | 4,888 KiB | 2,976 KiB | 63,219,692 KiB |
| 4,096 | 4,102 | 1 | 8,476 KiB | 6,564 KiB | 62,721,468 KiB |
| 8,192 | 8,198 | 1 | 13,252 KiB | 11,340 KiB | 61,982,940 KiB |
| 16,384 | 16,390 | 1 | 23,836 KiB | 21,924 KiB | 60,550,840 KiB |

Six owner FDs are control/trace/stdio overhead in addition to the 16,384 TAP
FDs. Only that Rust process's RSS/PSS is reported. Kernel netdevices, queues,
OVS flows and neighbors are outside that process RSS/PSS. Whole-host
MemAvailable fell from 63,418,752 KiB before the final run to 60,550,840 KiB at
the full-population gateway audit (2.735 GiB difference); that whole-host delta
also includes other processes, receipt buffers and page cache. It is not an
attributed per-port kernel allocation measurement. Raw slab and memory
snapshots are retained. There were zero new OOM kills and the 8 GiB reserve
was never approached. The owner uses its existing hard NOFILE limit as its
process-local soft limit; global limits are unchanged.

Final exact cleanup took **2.699 s to delete the datapath** and **99.548 s to
close its 16,384 TAP FDs**, then the owned namespace was deleted. The long TAP
closure is an observed cost, not a production recovery guarantee. Native run
wall time was 149.869 s including audits, cleanup and evidence compression.

All successful runs matched normalized foreign links, addresses, routes,
rules, nftables, BPF objects, namespaces, sysctls and neighbor IP/device/MAC
identities. Foreign OVS datapaths remained empty. Raw neighbor states and a
live root-namespace neighbor event capture are preserved; the final live
capture is empty. Normalization excludes cache ages, reference counts, packet
counters and bridge GC timer values, never neighbor identity or MAC. Thus
dynamic neighbor preservation is observed separately from configuration.

The initial attempt loaded the stock OVS module and its stock dependencies;
they are intentionally left loaded. Module inventory equality is not asserted,
and no global unload, foreign object cleanup or global flush was performed.
Post-release readback independently finds all 12 recorded lease/runner/harness
PIDs absent, every owned namespace path and lease-owner path absent, no OVS
userspace daemon, and zero root-namespace OVS datapaths.

## Attempts and retained reproduction

| Increment | Native result | Purpose / correction |
|---|---|---|
| a | DOESN'T-WORK setup; zero TAPs | Omitted NLM_F_ECHO, then incorrectly expected a reply after a valid creation ACK. Exact isolated cleanup completed. Normalized complement initially differed only in bridge GC timer and neighbor reference count; raw neighbor event capture was empty. |
| b | WORKS; 16,384 held TAPs | Added the stock ECHO flag, corrected normalization of age/reference fields, proved all-port kernel ICMP and counted real kernel objects. |
| c | WORKS; 16,384 held TAPs | Added every-port ARP receipts and full-population exact bidirectional TCP frame forwarding; measured deletion/closure separately. |
| d | WORKS; 16,384 held TAPs | Added complete actual Netlink request/reply capture and independent permanent-neighbor readback; closes the zero packet-EXECUTE evidence gap. |

Each increment retains its own source, executed lock, source manifest, binary
hash, native stdout/stderr, lease receipt, independent readbacks and cleanup.
Upstream `.c` files are pinned reference source, never compiled probe or eBPF
code. The final independent receipt verification passes:

```sh
python3 spike-scratch/netns-density-295-ovs-kernel/verify-receipts.py
```

[Verification result](../../../../spike-scratch/netns-density-295-ovs-kernel/receipt-verification.json)
records every endpoint covered, exact TCP equality, valid IP/TCP/ICMP
checksums, zero packet EXECUTE / USERSPACE actions and completed post-release
cleanup. [Compression manifest](../../../../spike-scratch/netns-density-295-ovs-kernel/compression-manifest.json)
records hashes of original bytes; large captures are lossless gzip. Exact
original native archives remain under ignored `out/`; shared receipts redact
only the configured metal address. Launches were:

```sh
python3 spike-scratch/netns-density-295-ovs-kernel/launch.py increment-a
python3 spike-scratch/netns-density-295-ovs-kernel/launch.py increment-b
python3 spike-scratch/netns-density-295-ovs-kernel/launch.py increment-c
python3 spike-scratch/netns-density-295-ovs-kernel/launch.py increment-d
```

These retained increments are immutable attempts. A new run needs a new
increment/output directory; rerunning into an existing evidence directory is
intentionally refused.

## Documentary interaction with #303 / #306 and disposition

The cached [issue #303](https://github.com/overdrive-sh/overdrive/issues/303) and
[PR #306](https://github.com/overdrive-sh/overdrive/pull/306) bodies describe
guest workload TCP sockets with guest kTLS, host-held SVID private keys and
host rustls/policy, and vsock for handshake/resolution coordination. Keeping
Ethernet/IPv4/TCP bytes intact in kernel switching is compatible in principle
with that application-data transport. This experiment does not test guest
kTLS, key custody, handshake/resolution, established connections, policy,
control-process restart or any combined guest/mTLS execution.

It also supplies no evidence that shared-memory/vsock intrinsically requires a
userspace relay. The earlier TCP/vsock relay prototype and the Unikraft article
do not establish that intrinsic requirement, and an unspecified custom proxy
does not locate the forwarding boundary.

**Disposition recommendation:** retain these receipts as a validated OVS/TAP
comparison for independent review. The stock OVS module removes the observed
Linux bridge's 1,023-port allocator obstacle at 16,384 actual attachments in
this harness. It remains a finite vport/flow/resource system, with a material
measured TAP cleanup cost. No production adoption, walking-skeleton promotion,
DESIGN dispatch or roadmap approval follows from this result. The selected
no-TAP shared-memory/vsock kernel application path needs its own evidence.
