# Right-Sizing Approaches Across Cloud Providers and Orchestrators

**Date**: 2026-04-19
**Researcher**: Nova (nw-researcher)
**Question**: How do cloud providers and orchestrators actually achieve CPU/memory right-sizing in production — live hotplug, or restart-based replacement?
**Confidence**: High
**Sources**: 22 primary/authoritative

---

## The one-paragraph answer

The industry overwhelmingly uses **stop-and-replace, not live hotplug**, to change VM CPU/memory. Every major IaaS (AWS EC2, GCP, Azure, DigitalOcean, Hetzner, Fly.io) requires the VM to be stopped or accepts a forced restart to change size; only Azure and a few VMware-derived platforms offer partial in-place resize, and even Azure's docs explicitly warn it *"should be considered a disruptive operation."* However, **at the container/cgroup layer — not the VMM layer — the industry has converged on live resize**: Kubernetes in-place pod resize reached stable GA in v1.35 (December 2025) via cgroup writes through the CRI `UpdateContainerResources` API, and KubeVirt offers virtio-mem-backed memory hotplug.

**Consequence for Helios §14**: live cgroup resize for **process** workloads has always been a kernel capability and does not require Cloud Hypervisor. Live *VM* CPU/memory hotplug is a real but narrow niche — KubeVirt, Harvester, VMware vSphere — and the ceiling on its value is capped by well-documented guest-OS side effects. The honest read: §14's live-hotplug-for-VMs promise is a **differentiator in a small but real niche** (persistent microVMs that must not restart), not an industry norm.

---

## Resize mechanism by platform

| Platform | Running resize? | Mechanism |
|---|---|---|
| **AWS EC2** | No | Stop → change instance type → Start. *"You must stop your instance before you can change its instance type."* |
| **GCP Compute Engine** | No | *"You cannot change the machine type of a running VM."* Must stop, edit, restart. |
| **Azure VMs** | Yes (but restarts) | *"Changing the size of a running VM will cause it to restart... should be considered a disruptive operation."* |
| **DigitalOcean** | No | *"You need to power it off."* ~1 min per GB of used disk. |
| **Hetzner Cloud** | No | *"If your server is not powered off, the rescale button will be inactive."* |
| **Fly.io Machines** | No | `fly machine update --vm-memory`: *"Flyctl restarts the Machines to use the new setting."* |
| **AWS Lambda** | N/A (per-invocation) | Memory is static function config (128 MB–10,240 MB); no mid-invocation adjust. |
| **Google Cloud Run** | N/A (per-revision) | Memory/CPU set on revision. No live adjust. |
| **Azure Functions Flex** | N/A (per-app) | Fixed instance sizes (512 / 2,048 / 4,096 MB). |
| **Cloudflare Workers** | N/A (per-invocation) | 128 MB per isolate fixed. |
| **Kubernetes (in-place pod resize)** | **Yes — stable in v1.35 (Dec 2025)** | Kubelet → CRI `UpdateContainerResources` → containerd/CRI-O → cgroup write. |
| **Nomad** | No | Resource changes trigger *destructive updates* — new allocations replace old. |
| **ECS / Fargate** | No | Update task definition → deploy new tasks → drain old. |
| **KubeVirt** | Yes (via live migration) | Memory hotplug in v1.1+, Linux ≥5.8 guest required. Implemented via live migration of the VMI. |
| **VMware vSphere Hot-Add** | Yes | Hot-add only (no hot-remove without poweroff). **Disables vNUMA** (perf regression on ≥8 vCPU). |
| **Cloud Hypervisor (VMM)** | Yes | CPU hotplug (ACPI), memory hotplug via virtio-mem or ACPI, virtio device hotplug. |
| **Firecracker** | Partial | virtio-mem memory hotplug landed with 2024 fixes; **CPU hotplug parked (issue #2609)**. |

---

## Key findings with evidence

### Finding 1 — Public cloud IaaS universally requires stop/start for VM resize
- **AWS**: *"You must stop your instance before you can change its instance type."*
- **GCP**: *"You can only change the machine type of a stopped VM."*
- **Azure**: *"Changing the size of a running VM will cause it to restart... disruptive."*
- **DO, Hetzner**: Power-off required.

**Confidence**: High. Three hyperscalers plus two budget providers all agree: restart-based is the industry norm.

### Finding 2 — Fly.io explicitly restarts for size changes
*"Flyctl restarts the Machines to use the new setting"* when using `fly scale vm`. This is driven by Firecracker constraint (no CPU hotplug), not principle — but no Fly engineering post defends restart-based over hotplug.

### Finding 3 — Kubernetes reached GA for in-place container resize in v1.35 (Dec 2025)
- Implementation: *"The kubelet uses the CRI to tell containerd or CRI-O... The runtime adjusts cgroups accordingly, no restart."*
- Container PID 1 continues; pod UID preserved.
- Memory decrease now permitted at GA.
- VPA's `InPlaceOrRecreate` mode graduated to Beta in 1.35.
- **Mechanism**: CRI `UpdateContainerResources` → cgroup v2 write. Kernel capability, not a new feature.

### Finding 4 — Nomad, ECS, Fargate: all restart-based
Nomad resource changes are destructive updates. ECS/Fargate requires task definition update and redeploy.

### Finding 5 — Serverless platforms: resources are static deployment config
Lambda, Cloud Run, Workers, Azure Functions all set memory/CPU as part of the deployment unit. No live adjust.

### Finding 6 — The live VM hotplug niche exists but is small and opinionated
**Who does it**: VMware vSphere (hot-add only, vNUMA penalty), KubeVirt (via live migration, Linux ≥5.8), Harvester, Proxmox.

**Known gotchas**:
- vSphere: CPU hot-add disables vNUMA (perf regression on ≥8 vCPU VMs)
- VMware: no hot-remove without poweroff
- virtio-mem: Linux guest ≥5.8 only; Windows tech-preview
- All platforms: guest kernel must recognise new CPU/memory online

**Production documentation is narrow**: no primary-source evidence that Netflix, Stripe, or Cloudflare publicly discuss VM-level live hotplug as a right-sizing strategy.

### Finding 7 — K8s VPA + in-place is the closest industry analogue to Helios §14
VPA's `InPlaceOrRecreate` mode patches container resource spec via `/resize` subresource. If node has capacity: live cgroup write. If not: evict-and-recreate.

**What's missing vs Helios §14**: K8s VPA polls metrics-server at fixed intervals (minutes). It cannot *prevent* OOM — it reacts to metrics. Helios §14 describes eBPF-observed pressure with **pre-OOM live resize**, which is structurally different (sub-second reaction, in-kernel signal). No K8s VPA equivalent for eBPF-driven OOM-predictive resize was found.

### Finding 8 — For process/container workloads, cgroup live resize is a kernel feature, not a VMM feature
K8s's in-place resize is `echo <value> > /sys/fs/cgroup/.../memory.max`. It has been possible since cgroups v1 and is trivial in cgroups v2.

**Consequence**: Process driver right-sizing on Helios does not depend on Cloud Hypervisor. Any claim that §14's process-workload right-sizing requires Cloud Hypervisor is confused — it requires cgroups v2.

---

## Honest analysis — does §14's live-hotplug claim hold up?

| §14 claim | Defensible? | Notes |
|---|---|---|
| Live cgroup resize for process workloads | ✅ Fully | Kernel feature. Does not require Cloud Hypervisor. |
| eBPF-observed pressure → pre-OOM cgroup write | ✅ Plausible, real novelty | K8s VPA does not use eBPF pressure signals; they poll metrics-server. The eBPF-driven path is a genuine Helios differentiator for process workloads. |
| Live VM CPU/memory hotplug via Cloud Hypervisor | ⚠️ Real but narrow | CH supports it. KubeVirt and Harvester do it. Industry IaaS does not. Feature exists; question is whether anyone needs it. |
| *"This is not possible with Firecracker"* (CPU) | ✅ Correct | Firecracker CPU hotplug parked (issue #2609). |
| *"This is not possible with Firecracker"* (memory) | ❌ Already outdated | Firecracker has virtio-mem memory hotplug since 2024. |
| *"Uniform across all workload types"* | ⚠️ Overstated | Uniform mechanism (hotplug or cgroup write), but gotchas differ sharply. VM hotplug requires guest kernel cooperation; cgroup write does not. |

### Does Helios find a real gap? Yes — but a narrow one:

1. **Process/container/WASM workloads**: live cgroup resize is table stakes. K8s just reached GA on it. This part of §14 is defensible but not a differentiator — becoming baseline.
2. **VM workloads**: live CPU/memory hotplug is a real, measurable differentiator against Fly (Firecracker-bound) and all public IaaS (restart-based). Closest analogues: KubeVirt/Harvester.
3. **The persistent microVM use case** (§6) is where this matters most: *"resize a long-lived AI coding agent VM without snapshotting and restoring."* For ephemeral microVMs, restart-based is fine (Fly Sprites shows 300ms restore). For long-lived stateful VMs, hotplug avoids the snapshot dance.

### Is restart-based "fine"?
The industry's answer: **almost always yes, with two exceptions**:
- Stateful workloads where snapshot/restore cost is high (Helios's persistent microVM target)
- Workloads where even a few hundred ms of downtime is a hard violation (rare)

### Verdict on §14's load-bearing status
- The **process/container** half is genuinely cgroup-driven and does not depend on Cloud Hypervisor. If Helios dropped VM workloads entirely, §14 would still work for processes. Defensible but converging with industry (K8s 1.35).
- The **VM hotplug** half is the actual Cloud-Hypervisor-dependent claim. Real, narrow, primary beneficiary is persistent microVMs.

---

## Recommendations for whitepaper revision

1. **Separate process from VM in §14's prose.** Process right-sizing is a cgroup operation — works regardless of which VMM is installed. Only the VM right-sizing claim is Cloud-Hypervisor-dependent.

2. **Soften the uniqueness rhetoric.** KubeVirt (via live migration) and VMware (via hot-add) already do VM-level in-place resize. K8s 1.35 GA (Dec 2025) reached in-place for containers. The Helios novelty is:
   - **Unified control plane across both process and VM**
   - **eBPF-observed pressure signal driving it pre-OOM**
   — not the mechanical hotplug itself.

3. **Acknowledge restart-based is usually fine.** Most production VM resize *is* done by restart, and this works. Helios's differentiator is the long-lived stateful VM case (persistent microVM, §6) where restart is expensive. Explicitly tie §14 to that case.

4. **Correct the Firecracker-memory claim.** Firecracker has virtio-mem memory hotplug since 2024. The "This is not possible with Firecracker" framing only survives for **CPU hotplug**.

The §14 live-hotplug claim is **load-bearing for persistent microVMs and decorative for everything else**. Make §14 honest about which is which and it survives; keep the current framing and readers who know K8s 1.35 or KubeVirt will push back.

---

## Knowledge gaps

1. **No public evidence of production eBPF-driven right-sizing beyond monitoring**. Netflix/Stripe/Cloudflare have published on eBPF but not on "eBPF pressure → cgroup adjust before OOM" as a production loop. Helios's pattern appears genuinely novel — which sometimes means "tried and abandoned."
2. **VMware production adoption of hot-add**. Operators often keep it off because of vNUMA regression. No large-scale survey found.
3. **Fly.io's stated position on hotplug vs restart** — no published defense; choice appears driven by Firecracker constraint.
4. **Memory-decrease support across platforms**. K8s 1.35 explicitly lifted the ban. KubeVirt's live-migration-based memory hotplug unclear on shrink.

---

## Full citations

1. [AWS EC2 — Change instance type](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/change-instance-type-of-ebs-backed-instance.html)
2. [AWS EC2 — Instance type changes](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/ec2-instance-resize.html)
3. [GCP Compute Engine — Edit machine type](https://docs.cloud.google.com/compute/docs/instances/changing-machine-type-of-stopped-instance)
4. [Azure VM — Resize](https://learn.microsoft.com/en-us/azure/virtual-machines/sizes/resize-vm)
5. [Fly.io — Scale Machine CPU and RAM](https://fly.io/docs/launch/scale-machine/)
6. [Fly.io — Machine Sizing](https://fly.io/docs/machines/guides-examples/machine-sizing/)
7. [DigitalOcean — Resize Droplets](https://docs.digitalocean.com/products/droplets/how-to/resize/)
8. [Hetzner — Cloud servers FAQ](https://docs.hetzner.com/cloud/servers/faq/)
9. [AWS Lambda — Configure memory](https://docs.aws.amazon.com/lambda/latest/dg/configuration-memory.html)
10. [Cloud Run — Memory limits](https://docs.cloud.google.com/run/docs/configuring/services/memory-limits)
11. [Azure Functions Flex Consumption](https://learn.microsoft.com/en-us/azure/azure-functions/flex-consumption-plan)
12. [Cloudflare Workers — Limits](https://developers.cloudflare.com/workers/platform/limits/)
13. [K8s 1.35: In-Place Pod Resize GA](https://kubernetes.io/blog/2025/12/19/kubernetes-v1-35-in-place-pod-resize-ga/)
14. [K8s 1.33: In-Place Pod Resize Beta](https://kubernetes.io/blog/2025/05/16/kubernetes-v1-33-in-place-pod-resize-beta/)
15. [KEP-1287 — In-Place Update of Pod Resources](https://github.com/kubernetes/enhancements/blob/master/keps/sig-node/1287-in-place-update-pod-resources/README.md)
16. [Nomad — update block](https://developer.hashicorp.com/nomad/docs/job-specification/update)
17. [Nomad — Dynamic Application Sizing](https://developer.hashicorp.com/nomad/tutorials/autoscaler/dynamic-application-sizing-concepts)
18. [AWS ECS — Task definition parameters](https://docs.aws.amazon.com/AmazonECS/latest/developerguide/task_definition_parameters.html)
19. [KubeVirt — Memory Hotplug](https://kubevirt.io/user-guide/compute/memory_hotplug/)
20. [Cloud Hypervisor — Device Hotplug](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/hotplug.md)
21. [Firecracker — Memory Hotplug](https://github.com/firecracker-microvm/firecracker/blob/main/docs/memory-hotplug.md)
22. [Broadcom KB 321931 — vNUMA disabled if vCPU hotplug enabled](https://knowledge.broadcom.com/external/article/321931/vnuma-is-disabled-if-vcpu-hotplug-is-ena.html)
