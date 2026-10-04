# Independent research review — netns-density-295 replacement networking

## Metadata and scope

| Field | Value |
|---|---|
| Review ID | `research_rev_20261004_netns295_networking_replacement` |
| Date | 2026-10-04 |
| Role | Independent `nw-researcher-reviewer` |
| Model routing | GPT 6.1 Sol, extra-high (`xhigh`), as explicitly requested for this research/review task |
| Iteration | 1 |
| Reviewed report | [netns-density-295-networking-replacement.md](netns-density-295-networking-replacement.md) |
| Reviewed report SHA256 | `f0bde79440e4c82fc923da2b494f119f1b19ba119add3de5b1d88e8b5660d18e` |
| Repository HEAD | `9460d899897021d6cf140548e01aeb41365a68d4` |
| Verdict | **APPROVED for research and the recommendation to investigate candidate A first; two MEDIUM advisory precision notes** |
| Replacement topology / capacity approval | **Not granted**; native replacement feasibility remains unproved |
| DESIGN / roadmap disposition | **PENDING / NON-EXECUTABLE**; this review makes no status edit |

Loaded `/Users/marcus/.claude/agents/nw/nw-researcher-reviewer.md` and
`/Users/marcus/.claude/skills/nw-rr-critique-dimensions/SKILL.md`. The explicit
task controls the model, direct primary-source verification, native Markdown
artifact and bounded write scope; those instructions supersede the definition's
legacy Haiku, YAML-only and no-web defaults. This is a research review, not an
implementation review or native experiment.

The review read the whole report, independently checked the consequential
versioned limits and primary deployment claims, and compared the report's
historical audit with the existing ADR and spike receipts. It wrote only this
review. Production, support, tests, AGENTS, DES, raw measurements and design
artifacts were preserved. No commit, build, test suite, module operation or
kernel/network mutation was performed.

## Verdict and confidence

**Approve the report's bounded recommendation:** routed Ethernet TAP is a
reasonable first experiment under the current inherited-TAP, TCX,
TPROXY/kTLS/splice and singular-owner contract. The report supports this choice
with stock Linux mechanisms and concrete Firecracker/CNI precedents, while
explicitly withholding native feasibility, performance and permanent
extreme-density selection. It identifies the exact gateway/local-delivery,
neighbor admission and independent guard gaps that could invalidate candidate A.
That distinction is necessary: the research does not claim that standard
routing has already solved Overdrive's required packet path.

The report meaningfully reopens multiple bridges and shared-memory/vsock after
the official Unikraft evidence. It neither rejects 100000 instances from
Overdrive's chosen `/16` or map nor treats the later scaled-to-zero population
as a simultaneous-active-VM measurement. It preserves the conflict between the
May transport account and current TAP-pool documentation. Cloudflare's
Workers, Containers/Sandbox and edge forwarding evidence remains at its actual
execution and datapath boundaries.

Confidence is **high for the consequential source limits at their cited
versions**, **medium for candidate A as the first experiment**, and **unproved
for Overdrive replacement native feasibility or measured density/performance**.
No CRITICAL or HIGH report defect was established. The advisory notes below
sharpen already disclosed limitations and do not require an architecture or
production change.

## Independent evidence checks

### Native falsification and historical feasibility audit

The reviewed [factual record](../feature/netns-density-295/deliver/native-e18-bridge-capacity-falsification.md)
and its [iteration-2 independent approval](../feature/netns-density-295/deliver/review-design-e18-native-falsification.md)
support the report's qualification: actual `TapAttachBridge` / `set-master`
netlink `-54` refusal on `7.0.0-29-generic`, failed measurement, successful
cleanup, and no completed E18 population or samples. The report preserves the
missing benchmark digest and the distinction between the verified 891 source
entries and compiled executable identity. It correctly treats the exact
1023-success/1024th-refusal ordinal as an inference, not a sampled native count
or downstream-build attestation.

The local v7.0 bridge source independently confirms ten port bits, reservation
of port zero, and allocator `EXFULL` at exhaustion: `br_private.h:28–29` and
`br_if.c:401–418`. The source explains the refusal without manufacturing a
shutdown, retry or concurrency defect. [Versioned constants](https://github.com/torvalds/linux/blob/v7.0/net/bridge/br_private.h),
[allocator](https://github.com/torvalds/linux/blob/v7.0/net/bridge/br_if.c).

The original [spike findings](../feature/netns-density-295/spike/findings.md)
really contain two-guest transport and TCX journeys, the earlier local-delivery
failure, and Part D's N=16384 registry/listener comparison. Part D's rows at
1193–1194 count 32768 registry entries and either two or 32768 listeners; they
are not bridge-port receipts. [ADR-0117](../product/architecture/adr-0117-initial-shared-bridge-density-target.md)
explicitly describes the two-guest evidence and attachment-only measurement
scope. Consequently, the report's missing structural-feasibility gate is an
evidence-backed process finding. Its claim is bounded; it does not allege that
no spikes existed or assign invented blame.

### Capacity values, units and scope

| Claim checked | Independent source evidence | Assessment |
|---|---|---|
| Linux bridge: 1023 usable ports per bridge, uplinks included | v7.0 `br_private.h:28–29`, `br_if.c:401–418` | Correct upstream bound; deployment ordinal remains qualified. |
| Dynamic neighbors: default thresholds 128/512/1024, not an immutable interface/VM limit | v7.0 `ip-sysctl.rst:185–203`; `neighbour.c:496–517`, `:2104–2141` | Correct. The allocator tracks entries subject to GC, may reclaim before failing, and exempts permanent/external entries. Permanent registered entries bypass that GC admission, not memory/ownership cost. |
| OVS automatic ordinary OpenFlow IDs: 1…32767; explicit ordinary IDs below 65280 | Pinned OVS `configure.ac:16`, `ofproto.c:531`, `:2464–2514`, and ordinary port definitions | Correct at the cited 4.0.0 source. OpenFlow protocol width does not widen this allocator automatically. |
| Kernel OVS: ordinary IDs 1…65534 per datapath | v7.0 `datapath.h:24`; `ovs_vport_cmd_new` compares requested u32 against `DP_MAX_PORTS`; port zero is local | Correct; netlink attribute width is not implemented port capacity. |
| OVS logical switches share a same-type datapath | Pinned `ofproto-dpif.c:741–785`, plus official design FAQ | Correct. Additional logical bridges do not multiply the common kernel port space. Startup ownership deserves the advisory below. |
| VALE: 254 ports per switch; bridge-count seed eight is tunable | Pinned `netmap_bdg.h:74–80`, report's versioned tunable source | Correct single-switch exclusion; no unsupported fixed aggregate VM limit is claimed. |
| Existing Overdrive endpoint map: 65536 entries | Current `crates/overdrive-bpf/src/maps/guest_tcx.rs:19` | Correct explicit software bound. It is separate from the current `/16` address/admission policy and hardware demand. |
| Cilium endpoint IDs and endpoint-map keys: 65535 | Local pinned `pkg/endpoint/id/id.go:14–15`; `pkg/maps/lxcmap/lxcmap.go:24–25`, `:60–64` | Correct; keys/IDs are not promised guest capacity, and standard TC helpers do not inherit the whole product's allocator. |
| CH Unix-vsock: 1023 connection-map entries per muxer, RX queue 256, kill queue 128 | Pinned `unix/mod.rs:24–31`; `muxer.rs:579–609` | Values and per-muxer scope correct. “Established” is a less exact label than the actual admission predicate; advisory A1. |
| Cloudflare tubular: 1024 sockets, one million bindings | Pinned `ebpf/inet-kern.c:11–12`, map declarations and dispatcher | Correct sample software limits, not TAP/VM scale. |
| xdpcap: filters after program mutation, multi-buffer capture at most first page | Pinned README limitations at 74–82 | Correct; neither satisfies the accepted operator observer automatically. |

Independent sources: [neighbor allocator](https://github.com/torvalds/linux/blob/v7.0/net/core/neighbour.c),
[neighbor defaults](https://github.com/torvalds/linux/blob/v7.0/Documentation/networking/ip-sysctl.rst),
[OVS allocator](https://github.com/openvswitch/ovs/blob/b89dc80f0143f5bf3f58c48d54e82cc7f8d29da8/ofproto/ofproto.c),
[kernel OVS](https://github.com/torvalds/linux/blob/v7.0/net/openvswitch/datapath.c),
[CH admission](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/ae14eb5d8a4612efd8e1ed389da94a5c401c1d93/virtio-devices/src/vsock/unix/muxer.rs),
[tubular](https://github.com/cloudflare/tubular/blob/35817992491457fd026d3c69bf4daa498360d94c/ebpf/inet-kern.c),
[xdpcap](https://github.com/cloudflare/xdpcap/blob/5f60cc95b1ab2ce7c8443046b65cc41a01eb241a/README.md).

### Prior art and packet-path fit

The reviewer independently resolved the stated local Linux, Cloud Hypervisor
and Cilium HEAD pins and read consequential files at the pinned revisions using
`git show`, avoiding reliance on Cilium's staged working file. Firecracker's
pinned standalone TAP guide and CNI's pinned `ptp.go:52–62`, `:94–137` support
the limited precedent claimed: routed Ethernet and a guest route scheme which
sends peer traffic through a gateway. Neither is proof of Overdrive's guards,
TLS path or population. The clean-local-only OpenCapsule comparison is supported
by its pinned `network.rs` creation, `/30`, forwarding and nft operations; the
report does not pretend its unavailable public permalink was retrieved.
[Firecracker](https://github.com/firecracker-microvm/firecracker/blob/ab8181ed6a1e67426b75f9692b0128a9ea84dfd1/docs/network-setup.md),
[CNI](https://github.com/containernetworking/plugins/blob/0e648479e11c2c6d9109b14fc0c9ac64c677861b/plugins/main/ptp/ptp.go).

The report correctly identifies Ethernet/TAP, source IPv4/MAC/ARP validation,
host-local TCP interception, original VIP/port, protected leg-B/leg-C,
intentional leg-S plaintext, registered destination TAP/MAC, independent loss
guards, pre-first-VMM capture, activation ordering and foreign-state cleanup
as retained outcomes. It specifically discloses that the bridge-family guard
does not guard a standalone TAP or OVS path. Linux v7.0 `arp.c` and receive
ordering support the proposed investigation of local gateway replies and
TC-before-netdev-netfilter guarding. The report does not mistake that source
ordering for an approved rule/API schema or a native security result.

The replacement public/cross-crate, wire/persisted/config and semantic error
representations remain DESIGN decisions. The report expressly requires
`SPECIFICATION_AMBIGUITY` for an unspecified typed cause and forbids string
substitution. No invented method, error variant, registry, broker or recovery
protocol is proposed as accepted architecture.

### Unikraft and Cloudflare

Official Unikraft reports substantiate the earlier many-bridge/TAP deployment,
later mediated guest/host transport and extreme-density achievements. The
review independently read the May and September engineering accounts and
Prisma customer story. The report keeps the 100000-Pod availability result,
unknown simultaneous-active count and later snapshot experiment distinct.
Current TAP-pool wording was independently checked in the pinned
`prod-stable` source after the live documentation URL failed in the web tool;
this is an access-method limitation, not contrary evidence.
[May engineering account](https://unikraft.com/blog/1m-vms-single-box),
[Kraftlet report](https://unikraft.com/blog/millisecond-microvm-kubernetes),
[Prisma story](https://unikraft.com/customer-stories/prisma-postgres),
[versioned TAP documentation](https://github.com/unikraft-cloud/docs/blob/0564725fe346f86b253ab5421497cae795f8a4e5/pages/features/custom-network-configuration.mdx).

The reviewer also checked public Unikraft guest vsock source locally at the
stated pin; guest transport capability does not disclose the private cloud host
runtime. No unpublished benchmark/VMM/kernel commit or unseen video frame is
invented. Reported current pool documentation and the May account remain an
unresolved edition/version/path conflict rather than grounds to dismiss either.

Cloudflare's current lifecycle and outbound docs support the product isolation
and proxy distinctions. Its first-party Unimog article and published
`cls-redirect` code support stock TC/XDP mechanisms and observation tradeoffs,
not a VM-host attachment allocator. The tubular and xdpcap details were checked
directly at their pins. The report makes no unsupported claim that private
Cloudflare VM hosts use routed TAP or have any particular port capacity.
[Container lifecycle](https://developers.cloudflare.com/containers/concepts/architecture/),
[outbound behavior](https://developers.cloudflare.com/containers/configuration/outbound-traffic/),
[Unimog](https://blog.cloudflare.com/unimog-cloudflares-edge-load-balancer/).

## Findings and dispositions

### A1 — MEDIUM, advisory: label the actual CH connection admission set

**Location:** report line 244 and summaries using “1023 established
connections” or “streams.” **Proven documentary precision issue:**
`add_connection` gates the length of `conn_map`, not a count filtered to
`ConnState::Established`. The host path passes `new_local_init` into it at
`muxer.rs:479–494`; the guest path passes `new_peer_init` at `:720–739`.
Therefore initialization and other still-retained connection objects consume
the same admission space. The source comment describes established connections,
but the executable predicate is more precise.

**Suggested correction:** say “at most 1023 tracked active connection entries
per Unix-vsock muxer, including initialization/retained connection states,”
then explain that established application streams share that budget. Retain
the already correct per-muxer/per-VMM scope, 256/128 queue sizes and absence of
a host-VM-count inference. This does not show an Overdrive defect or require a
new protocol, backend edit or experiment.

**Disposition:** advisory; no approval blocker. Researcher may sharpen the
wording before a candidate-G experimental handoff.

### A2 — MEDIUM, advisory: make OVS startup authority concrete

**Location:** report lines 99–103 and the candidate-C integration/lifecycle
discussion. **Source evidence:** at the cited OVS pin,
`open_dpif_backer` enumerates same-type datapaths and deletes each successfully
opened datapath whose name differs from its own backer name
(`ofproto-dpif.c:760–779`). This is a concrete startup authority assumption
in addition to the common-backer port limit.
[Pinned startup path](https://github.com/openvswitch/ovs/blob/b89dc80f0143f5bf3f58c48d54e82cc7f8d29da8/ofproto/ofproto-dpif.c).

The report already requires foreign OVS configuration preservation and exact
daemon authority, so it does not approve unsafe startup. Naming this operation
would make that existing obligation easier to test if C is selected.
**Suggested correction:** add the versioned startup behavior to candidate C's
ownership caveat and require its future isolated startup/cleanup observation
to demonstrate the already retained foreign complement. Do not infer that
current Overdrive invokes this code, characterize it as a reproduced
production defect, or prescribe a namespace, persistence or controller remedy.

**Disposition:** advisory; no approval blocker and no scope expansion.

## Evaluation across the five research dimensions

| Dimension | Score | Evidence and practical limit |
|---|---:|---|
| Source selection bias | 0.92 | 96 unique remote citation URLs confirmed mechanically. Linux project/docs supply 35, about 36%, below the 60% single-publisher trigger. Several independent projects and competing mechanisms appear. Commercial interests and related pages are explicitly treated as dependent. |
| Evidence quality | 0.92 | Consequential allocator/default/queue values independently checked at exact source pins; local receipts support the historical audit. Commercial deployments are labeled primary claims with absent raw/build details. Two advisory precision notes remain. |
| Replicability | 0.88 | Exact local paths, commits, URLs, access dates, failed retrievals, dirty-source qualifications and ordered future probes are provided. The review verified the URL count and all relative file destinations; it did not repeat the author's entire 96-URL retrieval sweep. Deployed private builds remain unavailable. |
| Priority validation | 0.93 | Addresses the reproduced bridge refusal and removal of old NetSlot/netns costs. Competing bridges/modules/registered delivery and the transport object model are evaluated. Preference for A is justified as a first bounded experiment, not a measured performance winner. |
| Completeness | 0.94 | Packet/security/owner/observer gaps, structural and tunable limits, hardware costs, conflicting current docs, private-source gaps, reversal conditions and typed-error/API gates are explicit. Exact replacement architecture remains properly undecided. |

A canonical allocator file is authoritative evidence for its implemented
constant. Repeating it across three pages would not make those sources
independent. Conversely, source feasibility and vendor achievement do not
replace native Overdrive measurements. The report applies that distinction
consistently enough for research approval.

## Required proof still outstanding

Research approval authorizes no experiment or DELIVER resumption. The report's
future SPIKE must close these existing gaps through its explicitly approved
experimental contract and resource budget:

1. Attest the actual parent benchmark, target kernel/config/source association,
   module/VMM bytes and appliance resources; preserve old failed/raw evidence.
2. Prove the chosen Ethernet gateway/IP/MAC/guest-route scheme on two real
   guests with original peer/VIP destinations, shared DNS and the actual
   protected transport/leg-S delivery. Capture must precede the source VMM's
   CPU opportunity and account for loss/truncation.
3. Exercise the real owner/effect ordering and exact approved experimental
   composition. Primitive success is separate feasibility evidence; it cannot
   attest the current bridge-specific owner as a routed replacement.
4. Cross the old bridge and NetSlot boundaries with attachment-only owner
   populations and sampled failure indices/counts, and verify the selected
   dynamic-neighbor or permanent-entry contract without moving the ceiling.
   Sparse high OVS IDs do not establish populated capacity.
5. Witness source/ARP/destination isolation, independent guard losses,
   zero-frame activation, approved owner stop/restart and reverse cleanup with
   the exact foreign complement. Any missing interface/error representation
   returns to DESIGN; no string, public seam or invented state closes it.
6. For a claimed control-plane ordering/retry/convergence defect, first obtain
   a reproducible existing `overdrive-sim` invariant failure and printed seed
   through the current relevant production composition. Native kernel effects
   retain their separate evidence boundary. No such new defect is established
   by this research review.
7. Measure equal-resource attachment/lifecycle and active-traffic costs before
   selecting a permanent density architecture; obtain the proper E18 profiles
   and runtime-window receipts before advancing their dependent validation.
   Keep A/D/G object and active-versus-retained populations explicit.

These are approval conditions already in the report and accepted contract,
not new persistence, lifecycle or generalized hardening requirements. A later
normative replacement requires exact DESIGN, independent design review and
roadmap approval. The 16384 placeholder remains a hardware-dependent
measurement/admission choice rather than a capacity promise; 100000 is neither
promised nor declared impossible.

## Iteration history

| Iteration | Verdict | Findings / disposition |
|---|---|---|
| 1 — 2026-10-04 | **APPROVED — research and first bounded experiment recommendation only** | No CRITICAL/HIGH defect established. A1 and A2 are MEDIUM advisory precision notes. Native replacement feasibility, topology acceptance, E18/runtime bounds and roadmap execution remain pending. |

This complete on-disk artifact is the returned research-review evidence. It
does not substitute for any subsequent SPIKE, DESIGN or DELIVER review.

## Iteration 2 — advisory correction closure

| Field | Value |
|---|---|
| Date | 2026-10-04 |
| Reviewed report SHA256 | `ebe3ae10801b8e596987569a63aab2e1fd68335182c785b7a235d8859b870a1c` |
| Scope | Bounded verification of A1/A2 corrections and retained recommendation/approval boundaries |
| Latest verdict | **APPROVED — research and routed TAP as the first bounded experiment recommendation only** |
| Advisory disposition | **A1 CLOSED; A2 CLOSED** |
| Replacement feasibility / DESIGN / roadmap | **Unproved / PENDING / NON-EXECUTABLE** |

### Verified remediation

**A1 CLOSED.** The candidate-G summary at report line 67, performance table at
line 155 and detailed admission analysis at line 244 now use the actual
1023-entry connection-admission budget per Unix-vsock muxer/VMM. The detailed
text names `conn_map.len()`, initialization and other retained states, and the
host/guest initialization paths already verified in iteration 1. Established
application streams explicitly share that budget. The 256-entry RX and
128-entry kill queues remain distinct, and no host-wide VM-count limit is
inferred. No stale established-only 1023 label was found in the report.

**A2 CLOSED.** Candidate C at line 103 now names `open_dpif_backer`'s deletion
of successfully opened same-type datapaths whose names differ from its own
backer, with the same immutable source citation and exact line range verified
in iteration 1. It requires isolated daemon-startup/cleanup evidence of the
retained foreign complement before selection. S6 at line 304 includes that
observation. The report explicitly distinguishes this candidate source behavior
from an established current Overdrive production defect and prescribes no new
namespace, persistence subsystem or controller remedy.

### Verification and latest disposition

The reviewer independently recalculated the report hash, checked the corrected
summaries and detailed paragraphs, searched for the stale capacity terminology,
and verified that its relative file destinations resolve. The recommendation
at line 283 remains medium confidence for A as the first bounded experiment;
native replacement feasibility/performance remains unproved. The coverage
statement at line 458 retains pending replacement DESIGN and roadmap validation.
No new experiment, broad source audit or test suite was needed to close these
documentary notes.

**APPROVED.** There are no open research-review findings. All outstanding
native SPIKE, exact DESIGN/error/API, independent design-review, E18/runtime-bound
and roadmap approval conditions recorded above remain required. This latest
verdict grants no validated topology, capacity promise or DELIVER resumption.
Iteration 1 and its original evidence/findings are preserved above; this
iteration appends their verified dispositions. The closure review changed only
this review artifact and performed no commit or native operation.
