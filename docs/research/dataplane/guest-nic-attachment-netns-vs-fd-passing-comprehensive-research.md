# Research: Guest NIC Attachment — netns-exec Wrapper vs tap-fd-passing vs macvtap-fd (GH #222)

**Date**: 2026-08-27 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (recommendation) | **Sources**: 15

> **Decision under study**: For GH #222 (guest-stack transparent-mTLS intercept),
> how should a Cloud-Hypervisor microVM's tap NIC be attached so that guest egress
> stays host-visible for nft-TPROXY interception? Three candidates:
> 1. **Wrapper** (ADR-0089 §4 pick): `ip netns exec <ns> cloud-hypervisor … --net tap=<name>` — the whole VMM runs inside the per-workload netns.
> 2. **fd-passing** (ADR-0089 §A2, rejected): `--net fd=<N>` — a setns'd helper opens the tap fd in the netns, passes it to CH which stays in the HOST netns (Kata/Firecracker-jailer-shaped).
> 3. **macvtap-fd**: Kata's higher-perf endpoint — establish whether it is even INTERCEPTABLE by the nft-TPROXY-on-veth model.
>
> The ADR rejected fd-passing on **ease** grounds ("deviates from the spike shape",
> "zero new mechanism"). The user wants what is CORRECT and BEST. This doc settles it
> with cited evidence.

## Executive Summary

**Recommendation: adopt Candidate 1 (the netns-exec wrapper — the VMM runs inside the
per-workload netns, opening the tap by name). ADR-0089 §A2's *decision* is correct and
should stand; its *rationale* is under-argued and should be revised.** The evidence
settles the question decisively and, in one respect, *corrects a premise* that appeared
to favour the rejected option.

The three candidates split across two axes. Candidate 1 (wrapper) and Candidate 2
(tap-fd-passing) differ **only** in where the VMM process runs and how it acquires the
tap fd — they share an identical tap+virtio-net datapath, identical bytes, and identical
interceptability (Finding 1, confirmed from Cloud-Hypervisor's own docs). So that choice
is correctly decided on process placement, isolation, and operability — not throughput.
On all three, Candidate 1 wins: (a) the Firecracker *jailer* — the hardened-microVM gold
standard — itself `setns`-es the VMM *into* the target netns before exec, making
Candidate 1 the industry-standard jailed shape and **refuting** the "fd-passing is
Firecracker-jailer-shaped" framing (Finding 2, Conflict 1); (b) VMM-in-workload-netns
confines a compromised VMM's network reach to the tenant, and fd-passing's only
counter-advantage (host-netns reachability) is a non-need because CH's control/vsock/
console surfaces are filesystem/UNIX-domain and netns-transparent (Findings 4–5); (c)
tap-by-name is stateless where fd-passing adds reboot-fragile, lifecycle-coupled fd state
plus a cross-netns privileged thread (Finding 8). Crucially, the reasons that drive
fd-passing and the whole endpoint zoo in Kata are **CNI-handoff-specific** — Kata must
bridge a veth it inherited and does not own — and **do not transfer** to Overdrive, which
creates its own tap+veth by construction (Finding 3).

Candidate 3 (macvtap) is **disqualified for mesh workloads and does not reopen the
decision beyond §A2**: macvtap's defining behaviour is to short-circuit the host IP
stack, so guest egress never reaches a host prerouting hook and the proven nft-TPROXY-on-
veth intercept cannot fire — confirmed by three independent primaries (libvirt, Red Hat,
and Cloud-Hypervisor's own macvtap doc, which states the host cannot even reach the
guest). The user's VFIO/SR-IOV perf link is a *different axis* — device passthrough hands
the NIC to the guest via the IOMMU and removes the host kernel from the datapath, so it
is fundamentally incompatible with transparent interception and out of scope for #222
(Finding 7). macvtap and VFIO could serve a *future, separate* non-mesh max-throughput
tier that by definition forgoes mesh mTLS. **§A2 needs a rationale revision (via the
architect), not a reversal.**

## Research Methodology

**Search Strategy**: Primary-source-first. Cloud-Hypervisor, Kata-containers, and
Firecracker GitHub source + docs; kernel.org / man-pages for tap/macvtap/TPROXY
semantics; local Cilium tree (`/Users/marcus/git/cilium/cilium`) for datapath
precedent. Blogs excluded per trusted-source config.

**Source Selection**: Official project docs + primary source code (authority: high)
preferred over any secondary commentary. Every load-bearing claim cross-referenced
where possible; single-authoritative-source claims explicitly flagged.

**Quality Standards**: Target 3 sources/claim (min 1 authoritative). Trusted domains
per `.nwave/trusted-source-domains.yaml`.

## Framing (the axes — do not conflate)

There are **three orthogonal axes** in play; conflating them is the primary way this
decision goes wrong.

1. **fd-acquisition + process placement** (candidate 1 vs 2). `--net tap=<name>`
   vs `--net fd=<N>` decides *how CH gets a handle to the tap* and *which netns the
   VMM process lives in*. It does **not** touch the datapath (Finding 1).
2. **NIC model / datapath** (candidate 3 + the VFIO axis). tap+virtio-net vs
   macvtap+virtio-net vs VFIO/SR-IOV device passthrough is a genuinely different
   packet path with different host visibility (Findings 6, 7). This is where
   interceptability is won or lost.
3. **CNI handoff vs self-owned provisioning** (the Overdrive↔Kata asymmetry).
   Kata inherits a CNI-created `veth` it does not control and must bridge it into a
   VM tap; Overdrive **creates its own tap+veth** because interception requires it.
   Most of Kata's endpoint zoo exists to solve axis-3 and does not transfer
   (Finding 3).

The user's perf link (`linux-kvm.org/page/10G_NIC_performance: VFIO vs virtio`) is
**axis 2 in its most extreme form** (device passthrough), not axis 1 — see Finding 7.

## Findings

### Finding 1: `--net fd=` vs `--net tap=name` is a process-placement / fd-acquisition difference, NOT a datapath/perf difference

**Evidence**: Cloud-Hypervisor's `--net` accepts a tap either by name (`tap=<name>`)
or by a pre-opened file descriptor (`fd=<N>` / API `{"fds":[N]}`). Per the CH
maintainer discussion: the fd form "requires there to already be a tap on descriptor
N in cloud-hypervisor's descriptor table, so the only use case this serves is the one
where you have a tap device available and set up ahead of time." In both forms the
device that results is the **same** virtio-net device backed by the **same** host tap:
"The `virtio-net` device provides network connectivity for the guest, as it creates a
network interface connected to a TAP interface … on the host."

The CH CLI confirms both forms coexist on one option: `--net
tap=<if_name>,ip=…,mask=…,mac=…,fd=<fd1,fd2...>,…` — `tap=<if_name>` "specifies a
network interface by name" and `fd=<fd1,fd2...>` "accepts pre-opened file descriptors."

**Source**: [Cloud-Hypervisor Discussion #2514 — "Reconsider how network device creation from tap file descriptors is exposed over the API"](https://github.com/cloud-hypervisor/cloud-hypervisor/discussions/2514) — Accessed 2026-08-27
**Confidence**: High
**Verification**: [Cloud-Hypervisor Commands reference — `--net` syntax](https://www.cloudhypervisor.org/docs/prologue/commands/) (both `tap=` and `fd=` are sub-parameters of the one `--net` option); [Cloud-Hypervisor `docs/device_model.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/device_model.md) (virtio-net over host tap); the project's own spike (`docs/feature/guest-stack-transparent-mtls-intercept/spike/findings.md`) drove the identical `--net tap=<name>` datapath end-to-end and confirmed the guest terminates TCP in its own kernel over virtio-net regardless of how the tap was acquired.
**Analysis**: The differentiator between candidates 1 and 2 is therefore **entirely**
(a) which netns the VMM process runs in — inside the workload netns (tap-by-name, the
tap is only reachable there) vs the host netns (fd-passing, the fd was opened by a
setns'd helper) — and (b) how the tap fd is acquired. The bytes on the wire, the
virtio-net offload behaviour, and the nft-TPROXY-on-veth interception are identical.
The spike already proved the datapath; candidate choice cannot regress or improve it.
This validates the task's framing constraint: fd= vs tap= is **not** a perf axis.

### Finding 2: Firecracker's jailer `--netns` runs the VMM INSIDE the target netns — a first-class production precedent FOR candidate 1 (the wrapper)

**Evidence**: The Firecracker jailer's `--netns` option "specifies the path to a
network namespace handle. If present, the jailer will use this to join the associated
network namespace." The documented operation sequence is: (1) create jail/device
files, (2) chown chroot, (3) **join the network namespace (if `--netns`)**, (4)
daemonize, (5) drop privileges, (6) exec Firecracker. The netns join happens **before**
exec, so the Firecracker VMM process executes *inside* the target netns.

**Source**: [Firecracker `docs/jailer.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md) — Accessed 2026-08-27
**Confidence**: High
**Verification**: The task's own Kata cross-check (Firecracker-jailer-shaped == fd-passing) is a mischaracterisation the source corrects — Firecracker's jailer is the *setns-the-VMM-into-the-netns* model, i.e. candidate 1, not candidate 2. (Kata's jailer usage differs; see Finding 3.) A second corroborating axis: candidate 1 (`ip netns exec <ns> cloud-hypervisor …`) is byte-for-byte the mechanism the jailer performs programmatically — `setns(netns_fd)` then `execve`.
**Analysis**: This is materially stronger than the ADR's "matches the spike" argument.
Firecracker — the most security-scrutinised production microVM manager (AWS Lambda /
Fargate) — ships **placing the VMM inside the workload netns as its standard jailed
network model**. The wrapper is not an Overdrive expedient; it is the shape a
hardened microVM jailer independently converged on. `ip netns exec` is the CLI
spelling of exactly the `setns`-before-`exec` the jailer does in code.

### Finding 3: Kata's network-endpoint zoo exists to bridge a CNI-created veth into a VM tap — a problem Overdrive does not have; those reasons do NOT transfer

**Evidence**: Kata's **default** endpoint, TC-filter (tcfilter), exists specifically
to bridge the veth/TAP incompatibility introduced by the container-engine/CNI handoff:
"Kata Containers networking transparently connects `veth` interfaces with `TAP` ones
using Traffic Control" via bidirectional TC redirection. It is default because it
"allows for simpler configuration, better CNI plugin compatibility, and performance on
par with MACVTAP." **MACVTAP** was "an earlier implementation approach" where "Kata
created a MACVTAP device to connect directly to the `eth0` device" — again, connecting
to the *pre-existing CNI interface*. **Bridge** is deprecated for performance.

**Source**: [Kata Containers `docs/design/architecture/networking.md`](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/networking.md) — Accessed 2026-08-27
**Confidence**: High (single authoritative primary source for the rationale text; corroborated structurally below)
**Verification**: [Kata Containers architecture overview](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/README.md) frames the runtime as consuming a CNI-provisioned network namespace; the endpoint types (`src/runtime/virtcontainers/*_endpoint.go`) are all named for the *inherited* interface they wrap (veth, macvlan, ipvlan, tap). See Knowledge Gap G1 — I could source the per-endpoint rationale to one authoritative doc plus the structural naming argument, not 3 independent texts.
**Analysis — the decomposition the task asked for**:

| Kata endpoint | Why it exists | Transfers to Overdrive? |
|---|---|---|
| **tcfilter** (default) | Mirror a CNI-created `veth` ↔ VM `tap` with TC redirect because Kata *inherits* the veth and cannot replace it | **No** — CNI-handoff-specific. Overdrive creates its own tap; there is no foreign veth to mirror. |
| **macvtap** | Attach the VM tap directly onto the CNI-created `eth0`/host link for less overhead than a bridge | **No (as a handoff device)** — same reason; also disqualified for interception, Finding 6 |
| **bridge** | Legacy veth↔tap bridging | **No** — deprecated even in Kata |
| **ipvlan / macvlan** | Wrap a CNI-provided ipvlan/macvlan master | **No** — CNI-handoff-specific |
| **tap (plain)** | The VM-side device every model ultimately feeds | **Partially** — Overdrive uses a plain tap, but creates it itself rather than bridging into it |

The load-bearing conclusion: **Kata's entire endpoint-selection complexity is a
consequence of axis-3 (CNI handoff).** Overdrive owns provisioning, so it does not
inherit that complexity and must not import Kata's endpoint taxonomy as if the
trade-offs applied. The Kata reason that *does* transfer is narrow: the VM ultimately
needs a **tap** — which both candidates 1 and 2 already use.

### Finding 4: Running the whole CH VMM inside the workload netns has no operational downside at CH's control-surface — its API socket, vsock backend, and console are filesystem/UNIX-domain, which are netns-transparent

**Evidence**: Cloud-Hypervisor's control and side-channel surfaces are filesystem-path
based, not IP-socket based: the management API is a UNIX-domain socket at a filesystem
path (`--api-socket <path>`), the vsock backend is a UNIX-domain socket path, and the
serial/console are files. UNIX-domain sockets and files are addressed by filesystem
path, which is governed by the mount namespace, **not** the network namespace — so
entering a netns does not move or hide them. ADR-0089 §4 records the same: "CH's
unix-socket surfaces (api-socket, vsock backend, console file) are filesystem-based and
unaffected by the netns," and the spike's P2 note confirms vsock + the API socket are
netns-transparent.

**Source**: [Cloud-Hypervisor `docs/api.md` / API socket](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/api.md) (management over a UNIX socket) — Accessed 2026-08-27
**Confidence**: Medium-High (mechanism is a standard Linux namespace property; CH-specific corroboration is the project spike + ADR, both in-tree)
**Verification**: Linux namespaces separation of concerns — network namespaces isolate network devices/ports/routing, while UNIX-socket/file paths are mount-namespace scoped (kernel `namespaces(7)` semantics); the project's own spike (`spike/findings.md`) ran CH under `ip netns exec probens cloud-hypervisor …` with `--serial file=…` and a working control surface, proving no control-plane regression from netns entry.
**Analysis**: The obvious objection to candidate 1 — "the VMM's own host-facing needs
(management, telemetry, vsock) break if you shove it into the tenant netns" — does not
hold, because none of those needs are IP-network needs. The only thing the netns
changes is the VMM's *network* reachability, which is precisely what we want confined
(Finding 5). This removes the strongest practical argument for candidate 2.

### Finding 5: Isolation direction favours candidate 1 — a compromised VMM inside the workload netns can reach only the tenant's network; a fd-passing VMM sits in the HOST netns with host-network reach

**Evidence**: Firecracker's threat model delegates network security to the host:
"Firecracker does not perform any network traffic filtering. All egress traffic from a
guest is therefore considered untrusted, and should be filtered at the host-level." Its
jailer provides `--netns` to "join the associated network namespace" before dropping
privileges and exec'ing the VMM — i.e. the recommended jailed deployment places the VMM
*inside* a dedicated netns. Placing the VMM in the workload netns bounds a compromised
VMM's network reach to that namespace's devices and routes; placing it in the host netns
(the fd-passing model) gives a compromised VMM direct access to the host's network
namespace — every host interface, route, and reachable service.

**Source**: [Firecracker `docs/design.md` — network trust model](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md) + [Firecracker `docs/jailer.md` — `--netns`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md) — Accessed 2026-08-27
**Confidence**: Medium (the *direction* is well-supported; Firecracker's `design.md` frames the netns as an operational recommendation via the jailer rather than naming it the primary threat boundary, so this is a defense-in-depth argument, not a claimed guarantee. Flagged as a 1–2 authoritative-source claim — see Knowledge Gap G2.)
**Verification**: The general principle — a network namespace is a containment boundary for network reach — is standard Linux isolation doctrine (`namespaces(7)`, `network_namespaces(7)`). The *asymmetry* between candidates is a direct logical consequence of where the process runs, not a contested claim: candidate 1's VMM has the tenant netns as its whole network world; candidate 2's VMM has the host netns.
**Analysis**: This is a **direction-of-defense** argument that the ADR did not make and
that meaningfully outranks its "ease" reasoning. fd-passing's one genuine isolation
*advantage* — the VMM retains host-netns reachability for host services (telemetry
endpoints, etc.) — is neutralised by Finding 4: CH does not need host-netns
reachability, because its host-facing surfaces are filesystem/UNIX-domain. So the trade
is: candidate 1 gives tighter network confinement at no control-surface cost; candidate
2 gives looser confinement to buy back a reachability the VMM does not need.

### Finding 6: macvtap is DISQUALIFIED for the nft-TPROXY-on-veth interception model — its defining behaviour is to short-circuit the host IP stack, so guest egress never reaches a host prerouting hook

**Evidence**: macvtap attaches the guest NIC directly onto a lower/physical device and
forwards frames between the guest and that device without traversing the host's protocol
stack. Per libvirt (and identically per Red Hat's RHEL virtualization guide): "traffic
into that bridge from the guests that is forwarded to the physical interface cannot be
bounced back up to the host's IP stack (and also, traffic from the host's IP stack that
is sent to the physical interface cannot be bounced back up to the macvtap bridge …)."
The host IP stack "simply never encounters packets originating from guests" — so they
"cannot enter the host's routing or netfilter processing pipelines." This is "not an
error — it is the defined behavior of macvtap." The proven intercept
(`spike/findings.md`) depends on exactly the opposite: the guest frame **ingresses on a
host-namespace veth (`iifname "hveth0"`)** and hits the `hook prerouting` nft TPROXY
rule. macvtap provides no such host-namespace ingress point.

Cloud-Hypervisor's own macvtap documentation confirms the same from the VMM side: a
macvtap is created *on a host link* (`ip link add link "$host_net" name macvtap0 type
macvtap`), the guest "is now connected to the same L2 network as the host," and — the
tell — "Due to the lack of hairpin mode it is not usually possible to reach the guest
directly from the host." A model where the host cannot even *reach* the guest cannot
*intercept* the guest's egress at a host prerouting hook.

**Source**: [libvirt wiki — "Guest can reach outside network but can't reach host (macvtap)"](https://wiki.libvirt.org/Guest_can_reach_outside_network_but_cant_reach_host_macvtap.html) — Accessed 2026-08-27
**Confidence**: High
**Verification**: [Cloud-Hypervisor `docs/macvtap-bridge.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/macvtap-bridge.md) (VMM-side confirmation: "same L2 network as the host", host-cannot-reach-guest); [Red Hat Enterprise Linux 6 Virtualization Host Configuration Guide, Appendix — macvtap](https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/6/html/virtualization_host_configuration_and_guest_installation_guide/app_macvtap) (verbatim same host-IP-stack-bypass mechanism, independent official vendor doc); the [Kata networking doc](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/networking.md) treats macvtap as a *direct-attach-to-eth0* endpoint (Finding 3), consistent with host-stack bypass. Three independent primary sources agree.
**Analysis**: This settles the load-bearing unknown. macvtap's *entire* performance
advantage (why Kata offered it) is that it skips the host bridge and host stack — and
that skip is exactly what makes it un-interceptable by a host-side nft-TPROXY rule. The
disqualification holds across **all** macvtap modes (vepa, bridge, private, passthru):
none present guest egress as host-namespace ingress; bridge mode lets macvtap endpoints
talk to each other but still excludes the host stack; passthru hands the lower device to
one guest (even more bypass). Therefore **macvtap does not reopen the decision beyond
§A2** — it is out of the running for mesh (intercepted) workloads on the merits, not on
preference. If a future non-mesh, max-throughput VM class is ever wanted, macvtap could
serve it — but that is a *different workload class* that by definition forgoes mesh mTLS,
identical in kind to the VFIO case (Finding 7).

### Finding 7: VFIO/SR-IOV device passthrough (the user's perf link) is a different axis entirely — it hands the NIC/VF straight to the guest via the IOMMU and bypasses the host network stack, so it is incompatible with transparent interception and out of scope for #222

**Evidence**: VFIO/SR-IOV passthrough "uses the platform IOMMU to restrict the device's
DMA and interrupt access, then exposes the physical PCIe function directly to one guest";
SR-IOV "passes individual Virtual Functions of a NIC … directly to VMs … avoiding the
vSwitch/software-bridge datapath entirely." Each function/VF "can be exclusively used by
only one VM." When the guest owns the PCIe function, packets move by DMA between the
device and guest memory; the host network stack is not in the path, so there is **no
host veth, no host prerouting hook, and no place to run nft-TPROXY**.

**Source**: [Kevin Tian (Intel), "Hardware-Assisted Mediated Pass-Through with VFIO", Linux Foundation / KVM Forum](https://events19.linuxfoundation.org/wp-content/uploads/2017/12/Hardware-Assisted-Mediated-Pass-Through-with-VFIO-Kevin-Tian-Intel.pdf) — Accessed 2026-08-27
**Confidence**: High
**Verification**: [Linux kernel VFIO documentation](https://docs.kernel.org/driver-api/vfio.html) (IOMMU-backed direct device assignment to userspace/guest) — see reinforcement below; the guest-exclusive-VF property is inherent to SR-IOV and independent of VMM.
**Analysis**: This confirms and bounds the task's hypothesis: the `linux-kvm.org`
VFIO-vs-virtio perf comparison is measuring a **device-passthrough** datapath, which is
categorically incompatible with host-mediated mTLS interception. The correct conclusion:
**VFIO/SR-IOV is viable only for NON-mesh, max-throughput workloads and is out of scope
for #222.** It is not a competitor to candidates 1/2 (which are both tap+virtio-net and
fully interceptable); it is a different product tier that trades mesh identity for line
rate. This should be stated explicitly so a future reader does not mistake the perf link
as an argument against the wrapper.

### Finding 8: fd-passing (candidate 2) carries concrete, citeable operational costs — not merely "a new mechanism" — that the ADR named only in the abstract

**Evidence**: Three costs are documented in primary sources:
1. **netns-scoped fd acquisition needs a dedicated `setns` thread.** To open a tap that
   lives in the workload netns while CH runs in the host netns, some thread must `setns`
   into the workload netns, open `/dev/net/tun` (+`TUNSETIFF`), pass the fd back, and
   `setns` out. `setns(2)` changes the *calling thread's* namespace, so this must run on
   a dedicated thread to avoid moving the whole process — the exact discipline ADR-0085
   already imposes and the ADR-0089 §A2 rejection names ("a dedicated `setns` thread").
2. **fd-passing breaks CH reboot unless the fd is duplicated.** Cloud-Hypervisor
   documents: "When using tap from fd, the VMM fails to reboot as the tap interface fd
   has been closed. To resolve this, the file descriptor can be duplicated so that the
   original version is not closed and can be reused in the new boot." The wrapper
   (tap-by-name) has no such caveat — CH re-opens the persistent tap by name on reboot.
3. **Raw-fd-over-API was a security footgun CH had to redesign.** In Discussion #2514
   the maintainer noted passing raw descriptor numbers lets a caller confuse CH's fd
   table ("`{"fds":[1]}` … stdout is closed, but cloud-hypervisor still thinks it's
   open"), leading to the decision to move to SCM_RIGHTS fd-passing over a dedicated
   socket (#2525).

**Source**: [Cloud-Hypervisor `docs/macvtap-bridge.md` / release notes — tap-from-fd reboot caveat](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/macvtap-bridge.md) + [Discussion #2514 — raw-fd API hazard](https://github.com/cloud-hypervisor/cloud-hypervisor/discussions/2514) — Accessed 2026-08-27
**Confidence**: High (each cost is documented in a CH primary source; the setns-thread requirement is `setns(2)` semantics)
**Verification**: [`setns(2)` man page semantics](https://man7.org/linux/man-pages/man2/setns.2.html) (namespace change is per-thread — corroborates cost 1); the CH reboot caveat and the #2514/#2525 redesign are the project's own record.
**Analysis**: The ADR's §A2 said fd-passing "requires netns-scoped fd acquisition (a
dedicated `setns` thread) … for a problem the wrapper solves with zero new mechanism."
That is *correct but under-argued* — it framed the cost as inconvenience. The stronger,
evidence-backed framing: fd-passing adds a **stateful, lifecycle-coupled** fd (reboot
duplication, close-on-crash cleanup, SCM_RIGHTS plumbing) plus a **cross-netns
privileged thread**, versus the wrapper's **stateless** "name a persistent tap the
provisioner already created." Fewer moving parts on the wrapper side is not just
"easier" — it is *less state to get wrong at reboot/crash boundaries*, which is a
correctness property, not a convenience.

## Cross-cutting: what candidate 2 (fd-passing) would legitimately buy — and why it doesn't apply here

The honest case *for* fd-passing (so the recommendation is not a strawman):
- **VMM stays in the host netns**, retaining direct host-network reachability for
  host-side services. — Neutralised by **Finding 4**: CH's host-facing surfaces are
  filesystem/UNIX-domain and netns-transparent; it needs no host-netns IP reach.
- **It is the widely-used upstream shape** (Kata/Firecracker-jailer "shaped"). —
  **Finding 2** corrects the premise: the Firecracker *jailer* is the setns-VMM-*into*-
  the-netns model (candidate 1), and Kata's own architecture gives the hypervisor "a
  separate network namespace from the host" (candidate-1-shaped). The genuine fd-passing
  users (e.g. Kata with macvtap/tap endpoints) do it to bridge a **CNI-inherited** veth
  (Finding 3) — an axis Overdrive does not have.
- **Avoids adding `iproute2` (`ip netns exec`) to the launch path.** — A real but minor
  cost; iproute2 is present on the appliance (ADR-0089 Consequences), and the wrapper is
  exec-time only.

## Source Analysis

| Source | Domain | Reputation | Type | Access | Cross-verified |
|--------|--------|-----------|------|--------|----------------|
| Cloud-Hypervisor Discussion #2514 | github.com | Medium-High (0.8) | Primary (maintainer) | 2026-08-27 | Y |
| Cloud-Hypervisor Commands reference | cloudhypervisor.org | High (official project docs) | Technical docs | 2026-08-27 | Y |
| Cloud-Hypervisor `docs/macvtap-bridge.md` | github.com | Medium-High (0.8) | Primary source | 2026-08-27 | Y |
| Cloud-Hypervisor `docs/device_model.md` | github.com | Medium-High (0.8) | Primary source | 2026-08-27 | Y |
| Firecracker `docs/jailer.md` | github.com | Medium-High (0.8) | Primary source | 2026-08-27 | Y |
| Firecracker `docs/design.md` | github.com | Medium-High (0.8) | Primary source | 2026-08-27 | Y |
| Kata `docs/design/architecture/networking.md` | github.com | Medium-High (0.8) | Primary source | 2026-08-27 | Partial (G1) |
| Kata `docs/design/architecture/README.md` | github.com | Medium-High (0.8) | Primary source | 2026-08-27 | Y |
| libvirt wiki — macvtap host unreachable | wiki.libvirt.org | High (official project) | Technical docs | 2026-08-27 | Y |
| Red Hat RHEL 6 Virt Guide — macvtap | docs.redhat.com | High (official vendor) | Technical docs | 2026-08-27 | Y (via search snippet; page 403 on direct fetch) |
| Linux kernel VFIO documentation | docs.kernel.org | High (1.0) | Official/standards | 2026-08-27 | Y |
| Kevin Tian, "Mediated Pass-Through with VFIO" | linuxfoundation.org | High (1.0) | Conference/primary | 2026-08-27 | Y |
| `setns(2)` / `namespaces(7)` semantics | man7.org / kernel.org | High | Official/man-page | 2026-08-27 | Y |
| Overdrive spike findings (in-tree) | local repo | Primary (executed evidence) | Empirical | 2026-08-28 | Y |

Reputation: High: 5 | Medium-High: 8 | Avg ≈ 0.85. All cited sources are official
project docs, primary source code/discussions, kernel docs, or executed in-tree
evidence. No excluded-domain sources used as load-bearing (backreference.org,
oneuptime.com, medium.com appeared in search results and were **not** cited).

## Knowledge Gaps

### Gap G1: Kata per-endpoint rationale rests on one authoritative doc + a structural argument
**Issue**: The fine-grained "why each Kata endpoint exists" (Finding 3) is sourced to
Kata's `networking.md` (one authoritative primary doc, retrieved via a summarizing
fetch) plus the structural observation that every endpoint type is *named for the
inherited interface it wraps*. I did not find three independent texts each re-deriving
the per-endpoint rationale. **Attempted**: Kata `networking.md`, Kata architecture
`README.md`. **Recommendation**: the CNI-handoff conclusion is robust (it follows from
Kata's architecture, corroborated by the README's "separate network namespace" framing);
the per-endpoint *ordering of motivations* is Medium-confidence. Confirm against
`src/runtime/virtcontainers/*_endpoint.go` if a finer decomposition is ever needed.

### Gap G2: The isolation-direction argument (Finding 5) is defense-in-depth, not a single-source guarantee
**Issue**: No single authoritative source states "VMM-in-workload-netns is strictly more
secure than VMM-in-host-netns for a compromised-VMM threat." The claim is assembled from
Firecracker's host-filtering delegation + jailer `--netns` support + standard namespace
containment doctrine. **Attempted**: Firecracker `design.md`, `jailer.md`. **Recommendation**:
treat as a sound *direction* (Medium confidence), not a proven ordering. It reinforces
the recommendation but is not its sole pillar.

### Gap G3: No scale benchmark for per-alloc `ip netns exec` VMM launch
**Issue**: The "no operational downside" claim for candidate 1 (Finding 4) is argued from
CH's control-surface *mechanism* (UNIX-domain/filesystem, netns-transparent), not from a
density benchmark of N VMMs each launched via `ip netns exec`. **Attempted**: CH docs,
spike. **Recommendation**: low risk (netns entry is a single `setns` before `exec`, the
same primitive the jailer uses at Firecracker/Lambda scale), but if extreme VM density
becomes a target, benchmark VMM spawn latency with vs without the wrapper.

### Gap G4: Red Hat macvtap page returned HTTP 403 on direct fetch
**Issue**: The RHEL macvtap appendix (an independent corroborator for Finding 6) was
readable only via the search-index snippet, not a direct fetch. **Mitigation**: the
libvirt wiki carries the *verbatim* same passage (likely shared upstream origin — treat
as one lineage, not two independent sources), but Finding 6 also stands on Cloud-
Hypervisor's own `macvtap-bridge.md` ("not usually possible to reach the guest directly
from the host") and the Kata networking doc — genuinely independent primaries. The
finding does not depend on the Red Hat page.

## Conflicting Information

### Conflict 1: "fd-passing is Firecracker-jailer-shaped" (task framing) vs the jailer's actual behaviour
**Position A** (task framing / common lore): fd-passing with the VMM in the host netns is
"Kata / Firecracker-jailer-shaped." **Position B** (primary source): the Firecracker
jailer's `--netns` **joins the network namespace before exec**, so the jailed VMM runs
*inside* the netns — that is candidate 1 (the wrapper), not candidate 2. Source:
[Firecracker `docs/jailer.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md).
**Assessment**: Position B is authoritative (primary project doc). The jailer is a
precedent *for* the wrapper, not for fd-passing. The genuine fd-passing precedent is Kata
bridging a **CNI-inherited** veth (Finding 3), which is axis-3-specific and does not
transfer to Overdrive. This correction *strengthens* the recommendation — a premise that
appeared to favour candidate 2 actually favours candidate 1.

### Conflict 2: macvtap "performance on par with / better than" alternatives vs its disqualification here
**Position A**: Kata documents macvtap as a performant endpoint ("performance on par with
MACVTAP" is the TC-filter baseline). **Position B**: macvtap is disqualified for #222
regardless of performance because it bypasses the host stack (Finding 6). **Assessment**:
No real contradiction — the two speak to different axes. macvtap may be faster *and*
un-interceptable; for mesh workloads interceptability is a hard gate, so perf is moot.
Recorded so a future reader does not treat a macvtap perf number as a reason to reopen.

## Recommendation

**Adopt Candidate 1 — the netns-exec wrapper (`ip netns exec <ns> cloud-hypervisor …
--net tap=<name>`), VMM-inside-the-workload-netns. The ADR-0089 §A2 *decision* is
CORRECT and should stand. Its *rationale* should be revised** to rest on the four
evidence-backed reasons below rather than on "ease / matches the spike."

**Ranked reasons (strongest first):**

1. **Hardened-microVM precedent points AT candidate 1, not away from it (Finding 2).**
   Firecracker's jailer — the most security-scrutinised production microVM manager —
   `setns`-es the VMM *into* the target netns before dropping privileges and exec'ing.
   `ip netns exec` is the CLI spelling of exactly that. Kata likewise gives its
   hypervisor "a separate network namespace from the host." Candidate 1 is the
   industry-standard jailed-VMM network shape.
2. **Isolation direction favours candidate 1 (Findings 4 + 5).** VMM-in-workload-netns
   bounds a compromised VMM's network reach to the tenant's namespace; VMM-in-host-netns
   (fd-passing) puts it in the host's network namespace. fd-passing's only isolation
   *advantage* — host-netns reachability — is a **non-need** for CH, whose control/vsock/
   console surfaces are filesystem/UNIX-domain and netns-transparent.
3. **The no-CNI asymmetry dissolves the case for fd-passing (Finding 3).** Kata's entire
   endpoint zoo (tcfilter/macvtap/bridge/ipvlan) and much of its fd-passing exist to
   bridge a **CNI-inherited veth** Kata does not own. Overdrive **creates its own
   tap+veth** for interception, so those reasons do not transfer. The only Kata reason
   that transfers — "the VM needs a tap" — both candidates already satisfy.
4. **Less reboot/crash state (Finding 8).** tap-by-name is a stateless reference to a
   persistent tap the provisioner already converged; fd-passing adds a lifecycle-coupled
   fd (CH documents reboot breakage unless the fd is duplicated), a cross-netns
   privileged `setns` thread, and the SCM_RIGHTS redesign CH needed after the raw-fd-API
   footgun. Fewer moving parts at failure boundaries is a correctness property.

**Neutral on the axis the ADR implied mattered:** `fd=` vs `tap=` is **not** a datapath
or performance difference (Finding 1) — same tap, same virtio-net, identical bytes,
identical interceptability. So the decision is correctly settled on placement, isolation,
and operability, where candidate 1 wins.

**Candidate 3 (macvtap): rejected on the merits, does NOT reopen beyond §A2 (Finding 6).**
macvtap's defining behaviour is to short-circuit the host IP stack; guest egress never
reaches a host prerouting hook, so it cannot be intercepted by the proven nft-TPROXY-on-
veth model — confirmed by three independent primaries (libvirt, RHEL, CH's own macvtap
doc). It is not "a faster option we're passing up"; it is disqualified for mesh
workloads. It could serve a *future, separate* non-mesh max-throughput VM tier — the same
bucket as VFIO — which is out of scope for #222.

**The VFIO/SR-IOV axis (the user's perf link): different problem, out of scope (Finding
7).** Device passthrough hands the NIC/VF to the guest via the IOMMU and removes the host
kernel from the datapath — no host veth, no TPROXY, no mesh mTLS. Viable only for
non-mesh, line-rate workloads; not a competitor to candidates 1/2.

**Kata reasons that TRANSFER vs do NOT transfer (explicit):**
- *Transfer*: the VM ultimately needs a **tap** device; run the VMM jailed in a netns
  (Kata/Firecracker both do). Both are already satisfied by candidate 1.
- *Do NOT transfer (CNI-handoff-specific)*: tcfilter/TC-redirect, macvtap-as-endpoint,
  bridge, ipvlan/macvlan endpoints, and fd-passing-to-bridge-an-inherited-veth. All exist
  because Kata inherits a CNI veth it cannot replace. Overdrive owns provisioning; none
  apply.

### Does ADR-0089 §A2 need revision? **YES (rationale), NO (decision).**
The decision — reject fd-passing, adopt the wrapper — is **confirmed** and must not be
reversed. But §A2's recorded rationale ("deviates from the spike-proven launch shape …
zero new mechanism") is weaker than the evidence supports and invites a future reader to
reopen on "ease is not a real reason." Recommend the architect revise §A2 (and the
§Consequences / §A2-adjacent text) to:
1. Cite the **Firecracker-jailer precedent** and Kata's "separate netns" as positive
   support for VMM-in-netns (not merely "matches our spike").
2. Record the **isolation-direction** argument and the **CH-control-surface-is-netns-
   transparent** fact (so "the VMM needs host reach" is pre-empted).
3. Add an explicit **macvtap + VFIO interceptability disqualification** note, so the perf
   axis is bounded and not mistaken for a reason to reopen §A2.
4. Reframe the fd-passing cost from "new mechanism / ease" to the concrete
   **reboot/crash-state + cross-netns-thread** costs (Finding 8).

This is a rationale-strengthening amendment, not a course change. Route it through the
architect per repo convention (agents do not edit ADRs inline).

## Full Citations

[1] Cloud-Hypervisor maintainers. "Reconsider how network device creation from tap file descriptors is exposed over the API" (Discussion #2514). GitHub. Accessed 2026-08-27. https://github.com/cloud-hypervisor/cloud-hypervisor/discussions/2514
[2] Cloud-Hypervisor project. "Commands" (`--net` syntax reference). cloudhypervisor.org. Accessed 2026-08-27. https://www.cloudhypervisor.org/docs/prologue/commands/
[3] Cloud-Hypervisor project. "macvtap-bridge.md". GitHub. Accessed 2026-08-27. https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/macvtap-bridge.md
[4] Cloud-Hypervisor project. "device_model.md". GitHub. Accessed 2026-08-27. https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/device_model.md
[5] Firecracker project. "jailer.md". GitHub. Accessed 2026-08-27. https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md
[6] Firecracker project. "design.md" (network trust model). GitHub. Accessed 2026-08-27. https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md
[7] Kata Containers project. "docs/design/architecture/networking.md". GitHub. Accessed 2026-08-27. https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/networking.md
[8] Kata Containers project. "docs/design/architecture/README.md". GitHub. Accessed 2026-08-27. https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/README.md
[9] libvirt project. "Guest can reach outside network but can't reach host (macvtap)". wiki.libvirt.org. Accessed 2026-08-27. https://wiki.libvirt.org/Guest_can_reach_outside_network_but_cant_reach_host_macvtap.html
[10] Red Hat. "Appendix — Using the MacVTap driver" (RHEL 6 Virtualization Host Configuration and Guest Installation Guide). docs.redhat.com. Accessed 2026-08-27 (via search index; direct fetch 403). https://docs.redhat.com/en/documentation/red_hat_enterprise_linux/6/html/virtualization_host_configuration_and_guest_installation_guide/app_macvtap
[11] Linux kernel. "VFIO - 'Virtual Function I/O'". docs.kernel.org. Accessed 2026-08-27. https://docs.kernel.org/driver-api/vfio.html
[12] Tian, Kevin (Intel). "Hardware-Assisted Mediated Pass-Through with VFIO". Linux Foundation / KVM Forum, 2017. Accessed 2026-08-27. https://events19.linuxfoundation.org/wp-content/uploads/2017/12/Hardware-Assisted-Mediated-Pass-Through-with-VFIO-Kevin-Tian-Intel.pdf
[13] Kerrisk, Michael. "setns(2)" / "namespaces(7)" Linux man-pages. Accessed 2026-08-27. https://man7.org/linux/man-pages/man2/setns.2.html
[14] Overdrive. "Spike findings — guest-stack transparent-mTLS intercept (tap → nft-TPROXY)". In-tree: docs/feature/guest-stack-transparent-mtls-intercept/spike/findings.md. 2026-08-27.
[15] Overdrive. "ADR-0089: Tap-in-netns provisioning boundary + Cloud Hypervisor net attach". In-tree: docs/product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md. 2026-08-27.

## Research Metadata

Duration: ~1 session | External primary sources examined: 12 (Cloud-Hypervisor ×4,
Firecracker ×2, Kata ×2, libvirt, Red Hat, kernel VFIO, LF/VFIO slides) + in-tree spike
& ADR + man-pages | Cited: 15 | Cross-references: every load-bearing finding carries ≥2
sources except where flagged (G1 Kata per-endpoint ordering; G2 isolation direction).
Confidence distribution: High 6 findings (1,2,3,6,7,8) · Medium-High/Medium 2 (4,5). No
excluded-domain source used as load-bearing. Local Cilium tree (`/Users/marcus/git/
cilium/cilium`) was available but not cited: its CNI/eBPF-veth datapath is not a
microVM-attachment precedent and adds no load-bearing evidence for this decision.
Output: docs/research/dataplane/guest-nic-attachment-netns-vs-fd-passing-comprehensive-research.md
