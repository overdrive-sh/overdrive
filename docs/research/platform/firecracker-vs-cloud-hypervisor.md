# Firecracker vs Cloud Hypervisor: Why Overdrive diverges from Fly

**Date**: 2026-04-19
**Researcher**: Nova (nw-researcher)
**Question**: If Fly.io runs large-scale production on Firecracker, why does Overdrive choose Cloud Hypervisor as its sole VMM?
**Confidence**: High on feature matrix; Medium on Fly fork status; High on Fly Sprites counterfactual
**Sources**: 20 primary/authoritative

---

## Executive summary (the one-paragraph answer)

The whitepaper's headline thesis — *"Cloud Hypervisor is a strict superset of Firecracker's useful capabilities for Overdrive's workload mix"* — is **partially defensible but materially overstated in the current §6 table**. Upstream Firecracker as of 2025–2026 has closed three of the four capability gaps the whitepaper cites against it:

- **AArch64** has been production-supported for years
- **userfaultfd-based snapshot/restore with lazy memory paging** actually shipped in Firecracker *first* (Cloud Hypervisor added it later)
- **Memory hotplug via virtio-mem** has landed in Firecracker with 2024 fixes

The genuine, still-current gaps are narrower:

- **virtiofs / vhost-user-fs** — Firecracker's stance is "deferred pending threat-model review" (effectively unavailable)
- **CPU hotplug** — Firecracker issue #2609 is parked at low priority
- **Windows guests** — explicit Firecracker non-goal; Cloud Hypervisor supports it

The strongest argument for Cloud Hypervisor is therefore **not** that Firecracker can't do persistent microVMs — Fly.io's January 2026 **Sprites** launch proves it can at 300 ms checkpoint/restore on what appears to be vanilla-or-near-vanilla Firecracker — but that Overdrive specifically wants **cross-workload volume sharing via virtiofs between VMs, processes, and unikernels under one VMM**, which Firecracker will not provide.

Additionally, the whitepaper's claim that *"Cloud Hypervisor exposes a VMGenID device"* appears to be **unverified against upstream** and should be softened or removed.

---

## Feature matrix — corrected against primary sources (Apr 2026)

| Capability | Whitepaper §6 claim | Primary-source reality | Impact on §6 |
|---|---|---|---|
| Fast boot (~125–200 ms) | Both ✅ | Both ✅ | Correct |
| Full VM (arbitrary OS) | Firecracker ❌, CH ✅ | **Correct**. Firecracker is Linux/OSv only; Windows explicitly unsupported. CH supports Windows 10 / Server 2019. | Correct |
| virtiofs filesystem sharing | Firecracker ❌, CH ✅ | **Correct, upstream**. Firecracker issue #1180 is "deferred pending threat-model review"; CH ships `virtio-fs` via `virtiofsd` over vhost-user. | Correct |
| CPU hotplug | Firecracker ❌, CH ✅ | **Correct**. Firecracker issue #2609 ("Hot-plug vCPUs") is labeled *Priority: Low / Parked*. CH headlines CPU/memory/device hotplug. | Correct |
| Memory hotplug | Firecracker ❌, CH ✅ | **Wrong as stated**. Firecracker added virtio-mem memory hotplug; 2024 releases fixed KVM slot alignment bugs. CH implementation is more mature, but "❌" is inaccurate. | **Whitepaper should be revised** |
| AArch64 | Firecracker ❌, CH ✅ | **Wrong**. Firecracker FAQ: "Intel, AMD and 64-bit ARM processors are supported for production workloads." ARM milestone is closed. CH supports x86-64, AArch64, experimental riscv64. | **Whitepaper should be revised** |
| Snapshot/restore + userfaultfd lazy paging | implicit in §6 Persistent MicroVMs | **Firecracker shipped this first** and is the reference implementation. CH added `memory_restore_mode=ondemand` later. | Cannot be used as a differentiator |
| VMGenID | "Cloud Hypervisor exposes a VMGenID device" (§6) | **Unverified**. No VMGenID issue, PR, or docs reference found in the cloud-hypervisor repository. Spec originates from Microsoft/QEMU; QEMU and Hyper-V ship it; CH doesn't appear to. | **Whitepaper claim likely incorrect — remove or qualify** |
| Rust-native, no central daemon | Both ✅ | Both ✅ | Correct |

---

## The Fly Sprites counterfactual (Jan 2026)

Fly.io launched **Sprites** on 9 January 2026: "lightweight, persistent VMs" with 100 GB NVMe, ~300 ms checkpoint/restore, scale-to-zero on idle, targeted at AI coding agents.

By Fly's own documentation and DevClass reporting, **Sprites run on Firecracker**. Fly's public architecture still names upstream Firecracker (v1.7.0 deployment announcement), with no published fork. A community `m8sa/fly-firecracker` mirror shows minor divergences (virtio-block off-by-one; raising max virtio devices from 11 → 19), but this is not Fly's official repository.

Fly's own implementation post deliberately elides the hypervisor layer and emphasises their **storage stack** innovation:

- Data chunks on S3-compatible object storage
- Metadata in local SQLite made durable via Litestream
- NVMe as read-through cache

This is conceptually identical to the `overdrive-fs` design described in whitepaper §17.

**The existence of Sprites is a direct empirical refutation of any framing like "Firecracker cannot support persistent microVM workloads."**

---

## Why AWS's and Fly's constraints differ from Overdrive's (the reasoning chain)

### 1. Firecracker's charter is explicit about minimalism

The upstream `CHARTER.md` states four tenets, one of which is:

> *"Minimalist in Features: If it's not clearly required for our mission, we won't build it."*

Mission = "secure, multi-tenant, minimal-overhead execution of container and function workloads" — i.e., Lambda and Fargate. The NSDI 2020 paper confirms: Firecracker "stripped out" QEMU device support — *"USB, display, sound… a Lambda would never use."*

**This is a feature of the project, not a limitation to apologise for.**

### 2. Fly's workload mix overlaps with Firecracker's sweet spot

Fly Machines are user-owned but still Linux-centric, short-to-medium-lived, and don't need cross-workload volume sharing in a single VMM. When Fly needed persistence for Sprites, they did *not* add virtiofs to Firecracker — they built a **storage layer above the VMM** (block device + object-backed COW + metadata sidecar).

This is the same architectural shape as `overdrive-fs` in §17. The VMM stayed minimal; the durability moved up the stack.

### 3. Overdrive's workload mix is genuinely broader than Fly's or AWS Lambda's

Whitepaper §6 explicitly lists processes, microVMs, full VMs, unikernels, and WASM under one control plane with one identity and one dataplane. The specific *architectural* requirements:

- **virtiofs between a VM and a process workload** for cross-driver shared volumes (§6: *"VM workload writes to `/shared-volume` (virtiofs mount), Process workload reads `/shared-volume` (bind mount)"*). Firecracker's `virtio-block`-only model cannot express a mount that a non-VM workload on the same host also sees.
- **Full-VM workloads including Windows** for future enterprise lift-and-shift or specialised device emulation.
- **Unikernel + virtiofs** for Unikraft integration (Unikraft added virtiofs in mainline December 2025).

These three needs are genuinely out of Firecracker's declared mission space. They are *not* cargo-culted features.

### 4. Fly Sprites shows Firecracker *is* enough for the persistent-microVM use case alone

If Overdrive's only stateful workload were AI coding agents, the Firecracker + object-backed FS approach Fly uses would be sufficient. **The case for Cloud Hypervisor collapses to "we also want full VMs, Windows guests, and virtiofs-shared volumes between drivers."**

---

## Honest trade-offs of picking Cloud Hypervisor

| Axis | Cost of the choice |
|---|---|
| Attack surface | CH is ~50k LoC Rust vs Firecracker's smaller minimalist codebase. Still vastly smaller than QEMU (~2M LoC C), but strictly larger than Firecracker. Northflank's 2026 review notes this explicitly. |
| Cold-boot time | CH is typically ~200 ms vs Firecracker's ~125 ms — the whitepaper already accepts this 75 ms cost. |
| Memory footprint | Firecracker's "<5 MiB footprint" claim (NSDI) is harder to match in CH because the device model is richer. Not quantified in upstream CH docs. |
| Production deployment scale | Firecracker backs **all of AWS Lambda and Fargate** — trillions of invocations and the largest single production deployment of any Rust VMM. CH production deployments include Kata Containers (one of several supported backends) and Intel/OpenStack contexts; no publicly claimed Lambda-scale deployment. Overdrive accepts younger battle-testing. |
| Ecosystem/tooling | Firecracker has a much larger ecosystem (firecracker-containerd, Kata Firecracker runtime, Fly, E2B, Ubicloud, Fireactions). CH ecosystem is smaller. |
| Upstream velocity | Both projects are active; CH has a broader feature surface and thus a wider change footprint. |

None of these are disqualifying for Overdrive, but **§20 ("Efficiency Comparison") currently does not acknowledge them**. At minimum, the Overdrive position should be honest that Firecracker is more minimal and more production-proven at scale, and the Cloud Hypervisor choice is a principled acceptance of that extra surface area in exchange for feature breadth.

---

## Other production orchestrator signals (industry check)

- **Kata Containers**: supports *both* Firecracker and Cloud Hypervisor as alternative backends. Kata's own docs state CH *"provides better compatibility [than Firecracker] at the expense of exposing additional devices: **file system sharing and direct device assignment**."* **This is the clearest independent third-party endorsement of the exact trade-off Overdrive is making.**
- **AWS**: Firecracker for Lambda/Fargate, QEMU for EC2 — the two-VMM model Overdrive explicitly rejects.
- **Fly.io**: Firecracker for Machines and Sprites — proves Firecracker + external storage can support persistence. Does *not* need virtiofs because all Fly workloads are VMs.
- **Ubicloud**: Firecracker + custom. Same minimal-surface reasoning as AWS.
- **E2B / Northflank / Modal**: Firecracker-based for AI sandbox use cases. None mention Cloud Hypervisor as an alternative they've tried and rejected.

The industry shift Overdrive rides is real but modest — **Cloud Hypervisor is a respected second option for workloads that need CH features, not a displacing default**.

---

## Recommendations for whitepaper revision

1. **Revise §6 feature matrix.** At minimum flip Firecracker's AArch64 cell to ✅ and memory hotplug cell to partial/✅ with a note about virtio-mem. The table loses some rhetorical force but gains defensibility.

2. **Remove or soften the VMGenID claim.** *"Cloud Hypervisor exposes a VMGenID device"* is not supported by any primary source found; no issue, PR, or docs reference exists in upstream. Either cite a specific commit/PR, soften to "can be implemented via a userspace shim," or drop the VMGenID mention. The entropy-reuse concern is real; the VMGenID solution needs to actually exist in CH to be used.

3. **Reframe the core argument** from *"Firecracker can't do these things"* to *"Firecracker's charter deliberately excludes virtiofs, Windows guests, and CPU hotplug, and Overdrive's multi-driver workload mix requires all three."* This is stronger because it aligns with Firecracker's own stated position and survives the "but Fly Sprites exists" objection.

4. **Acknowledge Fly Sprites directly in §6.** A one-paragraph footnote:

   > *"Persistent microVMs on Firecracker are demonstrably viable — Fly.io's Sprites (Jan 2026) ships this at 300 ms checkpoint/restore using a custom object-backed storage layer analogous to `overdrive-fs`. Overdrive chooses Cloud Hypervisor not because Firecracker cannot host persistent VMs, but because Overdrive unifies VM and non-VM workloads in one dataplane and requires virtiofs for cross-driver volume sharing."*

   This turns a latent objection into a strength.

5. **Acknowledge the trade-offs honestly in §20.** Cloud Hypervisor is larger and less battle-tested than Firecracker. The whitepaper already accepts a 75 ms cold-boot cost; it should also acknowledge the larger attack surface and smaller production-deployment evidence base.

---

## Knowledge gaps (unresolved)

1. **Does Fly.io run a private Firecracker fork?** Primary sources are ambiguous. Fly's v1.7.0 blog post reads like they ship upstream; the community `m8sa/fly-firecracker` mirror shows small divergences but is not Fly's official repo. No direct "here is our fork" or "we ship vanilla" statement from Thomas Ptacek, Kurt Mackey, or Fly engineering has been located. **Follow-up**: a direct read of `fly.io/blog` archive or a `gh api` query against `superfly/*` repos could resolve this.

2. **Exact Firecracker version that added virtio-mem.** PR #5534 is referenced but the specific release version (1.14 vs earlier) was not confirmed from release notes.

3. **Cloud Hypervisor VMGenID**: Absence not fully provable without a direct repo grep for `vmgenid` in cloud-hypervisor source.

4. **Fly's opinion of Firecracker's limits.** No blog post by Ptacek / Mackey / Fly engineering specifically enumerating Firecracker features they wish existed. The Sprites announcement studiously avoids the topic.

---

## Full citations

1. [Firecracker CHARTER](https://github.com/firecracker-microvm/firecracker/blob/main/CHARTER.md) — **High authority** (project charter)
2. [Firecracker FAQ](https://github.com/firecracker-microvm/firecracker/blob/main/FAQ.md) — **High authority** (official)
3. [Firecracker README](https://github.com/firecracker-microvm/firecracker) — **High authority** (official)
4. [Firecracker issue #1180: Host Filesystem Sharing](https://github.com/firecracker-microvm/firecracker/issues/1180) — **High authority** (maintainers' position on virtiofs)
5. [Firecracker issue #2609: Hot-plug vCPUs](https://github.com/firecracker-microvm/firecracker/issues/2609) — **High authority** (roadmap status)
6. [Firecracker memory-hotplug docs](https://github.com/firecracker-microvm/firecracker/blob/main/docs/memory-hotplug.md) — **High authority** (virtio-mem status)
7. [Firecracker snapshot / userfaultfd docs](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/handling-page-faults-on-snapshot-resume.md) — **High authority**
8. [Firecracker NSDI 2020 paper (Agache et al.)](https://www.usenix.org/system/files/nsdi20-paper-agache.pdf) — **High authority** (peer-reviewed design rationale)
9. [Cloud Hypervisor README](https://github.com/cloud-hypervisor/cloud-hypervisor) — **High authority** (official feature list)
10. [Cloud Hypervisor virtio-fs docs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/fs.md) — **High authority** (virtiofsd production setup)
11. [Cloud Hypervisor snapshot/restore docs](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/snapshot_restore.md) — **High authority** (userfaultfd mode)
12. [Kata Containers hypervisors docs](https://github.com/kata-containers/kata-containers/blob/main/docs/hypervisors.md) — **High authority** (independent Kata comparison)
13. [Fly.io — Fly Machines blog](https://fly.io/blog/fly-machines/) — **High authority**
14. [Fly.io — We shipped Firecracker v1.7.0](https://community.fly.io/t/we-shipped-firecracker-v1-7-0/20140) — **High authority** (Fly production VMM version)
15. [Fly.io — Code And Let Live (Sprites launch)](https://fly.io/blog/code-and-let-live/) — **High authority** (Jan 2026 Sprites official)
16. [Fly.io — The Design & Implementation of Sprites](https://fly.io/blog/design-and-implementation/) — **High authority** (storage stack)
17. [Simon Willison — Fly Sprites.dev analysis](https://simonwillison.net/2026/Jan/9/sprites-dev/) — **Medium-High** (respected secondary)
18. [DevClass — Fly Sprites coverage](https://devclass.com/2026/01/13/fly-io-introduces-sprites-lightweight-persistent-vms-to-isolate-agentic-ai/) — **Medium-High** (industry press)
19. [Northflank — Cloud Hypervisor vs Firecracker 2026](https://northflank.com/blog/what-is-aws-firecracker) — **Medium** (vendor blog, cross-checked)
20. [Micah Lerner — Firecracker NSDI paper summary](https://www.micahlerner.com/2021/06/17/firecracker-lightweight-virtualization-for-serverless-applications.html) — **Medium-High** (widely-cited paper digest)
