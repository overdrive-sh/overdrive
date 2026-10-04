# Aya Rust SK_SKB TCP/UDP ↔ stock vhost-vsock probe

Date: 2026-10-04. Wave: NW-SPIKE PROBE. No production change, kernel patch,
custom kernel module, TAP, userspace application payload relay or promotion.

## Verdict

**WORKS — TCP:** Aya Rust SK_SKB egress redirection carries real bytes in both
application directions between an ordinary TCP connection and stock vhost-vsock
shared virtqueues. A functional owner passed 262,144 bytes each direction,
deliberate guest receive backpressure, and reverse data after guest FIN. A
separate scale run held **16,384 actual independent TCP forwarding owners**:
16,384 started vhost devices/CIDs/memory contexts and 32,768 occupied SockHash
socket entries. Every owner passed a fresh CID-tagged bidirectional exchange at
that final cardinality.

**WORKS — nonempty UDP at the measured boundary:** after BPF `pull_data(len)`
linearization, 59,000 bytes passed as one logical SEQPACKET message and one
ordinary UDP datagram. Two actual IPv4 peers with different source addresses and
ports shared one UDP server port and retained their CID/address/port association.
Native endpoint receive truncation preserved the following datagram.

**DOESN'T-WORK — the complete requested UDP contract:** zero-length datagrams
were lost in both directions through the exact linearized BPF object. The
unregistered ordinary SEQPACKET endpoint baseline separately showed host
`send(0)` returning zero without emitting a virtio message, while a synthetic
peer's zero-length EOM message was observable as a zero-length host receive.
This is a precise negative for this SK_SKB/vhost candidate on the tested kernel;
it does not establish that every eBPF program type, framing scheme or vsock
backend has the same restriction.

UDP forwarding at 16,384 owners was not measured. Ordinary guest socket
transparency, guest kTLS/mTLS, Cloud Hypervisor integration, SSH reachability,
IPv6, controller restart survival and throughput remain unproved.

## Placement, backend and owner boundaries

All new native runs used the canonical exclusive lease on the qualified metal
fixture. BPF executes **inside a private QEMU/KVM host-role VM**, with stock
Ubuntu `7.0.0-29-generic` (`7.0.12`) and its matching stock vhost modules. The
physical host kernel runs QEMU and builds the probe; no experimental BPF or
custom module is loaded into it. No Lima run was made for this spike.

The private VM has only `lo`. Its addresses `127.0.0.1`, `127.0.0.2` and
`127.0.0.3` are real native IPv4 endpoint bindings inside that VM. They establish
socket forwarding and address association, rather than Internet routing or
unmodified guest networking.

Each synthetic guest endpoint opens its own `/dev/vhost-vsock`, registers a
different CID, installs its own 256KiB memory context and split virtqueues,
provides independent eventfds and sets `VHOST_VSOCK_SET_RUNNING`. This is a real
stock shared-memory backend, not AF_VSOCK local loopback substituted for it.
The Rust endpoint driver produces and consumes guest-side traffic only.

The Aya controller creates/listens/accepts/connects the proxy-leg sockets and
installs source-cookie → destination-key routes. A single SK_SKB program
redirects to the destination socket's **egress** path in both application
directions. It never uses SK_MSG. The controller never reads or writes payload
on either mapped proxy-leg socket. Ordinary TCP/UDP test application sockets
produce and consume their own bytes. The unregistered native vsock baseline is
also an endpoint actor and is never part of a forwarding pair.

The functional TCP run uses an incoming ordinary TCP leg: a test application
connects to the controller's TCP listener. The scale run uses an outgoing
ordinary TCP leg: the controller connects to the test application's listener.
In both cases the synthetic guest initiates the independently paired vsock
session. This does not prove an ordinary guest `connect` or `accept` path.

The functional half-close uses a guest virtio shutdown control message and a
controller TCP `shutdown(Write)` to propagate lifecycle control. Payload still
flows only through BPF. Automatic FIN propagation between unrelated sockets is
not implied by that result.

Cloud Hypervisor's Unix STREAM muxer is not this registered stock AF_VSOCK
backend. No claim is made that an AF_VSOCK connect reaches a Cloud Hypervisor
Unix muxer automatically.

## Predicted versus actual native behavior

| Probe | Predicted output | Actual result |
|---|---|---|
| d: SK_SKB STREAM pair | Both payload legs arrive, BPF counters increment | 19-byte TCP ↔ vsock exchanges passed; 33 verified instructions, 192 JIT bytes |
| d: SK_SKB SEQPACKET/UDP pair | One nonempty datagram each direction | 19 bytes each direction passed, EOM=1, counters incremented |
| e: large TCP/backpressure/FIN | Byte exact, no loss, reverse after guest FIN | 262,144 bytes each direction passed; reverse tail passed |
| e: plain redirect UDP 0 | One zero-length message/datagram each direction | No forward message or reverse datagram |
| e: plain redirect UDP 59,000 | One logical message/datagram | Forward [32768,26232], both EOM=1; reverse UDP receives [32768,16384,8192,1656] |
| j/k: BPF-linearized UDP 59,000 | One logical message/datagram | One 59,000-byte vsock packet, EOM=1, and one 59,000-byte UDP receive |
| j/k: shared server port | CID tags remain associated with actual source IP/port | Distinct 127.0.0.2/127.0.0.3 peers passed on one server port |
| k: linearized UDP 0 | A zero-length message/datagram | Still no forward message; reverse UDP receive timed out with EAGAIN |
| j/k: missing source/destination or vsock ingress flag | No fallback delivery, negative counter increments | All three paths failed closed |
| h: 16,384 TCP owners | Actual held devices, sockets, mappings, traffic and occupied keys reconcile | All 16,384 owners reconciled; 32,768 occupied SockHash keys |

Native protocol flags are decoded from the actual virtio headers:
`VIRTIO_VSOCK_SEQ_EOM=1`, `SEQ_EOR=2`. The two plain-redirect forward packets in e
each had EOM=1, so they were **two logical messages**, not fragments of one.
The reverse lengths came from separate ordinary UDP `recv` calls. j/k assemble
logical messages by EOM and consume credit-update controls separately. The new
linearization attempts preserve e's original failure evidence.

Selected original output:

```json
{"bytes_each_direction":262144,"cid":295101,"counters":[86,153],"event":"tcp_large_halfclose_backpressure_pass","fin_propagation":"userspace controller shutdown control, zero payload reads/writes","memory":"0x7e3f5d74e000","real_vhost_fd":11,"reverse_after_guest_fin":true}
{"actual_adapter":"127.0.0.1:48544","actual_peer":"127.0.0.3:41505","cid":295113,"counters":[86,153,3,4],"event":"udp_boundary_probe","forward_byte_exact":true,"forward_flags":[1,1],"forward_one_datagram":false,"forward_packet_lengths":[32768,26232],"input_len":59000,"reverse_byte_exact":true,"reverse_one_datagram":false,"reverse_udp_lengths":[32768,16384,8192,1656]}
{"event":"udp_linearized_large_pass","flags_eom_last_only":true,"forward_flags":[1],"forward_vsock_packet_lengths":[59000],"input_len":59000,"reverse_one_datagram":true,"reverse_udp_len":59000}
{"counter_after":[4,4,1,1,0,2],"counter_before":[4,4,1,1,0,1],"event":"linearized_bpf_zero_datagram_probe","failure_counters":[1,2,0,0],"forward_wire":null,"reverse_errno":11,"reverse_udp_receive":null}
```

The linearized zero test's destination counter increments for UDP→vsock, yet
no virtio payload message arrives. The synthetic zero-EOM peer message produces
no additional reverse hit and no UDP datagram. The independent unregistered
vsock endpoint baseline distinguishes those results from a decoder that simply
cannot observe zero-length messages. On this stock setup, creating
`AF_VSOCK/SOCK_DGRAM` returns `ENODEV` (19); that observation is scoped to the
loaded stock transports, not a global AF_VSOCK verdict.

## Scale, resources and cleanup

h's final audit occurred after all owners were admitted and **all 16,384 were
rechecked while still held together**. It independently reconciled open vhost
FDs, unique memory mapping addresses, CID-tagged wire traffic and kernel
SockHash occupied-key iteration. Per-destination BPF counters were at least two
in each direction for every owner. No idle expiration closes earlier owners.
Each counted owner has one accepted vsock socket and one paired TCP proxy-leg
socket; a separate ordinary TCP endpoint socket generates/receives the tags.
These are independent transport/socket owners, not aliases or streams behind
one vhost device, and not 16,384 running guest OS instances.

```json
{"all_owners_rechecked_with_cid_tags":true,"cid_max":316383,"cid_min":300000,"different_guest_memory_mappings":16384,"each_direction_bpf_hit_min":2,"elapsed_s":31.623357002,"event":"final_live_audit","fds":131086,"kernel_sockhash_occupied_keys":32768,"live_independent_transport_owners":16384,"started_vhost_devices":16384}
```

The private scale VM used 16GiB RAM, four online/64 possible CPUs, stock
PID/thread sysctls and a raised **probe process** FD limit of 262,144. Physical
admission reserved 8GiB for the host. The final process had 131,086 FDs,
16,385 tasks and approximately 400,992KiB RSS; most tasks are stock vhost
workers, rather than userspace payload relay workers. The 256KiB mappings are
independent virtual memory contexts; the traffic probe does not make every page
resident. Small per-owner CID tags prove capacity, not bulk throughput at scale.

Outer QEMU raw CPU/status/FD samples are complete in h. Across the observed
283.153s interval, QEMU consumed 211.38 CPU seconds (0.7465 mean CPU cores); peak
QEMU RSS was 4,459,736KiB. Those measurements include admission, checks and slow
retirement and are not a throughput or zero-copy result.

h explicitly removes its routes/socket entries, resets and closes every owned
session, stops every vhost device and unmaps every endpoint memory context,
then drops listeners, map FDs, BPF attachment and program ownership. Its control
process FDs return exactly **4 → 4**. Retirement took **250.815s**.
`vhost_vsock` module usage read back **0 inside the still-running private VM**.
This is the observed module-reference surface, not an allocation-level heap
leak proof or a promise of fast production teardown. The VM then shut down and
its QEMU PID disappeared. Small j/k runs also return FDs 4 → 4 and record native
`BPF_PROG_DETACH=0`.

Physical host modules and network link inventory match before/after h exactly.
Final post-release readback shows no owned QEMU PID and no canonical lease owner
file. Foreign host links/modules were preserved; no global module/network flush
or host sysctl change was performed. Builds and private raw archives remain
under ignored `out/` for evidence, rather than being treated as runtime residue.

## Toolchain and source pinning

Aya userspace is 0.13.1; kernel BPF is `aya-ebpf` 0.1.1, `no_std` Rust only,
compiled for `bpfel-unknown-none` through the installed nightly toolchain and
`bpf-linker`. Native build logs and both executed Cargo locks are retained.
`rustc` host is 1.95.0; nightly Cargo reports 1.101.0-nightly
(`f3865b2a4`, 2026-09-29); the installed bpf-linker reports 0.0.0.

Native kernel config/BTF pins, load/attach syscalls, program tags, verified
instruction counts and source/ELF hashes are retained. Aya's successful first
load requests no verifier text buffer even when VerifierLogLevel is selected;
the raw `BPF_PROG_LOAD` acceptance and kernel `verified_insns` are the successful
verifier evidence. No successful textual verifier log is invented.

| Executed object | Actual ELF SHA256 | Program tag / verified instructions |
|---|---|---|
| e plain SockMap | `d3eeac69bd97a0544d4376e29ba7c55b0f22e148cd9325ee63d1ed0732a36942` | `9edc478960b90593` / 33 |
| h SockHash scale | `3936f2b67fa4108c21e42b78420f3648cb50d109795628bc02c56dc0189eda4b` | `c44b2b5ab882e87c` / 41 |
| k linearized SockMap | `0443c5285c62ea593957768ede5f099ca13ee494a736d023d75193b4f8458242` | `b3ead938f76cfde6` / 83 |

These are hashes of the executed BPF ELF, not the hashing tool. Stock kernel
image SHA256 is
`b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`.
j/k print the private VM's BTF hash; physical BTF/config pins are also retained.

The user's Linux reference is retained with exact source hashes at HEAD
`502d45774af09f1c681c754c4b7cdfb5d7f72fd9` (7.2). Its sock-map, vsock-BPF,
vhost and SKB code explain the candidate and restrictions, but those files were
not the executed kernel build. Native claims use the stock 7.0.12 kernel.
In that reference, SK_SKB permits vsock egress and rejects vsock ingress;
SK_MSG rejects vsock destinations. `skb_send_sock` sends separate SKB head/frags
and performs no send for zero length. BPF linearization removes the nonempty
fragmentation observed natively. Same-socket kTLS and psock coexistence is not
proved by opaque payload forwarding on these non-TLS host sockets.

Aya API shape was checked against the local 0.13.1/0.1.1 crate sources and
[official SockHash documentation](https://docs.rs/aya/0.13.1/aya/maps/sock/struct.SockHash.html)
and [SK_SKB program documentation](https://docs.rs/aya/0.13.1/aya/programs/sk_skb/struct.SkSkb.html).

## Preserved attempts and artifact index

| Increment | Outcome |
|---|---|
| a | Canonical bootstrap rsync raced unrelated live code-review SQLite files; no native probe executed. |
| b | Aya socket-map API borrowing compile error; no BPF/kernel execution. |
| c | Nightly BPF target lacked AtomicU64 fetch_add; changed subsequent attempt to per-CPU counters. |
| d | First actual SK_SKB TCP and nonempty UDP bidirectional proof. |
| e | Large TCP/FIN/backpressure proof and original UDP zero/fragmentation negatives. |
| f | 16,384 actual devices and final tagged traffic completed; audit incorrectly treated SockMap array indices as occupied entries and stopped. |
| g | JSON macro fixture compile error before native execution. |
| h | Corrected independent SockHash scale/audit and explicit retirement, complete native pass. |
| i | Linearized large UDP/shared server peers passed; fixture then asserted on a legitimate credit-update control packet. |
| j | Logical message assembly, linearized UDP, truncation, negatives and native zero/DGRAM baseline completed. |
| k | Same BPF object; repeats mapped zero datagrams, all positive/negative checks and cleanup, complete native pass. |

Executed attempt Rust/scripts remain byte-identical to their execution source
manifests. Generated executed lock files are restored alongside their sources.
No original raw native receipt is replaced with a derived file. Complete remote
`evidence/` was retrieved after each attempt **before the next source sync**;
private archive hashes are in the launcher captures. Local launcher logs are
preserved separately. Unlike the previous spike's i retrieval gap, this spike
retains the complete native diagnostic JSON and outer CPU samples.

- [Complete attempt index](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/attempt-index.json)
- [Plain TCP/UDP native serial](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/increment-e/evidence/native-serial.log)
- [Final scale native serial](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/increment-h/evidence/native-serial.log)
- [Final scale original CPU samples](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/increment-h/evidence/qemu-resources.json)
- [Final scale resource derivation](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/increment-h/evidence/resource-summary.derived.json)
- [Final linearized UDP native serial](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/increment-k/evidence/native-serial.log)
- [Final UDP executed pins](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/increment-k/evidence/executed-artifact-pins.json)
- [Linux source-reference pins](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/linux-reference-manifest.json)
- [Final host/lease cleanup readback](../../../../spike-scratch/netns-density-295-aya-vsock-proxy/post-release-readback.json)

Reproduction uses a **new increment/output ID** and the preserved launcher:

```sh
python3 spike-scratch/netns-density-295-aya-vsock-proxy/launch.py increment-h
python3 spike-scratch/netns-density-295-aya-vsock-proxy/launch.py increment-k
```

Existing output IDs refuse overwrite. Sources, scripts and receipts are kept
for the scoped spike commit; builds/images stay ignored. No production,
ADR/roadmap/DES edit or commit was made by the probe agent.

## Gate recommendation

Retain the native TCP and nonempty UDP evidence. **Do not promote this prototype
as satisfying the complete TCP/UDP contract:** zero-length UDP remains a native
failure and ordinary guest/backend integration remains unvalidated. No new
custom kernel module or userspace payload forwarding fallback was introduced.
