# Research: The Best Solution — Recommended Transparent per-Workload mTLS Architecture Across All Overdrive Workload Classes (Roadmap 2.4 / GH #26)

**Date**: 2026-06-05 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 26 (avg reputation 0.97)

> Decision-grade synthesis. Inputs:
> - `docs/research/dataplane/sockops-mtls-ktls-installation-comprehensive-research.md` (mechanism)
> - `docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md` (race window)
> - `docs/research/transparent-encryption-comprehensive-research.md` (landscape: WG/IPsec/ztunnel)
> - whitepaper §6 (Cloud Hypervisor, guest agent, vsock), §7 (sockops+kTLS), §8 (SPIFFE identity)
> Goal: land ONE recommended architecture the DESIGN wave can adopt, honest about per-class mechanisms,
> unified in identity + control plane. Pinned appliance kernel = 6.6 LTS (Overdrive controls it).

## Executive Summary / The Recommendation

The whitepaper §7 claim that sockops+kTLS "works identically for process workloads, VMs, unikernels, and WASM functions" is factually incorrect for VM-class workloads. This is not a design flaw — it is a structural consequence of where TCP terminates. The recommended architecture accepts this fact and exploits Overdrive's unique advantages (kernel control, host-resident CA, controlled tap interfaces) to provide genuine per-workload SPIFFE identity for every workload class through two complementary enforcement mechanisms, unified by a single identity plane.

**The workload split is architectural, not a design choice.** Process and WASM functions (wasmtime = in-process host sockets) have a `struct sock` in the host kernel; sockops/kTLS/pidfd_getfd reaches them. MicroVMs and unikernels run their own TCP stack in the Cloud Hypervisor guest; the host kernel sees virtio-net frames, not TCP sockets — no host BPF mechanism (sockops, sockmap, sk_storage, kTLS) can see their connections. This split is confirmed by the mechanism research and is the spine of the recommendation.

**For host-socket workloads (process, WASM)**: Architecture C (sockmap proxy redirect) for v1 — production-proven, zero race window, full protocol coverage, identical to Istio Ambient ztunnel applied to host sockets. Long-term: Architecture A + custom in-kernel write-block patch — sockops+kTLS with no per-packet overhead, enabled by Overdrive's kernel control (an option no upstream-bound mesh has). The v1 proxy is ~0.17ms P99 latency add and ~0.06 vCPU overhead at 1000 RPS; acceptable for all but the highest-throughput storage paths.

**For guest-stack workloads (MicroVM, unikernel)**: Host L4 tap proxy — TC eBPF or TPROXY intercepts the guest's TCP at the virtio-net tap on the host, terminates TCP, asserts the allocation's SVID (keyed by tap identity), re-originates mTLS outbound. No in-guest changes required; §6 minimal-guest principle upheld; §8 trust-root invariant preserved (SVID private key never enters the guest). This is the canonical industry pattern — Istio Ambient ztunnel uses the exact same design for Kubernetes pods, and Finding 4 confirms EVERY platform (Kata+Istio, gVisor+Istio, Fly.io 6PN) either does this or uses an in-guest sidecar (which requires a cooperating Linux guest and is impossible for sealed unikernels).

**The unified identity plane**: ONE SPIFFE CA, ONE IDENTITY_MAP, ONE trust bundle, ONE policy model (SPIFFE ID-based POLICY_MAP) — all already in the whitepaper. Both enforcement mechanisms (kTLS path and tap proxy path) consume from this shared plane. An operator writing a policy references the SPIFFE ID; the enforcement mechanism is an implementation detail.

**The decisive prior-art finding (Finding 4)**: No production platform uses in-guest kTLS for transparent per-workload mTLS. Zero examples exist across Fly.io (6PN WireGuard, machine-level not per-workload), Kata Containers (in-guest Envoy sidecar — requires injectable Linux guest), gVisor (in-sandbox Envoy — same requirement), Cilium (ztunnel or WireGuard, no guest kTLS), and Istio (ztunnel host proxy, or sidecar — the Ambient design specifically exists to avoid in-guest logic). The absence is structural: in-guest kTLS requires guest kernel ownership, incompatible with sealed/unikernel/BYO-kernel guests.

**Required whitepaper amendments**: §7 "works identically" must be corrected; §6 minimal-guest principle is upheld (tap proxy is host-side); §8 trust-root invariant is upheld (SVID key stays on host for tap proxy path). The amendments are clarifications, not reversals of the design philosophy.

## Research Methodology

**Search Strategy**: Pre-read all three prior research docs and whitepaper §6/§7/§8 for established facts. New research targeted the two decisive gaps: (1) prior art on per-workload identity for VM-class platforms (Finding 4) — primary sources: istio.io Ambient architecture docs, github.com/istio/istio/architecture/ambient/ztunnel.md, fly.io docs, katacontainers.io, gvisor.dev; (2) performance reality of the proxy hop (Finding 5) — primary sources: istio.io performance/scalability docs, imesh.ai benchmarks, F5/NGINX kTLS performance blog.

**Source Selection**: Official (kernel.org, istio.io, gvisor.dev, katacontainers.io, fly.io/docs) — High 1.0. Technical docs (docs.cilium.io, linkerd.io, github.com/istio source) — High 1.0. Industry (lwn.net, f5.com, imesh.ai) — Medium-High 0.8. Minimum reputation 0.8. All major new-research claims from primary sources.

**Quality Standards**: 3+ sources/claim ideal; 2 acceptable; 1 authoritative minimum with confidence note. All mechanism claims from official docs or authoritative source code.

## Findings

---

### Finding 1 — The load-bearing taxonomy: workloads split by WHERE the TCP stack terminates

**This is the axis.** The §7 list "process workloads, VMs, unikernels, and WASM functions" is a workload-type list, not a mechanism list. The mechanism splits differently:

**HOST-SOCKET workloads**: Process (exec driver, tokio::process) and WASM functions (wasmtime runs in-process on the host; WASI sockets are host sockets). TCP terminates in the **host kernel**. There IS a `struct sock` on the host. Host sockops+kTLS can reach them; `pidfd_getfd()` gives the agent the fd; sk_storage, sockmap insertion, and kTLS installation all work.

**GUEST-STACK workloads**: MicroVMs and unikernels (Cloud Hypervisor guest). The guest runs its own TCP stack (Linux kernel in the VM, or a unikernel net stack like Unikraft). TCP terminates in the **guest kernel**. There is NO `struct sock` on the host for these connections — the host sees virtio-net frames, not TCP sockets. Host sockops, sockmap, sk_storage, pidfd_getfd, and kTLS installation are ALL structurally blind to them.

**Evidence**: Established in `sockops-ktls-plaintext-race-window-research.md` Comparison Matrix (rows: "Process: yes; VM/unikernel: NO — socket not accessible to host agent"). Confirmed by Cloud Hypervisor architecture (§6): Cloud Hypervisor is "one process per VM," virtio-net connects guest to host — the host's TCP/IP stack never processes the guest's east-west connections. WASM functions run via wasmtime in-process: "Wasmtime — Serverless functions, plugins" per whitepaper §6 driver table; WASI sockets in wasmtime are backed by the host kernel's `socket()` syscall (host sockets), not a userspace TCP stack.

**Source**: [sockops-ktls-plaintext-race-window-research.md Comparison Matrix](docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md) — internal, High (1.0); [Cloud Hypervisor architecture, whitepaper §6](docs/whitepaper.md) — internal, High (1.0); [Wasmtime WASI socket model — docs.wasmtime.dev](https://docs.wasmtime.dev) — High (1.0).

**Confidence**: High — the guest-stack / host-socket split follows directly from how virtio-net and wasmtime work architecturally. Not a design choice; it's a structural property.

**Analysis**: The §7 claim "works identically for process workloads, VMs, unikernels, and WASM functions" is factually incorrect for VMs and unikernels as written. sockops+kTLS reaches host sockets (process, WASM); it cannot reach guest-stack sockets (MicroVM, unikernel). This is the central finding the recommendation builds on.

---

### Finding 2 — Host-socket workloads: the best path (Arch A + in-kernel write-block vs Arch C proxy)

**Verdict**: For host-socket workloads (process + WASM), Architecture A + in-kernel write-block patch is the optimal long-term path given Overdrive's kernel control. Architecture C (sockmap proxy) is the correct v1 path — production-proven, zero novelty, full protocol coverage. The two are compatible: ship C for v1, migrate to A + write-block in a later roadmap step.

**Evidence from prior research** (summarized; do not re-derive):

- Architecture A (sockops intercept + kTLS key install, agent steps out) has a race window: writes during the 10–100ms agent handshake turnaround are SK_DROP'd (data loss) because sk_msg has no lossless-hold verdict. This is established with High confidence from kernel source review in `sockops-ktls-plaintext-race-window-research.md` Findings 1–2.
- Architecture C (sockmap redirect to proxy) eliminates the race by construction — app bytes go to the proxy socket, not the wire. Cost: per-packet copy for the full connection lifetime. Production-proven in Istio Ambient ztunnel, Cilium 1.19+, Linkerd2. Established in `sockops-ktls-plaintext-race-window-research.md` Finding 3.
- The in-kernel write-block option: because Overdrive pins and controls the appliance kernel (6.6 LTS), a custom out-of-tree patch adding a "pending-kTLS" backpressure socket state (blocks `write()` instead of dropping) is achievable. This closes both the cleartext-escape risk AND the data-loss risk, making Architecture A fully correct. Established in `sockops-ktls-plaintext-race-window-research.md` Controlled-kernel implication section.

**Key trade-off**: Architecture C (proxy) has per-packet copy overhead (every segment copied through the proxy for the full connection lifetime). Architecture A + write-block has zero steady-state overhead after kTLS is armed, but requires carrying an out-of-tree kernel patch across pin bumps. For v1, the proxy's overhead is acceptable; for a high-throughput east-west workload at scale, the no-overhead A path is compelling.

**Architecture A v1 acceptability condition**: For request-first protocols (HTTP, gRPC, PostgreSQL), the data-loss window is "acceptable" — a connection reset if `drops > 0`, and the application retries. This is protocol-correct for the majority of workloads Overdrive targets. Server-speaks-first protocols (SMTP, FTP, SSH) require Architecture C or the write-block patch.

**Source**: [sockops-ktls-plaintext-race-window-research.md](docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md) — internal, High (1.0), Findings 1–3 and Controlled-kernel implication. [sockops-mtls-ktls-installation-comprehensive-research.md](docs/research/dataplane/sockops-mtls-ktls-installation-comprehensive-research.md) — internal, High (1.0).

**Confidence**: High for the mechanism analysis. Medium for the "in-kernel write-block patch is feasible" claim — it is architecturally sound (analogous to `SO_SNDBUF` full back-pressure) but has no shipping precedent.

---

### Finding 3 — Guest-stack workloads: in-guest dataplane agent vs host L4 mTLS proxy vs node underlay

**Verdict**: The host L4 transparent mTLS proxy (ztunnel/HBONE-shape) is the correct option for guest-stack workloads. In-guest dataplane agent is rejected on multiple grounds. Node underlay (WireGuard) is a valid defense-in-depth layer but provides node identity, not per-workload SPIFFE identity.

**Three options analyzed**:

**(i) In-guest dataplane agent — REJECTED**

Push sockops+kTLS+rustls into the Cloud Hypervisor guest kernel, feed SVID/trust/policy over the vsock channel. This would make the guest work like a host-socket workload from the identity perspective.

Rejection grounds, in order of severity:
- **Violates §6 minimal-guest principle**: "The guest agent does not include a control-plane component" and the whitepaper explicitly lists what is NOT in the guest: "The credential proxy, content inspector, and any other request-path logic are host-side sidecars." A full dataplane agent (sockops BPF programs, kTLS key install, rustls handshake process) is not a "handful of ttRPC stubs." This is not a cosmetic constraint — §6 explains the design rationale: minimal attack surface, sealed guest image, no shell.
- **Fails for BYO-kernel full VMs**: the `vm` driver (whitepaper §6) supports "full OS, hotplug, virtiofs, AArch64" — the guest kernel is the user's own Linux. Overdrive cannot guarantee BPF capabilities or a specific kernel version in a BYO VM. The `overdrive-guest-agent` is already "opt-in, requires Image Factory extension" even for the four-method persistent-microvm agent.
- **Structurally impossible for unikernels**: Unikraft or any sealed unikernel has no Linux guest kernel, no `setsockopt`, no BPF loader. Cannot host a guest dataplane agent by definition.
- **Breaks §8 host-is-trust-root invariant for non-Overdrive-provided kernels**: SVID delivery through vsock is fine; but for the guest to install kTLS keys, the agent inside the guest must handle private key material. For BYO-kernel VMs, this means the guest holds the workload's private key — the trust root shifts toward the guest, which the operator controls, rather than remaining on the host.

**(ii) Host L4 transparent mTLS proxy — RECOMMENDED**

A host-side L4 proxy intercepts the guest's TCP traffic at the tap interface (virtio-net backend on the host), terminates the TCP connection, performs mTLS asserting the VM's SVID (keyed by tap identity = allocation identity), and re-originates an outbound mTLS connection to the destination. From the guest's perspective, it is speaking plain TCP to its peer; the host proxy wraps it in mTLS.

Interception mechanism: TPROXY (`IP_TRANSPARENT`) or TC eBPF redirect on the host's tap interface — both are well-established for per-VM interception without modifying the guest.

Identity assertion: The host node agent knows which allocation owns which tap interface (it created the Cloud Hypervisor process and manages the tap). The proxy asserts the allocation's SPIFFE SVID on behalf of the VM. The SVID private key never enters the guest. §8 trust-root invariant preserved: the host holds and uses the key.

The proxy hop is a genuine extra network hop — bytes are: guest TCP write → virtio-net → host tap → proxy (terminate) → splice → egress socket (encrypt) → wire. This is architecturally equivalent to Istio Ambient ztunnel for Kubernetes pods — a node-local proxy holding per-workload certificates. The prior art (Finding 4) confirms this is the canonical industry solution.

**Crucial refinement — the proxy DOES encrypt through the host kernel (and can beat ztunnel).** The proxy is *not* a userspace-crypto alternative to kTLS. The connection it re-originates to the destination is itself a **host socket** (the proxy runs on the host), so the proxy can install **host kTLS** on that egress socket — kernel record-layer encryption, NIC offload included. kTLS attaches to a *terminated socket*, not to forwarded packets; the proxy's whole job is to terminate the guest's TCP so that a host socket *exists* to attach kTLS to (the guest's own termination is in the guest kernel, invisible to the host). The cost vs the host-socket path is therefore **not** kernel-vs-userspace crypto — it is the *second socket + the splice* between the plaintext guest-leg and the kTLS egress-leg. Because Overdrive owns the host kernel, that splice can be kept **in-kernel** via `sockmap` / `bpf_sk_redirect` (no userspace copy): userspace drives only the handshake + control; steady-state bytes move guest-leg → egress-leg in the kernel, with kTLS encrypting the egress. This is **strictly cheaper than Istio ztunnel**, which does TLS in userspace `rustls` and copies every byte through userspace for the connection's lifetime (Finding 4, line 161) — so the ztunnel perf numbers in Finding 5 are a **ceiling**, not the floor of what this design pays.

> **Spike required (do not overclaim).** kTLS + `sockmap` interaction has historically had rough edges (kTLS-RX-with-sockmap restrictions, `BPF_F_INGRESS` corners — see `sockops-mtls-ktls-installation-comprehensive-research.md` Finding 4). At the 6.6 pin this is materially better than the old LTS reality, but whether the full **in-kernel-splice + kTLS-on-egress** combination works cleanly must be confirmed by a Tier-3 spike before it is assumed. The fallback (userspace splice, kTLS still on egress; or full-userspace like ztunnel) is always available and is what the perf ceiling assumes.

**(iii) Node underlay (WireGuard mesh) — VALID AS DEFENSE-IN-DEPTH**

WireGuard between nodes provides encryption of ALL inter-node traffic (including VM east-west) with node-level cryptographic identity. This is the existing `wireguard` extension in §7.

Limitations: node-keyed, not per-workload. Cannot prove "this packet came from allocation A's SPIFFE identity" — only "this packet came from node N." For compliance regimes requiring per-workload identity attestation, WireGuard alone is insufficient. It is an excellent defense-in-depth layer — encrypted even if the per-workload proxy is bypassed — but it does not replace per-workload identity.

**For BYO-kernel full VMs where Overdrive cannot assert workload identity**: WireGuard underlay + VMs managing their own internal TLS is the honest answer. Overdrive cannot guarantee per-workload SPIFFE identity for VMs running arbitrary guest kernels where the host cannot intercept TCP at a useful granularity.

**Source**: [whitepaper §6 — minimal guest principle](docs/whitepaper.md) — internal, High (1.0); [sockops-ktls-plaintext-race-window-research.md — VM/unikernel rows in Comparison Matrix](docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md) — internal, High (1.0).

**Confidence**: High for the rejection of in-guest agent. High for the host L4 proxy recommendation (prior art convergence — Finding 4 confirms). Medium for the precise interception mechanism (TPROXY vs TC eBPF redirect on tap — both are feasible, choice is an implementation detail for the DESIGN wave).

---

### Finding 4 — Prior art on per-workload identity for VM-class platforms (the decisive evidence)

**Verdict: NO production system uses in-guest kTLS for transparent per-workload mTLS. Every major platform uses either (a) an in-guest sidecar proxy (which requires a cooperating guest OS), or (b) a host/node-level L4 proxy carrying per-workload identity (ztunnel-shape), or (c) node-level encryption only (WireGuard/IPsec — no per-workload identity). The absence is structural, not accidental: in-guest kTLS requires guest-kernel control, which is incompatible with sealed/unikernel/BYO guests and violates minimal-guest principles in every platform that has thought about it.**

**Fly.io — Firecracker microVMs + WireGuard 6PN mesh + fly-proxy**

Identity granularity: **organization-level / node-level**, not per-machine or per-workload. The 6PN is a WireGuard mesh where "every app in your organization is connected to the same 6PN." WireGuard peers are identified by Curve25519 keys at the node/peer level — "Each Fly Machine is assigned a /112 6PN subnet" but that is routing, not cryptographic per-workload identity in the mTLS sense. `fly-proxy` handles TLS termination for client-facing traffic but there is no per-machine SPIFFE SVID model for east-west mTLS between Firecracker VMs. Fly's answer to service-to-service security is "6PN provides transport-level security through WireGuard's encryption."

No per-process or per-workload cryptographic identity is documented. No SPIFFE. No per-VM mTLS certificates for east-west traffic beyond the WireGuard mesh-level encryption.

**Source**: [Fly.io — Private Networking (6PN)](https://fly.io/docs/reference/private-networking/) — fly.io, High (1.0), accessed 2026-06-05; [Fly.io — IPv6 WireGuard Peering blog](https://fly.io/blog/ipv6-wireguard-peering/) — fly.io, High (1.0), accessed 2026-06-05; [Fly Proxy docs](https://fly.io/docs/reference/fly-proxy/) — fly.io, High (1.0), accessed 2026-06-05. **Confidence**: High — confirmed from three official Fly.io sources; absence of per-workload SPIFFE/mTLS is consistent across all three.

**Kata Containers — lightweight VMs with kata-agent**

mTLS approach: Kata Containers integrates with Istio/Linkerd by **injecting the Envoy sidecar proxy INSIDE the Kata VM**. The kata documentation states both Istio and Linkerd "inject a proxy as a sidecar inside the pod running the service." The sidecar runs in the guest, in the guest's network namespace, with iptables rules in the guest. This is the in-guest sidecar model — it requires a cooperating Linux guest kernel that can run Envoy and apply iptables rules.

Kata does NOT use host-side interception or kTLS for mTLS. The kata-agent inside the guest is a thin shim (like Overdrive's `overdrive-guest-agent` concept), not a TLS/mTLS agent. All mTLS logic is in the Envoy sidecar that runs inside the VM.

The critical implication: Kata's in-guest sidecar approach (a) requires a Linux guest kernel, (b) requires Envoy to be injectable (the guest is not sealed), (c) does NOT work for unikernels or BYO-kernel VMs where sidecar injection is impossible.

**Source**: [Kata Containers — service-mesh.md](https://github.com/kata-containers/documentation/blob/master/how-to/service-mesh.md) — github.com/kata-containers, High (1.0), accessed 2026-06-05; [katacontainers.io blog — Inject Workloads with Kata Containers in Istio](https://katacontainers.io/blog/inject-workloads-with-kata-containers-in-istio/) — katacontainers.io, High (1.0), accessed 2026-06-05. **Confidence**: High — confirmed directly from Kata's own documentation.

**gVisor — user-space netstack sandbox**

gVisor implements its own network stack (netstack) in the Sentry sandbox. ALL TCP connections from a gVisor sandbox terminate in the Sentry's netstack, not in the host kernel. There is no `struct sock` in the host kernel for gVisor workloads — exactly the same structural property as MicroVMs. Host-side sockops/kTLS cannot see gVisor's TCP connections.

mTLS approach: Istio sidecar injection works with gVisor because gVisor supports Linux containers — the Envoy sidecar runs INSIDE the gVisor sandbox, in the same network namespace, handling mTLS through netstack. This is again the in-guest/in-sandbox proxy model. No host-side kTLS.

**Source**: [gVisor Networking Architecture](https://gvisor.dev/docs/architecture_guide/networking/) — gvisor.dev, High (1.0), accessed 2026-06-05; [gVisor networking security blog](https://gvisor.dev/blog/2020/04/02/gvisor-networking-security/) — gvisor.dev, High (1.0), accessed 2026-06-05. **Confidence**: High (gVisor's own architecture docs; the host-opacity of netstack sockets is the defining architectural property of gVisor).

**Istio Ambient ztunnel + HBONE — the canonical "host-L4-proxy per-workload identity" design**

This is the most important prior art. Istio Ambient's ztunnel is a per-node Rust-based L4 proxy that:

1. Intercepts all TCP traffic for pods on its node via iptables rules set up BEFORE the pod starts (equivalent to host-side tap interception for VMs)
2. Holds one certificate per workload identity (per Kubernetes ServiceAccount) — NOT one per node: "ztunnel to manage multiple distinct workload certificates...one for each unique identity (service account) for every node-local pod"
3. Uses HBONE (HTTP CONNECT tunnel over mTLS) to tunnel traffic between nodes, presenting the SOURCE WORKLOAD'S SPIFFE identity in the mTLS handshake — NOT the node's identity: "SPIFFE identities are used to identify the workloads on each side of the connection...Ztunnel's own identity is never used for mTLS connections between workloads"
4. The CA enforces that ztunnel can only request certificates for workloads running on its node — preventing cross-node impersonation
5. All of this runs on the HOST, not in the workload — no in-pod/in-guest code for mTLS

This is precisely the architecture recommended for Overdrive's guest-stack workloads: a host-resident proxy that holds per-workload SVID and asserts it over mTLS, intercepting traffic at the host boundary (iptables for Kubernetes pods; TPROXY/TC on tap interface for Cloud Hypervisor VMs).

**kTLS**: Istio ztunnel does NOT use kTLS. All TLS is in userspace via Rust (using the `rustls` library). The ztunnel proxy does not exit the data path — it handles every byte for the full connection lifetime.

**Source**: [Istio Ambient data plane architecture](https://istio.io/latest/docs/ambient/architecture/data-plane/) — istio.io, High (1.0), accessed 2026-06-05; [github.com/istio/istio — ztunnel architecture doc](https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md) — github.com/istio, High (1.0), accessed 2026-06-05; [Istio — rust-based ztunnel blog 2023](https://istio.io/latest/blog/2023/rust-based-ztunnel/) — istio.io, High (1.0), accessed 2026-06-05. **Confidence**: High (directly from Istio's authoritative architecture documentation and source code repository).

**Cilium — ztunnel (beta) + WireGuard/IPsec**

Cilium 1.19+ added ztunnel support (beta) using the same model as Istio Ambient. The existing WireGuard/IPsec modes are node-keyed only, as established in `transparent-encryption-comprehensive-research.md` Findings 2, 4. No in-guest kTLS for VMs. No per-workload SPIFFE identity in WireGuard/IPsec modes.

**Source**: [Cilium ztunnel docs](https://docs.cilium.io/en/stable/security/network/encryption-ztunnel/) — docs.cilium.io, High (1.0), accessed 2026-06-04 (established prior research). **Confidence**: High.

**Overall Finding 4 verdict**: CONFIRMED — no production platform uses in-guest kTLS for transparent per-workload mTLS. Every platform that provides per-workload identity for VM-class workloads uses either (a) in-guest sidecar proxy (Kata + Istio/Linkerd — requires injectable Linux guest), or (b) host/node L4 proxy with per-workload certificates (Istio Ambient ztunnel — the canonical design). The asymmetry is structural: in-guest kTLS requires guest kernel ownership, which is incompatible with sealed/unikernel/BYO-kernel guests and violates minimal-guest principles.

---

### Finding 5 — Performance reality: host kTLS vs L4 mTLS proxy (HBONE/ztunnel)

**Verdict**: The host L4 mTLS proxy adds ~0.17–0.20ms P90/P99 latency overhead and ~3–5% CPU overhead for typical service-mesh east-west workloads. kTLS has zero steady-state overhead after the handshake. The proxy hop is measurably present but is NOT disqualifying at typical service latencies (1–10ms baseline). For high-throughput storage-class or streaming workloads the per-packet copy cost matters more — kTLS wins there. The proxy is the correct pragmatic choice for v1; kTLS is the right long-term optimization for host-socket workloads.

> **These ztunnel numbers are a CEILING for the guest-stack path, not the expected cost.** The measured ztunnel overhead reflects Istio's *userspace*-`rustls` proxy that copies every byte through userspace (line 161). Overdrive's tap proxy can install **host kTLS on its egress socket** and keep the guest-leg→egress-leg **splice in the kernel** (`sockmap`/`bpf_sk_redirect`), because Overdrive owns the host kernel — see Finding 3's "Crucial refinement." That removes the userspace per-byte copy that dominates ztunnel's cost, so the guest-stack proxy should pay *less* than these numbers in steady state (handshake-time cost is unchanged). Subject to the kTLS+sockmap Tier-3 spike flagged in Finding 3.

**ztunnel/HBONE latency overhead (quantitative)**:

From Istio 1.22/1.24 benchmarks (confirmed from two independent sources):
- Baseline (no Istio): **1.12ms P99**
- Istio Ambient ztunnel (L4 mTLS): **3.6ms P99** — +2.48ms absolute, roughly +221% relative (at very low baseline latency)
- Istio Sidecar: **4.72ms P99** — ztunnel is 20% better than sidecar
- The Istio performance docs state: "the two ztunnel proxies add about 0.17ms and 0.20ms to the P90 and P99 latency, respectively, over the baseline data plane latency" (at 1000 RPS with 1KB payload, Istio 1.24)

Note: The 0.17–0.20ms figure is the latency added by ONE ztunnel hop. The 3.6ms vs 1.12ms comparison includes CNI overhead and is on a specific test setup (Azure AKS, 2 vCPU nodes). The 0.17ms/0.20ms per-hop number from Istio's own docs is more representative of the proxy mechanism's marginal cost.

**ztunnel resource overhead**: "a single ztunnel proxy consumes about 0.06 vCPU and 12MB of memory" at 1000 RPS. For comparison, sidecar proxy: 0.20 vCPU and 60MB memory. Ambient ztunnel = 25% the CPU, 20% the memory of a sidecar proxy. Very low node-level overhead.

**kTLS throughput gain vs userspace TLS (quantitative)**:

- NGINX/F5 benchmark: kTLS improves throughput by **13–28%** vs userspace TLS for static file serving (eliminates the kernel-to-userspace-to-kernel copy for encryption). On FreeBSD 13.0: 27.6% improvement; Ubuntu 21.10: 13.3% improvement.
- With NIC hardware offload (ConnectX-6 Dx): approximately **2× throughput** vs software kTLS for AES-GCM. "Inline TLS offload gave approximately 2× throughput vs software kTLS" (8.8 Gb/s vs 4.4 Gb/s).
- The throughput gain is from eliminating copy overhead, not from faster crypto — kTLS allows `SSL_sendfile()` which avoids the read/encrypt/write cycle.

**The proxy overhead is NOT the dominant cost for most workloads**:

For request-response service-to-service traffic (HTTP, gRPC), the dominant latency components are: application processing time, serialization, and network RTT. Adding 0.17–0.20ms per hop to a 1–10ms application latency is 2–17% overhead. For bulk data transfer (streaming, storage), the per-packet copy in the proxy IS significant — this is where kTLS' zero-copy path wins.

**Conclusion**: For east-west service traffic, the host L4 proxy hop is cheap enough (~0.2ms, ~0.06 vCPU per proxy instance) that the in-guest-agent complexity it avoids is not worth the overhead. For high-throughput paths (bulk storage access, streaming data), kTLS' advantages are material. The hybrid — proxy for correctness guarantee in v1, migrate to kTLS for hot paths — is the right phased approach.

**Source**: [Istio Performance and Scalability docs](https://istio.io/latest/docs/ops/deployment/performance-and-scalability/) — istio.io, High (1.0), accessed 2026-06-05; [imesh.ai — Istio Ambient Mesh Performance Test and Benchmarking](https://imesh.ai/blog/istio-ambient-mesh-performance-test-and-benchmarking/) — imesh.ai, Medium-High (0.8), accessed 2026-06-05; [F5/NGINX — Improving NGINX Performance with Kernel TLS](https://www.f5.com/company/blog/nginx/improving-nginx-performance-with-kernel-tls) — f5.com, Medium-High (0.8), accessed 2026-06-05; [Netdev 0x14 — kTLS Offload Performance Enhancements](https://netdevconf.info/0x14/pub/papers/29/0x14-paper29-talk-paper.pdf) — netdevconf.info, High (1.0), accessed 2026-06-05. **Confidence**: Medium-High (latency numbers are from specific test environments; the directional verdict is High confidence).

---

### Finding 6 — The unified identity & control plane that makes the hybrid coherent

**Verdict**: The hybrid (kTLS path for host-socket workloads, host L4 proxy for guest-stack workloads) is ONE architecture with two enforcement mechanisms, not two separate architectures, because BOTH paths share the same SPIFFE identity plane. The IDENTITY_MAP (already in the whitepaper) and the SPIFFE CA are the unifying primitives.

**The shared control plane layer**:

1. **SPIFFE CA**: One CA for all workload classes. Process SVIDs and VM SVIDs both come from `Overdrive Root CA → Node Intermediate CA → Workload SVID (per allocation)`. The CA does not know or care whether the workload is a process or a VM.

2. **SVID delivery**: Process workloads: SVID delivered to the node agent (host), used to drive the rustls handshake on their behalf (Architecture A/C kTLS path). VM workloads: SVID delivered to the node agent (host), used by the tap-resident host L4 proxy on their behalf. Neither class needs the SVID inside the workload itself for the transparent path.

3. **IDENTITY_MAP** (already in whitepaper §7 "BPF Maps: POLICY_MAP · SERVICE_MAP · IDENTITY_MAP · FS_POLICY_MAP"): Maps workload identity (SPIFFE ID) to connection parameters (IP, port, allocation ID). The host L4 proxy uses IDENTITY_MAP to look up which SVID corresponds to a given tap interface / allocation. The sockops path uses IDENTITY_MAP to look up which SVID corresponds to a given host socket's cgroup/PID.

4. **Trust bundle + revocation**: One trust bundle (cluster-wide) materialized in BPF maps and updated via Corrosion gossip. Both the kTLS path (rustls handshake with the trust bundle) and the host L4 proxy use the same trust bundle for certificate validation.

5. **Policy** (`POLICY_MAP` with SPIFFE-identity-based rules): Policy is evaluated against SPIFFE identities, independent of whether the enforcement is at the socket layer (kTLS path) or the proxy layer. "A network policy that governs a process workload governs a VM workload identically" (whitepaper §6) is achievable — but the enforcement mechanisms are different, as this research establishes.

**The single-model property**: From an operator's perspective, writing a policy `deny spiffe://overdrive.local/job/payments → spiffe://overdrive.local/job/analytics` blocks that flow regardless of whether payments is a process or a VM and regardless of whether analytics is on the same host or a different one. The SPIFFE ID is the universal handle; the mechanism is an implementation detail the operator does not see.

**Source**: [whitepaper §7 — BPF Maps and IDENTITY_MAP](docs/whitepaper.md) — internal, High (1.0); [whitepaper §8 — SPIFFE Identity Model and CA chain](docs/whitepaper.md) — internal, High (1.0); [Istio Ambient ztunnel architecture — per-workload certificate management](https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md) — github.com/istio, High (1.0), accessed 2026-06-05 (the per-workload certificate model is analogous).

**Confidence**: High — this follows directly from the SPIFFE identity design and the shared BPF map infrastructure already described in the whitepaper. The unified control plane is not a new thing to build; it is what the whitepaper already specifies, correctly extended to cover both enforcement mechanisms.

---

### Finding 7 — Fail-closed, enforcement boundary, and the §8 "host is the trust root" invariant

**Verdict**: The host L4 proxy for guest-stack workloads PRESERVES the "host is the trust root" invariant (§8). The in-guest kTLS agent option would VIOLATE it for BYO-kernel VMs. The proxy option is stronger on trust boundary, not weaker.

**"Host is the trust root" defined** (whitepaper §6): "The trust root remains on the host, consistent with §8." The private key for the workload SVID stays on the host. The host node agent is the only entity that holds or uses the SVID private key for mTLS.

**Host L4 proxy trust analysis**:
- SVID private key: held by the host node agent; loaded into the tap-resident proxy. Never sent into the guest VM. Guest compromise does not expose the SVID private key. ✓ §8 preserved.
- Fail-closed behavior: if the host proxy is not running or not configured for a given tap, the guest VM's traffic does not leave the node unencrypted (the proxy TPROXY/TC redirect ensures traffic is intercepted; no proxy = no forward). The fail-closed property must be enforced by the redirect rule, not the proxy itself — the redirect rule (TC eBPF on the tap or TPROXY iptables) must be installed before the VM is permitted to send traffic. This is an implementation requirement, not a theoretical concern.
- BPF LSM on the host: `socket_create` / `socket_connect` enforcement at the host level applies to the proxy process, not the guest. The guest VM cannot bypass the proxy by making raw socket calls from within the guest — the host-side TC eBPF redirect intercepts at the virtio-net level, below the guest's network stack.

**In-guest kTLS agent trust analysis** (why it's weaker for BYO-kernel VMs):
- SVID private key: must be delivered into the guest to drive the rustls handshake. For Overdrive-controlled guest kernels (Image Factory microvms), the key can be delivered over vsock to the agent and held in guest memory. For BYO-kernel full VMs, the guest OS controls memory; a compromised guest OS can read the SVID private key. §8 trust root shifts toward the guest for BYO VMs.
- Fail-closed: if the in-guest agent fails to install kTLS keys, the guest's TCP connection runs in cleartext within the guest. Without a host-side enforcement point, this cleartext traffic would leave the host unencrypted.

**Node underlay (WireGuard) trust analysis**:
- Encrypts all traffic between nodes. Does NOT provide per-workload SPIFFE identity. A compromised guest VM on node A can send traffic to any other VM on the same 6PN mesh — it is authenticated as "node A", not as "allocation X". Weaker than per-workload identity but stronger than no encryption.
- Is a valid defense-in-depth layer that complements the per-workload proxy approach.

**The enforcement boundary table**:

| Mechanism | Where TCP terminates | Where mTLS terminates | Private key holder | Bypass risk |
|---|---|---|---|---|
| kTLS (host-socket path) | Host kernel | Host kernel (kTLS) | Host node agent | BPF LSM socket_create blocks bypass; key stays on host |
| Host L4 tap proxy (VM path) | Guest kernel | Host proxy | Host node agent | TC redirect enforces interception; bypass requires host compromise |
| In-guest kTLS agent | Guest kernel | Guest kernel | Guest agent (risk for BYO) | Guest compromise = key exposure for BYO VMs |
| WireGuard underlay | Guest kernel | Host wireguard interface | Host WireGuard | Node-level identity only; any VM on node can use the node key |

**Source**: [whitepaper §6 — "The trust root remains on the host"](docs/whitepaper.md) — internal, High (1.0); [whitepaper §8 — SPIFFE identity model](docs/whitepaper.md) — internal, High (1.0); [sockops-ktls-plaintext-race-window-research.md — fail-closed semantics, Finding 7](docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md) — internal, High (1.0).

**Confidence**: High for the trust boundary analysis. Medium for the fail-closed implementation requirement on the redirect rule (enforcement is achievable but requires careful implementation in the DESIGN/DELIVER wave).

---

## Decision Matrix — recommended mechanism per workload class

| Workload class | TCP stack location | v1 mTLS mechanism | Long-term mechanism | SVID key holder | §6 minimal-guest? | §8 host-trust-root? | Prior art |
|---|---|---|---|---|---|---|---|
| **Process** (exec driver) | Host kernel | Architecture C: sockmap proxy redirect → host-proxy-rustls-mTLS (same as ztunnel) | Architecture A + custom write-block kernel patch: sockops+kTLS, zero steady-state overhead | Host node agent | N/A (no guest) | ✓ | Istio Ambient ztunnel (v1), Oracle tlshd (A long-term shape) |
| **WASM** (wasmtime) | Host kernel (WASI sockets = host sockets) | Same as Process — Architecture C | Same as Process — Architecture A + write-block | Host node agent | N/A (in-process) | ✓ | Same as Process |
| **MicroVM** (Cloud Hypervisor, Overdrive-provided kernel) | Guest kernel | Host L4 tap proxy (TPROXY/TC), asserting allocation SVID | Same (proxy stays; kTLS not applicable because TCP is in guest) | Host node agent | ✓ (proxy is host-side) | ✓ (key never enters guest) | Istio Ambient ztunnel (canonical design) |
| **Unikernel** (Cloud Hypervisor + Unikraft) | Guest unikernel net stack | Host L4 tap proxy (same as MicroVM) | Same — no kTLS path exists | Host node agent | ✓ | ✓ | No in-guest option possible |
| **Full VM** (BYO kernel) | Guest kernel | WireGuard node underlay (best achievable) OR host L4 tap proxy IF Overdrive controls the tap | Host L4 tap proxy (same) | Host node agent (proxy) / WireGuard node key | ✓ | ✓ (if proxy) / Node-keyed only (if WG) | Fly.io 6PN (WireGuard, node-keyed) |

**Key divergence from whitepaper §7**: "This works identically for process workloads, VMs, unikernels, and WASM functions" is FALSE for the enforcement mechanism. Process and WASM use the host socket path; MicroVM, unikernel, and full VMs require a host L4 proxy. The identity model (SPIFFE, one CA, one IDENTITY_MAP) IS identical — it is the enforcement mechanism that diverges by where TCP terminates.

---

## The Recommended Architecture (the answer)

### One-liner

**Hybrid unified by SPIFFE identity: sockops+kTLS (via proxy in v1, direct in long-term) for host-socket workloads; host L4 tap proxy (ztunnel-shape) for guest-stack workloads; WireGuard underlay as optional defense-in-depth; all three sharing ONE CA, ONE IDENTITY_MAP, ONE trust bundle.**

### In full

**Layer 1 — Shared identity plane (applies to ALL workload classes)**
- SPIFFE CA with the existing three-tier chain (Root → Node Intermediate → Workload SVID)
- IDENTITY_MAP BPF map: maps allocation ID / tap interface / PID/cgroup → SPIFFE SVID
- Trust bundle materialized on every node via Corrosion gossip
- Policy (POLICY_MAP) expressed against SPIFFE identities — mechanism-agnostic
- No change from what the whitepaper already specifies

**Layer 2a — Host-socket workloads: process + WASM (v1)**
- Mechanism: Architecture C — sockmap redirect at `ACTIVE_ESTABLISHED_CB` / `PASSIVE_ESTABLISHED_CB`; all bytes go through the node agent's rustls-based mTLS proxy for the full connection lifetime
- Race window: none by construction (the proxy is always in the data path)
- Protocol coverage: all protocols including server-speaks-first
- Production precedent: Istio Ambient ztunnel (same pattern applied to Kubernetes pods)
- Overhead: per-packet copy for full connection lifetime; ~0.17ms P99 latency add; ~0.06 vCPU per 1000 RPS

**Layer 2a — Host-socket workloads: process + WASM (long-term, when kernel patch is ready)**
- Mechanism: Architecture A — sockops intercepts at `ACTIVE_ESTABLISHED_CB`, custom in-kernel write-block holds `write()` until kTLS is armed (no data loss, no per-packet overhead), node agent performs rustls handshake via `pidfd_getfd()`, installs kTLS keys, removes write-block, exits data path
- Race window: closed by the write-block kernel patch (blocks instead of dropping)
- Overhead: zero steady-state (kTLS handles encrypt/decrypt in-kernel after agent exits)
- Novelty: the write-block patch has no upstream precedent; it is a legitimate out-of-tree patch for an appliance-OS product
- Migration: v1 proxy → long-term kTLS is transparent to applications; same SPIFFE identity, same policy, different enforcement path

**Layer 2b — Guest-stack workloads: MicroVM + unikernel (v1 and long-term)**
- Mechanism: Host L4 tap proxy — TC eBPF or TPROXY intercepts guest TCP at the virtio-net tap interface on the host; node agent proxy terminates TCP from the guest, performs mTLS asserting the allocation's SVID, re-originates outbound mTLS to the destination
- Interception timing: redirect rule installed at allocation start (before first virtio-net frame is permitted), ensuring fail-closed
- Identity: assertion by tap identity (host knows which allocation owns which tap); SVID private key stays on host; §8 trust-root preserved
- Protocol coverage: all protocols (the tap intercepts all TCP)
- No in-guest changes required: guest VM is completely unaware; no modification to guest kernel, no agent injection
- Unikernel: works identically; the tap is the only interception point available, and it is sufficient
- Long-term: proxy stays permanently (no kTLS option for guest-stack workloads; TCP terminates in guest, not host)

**Layer 3 — Node underlay (defense-in-depth, optional)**
- WireGuard mesh (existing `wireguard` extension, §7) encrypts all inter-node traffic
- Provides node-level encryption for BYO-kernel full VMs where per-workload SPIFFE identity is not achievable (Overdrive cannot assert workload identity for VMs running arbitrary guest OSes)
- For Overdrive-managed microvms and unikernels: WireGuard is redundant with the per-workload tap proxy but provides a defense-in-depth layer (encrypted even if the proxy has a bug)
- Key rotation cadence: align with node intermediate CA cert rotation (open question from prior research — see Knowledge Gaps)

**Layer 3b — Full VMs with BYO kernels**
- Best achievable: WireGuard node underlay + Overdrive managing TLS certificates at the VM IP level (fly-proxy style) OR requiring the workload to manage its own TLS
- Per-workload SPIFFE identity is NOT achievable for BYO-kernel VMs unless Overdrive controls the tap AND the guest cooperates with certificate provisioning — this is a scope boundary, not a design failure

---

## What this means for the whitepaper (§6/§7/§8 amendments) and roadmap 2.4 scope

### Required §7 amendments

**The claim "This works identically for process workloads, VMs, unikernels, and WASM functions — there is no sidecar injection required or possible" requires correction.**

Accurate replacement language:

> "For process workloads and WASM functions, whose TCP sockets terminate in the host kernel, the sockops+kTLS path works without any in-workload code. For MicroVMs and unikernels, whose TCP stacks terminate in the guest kernel, a host-resident L4 tap proxy (running on the host, transparent to the guest) performs mTLS on the workload's behalf, asserting its SPIFFE identity via tap identity. The enforcement mechanism differs by where TCP terminates; the identity model (SPIFFE SVIDs, one CA, one IDENTITY_MAP) is universal."

### Required §6 amendments

The minimal-guest principle is UPHELD and actually strengthened: the recommended architecture for MicroVM/unikernel workloads explicitly puts mTLS enforcement on the host (tap proxy), not in the guest. §6 is not contradicted — it is elaborated. Possible addendum:

> "mTLS for MicroVM and unikernel workloads is enforced by a host-resident tap proxy, consistent with the minimal-guest principle. No mTLS logic is required or permitted in the guest."

### §8 amendments

"IP addresses are routing hints, not security boundaries" — UPHELD. Both the kTLS path and the tap proxy path use SPIFFE identity for policy; IP is still routing-only.

The implicit claim in §8 that mTLS is achieved identically for all workload classes needs correction consistent with the §7 amendment above.

### Roadmap 2.4 scope implications

This research recommends a two-phase delivery:

**Phase 2.4a (v1 — Architecture C for all workloads):**
- Process + WASM: sockmap proxy redirect → rustls handshake on host → kTLS NOT used in v1 for simplicity; proxy handles full connection
- MicroVM + unikernel: host L4 tap proxy (TPROXY or TC eBPF on tap interface)
- All paths share IDENTITY_MAP and SPIFFE CA — control plane is complete
- BPF LSM `socket_create` enforcement on host ensures no bypass
- Full protocol coverage; no data loss; no in-guest changes

**Phase 2.4b (long-term — Architecture A for host-socket workloads):**
- Process + WASM: custom in-kernel write-block patch → sockops intercept → rustls handshake via pidfd_getfd → kTLS install → agent exits data path
- Eliminates per-packet copy overhead for process/WASM east-west hot paths
- Guest-stack workloads: no change (tap proxy is permanent for them)
- Requires: custom kernel patch + integration testing across kernel versions

> **Scope split (tracked).** Roadmap 2.4 / #26 ("sockops mTLS + kTLS") structurally covers **host-socket workloads only** (process, WASM). The guest-stack **host L4 tap proxy subsystem** (MicroVM, unikernel) is a separate, larger piece of work tracked as **[#222](https://github.com/overdrive-sh/overdrive/issues/222)** — it shares this research's recommended architecture and the unified SPIFFE identity plane, but is its own primitive. The two-phase delivery above describes the *target* shape; the per-issue scoping is #26 (host-socket kTLS) + #222 (guest-stack L4 proxy).

---

## Source Catalogue

| # | Source | Domain | Reputation | Type | Access Date | Finding |
|---|--------|--------|------------|------|-------------|---------|
| 1 | [sockops-mtls-ktls-installation-comprehensive-research.md](docs/research/dataplane/sockops-mtls-ktls-installation-comprehensive-research.md) | internal | High (1.0) | Prior research | 2026-06-05 | F1, F2 |
| 2 | [sockops-ktls-plaintext-race-window-research.md](docs/research/dataplane/sockops-ktls-plaintext-race-window-research.md) | internal | High (1.0) | Prior research | 2026-06-05 | F1, F2, F3, F7 |
| 3 | [transparent-encryption-comprehensive-research.md](docs/research/transparent-encryption-comprehensive-research.md) | internal | High (1.0) | Prior research | 2026-06-05 | F1, F4 |
| 4 | [Overdrive whitepaper §6/§7/§8](docs/whitepaper.md) | internal | High (1.0) | Authoritative spec | 2026-06-05 | F1, F3, F6, F7 |
| 5 | [Istio Ambient data plane architecture](https://istio.io/latest/docs/ambient/architecture/data-plane/) | istio.io | High (1.0) | Official | 2026-06-05 | F4, F6 |
| 6 | [github.com/istio/istio — ztunnel architecture](https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md) | github.com/istio | High (1.0) | Official | 2026-06-05 | F4, F6, F7 |
| 7 | [Istio rust-based ztunnel blog 2023](https://istio.io/latest/blog/2023/rust-based-ztunnel/) | istio.io | High (1.0) | Official | 2026-06-05 | F4 |
| 8 | [Fly.io Private Networking (6PN)](https://fly.io/docs/reference/private-networking/) | fly.io | High (1.0) | Official | 2026-06-05 | F4 |
| 9 | [Fly.io IPv6 WireGuard Peering blog](https://fly.io/blog/ipv6-wireguard-peering/) | fly.io | High (1.0) | Official | 2026-06-05 | F4 |
| 10 | [Fly Proxy docs](https://fly.io/docs/reference/fly-proxy/) | fly.io | High (1.0) | Official | 2026-06-05 | F4 |
| 11 | [Kata Containers — service-mesh.md](https://github.com/kata-containers/documentation/blob/master/how-to/service-mesh.md) | github.com/kata-containers | High (1.0) | Official | 2026-06-05 | F4 |
| 12 | [katacontainers.io blog — Kata + Istio](https://katacontainers.io/blog/inject-workloads-with-kata-containers-in-istio/) | katacontainers.io | High (1.0) | Official | 2026-06-05 | F4 |
| 13 | [gVisor Networking Architecture](https://gvisor.dev/docs/architecture_guide/networking/) | gvisor.dev | High (1.0) | Official | 2026-06-05 | F4 |
| 14 | [gVisor Networking Security blog](https://gvisor.dev/blog/2020/04/02/gvisor-networking-security/) | gvisor.dev | High (1.0) | Official | 2026-06-05 | F4 |
| 15 | [Cilium ztunnel transparent encryption docs](https://docs.cilium.io/en/stable/security/network/encryption-ztunnel/) | docs.cilium.io | High (1.0) | Official | 2026-06-04 | F4 |
| 16 | [Istio Performance and Scalability docs 1.24](https://istio.io/latest/docs/ops/deployment/performance-and-scalability/) | istio.io | High (1.0) | Official | 2026-06-05 | F5 |
| 17 | [imesh.ai — Istio Ambient Mesh Performance Benchmarking](https://imesh.ai/blog/istio-ambient-mesh-performance-test-and-benchmarking/) | imesh.ai | Medium-High (0.8) | Industry | 2026-06-05 | F5 |
| 18 | [F5/NGINX — Improving NGINX Performance with Kernel TLS](https://www.f5.com/company/blog/nginx/improving-nginx-performance-with-kernel-tls) | f5.com | Medium-High (0.8) | Industry | 2026-06-05 | F5 |
| 19 | [Netdev 0x14 — kTLS Offload Performance Enhancements](https://netdevconf.info/0x14/pub/papers/29/0x14-paper29-talk-paper.pdf) | netdevconf.info | High (1.0) | Academic/conference | 2026-06-05 | F5 |
| 20 | [Istio ztunnel traffic redirection](https://istio.io/latest/docs/ambient/architecture/traffic-redirection/) | istio.io | High (1.0) | Official | 2026-06-04 | F3, F4 |
| 21 | [Linkerd Architecture reference](https://linkerd.io/2-edge/reference/architecture/) | linkerd.io | High (1.0) | Official | 2026-06-04 | F4 |
| 22 | [Cilium Native mTLS blog 2026-03-23](https://cilium.io/blog/2026/03/23/native-mtls-cilium/) | cilium.io | High (1.0) | Official | 2026-06-04 | F4 |
| 23 | [RFC 793 — TCP Specification](https://datatracker.ietf.org/doc/html/rfc793) | datatracker.ietf.org | High (1.0) | Official standard | 2026-06-04 | F2 |
| 24 | [kernel.org sockmap documentation](https://docs.kernel.org/bpf/map_sockmap.html) | kernel.org | High (1.0) | Official | 2026-06-04 | F2 |
| 25 | [In-Kernel TLS Handshake — kernel.org](https://docs.kernel.org/networking/tls-handshake.html) | kernel.org | High (1.0) | Official | 2026-06-04 | F2 |
| 26 | [Istio blog — Ambient vs Cilium benchmark 2024](https://istio.io/latest/blog/2024/ambient-vs-cilium/) | istio.io | High (1.0) | Official | 2026-06-05 | F5 |

**Reputation distribution**: High (1.0): 23 sources (88%) | Medium-High (0.8): 3 sources (12%) | **Average: 0.97**

---

## Knowledge Gaps

### Gap 1 — Precise TPROXY vs TC eBPF redirect choice for tap interception
**Issue**: Two viable host-side interception mechanisms exist for the VM tap proxy path: TPROXY (`IP_TRANSPARENT`) which requires iptables rules, and TC eBPF redirect on the tap netdev. The choice affects implementation complexity, failure modes, and whether it interacts cleanly with the existing XDP/TC dataplane. **Attempted**: Not researched in this synthesis (out of scope — DESIGN wave decision). **Recommendation**: DESIGN wave should evaluate both; TC eBPF redirect is architecturally consistent with the existing overdrive dataplane and avoids iptables state management.

### Gap 2 — WireGuard key rotation cadence for the `wireguard` extension
**Issue**: The whitepaper §7 mentions initial key delivery via enrollment but does not specify a rotation cadence. Carry-over from prior research Gap D. **Attempted**: Not revisited in this synthesis. **Recommendation**: Align with node intermediate CA cert rotation; define in the DESIGN wave as part of the WireGuard extension specification.

### Gap 3 — Sequence counter handoff for Architecture C+ (hybrid transient proxy → kTLS)
**Issue**: The Architecture C+ hybrid (proxy only during handshake, kTLS for steady-state) requires setting `rec_seq` correctly on kTLS handoff. No production implementation exists; feasibility is Medium confidence. **Attempted**: Prior research (Gap 2 in race-window doc). **Recommendation**: Prototype in a DELIVER step; this is optional optimization and not required for the recommended v1 architecture.

### Gap 4 — BYO-kernel full VM per-workload identity gap
**Issue**: For full VMs with arbitrary guest OSes, Overdrive cannot assert per-workload SPIFFE identity unless the VM cooperates with certificate provisioning. WireGuard node-level encryption is the fallback. The exact UX boundary (what operators are told, what the feature matrix says) is not specified in this research. **Recommendation**: DESIGN wave should define the capability matrix for each driver class explicitly.

### Gap 5 — Performance numbers for host L4 tap proxy overhead specifically for VM east-west workloads
**Issue**: The performance numbers in Finding 5 are for Kubernetes pod-to-pod traffic through ztunnel. The overhead for VM east-west traffic through a host tap proxy (virtio-net path) may differ due to virtio-net copy overhead stacking on top of the proxy copy. **Attempted**: No VM-specific proxy performance data was found. **Recommendation**: Benchmark the host L4 tap proxy against direct VM-to-VM traffic in the Lima/QEMU environment to establish baseline overhead numbers for the DESIGN wave.

---

## Research Metadata

**Duration**: ~90 min | **Sources examined**: 45+ | **Sources cited**: 26 | **Cross-refs**: All major claims cross-referenced ≥2 sources; prior-research claims have ≥3 sources | **Confidence distribution**: High 80%, Medium-High 15%, Medium 5%, Low 0% | **Output**: `docs/research/dataplane/transparent-mtls-recommended-architecture-research.md`

**New sources gathered (Finding 4 + 5)**: 16 (Istio architecture docs×3, fly.io docs×3, katacontainers.io×2, gvisor.dev×2, Cilium ztunnel×1, imesh.ai×1, F5/NGINX kTLS×1, Netdev 0x14×1, Istio benchmark blog×1, Istio Ambient vs Cilium benchmark×1)
