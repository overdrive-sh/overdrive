# Aya Rust zero-length UDP over stock vhost-vsock

## Verdict

**WORKS in both directions at the measured native boundary.** A Rust Aya
SK_SKB program encodes every owned UDP payload with a private eight-byte prefix.
A Rust Aya TCX egress program decodes the prefix at the IPv4/UDP packet boundary.
On stock Ubuntu `7.0.0-29-generic` (`7.0.12`), this carried actual zero-length
UDP messages through real vhost-vsock shared virtqueues and returned genuine
empty datagrams to ordinary UDP application sockets.

Two independently started vhost-vsock devices/CIDs and memory contexts each
completed `0, 0, 1, 0, 1431, 59000, 0` payload bytes in **each direction**.
Eight real guest-side empty message receipts and eight ordinary host UDP empty
receives reconcile independently in the captured events. Each empty host
receive had `POLLIN`, the actual source IP/port, and a **65,536-byte receive
buffer**. Actual post-decoding packet bytes show IPv4 total length **28**, UDP
length **8**, skb length **42** including the loopback L2 header, and a valid
IPv4 checksum. Following nonempty datagrams and final no-readiness checks prove
that zero was a message event rather than EOF, absence of data, or a zero-sized
receive buffer masking nonempty data.

This is a focused mechanism probe. It does not validate ordinary guest UDP
socket transparency, a guest OS datapath, SSH, guest kTLS/mTLS, production
integration, IPv6, the full UDP length range, UDP scale, or performance.

## Actual forwarding and ownership

All BPF execution was inside an owned private QEMU/KVM **host-role VM** on the
qualified metal fixture. The VM booted the unchanged stock kernel and matching
stock vhost modules. The canonical exclusive metal lease covered source sync,
build and execution. No Lima execution, kernel patch, custom module, TAP,
physical NIC modification, new network device, global sysctl tuning, or
userspace proxy payload relay was used. The VM's only network link was `lo`.

The forward application path was:

1. Ordinary UDP app `127.0.0.2:36271` or `127.0.0.3:52321` sends its actual
   payload to the real shared server port `127.0.0.1:49468`.
2. TCX checks the owned tuple and leaves this native UDP packet unchanged.
3. SK_SKB sees the actual UDP socket payload, including zero length. Its stock
   `bpf_skb_adjust_room(+8, 0, 0)` helper prepends `ZUD1` plus the big-endian
   logical payload length. It writes that prefix in BPF and linearizes the
   framed skb with `bpf_skb_pull_data(len)`.
4. SockHash egress redirection sends the now nonempty SEQPACKET message to the
   accepted stock AF_VSOCK socket and the real started vhost-vsock queue.
5. The independent synthetic guest endpoint consumes the completed RX
   descriptor, verifies the real virtio `OP_RW`/EOM frame and decodes its own
   logical message from the received bytes. It never invents an empty message
   without receiving a frame.

In reverse, the synthetic guest endpoint sends its own encoded logical message
through its real TX virtqueue. SK_SKB validates the private prefix and declared
length, then redirects the framed skb to the connected ordinary UDP proxy's
egress. TCX on `lo` validates the registered tuple and prefix, saves the actual
UDP header, invokes `bpf_skb_adjust_room(-8, BPF_ADJ_ROOM_NET, 0)`, restores the
UDP header, updates the IP/UDP lengths and IP checksum, and reads back the
actual transformed packet header. The resulting network packet retains its
IPv4 and UDP headers even when its application payload is empty. The ordinary
UDP application then receives that real datagram.

The Aya controller creates/listens/accepts/connects proxy sockets and installs
cookie routes and tuple maps. It performs **zero payload reads or writes on
mapped proxy-leg sockets**. Ordinary application socket send/receive calls and
guest virtqueue memory operations are endpoint actors. The guest actor's
private codec is not an ordinary guest kernel UDP path. No zero-copy claim is
made: the stock helpers may reallocate, unclone or linearize skbs, and endpoint
socket/virtqueue operations copy bytes.

The prefix is applied to **every** controlled message. An ordinary application
payload exactly equal to `ZUD1\0\0\0\0` passes unchanged in both directions,
with a distinct outer logical length of eight. The fixed owned-address/server
selector and explicit tuple map constrain decoding. A separate ordinary UDP
empty datagram from `127.0.0.4` to an unrelated server passed unchanged.

The IPv4 probe explicitly sets per-socket `SO_NO_CHECK` on the ordinary app and
proxy transmit sockets; the observed UDP checksum is zero, which is a legal
IPv4 UDP representation. General UDP checksum/offload behavior is unvalidated.
Rust `recv_from` does not expose `msg_flags`; no `MSG_TRUNC` flag receipt is
claimed. The native UDP length and nonzero receive capacity directly establish
that the zero cases contain no hidden payload.

## Predicted versus actual evidence

| Case | Expected | Actual native result |
|---|---|---|
| Ordinary app sends zero | One framed guest message | `OP_RW`, wire length 8, EOM=1, prefix `ZUD1`/logical length 0 |
| Guest sends framed zero | One real empty ordinary UDP datagram | `POLLIN=1`, `recv_from=0`, actual `127.0.0.1:49468`, IP length 28, UDP length 8 |
| Repeated empties and following data | Exact count/order; socket stays usable | Each peer completed `0,0,1,0,1431,59000,0` both ways |
| Two actual peer addresses, one server port | CID/tuple association retained | CIDs 295300/295301 retain `127.0.0.2:36271` / `127.0.0.3:52321` |
| 59,000 application bytes | One logical message/datagram | One 59,008-byte framed vhost message and one 59,000-byte ordinary UDP receive |
| Prefix-shaped application payload | Ordinary bytes preserved | Eight original bytes retained both ways, framed wire length 16 |
| Invalid magic / length mismatch | No ordinary UDP delivery | Both dropped by SK_SKB; invalid-frame counter becomes 2 |
| Removed guest-to-host tuple | No UDP fallback | TCX drops; tuple-miss counter becomes 1 |
| Removed host-to-guest cookie route | No vhost fallback | No `OP_RW` message; source-route miss counter becomes 1 |
| Foreign tuple | No transformation | Separate ordinary UDP empty receive retains real foreign address |

Selected exact native output (the complete events retain all packet/virtqueue
headers, descriptors, used-ring indices, addresses and counters):

```json
{"event":"zero_message_counts","expected_each":8,"ordinary_app_empty_receives":8,"ordinary_app_empty_sends":8,"real_guest_empty_decodes":8}
{"controller_payload_reads_writes":0,"event":"complete","final_fds":4,"initial_fds":4}
```

The first zero case's actual packet/readiness receipt is retained in
[complete native serial](../../../../spike-scratch/netns-density-295-aya-zero-udp/increment-f/evidence/native-serial.log)
and [parsed events](../../../../spike-scratch/netns-density-295-aya-zero-udp/increment-f/evidence/events.derived.json).
The independent [receipt reconciliation](../../../../spike-scratch/netns-density-295-aya-zero-udp/increment-f/evidence/receipt-summary.derived.json)
counts actual `OP_RW`/eight-byte/logical-zero receipts separately from the
ordinary UDP roundtrip receipts. Guest TX/RX receipts capture the **actual**
44-byte virtio header, descriptor chains, used-ring heads and avail/used
progression; credit-update `OP=6` controls are distinguished from messages.
The post-TCX header witness comes from a constant 42-byte
`bpf_skb_load_bytes` read **after** transform, rather than expected metadata.

## Native program and source pins

| Object | Native program ID | Tag | Verified instructions | JIT bytes |
|---|---:|---|---:|---:|
| SK_SKB `route` | 8 | `36f905163fccb7de` | 255 | 1397 |
| TCX `packet` | 9 | `a1d3be6181987314` | 481 | 1967 |

TCX queried `lo` egress with effective program IDs `[9]`, revision 2. The
SockHash has 16-entry capacity and four actual paired sockets for this probe;
no UDP scale claim follows. Map IDs are in each `bpf_loaded` receipt and raw
`BPF_MAP_CREATE` / `BPF_PROG_LOAD` / `BPF_LINK_CREATE` calls are retained by
native strace. Successful loads requested no textual verifier buffer on Aya's
first load; accepted native load syscalls and verified instruction counts are
the successful verifier evidence. The failed d load retains full verifier text.

Executed ELF SHA256: `43bbbd05fea91ad9c0898024171939932d45c55dc239b339b075e3cbc1d77f45`.
Executed controller binary SHA256: `4fe83b0ef6144b1466006d7a20d75c4302a437a9a6e30046afe16ab81aad20ea`.
Stock kernel SHA256: `b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`.
The complete kernel/module/initramfs/binary hashes, actual stock config/BTF and
both executed Cargo locks are in
[executed pins](../../../../spike-scratch/netns-density-295-aya-zero-udp/increment-f/evidence/executed-artifact-pins.json).
Aya userspace is 0.13.1; aya-ebpf is 0.1.1. Builds use Rust
`bpfel-unknown-none`, nightly Cargo 1.101.0-nightly, installed bpf-linker and
`no_std` Rust; there is no C eBPF, clang or libbpf code.

The retained Linux reference is HEAD `502d45774af09f1c681c754c4b7cdfb5d7f72fd9`
(7.2), with exact hashes in
[reference manifest](../../../../spike-scratch/netns-density-295-aya-zero-udp/linux-reference-manifest.json).
It is primary-source reference material, not the executed 7.0.12 kernel build.
It distinguishes SK_SKB's prefix-growing helper implementation from TC's
network header helper and explains why `skb_send_sock` skips a truly zero-length
application skb. The measured replacement always sends at least eight bytes
through that sockmap/vsock leg and produces empty UDP at the packet boundary.
Aya's documented [SchedClassifier API](https://docs.rs/aya/0.13.1/aya/programs/tc/struct.SchedClassifier.html)
supports the native TCX attachment used here.

## Preserved attempts

| Increment | Outcome |
|---|---|
| a | Rust fixture byte-literal compile error; no BPF/native forwarding executed. |
| b | Native TC tail framing WORKS for repeated empty, 1-byte and 1431-byte datagrams both ways on the first peer; 59,000-byte forwarding times out. The 7.2 reference TC tail helper has `SKB_MAX_ALLOC` constraints consistent with the timeout, but b did not record the helper errno, so the precise native cause is not claimed. |
| c | Canonical lease acknowledgement failed before source sync; no remote probe source or native execution. |
| d | SK_SKB prefix object verified/loaded. TC witness capture rejected by native verifier: `R4 invalid zero-sized read: u64=[0,47]`; no forwarding run. |
| e | Constant-size witness fix prepared; canonical lease acknowledgement failed before source sync, so it was not executed. |
| f | Constant 42-byte actual header capture; acquisition timeout 15s; full native prefix/shrink proof, negative cases and explicit cleanup **WORKS**, exit 0. |

Every executed increment retains its sources byte-identical to its execution
source manifest. Complete remote evidence for a/b/d/f was retrieved **before**
the next source sync. c/e retain local captures of the lease-acquisition failure
and launcher return code; no source sync or remote probe execution occurred.
The launcher archives its exact code per attempt, including the original
one-second acquisition settings and f's 15-second setting. Retrieval runs after
the retained `native-launch.log` closes: its success/hash and failure messages
were printed to the console and are **not retained in those launch logs**.
No historical retrieval-error receipt is claimed for c/e. The ignored original
private archives for a/b/d/f remain under `out/`; their current SHA256 readbacks
and byte-for-byte comparisons of original b/d/f serial members with the retained
native serial files are recorded in
[archive readback](../../../../spike-scratch/netns-density-295-aya-zero-udp/archive-readback.derived.json).
That file is a post-execution derived readback, not a historical launcher
receipt. The [independent review](review-aya-zero-udp.md) also records archive
hashes from its own readback. Failed attempts are preserved.

The final command was:

```sh
python3 spike-scratch/netns-density-295-aya-zero-udp/launch.py increment-f
```

For another native execution, copy a preserved source increment to a **new**
increment/output ID first; the runner refuses existing output/evidence IDs.
No new production, ADR, roadmap, DES edit, design promotion or commit was made
by the probe agent.

## Cleanup and resource boundary

The runner removes all four SockHash entries and reads back zero occupied keys,
resets sessions, stops both vhost devices, closes their eventfds/sockets and
unmaps the two independent 256KiB memory contexts. It explicitly detaches TCX,
queries no egress program on `lo`, detaches SK_SKB, and drops remaining map,
program and listener ownership. Process FDs return **4 → 4**. The still-running
private VM reads `vhost_vsock` module usage **0**, then powers down. This is a
module-reference/FD/attachment observation, not a kernel heap leak proof.

QEMU exits 0 after 6.530s
including boot, probe and shutdown; its PID is absent. Raw CPU/RSS/FD samples
and a labeled resource derivation are retained; this is no throughput result.
Post-release readback confirms no owned QEMU PID, no canonical owner file, and
physical host module/link inventories exactly equal to the pre-run readback.
Foreign physical host BPF objects were not enumerated before/after; the code
and native launcher boundaries place every experimental BPF load only inside
the private VM. No experimental BPF load was run in the physical kernel.
Builds, images and private raw archives remain in ignored `out/` directories.

## Limits and gate recommendation

Retain this proof that **zero-length UDP can be accomplished with Aya Rust
BPF and stock vhost-vsock**, using private framing plus packet-boundary
unframing. No custom module or userspace proxy payload relay was needed.

The eight-byte prefix adds transport overhead. The full UDP maximum length is
unvalidated, and this probe does not approve reducing any accepted payload
maximum. Ordinary guest transparency, production guest/header codec ownership,
checksum/offload variants, Internet routing, IPv6, UDP scale and throughput
require their own approved work. The earlier 16,384-owner TCP result is not
UDP scale evidence. Keep this as a mechanism result for DESIGN; do not promote
the scratch protocol into production or change accepted contracts from it.
