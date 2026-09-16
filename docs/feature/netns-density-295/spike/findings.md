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
