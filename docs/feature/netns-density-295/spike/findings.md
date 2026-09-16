# Spike Findings — netns-density-295

## Current verdict: WORKS (Part A and Part B)

**The assigned mechanism works on the configured native metal host.** Two real
Cloud Hypervisor microVMs used one shared Linux bridge and one `/24`, with no
host per-workload netns, veth pair, or `/30`. The client guest resolved
`peer.mesh` through a DNS socket on the shared bridge, dialed ordinary
plaintext TCP, and received the peer guest's byte-distinct reply. The source
tap's bridge rule delivered traffic to leg-F; a real leg-B↔leg-C TCP segment
carried mutually authenticated TLS 1.3 with kernel TLS TX/RX. Both captured
stream directions contain `0x17` records and zero application cleartext.

The user explicitly authorized native metal execution and removed the kernel
version gate on 2026-09-14 local time. That instruction supersedes the older
Lima/pinned-kernel wording in the issue and the first prerequisite stop
retained below. Kernel version is metadata, not a verdict gate.

Phase 1 only: no promotion decision, production change, commit, or
`wave-decisions.md` was made. **There is no remaining blocker.** The assumption,
hypothesis, prediction, and falsification recorded below remain unchanged.

## Working mechanism and its measured boundary

```text
guest A 10.95.0.2/24 — plaintext to peer.mesh → 10.95.0.3:9000
  tap295a → br295p per-port mark + Ethernet destination rewrite to bridge
  IP prerouting → IP_TRANSPARENT leg-F (getsockname = 10.95.0.3:9000)
  leg-B 10.95.0.1:60932 ⇄ TLS 1.3 / kTLS over lo ⇄ leg-C 10.95.0.3:9000
  leg-S 10.95.0.1:60948 → br295p → tap295b → plaintext peer guest B
  byte-distinct response follows the reverse socket path

guest DNS → tap295a → shared bridge address 10.95.0.1:53
```

The TCP bridge rule matches the **physical source tap**, stamps `0x295a`,
rewrites only the Ethernet destination to the bridge's MAC, and sets packet
type `host`. Normal bridge delivery then enters the IP stack on `br295p`,
where that mark selects leg-F's TPROXY listener. The IPv4 destination and TCP
port remain unchanged; Rust `getsockname` proves their preservation.

Leg-B's own socket mark selects the same-node output-divert rule and leg-C's
inbound TPROXY rule for the destination guest port. Leg-S has a different
socket mark and reaches guest B directly after decryption. One shared policy
route sends TPROXY-marked traffic to the local IP stack. There is one bridge
subnet route, not a per-guest routed cell.

DNS uses ordinary local delivery to the shared bridge's bound UDP socket.
The per-tap bridge rule observes both A and AAAA requests; the responder
returns the peer A record and an empty AAAA answer. The guest's own rootfs has
`/etc/resolv.conf` pointing to `10.95.0.1`. No host `/etc/netns/*/resolv.conf`
or host per-workload namespace is involved.

This is a standalone Rust mechanism assembly matching production socket,
TPROXY, rustls handshake, and kTLS primitives, without an `overdrive-*` crate
dependency. The agent-role sockets share one host process; leg-B↔leg-C is a
real kernel TCP segment observed on **loopback**, the same-node case. It is
not a claim about cross-host routing, production `serve` enrollment, or
workload-density/throughput limits. Guest credentials are absent; the two
agent roles mint and hold their mutually verified certificates in host memory.

## Final metal execution and raw evidence

Working source and final capture:
[`increment-u-netns-density-295-default-offload-20260913T2302`](../../../../spike-scratch/increment-u-netns-density-295-default-offload-20260913T2302/).
Its [run transcript](../../../../spike-scratch/increment-u-netns-density-295-default-offload-20260913T2302/capture-attempt-08.log)
contains the actual canonical lease acquisition, build, commands, stdout,
stderr, outcome, and cleanup. Its
[`evidence-attempt-08/`](../../../../spike-scratch/increment-u-netns-density-295-default-offload-20260913T2302/evidence-attempt-08/)
retains `lo.pcap`, both tap pcaps, guest consoles, host logs, nft trace, and
capture-process statistics.

Exact execution command, from the workspace root:

```sh
OVERDRIVE_METAL_KERNEL=/var/tmp/spike-increment-n/kernel \
OVERDRIVE_METAL_ROOTFS=/var/tmp/spike-increment-n/rootfs.ext4 \
OVERDRIVE_METAL_SCENARIO=netns-density-295-attempt-08 \
cargo xtask metal run -- env CC_x86_64_unknown_linux_musl=cc \
  bash spike-scratch/increment-u-netns-density-295-default-offload-20260913T2302/run.sh
```

The existing rootfs above satisfies the runner's artifact preflight; it is
**not booted** by this probe. The script builds its own static Rust `/init`
and fresh client/server rootfs images under the unique native run directory
`/var/tmp/gh295-p.uZr6yx9h`. It reads the existing kernel without modifying it.
The actual VMM commands and fresh rootfs paths appear in the transcript.

Substrate: native `x86_64`, `systemd-detect-virt=none`, host and guest kernel
`7.0.0-29-generic`, Cloud Hypervisor `v53.0`, nftables `v1.1.6`, iproute2
`6.19.0`. The canonical runner's native CPU/KVM open/API/create-VM preflight
passed. Both microVMs used one vCPU and 256 MiB; default NIC offloads remained
enabled in the final run. Source commit remains
`5377fcb85681b33fb27da6025374fa5b1990fd3f`; the user's dirty `AGENTS.md` was
preserved throughout.

Actual final output excerpts:

```text
GUEST DNS RESOLVED peer.mesh=10.95.0.3:9000
GUEST PLAINTEXT REQUEST=GH295-PLAINTEXT-GUEST-REQUEST-7
GUEST ROUNDTRIP SUCCESS RESPONSE=GH295-BYTE-DISTINCT-PEER-RESPONSE-42 elapsed_seconds=0.048952
GUEST EXIT=0

LEG_F owner=tap295a GETSOCKNAME_ORIGINAL=10.95.0.3:9000 PEER=10.95.0.2:39770
LEG_B WIRE local=10.95.0.1:60932 peer=10.95.0.3:9000 mark=0x2951
LEG_C owner=tap295b GETSOCKNAME_ORIGINAL=10.95.0.3:9000 PEER=10.95.0.1:60932
LEG_B MTLS_AUTHENTICATED server_certificates=1 version=Some(TLSv1_3)
LEG_B KTLS_ARMED direction=1 tls_version=0x0304 cipher=52 seq=0
LEG_B KTLS_ARMED direction=2 tls_version=0x0304 cipher=52 seq=0
LEG_C MTLS_AUTHENTICATED client_certificates=1 version=Some(TLSv1_3)
LEG_C KTLS_ARMED direction=1 tls_version=0x0304 cipher=52 seq=0
LEG_C KTLS_ARMED direction=2 tls_version=0x0304 cipher=52 seq=0
LEG_S CLEAR_TO_GUEST local=10.95.0.1:60948 peer=10.95.0.3:9000 mark=0x2952
LEG_C COMPLETE request_bytes=32 response_bytes=37
LEG_F COMPLETE request_bytes=32 response_bytes=37

WIRE_SCAN ('10.95.0.1', 60932, '10.95.0.3', 9000) stream_bytes=797 tls_records={20: 1, 22: 1, 23: 2} application_data_0x17=2 plaintext=0 gaps=0
WIRE_SCAN ('10.95.0.3', 9000, '10.95.0.1', 60932) stream_bytes=796 tls_records={20: 1, 22: 1, 23: 2} application_data_0x17=2 plaintext=0 gaps=0
SHARED_L2 source_tap_plaintext_request=1 peer_tap_agent_plaintext_request=1 peer_tap_plaintext_response=1 guest_to_guest_bypass_packets=0
VERDICT=WORKS: two real microVMs, guest plaintext DNS dial, per-port bridge-local+TPROXY, mutual TLS1.3 with kTLS TX/RX, zero cleartext on leg-B/leg-C, no direct shared-L2 bypass
CLEANUP COMPLETE preserved_evidence=/var/tmp/gh295-p.uZr6yx9h original_exit=0
TOTAL_ELAPSED_SECONDS=15.850261
```

The scanner reassembles both captured inter-agent TCP directions, rejects
gaps/conflicting retransmissions, consumes every stream byte as a complete
TLS record, and searches for both distinct application payloads. Guest and
peer capture positives accompany that negative check. Independently decoding
the source tap pcap shows the guest's SYN and plaintext data were originally
addressed to **guest B's MAC `02:00:00:95:00:03`**, so this was real on-subnet
guest-to-guest traffic intercepted at the switch port. No TCP packet with
source guest A and destination guest B reached the peer tap. Plaintext on the
peer tap belongs to the agent's **decrypted leg-S connection**, as intended.

Final capture statistics: loopback 17 captured / 34 received by filter,
source tap 13 / 13, peer tap 14 / 14; all report **zero kernel capture drops**.
The two reconstructed inter-agent byte streams have no gaps.

## Metal timing, failed attempts, and corrected assumptions

Mechanism work resumed at **22:37:29 UTC**. The final successful run started
at **23:02:55 UTC**, ended about **23:03:11 UTC**, and measured **15.850261 s**
including a **12.63 s** cold build. Guest DNS plus request/response took
**48.952 ms**. A preceding successful run with offloads disabled took
**49.117 ms** for the guest journey. These are point measurements, not a
benchmark or density claim. The post-cleanup check completed at 23:05:12 UTC,
27m43s after resumption; the one-hour maximum was not approached.
Final local scope verification and retained findings completed at 23:09:53
UTC: **32m24s** for the resumed Phase 1, including investigation and reporting.

All attempts remain in distinct scratch increments with their raw logs:

| Increment / capture | Observation | Disposition within this probe |
|---|---|---|
| `increment-p-netns-density-295-metal-20260913T2237`, attempts 01–02 | Missing musl C compiler selection, then musl ioctl argument-width compile errors | Selected existing `cc`; cast ioctl requests to the target ABI. These are build corrections, not feasibility results. |
| Same increment, attempt 03 | Two guests booted; bare `broute` DNS packets never reached the responder | Retained first failed runtime transcript and pcaps. |
| `increment-q-netns-density-295-broute-host-20260913T2252`, attempt 04 | Adding packet type `host` did not repair DNS | Rejected that intervention as sufficient. |
| `increment-r-netns-density-295-offload-20260913T2254`, attempt 05 | Disabling NIC offloads still left DNS failing; nft trace showed IP prerouting on `tap295a` | Checksum/offload change was not a remedy. |
| `increment-s-netns-density-295-bridge-dns-20260913T2257`, attempt 06 | Normal bridge-local DNS succeeded; bare-port TCP `broute` reached the TPROXY rule but never completed a connection | The shared-bridge DNS home was demonstrated; bare `broute` alone remained insufficient. |
| `increment-t-netns-density-295-bridge-local-20260913T2259`, attempt 07 | Per-port mark + bridge-MAC delivery + IP TPROXY completed the entire journey | **WORKS**, with offloads disabled. |
| `increment-u-netns-density-295-default-offload-20260913T2302`, attempt 08 | The same working mechanism completed with default NIC offloads | **WORKS**; disabling offloads is unnecessary for the measured journey. |

The important correction is that **literal `broute` is not equivalent to
normal bridge-local IP delivery** in this setup. Passing traffic enters IP
prerouting on the bridge with a source-port mark; failed bare-broute traffic
entered on the addressless tap. The kernel's precise drop site for the
failed variants was not isolated, so this is an observed working/failed
configuration distinction, not a claimed kernel root cause. Upstream's
[bridge receive path](https://raw.githubusercontent.com/torvalds/linux/v7.0/net/bridge/br_input.c)
and [nft metadata implementation](https://raw.githubusercontent.com/torvalds/linux/v7.0/net/bridge/netfilter/nft_meta_bridge.c)
corroborate that these delivery paths differ; the verdict comes from the
executions above.

The first runtime attempt used `panic=1`, so Cloud Hypervisor reopened the
guest console on automatic guest reboot and the first guest panic text was
not retained. Its outer transcript and pcaps remain. Every later attempt
used `panic=0`; its first panic and subsequent cleanup were preserved. This
capture mistake is not evidence for or against the networking mechanism.

## Cleanup and retained scope

The independent [post-cleanup capture](../../../../spike-scratch/increment-u-netns-density-295-default-offload-20260913T2302/post-cleanup.log)
confirmed `br295p`, `tap295a`, and `tap295b` absent, no `gh295p` nft table,
no rule/table-295 routes, and the original `rp_filter=0`. Existing
`overdrive-mtls` rules and the unrelated pre-existing veth pair were preserved.
Each owned Cloud Hypervisor process and the host probe exited; diagnostic
processes were stopped by their recorded PIDs. Native run directories and
build outputs remain as scratch; no disk images are added to the repository.

Final binary SHA-256:
`1f1465db80cc82fef25319db05078ae98c5d807de1895c2db2559b5a16682780`.
Read-only kernel SHA-256:
`b51367c7dab2f3824ca811c7e33b7f6bb0ddc8122b48248335ba6164de8d9682`.
Final Rust source SHA-256:
`ed32624fdf92f721d3cbd8d693a2bed3a64f3b9967b7fb409b8c30f15e98b1f9`.

This establishes a feasible shared-bridge home for the bounded IPv4
interception/DNS journey. It does not select an architecture or authorize
promotion. Findings and all throwaway sources/captures are retained for the
orchestrator's user promotion gate.

---

## Historical Lima prerequisite stop — superseded, no mechanism verdict

The original report below used the label “BIGGER THAN EXPECTED” for a
prerequisite mismatch. It is retained as history of the first dispatch,
**not** the current verdict. The user's subsequent native-metal/no-kernel-gate
instruction resolved that mismatch; the WORKS evidence above supersedes it.

Phase 1 could not exercise the requested mechanism on the required substrate.
The requested `cargo xtask lima run --` target is running **7.0.0-30-generic**,
not the repository's pinned **6.18 LTS** kernel. It is an **aarch64 Apple
virtualized host**; the current repository real-guest rule requires native,
nonvirtualized x86_64 KVM and explicitly treats Lima/nesting as non-signal.

**Feasibility remains unknown.** This is neither a demonstrated WORKS nor a
demonstrated DOESN'T WORK for shared-switch interception. No microVM was booted,
no TPROXY/DNS/mTLS path was exercised, and no wire capture exists. The stop is
an observed prerequisite mismatch, not expiration of the one-hour timebox.

## Question tested

Authoritative input: [issue #295 and its complete comment thread](https://github.com/overdrive-sh/overdrive/issues/295),
read with `gh issue view 295 --comments` and
`gh issue view 295 --json title,body,comments,url`.

Exactly one assumption was assigned:

> The transparent-mTLS interception (leg-C inbound + leg-F outbound TPROXY
> origination/termination) and dial-by-name DNS can be driven at the tap /
> shared-vswitch per-port layer with NO per-workload netns, and still
> transparently mTLS a real Cloud-Hypervisor microVM's east-west traffic on a
> real kernel.

The [first issue comment](https://github.com/overdrive-sh/overdrive/issues/295#issuecomment-5656342040)
pins the following statements; none was weakened to fit the available host:

- **Hypothesis:** a guest tap attached to one shared bridge/vswitch (flat
  subnet, no per-workload netns/veth/`/30`) can have its egress/ingress
  TPROXY-captured and mTLS-legged per-port, byte-equivalent to today's
  per-netns capture.
- **Predicted outcome:** a byte-distinct request/response round-trips through
  the agent's leg-B↔leg-C wire carrying TLS 1.3 `application_data` (`0x17`)
  records with zero cleartext, while the guest dials plaintext by name
  (dial-by-name resolves without a per-netns `resolv.conf`).
- **Falsification:** IP_TRANSPARENT / TPROXY does not fire without the
  per-netns routing anchor (the guest's default-route gateway that today
  lives on the veth end), OR per-port DNS cannot be served without the
  per-netns `resolv.conf`, OR the shared-L2 segment leaks cleartext between
  guests.

The guest-side egress dialer must be plaintext and credential-free. TLS
observation belongs on leg-B↔leg-C. Neither that prediction nor any listed
falsification was observed in this attempt.

## Executed evidence

Source commit: `5377fcb85681b33fb27da6025374fa5b1990fd3f`.
Initial tracked dirty state: only the user's `AGENTS.md` modification.
The agent did not modify, stage, or commit that file.

Executed from the repository root:

```sh
cargo xtask lima run -- bash \
  spike-scratch/increment-o-netns-density-295-preflight-20260913T2227/probe.sh
```

The wrapper executed as root inside the existing `overdrive` Lima instance.
The retained [raw stdout/stderr](../../../../spike-scratch/increment-o-netns-density-295-preflight-20260913T2227/evidence-mpwZDSO7/preflight.log)
records every diagnostic command, start/completion timestamp, and exit status.
Extracts from the actual 2026-09-13 22:27:28 UTC execution:

```text
COMMAND uname -r
7.0.0-30-generic
COMMAND uname -m
aarch64
COMMAND systemd-detect-virt
apple
COMMAND id
uid=0(root) gid=0(root) groups=0(root)
COMMAND cloud-hypervisor --version
cloud-hypervisor v53.0
Migration Protocol Versions: 0
COMMAND ls -l /dev/kvm
crw-rw-rw- 1 root kvm 10, 232 Sep 11 06:46 /dev/kvm
COMMAND ls -l /lib/modules
total 8
drwxr-xr-x 6 root root 4096 Aug 23 18:31 7.0.0-30-generic
drwxr-xr-x 6 root root 4096 Sep  6 16:29 7.0.0-31-generic
COMMAND nft --version
nftables v1.1.6 (Commodore Bullmoose #7)
PINNED_KERNEL_MATCH=no REQUIRED=6.18_LTS ACTUAL=7.0.0-30-generic
NATIVE_GUEST_EVIDENCE_REQUIREMENT=x86_64_nonvirtualized
ACTUAL_ARCH=aarch64 ACTUAL_VIRTUALIZATION=apple
MECHANISM_EXECUTED=no
MICROVM_BOOTED=no
WIRE_CAPTURE=none
STOP_REASON=required_pinned_kernel_and_allowed_real_guest_substrate_unavailable_in_this_Lima_instance
PREFLIGHT_ELAPSED_SECONDS=0.069477
PROBE_OUTCOME=BLOCKED_BEFORE_MECHANISM
```

The probe exited **2**. The enclosing xtask exited **1**, reporting
`limactl shell <cmd> failed with exit status: 2`. This was a real execution
of prerequisite diagnostics, not a compile-only mechanism result.

The `/boot` listing in the capture contains only the installed 7.0.0-30 and
7.0.0-31 kernel images. This establishes what is installed there; it does
not assert that a 6.18 image is absent from every possible filesystem path.
Presence of `/dev/kvm` is not claimed as proof of successful KVM_CREATE_VM:
that ioctl and Cloud Hypervisor guest boot were not attempted after the
kernel/substrate mismatch was established.

## Alternatives checked within the requested scope

The host-side `limactl list --format '{{.Name}} {{.Status}} {{.Arch}} {{.VMType}}'`
returned:

```text
obi-spike Stopped aarch64 vz
overdrive Running aarch64 vz
```

The requested wrapper fixes `LIMA_INSTANCE = "overdrive"` at
`xtask/src/main.rs:529`; its `Run` implementation selects that instance at
line 659. No alternative instance was started or reconfigured.

The apparent pinned-kernel escape route, `integration_vm` at
`xtask/src/main.rs:1175`, is currently a placeholder rather than a kernel
boot implementation. It was inspected, not treated as successful execution.

The current real-guest restriction is explicit in
`.claude/rules/testing.md:1473`–1493: native, nonvirtualized x86_64 KVM is the
authoritative boot substrate, and a real-guest feature run must fail closed
when the substrate probes do not pass. The pinned 6.18 LTS requirement is in
that file at line 1326 and ADR-0068. These constraints cannot be satisfied by
the currently running Lima instance. Even if the issue's Lima instruction
were intended to permit nested mechanism experimentation, the independently
required pinned host kernel is still missing from the selected running host.

## Timing

- First recorded input-reading timestamp: 2026-09-13 22:23:27 UTC.
- Decisive retained preflight: 2026-09-13 22:27:28 UTC, about four minutes
  after that timestamp; elapsed preflight time measured with
  `time.perf_counter()` was **0.069477 seconds**.
- Enclosing capture command wall time: **1.023513 seconds**.
- Recorded work interval through final scope verification: 22:23:27–22:29:38
  UTC (**6 minutes 11 seconds**, within the one-hour maximum; initial
  instruction reads began just before the first recorded timestamp).
- Mechanism timing: **not measured**. No performance requirement was assigned
  beyond the one-hour maximum probe duration.

## Edge cases and incorrect assumptions

1. `cargo xtask lima run --` selects a Linux environment; it does not select
   the pinned appliance kernel. This host has advanced to 7.0.
2. Installed Cloud Hypervisor and a character device at `/dev/kvm` do not
   establish the required nonvirtualized substrate or a successful guest boot.
3. Existing `spike-scratch/increment-n-guest-tap-tproxy/` was audited before
   reuse was considered. Its topology explicitly creates `probens`, a veth
   pair, and two `/30`s; its C listener replies directly, without the required
   DNS or leg-B↔leg-C mTLS proof. Its cleanup also uses broad process matches.
   It therefore cannot be adopted as evidence for this assumption. None of
   its files or historical evidence was changed or executed.
4. Earlier microVM findings distinguish Apple nesting artifacts from the
   aarch64 architecture itself. This attempt makes no claim that aarch64
   inherently prevents shared-switch interception.

## Design implications and blocker

This attempt provides **no feasibility evidence for selecting either dataplane
model**. The ADR must not cite it as a negative mechanism result or as support
for the shared-switch proposal.

The orchestrator must resolve the execution-substrate conflict before the
mechanism can be probed: the requested runner selects virtualized Lima on
7.0, while the assignment requires the pinned kernel and current repository
rules require native real-guest evidence. Changing runner/substrate or
reconfiguring the shared VM is outside this bounded Phase-1 attempt. No such
change was made.

## Retained scratch and handoff

Throwaway diagnostic source and raw capture remain at
`spike-scratch/increment-o-netns-density-295-preflight-20260913T2227/`.
The unique directory preserves all earlier increments and the first captured
failure. No production code, tests, public APIs, networking objects, kernel
configuration, GitHub issues, or commits were created or changed.

Phase 1 returns a genuine blocker. No PROMOTE/DISCARD/PIVOT choice was made;
`wave-decisions.md` was not created.

---

## Part B — production-core proof after Exec removal

### Part-B verdict: WORKS

The stronger Part-B assumption works on the configured native metal target at
source commit `67d5adba72ad97e83ccc9cc7893f943e1a703d90`. Two real Cloud
Hypervisor microVMs used two TAP ports on one Linux bridge and one shared `/24`.
The client guest resolved `peer.mesh`, dialed ordinary credential-free plaintext,
and completed a two-phase, byte-distinct request/response exchange. The second
exchange occurred after the mTLS session was established and crossed the actual
production zero-copy pumps.

Unlike Part A's standalone reproduction of the kernel primitives, Part B linked
and executed the production `HostMtlsEnforcement`. Its real TLS 1.3 handshake,
kTLS TX/RX arm, and four splice pumps carried leg-B↔leg-C. The production
`RcgenCa` issued both SVIDs from OS entropy, the production `IdentityMgr` held
them and the trust bundle, and the production `ServiceBackendsResolve` classified
the recovered original destination. The production `CgroupManager` over
`RealCgroupFs` created, populated, and removed one cgroup-v2 scope per VMM.

No final-path per-workload netns, veth pair, transit or guest `/30`, `NetSlot`,
or `host_veth` was created or consulted. The host already contained unrelated
pre-existing `ovd-ns-*`, `ovd-hv-*`, and `/30` objects from other workloads; the
Part-B before/after snapshots prove that set was byte-identical. The claim is
therefore precise: the successful Part-B path has no dependency on those
objects; it is not a claim that the leased host was globally empty of unrelated
objects.

Phase 1 only: Part B makes no architecture selection, promotion decision,
production change, commit, or `wave-decisions.md`.

### Production components actually reused

| Concern | Production component executed | Part-B evidence |
|---|---|---|
| TLS 1.3, kTLS TX/RX, zero-copy data movement | `overdrive_dataplane::mtls::HostMtlsEnforcement` through the `MtlsEnforcement` port | Earned-Trust probe passed; real outbound and inbound handles reached Established; `ss` showed two `tcp-ulp-tls version: 1.3 ... rxconf: sw txconf: sw` records; post-ready `strace` captured 14 successful `splice(2)` calls. |
| Workload CA and SVIDs | `overdrive_host::RcgenCa` + `OsEntropy`, using `Ca::issue_intermediate`, `Ca::issue_svid`, and `Ca::trust_bundle` | Two canonical `SpiffeId::for_allocation` identities were minted and used by the real mutual handshake. |
| Node-held identity custody | `overdrive_control_plane::identity_mgr::IdentityMgr` through `IdentityRead` | `held_count=2`; neither guest image contained a certificate or key. |
| Mesh classification | `overdrive_control_plane::mtls_resolve_adapter::ServiceBackendsResolve` through `MtlsResolve` | `probe()` passed; recovered `10.95.0.3:9000` classified `Mesh` to the same healthy backend. A `SimObservationStore` supplied the one scratch row; the resolver itself was the production host adapter. |
| Per-VM resource/lifecycle boundary | `overdrive_worker::CgroupManager` + `overdrive_host::RealCgroupFs` + production `CgroupPath::for_alloc` | Created two scopes, applied production resource-limit writes, placed each real Cloud Hypervisor PID, and removed both scopes during cleanup. |
| Domain contracts | `AllocationId`, `WorkloadId`, `SpiffeId`, `ServiceId`, `ServiceBackendRow`, `Backend`, `InterceptedConnection`, `Routed`, `MtlsResolution` | The scratch assembly crossed the real production port shapes rather than reproducing them. |

The production `CloudHypervisorVmm`/`VmDriver` launch surface was not reusable
for this no-netns proof: its current `VmNetworkAttachment` still carries a
netns and the VMM adapter launches networked VMs through `ip netns exec`.
Changing that public API in a spike would violate the production-surface rule.
Part B therefore launched Cloud Hypervisor directly in scratch, attached the
host-netns TAPs, and reused the production cgroup owner independently. This is
the concrete obsolete API seam that DESIGN must replace; it is not a mechanism
falsification.

### Scratch-only missing mechanism

The only new mechanism is in
[`increment-v-part-b-production-core-netns-density-295-20260916T002721Z`](../../../../spike-scratch/increment-v-part-b-production-core-netns-density-295-20260916T002721Z/):

1. `tap295ba` and `tap295bb` are ordinary ports on `br295b`, whose only subnet
   is `10.95.0.0/24`.
2. A bridge-prerouting rule matches the physical source TAP `tap295ba`, retains
   the IPv4 destination and TCP port, stamps the source-port mark, rewrites only
   the Ethernet destination to the bridge MAC, and requests host delivery.
3. One policy route plus IP-prerouting TPROXY delivers the guest's original
   plaintext connection to leg F. `getsockname()` recovers
   `10.95.0.3:9000`.
4. `ServiceBackendsResolve` returns the healthy mesh backend. The real outbound
   `HostMtlsEnforcement` dials leg B.
5. A scratch output classifier marks host leg-B traffic while excluding the
   production leg-S mark `0x2`; the same policy route delivers leg B to the
   inbound TPROXY listener as leg C.
6. The real inbound `HostMtlsEnforcement` terminates leg C and uses its
   production marked leg-S dial to the server guest.
7. A minimal scratch UDP responder binds the shared bridge address
   `10.95.0.1:53`. The guest rootfs—not any host `/etc/netns` path—names that
   resolver and receives `10.95.0.3` for `peer.mesh`.

The scratch mechanism uses nftables/TAP/bridge policy only. No production API
surface was added and no production file under `crates/`, `xtask/`, `examples/`,
or `verification/` was edited.

### Canonical execution and retained evidence

The final command was:

```sh
OVERDRIVE_METAL_KERNEL=/var/tmp/spike-increment-n/kernel \
OVERDRIVE_METAL_ROOTFS=/var/tmp/spike-increment-n/rootfs.ext4 \
OVERDRIVE_METAL_SCENARIO=netns-density-295-part-b-attempt-15 \
cargo xtask metal run -- \
  bash spike-scratch/increment-v-part-b-production-core-netns-density-295-20260916T002721Z/run.sh
```

The canonical runner acquired the exclusive metal lease and passed its native
`x86_64`, non-virtualized KVM preflight. Metadata: kernel
`7.0.0-29-generic`, Cloud Hypervisor `v53.0`. Kernel version remains metadata,
not a gate, per the user instruction already recorded in Part A.

The local command capture is
[`capture-attempt-15.log`](../../../../spike-scratch/increment-v-part-b-production-core-netns-density-295-20260916T002721Z/capture-attempt-15.log).
The copied raw evidence directory is
[`evidence-attempt-15/`](../../../../spike-scratch/increment-v-part-b-production-core-netns-density-295-20260916T002721Z/evidence-attempt-15/),
including all three pcaps, per-thread strace files, `ss-ktls.log`, both guest
consoles, host log, cgroup proof, topology snapshots/diffs, cleanup complements,
capture statistics, and the remote transcript. Disposable ext4 build images
were not retained in the repository evidence.

Selected actual output:

```text
PRODUCTION_IDENTITY_HELD client=spiffe://overdrive.local/workload/gh295b-client-workload/alloc/gh295b-client server=spiffe://overdrive.local/workload/peer/alloc/gh295b-server held_count=2
PRODUCTION_RESOLVER_PROBE_OK
PRODUCTION_MTLS_KTLS_SPLICE_PROBE_OK
GUEST DNS RESOLVED peer.mesh=10.95.0.3:9000
PRODUCTION_RESOLVER_MESH orig_dst=10.95.0.3:9000 backend=10.95.0.3:9000 expected_svid=None
PRODUCTION_INBOUND_KTLS_SPLICE_ESTABLISHED id=gh295b-server#1
PRODUCTION_OUTBOUND_KTLS_SPLICE_ESTABLISHED id=gh295b-client#0
PRODUCTION_MTLS_BOTH_ESTABLISHED
GUEST STEADY ROUNDTRIP SUCCESS request_bytes=40 response_bytes=41 elapsed_seconds=0.096905
WIRE_SCAN ('10.95.0.1', 44160, '10.95.0.3', 9000) stream_bytes=1403 tls_records={20: 1, 22: 1, 23: 3} application_data_0x17=3 plaintext=0 gaps=0
WIRE_SCAN ('10.95.0.3', 9000, '10.95.0.1', 44160) stream_bytes=1354 tls_records={20: 1, 22: 1, 23: 3} application_data_0x17=3 plaintext=0 gaps=0
SHARED_L2 source_tap_steady_request=1 peer_tap_agent_steady_request=1 peer_tap_steady_response=1 guest_to_guest_bypass_packets=0
ZERO_COPY_STRACE successful_splice_syscalls=14 steady_request_host_write_hits=0 steady_response_host_write_hits=0
KTLS_SS bidirectional_tls13_socket_records=2
VERDICT=WORKS: actual production HostMtlsEnforcement executed TLS1.3 kTLS TX/RX plus zero-copy splice in both directions across two real shared-bridge microVM TAPs; no netns/veth/NetSlot/host_veth or /30 final-path dependency; direct L2 bypass absent
```

The zero-copy capture began only after
`PRODUCTION_MTLS_KTLS_SPLICE_PROBE_OK` and `HOST READY`; its 14 successful
`splice(2)` calls therefore belong to the real guest journey, not the adapter's
loopback Earned-Trust sentinel. The steady-state request (40 bytes) and response
(41 bytes) appear on the guest-facing TAPs but appear zero times in any traced
host `write`/`writev`/`sendto`/`sendmsg` buffer. The actual traces show the
request/response lengths moving socket→pipe→socket through the production pump
threads.

### Wire confidentiality and direct-bypass proof

The scanner reassembled both captured leg-B↔leg-C TCP directions, rejected
gaps and conflicting retransmissions, consumed every byte as complete TLS
records, required application-data record type `0x17` in both directions, and
searched for the warmup plus steady-state request/response markers. Both streams
had zero marker hits and zero gaps. Capture totals were loopback 17/34,
source TAP 18/18, peer TAP 13/13, all with zero kernel drops.

The guest was on the same `/24` as the peer and resolved the peer's real
`10.95.0.3` address, so its original L2 traffic was naturally addressed toward
the peer. The source-TAP capture contains the steady plaintext request. The
peer-TAP capture contains only the agent's decrypted leg-S request and the
server's response. It contains zero packet from guest A (`10.95.0.2`) directly
to guest B (`10.95.0.3`). The bridge and IP interception counters were nonzero
(8 guest-A TCP packets; 10 leg-B packets), independently proving the two catch
points fired.

### Per-VM cgroup proof

While both VMMs were live, the production cgroup owner reported:

```text
CGROUP_PROOF alloc=gh295b-server pid=2829925 scope=/sys/fs/cgroup/overdrive.slice/workloads.slice/gh295b-server.scope cgroup_procs=2829925, exe=/usr/local/bin/cloud-hypervisor
CGROUP_PROOF alloc=gh295b-client pid=2829969 scope=/sys/fs/cgroup/overdrive.slice/workloads.slice/gh295b-client.scope cgroup_procs=2829969, exe=/usr/local/bin/cloud-hypervisor
```

`/proc/<pid>/cgroup` independently named the same two scopes. These were the
real Cloud Hypervisor PIDs, not timeout wrappers. `CgroupManager::cgroup_kill`
and `remove_workload_scope` removed both at cleanup.

### Topology absence and cleanup

Runtime inventory and source inspection established:

- the Part-B resources were exactly `br295b`, `tap295ba`, `tap295bb`, one
  `10.95.0.0/24` connected route, the scoped nft tables/rule/route, and two
  per-VM cgroups;
- scratch Rust/Cargo source contains no `NetSlot` or `host_veth` and invokes no
  netns/veth mechanism;
- `ip netns` and veth before/after files are byte-identical (`0`-byte diffs);
- `/etc/netns` before/after is byte-identical (`0`-byte diff);
- the route diff contains only unrelated IPv6 RA expiry-counter movement; the
  explicit `lookup 295` rule and table-295 route cleanup complements are
  `ABSENT`;
- bridge, both TAPs, both nft tables, policy rule, policy route, and both cgroup
  scopes are all recorded `ABSENT` after cleanup.

Unrelated host netns/veth/routes were observed and preserved; no broad sweep or
cleanup touched them.

### Attempts retained append-only

| Attempt | Observation | Disposition |
|---|---|---|
| 09 | Existing BPF object satisfied existence but failed the dataplane crate's mtime freshness check during the linked host build. | Preserved first Part-B failure. The probe does not execute the embedded BPF object, so later builds used the documented absolute `OVERDRIVE_BPF_OBJECT` override; actual mTLS code remained the linked production module. |
| 10 | Calling `cargo xtask bpf-build` on the metal target tried to invoke Lima, which is intentionally absent there. | Rejected that target-inappropriate build path; used the existing object only to satisfy the unrelated link-time include. |
| 11 | Scratch compile exposed the non-root `ServiceId` module path and an ambiguous `SocketAddrV4` parse. | Corrected scratch imports/type annotation; no production surface changed. |
| 12 | Host startup rejected an empty-path CA subject. | Production evidence identified the correct canonical CA subject `spiffe://overdrive.local/overdrive/ca`; corrected scratch input. Cleanup completed. |
| 13 | A shell quoting error stopped after topology setup. | Syntax corrected and `bash -n` re-run; exact owned resources and cgroups cleaned, with zero netns/veth diffs. |
| 14 | Both intended server/client images booted the client role because `/proc/cmdline` was read before mounting procfs. The client then received `EHOSTUNREACH`; this was a guest-fixture role bug, not network-mechanism falsification. | Original consoles and host logs were recovered in `recovery-attempt-14.log`. Explicit PID recovery plus production cgroup cleanup removed the owned state. Procfs mount was moved before the role read. |
| 15 | Full production-core journey passed every oracle and cleanup complement. | **WORKS**. |

### Timing, wrong assumptions, and edge cases

- Final run: `15.533814 s`, including cached Rust builds, fresh ext4 image
  construction, two real microVM boots, the journey, capture analysis, and
  cleanup. The guest DNS + warmup + steady exchange measured `96.905 ms`.
  These are point measurements, not a benchmark.
- Part-B execution started at 00:35:57 UTC and the successful run completed at
  00:46:13 UTC; reporting and evidence retrieval followed. It remained well
  inside the one-hour spike maximum.
- A production crate dependency can require an unrelated embedded BPF object at
  build time even when the exercised module is mTLS-only. On metal, `xtask
  bpf-build` is not the usable refresh path because that command delegates to
  Lima. This was a build-system edge case, not runtime evidence.
- The actual `ServiceBackendsResolve` works unchanged against a backend on a
  shared bridge. It is topology-neutral. The `IdentityMgr`, CA/SVID, and
  `HostMtlsEnforcement` are likewise topology-neutral.
- The current VMM network attachment and DNS responder are not topology-neutral:
  the former carries a netns, while the latter's fallback constructor requires
  `NetSlotAllocator`. Part B omitted both obsolete mechanism dependencies and
  supplied the minimum scratch TAP launch/shared-bridge DNS home.
- Existing node-global per-workload networking objects make a global
  “no netns/veth exists” assertion dishonest. Before/after complement evidence
  is the correct oracle: the successful path neither used nor changed them.

### Part-B artifact hashes

```text
host.rs        07ada8b445864e9118d5d18ec64e8dff53de15b1b9c9866e388b891a1cba8833
guest.rs       0a4a8f5a36f72b17a781398e0976d13a254870b0d08dfb429308a46e87dcb8e4
run.sh         5eb21193f44f424e8fb907c6e3bd2eea0f593533dffb006b4151a3f8bc551afe
Cargo.toml      4165f7e2d0d02abff862371b4fea56c7fb930178bf1675786595d585703642ba
lo.pcap         799c5f7b8bacad66f09a4a2713674991de142dccb4fb68711834d3142a322c9c
tap295ba.pcap   c69c7773d2675f61eeb94281cf95e85566b72bf3991e2df8a7861e6a270ec673
tap295bb.pcap   92bc8e376059d0b3024a221cdc838ac9cb9cac5b508d263c5079ae990d462c1a
transcript.log  0058537351620d88111042dbd8ca3fc5461094cdfabd0b00581d60f0164a8a99
```

Part B therefore validates the assigned bounded same-node mechanism with the
actual production enforcement core and supporting identity/resolver/cgroup
components. It does not establish cross-host routing, target density,
throughput, or a final production API shape.

---

## Part C — TCX production-core proof

### Part-C verdict: WORKS

The assigned TCX assumption works on the configured native metal target at
source commit `cadfd1aea827afc14146b762dfb1207f0a6db9e7`. A Rust/aya-rs
`BPF_PROG_TYPE_SCHED_CLS` endpoint classifier attached through TCX ingress to
each real microVM TAP replaced Part B's nftables bridge-family classifier. The
only nftables table owned by the final run was `table ip gh295c`, limited to
IP-prerouting/output TPROXY and shared-gateway DNS observation.

Two real Cloud Hypervisor microVMs on `tap295ca` and `tap295cb`, attached to
one `br295c` and one `10.95.0.0/24`, completed the credential-free plaintext
DNS plus two-phase byte-distinct request/response journey through the same
actual production identity, resolver, `HostMtlsEnforcement`, kTLS, splice, and
cgroup components used in Part B. Both captured leg-B↔leg-C directions were
complete TLS streams with application-data records, zero application
cleartext, and zero gaps. No direct guest-A→guest-B packet reached the peer
TAP.

The TCX program also failed closed on the endpoint states assigned by this
probe: an absent endpoint-map registration, a wrong source MAC, a wrong source
IPv4 address, and non-TCP direct guest-to-guest traffic each incremented its
own counter. None of the four injected frames reached loopback or the peer
TAP. The two TCX links and both maps remained live after the original aya
loader process exited, were independently adopted from bpffs, and were
explicitly unpinned/detached during cleanup.

Phase 1 only. Part C makes no promotion choice, architecture decision,
production edit, commit, or `wave-decisions.md` change.

### Classifier program, maps, and exact verdicts

The scratch classifier is
[`bpf/main.rs`](../../../../spike-scratch/increment-w-part-c-tcx-netns-density-295-20260916T014247Z/bpf/main.rs).
It is a `#![no_std]`, `#![no_main]` aya-ebpf Rust binary; no C, clang BPF
source, or userspace packet-forwarding loop exists.

The one `#[classifier]` program, `gh295c_endpoint`, runs at TCX ingress and:

1. keys `ENDPOINTS` by the real ingress ifindex;
2. drops before bridge delivery when the ifindex is unregistered;
3. validates the source Ethernet MAC and IPv4/ARP sender address against the
   registered endpoint;
4. allows validated host/gateway-destined traffic unchanged;
5. allows validated ARP so an on-subnet peer MAC can be resolved;
6. drops validated non-TCP traffic whose destination MAC is not the bridge;
7. for validated TCP addressed toward a peer MAC, rewrites only Ethernet
   destination to `02:00:00:95:00:01`, sets skb mark `0x295a`, changes packet
   type to `PACKET_HOST`, and returns `TC_ACT_OK` so the bridge hands the
   original IPv4 destination and TCP port to host IP prerouting.

The program has two maps:

| Map | Shape | Final measured metadata |
|---|---|---|
| `ENDPOINTS` | `HashMap<ifindex, Endpoint>` where `Endpoint` is expected IPv4, expected MAC, and bridge MAC | key 4 B, value 20 B, 16 maximum entries in the bounded probe, `3,840` B memlock |
| `COUNTERS` | eight-slot `Array<u64>` | key 4 B, value 8 B, 8 entries, `368` B memlock |

The loaded program reported `296` verified instructions, `2,640` translated
bytes, `1,468` JITed bytes, and `4,096` B memlock. This is one kernel/toolchain
observation, not a cross-kernel budget baseline.

### TCX link lifecycle proof

The aya 0.13.1 loader explicitly used
`TcAttachOptions::TcxOrder(LinkOrder::first())`; it did not use or fall back to
legacy clsact/netlink attachment. It attached one link to each TAP and pinned
the links plus maps under `/sys/fs/bpf/gh295c`:

```text
TCX_PROGRAM_LOADED id=30155 verified_insns=Some(296) memlock_bytes=4096 elapsed_ms=50.811
TCX_LINK_PINNED iface=tap295ca ifindex=57064 pin=/sys/fs/bpf/gh295c/link-tap295ca attach_ms=7.624
TCX_LINK_PINNED iface=tap295cb ifindex=57065 pin=/sys/fs/bpf/gh295c/link-tap295cb attach_ms=7.943
TCX_LOADER_EXITING links_and_maps_pinned=true
```

After that process exited, a separate `tcx-loader inspect` process opened both
pins with `PinnedLink::from_pin`, queried the kernel TCX multi-program API, and
found one program on each interface at revision 2:

```text
TCX_LINK_ADOPTED pin=/sys/fs/bpf/gh295c/link-tap295ca live=true
TCX_LINK_ADOPTED pin=/sys/fs/bpf/gh295c/link-tap295cb live=true
TCX_QUERY iface=tap295ca revision=2 programs=1
TCX_QUERY_PROGRAM iface=tap295ca id=30155 name=gh295c_endpoint verified_insns=Some(296) memlock_bytes=4096
TCX_QUERY iface=tap295cb revision=2 programs=1
TCX_QUERY_PROGRAM iface=tap295cb id=30155 name=gh295c_endpoint verified_insns=Some(296) memlock_bytes=4096
```

`bpftool link show pinned` independently rendered both as link type `tcx`,
attach type `tcx_ingress`, targeting the exact owned TAP ifindices. Cleanup
adopted both pinned links again, removed each pin, closed the returned FDs, and
queried zero TCX programs on both TAPs before the TAPs were deleted.

### Security-negative proof

The negative population ran against the real attached program after the
original loader exited:

- before endpoint registration, a correctly shaped source frame produced
  `map_miss=1`;
- after registering both real TAP endpoint facts, a wrong source MAC produced
  `spoof_mac=1`;
- the correct MAC with source `10.95.0.99` produced `spoof_ip=1`;
- a correctly sourced UDP frame addressed directly to the peer MAC produced
  `direct_bypass_drop=1`.

The counters immediately after those four single-frame injections were:

```text
TCX_COUNTER gateway_pass=0
TCX_COUNTER intercept=0
TCX_COUNTER map_miss=1
TCX_COUNTER spoof_mac=1
TCX_COUNTER spoof_ip=1
TCX_COUNTER direct_bypass_drop=1
TCX_COUNTER arp_pass=0
TCX_COUNTER malformed_drop=0
TCX_NEGATIVE_WIRE iface=lo escaped_packets=0
TCX_NEGATIVE_WIRE iface=tap295cb escaped_packets=0
```

The source-TAP negative pcap contains the injected frames, providing the
capture-works positive control. The loopback and peer-TAP negative pcaps are
24-byte header-only pcaps with zero packet records. Thus the counter evidence
is paired with an external no-escape oracle; it is not program bookkeeping
alone.

After the real journey the cumulative classifier counters were:

```text
TCX_COUNTER gateway_pass=8
TCX_COUNTER intercept=8
TCX_COUNTER map_miss=1
TCX_COUNTER spoof_mac=1
TCX_COUNTER spoof_ip=1
TCX_COUNTER direct_bypass_drop=13
TCX_COUNTER arp_pass=4
TCX_COUNTER malformed_drop=0
```

The extra direct-bypass drops are fail-closed non-IPv4/non-ARP or non-TCP
peer-destined guest traffic observed during real guest boot; they do not weaken
the exact one-per-negative proof captured before boot.

### Bridge-nft boundary and valid journey

No bridge-family nft table or rule was installed. During the live TCX journey:

```text
$ nft list tables
table ip gh295c

$ nft list table bridge gh295c
Error: No such file or directory

BRIDGE_NFT_CLASSIFIER_ABSENT=true
```

The retained IP table did only what transparent socket delivery requires:

- source mark `0x295a` → leg F at `127.0.0.1:15294`;
- output leg-B mark/re-route → leg C at `127.0.0.1:15295`;
- production leg-S mark `0x2` excluded from leg-B recapture;
- UDP `:53` observed on `br295c` and delivered to the shared bridge socket.

The valid journey produced nonzero TCX `intercept=8`, IP leg-F TPROXY `8`
packets, output/leg-C TPROXY `10` packets, and shared DNS `2` packets. The
production resolver recovered and classified the unchanged original
`10.95.0.3:9000` destination.

### Production components reused and zero-copy proof

Part C includes the complete Part-B host and credential-free guest source
directly, so the following production components executed unchanged:

- `RcgenCa` + `OsEntropy` and two real workload SVIDs;
- `IdentityMgr` through `IdentityRead`;
- `ServiceBackendsResolve` through `MtlsResolve`;
- `HostMtlsEnforcement` through `MtlsEnforcement` for outbound and inbound;
- `CgroupManager`, `RealCgroupFs`, and production `CgroupPath` for each VMM.

Actual output:

```text
PRODUCTION_IDENTITY_HELD client=spiffe://overdrive.local/workload/gh295b-client-workload/alloc/gh295b-client server=spiffe://overdrive.local/workload/peer/alloc/gh295b-server held_count=2
PRODUCTION_RESOLVER_PROBE_OK
PRODUCTION_MTLS_KTLS_SPLICE_PROBE_OK
GUEST DNS RESOLVED peer.mesh=10.95.0.3:9000
PRODUCTION_RESOLVER_MESH orig_dst=10.95.0.3:9000 backend=10.95.0.3:9000 expected_svid=None
PRODUCTION_INBOUND_KTLS_SPLICE_ESTABLISHED id=gh295b-server#1
PRODUCTION_OUTBOUND_KTLS_SPLICE_ESTABLISHED id=gh295b-client#0
PRODUCTION_MTLS_BOTH_ESTABLISHED
GUEST STEADY ROUNDTRIP SUCCESS request_bytes=40 response_bytes=41 elapsed_seconds=0.091131
```

`ss` showed two live sockets each carrying `tcp-ulp-tls version: 1.3 cipher:
aes-gcm-256 rxconf: sw txconf: sw`. Strace attached only after the production
Earned-Trust probe and captured 14 successful `splice(2)` calls on the real
guest journey. The 40-byte steady request and 41-byte steady response occurred
zero times in host `write`/`writev`/`sendto`/`sendmsg` buffers.

The two reconstructed leg-B↔leg-C streams reported:

```text
WIRE_SCAN ('10.95.0.1', 38042, '10.95.0.3', 9000) stream_bytes=1404 tls_records={20: 1, 22: 1, 23: 3} application_data_0x17=3 plaintext=0 gaps=0
WIRE_SCAN ('10.95.0.3', 9000, '10.95.0.1', 38042) stream_bytes=1353 tls_records={20: 1, 22: 1, 23: 3} application_data_0x17=3 plaintext=0 gaps=0
SHARED_L2 source_tap_steady_request=1 peer_tap_agent_steady_request=1 peer_tap_steady_response=1 guest_to_guest_bypass_packets=0
ZERO_COPY_STRACE successful_splice_syscalls=14 steady_request_host_write_hits=0 steady_response_host_write_hits=0
KTLS_SS bidirectional_tls13_socket_records=2
```

All three journey pcaps reported zero kernel capture drops.

### Per-VM cgroups

The real Cloud Hypervisor PIDs, not wrapper processes, were owned by the two
production-shaped scopes:

```text
CGROUP_PROOF alloc=gh295b-server pid=2834928 scope=/sys/fs/cgroup/overdrive.slice/workloads.slice/gh295b-server.scope cgroup_procs=2834928, exe=/usr/local/bin/cloud-hypervisor
CGROUP_PROOF alloc=gh295b-client pid=2834971 scope=/sys/fs/cgroup/overdrive.slice/workloads.slice/gh295b-client.scope cgroup_procs=2834971, exe=/usr/local/bin/cloud-hypervisor
```

Both scopes were killed/removed by the production `CgroupManager` during
cleanup.

### Canonical command, evidence, and timing

Final execution:

```sh
OVERDRIVE_METAL_KERNEL=/var/tmp/spike-increment-n/kernel \
OVERDRIVE_METAL_ROOTFS=/var/tmp/spike-increment-n/rootfs.ext4 \
OVERDRIVE_METAL_SCENARIO=netns-density-295-part-c-attempt-03 \
cargo xtask metal run -- \
  bash spike-scratch/increment-w-part-c-tcx-netns-density-295-20260916T014247Z/run.sh
```

The runner acquired the canonical exclusive metal lease and passed native
`x86_64`, non-virtualized KVM preflight. Metadata: kernel
`7.0.0-29-generic`, Cloud Hypervisor `v53.0`.

Retained artifacts:

- scratch source and all append-only attempt captures:
  [`increment-w-part-c-tcx-netns-density-295-20260916T014247Z`](../../../../spike-scratch/increment-w-part-c-tcx-netns-density-295-20260916T014247Z/);
- final outer capture:
  [`capture-attempt-03.log`](../../../../spike-scratch/increment-w-part-c-tcx-netns-density-295-20260916T014247Z/capture-attempt-03.log);
- raw final evidence, including BPF ELF, bpftool JSON, pin/adoption records,
  negative and journey pcaps, strace, `ss`, consoles, counters, cgroups,
  topology complements, and transcript:
  [`evidence-attempt-03/`](../../../../spike-scratch/increment-w-part-c-tcx-netns-density-295-20260916T014247Z/evidence-attempt-03/).

Point timings:

- aya object load + verifier: `50.811 ms`;
- TCX attach/pin on `tap295ca`: `7.624 ms`;
- TCX attach/pin on `tap295cb`: `7.943 ms`;
- guest DNS + warmup + steady-state exchange: `91.131 ms`;
- complete final attempt, including clean BPF build, ext4 construction, two
  microVM boots, negatives, journey, capture analysis, and cleanup:
  `27.297022 s`.

These are individual observations, not a benchmark and not evidence for 16k
ports or a throughput distribution. Part-C work from the first 01:47:41 UTC
attempt through the successful 01:49:44 UTC cleanup remained well inside the
one-hour spike maximum.

### Attempts retained append-only

| Attempt | Observation | Disposition |
|---|---|---|
| 01 | The first userspace-loader compile exposed aya 0.13.1 API details: `FdLink` is exported under `programs::links`, and typed `HashMap`/`Array` adapters accept a `Map` variant rather than bare pinned `MapData`. A local borrow in the synthetic-frame builder also needed a separate length value. | Corrected scratch-only imports/conversions/borrow. No kernel state was created. |
| 02 | The program loaded at 296 verified instructions, both TCX links attached and survived loader exit, and independent adoption/query succeeded. `bpftool link show pinned` rendered the valid TCX link but returned exit 255 on this bpftool build, causing the pipefail runner to stop. | Preserved the failure and complete cleanup. Kept the load/adoption evidence; treated bpftool's rendered output as diagnostic and relied on aya's query/adoption plus bpftool JSON for the gate. |
| 03 | Every security, lifecycle, wire, zero-copy, cgroup, topology, and cleanup oracle passed. | **WORKS.** |

### Cleanup and topology complement

Cleanup explicitly adopted/unpinned/detached only the two owned TCX links,
removed only the two owned maps, removed the owned IP nft table/rule/route,
deleted the two TAPs and bridge, and removed the two per-VM cgroups. It did not
sweep node-global BPF or network state.

The before/after `bpftool -j` link, program, and map snapshots are pairwise
byte-identical. Netns, veth, and `/etc/netns` snapshots are also byte-identical
with zero-byte diffs. Cleanup complements recorded every owned link, nft table,
policy rule/route, bpffs directory, and cgroup as `ABSENT`.

No final-path per-workload netns, veth pair, transit/guest `/30`, `NetSlot`, or
`host_veth` was created or consulted. Unrelated pre-existing host objects were
observed and preserved.

### Wrong assumptions and precise D-295-2 implications

1. Aya 0.13.1 already has the required high-level TCX support. No raw
   `BPF_LINK_CREATE` hand-roll was necessary: explicit TCX ordering, link
   conversion, bpffs pinning, pinned-link adoption, and TCX query all worked.
2. `TcContext::change_type(PACKET_HOST)`, Ethernet destination rewrite, and
   skb mark at TAP ingress were sufficient to reproduce Part B's working
   bridge-local delivery shape. TCX did not need to redirect or copy packets
   into userspace.
3. Endpoint identity is naturally keyed by ingress ifindex, allowing separate
   observable map-miss, MAC-spoof, and IP-spoof verdicts with one shared
   program and map family.
4. Pinned TCX links remove loader-process lifetime as an attachment hazard,
   but they do not by themselves make an intentionally unpinned/detached TAP
   fail closed. The proposed Option-B text's bridge-level default-drop guard
   remains a separate architecture requirement if DESIGN selects TCX; this
   spike proved map-miss fail-closed and pin survival/adoption, not safe live
   operation after deliberate link removal.
5. The still-PROPOSED D-295-2 evidence statements that TCX is unexercised and
   that only Option A has real-metal catch-point evidence are now false. Part C
   establishes Option B as a real-metal-feasible alternative with concrete
   verifier, memory, attach, security-negative, pin-lifecycle, and production-
   core evidence.
6. Feasibility does not itself select Option B over Option A. DESIGN must now
   compare two measured functional catch mechanisms on their actual costs:
   Option B adds a 296-verified-instruction program, 4,208 B measured map
   memlock in this bounded probe, 4,096 B program memlock, and one TCX link per
   TAP; it provides per-endpoint source validation and explicit counters.
   Option A retains nft-only lifecycle and still lacks the proposed set/vmap
   scale measurement. The user must choose the revised D-295-2 after this
   evidence is incorporated and independently reviewed.

### Part-C artifact hashes

```text
Cargo.toml             97a5134d6c4ac7dfc16fb17cb758aecf793b1a82e7bd1b8fcc0aad21a807c608
host.rs                4d7efb1ec09595bc0cf9685101155c0557ac87fe3e1d4209546400f401f1a0e6
guest.rs               356a990f3995f0c0ef5877b5faaa4babd0096509f51beba387b357f96edb1f48
tcx_loader.rs          fd48ff87299b631f46cbb73defa7bec95cef70989a5a11dfdd6318c3f389dae1
run.sh                 b8418422667ff6d32fe604ee96070eff78402d9f517fbc50a02dff802f7f3eb1
bpf/Cargo.toml         6a675581c5be245f14f67c5b3fb5c801db94502d2021406bad1a367ab5c2b3c2
bpf/main.rs            4e79cd11f92612e8ab3e6f267cf8566204b650e445ae09912233e8dc8f400edc
gh295c-endpoint-bpf.o  91e889d41c6ddf9a6bd6ac3b02db014dd48f6266466c487a76078ea015516a08
lo.pcap                bf5657c396260525314574721f3c2e718e82a5a48f261313e3e0e23ad50b7305
tap295ca.pcap          376b878e87821dc9e1e9be34fe2b5f511c534ee89af061959a15421991c1c754
tap295cb.pcap          322cb93f5a41aa91066fc46b4bf1b55b023d127ad6fddf5e03095abf9c94f604
transcript.log         489734923e3ff840f022b7f54ff9b1ae0d5ee3312f44ba8ac4220704f503f053
```

Part C therefore validates the bounded same-node TCX Option-B mechanism. It
does not prove 16k scale, throughput, cross-host routing, live-link-loss
fail-closed behavior, or a final production API shape.

---

## Part D — shared listeners versus async per-allocation listeners

### Part-D verdict: WORKS

One node-shared leg-F listener plus one node-shared leg-C listener preserved
the assigned transparent-mTLS semantics on the configured native metal target
at source commit `7a00464969e98fe00764e0ab707428e8cd82a026`.

The successful real path reused Part C's shared bridge, per-TAP TCX endpoint
classifier, and IP-only nft TPROXY boundary. Two real Cloud Hypervisor
microVMs completed credential-free plaintext DNS and a byte-distinct two-phase
request/response journey through the production `RcgenCa`, `IdentityMgr`,
`ServiceBackendsResolve`, `HostMtlsEnforcement`, TLS 1.3, kTLS TX/RX, four
splice pumps, and per-VMM `CgroupManager` scopes. Exactly two transparent
listeners and two idle accept tasks served both allocations.

The shared accept owner selected an immutable allocation capability before
calling enforcement:

- leg F keyed the accepted socket's validated source guest address to
  `(AllocationId, generation, SpiffeId)`, then recovered and resolved the
  original destination;
- leg C keyed the accepted socket's recovered original destination address and
  port to the server capability;
- the connection owner rechecked that exact capability was active before
  enforcement and again before publishing the returned
  `EnforcedConnection` under the allocation owner;
- a capability retired during enforcement causes the resulting handle to be
  torn down rather than published or re-attributed.

Unknown source, unknown destination, stale predecessor capability, and a new
connection after removal all failed closed in executable socket sequences.
Controlled source-IP reuse retained the predecessor capability on the already
accepted connection and selected only the successor generation and successor
SVID on the next accepted connection. Stopping the client allocation drained
only its enforcement handle; the server handle and both shared listener tasks
remained live until their own owner shutdown.

At the proposed T1 population (`N=16,384`), a bounded real-resource comparison
created actual nonblocking `IP_TRANSPARENT` TCP listeners and Tokio accept
tasks. The shared shape used 2 listener FDs and 2 tasks. The per-allocation
shape used 32,768 listener FDs and 32,768 tasks. Its measured incremental RSS
was about 28.8 MiB versus 316 KiB for the shared listener/task layer. These are
single point observations, not a performance benchmark.

Phase 1 only. Part D makes no architecture decision, promotion choice,
production edit, commit, or `wave-decisions.md` change.

### Scratch mechanism

The Part-D source and append-only evidence live in
[`increment-x-part-d-shared-listeners-netns-density-295-20260916T101440Z`](../../../../spike-scratch/increment-x-part-d-shared-listeners-netns-density-295-20260916T101440Z/).

Its scratch-only `Registry` maintains two read paths:

```text
source guest IPv4              -> immutable Capability
original destination IPv4:port -> immutable Capability

Capability = AllocationId + generation + canonical allocation SpiffeId
```

`ConnectionOwner` maintains the active capability set and
`Capability -> Vec<EnforcedConnection>` ownership. Accept follows:

```text
accept socket
  -> recover source/original-destination facts
  -> clone immutable capability
  -> claim only if that exact generation is active
  -> call production HostMtlsEnforcement with capability.alloc
  -> publish only if the exact generation remains active
     else teardown the newly returned handle immediately
```

Allocation stop removes the exact generation from the active set, takes only
that capability's handles, and awaits production enforcement teardown. It
does not close, replace, or cancel either node-shared accept listener.

### Allocation, generation, reuse, and negative evidence

Before the real microVM journey, the same registry/owner implementation drove
real accepted TCP sockets through the transition sequence. Production
`RcgenCa` had minted SVIDs for predecessor, server, and successor, all held by
the real `IdentityMgr`.

Actual output:

```text
SHARED_NEGATIVE unknown_source=FAIL_CLOSED
SHARED_REUSE accepted_alloc=gh295d-client accepted_generation=1 successor_alloc=gh295d-successor successor_generation=2 publish=TEARDOWN_NOT_REATTRIBUTE
SHARED_SUCCESSOR_NEW_CONNECT alloc=gh295d-successor generation=2 identity=spiffe://overdrive.local/workload/gh295d-successor-workload/alloc/gh295d-successor
SHARED_NEGATIVE post_removal_new_connect=FAIL_CLOSED
SHARED_NEGATIVE unknown_destination=FAIL_CLOSED
```

The stale sequence was:

1. register predecessor generation 1 at the source address;
2. accept a real socket and clone the predecessor capability;
3. retire predecessor generation 1;
4. reassign the address to successor generation 2;
5. verify the accepted capability still names predecessor generation 1 and its
   held predecessor SVID;
6. verify publish is rejected and the socket is torn down rather than moved;
7. accept a new socket and verify it selects successor generation 2 and the
   held successor SVID;
8. remove successor generation 2 and prove a later connect fails closed.

This is the narrow capability-selection and owner-fence proof. The successor
connection did not run a second full kTLS session; it proved the exact
`Capability -> IdentityMgr::svid_for` input that the same shared accept loop
passes to production enforcement. The real generation-1 microVM journey below
independently proves that this selected capability drives the actual
`HostMtlsEnforcement` path.

### Real shared-listener journey and allocation-scoped stop

The two node-shared listeners announced:

```text
SHARED_LISTENERS_READY leg_f=127.0.0.1:15294 leg_c=127.0.0.1:15295 listener_count=2 accept_task_count=2
SHARED_LEG_F_CAPABILITY source=10.95.0.2 orig_dst=10.95.0.3:9000 alloc=gh295d-client generation=1 identity=spiffe://overdrive.local/workload/gh295d-client-workload/alloc/gh295d-client
SHARED_LEG_C_CAPABILITY orig_dst=10.95.0.3:9000 alloc=gh295d-server generation=1 identity=spiffe://overdrive.local/workload/peer/alloc/gh295d-server
SHARED_CONNECTIONS_ESTABLISHED first=F:gh295d-client:1 second=C:gh295d-server:1
```

The client guest completed:

```text
GUEST DNS RESOLVED peer.mesh=10.95.0.3:9000
GUEST STEADY ROUNDTRIP SUCCESS request_bytes=40 response_bytes=41 elapsed_seconds=0.098927
```

The owner then stopped the client capability before the server capability:

```text
SHARED_STOP_ISOLATION stopped_alloc=gh295d-client generation=1 drained_handles=1 server_handles_before=1 leg_f_listener_alive=true leg_c_listener_alive=true
SHARED_SERVER_STOP drained_handles=1
SHARED_LISTENER_OWNER_SHUTDOWN_COMPLETE
```

Thus one allocation stop neither closed a node-shared listener nor removed the
unrelated server allocation's owned handle.

### Production kTLS/splice, wire, TCX, nft, and cgroup proof

`ss` observed two sockets each carrying `tcp-ulp-tls version: 1.3`,
`rxconf: sw`, and `txconf: sw`. Strace attached after the production
Earned-Trust probe and found 14 successful `splice(2)` calls on the real guest
journey; neither steady-state plaintext marker appeared in any host
`write`/`writev`/`sendto`/`sendmsg` buffer.

The reassembled encrypted streams were:

```text
WIRE_SCAN ('10.95.0.1', 46102, '10.95.0.3', 9000) stream_bytes=1405 tls_records={20: 1, 22: 1, 23: 3} application_data_0x17=3 plaintext=0 gaps=0
WIRE_SCAN ('10.95.0.3', 9000, '10.95.0.1', 46102) stream_bytes=1354 tls_records={20: 1, 22: 1, 23: 3} application_data_0x17=3 plaintext=0 gaps=0
SHARED_L2 source_tap_steady_request=1 peer_tap_agent_steady_request=1 peer_tap_steady_response=1 guest_to_guest_bypass_packets=0
ZERO_COPY_STRACE successful_splice_syscalls=14 steady_request_host_write_hits=0 steady_response_host_write_hits=0
KTLS_SS bidirectional_tls13_socket_records=2
```

All journey pcaps reported zero kernel drops. The Part-C aya-rs classifier
remained attached through TCX ingress on both real TAPs, reporting
`intercept=8`, `gateway_pass=8`, and `arp_pass=4`. No bridge-family nft table
was installed; the owned `table ip gh295d` contained only DNS observation and
leg-F/leg-C/output TPROXY rules.

The production per-VMM cgroup owner placed the real Cloud Hypervisor PIDs:

```text
CGROUP_PROOF alloc=gh295d-server pid=2843136 scope=/sys/fs/cgroup/overdrive.slice/workloads.slice/gh295d-server.scope cgroup_procs=2843136, exe=/usr/local/bin/cloud-hypervisor
CGROUP_PROOF alloc=gh295d-client pid=2843180 scope=/sys/fs/cgroup/overdrive.slice/workloads.slice/gh295d-client.scope cgroup_procs=2843180, exe=/usr/local/bin/cloud-hypervisor
```

### T1-shaped listener/task/FD/RSS comparison

The metal preflight observed a 524,288 soft/hard file-descriptor limit after
raising the inherited soft limit to the permitted hard limit and about 60 GiB
available memory. It therefore selected the requested `N=16,384` rather than
a fallback population.

Both fresh comparison processes first built the same compact 32,768-entry
source/destination registry, then measured only the listener/task layer. Every
listener was a real nonblocking `IP_TRANSPARENT` IPv4 TCP socket; every idle
task was a real Tokio task blocked in `accept()`.

| Observation | Node-shared | Async per-allocation | Difference |
|---|---:|---:|---:|
| Allocations (`N`) | 16,384 | 16,384 | same |
| Registry entries | 32,768 | 32,768 | same |
| Listening sockets | 2 | 32,768 | 32,766 fewer shared |
| Idle accept tasks | 2 | 32,768 | 32,766 fewer shared |
| Measured FD delta | 2 | 32,768 | 32,766 fewer shared |
| Registry RSS delta | 1,440 KiB | 1,480 KiB | process-noise equivalent |
| Listener/task RSS delta | 316 KiB | 29,484 KiB | 29,168 KiB lower shared |
| Setup point time | 0.042 ms | 405.335 ms | point observation only |
| Teardown point time | 101.648 ms | 330.201 ms | includes fixed 100 ms settle |

Exact output:

```text
RESOURCE_RESULT mode=shared n=16384 registry_entries=32768 listeners=2 idle_accept_tasks=2 fd_before=10 fd_after=12 fd_delta=2 rss_start_kib=3956 rss_after_registry_kib=5396 registry_rss_delta_kib=1440 rss_after_listeners_kib=5716 listener_task_rss_delta_kib=316 setup_ms=0.042 kernel_socket_memory=not_process_attributable
RESOURCE_RESULT mode=per-allocation n=16384 registry_entries=32768 listeners=32768 idle_accept_tasks=32768 fd_before=10 fd_after=32778 fd_delta=32768 rss_start_kib=4044 rss_after_registry_kib=5524 registry_rss_delta_kib=1480 rss_after_listeners_kib=35012 listener_task_rss_delta_kib=29484 setup_ms=405.335 kernel_socket_memory=not_process_attributable
```

The registry used compact `(u64, generation)` values to keep the comparison
focused on listener/task cardinality; its RSS is a lower bound for a final
typed capability registry. Kernel socket memory was not reported because the
available `/proc/net/sockstat` counters are node-global and cannot honestly be
attributed to one comparison process. The FD and task counts are exact
structural T1 counts; RSS and timings are single-run observations, not a
benchmark.

### Local Cilium population diff

The requested local prior-art audit used
`/Users/marcus/Git/cilium/cilium` at commit
`e99150f8d8f403eca51ed82138d4ae20a265c8f3`.

| Question | Cilium fact from local source | Comparison to Overdrive |
|---|---|---|
| Endpoint TCX attachment | `pkg/datapath/loader/endpoint.go` `reloadEndpoint` attaches `FromContainer` at endpoint-device ingress and optionally `ToContainer` at egress. `pkg/datapath/loader/tcx.go` creates a TCX link, pins it in the per-endpoint bpffs link directory, updates existing pinned links, and queries by attach type. Endpoint deletion removes the links directory before the endpoint bpffs directory. | Aligns with Part C's per-TAP TCX/pinned-link lifecycle. It does not argue for per-endpoint proxy listeners. |
| Proxy listener cardinality | `pkg/proxy/proxyports/proxyports.go` has shared HTTP/TLS ingress/egress proxy-port records with `nRedirects` reference counts. `pkg/proxy/envoyproxy.go` derives listener name from the shared proxy-port name and port. `pkg/envoy/xds_server.go` `addListener` increments `listenerCount[name]` and reuses the same listener; `removeListener` deletes it only when the count reaches zero. | Cilium's endpoint datapath programs are per endpoint, while its proxy listeners are node-shared per protocol/direction. This is direct prior art for separating endpoint classification cardinality from listener cardinality. |
| Accepted/proxied connection identity | `bpf/lib/proxy.h` marks or socket-assigns traffic to the shared proxy. `bpf/lib/identity.h` defines the security-identity mark encoding/decoding, and `bpf/bpf_lxc.c` handles the proxy-egress endpoint-ID mark. `pkg/envoy/xds_server.go` configures the `cilium.bpf_metadata` listener filter with bpffs root, ipcache name, direction, and proxy ID. `pkg/fqdn/dnsproxy/proxy.go` independently demonstrates the userspace pattern: parse accepted remote IP, call `LookupEndpointByIP`, then key policy by endpoint ID and destination security identity. | Supports recovering an endpoint/security owner after a shared accept. Overdrive cannot copy the metadata shape verbatim: it must select a platform-held allocation SVID before rustls/kTLS enforcement, so Part D uses an immutable allocation capability rather than only numeric security identity. |
| IP reuse / stale generation | `pkg/fqdn/lookup/endpoint.go` resolves IP through `endpointManager.LookupIP`. `pkg/endpointmanager/manager.go` stores IP references in `endpointsAux`; `unexpose` removes endpoint and auxiliary references before `Endpoint.Delete`, and `expose` installs a new endpoint under the manager lock. No explicit allocation-generation token exists in these local agent sources. | Cilium supplies useful remove-before-delete ordering but not Overdrive's exact same-IP allocation-generation contract. Part D's immutable `(AllocationId, generation, SpiffeId)` capability and publish fence are a genuine Overdrive divergence, not copied Cilium machinery. |
| Per-endpoint teardown with shared listener | `pkg/endpoint/bpf.go` removes only obsolete endpoint redirect IDs. `pkg/proxy/proxy.go` closes that redirect implementation and releases one shared proxy-port reference. `pkg/envoy/xds_server.go` decrements listener count and preserves the listener while any redirect still references it. | Aligns with Part D's capability-scoped handle drain while shared listeners remain. Cilium delegates accepted-connection draining to Envoy; Overdrive owns concrete `EnforcedConnection` handles and must explicitly teardown kTLS/splice state. |
| Singleton DNS ownership | `pkg/fqdn/dnsproxy/proxy.go` states one `DNSProxy` singleton always runs inside `cilium-agent`; `NewDNSProxy` owns one allowed-policy map keyed by endpoint ID. `pkg/fqdn/bootstrap/dns_proxy.go` constructs it once; `fqdn_bootstrapper.go` calls `Listen`, installs one static proxy port, and takes a reference so it is never released. | Strongly aligns with already-approved D-295-6's one shared-gateway responder. Cilium's DNS proxy enforces per-endpoint DNS policy; Overdrive's responder owns mesh naming/health answers instead, so policy semantics are not interchangeable. |

Genuine divergences:

1. Cilium's shared Envoy listeners enforce L7 policy; Overdrive's shared leg-F
   and leg-C listeners must choose the correct allocation SVID before the
   production rustls handshake and then preserve kernel kTLS/splice ownership.
2. Cilium's local source exposes endpoint/security IDs and IP-manager ordering,
   but no explicit allocation generation. Overdrive requires generation-aware
   immutable capabilities because one guest address may be reused by a
   successor while predecessor connections still exist.
3. Cilium delegates listener/connection drain semantics to Envoy and xDS
   reference counting. Overdrive owns `EnforcedConnection` handles directly;
   allocation stop must fence in-flight publish and await only that
   allocation's handles.
4. Cilium uses several shared listeners by proxy type and direction. Part D's
   claim is narrower: exactly one shared TCP leg F and one shared TCP leg C for
   the current Overdrive mTLS path, not one universal listener for every future
   protocol.
5. Cilium's singleton DNS proxy is a policy-enforcing forward proxy. Overdrive's
   shared DNS responder answers its own mesh-name contract and does not inherit
   Cilium's FQDN-policy ownership.

The population diff therefore supports node-shared listener cardinality but
also identifies the generation capability and explicit kTLS-handle owner that
Overdrive must add rather than assuming Cilium's metadata is sufficient.

### Attempts retained append-only

| Attempt | Observation | Disposition |
|---|---|---|
| 01 | The inherited metal shell soft `RLIMIT_NOFILE` was 1,024. The initial fallback calculation incorrectly selected `N=8,192`; the real per-allocation comparison correctly failed at `EMFILE` after the shared measurement. | Preserved first failure. No kernel topology was created. The next attempt raised the soft limit to the permitted hard limit and derived any fallback from the actual FD ceiling. |
| 02 | The full `N=16,384` resource comparison passed. The Part-C loader was still hard-coded to `tap295ca`/`tap295cb`, while this first runner revision created `tap295da`/`tap295db`; load failed with `ENODEV` after pinning maps but before attaching links. | Preserved failure and normal network/cgroup cleanup. An exact recovery command removed only `/sys/fs/bpf/gh295d/{endpoints,counters}` and its empty directory. The final runner reused the classifier's real TAP names. |
| 03 | Resource comparison, generation/reuse negatives, two-microVM shared-listener journey, allocation-scoped stop, zero-copy/wire/TCX/cgroup proof, and cleanup all passed. | **WORKS.** |

### Canonical command, timing, evidence, and cleanup

Final execution:

```sh
OVERDRIVE_METAL_KERNEL=/var/tmp/spike-increment-n/kernel \
OVERDRIVE_METAL_ROOTFS=/var/tmp/spike-increment-n/rootfs.ext4 \
OVERDRIVE_METAL_SCENARIO=netns-density-295-part-d-attempt-03 \
cargo xtask metal run -- \
  bash spike-scratch/increment-x-part-d-shared-listeners-netns-density-295-20260916T101440Z/run.sh
```

The canonical exclusive lease and native x86_64/non-virtualized KVM preflight
passed. Substrate metadata: kernel `7.0.0-29-generic`, Cloud Hypervisor
`v53.0`.

Retained evidence:

- final outer capture:
  [`capture-attempt-03.log`](../../../../spike-scratch/increment-x-part-d-shared-listeners-netns-density-295-20260916T101440Z/capture-attempt-03.log);
- raw consoles, pcaps, strace, `ss`, TCX/bpftool snapshots, cgroup evidence,
  resource measurements, cleanup complements, and transcript:
  [`evidence-attempt-03/`](../../../../spike-scratch/increment-x-part-d-shared-listeners-netns-density-295-20260916T101440Z/evidence-attempt-03/).

Final full attempt: `22.557271 s`, including the clean BPF build, T1-shaped
resource comparison, ext4 creation, two microVM boots, capability negatives,
journey, capture analysis, and cleanup. Guest journey: `98.927 ms`. These are
point timings, not benchmark claims. Work from the first attempt at 10:19:54
UTC through final cleanup at 10:21:56 UTC remained far inside the one-hour
maximum.

Cleanup explicitly unpinned/detached only the two owned TCX links, removed the
two owned maps, IP nft table/rule/route, bridge and TAPs, and both per-VM
cgroups. Before/after BPF link/program/map JSON files are byte-identical.
Netns, veth, and `/etc/netns` diffs are zero bytes. Every owned cleanup
complement is `ABSENT`; unrelated host state was preserved.

No final-path per-workload netns, veth pair, `/30`, `NetSlot`, or `host_veth`
was created or consulted.

### Precise implication for still-unapproved D-295-5 / ADR-0120

The proposed ADR-0120 premise that shared listeners lack a trustworthy way to
recover allocation identity is now experimentally closed for the bounded
same-node path. Validated source guest address plus recovered original
destination were sufficient to select immutable generation-aware capabilities
before production enforcement; Cilium independently demonstrates that
per-endpoint TCX and node-shared proxy listeners are compatible cardinality
choices.

The proposed resource consequence is also materially changed: at T1,
async-per-allocation removes the blocking-pool ceiling but still consumes
32,768 real listener FDs and tasks, while the shared shape consumes 2 plus the
registry both alternatives already need. The single-run incremental RSS
difference was about 28.5 MiB.

This evidence supports revising D-295-5 toward node-shared leg-F/leg-C
listeners with:

- source and destination capability indexes;
- immutable `(AllocationId, generation, SpiffeId)` capture at accept;
- active-generation claim before enforcement;
- publish-after-enforcement recheck with immediate teardown on retirement;
- allocation-scoped handle maps and awaited stop;
- listener lifetime owned by the node, not any allocation.

It does not itself approve that revision. DESIGN must pin the exact internal
contract, concurrency/linearization points, boot rebuild source, address-reuse
ordering, and failure projection, then obtain user approval and independent
review. A second full successor kTLS handshake, multi-connection stop race,
connection-flood behavior, cross-host routing, and long-run performance remain
unproved.

### Part-D artifact hashes

```text
Cargo.toml            c8e3e61944b7015682a1e41b25a3d980ed66a1dd0ee63db76e3bb17a7472e527
shared_host.rs        787dadeb8807a953388655857221c94cfceb4dd5b86d7bb4160f21f760206afa
run.sh                1b9dc8c63534618719cd70759c2774a5ed62a33f9a941478b229b6d5e7bdb013
gh295d-endpoint-bpf.o 91e889d41c6ddf9a6bd6ac3b02db014dd48f6266466c487a76078ea015516a08
lo.pcap               66df2e85224b346462a0550ca728e2119ff574a0830772f69399132fb27cbf51
tap295ca.pcap         34809fd66b7f0ffed0feca8d64fdad887e628b8b39cf4042ee21341be39a026e
tap295cb.pcap         e1e58fa2128bb31488a4fced121e940aaa18f939bf8f2792973142d373192fc9
transcript.log        48cefcfc1d9e9284ce20b35899499d03b1d6db346ee44df3963091434b938a45
```

Part D therefore validates the bounded node-shared listener mechanism and its
material T1 cardinality advantage. It does not select or approve the final
D-295-5 contract.
