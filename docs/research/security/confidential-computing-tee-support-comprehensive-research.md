# Research: What would be needed for Overdrive to support TEE / confidential computing?

**Date**: 2026-11-07 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium | **Sources**: (in progress)

> STATUS: skeleton created; findings are appended progressively below.

## Executive Summary
Overdrive can reach a CVM launch tier on the existing stack with moderate effort, but a CVM tier that defends against an untrusted operator requires redesigning workload identity and the mesh.

1. **Hardware/kernel readiness is good on x86.** Linux 7.0 has in-tree KVM support for SEV-SNP (`KVM_X86_SNP_VM`) and TDX (`KVM_X86_TDX_VM`), `guest_memfd`, and guest attestation drivers; Arm CCA host support is absent (guest driver only). Cloud Hypervisor already carries `sev_snp` (KVM + MSHV, IGVM, measured boot) and `tdx` features.
2. **The sharpest Overdrive-specific blocker is vsock.** CC guests require `VIRTIO_F_ACCESS_PLATFORM`; Linux vhost-vsock only offers it with a vhost IOTLB the CH backend does not implement, and the fork's docs explicitly reject CVMs for vhost-kernel vsock. ADR-0146's in-kernel data path stays; the fork gains vhost IOTLB support, spiked first.
3. **The host-held SVID/host-kTLS identity model is incompatible with an untrusted-host threat model** (host sees leaf keys and, until #303, plaintext at leg-S). Eleven host-trusted components are enumerated (Part E). In-guest identity bound to attestation (RFC 9334 Verifier → CA as Relying Party) plus #303 is the path.
4. **Threat-model tension is real and must be resolved by the user.** A "tier 0" (CVM memory encryption, host-mediated identity) is coherent against host-bug and physical/forensic adversaries; it is not a defence against a malicious operator. Physical interposer attacks stay out of SNP/TDX scope.
5. **Prior art converges** (CoCo, Contrast): pod-in-CVM, mesh inside the TCB, external owner-run KBS/verifier, image pulled/verified in guest.

Confidence: Medium overall; kernel and CH facts are High (source-read); many vendor-spec facts rest on search summaries (see Knowledge Gaps).

## Research Methodology
**Search Strategy**: repo-first (ADRs, crates, vendored CH + Linux source), then vendor primary docs / RFCs / papers.
**Source Selection**: official + academic + technical docs; trusted-domain list from `.nwave/trusted-source-domains.yaml`. Vendor primary docs not on that list (amd.com, intel.com, arm.com, confidentialcontainers.org, spiffe.io is on list) are flagged as "vendor-primary, off-list".
**Quality Standards**: 3 sources/claim ideal; repo facts cited as `path:line`; kernel claims cited as `path:line @ v7.0`.
**Environment note**: this worktree (`bangalore-v1`) has empty `vendors/` submodule directories. Kernel and Cloud Hypervisor source were read read-only from the sibling checkout `/Users/marcus/conductor/workspaces/helios/wellington-v2/vendors/{linux,cloud-hypervisor}` (linux `Makefile` VERSION=7 PATCHLEVEL=0 SUBLEVEL=0). The fork's exact commit in that sibling was not verified against `overdrive/vhost-kernel-vsock`.

## Findings

Legend: **[REPO]** = verified in this repository's files; **[LINUX-7.0]** = read in vendored Linux source (sibling checkout, see note); **[CH-FORK]** = read in vendored Cloud Hypervisor tree (sibling checkout; fork delta not diffed); **[EXT]** = external source.

### Part A — Verified facts about Overdrive and its vendored components

#### A1. Overdrive has zero confidential-computing code or design today
**Evidence**: Case-insensitive grep for `sev_snp|igvm|tdx|guest_memfd|swiotlb|TSM` over `crates/**`, `infra/**`, ADR-0068 and ADR-0161 returned a single hit in `crates/overdrive-testing/src/vm_fixture.rs` and no matching line (match was on a substring, not a CC feature). `docs/research/` has no file matching `*confiden*|*tee*|*attest*|*sev*|*tdx*`. [REPO]
**Confidence**: High (negative result; grep-scoped). Brief.md mentions "node attestation" only as a multi-node identity item (#36) (`docs/product/architecture/brief.md:7407`, `:7857`). [REPO]

#### A2. Launch contract: `VmConfig` has no platform/firmware/payload axis
**Evidence**: `VmConfig { alloc, kernel: KernelImage, rootfs: RootfsPlan, cmdline: KernelCmdline, memory: MemoryPlan, vcpus, run_dir, confinement, cgroup_scope }` (`crates/overdrive-core/src/vm/config.rs:873-883`). `--kernel` is direct kernel boot; the module doc warns an unloadable `--kernel` is silently reinterpreted as UEFI firmware (`config.rs:23`, `:126`). Memory is `--memory size=<N>M,prefault=on` (`config.rs:291`). The `Vmm` port trait is at `crates/overdrive-core/src/traits/vmm.rs:50`; ADR-0082 makes `VmConfig` the anti-corruption value against CH. [REPO]
**Implication**: A CVM tier needs new, validated config axes (platform=sev_snp|tdx, IGVM/firmware payload, SNP policy, host_data / ID-block) — a *design-contract* change that CLAUDE.md § "Implement to the design" requires DESIGN (ADR + feature-delta) to pin, not a crafter improvisation.

#### A3. Workload identity is host-held; mTLS is host-kernel-mediated today, moving to in-guest (#303)
**Evidence**: CLAUDE.md "Workload identity model": workloads hold no SVID; `IdentityMgr` holds `SvidMaterial` in memory on the worker/control plane, kernel (sockops+kTLS on the host) is the consumer. ADR-0186: leg-S to a VM backend currently relays **plaintext through host userspace** ("Plaintext of VM mesh connections passes through leg-S's userspace until #303"); end state is in-guest kernel mTLS after which "the host forwards ciphertext through the in-kernel forwarder (ADR-0197) and no plaintext crosses the host". [REPO] (`gh issue view 303 --comments` could not be run: the researcher toolset has no shell; see Knowledge Gaps.)
**Confidence**: High for the facts as stated in ADRs.

#### A4. Host-side components that touch guest bytes or guest trust (inventory input for Q4)
**Evidence** [REPO]:
- Beacon channel (guest dials host over vsock, default port 1234) carries `READY/EXEC/EXIT/SHUTDOWN`, planned `LISTEN` (ADR-0268). Host reader trusts the guest's lines for lifecycle facts.
- Flow-shaped channels: stream 1240, control 1243 (ADR-0268 Context; ADR-0217/0218 give the guest its ports via the kernel command line).
- In-kernel flow forwarder module loaded by host image at boot and by `overdrive-init` in the guest (ADR-0205, ADR-0197); guest-side Aya capture + forwarder, `overdrive-init` is PID 1 (ADR-0150).
- KEK for the workload-CA root: kernel keyring, delivered by systemd-creds (host-key/TPM-backed) (ADR-0063 D3, `adr-0063...md:195-208`).
- Image factory / BYO kernel+rootfs: `[vm]` spec kernel/rootfs paths are temporary (project memory; `docs/research/image-factory.md`).
- Rootfs is a host-made reflink clone of a master image attached as a disk (ADR-0082 context; `config.rs:826-845`).
**Confidence**: High (each is an ADR statement); completeness of inventory is Medium (not every ADR read).

#### A5. Linux 7.0 host has KVM support for SEV-SNP, TDX and guest_memfd; Arm CCA host (RME) is absent
**Evidence** [LINUX-7.0]:
- VM types: `KVM_X86_SW_PROTECTED_VM 1`, `KVM_X86_SEV_VM 2`, `KVM_X86_SEV_ES_VM 3`, `KVM_X86_SNP_VM 4`, `KVM_X86_TDX_VM 5` (`arch/x86/include/uapi/asm/kvm.h:969-973`).
- `config KVM_INTEL_TDX` (`default y`, depends on `INTEL_TDX_HOST`) (`arch/x86/kvm/Kconfig:134-144`); `config KVM_AMD_SEV` (`default y`, selects `KVM_GENERIC_MEMORY_ATTRIBUTES`, `HAVE_KVM_ARCH_GMEM_PREPARE/INVALIDATE/POPULATE`, depends on `CRYPTO_DEV_SP_PSP`) (`arch/x86/kvm/Kconfig:156-165`). `config INTEL_TDX_HOST` at `arch/x86/Kconfig:1907`.
- SNP launch commands `KVM_SEV_SNP_LAUNCH_START/UPDATE/FINISH` (`arch/x86/kvm/svm/sev.c:2672-2678`); SNP advertised as supported VM type (`sev.c:2970`). TDX `KVM_TDX_INIT_VM`, `KVM_TDX_INIT_MEM_REGION`, `KVM_TDX_FINALIZE_VM` (`arch/x86/kvm/vmx/tdx.c:2865-3244`).
- `KVM_CREATE_GUEST_MEMFD` (`include/uapi/linux/kvm.h:1643`), `KVM_SET_MEMORY_ATTRIBUTES` / `KVM_MEMORY_ATTRIBUTE_PRIVATE` (`:1632`, `:1641`), `config KVM_GUEST_MEMFD` (`virt/kvm/Kconfig:106`).
- Guest-side attestation drivers: `SEV_GUEST` (`drivers/virt/coco/sev-guest/Kconfig`), `TDX_GUEST_DRIVER`, `ARM_CCA_GUEST`, all selecting `TSM_REPORTS` (configfs report interface); ioctls `SNP_GET_REPORT` (`include/uapi/linux/sev-guest.h:80`) and `TDX_CMD_GET_REPORT0` (`include/uapi/linux/tdx-guest.h:40`).
- No `arch/arm64/kvm/rme*` file exists, and `arch/arm64/kvm/{Kconfig,arm.c}` mention no RME/CCA → Linux 7.0 has the CCA *guest* driver but no CCA *host* KVM support.
**Confidence**: High (direct source reading; Arm negative result is grep/glob-scoped → Medium-High).
**Analysis**: ADR-0068's pinned-kernel rule means these Kconfig symbols are an *image configuration* decision (same class as `CONFIG_NET_HANDSHAKE`, testing.md "Kernel matrix"); KVM_AMD_SEV and KVM_INTEL_TDX default to `y` but need firmware/BIOS enablement (Part G).

#### A6. Cloud Hypervisor tree already contains SEV-SNP (KVM and MSHV) and TDX paths
**Evidence** [CH-FORK]:
- Cargo features `sev_snp` (implies `igvm`) and `tdx` at `cloud-hypervisor/Cargo.toml:51-58`, `vmm/Cargo.toml:14-41`, `hypervisor/Cargo.toml:13-14`.
- Release notes (v53 tree): "AMD SEV-SNP confidential VMs can now be launched on KVM in addition to MSHV. Cloud Hypervisor uses `guest_memfd` to back private memory and loads an IGVM-packaged firmware (e.g. Oak stage0)…"; "Measured-boot support has been completed for KVM SEV-SNP guests… kernel, command line, and initrd are reflected in the launch measurement, and a signed SNP ID block can be passed" (`release-notes.md:657-668`).
- KVM path creates a per-region `guest_memfd` and sets `KVM_MEMORY_ATTRIBUTE_PRIVATE` per region (`hypervisor/src/kvm/mod.rs:1113-1198`); hard-coded bootloader GPA/VMSA GPA with a TODO (`kvm/mod.rs:220-226`).
- Config validation: SEV-SNP requires an IGVM payload (`ValidationError::SevSnpRequiresIgvm`) and `host_data` of exactly 64 bytes (`vmm/src/config.rs:3143-3165`); `--platform sev_snp=on` (`config.rs:894-895, 963`); TDX forbids CPU hotplug and requires firmware (`config.rs:3138`, `:172`, `:287-292`).
- Upstream integration tests for CVM exist: `cloud-hypervisor/tests/integration_cvm.rs` (gated `sev_snp`) incl. `test_virtio_vsock`.
- TDX docs: `docs/intel_tdx.md` — requires "the host OS compiled from the KVM TDX tree" (out-of-tree at the time of writing), TDVF firmware or TDshim direct boot, `--platform tdx=on`; release notes list TDX as an area that "evolves substantially between releases" (`README.md:315`).
- **vhost-kernel vsock (our fork's feature) vs CVM**: `docs/vsock.md:100` — "`iommu=on` is rejected (the backend does not implement the vhost IOTLB), as are confidential VMs that force `VIRTIO_F_ACCESS_PLATFORM`." Snapshot/restore and live migration are refused with vhost-kernel vsock (`vsock.md:101`).
**Confidence**: High for presence of code; Medium for "works on real HW" (only the release notes claim it). The sibling checkout may not be byte-identical to the fork head.
**Analysis (important)**: Upstream CH SEV-SNP on KVM exists but is young (v53-era, IGVM-only, hard-coded layout). TDX in CH relies on historic out-of-tree KVM TDX; Linux 7.0 now carries KVM TDX in-tree so the doc is stale w.r.t. host kernel but the CH TDX code path was not verified against mainline 7.0 UAPI (Knowledge Gap). **The Overdrive-specific blocker is vsock**: a CVM guest's virtio stack requires `VIRTIO_F_ACCESS_PLATFORM` (below), and the fork's vhost-kernel vsock backend rejects it today.

#### A7. Linux guest requires VIRTIO_F_ACCESS_PLATFORM when memory is encrypted; vhost-vsock supports the bit only via IOTLB
**Evidence** [LINUX-7.0]: `drivers/virtio/virtio.c:210-222` — if `virtio_check_mem_acc_cb(dev)` (restricted memory access, set for CC guests) the device **must** provide `VIRTIO_F_VERSION_1` and `VIRTIO_F_ACCESS_PLATFORM`, else probe fails with `-ENODEV`. `arch/x86/mm/mem_encrypt.c:20-25` `force_dma_unencrypted()` returns true under `CC_ATTR_GUEST_MEM_ENCRYPT` ("all DMA must be to unencrypted addresses"), i.e. swiotlb bounce-buffering through shared pages. `drivers/vhost/vsock.c:32-38` lists `VIRTIO_F_ACCESS_PLATFORM` as an offerable feature; `vhost_vsock_set_features` (`vsock.c:839-842`) calls `vhost_init_device_iotlb()` when it is acked — i.e. the VMM must then feed IOTLB updates (`VHOST_IOTLB_MSG`) to vhost.
**Confidence**: High.
**Analysis**: This is the concrete engineering item for the fork: implement the vhost IOTLB message protocol in CH's vhost-kernel vsock backend (and for any vhost-net use) — or keep vsock in userspace virtqueue emulation (loses ADR-0146's in-kernel data path). Not measured: whether vhost workers can read the *shared* (non-guest_memfd) pages via the VMM's userspace mapping with SNP/TDX (Knowledge Gap; needs a spike per `.claude/rules/design.md` § contract-shaping validation).

### Part B — Attestation and trust architecture (external)

#### B1. RFC 9334 gives the vocabulary; Overdrive's SVID issuer maps to Relying Party (+ Verifier)
**Evidence**: Attester "produces Evidence"; Verifier "appraises the validity of Evidence … and produces Attestation Results to be used by a Relying Party"; Reference Values are "known-good values"; passport model (Attester carries the Verifier's result to the RP) vs background-check model (RP forwards Evidence to a Verifier) (RFC 9334 §4.1, §4.2, §5.1, §5.2). Freshness via nonces/epoch IDs; "in many TEEs today, a clock is only available outside the TEE" (§10.1). [EXT: [RFC 9334](https://www.rfc-editor.org/rfc/rfc9334.html), IETF, accessed 2026-11-07]
**Confidence**: High (single normative source; it is the standard).
**Analysis (interpretation)**: In Overdrive terms the guest is the Attester, the node-local or control-plane attestation verifier is the Verifier, and the SVID issuer (workload CA, ADR-0063) is the Relying Party that acts on Attestation Results. The background-check model fits because the issuer must obtain a fresh nonce-bound result per issuance.

#### B2. AMD SEV-SNP evidence: VCEK chain from AMD KDS, 64-byte REPORT_DATA, measurement + ID block + policy
**Evidence**: Report signed by VCEK (ECDSA P-384, signature over bytes 0x00–0x29F at 0x2A0), chain AMD Root (ARK) → SEV CA (ASK) → VCEK, VCEK derived from chip secret + REPORTED_TCB (SP bootloader SVN, SP OS SVN, SNP firmware SVN, microcode); VCEK cert fetched from `kdsintf.amd.com` per chip ID + TCB. 64-byte guest-supplied REPORT_DATA binds nonce/TLS key hash. [EXT: [AMD SNP Firmware ABI spec 56860 rev 1.58](https://www.amd.com/content/dam/amd/en/documents/developer/56860.pdf) (as summarised via search; PDF not read directly); [Edgeless Contrast AMD details](https://docs.edgeless.systems/contrast/1.14/architecture/attestation/amd-details); [arXiv 2608.04039 primer §6](https://arxiv.org/html/2608.04039v1)]
Verifier duties per primer §6.8: "pin the ARK, check the CRL, enforce a minimum REPORTED_TCB, and check REPORT_DATA". Report v5 adds `LAUNCH_MIT_VECTOR/CURRENT_MIT_VECTOR`; PLATFORM_INFO reports SMT, ciphertext hiding, alias-check completion.
**Confidence**: High (3 independent sources: vendor spec via summary, Contrast docs, academic primer). Caveat: AMD PDF itself not opened.
**Operational consequence**: KDS is an online AMD dependency; Contrast docs do not describe caching, so offline/air-gapped appliance operation needs a local collateral cache design (Gap).

#### B3. Intel TDX evidence: DCAP quote, PCK chain via PCS, MRTD + RTMR0–3, verifier must compare measurements itself
**Evidence**: Quote carries MRTD (build-time), RTMR0–3 (runtime), MR_SEAM, TEE TCB SVN; signed via the TD Quoting Enclave and verified against PCK cert chain from Intel PCS/PCCS with TCBInfo + QEIdentity + CRL collateral; "the QVL does not verify … MRCONFIGID and MRTD. The relying party has to compare those itself". Linux exposes quotes via configfs-tsm. [EXT: Intel DCAP Quoting Library API (01.org, via search summary); [arXiv 2303.15540 "Intel TDX Demystified"](https://arxiv.org/pdf/2303.15540); enclaive/dev.to tutorials (medium-trust, used only for corroboration)]. Kernel side [LINUX-7.0]: `TDX_GUEST_DRIVER` selects `TSM_REPORTS`; `TDX_CMD_GET_REPORT0` ioctl (`include/uapi/linux/tdx-guest.h:40`).
**Confidence**: Medium (Intel primary spec not opened; relies on summaries + one academic paper).

#### B4. Measured boot with CH: ID block + `host_data` give a verifier something deterministic to pin
**Evidence**: CH KVM SEV-SNP measured boot "kernel, command line, and initrd are reflected in the launch measurement, and a signed SNP ID block can be passed" ([CH-FORK] `release-notes.md:664-668`). Contrast uses an anonymous-signed ID block so that *no one holds the ID key* ("there exists no private key for the `ID_KEY`") and binds a Kata initdata digest in `HOST_DATA`; REPORT_DATA = digest of relying-party nonce + CVM TLS public key; note its policy lists `ABI_MINOR = 31 on cloud-hypervisor` ([EXT] Contrast docs above). CH validation requires `host_data` of exactly 64 bytes ([CH-FORK] `vmm/src/config.rs:3143-3152`).
**Confidence**: Medium-High.
**Implication**: Overdrive's direct-kernel-boot model (kernel + rootfs + cmdline) maps naturally onto SNP's launch digest *if* the rootfs is covered by dm-verity (or similar) with the root hash on the measured command line. Today the rootfs is a mutable reflink clone (`config.rs:826-845` [REPO]); the cmdline also carries per-VM vsock ports (ADR-0217) — per-VM variable cmdline bytes change the launch digest per alloc, so they must be moved to `host_data`/initdata or covered by verifier policy (design question, §Q).

#### B5. Confidential Containers / Trustee is the closest reusable architecture for "attestation-gated secret release"
**Evidence**: Trustee = KBS (relying party, releases secrets after attestation and policy), Attestation Service (verifier), RVPS (reference-value provider). Two policy layers: attestation policy (AS) and resource policy (KBS). Guest components: attestation-agent, confidential-data-hub, image-rs (pull inside the guest), kata-agent; "Everything else, including the hypervisor, other pods, and the control plane, is outside of the enclave and untrusted." Trustee "should be deployed in a trusted environment … on behalf of the guest owner". Warning from Red Hat pattern: AS default policies are permissive; co-locating KBS with the workloads' own cluster means a compromised cluster could affect both. CoCo also warns: "does not guarantee that any application run with Confidential Containers is confidential or secure." [EXT: [CoCo design overview](https://confidentialcontainers.org/docs/architecture/design-overview/); [CoCo attestation architecture](https://confidentialcontainers.org/docs/attestation/architecture/); [Trustee repo](https://github.com/confidential-containers/trustee)] (domain `confidentialcontainers.org` is off the trusted list; vendor-primary per the task brief; `github.com` is on-list.)
**Confidence**: High.

#### B6. Stateful guest identity: SVSM/vTPM and rollback/cloning
**Evidence**: COCONUT-SVSM runs at VMPL0; vTPM merged but ephemeral (EK regenerated each boot); persistent-state design attests early, fetches a state key from a KBS through a host proxy, and names two malicious-host threats — rollback and cloning — mitigated by "one successful attestation per boot request". [EXT: KVM Forum 2024 slides (Rödel/Hopkins; Garzarella/Steffen, gitlab.com/qemu-project — off-list, vendor/community primary); [Narayanan et al., ACSAC 2023, arXiv 2303.16463](https://arxiv.org/pdf/2303.16463)]. Primer §5.1: "The SVSM runs at VMPL0 and proxies PVALIDATE and VMSA creation for a guest OS at VMPL2"; "Enlightened-guest model: the OS itself runs at VMPL0".
**Confidence**: Medium (conference slides + 1 paper).
**Relevance**: Overdrive guests are "reflink clone of a master rootfs, disposable per allocation" — mostly stateless, so a vTPM/SVSM is *not* required for a first tier; persistent-microVM (`persistent = true`) workloads are where rollback/cloning matter (see `docs/research/platform/persistent-microvm-checkpoint-restore-comprehensive-research.md`, not re-read here).

### Part C — Threat-model reality of current TEEs

#### C1. What SEV-SNP/TDX do and do not protect
**Evidence** (primer §2.1–2.5, §4.7): protects guest memory confidentiality/integrity against the hypervisor, registers encrypted on exit; **out of scope**: DoS ("Availability cannot be enforced against an adversary who controls the power switch"), side channels (cache, branch predictor, interrupt timing, accessed/dirty bits, NPF patterns), physical attacks (interposers, DRAM probing, glitching), and software bugs/backdoored images ("the measurement itself is identity, not behavior"). I/O: "DMA must use shared pages (C=0)… Linux's SWIOTLB implements this transparently for SEV guests"; "Never put secrets in shared pages, and validate everything received through them"; sensitive I/O should still use TLS/LUKS. [EXT: [arXiv 2608.04039](https://arxiv.org/html/2608.04039v1)]
**Corroboration**: Kernel [LINUX-7.0] `arch/x86/mm/mem_encrypt.c:19-25` ("For SEV, all DMA must be to unencrypted addresses"). Interrupt-injection attacks on CVMs: Heckler (arXiv 2404.03387; SEV-SNP and TDX, kernel-patched for `int 0x80`) and WeSee (arXiv 2404.03526; SEV-SNP only because TDX prohibits #VE injection). Physical aliasing/interposer: BadRAM (IEEE S&P 2025), Battering RAM (S&P 2026); the primer says these "confirm rather than refute the placement of physical attacks outside the threat model" and notes AMD firmware boot-time DRAM alias check. [EXT: [Heckler](https://arxiv.org/html/2404.03387v1), [WeSee](https://arxiv.org/html/2404.03526v1), via search results]
**Confidence**: High for the threat-model statement (≥3 sources).

#### C2. Consequence — software-only attacks on TEEs keep appearing; attestation must enforce a minimum TCB and mitigation vector
Interpretation: verifier policy must pin minimum REPORTED_TCB / mitigation vectors, because guest-visible kernel patches (Heckler) and firmware (alias check) are part of the evidence. This means TCB versioning and a rolling firmware pipeline are operational requirements, not nice-to-haves (see Part G).

### Part D — Fit of TEE flavours to a microVM orchestrator

#### D1. Confidential VMs (SEV-SNP, TDX) fit; process enclaves (SGX) and Nitro-style enclaves do not
**Evidence**: Overdrive runs only microVMs under CH (CLAUDE.md "Vendored components"; ADR-0110) [REPO]. CoCo supports "both VM-based and process-based TEEs" but its pod-centric design wraps a whole VM (CoCo design overview). Nitro Enclaves have "no storage, no networking, no interactive access … vsock-only", carved from a parent instance and attested via PCRs to KMS condition keys ([AWS docs](https://docs.aws.amazon.com/enclaves/latest/user/flow.html), accessed 2026-11-07) — an AWS-hypervisor feature Overdrive cannot reuse and whose restricted I/O model does not match general workloads. SGX/process enclaves require application rewrites or library OSes (Enarx/Gramine) — **not researched in this pass** (Knowledge Gap; no source read).
**Confidence**: Medium-High for CVM fit (repo + 2 external); Low for SGX/Enarx comparison (no source).
**Analysis (interpretation)**: A CVM *is* a microVM with different launch flags, so SNP/TDX are the only TEEs that preserve Overdrive's "any Linux workload in a microVM" contract. Arm CCA is out of near-term scope: Linux 7.0 lacks the CCA host (confirmed in tree, A5); the KVM CCA series was still under review at v17 in Sept 2026 ([LKML archive via search](https://ratatoskr.run/kvm/2026/09/17532460/t)), and Overdrive's metal qualification is literal x86_64 only (`infra/metal/native-preflight.sh:15`) [REPO]. SEV-SNP is the most plausible first target because CH already boots it on KVM with measured boot (A6); TDX second (CH TDX docs target an out-of-tree host tree, though Linux 7.0 now has in-tree KVM TDX, A5).

#### D2. Prior art — what goes inside the TCB
| System | Isolation unit | In TCB | Outside TCB | Source |
|---|---|---|---|---|
| Confidential Containers (Kata+CoCo) | pod-in-CVM | workload, kata-agent, image-rs, AA, CDH, guest kernel; Trustee (external, owner-run) | hypervisor, host OS, kubelet/containerd, control plane | CoCo design overview |
| Edgeless Contrast | pod-in-CVM | workload + sidecars (init, **service mesh**) + Coordinator (policy/identity) | cloud infra, cluster operators; even Edgeless via anonymous ID-key | [Contrast docs](https://docs.edgeless.systems/contrast/1.8/basics/security-benefits) |
| Edgeless Constellation (unmaintained) | whole K8s node/cluster | Kubernetes inside CVM | hypervisor/host | Edgeless blog "From Constellation to Contrast": cluster-wide approach puts the cluster operator in the TCB, defeating the multi-party model |
| AWS Nitro Enclaves | carved-out enclave | enclave image, Nitro hypervisor | parent instance (relays via vsock) | AWS docs |
| Azure CVMs (SEV-SNP vTOM) | VM + paravisor (OpenHCL, VMPL/VTL) | guest OS + Microsoft paravisor/vTPM | host | MS Learn Q&A + [Proof of Cloud](https://arxiv.org/html/2510.12469v1) |
| GCP Confidential VM | VM | guest, vTPM measured boot (details from a third-party blog only) | host | **Low confidence** |
| Firecracker / Fly | microVM | n/a — no SEV/TDX in upstream Firecracker; research fork only (SEVeriFast, ASPLOS'24) | | search results; Fly: nothing found |
**Key lesson for Overdrive** (interpretation, supported by Contrast + Constellation rows): the *service mesh/identity proxy must live inside the CVM* or the mesh becomes the host-side plaintext tap. This is exactly what #303 (in-guest kernel mTLS) already points to.

### Part E — Inventory: host-trusted components that a CVM threat model demotes

Method: each row is "what the host can do/see today" (from ADRs read, [REPO]) vs. "what CC changes" (from Parts A–C). The "Resolution" column is a proposal, not a decision.

| # | Component (today) | Why it is host-trusted today | Under an untrusted-host CVM threat model | Resolution direction |
|---|---|---|---|---|
| E1 | Host `IdentityMgr` holding `SvidMaterial` (CLAUDE.md; ADR-0067) | Host mints, holds and supplies SVID keys; workloads hold none | **Fatal**: host sees leaf keys → can impersonate any workload | SVID private key generated *inside* the guest, bound to attestation (REPORT_DATA/CSR); CA signs only after verifying evidence. Inverts today's "workloads hold nothing" rule for the CVM tier |
| E2 | Host sockops + kTLS mTLS (ADR-0069/0071) and TPROXY interception | Host terminates/originates TLS on behalf of workload | Host would see plaintext on leg-F/leg-S; mTLS must terminate in the guest | #303 in-guest kernel mTLS; host forwards ciphertext only (ADR-0186 states this end state) |
| E3 | Leg-S plaintext relay to VM backends (ADR-0186) | Temporary bridge, "plaintext … passes through leg-S's userspace" | Incompatible by construction | Must be gone (#303) before CVM tier |
| E4 | Host DNS/dial-by-name, VIP maps, service map hydrator | Host decides destination | Host can redirect traffic (DoS/misdirection); confidentiality survives only if peers authenticate by SPIFFE ID in-guest | Peer identity pinned in-guest (mTLS w/ attestation-aware trust bundle) |
| E5 | Workload-CA root, KEK in kernel keyring via systemd-creds (ADR-0063) | Host holds the CA that mints identities | CA is the *Relying Party*; if CVM tenant ≠ platform operator, tenant must trust that CA or bring own (see F) | Keep CA operator-trusted in tier 1; per-tenant CA/KBS in tier 2 |
| E6 | Image factory / rootfs master + reflink clone; kernel/cmdline (docs/research/image-factory.md; `config.rs:826-845`) | Host builds and supplies rootfs/kernel | Host can swap rootfs: measurement covers only kernel/initrd/cmdline unless rootfs is integrity-protected | dm-verity root hash on measured cmdline, or initrd-embedded rootfs; publish reference values from image factory (signed) |
| E7 | Beacon channel (vsock 1234), control 1243, stream 1240 (ADR-0268) | Host trusts guest lines for lifecycle (READY/EXIT/LISTEN) | Host-side parsers are the attack surface *from* guest (already hostile-guest tolerant?) and the guest cannot trust the host's replies | Treat all as untrusted shared-memory I/O; guest must not accept host-authored config as security input (e.g. vsock ports on cmdline are fine only if measured or non-security) |
| E8 | Exec/probes into guest, logs, serial console file (`--serial file=`, `config.rs:295`) | Host reads console; probes run via vsock | Console/log output leaves the CVM unencrypted; exec = host-initiated channel into the TCB | Disable serial/debug in CVM tier (SNP policy DEBUG=0); logs via guest-encrypted sink or accept as non-confidential; probes remain (health is not secret) |
| E9 | Host-initiated config injection (cmdline, env, secrets in spec) | Spec is intent in `IntentStore` (host-readable) | Secrets in `AllocationSpec` visible to operator | Secrets only via attested release (KBS-like); spec carries references |
| E10 | Observability: `alloc status`, restart counts, crash reason (ADR-0078) | Host derives from VMM exit | Exit/crash causes are host-observable metadata (not memory) — acceptable leak unless stated otherwise | Document as accepted metadata exposure |
| E11 | Snapshot / checkpoint-restore / live migration (CH docs; persistent microVM) | Host can snapshot RAM | Not possible for encrypted memory in the same way; vhost-kernel vsock already refuses snapshot/migration (`vsock.md:101`) | Persistent CVM = restart from sealed state, not memory snapshot |
**Confidence**: Medium — inventory is derived from ADRs read in this pass; not an audit of all 270+ ADRs.

### Part F — Threat-model tension: what would CC actually serve for Overdrive?

**Repo premise**: CLAUDE.md "Overdrive runs on its own appliance OS": "Do not design for a root-level compromise. An attacker with root on the node owns it", no operator shell, only Overdrive writes kernel state [REPO]. SEV-SNP/TDX *invert* that for guest memory: the CPU, not the host, protects it. These are reconcilable only if the design states exactly which adversary each mechanism addresses.

Candidate adversaries (interpretation):
1. **Platform-operator-as-adversary (tenant ≠ operator)** — e.g. a customer runs Overdrive on someone else's metal, or an Overdrive-hosting provider's staff are untrusted. Needs the full CVM stack: E1–E9 all resolved; attestation, per-tenant KBS. This is the CoCo/Contrast model. Largest scope; contradicts "no root compromise design" *by intent*, so needs an explicit user-approved scope exception (`.claude/rules/design.md` § surface material decisions).
2. **Underlying-infrastructure adversary (bare-metal provider, colo, physical access, cold-boot/DIMM theft, memory forensics)** — memory encryption alone gives value; no attestation chain needed for confidentiality-at-rest-in-RAM, but physical interposer attacks are out of the SEV-SNP/TDX threat model (BadRAM/Battering RAM, primer §2.5), so value is bounded.
3. **Host-software bug / VMM escape / co-tenant** — memory encryption plus RMP/TDX integrity reduces blast radius of a CH compromise (CH is already confined by seccomp/Landlock, ADR-0143). Here the host is *assumed benign but fallible*, consistent with CLAUDE.md ("one Overdrive component's bug"). This is the lowest-friction fit.
4. **Compliance/regulatory** (confidential computing attestations demanded by regulated customers) — partly satisfied by evidence output, requires a verifier/attestation product surface.

**Is a partial tier coherent?** "CVM for memory encryption + attestation-gated secrets" (host still mediates networking) is coherent **only** if:
- the secrets released are *application* secrets delivered to the guest via KBS (E9), and the guest — not the host — originates TLS for them; and
- the SVID/identity path for that tier is guest-held (E1/E2), otherwise the host's `IdentityMgr` still sees keys that protect the same traffic and the tier delivers memory encryption against *physical/forensic* adversaries only.
In other words: **adversary class 3 and 2 are serviceable by a "tier 0" (CVM launch + SNP policy, host-mediated identity)**; adversary class 1 requires E1–E3 (guest-held identity, #303). The "attestation-gated secrets" half is coherent without #303 only for secrets that never transit host-mediated mTLS. Interpretation; confidence Medium (no external source speaks to Overdrive's model).
**CoCo's own caveat** applies: "does not guarantee that any application … is confidential or secure"; Red Hat pattern warns that co-locating the KBS in the same cluster as workloads lets a compromised cluster affect both (B5).

### Part G — Hardware and operations

- **CPUs** (search summaries of NVIDIA/Lenovo guides; vendor-primary AMD/Intel pages not opened): SEV-SNP — EPYC 7003 (Milan), 9004 (Genoa/Bergamo), 8004 (Siena), 9005 (Turin); Milan uses AES-128, Genoa+ AES-256-XTS. TDX — Emerald Rapids (TDX module 1.x) and Granite Rapids/Xeon 6 (2.x); Sapphire Rapids was reported as limited availability (**not confirmed by sources read**; the task brief listed Sapphire Rapids — treat as unverified). Confidence: Medium.
- **BIOS**: SEV-SNP Support = Enabled; SNP Memory (RMP table coverage) = Enabled; IOMMU enabled; RMP ≈ 16 B per 4 KB page ≈ 0.4 % of RAM; verify with `dmesg | grep -i -e rmp -e sev`. TDX: single BIOS enable (+ TDX module via firmware). [EXT: Lenovo Press UEFI guide via search]. Overdrive's appliance-OS rule means these are *node qualification probes* (Earned-Trust style, `brief.md` SD-5) — cf. existing `infra/metal/native-preflight.sh` which is fail-closed for x86_64/KVM and `hypervisor` flag [REPO].
- **Firmware/TCB lifecycle**: AMD advisory SB-3020 (RMP initialization vulnerability) was fixed by SEV firmware/AGESA updates on Genoa/Turin; the VCEK is derived from REPORTED_TCB so any firmware bump changes the signing key and requires a new KDS fetch; the verifier must enforce a minimum TCB (primer §6.8). Operationally: BIOS/firmware updates become part of the appliance image/ops story, which today is purely kernel+image (ADR-0068) [REPO].
- **The metal host**: `infra/metal/README.md:184` mentions an "EM-I120E-NVMe (EPYC)" offer (UEFI). Whether that EPYC generation supports SNP, and whether the provider's BIOS exposes the SNP toggles, was **not established** (Gap). No nested virtualization applies: SEV-SNP/TDX run on literal bare metal, matching Overdrive's no-nesting stance; CoCo notes "A confidential guest generally cannot be started inside of a non-confidential guest."
- **Memory semantics** [LINUX-7.0]: `guest_memfd` mappings are `mapping_set_inaccessible` and must be unevictable/unmovable (`virt/kvm/guest_memfd.c:593-596`). Implications (interpretation, unverified): no host swap/KSM/THP-compaction for CVM RAM; `memory.max = memory_bytes + reserve` (brief.md SD-4) still bounds but the "prefault" / RSS-based reserve measurement (A-2) does not transfer; whether guest_memfd pages are charged to the VMM's memcg was not verified in source (Gap).
- **Performance**: all virtio I/O bounces through shared swiotlb pages (primer §3.3.2; `mem_encrypt.c:19-25`), so vsock and tap throughput regress by an *unmeasured* amount. ADR-0186 already measured a −12–37 % vsock client→guest bulk penalty for the *non-CC* relay — CC cost would stack on that. No published benchmark was gathered here (Gap; needs a spike on metal).

### Part H — Gap list and phased roadmap (proposals only; no issues created)

Per `.claude/rules/design.md` ("No unrequested scope — agents propose, the user adds") each item below is a **proposal**; the reachable-failure justification is the threat classes in Part F.

| ID | Component | Change | Effort | Risk | Notes / evidence |
|---|---|---|---|---|---|
| G1 | Contract-shaping spike: vhost-kernel vsock + SNP guest | Does vhost-vsock work with shared pages and `ACCESS_PLATFORM`+IOTLB under SNP on KVM 7.0? | S–M | **High** (could invalidate ADR-0146 in-kernel data path) | A7; `vsock.md:100` |
| G2 | CH fork: vhost IOTLB support in vhost-kernel vsock backend (and vhost-net if used) | Implement IOTLB msg protocol, drop the `iommu=on`/CVM rejection | M–L | High | `vsock.md:100`; `vhost_vsock_set_features` `vsock.c:839-842` |
| G3 | CH fork: rebase/verify `sev_snp` (+`igvm`) feature builds with our fork's vsock/Landlock/seccomp changes | Build feature, run `integration_cvm.rs` subset on SNP metal | M | Med | A6; `virtio-devices/src/seccomp_filters.rs` has sev_snp entries |
| G4 | `VmConfig`/`Vmm` contract | New validated axes: platform (none\|sev_snp\|tdx), IGVM/firmware payload, SNP policy, `host_data`/ID block, debug-off; per-platform refusals (no balloon/hotplug/snapshot/serial-file). Needs ADR + feature-delta (DESIGN) | M | Med | A2; CH validation errors at `config.rs:3143-3165` |
| G5 | Guest firmware/boot artifact | IGVM stage0 (e.g. Oak/COCONUT-SVSM) packaging in image factory; measured direct-boot of kernel+initrd+cmdline; dm-verity rootfs root hash | L | Med | B4, E6; hard-coded layout TODO in CH (`kvm/mod.rs:220-226`) |
| G6 | Guest kernel config (appliance guest kernel) | `AMD_MEM_ENCRYPT`, `SEV_GUEST`/`TDX_GUEST_DRIVER`, `TSM_REPORTS`, swiotlb sizing, virtio-vsock/net; note ADR-0150 guest uses Aya + in-kernel forwarder module built per kernel (ADR-0203) → module must build for CC guest kernel | M | Med | A5, ADR-0203/0205 |
| G7 | Host kernel/image config & node probe | Ensure `KVM_AMD_SEV`, `CRYPTO_DEV_SP_PSP`, `KVM_INTEL_TDX`/`INTEL_TDX_HOST` in pinned config; Earned-Trust probe: SNP/TDX enabled, RMP initialised, firmware ≥ minimum TCB | S–M | Low | A5, Part G |
| G8 | Attestation verifier ("Overdrive Verifier") | RFC 9334 Verifier: SNP report verification (ARK pin, VCEK via KDS w/ local cache, CRL, min TCB, policy, MEASUREMENT vs RVPS), TDX quote via DCAP QVL/`dcap-qvl`; reference values from image factory signing | L | Med | B1–B3 |
| G9 | Identity (SVID) changes | CVM tier: guest generates key, CSR + REPORT_DATA binding; CA (RP) issues SVID after verifier result; SPIRE-style selector (`amd_sev_snp/chip_id/measurement`) as prior art; reverses "workloads hold nothing" for CVM tier | L | High | E1, E5; ACM SPIRE papers |
| G10 | Networking | Depend on #303 (in-guest mTLS); delete leg-S plaintext relay for CVM allocs; guest owns peer verification | L (mostly #303) | High | E2–E4, ADR-0186 |
| G11 | Secret release (KBS-like) | Resource policy keyed on attestation claims; started on node-local or external Trustee (CoCo Trustee is reusable, but bringing a Rust/Go dependency + its permissive defaults warning) | M–L | Med | B5 |
| G12 | Observability/ops limits | Disable serial file & debug; define what logs/exec exist; document metadata leaks; alloc status shows `confidential: {platform, tcb, verifier-verdict}` | M | Low | E8, E10 |
| G13 | Lifecycle semantic | No snapshot/restore/migration for CVMs; `persistent=true` + CVM needs sealed-state design (SVSM vTPM or KBS-keyed disk) | M–L | Med | B6, `vsock.md:101` |
| G14 | Test strategy | CC tests only meaningful on SNP/TDX metal (non-nested): new metal lane; DST: attestation verifier is pure (SimClock freshness); Quint spec for attest→issue ordering (concurrent protocol) | M | Med | testing.md metal rules |

**Phasing (interpretation)**:
- **Phase 0 — qualification (S)**: confirm metal CPU/BIOS supports SNP (Gap); run G1 spike on 7.0 with CH `sev_snp` + a stock vsock path to learn the real blocker.
- **Phase 1 — "Confidential launch" tier (M–L)**: G3, G4, G5, G6, G7, G12 + G2 (or fall back to userspace virtio vsock for CVM, rejected by ADR-0146 so would need an ADR). Value: memory encryption vs host bugs/physical forensics (adversaries 2,3). Identity stays host-held → **explicitly document that it does not defend against an adversarial operator**.
- **Phase 2 — attested identity (L)**: G8, G9, G10 (requires #303 landed), G11. Value: adversary 1 for workload-to-workload traffic.
- **Phase 3 — stateful/persistent & multi-party (L)**: G13, per-tenant CA/KBS, SVSM/vTPM, optional TDX second backend, Arm CCA when host KVM merges.

### Open questions for the user (outcome-changing)
1. Which adversary (F-1 operator, F-2 physical, F-3 host-bug, F-4 compliance) is the target? Phase 1 vs Phase 2 depends entirely on this.
2. Is a documented *scope exception* to "no root-compromise design" acceptable for a CVM tier?
3. AMD-first or Intel-first, given the actual metal fleet?

## Source Analysis
| Source | Domain | Reputation | Type | Access | Cross-verified |
|---|---|---|---|---|---|
| Linux v7.0 source (sibling checkout) | kernel.org (vendored) | High | official | 2026-11-07 | Y (multiple files) |
| Cloud Hypervisor tree + docs (sibling checkout) | github.com (vendored) | Medium-High | technical | 2026-11-07 | Y (code+docs+release notes) |
| Overdrive ADRs/CLAUDE.md/crates | repo | High (authoritative for repo facts) | internal | 2026-11-07 | Y |
| RFC 9334 | rfc-editor.org | High | official | 2026-11-07 | N (single normative) |
| arXiv 2608.04039 SEV-SNP primer | arxiv.org | High | academic | 2026-11-07 | Y (Contrast, AMD spec summary) |
| arXiv 2404.03387 Heckler, 2404.03526 WeSee, 2303.15540, 2303.16463, 2510.12469 | arxiv.org | High | academic | 2026-11-07 | Partly (via search summaries, not fully read) |
| AMD SNP ABI 56860 | amd.com | High (vendor primary; off-list) | official | 2026-11-07 | via search summary only |
| Intel DCAP API doc | 01.org / intel | High (vendor primary; off-list) | official | 2026-11-07 | via search summary only |
| CoCo docs / Trustee | confidentialcontainers.org, github.com | High (CNCF project; off-list domain) | official | 2026-11-07 | Y (design overview + attestation arch + search) |
| Edgeless Contrast/Constellation docs | docs.edgeless.systems | Medium-High (vendor, commercial interest) | industry | 2026-11-07 | Y with CoCo |
| AWS Nitro Enclaves docs | docs.aws.amazon.com | High | official | 2026-11-07 | N |
| ACM SPIRE+SEV-SNP papers | dl.acm.org | High | academic | 2026-11-07 | via search summary |
| LKML archive (Arm CCA v17) | ratatoskr.run / lkml.iu.edu | Medium (mirror) | technical | 2026-11-07 | N; off-list |
| Lenovo Press, NVIDIA CC guide (BIOS/CPU) | lenovopress.lenovo.com, docs.nvidia.com | Medium-High (vendor; off-list) | technical | 2026-11-07 | via search summary |
| KVM Forum 2024 slides (COCONUT-SVSM) | gitlab.com/qemu-project | Medium-High (off-list) | technical | 2026-11-07 | Y with ACSAC paper |
| dev.to / enclaive / third-party blogs | dev.to etc. | Medium | corroboration only | 2026-11-07 | used sparingly |
Reputation: High ≈ 60 %, Medium-High ≈ 30 %, Medium ≈ 10 %. Many external claims rest on **search-result summaries**, not full page reads, as flagged.

## Knowledge Gaps
1. **`gh issue view 303 --comments` not run** — researcher toolset has no shell; ADR-0186 and c4-diagrams only. Recommend orchestrator pastes #303 comments.
2. **Vendored submodules empty in this worktree**; sibling checkout used. Fork delta vs upstream v53 and exact fork commit not diffed.
3. **Not verified on hardware**: CH SEV-SNP boot on our kernel 7.0; vhost-vsock with shared pages under SNP; CH TDX against mainline 7.0 KVM TDX UAPI.
4. **No CC performance data** for virtio-vsock/tap with swiotlb (searched: only qualitative statements).
5. **SGX/Enarx/Gramine** comparison not researched. **Fly.io confidential** — nothing found.
6. **Metal host EPYC generation / BIOS SNP exposure** unknown.
7. **guest_memfd memcg accounting and interaction with cgroup `memory.max`/prefault reserve** not verified in source.
8. **TDX details** (TD quote, Intel Trust Authority, TDX Connect), **GCP** primary docs, **Arm CCA** attestation (CCA token/EAT) not read from primary sources.
9. AMD/Intel primary specs not opened directly (search summaries only); KDS caching/offline mirroring design not researched.

## Conflicting Information
- **TDX host availability**: CH `docs/intel_tdx.md` says TDX needs the out-of-tree "KVM TDX tree"; Linux 7.0 has in-tree `KVM_INTEL_TDX` (`arch/x86/kvm/Kconfig:134`). Assessment: the CH doc is stale relative to mainline; source code in 7.0 is authoritative for the host, but CH's TDX code path against the mainline UAPI is unverified.
- **TDX CPU generation**: task brief lists Sapphire Rapids+; Lenovo/NVIDIA guides (via search) list Emerald Rapids as first broadly supported. Assessment: treat Emerald Rapids+ as the safe floor until Intel primary docs are checked.
- **Attestation report version**: Contrast docs mention VERSION 3, older doc 0x02, primer v5 — evolving ABI; verifier must handle versions by CPU family (primer §6.2.2).

## Recommendations for Further Research
1. G1 spike (vsock under SNP on KVM 7.0) per `.claude/rules/spike.md`; it decides Phase 1 scope.
2. Read AMD 56860 and Intel TDX module/DCAP specs directly; capture exact report/quote fields for the verifier contract.
3. Measure swiotlb cost on metal for vsock + tap.
4. Research SPIFFE/SPIRE hybrid attestation and "attested TLS" (report in SVID extension) as alternatives to CA-side verification.
5. Research Enarx/Gramine/SGX for completeness; research Intel Trust Authority and AMD KDS offline caching.

## Full Citations
[1] IETF. "RFC 9334: Remote ATtestation procedureS (RATS) Architecture". rfc-editor.org. 2023. https://www.rfc-editor.org/rfc/rfc9334.html. Accessed 2026-11-07.
[2] AMD. "SEV Secure Nested Paging Firmware ABI Specification (56860, rev 1.58)". https://www.amd.com/content/dam/amd/en/documents/developer/56860.pdf. Accessed 2026-11-07 (via search summary).
[3] Asad, McClurg. "AMD SEV-SNP: A Confidential Computing Primer". arXiv 2608.04039. https://arxiv.org/html/2608.04039v1. Accessed 2026-11-07.
[4] Edgeless Systems. "Contrast: AMD SEV-SNP attestation details (v1.14)". https://docs.edgeless.systems/contrast/1.14/architecture/attestation/amd-details. Accessed 2026-11-07.
[5] Confidential Containers. "Design Overview". https://confidentialcontainers.org/docs/architecture/design-overview/. Accessed 2026-11-07.
[6] Confidential Containers. "Attestation architecture"; Trustee repo. https://confidentialcontainers.org/docs/attestation/architecture/ ; https://github.com/confidential-containers/trustee. Accessed 2026-11-07 (search summary).
[7] Schlüter et al. "Heckler". arXiv 2404.03387; "WeSee". arXiv 2404.03526. Accessed 2026-11-07 (search summary).
[8] Narayanan et al. "Remote attestation of SEV-SNP confidential VMs using e-vTPMs". ACSAC 2023, arXiv 2303.16463. Accessed 2026-11-07 (search summary).
[9] Pontes et al. "Attesting AMD SEV-SNP Virtual Machines with SPIRE". LADC 2023, ACM 10.1145/3615366.3615419; "Multi-Platform and Vault-Free Attestation of Confidential VMs", ACM 10.1145/3697090.3698036. Accessed 2026-11-07 (search summary).
[10] Cheng et al. "Intel TDX Demystified". arXiv 2303.15540. Accessed 2026-11-07 (search summary).
[11] AWS. "Nitro Enclaves workflow". https://docs.aws.amazon.com/enclaves/latest/user/flow.html. Accessed 2026-11-07.
[12] Linux v7.0 source, files cited inline. Vendored (sibling checkout). Accessed 2026-11-07.
[13] Cloud Hypervisor tree, files cited inline. Vendored (sibling checkout). Accessed 2026-11-07.
[14] Overdrive repo: CLAUDE.md; ADR-0063, 0067, 0082, 0150, 0186, 0205, 0268; `crates/overdrive-core/src/vm/config.rs`; `infra/metal/*`. Accessed 2026-11-07.
[15] "Proof of Cloud". arXiv 2510.12469. https://arxiv.org/html/2510.12469v1. Accessed 2026-11-07 (search summary).
[16] Arm CCA KVM series v17, LKML mirror. https://ratatoskr.run/kvm/2026/09/17532460/t. Accessed 2026-11-07.

## Research Metadata
Examined ≈ 45 sources (files + web) | Cited ≈ 30 | Cross-refs: SNP evidence (3), CVM/virtio constraint (kernel + CH docs + primer), CoCo/Contrast (2) | Confidence: High 40 %, Medium 50 %, Low 10 % | Output: docs/research/security/confidential-computing-tee-support-comprehensive-research.md
