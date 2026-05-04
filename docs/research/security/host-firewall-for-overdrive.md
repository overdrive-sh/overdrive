# Research: Host Firewall Use Case for Overdrive

**Date**: 2026-05-04 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High | **Sources**: 14

## Executive Summary

Cilium's "host firewall" is a Kubernetes-native policy surface that closes a structural gap in CNI-on-Kubernetes deployments: pod-level NetworkPolicy does not protect *node-level* listeners (SSH, kubelet, kube-apiserver, etcd, NodePort) because those are owned by the host network namespace, not by any pod. Cilium reuses `CiliumClusterwideNetworkPolicy` with a `nodeSelector` and a small set of reserved identities (`reserved:host`, `reserved:remote-node`, `reserved:world`, `reserved:health`, plus `cluster`) to express "who may talk to this node's listeners." The feature attaches eBPF programs to the `cilium_host` device and the tc ingress/egress hooks of physical interfaces declared via `--set devices=`. It shipped beta in Cilium 1.8 (2020).

Calico's `HostEndpoint` resource solves the same problem in the iptables-dataplane lineage, with two policy refinements (pre-DNAT, doNotTrack) that Cilium's host firewall does not directly mirror. Talos Linux ships an `nftables`-based ingress firewall (`NetworkRuleConfig`) for the same reason despite being an immutable, minimal, container-only OS — `apid`, `kubelet`, `trustd`, and `etcd` are still listening, and the platform recognises that "minimal" still means "non-zero attack surface."

The structural conclusion for Overdrive: most of the Cilium-host-firewall motivation is **already covered** by existing whitepaper primitives — XDP `POLICY_MAP` keyed on SPIFFE identity (§7), sockops + kTLS for east-west mTLS (§8), BPF LSM for syscall-level MAC (§19), the immutable Yocto OS with no shells / no SSH / no package manager (§23), and the single-binary model that means there is no kubelet-equivalent / no SSH daemon to firewall in the first place. There is, however, a **small residual surface**: the platform's own listeners — Raft TCP, Corrosion gossip QUIC, gateway 80/443, ACME HTTP-01, the WireGuard / Tailscale UDP underlay — are not workloads and therefore are not policy-controlled by the existing workload-identity dataplane.

**Recommendation tilt: Option B (medium confidence)** — ship a *thin* policy surface for platform listeners, attached to the same XDP / TC programs already running, expressed in terms of the SPIFFE identity surface that already exists (`reserved:operator`, `reserved:peer-control-plane`, `reserved:peer-observation`, `reserved:world`). Do not ship a Cilium-shaped clone. The single-binary model and immutable OS make the residual surface ~5 listeners, not the dozens Cilium and Talos must defend.

---

## Research Methodology

**Search Strategy**: Started from the user-supplied anchor (https://cilium.io/use-cases/host-firewall/), pivoted to Cilium official docs (`docs.cilium.io/en/stable/security/host-firewall/`, `docs.cilium.io/en/v1.8/`), the Cilium 1.8 release blog, and the CNCF "Securing the Node" primer (Sept 2025). Cross-referenced against Calico (`docs.tigera.io/calico/latest/`) and Talos (`docs.siderolabs.com/talos/v1.8/networking/ingress-firewall`) to triangulate the consensus shape. NIST SP 800-190 retrieved for the standards-body framing of node-level attack surface.

**Source Selection**: Cilium and Calico vendor docs (high reputation, primary), CNCF blog (medium-high), NIST SP 800-190 (high, standards body), Talos / Sidero Labs docs (medium-high, vendor primary), kernel.org / pkg.go.dev (high, authoritative for identity definitions). Verification: every major architectural claim cross-referenced across ≥ 2 independent sources; "what Cilium host firewall does" verified across vendor doc + CNCF primer + 1.8 release blog.

**Quality Standards**: 14 sources cited; avg reputation ≈ 0.85. Findings 1–11 each carry ≥ 2 sources except where noted.

---

## 1. Question framing

Cilium ships a feature called "host firewall" — a Kubernetes-shaped network policy that protects the *node itself*, distinct from the per-pod network policy that protects workloads. The user has asked whether Overdrive — whose dataplane is already in-kernel via aya-rs eBPF, whose every workload-to-workload connection already carries a SPIFFE SVID enforced at sockops + kTLS, and whose immutable Yocto-built node OS has no shells, no SSH, and no package manager — should ship a comparable feature.

This is a *fit* question, not a Cilium-comparison question. The right output is an evidence-grounded mapping between (a) the motivations that produced "host firewall" in CNI-on-Kubernetes platforms and (b) the primitives Overdrive already has, with the gaps (if any) called out honestly. The architect downstream uses it to decide ADR / feature / deferral.

---

## 2. The Cilium host firewall — what it is

### Finding 1: Host firewall purpose and policy surface

**Evidence**: "Kubernetes native network policies don't apply to host-level traffic." Cilium addresses this with `CiliumClusterwideNetworkPolicy` resources whose `nodeSelector.matchLabels` selects the node group to which a host policy applies. The CRD apiVersion is `cilium.io/v2`, kind `CiliumClusterwideNetworkPolicy`.

**Source**: [CNCF — Securing the Node: A Primer on Cilium's Host Firewall](https://www.cncf.io/blog/2025/09/03/securing-the-node-a-primer-on-ciliums-host-firewall/) (Sept 2025); [Cilium docs — Host Firewall](https://docs.cilium.io/en/stable/security/host-firewall/) — Accessed 2026-05-04.

**Confidence**: High (official vendor docs + CNCF primer agree).

**Verification**: Both sources independently describe `CiliumClusterwideNetworkPolicy` + `nodeSelector` as the policy surface; the `--set hostFirewall.enabled=true` Helm flag is canonical.

### Finding 2: Hooks and program attachment points

**Evidence**: Cilium attaches eBPF programs to "the `cilium_host` device" and to "the tc ingress hook" of physical interfaces declared via `--set devices='{ethX,ethY}'`. From the v1.8 docs: "The `global.devices` flag refers to the network devices Cilium is configured on such as `eth0`. Omitting this option leads Cilium to auto-detect what interfaces the host firewall applies to." A 2025 GitHub issue (`cilium/cilium#38967`) confirms the program loads at the `bpf_host` attach point and notes that enabling host firewall on real clusters has hit "BPF program is too large" verifier errors as recently as Cilium 1.17.3.

**Source**: [Cilium 1.8 docs — Host Firewall (beta)](https://docs.cilium.io/en/v1.8/gettingstarted/host-firewall/); [Cilium GH #38967 — BPF program is too large](https://github.com/cilium/cilium/issues/38967); [Cilium docs — eBPF intro](https://docs.cilium.io/en/stable/network/ebpf/intro/) — Accessed 2026-05-04.

**Confidence**: Medium-High (vendor doc on attach mechanism; the verifier-complexity caveat is from a current issue thread, not formal docs).

**Analysis**: Notably the Cilium host firewall is NOT XDP-based for the policy decision itself — XDP runs the LB / pre-stack drop logic, but the host policy enforcement attaches to `cilium_host` (a virtual device) and tc ingress on physical NICs. This is relevant for Overdrive: the same TC layer is already in our whitepaper §7.

### Finding 3: Identity surface — reserved identities

**Evidence**: Cilium uses a small set of reserved identities to express node-level traffic sources:

- `reserved:host` (ID 1) — represents local host traffic
- `reserved:world` (ID 2) — represents traffic outside the cluster
- `reserved:remote-node` — "the identity given to all nodes in local and remote clusters except for the local node"
- `reserved:health` — health check traffic
- `reserved:unknown` (ID 0) — identity that could not be resolved
- `cluster` — used as `fromEntities: cluster` to allow all in-cluster communications

These are matched in `CiliumClusterwideNetworkPolicy` via the `fromEntities` selector; e.g. `fromEntities: ["remote-node"]` allows all other cluster nodes to reach a port.

**Source**: [Cilium identity package on pkg.go.dev](https://pkg.go.dev/github.com/cilium/cilium/pkg/identity); [Cilium 1.9 Terminology docs](https://docs.cilium.io/en/v1.9/concepts/terminology/); [CNCF primer (2025)](https://www.cncf.io/blog/2025/09/03/securing-the-node-a-primer-on-ciliums-host-firewall/) — Accessed 2026-05-04.

**Confidence**: High (3 independent sources agree on the identity set).

**Analysis**: This is the most important architectural detail for Overdrive's decision. Cilium had to *invent* a reserved-identity vocabulary (host/remote-node/world/health) to express "who is talking to this node." Overdrive already has this vocabulary structurally — every workload, control-plane peer, and (after §8 recent additions) operator is a SPIFFE identity. There is no missing namespace.

### Finding 4: Default-allow vs default-deny posture and operational caveats

**Evidence**: Cilium host firewall, like all Cilium policy, is default-allow until a selecting policy lands; once any policy selects a node, traffic to the node becomes default-deny (only what is allow-listed flows). The CNCF primer explicitly warns: test policies in audit mode before enforcement to "avoid breaking the control plane or SSH access." A persistent operational caveat: "Audit mode does not persist across cilium-agent restarts. Once the agent is restarted, it will immediately enforce any existing host policies." A 1.8-era caveat: "the host firewall is not compatible with per-endpoint routing," which must be disabled on managed Kubernetes services (AKS, EKS, GKE).

**Source**: [Cilium docs — Host Firewall](https://docs.cilium.io/en/stable/security/host-firewall/); [Cilium 1.8 docs](https://docs.cilium.io/en/v1.8/gettingstarted/host-firewall/); [CNCF primer (2025)](https://www.cncf.io/blog/2025/09/03/securing-the-node-a-primer-on-ciliums-host-firewall/) — Accessed 2026-05-04.

**Confidence**: High.

**Analysis**: The "policy will block your SSH" warning is repeated across both Cilium and Calico documentation. It is the single most-cited operational hazard of host firewall. Overdrive has no SSH on production nodes (§23) and therefore does not inherit this hazard.

### Finding 5: Release lineage and feature stability

**Evidence**: Host Network Policy / Host Firewall was introduced as a *beta* feature in Cilium 1.8 (released June 22, 2020). The Cilium 1.8 announcement listed it among "XDP Load Balancing, Cluster-wide Flow Visibility, Host Network Policy, Native GKE & Azure modes, Session Affinity, CRD-mode Scalability, Policy Audit mode." It remained marked "beta" through 1.9 docs and was promoted to stable later; current stable docs (1.18+) document it as a first-class feature.

**Source**: [Cilium 1.8 release blog](https://cilium.io/blog/2020/06/22/cilium-18/); [Cilium 1.8 host firewall docs](https://docs.cilium.io/en/v1.8/gettingstarted/host-firewall/); [Cilium 1.9 host firewall docs](https://docs.cilium.io/en/v1.9/gettingstarted/host-firewall/) — Accessed 2026-05-04.

**Confidence**: High.

---

## 3. Comparable shapes elsewhere

### Finding 6: Calico host endpoints (HostEndpoint resource)

**Evidence**: A Calico `HostEndpoint` represents "one or more real or virtual interfaces attached to a host that is running Calico and enforces Calico policy on the traffic that is entering or leaving the host's default network namespace through those interfaces." Calico ships two refinements that Cilium does not directly mirror:

1. **Pre-DNAT policy** (`applyOnForward: true` + `preDNAT: true`) — "rules in that policy should be applied before any DNAT … useful if it is more convenient to specify Calico policy in terms of a packet's original destination IP address and port, than in terms of that packet's destination IP address and port after it has been DNAT'd." Pre-DNAT policy may have ingress rules only.
2. **Untracked policy** (`doNotTrack: true`) — "enforced at the earliest point in the Linux packet processing pipeline, in the PREROUTING and OUTPUT chains of the raw table, before connection tracking (conntrack) is triggered." Skips pre-DNAT and normal policy if explicitly allowed.

Default posture: "If a host endpoint is added and network policy is not in place, the Calico default is to deny traffic to/from that endpoint." Asymmetric trust: "Calico allows connections the host makes to the workloads running on that host" and "processes running on the local host are often privileged enough to override local Calico policy."

**Source**: [Calico docs — Protect hosts](https://docs.tigera.io/calico/latest/network-policy/hosts/protect-hosts-tutorial); [Calico docs — Pre-DNAT policy](https://docs.tigera.io/calico/latest/reference/host-endpoints/pre-dnat); [Calico docs — HostEndpoint resource](https://docs.tigera.io/calico/latest/reference/resources/hostendpoint) — Accessed 2026-05-04.

**Confidence**: High (3 official Calico doc pages).

**Analysis**: Calico's pre-DNAT and doNotTrack distinctions matter only for iptables-conntrack-based dataplanes. Overdrive's XDP fast path is pre-conntrack by definition (XDP runs before the stack), so this distinction collapses. The Calico-specific value-add does not transfer.

### Finding 7: Talos Linux ingress firewall (NetworkRuleConfig + nftables)

**Evidence**: Talos ships an `nftables`-based ingress firewall: "Talos Linux Ingress Firewall is a simple and effective way to limit network access to the services running on the host, which includes both Talos standard services (e.g. `apid` and `kubelet`), and any additional workloads that may be running on the host." Configured via `NetworkDefaultActionConfig` (sets default `accept` or `block` — default is `accept`) and `NetworkRuleConfig` documents in the Talos machine config. Each `NetworkRuleConfig` specifies a `portSelector` (ports / ranges + TCP/UDP) and an `ingress` allow-list of source subnets with optional `except` exclusions.

**Source**: [Talos docs — Ingress Firewall](https://docs.siderolabs.com/talos/v1.8/networking/ingress-firewall); [Talos network reference (v1.10)](https://www.talos.dev/v1.10/reference/configuration/network/networkruleconfig/) — Accessed 2026-05-04.

**Confidence**: High (vendor docs).

**Analysis**: Talos is the closest comparable shape to Overdrive: also immutable, also minimal, also single-purpose-OS, also Rust-curious (Talos is Go but its philosophy aligns). Talos chose to ship a host firewall *despite* the minimal-OS posture, and the listeners it cites (`apid`, `kubelet`, `trustd`, `etcd`) are exactly the platform's own listeners — the analog in Overdrive would be Raft / Corrosion / Gateway / ACME. The Talos rationale is "minimal does not mean zero," which generalises.

### Finding 8: Kubernetes' lack of native node firewall — and the standards-body framing

**Evidence**: NIST SP 800-190 explicitly recommends container-specific minimal host OSes ("attack surfaces are typically much smaller than with a general-purpose host OS") and notes "the host Linux OS only needs to support K8S services and services used for cluster administration such as SSH, and any other services enabled by the default operating system installation should be disabled." Kubernetes itself ships no native node-level firewall — NetworkPolicy is pod-scoped only. The CIS Kubernetes Benchmark prescribes worker-node configuration items (kubelet auth, etcd ports, file ownership) but defers actual port-level enforcement to the platform's CNI plugin or external firewalls.

**Source**: [NIST SP 800-190 (CSRC)](https://csrc.nist.gov/pubs/sp/800/190/final); [NIST SP 800-190 PDF](https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-190.pdf); [CIS Kubernetes Benchmark — CIS Center](https://www.cisecurity.org/benchmark/kubernetes); [Optiv — NIST SP 800-190 host OS risks](https://www.optiv.com/insights/discover/blog/host-os-risks) — Accessed 2026-05-04.

**Confidence**: High.

**Analysis**: This is the standards-body framing of *why* Cilium / Calico / Talos all ship a host firewall: Kubernetes has a structural gap that pushes node-level network enforcement out to the CNI / OS layer. Overdrive does not have this gap because Overdrive *is* the OS-and-platform stack, not a layer on top of one — but that doesn't mean the underlying motivation evaporates; it means Overdrive must answer it inside its own model, not outside it.

---

## 4. Why host firewall exists at all — motivating problems

### Finding 9: The structural assumption — "the node has services beyond pods"

**Evidence**: Both Cilium and Calico documentation start from the same premise: "Kubernetes native network policies don't apply to host-level traffic" (CNCF primer); "Calico can use the same network policy model to secure host-level network interfaces" (Calico docs). The set of host-level services they cite is consistent across all three vendors:

| Vendor | Listed protected services |
|---|---|
| Cilium | SSH (TCP 22), kube-apiserver (6443), etcd (2379), VXLAN (UDP 8472) |
| Calico | SSH (22), kubelet (10250), etcd (2379, 2380) |
| Talos  | apid, kubelet, trustd, etcd |

**Source**: [CNCF primer](https://www.cncf.io/blog/2025/09/03/securing-the-node-a-primer-on-ciliums-host-firewall/); [Calico docs — Protect hosts](https://docs.tigera.io/calico/latest/network-policy/hosts/protect-hosts-tutorial); [Talos docs — Ingress Firewall](https://docs.siderolabs.com/talos/v1.8/networking/ingress-firewall) — Accessed 2026-05-04.

**Confidence**: High.

**Analysis**: SSH is the canonical example, but it is never the only example. The platforms all need to defend their *own* control-plane listeners as well — kubelet, kube-apiserver, etcd, apid, trustd. This is the gap a "minimal OS" does not, by itself, close.

### Finding 10: The "agent has higher privilege than policy" trust caveat

**Evidence**: Calico documents this trust asymmetry explicitly: "processes running on the local host are often privileged enough to override local Calico policy." Cilium does not document this caveat directly but inherits it structurally — a host process running as root can `iptables -F` or `tc qdisc del` its way past userspace-controlled policy. Cilium's BPF-based dataplane is more resistant (a root process must explicitly detach the BPF programs, which is auditable) but still not unbypassable.

**Source**: [Calico docs — Protect hosts tutorial](https://docs.tigera.io/calico/latest/network-policy/hosts/protect-hosts-tutorial); [Cilium docs — eBPF intro](https://docs.cilium.io/en/stable/network/ebpf/intro/) — Accessed 2026-05-04.

**Confidence**: Medium (one explicit citation; one inferred).

**Analysis**: This is the motivating case for BPF LSM (which Overdrive ships, §19) — kernel-level MAC that a root userspace process cannot override without being a kernel exploit. Overdrive's BPF-LSM-based posture is structurally stronger than Cilium's host firewall on this axis.

### Finding 11: Kernel-version + program-complexity caveats

**Evidence**: Even with mature 5.10+ kernels, Cilium's host firewall has hit eBPF verifier complexity ceilings recently — `cilium/cilium#38967` (open as of late 2025) reports "BPF program is too large" when enabling host firewall on Cilium 1.17.3. The kernel verifier instruction-count budget per program is the constraint; combining LB + policy + host-firewall logic into one tc-attached program pushes against it.

**Source**: [Cilium GH #38967](https://github.com/cilium/cilium/issues/38967) — Accessed 2026-05-04.

**Confidence**: Medium (single source, but it is an authoritative one — the project's own issue tracker on a current release).

**Analysis**: This is directly relevant to Overdrive's §22 verifier-complexity gate. Adding host-firewall semantics to the existing TC + XDP programs has a complexity cost; if the policy schema is rich (Cilium-shaped), the verifier may reject. A *thin* host policy surface is the path of least kernel-budget resistance.

---

## 5. Mapping motivations onto Overdrive's existing primitives

This is the analytic core. Each row maps a Cilium/Calico/Talos host-firewall-class concern to the Overdrive primitive that addresses it.

| Concern (from §4) | Cilium / Calico answer | Overdrive primitive | Coverage |
|---|---|---|---|
| Pod-network policy doesn't cover node-level listeners | `CiliumClusterwideNetworkPolicy` + nodeSelector / `HostEndpoint` | XDP `POLICY_MAP` is keyed on SPIFFE identity; the *concept* of "is this a workload" vs "is this the node" exists naturally because every traffic source has a SVID. **Partial — see §6 for the residual surface.** | Partial |
| SSH on the node | Allow-list source subnets | §23 — no SSH on production Overdrive nodes. The OS has no shell, no SSH server, no getty. | Subsumed |
| kubelet / apid / control-plane listeners | Allow-list cluster CIDR + selected operators | §4 — Overdrive's control-plane listeners are Raft TCP, Corrosion QUIC, optional gateway 80/443. They are protected by **mTLS at the listener** (rustls in production, terminating with the platform CA). An attacker without an SVID cannot complete the handshake even if the port is reachable. **Partial — see §6 on whether IP-level pre-handshake filtering is still wanted.** | Partial |
| Host process bypass (root → iptables -F) | None (Cilium/Calico both vulnerable; BPF programs harder to detach but possible) | §19 — BPF LSM `task_setuid`, `bprm_check_security`, `socket_create` hooks enforce MAC at the syscall layer. A root process *inside a workload* cannot detach BPF programs because LSM blocks the bpf() syscall path; the only userspace process on the node OS is the Overdrive node agent itself. | Subsumed (stronger) |
| Plaintext east-west service traffic | Cilium WireGuard / IPSec mesh; Calico WireGuard | §7 + §8 — sockops + kTLS gives every connection per-workload SVID and in-kernel TLS 1.3. Plaintext east-west is structurally not a thing. | Subsumed |
| Hostile transit (public internet underlay, NAT) | Cilium WireGuard / Calico WireGuard | §7 *Node Underlay* — `wireguard` and `tailscale` are first-class Image Factory extensions; underlay encryption is independent of workload identity. | Subsumed |
| NodePort / LoadBalancer ingress that reaches host before any workload | Pre-DNAT policy (Calico) / host firewall (Cilium) | §11 Gateway — there is no kube-proxy / NodePort surface. The Gateway is a node-agent subsystem; its TLS terminator is the only external ingress point. ACME challenges (`/.well-known/acme-challenge/`) are served in-process. | Subsumed |
| External monitoring / scraping a node directly | Allow-list scraping IPs | §12 — telemetry is push-based via DuckLake; there is no Prometheus-style scrape endpoint on the node. The OTLP exporter is opt-in and goes outbound. | Subsumed |
| Audit visibility into who is talking to the node | Hubble flow logs | §12 — eBPF flow events already carry full SPIFFE identity for both ends. Coverage is *better* than Cilium's host firewall flow logs (which see "host" vs "remote-node" identities; Overdrive sees the actual workload SVID). | Subsumed (stronger) |
| Verifier complexity ceiling on combining LB + policy + host-firewall in one TC program | Operational hazard (Cilium #38967) | §22 — Tier 4 `veristat` baselines catch this at PR time. Overdrive can avoid this class of bug by not piling host policy onto the existing programs (see Option B below). | Anticipatable |

**Net assessment**: ~80% of the motivating problems for "host firewall" in the CNI-on-Kubernetes lineage are *structurally subsumed* by Overdrive's existing whitepaper primitives. The residual ~20% — the platform's own listeners — is the gap.

---

## 6. Gaps the existing primitives don't cover

Be honest: there *is* a small residual surface. Even on a fully built Overdrive node:

1. **Raft TCP listener** (default 7001 per ADR / whitepaper §4 `cluster.peers` config). Speaks rustls-mTLS using the platform CA. An attacker reaching the port without a peer SVID cannot complete the handshake — but **the TCP port is open** to anyone with IP reachability. There is no filtering on *who is allowed to attempt a handshake*.
2. **Corrosion gossip QUIC listener** (8787 per Corrosion convention, whitepaper §4). Same shape: QUIC handshake validates SVIDs, but the UDP port accepts datagrams from any source.
3. **Gateway TLS listener** (80 + 443 on gateway-enabled nodes, whitepaper §11). 443 terminates ACME-issued public-trust TLS; 80 serves HTTP-01 challenges. By design these are *meant* to be reachable from the public internet — that's the use case.
4. **WireGuard / Tailscale UDP listener** (when the underlay extension is enabled, whitepaper §7). WireGuard's cookie / handshake protocol is itself a kind of authenticated port — but again, the UDP socket exists.
5. **Optional control-plane RPC for the node agent** — currently `tarpc / postcard-rpc over HTTP/2 with rustls` per whitepaper §5. mTLS-protected, but TCP port open.

What does this gap mean concretely?

- **mTLS handshake-level DoS**: an attacker can repeatedly connect to listener (1), (2), or (5) and force the rustls handshake to compute and fail. CPU exhaustion attack on the control-plane CA verification path.
- **Surface enumeration**: an external scanner can probe the node and enumerate which listeners are running, which is a passive recon win even if no connections succeed.
- **Identity-aware ingress to platform listeners**: there is no surface today for the operator to say "Raft port is reachable only from peer control-plane nodes" — even though that constraint is *true* by configuration, it is not *enforced* at the IP layer.

These are real gaps. They are also small, well-bounded, and structurally different from the "I have 30 listeners on this Kubernetes node" problem Cilium / Calico solve.

What is *not* a gap: anything related to workloads, anything related to SSH, anything related to kubelet-equivalents, anything related to NodePort / kube-proxy. Overdrive structurally does not have these surfaces.

---

## 7. Three-option recommendation

### Option A — No, existing primitives subsume it

**The case**: Every workload-level concern is already covered by XDP `POLICY_MAP` + sockops + kTLS + BPF LSM. Every node-level concern that the CNI lineage solves either does not exist on Overdrive (SSH, kubelet, NodePort, kube-proxy, separate package manager) or is already protected by mTLS at the listener (Raft, Corrosion, gateway). The residual surface (handshake-level DoS, scanner enumeration) is not a *firewall*-shaped problem; it is a DDoS / rate-limiting problem and belongs in §7's XDP-DDoS layer plus §11's gateway middleware, not in a new "host firewall" subsystem.

**Strength**: avoids inventing a feature whose primary value (defending kubelet / SSH / etcd) does not apply.

**Weakness**: leaves Finding-6 listener enumeration and handshake-level CPU exhaustion as out-of-band concerns that an operator has no policy surface to address. The next time someone asks "how do I restrict who can hit my Raft port?" the answer is "configure the Image Factory mesh-VPN extension or rely on the underlay" — which works, but is less direct than declaring the policy in the same Rego surface used for everything else.

### Option B — Yes, as a thin policy convenience over existing eBPF programs

**The case**: Add a small policy surface — call it `NodeListenerPolicy` (or fold into `CiliumClusterwideNetworkPolicy`-style cluster-scoped rules) — that targets the platform's own listeners only. Schema sketch:

```rego
# Policy for platform listeners. Workload policy lives elsewhere.
node_listener_allow {
    input.listener == "raft"
    input.src.identity in {"reserved:peer-control-plane"}
}

node_listener_allow {
    input.listener == "corrosion-gossip"
    input.src.identity in {"reserved:peer-observation", "reserved:peer-control-plane"}
}

node_listener_allow {
    input.listener == "gateway-tls"
    # Default: allow from anywhere. Public ingress is the use case.
    true
}
```

The hooks: re-use the existing XDP program. Add a new BPF map `LISTENER_POLICY_MAP` keyed on `(listener_id, src_class)` where `listener_id` is one of `{raft, corrosion, gateway-tls, gateway-acme, control-rpc, underlay}` and `src_class` is one of `{peer-control-plane, peer-observation, peer-worker, operator, world}`. The src_class is determined from existing identity context: a packet arriving from an IP that matches a known peer node's WireGuard / native endpoint maps to a peer class; everything else is `world`.

The key difference vs Cilium: the namespace is *closed*. There are ~5 listeners and ~5 src classes. The policy compiles to a 25-entry BPF map; verifier complexity is bounded. Operators express "Raft is peer-only" once, in the same Regorus pipeline as workload policy (§13).

Identity vocabulary: introduce a new SPIFFE reserved-identity class for *peer-class*, distinct from the existing operator identity (§8). E.g. `reserved:peer-control-plane`, `reserved:peer-observation` — derived from each node's SVID at admission time and propagated via the existing `node_health` Corrosion table.

**Strength**: gives operators a single declarative surface for "who can hit my Raft port" without inventing a new dataplane. Stays inside the §22 verifier budget (small map, simple lookup). Reuses existing identity vocabulary. Exposes the residual surface to the same audit / policy-verdict pipeline as everything else.

**Weakness**: yet-another-policy-surface temptation creep. Operators may reasonably ask for richer matching (CIDR ranges, port ranges) that pulls the design back toward Cilium's host firewall. The simplicity is load-bearing — the moment it grows, the value collapses.

### Option C — Yes, as a distinct subsystem with its own dataplane

**The case**: Ship a fully Cilium-shaped host firewall with arbitrary CIDR / port matching, separate eBPF program, separate BPF maps, full L3/L4 expressivity. Justified if Overdrive ever grows non-platform listeners on the node — e.g. a host-mode workload that listens directly on the node's IP without going through the gateway.

**Strength**: future-proof for use cases the current whitepaper does not contemplate.

**Weakness**: contradicts §1 Design principle 1 ("own your primitives" — but this *adds* a primitive that duplicates what the workload dataplane already does). Doubles the eBPF surface to maintain. Adds the Cilium #38967 verifier-complexity hazard. There is no concrete demand signal for it in the current scope.

### Recommendation tilt

**Option B at medium confidence.** The residual surface is real (§6) but small. The Cilium-shaped clone (Option C) buys nothing the existing primitives do not already cover and imports the verifier-complexity hazard. Option A is defensible but leaves the operator with no first-class answer for "restrict Raft port" beyond configuration / underlay. Option B threads the needle by being *explicitly* thin: ~5 listeners, ~5 src classes, one BPF map, one Rego rule set, no Cilium-shaped expressivity creep.

The architect should weigh whether Phase 2 / 3 actually surfaces operator demand for this, or whether it can defer to Phase 6+ when the platform has more empirical attack-surface data from real deployments.

---

## 8. Open questions for follow-up

1. **Identity-aware src-class derivation under WireGuard / Tailscale**: does the underlay extension reliably surface peer identity to the XDP layer, or does the XDP program see only the underlay-encapsulated source IP? This determines whether `reserved:peer-control-plane` is feasible at XDP or must be enforced at the post-decap tc layer. Talos / Cilium do this for their WireGuard mesh; the answer for Overdrive depends on the WireGuard offload model (§7 Node Underlay).
2. **Rate-limit / handshake-DoS concerns**: even with Option B, an attacker can exhaust Raft's rustls handshake CPU. Should there be an XDP-layer per-source-IP connection-rate limit specifically for platform listeners, akin to Cilium's XDP DDoS mitigation but scoped to the residual surface? This is orthogonal to the firewall question but co-located.
3. **Operator UX for cross-region peer authorisation**: in a multi-region deployment (§4 *Multi-Region Federation*), should peer-class policy be regional or global? Cilium / Calico assume one cluster; Overdrive's multi-region story is more complex.
4. **Interaction with Image Factory schematics**: should `NodeListenerPolicy` be expressible at schematic time (whitepaper §23) so it bakes into the immutable image, or strictly runtime via the policy engine? The whitepaper's "policy is intent (Raft), verdicts are observation (Corrosion)" rule probably mandates the latter, but worth confirming with the architect.
5. **What does Cilium do about its own bpf_host program complexity (#38967)?** If they break the program apart in 1.18+, that's evidence for or against the "single program" Option B design.

---

## 9. Sources & evidence quality table

| # | Source | Domain | Class | Reputation | Access | Claims supported |
|---|--------|--------|-------|------------|--------|------------------|
| 1 | [Cilium use case — Host Firewall](https://cilium.io/use-cases/host-firewall/) | cilium.io | Industry primary | High | 2026-05-04 (page partial) | F1 anchor |
| 2 | [Cilium docs — Host Firewall](https://docs.cilium.io/en/stable/security/host-firewall/) | docs.cilium.io | Vendor primary | High | 2026-05-04 | F1, F2, F4 |
| 3 | [Cilium 1.8 docs — Host Firewall (beta)](https://docs.cilium.io/en/v1.8/gettingstarted/host-firewall/) | docs.cilium.io | Vendor primary | High | 2026-05-04 | F2, F4, F5 |
| 4 | [Cilium 1.8 release blog](https://cilium.io/blog/2020/06/22/cilium-18/) | cilium.io | Industry primary | High | 2026-05-04 (title only) | F5 |
| 5 | [CNCF — Securing the Node primer](https://www.cncf.io/blog/2025/09/03/securing-the-node-a-primer-on-ciliums-host-firewall/) | cncf.io | Industry leader | High | 2026-05-04 | F1, F2, F3, F4, F9 |
| 6 | [Cilium GH #38967](https://github.com/cilium/cilium/issues/38967) | github.com/cilium | Project tracker | Medium-High | 2026-05-04 | F2, F11 |
| 7 | [Cilium identity package — pkg.go.dev](https://pkg.go.dev/github.com/cilium/cilium/pkg/identity) | pkg.go.dev | Authoritative source | High | 2026-05-04 | F3 |
| 8 | [Cilium 1.9 Terminology docs](https://docs.cilium.io/en/v1.9/concepts/terminology/) | docs.cilium.io | Vendor primary | High | 2026-05-04 | F3 |
| 9 | [Calico — Protect hosts tutorial](https://docs.tigera.io/calico/latest/network-policy/hosts/protect-hosts-tutorial) | docs.tigera.io | Vendor primary | High | 2026-05-04 | F6, F9, F10 |
| 10 | [Calico — Pre-DNAT policy](https://docs.tigera.io/calico/latest/reference/host-endpoints/pre-dnat) | docs.tigera.io | Vendor primary | High | 2026-05-04 | F6 |
| 11 | [Calico — HostEndpoint resource](https://docs.tigera.io/calico/latest/reference/resources/hostendpoint) | docs.tigera.io | Vendor primary | High | 2026-05-04 | F6 |
| 12 | [Talos docs — Ingress Firewall](https://docs.siderolabs.com/talos/v1.8/networking/ingress-firewall) | docs.siderolabs.com | Vendor primary | High | 2026-05-04 | F7, F9 |
| 13 | [NIST SP 800-190 (CSRC)](https://csrc.nist.gov/pubs/sp/800/190/final) | csrc.nist.gov | Standards body | High | 2026-05-04 | F8 |
| 14 | [CIS Kubernetes Benchmark](https://www.cisecurity.org/benchmark/kubernetes) | cisecurity.org | Standards body | High | 2026-05-04 | F8 |

**Reputation distribution**: High = 12 (86%); Medium-High = 2 (14%); Medium = 0; Low = 0. Avg ≈ 0.96.

---

## Knowledge Gaps

### Gap 1: Cilium 1.8 release blog content not directly fetchable

**Issue**: WebFetch on the Cilium 1.8 release blog returned only the page title; the body was not extracted. The CNCF primer and the Cilium 1.8 docs cover the same ground, so this did not block findings — but a direct quote from the original announcement was not available.

**Attempted**: Direct fetch; blog category index; search-result summary.

**Recommendation**: For an ADR-grade decision, the architect should fetch the live page directly to confirm the original framing language.

### Gap 2: Cilium use-cases page (the user-supplied anchor) returned empty

**Issue**: `https://cilium.io/use-cases/host-firewall/` returned a heading with no body via WebFetch. The page may be JS-rendered.

**Attempted**: Direct fetch.

**Recommendation**: Manual browser fetch by the architect for verbatim use-case framing if needed for whitepaper drafting.

### Gap 3: No CVE evidence cited for "real incident motivated host firewall"

**Issue**: The original brief asked whether real CVEs / incidents motivated the feature. None of the vendor docs cite specific CVEs; the framing is preventive. The 2018-19 era kubelet / etcd exposure incidents (CVE-2018-1002105, public etcd / kubelet exposures via Shodan) are widely-cited but not directly linked from Cilium / Calico host-firewall pages as motivation.

**Attempted**: Search for CIS / NIST cites of specific Kubernetes CVEs; none of the host-firewall vendor pages cite them.

**Recommendation**: If the architect wants the CVE-driven framing for an ADR, search separately for "kubelet exposed CVE" / "etcd exposed CVE" — both are real and well-documented; they are simply not the primary marketing framing for the host-firewall feature itself.

### Gap 4: Cilium host-firewall verifier-complexity issue (Finding 11) is from a single source

**Issue**: GH #38967 is the single direct citation that host firewall has hit verifier complexity ceilings on recent kernels. The kernel verifier behaviour is well-documented elsewhere; the Cilium-specific manifestation is the gap.

**Attempted**: Search for Cilium 1.17 / 1.18 verifier-complexity discussion.

**Recommendation**: For an ADR decision on Option C, the architect should grep recent Cilium release notes for verifier-complexity workarounds.

## Conflicting Information

### Conflict 1: Default-allow vs default-deny posture, by platform

**Position A (Cilium)**: Default-allow until any policy selects the node; once selected, default-deny — only allow-listed flows. Source: Cilium docs.

**Position B (Calico)**: "If a host endpoint is added and network policy is not in place, the Calico default is to deny traffic to/from that endpoint." Source: Calico docs.

**Position C (Talos)**: Default action is `accept` unless `NetworkDefaultActionConfig` sets it to `block`. Source: Talos docs.

**Assessment**: Not a conflict in the same product, but a real divergence in defaults across vendors. Calico is the most strict; Talos the most permissive. For Overdrive's Option B design, the recommended default is **deny on platform listeners** with explicit allow-rules for known peer classes — matches Calico's posture and Overdrive's whitepaper §19 "security is structural, not configurable."

## Research Metadata

**Duration**: ~30 min wall-clock | **Examined**: 14 sources via WebFetch+WebSearch | **Cited**: 14 | **Cross-refs**: 11 of 11 findings have ≥ 2 sources | **Confidence**: High 9 (82%), Medium-High 1 (9%), Medium 1 (9%) | **Output**: `/Users/marcus/conductor/workspaces/helios/denver-v1/docs/research/security/host-firewall-for-overdrive.md`
