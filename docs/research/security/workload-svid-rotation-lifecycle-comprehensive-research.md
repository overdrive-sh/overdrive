# Research: Workload-Identity Certificate (X.509-SVID) Lifecycle in Production Systems — Issuance, Rotation, CA-Root Persistence, and Crash/Restart Recovery

**Date**: 2026-06-09 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (Decisions 1 & 2); Medium-High (Decision 3, with one flagged gap) | **Sources**: 29

## Purpose — the three Overdrive decisions this must inform

Overdrive is a Rust workload platform whose mTLS data path is **kernel-mediated** (eBPF sockops + kTLS): workloads are identity-unaware and hold NO cert material; the worker/control-plane holds the SVID material (cert + leaf private key) in memory and the kernel consumes it. A durable audit row (`issued_certificates`) persists issuance *facts* (spiffe_id, serial, not_before, not_after, node_id, issued_at) but NOT the cert bytes or the private key. The built-in CA is internal X.509 (rcgen, P-256, 3-tier root→intermediate→leaf-SVID); when persistent its root key is envelope-encrypted at rest.

The three decisions:

1. **Rotation as durable journaled WORKFLOW vs control-loop/reconciler.** Do comparable systems model internal (non-ACME) cert rotation as a crash-resumable, journaled multi-step workflow — or universally as a continuous control loop / periodic renewal?
2. **CA-root persistence across restart.** How is the signing CA root/intermediate persisted so pre-restart certs chain-verify post-restart? How is the root key protected at rest? What breaks if the root is ephemeral?
3. **Restart/crash recovery — REUSE vs RE-ISSUE, leaf-keys-at-rest.** On restart, re-issue fresh leaf certs or reload/reuse still-valid material? Is leaf private-key material persisted at rest, or held only in memory/kernel and re-issued/re-attested on restart?

## Executive Summary

Across eight production workload-identity / PKI systems (SPIFFE/SPIRE, Cilium, Istio+Envoy SDS, Linkerd, HashiCorp Vault PKI/Consul Connect, cert-manager, Kubernetes kubelet, Talos Linux), three patterns are essentially universal and directly settle two of Overdrive's three decisions. **First, internal certificate rotation is everywhere a continuous control loop / reconciler — never a durable journaled workflow.** Rotation fires at a fraction of cert lifetime (SPIRE half-life, cert-manager 2/3, kubelet 70–90% elapsed) with an overlap window to absorb restarts/outages; internal-CA *issuance* is a single synchronous CSR→sign→return call. The genuinely multi-step, wait-bearing flow is ACME public-cert issuance (order → DNS-01 wait → validate → finalize), which an internal rcgen CA does not perform. The only workflow-shaped sequence in the corpus is *CA-root* rotation (Talos/Linkerd bundle→switch→drain), and even that is implemented as an operator-driven procedure, not a Temporal-style workflow. **Second, the CA root/intermediate MUST persist across restart; an ephemeral per-boot root is a named, well-understood failure** — SPIRE states plainly that an in-memory CA key "results in a new CA being generated on each restart, which breaks certificate continuity and is unsuitable for production." Every comparator persists the CA and treats leaves as disposable (long-CA-TTL / short-leaf-TTL asymmetry). Overdrive's envelope-encrypted-root-at-rest plan sits in the recognized middle protection tier (Vault's barrier model), with HSM/KMS as the stronger option.

**Third, on leaf private keys and restart recovery, Overdrive's "no leaf keys at rest, held in memory/kernel" posture is the secure-default majority** — SPIRE (in-memory cache), Istio (SDS in-memory, no Secret), Linkerd (tmpfs, "never persist to disk"), and Vault ("does not store generated private keys, except for CA") all keep leaf keys out of durable storage; SPIFFE frames this as minimizing leak exposure. The systems that *do* persist leaf keys (cert-manager Secret, kubelet `--cert-dir`) are those where the holder IS the identity owner, with no broker. Overdrive's worker-holds-material / kernel-consumes / identity-unaware-workload model maps directly onto the **Cilium-agent / sidecar-broker** class — and that class **re-issues / re-attests on restart** precisely because nothing durable survives.

**The one unresolved tension** (and the only point resting on a gap): Overdrive proposes to combine "no leaf keys at rest" with "read the `issued_certificates` audit row on restart and don't re-mint a still-valid cert." **No surveyed system combines these.** The systems that skip re-minting on restart (kubelet, cert-manager) do so by re-reading the *actual persisted cert+key*; the systems that hold no durable leaf material re-issue. An audit row carrying only issuance *facts* (spiffe_id, serial, validity window) cannot reconstruct a lost private key, so a restart that loses the in-memory/kernel key MUST re-mint regardless of the row. The recommendation is therefore to keep leaf keys out of disk (High confidence), re-issue on a key-losing restart (High), and narrow the audit row's restart role to rotation-scheduling and over-issuance dedup rather than "skip re-issuance" authorization (Medium-High) — and to validate against Overdrive's actual kernel/keyring survival semantics before relying on the row to suppress re-issuance.

## Research Methodology
**Search Strategy**: Per-comparator targeted WebSearch scoped to the trusted-domain list, followed by WebFetch of the canonical doc page for exact phrasing; adversarial cross-referencing of every load-bearing claim across ≥2 independent project/vendor docs (and GitHub source where the doc page was thin). Searched official project docs first (spiffe.io, docs.cilium.io, istio.io, linkerd.io, cert-manager.io, kubernetes.io, talos.dev/docs.siderolabs.com), official-vendor docs (developer.hashicorp.com, flagged), and GitHub issue/source for SPIRE rotation internals. One academic preprint (arXiv) used for context on the credential-broker pattern, not as load-bearing evidence.
**Source Selection**: Types: official / open_source / academic / technical_documentation | Reputation: high / medium-high min | Verification: cross-referencing across vendor docs, project docs, and source code
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced | Avg reputation achieved ≈ 0.96

---

## Per-Comparator Findings

### 1. SPIFFE / SPIRE (canonical workload identity)

**Finding 1.1 — SVID delivery: short-lived, automatically rotated, delivered via the Workload API; the agent caches them in memory.**
**Evidence**: SPIRE "delivers workload-specific, short-lived, automatically rotated keys and certificates (X.509-SVIDs)… suitable for establishing mTLS directly to workloads via the Workload API." The agent flow: "The agent then sends workload CSRs to the server which the server signs and returns as workload SVIDs to the client. The client puts them in cache."
**Source**: [SPIFFE — SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) — Accessed 2026-06-09; [SPIFFE — SPIRE Use Cases](https://spiffe.io/docs/latest/spire-about/use-cases/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [SPIFFE — Working with SVIDs](https://spiffe.io/docs/latest/deploying/svids/) (Workload API is the delivery surface)
**Analysis**: Leaf material is cached in the agent process memory and streamed to workloads over the Workload API (a Unix domain socket). The agent is the holder; the SVID is not written to a durable store by SPIRE itself.

**Finding 1.2 — Default rotation = half-life (50% of SVID lifetime); configurable via `availability_target`.**
**Evidence**: "Currently spire-agent svid rotation happens at half life of svid validity. This is the default rotation strategy." With `availability_target` set, "the agent will rotate an X509 SVID when its remaining lifetime reaches the availability_target… If set, must be at least 24h," and "the grace period (SVID lifetime - availability_target) must be at least 12h. If not satisfied, the agent will rotate the SVID by the default rotation strategy (1/2 of lifetime)."
**Source**: [SPIFFE — SPIRE Agent Configuration Reference](https://spiffe.io/docs/latest/deploying/spire_agent/) — Accessed 2026-06-09; [spiffe/spire issue #4268 "Avoid spiky svid renewal requests"](https://github.com/spiffe/spire/issues/4268) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [spiffe/spire issue #1754 "Configurable SVID Rotation Frequency"](https://github.com/spiffe/spire/issues/1754); [spiffe/spire doc/spire_agent.md](https://github.com/spiffe/spire/blob/main/doc/spire_agent.md)
**Analysis**: Half-life rotation is the canonical norm. `availability_target` is explicitly framed as "minimum time to gracefully handle SPIRE Server or Agent **downtime**" — i.e. the rotation overlap window exists so a restart/outage does not strand a workload with an expired cert.

**Finding 1.3 — CA signing keys: memory KeyManager loses the CA on restart; disk KeyManager persists it. Memory is "unsuitable for production."**
**Evidence**: Memory KeyManager = "A key manager which manages unpersisted keys in memory." Consequence: "When the SPIRE Server restarts, all keys are lost. This results in a new CA being generated on each restart, which breaks certificate continuity and is unsuitable for production environments." Disk KeyManager = "A key manager which manages keys persisted on disk… The same CA and keys are maintained across restarts, ensuring certificate continuity."
**Source**: [SPIFFE — SPIRE Server Configuration Reference](https://spiffe.io/docs/latest/deploying/spire_server/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [spiffe/spire doc/plugin_server_keymanager_disk.md](https://github.com/spiffe/spire/blob/main/doc/plugin_server_keymanager_disk.md) ("maintains a set of private keys that are persisted to disk"); [spiffe/spire doc/spire_server.md](https://github.com/spiffe/spire/blob/main/doc/spire_server.md)
**Analysis**: This is direct, authoritative support for Overdrive Decision 2: an ephemeral/regenerated-per-boot CA root **breaks chain-verification of pre-restart certs** — SPIRE names exactly this failure ("breaks certificate continuity… unsuitable for production"). The signing key (the *CA root/intermediate*) MUST persist; this is independent of whether *leaf* keys persist.

**Finding 1.4 — Default TTLs: X.509-SVID 1h, CA 24h. Upstream-CA reloaded on every CSR for seamless rotation.**
**Evidence**: Defaults: `default_x509_svid_ttl` = 1 hour, `ca_ttl` = 24 hours, `agent_ttl` inherits the SVID TTL. The disk UpstreamAuthority "reloads CA credentials on all CSR requests… ensures the spire-server process does not need to be restarted to load a new UpstreamAuthority from disk, providing a seamless rotation; and… a failed disk does not affect a running spire-server until the loaded UpstreamAuthority expires."
**Source**: [SPIFFE — SPIRE Server Configuration Reference](https://spiffe.io/docs/latest/deploying/spire_server/) — Accessed 2026-06-09; [spiffe/spire doc/plugin_server_upstreamauthority_disk.md](https://github.com/spiffe/spire/blob/main/doc/plugin_server_upstreamauthority_disk.md) — Accessed 2026-06-09
**Confidence**: High
**Verification**: cross-referenced two SPIRE docs above.
**Analysis**: Short leaf TTL (1h) + much longer CA TTL (24h) is the canonical asymmetry — leaf certs are disposable and re-minted frequently; the CA is the durable anchor. No notion of a "journaled multi-step rotation workflow" appears anywhere: rotation is a periodic rotator loop in the agent (timer-driven, fires at half-life), and signing is a single synchronous CSR→sign→return call.

**Finding 1.5 — Agent restart re-attests; it does not reload persisted leaf keys. Re-attestation is "a full restart."**
**Evidence**: "When an agent SVID expires or is otherwise invalid (e.g. agent has been evicted) the agent needs to re-attest. Currently the only way for this to happen is for the agent to undergo a full restart." A feature request seeks a "soft restart of only the right set of subsystems… impacted by the agent SVID not being valid."
**Source**: [spiffe/spire issue #1847 "Agent soft-restart for re-attestation"](https://github.com/spiffe/spire/issues/1847) — Accessed 2026-06-09
**Confidence**: Medium-High (GitHub issue from maintainers; cross-referenced with concepts doc)
**Verification**: [SPIFFE — SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/) (agent caches SVIDs; cache is volatile)
**Analysis**: On agent restart the cache is empty; the agent **re-attests** to the server and **re-fetches** SVIDs (re-issue / re-attest), it does not reload escrowed leaf keys from disk. This is direct support for Overdrive Decision 3's "re-attest / re-issue, no leaf keys at rest" posture — SPIRE's default is exactly that.

### 2. Cilium (eBPF / kernel-mediated — MOST relevant)

**Finding 2.1 — Cilium mutual authentication uses SPIFFE/SPIRE; the Cilium *agent* holds identity and requests SVIDs on behalf of workloads. Workloads do NOT hold cert material directly.**
**Evidence**: "Cilium uses SPIFFE… through its production implementation, SPIRE." Workloads do not hold cert material directly: when a workload identity is requested, the SPIRE agent "passes it back… in the SVID format… [which] includes a TLS keypair in the X.509 version." Crucially, "Cilium agents themselves obtain a common SPIFFE identity and can themselves ask for identities on behalf of other workloads — shifting certificate request responsibility to agents rather than individual pods."
**Source**: [Cilium docs — Mutual Authentication](https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/) — Accessed 2026-06-09
**Confidence**: High (official Cilium docs)
**Verification**: cross-reference pending (Isovalent blog / Cilium mutual-auth example) — see Finding 2.2.
**Analysis**: This is the closest analog to Overdrive's model: a node-local agent (Cilium agent ≈ Overdrive worker) holds identity material and brokers it on behalf of identity-unaware workloads. The pods themselves are not cert-holders — the agent is. Supports Overdrive's "worker holds material, workload is identity-unaware" split.

**Finding 2.2 — Cilium's mutual auth is brought "out-of-band for regular connections" — the handshake is decoupled from the data flow.**
**Evidence**: Cilium "brings the mutual authentication handshake out-of-band for regular connections," i.e. the auth handshake happens separately from the actual data path.
**Source**: [Cilium docs — Mutual Authentication](https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/) — Accessed 2026-06-09
**Confidence**: Medium (single official source for this exact phrasing; needs cross-ref)
**Verification**: PENDING — to cross-reference with cilium.io blog / Isovalent.
**Analysis**: Cilium's current GA mutual-auth does the *authentication* via SPIRE-issued certs out-of-band, then enforces in the datapath; it is NOT (in the GA path) terminating TLS in the kernel with kTLS the way Overdrive proposes. This is a relevant *divergence* to flag — see Knowledge Gaps.

### 3. Istio + Envoy SDS

**Finding 3.1 — Issuance is a single synchronous CSR→sign→return; the istio-agent generates the key + CSR locally and istiod signs it.**
**Evidence**: Six-step flow: (1) "istiod offers a gRPC service to take certificate signing requests (CSRs)." (2) "When started, the Istio agent creates the private key and CSR, and then sends the CSR with its credentials to istiod for signing." (3) "The CA in istiod validates the credentials… Upon successful validation, it signs the CSR to generate the certificate." (4) "When a workload is started, Envoy requests the certificate and key from the Istio agent via the Envoy secret discovery service (SDS) API." (5) "The Istio agent sends the certificates received from istiod and the private key to Envoy via the Envoy SDS API." (6) "Istio agent monitors the expiration of the workload certificate. The above process repeats periodically for certificate and key rotation."
**Source**: [Istio — Security concepts](https://istio.io/latest/docs/concepts/security/) — Accessed 2026-06-09
**Confidence**: High (official Istio docs, canonical numbered steps)
**Verification**: [Istio — Provisioning Identity through SDS (1.1)](https://istio.io/v1.1/docs/tasks/security/auth-sds/); [Istio security concepts (1.16)](https://istio.io/v1.16/docs/concepts/security/)
**Analysis**: For an *internal* CA, issuance is a single signing call, not a multi-step wait-bearing sequence. Rotation is a periodic "monitor expiry → repeat the issuance" loop in the agent — a control loop, not a journaled workflow. Direct support for Overdrive Decision 1's "reconciler/periodic re-issue" framing.

**Finding 3.2 — SDS delivers cert+key in-memory to Envoy; no on-disk secret volume, no Kubernetes Secret, no Envoy restart on rotation.**
**Evidence**: "The sidecar Envoy is able to dynamically renew the key and certificate through the SDS API, and certificate rotations no longer require Envoy to restart." "The secret volume mount is no longer needed: the reliance on Kubernetes secrets is eliminated." The private key is generated by the agent and pushed over SDS in-memory.
**Source**: [Istio — Provisioning Identity through SDS (1.1)](https://istio.io/v1.1/docs/tasks/security/auth-sds/) — Accessed 2026-06-09; [Istio — Security concepts](https://istio.io/latest/docs/concepts/security/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: cross-referenced two Istio docs.
**Analysis**: Leaf key/cert live in memory in the agent/Envoy process and are streamed over SDS; they are explicitly NOT escrowed to disk or to a K8s Secret. On Envoy/agent restart, a fresh key+CSR is generated and re-issued — no leaf-key reload. Supports Overdrive Decision 3 (leaf material in memory, re-issue on restart). Note: istiod's *own* CA signing key persistence is a separate concern (typically a mounted CA secret); the leaf path is ephemeral.

### 4. Linkerd

**Finding 4.1 — Proxy private key is generated at startup into a tmpfs emptyDir that "stays in memory and never leaves the pod." Proxy certs expire after 24h.**
**Evidence**: "TLS certificates issued to proxies expire after 24 hours. At startup, the proxy generates a private key, stored in a tmpfs emptyDir which stays in memory and never leaves the pod. The proxy connects to the control plane's identity component, validating the connection… with the trust anchor, and issues a certificate signing request (CSR)." "This in-memory storage ensures that proxy private keys never persist to disk."
**Source**: [Linkerd — Automatic mTLS](https://linkerd.io/2.19/features/automatic-mtls/) — Accessed 2026-06-09 (via search-engine excerpt; direct fetch failed — flagged as single-render-path source, cross-referenced below)
**Confidence**: High (official Linkerd docs; corroborated by an independent Linkerd workshop writeup)
**Verification**: [Linkerd — "A deep dive into Kubernetes mTLS with Linkerd" (2023-01-30)](https://linkerd.io/2023/01/30/mtls-and-linkerd/); [Linkerd — Generating your own mTLS root certificates](https://linkerd.io/2-edge/tasks/generate-certificates/)
**Analysis**: This is the single most explicit statement in the corpus of the "leaf key in memory, never on disk, re-generated on restart" posture — and it is framed as a *security feature*, not a limitation. On proxy restart Linkerd generates a NEW key and issues a fresh CSR (re-issue), never reloading escrowed leaf material. Direct, strong support for Overdrive Decision 3.

**Finding 4.2 — Two-layer trust hierarchy: a trust anchor (root, 365d default) + an issuer cert/key, persisted as a cluster Secret. These persist across restarts; workload certs chain to them.**
**Evidence**: "On the control plane side, Linkerd maintains a set of credentials in the cluster: a trust anchor, and an issuer certificate and private key… workload certificates are issued by the Linkerd identity issuer, and the identity issuer is issued by the Linkerd trust anchor." The issuer cert/key live in "the linkerd-identity-issuer Secret in the linkerd namespace." Trust anchor validity is "365 days if generated by linkerd install."
**Source**: [Linkerd — Manually Rotating Control Plane TLS Credentials](https://linkerd.io/2-edge/tasks/manually-rotating-control-plane-tls-credentials/) — Accessed 2026-06-09; [Linkerd — Automatically Rotating Control Plane TLS Credentials](https://linkerd.io/2-edge/tasks/automatically-rotating-control-plane-tls-credentials/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: cross-referenced two Linkerd task docs + concepts excerpt.
**Analysis**: The CA/issuer (the durable anchor) IS persisted — to a Secret store — while leaf material is NOT. This is the same split Overdrive proposes: persist the CA root/intermediate, keep leaf keys ephemeral. The 3-tier structure (anchor→issuer→workload) mirrors Overdrive's root→intermediate→leaf.

**Finding 4.3 — Rotation is asymmetric: issuer rotation is auto-detected by proxies (no restart needed); trust-anchor rotation is a deliberate multi-step bundle-and-drain process requiring restarts.**
**Evidence**: "When cert-manager rotates the identity issuer, it will update the linkerd-identity-issuer Secret… at which point every Linkerd proxy will automatically notice this change and start using the new certificate." But trust-anchor rotation: "bundle it with the old one, rotate the issuer certificate and key pair, and finally remove the old trust anchor from the bundle. All meshed workloads need explicit restarts." "Rotating the trust anchor without downtime is a multi-step process."
**Source**: [Linkerd — Automatically Rotating Control Plane TLS Credentials](https://linkerd.io/2-edge/tasks/automatically-rotating-control-plane-tls-credentials/) — Accessed 2026-06-09; [Linkerd — Manually Rotating Control Plane TLS Credentials](https://linkerd.io/2-edge/tasks/manually-rotating-control-plane-tls-credentials/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: cross-referenced two Linkerd task docs.
**Analysis**: Nuance for Overdrive Decision 1: the genuinely *multi-step, overlap-bearing* sequence in Linkerd is **trust-anchor rotation** (bundle-old-and-new → rotate issuer → drain old), not leaf rotation. Leaf rotation is a continuous, automatic, restart-free control loop. If any part of Overdrive's cert lifecycle is "workflow-shaped" it would be a *root/intermediate* rotation (bundle → re-sign → drain), not routine leaf re-issue. Even so, Linkerd implements anchor rotation as an operator-driven runbook / cert-manager reconcile, not a durable-execution journaled workflow.

### 5. HashiCorp Vault PKI / Consul Connect

**Finding 5.1 — Vault PKI does NOT store issued leaf private keys; the key is returned to the requesting client exactly once and then discarded. Only CA private keys are stored.**
**Evidence**: "this secrets engine does _not_ store generated private keys, except for CA certificates." "The only place a private key is ever returned is to the requesting client." Corollary from setup docs: "The private key is not stored. If you do not save the private key from the response, you will need to request a new certificate."
**Source**: [Vault — PKI secrets engine considerations](https://developer.hashicorp.com/vault/docs/secrets/pki/considerations) — Accessed 2026-06-09 (official vendor doc; reputation treated as official-vendor — cross-referenced below)
**Confidence**: High
**Verification**: [Vault — PKI secrets engine](https://developer.hashicorp.com/vault/docs/secrets/pki); [Vault — Set up and use the PKI secrets engine](https://developer.hashicorp.com/vault/docs/secrets/pki/setup)
**Analysis**: The most prominent general-purpose PKI engine in the industry makes "we do not escrow leaf private keys; CA keys are the only persisted keys" a *design invariant*. This is direct, authoritative support for Overdrive Decision 3's posture — persisting workload leaf private keys at rest is contrary to the dominant design; the CA key is the one persisted key.

**Finding 5.2 — Vault's stated philosophy: keep certificate lifetimes short; a lost key is recoverable simply by letting the short-lived cert expire and re-issuing.**
**Evidence**: Section heading "Keep certificate lifetimes short, for CRL's sake," aligned with "Vault's philosophy of short-lived secrets," and "in most cases, if the key is lost, the certificate can simply be ignored, as it will expire shortly."
**Source**: [Vault — PKI secrets engine considerations](https://developer.hashicorp.com/vault/docs/secrets/pki/considerations) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [Vault — PKI rotation primitives](https://developer.hashicorp.com/vault/docs/secrets/pki/rotation-primitives)
**Analysis**: Short-lived leaf + re-issue-on-loss is the explicit recommended posture. "If the key is lost, ignore the cert; it expires shortly" is precisely Overdrive's restart story — a lost in-memory leaf key on restart is a non-event because the cert is short-lived and re-minted. Reinforces "no leaf escrow needed."

**Finding 5.3 — Vault PKI supports rotation primitives and HSM/KMS-managed CA keys; the CA key may be protected by an external KMS/HSM rather than stored as plaintext.**
**Evidence**: Vault offers "rotation primitives" for the PKI engine and a "Generate certificates with HSM or KMS managed keys" path, where the CA key is held in an external KMS/HSM.
**Source**: [Vault — PKI rotation primitives](https://developer.hashicorp.com/vault/docs/secrets/pki/rotation-primitives) — Accessed 2026-06-09; [Vault — Generate certificates with HSM or KMS managed keys](https://developer.hashicorp.com/vault/tutorials/pki/managed-key-pki) — Accessed 2026-06-09
**Confidence**: Medium-High (official vendor docs; 2 sources)
**Verification**: cross-referenced two Vault docs.
**Analysis**: CA-key-at-rest protection options range from Vault's own encrypted storage (its "barrier" / seal) up to HSM/KMS. Supports Overdrive Decision 2's "envelope-encrypted root key at rest" as a recognized middle-ground posture (Vault's barrier ≈ envelope encryption; HSM/KMS is the stronger tier).

### 6. cert-manager (Kubernetes)

**Finding 6.1 — Renewal is a continuous reconcile/control loop; default renewal at 2/3 of cert duration; `renewBefore` / `renewBeforePercentage` tune the trigger.**
**Evidence**: "Once an X.509 certificate has been issued, cert-manager will calculate the renewal time… By default this will be 2/3 through the X.509 certificate's duration. If `spec.renewBefore` or `spec.renewBeforePercentage` has been set, it will be the effective `spec.renewBefore` amount of time before expiry." "The control loop continuously monitors certificates and triggers renewal based on the configured `renewBefore` timing."
**Source**: [cert-manager — Certificate resource](https://cert-manager.io/docs/usage/certificate/) — Accessed 2026-06-09
**Confidence**: High (official cert-manager docs)
**Verification**: [cert-manager — FAQ](https://cert-manager.io/docs/faq/); [cert-manager — Certificate Resources (v1.4)](https://cert-manager.io/v1.4-docs/usage/certificate/)
**Analysis**: cert-manager is the canonical *reconciler*-shaped renewal model — a Kubernetes controller that observes desired (Certificate spec) vs actual (the Secret's cert) and re-issues when the renewal threshold is reached. No journaling, no durable-workflow primitive. Direct support for Overdrive Decision 1's "control loop / reconciler" framing. Note the trigger is ~2/3-life here vs SPIRE's 1/2-life — both are "fraction-of-lifetime," differing by constant.

**Finding 6.2 — cert+key are stored in a Kubernetes Secret and REUSED until renewal; re-issuance is triggered when the Secret is missing/corrupt or the spec changes. `rotationPolicy: Never` reuses the key; `Always` regenerates it.**
**Evidence**: "To determine if a certificate needs to be re-issued, cert-manager looks at the spec of Certificate resource and latest CertificateRequests as well as the data in Secret containing the X.509 certificate. The issuance process will always get triggered if the Secret… does not exist, is missing private key or certificate data or contains corrupt data." `rotationPolicy: Never`: "A private key is only generated if one does not already exist in the target Secret… All further issuances will re-use this private key." `rotationPolicy: Always` (recommended): "a new private key will be generated each time an action triggers the reissuance."
**Source**: [cert-manager — Certificate resource](https://cert-manager.io/docs/usage/certificate/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: [cert-manager — FAQ](https://cert-manager.io/docs/faq/)
**Analysis**: This is the clearest example of a **reuse-until-renewal** restart model with leaf keys at rest — but note: cert-manager's at-rest store IS a Kubernetes Secret (etcd, base64, optionally encrypted-at-rest). It is a *deliberately different* posture from SPIRE/Istio/Linkerd/Vault (memory-only leaf). cert-manager's "look at the Secret to decide whether re-issue is needed; if present and valid, don't re-mint" is the closest analog to Overdrive's proposed "respect the audit row / existing material until near-expiry, don't re-mint" — except cert-manager respects the *actual cert bytes in the Secret*, whereas Overdrive would respect an *audit fact row* (no bytes). This nuance is load-bearing for Decision 3 — see synthesis.

### 7. Kubernetes kubelet certificate rotation

**Finding 7.1 — kubelet rotation is a CSR-based control loop: it generates a new key + CSR as the cert approaches expiry, triggering "between 30% and 10% of the time remaining."**
**Evidence**: "Kubernetes contains kubelet certificate rotation, that will automatically generate a new key and request a new certificate from the Kubernetes API as the current certificate approaches expiration." "As the expiration of the signed certificate approaches, the kubelet will automatically issue a new certificate signing request… This can happen at any point between 30% and 10% of the time remaining on the certificate."
**Source**: [Kubernetes — Configure Certificate Rotation for the Kubelet](https://kubernetes.io/docs/tasks/tls/certificate-rotation/) — Accessed 2026-06-09 (official)
**Confidence**: High
**Verification**: [Kubernetes — TLS bootstrapping](https://kubernetes.io/docs/reference/access-authn-authz/kubelet-tls-bootstrapping/); [Kubernetes — Certificate Signing Requests](https://kubernetes.io/docs/reference/access-authn-authz/certificate-signing-requests/)
**Analysis**: Another fraction-of-lifetime control loop (30%–10% remaining ≈ rotate at 70%–90% elapsed). The jittered window (30%→10%) is an anti-thundering-herd measure — relevant to Overdrive's rotation-trigger design. No journaling; the kubelet's `certificate manager` is a goroutine loop, not a durable workflow.

**Finding 7.2 — kubelet writes the signed cert to disk (`--cert-dir`) and reuses it across restart via the `kubelet-client-current.pem` symlink.**
**Evidence**: The kubelet "will retrieve the signed certificate from the Kubernetes API and write that to disk, in the location specified by `--cert-dir`." kubeadm "configures a kubelet with automatic rotation of client certificates by using the `/var/lib/kubelet/pki/kubelet-client-current.pem` symlink… indicating that certificates stored on disk are persisted and reused across kubelet restarts." On bootstrap, "if it is configured to bootstrap… it will use its initial certificate to connect… and issue a certificate signing request."
**Source**: [Kubernetes — Configure Certificate Rotation for the Kubelet](https://kubernetes.io/docs/tasks/tls/certificate-rotation/) — Accessed 2026-06-09; [Kubernetes — Certificate Management with kubeadm](https://kubernetes.io/docs/tasks/administer-cluster/kubeadm/kubeadm-certs/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: cross-referenced two Kubernetes docs.
**Analysis**: kubelet is the **on-disk reuse-across-restart** posture (contrast SPIRE/Istio/Linkerd memory-only). On restart the kubelet reads the still-valid cert from `--cert-dir` and reuses it — it does NOT re-issue if the on-disk cert is still valid. This is the strongest precedent for a "reuse, don't re-mint" restart model — but it reuses *the actual cert+key bytes on disk*, which is a different identity-owner from Overdrive (kubelet IS the workload here; the cert is the node's own credential, not a brokered workload SVID).

### 8. Talos Linux

**Finding 8.1 — Talos auto-rotates all server-side certs (etcd, Kubernetes, Talos API). Routine leaf rotation is automatic and ties to node reboot/upgrade.**
**Evidence**: "Talos Linux automatically manages and rotates all server side certificates for etcd, Kubernetes, and the Talos API. However, the kubelet needs to be restarted at least once a year in order for the certificates to be rotated, and any upgrade/reboot of the node will suffice for this effect."
**Source**: [Talos — How to manage PKI and certificate lifetimes (v1.10)](https://www.talos.dev/v1.10/talos-guides/howto/cert-management/) — Accessed 2026-06-09 (official); [Sidero Labs — cert-management (v1.7)](https://docs.siderolabs.com/talos/v1.7/security/cert-management) — Accessed 2026-06-09
**Confidence**: High (two official Talos/Sidero docs)
**Verification**: cross-referenced talos.dev + docs.siderolabs.com.
**Analysis**: Routine cert rotation is a managed background process, not a journaled workflow. Relevant because Overdrive uses Talos-shape operator auth — the design influence is direct.

**Finding 8.2 — Talos *CA rotation* is an explicit ORDERED multi-step bundle-and-drain: add new CA to `acceptedCAs` (trust both) → switch active `.machine.ca` to new CA+key (keep old in `acceptedCAs`) → remove old CA. It does not interrupt connections and does not require a reboot; a `--dry-run` previews the steps.**
**Evidence**: Step 1: "add the new CA certificate to `.machine.acceptedCAs`, allowing nodes to trust both old and new CAs temporarily." Step 2: "Update `.machine.ca` with the new CA certificate and key, while retaining the old CA in `.machine.acceptedCAs`." Step 3: "Delete the old CA certificate from `.machine.acceptedCAs` across all nodes." "Talos API CA rotation doesn't interrupt connections within the cluster, and it doesn't require a reboot of the nodes." Preview: run "with `--dry-run=true`."
**Source**: [Sidero Labs — CA Rotation (v1.10)](https://docs.siderolabs.com/talos/v1.10/security/ca-rotation) — Accessed 2026-06-09 (official; redirected from talos.dev/v1.10/advanced/ca-rotation); [Talos — CA Rotation (v1.7)](https://www.talos.dev/v1.7/advanced/ca-rotation/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: cross-referenced two Talos/Sidero docs.
**Analysis**: This is the canonical **trust-overlap rollover** for a *CA root*: trust both → switch active → drain old. It is genuinely multi-step and ordering-sensitive (the "bundle" must exist before the switch, the switch before the drain) — i.e. it has the structural shape that *could* justify a journaled workflow. But Talos implements it as a CLI-driven `rotate-ca` command applying machine-config patches in sequence (operator-initiated, idempotent config reconcile), NOT as a durable-execution journaled workflow. The dry-run + ordered patches read as a controlled procedure / reconcile, not a Temporal-style workflow.

**Finding 8.3 — The Talos CA key+cert is stored in the machine configuration (`.machine.ca`) and the `secrets.yaml` secrets bundle; the bundle is the durable source for regenerating config.**
**Evidence**: "The CA certificate and key are stored in the machine configuration under `.machine.ca`." "Update `secrets.yaml` with the new CA key and certificate if using that for machine configuration generation." A saved secrets bundle "can be used with `talosctl gen config --with-secrets` to regenerate configuration."
**Source**: [Sidero Labs — CA Rotation (v1.10)](https://docs.siderolabs.com/talos/v1.10/security/ca-rotation) — Accessed 2026-06-09; [Talos — Managing Talos PKI (v1.4)](https://www.talos.dev/v1.4/talos-guides/configuration/managing-pki/) — Accessed 2026-06-09
**Confidence**: High
**Verification**: cross-referenced two Talos docs.
**Analysis**: CA key is durably persisted (machine config + secrets bundle); leaf/server certs are derived and rotated. Same root-persists / leaf-ephemeral split as every other comparator. Supports Overdrive Decision 2.

---

## Cross-Comparator Synthesis Table
| System | Rotation mechanism | CA-root persistence | Leaf-key at rest? | Restart = reuse \| reissue \| re-attest | Who holds material |
|--------|--------------------|--------------------|--------------------|------------------------------------------|--------------------|
| **SPIRE** | Agent rotator loop; **half-life (50%)** default, `availability_target` + ≥12h grace | Disk KeyManager / UpstreamAuthority persists CA; **memory KeyManager loses CA on restart = "unsuitable for production"** | **No** (agent cache, in memory) | **Re-attest + re-issue** (volatile cache) | Node-local **agent** (broker), via Workload API |
| **Cilium** | SPIFFE/SPIRE-backed; auth out-of-band of data flow | Via SPIRE server (persisted) | **No** (agent-held) | Re-issue/re-attest (SPIRE-backed) | **Cilium agent** holds identity, asks on behalf of pods |
| **Istio + Envoy SDS** | istio-agent monitors expiry → periodic re-issue loop | istiod CA key persisted (mounted CA secret) | **No** (SDS pushes cert+key in-memory to Envoy; no Secret/volume) | **Re-issue** (new key+CSR on start) | **Sidecar** (istio-agent → Envoy), in memory |
| **Linkerd** | Continuous auto-renew; 24h proxy certs; issuer auto-detected | Trust anchor (365d) + issuer cert/key in cluster Secret (persisted) | **No** (tmpfs; "stays in memory… never persist[s] to disk") | **Re-issue** (new key + CSR on proxy start) | **Sidecar** (linkerd-proxy), in memory (tmpfs) |
| **Vault PKI / Consul Connect** | Single issue call; short TTL philosophy; rotation primitives | **CA key persisted** (Vault barrier / optional HSM/KMS); only CA keys stored | **No** ("does not store generated private keys, except for CA"; returned to client once) | Re-issue on request (client lost key → "ignore cert, expires shortly") | **Requesting client** (returned once, not escrowed) |
| **cert-manager** | **Reconciler control loop**; renew at **2/3 of duration** default; `renewBefore` | CA Issuer persisted (Secret / external) | **Yes** (K8s Secret) — but `rotationPolicy: Always` *recommended* to regenerate key | **Reuse from Secret if valid**; re-issue only at threshold / missing / corrupt | Stored in **K8s Secret**, mounted by workload |
| **kubelet** | CSR control loop; renew **between 30%–10% time remaining** (jittered) | Cluster CA persisted (kube-controller-manager signer) | **Yes** (`--cert-dir` on disk) | **Reuse from disk if valid** (`kubelet-client-current.pem`) | **The workload itself** (kubelet = identity owner) on disk |
| **Talos Linux** | Auto-rotate server certs; **CA rotation = ordered bundle→switch→drain** (`rotate-ca`, no reboot, `--dry-run`) | CA key+cert in machine config `.machine.ca` + `secrets.yaml` bundle (persisted) | Server/node certs derived; leaf rotated on reboot/upgrade | Reuse persisted material; rotate on reboot | **The node / machine** (Talos-managed) |

---

## Cross-Cutting Findings

### Rotation trigger policy norms (fraction-of-lifetime, renewBefore, overlap)
**Finding X.1 — Every comparator triggers rotation at a fraction of the cert lifetime; values cluster around 50–67% elapsed, with jitter/overlap windows to absorb downtime and avoid herds.**
- SPIRE: **half-life (50%)** default; `availability_target` shifts it earlier and mandates a ≥12h grace/overlap window framed explicitly as "minimum time to gracefully handle SPIRE Server or Agent **downtime**." ([SPIRE Agent config](https://spiffe.io/docs/latest/deploying/spire_agent/), [spire#4268](https://github.com/spiffe/spire/issues/4268))
- cert-manager: **2/3 of duration** default; tunable via `renewBefore` / `renewBeforePercentage`. ([cert-manager Certificate](https://cert-manager.io/docs/usage/certificate/))
- kubelet: renews **between 30% and 10% of time remaining** (i.e. 70–90% elapsed) — a jittered window. ([k8s cert rotation](https://kubernetes.io/docs/tasks/tls/certificate-rotation/))
- Linkerd: 24h proxy certs, continuous auto-renew well before expiry. ([Linkerd auto-mTLS](https://linkerd.io/2.19/features/automatic-mtls/))
**Confidence**: High (4 independent official sources)
**Analysis**: "Rotate at a fraction of lifetime, with an overlap window so a swap/restart/outage drops no connection" is universal. The exact fraction varies (50% SPIRE, 67% cert-manager, 70–90% kubelet) — Overdrive should pick one fraction and document an overlap window sized to its expected restart/outage duration (SPIRE's ≥12h grace is the most explicit precedent).

### Internal-CA issuance: single synchronous signing call vs multi-step sequence (contrast ACME)
**Finding X.2 — For an INTERNAL CA, issuance is a single synchronous CSR→validate→sign→return call in every comparator. The genuinely multi-step, wait-bearing flow is ACME (external/public certs) — order → DNS-01 challenge → wait for propagation → validate → finalize — which none of the internal paths share.**
- SPIRE: agent sends CSR; server "signs and returns" — one call. ([SPIRE Concepts](https://spiffe.io/docs/latest/spire-about/spire-concepts/))
- Istio: "validates the credentials carried in the CSR. Upon successful validation, it signs the CSR to generate the certificate" — one call. ([Istio Security](https://istio.io/latest/docs/concepts/security/))
- Vault PKI: issue endpoint returns cert+key in one response. ([Vault PKI](https://developer.hashicorp.com/vault/docs/secrets/pki))
- ACME (contrast): inherently multi-step with an external DNS/HTTP validation wait — this is the only flow with the "order → wait → validate → finalize" shape. ([cert-manager ACME troubleshooting](https://cert-manager.io/docs/troubleshooting/acme/))
**Confidence**: High
**Analysis**: Decisive for Overdrive Decision 1. Overdrive's CA is **internal** (rcgen, no ACME, no DNS challenge). Internal signing has no external wait to journal across — it is a single synchronous operation. The "request → wait for DNS propagation → validate → publish" four-step shape in Overdrive's own workflow precedent (`.claude/rules/workflows.md`) describes the **ACME/public** cert case, which Overdrive's internal CA does not perform. This is the cleanest evidence that internal-cert *issuance* is not workflow-shaped.

### Who holds leaf material (workload / sidecar / kernel) → restart recovery
**Finding X.3 — The holder of leaf material dictates restart recovery, and the dominant pattern is: a node-local agent/sidecar holds in-memory leaf material on behalf of identity-unaware workloads; on restart it RE-ISSUES (re-attests + fetches fresh) rather than reloading escrowed keys.**
- SPIRE: agent caches SVIDs in memory; on restart re-attests and re-fetches (cache is volatile). ([spire#1847](https://github.com/spiffe/spire/issues/1847))
- Cilium: the **Cilium agent** holds a SPIFFE identity and "ask[s] for identities on behalf of other workloads" — pods are not cert-holders. ([Cilium mutual auth](https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/))
- Istio/Linkerd: the **sidecar** (Envoy / linkerd-proxy) holds the in-memory key; on restart it generates a new key + CSR. ([Istio Security](https://istio.io/latest/docs/concepts/security/), [Linkerd auto-mTLS](https://linkerd.io/2.19/features/automatic-mtls/))
- kubelet: the **workload itself** (kubelet) holds its cert on disk and reuses it on restart. ([k8s kubeadm certs](https://kubernetes.io/docs/tasks/administer-cluster/kubeadm/kubeadm-certs/))
**Confidence**: High (5 sources across 4 comparators)
**Analysis**: Overdrive's "worker holds the SVID material; workload is identity-unaware" maps directly onto the **Cilium-agent / sidecar-broker** model — and that model's restart recovery is RE-ISSUE/RE-ATTEST, not escrow-reload. kubelet (on-disk reuse) is the outlier, and it is the outlier precisely because the kubelet IS the workload/identity-owner, not a broker.

### Security posture on persisting workload leaf private keys at rest
**Finding X.4 — Persisting workload leaf private keys at rest is the minority posture and is explicitly contrasted against the "short-lived, never-escrowed, generated-at-runtime" model that SPIFFE/Vault treat as the secure default.**
**Evidence**:
- SPIFFE: "all private keys (and corresponding certificates) are short lived, rotated frequently and automatically" with the rationale "to minimize exposure from a key being leaked or compromised"; the Workload API model means "your application need not co-deploy any authentication secrets." ([SPIFFE Concepts](https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/))
- Vault PKI: "this secrets engine does _not_ store generated private keys, except for CA certificates"; "the only place a private key is ever returned is to the requesting client"; "if the key is lost, the certificate can simply be ignored, as it will expire shortly." ([Vault PKI considerations](https://developer.hashicorp.com/vault/docs/secrets/pki/considerations))
- Linkerd: proxy key in tmpfs "stays in memory and never leaves the pod… never persist[s] to disk." ([Linkerd auto-mTLS](https://linkerd.io/2.19/features/automatic-mtls/))
- Counter-examples (DO persist leaf at rest): cert-manager (K8s Secret) and kubelet (`--cert-dir`) — but cert-manager *recommends* `rotationPolicy: Always` to "rotat[e] both certificate and private key simultaneously [to] reduce exposure risk from compromised keys." ([cert-manager Certificate](https://cert-manager.io/docs/usage/certificate/))
**Confidence**: High (4+ sources; one explicit pro-escrow counter-pattern with its own caveat)
**Analysis**: The weight of evidence treats memory-only / no-escrow leaf keys as the secure default; the systems that DO persist leaf keys (cert-manager, kubelet) do so because the holder IS the identity owner and there is no separate broker — and even they recommend regenerating the key on each issuance. Overdrive's "no leaf private keys at rest, held in memory and consumed by the kernel" posture is squarely the secure-default camp. **Dissent flagged**: cert-manager/kubelet show on-disk leaf escrow is *not categorically wrong* — it is acceptable when the holder owns the identity and the store is access-controlled (etcd encryption-at-rest, file perms). Overdrive's choice is defensible AND aligned with the broker-model majority.

### Evidence for/against "read audit record on restart → don't re-mint a still-valid cert"
**Finding X.5 — Multiple systems implement "on restart, inspect existing still-valid material and DON'T re-mint" — but they inspect the actual cert bytes (kubelet on-disk cert; cert-manager's Secret), not a metadata-only audit row. No comparator was found that decides re-issuance from an *issuance/audit record that lacks the cert bytes*.**
**Evidence**:
- kubelet: on restart reads `kubelet-client-current.pem`; if still valid, reuses it (re-issues only "as the current certificate approaches expiration"). ([k8s cert rotation](https://kubernetes.io/docs/tasks/tls/certificate-rotation/), [k8s kubeadm certs](https://kubernetes.io/docs/tasks/administer-cluster/kubeadm/kubeadm-certs/))
- cert-manager: "looks at… the data in [the] Secret containing the X.509 certificate" and only re-issues if it "does not exist, is missing private key or certificate data or contains corrupt data," or the renewal threshold is reached. ([cert-manager Certificate](https://cert-manager.io/docs/usage/certificate/))
- SPIRE: the OPPOSITE — the agent's cache is volatile, so a restarted agent re-attests and re-fetches; it does NOT consult a persisted record to skip re-issuance. ([spire#1847](https://github.com/spiffe/spire/issues/1847))
**Confidence**: Medium-High (the "reuse-if-valid" pattern is well-attested; the metadata-only-row variant is NOT attested anywhere — this is a genuine gap)
**Analysis**: This is the most consequential finding for Overdrive Decision 3, and it cuts two ways. (1) The general principle "if valid material exists, don't re-mint" is strongly supported (kubelet, cert-manager). (2) BUT in every supporting case the decision is made by re-reading the **actual cert/key**, because they ARE persisted. Overdrive's proposed model — decide from an **audit *fact* row** (`spiffe_id, serial, not_before, not_after`) with **no cert bytes and no leaf key at rest** — has NO direct precedent. The closest analog (SPIRE, the model Overdrive most resembles) does the opposite: it discards the volatile cache and **re-issues** on restart precisely *because* it holds no durable leaf material. **This is a single-source-gap that must be flagged**: Overdrive would be combining "no leaf keys at rest" (SPIRE/Linkerd/Vault posture) with "don't re-mint based on a metadata row" (kubelet/cert-manager posture) — a combination no surveyed system uses. The unresolved tension: if the leaf key is only in volatile memory, a restart that loses the key MUST re-mint regardless of the audit row, because the row cannot reconstruct the lost private key. The audit row can only justify *not re-issuing* if the leaf material also survived the restart — which contradicts "no leaf keys at rest." See Implications → Decision 3 and Conflicting Information.

---

## Implications for Overdrive's Three Decisions

### Decision 1 — Rotation: durable journaled WORKFLOW vs control-loop/reconciler

**What the evidence supports: a control loop / reconciler, NOT a durable journaled workflow — for routine internal leaf rotation.**

- **No surveyed system models internal cert rotation as a durable-execution journaled workflow.** Every comparator implements rotation as a continuous control loop: SPIRE's agent rotator goroutine (Finding 1.2, X.1), Istio's "monitor expiry → repeat issuance" loop (3.1), cert-manager's reconcile controller (6.1), kubelet's certificate-manager loop (7.1), Linkerd's continuous auto-renew (4.1). This is the **reconciler** shape, not the workflow shape. (5 independent comparators — High confidence.)
- **Internal-CA issuance is a single synchronous signing call** (Finding X.2). The genuinely multi-step, wait-bearing flow — order → DNS-01 → wait for propagation → validate → finalize — is **ACME / external public-cert issuance**, which Overdrive's internal rcgen CA does not perform. There is no external wait to journal across. Per Overdrive's own `.claude/rules/workflows.md` decision rule, a workflow requires "≥2 ordered steps, at least one with an external side effect **or a wait**," where "a crash mid-sequence must not repeat completed steps." A single synchronous sign has neither the multi-step structure nor an expensive-to-repeat completed step.
- **Maps cleanly to Overdrive's reconciler discipline** (`.claude/rules/reconcilers.md`): cert rotation is "keep the issued cert looking valid (≥ fraction-of-lifetime remaining) forever" — a *forever-converging standing invariant*, the explicit inverse of a terminating workflow. Desired = "a current, non-expired SVID exists for each running workload"; actual = the audit row's `not_after`; the reconciler re-mints when `now ≥ not_before + fraction·(not_after − not_before)`.

**Where a workflow *could* be justified — and the dissent.** The one place the surveyed systems use a genuinely ordered, overlap-bearing, crash-sensitive sequence is **CA-ROOT/intermediate rotation** (Talos `rotate-ca`: add-to-acceptedCAs → switch active → drain old, Finding 8.2; Linkerd trust-anchor: bundle-old-and-new → rotate issuer → remove old, Finding 4.3). That bundle→switch→drain sequence *is* workflow-shaped (re-running a completed "switch active CA" step from the top would be incorrect). Even so, **both Talos and Linkerd implement it as an operator-driven CLI procedure / cert-manager reconcile, not a durable-execution journaled workflow** — so even root rotation is not done as a Temporal/Restate-style workflow in the surveyed corpus. **Recommendation**: routine internal SVID rotation = reconciler action (strong, multi-source). If Overdrive ever automates *CA-root* rotation (trust-bundle overlap rollover), revisit whether that specific ordered sequence warrants the workflow primitive — that is the only candidate, and it is out of scope for routine leaf rotation.

### Decision 2 — CA-root persistence across restart

**What the evidence supports: the CA root/intermediate MUST persist across restart; an ephemeral/per-boot root is a named, well-understood failure. Envelope-encryption-at-rest is a recognized middle tier; HSM/KMS is the stronger tier.**

- **Direct, authoritative evidence that an ephemeral root breaks chain-verification.** SPIRE names exactly this: memory KeyManager → "When the SPIRE Server restarts, all keys are lost. This results in a new CA being generated on each restart, which **breaks certificate continuity** and is **unsuitable for production**" (Finding 1.3). Disk KeyManager → "The same CA and keys are maintained across restarts, ensuring certificate continuity." This is the single clearest statement of what breaks if Overdrive's root is regenerated per boot: **every SVID issued before the restart fails to chain-verify after**, because they were signed by a key the new boot no longer holds, and peers validating against the old root reject the new one (and vice versa).
- **Universal pattern: root persists, leaf is ephemeral.** Every comparator persists the CA/issuer and treats leaves as disposable — SPIRE (1.3/1.4), Linkerd (anchor+issuer in Secret, 4.2), Vault ("only CA keys stored", 5.1), Talos (`.machine.ca` + `secrets.yaml`, 8.3), cert-manager/kubelet (cluster CA persisted). The long-CA-TTL / short-leaf-TTL asymmetry (SPIRE: 24h CA vs 1h SVID, Finding 1.4) is the canonical shape.
- **Root-key-at-rest protection spans a recognized spectrum**: encrypted store / "barrier" (Vault's seal ≈ envelope encryption), cluster Secret with etcd encryption-at-rest (Linkerd, cert-manager), up to **HSM/KMS-managed keys** (Vault managed-key PKI, Finding 5.3). Overdrive's "root key envelope-encrypted at rest when persistent" sits in the recognized middle tier — defensible, with HSM/KMS as the documented stronger option if the threat model demands it.

**Recommendation (High confidence, multi-source):** persist the built-in CA root + intermediate durably; never regenerate per boot. Envelope-encrypt the root key at rest (Overdrive's stated plan) — aligned with Vault's barrier model. Keep CA TTL ≫ leaf TTL. The audit row alone is insufficient for Decision 2: it persists issuance *facts*, not the *signing key* — so CA-key persistence is a separate, mandatory mechanism.

### Decision 3 — Restart recovery: REUSE vs RE-ISSUE, and leaf-keys-at-rest

**What the evidence supports (two parts that pull in tension for Overdrive's specific proposal):**

**(a) Leaf private keys at rest — Overdrive's "no leaf keys at rest, in-memory/kernel only" is the secure-default, majority posture.** SPIRE (in-memory cache, 1.1), Istio (SDS in-memory, no Secret, 3.2), Linkerd (tmpfs, "never persist[s] to disk", 4.1), and Vault ("does not store generated private keys except for CA", 5.1) all keep leaf keys out of durable storage; SPIFFE frames this as minimizing "exposure from a key being leaked or compromised" (X.4). The systems that DO persist leaf keys (cert-manager Secret, kubelet `--cert-dir`) are the ones where the holder IS the identity owner (no broker) — and cert-manager still recommends regenerating the key each issuance. **Overdrive's worker-holds-material / kernel-consumes / no-leaf-escrow model matches the Cilium-agent / sidecar-broker majority.** (High confidence, 4+ sources.)

**(b) Restart = RE-ISSUE for the broker model; REUSE only when the actual material survives.** The systems Overdrive most resembles (SPIRE agent, Cilium agent, Istio/Linkerd sidecars) **re-issue / re-attest on restart** precisely *because* they hold no durable leaf material (Finding X.3). "Reuse-if-still-valid-don't-re-mint" (kubelet, cert-manager) is well-supported as a principle (X.5) — **but in every supporting case the decision is made by re-reading the actual persisted cert+key**, not a metadata-only record.

**The unresolved tension Overdrive must confront (single-source-gap; flagged adversarially):**
Overdrive proposes to combine **(b-reuse)** "read the `issued_certificates` audit row on restart → if still valid, don't re-mint" with **(a)** "no leaf keys at rest." **No surveyed system combines these two.** The combination is internally tense:

- If the leaf **private key is only in volatile memory/kernel** and is **lost on a worker/control-plane restart**, then the audit row (`spiffe_id, serial, not_before, not_after`) **cannot reconstruct the private key**. The peer-facing cert without its private key is useless for completing a TLS handshake. So a restart that loses the key **MUST re-mint regardless of the audit row** — the row can only justify skipping re-issuance if the *key material also survived*, which contradicts "no leaf keys at rest."
- The audit row can legitimately serve a **different** purpose on restart: as an idempotency / dedup record ("we already have a valid issuance for this spiffe_id; do not *over-issue* a second cert this tick") and as a **rotation scheduler input** ("compute next-rotation from `not_after`, don't re-mint early") — consistent with Overdrive's own "persist inputs, not derived state" rule (`.claude/rules/development.md`) and with cert-manager's renewal-time computation from cert duration.

**Recommendation (Medium-High confidence; the leaf-key posture is High, the audit-row-restart model is the gap):**
1. **Leaf keys at rest: NO.** Keep leaf private keys in memory/kernel only — strongly supported, secure-default, matches the broker-model majority. (High confidence.)
2. **Restart recovery: RE-ISSUE / RE-ATTEST**, mirroring SPIRE/Cilium/Istio/Linkerd — *if the leaf key did not survive the restart* (the kernel/worker memory was lost). This is the honest consequence of (1). Short leaf TTLs (SPIRE 1h, Linkerd 24h) make re-issue-on-restart cheap and the dropped-connection window bounded — and the rotation overlap window (SPIRE ≥12h grace) is explicitly designed to cover restart/outage gaps (X.1).
3. **Use the `issued_certificates` audit row as a rotation-scheduler input and over-issuance dedup, NOT as a "skip re-issue because a still-valid cert exists" signal** — because the row cannot vouch for a private key it does not store. "Respect-the-audit-row-until-near-expiry, don't re-mint" is sound **only** in the narrow sense of *not rotating early* (don't re-mint a cert whose `not_after` is still far off **and whose key is still live in memory**); it is NOT sound as a basis for skipping re-issuance after a restart that lost the key.
4. **If Overdrive genuinely wants reuse-across-restart** (kubelet/cert-manager style), it must persist the leaf key+cert (encrypted), which contradicts decision (1). The two are mutually exclusive — the surveyed evidence forces a choice, and the majority/secure-default choice is (1)+(2)+(3), not reuse-with-escrow.

**Dissent / where the recommendation rests on a gap:** the specific "decide-from-a-metadata-audit-row-with-no-bytes" model has **no precedent in the corpus** (X.5). The recommendation above resolves the gap by *narrowing* the audit row's role to scheduling/dedup, which is well-grounded; but if Overdrive's design intends the audit row to authorize *not re-issuing a workload's SVID after a key-losing restart*, that is unsupported and (per the analysis above) incorrect. This is the single most important point to validate against Overdrive's actual restart semantics (does kernel/worker memory survive the restart in question?).

---

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| SPIRE Concepts | spiffe.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| SPIRE Agent Config Reference | spiffe.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| SPIRE Server Config Reference | spiffe.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| Working with SVIDs | spiffe.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| SPIFFE Concepts | spiffe.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| SPIFFE Comparisons | spiffe.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| spire#1847 (soft-restart re-attest) | github.com | Medium-High (0.8) | open_source/issue | 2026-06-09 | Y |
| spire#4268 (svid renewal) | github.com | Medium-High (0.8) | open_source/issue | 2026-06-09 | Y |
| spire doc/plugin_server_keymanager_disk.md | github.com | High (1.0) | open_source/official | 2026-06-09 | Y |
| spire doc/plugin_server_upstreamauthority_disk.md | github.com | High (1.0) | open_source/official | 2026-06-09 | Y |
| Cilium Mutual Authentication | docs.cilium.io | High (1.0) | open_source/official | 2026-06-09 | Partial (see Gap 2) |
| Istio Security concepts | istio.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| Istio Provisioning Identity through SDS | istio.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| Linkerd Automatic mTLS | linkerd.io | High (1.0) | open_source/official | 2026-06-09 | Y (direct fetch failed; search-excerpt + cross-ref) |
| Linkerd Manually Rotating Control Plane TLS | linkerd.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| Linkerd Automatically Rotating Control Plane TLS | linkerd.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| Linkerd "deep dive into K8s mTLS" workshop | linkerd.io | High (1.0) | open_source/official-blog | 2026-06-09 | Y |
| Vault PKI secrets engine considerations | developer.hashicorp.com | High (0.9, official-vendor) | official-vendor-doc | 2026-06-09 | Y |
| Vault PKI secrets engine | developer.hashicorp.com | High (0.9, official-vendor) | official-vendor-doc | 2026-06-09 | Y |
| Vault PKI rotation primitives | developer.hashicorp.com | High (0.9, official-vendor) | official-vendor-doc | 2026-06-09 | Y |
| Vault HSM/KMS managed-key PKI | developer.hashicorp.com | High (0.9, official-vendor) | official-vendor-doc | 2026-06-09 | Y |
| cert-manager Certificate resource | cert-manager.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| cert-manager FAQ | cert-manager.io | High (1.0) | open_source/official | 2026-06-09 | Y |
| K8s Configure Certificate Rotation for the Kubelet | kubernetes.io | High (1.0) | official | 2026-06-09 | Y |
| K8s Certificate Management with kubeadm | kubernetes.io | High (1.0) | official | 2026-06-09 | Y |
| Talos How to manage PKI (v1.10) | talos.dev | High (1.0) | open_source/official | 2026-06-09 | Y |
| Sidero Labs CA Rotation (v1.10) | docs.siderolabs.com | High (1.0) | open_source/official | 2026-06-09 | Y |
| Talos Managing PKI (v1.4) | talos.dev | High (1.0) | open_source/official | 2026-06-09 | Y |
| arXiv 2504.14761 (Credential Broker Patterns for CI/CD) | arxiv.org | High (1.0) | academic (preprint) | 2026-06-09 | N (context only; not load-bearing) |

Reputation summary: High: 25 (~86%) | Medium-High: 3 (~10%) | Vendor-official (0.9): counted as High band. **Average reputation ≈ 0.96.** All sources from the trusted-domain list (no excluded domains). Citation coverage of major claims: every Finding carries ≥1 authoritative source; the large majority carry 2–3 cross-referenced sources (>95% target met). HashiCorp `developer.hashicorp.com` flagged per instructions as official-vendor (not in the base list) and cross-referenced.

## Knowledge Gaps

### Gap 1 — No precedent for "decide re-issuance from a metadata-only audit row (no cert bytes, no key at rest)"
**Issue**: Overdrive's proposed restart model (respect the `issued_certificates` audit row, don't re-mint, no leaf keys at rest) has no direct analog. Every "reuse-if-valid-don't-re-mint" system (kubelet, cert-manager) decides from the actual persisted cert+key; every "no-leaf-at-rest" system (SPIRE/Istio/Linkerd/Vault) re-issues on restart. **Attempted**: searched SPIRE/Cilium/Istio/Linkerd/Vault/cert-manager/kubelet/Talos docs + GitHub issues. **Recommendation**: validate against Overdrive's *actual* restart semantics — specifically whether the kernel-held leaf key survives the restart in question (kTLS material in kernel keyring across a worker-process restart vs a node reboot are different cases). If the key does not survive, the audit row cannot justify skipping re-issuance (see Decision 3). Consider a short Tier-3-style spike to confirm kernel/keyring survival behavior, per the project's "no-Tier-2-hook-firing-scope → Tier-3-spike" memory.

### Gap 2 — Cilium GA datapath: SPIRE-issued certs vs in-kernel kTLS termination
**Issue**: Cilium's GA mutual authentication does the *authentication* via SPIRE out-of-band and enforces in the eBPF datapath, but the surveyed docs do not confirm Cilium terminates TLS in-kernel via kTLS the way Overdrive proposes (sockops + kTLS). Where the leaf key sits *relative to the kernel* in Cilium is not stated in the official mutual-auth page. **Attempted**: Cilium stable mutual-auth docs. **Recommendation**: if the kernel-mediated kTLS analog matters for design, research Cilium's WireGuard/IPsec transparent-encryption path and any kTLS work separately — it is a distinct mechanism from the SPIRE-backed mutual-auth handshake covered here. This does not affect the agent-holds-material finding (2.1), which is well-supported.

### Gap 3 — istiod CA signing-key persistence specifics
**Issue**: The leaf path (SDS in-memory) is well-documented; istiod's *own* CA signing key persistence (self-signed root in a mounted Secret vs an external CA / cert-manager istio-csr) was not drilled to a primary quote. **Attempted**: Istio security concepts. **Recommendation**: low priority — the cross-comparator pattern (CA persists, leaf ephemeral) is established by 7 other sources; the istiod specifics would only refine, not change, Decision 2.

## Conflicting Information

### Conflict 1 — Restart recovery: RE-ISSUE vs REUSE
**Position A (re-issue on restart)**: SPIRE/Istio/Linkerd/Cilium — broker holds no durable leaf material, so restart triggers re-attest + re-issue. Source: [spire#1847](https://github.com/spiffe/spire/issues/1847), [Linkerd auto-mTLS](https://linkerd.io/2.19/features/automatic-mtls/) — reputation High/0.8.
**Position B (reuse from persisted material)**: kubelet/cert-manager — read still-valid cert+key from disk/Secret, reuse until renewal threshold. Source: [k8s kubeadm certs](https://kubernetes.io/docs/tasks/administer-cluster/kubeadm/kubeadm-certs/), [cert-manager Certificate](https://cert-manager.io/docs/usage/certificate/) — reputation High/1.0.
**Assessment**: Not a true contradiction — it is conditioned on **who holds the material and whether it persists**. Broker-model systems (Overdrive's class) re-issue because nothing survives; identity-owner systems with on-disk material reuse. Both are correct within their architecture. Overdrive's broker + no-leaf-at-rest design lands it in Position A. The apparent appeal of Position B for Overdrive (skip re-issue via audit row) is unsupported because Overdrive lacks the persisted *key* that Position B systems rely on (Gap 1).

### Conflict 2 — Is persisting leaf private keys at rest acceptable or an anti-pattern?
**Position A (anti-pattern / avoid)**: SPIFFE ("minimize exposure from a key being leaked"), Vault ("does not store generated private keys"), Linkerd ("never persist to disk"). Reputation High.
**Position B (acceptable with controls)**: cert-manager and kubelet persist leaf keys (Secret / `--cert-dir`); cert-manager nonetheless recommends `rotationPolicy: Always` to regenerate the key each issuance. Reputation High.
**Assessment**: The secure-default consensus is no-escrow/short-lived (Position A), but Position B shows on-disk leaf escrow is not categorically wrong when access-controlled and the holder owns the identity. For Overdrive's broker + kernel-mediated model, Position A is the better fit and is what Overdrive already proposes.

## Full Citations
[1] SPIFFE. "SPIRE Concepts." spiffe.io. https://spiffe.io/docs/latest/spire-about/spire-concepts/. Accessed 2026-06-09.
[2] SPIFFE. "SPIRE Agent Configuration Reference." spiffe.io. https://spiffe.io/docs/latest/deploying/spire_agent/. Accessed 2026-06-09.
[3] SPIFFE. "SPIRE Server Configuration Reference." spiffe.io. https://spiffe.io/docs/latest/deploying/spire_server/. Accessed 2026-06-09.
[4] SPIFFE. "Working with SVIDs." spiffe.io. https://spiffe.io/docs/latest/deploying/svids/. Accessed 2026-06-09.
[5] SPIFFE. "SPIFFE Concepts." spiffe.io. https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/. Accessed 2026-06-09.
[6] SPIFFE. "How does SPIRE compare to other tools?" spiffe.io. https://spiffe.io/docs/latest/spire-about/comparisons/. Accessed 2026-06-09.
[7] spiffe/spire. "Agent soft-restart for re-attestation (#1847)." github.com. https://github.com/spiffe/spire/issues/1847. Accessed 2026-06-09.
[8] spiffe/spire. "Avoid spiky svid renewal requests to SPIRE server (#4268)." github.com. https://github.com/spiffe/spire/issues/4268. Accessed 2026-06-09.
[9] spiffe/spire. "doc/plugin_server_keymanager_disk.md." github.com. https://github.com/spiffe/spire/blob/main/doc/plugin_server_keymanager_disk.md. Accessed 2026-06-09.
[10] spiffe/spire. "doc/plugin_server_upstreamauthority_disk.md." github.com. https://github.com/spiffe/spire/blob/main/doc/plugin_server_upstreamauthority_disk.md. Accessed 2026-06-09.
[11] Cilium. "Mutual Authentication." docs.cilium.io. https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/. Accessed 2026-06-09.
[12] Istio. "Security." istio.io. https://istio.io/latest/docs/concepts/security/. Accessed 2026-06-09.
[13] Istio. "Provisioning Identity through SDS." istio.io. https://istio.io/v1.1/docs/tasks/security/auth-sds/. Accessed 2026-06-09.
[14] Linkerd. "Automatic mTLS." linkerd.io. https://linkerd.io/2.19/features/automatic-mtls/. Accessed 2026-06-09.
[15] Linkerd. "Manually Rotating Control Plane TLS Credentials." linkerd.io. https://linkerd.io/2-edge/tasks/manually-rotating-control-plane-tls-credentials/. Accessed 2026-06-09.
[16] Linkerd. "Automatically Rotating Control Plane TLS Credentials." linkerd.io. https://linkerd.io/2-edge/tasks/automatically-rotating-control-plane-tls-credentials/. Accessed 2026-06-09.
[17] Linkerd. "Workshop recap: A deep dive into Kubernetes mTLS with Linkerd." linkerd.io. https://linkerd.io/2023/01/30/mtls-and-linkerd/. Accessed 2026-06-09.
[18] HashiCorp. "PKI secrets engine considerations." developer.hashicorp.com. https://developer.hashicorp.com/vault/docs/secrets/pki/considerations. Accessed 2026-06-09.
[19] HashiCorp. "PKI secrets engine." developer.hashicorp.com. https://developer.hashicorp.com/vault/docs/secrets/pki. Accessed 2026-06-09.
[20] HashiCorp. "PKI secrets engine - rotation primitives." developer.hashicorp.com. https://developer.hashicorp.com/vault/docs/secrets/pki/rotation-primitives. Accessed 2026-06-09.
[21] HashiCorp. "Generate certificates with HSM or KMS managed keys." developer.hashicorp.com. https://developer.hashicorp.com/vault/tutorials/pki/managed-key-pki. Accessed 2026-06-09.
[22] cert-manager. "Certificate resource." cert-manager.io. https://cert-manager.io/docs/usage/certificate/. Accessed 2026-06-09.
[23] cert-manager. "FAQ." cert-manager.io. https://cert-manager.io/docs/faq/. Accessed 2026-06-09.
[24] Kubernetes. "Configure Certificate Rotation for the Kubelet." kubernetes.io. https://kubernetes.io/docs/tasks/tls/certificate-rotation/. Accessed 2026-06-09.
[25] Kubernetes. "Certificate Management with kubeadm." kubernetes.io. https://kubernetes.io/docs/tasks/administer-cluster/kubeadm/kubeadm-certs/. Accessed 2026-06-09.
[26] Talos Linux. "How to manage PKI and certificate lifetimes with Talos Linux (v1.10)." talos.dev. https://www.talos.dev/v1.10/talos-guides/howto/cert-management/. Accessed 2026-06-09.
[27] Sidero Labs. "CA Rotation (v1.10)." docs.siderolabs.com. https://docs.siderolabs.com/talos/v1.10/security/ca-rotation. Accessed 2026-06-09.
[28] Talos Linux. "Managing Talos PKI (v1.4)." talos.dev. https://www.talos.dev/v1.4/talos-guides/configuration/managing-pki/. Accessed 2026-06-09.
[29] Patwardhan, A. et al. "Decoupling Identity from Access: Credential Broker Patterns for Secure CI/CD." arXiv:2504.14761. https://arxiv.org/pdf/2504.14761. Accessed 2026-06-09. (context only)

## Research Metadata
Duration: ~1 session | Sources examined: ~30 | Sources cited: 29 | Cross-references: every load-bearing Finding has ≥2 sources except where noted (Cilium 2.2, Linkerd 4.1 direct-fetch fallback, SPIRE 1.5 issue) | Confidence distribution: High ~80% (Findings 1.1–1.4, 3.1–3.2, 4.1–4.3, 5.1–5.2, 6.1–6.2, 7.1–7.2, 8.1–8.3, X.1–X.4, Decisions 1 & 2), Medium-High ~15% (1.5, 5.3, X.5, Decision 3 audit-row aspect), Medium ~5% (2.2) | Tool failures: Linkerd auto-mTLS direct WebFetch failed twice (recovered via search-engine excerpt + two independent Linkerd cross-references); one Talos URL 301-redirected to docs.siderolabs.com (followed; trusted domain) | Output: docs/research/security/workload-svid-rotation-lifecycle-comprehensive-research.md
