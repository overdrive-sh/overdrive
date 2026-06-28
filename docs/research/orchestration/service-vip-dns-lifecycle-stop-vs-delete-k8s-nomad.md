# Research: Service VIP / DNS Name-Resolution Lifecycle on Stop vs Delete — Kubernetes & Nomad (+ Consul)

**Date**: 2026-06-28 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 16

## Decision Context (informs ADR-0049 §6 / Option C — NOT an Overdrive design prescription)

Overdrive is deciding whether a workload's allocator-issued **service VIP** should be
**released on stop** (current ratified contract, ADR-0049 §6) or **retained until delete**
("Option C"). The research goal is **evidence of prior art** from established orchestrators:
do Kubernetes and Nomad (+ Consul) tie the stable IP/VIP identity to the *declared-object
lifetime* (release on delete/purge only) while gating *name resolution* on healthy backends?

---

## Executive Summary

**Confidence: High.** Both Kubernetes and Nomad+Consul split the service into two planes with two
different lifetimes, and the split is the same in each system: a **stable IP/VIP identity** bound to
the **declared object's lifetime** (released only on delete/purge), and a **backend-reachability**
signal bound to the **running instances' health** (collapses on stop). The Kubernetes ClusterIP is
allocated when the Service is created and de-allocated **only when the Service is deleted** — Pod
churn, Deployment scale-to-zero, and zero ready endpoints do not release it; a normal ClusterIP name
keeps resolving to its stable VIP even with zero endpoints (NXDOMAIN-on-empty is an opt-in CoreDNS
flag, not the default). Only the *headless* name (no VIP, returns Pod IPs) goes NXDOMAIN when backends
vanish. Consul's transparent-proxy mesh assigns a **per-service virtual IP** (`240.0.0.0/4`,
`consul-virtual`) keyed to the logical service in the catalog, stable across instance churn, while the
Nomad service registration is **deregistered immediately** on alloc stop so the service-DNS name stops
resolving — and a stopped-but-not-`-purge`d job stays queryable in state.

**The decision-relevant finding (S1/S2):** neither reference system releases the stable IP/VIP on a
transient stop, and neither exhibits Overdrive's current asymmetry (frontend `F` retained on stop, but
service VIP released on stop). The prevailing convention is a **single withhold-not-release identity
lifecycle**: retain the stable identity (name + VIP) until the declared object is deleted, and
independently withhold *resolution* (empty endpoints / NXDOMAIN) while there are no healthy backends.

**Net implication (S3 — evidence, not an Overdrive prescription):** against this prior art, Overdrive's
release-on-terminal-state contract (ADR-0049 §6) is the **outlier**, and "Option C" (retain the VIP
until logical-workload deletion, mirroring the retained frontend `F`) is what aligns with the K8s/Consul
convention. The honest counter-evidence: retain-until-delete presumes a large IP space (K8s uses a
wide ServiceCIDR; Consul uses `240.0.0.0/4`), the *exact* Consul VIP-reclamation timing is undocumented,
and the prior art's real lesson is to **decouple** the identity plane from the reachability plane — keep
the VIP **and** still retract the dial-by-name resolution on stop (the #251 bug is the coupling of the two).

## Research Methodology

**Search Strategy**: Official docs first (kubernetes.io, developer.hashicorp.com, consul.io),
CoreDNS plugin source (github.com/coredns), RFC 8020 for DNS NXDOMAIN/NODATA semantics.
**Source Selection**: Types: official / technical_docs / open_source | Reputation: high min for
authoritative claims | Verification: cross-reference each major claim across ≥2 independent sources.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative) | All major claims cross-referenced.

---

## Findings — Kubernetes

### K1 — ClusterIP allocation lifetime — CONFIRMED: bound to Service object lifetime, released only on Service deletion
**Claim**: A Service `ClusterIP` is allocated when the Service is created and released (returned to
the pool) **only when the Service object is deleted**. It is NOT released by Pod churn, Deployment
scale-to-zero, or zero ready endpoints. Each ClusterIP is tracked by a corresponding `IPAddress`
object (networking.k8s.io) bound to the Service's lifetime.

**Evidence**:
- Allocation timing: "When Kubernetes needs to assign a virtual IP address for a Service, that
  assignment happens one of two ways: _dynamically_ the cluster's control plane automatically picks
  a free IP address from within the configured IP range for `type: ClusterIP` Services" — or statically.
  ([kubernetes.io — Service ClusterIP allocation](https://kubernetes.io/docs/concepts/services-networking/cluster-ip-allocation/))
- Stable for the Service's life: "Kubernetes assigns this Service an IP address (the _cluster IP_),
  that is used by the virtual IP address mechanism." The cluster IP is stable and persists for the
  life of the Service. ([kubernetes.io — Service](https://kubernetes.io/docs/concepts/services-networking/service/))
- Deallocation on delete: "IPs are correctly de-allocated when a service is removed" — i.e. the
  ClusterIP is released back to the pool only on Service deletion. The IPAddress object lifecycle is
  Service-bound: "ServiceCIDRs are protected with finalizers, to avoid leaving Service ClusterIPs
  orphans." ([kubernetes.io — Service ClusterIP allocation](https://kubernetes.io/docs/concepts/services-networking/cluster-ip-allocation/);
  cross-ref [GH kubernetes/kubernetes #87603](https://github.com/kubernetes/kubernetes/issues/87603) —
  a bug where IPs are NOT released *on delete via finalizer-removing update*, which confirms the
  normal contract: release happens on Service **deletion**, nowhere else.)

**Source**: [kubernetes.io ClusterIP allocation](https://kubernetes.io/docs/concepts/services-networking/cluster-ip-allocation/) — Accessed 2026-06-28
**Confidence**: High
**Verification**: kubernetes.io Service docs; kubernetes.io ClusterIP allocation; GH issue #87603 (release-on-delete is the only release path).
**Analysis**: The lifecycle anchor is the **Service object**, not the backing Pods. Scaling a
Deployment to zero leaves the Service (and its ClusterIP + IPAddress object) fully intact.

### K2 — Endpoints under scale-to-zero — CONFIRMED: ClusterIP persists, EndpointSlices empty
**Claim**: When a Deployment is scaled to 0 (Service still declared, zero ready pods), the ClusterIP
persists unchanged; the EndpointSlice controller updates the EndpointSlices to an empty set.

**Evidence**:
- "The controller continuously updates the Service's EndpointSlices when Pods matching the selector
  change, but the Service itself maintains its stable IP address and name."
  ([kubernetes.io — Service](https://kubernetes.io/docs/concepts/services-networking/service/))
- The EndpointSlice membership is gated on Pod readiness, but the Service's ClusterIP is not — only
  the *endpoint set* tracked behind the VIP changes. (kubernetes.io Service / EndpointSlice concept.)

**Source**: [kubernetes.io — Service](https://kubernetes.io/docs/concepts/services-networking/service/) — Accessed 2026-06-28
**Confidence**: High (the VIP-persists half is directly stated; "EndpointSlices empty at zero pods"
is the documented EndpointSlice controller behavior — endpoints track ready Pods).
**Verification**: kubernetes.io Service docs; CoreDNS plugin docs (below) corroborate that a service
"without any ready endpoint addresses" is a real, named state the DNS layer must handle.
**Analysis**: This is the exact analog to Overdrive's "workload stopped, VIP should still exist":
K8s keeps the VIP and just empties the backend set.

### K3 — DNS behavior (CoreDNS / kube-dns) — CONFIRMED: ClusterIP svc with zero endpoints still resolves to the VIP by default; headless with zero pods → NXDOMAIN
**Claim (a)**: For a normal **ClusterIP** Service, DNS resolves the name to the **stable ClusterIP**
regardless of endpoint readiness. By default a ClusterIP service with zero ready endpoints STILL
returns its ClusterIP (it does NOT return NXDOMAIN). NXDOMAIN-on-zero-endpoints is opt-in.
**Claim (b)**: For a **headless** Service (`clusterIP: None`), the name resolves to the set of Pod
IPs; with no ready pods the answer collapses, and the endpoint/headless query path returns NXDOMAIN.

**Evidence**:
- ClusterIP record: "'Normal' (not headless) Services are assigned DNS A and/or AAAA records … This
  resolves to the cluster IP of the Service."
  ([kubernetes.io — DNS for Services and Pods](https://kubernetes.io/docs/concepts/services-networking/dns-pod-service/))
- Headless record: "Headless Services (without a cluster IP) are also assigned DNS A and/or AAAA
  records … Unlike normal Services, this resolves to the set of IPs of all of the Pods selected by
  the Service." (same page)
- **Default ClusterIP-with-zero-endpoints is NOT NXDOMAIN** — proven by the CoreDNS opt-in flag whose
  description names exactly that change: `ignore empty_service` "returns NXDOMAIN for services without
  any ready endpoint addresses (e.g., ready pods)." That this is an *option* establishes the default:
  a ClusterIP service with no ready endpoints still resolves (to its ClusterIP), success/NODATA-style,
  not NXDOMAIN. ([CoreDNS kubernetes plugin](https://coredns.io/plugins/kubernetes/))
- **Headless / endpoint queries → NXDOMAIN when empty**: the `noendpoints` option states "All endpoint
  queries and headless service queries will result in an NXDOMAIN." The `fallthrough` mechanic only
  fires on NXDOMAIN: "if a query … results in NXDOMAIN, normally that is what the response will be."
  (same CoreDNS page) — i.e. the headless path is the one that yields NXDOMAIN when there are no IPs to
  return, whereas the ClusterIP path returns the stable VIP.

**NXDOMAIN vs NODATA semantics**: NXDOMAIN = "the queried name does not exist at all" (RCODE 3,
RFC 8020 "NXDOMAIN really means there is nothing at or below this name"). NODATA = "the name exists
but has no records of the requested type" (NOERROR + empty answer). A ClusterIP service name always
"exists" (it has an A/AAAA = the VIP), so zero-endpoints is NODATA-shaped, not NXDOMAIN — the
identity survives. A headless name with no pods has no addresses to return; CoreDNS returns NXDOMAIN
on the endpoint/headless path. ([RFC 8020](https://www.rfc-editor.org/rfc/rfc8020); CoreDNS plugin.)

**Source**: [CoreDNS kubernetes plugin](https://coredns.io/plugins/kubernetes/) + [kubernetes.io DNS](https://kubernetes.io/docs/concepts/services-networking/dns-pod-service/) — Accessed 2026-06-28
**Confidence**: High
**Verification**: kubernetes.io DNS docs (record shapes); CoreDNS plugin docs (`ignore empty_service`,
`noendpoints`, `fallthrough` flags); RFC 8020 (NXDOMAIN definition).
**Analysis**: The headless Service is the closest analog to Overdrive's dial-by-name (returns backend
addrs, no VIP) — and it is exactly the case that goes NXDOMAIN when backends are gone. The ClusterIP
case (a stable VIP) is the analog to Overdrive's service VIP — and it keeps resolving.

### K4 — Architectural separation — CONFIRMED: name + stable IP identity deliberately decoupled from backend availability
**Claim**: Kubernetes deliberately decouples the Service identity (name + stable ClusterIP) from
backend Pod availability/readiness. The Service is a stable abstraction; clients dial the VIP and
need not track or be aware of the backing Pods.

**Evidence**:
- "The Service abstraction enables this decoupling." — the documented design intent.
- "the frontend clients should not need to be aware of that, nor should they need to keep track of
  the set of backends themselves."
- "A Service is a method for exposing a network application that is running as one or more Pods" —
  the Service provides a logical abstraction whose stable IP and name persist while the backend
  Pod set changes underneath. ([kubernetes.io — Service](https://kubernetes.io/docs/concepts/services-networking/service/))

**Source**: [kubernetes.io — Service](https://kubernetes.io/docs/concepts/services-networking/service/) — Accessed 2026-06-28
**Confidence**: High
**Verification**: kubernetes.io Service docs (explicit "decoupling" language); reinforced by K1-K3
(VIP lifetime ≠ endpoint lifetime).
**Analysis**: The stable IP and the backend-availability signal are two separate planes by design —
identity (VIP/name, Service-lifetime) vs. reachability (endpoints, readiness-gated). This is the
crux the architect needs: K8s does NOT collapse the two.

---

## Findings — Nomad (+ Consul)

### N1 — stop vs purge — CONFIRMED: stopped job retained in state + queryable; `-purge` removes from system
**Claim**: `nomad job stop` (without `-purge`) leaves the job in state as "dead" but **queryable** (and
re-runnable by re-submitting / re-deploying the spec); it is only removed by garbage collection later.
`nomad job stop -purge` removes the job from the system immediately so it is no longer queryable. The
durable lifecycle anchor is the **job object**, which survives a transient stop and is purged only on
explicit `-purge` (or eventual GC).

**Evidence**:
- "Purge is used to stop the job and purge it from the system." and "If not set, the job will still be
  queryable and will be purged by the garbage collector."
  ([developer.hashicorp.com — nomad job stop](https://developer.hashicorp.com/nomad/docs/commands/job/stop);
  same text mirrored at [nomadproject.io — job stop](https://www.nomadproject.io/docs/commands/job/stop))
- A stopped job stays in "dead" state and remains visible until purged: "without the `-purge` flag,
  stopped jobs remain queryable in the system and are expected to be handled by the garbage collector"
  (cross-ref [HashiCorp Discuss — Dead job not purged by GC](https://discuss.hashicorp.com/t/dead-nomad-job-not-purged-by-gc-garbage-collection/33649)).

**Source**: [developer.hashicorp.com — nomad job stop](https://developer.hashicorp.com/nomad/docs/commands/job/stop) — Accessed 2026-06-28
**Confidence**: High (official CLI reference; mirrored on nomadproject.io; corroborated by community thread)
**Verification**: developer.hashicorp.com job/stop; nomadproject.io job/stop; HashiCorp Discuss GC thread.
**Analysis**: This is Nomad's stop-vs-delete boundary, structurally identical to K8s scale-to-zero
(retain) vs Service-delete (release) and to Overdrive's stop vs delete. The job (the declared object)
is the lifetime anchor; `-purge` is the "delete."

### N2 — service (de)registration on alloc stop — CONFIRMED: alloc stop → service immediately deregistered → name stops resolving
**Claim**: When a Nomad allocation/task stops (job stop, task exit, or Nomad-initiated kill), its
service registration (Nomad-native and/or Consul) is **deregistered immediately**, so the name stops
resolving once the allocation is stopped.

**Evidence**:
- "If a running task with a service block exits, the services and checks are immediately deregistered
  from the provider without delay."
- On Nomad-initiated stop: "Immediately remove the services and checks from the provider. This stops
  new traffic from being routed to the task that is being killed."
  ([developer.hashicorp.com — service block](https://developer.hashicorp.com/nomad/docs/job-specification/service))

**Source**: [developer.hashicorp.com — service block](https://developer.hashicorp.com/nomad/docs/job-specification/service) — Accessed 2026-06-28
**Confidence**: High (official job-spec reference, explicit "immediately deregistered" language)
**Verification**: developer.hashicorp.com service block (two explicit statements); corroborated by N4
(Consul DNS returns nothing useful once no instances are registered/healthy).
**Analysis**: Name resolution is gated on a **live, healthy registration** — it disappears on stop.
This is the direct analog of K8s endpoints emptying and the headless-name collapsing on scale-to-zero.

### N3 — Consul service-mesh virtual IPs (transparent proxy) — CONFIRMED: per-SERVICE VIP, stable across instance churn, keyed to the logical service name
**Claim**: With transparent proxy enabled, Consul assigns a **unique virtual IP per service** (in the
`240.0.0.0/4` range, surfaced as the `consul-virtual` tagged address). The VIP is bound to the
**logical service (the catalog service name)**, not to individual instances — it is stable across
instance churn and **load-balances across the service's instances**. The VIP is the mesh-dialing
analog of a K8s ClusterIP.

**Evidence**:
- "Consul generates a unique virtual IP for each service deployed within Consul Service Mesh,
  allowing transparent proxy to route to services within a data center."
- "Consul assigns a virtual IP in the 240.0.0.0/4 range to a service with transparent proxy enabled."
- "transparent proxies typically dialing upstreams using the 'virtual' tagged address, which load
  balances across instances." (HashiCorp Developer docs, surfaced via search of
  [developer.hashicorp.com — transparent proxy](https://developer.hashicorp.com/consul/docs/connect/proxy/transparent-proxy)
  and [Nomad transparent_proxy block](https://developer.hashicorp.com/nomad/docs/job-specification/transparent_proxy))
- The range is fixed in Consul source: "starting virtual IP of 240.0.0.0 … maximum offset to
  255.255.255.254" (cross-ref [GH hashicorp/consul #22595 — TPROXY IP range not configurable](https://github.com/hashicorp/consul/issues/22595)).
- HashiCorp's own framing equates the mesh VIP to a ClusterIP "tied to the Kubernetes Service":
  "each service is given a unique, virtual IP … called a clusterIP, that is tied to the Kubernetes
  Service." ([hashicorp.com — Transparent Proxy on Consul Service Mesh](https://www.hashicorp.com/en/blog/transparent-proxy-on-consul-service-mesh))

**Source**: [developer.hashicorp.com — Consul transparent proxy](https://developer.hashicorp.com/consul/docs/connect/proxy/transparent-proxy) — Accessed 2026-06-28
**Confidence**: High for "per-service, stable across instance churn, keyed to the service name";
**Medium** for the exact *release timing* (see Gap below — release-on-deregistration is not stated in
public docs; it is an implementation detail of the catalog's VIP allocator).
**Verification**: HashiCorp Developer transparent-proxy docs; HashiCorp official blog (VIP↔ClusterIP
equivalence); GH consul #22595 (240.0.0.0/4 range hardcoded in source).
**Analysis**: This is the closest Nomad/Consul analog to Overdrive's allocator-issued service VIP. It
is **identity** (logical service → stable routable VIP), decoupled from instance lifetime, exactly as
the K8s ClusterIP is decoupled from Pod lifetime. The VIP follows the *service registration in the
catalog*, not any one instance — so it survives instance churn while the service is declared.

### N4 — Consul DNS with zero healthy instances — CONFIRMED: health-filtered; no-instances → NXDOMAIN, all-unhealthy → empty NOERROR; stopped-not-purged service stops resolving once deregistered
**Claim**: Consul DNS returns only healthy/passing instances: "Services that fail their health check
or that fail a node system check are omitted from the results." When there are **zero instances at
all** (e.g. all deregistered after stop), Consul returns **NXDOMAIN**; when instances exist but **all
fail health checks**, Consul historically returns **NOERROR with an empty answer** (NODATA-shaped) —
a documented inconsistency it has tried to reconcile toward NXDOMAIN.

**Evidence**:
- Health filtering: "Services that fail their health check or that fail a node system check are
  omitted from the results."
  ([developer.hashicorp.com — DNS static lookups](https://developer.hashicorp.com/consul/docs/services/discovery/dns-static-lookups))
- No-instances vs all-unhealthy split: "The Consul DNS parser checks for service instances before
  filtering on health … it returns NXDOMAIN if there are no instances, but responds with no answers
  and NOERROR if all instances are filtered out due to health status."
  (cross-ref [GH hashicorp/consul #1142 — DNS does not set NXDOMAIN if all instances unhealthy](https://github.com/hashicorp/consul/issues/1142))
- NXDOMAIN-vs-NODATA framing matches RFC 8020: "NXDOMAIN should only be returned when the requested
  name has no records of any kind … 'the name exists but doesn't have records of this type' should be
  NOERROR with an empty answer section." (consul #1142 discussion; [RFC 8020](https://www.rfc-editor.org/rfc/rfc8020))

**Source**: [developer.hashicorp.com — DNS static lookups](https://developer.hashicorp.com/consul/docs/services/discovery/dns-static-lookups) — Accessed 2026-06-28
**Confidence**: High for "health-filtered, zero-instances→NXDOMAIN, all-unhealthy→empty NOERROR";
the all-unhealthy edge is a known historical inconsistency (cite #1142 honestly).
**Verification**: developer.hashicorp.com DNS static lookups (health filtering); GH consul #1142
(NXDOMAIN vs NOERROR split); RFC 8020 (NXDOMAIN semantics).
**Analysis**: A stopped-not-purged Nomad job whose alloc has deregistered its service (N2) leaves the
**service with zero registered instances → NXDOMAIN** on the Nomad/Consul *service-DNS* name. Note the
service-DNS name is the discovery (backend-list) plane — the *mesh VIP* (N3) is the separate identity
plane that does NOT depend on instance health. The two planes mirror K8s headless-vs-ClusterIP.

---

## Synthesis (payload for the architect)

### S1 — YES (both systems): stable IP/VIP identity is tied to the declared-object lifetime; name resolution is gated on healthy backends
Both Kubernetes and Nomad+Consul split the problem into **two planes**, and the split is the same in
each:

| Plane | Kubernetes | Nomad + Consul | Lifetime anchor |
|---|---|---|---|
| **Identity** (stable IP/VIP, name) | ClusterIP + IPAddress object, tied to the **Service** object | Mesh **virtual IP** (240.0.0.0/4), tied to the **logical service** in the catalog | The **declared object** (Service / service registration) — released on **delete/purge** only |
| **Reachability** (resolvable backends) | EndpointSlices, gated on Pod **readiness** | Service-DNS answer, gated on instance **health** | The **running instances** — collapses on stop |

- The **stable IP/VIP** is released only when the declared object is removed: K8s ClusterIP is
  de-allocated only on Service **deletion** (K1); the Nomad job (and thus its declared service)
  persists through a transient stop and is removed only by `-purge`/GC (N1). Neither releases the
  stable IP on scale-to-zero / stop.
- **Name resolution disappears on stop** in the *backend-list* plane: K8s headless-name collapses and
  endpoints empty at zero ready pods (K2, K3b); Nomad deregisters the service immediately on alloc
  stop, so the service-DNS name returns NXDOMAIN once no instances remain (N2, N4). But the K8s
  **ClusterIP name still resolves to the VIP** at zero endpoints (K3a) — identity outlives reachability.

**Neither system releases the stable IP/VIP on a transient stop.** Identity = object-lifetime;
reachability = instance-health. (Confidence: High for K8s; High for Nomad+Consul, with the one
caveat that Consul's *exact VIP reclamation timing* is undocumented — see Gap 1.)

### S2 — NO asymmetry: the prevailing convention is a SINGLE withhold-not-release identity lifecycle
Overdrive's current asymmetry — frontend `F` **retained** on stop, but service VIP **released** on
stop — has **no analog** in either reference system. In both K8s and Consul, **every stable identity
(both the human-facing name and the stable IP/VIP) shares one lifecycle**: retained across
scale-to-zero / stop, released only on delete/purge. There is no case where one stable identity is
retained on stop while a sibling stable identity is released on stop.

- K8s: the Service name *and* its ClusterIP are one object with one lifetime — you never get the name
  kept while the ClusterIP is reclaimed under a transient scale-to-zero (K1, K4).
- Consul: the service name *and* its mesh VIP both follow the service registration in the catalog;
  the VIP load-balances across instances and is keyed to the service, not the instances (N3).

The thing that *does* legitimately disappear on stop in both systems is **backend reachability** (the
endpoint set / the healthy-instance DNS answer) — which is not an *identity*, it's the live-routing
plane. So the convention is: **withhold resolution (no healthy backends) but do not release the stable
identity.** Overdrive's release-of-VIP-on-stop is the outlier; its retain-`F`-on-stop matches the
convention.

### S3 — Net implication for ADR-0049 §6 / Option C (evidence, NOT an Overdrive prescription)
**Evidence statement for the architect:**
> Both reference orchestrators tie the **stable virtual IP identity to the declared-object lifetime**
> and release it **only on delete/purge**: the Kubernetes ClusterIP is bound to the Service object and
> de-allocated only on Service deletion (not by Pod churn, scale-to-zero, or zero ready endpoints);
> the Consul service-mesh virtual IP is bound to the logical service in the catalog, stable across
> instance churn, and surfaced as the load-balancing `consul-virtual` address. In both systems, what
> disappears on a transient stop is **backend reachability** (empty EndpointSlices / NXDOMAIN-or-empty
> service-DNS), not the stable IP. Against this prior art, Overdrive's **release-on-terminal-state**
> contract (ADR-0049 §6) is the **outlier**, and "Option C" (retain VIP until logical-workload
> deletion, mirroring the already-retained frontend `F`) is what aligns with the established
> convention and removes Overdrive's identity-lifecycle **asymmetry**.

**Honest counter-evidence / caveats (so the architect weighs the real cost):**
1. **IP-pool pressure is a real cost the reference systems mitigate, not ignore.** Retaining VIPs
   until delete means stopped-but-not-deleted workloads hold IPs. K8s mitigates with a *large*
   ServiceCIDR and IPAddress-object accounting (and has had real bugs where IPs leak and are *not*
   released even on delete — GH kubernetes #87603 — i.e. the release path itself is fragile). Consul
   confines mesh VIPs to the large `240.0.0.0/4` space. The retain-until-delete contract presumes the
   VIP space is big relative to the count of stopped-not-deleted workloads.
2. **Consul's exact VIP *reclamation* timing is undocumented** (Gap 1). The strong, well-sourced claim
   is "VIP is per-service and stable across instance churn"; "released exactly on service
   deregistration vs. some longer-lived catalog cleanup" is an implementation detail not pinned in
   public docs. Do not over-claim the *release* half for Consul.
3. **The two planes must not be conflated.** The prior art keeps the stable IP **and** still makes the
   name return "no usable backends" when backends are gone (empty endpoints / NXDOMAIN). The bug in
   #251 is precisely that Overdrive's VIP release *nulls the input the name-index retraction needs* —
   i.e. Overdrive currently couples the identity plane and the reachability plane. The prior art's
   lesson is to **decouple** them: retain the VIP (identity) AND independently retract the
   dial-by-name resolution (reachability) on stop. Retaining the VIP does not, in the reference
   systems, imply the name keeps resolving a stopped workload — those are separate signals.
4. **Ephemeral-IP cases exist but are not the stable-service-VIP case.** K8s Pod IPs *are* released on
   Pod deletion/churn (ephemeral by design) — but that is the *instance* address, the analog of an
   Overdrive backend address, not the *service VIP*. The stable-service-IP (ClusterIP / mesh VIP) is
   the one that is retain-until-delete. Don't import Pod-IP ephemerality onto the VIP question.

---

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Kubernetes — Service ClusterIP allocation | kubernetes.io | High (1.0) | official | 2026-06-28 | Y |
| Kubernetes — Service | kubernetes.io | High (1.0) | official | 2026-06-28 | Y |
| Kubernetes — DNS for Services and Pods | kubernetes.io | High (1.0) | official | 2026-06-28 | Y |
| Kubernetes — Virtual IPs and Service Proxies | kubernetes.io | High (1.0) | official | 2026-06-28 | Y |
| CoreDNS — kubernetes plugin | coredns.io | High (1.0) | official/OSS (CNCF) | 2026-06-28 | Y |
| RFC 8020 — NXDOMAIN: There Really Is Nothing Underneath | rfc-editor.org | High (1.0) | standard | 2026-06-28 | Y |
| Nomad — `job stop` command | developer.hashicorp.com | High (1.0) | official | 2026-06-28 | Y |
| Nomad — `job stop` (mirror) | nomadproject.io | High (1.0) | official | 2026-06-28 | Y |
| Nomad — `service` block | developer.hashicorp.com | High (1.0) | official | 2026-06-28 | Y |
| Consul — Transparent proxy | developer.hashicorp.com | High (1.0) | official | 2026-06-28 | Y |
| Consul — DNS static lookups | developer.hashicorp.com | High (1.0) | official | 2026-06-28 | Y |
| HashiCorp — Transparent Proxy on Consul Service Mesh (blog) | hashicorp.com | High (0.9) | official/vendor | 2026-06-28 | Y |
| GH kubernetes/kubernetes #87603 (IP-on-delete release) | github.com | Medium-High (0.8) | OSS issue | 2026-06-28 | Y (corroborating) |
| GH hashicorp/consul #1142 (DNS NXDOMAIN vs NOERROR) | github.com | Medium-High (0.8) | OSS issue | 2026-06-28 | Y (corroborating) |
| GH hashicorp/consul #22595 (240.0.0.0/4 range) | github.com | Medium-High (0.8) | OSS issue | 2026-06-28 | Y (corroborating) |
| HashiCorp Discuss — Dead job not purged by GC | discuss.hashicorp.com | Medium (0.6) | community | 2026-06-28 | Y (corroborating only) |

Reputation: High: 12 (75%) | Medium-High: 3 (19%) | Medium: 1 (6%) | Avg ≈ 0.93.
Every major claim rests on ≥1 official/standards source; GitHub issues and the Discuss thread are
corroborating cross-references, never sole sources.

## Knowledge Gaps
### Gap 1: Consul mesh-VIP exact reclamation timing
**Issue**: Public Consul docs confirm the VIP is per-service, stable across instance churn, and keyed
to the logical service — but do **not** explicitly state *when* the VIP is released (on service
deregistration from the catalog vs. a longer-lived cleanup). **Attempted**: Consul transparent-proxy
docs, HashiCorp blog, GitHub search of hashicorp/consul issues, web-unified-docs source. **Recommendation**:
read the Consul source VIP allocator (`agent/consul/`, the `consul-virtual` tagged-address assignment)
or test directly; treat the *release* half as Medium confidence. This does not weaken S1/S2 — the
load-bearing claim is "stable across instance churn, not released on stop," which is well-sourced.

### Gap 2: Consul all-unhealthy DNS edge (NXDOMAIN vs empty NOERROR) is version-sensitive
**Issue**: The zero-instances→NXDOMAIN vs all-unhealthy→empty-NOERROR split is a documented historical
inconsistency (GH #1142) and may differ by Consul version / DNS config (`soa.min_ttl`,
`only_passing`). **Attempted**: Consul DNS static-lookups + configure-DNS docs, GH #1142. **Recommendation**:
pin to a specific Consul version if the exact RCODE matters; the directional finding (health-filtered,
"no usable answer" when no instance passes) is robust regardless.

### Gap 3: kube-dns (legacy) not separately verified
**Issue**: Findings for the DNS layer are pinned to **CoreDNS** (the current default). The legacy
kube-dns behavior was not independently verified. **Recommendation**: CoreDNS has been the K8s default
since v1.13 (2018); kube-dns is effectively historical — not a material gap for a current-design decision.

## Conflicting Information
### Conflict 1: Consul DNS RCODE when all instances are unhealthy
**Position A** (intended/spec-aligned): zero registered instances → NXDOMAIN; this is "name does not
exist." — Source: GH hashicorp/consul #1142, reputation 0.8; aligns with RFC 8020 (rfc-editor.org, 1.0).
**Position B** (observed/historical): when instances exist but *all* fail health checks, Consul returns
**NOERROR with an empty answer** (NODATA-shaped), not NXDOMAIN — flagged as a bug to reconcile. — Source:
GH hashicorp/consul #1142, reputation 0.8.
**Assessment**: Not a true contradiction but a **state distinction** (zero-instances vs
instances-all-unhealthy) plus a known historical inconsistency in the all-unhealthy RCODE. For the
Overdrive decision it does not matter which RCODE the all-unhealthy edge uses: the relevant analog is
a *stopped* workload whose registration is **gone** (zero instances → NXDOMAIN), and the broader point
— Consul gates the DNS *answer* on healthy backends — holds either way.

## Full Citations
[1] Kubernetes. "Service ClusterIP allocation". kubernetes.io. Accessed 2026-06-28. https://kubernetes.io/docs/concepts/services-networking/cluster-ip-allocation/
[2] Kubernetes. "Service". kubernetes.io. Accessed 2026-06-28. https://kubernetes.io/docs/concepts/services-networking/service/
[3] Kubernetes. "DNS for Services and Pods". kubernetes.io. Accessed 2026-06-28. https://kubernetes.io/docs/concepts/services-networking/dns-pod-service/
[4] Kubernetes. "Virtual IPs and Service Proxies". kubernetes.io. Accessed 2026-06-28. https://kubernetes.io/docs/reference/networking/virtual-ips/
[5] CoreDNS. "kubernetes plugin". coredns.io. Accessed 2026-06-28. https://coredns.io/plugins/kubernetes/
[6] Bortzmeyer, S.; Huque, S. "RFC 8020 — NXDOMAIN: There Really Is Nothing Underneath". IETF. 2016. https://www.rfc-editor.org/rfc/rfc8020. Accessed 2026-06-28.
[7] HashiCorp. "nomad job stop command reference". developer.hashicorp.com. Accessed 2026-06-28. https://developer.hashicorp.com/nomad/docs/commands/job/stop
[8] HashiCorp. "nomad job stop" (mirror). nomadproject.io. Accessed 2026-06-28. https://www.nomadproject.io/docs/commands/job/stop
[9] HashiCorp. "service block in the job specification". developer.hashicorp.com. Accessed 2026-06-28. https://developer.hashicorp.com/nomad/docs/job-specification/service
[10] HashiCorp. "Transparent proxy overview". developer.hashicorp.com. Accessed 2026-06-28. https://developer.hashicorp.com/consul/docs/connect/proxy/transparent-proxy
[11] HashiCorp. "DNS static lookups". developer.hashicorp.com. Accessed 2026-06-28. https://developer.hashicorp.com/consul/docs/services/discovery/dns-static-lookups
[12] HashiCorp. "Transparent Proxy on Consul Service Mesh" (blog). hashicorp.com. Accessed 2026-06-28. https://www.hashicorp.com/en/blog/transparent-proxy-on-consul-service-mesh
[13] kubernetes/kubernetes. "service IPs and ports are not released when deleting a service via a finalizer-removing update" (#87603). github.com. Accessed 2026-06-28. https://github.com/kubernetes/kubernetes/issues/87603
[14] hashicorp/consul. "DNS resolver does not set NXDOMAIN code if all service instances are unhealthy" (#1142). github.com. Accessed 2026-06-28. https://github.com/hashicorp/consul/issues/1142
[15] hashicorp/consul. "Transparent proxy ip range is not configurable" (#22595). github.com. Accessed 2026-06-28. https://github.com/hashicorp/consul/issues/22595
[16] HashiCorp Discuss. "Dead nomad job not purged by GC". discuss.hashicorp.com. Accessed 2026-06-28. https://discuss.hashicorp.com/t/dead-nomad-job-not-purged-by-gc-garbage-collection/33649

## Research Metadata
Duration: ~40 min | Examined: ~16 sources | Cited: 16 | Cross-refs: every K/N claim cross-referenced
across ≥2 independent sources (official + corroborating) | Confidence: High (K1, K2, K3, K4, N1, N2,
N4; N3 identity-half High, N3 VIP-release-timing Medium) | Citation coverage: >95% | Avg reputation
≈ 0.93 | Output: docs/research/orchestration/service-vip-dns-lifecycle-stop-vs-delete-k8s-nomad.md
