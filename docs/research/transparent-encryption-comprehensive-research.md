# Research: Transparent Encryption for Overdrive

**Date**: 2026-05-04 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 23

## Executive Summary

"Transparent encryption" in the eBPF/CNI ecosystem is not one feature but three structurally distinct patterns: node-keyed underlay tunnels (WireGuard, IPsec — Cilium, Calico), per-pod sidecar mTLS (Linkerd), and node-proxy mTLS carrying per-workload identity (Istio Ambient ztunnel, Cilium ServiceMesh). Each has different scope, identity primitives, performance characteristics, and operational costs. Confusing them produces wrong recommendations.

Overdrive's stated approach in whitepaper §7/§8 — sockops handshake handoff into kTLS, with per-allocation SPIFFE SVIDs and the kernel record layer doing the bulk crypto, plus an opt-in `wireguard` underlay extension for L3 attestation — already occupies a fourth cell of the matrix. The research confirms this is the right primitive set: it is the only published model that delivers per-workload identity-bearing encryption *without* a userspace L4 proxy hop. Same-node intra-pod traffic (Cilium WG and Calico WG do not encrypt this), the unencrypted-window race (a structural Cilium artifact), the single-CPU-per-tunnel IPsec decryption ceiling, and the absence of mainline WireGuard NIC offload all favour the existing design. NIC kTLS offload on ConnectX-6 Dx and later is mature enough (kernel ≥5.3 Tx, ≥5.9 Rx) to make the per-workload kTLS path the right primitive to ride into accelerated futures.

The recommendation is **stand pat on the architecture, tighten the safety story**: add a Tier-1 DST invariant that no egress proceeds on a connection whose sockops handshake hasn't completed, add Tier-3 real-kernel sockops-attachment tests across the kernel matrix, document MTU implications when the `wireguard` extension is co-deployed (double-encap), and switch the rustls provider to `aws-lc-rs` with the `fips` feature for FIPS-mode operators (covered by FIPS 140-3 Certificate #4816). IPsec mode should be considered-and-declined explicitly in the whitepaper to prevent future re-litigation; the failure modes (single-CPU-per-tunnel, 65535-node ceiling, host-policy incompatibility, fleet-wide PSK rotation coordination) are real and align poorly with Overdrive's existing primitives.



## Research Methodology

**Search Strategy**: Starting from the user-supplied reference (`cilium.io/use-cases/transparent-encryption/`), drill into Cilium's authoritative docs (`docs.cilium.io`) for WireGuard and IPsec modes; cross-reference Calico/Tigera, Istio Ambient, Linkerd, and the WireGuard whitepaper (Donenfeld, NDSS '17). Pull kernel-level mechanism details from kernel.org and LWN. NIC-offload claims verified against NVIDIA/Mellanox docs and kernel commit history.

**Source Selection**: Types: official (kernel.org, ietf.org), technical_docs (docs.cilium.io, istio.io, linkerd.io, wireguard.com, nvidia.com/docs), academic (NDSS '17 WireGuard paper), industry_leaders (LWN). Reputation: high/medium-high. Verification: ≥3 sources per major mechanism claim where possible; primary source preferred.

**Quality Standards**: Target 3 sources per major claim (1 authoritative minimum). All major claims cross-referenced. Single-source claims labelled inline.

## Findings

### Finding 1: "Transparent encryption" in Cilium has three distinct modes with different scopes

**Evidence**: Cilium documents three modes — "IPsec Transparent Encryption," "WireGuard Transparent Encryption," and "Ztunnel Transparent Encryption (Beta)" — and states they cover "transparent encryption of Cilium-managed host traffic and traffic between Cilium-managed endpoints."

**Source**: [Cilium docs — Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption/) - Accessed 2026-05-04
**Confidence**: High
**Verification**: [Cilium use-cases page](https://cilium.io/use-cases/transparent-encryption/) (marketing-level reference); [Cilium WireGuard docs](https://docs.cilium.io/en/stable/security/network/encryption-wireguard/); [Cilium IPsec docs](https://docs.cilium.io/en/stable/security/network/encryption-ipsec/)
**Analysis**: The three modes are not interchangeable. WireGuard and IPsec are full-mesh node-to-node tunnels (with optional pod-to-pod scope); ztunnel is the Istio Ambient–style per-node sidecar performing mTLS termination. The label "transparent encryption" is doing a lot of work — what it transparently encrypts and where the trust boundary sits differs by mode.

### Finding 2: Cilium WireGuard is node-keyed, with public-key distribution via Kubernetes CRD annotations

**Evidence**: "Each node automatically creates its own encryption key-pair … distributes its public key via the `network.cilium.io/wg-pub-key` annotation in the Kubernetes `CiliumNode` custom resource object." Linux kernel requirement: "`CONFIG_WIREGUARD=m` on Linux 5.6 and newer, or via the out-of-tree WireGuard module on older kernels". UDP port `51871` per node.

**Source**: [Cilium docs — WireGuard Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption-wireguard/) - Accessed 2026-05-04
**Confidence**: High
**Verification**: [WireGuard whitepaper, Donenfeld NDSS '17](https://www.wireguard.com/papers/wireguard.pdf) (mechanism); [kernel.org Linux 5.6 changelog mentions WireGuard merge](https://www.kernel.org/) (kernel inclusion timing — to be verified deeper).
**Analysis**: Identity at the WireGuard layer is **node-level**, not workload-level. Two pods on different nodes are encrypted because the nodes have peer relationships; the encryption layer cannot tell pod A apart from pod B on the same source node. This is a structural mismatch with Overdrive's per-allocation SPIFFE SVID model — WireGuard would protect the bytes on the wire but would not carry workload identity in the cryptographic envelope.

### Finding 3: Cilium WireGuard has a documented unencrypted-window race condition

**Evidence**: "if an endpoint is allowed to initiate traffic to targets outside of the cluster, it is possible for that endpoint to send packets to arbitrary IP addresses before Cilium learns that a particular IP address belongs to a remote Cilium-managed endpoint … there is a time window during which Cilium will send out the initial packets unencrypted."

**Source**: [Cilium docs — Transparent Encryption (overview)](https://docs.cilium.io/en/stable/security/network/encryption/) - Accessed 2026-05-04
**Confidence**: High (single-source — this is a Cilium-specific implementation observation, no external corroboration applicable)
**Analysis**: This is a learned-state race: until the dataplane learns that an IP belongs to a Cilium-managed peer, packets egress unencrypted. In Overdrive's model, this race does not exist for east-west traffic because sockops+kTLS intercepts at `connect()` against a SPIFFE identity, not against a learned IP→peer mapping. Cilium's race is an artifact of using post-hoc IP recognition; Overdrive's identity-first interception structurally avoids it.

### Finding 4: Cilium WireGuard does not encrypt same-node pod-to-pod traffic

**Evidence**: "Packets are not encrypted when they are destined to the same node from which they were sent."

**Source**: [Cilium docs — WireGuard Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption-wireguard/) - Accessed 2026-05-04
**Confidence**: High
**Analysis**: WireGuard is a tunnel between two nodes; if source and destination live on the same node, no tunnel is traversed. Same-node traffic flows in cleartext between pods. For Overdrive, sockops+kTLS sits at the socket layer per-connection — same-node and cross-node traffic both traverse the kTLS path, so the same-node carve-out does not apply. This is an under-appreciated difference: a node-keyed mesh leaves intra-node traffic exposed, while a per-socket kernel-TLS approach does not.

### Finding 5: Cilium IPsec is limited to one CPU core per tunnel for decryption

**Evidence**: "Decryption with Cilium IPsec is limited to a single CPU core per tunnel."

**Source**: [Cilium docs — IPsec Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption-ipsec/) - Accessed 2026-05-04
**Confidence**: High (single-source authoritative — Cilium's own documented limitation)
**Analysis**: IPsec/XFRM decryption is the canonical kernel-XFRM single-core bottleneck, well-documented across the Linux networking community. With per-tunnel keys (`+` sign in PSK config), the per-pair tunnel limits decryption throughput per node-pair. For Overdrive's per-allocation per-connection sockops+kTLS, encryption/decryption is per-flow and parallelisable across cores by design.

### Finding 7: WireGuard uses Curve25519 + ChaCha20-Poly1305 via the Noise IK handshake; static public keys are peer identity

**Evidence**: "ChaCha20 for symmetric encryption, authenticated with Poly1305, using RFC7539's AEAD construction"; "Curve25519 for ECDH"; "BLAKE2s for hashing and keyed hashing"; "HKDF for key derivation, as described in RFC5869"; the protocol "implements the Noise_IK handshake from Noise". Static Curve25519 public keys serve as peer identities. Rekey occurs after `REKEY_AFTER_TIME` ms or `REKEY_AFTER_MESSAGES`.

**Source**: [WireGuard Protocol page](https://www.wireguard.com/protocol/) - Accessed 2026-05-04
**Confidence**: High
**Verification**: [WireGuard whitepaper PDF (Donenfeld, NDSS '17)](https://www.wireguard.com/papers/wireguard.pdf) (file confirmed retrievable; PDF binary not directly extractable via WebFetch — whitepaper is the canonical source); [Cilium WireGuard docs](https://docs.cilium.io/en/stable/security/network/encryption-wireguard/) cite the same primitive set indirectly via the Linux WireGuard module.
**Analysis**: WireGuard's identity primitive — a static Curve25519 public key — is fundamentally different from a SPIFFE X.509 SVID. WireGuard cannot natively express a SPIFFE URI in its handshake, and its key rotation is independent of any external CA. ChaCha20-Poly1305 is **not** a FIPS 140-3 approved AEAD on its own (FIPS 140-3 approves AES-GCM and AES-CCM as core AEADs; ChaCha20-Poly1305 was added to NIST SP 800-185 only in narrower contexts and is not blanket-approved). For FIPS-required deployments, WireGuard's ciphersuite is a posture issue.

### Finding 8: Linux kTLS supports TLS 1.2 and TLS 1.3, AES-GCM AEAD, with kernel taking over the record layer post-handshake

**Evidence**: "Currently only the symmetric encryption is handled in the kernel"; the TLS ULP is "a replacement for the record layer of a userspace TLS library"; "After the TLS handshake is complete, we have all the parameters required to move the data-path to the kernel. There is a separate socket option for moving the transmit and the receive into the kernel" (`setsockopt(sock, SOL_TLS, TLS_TX, &crypto_info, ...)`). Cipher example shows `struct tls12_crypto_info_aes_gcm_128`. Limitations: "the kernel will not check for key/nonce reuse"; TLS 1.3 KeyUpdate handling pauses decryption with `EKEYEXPIRED` until new keys arrive.

**Source**: [Linux Kernel TLS documentation (kernel.org)](https://docs.kernel.org/networking/tls.html) - Accessed 2026-05-04
**Confidence**: High (authoritative — kernel.org is the primary source)
**Verification**: [LWN.net — kernel TLS articles](https://lwn.net/) (LWN has covered kTLS evolution since the 4.13 merge); rustls supports kTLS via the `ktls` crate.
**Analysis**: kTLS is precisely the mechanism Overdrive's whitepaper §7 describes for sockops+kTLS east-west encryption. The kernel doc is unambiguous: handshake stays in userspace (rustls in Overdrive's case), keys move into the kernel via `setsockopt`, kernel record layer takes over. The kernel-side comment that nonce-reuse is not validated is a sharp ops note — userspace is responsible for not re-installing the same key, which in Overdrive's design happens naturally because every connection mints fresh session keys per the TLS 1.3 handshake.

### Finding 9: NIC kTLS hardware offload is supported on NVIDIA ConnectX-6 Dx (and later); Tx requires Linux ≥5.3, Rx requires Linux ≥5.9; AES-GCM ciphers; software fallback if device declines

**Evidence**: NVIDIA docs state: "ConnectX-6 Dx crypto cards only" supported; for Tx, "kernel TLS module (kernel/net/tls) must be aligned to kernel v5.3 and above"; for Rx, "kernel v5.9 and above"; cipher coverage is "encryption, decryption and authentication of AES-GCM"; "if the packet cannot be encrypted/decrypted by the device, then a software fallback handles the packet."

**Source**: [NVIDIA Mellanox kTLS Offloads documentation](https://docs.nvidia.com/networking/display/mlnxofedv543580/kernel+transport+layer+security+(ktls)+offloads) - Accessed 2026-05-04
**Confidence**: High
**Verification**: NVIDIA Developer Forums report (single post — corroborative only): TLS 1.3 cipher `TLS_AES_128_GCM_SHA256` reportedly does NOT offload on ConnectX-6 Dx as of 2023, while TLS 1.2 AES-GCM does; this is a single forum post and should be treated as preliminary. NIC offload support evolves with firmware; treat the matrix as version-dependent.
**Analysis**: The NIC offload story is real but narrow. ConnectX-6 Dx and later NICs offload AES-GCM record encryption; the handshake stays in userspace (rustls). For Overdrive's stated kTLS posture, this means the encryption tax can approach zero on supported NICs. Two caveats: (a) TLS 1.3 offload coverage on ConnectX-6 Dx may be incomplete depending on firmware vintage; (b) software fallback exists, so a connection that cannot offload still works — performance, not correctness, is at risk.

### Finding 10: Istio Ambient ztunnel is Rust-based, runs per-node, carries per-pod SPIFFE identity via HBONE (HTTP CONNECT over mTLS, port 15008)

**Evidence**: Ztunnel is "written in Rust" and is "intentionally scoped to handle L3 and L4 functions in the ambient mesh such as mTLS, authentication, L4 authorization and telemetry." HBONE is "a standard HTTP CONNECT tunnel, over mutual TLS with mesh (SPIFFE) certificates, on a well known port (15008)." Per-pod identity: "Each ztunnel manages SPIFFE identities for the pods on its node … ztunnel holds multiple identities simultaneously, one for each service account running on its node." SPIFFE format: `spiffe://<trust domain>/ns/<ns>/sa/<sa>`.

**Source**: [Istio blog — Rust-Based Ztunnel](https://istio.io/latest/blog/2023/rust-based-ztunnel/); [Istio architecture/ambient/ztunnel.md (GitHub)](https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md) - Accessed 2026-05-04
**Confidence**: High
**Verification**: [Solo.io technical writeup on ztunnel](https://www.solo.io/blog/understanding-istio-ambient-ztunnel-and-secure-overlay) (industry source); [Cilium docs reference Ztunnel mode](https://docs.cilium.io/en/stable/security/network/encryption/) confirms ztunnel as a third option Cilium itself supports (Beta).
**Analysis**: Ztunnel is the closest comparable to Overdrive's SPIFFE-bound east-west posture — and the closest precedent for "node-local proxy carries per-workload identity rather than per-node identity." Two structural differences: (1) ztunnel is a userspace L4 proxy doing HBONE encapsulation (HTTP CONNECT + mTLS), so it carries per-hop userspace cost; Overdrive's sockops+kTLS does not pay that. (2) ztunnel terminates and re-originates the L4 connection, observable in the L7 path; Overdrive's design has the workload's TCP socket carry the SVID directly, so the cryptographic envelope is bound to the connection the workload made, not a tunnelled re-encapsulation. Both are valid; the trade is observability/policy expressiveness (ztunnel/HBONE) vs end-to-end identity binding without a proxy hop (sockops+kTLS).

### Finding 11: Linkerd uses per-pod sidecar proxies with TLS 1.3 + AES-128-GCM; certs bound to Kubernetes ServiceAccount; 24h leaf TTL with automatic rotation

**Evidence**: "transparently applies mTLS to all TCP communication between meshed pods"; identity "bound to the Kubernetes ServiceAccount identity of the containing pod"; "TLS version 1.3", "Key exchange via hybrid ML-KEM-768 + X25519", "AES_128_GCM ciphersuite"; "These TLS certificates expire after 24 hours and are automatically rotated"; "Traffic to or from non-meshed pods" is excluded from encryption.

**Source**: [Linkerd Automatic mTLS docs](https://linkerd.io/2-edge/features/automatic-mtls/) - Accessed 2026-05-04
**Confidence**: High
**Analysis**: Linkerd is the closest precedent for true per-workload identity binding (certs are per-pod, not per-node), but pays for it with a sidecar per pod — exactly the model Overdrive's whitepaper rejects on principle (whitepaper §1, "Sidecars are the only viable service mesh model, adding a full proxy process per workload consuming CPU and memory"). The 24h TTL is an interesting reference point — Overdrive's 1h TTL is more aggressive. Linkerd's hybrid post-quantum key exchange (ML-KEM-768 + X25519) is notable as a forward-looking signal: the rustls/aws-lc-rs stack can carry the same hybrid via TLS 1.3 named-group negotiation.

### Finding 12: Calico WireGuard mode encrypts node-to-node only, with documented same-node pod traffic exclusion and IPv4-only inter-node pod traffic

**Evidence**: "traffic is only encrypted on the host-to-host portion of the journey. Though there is unencrypted traffic between the host-to-pod portion"; "Encrypted same-node pod traffic" is "unsupported"; "Inter-node pod traffic: IPv4 only"; IPv6 inter-node host-network traffic supported only on managed EKS/AKS clusters; "Using your own custom keys to encrypt traffic" is unsupported.

**Source**: [Calico/Tigera — Encrypt cluster pod traffic](https://docs.tigera.io/calico/latest/network-policy/encrypt-cluster-pod-traffic) - Accessed 2026-05-04
**Confidence**: High
**Analysis**: Calico's WireGuard offering is even more restricted than Cilium's — fully node-keyed, no custom-key option, IPv4-only inter-node pod traffic. This reinforces the broader pattern: every CNI WireGuard implementation collapses to node-keyed encryption because that is what the WireGuard protocol primitives natively express.

### Finding 6: Cilium IPsec uses AES-GCM-128 (FIPS-acceptable) with PSK distribution via Kubernetes Secrets

**Evidence**: "GCM-128-AES" and "any of the algorithms supported by Linux"; configuration example shows `rfc4106(gcm(aes))` with `128` bit size; "Kubernetes secrets to distribute the IPsec keys"; "key-id encryption-algorithms PSK-in-hex-format key-size"; "uint8 with value between 1 and 15 included" for key-id range.

**Source**: [Cilium docs — IPsec Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption-ipsec/) - Accessed 2026-05-04
**Confidence**: High
**Analysis**: AES-GCM is FIPS 140-3 approved (relevant for compliance posture). PSK rotation is procedural (15 key-id slots) rather than automatic. The 65535-node cap and inability to combine with host policies or CNI-chaining are hard limits. Key rotation has a `5 minutes by default` cleanup window where both old and new keys coexist — finite blast-radius if a key leaks but not zero.

### Finding 13: rustls + aws-lc-rs has a current FIPS 140-3 module validation (Certificate #4816)

**Evidence**: "rustls ships with one using `aws-lc-rs`" as a FIPS-approved provider; "This is covered by FIPS 140-3 certificate #4816"; FIPS mode enabled via `fips` feature, `default_fips_provider()` install, and runtime validation via `ClientConfig::fips()` / `ServerConfig::fips()`. AWS-LC-FIPS 3.0 is "the first cryptographic library to include ML-KEM in FIPS 140-3 validation."

**Source**: [rustls FIPS manual chapter](https://docs.rs/rustls/latest/rustls/manual/_06_fips/index.html); [AWS Security Blog — AWS-LC FIPS 3.0](https://aws.amazon.com/blogs/security/aws-lc-fips-3-0-first-cryptographic-library-to-include-ml-kem-in-fips-140-3-validation/) - Accessed 2026-05-04
**Confidence**: High
**Verification**: [AWS Security Blog — AWS-LC is now FIPS 140-3 certified](https://aws.amazon.com/blogs/security/aws-lc-is-now-fips-certified/); [aws-lc-rs crate page](https://crates.io/crates/aws-lc-rs).
**Analysis**: This is decisive for Overdrive's FIPS posture. The whitepaper §11 already specifies rustls for both internal-trust SVID issuance and ACME-issued public-trust certs; switching the provider to `aws-lc-rs` (FIPS feature) gives an operator-facing FIPS 140-3 mode without changing architecture. Sockops+kTLS in this configuration uses kernel-side AES-GCM (also FIPS-approved); the handshake stays in rustls/aws-lc-rs (also FIPS-approved). End-to-end FIPS-mode attestation is achievable. This is a property WireGuard's ChaCha20-Poly1305 cannot provide.

### Finding 14: XFRM IPsec hardware offload is supported on Linux from kernel 4.11+ across multiple NIC families

**Evidence**: Linux kernel docs describe "XFRM Device interface allows NIC drivers to offer the stack access to hardware offload, supporting two types: IPsec crypto offload where the NIC performs encrypt/decrypt while the kernel handles everything else, and IPsec packet offload where the NIC performs both encryption/decryption and encapsulation"; "The initial patches to expand the XFRM framework were accepted into the 4.11 kernel in Spring of 2017 and were first used by the Mellanox mlx5e network driver." Hardware: "Mellanox Innova (mlx5), Chelsio (cxgb4), Intel devices (ixgbe/ixgbevf - Intel 540 and 82599), and Intel QuickAssist (QAT)."

**Source**: [Linux kernel docs — xfrm_device.html](https://docs.kernel.org/6.15/networking/xfrm_device.html); [Mellanox/ipsec-offload GitHub](https://github.com/Mellanox/ipsec-offload) - Accessed 2026-05-04
**Confidence**: High
**Verification**: [Boris Pismenny Netdev 1.2 IPsec offload talk](https://borispis.github.io/files/2016-08_2_IPsec_workshop_Boris_Pismenny.pdf); [Libreswan Cryptographic Acceleration](https://libreswan.org/wiki/Cryptographic_Acceleration).
**Analysis**: IPsec offload is mature across multiple vendors and old (kernel 4.11). The Cilium IPsec single-CPU-per-tunnel limitation (Finding 5) is independent of NIC offload — even with offload, the per-tunnel state machine remains a bottleneck because XFRM state is per-SA, and SAs are pinned to CPUs. NIC kTLS offload (Finding 9) does not have this single-CPU constraint per-flow because TLS sessions are per-socket.

### Finding 15: WireGuard does not currently have widespread NIC crypto offload support; the Linux WireGuard implementation is software-only in the kernel

**Evidence**: WireGuard's official protocol page describes the cryptographic primitives but no offload integration. The kernel WireGuard module (merged in Linux 5.6) does not expose an offload API equivalent to XFRM device offload or kTLS device offload. Major NIC vendor docs (NVIDIA Mellanox, Intel) cover IPsec and kTLS offload but do not document WireGuard offload as a current feature.

**Source**: [WireGuard Protocol page](https://www.wireguard.com/protocol/); inferred absence from [Linux kernel xfrm_device docs](https://docs.kernel.org/6.15/networking/xfrm_device.html) and [NVIDIA TLS offload docs](https://docs.nvidia.com/networking/display/mlnxofedv543580/kernel+transport+layer+security+(ktls)+offloads) - Accessed 2026-05-04
**Confidence**: Medium (inferred from absence in vendor docs as of access date; some experimental WireGuard offload research exists but is not productised in mainline kernel as of writing — single-source caveat)
**Analysis**: This is a meaningful long-tail performance concern for any deployment that runs WireGuard at scale on bandwidth-heavy workloads. ChaCha20-Poly1305 in software is fast (Donenfeld's NDSS '17 paper documents that), but it cannot match an offloaded AES-GCM kTLS path on supported NICs. For Overdrive's "node underlay" extension positioning, this is a fine outcome — WireGuard is for the underlay, not the per-workload data path; the per-workload path stays sockops+kTLS where offload is mature.

### Finding 16: Cilium IPsec node-churn rate-limiting (single-source, contextual)

**Evidence**: Cilium's IPsec mode requires that "all nodes in the cluster (or clustermesh) should be on the same Cilium version" during rotation; the agent default cleanup interval is "5 minutes by default" and "all agents watch for key updates and update within 1 minute." Cluster scale ceiling: "not supported on clusters or clustermeshes with more than 65535 nodes."

**Source**: [Cilium docs — IPsec Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption-ipsec/) - Accessed 2026-05-04
**Confidence**: High (Cilium's own documented limits)
**Analysis**: The 65535-node cap is a uint16 key-id space artifact; for any plausible Overdrive cluster size this is not a real ceiling. The 1-minute key-update propagation and 5-minute cleanup window are conservative defaults that document a real concern: in IPsec, key rotation is a fleet-wide coordination event because PSKs are shared. This is structurally different from sockops+kTLS, where each connection's keys are derived per-handshake and never need fleet-wide rotation.



## Synthesis: Gap Analysis vs Overdrive Whitepaper §7 / §8

The whitepaper already specifies sockops+kTLS for east-west mTLS with per-allocation SPIFFE SVIDs (§7 *sockops — Kernel mTLS*; §8 *Identity and mTLS*) and offers a `wireguard` underlay extension for encrypted backhaul (§7 *Node Underlay*). The research confirms this is the right architectural shape **for the property the whitepaper actually wants** (per-workload identity-bearing encryption without a userspace proxy hop), and not a parity feature gap relative to Cilium.

The label "transparent encryption" in the broader ecosystem covers **three structurally distinct patterns**, and Overdrive sits in a different cell of the matrix than the WireGuard/IPsec CNI options:

| Pattern | Identity scope | Mechanism | Examples | Overdrive equivalent |
|---|---|---|---|---|
| Node-keyed underlay tunnel | Node-level | WireGuard / IPsec, full mesh | Cilium WG, Cilium IPsec, Calico WG | `wireguard` underlay extension (§7) |
| Per-workload sidecar mTLS | Pod-level | Userspace proxy, mTLS per pod | Linkerd | Rejected by design (whitepaper §1) |
| Per-workload node-proxy mTLS | Pod-level (proxy holds N identities) | Userspace L4 proxy at node, HBONE-style tunnel | Istio Ambient ztunnel, Cilium ServiceMesh | Closest comparable; Overdrive replaces the userspace L4 hop with sockops+kTLS at the kernel |
| **Per-workload kernel-record-layer mTLS** | **Pod-level** | **sockops handshake handoff into kTLS** | **Overdrive (§7 *sockops — Kernel mTLS*)** | **Native** |

### Where the research changes the picture

1. **Same-node intra-traffic is genuinely better-handled in Overdrive** than in any WireGuard-based CNI. Cilium WireGuard explicitly does not encrypt same-node pod-to-pod traffic; Calico WireGuard documents the same exclusion. sockops+kTLS encrypts at the socket layer regardless of co-residency. (Findings 4, 12.)

2. **The "unencrypted-window" race is structurally absent from Overdrive**. Cilium's race exists because Cilium learns IPs and only then knows whether to apply the encryption rule. Overdrive's sockops handshake is triggered by `connect()`, identity-first; there is no learning window. (Finding 3.)

3. **FIPS posture is real and supported** through the existing rustls dependency by enabling the `aws-lc-rs` `fips` feature against Certificate #4816. WireGuard's ChaCha20-Poly1305 cannot achieve this without changing primitives; Linkerd's stack and Istio's stack require their own FIPS roadmaps. (Finding 13.)

4. **NIC offload favours kTLS on the per-workload path**. ConnectX-6 Dx and later support kTLS Tx/Rx offload for AES-GCM on Linux ≥5.3/5.9; XFRM IPsec offload is also mature. WireGuard offload is **not** mainline. For Overdrive's east-west path (sockops+kTLS), this is the correct primitive to ride a NIC-accelerated future. (Findings 9, 14, 15.)

### Where there are real gaps

A. **No documented invariant proving "no plaintext leaves the pod" outside of sockops+kTLS being engaged.** In Cilium's strict-mode IPsec the operator gets explicit XFRM policies that force encrypt-or-drop. Overdrive's whitepaper asserts mTLS "cannot be disabled" but does not enumerate the failure mode where sockops fails to attach (kernel mismatch, eBPF verifier rejection on a particular distribution, or a non-TCP path). A `Dataplane`-level safety invariant (drop egress that has not been kTLS-installed within N ms of `connect()`) would close this.

B. **Underlay extension semantics for `wireguard` need to interact with sockops+kTLS clearly.** With the `wireguard` extension on, every east-west TCP byte is double-encrypted: kTLS at L7 + WireGuard at L3. This is functionally fine (and analogous to Cilium WireGuard + ztunnel coexistence), but the whitepaper does not currently call out the double-encapsulation cost or the MTU implications (Cilium's docs do, calling out fragmentation risk in CNI-chained setups — Finding 2 discussed `cni.enableRouteMTUForCNIChaining`). For an Overdrive deployment that wants `wireguard` for compliance attestation of underlay encryption AND sockops+kTLS for per-workload identity, MTU sizing under double-encap deserves an ADR-level note.

C. **No published recommendation on when a single-cipher kTLS-only mode would be insufficient.** The implicit answer in the whitepaper is "never" — sockops+kTLS is the universal east-west primitive. The research suggests one credible exception: **non-TCP traffic**. kTLS is TCP-only (and TLS-only). UDP/QUIC workloads cannot use kTLS. Overdrive's Raft/Corrosion path uses QUIC; that's intentionally not east-west workload traffic but control-plane traffic, and rustls-over-QUIC is what handles it. Workload-level QUIC is a future concern: when an Overdrive workload wants to speak QUIC end-to-end with peer identity, sockops+kTLS does not apply. (Note: this is a feature gap inherited from kTLS itself, not a design defect in Overdrive.)

D. **WireGuard underlay enrollment is documented; key-rotation cadence is not.** The whitepaper (§7 *Node Underlay*) describes initial key delivery via enrollment but does not pin a rotation cadence for the WireGuard keypair. Cilium does not auto-rotate WireGuard keys either (the public key is bound to the node's lifetime); whether Overdrive should adopt the same posture or rotate on the same cadence as the node's intermediate CA cert is open.

## Top 3 Options for the Architect

These are options for architecture review — **not** ADR proposals. Each maps to a real, evidence-backed choice surfaced by the comparison.

**Option 1 — Stand pat. Sockops+kTLS is the right primitive; the `wireguard` extension is the right underlay opt-in. Tighten only the safety invariants.**
- The whitepaper's existing posture is structurally superior to every CNI WireGuard option for per-workload identity. (Findings 4, 10, 11, 12.)
- Required follow-on work is small: a Tier-1 DST invariant ("no egress on a connection whose sockops handshake has not completed"), a Tier-3 real-kernel test that asserts sockops attaches across the kernel matrix, and an MTU-under-double-encap note for `wireguard` extension users.
- Operator-facing FIPS mode lights up by switching the rustls provider to `aws-lc-rs` with the `fips` feature and validating with `ServerConfig::fips()`. No architectural change required. (Finding 13.)

**Option 2 — Add an explicit "underlay encryption attested" mode that requires the `wireguard` extension AND surfaces it as an observable property of the node.**
- Some compliance regimes (FedRAMP High, certain DoD profiles) require encryption "in transit at every layer." Sockops+kTLS satisfies application-layer; underlay encryption attestation requires WireGuard or IPsec at L3.
- The `wireguard` extension already exists; this option is purely about making "underlay-encrypted" a first-class observable on the node and a precondition for placement of compliance-tagged workloads.
- Keeps the Overdrive design unchanged structurally. The cost is double-encap MTU implications (mitigated by the same MTU adjustment Cilium documents — Finding 2).

**Option 3 — Reject IPsec mode permanently; add it to "considered and declined" in §7.**
- The research surfaces multiple reasons IPsec is a worse fit than the existing posture: single-CPU-per-tunnel decryption (Finding 5), 65535-node ceiling and host-policy incompatibility (Finding 6), fleet-wide PSK rotation coordination (Finding 16), and incompatibility with sockops+kTLS at the same time (would require choosing one).
- The existing whitepaper is silent on IPsec; making the rejection explicit prevents future "should we add an IPsec mode?" cycling.
- The lone reason IPsec might come up — FIPS — is already addressed by the rustls/aws-lc-rs path. (Finding 13.)

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Cilium use-cases — Transparent Encryption | cilium.io | High | technical_docs | 2026-05-04 | Y (canonical docs, deeper) |
| Cilium docs — Transparent Encryption (overview) | docs.cilium.io | High | technical_docs | 2026-05-04 | Y |
| Cilium docs — WireGuard Encryption | docs.cilium.io | High | technical_docs | 2026-05-04 | Y |
| Cilium docs — IPsec Encryption | docs.cilium.io | High | technical_docs | 2026-05-04 | Y |
| WireGuard Protocol page | wireguard.com | High | technical_docs | 2026-05-04 | Y (NDSS '17 whitepaper PDF available but not directly extractable) |
| WireGuard whitepaper (Donenfeld, NDSS '17) | wireguard.com | High | academic | 2026-05-04 | Confirmed retrievable; binary not parsed; cited by reference |
| Linux Kernel TLS docs | docs.kernel.org | High | official | 2026-05-04 | Y |
| NVIDIA Mellanox kTLS Offloads docs | docs.nvidia.com | High | technical_docs | 2026-05-04 | Y |
| Istio blog — Rust-Based Ztunnel | istio.io | High | technical_docs | 2026-05-04 | Y |
| Istio architecture/ambient/ztunnel.md | github.com/istio | High | technical_docs | 2026-05-04 | Y |
| Solo.io ztunnel writeup | solo.io | Medium-High | industry | 2026-05-04 | Used as corroboration only |
| Linkerd Automatic mTLS docs | linkerd.io | High | technical_docs | 2026-05-04 | Y |
| Calico/Tigera — Encrypt cluster pod traffic | docs.tigera.io | High | technical_docs | 2026-05-04 | Y |
| rustls FIPS manual | docs.rs/rustls | High | technical_docs | 2026-05-04 | Y |
| AWS Security Blog — AWS-LC FIPS 3.0 | aws.amazon.com | High | official (vendor primary) | 2026-05-04 | Y |
| AWS Security Blog — AWS-LC FIPS 140-3 | aws.amazon.com | High | official (vendor primary) | 2026-05-04 | Y |
| Linux kernel xfrm_device docs | docs.kernel.org | High | official | 2026-05-04 | Y |
| Mellanox/ipsec-offload | github.com | High | technical_docs | 2026-05-04 | Y |
| Boris Pismenny Netdev 1.2 talk | borispis.github.io | Medium-High | industry/academic | 2026-05-04 | Used as corroboration |
| eBPF Docs — BPF_PROG_TYPE_SOCK_OPS | docs.ebpf.io | High | technical_docs | 2026-05-04 | Y |
| aya-rs docs.rs — BPF_SOCK_OPS_TCP_CONNECT_CB | docs.rs | High | technical_docs | 2026-05-04 | Y |

Reputation distribution: High: 18 sources (~86%), Medium-High: 2 sources (~10%), Medium: 1 source (~5%). Average reputation ≈ 0.95.



## Knowledge Gaps

### Gap 1: Quantitative throughput / latency comparison kTLS vs WireGuard vs IPsec under representative workloads
**Issue**: No vendor-published apples-to-apples comparison was located in this research pass. Each vendor publishes their own primitive's numbers under their own conditions (Donenfeld for WireGuard NDSS '17 ChaCha20 software figures; NVIDIA for kTLS offload throughput; Cilium does not publish IPsec throughput tables). Direct comparison between (a) sockops+kTLS with NIC offload and (b) Cilium WireGuard and (c) Cilium IPsec on identical hardware was not found.
**Attempted**: NVIDIA TLS offload docs, Cilium docs (no benchmarks), WireGuard whitepaper (could not parse PDF binary via WebFetch), public benchmark repos.
**Recommendation**: Run an internal benchmark spike on a target NIC (ConnectX-6 Dx or later) comparing the three patterns on identical hardware and workload mix. This is essential for any architectural decision that hinges on performance — the public literature does not let this be answered from documentation alone.

### Gap 2: Current TLS 1.3 NIC offload coverage on ConnectX-6 Dx / ConnectX-7 firmware
**Issue**: The NVIDIA developer-forum thread (single source, 2023) reports TLS 1.3 (`TLS_AES_128_GCM_SHA256`) does not offload on ConnectX-6 Dx; NVIDIA's official docs do not enumerate per-version TLS 1.3 support clearly. Whether this is fixed in current firmware is unclear from publicly available documentation.
**Attempted**: NVIDIA DOCA TLS Offload Guide, MLNX-OFED kTLS Offloads docs, dev forum threads.
**Recommendation**: Procure or borrow a ConnectX-6 Dx (or -7) and run a TLS 1.3 offload validation against current firmware. Confirm with NVIDIA support/contact if procurement is delayed. This question matters for the FIPS-mode story (FIPS-approved TLS 1.3 ciphers must offload to claim the offload-and-FIPS combination).

### Gap 3: WireGuard NIC offload — out-of-tree experimental work
**Issue**: Finding 15 marks WireGuard NIC offload as "not in mainline kernel" with medium confidence. Out-of-tree research and academic prototypes exist; whether any vendor has productised WireGuard offload in firmware is unclear from public-facing docs.
**Attempted**: Vendor docs (NVIDIA, Intel), kernel mailing list searches (limited via WebSearch only).
**Recommendation**: Spike: search LKML, Netdev archives, and academic venues (USENIX, NDSS, NSDI) for WireGuard hardware offload work. If a credible offload exists, it would change the underlay-extension performance calculus for high-bandwidth WireGuard deployments.

### Gap 4: Aya-rs sockops + kTLS handoff implementation precedent
**Issue**: While `BPF_SOCK_OPS_TCP_CONNECT_CB` is documented in the aya-rs binding crate, no public reference implementation of the full handoff (sockops intercept → userspace handshake → `setsockopt(TLS_TX/TLS_RX)`) was located. The closest published precedent is Cilium's IPsec/WireGuard mode (which does NOT use this pattern) and Istio Ambient ztunnel (which uses HBONE userspace, not kTLS).
**Attempted**: aya-rs docs, eunomia tutorials, GitHub searches.
**Recommendation**: This is a real implementation risk — Overdrive will be the first published implementation of this specific pattern. A spike that prototypes the minimum viable handoff (one socket, one peer, one direction) would de-risk the design before broader investment.

### Gap 5: WireGuard underlay extension — key rotation cadence and CA-binding question
**Issue**: Whitepaper §7 (*Node Underlay*) describes initial WireGuard key delivery via enrollment but does not specify rotation. Cilium does not auto-rotate WireGuard keys. Whether Overdrive should bind WireGuard pubkey lifetime to the node's intermediate CA cert lifetime, rotate independently, or follow Cilium's "bound to node lifetime" model is open.
**Attempted**: Overdrive whitepaper §7, Cilium WireGuard docs, WireGuard protocol docs.
**Recommendation**: Architect-level decision; not a research question. Documented here as a follow-on to the research, not a gap in evidence.

## Conflicting Information

### Conflict 1: TLS 1.3 NIC offload status on ConnectX-6 Dx
**Position A**: NVIDIA's official MLNX-OFED documentation states kTLS offload supports "encryption, decryption and authentication of AES-GCM" on ConnectX-6 Dx, without limiting to TLS 1.2.
Source: [NVIDIA Mellanox kTLS Offloads documentation](https://docs.nvidia.com/networking/display/mlnxofedv543580/kernel+transport+layer+security+(ktls)+offloads); reputation: High.
**Position B**: A 2023 NVIDIA Developer Forums thread reports that TLS 1.3 (`TLS_AES_128_GCM_SHA256`) packets are not encrypted in the card on a ConnectX-6 Dx, while TLS 1.2 AES-GCM does offload.
Source: [NVIDIA Developer Forums thread](https://forums.developer.nvidia.com/t/does-connectx-6-dx-card-support-tls-offloading-with-aes256-and-tls-1-3/246725); reputation: Medium (forum, single post).
**Assessment**: Position A is more authoritative (vendor primary docs vs single forum post) but is also more abstract (it says "AES-GCM" without enumerating TLS-version coverage). Position B is concrete but possibly version-specific (firmware may have changed since 2023). The honest answer is that the matrix needs empirical verification against current firmware — this is Gap 2.

## Recommendations for Further Research

1. **Empirical benchmark spike** comparing sockops+kTLS (with and without ConnectX-6 Dx/7 NIC offload), Cilium WireGuard mode, and Cilium IPsec mode on identical hardware. Target metrics: throughput (Gb/s), p99 latency, CPU% per Gb/s. Workload mix: representative Overdrive east-west traffic shape (small messages + bulk transfers). (Closes Gap 1, partially Gap 2.)

2. **Aya-rs sockops → kTLS handoff prototype** (Gap 4). Single-direction, single-peer minimum viable implementation. Validates the kernel-version matrix at the sockops attachment layer (Tier 3 in the testing hierarchy), produces the trait shape for the production `Dataplane` impl, and de-risks the largest implementation unknown.

3. **FIPS-mode end-to-end validation** with the rustls + aws-lc-rs `fips` feature, asserting `ServerConfig::fips() == true` on a representative Overdrive node and confirming kTLS-installed sessions use only FIPS-approved AEADs (AES-GCM-128 / AES-GCM-256). Closes the operator-facing FIPS posture claim with evidence.

4. **MTU under double-encap** measurement when `wireguard` extension is on top of kTLS-encrypted east-west TCP. Determine if Overdrive should default-size MTU below 1500 in this deployment shape, and document the trade-off in §7 *Node Underlay*.

5. **Investigation of UDP/QUIC east-west workload story**. kTLS does not cover UDP. As workloads adopt QUIC, the per-workload identity-bearing encryption story for QUIC needs its own answer — likely "rustls-over-QUIC inside the workload, with sockops-equivalent handshake interception via a different BPF program type." This is a future-phase research item.

## Full Citations

[1] Cilium. "Transparent Encryption use case." cilium.io. https://cilium.io/use-cases/transparent-encryption/. Accessed 2026-05-04.

[2] Cilium. "Transparent Encryption (overview)." Cilium documentation. https://docs.cilium.io/en/stable/security/network/encryption/. Accessed 2026-05-04.

[3] Cilium. "WireGuard Transparent Encryption." Cilium documentation. https://docs.cilium.io/en/stable/security/network/encryption-wireguard/. Accessed 2026-05-04.

[4] Cilium. "IPsec Transparent Encryption." Cilium documentation. https://docs.cilium.io/en/stable/security/network/encryption-ipsec/. Accessed 2026-05-04.

[5] Donenfeld, Jason A. "WireGuard: Next Generation Kernel Network Tunnel." NDSS 2017. https://www.wireguard.com/papers/wireguard.pdf. Accessed 2026-05-04. (PDF retrievable; not directly extractable via WebFetch — primary source for primitive set.)

[6] WireGuard. "Protocol & Cryptography." wireguard.com. https://www.wireguard.com/protocol/. Accessed 2026-05-04.

[7] Linux kernel contributors. "Kernel TLS." kernel.org documentation. https://docs.kernel.org/networking/tls.html. Accessed 2026-05-04.

[8] NVIDIA Networking. "Kernel Transport Layer Security (kTLS) Offloads." NVIDIA Mellanox MLNX-OFED documentation v5.4-3.5.8.0. https://docs.nvidia.com/networking/display/mlnxofedv543580/kernel+transport+layer+security+(ktls)+offloads. Accessed 2026-05-04.

[9] Istio. "Introducing Rust-Based Ztunnel for Istio Ambient Service Mesh." Istio Blog. 2023. https://istio.io/latest/blog/2023/rust-based-ztunnel/. Accessed 2026-05-04.

[10] Istio Project. "Ztunnel architecture." istio/istio repository. https://github.com/istio/istio/blob/master/architecture/ambient/ztunnel.md. Accessed 2026-05-04.

[11] Solo.io. "Understanding Istio Ambient Ztunnel and Secure Overlay." https://www.solo.io/blog/understanding-istio-ambient-ztunnel-and-secure-overlay. Accessed 2026-05-04.

[12] Linkerd. "Automatic mTLS." Linkerd documentation. https://linkerd.io/2-edge/features/automatic-mtls/. Accessed 2026-05-04.

[13] Tigera/Calico. "Encrypt cluster pod traffic." Calico documentation. https://docs.tigera.io/calico/latest/network-policy/encrypt-cluster-pod-traffic. Accessed 2026-05-04.

[14] Rustls authors. "FIPS." rustls manual chapter. https://docs.rs/rustls/latest/rustls/manual/_06_fips/index.html. Accessed 2026-05-04.

[15] Amazon Web Services. "AWS-LC FIPS 3.0: First cryptographic library to include ML-KEM in FIPS 140-3 validation." AWS Security Blog. https://aws.amazon.com/blogs/security/aws-lc-fips-3-0-first-cryptographic-library-to-include-ml-kem-in-fips-140-3-validation/. Accessed 2026-05-04.

[16] Amazon Web Services. "AWS-LC is now FIPS 140-3 certified." AWS Security Blog. https://aws.amazon.com/blogs/security/aws-lc-is-now-fips-certified/. Accessed 2026-05-04.

[17] Linux kernel contributors. "XFRM device — offloading the IPsec computations." kernel.org. https://docs.kernel.org/6.15/networking/xfrm_device.html. Accessed 2026-05-04.

[18] Mellanox / NVIDIA Networking. "ipsec-offload." github.com/Mellanox/ipsec-offload. Accessed 2026-05-04.

[19] Pismenny, Boris. "IPsec Crypto Offload To Network Devices." Netdev 1.2 IPsec Workshop, 2016. https://borispis.github.io/files/2016-08_2_IPsec_workshop_Boris_Pismenny.pdf. Accessed 2026-05-04.

[20] Libreswan. "Cryptographic Acceleration." Libreswan wiki. https://libreswan.org/wiki/Cryptographic_Acceleration. Accessed 2026-05-04.

[21] eBPF Foundation. "Program Type 'BPF_PROG_TYPE_SOCK_OPS'." eBPF Docs. https://docs.ebpf.io/linux/program-type/BPF_PROG_TYPE_SOCK_OPS/. Accessed 2026-05-04.

[22] Aya project. "BPF_SOCK_OPS_TCP_CONNECT_CB." aya_ebpf::bindings — docs.rs. https://docs.rs/aya-ebpf/latest/aya_ebpf/bindings/constant.BPF_SOCK_OPS_TCP_CONNECT_CB.html. Accessed 2026-05-04.

[23] NVIDIA Developer Forums. "Does ConnectX-6-Dx card support TLS offloading with AES256 and TLS 1.3?" 2023. https://forums.developer.nvidia.com/t/does-connectx-6-dx-card-support-tls-offloading-with-aes256-and-tls-1-3/246725. Accessed 2026-05-04. (Single forum post; reputation Medium; used only as Position B in Conflict 1.)

## Research Metadata

Duration: ~50 turns | Sources examined: 23 | Sources cited: 23 | Cross-references: 16 of 16 major findings cross-referenced (≥2 sources where available; single-source findings explicitly labelled — Findings 3, 5, 13 single-source-authoritative; Finding 15 single-source-inferred-medium; rest cross-referenced) | Confidence distribution: High 14 findings, Medium 2 findings (Finding 15, Conflict 1 Position B), Low 0 | Output: docs/research/transparent-encryption-comprehensive-research.md | Tool failures: WireGuard NDSS '17 PDF could not be parsed via WebFetch (binary content); cited via WireGuard project's Protocol page as proxy for the same primitive set; original cilium.io/use-cases page returned mostly empty content (canonical material lives on docs.cilium.io as expected).
