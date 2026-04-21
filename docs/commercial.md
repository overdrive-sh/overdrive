# Overdrive: Commercial Model

**Version 0.3 — Draft**

---

## Overview

Overdrive is an open-source developer platform — functions, durable objects, sandboxed agents (persistent microVMs for AI coding agents, long-running autonomous workers, databases, CI runners), queues, cron, KV, per-workload SQL, and S3-compatible object storage — running on infrastructure you own. It is source-available under the Functional Source License (FSL-1.1-ALv2); every release converts to Apache 2.0 two years after publication under the irrevocable future-grant built into the licence.

The commercial opportunity is twofold. First, the developer-platform experience most teams today rent from Cloudflare, Vercel, or Fly.io is a cleanly-scoped purchase that every engineering team understands — and every EU regulatory regime (DORA, NIS2, sovereign-cloud tenders) prefers to operate outside a US hyperscaler. Second, the operational complexity that makes a developer platform *possible* is what Kubernetes shops already pay through the nose to assemble: etcd, CNI plugins, cert-manager, a service mesh, an ingress controller, Prometheus, alertmanager, an identity plane, a policy engine, and a dozen other components each deployed, upgraded, and debugged independently. Overdrive collapses both halves into one binary.

The cloud platform business charges for not having to operate that stack. The enterprise self-hosted business charges for running it on your own hardware with warranty, compliance, and support. The developer-platform positioning is the front door; the orchestrator layer underneath is the credibility.

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

### Tier 0 — Self-hosted (free)

The full source-available binary under FSL-1.1-ALv2. Functions, durable objects, sandboxed agents, queues, cron, KV, per-workload SQL, object storage — the complete developer-platform surface, self-hosted on one box, one cluster, or a fleet. No tier gating, no feature fences. Apache 2.0 future grant at the two-year anniversary.

Target: the community flywheel — individual developers, side projects, OSS adopters, regional clouds trialling the platform, teams validating the stack before going managed or enterprise. The "download and try it" on-ramp for every downstream tier.

Billing model: none. This is the flywheel.

### Tier 1 — Serverless &amp; Managed Workloads (flagship)

The developer-platform front door on our cloud. Deploy a function, a durable object, a sandboxed agent (AI coding agents, autonomous workers), a stateful VM (Postgres, CI runners, dev environments), or a scheduled job; the platform schedules it within a tenant namespace with kernel-isolated primitives. Sub-10 ms cold start on WASM; tens-of-ms resume on persistent microVMs. Scale-to-zero. The credential proxy and content inspector ship by default — built for the AI-agent workloads no one else has a story for.

Target: developers who want Cloudflare / Vercel / Fly.io economics without hyperscaler lock-in; teams subject to EU data-residency or DORA / NIS2 regimes that preclude US-cloud hosting; AI-agent teams needing egress control; regional clouds building a developer offering on top of Overdrive.

Billing model: per invocation + per GB-second for function / durable-object workloads; per vCPU-hour + per GB-hour for containerised workloads. Minimum billing unit is one invocation.

### Tier 2 — Managed Overdrive

Full Overdrive cluster as a service for platform teams running their own internal developer platform. Tenant gets a control plane and worker node pool; they submit jobs, define policies, and operate their cluster through the Overdrive API and CLI. The infrastructure layer handles the underlying hardware, networking, and physical security.

Target: platform-engineering teams building internal developer platforms, companies migrating off self-managed Kubernetes, organisations that want to re-sell an Overdrive-based developer platform to their own tenants.

Billing model: per vCPU-hour + per GB-hour of memory consumed by the tenant cluster (including control plane overhead).

### Tier 3 — Bare Metal Dedicated

Tenant gets dedicated physical nodes within the managed platform. Full hardware performance, no VM overhead, still managed by the Overdrive control plane — node provisioning, OS management, eBPF dataplane, mTLS, observability all included. Tenant workloads run directly on the hardware.

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

This density advantage is the primary margin driver for Tiers 1 and 2.

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

### Phase 1 — Open Source Developer-Platform Traction

Ship the source-available binary with the developer-platform surface as the public pitch: `overdrive deploy` a function, attach a durable object, stand up a sandboxed agent or a Postgres workload, wire a queue and a cron, use KV and per-workload SQL, front it with a stable HTTPS URL. The "everything Cloudflare does, on infrastructure you own" framing is the top-of-funnel. The orchestrator layer underneath is the credibility, not the lede.

Channels: Hacker News technical deep-dives, Rust community (TWiR, users.rust-lang.org), r/selfhosted, Wasm / eBPF conferences, a public demo of `overdrive dev` with hot-reload against the full primitive surface. Parallel presence at KubeCon / eBPF Summit for the platform-engineering sub-audience.

Measure: GitHub stars, SDK npm / crates.io / PyPI installs, first-hour activation on `overdrive deploy`, self-hosted deployment count, enterprise inbound.

### Phase 2 — Cloud Platform (Serverless &amp; Managed Workloads)

Launch Tier 1 on owned bare metal. Primary acquisition: developers who hit the free Tier 0 on a single box and need to scale past it — zero switching cost because the binary is the same. Secondary acquisition: teams priced out of or regulatorily blocked from Cloudflare / Vercel / Fly.io who need a credible sovereign-cloud alternative.

The credential proxy, content-inspection sidecar, and domain-allowlist model solve prompt-injection and credential-exfiltration problems every team building production AI agents faces — position the AI-agent story as a specific wedge within Tier 1 rather than a separate product.

### Phase 3 — Managed Overdrive for Platform Teams

Launch Tier 2. Target: platform-engineering teams migrating off EKS / GKE who want the developer-platform experience they already give their internal developers on an internal PaaS, without running the four-failure-domain Kubernetes stack underneath.

This phase is also when regional clouds and sovereign-cloud providers become first-class customers — they buy Managed Overdrive (Tier 2) as the foundation on which they re-sell Tier 1 experiences to their own end-users. The €180M EU sovereign-cloud tender and the wider DORA / NIS2 compliance environment make this a load-bearing channel rather than an afterthought.

### Phase 4 — Enterprise Self-Hosted + Bare Metal

Enterprise licensing and Tier 3 bare metal target organisations with compliance requirements that preclude the cloud platform: financial services, government, regulated healthcare, defence contractors.

The built-in mTLS, BPF LSM enforcement, SPIFFE identity model, and audit-grade eBPF telemetry address requirements that most cloud platforms handle poorly. FIPS crypto, HSM integration, and compliance policy packs (DORA, NIS2, SOC2, HIPAA) are enterprise licence features.

Air-gap deployment is a first-class scenario — one binary, embedded storage, no external dependencies makes Overdrive easier to deploy in classified environments than any platform with a dozen moving parts.

EU presence and Danish citizenship provides direct access to European government procurement — a market underserved by US-headquartered vendors and facing increasing data sovereignty requirements under DORA, NIS2, and GDPR.

---

## Revenue Streams Summary

| Stream | Model | Target |
|---|---|---|
| Cloud — Serverless &amp; Managed Workloads (Tier 1) | Per invocation + GB-second, or per vCPU-hour + GB-hour | Developers, AI-agent teams, sovereign-cloud end-users |
| Cloud — Managed Overdrive (Tier 2) | Per vCPU-hour + GB-hour | Platform teams, regional clouds re-selling the platform |
| Cloud — Bare Metal (Tier 3) | Per node-hour | ML, HPC, latency-sensitive |
| Enterprise Licence | Annual per-node subscription | Regulated enterprises |
| Enterprise Support | Annual subscription, tiered SLA | Enterprise + self-hosted |
| Commercial Licence | Annual, negotiated | Embedding Overdrive in a product |

Cloud revenue scales with tenant consumption — Tier 1 scales horizontally with developer count, Tier 2 scales with platform-team count, Tier 3 scales with regulated workload count. Enterprise licence revenue is predictable and high-margin — no infrastructure cost against the contract value. Support contracts compound with the install base.

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

Overdrive has three commercial pillars, all fed by the same developer-platform front door:

**Cloud platform.** Run Overdrive on bare metal, sell tenants access at four abstraction levels — free self-hosted (Tier 0) funnels into serverless-and-managed-workloads (Tier 1), which funnels into full managed clusters (Tier 2) and dedicated bare metal (Tier 3) for tenants who outgrow shared infrastructure. The self-hosting architecture means tenants migrate upward without changing tooling. Billing is exact, funded by kernel-level telemetry.

**Enterprise self-hosted licensing.** Sell commercial licences and support contracts to regulated enterprises that cannot use the cloud platform. FIPS crypto, HSM integration, compliance policy packs, and air-gap tooling are enterprise-only features. High-margin, predictable annual revenue. The install base funds platform development.

**Source-available flywheel.** FSL's Competing Use restriction prevents hyperscaler commodity competition for the first two years of each release; the Apache 2.0 future grant guarantees the community a path to true open source on a published schedule. Developer-platform community adoption (the largest audience by volume) validates the product at the primitive surface; enterprise customers contribute patches at the infrastructure layer; regional clouds adopt the managed tier to re-sell on top. The permissive internal-use grant shortens sales cycles — engineers already know the product before procurement gets involved, and legal review clears it without the copyleft objections AGPL would have triggered.

The business model is simple: absorb complexity, return simplicity, charge for the difference. The developer-platform framing is how the difference becomes visible.
