# Independent review: OVS stock-kernel 16,384-TAP comparison

## Review metadata and verdict

- Feature: `netns-density-295`.
- Review date: 2026-10-04.
- Iteration: 1.
- Reviewer role: independent nWave researcher reviewer; native probe and evidence review.
- Reviewed finding: [ovs-kernel-findings.md](ovs-kernel-findings.md).
- Reviewed probe: [retained standalone increments](../../../../spike-scratch/netns-density-295-ovs-kernel/).
- **Verdict: APPROVED for the bounded OVS/TAP comparison.**
- Blocking findings: none.
- Remediation required: none.

The retained evidence proves that the stock, in-tree `openvswitch` module on
the measured Linux `7.0.0-29-generic` host held 16,384 distinct TAP netdevices,
16,384 NETDEV vports and 16,384 live owner FDs simultaneously. All those
attachments participated in the gateway exchange and in byte-identical TCP
frame forwarding through kernel OUTPUT actions. This approval covers that
capacity and frame-forwarding result.

It grants no production architecture approval, roadmap validation, DESIGN
promotion or DELIVER advancement. The selected no-TAP shared-memory/vsock
application path, its ordinary-TCP adaptation and its kernel forwarding remain
unproven by this experiment. Combined guest/mTLS execution, established TCP
sessions, guest boots and production fault invariants are outside this review's
acceptance boundary.

## Method and scope

The reviewer inspected `.claude/rules/spike.md`, the local researcher-reviewer
role and its critique dimensions, the findings, all final Rust/Python probe
sources, the retained Linux UAPI and relevant kernel datapath implementation,
native stdout/events, object/resource readbacks, module provenance, source
manifests, original private native archives and cleanup receipts. The earlier
network-replacement research supplied context about standard OVS backers; its
earlier architecture recommendation was not treated as a current invariant.
Cached #303/#306 bodies were read solely to verify the findings' bounded
documentary statement about guest sockets, guest kTLS and host coordination.

Independent, read-only Python analyses decoded the actual Netlink bytes,
cross-joined every recorded owner FD, TAP and vport, decoded all returned flow
keys/masks/actions, verified all packet receipts and checked source/receipt
hashes. These analyses did not rely on the harness's printed zero counters or
the summary verifier's hardcoded family numbers. No additional native population
run, guest boot, production edit, roadmap edit, DES event or commit was made.
The only reviewer-written file is this artifact; pre-existing dirty work was
preserved.

The source-diversity rule for comparative literature does not require three
vendors to corroborate a native object count. Here, direct kernel readbacks,
independently enumerated process state, recorded packet bytes, UAPI semantics
and inspected execution code supply distinct evidence for the same bounded
claim. The findings preserve material limits rather than promoting the result
into an unmeasured product conclusion.

## Evidence and verification

### Actual object population and owner lifetime

The final Rust [source](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/src/main.rs)
opens `/dev/net/tun` separately for every attachment, requests
`IFF_TAP | IFF_NO_PI`, creates an explicit NETDEV vport and retains every
`File` in the owner vector until cleanup. It contains no descriptor duplication,
alias creation or sparse high-ID capacity surrogate. The process waits for the
independent observer after each cohort and again after the paired TCP exchange.

The reviewer cross-joined the retained identity list with independent
`ip -j -d link`, `/proc/<pid>/fdinfo` and Generic Netlink vport dumps for every
cohort. For each identity, the FD's `iff` name, TAP name/ifindex, kernel vport
name/ifindex and expected vport number agree. Names, ifindices, owner FD numbers
and vport numbers are distinct. Every TAP is nonpersistent, single-queue, lacks
PI/vnet headers and has the OVS master; every NETDEV vport has upcall PID `[0]`.
Port 0 is independently identified as the separate INTERNAL vport.

| Audit | Independent TAP/FD/vport bijection | Total vports | Total namespace netdevices | Kernel flows | Owner FDs |
|---|---:|---:|---:|---:|---:|
| 4 held | 4 | 5 | 6 | 8 | 10 |
| 1,024 held | 1,024 | 1,025 | 1,026 | 2,048 | 1,030 |
| 4,096 held | 4,096 | 4,097 | 4,098 | 8,192 | 4,102 |
| 8,192 held | 8,192 | 8,193 | 8,194 | 16,384 | 8,198 |
| 16,384 gateway audit | 16,384 | 16,385 | 16,386 | 32,768 | 16,390 |
| 16,384 TCP audit | 16,384 | 16,385 | 16,386 | 32,768 | 16,390 |

Primary full-population evidence: [identities](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/identities-16384.json.gz),
[kernel dump](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/stage-16384-kernel-dump.json.gz),
[links](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/stage-16384-links.json.gz),
[FD readback](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/stage-16384-tap-fdinfo.json.gz)
and [TCP audit](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/tcp-paired-16384-kernel-dump.json.gz).

### Stock kernel implementation and exact control wire

The [module attestation](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/kernel-module-attestation.txt),
[module file hash](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/module-file-sha256.json)
and [post-release readback](../../../../spike-scratch/netns-density-295-ovs-kernel/post-release-readback.json)
agree on the distribution module path, package `7.0.0-29.29`, vermagic,
`intree: Y`, srcversion `1385EBD8343BF79830D5E14`, an empty loaded-module
taint field and module hash
`5325df6f19026a892e5df54e4f03a56ec62ae7b9044301be4d067666b4b74234`.
The optional `linux-modules-extra` package is absent and is not needed for the
attested installed module. No claim equates the retained upstream reference
source with a separately verified downstream source build.

The reviewer decoded the complete final [wire capture](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/netlink-wire.jsonl.gz)
from its bytes and resolved family IDs from the actual `CTRL_CMD_GETFAMILY`
replies: datapath 43, vport 44, flow 45 and packet 46. The four family resolutions
are included. Every request sequence from 1 through 81,943 occurs exactly once;
each receives exactly one object reply and one successful ACK whose embedded
request header matches. All 163,886 received datagrams are structurally decoded.
Thus completeness is checked across request sequences and their replies,
including the final acknowledged datapath deletion.

| Decoded request family/command | Count |
|---|---:|
| Generic Netlink controller GETFAMILY | 4 |
| OVS datapath NEW | 1 |
| OVS datapath GET | 16 |
| OVS datapath DEL | 1 |
| OVS vport NEW | 16,384 |
| OVS flow NEW/update | 65,536 |
| OVS flow DEL | 1 |
| Any OVS packet-family request, including EXECUTE | 0 |

Every captured flow installation contains exactly one OUTPUT action with a
four-byte port number; none contains a USERSPACE or nested forwarding action.
Every vport creation sends the zero upcall PID, and independent kernel dumps
confirm it for all vports, including port 0. The datapath creation sends zero
upcall PID and user features; kernel readbacks retain user features zero.
There is no captured packet-family receive. The capture is the harness control
socket, not a monitor of every system socket.

This wire result is supported by the inspected source paths: the Rust owner
has a single traced Generic Netlink control socket; the Python observer uses
GET/dump operations; the runner invokes configuration and observation tools.
The process inventories and tool receipts show no OVS userspace forwarding
daemon. Neither source contains another packet-forwarding channel or an
application TCP socket. A userspace OVS netdev/DPDK fallback is not involved.

The retained [UAPI](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/openvswitch-v7.0.h)
defines OUTPUT/USERSPACE and packet EXECUTE distinctly, identifies upcall PID 0
as disabled, and defines `n_lost` as misses not sent to userspace. The inspected
[kernel datapath](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/datapath-v7.0.c)
selects the configured recipient on a miss and returns `-ENOTCONN`, incrementing
`n_lost`, when that recipient is zero. Matched flows execute their kernel
actions. These mechanisms agree with the actual wire, kernel dumps and native
counters.

### Reachable flow matches and packet coverage

The reviewer decoded all returned flow keys, masks and actions at all six held
audits. The gateway flows match either exact guest ingress with OUTPUT 0, or
exact local ingress and guest destination MAC with OUTPUT to that guest.
Ingress masks are disjoint. Returned ethertype masks are zero, so the supplied
`0x88b5` key does not exclude actual ARP or IPv4 traffic. During the TCP audit,
each guest ingress OUTPUT selects exactly its `index XOR 1` peer; local-return
flows remain intact. The retained kernel flow counters show actual hits.

Independent packet analysis checked every cohort receipt, not a sample:

| Population | Checked ARP round trips | Checked ICMP round trips | Cohort kernel hit increase |
|---:|---:|---:|---:|
| 4 | 4 | 8 | 24 |
| 1,024 | 1,024 | 2,048 | 6,144 |
| 4,096 | 4,096 | 8,192 | 24,576 |
| 8,192 | 8,192 | 16,384 | 49,152 |
| 16,384 | 16,384 | 32,768 | 98,304 |

Every identity has exactly one ARP exchange and one ICMP exchange for each
phase 1 and 2 at its cohort. Gateway/endpoint MACs, IPs, ARP fields, ICMP
identity/payload and IPv4/ICMP checksums agree. Kernel-selected reply IPv4 IDs
are correctly excluded from byte-identity claims. The 178,200 cumulative
gateway hits equal six matched kernel traversals per identity per cohort.

All 16,384 [paired TCP receipts](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/paired-tcp-traffic-16384.jsonl.gz)
were checked for complete 86-byte equality, both peer directions, unique source
identity, IPv4 length/address/header checksum, TCP ports/sequence/header,
32-byte identity payload and TCP pseudo-header checksum. Every source appears
exactly once and each destination is its XOR-1 peer. Kernel hits increase by
exactly 16,384 to 194,584. These are valid synthetic TCP frames; the fixture
does not establish TCP sessions or exercise guest kTLS.

The cold control precedes all flows; the removal control deletes the actual
port-1 ingress key/mask after peer flows are restored. Native before/after
counters show precisely one added miss and lost packet for each, zero added
hits, and the expected flow count. The inspected receiver observes no matching
reply within a **150 ms** poll window. The drop conclusion also follows from
the zero recipients and stock kernel miss path; no unbounded liveness claim is
inferred from that finite receive observation. Configured traffic adds no
misses, and final misses/lost are exactly 2/2.

The TAP writers/readers are synthetic endpoint queue participants. They never
copy a received frame to another TAP or terminate/relay an application TCP
connection. Userspace configuration, a packet upcall slow path and an
application relay are separate mechanisms. This fixture uses userspace
configuration and observers, with kernel packet forwarding and disabled
upcalls. It supplies no evidence about the placement or role of Unikraft's
unspecified custom proxy.

### Resource and timing scopes

The independent [resource readback](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/stage-16384-resources.json.gz)
confirms owner PID 836040, 16,390 FDs, one thread, RSS 23,836 KiB and PSS
21,924 KiB. Its `/proc/<pid>/exe` hash agrees with the native built-binary hash.
These are the Rust owner's resources; they omit kernel object memory and
other control/observer processes. Whole-host MemAvailable figures are separately
labeled, and the 2.735 GiB host delta is not attributed per-port kernel memory.
OOM counter delta is zero and retained snapshots preserve the reserve check.
The owner raises only its process-local soft FD limit to the existing hard
limit; global limits and neighbor thresholds are unchanged.

Timers in the actual source are interval measurements, not completion
timestamps. Attach timings measure each incremental attach/flow-install loop;
gateway and paired TCP timings include packet receipt work and relevant
control-stat readback, while paired TCP traffic timing excludes the preceding
flow rewrite. They are serial microprobe timings, not throughput or E18 data.
The source separately times acknowledged datapath deletion and destruction of
the TAP-owner vector. [Native stdout](../../../../spike-scratch/netns-density-295-ovs-kernel/increment-d/evidence/harness.stdout)
records **2.698842701 s** for datapath deletion and **99.547734967 s** for TAP FD
closure. Runner wall time is **149.868527790 s**, including audits, cleanup and
receipt compression. The material TAP teardown cost is openly reported and
cannot be treated as evidence of a five-second production recovery fit.

### Provenance, preserved increments and cleanup

All source manifests for a/b/c/d match the retained source bytes. Every
native built event's Rust-source hash matches its own increment; every
executed Cargo lock matches the retained lock. Final post-release native
source hashes match all retained increment-d sources. Independent owner
executable readbacks match the final binary hash
`d0b515251843f182eda348f82a8316bc3b463084f5d5490ec5b06e5f9c3729d2`.
The probe is a separate Cargo workspace importing only `libc` and `serde_json`.
The retained `.c` files are reference material, never compiled probe/eBPF code.

The reviewer checked all 12 lossless-compression manifest entries against
their original byte hashes and retained gzip hashes. The native private
archives' six increment-c and seven increment-d packet/control captures are
byte-identical to retained captures after decompression. All 13 local evidence
links in the findings resolve. The eligible untracked set contains 314 source,
script and receipt files; ignored `out/` archives, build targets, images and
compiled objects are absent from that eligible set.

Increment-a's failure remains intact: its native source omitted ECHO, received
an ACK, then indexed an absent object reply before any TAP was created. Its
owned local datapath remained until exact namespace deletion. Its original
failed final result and complement differences are preserved. Independent raw
comparison locates those differences exclusively at bridge `gc_timer`
169.37→165.57 and neighbor `refcnt` absent→1. The live root-namespace neighbor
event capture is empty. Later increments correct those transient normalization
fields and the ECHO request without rewriting a's source or verdict.

For b/c/d, every raw administrative inventory command succeeds, normalized
foreign configurations match, and root-namespace OVS dumps are empty before
and after. Successful runs' owned namespace inventory contains only loopback
after the harness has deleted its datapath and closed the TAP FDs, before
namespace deletion. Final independent readback finds all 12 recorded
lease/runner/harness PIDs, four owned namespace paths and lease-owner metadata
absent. Stock OVS and its dependencies deliberately remain loaded; module
inventory equality is expressly excluded.

Neighbor IP/device/MAC identities are retained in normalization; reference
counts, dynamic state/cache ages and bridge GC timer values are qualified.
Raw neighbor states and empty event captures are preserved. The live event
observer covers the runner's before/after work until it is stopped before the
final inventory; it observes the root namespace, not an indefinite future or
every namespace. Foreign-complement approval is bounded to the recorded
inventories and observer interval. No global unload, flush or foreign cleanup
is used.

## Findings and remediation disposition

| Iteration | Blocking findings | Required remediation | Disposition |
|---|---|---|---|
| 1 | None | None | APPROVED for capacity and synthetic frame forwarding on the attested stock kernel |

The findings correctly distinguish real attachment capacity, packet
preservation, userspace control, disabled packet upcalls and application
relaying. They expose the measured teardown cost and owner-only memory scope,
preserve the failed increment and loaded-module exception, and leave the
selected shared-memory/vsock application path unresolved. No architectural
mechanism or additional production acceptance gate is prescribed by this
review.
