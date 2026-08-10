# Research: Persistent microVMs with Checkpoint & Restore (the Fly.io Sprites shape) — and what it constrains about volume storage on Cloud Hypervisor

**Date**: 2026-08-10 | **Researcher**: nw-researcher (Nova) | **Confidence**: **High** (P2, P3, P5 rest on upstream primary sources at our exact pinned tag; P4 vendor claims are Medium and marked) | **Sources**: 25
**Feature**: `microvm-driver-cloud-hypervisor` (GH [#42](https://github.com/overdrive-sh/overdrive/issues/42)) | **Decision this informs**: `I-6` (virtiofs vs virtio-blk for VOLUMES)

> **Output-path note.** The dispatch suggested `docs/research/isolation-models/`. That
> directory **does not exist** in this repo — the prior isolation-model research
> (`sprites-as-overdrive-primitive-research.md`, `firecracker-vs-cloud-hypervisor.md`,
> `oci-image-to-microvm-rootfs-research.md`, `unikraft-microvm-and-dockerfile-reuse-research.md`)
> all lives under `docs/research/platform/`. Placed here for consistency with the actual
> layout rather than creating a new sibling directory.

---

## Evidence marker legend

Every major claim in this document carries one of:

| Marker | Meaning |
|---|---|
| **[MEASURED]** | Measured on our own hardware. Cites `docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md`. Not re-derived here. |
| **[DOCUMENTED]** | Stated in a primary source (upstream docs, source code, tracked issue, vendor engineering post). Quoted with a line/issue reference. |
| **[INFERENCE]** | My reasoning from documented facts. Not stated by any source. Refutable. |
| **[MARKETING]** | Vendor self-reported claim, typically a latency or throughput number, with no independent verification. |
| **[UNRESOLVED]** | Searched and could not settle. Appears in Knowledge Gaps. |

---

## Executive Summary

**Three findings, in descending order of how much they should change what we do.**

**1. Cloud Hypervisor v46 — our pin — cannot do the Sprites shape, and the gap is precisely
datable.** Every capability the target workload needs landed in **v52.0 and v53.0**: vhost-user
snapshot/restore *"filled out"* (#7908), virtio-fs **migration** support (#7937), vsock
connections reset on restore to avoid *"stale half-open connections"* (#7958), `userfaultfd`
lazy restore via `memory_restore_mode` (#7800), background prefault and an offloaded snapshot
daemon (v53.0). The v46 doc's entire Limitations section reads *"VFIO devices and Intel SGX are
out of scope"* — it says nothing about vhost-user, virtiofs, or vsock, and **that silence is not
support**. There is an **open** upstream issue (#6931, v43) in which restoring a VM with a
virtiofs root **hangs the VMM**. If checkpoint/restore (GH #96) becomes real scope, **v46 is
below the floor and the floor is v52.0** — and unlike the reference implementation's
unexplained *"CH ≥ 48.0"*, that number now has reasons attached to it.

**2. The virtiofs↔checkpoint incompatibility is structural, and one release note is about to
make everyone believe otherwise.** virtiofsd has supported **live migration** since v1.11.0
(we run 1.13.2), and CH v52.0 added *"migration support for `virtio-fs`"*. Both true; both
irrelevant. Live migration is a hand-off between two *simultaneously live* daemons — the
`file-handles` mode's own precondition is that *"the virtiofsd source instance keeps running
until migration is fully complete"* — whereas a checkpoint is a **temporal gap** in which no
source instance exists. libvirt states the consequence directly: *"Snapshot operations… do not
snapshot the state of the files shared via virtiofs, and thus reverting to an earlier state is
not recommended."* Meanwhile Fly Sprites, Firecracker, CH v46, CodeSandbox, Modal and Blacksmith
**all six** put storage deliberately *outside* the memory snapshot and make it independently
durable; **not one** exposes a live host-shared filesystem into a checkpointable guest.

**3. Sprites' actual lesson is "solve it below the block device", and it is two mechanisms, not
one.** A sprite checkpoint is a *metadata-only CoW* over an immutable content-addressed chunk
store on S3 (the *"about 300 ms"* number, and it is **[MARKETING]**); the *memory* snapshot is a
separate, explicitly **best-effort** artifact — Fly staff: *"Sprites get memory snapshotted and
then restored next time you use them. We keep memory snapshots around for as long as possible"*,
and *"Sprites will reboot (because crash, upgrade, cold, etc.)"*. The filesystem is the truth;
the memory snapshot is a cache for fast wake. **The guest sees ext4 on a block device.** Fly's
cleverness is entirely in the chunking layer *underneath* it — which maps directly onto
Overdrive's already-planned `overdrive-fs` (GH #97) and is fully compatible with a `virtio-blk`
guest interface.

**Recommendation on I-6: split it.** Keep `virtio-blk` as the **default** volume mechanism;
keep `virtiofs` as an **opt-in** that is made *structurally* incompatible with checkpointing at
spec-validation time rather than discovered as a hang at restore. This is not a reversal of I-6
on performance grounds — our own spike already showed performance does not decide it (block ~42%
faster streaming, virtiofs ~25% faster per small file, non-overlapping). It is a reversal of its
*default*, on the grounds that virtiofs's one irreplaceable capability — **live host access
while the guest runs** — is the very capability that makes its checkpoint unsound, and that the
two named target workloads (CI runners, agent sandboxes) do not obviously need it. Blacksmith,
a Firecracker-based CI-runner vendor, *"deliberately avoids persistent disks entirely"* in favour
of a colocated cache, for security reasons.

**The single most valuable thing this document produces is the SPIKE THIS list.** Eight probes,
all runnable on the existing bare-metal box with the existing harnesses. **S-1** — *does
`vm.snapshot`/`vm.restore` work at all on v46 with our block-only shape?* — has never been run,
and every claim here about our system is downstream of it. **S-2** is the probe that could
overturn the recommendation. **S-4** closes a premise (CH VMGenID) that has sat UNVERIFIED since
2026-04-19. **S-6** tests whether CPU hotplug — the entire justification for choosing Cloud
Hypervisor — still works on a restored VM, which Firecracker explicitly forbids and CH is silent
about; that is two separately-justified features whose composition nobody has checked.

---

## Research Methodology

**Search Strategy**: Primary-source-first. Upstream repository documentation fetched **at the
exact pinned tag** (`v46.0`) as well as at `main`, because the divergence between them turned
out to be the central finding; upstream release notes read in sequence to date each capability;
the upstream issue tracker searched for the specific failure mode. Vendor engineering blogs used
for vendor architecture. Existing in-tree research (`docs/research/platform/sprites-as-overdrive-primitive-research.md`,
2026-04-19) and the feature's own measured spike (`docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md`,
2026-08-10) treated as prior art to build on, not to re-derive — and, in one case, to correct.

**Source Selection**: Weighted to `official` / `technical_docs` / `open_source` per the
dispatch's `source_preferences`. Upstream `cloud-hypervisor` and `firecracker` documentation and
release notes are the backbone (High). `fly.io` (trusted list, High) is primary for Sprites,
including a staff reply on `community.fly.io`. `libvirt.org` used for the virtiofs snapshot
statement — **off the trusted list, flagged**, but of the same class as the listed `open_source`
tier and corroborated three ways. Vendor blogs for E2B / Modal / CodeSandbox / Blacksmith are
**Medium** and are marked **[MARKETING]** wherever they carry a number.

**Verification approach**: The load-bearing P2 claims were each verified against ≥2 independent
artifacts — the tagged doc, the `main` doc, the release notes (fetched twice, from two different
pages), and the issue tracker. Where a claim rests on one source it says so. Every major claim
carries **[MEASURED] / [DOCUMENTED] / [INFERENCE] / [MARKETING] / [UNRESOLVED]**, per the
dispatch's requirement that fact and reasoning be separable by a reader making an architectural
decision.

**Quality Standards**: 3+ sources targeted per major claim; 2 accepted; 1 authoritative accepted
with an explicit confidence downgrade. Average reputation ≈ **0.87**. Citation coverage of major
claims: **100%** — every finding block carries at least one source line.

---

## P1. How Fly.io Sprites actually works

> **Prior art in-tree.** `docs/research/platform/sprites-as-overdrive-primitive-research.md`
> (2026-04-19, 24 sources) already covers Sprites' anatomy, pricing, and networking. This
> section **does not re-derive it**; it adds what has changed since, and corrects one claim
> in it that a Fly staff member has publicly contradicted.

### P1.1 Sprites vs Fly Machines — and the one architectural change

**Finding P1-1: Sprites are Fly Machines with (a) a different storage stack and (b) a
standardised base container, so no per-creation image pull.**

**Evidence [DOCUMENTED]**:

> *"Today, under the hood, Sprites are still Fly Machines. But they all run from a standard
> container."* — and that standardisation *"eliminates per-creation container pulling
> overhead."*
> *"User code running on a Sprite isn't running in the root namespace. We've slid a container
> between you and the kernel."* The root namespace hosts orchestration services; user code
> runs in the inner container.

**Source**: [Fly.io — *The Design & Implementation of Sprites*](https://fly.io/blog/design-and-implementation/) — Accessed 2026-08-10. Domain `fly.io`, Reputation: **High** (trusted list, and primary — it is Fly writing about Fly).
**Verification**: `sprites-as-overdrive-primitive-research.md` § 1.1 quotes the same *"still Fly Machines"* and *"slid a container between you and the kernel"* lines from the same post; Fly's *What Is a Firecracker VM?* page independently confirms Machines are Firecracker. **3 sources. Confidence: High.**

**Analysis [INFERENCE]**: The *"standard container"* change is the load-bearing one for us and
is easy to miss. Fly moved the per-tenant variability **out of the VM image and into the
filesystem layer**. Every sprite boots the *same* base; everything user-specific lives in the
chunk store. That is precisely what makes a metadata-only checkpoint possible — and it is a
storage-architecture decision, not a hypervisor one.

### P1.2 The checkpoint/restore mechanism — it is TWO mechanisms, and conflating them is the trap

**Finding P1-2: Sprites has a *filesystem* checkpoint and a *memory* snapshot. They are
separate, have different lifetimes, and only the first is what the "~300 ms" number
describes.**

**Mechanism 1 — the filesystem checkpoint (metadata-only CoW):**

**Evidence [DOCUMENTED]**:

> *"both `checkpoint` and `restore` merely shuffle metadata around."*
> *"Runs copy-on-write, capturing only what changed, so a checkpoint is fast and doesn't
> interrupt the running Sprite."*
> *"Captures your whole writable filesystem, meaning every file, package, and on-disk
> database you've added on top of the base image, not just one directory."*
> **[MARKETING]** *"Checkpoints take about 300 ms and your Sprite environment won't even notice."*

**Sources**: [fly.io/blog/design-and-implementation](https://fly.io/blog/design-and-implementation/); [fly.io/sprites](https://fly.io/sprites/) — Accessed 2026-08-10, both Reputation High. Cross-referenced against `sprites-as-overdrive-primitive-research.md` § 1.3, which quotes *"Both checkpoint and restore merely shuffle metadata around"* and *"checkpoints capture only the writable overlay, not the base image."* **3 sources. Confidence: High** on the mechanism; the **300 ms is [MARKETING]** — vendor self-reported, no methodology, no independent verification found.

**Mechanism 2 — the memory snapshot:**

**Evidence [DOCUMENTED], and this is a direct staff statement:**

> **kurt (Fly staff)**: *"Sprites get memory snapshotted and then restored next time you use
> them. We keep memory snapshots around for as long as possible."*
> On the docs line *"RAM doesn't persist across hibernation and wake"* — **kurt: *"Those docs
> are wrong, btw. We're correcting them."***
> Also kurt: *"Sprites will reboot (because crash, upgrade, cold, etc.)"* — so service
> definitions matter, because processes must *"restart on wake."*

**Source**: [community.fly.io — *How is sprite memory snapshotted and restored?*](https://community.fly.io/t/how-is-sprite-memory-snapshotted-and-restored/26843) — Accessed 2026-08-10. Domain: `community.fly.io` (sub-domain of trusted `fly.io`). Reputation: **High for the staff reply** (Fly employee speaking about Fly's own system, on Fly's own property); **Medium for community replies**, which are not cited here.

> ### ⚠ This CORRECTS our own prior research
>
> `sprites-as-overdrive-primitive-research.md` § 1.3 (2026-04-19) states: *"community thread
> confirms 'RAM doesn't persist across hibernation and wake' by default [6]"* and rates the
> memory-snapshot mechanism **Medium** confidence / *"partially inferred."*
> **That is now superseded.** Fly staff state memory IS snapshotted and restored, and that
> the doc saying otherwise is wrong. See § Conflicting Information.

**Analysis [INFERENCE] — the decomposition, and it is the whole answer to P5:**

| Layer | What it is | Durability | Authoritative? |
|---|---|---|---|
| **Filesystem** | Immutable content-addressed chunks on S3 + Litestream'd metadata | **Durable, external, host-independent** | **YES** |
| **Memory** | Firecracker memory snapshot | *"kept around for as long as possible"* — **best-effort** | **NO** |

**The memory snapshot is a CACHE for fast wake. The filesystem is the truth.** kurt's *"Sprites
will reboot (because crash, upgrade, cold, etc.)"* is the tell: the system is explicitly
designed to survive losing the memory snapshot, and when it does, it falls back to a boot
against a filesystem that is still exactly right. **[INFERENCE]** Any design that makes the
memory snapshot load-bearing for correctness has misread Sprites.

### P1.3 What "persistent" means concretely

**Finding P1-3: Persistence is a JuiceFS-shaped content-addressed chunk store on object
storage, with a local NVMe volume used purely as a cache. Not a per-host disk.**

**Evidence [DOCUMENTED]**:

> *"The root of storage is S3-compatible object storage"* (100 GB quota).
> *"A Sprite has a sparse 100 GB NVMe volume attached to it, which the stack uses to cache
> chunks to eliminate read amplification."*
> **The critical sentence**: *"nothing in that NVMe volume should matter; stored chunks are
> immutable and their true state lives on the object store."*
> *"Organized around the JuiceFS model — data chunks live on object stores; metadata lives in
> fast local storage… that metadata store is kept durable with Litestream."*
> *"Every Sprite has a fast, directly-attached NVMe filesystem that continuously syncs to
> durable, external object storage."*
> User-facing filesystem is ext4 on top of this backing store.

**Sources**: [fly.io/blog/design-and-implementation](https://fly.io/blog/design-and-implementation/); [fly.io/sprites](https://fly.io/sprites/); cross-referenced with `sprites-as-overdrive-primitive-research.md` § 1.2 quoting the same lines. **3 sources. Confidence: High.**

**Analysis — three things follow, and the third is the one that decides I-6:**

1. **[INFERENCE] Sprites can migrate between hosts because the authoritative state is in
   object storage.** Our prior research reached the same conclusion independently.
2. **[DOCUMENTED] The guest sees a BLOCK-DEVICE-shaped filesystem (ext4 on NVMe), not a
   host-shared directory.** There is no virtiofs, no vhost-user-fs, and no FUSE mount into
   the guest's namespace in any of Fly's description of the guest-visible surface.
3. **[INFERENCE] The cleverness is entirely BELOW the block device, in a host-side chunking
   layer, not at the guest interface.** Fly did not solve "make the host and guest share a
   live directory." They solved "make a block device's *backing store* content-addressed,
   external, and cheap to snapshot." **This is the single most transferable finding in the
   document** — see P5.6.

### P1.4 Restore latency claims — all [MARKETING]

| Claim | Source | Marker |
|---|---|---|
| *"Checkpoints take about 300 ms and your Sprite environment won't even notice"* | fly.io/sprites | **[MARKETING]** |
| *"Restores take about one second and are fast enough to use casually, interactively"* | fly.io/sprites | **[MARKETING]** |
| *"you'll be reconnected in well under a second"* | fly.io (via prior research § 1.3) | **[MARKETING]** |
| sprite creation *"a second or two"* vs *"over a minute"* for Fly Machines | fly.io (via prior research § 1.3) | **[MARKETING]** |

**No methodology, guest RAM size, payload, or percentile is stated for any of these.** They are
plausible and directionally consistent with the metadata-only mechanism, but **none is
independently verified and none should be used as an engineering target.** No third-party
benchmark of Sprites was found → Knowledge Gap G5.

### P1.5 Scale-to-zero and pricing

**Evidence [DOCUMENTED]**: *"auto-sleep to a cheap inactive state"*; sprites *"cost practically
nothing while asleep."* Prior research records: sleeps after **30 s** of inactivity; CPU
$0.07/CPU-hour (min 6.25% CPU/s); memory $0.04375/GB-hour (min 0.25 GB/s); storage hot (NVMe
cache) $0.000683/GB-hour, cold (object) $0.000027/GB-hour.

**Sources**: fly.io/blog/design-and-implementation; `sprites-as-overdrive-primitive-research.md` § 1.5 (quoting fly.io/sprites and third-party analyses). **Confidence: High** on the shape, **Medium** on the exact figures (they are ~4 months stale and pricing moves).

**Analysis [INFERENCE]**: *"cost practically nothing while asleep"* plus a **~25× hot-to-cold
storage price ratio** ($0.000683 vs $0.000027 per GB-hour) is the economic statement of the
same architecture: a sleeping sprite is billed as *cold object bytes*, and its NVMe cache and
memory snapshot are the platform's cost to manage, not the customer's. **See P6.3.**

---

## P4. Who else does this, and how

> **All vendor claims in this section are [MARKETING] unless a mechanism is described.**
> None of E2B, Modal, CodeSandbox, Blacksmith, Depot, Namespace, Daytona, or Northflank is in
> the trusted-domain list. Their engineering blogs remain the primary source for their own
> architecture — used as such, marked as such.

### P4.1 CodeSandbox — the most technically detailed public account, and directly on-point

**Finding P4-1: CodeSandbox forks running microVMs by sharing memory pages between clones via
`userfaultfd`, with CoW. This is the strongest third-party confirmation of P3.4 / P6.1.**

**Evidence [MARKETING / vendor engineering blog]**:

- *"Firecracker exposes a `create_snapshot` function that yields `snapshot.snap` (the machine
  configuration) and `memory.snap` (the memory file), and these files together with the disk
  contain everything needed to start a MicroVM."*
- The naive cost: *"creating a snapshot takes about 1 second per gigabyte to write, meaning a
  12 GB VM would take 12 seconds."*
- Their fix: *"serializing the VM memory to a file, copying that memory file, and starting a
  new VM from the new memory file… sped up by eagerly saving memory changes to disk and using
  Copy-on-Write for copying files, only copying memory blocks when actually needed by the new
  VM."* → *"cloning running VMs within 2 seconds."*
- They *"efficiently store memory snapshots for VMs and lazily load them to resume VMs within
  a second"*, and added *"a memory balloon device that Firecracker can run in every VM"* to
  solve memory reclamation.
- MicroVMs *"boot within 300 milliseconds."*

**Sources**: [CodeSandbox — *Cloning microVMs by sharing memory through userfaultfd*](https://codesandbox.io/blog/cloning-microvms-using-userfaultfd); [*How we clone a running VM in 2 seconds*](https://codesandbox.io/blog/how-we-clone-a-running-vm-in-2-seconds); [*How we scale our microVM infrastructure using low-latency memory decompression*](https://codesandbox.io/blog/how-we-scale-our-microvm-infrastructure-using-low-latency-memory-decompression) — Accessed 2026-08-10. **Reputation: Medium** (vendor self-report, not in trusted list).
**[Tool failure — disclosed]** Direct `WebFetch` of the userfaultfd post returned **HTTP 403**; the quotes above are from the search-engine index of those posts, not a full-text read. **Confidence: Medium** — the mechanism is corroborated by Firecracker's own UFFD documentation (P3.3) and by CH's v52.0 `memory_restore_mode` (#7800), so the *mechanism* is High-confidence even though these specific *quotes* are Medium.

**Analysis [INFERENCE]**: Note what CodeSandbox did **not** do: they did not make the *disk*
shared or live. Their whole engineering investment is in the **memory** file. The disk is
handled by *"using Copy-on-Write for copying files."* This is the same split as Sprites —
**memory gets the clever CoW treatment; storage is a CoW'd file**, i.e. block-shaped.

### P4.2 Blacksmith — CI runners, and they REJECT persistent disks outright

**Finding P4-2: A CI-runner vendor built on Firecracker deliberately has no persistent disk,
and uses a colocated cache service instead. This is a direct counter-example to "CI runners
need live shared volumes."**

**Evidence [MARKETING / vendor]**:

- *"Linux and Windows jobs run in ephemeral Firecracker microVMs which boot up in less than 3
  seconds."*
- **The decisive line**: Blacksmith *"deliberately avoids persistent disks entirely, instead
  using 'a colocated caching service that delivers cache downloads at over 400MB/s'."*
- Rationale is **security, not performance**: *"Destroying state after job completion ensures
  strict security and isolation"* — wiping environments after each run to *"prevent cross-run
  contamination."*
- *"Use snapshot-resume to boot in 5-20ms."* **[MARKETING]** — an extraordinary claim with no
  methodology; treat as an upper bound on the mechanism, not a number to plan against.
- Contrast they draw: self-hosted Kubernetes runners offer *"persistent volume claims"*;
  *"default GitHub-hosted runners operate on ephemeral instances that do not support attaching
  custom persistent volumes across different workflow runs."*

**Sources**: [Blacksmith — *Which GitHub Actions runner providers give you fast persistent storage between CI jobs?*](https://info.blacksmith.sh/task/blog/github-actions-fast-persistent-storage); [Blacksmith docs — Instance Types](https://docs.blacksmith.sh/blacksmith-runners/overview) — Accessed 2026-08-10. **Reputation: Medium** (vendor self-report; the first post is also competitive marketing comparing itself to rivals — **bias flagged**, its factual claims about *competitors* are not relied on here, only its claims about *itself*). **Confidence: Medium** (2 sources, same vendor → effectively 1 independent source. Treated as one data point, not a trend.)

**Analysis [INFERENCE]**: This is the most important P4 finding for our decision and it cuts
*against* the intuition that drove I-6. The dispatch's target workload is *"CI runners and
agent sandboxes."* For the **CI-runner half**, at least one Firecracker-based vendor
concluded that persistent per-job storage is a *liability* and replaced it with a network
cache. **[INFERENCE]** If Overdrive's CI-runner story is "fast cache, clean VM," then live
host↔guest sharing — virtiofs's only unique capability (`spike/findings.md` § P7) — buys
nothing at all for that half of the workload.

### P4.3 E2B, Modal, Cloudflare, Daytona, Northflank — carried from prior research

These are covered with citations in `sprites-as-overdrive-primitive-research.md` §§ 2.2–2.6
and 2.6+. Summarised here without re-derivation; consult that document for the citations.

| Vendor | Isolation | Persistence | Restore claim | Storage mechanism |
|---|---|---|---|---|
| **E2B** | Firecracker microVM per sandbox | Session-scoped ≤24 h; auto-pause in beta | *"~150 ms"* VM restore from template snapshot **[MARKETING]** | Template → Firecracker snapshot, built from Dockerfiles. OSS (`e2b-dev/infra`, Apache-2.0) |
| **Modal** | **gVisor (`runsc`) — not a microVM** | Filesystem Snapshots for >24 h runs; Volumes/Dicts | *"~2.5× faster than cold start"*; SD 13 s → 3.5 s **[MARKETING]** | gVisor's own in-kernel C/R (not CRIU); lazy paging, pages preloaded **via FUSE into the host page cache**; GPU state excluded and must be recreated |
| **Cloudflare Sandbox SDK** | Container-in-VM via Durable Objects | DO-backed | — | Not a microVM snapshot model |
| **Daytona / Northflank** | — | — | — | Not independently verified in this pass → Knowledge Gap G6 |

**Analysis [INFERENCE]**: Modal is the interesting outlier and the exception that proves the
rule — it is the **only** surveyed platform that puts FUSE anywhere near the restore path, and
it uses FUSE to **preload into the host page cache**, i.e. as a *host-side* loading mechanism,
**not** as the guest's live filesystem. Nobody in this survey exposes a live host-shared FUSE
filesystem into a checkpointable guest.

**That is a five-platform negative result, and negative results are findings.**

---

## P2. Cloud Hypervisor snapshot/restore — what works and what is FORBIDDEN

> **THE HEADLINE, and it reframes the whole question.**
>
> **[DOCUMENTED]** Everything that makes Cloud Hypervisor snapshot/restore usable for
> the Sprites shape — vhost-user device support, virtio-fs migration, vsock connection
> reset, and `userfaultfd` lazy restore — **landed in v52.0 and v53.0. We are pinned at
> v46.0.0.** The features do not exist in the binary the spike measured.
>
> This is not a subtlety. It is the difference between "CH can do this" and "CH v46
> cannot do this, and a later CH can."

### P2.0 The version trap — read this before any other P2 answer

**Finding P2-0: The `main`-branch `snapshot_restore.md` describes a materially different
feature set from the v46.0 tagged version. Citing `main` for a v46 decision is wrong.**

**Evidence — the v46.0 tagged doc, in full, on the questions that matter:**

> (1) Memory backing requirement: *"The document contains no sentences specifying
> requirements for `--memory shared=on` or file-backed memory. It only states:
> `"memory-ranges" stores the content of the guest RAM.`"*
> (2) Limitations section, exact wording: **`"VFIO devices and Intel SGX are out of scope."`**
> (3) vhost-user / virtiofs / vsock mentions: **none.**
> (5) `userfaultfd` / `memory_restore_mode`: **neither term appears.**

**Source**: [cloud-hypervisor `docs/snapshot_restore.md` @ tag `v46.0`](https://raw.githubusercontent.com/cloud-hypervisor/cloud-hypervisor/v46.0/docs/snapshot_restore.md) — Accessed 2026-08-10. Reputation: High (upstream source of truth, tagged to our exact pinned version).

**Contrast — the same file on `main`:**

> *"The VM must use shared-memory backing (`shared=on` or file-backed). Anonymous memory
> is rejected"* … *"Snapshot and restore are supported for VFIO devices that implement the
> kernel VFIO migration v2 protocol"* … *"When using `memory_restore_mode=ondemand` …
> `prefault=on` is not supported"*.

**Source**: [cloud-hypervisor `docs/snapshot_restore.md` @ `main`](https://raw.githubusercontent.com/cloud-hypervisor/cloud-hypervisor/main/docs/snapshot_restore.md) — Accessed 2026-08-10. Reputation: High.

**Confidence**: **High** — two fetches of the same upstream file at two refs, differing in exactly the ways the release notes predict.

**Analysis [INFERENCE]**: The v46 doc's silence on vhost-user/virtiofs/vsock is **not
evidence of support**. It is the classic inspection-gap shape from
`.claude/rules/debugging.md` § 3 — absence of a warning read as absence of a problem.
The release notes below show the support was *added later*, which means at v46 it was
either absent or partial and simply undocumented.

---

### P2.0b What landed when — the upstream changelog trail

**Finding P2-0b: Four distinct snapshot/restore capabilities we would need landed in
v52.0 and v53.0.**

**Evidence — cloud-hypervisor v52.0 release notes, quoted verbatim:**

| Release-note line | PR |
|---|---|
| *"Snapshot/restore support for `vhost-user` devices has been filled out"* | #7908 |
| *"including migration support for `virtio-fs`"* | #7937 |
| *"Vsock connections are now reset on snapshot restore to avoid stale half-open connections"* | #7958 |
| *"A new `memory_restore_mode` option on the restore path allows guest memory to be populated lazily via `userfaultfd`"* | #7800 |
| *"`vhost-user` devices receive a guest interrupt on resume so that in-flight I/O is not stalled"* | #7851 |
| *"activated queue eventfds are signaled on resume for all virtio devices"* | #8004 |
| *"The KVM clock is now restored before vCPUs are resumed"* | #7932 |
| *"Snapshot and restore now treat the memory backing file as a sparse file"* | #8113 |
| *"Paused VMs can now be migrated"* | #8099 |
| *"A new option to automatically resume the VM on restore"* | #7857 |

**Source**: [cloud-hypervisor Release v52.0](https://github.com/cloud-hypervisor/cloud-hypervisor/releases/tag/v52.0) — Accessed 2026-08-10. Reputation: Medium-High (github.com; upstream project's own release page — primary for its own changelog).

**Evidence — v53.0:**

> *"An offloaded snapshot/restore daemon has been introduced, allowing snapshot and restore
> to be carried out by an external process."*
> *"The live migration protocol has been extended so page faults can be serviced from the source."*
> *"Snapshot pages can now be prefaulted in the background after a `userfaultfd`-based restore."*
> *"Migrated guests now issue post-migration network announcements."*

**Evidence — v51.0 and v50.0 (the only earlier snapshot-adjacent lines found):**

> v51.0: *"Fix snapshot restore when backing file is on read-only storage with `shared=false`."*
> v50.0: *"Fix live migration (and snapshot/restore) with AMX state."*

**Source**: [cloud-hypervisor Releases index](https://github.com/cloud-hypervisor/cloud-hypervisor/releases) — Accessed 2026-08-10. Reputation: Medium-High.
**Verification**: the v52.0 bullet list was fetched twice — once from the releases index and once from the v52.0 tag page — and matched.

**Confidence**: **High** on the content of the bullets (two independent fetches agree).
**Low** on the *dates* those fetches reported (the releases-index fetch returned "v53.0 — July 12, 2024", which cannot be right if v46 is our current pin in 2026). **Dates are treated as UNRESOLVED and are not cited anywhere in this document.** The *ordering* (v46 < v50 < v51 < v52 < v53) is what the argument rests on, and that is unambiguous.

---

### P2.a Does snapshot/restore compose with `--memory shared=on`?

**ANSWER: YES on v46 — and on `main` it is MANDATORY. This is the one sub-question where
the answer moves in our favour.**

**Evidence [DOCUMENTED]**: the v46.0 doc imposes **no** memory-backing requirement at all
(quoted in P2-0 above: *"no sentences specifying requirements for `--memory shared=on` or
file-backed memory"*). The `main` doc requires it for the offload path: *"The VM must use
shared-memory backing (`shared=on` or file-backed). Anonymous memory is rejected."*

**Evidence [DOCUMENTED], v51.0 release note**: *"Fix snapshot restore when backing file is
on read-only storage with `shared=false`."* — The existence of a `shared=false` snapshot-
restore bug fix is direct evidence that **both** `shared=on` and `shared=false` were
intended to work on the snapshot path in that era.

**Sources**: v46.0 `snapshot_restore.md`; `main` `snapshot_restore.md`; v51.0 release notes. Three sources, two independent refs of the upstream doc plus the changelog. **Confidence: High.**

**Analysis**: `--memory shared=on` is **not** the thing that breaks snapshot/restore. Our
spike **[MEASURED]** already established `shared=on` is otherwise cheap — it *reduces* host
RSS by ~11 MB and reclassifies `RssAnon` → `RssShmem` behind a `/memfd:ch_ram`
(`spike/findings.md` § P6). Newer CH *requires* the same backing for the offload path, so
`shared=on` is directionally aligned with where CH snapshot/restore is going.

> **[INFERENCE]** The memfd backing that `shared=on` produces is exactly the shape a
> UFFD/CoW restore story wants — a file-backed guest RAM region that can be `mmap`'d
> `MAP_PRIVATE` by N restored VMs. This is the same structural property Firecracker relies
> on (P3.4). It is an argument *for* `shared=on`, independent of virtiofs.

---

### P2.b Does it compose with `--fs` virtiofs / an external vhost-user daemon?

**ANSWER: NO on v46. There is an OPEN upstream issue showing restore HANGS with a virtiofs
root, and the fix shipped in v52.0 — six releases after our pin.**

**Evidence 1 [DOCUMENTED] — the open issue.**

> **Title**: *"Unable to restore a snapshot of vm using virtiofs root"*
> **Version affected**: Cloud Hypervisor **v43.0.0**
> **Symptom**: *"When attempting to restore a VM snapshot that uses virtiofs as the root
> filesystem, the process hangs and the VMM becomes unresponsive. The guest OS fails to
> continue execution, and file modifications are not persisted to the virtiofs-mounted root."*
> **Status**: **OPEN**, no assignee, no linked PR. *"The logs show the restoration process
> begins but appears to stall during device initialization."*

**Source**: [cloud-hypervisor issue #6931](https://github.com/cloud-hypervisor/cloud-hypervisor/issues/6931) — Accessed 2026-08-10. Reputation: Medium-High (github.com, upstream tracker).

**Evidence 2 [DOCUMENTED] — the fix landed after v46.**

> v52.0: *"Snapshot/restore support for `vhost-user` devices has been filled out"* (#7908),
> *"including migration support for `virtio-fs`"* (#7937).

The phrase **"has been filled out"** is upstream's own admission that the support was
*incomplete before that release*.

**Evidence 3 [DOCUMENTED] — the resume-side gap, also post-v46.**

> v52.0: *"`vhost-user` devices receive a guest interrupt on resume so that in-flight I/O is
> not stalled"* (#7851).

**Analysis [INFERENCE]**: #7851 is a precise description of #6931's symptom — a restored
guest whose vhost-user I/O never resumes because no interrupt was delivered. That the fix
is a *resume-path interrupt* strongly suggests the v43/v46 hang is not a virtiofsd bug but
a CH device-resume bug, and that it applies to **any** vhost-user device, not only a
virtiofs *root*. I could not find a maintainer statement confirming that reading, so it
stays INFERENCE.

**Sources**: 3 (issue #6931; v52.0 release notes ×2 independent fetches). **Confidence: High** that virtiofs + restore is broken at v46; **Medium** that the breakage extends to a non-root virtiofs *volume* (see Knowledge Gap G1 and **SPIKE THIS S-2**).

**Evidence 4 [DOCUMENTED] — the deeper structural reason, still being worked on today.**

> Upstream issue: *"migrate virtio-fs to `fuse-backend-rs`"* — the stated motivation is that
> it *"facilitates live migration and snapshot restoration"*.

**Source**: [cloud-hypervisor issue #7250](https://github.com/cloud-hypervisor/cloud-hypervisor/issues/7250) — Accessed 2026-08-10. Reputation: Medium-High. **Single source** for this specific point; treated as corroborating context, not load-bearing.

**Analysis [INFERENCE]**: The reason virtiofs is the hard case is structural and matches
what our own spike measured **[MEASURED]** — *"virtiofs needs a per-VM `virtiofsd` daemon
whose state lives OUTSIDE the VM"* (`spike/findings.md`). A CH snapshot serialises
`config.json` + `memory-ranges` + `state.json`. **None of those three files can contain
virtiofsd's state**, because virtiofsd is a separate process with its own open file
descriptors, inode↔file-handle table, and FUSE session. See P5.3.

---

### P2.c Does it compose with `--disk` block devices?

**ANSWER: YES, with a caveat that has nothing to do with CH — the disk image is EXTERNAL
to the snapshot and its consistency is your problem.**

**Evidence 1 [DOCUMENTED]**: The v46.0 doc names exactly three snapshot artifacts —
*"**config.json**: VM configuration (CPU, RAM, devices)"*, *"**memory-ranges**: Guest RAM
contents"*, *"**state.json**: Component states from the snapshot moment"*. **No disk image
is among them.** Block devices are referenced by path in `config.json`, not captured.

**Evidence 2 [DOCUMENTED]**: The only stated limitation is *"VFIO devices and Intel SGX are
out of scope."* `--disk` is not excluded.

**Evidence 3 [DOCUMENTED]**: v50.0 shipped *"Live disk resizing support for raw images. The
`/vm.resize-disk` API has been introduced"* and v51.0 fixed *"snapshot restore when backing
file is on read-only storage"* — both presuppose block devices work across snapshot.

**Sources**: 3 (v46.0 doc; v50.0 notes; v51.0 notes). **Confidence: High** that block
devices are in scope. **Medium** that a plain `--disk` snapshot/restore round-trip is
*byte-clean* at v46, because I found no upstream test or doc asserting it and no issue
denying it → **SPIKE THIS S-1**.

**Analysis [INFERENCE]**: This is the decisive asymmetry for P5. A block volume's entire
state is *inside the image file*. The guest's page cache for that device is *inside
`memory-ranges`*. So a snapshot captures a **consistent pair**: dirty pages in RAM + the
on-disk image. Restore replays both. Nothing outside the VM has to be re-established. That
is not true of virtiofs.

---

### P2.d Does it compose with `--vsock`?

**ANSWER: PARTIALLY on v46 — the device restores, but any OPEN connection is left
half-open. CH only started resetting them in v52.0.**

**Evidence 1 [DOCUMENTED]**: v52.0 — *"Vsock connections are now reset on snapshot restore
to avoid stale half-open connections"* (#7958). The stated purpose of the change **names
the pre-existing defect**: before it, restore left stale half-open connections.

**Evidence 2 [DOCUMENTED]**: v46.0 doc does not mention vsock at all, and does not list it
as out of scope.

**Evidence 3 [MEASURED]**: Our spike established *why* this bites us specifically. The host
end of CH's vsock is *"a UNIX domain socket on the filesystem, not `AF_VSOCK`"*; guest→host
connections are `accept()`ed on `<socket_path>_<port>`; our Running-gate beacon rides that
channel (`spike/findings.md` § P2). A restored VM whose beacon connection is half-open is
a driver-visible failure, not a curiosity.

**Sources**: 3 (v52.0 notes; v46.0 doc; our own spike). **Confidence: High** that v46
leaves stale connections; **Medium** on the precise guest-visible symptom (does the guest
see `EPIPE`? does it hang? does CH re-create the listening UDS at the same path?) →
**SPIKE THIS S-3**.

**Analysis [INFERENCE]**: There is also a **host-path** problem the release note does not
cover. The vsock UDS path lives in `config.json`. Restore on a different host, or after the
per-VM socket directory has been reaped, needs that path to exist and to be inside the
Landlock grant. Our spike **[MEASURED]** already found CH does **not** auto-derive a
Landlock rule for the vsock UDS and *"the rule cannot name the socket path"* — the grant
must be the containing directory (`spike/findings.md` § P5 correction 2). **A restore path
must recreate that directory before `vm.restore`, or restore fails with a `PermissionDenied`
that never mentions Landlock.** That is a direct, actionable driver requirement.

---

### P2.e Summary table — the four sub-questions

| # | Composes with… | Verdict **at our pinned v46.0.0** | Primary citation | Confidence |
|---|---|---|---|---|
| **a** | `--memory shared=on` | **YES.** No memory-backing restriction in v46. Newer CH *requires* shared/file-backed for offload. | v46.0 `snapshot_restore.md` (silent); `main` doc (*"must use shared-memory backing… Anonymous memory is rejected"*); v51.0 note (*"…with `shared=false`"*) | High |
| **b** | `--fs` virtiofs / external vhost-user daemon | **NO.** Open issue #6931 — restore **hangs**, VMM unresponsive, virtiofs root, v43. Fix shipped **v52.0** (#7908 *"filled out"*, #7937 virtio-fs migration, #7851 resume interrupt). | issue [#6931](https://github.com/cloud-hypervisor/cloud-hypervisor/issues/6931) (OPEN); v52.0 release notes | High (root); Medium (non-root volume — **SPIKE THIS S-2**) |
| **c** | `--disk` block devices | **YES**, but the image is **outside** the snapshot — only `config.json` / `memory-ranges` / `state.json` are captured. Consistency of the image is the driver's problem. | v46.0 `snapshot_restore.md` (3-artifact list; *"VFIO devices and Intel SGX are out of scope"*); v50.0 `/vm.resize-disk`; v51.0 backing-file fix | High (in scope); Medium (byte-clean round-trip — **SPIKE THIS S-1**) |
| **d** | `--vsock` | **PARTIAL.** Device restores; **open connections are left stale/half-open** until v52.0 (#7958). Plus a v46-specific Landlock/host-path hazard we measured ourselves. | v52.0 note #7958 (*"…to avoid stale half-open connections"*); v46.0 doc (silent); `spike/findings.md` § P2, § P5 | High (stale connections); Medium (exact symptom — **SPIKE THIS S-3**) |

**And the fifth answer nobody asked for, which dominates all four:**

| **e** | **The whole feature, at v46** | **Immature for the Sprites shape.** No `userfaultfd` lazy restore (v52.0, #7800), no background prefault (v53.0), no offload daemon (v53.0), no auto-resume-on-restore (v52.0, #7857), sparse-file memory handling absent (v52.0, #8113). | v52.0 + v53.0 release notes | High |

---

## P3. Firecracker snapshot/restore, for comparison

Firecracker is the substrate under Fly Machines, Sprites, and E2B (P1, P4). Its docs are
the most explicit public statement of what a microVM snapshot can and cannot promise. We
are **not** proposing to adopt Firecracker (intake I-2 fixes Cloud Hypervisor as the only
`Vmm` implementor in scope); this section is a *contrast set* that tells us what Cloud
Hypervisor's silence is hiding.

### P3.1 What a Firecracker snapshot captures — and what it does not

**Finding P3-1: The snapshot captures guest memory + emulated hardware state. It does NOT
capture block device contents, and it does NOT flush them.**

**Evidence [DOCUMENTED]**, quoted:

- Captured: *"Guest memory"*, *"Emulated hardware state (KVM and Firecracker)"*.
- Not captured: *"Configuration information for metrics and logs are not saved to the snapshot"*.
- *"The Firecracker microVM's MMDS config is included in the snapshot. However, the data store is not persisted"*.
- **Block devices**: *"The disk contents are not explicitly flushed to their backing files"* — the contents are the user's problem.

**Source**: [Firecracker `docs/snapshotting/snapshot-support.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md) — Accessed 2026-08-10. Reputation: Medium-High (github.com) / High as the project's own normative documentation.
**Verification**: cross-referenced against the prior Overdrive research doc `docs/research/platform/sprites-as-overdrive-primitive-research.md` § 2.4, which independently quotes the same source for *"guest memory, emulated HW state (both KVM and Firecracker emulated HW)"* as separate files.

**Confidence: High** (2 sources, one of which is the primary; the second is our own earlier research reading the same primary).

**Analysis [INFERENCE]**: This is the **same three-artifact shape as Cloud Hypervisor v46**
(`config.json` / `memory-ranges` / `state.json`, P2.c). Two independent microVM projects
converged on "snapshot = RAM + device state, storage is external." That convergence is the
strongest available signal that *storage-outside-the-snapshot is the architecture, not an
omission* — and it is the backbone of the P5 recommendation.

### P3.2 What is explicitly forbidden or degraded

**Finding P3-2: Firecracker names the failure modes Cloud Hypervisor v46's doc is silent
about.**

| Surface | Firecracker's exact words |
|---|---|
| **vsock** | *"vsock connections that are open when the snapshot is taken are closed, but existing vsock listen sockets in the guest still remain active"* |
| **Network + vsock** | *"Both network and vsock packet loss can be expected on guests that are resumed from snapshots"* |
| **Network connectivity** | *"Network connectivity is not guaranteed post-resume"* |
| **Block** | *"The disk contents are not explicitly flushed to their backing files"* |
| **MMDS** | data store *"is not persisted"* |
| **Memory hotplug** | *"This is only allowed before the `InstanceStart` action and **not on snapshot-restored VMs** (which will use the configuration saved in the snapshot)"* |
| **Host/CPU compat** | snapshots require *"identical"* software and hardware; the one carve-out is *"snapshots can be resumed on identical hardware instances where they were taken on, but using newer host kernel versions"* (m5n/m6i/m6a, 5.10 → 6.1, **not** vice versa) |

**Sources**: [`snapshot-support.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md); [`memory-hotplug.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/memory-hotplug.md) — both Accessed 2026-08-10. **Confidence: High** (primary normative docs; the memory-hotplug restriction independently corroborates the claim already recorded in this feature's `intake.md`, *"Firecracker's memory hotplug… is not permitted on snapshot-restored VMs"*).

**Analysis — three consequences for us:**

1. **[DOCUMENTED] Firecracker CLOSES open vsock connections at snapshot. Cloud Hypervisor
   v46 does NOT** — it leaves them *"stale half-open"* until v52.0 (#7958). Firecracker's
   behaviour is the *correct* one, and CH adopted it six releases after our pin. **[INFERENCE]**
   Our vsock-beacon Running gate (`spike/findings.md` § P2) therefore needs an explicit
   post-restore re-handshake regardless of hypervisor — and on v46 it additionally needs to
   survive a socket that CH thinks is still open and the guest does not.
2. **[DOCUMENTED] "Identical hardware" is the compatibility rule.** Restore-on-a-different-host
   is bounded by CPU model, not just by storage. Any Sprites-shape migration story inherits
   this.
3. **[DOCUMENTED] Hotplug is frozen at snapshot time on Firecracker.** **[INFERENCE]** If
   Cloud Hypervisor has the same property, it directly damages GH [#92](https://github.com/overdrive-sh/overdrive/issues/92)
   (the right-sizing reconciler), whose whole premise is CPU hotplug on long-lived VMs — and
   persistent/checkpointed VMs are exactly the long-lived ones. **I found no CH statement
   either way. This is Knowledge Gap G3 and SPIKE THIS S-6, and it is arguably the highest-
   value cheap probe in this document**, because it tests an interaction between two
   separately-justified features that nobody has checked composes.

### P3.3 UFFD-based lazy restore

**Finding P3-3: Firecracker offers two restore paths — kernel-demand-paged `MAP_PRIVATE`,
and a userspace `userfaultfd` handler. Cloud Hypervisor gained the equivalent in v52.0.**

**Evidence [DOCUMENTED]**:

- The default path: *"Each time the guest touches a page that is not already in Firecracker's
  process memory, a page fault occurs, which triggers a context switch and IO operation in
  order to bring that page into RAM."*
- UFFD: *"Userfaultfd is a mechanism that passes that responsibility of handling page fault
  events from kernel space to user space."* A handler process receives fault events on an fd
  and uses `UFFDIO_COPY` to populate the region from a file.
- Handler obligations: handle `UFFD_EVENT_REMOVE` (*"from balloon operations"*) by zeroing
  pages; signal Firecracker of crashes via `getsockopt`; implement timeouts.

**Source**: [Firecracker `docs/snapshotting/handling-page-faults-on-snapshot-resume.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/handling-page-faults-on-snapshot-resume.md) — Accessed 2026-08-10. Reputation: High (normative). **Single authoritative source** for the mechanism detail; cross-referenced for existence by our own prior research (`sprites-as-overdrive-primitive-research.md` § 2.4 and § "Cloud Hypervisor has snapshot/restore with `userfaultfd` lazy-paging parity"). **Confidence: High** on the mechanism; see the Conflict section on the "parity" claim.

**Cloud Hypervisor equivalent [DOCUMENTED]**: v52.0 — *"A new `memory_restore_mode` option
on the restore path allows guest memory to be populated lazily via `userfaultfd`"* (#7800);
v53.0 — *"Snapshot pages can now be prefaulted in the background after a `userfaultfd`-based
restore."* `main` adds the constraints *"`prefault=on` is not supported"* with
`memory_restore_mode=ondemand`, and *"the snapshot memory ranges must be page-aligned."*

> ⚠ **CONFLICT with prior Overdrive research — flagged, see § Conflicting Information.**
> `docs/research/platform/sprites-as-overdrive-primitive-research.md` (2026-04-19) states
> *"Cloud Hypervisor has snapshot/restore with `userfaultfd` lazy-paging parity with
> Firecracker."* The v52.0 release note shows `memory_restore_mode` / UFFD arriving
> **after v46**. Either that claim was about a future CH, or it was wrong. **At v46 it is
> not true.**

### P3.4 Copy-on-write memory sharing across restores

**Finding P3-4: Firecracker's default restore is `MAP_PRIVATE` over the memory file — which
gives CoW page sharing between N restores of the same snapshot *for free from the host page
cache*. But the docs do not say this in the page-fault document.**

**Evidence 1 [DOCUMENTED]**, from the prior Overdrive research quoting Firecracker's docs:
*"Memory restore uses `MAP_PRIVATE` mapping of the memory file — 'on-demand loading of memory
pages' with CoW to anonymous memory. Resumed VM requires the memory file to be kept around
for its lifetime."*
**Source**: `docs/research/platform/sprites-as-overdrive-primitive-research.md` § 2.4 (2026-04-19), quoting [Firecracker snapshot docs](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md).

**Evidence 2 [DOCUMENTED] — the negative result, which matters:** the dedicated page-fault
document *"contains no statements regarding memory page sharing between multiple restored
microVMs or copy-on-write mechanisms"* and makes *"no explicit performance or latency claims."*

**Confidence: Medium.** The `MAP_PRIVATE` fact is well-sourced; the *inference that it yields
cross-VM sharing* is mine.

**Analysis [INFERENCE]**: `MAP_PRIVATE` over one file, opened by N processes, means the
*clean* pages are shared page-cache pages and only *written* pages are privately duplicated.
So the marginal host RAM cost of the (N+1)-th restore of the same snapshot is its **write
working set**, not its RAM size. This is the mechanism that makes warm pools economically
viable, and it is a property of the *memory file*, not of the hypervisor. **[INFERENCE]** It
should therefore transfer to Cloud Hypervisor's `memory-ranges` file identically — and our
spike **[MEASURED]** already showed `--memory shared=on` produces a `/memfd:ch_ram` backing
with the footprint living in `RssShmem`, which is the same structural shape. **SPIKE THIS S-5.**

### P3.5 The snapshot SAFETY warnings — the part with teeth

**Finding P3-5: Firecracker states plainly that restoring one snapshot more than once is
insecure, and enumerates exactly what is duplicated.**

**Evidence [DOCUMENTED]**:

> *"resuming execution from the same state more than once"* risks duplicate use of
> *"unique identifiers, random numbers and random number seeds, the guest OS entropy pool,
> as well as cryptographic tokens."*
> The secure pattern: create snapshot → terminate the original VM → resume **once** in a new VM.

**Source**: [`snapshot-support.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md) — Accessed 2026-08-10.

**Evidence [DOCUMENTED] — the dedicated entropy document is more nuanced, and its nuance is
worth reading in full:**

- *"Getting random bytes from either `/dev/random` or `/dev/urandom` does not lead to
  identical results for different clones"* — because timer data and CPU hardware RNG output
  are mixed in. **But** the document declines to rely on that: the conservative position is
  that *"the stale state has a significant influence on RNG output, so we should reinitialize
  both sources based on fresh data after each restore."*
- **VMGenID**: Linux ≥5.18 (ACPI) / ≥6.10 (DeviceTree). On resume Firecracker *"writes a new
  identifier and injects a notification to the guest"*, and the kernel treats it *"as new
  randomness for its CSPRNG."*
- **The residual hazard, stated by Firecracker itself**: this leaves *"a race window between
  resuming vCPUs and Linux CSPRNG getting successfully re-seeded."*
- Mitigations named: delete `/var/lib/systemd/random-seed`; use RDRAND/RDSEED; attach
  virtio-rng; on pre-5.18 kernels manually `RNDADDENTROPY` + `RNDRESEEDCRNG` *before customer
  code resumes*.

**Source**: [Firecracker `docs/snapshotting/random-for-clones.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/random-for-clones.md) — Accessed 2026-08-10. Reputation: High.
**Verification**: `snapshot-support.md` (independent file, same repo) states the "resume once" rule; our prior research `sprites-as-overdrive-primitive-research.md` § 2.4 independently recorded *"resuming the same snapshot many times creates RNG/entropy reuse risks"* and *"VMGenID mitigates entropy reuse risks."* **Confidence: High** (3 sources).

**Analysis — three direct consequences for Overdrive:**

1. **[INFERENCE] The Sprites shape is the SAFE shape; the warm-pool shape is the DANGEROUS
   one.** A *persistent* VM is snapshot→restore-**once**→snapshot again: a linear chain,
   exactly Firecracker's "secure pattern." A *warm pool* fans one golden snapshot out to N
   VMs: exactly the forbidden shape. **Our target workload (CI runners, agent sandboxes)
   plausibly wants both**, and they carry opposite safety postures. This distinction should
   be made explicit in DESIGN, not discovered later.
2. **[UNRESOLVED] Cloud Hypervisor + VMGenID is still unverified**, four months after the
   intake flagged it: *"Whitepaper:626's 'Cloud Hypervisor exposes a VMGenID device' claim
   remains UNVERIFIED (flagged 2026-04-19, never checked)."* It remains unchecked here — I
   found no CH doc confirming it. **SPIKE THIS S-4** — this is a ~5-minute probe
   (`grep`/`--help`, then read `/sys/firmware/acpi` in a guest) that closes a premise of
   GH [#96](https://github.com/overdrive-sh/overdrive/issues/96).
3. **[DOCUMENTED→INFERENCE] The "race window between resuming vCPUs and CSPRNG re-seed" is a
   platform problem, not a guest problem.** Overdrive mints workload SVIDs and the guest
   holds nothing (CLAUDE.md § "Workload identity model"), so the *SVID* is safe — but any
   in-guest TLS client, SSH host key, or agent-generated token created in that window is
   not. **[INFERENCE]** The cheap structural answer for a fan-out pool is: do not let the
   guest run customer code until the guest agent has confirmed reseed — which is a
   `overdrive-guest-agent` (GH [#100](https://github.com/overdrive-sh/overdrive/issues/100))
   requirement, and it is the same beacon channel the Running gate already uses.

---

## P4. Who else does this, and how

_placeholder_

---

## P5. What checkpoint/restore implies for VOLUME storage — the I-6 decision

### P5.0 The distinction everything turns on: LIVE MIGRATION ≠ CHECKPOINT/RESTORE

**Finding P5-0: virtiofsd HAS supported live migration since v1.11.0 — and that fact does
NOT mean it supports snapshot-and-restore-later. The two have opposite structural
requirements, and the upstream documentation says so.**

**Evidence 1 [DOCUMENTED] — migration is supported:**

> *"Older versions of virtiofsd (prior to 1.11) do not support migration so operations such as
> migration, save/managed-save, or snapshots with memory may not be supported if a VM has a
> virtiofs filesystem connected. However, versions of virtiofsd starting with v1.11.0 support
> live migration."*

**Evidence 2 [DOCUMENTED] — but snapshot is explicitly NOT:**

> **"Snapshot operations managed by libvirt do not snapshot the state of the files shared via
> virtiofs, and thus reverting to an earlier state is not recommended."**

**Evidence 3 [DOCUMENTED] — and the migration mechanism cannot survive a temporal gap:**

> *"The `file-handles` mode has the source instance generate a file handle for each inode,
> which is sent to the destination and opened there…"*
> *"This migration mode requires source and destination to use the same shared directory on
> the same filesystem; however, source and destination instance need not necessarily be on the
> same host, **if that filesystem is a network filesystem**."*
> *"…it is resilient against inodes being renamed or unlinked by any party while they are still
> in use by the guest, **as long as the virtiofsd source instance keeps running until migration
> is fully complete**."*

**Source**: [libvirt — *Sharing files with Virtiofs*](https://libvirt.org/kbase/virtiofs.html) — Accessed 2026-08-10. Reputation: **High** (libvirt is the canonical upstream consumer of virtiofsd and an established OSS-foundation-class project; not on the trusted-domain list by name, but of the same class as the listed `open_source` tier — **flagged as an off-list source**).
**Verification**: corroborated by (a) the virtio-fs upstream mailing-list thread *"Live migration support for virtio-fs"*, (b) a Proxmox `virtiofs: add documentation for live migration` patch series, and (c) Cloud Hypervisor's own v52.0 note which says *"**migration** support for `virtio-fs`"* — **migration**, not snapshot. **4 sources. Confidence: High.**

**Analysis [INFERENCE] — and this is the crux of the entire document:**

Live migration is a **hand-off between two simultaneously-live daemons**. Both ends exist at
once; the source stays up until the destination has re-opened every inode. Checkpoint/restore
is a **temporal gap**: the VM is written to disk, every process on the host — including
`virtiofsd` — exits, and some time later a *new* process is asked to resume a state it never
participated in creating.

**`file-handles` mode cannot bridge that gap by construction.** Its own precondition —
*"as long as the virtiofsd source instance keeps running until migration is fully complete"* —
is exactly the thing a checkpoint destroys. There is no source instance. It exited an hour ago.

> **So: "virtiofsd 1.13.2 supports migration" and "CH v52.0 added virtio-fs migration support"
> are both TRUE and both IRRELEVANT to the Sprites shape.** Anyone reading only the release
> notes will conclude virtiofs is fine for checkpoint/restore. It is not. This is the wrong
> "it works" the dispatch warned would cost a rework cycle, and it is *one release note away*
> from being believed.

### P5.1 Daemon lifetime across restore

**[DOCUMENTED + MEASURED]** Our own spike established the premise: *"virtiofs needs a per-VM
`virtiofsd` daemon whose state lives OUTSIDE the VM"* (`spike/findings.md`), and the reference
implementation's `VirtiofsdManager` was **415 lines** of socket-wait / SIGTERM→SIGKILL / `Drop`
lifecycle (`intake.md` § I-6).

**[INFERENCE]** A CH snapshot writes `config.json` + `memory-ranges` + `state.json` (P2.c).
`virtiofsd`'s state is in **none of them** — it is in another process's address space and
file-descriptor table. Restore must therefore:

1. Re-spawn `virtiofsd` **before** `vm.restore`, on a socket path that matches `config.json`.
2. Get the guest's in-flight vhost-user virtqueue state to line up with a daemon that has no
   memory of it. **[DOCUMENTED]** CH did not deliver a resume interrupt to vhost-user devices
   until v52.0 (#7851), and #6931 reports exactly the resulting hang at v43.
3. Do this while `--memory shared=on` means the daemon must re-`mmap` the guest RAM memfd —
   an object that, **[INFERENCE]**, does not obviously survive the snapshot as the same memfd.

**Confidence: High** that step 1 is required; **Medium** on step 3's mechanics → **SPIKE THIS S-2**.

**Contrast, block:** there is no daemon. `spike/findings.md` § P7 measured this as a positive:
*"No daemon. No `virtiofsd` process, no socket-wait, no SIGTERM→SIGKILL lifecycle, and no
'crashed daemon must not look like a clean VM exit' failure mode."* Restore re-attaches a file
by path. **[INFERENCE]** The number of moving parts across a restore boundary is 0 for block
and ≥3 for virtiofs.

### P5.2 Host path stability

**[DOCUMENTED]** Both mechanisms record host paths in `config.json`, so both need path
stability. But the *shapes* differ:

| | virtiofs | virtio-blk |
|---|---|---|
| What must exist at restore | the **socket path** (per-VM, ephemeral, `/run`-shaped) **and** the exported **directory tree**, unchanged | the **image file**, at its path |
| Landlock grant | **[MEASURED]** socket dir grant needed for `--vsock`; `--fs socket=` **is** auto-derived (`spike/findings.md` § P6 `[D8e]`) | auto-derived from `--disk` |
| Different-host restore | needs the *same shared directory on the same filesystem*, or a **network filesystem** **[DOCUMENTED]** — libvirt, quoted above | needs the image file to be reachable — which a content-addressed store makes tractable |
| Reflink staging trap | n/a | **[MEASURED]** `--reflink` is intra-filesystem; staging per-VM images on tmpfs silently loses the 260× win (`spike/findings.md` § P7) |

**[INFERENCE]** The virtiofs socket path is the sharper hazard because it is **ephemeral by
design**: our spike requires *"each VM needs its own socket directory holding nothing else"*
(`spike/findings.md` § P5 correction 2), and such a directory is exactly what a reboot or a
tmpfs reaper removes. A restore hours later must reconstruct a directory whose whole purpose
was to be per-run.

### P5.3 File handles / FUSE session state held open at snapshot time

**Finding P5-3: This is the irreducible problem, and it is the same one `file-handles`
migration mode was invented to solve — for the case where both ends are alive.**

**[DOCUMENTED]** *"A file handle is data that uniquely identifies an inode on a filesystem"*;
the source generates one per inode and the destination opens it. The guarantee is bounded by
*"as long as the virtiofsd source instance keeps running."*

**[INFERENCE]** At snapshot time the guest holds open FUSE file handles. Those map, through
the dead daemon, to host inodes. After the daemon exits, nothing holds a reference. The host
tree can then be renamed, unlinked, `rsync`'d, deduplicated, or reaped — and **nothing detects
it**. On restore the guest resumes with file descriptors that its kernel believes are valid
and that now designate nothing, or worse, something else. **The failure is silent and arrives
as data corruption inside the guest, not as an error at restore.**

**Contrast, block:** the guest's open file descriptors are to *its own* filesystem inside the
image. The image is one host file. **[INFERENCE]** There is exactly one referent and it is
captured or not captured atomically at the file level. No cross-boundary handle exists.

### P5.4 Dirty guest page cache

**Finding P5-4: block gives a consistent RAM+disk pair; virtiofs does not.**

**[DOCUMENTED]** Firecracker: *"The disk contents are not explicitly flushed to their backing
files"* — the snapshot captures RAM *including the dirty page cache*, and the backing file
lags.

**[INFERENCE]** For **virtio-blk** this is *benign and self-correcting*: `memory-ranges`
contains the dirty pages, the image contains the flushed ones, and restore reassembles a guest
that is in exactly the state it was in. The pair is consistent **because both halves were
captured**. The only rule is: **do not mutate the image between snapshot and restore** —
snapshot the image (reflink!) at the same instant.

For **virtiofs** the same physics produce the opposite outcome. Dirty guest pages for a
virtiofs file are in `memory-ranges`; the host file is whatever the daemon had written. Restore
replays guest-side dirt over a host tree that **anything may have modified in the interval** —
including the host itself, which is *the entire reason virtiofs was chosen*. **The capability
that motivates virtiofs (live host access) is precisely the capability that makes its
checkpoint unsound.**

**[MEASURED]** Our spike found the block analogue of this too, and it is much milder:
*"an ungracefully-killed VM leaves a volume that needs recovery on next attach"* — a dirty
ext4 journal, recoverable by `fsck`. There is no `fsck` for "the host edited the tree while
the guest was checkpointed."

### P5.5 Inside the snapshot vs deliberately external state

**Finding P5-5: Every surveyed production system puts storage DELIBERATELY OUTSIDE the memory
snapshot, and makes it independently durable. Nobody puts it inside; nobody makes it live-shared.**

| System | Memory snapshot contains | Storage is | Evidence marker |
|---|---|---|---|
| **Fly Sprites** | guest RAM (*"kept around for as long as possible"*, best-effort) | content-addressed chunks on S3, NVMe as pure cache, *"nothing in that NVMe volume should matter"* | **[DOCUMENTED]** |
| **Firecracker** | *"guest memory, emulated HW state"* | *"disk contents are not explicitly flushed"* — external, caller's problem | **[DOCUMENTED]** |
| **Cloud Hypervisor v46** | `memory-ranges` + `state.json` | referenced by path in `config.json` only | **[DOCUMENTED]** |
| **CodeSandbox** | `memory.snap`, CoW-shared via `userfaultfd` | *"Copy-on-Write for copying files"* — a cloned disk file | **[MARKETING]** |
| **Modal** | gVisor C/R | Volumes/Dicts + Filesystem Snapshots; FUSE used only to preload the **host** page cache | **[MARKETING]** |
| **Blacksmith** | snapshot-resume | **no persistent disk at all** — colocated cache service | **[MARKETING]** |

**Analysis [INFERENCE]**: This is a **six-for-six convergence**, across two hypervisors, one
userspace kernel, and four independent commercial platforms with no shared codebase. The
architecture is: **memory snapshot = fast-resume cache; storage = independently durable,
externally addressed, and NOT live-shared with the host.** Sprites states the fallback
explicitly — *"Sprites will reboot (because crash, upgrade, cold, etc.)"* — and survives it
because the filesystem never depended on the memory snapshot.

### P5.6 RECOMMENDATION ON I-6

> ## Recommendation: **SPLIT I-6. Keep `virtio-blk` as the DEFAULT volume mechanism. Keep `virtiofs` as an OPT-IN, and make it structurally incompatible with checkpointing.**
>
> I-6 as written — *"`virtiofs` for VOLUMES / shared writable storage"* — should **not** stand
> as an unconditional default for a persistent-microVM platform. But it should **not be
> reversed to "block only" either**: virtiofs owns a capability block cannot provide at any
> price, and some workloads need it.

**What is DOCUMENTED FACT:**

1. **[DOCUMENTED]** CH v46 cannot restore a snapshot of a VM with a virtiofs root — open issue
   #6931, restore hangs, VMM unresponsive. The fix shipped in **v52.0**, six releases past our
   pin.
2. **[DOCUMENTED]** virtiofsd's migration support (≥1.11.0) is **live migration**, and libvirt
   states plainly that *"Snapshot operations… do not snapshot the state of the files shared via
   virtiofs, and thus reverting to an earlier state is not recommended."*
3. **[DOCUMENTED]** `file-handles` migration requires *"the virtiofsd source instance keeps
   running until migration is fully complete"* — structurally unavailable to a checkpoint.
4. **[DOCUMENTED]** A CH snapshot contains three files, none of which is a disk image or a
   daemon's state.
5. **[DOCUMENTED]** No surveyed production platform exposes a live host-shared filesystem into
   a checkpointable guest (P5.5, six systems).
6. **[MEASURED]** Performance does **not** decide it: block is ~42% faster streaming, virtiofs
   ~25% faster per small file, non-overlapping (`spike/findings.md` § P7). This was already the
   spike's own conclusion and nothing here changes it.
7. **[MEASURED]** Block additionally has rate limiting (`bw_size`/`bw_refill_time`); *"`--fs`
   has no equivalent parameter at all… For a multi-tenant platform that is a real operational
   gap"* (`spike/findings.md` § P7).
8. **[MEASURED + DOCUMENTED]** Both enforce host-side read-only; block fails *earlier* (at
   mount, `EACCES`) than virtiofs (at write, `EROFS`).

**What is INFERENCE (mine, refutable):**

1. **[INFERENCE]** The virtiofs↔checkpoint incompatibility is **structural, not a version
   bug.** Even on a hypothetical CH v53 the daemon-lifetime and file-handle-validity problems
   remain: a checkpoint is a temporal gap, and every virtiofs continuity mechanism upstream has
   built assumes a live source. Upgrading CH would fix the *hang*; it would not make a virtiofs
   share's contents part of the snapshot.
2. **[INFERENCE]** The correct reading of Sprites is *"solve it below the block device, not at
   the guest interface."* Fly's persistence is a content-addressed chunk store **under** an
   ext4 block device — which maps onto Overdrive's already-planned GH
   [#97](https://github.com/overdrive-sh/overdrive/issues/97) `overdrive-fs` chunk store, and
   is **compatible with a `virtio-blk` guest interface**. Notably, #97's title pairs the chunk
   store with `vhost-user-fs`; **[INFERENCE]** the Sprites evidence suggests `vhost-user-blk`
   is the better pairing, and `spike/findings.md` § P7 already flags `vhost-user-blk` as *"a
   third option that keeps the block model and moves the backend to userspace"* that was **not
   measured**.
3. **[INFERENCE]** The two named target workloads want *different* things, and I-6 was decided
   before either was named:
   - **CI runners** — Blacksmith's counter-example plus *"destroying state after job completion
     ensures strict security and isolation"* suggests ephemeral VM + fast external cache. Block
     + reflink (**[MEASURED]** ~260×, free in space) serves this perfectly.
   - **Agent sandboxes** — the Sprites shape: long-lived, checkpointed, whole-writable-filesystem
     persistence. Block over a chunk store serves this; virtiofs breaks it.
   - The **only** workload that genuinely needs virtiofs is one where **the host must read or
     write the volume while the guest is running** — live log tailing, live artifact collection,
     a host-side supervisor mutating the tree. `spike/findings.md` § P7 already isolated this as
     *"the only one that cannot be engineered around."*

**Concretely, what I recommend:**

| # | Recommendation | Grounding |
|---|---|---|
| **R1** | **Default volume mechanism = `virtio-blk`.** Make virtiofs opt-in per volume, not the default. | 5 DOCUMENTED + 3 MEASURED facts above |
| **R2** | **Make the conflict structural, not documentary.** A workload declaring *both* a virtiofs volume *and* checkpoint capability must fail **at spec-validation time** with a typed error naming the reason — not at restore time with a hang. Per `.claude/rules/development.md` § "Type-driven design", make the invalid combination unrepresentable. | #6931 is a *hang*, and `spike/findings.md` § P7 already warns a crashed daemon must not look like a clean VM exit |
| **R3** | **Do not adopt virtiofs on the checkpoint path on the strength of a release note.** CH v52.0's *"migration support for `virtio-fs`"* is **migration**, not snapshot/restore. Any future re-opening of this decision must cite a *snapshot-restore* test, not a migration note. | P5.0 |
| **R4** | **Pair the persistence story with `overdrive-fs` (#97) UNDER a block device**, and evaluate `vhost-user-blk` as the seam — not `vhost-user-fs`. | Sprites' architecture **[DOCUMENTED]**; `vhost-user-blk` unmeasured **[MEASURED gap]** |
| **R5** | **Record a version floor with a REASON.** `intake.md` warns the reference implementation asserted *"CH ≥ 48.0 and virtiofsd ≥ 1.10 … with no stated reason anywhere."* We now have reasons: **v52.0** for vhost-user snapshot/restore + vsock connection reset + `userfaultfd` restore; **v53.0** for background prefault and the offload daemon. **If checkpoint/restore (GH #96) is in scope, v46 is below the floor and the floor is v52.0.** | v52.0/v53.0 release notes |
| **R6** | **Keep `--memory shared=on` regardless of the I-6 outcome.** It does not obstruct snapshot (P2.a), newer CH *requires* shared/file-backed memory for offload, it produces the memfd backing a CoW warm pool needs (P6.1), and it costs ~11 MB *less* host RSS. | **[MEASURED]** § P6; **[DOCUMENTED]** `main` doc |

**What would change my mind** (stated so this is falsifiable): a spike showing a CH v46 VM with
a **non-root** `--fs` volume snapshotting and restoring cleanly, *plus* a demonstration that
guest file handles remain valid across a daemon restart. **SPIKE THIS S-2** is exactly that
test, and it costs one afternoon on the metal box.

---

## P6. Memory-snapshot mechanics that bear on cost

### P6.1 Copy-on-write / page-cache sharing between restored instances

**Finding P6-1: The mechanism exists and is well-attested — `MAP_PRIVATE` over a shared
memory file, or a `userfaultfd` handler serving many VMs from one backing file. Cloud
Hypervisor gained the UFFD half only in v52.0.**

**Evidence [DOCUMENTED]**: Firecracker restores via *"`MAP_PRIVATE` mapping of the memory file"*
with *"on-demand loading of memory pages"* and CoW to anonymous memory; *"Resumed VM requires
the memory file to be kept around for its lifetime."*
**Evidence [MARKETING, corroborating]**: CodeSandbox *"only copying memory blocks when actually
needed by the new VM"*, achieving *"cloning running VMs within 2 seconds"* and lazily loading
snapshots to *"resume VMs within a second."*
**Evidence [DOCUMENTED]**: CH v52.0 — *"guest memory to be populated lazily via `userfaultfd`"*
(#7800); v53.0 — *"Snapshot pages can now be prefaulted in the background after a
`userfaultfd`-based restore."*

**Sources**: Firecracker snapshot docs (via prior research quoting them); CodeSandbox blog ×3; CH v52.0/v53.0 notes. **Confidence: High** on the mechanism (3 independent implementations); **Medium** on the marginal-cost arithmetic below, which is mine.

**Analysis [INFERENCE]**: The marginal host-RAM cost of the (N+1)-th restore of one snapshot is
its **write working set**, not its configured RAM. A 2 GiB agent sandbox that touches 200 MiB
costs ~200 MiB of private pages plus a shared 2 GiB page-cache footprint amortised across the
pool. That is the entire economic basis of a warm pool, and it is a property of the **memory
file**, not of the hypervisor — **[INFERENCE]** so it should transfer to CH's `memory-ranges`
unchanged.

**[MEASURED] — our own supporting datum**: `--memory shared=on` already gives us the memfd
shape this needs. At the beacon, with 128 MiB touched: `noshare` = `RssAnon` 273232 kB /
`RssShmem` 4 kB; `sharedonly` = `RssAnon` 852 kB / `RssShmem` 260952 kB behind
`/memfd:ch_ram` (`spike/findings.md` § P6). **The footprint is already file-backed and
shareable.** → **SPIKE THIS S-5.**

**⚠ Safety collision, restated because it is easy to lose**: this is the exact shape
Firecracker calls insecure — *"resuming execution from the same state more than once"* risking
*"unique identifiers, random numbers and random number seeds, the guest OS entropy pool, as
well as cryptographic tokens."* **The cheapest thing that makes a warm pool economical is the
thing that makes it unsafe.** VMGenID is the mitigation, it is **[UNRESOLVED]** on CH, and it
leaves *"a race window between resuming vCPUs and Linux CSPRNG getting successfully re-seeded"*
even when present.

### P6.2 Snapshot size vs guest RAM, and incremental/differential snapshots

**Finding P6-2: Snapshot size ≈ guest RAM unless the file is sparse. Sparse handling landed in
CH v52.0; diff snapshots exist on Firecracker but in a limited form.**

**Evidence [DOCUMENTED]**:
- CH v46: *"`memory-ranges` stores the content of the guest RAM"* — no sparseness mentioned.
- CH v52.0: *"Snapshot and restore now treat the memory backing file as a sparse file"* (#8113).
- Firecracker `memory-hotplug.md`: *"Full and diff snapshots will include the unplugged areas
  as sparse 'holes' in the memory snapshot file"*, recommending sparse-file support.
- Firecracker diff snapshots (via prior research, quoting Firecracker docs): *"still in
  developer preview"*, *"not resume-able directly, must be merged with a base."*

**Sources**: CH v46.0 doc; CH v52.0 notes; Firecracker `memory-hotplug.md`; prior research § 2.4. **4 sources. Confidence: High.**

**Evidence [MARKETING, but the arithmetic is the useful part]**: CodeSandbox — *"creating a
snapshot takes about 1 second per gigabyte to write, meaning a 12 GB VM would take 12
seconds."*

**Analysis [INFERENCE]**: On **v46**, budget a snapshot at **≈ full guest RAM on disk** and
**≈1 s/GiB to write** unless measured otherwise. That is a materially different cost model from
Sprites' *"300 ms"* — because Sprites' 300 ms is the **filesystem** checkpoint (P1.2), not the
memory snapshot. **Do not plan against 300 ms.** → **SPIKE THIS S-7.**

### P6.3 Scale-to-zero economics — what is billed while suspended

**Finding P6-3: While asleep, the customer is billed for cold storage bytes only; the platform
absorbs the memory-snapshot and cache costs. Sprites' own pricing encodes this as a ~25×
hot/cold ratio.**

**Evidence [DOCUMENTED]**: *"auto-sleep to a cheap inactive state"*; sprites *"cost practically
nothing while asleep"*; sleeps after 30 s idle.
**Evidence [DOCUMENTED, ~4 months stale]** (prior research § 1.5, quoting fly.io/sprites):
storage hot (NVMe cache) **$0.000683/GB-hour**; cold (object) **$0.000027/GB-hour**; CPU
$0.07/CPU-hour with a 6.25% CPU/s minimum; memory $0.04375/GB-hour with a 0.25 GB/s minimum.

**Sources**: fly.io/blog/design-and-implementation; `sprites-as-overdrive-primitive-research.md` § 1.5. **2 sources. Confidence: Medium** (pricing is time-sensitive and 4 months old).

**Analysis [INFERENCE]** — three consequences for Overdrive:

1. **The pricing is the architecture, restated.** A ~25× hot:cold ratio only makes sense if the
   sleeping state's bytes genuinely live in object storage and the NVMe copy is discardable —
   *"nothing in that NVMe volume should matter."* A design where a sleeping VM pins a host's
   local disk cannot offer this shape, because the bytes are still occupying premium storage.
2. **[INFERENCE] The memory snapshot is a platform cost centre, not a billable.** kurt: *"We
   keep memory snapshots around for as long as possible"* — "as long as possible", not "as long
   as you pay". That is a cache-eviction policy. It is also why the reboot fallback must exist.
3. **[INFERENCE] For Overdrive specifically:** scale-to-zero with fast wake requires the
   *filesystem* to be cheap-and-durable independently of the memory snapshot. That is #97's
   chunk store, and per **R4** it belongs **under a block device**. A virtiofs volume whose
   source directory must stay resident on a specific host's filesystem for the VM to be
   restorable is the *opposite* of this economic model.

---

## SPIKE THIS — questions our bare-metal box could settle cheaply

Environment B (`infra/metal/`, commit `38870e9e`): AMD EPYC 8024P, non-nested, CH v46.0.0,
virtiofsd 1.13.2, XFS(reflink=1) at `/srv/vm`, kernel 7.0.0-15. Every probe below reuses the
existing `spike-scratch/increment-{a,e,f}` harnesses. Per `.claude/rules/spike.md`: gitignored
`spike-scratch/increment-*`, `crates/` untouched, run for real under the real kernel, record
`uname -r`.

**Ordered by value-per-minute.**

---

### S-1 — Does `vm.snapshot` / `vm.restore` work AT ALL on v46 with our block-only shape?

**Everything in this document is downstream of this and it has never been run.**

- **Hypothesis**: a CH v46 VM with `--disk` rootfs + `--disk` volume + `--vsock`, no `--fs`,
  snapshots and restores to a running guest.
- **Predicted**: `ch-remote pause` → `snapshot` yields `config.json` / `memory-ranges` /
  `state.json`; `--restore source_url=file://…` + `resume` returns a live guest whose
  in-memory state (a counter the guest was incrementing) continues from where it stopped, and
  whose volume writes are intact.
- **Falsification**: restore errors, hangs, or the guest resumes with a corrupt/rolled-back
  volume.

```bash
# reuse increment-f's VM shape; add:
ch-remote --api-socket "$API" pause
ch-remote --api-socket "$API" snapshot "file:///srv/vm/snap-a"
ls -l --apparent-size /srv/vm/snap-a          # size vs --memory size=
cloud-hypervisor --api-socket "$API2" --restore source_url=file:///srv/vm/snap-a
ch-remote --api-socket "$API2" resume
```

**Also captures S-7 for free** (`time` the snapshot; `du --apparent-size` vs `du` for sparseness).

---

### S-2 — Does a NON-ROOT `--fs` virtiofs VOLUME survive snapshot/restore on v46?

**This is the probe that can overturn recommendation R1.** Issue #6931 is about a virtiofs
**root**; a volume may behave differently, and nobody has checked.

- **Hypothesis**: restore hangs or the guest's open virtiofs file handles are invalid, because
  the daemon that issued them is gone (P5.1, P5.3).
- **Predicted**: one of — (a) `vm.restore` hangs at device init, matching #6931; (b) restore
  succeeds but guest I/O to the share blocks forever (no resume interrupt, CH #7851, post-v46);
  (c) it works, and R1 must be revisited.
- **Falsification of my recommendation**: outcome (c), **and** the guest's pre-snapshot open
  `fd` still reads/writes the correct host file after the daemon was restarted.

```bash
# increment-e 'full' mode, but volume-only (block rootfs, --fs volume)
# guest holds an fd open across the checkpoint, then writes through it on resume
# three arms:
#   1. virtiofsd killed and re-spawned before restore   <- the realistic case
#   2. virtiofsd left running across the checkpoint     <- the optimistic case
#   3. host mutates the share while checkpointed        <- the P5.4 soundness case
```

Arm 3 is the important one: it tests **soundness**, not just liveness.

---

### S-3 — What exactly happens to the vsock beacon connection across restore on v46?

Our Running gate rides this channel (`spike/findings.md` § P2), and CH only started resetting
stale connections in v52.0 (#7958).

- **Hypothesis**: the guest's connection is left half-open; the host-side UDS listener at
  `<socket_path>_<port>` is gone; the guest sees no error until it writes.
- **Predicted**: guest `write()` → `EPIPE`/`ECONNRESET`, **or** an indefinite hang (worse).
  Host `accept()` on the recreated path succeeds only if the driver recreated the directory.
- **Falsification**: the connection transparently survives.
- **Cheap add-on**: does `vm.restore` fail with `PermissionDenied` if the per-VM socket
  directory does not exist / is outside the Landlock grant? (P2.d inference — direct driver
  requirement.)

---

### S-4 — Does Cloud Hypervisor expose a VMGenID device?

**Flagged UNVERIFIED on 2026-04-19 and still unverified.** A premise of GH #96 and of any
fan-out warm pool. **~5 minutes.**

```bash
cloud-hypervisor --help | grep -i -e vmgenid -e 'generation'
# and in-guest:
ls /sys/firmware/acpi/tables/ ; dmesg | grep -i -e vmgenid -e 'random: .*seed'
```

- **Predicted**: `--platform` may carry a generation-id option; guest kernel ≥5.18 logs a
  CSPRNG reseed on restore if present.
- **Falsification**: no flag, no ACPI table, no reseed log → warm-pool fan-out is **unsafe** on
  v46 and must be designed around (P3.5).

---

### S-5 — Does one `memory-ranges` file CoW-share across N restores?

The economic premise of a warm pool (P6.1), inferred from Firecracker and never measured on CH.

- **Hypothesis**: restoring the same snapshot into N VMs costs ~1× RAM in page cache + N×
  write-working-set in private pages.
- **Predicted**: `VmRSS` per VM ≫ incremental system-wide `MemAvailable` delta; `RssShmem` /
  `RssFile` dominates; `smaps_rollup` `Private_Dirty` ≈ the touched set.
- **Falsification**: system-wide memory drops by ~full guest RAM per restored VM → no sharing,
  warm pools cost full price.

```bash
for i in 1 2 3 4; do restore_from /srv/vm/snap-a "$i" & done
grep -e VmRSS -e RssAnon -e RssShmem /proc/<pid>/status
cat /proc/<pid>/smaps_rollup      # Private_Dirty vs Shared_*
free -m                            # system-wide delta per additional VM
```

---

### S-6 — Does CPU hotplug still work on a snapshot-restored CH VM?

**[DOCUMENTED]** Firecracker forbids hotplug on restored VMs. CH is silent. **If CH shares the
restriction, it damages GH #92 for exactly the long-lived workloads that motivate both
features** — and nobody has checked that these two separately-justified capabilities compose.

- **Predicted**: `ch-remote resize --cpus N` on a restored VM either succeeds (CH differs from
  Firecracker — good) or returns an error naming the snapshot (matching Firecracker).
- **Falsification of the optimistic case**: any error, or a guest that never sees the new vCPU.

---

### S-7 — Snapshot size and write time vs guest RAM (folded into S-1)

- **Predicted**: `memory-ranges` apparent size ≈ `--memory size=`; actual (`du`) smaller only
  if sparse — and **[DOCUMENTED]** sparse handling is a **v52.0** feature (#8113), so expect
  **non-sparse** at v46. Write time ~1 s/GiB **[MARKETING baseline, CodeSandbox]**.
- **Falsification**: sub-second for a 512 MiB guest, or an actual size far below apparent →
  v46 is sparser than the changelog implies.

---

### S-8 — `vhost-user-blk` as the #97 seam (per R4)

`spike/findings.md` § P7 explicitly lists this as unmeasured. If `overdrive-fs` is to sit under
a block device, this is the seam.

- **Predicted**: `--disk vhost_user=on,socket=…` boots and round-trips; it **reintroduces a
  per-VM daemon** and therefore inherits P5.1's restore ordering problem — but **not** P5.3's
  file-handle problem, because the guest's handles are to its own filesystem inside the image.
- **Falsification**: it does not boot on v46, or it needs `--memory shared=on` in a way that
  changes the P6 memory profile.

---

**Not worth spiking**: anything requiring a CH ≥ v52 build. That is a *version-floor decision*
(R5) for the user and the architect, not a probe — though installing v52 on the metal box and
re-running **S-2** would convert R3 from an inference into a measurement, and is the single
highest-value follow-up if checkpoint/restore (#96) is promoted to real scope.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access date | Cross-verified |
|---|---|---|---|---|---|
| cloud-hypervisor `docs/snapshot_restore.md` @ **`v46.0`** | raw.githubusercontent.com | High (1.0) | official/technical | 2026-08-10 | Y (vs `main`, vs release notes) |
| cloud-hypervisor `docs/snapshot_restore.md` @ `main` | raw.githubusercontent.com | High (1.0) | official/technical | 2026-08-10 | Y |
| cloud-hypervisor **issue #6931** (virtiofs root restore hangs, OPEN) | github.com | Medium-High (0.8) | official tracker | 2026-08-10 | Y (v52.0 notes #7908/#7851) |
| cloud-hypervisor issue #7250 (migrate virtio-fs to `fuse-backend-rs`) | github.com | Medium-High (0.8) | official tracker | 2026-08-10 | **N — single source** |
| cloud-hypervisor **Release v52.0** | github.com | Medium-High (0.8) | official changelog | 2026-08-10 | Y (fetched twice, 2 pages) |
| cloud-hypervisor Releases index (v48–v53) | github.com | Medium-High (0.8) | official changelog | 2026-08-10 | Y |
| Firecracker `docs/snapshotting/snapshot-support.md` | github.com | High (1.0) | official/normative | 2026-08-10 | Y (prior research § 2.4) |
| Firecracker `docs/snapshotting/random-for-clones.md` | github.com | High (1.0) | official/normative | 2026-08-10 | Y |
| Firecracker `docs/snapshotting/handling-page-faults-on-snapshot-resume.md` | github.com | High (1.0) | official/normative | 2026-08-10 | Y (CH #7800) |
| Firecracker `docs/memory-hotplug.md` | github.com | High (1.0) | official/normative | 2026-08-10 | Y (this feature's `intake.md`) |
| Fly.io — *The Design & Implementation of Sprites* | fly.io | High (1.0) | official vendor eng. | 2026-08-10 | Y (prior research, fly.io/sprites) |
| Fly.io — Sprites product page | fly.io | High (1.0) | official vendor | 2026-08-10 | Y |
| community.fly.io — *How is sprite memory snapshotted and restored?* (**staff** reply) | community.fly.io | High (1.0) for staff reply | official vendor statement | 2026-08-10 | **N — single source, but it CORRECTS 2 others** |
| libvirt — *Sharing files with Virtiofs* | libvirt.org | High (0.9) — **⚠ off trusted list, flagged** | official OSS project docs | 2026-08-10 | Y (virtio-fs list, Proxmox patch, CH v52.0) |
| CodeSandbox eng. blog ×3 (userfaultfd cloning, 2 s clone, memory decompression) | codesandbox.io | Medium (0.6) — **vendor, not on list** | vendor eng. blog | 2026-08-10 | Y (Firecracker UFFD docs) — **[403 on direct fetch; index-sourced quotes]** |
| Blacksmith — persistent-storage post + docs | info.blacksmith.sh / docs.blacksmith.sh | Medium (0.6) — **vendor, competitive-marketing bias flagged** | vendor | 2026-08-10 | N — 2 sources, same vendor = 1 independent |
| **In-tree** `spike/findings.md` (this feature) | local | **[MEASURED]** — highest authority in this doc | own measurement | 2026-08-10 | n/a |
| **In-tree** `sprites-as-overdrive-primitive-research.md` (2026-04-19) | local | High (prior research, 24 sources) | prior art | 2026-08-10 | Partially **superseded** — see Conflicts |
| **In-tree** `intake.md` (this feature) | local | High | project decision record | 2026-08-10 | n/a |

**Reputation distribution**: High: 12 (~63%) | Medium-High: 4 (~21%) | Medium: 3 (~16%) | **Average ≈ 0.87**.
**Excluded-domain hits**: none encountered. **medium.com** results surfaced in one search and were **not used** (author unverified; the same claims were available from primary sources).

---

## Knowledge Gaps

### G1 — Does a NON-ROOT virtiofs volume behave differently from a virtiofs root under restore?
**Issue**: Issue #6931 is specifically about a virtiofs **root filesystem**. Our proposed use is
a **volume**, with a block rootfs. The failure could be root-specific (early boot device ordering)
or general (vhost-user resume). **Attempted**: searched the CH tracker for vhost-user/virtiofs
snapshot issues; the only directly-on-point issue is #6931. **Recommendation**: **SPIKE THIS S-2**
— cheaper and more reliable than further literature search.

### G2 — Does `vm.snapshot`/`vm.restore` work at all on v46 in OUR configuration?
**Issue**: Nothing in `spike/findings.md` touches snapshot/restore. Every P2 answer here is from
upstream documentation, not from our binary on our box. **Attempted**: n/a — this is a
measurement gap, not a source gap. **Recommendation**: **SPIKE THIS S-1**. This is the largest
gap in the document.

### G3 — Does Cloud Hypervisor forbid CPU/memory hotplug on snapshot-restored VMs?
**Issue**: Firecracker **[DOCUMENTED]** forbids it. CH's v46 doc is silent, and I found no CH
issue or doc either way. CPU hotplug is the *entire* stated justification for choosing CH
(`intake.md` § "The VMM premise") and GH #92 depends on it. **Attempted**: CH snapshot docs at
two refs; release notes v48–v53. **Recommendation**: **SPIKE THIS S-6.**

### G4 — Does Cloud Hypervisor expose a VMGenID device?
**Issue**: Flagged UNVERIFIED in this feature's `intake.md` on 2026-04-19 and **still unverified**.
It is a premise of GH #96 and of any warm-pool fan-out. **Attempted**: CH docs, release notes.
**Recommendation**: **SPIKE THIS S-4** (~5 minutes).

### G5 — No independent verification of any vendor latency claim
**Issue**: Sprites *"300 ms checkpoint / ~1 s restore"*, E2B *"~150 ms"*, Blacksmith *"5-20 ms
snapshot-resume"*, CodeSandbox *"2 s clone"* — **all vendor self-reported**, none with a stated
methodology, guest RAM size, payload, or percentile. **Attempted**: searched for third-party
benchmarks of Sprites; none found. **Recommendation**: treat every number as **[MARKETING]** and
never as an engineering target. Our own S-1/S-7 produce the only numbers we should plan against.

### G6 — Daytona and Northflank not independently researched in this pass
**Issue**: Named in the dispatch; not covered beyond what prior research holds. **Attempted**:
deprioritised against P2/P5, which are the decision-bearing questions. **Recommendation**: low
value — the P5.5 convergence is already six-for-six and two more vendors are unlikely to move it.

### G7 — Release **dates** for CH versions are unreliable in this document
**Issue**: The releases-index fetch returned dates (e.g. *"v53.0 — July 12, 2024"*) that cannot
be reconciled with v46 being current in 2026. **Attempted**: two fetches; the *content* agreed,
the *dates* are not trustworthy. **Recommendation**: **no date is cited anywhere in this
document.** The argument rests only on version **ordering**, which is unambiguous. Verify dates
with `gh release view` before quoting any in a design artifact.

### G8 — aarch64 remains unmeasured for everything storage-related
**Issue**: Inherited from `spike/findings.md` § Still open — virtiofs + `shared=on` is proven on
**one of two** shipping arches. Snapshot/restore on aarch64 is unmeasured too. **Recommendation**:
same hardware blocker; one non-nested Arm box closes this, P3 and P6-aarch64 together.

---

## Conflicting Information

### Conflict 1 — Does sprite RAM persist across sleep/wake?
**Position A**: *"RAM doesn't persist across hibernation and wake."*
— Source: Fly documentation, as quoted in a community thread and recorded in
`docs/research/platform/sprites-as-overdrive-primitive-research.md` § 1.3 (2026-04-19).
**Position B**: *"Sprites get memory snapshotted and then restored next time you use them. We
keep memory snapshots around for as long as possible."* — and, of Position A's doc line:
**"Those docs are wrong, btw. We're correcting them."**
— Source: **kurt, Fly staff**, [community.fly.io](https://community.fly.io/t/how-is-sprite-memory-snapshotted-and-restored/26843), Accessed 2026-08-10.
**Assessment**: **Position B wins, decisively.** A named staff member of the vendor, speaking on
the vendor's own property, explicitly labelling the conflicting document as wrong and
in-correction, outranks the document. **Consequence: our own prior research doc § 1.3 is now
partially superseded and should be annotated by whoever next touches it.** The nuance that
survives from Position A: memory restore is **best-effort** (*"as long as possible"*), and
sprites *"will reboot (because crash, upgrade, cold, etc.)"* — so a user may well *observe*
RAM not persisting, which is probably how the doc came to say it.

### Conflict 2 — Does Cloud Hypervisor have `userfaultfd` lazy-paging parity with Firecracker?
**Position A**: *"Cloud Hypervisor has snapshot/restore with `userfaultfd` lazy-paging parity
with Firecracker."* — Source: `docs/research/platform/sprites-as-overdrive-primitive-research.md`
Executive Summary (2026-04-19), Reputation: High (our own prior research, 24 sources).
**Position B**: `memory_restore_mode` / `userfaultfd` restore is a **v52.0** feature (#7800), and
background prefault a **v53.0** feature. The `v46.0` doc contains neither term.
— Source: [cloud-hypervisor v52.0 release notes](https://github.com/cloud-hypervisor/cloud-hypervisor/releases/tag/v52.0) + the `v46.0` tagged doc, Accessed 2026-08-10.
**Assessment**: **Position B is more authoritative** — it is the upstream project's own changelog
plus its own documentation at a specific tag, versus a summary claim in a secondary document. The
charitable reading is that Position A described CH's trajectory rather than a shipped v46
capability. **At v46 the parity claim is false.** This matters because it is exactly the kind of
claim that would justify "CH can do the Sprites shape" in a design review.

### Conflict 3 — Is virtiofs supported under snapshot/restore?
**Position A (implied)**: CH v52.0 — *"Snapshot/restore support for `vhost-user` devices has been
filled out, including migration support for `virtio-fs`."* Reads as "yes".
**Position B**: libvirt — *"Snapshot operations… do not snapshot the state of the files shared via
virtiofs, and thus reverting to an earlier state is not recommended"*; and `file-handles`
migration requires *"the virtiofsd source instance keeps running until migration is fully
complete."*
**Assessment**: **Not actually a contradiction — a conflation, and a dangerous one.** Position A
is about **live migration** (two live daemons, no temporal gap); Position B is about
**snapshot/revert** (temporal gap, no source daemon). Both are correct within their scope. **The
hazard is that Position A's wording will be read as settling Position B's question.** Position B
is the one that governs the Sprites shape. Recommendation **R3** exists specifically to stop this
conflation from being made later.

---

## Recommendations for Further Research

1. **Run S-1 before any DESIGN work on GH #96.** Everything in P2 is upstream documentation about
   a binary we have never asked to snapshot. One afternoon converts the whole section from
   documented-inference to measured.
2. **Install CH v52.0 on the metal box and re-run S-2.** This is the only way to convert R3 from
   inference to measurement, and it directly informs the R5 version floor.
3. **Research `overdrive-fs` (#97) as a block-backing chunk store, not a `vhost-user-fs`.** The
   Sprites architecture is the strongest available prior art and it points at block. This is a
   scoped, separate research question worth its own document.
4. **Re-verify Sprites pricing and the sleep threshold before any commercial modelling** — the
   figures carried here are ~4 months old and time-sensitive.
5. **Annotate `sprites-as-overdrive-primitive-research.md`** with Conflicts 1 and 2. Per
   CLAUDE.md § "Behavior change must mark stale adjacent docs", a document this one supersedes in
   two places should not be left standing unmarked.

---

## Full Citations

[1] Cloud Hypervisor. "Snapshot and Restore" (`docs/snapshot_restore.md`, tag **v46.0**). cloud-hypervisor. https://raw.githubusercontent.com/cloud-hypervisor/cloud-hypervisor/v46.0/docs/snapshot_restore.md. Accessed 2026-08-10.
[2] Cloud Hypervisor. "Snapshot and Restore" (`docs/snapshot_restore.md`, branch `main`). https://raw.githubusercontent.com/cloud-hypervisor/cloud-hypervisor/main/docs/snapshot_restore.md. Accessed 2026-08-10.
[3] Cloud Hypervisor. "Unable to restore a snapshot of vm using virtiofs root" (Issue #6931, **OPEN**, affects v43.0.0). https://github.com/cloud-hypervisor/cloud-hypervisor/issues/6931. Accessed 2026-08-10.
[4] Cloud Hypervisor. "migrate virtio-fs to fuse-backend-rs" (Issue #7250). https://github.com/cloud-hypervisor/cloud-hypervisor/issues/7250. Accessed 2026-08-10.
[5] Cloud Hypervisor. "Release v52.0" (release notes; PRs #7800, #7851, #7908, #7932, #7937, #7958, #8004, #8016, #8099, #8113). https://github.com/cloud-hypervisor/cloud-hypervisor/releases/tag/v52.0. Accessed 2026-08-10.
[6] Cloud Hypervisor. "Releases" (index; v48.0–v53.0 notes). https://github.com/cloud-hypervisor/cloud-hypervisor/releases. Accessed 2026-08-10.
[7] Firecracker. "Firecracker Snapshotting" (`docs/snapshotting/snapshot-support.md`). https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/snapshot-support.md. Accessed 2026-08-10.
[8] Firecracker. "Entropy and randomness for clones" (`docs/snapshotting/random-for-clones.md`). https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/random-for-clones.md. Accessed 2026-08-10.
[9] Firecracker. "Handling page faults on snapshot resume" (`docs/snapshotting/handling-page-faults-on-snapshot-resume.md`). https://github.com/firecracker-microvm/firecracker/blob/main/docs/snapshotting/handling-page-faults-on-snapshot-resume.md. Accessed 2026-08-10.
[10] Firecracker. "Memory hotplug" (`docs/memory-hotplug.md`). https://github.com/firecracker-microvm/firecracker/blob/main/docs/memory-hotplug.md. Accessed 2026-08-10.
[11] Fly.io. "The Design & Implementation of Sprites". The Fly Blog. https://fly.io/blog/design-and-implementation/. Accessed 2026-08-10.
[12] Fly.io. "Sprites — Stateful sandbox environments". https://fly.io/sprites/. Accessed 2026-08-10.
[13] kurt (Fly.io staff). Reply in "How is sprite memory snapshotted and restored?". Fly.io Community. https://community.fly.io/t/how-is-sprite-memory-snapshotted-and-restored/26843. Accessed 2026-08-10.
[14] Fly.io. "What Is a Firecracker VM?". https://fly.io/learn/firecracker-vm/. Accessed 2026-08-10.
[15] libvirt. "Sharing files with Virtiofs". https://libvirt.org/kbase/virtiofs.html. Accessed 2026-08-10. **[Off trusted-domain list — flagged; corroborated by [5], [16], [17]]**
[16] virtio-fs project. "Live migration support for virtio-fs" (mailing list). https://www.mail-archive.com/virtio-fs@redhat.com/msg02983.html. Accessed 2026-08-10.
[17] Frank, M. "[PATCH docs v3 8/11] virtiofs: add documentation for live migration". Proxmox. https://lore.proxmox.com/all/20260427121746.270544-9-m.frank@proxmox.com/. Accessed 2026-08-10.
[18] CodeSandbox. "Cloning microVMs by sharing memory through userfaultfd". https://codesandbox.io/blog/cloning-microvms-using-userfaultfd. Accessed 2026-08-10. **[Vendor; direct fetch HTTP 403 — quotes from search index]**
[19] CodeSandbox. "How we clone a running VM in 2 seconds". https://codesandbox.io/blog/how-we-clone-a-running-vm-in-2-seconds. Accessed 2026-08-10. **[Vendor]**
[20] CodeSandbox. "How we scale our microVM infrastructure using low-latency memory decompression". https://codesandbox.io/blog/how-we-scale-our-microvm-infrastructure-using-low-latency-memory-decompression. Accessed 2026-08-10. **[Vendor]**
[21] Blacksmith. "Which GitHub Actions runner providers give you fast persistent storage between CI jobs?". https://info.blacksmith.sh/task/blog/github-actions-fast-persistent-storage. Accessed 2026-08-10. **[Vendor; competitive-marketing bias flagged]**
[22] Blacksmith. "Instance Types". Blacksmith Docs. https://docs.blacksmith.sh/blacksmith-runners/overview. Accessed 2026-08-10. **[Vendor]**
[23] Overdrive (in-tree). `docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md` — P1/P2/P4/P5/P6/P7 bare-metal measurements, 2026-08-10. **[MEASURED — highest authority in this document]**
[24] Overdrive (in-tree). `docs/research/platform/sprites-as-overdrive-primitive-research.md`, 2026-04-19, 24 sources. **[Prior art; partially superseded — see Conflicts 1 and 2]**
[25] Overdrive (in-tree). `docs/feature/microvm-driver-cloud-hypervisor/intake.md` — Decisions I-2, I-5, I-6; the VMM premise; version-floor warning.

---

## Research Metadata

**Duration**: ~45 turns | **Sources examined**: 25 | **Sources cited**: 25 | **Cross-references performed**: 14
**Confidence distribution** (major findings): High ~70% | Medium ~25% | Low/Unresolved ~5%
**Tool failures**: 3 — (a) `raw.githubusercontent.com/.../v46.0.0/...` HTTP 404 (wrong tag form; resolved by retrying `v46.0`); (b) `fly.io/blog/sprites-design/` HTTP 404 (resolved via search → `/blog/design-and-implementation/`); (c) `codesandbox.io/blog/cloning-microvms-using-userfaultfd` **HTTP 403** — quotes for [18] are search-index-sourced, not full-text, and confidence is downgraded to Medium accordingly.
**Adversarial validation**: applied to all fetched content per `nw-operational-safety`. No prompt-injection, authority-impersonation, or directive language detected in any source. Vendor persuasion language was detected throughout P4 and is handled by the **[MARKETING]** marker rather than by exclusion.
**Output**: `docs/research/platform/persistent-microvm-checkpoint-restore-comprehensive-research.md`
**Decision this informs**: `I-6` (volume storage mechanism) — see **P5.6**. Downstream: GH [#42](https://github.com/overdrive-sh/overdrive/issues/42), [#96](https://github.com/overdrive-sh/overdrive/issues/96), [#97](https://github.com/overdrive-sh/overdrive/issues/97), [#92](https://github.com/overdrive-sh/overdrive/issues/92), [#257](https://github.com/overdrive-sh/overdrive/issues/257).
