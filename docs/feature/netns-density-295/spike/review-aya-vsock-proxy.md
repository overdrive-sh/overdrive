# Independent review: Aya Rust SK_SKB ↔ stock vhost-vsock spike

## Review metadata

| Field | Value |
|---|---|
| Date | 2026-10-04 |
| Feature | netns-density-295 |
| Wave | NW-SPIKE PROBE |
| Reviewer | Codex, independent review |
| Iteration | 1 |
| Verdict | **APPROVED** |
| Owned review artifact | `docs/feature/netns-density-295/spike/review-aya-vsock-proxy.md` |
| Implementation boundary | Standalone throwaway sources and native captures under `spike-scratch/netns-density-295-aya-vsock-proxy/` |

Approval concerns the quality and stated limits of the spike evidence. TCP and
the measured nonempty UDP cases work. **The complete UDP contract does not
work:** the final linearized candidate loses zero-length datagrams in both
directions. This review approves retaining that negative result; it does not
approve a production architecture, DESIGN promotion, or execution of pending
DELIVER step 08-04.

The review used `AGENTS.md`, `.claude/rules/spike.md`, the local software-crafter
reviewer definition and its review/TDD skills. The explicit user boundary
selects qualified METAL/private KVM execution and overrides the spike rule's
default Lima location. The reviewer definition's DELIVER/TDD machinery and
legacy model default do not turn this standalone PROBE into a DELIVER step.

## Scope and verification performed

The reviewer read the findings, attempt index, final evidence manifest and
release readback; all final BPF implementations; the complete e, h and k host
controllers/endpoint drivers; the execution scripts; the relevant i→j fixture
diff; original native serial and build records; h's original resource samples
and physical inventories; and the bounded Linux reference functions explaining
redirect direction and SKB fragmentation/zero length.

Read-only local audits independently established:

- All **135** evidence hashes in `final-evidence-manifest.json` match the files.
- All **10** executed source manifests, b through k, match every recorded file,
  including their recorded Python cache files. Rust sources and execution
  scripts were not edited after execution.
- Available original private archives a through k agree byte for byte with
  their extracted native receipts, excluding the launcher's explicitly separate
  local launch/source records. This includes complete serial, command, resource,
  pin and execution-manifest captures for the accepted runs.
- The actual `/program.o` and `/probe` SHA256 values printed inside every
  executed VM, d/e/f/h/i/j/k, match each run's artifact pins.
- i, j and k have identical BPF source SHA256 and identical executed BPF ELF
  SHA256. The i→j correction changes the endpoint message collector, without
  changing the forwarding program or replacing e's original negative evidence.
- h's physical modules and link inventories match before/after exactly.
- The reported h resource interval, CPU seconds and maximum QEMU RSS recompute
  from the original samples and native `CLK_TCK=100` record.

No new native run, test lane, production edit, design/roadmap/DES edit, or
commit was necessary for this review. Pre-existing dirty production work was
preserved.

## Payload path and execution provenance

**PASS.** The implementation is Aya Rust eBPF: the standalone host workspace
pins `aya=0.13.1`, the standalone BPF workspace pins `aya-ebpf=0.1.1`, and BPF
uses `#![no_std]`, `#[stream_verdict]`, maps and `SkBuffContext`. Native build
records invoke the Rust nightly `bpfel-unknown-none` target and `bpf-linker`.
The preserved C files are Linux source references, not probe implementations
or a custom module build.

The actual forwarding program reads the source socket's kernel cookie, looks
up the opposite socket key, and calls `redirect_skb` to destination egress.
Both mapped directions use this same program. e uses SockMap; h uses SockHash;
k adds `pull_data(ctx.len())` before SockMap redirection. These are forwarding
instructions rather than observation-only counters.

Native e serial lines 470–475 show an accepted `BPF_PROG_TYPE_SK_SKB` load and
successful `BPF_SK_SKB_STREAM_VERDICT` attachment, with 33 verified instructions
and 192 JIT bytes. k lines 473–478 show its accepted load and attachment with
83 verified instructions. Its loaded instruction stream contains helper
`0x27` (`bpf_skb_pull_data`) and `0x34` (`bpf_sk_redirect_map`). The successful
load requests no textual verifier log; kernel acceptance and program info are
the verifier evidence. h serial line 454 reports the SockHash program tag,
41 verified instructions and 281 JIT bytes from the executed program info.

| Executed candidate | Native ELF SHA256 | Program tag |
|---|---|---|
| e: plain SockMap | `d3eeac69bd97a0544d4376e29ba7c55b0f22e148cd9325ee63d1ed0732a36942` | `9edc478960b90593` |
| h: SockHash TCP scale | `3936f2b67fa4108c21e42b78420f3648cb50d109795628bc02c56dc0189eda4b` | `c44b2b5ab882e87c` |
| i/j/k: linearized SockMap | `0443c5285c62ea593957768ede5f099ca13ee494a736d023d75193b4f8458242` | `b3ead938f76cfde6` |

The host-role VM boots the stock kernel copied by the execution script:
`7.0.0-29-generic`, Ubuntu `7.0.12`, with stock image SHA256
`b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`.
The QEMU command uses `-accel kvm`, private RAM and `-nodefaults`; private
module/link readbacks show only the loaded stock vhost/vsock dependency set and
`lo`. j/k also print their private VM BTF hash. Canonical metal lease acquisition
appears in the original launch logs. Probe loading occurs in private `/init`,
after stock module insertion, rather than in the physical host build process.

### Socket and endpoint roles

Source review verifies the controller's no-payload-relay statement independently
of its self-reported zero counter. Mapped proxy socket descriptors are used for
socket setup, cookie/map operations, lifecycle `shutdown`, and retirement.
Payload reads/writes target ordinary test application sockets or guest endpoint
virtqueue memory. `libc::write` in `Peer::kick` writes an eight-byte eventfd
notification, not a mapped socket's payload. k's direct native vsock `send` and
`recv` calls target its separate unregistered baseline endpoint.

Each `Peer::new` opens a distinct `/dev/vhost-vsock`, creates a new 256KiB mmap
and independent queue eventfds, registers that memory with its own vhost FD,
sets its CID and starts that device. The driver encodes/decodes guest endpoint
virtio traffic. It does not read one proxy leg and write another. k's endpoint
echo during truncation is traffic generated by the synthetic guest endpoint,
not a host forwarding fallback. SEQPACKET feature negotiation is a real native
ioctl; k's offered feature readbacks also show the SEQPACKET bit.

## TCP functional and scale evidence

**PASS at the declared boundary.** e source lines 72–82 and original serial
lines 483 and 731 prove byte equality for 262,144 bytes in each direction,
withholding further guest receive descriptors for 300ms, followed by successful
drain. The BPF hit counts change on both mapped legs. Guest shutdown control is
followed by controller `proxy.shutdown(Write)`; the ordinary application sees
EOF and can still send the 16-byte `after-half-close` payload to the guest.
This proves controlled half-close behavior, not automatic FIN propagation
between unrelated sockets.

e's ordinary TCP application connects to the controller's listener. h's
controller connects to the ordinary TCP application's listener. The synthetic
guest initiates the vsock session in both fixtures. Bidirectional application
payload does not imply every connection-initiation or ordinary guest socket
path has been tested.

**PASS for 16,384 simultaneous TCP forwarding owners.** h source lines 14–28,
61–64 and 71–85 establish the owner path: new device/memory/CID, accepted vsock
socket, new outgoing TCP proxy socket and separate accepted application socket,
kernel `SO_COOKIE` reads, two SockHash inserts, and opposite-key route inserts.
Owners remain in one vector until after the final audit. Every owner completes
an initial CID-tagged exchange and a different CID-tagged exchange while the
entire final population remains held. The guest receive path checks destination
CID and port from the actual virtio header.

h serial lines 471–486 record all sixteen final traffic checkpoints at a held
population of 16,384. Line 487 records:

| Native audit surface | Result |
|---|---|
| Held forwarding owners | 16,384 |
| Open `/dev/vhost-vsock` FDs | 16,384 |
| Distinct live mmap addresses | 16,384 |
| CID range | 300000–316383 |
| Occupied kernel SockHash keys | 32,768 |
| Minimum per-destination BPF hit count | 2 |
| Process FDs | 131,086 |
| Process threads / RSS | 16,385 / 400,992KiB |
| Audit elapsed time | 31.623357002s |

The kernel SockHash key iteration, FD readback, mmap uniqueness and all-owner
tagged traffic complement the source's independent socket/device construction.
h does not retain a textual dump of every FD/cookie tuple; the claims rest on
the executed native reads and assertions plus the retained owner path. There is
no shared-device multiplexing or earlier-owner expiration in that path. The
count represents transport owners, not running guest operating systems, and
small tag exchanges establish capacity rather than bulk throughput. UDP scale
was not measured.

f remains a failed attempt. Its SockMap array key iteration returned 65,536
positions and its 32,768-entry assertion failed; that result is not treated as
occupied-socket evidence. h uses genuine SockHash occupied-key iteration.

## UDP boundaries, negatives and zero-length failure

**PASS as qualified nonempty UDP evidence; complete UDP remains negative.**
The original e capture independently reports `[32768,26232]` forward packets
with flags `[1,1]` and ordinary reverse UDP receives
`[32768,16384,8192,1656]` for the 59,000-byte input. The preserved virtio UAPI
defines EOM as 1 and EOR as 2. Both forward packets have EOM, so byte aggregation
does not make them one logical datagram.

k's actual BPF linearization precedes redirection. Native serial line 493 shows
one forward 59,000-byte packet with EOM=1 and one ordinary 59,000-byte UDP
receive. Source checks the complete byte contents in both directions. This
change is in the loaded kernel program, not userspace payload reassembly
between proxy sockets. The endpoint collector consumes op-6 credit controls
and ends logical-message collection on EOM.

i's native failure is specifically `(op,flags,len)=(6,0,0)` where its fixture
expected `(5,1,1431)`. j fixes that endpoint protocol collector. i's raw failure
is retained, while i/j/k BPF sources and executed ELF remain identical. The
large-message flags/lengths are still reported independently; the correction
does not conceal e's fragmentation.

k serial lines 491–492 prove CID-tagged bidirectional association for actual
`127.0.0.2:55260` and `127.0.0.3:44225` endpoint bindings sharing server port
41265. The source holds both connected UDP proxy sockets concurrently with
`SO_REUSEADDR`; these are actual distinct IPv4/port peers. k line 496 and source
lines 458–470 prove native UDP endpoint truncation to eight bytes followed by
the correct `after-truncation` datagram, with no trailing ghost receive.

The three no-delivery cases have independent fresh owners and the same BPF
object. k lines 508, 523 and 538 record missing source route, missing destination
socket entry, and `BPF_F_INGRESS` to vsock. Source lines 475–513 establish the
actual mutations, nonempty send, bounded receive timeout and cleanup. Failure
counters progress `[1,0,0,0]` → `[1,1,0,0]` → `[1,2,0,0]`: route miss has its
own counter; destination miss and rejected direction share the redirect-drop
counter. Their separate fixture cases identify the failures. The evidence does
not establish three distinct semantic counter slots. There is no relay fallback.

### Exact final zero-length result

k source lines 515–541 sends an ordinary empty UDP datagram through registered
linearized maps, waits for its guest endpoint, then sends an empty guest
op-5/EOM message and calls ordinary UDP `recv` with a one-byte buffer. Native
line 564 records no forward wire packet, reverse EAGAIN (`errno=11`), and counts
`[4,4,1,1,0,1]` → `[4,4,1,1,0,2]`. UDP→vsock reaches the program; the guest's
empty EOM input produces no additional reverse hit or UDP datagram.

The independent unregistered SEQPACKET endpoint calls real host `send(0)` and
receives a guest op-5/EOM empty message. k lines 568–569 record host send return
0 without a wire message, and a zero-length native host receive. Source lines
543–565 keep that peer connected and started, issuing its reset only after the
receive; no FIN/shutdown/EOF setup substitutes for the empty message. This
baseline also shows that the endpoint driver can represent zero-length EOM.
k line 570 records `AF_VSOCK/SOCK_DGRAM` creation failing with `ENODEV=19` on
this loaded stock setup.

The findings correctly retain the complete UDP failure and scope it to the
tested candidate/kernel. The pinned Linux 7.2 reference's `__skb_send_sock`
fragment loops and zero-length early exit explain a possible mechanism; those
reference files are not represented as the executed Ubuntu 7.0.12 source build
or a proof against every possible eBPF mechanism/backend.

### Capability scope: could another eBPF path preserve empty UDP?

The spike does not prove that eBPF generally cannot preserve an empty UDP
datagram. It proves that this SK_SKB destination-egress candidate cannot do so
on the measured stock setup. The 7.2 reference's `__skb_send_sock` at
`net/core/skbuff.c:3311` performs no send when its remaining length is zero;
the actual stock SEQPACKET host `send(0)` baseline also produces no virtio
message. Linearization changes SKB layout and leaves those zero-length cases
unchanged.

The same 7.2 source does expose `BPF_FUNC_skb_change_tail` to SK_SKB
(`net/core/filter.c:8916`). That is source-level helper admissibility, not a
native successful zero-datagram variant on the tested 7.0.12 kernel. Growing
an empty payload into a marker would transport nonempty bytes; trimming that
marker back to zero before the existing UDP egress path would still encounter
the zero-send problem. Framing alone therefore does not demonstrate a complete
fix for the final ordinary UDP datagram.

Redirecting to a UDP socket's ingress path or using packet-layer TCX/XDP are
unproved alternatives. UDP socket ingress would need to prove the correct
destination socket ownership and empty-message receive semantics; it cannot
be assumed equivalent to sending from a proxy socket to its ordinary peer.
Packet-layer UDP retains IP/UDP headers even with empty application payload,
but no tested path here connects that representation to the stock vsock
backend. None of these alternatives is promoted or implemented by this review.

## Resource measurements and retirement

**PASS for the stated observed surfaces.** h uses 16GiB private VM RAM, four
online/64 possible CPUs, stock PID/thread sysctls and a process-local FD limit
of 262,144. Host admission reserves 8GiB. Original QEMU sample endpoints are
1.000718209s and 284.153820704s, giving a **283.153102495s interval**. With the
recorded 100 ticks/s, CPU delta is 211.38s and mean consumption is
0.746521928 cores; peak QEMU RSS is 4,459,736KiB. The findings correctly include
admission, traffic checks and retirement, without claiming throughput,
zero-copy behavior or that process RSS measures all kernel allocations.

h source lines 86–88 removes both cookie routes and SockHash entries for each
owner, issues guest reset, drops the mapped and application sockets, stops
vhost and unmaps its memory in `Peer::drop`, then drops listeners, map-FD clone
and BPF ownership. Original serial line 488 records FDs **4→4**, retirement
**250.815174153s**, and stock `vhost_vsock` usage **0 while the private VM is
still running**. Subsequent serial records precede poweroff. This is a module
reference readback, not enumerated live-socket count or a per-allocation heap,
queue/task census or fast production teardown promise.

j/k complete with FDs 4→4 and successful native BPF detach. Earlier d/e final
FD counts are 8/7 respectively; the findings do not claim the final 4→4 result
for those exploratory fixtures. f's failed audit is likewise not given h's
explicit retirement claim.

The QEMU completion records show its owned PID absent. Physical before/after h
module bytes and link lists match exactly, and final release readback records
no owned QEMU PID or canonical lease owner. No physical module/network flush or
host sysctl mutation is present in the reviewed execution path. Ignored private
archives/build output are evidence/build storage, not claimed running owners.

## Contract Shape Compliance

**PASS for this spike boundary.** This work adds no production tests, Rust
properties, public production API, acceptance test lane or verification runner.
DELIVER DES phase order, mutation score, unit-test budget, per-test declarations
and DISCUSS outcome anchors are not applicable to these throwaway endpoint
probes. The applicable evidence contract is native execution through real
socket/vhost/BPF boundaries, retained positive and negative receipts, source
pinning, explicit endpoint/controller ownership, and honest claim limits.

No typed production error representation is invented by the spike. Its
assertions, syscall errno observations and counters report the experiments;
they are not proposed production error variants or port contracts.

## Findings, dispositions and final verdict

| Iteration | Finding | Disposition | Verdict |
|---|---|---|---|
| 1 | No blocking harness, provenance, payload-boundary or claim-scope defect found. | No remediation required. | **APPROVED** |

The retained failed attempts and final UDP zero failure are experimental
outcomes, not reasons to introduce a kernel module, userspace framing relay,
parser/framework replacement or production architecture during this review.
Guest transparency, Cloud Hypervisor integration, same-socket kTLS/mTLS, SSH,
IPv6, controller restart survival and UDP scale remain explicitly unproved.

**APPROVED** for a bounded spike commit containing the preserved Aya source,
receipts and qualified findings. The complete TCP/UDP production requirement
is unsatisfied because zero-length UDP remains lost. No DESIGN promotion or
production-delivery approval follows from this verdict.
