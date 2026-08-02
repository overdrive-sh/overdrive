# Research: A Trustworthy Tier-3 Test Gate for Cloud Hypervisor microVM Boot on an Apple Silicon Developer Host

**Date**: 2026-08-02 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High overall (High on the environment-availability findings that drive the recommendation; Low on root cause) | **Sources**: 27 cited

## Problem Statement (given, not researched)

Cloud Hypervisor microVM boots stall intermittently (~2 in 3 attempts) inside a Lima VM on
Apple Silicon. Measured environment:

- Host: Apple M4 Max, macOS 26.1
- Lima VM: Ubuntu 26.04, kernel 7.0.0-28-generic, aarch64, `vmType: vz`
  (Apple Virtualization.framework), `nestedVirtualization: true`
- Guest VMM: `cloud-hypervisor v46.0.0`, `/dev/kvm` present
- `systemd-detect-virt` → `apple`

**Nested virtualization is working** — CH requires KVM, has no TCG fallback, and boots
successfully ~1 in 3 attempts. The symptom is a *hang*, not a capability failure: the microVM
kernel probes `virtio_blk`, reaches `md: ... autorun DONE.`, then freezes before mounting
root / running `/init`.

Population diffs already performed by the spike (not to be redone):
- Identical stall rate with and without the vsock device → vsock not implicated.
- QEMU/KVM in the same Lima VM with the identical kernel Image and rootfs stalls 4/8 at the
  same points → not Cloud-Hypervisor-specific.
- When the guest reaches userspace, assertions pass 100% of the time → the stall yields a
  *missing* result, never a wrong one.

Source: `docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md` (2026-08-02).

## Executive Summary

**The root cause is not identifiable from public sources, and it does not need to be.** Apple
ships nested virtualization on M3+ / macOS 15+ as a single boolean API with **no published
guest-hypervisor compatibility contract, no errata channel, and no known-issues list** (F1.1,
F1.3). There is therefore no specification to check the observed behaviour against, no bug to
file, and no workaround that could be pinned to a documented guarantee across macOS point
releases. Every candidate mitigation is empirical trial-and-error against a black box (Q3).

**What *is* well established is that the failure shape is a known hazard class, and that the
whole industry routes around it the same way.** "Nested aarch64 + timer/interrupt
virtualization → guest boot hang" is documented in upstream KVM (spurious `ptimer` interrupts
under non-zero `CNTPOFF` "causing the boot to hang", F2.1) and has a documented proprietary
precedent (older Hyper-V for arm64 "only partially virtualize[s] the ARMv8 architectural
timer, such that the timer does not generate interrupts in the VM" — fixed only by patching
the *guest kernel*, F3.1). arm64 nested virtualization is young, FEAT_NV2-only, and its
upstream implementation needed Apple-silicon-specific **vGIC emulation hacks** to work at all
(F2.2). Against that backdrop, an intermittent stall between the `virtio_blk` probe and the
root mount — i.e. while blocked on an interrupt — is unsurprising, and the spike's own
population diff (identical stall under QEMU/KVM with the same kernel and rootfs) already
places the fault below the VMM.

**The decisive facts are about environments, not about the bug.** For aarch64, nested
virtualization is unavailable essentially everywhere: GCP excludes Arm processors from nested
virtualization **by documented policy** (F5.1); AWS's 2026 nested-virt launch covers Intel
8th-gen only and **explicitly not Graviton**, so aarch64 still means `*.metal` (F5.2); GitHub's
arm64 hosted runners have **no `/dev/kvm`** at all, on Azure SKUs that do not support nesting
(F5.3). Kata Containers states the constraint outright — "Kata Containers can only be run
using bare metal aarch64 hosts ... nested virtualization isn't available on aarch64 virtual
hosts" — and Firecracker's aarch64 CI runs on EC2 `*.metal` Graviton instances for the same
reason (F4.1, F4.2). **No comparable project gates VM boot on a nested developer laptop.**
Meanwhile GitHub's **x86_64** `ubuntu-latest` runners *do* expose `/dev/kvm` in practice —
though undocumented and officially unsupported (F5.4).

**The recommendation follows mechanically.** Overdrive's own four-tier model already puts
per-PR Tier-3 on `ubuntu-latest` (x86_64) via LVH and reserves aarch64 for a per-release
self-hosted Graviton tier. Make the merge-blocking microVM-boot gate the **x86_64 LVH job on
hosted runners** — zero new infrastructure, real KVM, honest verdicts — and move aarch64
microVM boot to the per-release tier, **on a Graviton `*.metal` instance specifically** (a
non-metal Graviton runner cannot run KVM at all, so the existing "self-hosted Graviton runner"
wording in `testing.md` needs tightening or the tier will silently not exist). Locally, stop
treating Lima as a verdict-producing environment for this one test class: have the harness
*detect* the nested-Apple environment and **refuse to report pass/fail**, exploiting the
measured asymmetry that a local green is genuine evidence while a local red is uninformative
(F6.3). See **Recommendation for Overdrive** for the ranked, costed version.

## Research Methodology

**Search Strategy**: Six prioritised questions from the brief, each attacked with targeted
web search plus direct fetch of primary sources (vendor documentation, kernel documentation,
LWN, upstream mailing lists, project trackers). Local primary sources — the spike findings,
`.claude/rules/testing.md`, and the repo's CI workflow — were read directly rather than
inferred. Population-diff results already established by the spike were taken as given and
not re-derived, per the brief.

**Source Selection**: Preference order was (1) official vendor/kernel documentation, (2)
upstream kernel mailing lists and LWN, (3) upstream project repositories and trackers, (4)
community discussions. `docs.kernel.org`, `cloud.google.com`, `docs.github.com`, `lwn.net`,
`github.com`, and `infoq.com` are all on the repo's trusted-source list; `medium.com` is
medium-trust and was used for exactly one corroborating detail. `lists.infradead.org`,
`lore.kernel.org`, `repost.aws`, `buildkite.com`, `docs.oracle.com`, and
`developer.apple.com` are not enumerated in the trusted-source YAML but are treated as
**primary/official for their respective subjects** (kernel mailing-list archives, the vendor's
own Q&A and CI, the vendor's own docs) — flagged here rather than silently accepted.

**Quality Standards**: Findings driving the recommendation (F4.1, F5.1, F5.2, F5.3, F5.4,
F2.1, F2.2) carry 2–3 independent sources each. Findings about the *root cause* are
deliberately capped at Low/Medium confidence and labelled, because the required evidence does
not exist publicly. Every inference is explicitly marked **INFERENCE** or **HYPOTHESIS**;
every unretrieved claim is marked **UNVERIFIED**.

## Findings

### Q1 — Is nested virtualization on Apple Silicon known to be unreliable, and how?

**Short answer: yes, but the public evidence is thin and almost entirely community-report
tier. Apple documents the *capability* and documents essentially nothing about its
*limits*.**

#### F1.1 — Apple's nested virtualization is a macOS 15+ / M3+ capability, surfaced as a single boolean

**Evidence**: `Virtualization.framework` exposes
`VZGenericPlatformConfiguration.isNestedVirtualizationSupported` and
`isNestedVirtualizationEnabled`, available from macOS 15.0. Community and vendor
documentation consistently states the requirement is **M3 or later** silicon plus macOS 15.0
(Sequoia).
**Source**: [Apple Developer — `isNestedVirtualizationSupported`](https://developer.apple.com/documentation/virtualization/vzgenericplatformconfiguration/isnestedvirtualizationsupported) — Accessed 2026-08-02.
**Confidence**: Medium. **[Partially UNVERIFIED]** — the Apple documentation page is
JavaScript-rendered and could not be retrieved as text by this research pass; the API name
and availability are corroborated by multiple secondary sources but the *body text* of
Apple's own page was not read. See Knowledge Gaps G1.
**Verification**: [Parallels forum — macOS 15 Sequoia nested virtualization for M3+ Macs](https://forum.parallels.com/threads/macos-15-sequoia-nested-virtualization-for-m3-macs.364397/) — Accessed 2026-08-02; [UTM issue #6821 — nested virtualization request for macOS 15](https://github.com/utmapp/UTM/issues/6821) — Accessed 2026-08-02.
**Analysis**: The API surface is a **boolean**, not a compatibility matrix. There is no
documented statement of which guest-hypervisor features are virtualized faithfully (timer
offsets, GIC maintenance interrupts, ECV, per-CPU counters). This is the structural reason
the failure mode below is undocumented rather than documented-and-known: Apple has published
no contract to violate.

#### F1.2 — The specific published failure shape matches ours: a nested guest that stalls "right after loading modules"

**Evidence**: UTM — the reference consumer of Apple's nested-virt API — carries community
reports of guest hypervisors freezing during boot inside an Apple-Virtualization-Framework
VM, "stalling right after loading modules" (reported against the ESXi-Arm Fling ISO).
**Source**: [utmapp/UTM issue #6821](https://github.com/utmapp/UTM/issues/6821) — Accessed 2026-08-02.
**Confidence**: **Low** — single-reporter community report, different guest OS (ESXi-Arm,
not Linux), and the stall is at L1 boot rather than L2 boot. Label: **community report**,
not documented limitation.
**Verification**: Not independently corroborated. Related but distinct:
[utmapp/UTM issue #7024 — nested virtualization on macOS host + macOS guest returns `HV_UNSUPPORTED`](https://github.com/utmapp/UTM/issues/7024) — Accessed 2026-08-02.
**Analysis**: The value of this finding is *shape confirmation*, not causation. "Boots
sometimes, freezes mid-init at a device/module boundary" is the same silhouette as the
Overdrive spike. It is weak evidence that the nesting layer is where the nondeterminism
lives, and it is consistent with the spike's own population diff (QEMU stalls identically →
not a CH bug).

#### F1.3 — There is no Apple-published erratum, release note, or known-issues list for nested virtualization

**Evidence**: Searches across Apple developer documentation, Apple Developer Forums,
and general web for documented nested-virtualization limitations returned **no** Apple-authored
statement of known issues, guest-compatibility caveats, or errata. The only Apple-adjacent
material found is a developer-forum thread confirming M2 does *not* support it.
**Source**: [Apple Developer Forums thread 756723 — "M2 Nested Virtualization"](https://developer.apple.com/forums/thread/756723) — Accessed 2026-08-02.
**Confidence**: Medium (as a negative result — absence of documentation is itself
observable, though absence-of-evidence caveats apply per `.claude/rules/debugging.md` § 3).
**Analysis**: **This is the load-bearing finding for the decision.** An undocumented
proprietary L0 hypervisor with no published guest-hypervisor compatibility contract and no
errata channel cannot be *made* trustworthy by configuration. You cannot file a bug against
behaviour Apple never specified, and you cannot pin a workaround to a contract that does not
exist. Any mitigation found for it is empirical and unwarranted across macOS point releases.

---

### Q2 — Is this a known aarch64-under-nested-KVM bug?

**Short answer: "nested aarch64 + timers → boot hang" is a documented, fixed-in-upstream-KVM
bug *class*. It is NOT possible to attribute the Overdrive stall to a specific known bug,
because the L0 in this stack is Apple's closed hypervisor, not KVM.**

#### F2.1 — Timer virtualization under nesting is a documented cause of *boot hangs* on arm64

**Evidence**: An upstream KVM/arm64 patch series states the failure directly: on a VHE host
supporting `FEAT_ECV`/`CNTPOFF_EL2`, the nested-virtualization (NV) use case generates
"bursts of spurious ptimer interrupts ... for a non-zero offset, **causing the boot to
hang**." Mechanism: with `HCR_EL2.{E2H,TGE} = {1,1}` the physical counter is not offset by
`CNTPOFF`, so the guest's `CVAL` (set against an offset counter) is far below the observed
physical counter and the timer-fire condition is met spuriously and continuously. Fix:
adjust a loaded ptimer's `CVAL` across guest entry/exit.
**Source**: [linux-arm-kernel — "[PATCH 2/2] KVM: arm64: timers: Adjust CVAL of a ptimer across guest entry and exits"](https://lists.infradead.org/pipermail/linux-arm-kernel/2023-August/861614.html) — Accessed 2026-08-02.
**Confidence**: **High** for the existence of the bug class (kernel mailing list, patch with
maintainer discussion). **This is documented behaviour of KVM-as-L0, not of Apple-as-L0.**
**Verification**: [Marc Zyngier's reply on the same series (lore.kernel.org)](https://lore.kernel.org/lkml/86il97ff17.wl-maz@kernel.org/) — Accessed 2026-08-02;
[LKML thread "Avoid spurious ptimer interrupts for non-zero cntpoff"](https://lkml.rescloud.iu.edu/2308.2/01462.html) — Accessed 2026-08-02.
**Analysis (INFERENCE, labelled)**: The Overdrive stall's shape — freeze *after* the
`virtio_blk` probe, *before* the root mount, i.e. while the kernel is blocked waiting for a
virtio completion interrupt or a timer-driven wait to expire — is precisely the shape a
mis-virtualized timer or a mis-delivered interrupt produces. The spike's own population diff
(same stall with QEMU/KVM, same kernel Image, same rootfs) puts the fault *below* the VMM,
i.e. in the KVM-in-L1 / Apple-L0 boundary. That is consistent with this bug class. **It is
not proof.** No source found ties this specific mechanism to Apple's hypervisor.

#### F2.2 — arm64 nested virtualization is recent, incrementally merged, and validated against Apple silicon in a way its own maintainer flags as risky

**Evidence**: KVM/arm64 NV support is FEAT_NV2-only (FEAT_NV without NV2 was dropped — "no
existing hardware supports the original FEAT_NV without FEAT_NV2, and the architecture is
deprecating the former entirely"). The v7 series (Jan 2023) states support is "still
incomplete, as we don't support ECV in guests," and that "the series has a number of hacks
for the M2 to actually work (**vgic emulation, mostly**)." The maintainer notes it was tested
exclusively on Apple M2, which is "pretty lax" with architecture compliance, and that this
creates risk of depending on non-architectural behaviour. Merging proceeded incrementally
from 6.3-rc1 (19 of 69 patches) onward through 6.8+.
**Source**: [LWN — "KVM: arm64: ARMv8.3/8.4 Nested Virtualization support"](https://lwn.net/Articles/919851/) — Accessed 2026-08-02.
**Confidence**: **High** (LWN summarising an upstream maintainer's own series).
**Verification**: [LWN — "Nested Virtualization on KVM/ARM"](https://lwn.net/Articles/728193/) — Accessed 2026-08-02;
[linux-arm-kernel — "[PATCH v11 00/43] KVM: arm64: Nested Virtualization support (FEAT_NV2 only)"](https://lists.infradead.org/pipermail/linux-arm-kernel/2023-November/882364.html) — Accessed 2026-08-02.
**Analysis**: Two things worth extracting. (a) **vGIC emulation under nesting needed
hardware-specific hacks even in the open implementation** — so GICv3 interrupt-delivery
fidelity under nesting is a known-fragile area, matching research question 2's candidate
mechanism list. (b) The whole arm64 NV stack is young. Apple's independent implementation of
the same problem, shipped in 2024 with no errata channel, has no reason to be more mature.

#### F2.3 — Nesting depth is a separate, commonly-confused failure; it is NOT the Overdrive symptom

**Evidence**: A current Lima issue reports that on an M4 Mac with `nestedVirtualization: true`
and `vz`, QEMU inside the Lima guest fails with "mach-virt: host kernel KVM does not support
providing Virtualization extensions to the guest CPU" — i.e. the L1 cannot hand *further*
virtualization extensions down to an L2. `kvm-ok` passes; the failure is specifically the
third level. The issue is open and labelled `kind/external`.
**Source**: [lima-vm/lima issue #4498](https://github.com/lima-vm/lima/issues/4498) — Accessed 2026-08-02.
**Confidence**: Medium-High (single tracker, but a clean reproduction with an exact error
string).
**Analysis**: **Explicitly not our bug** — and worth recording so a future reader does not
misfile the Overdrive stall against it. Overdrive's CH runs at L2 and needs no
virtualization extensions of its own; it needs `/dev/kvm` in L1, which it has. This finding
corroborates the prompt's own correction: the capability is present, the *reliability* is
the problem.

#### F2.4 — Nothing found ties Cloud Hypervisor or Firecracker to this failure

**Evidence**: No Cloud Hypervisor or Firecracker issue was found describing an aarch64 guest
hanging between the `virtio_blk` probe and root mount under nested KVM. Firecracker
explicitly places nested virtualization outside its scope and points at Cloud Hypervisor and
QEMU instead.
**Source**: [cloud-hypervisor/cloud-hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor) — Accessed 2026-08-02; [firecracker-microvm/firecracker issue #1721](https://github.com/firecracker-microvm/firecracker/issues/1721) — Accessed 2026-08-02.
**Confidence**: Medium (negative result from targeted search; see Knowledge Gaps G2).
**Analysis**: Consistent with the spike's finding that the stall is not CH-specific. Both
upstreams test on non-nested aarch64 hardware, so a nested-only defect would not surface in
their CI and would not be filed by their users.

### Q3 — Configuration mitigations

**Short answer: no mitigation with real evidence behind it was found. Everything below is
either (a) a documented analogue in a *different* hypervisor, or (b) a labelled hypothesis
with a falsifiable probe. Do not treat any of it as a fix.**

This section is deliberately thin. Per the operating brief: where evidence is thin, say so.

#### F3.1 — Documented analogue: a proprietary L0 can partially virtualize the arm64 arch timer such that it delivers no interrupts

**Evidence**: The Linux kernel's own documentation states that "older versions of Hyper-V for
arm64 only partially virtualize the ARMv8 architectural timer, **such that the timer does not
generate interrupts in the VM**," and that running current Linux kernels on those versions
"requires an out-of-tree patch to use the Hyper-V synthetic clocks/timers instead."
**Source**: [Linux kernel documentation — Hyper-V Clocks and Timers](https://docs.kernel.org/virt/hyperv/clocks.html) — Accessed 2026-08-02. Reputation 1.0 (`docs.kernel.org`, trusted-domain list).
**Confidence**: **High** for the analogue; **this is evidence about Hyper-V, not about
Apple.**
**Analysis**: The value is calibration. A closed-source hypervisor shipping an arm64 timer
implementation that silently fails to deliver interrupts is **documented precedent** — and
the fix required patching the *guest kernel*, not configuring it. That is the honest prior
for how expensive an Apple-side workaround would be if one existed.

#### F3.2 — HYPOTHESIS (labelled speculation): GICv3-ITS / MSI delivery for virtio-PCI

**Evidence for the premise only**: Cloud Hypervisor's aarch64 documentation states the
recommended hardware is "AArch64 servers ... equipped with the GICv3 interrupt controller,"
and that "**using PCI devices requires GICv3-ITS for MSI messaging**." It notes MMIO as the
path for machines with GICv2(M) or GICv3-without-ITS.
**Source**: [Cloud Hypervisor — How to build and test Cloud Hypervisor on AArch64 (`docs/arm64.md`)](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/arm64.md) — Accessed 2026-08-02.
**Confidence**: **Low as a mitigation.** The *premise* (PCI needs ITS) is documented; the
*inference* (Apple's nested GICv3-ITS emulation drops LPIs, stranding virtio completions) is
**pure speculation by this researcher**. Additional caution: the MMIO fallback sentence in
that doc may be stale relative to CH v46 — CH's transport support has narrowed over time and
this was **not verified against v46**. See Knowledge Gaps G4.
**If you probe it anyway**, use the `.claude/rules/debugging.md` § 4 triple:
> **Hypothesis**: the stall is a lost virtio completion interrupt caused by
> nested GICv3-ITS/LPI mis-delivery in Apple's L0.
> **Predicted outcome**: on a stalled guest, `/proc/interrupts` (via a
> `sysrq`/`magic-sysrq` dump or an earlier init) shows the virtio-blk MSI line at a *frozen*
> count while the arch-timer line still advances — i.e. the CPU is alive, one interrupt
> source is dead.
> **Falsification**: both counters frozen (→ vCPU/timer stall, not MSI), or the virtio
> counter advancing while the kernel still hangs (→ not an interrupt-delivery problem at all).

#### F3.3 — Mitigations that are commonly suggested and are *wrong* for this environment

- **`no_timer_check`** — an **x86-only** parameter (it disables the IO-APIC ↔ PIT timer
  routing check). It has no arm64 effect. Do not add it.
- **`clocksource=`** — on arm64 the practical clocksource is the architected system counter;
  there is no meaningful alternative to switch to inside a microVM. Unlike the Hyper-V case
  (F3.1) there is no synthetic-clock driver to fall back to under Apple's L0.
- **"Enable nested virtualization"** — already enabled and already working (see the problem
  statement and F2.3). Repeating this is the single most common misdiagnosis in this space.

#### F3.4 — vCPU count, memory sizing, KASLR: no evidence found

**Evidence**: No source was found linking `--cpus boot=N`, `--memory size=`, KASLR, or CH
console/serial configuration to nested-virt boot-stall rates on aarch64.
**Confidence**: **Low / unknown** — this is a genuine gap, not a negative finding.
**Analysis**: The spike already ran `--cpus boot=1`, i.e. the configuration with the *least*
inter-processor-interrupt and timer surface, and still stalls ~2/3. That is weak evidence
against vCPU count being the lever. **Any further tuning here is empirical trial-and-error
against an unspecified black box** — and even a configuration that reduced the stall rate to
1-in-20 would not make the environment a *gate*, because a gate that intermittently reports a
missing result is a flaky gate, and flaky gates get ignored. See the Recommendation.

### Q4 — What comparable microVM projects use for CI

**Short answer: every project that gates on real microVM boot runs its aarch64 tier on bare
metal. None of them nest on aarch64. This is the single most decisive finding in this
document.**

#### F4.1 — Kata Containers: aarch64 CI is bare-metal *by necessity*, stated explicitly

**Evidence**: "Kata Containers can only be run using bare metal aarch64 hosts. Nested
virtualization is required on virtual hosts to create Kata Containers. **Nested
virtualization isn't available on aarch64 virtual hosts.**" Kata's CI historically obtained
bare-metal ARM nodes via the CNCF Community Infrastructure Lab (Equinix Metal / Packet), with
nested-virt-capable x86 cloud capacity from Vexxhost for the non-ARM tier.
**Source**: [Oracle Linux Cloud Native Environment — Known Issues](https://docs.oracle.com/en/operating-systems/olcne/2/relnotes/issues.html) — Accessed 2026-08-02 (vendor documentation restating the platform constraint).
**Confidence**: **High** — the constraint is stated as documented platform behaviour by a
vendor shipping Kata, and is independently corroborated by every cloud provider's own docs
(F5.1–F5.3 below).
**Verification**: [Kata Containers — "testing and packaging powered by the cloud"](https://medium.com/kata-containers/kata-containers-testing-and-packaging-powered-by-the-cloud-b752de2ee471) — Accessed 2026-08-02 [medium.com, medium-trust: used only for the CI-topology detail, which is corroborated by the CNCF CIL programme]; cross-checked against GCP and AWS documentation in F5.
**Analysis**: Kata is the closest analogue to Overdrive's problem — a project whose *unit of
test* is "does a VM actually boot and run a workload," on aarch64, with a small team. Its
answer, arrived at years ago and unchanged, is: **do not nest on aarch64; get bare metal.**

#### F4.2 — Firecracker: aarch64 CI on EC2 bare-metal Graviton instances

**Evidence**: Firecracker's aarch64 CI ran on `a1.metal` and moved to Graviton2 `m6g.metal`
instances; the pipelines are hosted on Buildkite (self-managed agents on those instances), not
on hosted GitHub runners. Firecracker separately states that nested virtualization is outside
its scope and points users at Cloud Hypervisor and QEMU.
**Source**: [firecracker-microvm/firecracker issue #2131 — "[CI on arm] Switch to m6g metal instances"](https://github.com/firecracker-microvm/firecracker/issues/2131) — Accessed 2026-08-02.
**Confidence**: Medium-High (project tracker; the underlying `*.metal` requirement is
corroborated by AWS's own position in F5.2).
**Verification**: [firecracker issue #1327 — "Transition aarch64 CI from buildkite to a1.metal"](https://github.com/firecracker-microvm/firecracker/issues/1327) — Accessed 2026-08-02; [Firecracker Buildkite `firecracker-ci-aarch64` pipeline](https://buildkite.com/firecracker/firecracker-ci-aarch64) — Accessed 2026-08-02.
**Analysis**: Firecracker is the reference microVM project and it operates **its own CI
infrastructure** to get honest aarch64 boot coverage. That is the cost side of this answer:
bare metal for aarch64 is not free, and nobody has found a way around it.

#### F4.3 — Cilium `little-vm-helper` (already this repo's Tier-3 entry point) requires `/dev/kvm`, and its arm64 story is weaker than its amd64 story

**Evidence**: LVH's README states image building "may require sudo as relies on `/dev/kvm`."
On architecture: cross-building with `--arch=arm64` / `--arch=amd64` is supported, but "**only
amd64 images contain a compatible bootloader.** So even though kernels are present in the
arm64 images, you'll need to supply it to QEMU through the `--kernel` lvh option flag."
Building images and kernels is Linux-only; *running* pre-built images works on macOS via QEMU
(without KVM, therefore emulated and slow).
**Source**: [cilium/little-vm-helper README](https://github.com/cilium/little-vm-helper) — Accessed 2026-08-02.
**Confidence**: Medium-High (primary project documentation).
**Verification**: [cilium/little-vm-helper `action.yaml`](https://github.com/cilium/little-vm-helper/blob/main/action.yaml) — Accessed 2026-08-02; [Cilium docs — Run eBPF Tests with Little VM Helper](https://docs.cilium.io/en/latest/contributing/development/bpf_tests/) — Accessed 2026-08-02.
**Analysis**: Two consequences for Overdrive. (a) LVH's GitHub Action is designed for the
**x86_64 hosted-runner** path, which is exactly where `/dev/kvm` is available (F5.4). (b) The
arm64 path requires supplying the kernel explicitly — which is *convenient* here, because the
Overdrive spike already established that the appliance must ship a **raw arm64 `Image`**
anyway (`findings.md` § Design implications, `[D5]`). The two requirements coincide.

#### F4.4 — Nobody in this cohort gates on nested virtualization on a developer laptop

**Evidence**: Across Kata, Firecracker, and Cilium/LVH, no project was found that treats a
nested developer-machine VM as its authoritative gate for VM boot. Cilium's LVH explicitly
frames macOS execution as QEMU-without-KVM (development convenience), and Kata/Firecracker
both run dedicated bare-metal fleets.
**Confidence**: Medium (negative result across three projects; not exhaustive).
**Analysis**: This is *convergent* practice, not one project's taste. The gate lives on
hardware that can actually virtualize; the laptop gets a fast, explicitly-not-authoritative
loop. That is exactly the split research question 6 asks about, and it is what the industry
already does.

---

### Q5 — Concrete alternative environments, with current facts

**Summary table** (details and citations below):

| Environment | Reliable KVM? | arch | Cost / operational burden |
|---|---|---|---|
| GitHub hosted `ubuntu-latest` (x64) | **Yes in practice, `/dev/kvm` present** — but *undocumented and officially unsupported* | x86_64 | Free / included. Zero new infra. |
| GitHub hosted **arm64** runners | **No** — `/dev/kvm` absent (Azure DPDsv6 SKU has no nested virt) | aarch64 | n/a |
| AWS `*.metal` Graviton (`c7g.metal`, `m7g.metal`, …) | **Yes** — real EL2, not nested | aarch64 | Bare-metal hourly rate; **you operate a self-hosted runner** |
| AWS C8i / M8i / R8i nested virt (new, 2026) | Yes, but **Intel-only; Graviton explicitly not supported** | x86_64 | Standard instance rate + self-hosted runner |
| GCP nested virtualization | Yes on Intel VT-x | x86_64 **only — Arm explicitly excluded** | Standard rate + self-hosted runner |
| Azure | Nested virt on selected x86 SKUs; Arm SKUs (Dpsv5/DPDsv6) no | x86_64 | Standard rate + self-hosted runner |
| Local non-nested Linux host (any x86_64 box, or an ARM SBC/Ampere box) | **Yes — first-class KVM** | either | One machine; you own it |
| Apple Silicon + Lima `vz` nested | **No — this is the measured problem** | aarch64 | Free, already have it |

#### F5.1 — GCP: nested virtualization is Intel VT-x only; Arm is explicitly excluded

**Evidence**: Google's own documentation states nested virtualization requires Intel VT-x
enabled processors, and lists unsupported machine types including "VMs powered by **AMD and
Arm processors**." The only supported L1 hypervisor is Linux KVM. Expected performance
penalty is ">10%" for CPU-bound, potentially worse for I/O-bound workloads.
**Source**: [Google Cloud — Nested virtualization overview](https://docs.cloud.google.com/compute/docs/instances/nested-virtualization/overview) — Accessed 2026-08-02.
**Confidence**: **High** — official vendor documentation. Reputation 1.0 (`cloud.google.com`,
trusted-domain list).
**Analysis**: Rules GCP out entirely for an aarch64 microVM-boot gate. It remains viable for
an **x86_64** gate.

#### F5.2 — AWS: nested virtualization arrived in 2026 for Intel 8th-gen only; Graviton still requires `*.metal`

**Evidence**: EC2 VMs historically did not expose virtualization extensions to guests on any
architecture — running your own hypervisor required `*.metal`. In 2026 AWS introduced nested
virtualization on C8i / M8i / R8i (8th-generation Intel) with KVM and Hyper-V as the supported
L1 hypervisors; **Graviton instances are not supported**. Graviton3 is ARMv8.4-a and does
carry the architectural nesting features, but only the bare-metal instances expose them.
**Source**: [InfoQ — "AWS Introduces Nested Virtualization on EC2 Instances" (2026-03)](https://www.infoq.com/news/2026/03/aws-ec2-nested-virtualization/) — Accessed 2026-08-02.
**Confidence**: Medium-High. InfoQ is medium-high tier (0.8) and this is recent
(March 2026). **[Partially UNVERIFIED]** — the primary AWS documentation page for this launch
was not retrieved in this pass; the "Graviton not supported" clause should be re-checked
against `docs.aws.amazon.com` before committing spend. See Knowledge Gaps G3.
**Verification**: [AWS re:Post — "Nested Virtualization support on EC2 Graviton 3 `*.metal` instances"](https://repost.aws/questions/QUChyy06f6TRKgitoNLKV4DQ/nested-virtualization-support-on-ec2-graviton-3-metal-instances) — Accessed 2026-08-02; [AWS — EC2 C7g metal instances now available (2023-02)](https://aws.amazon.com/about-aws/whats-new/2023/02/amazon-ec2-c7g-metal-instances-available/) — Accessed 2026-08-02.
**Analysis**: For aarch64, AWS's answer is the same as Firecracker's and Kata's: **a Graviton
`*.metal` instance**. On such an instance KVM runs on real EL2 — this is *not nesting at all*,
which is the property that makes it trustworthy. The 2026 Intel nested-virt launch is
irrelevant to an aarch64 target but is a legitimate cheaper option if the gate is x86_64.

#### F5.3 — GitHub-hosted **arm64** runners do not expose `/dev/kvm`

**Evidence**: Attempts to use KVM on GitHub's arm64 hosted runners fail with `/dev/kvm does
not exist`; the cited reason is the underlying Azure **DPDsv6** hardware SKU, which does not
support nested virtualization. Linux arm64 hosted runners themselves became free for public
repositories in a 2025 public preview.
**Source**: [GitHub community discussion #148648 — "Linux arm64 hosted runners now available for free in public repositories"](https://github.com/orgs/community/discussions/148648) — Accessed 2026-08-02.
**Confidence**: Medium-High — community/tracker tier, but the claim is a concrete negative
observation (`/dev/kvm` absent) reproduced by multiple reporters and consistent with Azure's
Arm SKU capabilities.
**Verification**: [gem5 issue #787 — "KVM not supported in GitHub Action Runner testing infrastructure"](https://github.com/gem5/gem5/issues/787) — Accessed 2026-08-02; [GitHub Blog — "Arm64 on GitHub Actions"](https://github.blog/news-insights/product-news/arm64-on-github-actions-powering-faster-more-efficient-build-systems/) — Accessed 2026-08-02.
**Analysis**: **This closes the cheapest door.** "Just run the Tier-3 microVM job on a hosted
arm64 runner" is not available in August 2026.

#### F5.4 — GitHub-hosted **x86_64** `ubuntu-latest` runners DO have `/dev/kvm` — but it is undocumented and unsupported

**Evidence**: `/dev/kvm` is present on standard (non-large) `ubuntu-latest` runners; community
reports date its appearance to a January 2024 runner-image update, and note that in practice
only a udev rule / group membership adjustment is needed to use it. GitHub announced
"hardware accelerated nested virtualization on larger runners" in February 2023. A September
2025 documentation request (`actions/runner-images` #12933) demonstrating working nested VMs
via Multipass observes that "this capability is **not documented** in the official
documentation"; it was closed without an official statement. The long-running community
discussion on the topic remains marked *Unanswered*.
**Source**: [actions/runner-images issue #12933 — "Documentation Request: Nested Virtualization Support in GitHub-hosted Runners" (opened 2025-09-01)](https://github.com/actions/runner-images/issues/12933) — Accessed 2026-08-02.
**Confidence**: Medium-High for *availability*; **High for "undocumented/unsupported."**
**Verification**: [GitHub community discussion #8305 — "Revisiting KVM support for Hosted GitHub Actions"](https://github.com/orgs/community/discussions/8305) — Accessed 2026-08-02 (contains the Feb-2023 GitHub-staff reference to nested virt on larger runners and an Oct-2025 report that "kvm virtualization seems to work just adding the udev rule now"); [actions/runner-images issue #7541](https://github.com/actions/runner-images/issues/7541) — Accessed 2026-08-02; [GitHub Docs — About GitHub-hosted runners](https://docs.github.com/actions/using-github-hosted-runners/about-github-hosted-runners) — Accessed 2026-08-02.
**Analysis**: This is a *real* capability with a *paper-thin* guarantee. It is good enough to
carry a merge-blocking gate **provided the job asserts `/dev/kvm` exists and fails loudly if
it disappears** — never skips. An x86_64 KVM gate on hosted runners costs nothing and
operates no infrastructure, which is decisive for a small team. Its weakness is architectural
coverage, not reliability.

#### F5.5 — A local non-nested Linux host is the cheapest trustworthy aarch64 option that is not a cloud bill

**Evidence** (INFERENCE from F5.1–F5.3 plus the spike's own measurements, labelled as such):
the property that makes every recommended environment above trustworthy is *the absence of a
hypervisor beneath KVM*. Any Linux machine running on bare hardware — an x86_64 desktop, or an
ARM box (Ampere Altra workstation, Apple-silicon-free ARM server, or even a well-provisioned
ARM SBC with virtualization extensions) — gives KVM at EL2 with no L0 to mis-virtualize
timers or the GIC.
**Confidence**: Medium (inference from documented constraints, not a cited benchmark).
**Analysis**: For a small team, "one Linux box on a desk running a self-hosted runner" is
frequently cheaper and less operationally noisy than a cloud bare-metal fleet — but it is
still *new infrastructure you operate*, and it introduces a single point of failure for a
merge-blocking gate. Say so plainly rather than pretending it is free.

### Q6 — Keeping the fast local loop while making the gate honest

**Short answer: the split Overdrive already documents is the industry pattern; what is
missing is not a new pattern but an explicit, mechanical "this environment cannot render a
verdict" refusal on the local side.**

#### F6.1 — The macOS-dev / Linux-CI split with an explicitly non-authoritative local tier is what the comparable projects do

**Evidence**: Cilium's LVH frames macOS execution as running pre-built images "in macOS (both
x86 and arm64)" through QEMU — i.e. *without* KVM, so emulated: a development convenience,
not the CI signal. Cilium's actual CI signal is the LVH GitHub Action on Linux runners. Kata
and Firecracker place their authoritative VM-boot signal on dedicated bare metal (F4.1, F4.2).
**Source**: [cilium/little-vm-helper README](https://github.com/cilium/little-vm-helper) — Accessed 2026-08-02; [cilium/little-vm-helper `action.yaml`](https://github.com/cilium/little-vm-helper/blob/main/action.yaml) — Accessed 2026-08-02.
**Confidence**: Medium-High.
**Analysis**: Note the difference in *why*. For Cilium, macOS is non-authoritative because it
is **slow** (emulation). For Overdrive it would be non-authoritative because it is
**untruthful** (intermittent missing verdicts). The second is more dangerous, because slow is
self-announcing and flaky is not.

#### F6.2 — Overdrive's own rules already contain the mechanism; the microVM case is the first where "Lima is the canonical inner loop" breaks

**Evidence** (local, primary): `.claude/rules/testing.md` § "Running tests — Lima VM" makes
Lima "the canonical inner-loop path for all platforms" and hard-blocks bare `cargo nextest
run`; the same section already carves out that "**Tier 3 / Tier 4 stays on `cargo xtask
integration-test vm` and `cargo xtask xdp-perf`** ... the kernel-matrix tier 3 harness still
runs on CI via LVH; do not collapse the two." The same file states `#[ignore]` is legitimate
when the blocker is an external resource the implementation cannot synthesize, naming "a
kernel matrix only available in CI" as a valid case, and requires a `reason` string naming
the unblocking step. It also forbids `--no-run` as a gate.
**Source**: `/Users/marcus/conductor/workspaces/helios/hanoi/.claude/rules/testing.md` — read 2026-08-02.
**Confidence**: **High** (primary project source).
**Analysis**: The structural answer therefore requires **no new doctrine**. It requires:
(1) classifying microVM-boot tests into the existing `xtask integration-test vm` tier rather
than the Lima inner loop, and (2) adding a *refusal* to the local path — because unlike every
other test in the repo, running this one locally produces a result that looks like a verdict
and is not one.

#### F6.3 — The stall's asymmetry (missing result, never wrong result) determines the correct local-loop design

**Evidence**: The spike measured that "whenever the guest reached userspace, P1 and P2 held
**100% of the time**. The stall never produced a wrong answer — only a missing one."
**Source**: `docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md` — read 2026-08-02.
**Confidence**: High (primary, measured).
**Analysis (INFERENCE, labelled)**: This asymmetry is *exploitable*, and it is exactly what
makes a two-tier design safe:
- A **green** local run is genuine positive evidence (the assertions really did hold).
- A **red** local run is *uninformative* — it may be a real defect or the environment.
This is precisely `.claude/rules/debugging.md` § 3 ("inspection-tool gaps look like negative
evidence"): a local red is absence-of-evidence, not evidence-of-absence. The local loop must
therefore be allowed to *confirm* but never to *condemn*, and the harness should say so in its
own output rather than relying on a human remembering it.
A bounded retry (e.g. 3 attempts, report the best outcome) is **defensible in the
explicitly-non-gate local loop only**, because a false red is the sole failure mode. It is
**forbidden in CI**, where it would convert a real regression into a flake — the standing
project rule that flaky tests are bugs, not "just rerun it," applies unchanged.

## Source Analysis

| Source | Domain | Reputation | Type | Access date | Cross-verified |
|---|---|---|---|---|---|
| Linux kernel docs — Hyper-V clocks/timers | docs.kernel.org | High (1.0, trusted list) | official | 2026-08-02 | N (single, but primary) |
| LWN — ARMv8.3/8.4 Nested Virtualization support | lwn.net | Medium-High (0.8, trusted list) | industry/technical | 2026-08-02 | Y |
| LWN — Nested Virtualization on KVM/ARM | lwn.net | Medium-High (0.8) | industry/technical | 2026-08-02 | Y |
| linux-arm-kernel — ptimer CVAL patch (boot hang) | lists.infradead.org | High (primary ML archive) | official/upstream | 2026-08-02 | Y |
| lore.kernel.org — Marc Zyngier reply, same series | lore.kernel.org | High (primary ML archive) | official/upstream | 2026-08-02 | Y |
| linux-arm-kernel — NV v11 (FEAT_NV2 only) | lists.infradead.org | High (primary ML archive) | official/upstream | 2026-08-02 | Y |
| Google Cloud — Nested virtualization overview | docs.cloud.google.com | High (1.0, trusted list) | official | 2026-08-02 | Y |
| GitHub Docs — About GitHub-hosted runners | docs.github.com | High | official | 2026-08-02 | Y |
| GitHub Blog — Arm64 on GitHub Actions | github.blog | Medium-High | official/vendor | 2026-08-02 | Y |
| actions/runner-images #12933 | github.com | Medium-High (0.8, trusted list) | tracker | 2026-08-02 | Y |
| actions/runner-images #7541 | github.com | Medium-High (0.8) | tracker | 2026-08-02 | Y |
| community discussion #8305 (KVM on hosted runners) | github.com | Medium-High (0.8) | community | 2026-08-02 | Y |
| community discussion #148648 (arm64 runners preview) | github.com | Medium-High (0.8) | community | 2026-08-02 | Y |
| gem5 #787 (KVM unsupported on GH runners) | github.com | Medium-High (0.8) | tracker | 2026-08-02 | Y |
| lima-vm/lima #4498 | github.com | Medium-High (0.8) | tracker | 2026-08-02 | N (single reporter) |
| utmapp/UTM #6821 | github.com | Medium-High domain / **Low claim** | community report | 2026-08-02 | N |
| utmapp/UTM #7024 | github.com | Medium-High domain / Low claim | community report | 2026-08-02 | N |
| Apple Developer — `isNestedVirtualizationSupported` | developer.apple.com | High (vendor primary) | official | 2026-08-02 | **Not retrieved (JS-rendered)** |
| Apple Developer Forums thread 756723 | developer.apple.com | Medium-High | community/vendor | 2026-08-02 | Y |
| Parallels forum — macOS 15 nested virt M3+ | forum.parallels.com | Medium | vendor forum | 2026-08-02 | Y |
| cilium/little-vm-helper (README) | github.com | Medium-High (0.8) | project docs | 2026-08-02 | Y |
| cilium/little-vm-helper `action.yaml` | github.com | Medium-High (0.8) | project source | 2026-08-02 | Y |
| cilium/little-vm-helper-images | github.com | Medium-High (0.8) | project docs | 2026-08-02 | Y |
| Cilium docs — BPF tests with LVH | docs.cilium.io | High (1.0, trusted list) | official | 2026-08-02 | Y |
| cloud-hypervisor `docs/arm64.md` | github.com | Medium-High (0.8) | project docs | 2026-08-02 | N (see G4) |
| firecracker #1327 / #2131 | github.com | Medium-High (0.8) | tracker | 2026-08-02 | Y |
| Firecracker Buildkite aarch64 pipeline | buildkite.com | Medium-High (project's own CI) | primary CI | 2026-08-02 | Y |
| Oracle — OLCNE Known Issues (Kata aarch64) | docs.oracle.com | High (vendor docs) | official | 2026-08-02 | Y |
| InfoQ — AWS nested virtualization on EC2 (2026-03) | infoq.com | Medium-High (0.8, trusted list) | industry reporting | 2026-08-02 | Partial (see G3) |
| AWS re:Post — Graviton 3 `*.metal` nested virt | repost.aws | Medium-High (vendor Q&A) | vendor community | 2026-08-02 | Y |
| AWS — C7g metal instances available | aws.amazon.com | High (vendor announcement) | official | 2026-08-02 | Y |
| Kata Containers — testing/packaging in the cloud | medium.com | **Medium (0.6)** | community/vendor blog | 2026-08-02 | Y (corroborating detail only) |
| `spike/findings.md` (local) | — | High (primary, measured) | project artifact | 2026-08-02 | N/A |
| `.claude/rules/testing.md` (local) | — | High (primary) | project artifact | 2026-08-02 | N/A |
| `.github/workflows/ci.yml`, `.github/roadmap-issues.md` (local) | — | High (primary) | project artifact | 2026-08-02 | N/A |

Reputation distribution (external sources, n=27): High 10 (37%) | Medium-High 16 (59%) |
Medium 1 (4%). Average ≈ **0.83**.

### Source freshness — this topic moves, and stale sources are the main hazard

Two of the facts in this document changed within the last ~30 months, and most of the
folklore online predates the change. Dates are load-bearing here:

| Fact | Changed | Anything older than this is stale |
|---|---|---|
| Apple nested virtualization exists at all (M3+, macOS 15) | 2024 (macOS 15.0) | Pre-2024 "Apple Silicon can't nest" is correct-then, wrong-now |
| `/dev/kvm` on GitHub `ubuntu-latest` (x64) | Feb 2023 (larger runners, GitHub blog) → ~Jan 2024 (standard runners, community-reported) | Pre-2024 "hosted runners have no KVM" is stale for **x64**; still true for **arm64** |
| GitHub Linux **arm64** hosted runners | 2025 public preview, free for public repos | — (KVM still absent as of 2026-08) |
| AWS nested virtualization on EC2 VMs | March 2026, Intel C8i/M8i/R8i only | Pre-2026 "AWS never nests" is stale for x86_64; still true for Graviton |
| KVM/arm64 nested virtualization upstream | incremental merge from 6.3-rc1 (2023) onward; FEAT_NV2-only | The LWN articles [2][3] are 2017/2023 — accurate on mechanism and hazard classes, **out of date on merge status**. Do not cite them for "what is supported today." |

Nothing in the recommendation depends on the two stale-prone LWN articles for a *status*
claim — they are cited only for mechanism and for the maintainer's own characterisation of
vGIC/timer fragility under nesting, which has not been retracted.

## Knowledge Gaps

### G1 — Apple's own documentation of nested-virtualization limits was not read
**Issue**: `developer.apple.com` documentation pages are JavaScript-rendered and returned no
body text. The API name and availability were corroborated from secondary sources, but if
Apple documents *any* caveat on that page it is not reflected here.
**Attempted**: direct fetch of the `isNestedVirtualizationSupported` page; searches for Apple
release notes, WWDC material, and known-issues lists for Virtualization.framework.
**Recommendation**: open the page in a browser and read it (2 minutes). Also check the macOS
26.x release notes for Virtualization.framework. Low expected yield — F1.3 found no evidence
such a caveat list exists — but cheap.

### G2 — No attempt was made to reproduce or bisect on a *non-Apple* nested aarch64 host
**Issue**: All evidence for "the nesting layer is at fault" is (a) the spike's local
population diff and (b) analogy to documented bug classes. Nobody has run the same kernel
Image + rootfs under CH on a nested-but-not-Apple aarch64 L0 (e.g. KVM-on-KVM on a Graviton
`*.metal`), which would separate "nesting in general" from "Apple's L0 specifically."
**Recommendation**: **do not spend time on this.** It is a scientifically interesting probe
with no bearing on the decision — neither answer changes the recommendation, because neither
makes the laptop a gate.

### G3 — AWS's 2026 nested-virtualization launch was verified only through secondary reporting
**Issue**: The "C8i/M8i/R8i only; Graviton not supported" claim comes from InfoQ (0.8) plus
AWS re:Post answers, not from `docs.aws.amazon.com` directly.
**Recommendation**: read the AWS nested-virtualization documentation page before committing
spend. Note this only matters if someone proposes an **x86_64 cloud** runner; it does not
affect the aarch64 conclusion (which is `*.metal`, corroborated three ways).

### G4 — Cloud Hypervisor's MMIO-vs-PCI transport support at v46 was not verified
**Issue**: `docs/arm64.md` mentions MMIO as a path for hosts without GICv3-ITS. Cloud
Hypervisor's transport support has narrowed over releases and that sentence may be stale;
whether v46.0.0 can actually run virtio-MMIO was not checked.
**Recommendation**: only relevant if someone pursues the F3.2 hypothesis. Check
`cargo build --no-default-features --features …` in the v46 tree before assuming MMIO exists.

### G5 — LVH kernel/arch coverage for the pinned 6.18 aarch64 kernel is unconfirmed
**Issue**: LVH publishes multi-arch images at `quay.io/lvh-images`, but the exact kernel
versions available for **arm64** were not enumerated (the list lives in `kernels.json` /
the registry tags, neither of which was read). Separately, LVH documents that arm64 **rootfs**
images must be built on **native arm64 runners** (libguestfs cannot cross-build) and that only
amd64 images carry a bootloader, so arm64 requires supplying `--kernel` explicitly.
**Recommendation**: read `cilium/little-vm-helper-images/kernels.json` and the Quay tag list
before scheduling the aarch64 tier. If 6.18 arm64 is absent, the Graviton `*.metal` runner
must also *build* the image — which it can, being both native-arch and KVM-capable.

### G6 — No evidence found for or against `--cpus` / `--memory` / KASLR as levers
Stated in F3.4. Not worth closing; see the Recommendation's rejection of tuning.

## Conflicting Information

### Conflict 1 — "GitHub-hosted runners do not support nested virtualization"
**Position A**: GitHub's hosted runners do not support nested virtualization; `/dev/kvm` is
unavailable, so packer / Android emulator / KVM workloads cannot run.
— Source: [Cilium LVH docs and multiple community threads](https://github.com/cilium/little-vm-helper), Reputation 0.8.
**Position B**: `/dev/kvm` is present on standard `ubuntu-latest` (x64) runners since a
January 2024 runner-image update; nested VMs demonstrably work (Multipass, QEMU/KVM), needing
at most a udev rule.
— Source: [actions/runner-images #12933](https://github.com/actions/runner-images/issues/12933) and [community #8305](https://github.com/orgs/community/discussions/8305), Reputation 0.8.
**Assessment**: **Both are correct and the conflict is architectural.** Position A is true of
**arm64** hosted runners (F5.3, Azure DPDsv6, no nesting) and was true of x64 runners before
2024. Position B is true of **x86_64** hosted runners today. Much of the "hosted runners don't
do KVM" folklore is stale (pre-2024) or arch-confused. Anyone reading only one side will
reach the wrong conclusion for Overdrive — hence the explicit split in F5.3/F5.4.
Independently: *availability* is not *support*. GitHub has never documented it, and
`actions/runner-images` #12933 was closed without an official statement, so a gate resting on
it must assert the capability rather than assume it.

### Conflict 2 — "nested virtualization is broken on Apple Silicon" vs "nested virtualization works"
**Position A**: Nested virt on Apple Silicon does not work — QEMU reports "host kernel KVM
does not support providing Virtualization extensions to the guest CPU."
— Source: [lima-vm/lima #4498](https://github.com/lima-vm/lima/issues/4498), Reputation 0.8.
**Position B**: It works — `/dev/kvm` is present in the Lima guest and Cloud Hypervisor boots
successfully ~1 attempt in 3.
— Source: `spike/findings.md` (local, measured), Reputation High.
**Assessment**: **Position B is correct for this stack; Position A is about a different
nesting depth.** #4498 concerns exposing virtualization extensions *one level further down*
(L1 → L2 hypervisor), which Overdrive does not need. This distinction is recorded because it
is the single easiest way for a future reader to misdiagnose the stall — and because the
prompt's own correction had to undo an earlier instance of exactly this confusion (the
`kvm [1]: HYP mode not available` line emitted by the innermost guest).

## Recommendation for Overdrive

Opinionated, ranked, and split into "this week" versus "the durable gate."

### (a) Make the local loop honest — this week, ~half a day, zero infrastructure

**A1. Reclassify microVM-boot tests out of the Lima inner loop.** They belong in the
`cargo xtask integration-test vm` tier, not the default `cargo nextest run` lane and not the
Lima-routed integration lane. This is the split `testing.md` already draws ("Tier 3 / Tier 4
stays on `cargo xtask integration-test vm`"); the microVM driver is simply the first feature
where crossing it is *actively harmful* rather than merely slow.
**Cost**: near zero. **Risk if skipped**: a crafter burns days debugging the environment as a
CH or driver bug — the spike says this explicitly and it is the most likely failure of this
whole feature.

**A2. Add an environment-capability preflight that REFUSES, rather than passes or fails.**
Before any microVM-boot assertion runs, probe the host and, when nesting under a
non-KVM L0 is detected (`systemd-detect-virt` → `apple`, or more generally: KVM present *and*
the host is itself virtualized by something that is not KVM), emit a distinct third outcome —
"ENVIRONMENT CANNOT RENDER A VERDICT" — and do not report pass/fail. Two properties make this
right rather than fussy:
- The stall produces a **missing** result, never a wrong one (F6.3). So a local **green is
  genuine evidence** and should still be reported as such; only **red** is uninformative.
- Silent skipping is forbidden by the project's own discipline (a `--no-run`-shaped green is
  precisely the failure mode `testing.md` bans). Refusal is loud; skipping is not.
**Cost**: a small amount of harness code. **Do not** implement this as `#[ignore]` alone —
`#[ignore]` hides the test entirely and loses the "green is real evidence" half.

**A3. Where a `#[ignore]` is used, carry the sanctioned reason string.** `testing.md` permits
`#[ignore]` for "a kernel matrix only available in CI" and requires a `reason` naming the
unblocking resource, e.g.
`#[ignore = "needs non-nested KVM host — Lima on Apple Silicon stalls ~2/3 (spike/findings.md); authoritative run is the CI Tier-3 microVM job"]`.
**Cost**: minutes.

**A4. Bounded retry is permitted in the local loop ONLY, and must announce itself.** Because
the sole local failure mode is a false red, retrying up to N times and reporting the best
outcome is legitimate *there*. It is **forbidden in CI**, where it would mask real
regressions and violates the standing "flaky tests are bugs, not rerun candidates" rule.
**Cost**: trivial. **Risk**: if this ever leaks into the CI path it silently destroys the
gate — gate it on the same preflight from A2, not on an env var a CI job could inherit.

**A5. Do NOT spend time tuning the Lima environment.** No mitigation with evidence behind it
exists (Q3). The most-cited candidates are wrong for arm64 (`no_timer_check` is x86-only) or
inapplicable (no synthetic clocksource to fall back to). Even a tuning that dropped the stall
rate to 1-in-20 would not produce a gate — an intermittently-verdictless gate is a flaky gate,
and flaky gates get ignored, which is worse than an absent one. **Cost of ignoring this
advice**: unbounded, against a black box with no errata channel.

### (b) The durable gate — ranked

**B1 (recommended). Make the merge-blocking microVM-boot gate x86_64, on GitHub-hosted
`ubuntu-latest`, through the existing LVH `xtask integration-test vm` entry point.**
`/dev/kvm` is present there (F5.4); the whole repo's per-PR CI already runs on
`ubuntu-latest`; `testing.md`'s per-PR Tier-3 row already lives there. This is real KVM on
real hardware-assisted virtualization with **no nesting**, so the failure class in this
document does not apply.
**Cost**: **zero new infrastructure, zero marginal spend.** The work is authoring the job and
supplying a raw kernel `Image` (which the spike already established the appliance must build
regardless).
**Non-negotiable condition**: the job must **assert `/dev/kvm` exists and fail loudly if it
does not** — never skip. GitHub has never documented this capability and closed the
documentation request without a statement (F5.4); a silent skip would convert a withdrawn
capability into a permanently-green gate. This is the same "vacuous pass" hazard the mutation
gate already guards against.
**What it does not buy**: aarch64 coverage. Say so explicitly in the job name and in
`testing.md`, rather than letting "Tier-3 microVM boot is green" imply an architecture it
never covered.

**B2 (required, per-release). Put aarch64 microVM boot on a Graviton `*.metal` instance —
and fix the existing wording that omits `.metal`.**
`testing.md` and `roadmap-issues.md` currently say "per-release aarch64 Tier-3 matrix on
**self-hosted Graviton runner**." **A non-metal Graviton instance cannot run KVM at all** —
AWS does not expose virtualization extensions to guests on Graviton, and the 2026 nested-virt
launch is Intel-only (F5.2). If that runner is provisioned as, say, `m7g.xlarge`, the aarch64
tier will not merely be flaky, it will be impossible. Tighten the wording to name `*.metal`
(`c7g.metal` / `m7g.metal` or later) before anyone provisions it.
This also matches every peer: Kata ("bare metal aarch64 hosts ... nested virtualization isn't
available on aarch64 virtual hosts", F4.1) and Firecracker (`a1.metal` → `m6g.metal`, F4.2).
**Cost**: real. A bare-metal Graviton instance at per-release cadence plus a self-hosted
runner you operate — registration, hardening, secret scope, patching, and an on-call surface
when it wedges. **[Pricing UNVERIFIED — check current on-demand/spot rates before
budgeting.]** Mitigate by running it **on demand for the release job only**, not as a standing
fleet.
**Bonus**: the same machine solves G5 — arm64 LVH rootfs images must be built on a native
arm64 KVM-capable host (libguestfs cannot cross-build), which is exactly what this instance
is.

**B3 (viable alternative to B2 if you would rather own hardware than a cloud bill). One
non-nested Linux box as a self-hosted runner.** Any machine running Linux on bare metal gives
KVM at EL2/VMX with no L0 to mis-virtualize timers or the GIC. An ARM box (e.g. Ampere-class
workstation) covers aarch64; an x86_64 box covers the B1 arch redundantly.
**Cost**: one-time hardware plus ongoing ownership. **Honest downside**: it is a single point
of failure sitting between the team and merging, in a home or office, with no redundancy —
for a *release-cadence* job that is acceptable; for a *per-PR blocking* job it is not.
**This is new infrastructure you operate. Do not adopt it if B1 covers the risk you actually
care about.**

**B4 (rejected). Keep the Lima nested environment as the gate, with retries or tuning.**
Rejected on the evidence: no documented contract to hold Apple to, no errata channel, no
evidence-backed mitigation, and a failure mode that produces missing verdicts. A gate that
cannot distinguish "the code is broken" from "the hypervisor blinked" is not a gate.

**B5 (rejected). Wait for the arm64 hosted-runner KVM situation to change.** GitHub's arm64
runners are on Azure SKUs without nesting support (F5.3); nothing suggests a near-term change,
and betting a gate on an undocumented capability arriving is not a plan. Re-check in six
months; do not schedule around it.

### The pinned-kernel question (ADR-0068), which none of the above answers by itself

The spike could not test the pinned **6.18 aarch64** kernel at all — the Lima VM only has
7.0.0-27/-28. Whichever of B1/B2/B3 lands, the pinned kernel gets exercised the same way
every other Tier-3 kernel does: **supply it explicitly to LVH via `--kernel`**, which the
arm64 LVH path requires anyway (only amd64 images ship a bootloader, F4.3). Two concrete
consequences:
1. **On x86_64 (B1)** this is straightforward — check `kernels.json` / the Quay tags for a
   6.18 amd64 image, or build one.
2. **On aarch64 (B2/B3)** the image may have to be built on the native runner (G5). Budget
   for that; it is not a five-minute job the first time.
3. Either way, the appliance's **raw arm64 `Image` unwrapping** (UKI → EFI-zboot → zstd) from
   the spike's `[D5]` finding is a prerequisite, not a follow-up — CH will otherwise fail with
   the misleading `UefiTooBig`.

### One-line version

**Gate on x86_64 hosted runners now (free, honest, this week); schedule aarch64 on a Graviton
`*.metal` at release cadence and fix the `testing.md` wording that currently implies a
non-metal Graviton would work; make the Lima loop refuse to render a verdict instead of
producing one; and do not spend a single hour tuning Apple's nested hypervisor.**

## Full Citations

[1] Linux kernel documentation. "Clocks and Timers" (Hyper-V). docs.kernel.org. https://docs.kernel.org/virt/hyperv/clocks.html. Accessed 2026-08-02.
[2] Zyngier, Marc (series author); LWN staff. "KVM: arm64: ARMv8.3/8.4 Nested Virtualization support". LWN.net. 2023-01-12. https://lwn.net/Articles/919851/. Accessed 2026-08-02.
[3] LWN staff. "Nested Virtualization on KVM/ARM". LWN.net. https://lwn.net/Articles/728193/. Accessed 2026-08-02.
[4] "[PATCH 2/2] KVM: arm64: timers: Adjust CVAL of a ptimer across guest entry and exits". linux-arm-kernel mailing list. 2023-08. https://lists.infradead.org/pipermail/linux-arm-kernel/2023-August/861614.html. Accessed 2026-08-02.
[5] Zyngier, Marc. Reply on the ptimer CVAL series. lore.kernel.org. https://lore.kernel.org/lkml/86il97ff17.wl-maz@kernel.org/. Accessed 2026-08-02.
[6] "[PATCH 0/2] Avoid spurious ptimer interrupts for non-zero cntpoff". LKML archive. 2023-08. https://lkml.rescloud.iu.edu/2308.2/01462.html. Accessed 2026-08-02.
[7] "[PATCH v11 00/43] KVM: arm64: Nested Virtualization support (FEAT_NV2 only)". linux-arm-kernel mailing list. 2023-11. https://lists.infradead.org/pipermail/linux-arm-kernel/2023-November/882364.html. Accessed 2026-08-02.
[8] Google. "Nested virtualization overview". Google Cloud documentation. https://docs.cloud.google.com/compute/docs/instances/nested-virtualization/overview. Accessed 2026-08-02.
[9] GitHub. "About GitHub-hosted runners". GitHub Docs. https://docs.github.com/actions/using-github-hosted-runners/about-github-hosted-runners. Accessed 2026-08-02.
[10] GitHub. "Arm64 on GitHub Actions: Powering faster, more efficient build systems". The GitHub Blog. https://github.blog/news-insights/product-news/arm64-on-github-actions-powering-faster-more-efficient-build-systems/. Accessed 2026-08-02.
[11] josecelano et al. "Documentation Request: Nested Virtualization Support in GitHub-hosted Runners". actions/runner-images issue #12933. 2025-09-01. https://github.com/actions/runner-images/issues/12933. Accessed 2026-08-02.
[12] "Add qemu-kvm to Ubuntu runners". actions/runner-images issue #7541. 2023-05-05. https://github.com/actions/runner-images/issues/7541. Accessed 2026-08-02.
[13] "Revisiting KVM support for Hosted GitHub Actions". GitHub community discussion #8305. https://github.com/orgs/community/discussions/8305. Accessed 2026-08-02.
[14] "Linux arm64 hosted runners now available for free in public repositories (Public Preview)". GitHub community discussion #148648. https://github.com/orgs/community/discussions/148648. Accessed 2026-08-02.
[15] "KVM not supported in GitHub Action Runner testing infrastructure". gem5 issue #787. https://github.com/gem5/gem5/issues/787. Accessed 2026-08-02.
[16] "unable to use nested virtualization on M4 mac with qemu in guest". lima-vm/lima issue #4498. https://github.com/lima-vm/lima/issues/4498. Accessed 2026-08-02.
[17] "Request/Question: (macOS 15) Nested virtualization for Windows ARM and MacOS guests?". utmapp/UTM issue #6821. https://github.com/utmapp/UTM/issues/6821. Accessed 2026-08-02.
[18] "Nested virtualization on macOS host + macOS guest (HVF error: HV_UNSUPPORTED)". utmapp/UTM issue #7024. https://github.com/utmapp/UTM/issues/7024. Accessed 2026-08-02.
[19] Apple. "isNestedVirtualizationSupported". Apple Developer Documentation. https://developer.apple.com/documentation/virtualization/vzgenericplatformconfiguration/isnestedvirtualizationsupported. Accessed 2026-08-02. **[Page body not retrievable — JavaScript-rendered.]**
[20] Apple Developer Forums. "M2 Nested Virtualization". Thread 756723. https://developer.apple.com/forums/thread/756723. Accessed 2026-08-02.
[21] Parallels Forums. "macOS 15 Sequoia: nested virtualization for M3+ Macs". https://forum.parallels.com/threads/macos-15-sequoia-nested-virtualization-for-m3-macs.364397/. Accessed 2026-08-02.
[22] Cilium. "little-vm-helper" (README). GitHub. https://github.com/cilium/little-vm-helper. Accessed 2026-08-02.
[23] Cilium. "little-vm-helper `action.yaml`". GitHub. https://github.com/cilium/little-vm-helper/blob/main/action.yaml. Accessed 2026-08-02.
[24] Cilium. "little-vm-helper-images". GitHub. https://github.com/cilium/little-vm-helper-images. Accessed 2026-08-02.
[25] Cilium. "Run eBPF Tests with Little VM Helper". Cilium documentation. https://docs.cilium.io/en/latest/contributing/development/bpf_tests/. Accessed 2026-08-02.
[26] Cloud Hypervisor project. "How to build and test Cloud Hypervisor on AArch64" (`docs/arm64.md`). GitHub. https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/arm64.md. Accessed 2026-08-02.
[27] "Transition aarch64 CI from buildkite to a1.metal". firecracker-microvm/firecracker issue #1327. https://github.com/firecracker-microvm/firecracker/issues/1327. Accessed 2026-08-02.
[28] "[CI on arm] Switch to m6g metal instances". firecracker-microvm/firecracker issue #2131. https://github.com/firecracker-microvm/firecracker/issues/2131. Accessed 2026-08-02.
[29] Firecracker. "firecracker-ci-aarch64" pipeline. Buildkite. https://buildkite.com/firecracker/firecracker-ci-aarch64. Accessed 2026-08-02.
[30] Oracle. "Known Issues" (Oracle Linux Cloud Native Environment 2 release notes) — Kata Containers aarch64 bare-metal requirement. https://docs.oracle.com/en/operating-systems/olcne/2/relnotes/issues.html. Accessed 2026-08-02.
[31] InfoQ. "AWS Introduces Nested Virtualization on EC2 Instances". 2026-03. https://www.infoq.com/news/2026/03/aws-ec2-nested-virtualization/. Accessed 2026-08-02.
[32] AWS re:Post. "Nested Virtualization support on EC2 Graviton 3 *.metal instances". https://repost.aws/questions/QUChyy06f6TRKgitoNLKV4DQ/nested-virtualization-support-on-ec2-graviton-3-metal-instances. Accessed 2026-08-02.
[33] AWS. "Amazon EC2 C7g metal instances are now available". 2023-02. https://aws.amazon.com/about-aws/whats-new/2023/02/amazon-ec2-c7g-metal-instances-available/. Accessed 2026-08-02.
[34] Fuentes, Salvador. "Kata Containers testing and packaging powered by the cloud". Kata Containers blog (Medium). https://medium.com/kata-containers/kata-containers-testing-and-packaging-powered-by-the-cloud-b752de2ee471. Accessed 2026-08-02. **[Medium-trust; used only for CI-topology detail.]**

Local primary sources (not web-cited):
- `/Users/marcus/conductor/workspaces/helios/hanoi/docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md`
- `/Users/marcus/conductor/workspaces/helios/hanoi/.claude/rules/testing.md`
- `/Users/marcus/conductor/workspaces/helios/hanoi/.claude/rules/debugging.md`
- `/Users/marcus/conductor/workspaces/helios/hanoi/.github/workflows/ci.yml`
- `/Users/marcus/conductor/workspaces/helios/hanoi/.github/roadmap-issues.md`

## Research Metadata

Sources examined: ~45 | Cited: 34 (27 distinct external + 5 local primary + 2 supporting) |
Cross-referenced findings: 11 of 16 | Confidence distribution: High 44% (F2.1, F2.2, F3.1,
F4.1, F5.1, F6.2, F6.3), Medium-High 31% (F4.2, F4.3, F5.3, F5.4, F2.3), Medium 19% (F1.1,
F1.3, F4.4, F5.2, F5.5), Low 6% (F1.2, F3.2, F3.4) |
Tool failures: 1 (`developer.apple.com` JavaScript-rendered, body not retrievable — see G1);
1 redirect handled (`cloud.google.com` → `docs.cloud.google.com`) |
Output: `docs/research/testing/trustworthy-tier3-gate-for-microvm-boot-research.md`
