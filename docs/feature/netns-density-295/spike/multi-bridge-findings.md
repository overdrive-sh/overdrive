# Multi-bridge PROBE — netns-density-295

Status: **COMPLETE — scoped results below**. Date: 2026-10-04. PROBE only; no replacement DESIGN or promotion decision. No production/test/design/roadmap/DES edit or commit by this agent.

## Question and budget

Can a bounded set of stock Linux bridges in one host network domain provide more than 1023 actual TAP attachments and the required unchanged guest IPv4/gateway/peer/service traffic behavior, without per-VM network namespaces or Linux kernel patches?

Mechanism validation; no invented latency SLO. Sixty-minute dispatch budget. The 16384 population is a placeholder, and this probe does not promise 100000 simultaneously running microVMs.

## Verdict

**WORKS — actual attachment fanout.** Five stock Linux leaf bridges and one root retained **4097 actual TAP interfaces and 4097 queue FDs**, crossing the former single-bridge and NetSlot boundaries. Each leaf remained within 1023 total ports, including its uplink.

**WORKS — two real guests' ordinary networking.** Two actual Cloud Hypervisor guests, on different leaves among **1024 actual TAPs**, used the unchanged `/16`, gateway and deterministic guest MACs. Both completed peer TCP, gateway TCP, off-subnet TCP and gateway DNS exchanges.

**DOESN'T WORK — the exact untuned active-neighbor configuration.** In a separate 1100-TAP burst check, gateway ARP replies stopped at source **342**, after 341 successful distinct sources. Each of two leaves and the root held 341 neighbors: 1023 visible probe neighbors. Native `arp_cache` readback retained thresholds `128/512/1024`, reported 1025 total entries and raised `table_fulls 0→9`.

The broad question is **not fully proved or universally disproved**. Multiple bridges remove the port allocator bottleneck, and two real guests work; this untuned configuration fails larger active-neighbor admission. A differently contracted neighbor scheme is untested. Completing the full approved Overdrive security/owner contract is **BIGGER THAN EXPECTED** than this primitive: the replacement neighbor, guard and owner contracts are unpinned. No missing contract was filled with an invented API, changed guest prefix or weakened invariant. The roadmap remains pending.

The 250 ms ARP receive window is an observation bound, not a product SLO or permanent-failure claim. This is a failed burst/configuration, not a universal 341-VM ceiling. Existing cache state, GC age, number of bridges and traffic timing can change its failure point.

## Exact experiment and inputs

The probe read the [replacement research](../../../research/netns-density-295-networking-replacement.md), [independent review](../../../research/netns-density-295-networking-replacement-review.md), relevant [feature-delta requirements](../feature-delta.md), ADRs 0114/0116/0126 and retained inherited-TAP probe code. The feature delta records that separate DISCUSS files are absent and the original issue/spike record supplies DISCUSS-equivalent input. No normative artifact was revised.

- Each native epoch used one unnamed, unshared **node-level** network domain as a safety harness. All root/leaf bridges, veth uplinks and TAPs shared it. No VM had a separate network namespace.
- Root: `100.95.0.1/16`, MAC `02:01:00:00:00:01`, no physical uplink. Leaves: distinct host MACs `02:01:00:00:01:<ordinal>`, no guest IPv4 address, one veth uplink each, at most 1022 TAPs plus that uplink.
- Owner: actual `/dev/net/tun` queue FDs, uid-0 TAP ownership, disabled TUN offloads, retained queues. Primitive epochs used `IFF_TAP|IFF_NO_PI`; the VM epoch also used `IFF_VNET_HDR` and inherited selected queues into Cloud Hypervisor.
- Guest A: `100.95.0.2/16`, MAC `02:00:64:5f:00:02`, leaf 0. Guest B: `100.95.4.0/16`, MAC `02:00:64:5f:04:00`, leaf 1. Both used gateway/DNS `100.95.0.1`. `.4.0` is a valid host address in this `/16`.
- Scratch host-local `10.98.0.1/32` represented the off-subnet destination. Its TCP/UDP endpoints and the DNS answer were fixture services, not the production Service dataplane/DNS owner.

No kernel/constants/module implementation, production classifier, production nft table, guest prefix, gateway or guest-MAC scheme changed. No production `ovd-gbr0`, retained program or pin was adopted. No permanent-neighbor, route-per-leaf, ARP/eBPF workaround or threshold tuning was introduced.

## Native qualification and byte identity

Every mutation epoch used canonical `cargo xtask metal run`, its exclusive lease and native KVM preflight. Lease acquisition timeout was one second. The user deferred the vsock spike until completion; no other spike overlapped these epochs.

| Item | Native observation |
|---|---|
| Substrate | Nonvirtualized x86_64; KVM API/open/create-VM preflight passed |
| CPU | AMD EPYC 8024P, 8 cores / 16 logical CPUs |
| Kernel | `7.0.0-29-generic`, Ubuntu package `7.0.0-29.29`, kernel taint 0 |
| Source-package association | Image: `linux-signed 7.0.0-29.29`; modules: `linux 7.0.0-29.29` |
| TUN / bridge | TUN built in; stock in-tree bridge module v2.3, matching loaded srcversion `8AE48B38B6BD1D04D607403`, untainted |
| Package integrity | `dpkg -V linux-modules-7.0.0-29-generic`: rc 0, empty output |
| Cloud Hypervisor | v53.0; actual binary and running VMM executable bytes hashed |
| Memory before epoch 02 | Approximately 63.3 GB available; full meminfo retained |
| FD limit | Process-only soft limit `1024→16384`; hard limit remained `524288` |
| Other limits | `fs.nr_open=2147483584`, `kernel.pid_max=4194304`; ARP thresholds `128/512/1024` |

CPU/NUMA, NIC driver/firmware/queues/offloads, tool versions and kernel configuration are retained in preflight/final receipts. Traffic used virtual links; no physical-NIC throughput is attested.

| Actual bytes | SHA-256 |
|---|---|
| Parent `/proc/<probe-pid>/exe` | `52e0a13e60a981d8c4b6478be2ba5176f69da07948a056bf49cf6f077e30cb41` |
| CH / actual running VMM executable | `448af3d4e59b22c2987f7df94c213ad40fb53a10d437e42b5ee6c4fce7c29ecc` |
| Host vmlinuz and selected guest kernel | `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682` |
| Stock bridge module file | `572e0baa251e3f9ece7a039a105d556d29499ce6965488feeb7f0d702097f64a` |
| Stock veth module file | `eb3e9c5d4e26ee6c123f0be46d515d60f38b4042eaab10e06371bbd9410954d6` |
| Master guest rootfs, before/final after | `4f7f97841e22384f1aebb040e199d78844ab418ad4859e6c262809d557898d85` |
| Epoch 02 probe source | `1264cc87a882585f6a7b6ff9e9282ddb945c61c97e655064cd7dac965c8ab828` |
| Epoch 03 probe source | `5e17ffc6c64f245848ad72374239f4163db92d76e7c3a7411efdd9e7cb089321` |
| Epoch 03 guest source / executable | `c4899c7c2d59f07507e2e02211e5875effe9f5b72fad6bfa9df59349c4f57178` / `19a9b513f6621de0e0a844ad7ac7dee208fa0556d263bd2ab5cbdb59a207b820` |
| Epoch 04 probe source | `40dab97507c442d832c6a37b093d5fa466477d659fd813a36206c0d3d67f1641` |

Workspace source receipts identify HEAD `5ea2d0c506f5c1f0672c2a4b5b7c5d0d415d6bde`, canonical source markers and pre-existing dirty hashes. Parent attestation hashes actual retained executable bytes, not a subprocess utility's `/proc/self/exe`.

Package association/bytes are established; an exact deployed downstream source checkout/reproducible-build association was not obtained. Supplemental neighbor source was `/Users/marcus/git/linux` at `502d45774af09f1c681c754c4b7cdfb5d7f72fd9`, not claimed as deployed source.

## Fanout — native epoch 02

Every stage independently read `ip -j -d link`, bridge ports, FDB and owner fdinfo.

| Actual TAPs / queue FDs | Bridges | Tree bridge ports | Root uplinks | Largest leaf ports | Cumulative time with readback |
|---:|---:|---:|---:|---:|---:|
| 2 | 2 | 4 | 1 | 3 | 0.245 s |
| 1022 | 2 | 1024 | 1 | 1023 | 1.210 s |
| 1023 | 3 | 1027 | 2 | 1023 | 1.371 s |
| 1024 | 3 | 1028 | 2 | 1023 | 1.518 s |
| 1025 | 3 | 1029 | 2 | 1023 | 1.677 s |
| 2044 | 3 | 2048 | 2 | 1023 | 3.025 s |
| 4095 | 6 | 4105 | 5 | 1023 | 6.823 s |
| 4096 | 6 | 4106 | 5 | 1023 | 7.432 s |
| 4097 | 6 | 4107 | 5 | 1023 | 8.007 s |

At 4097, leaf port counts were `1023,1023,1023,1023,10`; root count was 5. The owner held 4097 distinct named queues and 4104 total FDs including observer/control FDs. **Zero VMMs** ran in this fanout epoch; only two queue endpoints subsequently emitted guest-shaped packets.

Creation/attachment/admin-up intervals summed to **5.100 s**. Stage time 8.007 s includes preflight/readbacks. Exact interface cleanup: **53.072 s**. Probe total: **61.496 s**; canonical launcher with sync/qualification: **91.669 s**.

Root-only baseline→4097 global counter deltas: `Slab +640364 KiB`, `SUnreclaim +637484 KiB`, `MemAvailable -775496 KiB`. Owner RSS: `30532→91364 KiB` (`+60832 KiB`). These include global host/kernel activity, RCU/default IPv6 state and the Python observer's JSON retention; they are not isolated per-TAP costs, steady-state bounds or a capacity estimate.

FDB count was **20637**, including local/permanent/multicast records, not that many active guest MACs. Final routes numbered 17853 across both families, largely default IPv6 link-local/local/multicast records. IPv4 had the one guest-prefix connected route plus root/loopback local/broadcast records. This was not 4097 active neighbors or endpoint-map entries.

## Traffic results

At 4097 actual TAPs, exact source/destination frame bytes proved cross-leaf ARP request/reply, IPv4 unicast both directions, gateway ARP/ICMP from both leaves and off-subnet UDP to `10.98.0.1:18951` with replies to the expected guest MAC. Single timings: ARP cross-leaf 5.169 ms / 26.5 µs; unicast 18.3 / 16.0 µs; gateway ARP 4.172 / 3.996 ms; gateway ICMP 38.2 / 25.2 µs; service UDP 53.3 / 37.5 µs. These few cold operations are not latency percentiles. Direct peer UDP was a deliberate primitive check and is forbidden by current production policy; its success is not policy compliance.

Epoch 03 booted **two real CH guests among 1024 TAPs** from copies of the selected image with a small credential-free static Rust fixture as PID 1. Both consoles recorded exact `/16`, gateway/MAC and `REAL_VM_ALL_TRAFFIC_PASS`.

| Journey | Guest A | Guest B |
|---|---:|---:|
| Peer TCP across leaves | PASS, 29 bytes, 5.778 ms | PASS, 29 bytes, 7.334 ms |
| Gateway TCP `100.95.0.1:18950` | PASS, 32 bytes, 2.871 ms | PASS, 32 bytes, 2.734 ms |
| Off-subnet TCP `10.98.0.1:18951` | PASS, 43 bytes, 0.408 ms | PASS, 43 bytes, 0.331 ms |
| UDP DNS gateway port 53 | PASS, peer address/transaction ID | PASS, peer address/transaction ID |

Host sockets independently recorded original local destinations, guest tuples and distinct payloads. Spawn→final-result times: 3.239 / 3.236 s. Guest build: 0.371 s. Each VMM had about 148948 KiB RSS, 10 threads and 256 MiB configured guest RAM. Running executable bytes matched CH. VMM fdinfo showed descriptor duplicates of the same inherited queue, not multiple queues.

Scratch VMMs ran uid/gid 0; the parent retained queue descriptors for the attachment count. Production confinement/seccomp/cgroup/descriptor-release lifecycle was not exercised. The epoch-03 generic `attachment-verdict` incorrectly logs `vm_count:0`; the separate actual-VM summary, two PIDs/consoles/executable receipts and packets establish **two VMs**. The logging field was corrected later; historical evidence remains intact.

An `any`-interface observer in the isolated domain was armed **before either VMM spawn**. It reported 30427 captured, 31240 received by filter, zero kernel drops. The received/captured difference is retained; this is not a lossless zero-leakage attestation.

**Security composition remains unproved.** TCP intentionally used fixture plaintext over the tree. No production TCX source IPv4/MAC/ARP validation, target MAC gate, independent classifier/IP-program-loss guard, TPROXY, protected leg-B/leg-C, TLS 1.3, kTLS, splice, listener-loss behavior or zero-frame activation was installed or relaxed. Successful ordinary traffic does not prove those mandatory outcomes. The scratch endpoints are not the production DNS/Service owners.

## Neighbor-pressure falsification — epoch 04

At the 4097-TAP primitive scale, only two identities already produced **12 learned IPv4 neighbors**, one per identity per bridge. That actual replication motivated the final bounded check.

Epoch 04 retained 1100 TAPs, two leaves/root and emitted one gateway ARP from each successive queue, with a distinct valid `/16` IP and deterministic guest MAC. No tuning or workaround was added. It stopped after three successive missing replies.

| Distinct sources attempted | Reply successes | Visible probe neighbors | Leaf 0 / leaf 1 / root | Native table_fulls |
|---:|---:|---:|---|---:|
| 100 | 100 | 300 | 100 / 100 / 100 | 0 |
| 300 | 300 | 900 | 300 / 300 / 300 | 0 |
| 342 | 341 | 1023 | 341 / 341 / 341 | 3 |
| 343 | 341 | 1023 | 341 / 341 / 341 | 6 |
| 344 | 341 | 1023 | 341 / 341 / 341 | 9 |

First missing reply: index 341, `100.95.1.87`; next `.88` and `.89`. Observation durations: 250.930, 251.654 and 252.556 ms. Workload: 1.442 s excluding setup. Independent `ip -j -s ntable show` retained `thresh3=1024`, total entries `1025`, `forced_gc_runs 0→11` and `table_fulls 0→9`. Visible neighbor identity dumps and per-source outcomes are separate receipts.

The complete tested primitive path was queue write→source TAP→source leaf→flooded leaf/root ARP receive→kernel neighbor admission→gateway reply or missing reply. Kernel exhaustion counters support the admission explanation independently of any remedy. Supplemental pinned source has one global `arp_tbl`, defaults `128/512/1024`, and `neigh_alloc` records `table_fulls` when forced GC cannot admit another GC-managed entry (`arp.c:152–183`, `neighbour.c:499–515`). Forced GC walks the table's global GC list. These are source-pin facts, not deployed-source attestation or a new Overdrive control-plane defect.

An exact alternative neighbor/ARP-admission contract is required before claiming larger active traffic. Global thresholds must not be treated as namespace-local settings. Tuning, suppressing leaf learning, permanent neighbors and alternative ARP delivery were neither performed nor selected.

## Cleanup and foreign-state evidence

The probe closed its queue FDs, stopped/awaited exact child PIDs, deleted only recorded veth/leaf/root names, restored the parent namespace, then let its unnamed domain die. No global route/nft/BPF purge, module unload, broad kill, foreign adoption, mount/loop setup, system-wide sysctl write or benchmark overwrite occurred.

| Epoch | Owned interface cleanup | Errors / remaining owned links | Host configuration complement |
|---|---:|---|---|
| 01 | Raw cleanup link dump has only loopback; logging failed afterward | Process/domain ended, no probe links in dump | External after snapshot retained |
| 02 | 53.072 s | 0 / 0 | GC-timer-only corrected normalization matches |
| 03 | 10.978 s | 0 / 0 | Matches |
| 04 | 12.730 s | 0 / 0 | Matches |

Comparisons cover host link/admin identity, addresses, routes, rules, nft configuration, BPF map/link identity and loaded-module configuration. Raw counters/lifetimes/timers/module use counts remain preserved. Normalization removes running values, not bridge/admin/threshold configuration. Epoch 02's sole remaining difference was the existing bridge's `gc_timer` (`35.81→0.0`); its exact diff and countdown justification are retained separately. Epoch 01's post-run external inventory is separate because a logging exception prevented its in-process final snapshot.

**Foreign dynamic-neighbor preservation is unproved.** No before snapshot of foreign neighbor identities/states was taken. Global ARP entries started at 7 and final post-cleanup readback reported 1. Global forced/normal GC can affect cache entries; available evidence cannot reconstruct their membership or assign every change. Configuration-complement equality must not be read as preservation of every foreign dynamic neighbor object. No foreign cache was flushed, reconstructed or tuned to repair the unknown baseline.

[Final cleanup attestation](../../../../spike-scratch/netns-density-295-multi-bridge/receipts/spike-multi-bridge-final-cleanup.json) independently confirms owner metadata absent, all eight recorded task/lease/VMM/capture PIDs absent, no `m295*` host link, master image hash unchanged and ARP thresholds still 128/512/1024. Every canonical launcher released its lease; no native mutation followed epoch 04.

## Design implications and unclosed gates

1. Stock bridge composition provides actual port fanout, while root/uplinks retain finite budgets. Five leaves were demonstrated, not 17 leaves, 16384 TAPs or 100000 running guests.
2. Existing `/16`, gateway and deterministic MACs worked for two real guests without routed leaf subnets or an invented ARP responder. Production TCX local delivery/guard placement remains unproved.
3. Default dynamic-neighbor admission failed a reachable native burst. Adding leaves is insufficient. Any alternative needs an exact, independently reviewed neighbor/ARP/OS contract and native admission/cleanup evidence.
4. One injected network owner must still own root/leaf/uplink identities, whole-tree inventory, allocation/shared damage, activation and reverse subtree cleanup. Exact public/cross-crate APIs, formats and typed failure mappings remain DESIGN's responsibility; this probe added none.
5. Protected traffic/source/egress/isolation/loss gates, actual owner effects, lifecycle/restart/foreign-state proofs and later hardware/runtime-bound evidence remain mandatory. No Sim seam, control-plane finding, persistence or recovery mechanism was invented.
6. FDB/default-route growth, ARP amplification, queue/broadcast cost, active neighbor/endpoint membership and loss-accounted observation require further evidence. No physical-NIC, VM-density or qualifying E18 bound follows from this probe.

Retain this probe and compare the second option before the user's later promotion/selection decision. No decision is inferred, and no further native workaround is authorized by these findings.

## Canonical code, increments and receipts

The user's source-location correction supersedes the dispatch's `/tmp` default. Canonical sources now live at [spike-scratch/netns-density-295-multi-bridge](../../../../spike-scratch/netns-density-295-multi-bridge/README.md): `probe.py`, essential `guest.rs`, and `launch.py` transport wrapper. Four distinct increment directories contain exact executed source bytes and that epoch's raw readbacks/logs/captures. The index gives exact current run commands and historical provenance. No `.context/spikes` mirror was created.

Historical epochs actually ran from remote `/tmp/spike_netns_density_295_multi_bridge/`. Original sources/artifacts remain there and locally; relocation does not pretend the new path was executed. Current source changes only output/source/launcher paths after measurements, and no native rerun attests those location changes. Earlier launch-wrapper bytes were not separately attested for every epoch; actual command text remains in native logs. No C/eBPF code was used; Python kernel/CLI tooling and a Rust guest fixture do not violate the aya-rs-only eBPF rule.

| Increment | Evidence |
|---|---|
| [01](../../../../spike-scratch/netns-density-295-multi-bridge/increment-01-native-20261004T085711Z/README.md) | 4097 fanout; preserved probe-only select FD-range / duplicate-log-keyword failures |
| [02](../../../../spike-scratch/netns-density-295-multi-bridge/increment-02-native-20261004T090250Z/README.md) | Corrected poll-based fanout, exact queue traffic, cleanup |
| [03](../../../../spike-scratch/netns-density-295-multi-bridge/increment-03-native-20261004T090824Z/README.md) | Two actual guests, consoles/pcap/source/executable receipts, cleanup |
| [04](../../../../spike-scratch/netns-density-295-multi-bridge/increment-04-native-20261004T091503Z/README.md) | Neighbor-pressure failure and independent neighbor/table readbacks |
| [Additional receipts](../../../../spike-scratch/netns-density-295-multi-bridge/receipts/) | Source manifests, parsed counts, package/NIC attestation, timer reconciliation, final cleanup and scope checks |

Original `.context/spike-multi-bridge-native-01` through `04` logs/archives remain retained. Exact target address is redacted in shared receipts; unredacted original archive bytes remain only in task-owned private scratch, with hashes printed in launcher receipts. No volatile value was edited to manufacture equality. Historical errors/VM-count field/timer difference remain intact. Build images/binaries are preserved in original ignored archives, not copied into review source directories; their actual hashes remain in receipts.

Large raw readbacks/capture files are losslessly gzip-compressed in the review/commit tree; exact uncompressed originals remain under ignored `out/uncompressed-evidence/` and in original archives. `receipts/evidence-preservation.json` records hashes and a verified byte-for-byte round trip. This is storage compression, not a rewrite or normalization of captured observations.

Pre-existing production/support/DES dirty hashes are unchanged. `AGENTS.md` changed concurrently outside this probe; this agent did not write it. Workspace additions are the owned findings, canonical probe/increment evidence and task-owned `.context` receipts. No production/test/design/roadmap edit or commit was made. Under the project spike rule, source/captured evidence are retained until superseded; root owns any scoped commit after verification. No deletion or promotion choice is inferred.
