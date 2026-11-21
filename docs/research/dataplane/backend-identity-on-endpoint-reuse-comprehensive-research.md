# Research: Backend identity expectation on endpoint (IP:port) reuse

**Date**: 2026-11-21 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High (granularity and source-of-expectation: High; Envoy/Cilium reuse internals: Medium) | **Sources**: 22

## Executive Summary

Ground rule (adopted from the requester, and consistent with the sources): a SPIFFE ID names a workload, not an address. The same ID legitimately appears on many IPs, and an IP is routinely reused by different workloads. Identity checking is therefore never keyed on the IP. The question is what a client expects of a backend, and where that expectation lives.

Every system surveyed (Istio, Linkerd, Consul, Cilium mutual auth) expects an identity at **workload / service-account / label-set granularity**, never at pod-instance granularity. The expected identity is derived from **service config or the control plane's current view of the service's endpoints** and is delivered or recomputed with each endpoint update. None binds an immutable per-instance identity to an endpoint slot and refuses a replacement. Where a per-endpoint identity travels with the endpoint (Linkerd `WeightedAddr.tls_identity`), it is the pod's ServiceAccount identity, shared by every replica, and endpoint removal is keyed by IP:port, so a replacement at the same IP:port is remove-then-add and the identity follows the current endpoint. Envoy and Cilium both key backend/host state by address, and treat identity/metadata as mutable attributes of that address slot (Cilium: backend ID is keyed by L3n4Addr, with security identity resolved separately by labels).

Overdrive is the outlier: `SpiffeId::for_allocation` makes the SPIFFE ID per allocation (`.../workload/<w>/alloc/<a>`), the gateway checks exact equality with the identity recorded for the receipt's `BackendId`, and ADR-0241 makes that `BackendId -> Backend.alloc` association immutable. A replacement allocation at the reused workload address then verifies against the wrong, dead instance. The evidence supports: **the expected identity at the gateway should be the workload's, not the allocation's.** The per-allocation SPIFFE ID is the root of the problem, because it is what forces the gateway to carry an instance-grain expectation bound to a slot. The "fresh BackendId per replacement" fix (the proposed ADR-0241 amendment) is internally consistent and repairs the symptom, but it keeps the one design element no surveyed system has (an expectation pinned to an instance and a slot) and adds an id per replacement. See "Recommendation".

## Research Methodology
**Search Strategy**: Read Overdrive code/ADRs/RCA locally; fetched primary docs and source (Istio, Linkerd proxy-api and destination controller, Envoy docs and issue tracker, Consul docs, Cilium docs and source at a pinned commit, SPIFFE standard).
**Source Selection**: only trusted domains (github.com, istio.io, linkerd.io, envoyproxy.io, developer.hashicorp.com, docs.cilium.io, spiffe.io). Source code at a pinned commit was used where the fetch tool could reach it; several large files were truncated by the fetch tool (see Knowledge Gaps).
**Quality Standards**: 2+ sources per major claim where possible; interpretations are labelled "Interpretation".

## Overdrive baseline (local sources)

**O-1. Overdrive's SPIFFE ID is per allocation.** `SpiffeId::for_allocation` derives `spiffe://overdrive.local/workload/<workload>/alloc/<alloc>` (`crates/overdrive-core/src/id.rs:310-344`), "the single derivation the reconcilers route through" (ADR-0067 D5; `docs/product/architecture/adr-0067-workload-identity-manager-svid-lifecycle.md`, section D5). 33 non-test call sites across 22 files use it (grep `for_allocation(`), including `svid_lifecycle.rs`, `workload_lifecycle.rs`, `service_lifecycle.rs`, `mtls_resolve_adapter.rs`, `mtls_intercept_worker.rs`. Confidence: High (code).

**O-2. The gateway checks exact equality with a slot-bound identity.** `check_selected_peer` requires a sole SPIFFE URI SAN and `actual == expected` (`crates/overdrive-dataplane/src/mtls/gateway_client.rs:54-66`). `expected` is the `SpiffeId` "the dataplane resolved from the receipt's `BackendId`" (same file, module doc lines 1-10). ADR-0241 makes the Dataplane own the `BackendId -> exact applied Backend.alloc` association and rejects (committed text) associating a live id with another allocation. Confidence: High.

**O-3. The memo conflates endpoint with identity.** Per the RCA (`docs/analysis/root-cause-analysis-public-ingress-gateway-502-after-serve-restart.md` section 3 WHY 3, `plan_service_apply`, `crates/overdrive-dataplane/src/lib.rs:1359-1410`), the allocator memo maps endpoint -> `BackendId`, and a different alloc at a memoised endpoint returns `BackendIdentityConflict`. The platform reuses a freed workload address for the replacement allocation. Bisect: first bad commit `4ebc41233` (boot resync programs the stale row), exposing a latent rule.

**O-4. Status discrepancy to resolve.** The working-tree ADR-0241 "Amendment 2026-11-21" says "approved by the user 2026-11-21". The requester states the user rejected it. This document treats the amendment as a proposal, per the requester.

**O-5. Workloads hold no SVID.** The kernel does mTLS; `IdentityMgr` holds the material (CLAUDE.md "Workload identity model"). So SVID material is already managed per allocation by the platform, independent of what SAN the certificate carries.

## Findings

### F-1 SPIFFE: an ID names a workload; instances share it
**Evidence**: "a string that uniquely and specifically identifies a workload"; a workload "may comprise multiple running instances of software, all of which perform the same task" (SPIFFE concepts). The standard: "The path component of a SPIFFE ID allows for the unique identification of a given workload"; path meaning "is left open-ended and is the responsibility of the administrator to define".
**Source**: [SPIFFE concepts](https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/), [SPIFFE-ID.md](https://github.com/spiffe/spiffe/blob/main/standards/SPIFFE-ID.md) - accessed 2026-11-21.
**Confidence**: High (two primary sources).
**Gap in the guidance**: the sources do not explicitly forbid instance-specific data in the path. The standard says the properties bound to the workload lifetime (such as the ID) are stable, and "if the volatility of the assertion in question is not clear, operators should err on the side of caution and exclude it from the SVID". Interpretation: a per-instance ID is permitted by the open-ended path rule but contradicts the defined unit (workload with many instances) and the stability expectation.

### F-2 Istio: identity = service account; expectation derived per service, not per endpoint
**Evidence**: "The Istio identity model uses the first-class `service identity` to determine the identity of a request's origin"; on Kubernetes it is the service account. Secure naming: "A mapping of identity A to service name B means 'A is authorized to run service B'" and the control plane distributes the mappings to proxies. The ServiceEntry reference says istiod verifies "the SANs of the service accounts associated with the pods of the service" plus listed SANs. `Service.ServiceAccounts` is "the service accounts that run the service"; `IstioEndpoint.ServiceAccount` exists per endpoint. The client validation context is built as exact-match `MatchSubjectAltNames` from a SAN list (`pilot/pkg/security/model/authentication.go`).
**Source**: [Istio security concepts](https://istio.io/latest/docs/concepts/security/), [ServiceEntry reference](https://istio.io/latest/docs/reference/config/networking/service-entry/), [pilot model/service.go](https://github.com/istio/istio/blob/master/pilot/pkg/model/service.go), [authentication.go](https://github.com/istio/istio/blob/master/pilot/pkg/security/model/authentication.go).
**Confidence**: High on granularity (service account); Medium on the exact wiring (the cluster-builder function that assembles the SAN list was not retrieved; the SAN list is service-scoped per the ServiceEntry doc). **IP reuse**: not documented as a special case; since the expectation is the service's account set, a replacement pod with the same account verifies. Interpretation, no direct source on IP reuse.

### F-3 Linkerd: identity delivered per endpoint, at ServiceAccount granularity, keyed removal by IP:port
**Evidence**: the proxy "verifies that the destination proxy's certificate is signed by the trust anchor and contains the expected identity"; the control plane returns "the destination's endpoint addresses, along with metadata including an identity name"; each certificate is "bound to the Kubernetes ServiceAccount identity of the containing pod" ([automatic mTLS](https://linkerd.io/2/features/automatic-mtls/)). Proto: `WeightedAddr` carries `addr` and `tls_identity`; `DnsLikeIdentity.name` is "a DNS-like name that encodes workload coordinates", format `{name}.{namespace}.{type}.identity.{control-namespace}.{trust-domain}`; `Update` has `add` (new endpoints) and `remove` ([destination.proto](https://github.com/linkerd/linkerd2-proxy-api/blob/main/proto/destination.proto)). Controller: `fmt.Sprintf("%s.%s.serviceaccount.identity.%s.%s", sa, ns, ...)` from `GetServiceAccountAndNS(address.Pod)`; `sendClientRemove` builds addresses "using only IP and port" ([endpoint_translator.go](https://github.com/linkerd/linkerd2/blob/main/controller/api/destination/endpoint_translator.go)).
**Confidence**: High. This is the closest analogue to "expected identity delivered per endpoint update", and it is the workload (ServiceAccount) identity, not the pod's.
**Interpretation**: because removal is IP:port-keyed and the identity is a pure function of the pod's service account, a replacement at the same IP:port is `remove` then `add` carrying the replacement's (usually identical) identity. The translator file did not show the watcher's replace logic (gap G-3).

### F-4 Envoy: hosts keyed by address; identity/SAN is config or metadata, not a slot invariant
**Evidence**: endpoints are "extra attributes carried in the discovery API response for each host"; routing depends on discovery plus health, and Envoy "is the only state in which Envoy will purge host data" is absent-and-unhealthy ([service discovery](https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/service_discovery)). `transport_socket_matches` selects "different transport sockets for different endpoints" via endpoint metadata, then locality metadata, then the cluster default; `update_merge_window` merges "health check/weight/metadata updates"; membership add/remove is not merged ([cluster.proto](https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/cluster/v3/cluster.proto)). Issue #22417: EDS diffs "by address comparison"; with duplicate addresses "only the first one will be retained" (eds.cc comment) ([issue](https://github.com/envoyproxy/envoy/issues/22417)).
**Confidence**: Medium. Documentation establishes that metadata is a mutable per-endpoint attribute and that membership is diffed by address. I could not retrieve `updateDynamicHostList` (the fetch tool truncated `upstream_impl.cc`), so "a metadata change on an existing address is applied in place, not refused" is Interpretation. Nothing in the documentation describes a refusal of an endpoint because its metadata or SAN differs from the previous occupant.

### F-5 Consul: per-service identity; the client does not pin an instance
**Evidence**: certificates "comply with the SPIFFE X.509 standard"; the leaf certificate is requested per service (`/v1/agent/connect/ca/leaf/<service-name>`); the destination is verified against the CA roots; "Neither Envoy nor the built-in proxy currently validate the SPIFFE URI of the chain beyond the leaf certificate" ([connect internals](https://developer.hashicorp.com/consul/docs/connect/connect-internals), [proxy integration](https://developer.hashicorp.com/consul/docs/connect/proxies/integrate)). Leaf SPIFFE shape seen in HashiCorp's Vault CA tutorial: `spiffe://<trust-domain>/ns/default/dc/dc1/svc/web` ([tutorial](https://developer.hashicorp.com/consul/tutorials/operate-consul/vault-pki-consul-connect-ca), via search result).
**Confidence**: Medium. Per-service granularity is clear (the leaf is issued per service name, with no instance id). Whether the client proxy's expected SAN for an upstream is the service SPIFFE ID was not stated on the fetched pages (G-2).

### F-6 Cilium: backend ID is keyed by address; security identity is label-derived and separate
**Evidence (backend IDs)**: in `pkg/service/service.go` (commit `3d4fbe05`), backends are held in `backendByHash` keyed by `b.L3n4Addr.Hash()`; `RestoreBackendID(b.L3n4Addr, b.ID)` re-registers the id for an address; `deleteOrphanBackends` calls `DeleteBackendID` when `backendRefCount[hash] == 0`; BPF entries are removed with `DeleteBackendByID` ([service.go @ 3d4fbe05](https://github.com/cilium/cilium/blob/3d4fbe05e3e8328c2dee195a13ad154514d7c42b/pkg/service/service.go)). A 2025 PR describes the loadbalancer BPF reconciler's allocator as mapping L3n4Addr to ServiceID and BackendID ([PR 47746](https://github.com/cilium/cilium/pull/47746)). Backend ID acquisition body was not retrieved (G-4).
**Evidence (identity)**: "security is based on the identity of a pod, which is derived through labels", and "This identity can be shared between pods" ([identity docs](https://docs.cilium.io/en/stable/security/network/identity/)). Mutual auth: "a workload means the same thing that a Cilium Security Identity does - a set of pods described by a label set"; the SPIFFE ID is encoded from it ([mutual authentication](https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/)).
**Stale backends**: with socket-LB, pods "can continue to send traffic to a service backend even after it's deleted", and Cilium terminates sockets connected to deleted backends ([kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/)). Conntrack handling of backend IDs is not described there (G-4).
**Confidence**: Medium-High. Key point: **an address-keyed backend ID is the production norm (Cilium), and it carries no identity**. Cilium's id->address memo is not the defect; binding identity to the id is.

## Synthesis

| Question | Finding | Sources |
|---|---|---|
| Granularity expected | Workload / service account / label set, never per pod instance | F-1, F-2, F-3, F-5, F-6 |
| Source of expectation | Istio: service config plus control-plane service-account set (per service). Linkerd: delivered per endpoint, value = ServiceAccount identity. Cilium: security identity from labels. Consul: per-service leaf | F-2, F-3, F-5, F-6 |
| Endpoint/IP reuse | Endpoint state keyed by address (Envoy diff by address, Linkerd removal by IP:port, Cilium backend hash by address); identity is an attribute that follows the current endpoint or is workload-level, so reuse needs no special case | F-3, F-4, F-6 |
| In-flight connections | Established TLS sessions are unaffected by endpoint-table changes; Cilium terminates sockets to deleted backends; Envoy drains pools on host removal under configurable options (`close_connections_on_host_set_change`) | F-4, F-6 |
| Any system pinning a per-instance identity to a slot and refusing a replacement? | **None found.** Linkerd is the only one with per-endpoint identity delivery, and it is service-account grain | F-2 to F-6 (absence of evidence; see G-5) |

**Reuse across different workloads** (an IP reused by a different workload, not a replica): in the workload-grain systems the expected identity comes from the service's current endpoint set, so when the address is rebound to a different service's pod, the endpoint is removed from the old service's set and the old service's SAN set does not match, and the connection fails the check. That is the designed protection: the check is "is this peer authorised to run service B", not "is this the same pod as last time". Interpretation, consistent with Istio's secure-naming definition.

## Recommendation for Overdrive

Evidence ranking of the three options, judged against F-1 to F-6:

**Option A (workload-grain expectation; supported by the evidence).** The gateway's expected identity is the workload's: either (A1) a per-workload SPIFFE ID, `spiffe://overdrive.local/workload/<w>`, shared by all allocations; or (A2, smaller) keep per-allocation SVID SANs, and have the gateway/dataplane compare the workload portion of the expected and presented IDs. A replacement at the same address then verifies, with no change to the BackendId memo. This matches Istio (service-account set per service), Linkerd (ServiceAccount identity), Consul (per-service leaf), Cilium (label-set identity), and SPIFFE's definition (F-1).
- A1 is the cleaner fit. Cost: `SpiffeId::for_allocation` and its 33 non-test call sites, ADR-0067 D5, and anything relying on alloc-grain SANs (`mtls_resolve_adapter`, `mtls_intercept_worker`, east-west intended-peer pinning and any audit that reads the SAN) must be reviewed; `issued_certificates` already persists `serial`, so instance-level audit survives without the SAN. Per-allocation certificate issuance and the per-allocation `IdentityMgr` map are unchanged.
- A2 changes only ADR-0234/ADR-0241 and `check_selected_peer`. Cost: the verifier parses SPIFFE path structure, which SPIFFE leaves to the administrator (F-1); acceptable only because Overdrive is the sole issuer, but it is a convention coupling, and the per-allocation ID remains the root cause for any other consumer.
- ADR-0241 under A: the `BackendId -> Backend.alloc` association stays useful for receipts, but the identity returned for a receipt is the workload's, so the immutable-per-id rule no longer constrains replacement. The rule "a BackendId is never rebound to another Backend.alloc" can stay, but nothing in the gateway correctness depends on it, and the memo can reuse the id for the replacement (as Cilium does), so `BackendIdentityConflict` would have no producer.

**Option B (expected identity delivered per endpoint update; slot's identity follows the current programmed allocation).** This is Linkerd's shape, and the closest to what is deployed, but only because Linkerd's delivered value is the service-account identity (F-3). With a per-allocation SPIFFE ID, B means the slot's expected identity changes when `update_service` programs `alloc-api-1` at the same endpoint. That requires the identity association to be mutable per slot, which contradicts ADR-0241's receipt guarantee (an in-flight receipt would resolve to the new allocation). Mutability is acceptable (the exact-peer check already fails closed on a stale receipt), but it removes the per-id immutability that ADR-0241 exists to provide. B does not address the root cause either.

**Option C (fresh BackendId per instance; the proposed amendment).** It preserves ADR-0241 and fixes the RCA's failure, and the RCA's design is internally sound for Overdrive's model. But against the evidence it is the only option that keeps a per-instance expectation bound to a slot and generates a new id per replacement (bounded by restart budget, per the amendment text). No surveyed system does this; Cilium's id is address-keyed and carries no identity. Treat C as a valid local repair if per-allocation SPIFFE IDs are a deliberate product requirement, not as the evidence-backed model.

**Which does the evidence support?** Option A. Rationale: all five identity systems expect workload-grain identity (F-1 to F-6); a replacement of the same workload at the same address is the normal case and must verify; endpoint state is address-keyed (F-3, F-4, F-6) so an address-keyed memo (Overdrive's existing allocator) is conventional. The distinctive, unsupported element is the per-allocation SPIFFE ID plus the slot-bound immutable expectation.

**Does the per-allocation SPIFFE ID need to change? Yes, if the goal is to follow the evidence.** It is the root of the problem in the sense that it is the only reason the gateway needs an instance-grain expectation. Caveat (not evidenced by this research): Overdrive may have reasons for alloc-grain identity (for example per-instance revocation or audit); this research found none documented in ADR-0067 D5 (a consolidation of existing helpers, not a security rationale) but did not review every ADR (G-6). Instance-level revocation can be served by certificate serial without encoding the allocation in the SAN.

Required follow-ups regardless of option, from the RCA and design rules: a composed Tier-1 sim invariant for replace-at-same-endpoint, host/Sim equivalence, and an operator-visible state for a permanently refused hydration. Any change to identity naming or ADR-0241 is a material design decision that goes to the user (`.claude/rules/design.md`).

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| SPIFFE concepts | spiffe.io | High | official | 2026-11-21 | Y (F-1) |
| SPIFFE-ID.md | github.com/spiffe | Medium-High | spec source | 2026-11-21 | Y |
| Istio security concepts | istio.io | High | official | 2026-11-21 | Y |
| Istio ServiceEntry reference | istio.io | High | official | 2026-11-21 | Y |
| istio model/service.go, authentication.go | github.com | Medium-High | source | 2026-11-21 | Y |
| Linkerd automatic mTLS | linkerd.io | High | official | 2026-11-21 | Y |
| linkerd2-proxy-api destination.proto | github.com | Medium-High | source | 2026-11-21 | Y |
| linkerd2 endpoint_translator.go | github.com | Medium-High | source | 2026-11-21 | Y |
| Envoy service discovery, cluster.proto | envoyproxy.io | High | official | 2026-11-21 | Y |
| Envoy issue 22417 | github.com | Medium-High | issue | 2026-11-21 | N |
| Consul connect internals, proxy integration | developer.hashicorp.com | High | official | 2026-11-21 | Y |
| Cilium identity, mutual auth, kube-proxy-free | docs.cilium.io | High | official | 2026-11-21 | Y |
| cilium pkg/service/service.go @ 3d4fbe05 | github.com | Medium-High | source | 2026-11-21 | Partial |
| cilium PR 47746 | github.com | Medium-High | PR | 2026-11-21 | N |

Local (not counted): id.rs, gateway_client.rs, ADR-0067, ADR-0241, the RCA.
Reputation: High 8 (57%), Medium-high 6 (43%).

## Knowledge Gaps
- **G-1 Envoy in-place update behaviour.** Searched: upstream_impl.cc (fetch truncated at ~60k chars), envoy docs, issues. Not found: the code showing what happens to a host at the same address when metadata or transport-socket match changes (in place vs replaced) and what happens to its connection pool. Recommend reading `updateDynamicHostList` at a pinned tag in a local clone.
- **G-2 Consul expected-SAN for an upstream.** Fetched pages state the leaf is per service and that SPIFFE URI chain validation is limited, but not the SAN the client expects. Recommend reading the Consul Envoy xDS code (`agent/xds`) for the upstream validation context.
- **G-3 Linkerd watcher replace semantics** (what the endpoints watcher sends when a pod IP is reused). Not in the translator file.
- **G-4 Cilium** backend-ID acquisition body and conntrack behaviour for deleted or reused backends (the pinned excerpt cut off before `AcquireBackendID`). Recommend reading `pkg/loadbalancer` and BPF `lb.h` backend slave handling in a local clone.
- **G-5 Negative claim.** "No production system binds a per-instance expected identity to a slot" is an absence-of-evidence finding over five systems, not a proof. Kuma, Traefik Mesh, SPIRE-direct users and AWS App Mesh were not examined.
- **G-6 Overdrive rationale for alloc-grain SPIFFE IDs** beyond ADR-0067 D5 and ADR-0234 was not fully reviewed (east-west ADRs, ADR-0072, revocation plans).

## Conflicting Information
No contradictions between external sources. One internal conflict: ADR-0241's uncommitted amendment says "approved by the user", while the requester says it was rejected (O-4); the requester's statement was applied.

## Recommendations for Further Research
1. Close G-1, G-2, G-4 by reading pinned-tag source in local clones of envoy, consul and cilium.
2. Enumerate every consumer of the SAN's `/alloc/<a>` component in Overdrive (33 call sites) to size Option A1.
3. If Option A is adopted, a spike under the DESIGN rules on a Tier-3 kernel for the `selected_backend_identity` read path is not needed (no kernel behaviour changes), but Quint model QC-3 must be revised.

## Full Citations
[1] SPIFFE. "SPIFFE Concepts". https://spiffe.io/docs/latest/spiffe-about/spiffe-concepts/. Accessed 2026-11-21.
[2] SPIFFE. "SPIFFE-ID.md". https://github.com/spiffe/spiffe/blob/main/standards/SPIFFE-ID.md. Accessed 2026-11-21.
[3] Istio. "Security concepts". https://istio.io/latest/docs/concepts/security/. Accessed 2026-11-21.
[4] Istio. "Service Entry". https://istio.io/latest/docs/reference/config/networking/service-entry/. Accessed 2026-11-21.
[5] istio/istio. pilot/pkg/model/service.go; pilot/pkg/security/model/authentication.go (master). https://github.com/istio/istio/tree/master/pilot/pkg. Accessed 2026-11-21.
[6] Linkerd. "Automatic mTLS". https://linkerd.io/2/features/automatic-mtls/. Accessed 2026-11-21.
[7] linkerd2-proxy-api. destination.proto. https://github.com/linkerd/linkerd2-proxy-api/blob/main/proto/destination.proto. Accessed 2026-11-21.
[8] linkerd2. controller/api/destination/endpoint_translator.go. https://github.com/linkerd/linkerd2/blob/main/controller/api/destination/endpoint_translator.go. Accessed 2026-11-21.
[9] Envoy. "Service discovery". https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/service_discovery. Accessed 2026-11-21.
[10] Envoy. "Cluster proto". https://www.envoyproxy.io/docs/envoy/latest/api-v3/config/cluster/v3/cluster.proto. Accessed 2026-11-21.
[11] envoyproxy/envoy. Issue 22417. https://github.com/envoyproxy/envoy/issues/22417. Accessed 2026-11-21.
[12] HashiCorp. "Consul service mesh connect internals"; "Custom proxy integration"; Vault CA tutorial. https://developer.hashicorp.com/consul/docs/connect/connect-internals ; https://developer.hashicorp.com/consul/docs/connect/proxies/integrate ; https://developer.hashicorp.com/consul/tutorials/operate-consul/vault-pki-consul-connect-ca. Accessed 2026-11-21.
[13] Cilium. "Identity"; "Mutual authentication"; "Kubernetes without kube-proxy". https://docs.cilium.io/en/stable/security/network/identity/ ; https://docs.cilium.io/en/stable/network/servicemesh/mutual-authentication/mutual-authentication/ ; https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-11-21.
[14] cilium/cilium. pkg/service/service.go @ 3d4fbe05e3e8328c2dee195a13ad154514d7c42b; PR 47746. https://github.com/cilium/cilium/blob/3d4fbe05e3e8328c2dee195a13ad154514d7c42b/pkg/service/service.go ; https://github.com/cilium/cilium/pull/47746. Accessed 2026-11-21.

## Research Metadata
Duration: ~25 tool turns | Examined: ~30 | Cited: 22 incl. local | Output: docs/research/dataplane/backend-identity-on-endpoint-reuse-comprehensive-research.md
Tool notes: WebFetch truncated large GitHub files; the fetch summariser sometimes declined to quote; Consul and Envoy per-endpoint verification details remain gaps (G-1, G-2).
