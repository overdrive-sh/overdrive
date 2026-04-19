# Overdrive: Commercial Model

**Version 0.3 — Draft**

---

## Overview

Overdrive is source-available under the Functional Source License (FSL-1.1-ALv2); every release converts to Apache 2.0 two years after publication under the irrevocable future-grant built into the licence. The commercial opportunity is not the software itself — it is the operational complexity the software absorbs. Every team running Kubernetes today operates etcd, CNI plugins, cert-manager, a service mesh, an ingress controller, Prometheus, alertmanager, and a dozen other components that each need to be deployed, upgraded, and debugged independently. Overdrive collapses that stack into one binary.

The cloud platform business charges for not having to operate that stack.

---

## The Core Insight: Overdrive Within Overdrive

Overdrive is self-hosting. You run Overdrive on bare metal. Tenants run their own Overdrive clusters — or simpler workload primitives — on top of it. Every layer of the platform becomes a multi-tenant product feature without additional engineering.

```
Physical bare metal
    │
Overdrive (infrastructure layer)
    │  manages nodes, provides tenant isolation
    ▼
Tenant A          Tenant B          Tenant C
Full Overdrive       Full Overdrive       WASM functions only
cluster           cluster
```

Tenant Overdrive clusters run as VM workloads on the infrastructure layer. Cloud Hypervisor provides VM-level isolation — each tenant control plane boots inside its own VM with its own kernel, network namespace, and storage. When a tenant scales their cluster, their new worker nodes are VM jobs scheduled by the infrastructure Overdrive. The tenant sees nodes appearing in their cluster; the operator sees VM jobs starting on bare metal.

---

## Product Tiers

### Tier 1 — Managed Overdrive

Full Overdrive cluster as a service. Tenant gets a control plane and worker node pool. They submit jobs, define policies, and operate their cluster through the Overdrive API and CLI. The infrastructure layer handles the underlying hardware, networking, and physical security.

Target: platform engineering teams building internal developer platforms, companies migrating off self-managed Kubernetes.

Billing model: per vCPU-hour + per GB-hour of memory consumed by the tenant cluster (including control plane overhead).

### Tier 2 — Managed Workloads

Tenant does not manage a cluster. They submit jobs directly to the platform. The platform's Overdrive schedules their workloads within a tenant namespace. Simpler operational model — no cluster to size, no control plane to monitor.

Target: engineering teams that want Nomad-style job submission without operating infrastructure.

Billing model: per vCPU-hour + per GB-hour, metered at the allocation level.

### Tier 3 — Serverless WASM

Tenant deploys WASM functions only. Sub-10ms cold start, scale-to-zero, zero cluster management. The WASM sidecar model applies — credential proxy, content inspection, domain allowlists — without any configuration beyond the function spec.

Target: developers who want Lambda economics without AWS lock-in. AI agent workloads with egress control requirements.

Billing model: per invocation + per GB-second of execution. Minimum billing unit is one invocation.

### Tier 4 — Bare Metal Dedicated

Tenant gets dedicated physical nodes within the infrastructure Overdrive. Full hardware performance, no VM overhead, still managed by the platform control plane — node provisioning, OS management, eBPF dataplane, mTLS, observability all included. Tenant workloads run directly on the hardware.

Target: compute-intensive workloads (ML training, HPC), latency-sensitive applications, tenants with compliance requirements around hardware isolation.

Billing model: per node-hour, reserved capacity discounts available.

---

## Isolation Architecture

Multi-tenancy is structural, not configured. The same primitives that provide workload isolation within a single cluster extend naturally to tenant isolation across the platform.

### Identity

The platform root CA issues a per-tenant intermediate certificate. All workload SVIDs under a tenant chain to their intermediate:

```
Platform Root CA
    ├── Infrastructure intermediate
    ├── Tenant A intermediate
    │       ├── control-plane SVID
    │       ├── worker-1 SVID
    │       └── job/payments SVID
    └── Tenant B intermediate
            └── ...
```

mTLS between tenants is impossible by construction — their certificate chains share no trust anchor below the root. Tenants cannot issue SVIDs outside their namespace. The platform controls the root.

### Network

Each tenant is assigned a VPC — a network segment isolated at the XDP layer. Cross-tenant packet delivery is dropped at the NIC driver level before it enters the kernel network stack:

```rust
fn xdp_tenant_isolation(ctx: XdpContext) -> u32 {
    let src_tenant = ALLOC_TENANT_MAP.get(&src_alloc_id);
    let dst_tenant = ALLOC_TENANT_MAP.get(&dst_alloc_id);

    if src_tenant != dst_tenant {
        return XDP_DROP;
    }
    XDP_PASS
}
```

A misconfigured workload, a compromised tenant, or a routing error cannot reach another tenant's network. Enforcement is in-kernel, below the application layer.

### State

Every write to the state store is namespaced by tenant ID. Tenant A cannot read or write Tenant B's state — enforced by the store abstraction, not by application-level access control. Tenant namespaces in the Raft log are isolated by key prefix with no cross-namespace reads permitted by the state machine.

### Compute

Tenant workloads run in a cgroup subtree scoped to their tenant ID. BPF LSM programs enforce that workloads cannot escape their subtree. VM workloads (Tiers 1 and 4) add a hardware isolation boundary via Cloud Hypervisor.

---

## Control Plane Density

A key commercial efficiency: small tenants do not require a dedicated 3-node Raft cluster. Overdrive' `LocalStore` (single-mode, redb direct) runs a full control plane in approximately 30MB RAM with no distributed consensus overhead.

```
1,000 small tenants × 30MB (LocalStore control plane) = 30GB RAM
vs
1,000 tenants × 3 × 8GB VMs (traditional HA control plane) = 24TB RAM
```

Tenants start in single-mode and are promoted to HA (`RaftStore`, 3-node) automatically when their SLA tier requires it or when their cluster exceeds a configured size threshold. The migration is non-destructive — `export_snapshot` / `bootstrap_from` handle the transition with zero downtime.

This density advantage is the primary margin driver for Tiers 2 and 3.

---

## Billing Infrastructure

The eBPF telemetry layer collects precise per-tenant, per-workload resource consumption at the kernel level with no sampling and no separate metering agent:

```sql
SELECT
    tenant_id,
    sum(cpu_cycles)           as total_cpu,
    max(rss)                  as peak_memory_bytes,
    sum(bytes_in + bytes_out) as total_network_bytes,
    count(distinct alloc_id)  as allocation_hours,
    sum(wasm_invocations)     as function_invocations
FROM telemetry
WHERE timestamp BETWEEN billing_period_start AND billing_period_end
GROUP BY tenant_id
```

Every CPU cycle, every byte of memory, every network byte, every function invocation is recorded in DuckLake with cryptographic workload identity. Billing is exact, not estimated. Disputes have kernel-level evidence.

The same telemetry feeds the LLM observability layer — cost anomaly detection, usage forecasting, and right-sizing recommendations are available to tenants as self-service tools.

---

## LLM Self-Healing as Retention

The LLM self-healing layer is the primary retention mechanism.

A cluster managed by Overdrive accumulates incident memory over time. Past incidents, their diagnoses, the actions that resolved them, and resource profiles built from continuous eBPF observation are all stored in libSQL and used by the LLM agent to reason about new anomalies. The platform's diagnostic accuracy and right-sizing precision improve with operational age.

This creates switching cost that is not contractual or technical lock-in — it is operational memory that does not migrate. A tenant who has run on the platform for 12 months has a cluster that understands their workload patterns, has resolved dozens of incidents, and has tuned resource profiles across their entire job fleet. Starting over elsewhere means starting from scratch.

---

## Source-Available Strategy

Overdrive is licensed under the Functional Source License (FSL-1.1-ALv2). The licence choice is deliberate.

**What FSL permits.** Internal use is unrestricted — any team, any scale, any industry. Modifications are permitted. Redistribution is permitted. Non-commercial education and research are explicitly carved out. For the overwhelming majority of users who run Overdrive to operate their own infrastructure, FSL is functionally equivalent to Apache 2.0 from day one.

**What FSL prohibits.** For two years after each release, the licence forbids offering a commercial product or service that substitutes for Overdrive or for our managed offering. A hyperscaler cannot take the codebase, wrap it in a managed console, and resell "Managed Overdrive" against us. This is the structural protection AGPL only weakly provided — AGPL permits any competitor who publishes their fork to host a commodity service at scale against the original project.

**Grant of Future License.** Every release converts to Apache 2.0 on its second anniversary under an irrevocable grant written into the licence itself. Users who need a true open source fallback for long-term planning have one, on a published schedule. The community ultimately receives every release under a permissive OSI-approved licence; the two-year window is the commercial protection, not a permanent enclosure.

**Why not AGPL.** AGPL's viral network-copyleft clause triggers enterprise legal review at most large organisations and routinely disqualifies software before technical evaluation begins. FSL permits internal modifications and internal deployment without copyleft obligations, removing the single largest legal objection to source-available infrastructure software in regulated industries. Sentry, Keygen, Sourcegraph, and an increasing share of commercial infrastructure projects converged on this model for the same reason.

**Why not a proprietary licence with a free tier.** The community flywheel — self-hosted adoption driving familiarity that shortens the sales cycle, external contributions improving the eBPF dataplane, the WASM runtimes, and the driver model — is structurally dependent on the source being available and modifiable. FSL preserves that flywheel; a proprietary licence does not.

A commercial licence is available for two cases FSL does not cover: organisations that want to embed Overdrive inside a commercial product they sell (a Competing Use under the licence), and organisations that want capabilities outside the source-available release. This is the third revenue stream alongside the cloud platform and support contracts.

---

## Enterprise Self-Hosted Licensing

Many enterprise buyers — financial services, government, regulated healthcare, defence — cannot use a managed cloud platform for their most sensitive workloads. Data sovereignty requirements, air-gap mandates, internal security policy, or regulatory frameworks (DORA, NIS2, FedRAMP, IL4/IL5) require the software to run entirely on their own infrastructure, under their own control.

These organisations still want the platform. They cannot use the cloud offering. They also often need capabilities not included in the source-available release — FIPS crypto, HSM integration, compliance policy packs, air-gap tooling — along with warranty-backed support contracts that procurement requires and FSL, as a software licence, does not provide. Enterprise licensing is the answer.

### What Enterprise Licensing Covers

**Commercial licence.** Grants rights beyond FSL — notably a warranty and indemnity for the software, a trademark grant for using the "Overdrive" name in customer-facing materials, and a carve-out from the Competing Use restriction for organisations that embed Overdrive inside a product they sell. Covers unlimited nodes within the licensed estate.

**Enterprise feature tier.** Certain features are developed for and licensed exclusively to enterprise customers — not included in the open source release:

| Feature | Why Enterprise-Only |
|---|---|
| FIPS 140-2/3 crypto backend | Requires aws-lc-rs FIPS mode, certification maintenance cost |
| Hardware HSM integration | TPM 2.0 / CloudHSM / Thales for root CA key storage |
| Air-gap installation tooling | Offline image bundler, air-gapped Garage mirror |
| Advanced audit log export | Signed, tamper-evident audit streams for compliance |
| Policy compliance packs | Pre-built Rego + WASM policy sets for DORA, NIS2, SOC2, HIPAA |
| SSO / SAML integration | Enterprise identity provider federation |
| Priority CVE response | SLA-backed security patch delivery |
| Long-term support releases | 24-month LTS with backported security fixes |

**Support contract.** Dedicated support channel, named engineers, SLA-backed response times. Includes architecture review for initial deployment and annual health checks.

### Pricing Model

Enterprise licensing is annual subscription, priced per node (physical or virtual) under management:

```
Starter (up to 50 nodes):        flat annual fee
Growth  (51–500 nodes):          per-node annual fee, volume discount
Scale   (501–5000 nodes):        per-node annual fee, larger discount
Strategic (5000+ nodes):         negotiated, multi-year
```

Air-gap and government deployments carry a premium — they require dedicated support engineering, longer sales cycles, and ongoing compliance maintenance.

Support tiers are sold separately and stack on top of the licence fee:

```
Community:   GitHub issues, no SLA (open source only)
Standard:    8×5, 24hr response, email + ticketing
Professional: 24×7, 4hr critical response, dedicated Slack channel
Enterprise:  24×7, 1hr critical response, named engineer, quarterly reviews
```

### The Air-Gap Use Case

Air-gapped deployments deserve specific treatment. A financial exchange, a defence contractor, or a government agency running Overdrive in a classified environment needs:

- All container images and WASM modules bundled and cryptographically signed for offline installation
- The Garage object store pre-seeded with all platform artifacts
- No outbound network calls from the control plane under any circumstances
- Audit tooling that produces compliance artefacts without external dependencies
- The LLM observability layer configured to use a locally hosted model (Ollama or equivalent) rather than an external API

This is a complete product configuration, not a stripped-down version. The platform's architecture actually makes air-gap deployment easier than most platforms — one binary, embedded storage, no external service dependencies by design.

### Why Enterprises Pay

The honest answer is that enterprise procurement pays for three things: capability, risk reduction, and a throat to choke.

**Capability.** The features listed above — FIPS crypto, HSM integration, compliance policy packs — are genuine requirements for regulated industries, not nice-to-haves. They cannot be assembled from open source components without significant engineering investment.

**Risk reduction.** An enterprise deploying open source software bears the full operational and security risk. A commercial licence with an LTS stream, SLA-backed CVE response, and named support engineers transfers a meaningful portion of that risk. This is a real purchase, not a donation.

**A throat to choke.** Procurement and legal teams require a contractual relationship with a vendor — warranty, indemnity, SLA, and a clear chain of accountability when something goes wrong in production. A source-available licence is a software licence, not a commercial relationship; it provides none of those. The commercial contract exists to satisfy procurement regardless of whether the engineering team strictly needs the enterprise feature tier.

### The Flywheel

Enterprise self-hosted customers generate a second-order benefit beyond direct revenue: they run Overdrive at scale in production environments the cloud platform cannot access. Their deployments surface edge cases, scale issues, and feature requirements that improve the platform for all users. Many enterprise customers contribute patches under the commercial licence — these flow back into the open source release. The enterprise customer base funds platform development that benefits the open source community.

---

## Go-To-Market

### Phase 1 — Open Source Traction

Ship the open source platform. Target infrastructure engineers and platform teams who are frustrated with Kubernetes complexity. The efficiency story (section 20 of the whitepaper), the unified workload model, and the WASM-native architecture are the primary technical hooks.

Measure: GitHub stars, self-hosted deployments, community contributions, enterprise inbound.

### Phase 2 — Cloud Platform (Managed Overdrive + Managed Workloads)

Launch Tiers 1 and 2 on owned bare metal. Initial target: teams already self-hosting Overdrive who want to offload operations. Low acquisition cost — they already know the product.

Expand to teams migrating off EKS/GKE who want operational simplicity without rebuilding their entire toolchain.

### Phase 3 — Serverless WASM + AI Agent Platform

Tier 3 targets the AI agent workload market specifically. The credential proxy, content inspection sidecar, and domain allowlist model solve the prompt injection and credential exfiltration problems that every team building production AI agents faces. This is a distinct product from the orchestration story — position it separately.

### Phase 4 — Enterprise Self-Hosted + Bare Metal

Enterprise licensing and Tier 4 bare metal target organisations with compliance requirements that preclude the cloud platform: financial services, government, regulated healthcare, defence contractors.

The built-in MLS, BPF LSM enforcement, SPIFFE identity model, and audit-grade eBPF telemetry address requirements that most cloud platforms handle poorly. FIPS crypto, HSM integration, and compliance policy packs (DORA, NIS2, SOC2, HIPAA) are enterprise licence features.

Air-gap deployment is a first-class scenario — one binary, embedded storage, no external dependencies makes Overdrive easier to deploy in classified environments than any platform with a dozen moving parts.

EU presence and Danish citizenship provides direct access to European government procurement — a market underserved by US-headquartered vendors and facing increasing data sovereignty requirements under DORA, NIS2, and GDPR.

---

## Revenue Streams Summary

| Stream | Model | Target |
|---|---|---|
| Cloud — Managed Overdrive | Per vCPU-hour + GB-hour | Platform teams |
| Cloud — Managed Workloads | Per vCPU-hour + GB-hour | Engineering teams |
| Cloud — Serverless WASM | Per invocation + GB-second | Developers, AI agents |
| Cloud — Bare Metal | Per node-hour | ML, HPC, latency-sensitive |
| Enterprise Licence | Annual per-node subscription | Regulated enterprises |
| Enterprise Support | Annual subscription, tiered SLA | Enterprise + self-hosted |
| Commercial Licence | Annual, negotiated | Embedding Overdrive in a product |

Cloud revenue scales with tenant consumption. Enterprise licence revenue is predictable and high-margin — no infrastructure cost against the contract value. Support contracts compound with the install base.

---

## Competitive Position

| | AWS Lambda | EKS / GKE | Nomad Cloud | Overdrive |
|---|---|---|---|---|
| Workload types | Functions only | Containers | Multi-type | All types |
| Cold start | 100ms–2s | N/A | N/A | ~1ms (WASM) |
| mTLS | Manual | Istio (complex) | Limited | Native, zero-config |
| AI agent security | None | None | None | Structural (sidecar) |
| Vendor lock-in | Complete | Moderate | Low | None (FSL → Apache 2.0) |
| Self-hosting option | No | Partial | Yes | Yes |
| LLM observability | No | No | No | Native |
| Multi-workload | No | No | Yes | Yes |

The gap that matters commercially: no existing platform handles AI agent workloads with structural security primitives. Prompt injection, credential exfiltration, and egress control are application-level concerns everywhere else. On Overdrive they are platform primitives. As AI agent adoption grows, this becomes a primary buying criterion.

---

## Summary

Overdrive has three commercial pillars:

**Cloud platform.** Run Overdrive on bare metal, sell tenants access to it at multiple abstraction levels — full cluster, managed workloads, serverless WASM, dedicated bare metal. The self-hosting architecture means tenants who outgrow one tier migrate upward without changing tooling. Billing is exact, funded by kernel-level telemetry.

**Enterprise self-hosted licensing.** Sell commercial licences and support contracts to regulated enterprises that cannot use the cloud platform. FIPS crypto, HSM integration, compliance policy packs, and air-gap tooling are enterprise-only features. High-margin, predictable annual revenue. The install base funds platform development.

**Source-available flywheel.** FSL's Competing Use restriction prevents hyperscaler commodity competition for the first two years of each release; the Apache 2.0 future grant guarantees the community a path to true open source on a published schedule. Community deployments validate the product and surface improvements. Enterprise customers contribute patches. The permissive internal-use grant shortens sales cycles — engineers already know the product before procurement gets involved, and legal review clears it without the copyleft objections AGPL would have triggered.

The business model is simple: absorb complexity, return simplicity, charge for the difference.
